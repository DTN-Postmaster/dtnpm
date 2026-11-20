//! Server initialization and lifecycle management for `dtnpm`.

use std::net::{SocketAddr, ToSocketAddrs};
#[cfg(feature = "web-core")]
use std::num::NonZeroU32;
use std::sync::Arc;

use anyhow::Context;
use hardy_async::TaskPool;
use hardy_bpa::{
    bpa::Bpa,
    cla::{Cla, ClaInit},
    node_ids::NodeIds,
    storage::{BundleStorage, MetadataStorage},
};
use hardy_localdisk_storage::LocalDiskStorage;
use hardy_proto::server::GrpcServer;
use hardy_sqlite_storage::SqliteStorage;
use tracing::{info, warn};

use dtnpm_core::cla_manager::ClaManager;
use dtnpm_core::config::{HamConfig, cla::ClaType, cla::ScheduleConfig};
use dtnpm_core::coordinator::{ContactCoordinator, ContactSchedule, ContactTarget};
use dtnpm_core::internal_storage::InternalStorage;
#[cfg(feature = "web-core")]
use dtnpm_core::log_buffer::LogBuffer;
use dtnpm_core::routing::RouteManager;
use dtnpm_core::security::DirectionalKeyStore;
use dtnpm_core::service_tracker::ServiceTracker;

#[cfg(feature = "web-core")]
use dtnpm_core::{BpaClient, LocalBpaClient};
#[cfg(feature = "web-core")]
use dtnpm_web_core::{ModuleContext, WebBundleInjector, WebModule};

/// The standalone BPA server instance.
pub struct HamServer {
    config: HamConfig,
    bpa: Arc<Bpa>,
    service_tracker: ServiceTracker,
    grpc_config: Option<hardy_proto::server::Config>,
    tcp_peers: Vec<(Arc<hardy_tcpclv4::Tcpclv4>, Vec<SocketAddr>)>,
    coordinators: Vec<ContactCoordinator>,
    #[cfg(feature = "web-core")]
    web_injector: Option<WebBundleInjector>,
    #[cfg(feature = "web-core")]
    web_modules: Vec<Arc<dyn WebModule>>,
    cla_manager: ClaManager,
    internal_storage: InternalStorage,
    route_manager: Arc<RouteManager>,
    keystore: Arc<DirectionalKeyStore>,
    _ephemeral_dir: Option<tempfile::TempDir>,
    tasks: TaskPool,
}

impl HamServer {
    /// Constructs a new `HamServer` from the provided configuration.
    pub async fn new(mut config: HamConfig) -> anyhow::Result<Self> {
        let tasks = TaskPool::new();
        let cla_manager = ClaManager::new();
        let mut ephemeral_dir = None;

        // 1. Storage setup
        let (metadata_storage, bundle_storage, internal_storage): (
            Arc<dyn MetadataStorage>,
            Arc<dyn BundleStorage>,
            InternalStorage,
        ) = if config.storage.in_memory {
            info!("Using ephemeral SQLite bundle and metadata storage for in-memory mode");
            let temp = tempfile::Builder::new()
                .prefix("dtn-ham-mem-")
                .tempdir()
                .context("Failed to create temporary directory for in-memory storage")?;
            let data_dir = temp.path().to_path_buf();
            let db_path = data_dir.join("bundles.db");
            let bundle_dir = data_dir.join("payloads");
            let _ = std::fs::create_dir_all(&data_dir);
            let _ = std::fs::create_dir_all(&bundle_dir);

            config.storage.data_dir = Some(data_dir.clone());
            config.storage.sqlite_path = Some(db_path);

            let meta_store = Arc::new(SqliteStorage::new(
                Some(data_dir),
                Some("bundles.db".to_string()),
                true,
            ));
            let bundle_store = Arc::new(LocalDiskStorage::new(Some(bundle_dir), Some(false), true));
            ephemeral_dir = Some(temp);
            let app_store =
                InternalStorage::new_in_memory_with_quota(config.application.quota.max_storage)?;
            (meta_store, bundle_store, app_store)
        } else {
            let data_dir = config.storage.effective_data_dir();
            let db_path = config.storage.effective_db_path();
            let bundle_dir = config.storage.effective_payload_dir();

            let _ = std::fs::create_dir_all(&data_dir);
            let _ = std::fs::create_dir_all(&bundle_dir);

            info!("Using SQLite metadata storage at {}", db_path.display());
            info!(
                "Using localdisk bundle payload storage at {}",
                bundle_dir.display()
            );

            let db_dir = Some(data_dir.clone());
            let db_name = db_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());

            let meta_store = Arc::new(SqliteStorage::new(db_dir, db_name, true));
            let bundle_store = Arc::new(LocalDiskStorage::new(Some(bundle_dir), Some(false), true));
            let app_store = InternalStorage::new_disk_with_quota(
                &data_dir,
                config.application.quota.max_storage,
            )?;
            (meta_store, bundle_store, app_store)
        };

        // 2. Parse Node IDs
        let mut endpoints = Vec::new();
        endpoints.push(config.node_id.parse().context("Invalid primary node-id")?);
        for ep in &config.admin_endpoints {
            endpoints.push(ep.parse().context("Invalid admin-endpoint")?);
        }
        let node_ids =
            NodeIds::try_from(endpoints.as_slice()).context("Invalid node-ids configuration")?;

        let mut builder = Bpa::builder()
            .node_ids(node_ids)
            .metadata_storage(metadata_storage)
            .bundle_storage(bundle_storage);

        let rfc9171 = config.rfc9171_validity.clone().unwrap_or_default();
        if let Some(enabled) = rfc9171.primary_block_integrity {
            builder = builder.primary_block_integrity(enabled);
        }
        if let Some(enabled) = rfc9171.bundle_age_required {
            builder = builder.bundle_age_required(enabled);
        }

        // 3. Register Dynamic Route Manager
        let route_manager = Arc::new(RouteManager::new(
            config.routes.clone(),
            config.config_path.clone(),
        ));
        builder = builder.routing_agent("ham_routes".to_string(), route_manager.clone());

        #[cfg(feature = "mod-rego-routing")]
        let rego_module = {
            let rego_yaml = config.rego_routing.as_ref();
            let rego_cfg = dtnpm_mod_rego_routing::RegoRoutingConfig {
                local_eid: config.node_id.clone(),
                node_eids: endpoints.iter().map(ToString::to_string).collect(),
                callsign: config.callsign.clone(),
                policy_dir: rego_yaml.and_then(|r| r.policy_dir.clone()),
                custom_ingress_rego: rego_yaml.and_then(|r| r.custom_ingress_rego.clone()),
                custom_egress_rego: rego_yaml.and_then(|r| r.custom_egress_rego.clone()),
                custom_routing_rego: rego_yaml.and_then(|r| r.custom_routing_rego.clone()),
                ..Default::default()
            };
            let module = Arc::new(
                dtnpm_mod_rego_routing::RegoRoutingModule::with_config(rego_cfg)
                    .context("Failed to initialize RegoRoutingModule")?,
            );
            builder =
                builder.routing_agent("rego_routing".to_string(), module.routing_agent().clone());
            let engine_for_rib = module.engine().clone();
            builder = builder.rib(move |default_rib| {
                Arc::new(dtnpm_mod_rego_routing::RegoRib::new(
                    default_rib,
                    engine_for_rib,
                ))
            });
            module
        };

        // 4. Configure CLAs
        let mut tcp_peers = Vec::new();
        let mut coordinators = Vec::new();
        for cla_config in config.clas.clone() {
            let (cla, init, cla_type_name): (Arc<dyn Cla>, ClaInit, &'static str) = match cla_config
                .cla_type
            {
                ClaType::TcpClv4(tcp_cfg) => {
                    let mut b = hardy_tcpclv4::Tcpclv4::builder();
                    if let Some(listeners) = tcp_cfg.listeners {
                        for addr in listeners {
                            b = b.listen(addr);
                        }
                    } else if let Some(addr) = tcp_cfg.address {
                        b = b.listen(addr);
                    } else {
                        b = b.listen_default();
                    }

                    if tcp_cfg.require_tls == Some(false) {
                        info!(
                            "TCPCLv4 '{}': TLS disabled for amateur radio compliance",
                            cla_config.name
                        );
                    } else if tcp_cfg.require_tls == Some(true) {
                        warn!("TCPCLv4 '{}': TLS enabled", cla_config.name);
                    }

                    if let Some(mru) = tcp_cfg.segment_mru {
                        b = b.segment_mru(mru);
                    }
                    if let Some(mru) = tcp_cfg.transfer_mru {
                        b = b.transfer_mru(mru);
                    }

                    let cla = Arc::new(b.build().context("Failed to build TCPCLv4 CLA")?);
                    let init = cla.cla_init();

                    if let Some(peers) = tcp_cfg.peers {
                        let mut resolved_peers = Vec::new();
                        for peer_str in peers {
                            match peer_str.to_socket_addrs() {
                                Ok(addrs) => {
                                    for addr in addrs {
                                        resolved_peers.push(addr);
                                        cla_manager.register_tcp_peer(cla.clone(), addr).await;

                                        if let Some(ref sched) = cla_config.schedule {
                                            let schedule = match sched {
                                                ScheduleConfig::Recurring { cron, duration } => {
                                                    match dtnpm_core::coordinator::cron::CronExpr::parse(cron) {
                                                        Ok(cron_expr) => Some(ContactSchedule::Recurring {
                                                            cron: cron_expr,
                                                            duration: *duration,
                                                        }),
                                                        Err(e) => {
                                                            warn!(
                                                                "Failed to parse recurring contact cron expression '{}' for TCPCL peer {}: {}",
                                                                cron, addr, e
                                                            );
                                                            None
                                                        }
                                                    }
                                                }
                                                ScheduleConfig::OneShot { start, end } => {
                                                    let start_dt = start.as_ref().and_then(|s| {
                                                        time::OffsetDateTime::parse(
                                                            s,
                                                            &time::format_description::well_known::Rfc3339,
                                                        )
                                                        .ok()
                                                    });
                                                    let end_dt = end.as_ref().and_then(|s| {
                                                        time::OffsetDateTime::parse(
                                                            s,
                                                            &time::format_description::well_known::Rfc3339,
                                                        )
                                                        .ok()
                                                    });
                                                    Some(ContactSchedule::OneShot {
                                                        start: start_dt,
                                                        end: end_dt,
                                                    })
                                                }
                                            };

                                            if let Some(schedule) = schedule {
                                                let endpoint = Arc::new(
                                                    dtnpm_core::coordinator::endpoint::TcpclContactEndpoint::new(
                                                        cla_manager.clone(),
                                                        addr,
                                                    ),
                                                );
                                                let target = ContactTarget {
                                                    callsign: addr.to_string(),
                                                    node_id: None,
                                                    schedule,
                                                    grace_period: std::time::Duration::from_secs(
                                                        30,
                                                    ),
                                                };
                                                coordinators.push(ContactCoordinator::new(
                                                    endpoint,
                                                    vec![target],
                                                ));
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to resolve TCPCL peer address '{}': {}",
                                        peer_str, e
                                    );
                                }
                            }
                        }
                        if !resolved_peers.is_empty() {
                            tcp_peers.push((cla.clone(), resolved_peers));
                        }
                    }

                    (cla, init, "tcpclv4")
                }

                #[cfg(feature = "cla-mercury")]
                ClaType::MercuryArq(mercury_cfg) => {
                    info!(
                        "Initializing Mercury Connected ARQ CLA '{}' (mycall={:?}, ctl={}, data={})",
                        cla_config.name,
                        mercury_cfg.my_callsign,
                        mercury_cfg.control_addr,
                        mercury_cfg.data_addr
                    );

                    let m_cfg = dtnpm_cla_mercury_arq::Config {
                        my_callsign: mercury_cfg.my_callsign.unwrap_or_else(|| {
                            config
                                .callsign
                                .clone()
                                .unwrap_or_else(|| "N0CALL".to_string())
                        }),
                        require_callsign: false,
                        control_addr: mercury_cfg.control_addr,
                        data_addr: mercury_cfg.data_addr,
                        bandwidth: mercury_cfg.bandwidth,
                        connect_timeout: mercury_cfg.connect_timeout,
                        grace_period: mercury_cfg.grace_period,
                        idle_timeout: mercury_cfg.idle_timeout,
                        peers: mercury_cfg
                            .peers
                            .iter()
                            .map(|p| dtnpm_cla_mercury_arq::PeerEntry {
                                callsign: p.callsign.clone(),
                                node_id: p.effective_node_id().parse().ok(),
                            })
                            .collect(),
                    };

                    let cla = Arc::new(dtnpm_cla_mercury_arq::MercuryCla::new(m_cfg));
                    let init = cla.cla_init();

                    let mut targets = Vec::new();
                    for p in &mercury_cfg.peers {
                        let peer_node: Option<hardy_bpv7::eid::NodeId> =
                            match p.effective_node_id().parse() {
                                Ok(nid) => Some(nid),
                                Err(e) => {
                                    warn!(
                                        "Invalid peer node ID '{}' for peer '{}': {}",
                                        p.effective_node_id(),
                                        p.callsign,
                                        e
                                    );
                                    None
                                }
                            };

                        cla_manager
                            .register_contact_peer(
                                p.callsign.clone(),
                                peer_node.clone(),
                                cla.clone(),
                            )
                            .await;

                        if let Some(ref sched) = p.schedule {
                            let schedule = match sched {
                                ScheduleConfig::Recurring { cron, duration } => {
                                    let cron_expr =
                                        dtnpm_core::coordinator::cron::CronExpr::parse(cron)
                                            .map_err(|e| {
                                                anyhow::anyhow!(
                                                    "Invalid cron for peer {}: {e}",
                                                    p.callsign
                                                )
                                            })?;
                                    ContactSchedule::Recurring {
                                        cron: cron_expr,
                                        duration: *duration,
                                    }
                                }
                                ScheduleConfig::OneShot { start, end } => {
                                    let start_dt = start.as_ref().and_then(|s| {
                                        time::OffsetDateTime::parse(
                                            s,
                                            &time::format_description::well_known::Rfc3339,
                                        )
                                        .ok()
                                    });
                                    let end_dt = end.as_ref().and_then(|s| {
                                        time::OffsetDateTime::parse(
                                            s,
                                            &time::format_description::well_known::Rfc3339,
                                        )
                                        .ok()
                                    });
                                    ContactSchedule::OneShot {
                                        start: start_dt,
                                        end: end_dt,
                                    }
                                }
                            };
                            targets.push(ContactTarget {
                                callsign: p.callsign.clone(),
                                node_id: peer_node,
                                schedule,
                                grace_period: mercury_cfg
                                    .grace_period
                                    .unwrap_or(std::time::Duration::from_secs(30)),
                            });
                        }
                    }

                    if !targets.is_empty() {
                        coordinators.push(ContactCoordinator::new(cla.clone(), targets));
                    }

                    (cla, init, "mercury_arq")
                }

                #[cfg(not(feature = "cla-mercury"))]
                ClaType::MercuryArq(_) => {
                    warn!(
                        "Ignoring Mercury ARQ CLA '{}': built without 'cla-mercury' feature",
                        cla_config.name
                    );
                    continue;
                }

                ClaType::Other { ref cla_type, .. } => match cla_type.as_str() {
                    #[cfg(feature = "cla-ltp")]
                    "ltp" => {
                        let ltp_cfg = cla_config
                            .cla_type
                            .parse_as::<dtnpm_cla_ltp_kiss::Config>("ltp")
                            .context("Failed to parse LTP CLA configuration")?
                            .expect("cla_type verified");
                        info!(
                            "Initializing LTP CLA '{}' (Engine ID {})",
                            cla_config.name, ltp_cfg.engine_id
                        );
                        let cla = dtnpm_cla_ltp_kiss::LtpCla::new(ltp_cfg);
                        let init = cla.cla_init();
                        let cla: Arc<dyn Cla> = Arc::new(cla);
                        (cla, init, "ltp")
                    }
                    #[cfg(feature = "cla-hqfbp")]
                    "hqfbp" => {
                        let hqfbp_cfg = cla_config
                            .cla_type
                            .parse_as::<dtnpm_cla_hqfbp_kiss::Config>("hqfbp")
                            .context("Failed to parse HQFBP CLA configuration")?
                            .expect("cla_type verified");
                        info!("Initializing HQFBP CLA '{}'", cla_config.name);
                        let cla = dtnpm_cla_hqfbp_kiss::HqfbpCla::new(hqfbp_cfg);
                        let init = cla.cla_init();
                        let cla: Arc<dyn Cla> = Arc::new(cla);
                        (cla, init, "hqfbp")
                    }
                    _ => {
                        warn!(
                            "Ignoring CLA '{}' with unsupported or disabled type '{}'",
                            cla_config.name, cla_type
                        );
                        continue;
                    }
                },
            };

            #[cfg(feature = "mod-rego-routing")]
            let cla: Arc<dyn Cla> = dtnpm_core::cla_observer::ObservedCla::wrap(
                cla_config.name.clone(),
                cla_type_name,
                cla,
                rego_module.engine().clone(),
            );

            builder = builder.cla(cla_config.name, cla, None, init);
        }

        #[cfg(feature = "web-core")]
        let web_injector = if config.web.is_some() {
            let injector = WebBundleInjector::new();
            builder = builder.cla(
                "web-sender",
                Arc::new(injector.clone()),
                None,
                ClaInit {
                    address_type: None,
                    lane_count: NonZeroU32::new(1),
                    max_bundle_size: None,
                },
            );
            Some(injector)
        } else {
            None
        };

        #[cfg(feature = "web-core")]
        let web_modules = {
            #[allow(unused_mut)]
            let mut modules: Vec<Arc<dyn WebModule>> = Vec::new();
            #[cfg(feature = "mod-chat")]
            modules.push(Arc::new(dtnpm_mod_chat::ChatModule::new()));
            #[cfg(feature = "mod-files")]
            modules.push(Arc::new(dtnpm_mod_files::FilesModule::new()));
            #[cfg(feature = "mod-senml")]
            modules.push(Arc::new(dtnpm_mod_senml::SenmlModule::with_config_path(
                config.config_path.clone(),
            )));
            #[cfg(feature = "mod-rego-routing")]
            {
                modules.push(rego_module);
            }
            for module in &modules {
                for pack in module.filter_packs() {
                    builder = builder.add_filters(pack);
                }
            }
            modules
        };

        let bpa = Arc::new(builder.build().await.map_err(anyhow::Error::from_boxed)?);

        let grpc_config = if config.grpc.enabled || config.grpc.auth.socket_path.is_some() {
            Some(hardy_proto::server::Config {
                address: config.grpc.address,
                services: vec![
                    "cla".into(),
                    "service".into(),
                    "application".into(),
                    "routing".into(),
                ],
            })
        } else {
            None
        };

        let service_tracker = ServiceTracker::new(bpa.clone());

        // Keystore setup with 0600 file permissions
        let keystore = if config.storage.in_memory {
            Arc::new(DirectionalKeyStore::empty())
        } else {
            let data_dir = config.storage.effective_data_dir();
            let keystore_path = data_dir.join("keystore.toml");
            Arc::new(DirectionalKeyStore::load_or_init(&keystore_path)?)
        };

        Ok(Self {
            config,
            bpa,
            service_tracker,
            grpc_config,
            tcp_peers,
            coordinators,
            #[cfg(feature = "web-core")]
            web_injector,
            #[cfg(feature = "web-core")]
            web_modules,
            cla_manager,
            internal_storage,
            route_manager,
            keystore,
            _ephemeral_dir: ephemeral_dir,
            tasks,
        })
    }

    /// Returns the list of compiled-in [`WebModule`] plugins enabled by Cargo features.
    #[cfg(feature = "web-core")]
    #[must_use]
    #[allow(clippy::vec_init_then_push, unused_mut)]
    pub fn web_modules(&self) -> Vec<Arc<dyn WebModule>> {
        self.web_modules.clone()
    }

    /// Builds a [`ModuleContext`] and the active [`WebModule`] list for web serving or tests.
    #[cfg(feature = "web-core")]
    pub fn build_web_context(&self) -> anyhow::Result<(ModuleContext, Vec<Arc<dyn WebModule>>)> {
        let injector = self.web_injector.clone().ok_or_else(|| {
            anyhow::anyhow!("Web injector not initialized despite web config being enabled")
        })?;
        let modules = self.web_modules();
        let bpa_client: Arc<dyn BpaClient> = Arc::new(LocalBpaClient::new(self.bpa.clone()));
        let ctx = ModuleContext::new(
            Arc::new(self.config.clone()),
            bpa_client,
            injector,
            self.service_tracker.clone(),
            self.internal_storage.clone(),
            self.cla_manager.clone(),
            self.route_manager.clone(),
            LogBuffer::global(),
            self.keystore.clone(),
        )
        .with_capabilities(&modules);
        Ok((ctx, modules))
    }

    /// Runs migrations, registers enabled [`WebModule`] services on the BPA,
    /// and constructs an Axum [`axum::Router`] for tests or standalone embedding.
    #[cfg(feature = "web-core")]
    pub async fn create_web_router(&self) -> anyhow::Result<axum::Router> {
        let (ctx, modules) = self.build_web_context()?;
        dtnpm_web_core::register_modules(&ctx, &modules).await?;
        Ok(dtnpm_web_core::create_router(ctx, modules))
    }

    /// Runs the BPA server to completion.
    pub async fn run(mut self) -> anyhow::Result<()> {
        info!("Starting DTN Postmaster (dtnpm) BPA server");
        self.bpa.start(true).await;

        // Register permanent built-in Echo service
        let echo_service = Arc::new(hardy_echo_service::EchoService::new());
        let echo_service_id = hardy_bpv7::eid::Service::Dtn("echo".into());
        match self
            .service_tracker
            .register_service_with_type(echo_service_id, echo_service, "Built-in Echo")
            .await
        {
            Ok(eid) => info!("Registered permanent built-in Echo service at EID: {}", eid),
            Err(e) => warn!("Failed to register built-in Echo service: {}", e),
        }

        // Start storage quota & expiration reaper
        let _reaper = dtnpm_core::internal_storage::spawn_expiration_reaper(
            self.internal_storage.clone(),
            self.service_tracker.clone(),
            self.tasks.cancel_token().clone(),
        );
        info!(
            "Application storage quota active: max {} bytes (visual lifetime expiration enabled)",
            self.internal_storage.max_storage_bytes()
        );

        let coordinators = std::mem::take(&mut self.coordinators);
        for coordinator in coordinators {
            let cancel = self.tasks.cancel_token().clone();
            coordinator.spawn(cancel);
        }

        for (cla, addrs) in &self.tcp_peers {
            for peer_addr in addrs {
                let peer_addr = *peer_addr;
                let cla = cla.clone();
                let cla_manager = self.cla_manager.clone();
                let cancel = self.tasks.cancel_token().clone();
                hardy_async::spawn!(self.tasks, "tcpcl_peer_connector", async move {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let mut retry_delay = std::time::Duration::from_secs(1);
                    const MAX_STARTUP_RETRIES: usize = 5;
                    let mut attempts = 0;

                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            res = cla.connect(&peer_addr) => {
                                match res {
                                    Ok(()) => {
                                        info!("Successfully connected to TCPCL peer {}", peer_addr);
                                        cla_manager
                                            .set_peer_status(
                                                peer_addr,
                                                dtnpm_core::cla_manager::PeerConnectionStatus::Connected,
                                            )
                                            .await;
                                        break;
                                    }
                                    Err(e) => {
                                        attempts += 1;
                                        cla_manager
                                            .set_peer_status(
                                                peer_addr,
                                                dtnpm_core::cla_manager::PeerConnectionStatus::Disconnected,
                                            )
                                            .await;
                                        if attempts >= MAX_STARTUP_RETRIES {
                                            tracing::info!(
                                                "TCPCL startup dialing to {} reached max attempts ({}); deferring to coordinator schedule.",
                                                peer_addr,
                                                attempts
                                            );
                                            break;
                                        }
                                        tracing::debug!(
                                            "TCPCL connect to {} failed ({}), retrying in {:?} (attempt {}/{})...",
                                            peer_addr,
                                            e,
                                            retry_delay,
                                            attempts,
                                            MAX_STARTUP_RETRIES
                                        );
                                    }
                                }
                            }
                        }
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = tokio::time::sleep(retry_delay) => {
                                retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(15));
                            }
                        }
                    }
                });
            }
        }

        if let Some(ref grpc_config) = self.grpc_config {
            let auth_cfg = &self.config.grpc.auth;
            if auth_cfg.enabled || auth_cfg.socket_path.is_some() {
                let use_ephemeral_internal = auth_cfg.enabled || !self.config.grpc.enabled;
                let internal_addr = if use_ephemeral_internal {
                    SocketAddr::from(([127, 0, 0, 1], 0))
                } else {
                    grpc_config.address
                };

                // Guard internal ephemeral loopback registrations behind GrpcRegistrationGate
                // so unprivileged local processes cannot bypass GrpcSecurityGateway.
                let registration_gate = if use_ephemeral_internal {
                    Some(Arc::new(
                        dtnpm_core::iam::grpc_guard::GrpcRegistrationGate::new(),
                    ))
                } else {
                    None
                };
                let grpc_target: Arc<dyn hardy_bpa::bpa::BpaRegistration> = match registration_gate
                {
                    Some(ref gate) => {
                        Arc::new(dtnpm_core::iam::grpc_guard::GuardedBpaRegistration::new(
                            Arc::new(self.service_tracker.clone()),
                            gate.clone(),
                        ))
                    }
                    None => Arc::new(self.service_tracker.clone()),
                };

                let internal_cfg = hardy_proto::server::Config {
                    address: internal_addr,
                    services: grpc_config.services.clone(),
                };
                let server = GrpcServer::new(&internal_cfg, grpc_target)
                    .map_err(|e| anyhow::anyhow!("Failed to create gRPC server: {e}"))?;
                let bound_addr = server.local_addr();
                let upstream_for_gateway = if bound_addr.ip().is_unspecified() {
                    SocketAddr::from(([127, 0, 0, 1], bound_addr.port()))
                } else {
                    bound_addr
                };
                let cancel = self.tasks.cancel_token().clone();
                hardy_async::spawn!(self.tasks, "grpc_server", async move {
                    if let Err(e) = server.serve(cancel).await {
                        tracing::error!("gRPC server failed: {e}");
                    }
                });

                for _ in 0..50 {
                    if tokio::net::TcpStream::connect(upstream_for_gateway)
                        .await
                        .is_ok()
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }

                let policy_engine = Arc::new(dtnpm_core::iam::PolicyEngine::load(
                    self.config.auth.policy_path.as_deref(),
                )?);
                let active_sock =
                    dtnpm_core::iam::grpc_guard::GrpcSecurityGateway::start_with_gate(
                        grpc_config.address,
                        upstream_for_gateway,
                        auth_cfg.clone(),
                        self.config.callsign.clone(),
                        policy_engine,
                        Some(self.internal_storage.clone()),
                        registration_gate,
                        self.tasks.cancel_token().clone(),
                    )
                    .await?;
                if auth_cfg.enabled {
                    info!(
                        "gRPC authenticated TCP gateway listening on {}",
                        grpc_config.address
                    );
                } else if self.config.grpc.enabled {
                    info!(
                        "gRPC application service listening on {}",
                        grpc_config.address
                    );
                }
                if let Some(sock) = active_sock {
                    info!("gRPC Unix Domain Socket listening on {}", sock.display());
                }
            } else {
                let server = GrpcServer::new(grpc_config, Arc::new(self.service_tracker.clone()))
                    .map_err(|e| anyhow::anyhow!("Failed to create gRPC server: {e}"))?;
                let cancel = self.tasks.cancel_token().clone();
                hardy_async::spawn!(self.tasks, "grpc_server", async move {
                    if let Err(e) = server.serve(cancel).await {
                        tracing::error!("gRPC server failed: {e}");
                    }
                });
                info!(
                    "gRPC application service listening on {}",
                    grpc_config.address
                );
            }
        }

        #[cfg(feature = "web-core")]
        if let Some(ref web_config) = self.config.web {
            let (ctx, modules) = self.build_web_context()?;
            dtnpm_web_core::register_modules(&ctx, &modules).await?;
            let cancel = self.tasks.cancel_token().clone();
            let addr = web_config.address;
            let web_handle = dtnpm_web_core::start_web_server(ctx, modules, addr).await?;
            let abort_handle = web_handle.abort_handle();
            hardy_async::spawn!(self.tasks, "web_server", async move {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        abort_handle.abort();
                    }
                    _ = web_handle => {}
                }
            });
        }

        info!("DTN Postmaster (dtnpm) started successfully. Press Ctrl+C to terminate.");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Shutdown signal received");
            }
            _ = self.tasks.cancel_token().cancelled() => {}
        }

        self.shutdown().await;
        info!("DTN Postmaster (dtnpm) stopped.");
        Ok(())
    }

    /// Gracefully shuts down background tasks and the BPA engine.
    pub async fn shutdown(&self) {
        self.tasks.shutdown().await;
        self.bpa.shutdown().await;
    }

    /// Accessor for the dynamic route manager.
    #[must_use]
    pub fn route_manager(&self) -> Arc<RouteManager> {
        self.route_manager.clone()
    }

    /// Accessor for the active configuration.
    #[must_use]
    pub fn config(&self) -> &HamConfig {
        &self.config
    }

    /// Accessor for the underlying `Bpa` instance.
    #[must_use]
    pub fn bpa(&self) -> Arc<Bpa> {
        self.bpa.clone()
    }

    /// Accessor for the web bundle injector if configured.
    #[cfg(feature = "web-core")]
    #[must_use]
    pub fn web_injector(&self) -> Option<WebBundleInjector> {
        self.web_injector.clone()
    }

    /// Accessor for the active service tracker.
    #[must_use]
    pub fn service_tracker(&self) -> ServiceTracker {
        self.service_tracker.clone()
    }

    /// Accessor for the CLA manager.
    #[must_use]
    pub fn cla_manager(&self) -> ClaManager {
        self.cla_manager.clone()
    }

    /// Accessor for the station internal storage.
    #[must_use]
    pub fn internal_storage(&self) -> InternalStorage {
        self.internal_storage.clone()
    }

    /// Accessor for the station directional keystore.
    #[must_use]
    pub fn keystore(&self) -> Arc<DirectionalKeyStore> {
        self.keystore.clone()
    }
}
