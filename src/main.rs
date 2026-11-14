//! CLI binary entry point for `dtnpm`.

use clap::{Parser, Subcommand};
use dtnpm::{HamConfig, LogBuffer, LogBufferLayer};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "dtnpm",
    author,
    version,
    about = "DTN Postmaster: Delay Tolerant Networking BPv7 agent for amateur radio and emergency communications"
)]
struct Cli {
    /// Path to YAML/TOML configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Override local node EID (e.g. 'dtn://n0call/' or 'ipn:1.0')
    #[arg(short, long)]
    node_id: Option<String>,

    /// Run with in-memory volatile storage (overrides config)
    #[arg(long)]
    in_memory: bool,

    /// Connect WebUI to a remote headless BPA node over gRPC (e.g. 'http://127.0.0.1:50051')
    #[cfg(feature = "web-core")]
    #[arg(long, value_name = "GRPC_URL")]
    remote_bpa: Option<String>,

    /// Increase verbosity level (-v: debug, -vv: trace)
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress verbose logging (warnings and errors only)
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print an annotated example configuration file
    GenerateConfig,
    /// Run the interactive configuration wizard
    Init {
        /// Optional path where to save the configuration file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

const EXAMPLE_CONFIG: &str = r#"# dtnpm: DTN Postmaster Station Configuration
# EID format can be DTN scheme (e.g. dtn://CALLSIGN/) or IPN scheme (e.g. ipn:NODE.0)
node-id: "dtn://n0call/"

# Storage settings (bundle storage directory & embedded SQLite database)
storage:
  data-dir: "~/.local/share/dtn-hdy-ham"
  in-memory: false

# Local gRPC endpoint for CLI tools (dtnsend, dtnprint, dtnping, etc.)
grpc:
  enabled: true
  address: "[::]:50051"

# Convergence Layer Adapters
clas:
  # 1. TCPCLv4 over Hamnet / 44Net / Internet (RFC 9174)
  - name: tcpcl-44net-gateway
    type: tcpclv4
    address: "[::]:4556" # Local listening port
    require-tls: false   # Disabled for amateur radio compliance (no encryption)
    peers:
      - "44.27.131.233:4556"

  # 2. HQFBP over KISS (TNC via TCP or Serial)
  - name: kiss-tnc-vhf
    type: hqfbp
    callsign: "N0CALL-1"
    transport:
      type: tcp # For Direwolf / soundmodem KISS server
      address: "127.0.0.1:8001"
      port: 0

  # 3. Licklider Transmission Protocol (LTP RFC 5326 over UDP or KISS)
  - name: ltp-udp-cislunar
    type: ltp
    engine-id: 1001
    transport:
      type: udp
      bind-addr: "0.0.0.0:1113"

# Static routing rules
routes:
  - pattern: "dtn://*/*"
    cla: "tcpcl-44net-gateway"

# RFC 9171 ingress validation policy toggles
rfc9171-validity:
  primary-block-integrity: false # Set false for interoperability with dtn-android-messenger / dtn7-rs
  bundle-age-required: false
"#;

#[cfg(feature = "web-core")]
#[allow(clippy::vec_init_then_push)]
async fn run_remote_webui(config: HamConfig, grpc_url: String) -> anyhow::Result<()> {
    use dtnpm_core::{
        BpaClient, GrpcBpaClient, InternalStorage, ServiceTracker, cla_manager::ClaManager,
        routing::RouteManager, security::DirectionalKeyStore,
    };
    use dtnpm_web_core::{
        ModuleContext, WebBundleInjector, WebModule, register_modules, start_web_server,
    };
    use hardy_bpa::{bpa::BpaRegistration, cla::ClaInit};
    use std::sync::Arc;

    let listen_addr = config
        .web
        .as_ref()
        .map(|w| w.address)
        .unwrap_or_else(|| "127.0.0.1:8080".parse().expect("valid default socket addr"));

    let internal_storage = if config.storage.in_memory {
        InternalStorage::new_in_memory_with_quota(config.application.quota.max_storage)?
    } else {
        let data_dir = config.storage.effective_data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        InternalStorage::new_disk_with_quota(&data_dir, config.application.quota.max_storage)?
    };

    let grpc_bpa = Arc::new(GrpcBpaClient::new(grpc_url));
    let bpa_reg: Arc<dyn BpaRegistration> = grpc_bpa.clone().as_registration();
    let bpa: Arc<dyn BpaClient> = grpc_bpa;
    let service_tracker = ServiceTracker::new(bpa_reg.clone());
    let injector = WebBundleInjector::new();
    let _ = bpa_reg
        .register_cla(
            "web-sender".to_string(),
            Arc::new(injector.clone()),
            None,
            ClaInit::default(),
        )
        .await;

    let mut modules: Vec<Arc<dyn WebModule>> = Vec::new();
    #[cfg(feature = "mod-chat")]
    modules.push(Arc::new(dtnpm_mod_chat::ChatModule::new()));
    #[cfg(feature = "mod-files")]
    modules.push(Arc::new(dtnpm_mod_files::FilesModule::new()));
    #[cfg(feature = "mod-senml")]
    modules.push(Arc::new(dtnpm_mod_senml::SenmlModule::with_config_path(
        config.config_path.clone(),
    )));

    let keystore = Arc::new(DirectionalKeyStore::empty());
    let ctx = ModuleContext::new(
        Arc::new(config.clone()),
        bpa,
        injector,
        service_tracker,
        internal_storage,
        ClaManager::new(),
        Arc::new(RouteManager::new(config.routes, config.config_path)),
        LogBuffer::global(),
        keystore,
    )
    .with_capabilities(&modules);

    register_modules(&ctx, &modules).await?;
    let _web_handle = start_web_server(ctx, modules, listen_addr).await?;

    info!("Remote WebUI listening on http://{listen_addr}");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let env_filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else if cli.quiet {
        EnvFilter::new("warn")
    } else {
        match cli.verbose {
            0 => EnvFilter::new("info"),
            1 => EnvFilter::new(
                "info,dtnpm=debug,hardy=debug,hardy_bpa=debug,hardy_tcpclv4=debug,hardy_ltp=debug",
            ),
            _ => EnvFilter::new(
                "debug,dtnpm=trace,hardy=trace,hardy_bpa=trace,hardy_tcpclv4=trace,hardy_ltp=trace",
            ),
        }
    };

    let log_buffer = LogBuffer::init_global(500);
    let log_layer = LogBufferLayer::new(log_buffer);

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(log_layer)
        .init();

    if let Some(Commands::GenerateConfig) = cli.command {
        print!("{EXAMPLE_CONFIG}");
        return Ok(());
    }

    if let Some(Commands::Init { output }) = cli.command {
        dtnpm::run_interactive_wizard(output.or(cli.config))?;
        return Ok(());
    }

    let mut config = if let Some(config_file) = HamConfig::find_config(cli.config.as_deref()) {
        HamConfig::load(Some(&config_file))?
    } else {
        dtnpm::run_interactive_wizard(cli.config.clone())?
    };

    if let Some(nid) = cli.node_id {
        config.node_id = nid;
    }
    if cli.in_memory {
        config.storage.in_memory = true;
    }

    #[cfg(feature = "web-core")]
    if let Some(grpc_url) = cli.remote_bpa {
        return run_remote_webui(config, grpc_url).await;
    }

    #[cfg(feature = "node")]
    {
        info!(
            "Starting DTN Postmaster BPA with node-id: {}",
            config.node_id
        );
        let server = dtnpm::HamServer::new(config).await?;
        server.run().await?;
        return Ok(());
    }

    #[cfg(all(not(feature = "node"), feature = "web-core"))]
    {
        let grpc_addr = config.grpc.address.to_string();
        let grpc_url = format!("http://{grpc_addr}");
        return run_remote_webui(config, grpc_url).await;
    }

    #[cfg(all(not(feature = "node"), not(feature = "web-core")))]
    {
        anyhow::bail!("Binary built without 'node' or 'web-core' features enabled");
    }
}
