use clap::Parser;
use dtnpm_core::cli_security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use dtnpm_core::resolve_grpc_port;
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink};
use hardy_bpv7::eid::{Eid, Service};
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::OnceCell;

/// A Bundle Protocol 7 Incoming Trigger Utility for Delay Tolerant Networking interacting with Hardy
#[derive(Parser, Debug)]
#[command(author, version, about = "Incoming trigger utility for Hardy BPA", long_about = None)]
struct Args {
    /// Local gRPC port of Hardy BPA (default = 50051)
    #[arg(short, long)]
    port: Option<u16>,

    /// Use IPv6
    #[arg(short = '6', long)]
    ipv6: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Specify local endpoint, e.g. 'incoming', or a group endpoint '7'
    #[arg(short, long)]
    endpoint: String,

    /// Just print the message
    #[arg(long)]
    print: bool,

    /// Command to execute for incoming bundles, param1 = source, param2 = payload file.
    /// Also exports DTN_SOURCE, DTN_DESTINATION, DTN_PAYLOAD_FILE, DTN_RETURN_PATH,
    /// DTN_TREB_TRACE, DTN_APRS_PATH, DTN_SPRAY_QUOTA, DTN_GRPC_PORT, and DTN_HDY_SOCKET.
    #[arg(short, long, default_value = "echo")]
    command: String,

    /// Path to keystore configuration file (defaults to ~/.config/dtn/keystore.toml)
    #[arg(long = "keystore")]
    keystore: Option<std::path::PathBuf>,

    /// Inline verification key material (string or hex)
    #[arg(long = "verify-key")]
    verify_key: Option<String>,

    /// Path to single verification key file
    #[arg(long = "verify-key-file")]
    verify_key_file: Option<String>,

    /// Verification policy for received bundles (strict, warn, or ignore) (default = "warn")
    #[arg(long = "verify-policy", default_value = "warn")]
    verify_policy: VerifyPolicy,

    /// Optional Bearer authentication token for protected gRPC BPA endpoints
    #[arg(long, env = "DTN_HDY_TOKEN")]
    token: Option<String>,

    /// Path to a file containing the Bearer authentication token (0600 recommended)
    #[arg(long = "token-file", value_name = "FILE", env = "DTN_HDY_TOKEN_FILE")]
    token_file: Option<std::path::PathBuf>,

    /// Optional Unix Domain Socket path for connecting to a protected local BPA
    #[arg(long, env = "DTN_HDY_SOCKET")]
    socket: Option<String>,
}

struct TriggerApp {
    sink: OnceCell<Box<dyn ServiceSink>>,
    local_eid: OnceCell<Eid>,
    grpc_port: String,
    socket: Option<String>,
    token_file: Option<String>,
    verbose: bool,
    print: bool,
    command: String,
    keystore: KeyStore,
    policy: VerifyPolicy,
    shutting_down: std::sync::atomic::AtomicBool,
    exec_semaphore: Arc<tokio::sync::Semaphore>,
}

#[derive(Debug, Default)]
struct TriggerEnv {
    destination: String,
    return_path: Option<String>,
    treb_trace: Option<String>,
    aprs_path: Option<String>,
    spray_quota: Option<u32>,
    grpc_port: String,
    socket: Option<String>,
    token_file: Option<String>,
}

fn extract_trigger_env(
    bundle: &hardy_bpv7::bundle::Bundle,
    raw: &[u8],
    local_eid: &Eid,
    grpc_port: &str,
    socket: Option<&str>,
    token_file: Option<&str>,
) -> TriggerEnv {
    let ext = dtnpm_core::mesh_blocks::extract_routing_extensions(bundle, raw);
    let reply_plan =
        dtnpm_core::mesh_blocks::derive_reply_routing_from_summary(bundle, raw, &ext, local_eid);
    let return_path = reply_plan
        .return_trajectory
        .as_ref()
        .map(|traj| dtnpm_core::mesh_blocks::format_aprs_path(&traj.path_elements));
    let treb_trace = ext.treb.as_ref().and_then(|treb| {
        if treb.hops.is_empty() {
            None
        } else {
            Some(
                treb.hops
                    .iter()
                    .map(|h| h.node_eid.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            )
        }
    });
    let spray_quota = ext.spray_permit.or_else(|| {
        ext.mesh_routing
            .as_ref()
            .and_then(|m| m.replication_control.as_ref())
            .map(|r| r.quota)
    });

    TriggerEnv {
        destination: bundle.primary.destination.to_string(),
        return_path,
        treb_trace,
        aprs_path: ext.aprs_path_formatted,
        spray_quota,
        grpc_port: grpc_port.to_string(),
        socket: socket.map(ToString::to_string),
        token_file: token_file.map(ToString::to_string),
    }
}

fn write_temp_file(data: &[u8], verbose: bool) -> anyhow::Result<NamedTempFile> {
    let mut data_file = NamedTempFile::new()?;
    data_file.write_all(data)?;
    data_file.flush()?;
    if verbose {
        eprintln!("[*] data file: {}", data_file.path().display());
    }
    Ok(data_file)
}

async fn execute_cmd(
    command: &str,
    data_file: NamedTempFile,
    source: &str,
    env: &TriggerEnv,
    verbose: bool,
) -> anyhow::Result<()> {
    let fname_param = format!("{}", data_file.path().display());

    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c");
    let full_script = format!("{} \"$1\" \"$2\"", command);
    cmd.arg(full_script);
    cmd.arg("--"); // placeholder for $0
    cmd.arg(source);
    cmd.arg(&fname_param);
    cmd.env("DTN_SOURCE", source);
    cmd.env("DTN_DESTINATION", &env.destination);
    cmd.env("DTN_PAYLOAD_FILE", &fname_param);
    cmd.env("DTN_RETURN_PATH", env.return_path.as_deref().unwrap_or(""));
    cmd.env("DTN_TREB_TRACE", env.treb_trace.as_deref().unwrap_or(""));
    cmd.env("DTN_APRS_PATH", env.aprs_path.as_deref().unwrap_or(""));
    cmd.env(
        "DTN_SPRAY_QUOTA",
        env.spray_quota.map(|q| q.to_string()).unwrap_or_default(),
    );
    cmd.env("DTN_GRPC_PORT", &env.grpc_port);
    if let Some(ref sock) = env.socket {
        cmd.env("DTN_HDY_SOCKET", sock);
    }
    if let Some(ref tok_file) = env.token_file {
        cmd.env("DTN_HDY_TOKEN_FILE", tok_file);
    }
    cmd.kill_on_drop(true);

    if verbose {
        eprintln!(
            "[*] Executing: /bin/sh -c {:?} -- {} {}",
            command, source, fname_param
        );
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("Trigger command timed out after 60s"))??;

    if !output.status.success() || verbose {
        eprintln!("[*] status: {}", output.status);
        std::io::stdout().lock().write_all(&output.stdout)?;
        std::io::stderr().lock().write_all(&output.stderr)?;
    }
    Ok(())
}

#[async_trait]
impl BpaService for TriggerApp {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!(
                "Trigger service registered successfully with EID: {}",
                source
            );
        }
        let _ = self.local_eid.set(source.clone());
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        if self.verbose {
            eprintln!("Trigger service unregistered");
        }
        if !self
            .shutting_down
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!("Error: Trigger service unregistered (connection lost). Exiting.");
            std::process::exit(1);
        }
    }

    async fn on_deliver(
        &self,
        _bundle_id: &hardy_bpv7::bundle::Id,
        _expiry: time::OffsetDateTime,
        total_len: u64,
        stream: &mut dyn hardy_bpa::stream::Receiver<hardy_bpa::stream::Segment>,
    ) -> hardy_bpa::services::Result<()> {
        let data = hardy_bpa::stream::buffer_stream(stream, total_len).await?;
        let (source, payload, trigger_env) = match hardy_bpv7::parse::parse(data.clone()) {
            Ok(parsed) => {
                let local_eid = self
                    .local_eid
                    .get()
                    .cloned()
                    .unwrap_or_else(|| parsed.bundle.primary.destination.clone());
                let env = extract_trigger_env(
                    &parsed.bundle,
                    &parsed.data,
                    &local_eid,
                    &self.grpc_port,
                    self.socket.as_deref(),
                    self.token_file.as_deref(),
                );
                let payload_bytes = parsed
                    .bundle
                    .blocks
                    .get(&1)
                    .and_then(|b| b.payload(&parsed.data))
                    .map(|bytes| bytes.to_vec())
                    .unwrap_or_else(|| data.to_vec());
                (parsed.bundle.primary.id.source, payload_bytes, env)
            }
            Err(_) => (
                Eid::Null,
                data.to_vec(),
                TriggerEnv {
                    grpc_port: self.grpc_port.clone(),
                    socket: self.socket.clone(),
                    token_file: self.token_file.clone(),
                    ..Default::default()
                },
            ),
        };

        if self.policy != VerifyPolicy::Ignore {
            let res = verify_bundle(&data, &self.keystore);
            match res {
                VerifyResult::Valid => {
                    if self.verbose {
                        eprintln!("Signature verified successfully for source {}", source);
                    }
                }
                VerifyResult::Invalid(reason) => {
                    eprintln!(
                        "WARNING: Signature verification failed for {}: {}",
                        source, reason
                    );
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!("Bundle dropped due to strict verification policy.");
                        return Ok(());
                    }
                }
                VerifyResult::Unsigned => {
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "WARNING: Unsigned bundle received from {}. Dropped due to strict verification policy.",
                            source
                        );
                        return Ok(());
                    } else if self.verbose {
                        eprintln!("Received unsigned bundle from {}", source);
                    }
                }
            }
        }

        if self.verbose {
            eprintln!("[<] Received bundle from {}", source);
        }

        if self.print {
            let now = match time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
            {
                Ok(t) => t,
                Err(_) => "unknown_time".to_string(),
            };
            let text = String::from_utf8_lossy(&payload);
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "[{}] {} → {}", now, source, text);
        } else {
            let verbose = self.verbose;
            let command_str = self.command.clone();
            let source_str = source.to_string();

            // Write temporary file
            let data_file = match write_temp_file(&payload, verbose) {
                Ok(df) => df,
                Err(e) => {
                    eprintln!("[!] Error creating temporary file: {}", e);
                    return Ok(());
                }
            };

            // Run command asynchronously under concurrency bound & timeout
            let Ok(permit) = self.exec_semaphore.clone().try_acquire_owned() else {
                eprintln!(
                    "[!] Dropping trigger command execution for {}: concurrency limit reached",
                    source
                );
                return Ok(());
            };
            tokio::spawn(async move {
                let _permit = permit;
                if let Err(e) =
                    execute_cmd(&command_str, data_file, &source_str, &trigger_env, verbose).await
                {
                    eprintln!("[!] Error executing command: {}", e);
                }
            });
        }
        Ok(())
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &hardy_bpv7::bundle::Id,
        _from: &Eid,
        _kind: hardy_bpa::services::StatusNotify,
        _reason: hardy_bpv7::status_report::ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
        // Ignored for triggering
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let port = resolve_grpc_port(args.port);

    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let grpc_addr = format!("http://{}:{}", localhost, port);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let mut keystore = KeyStore::load_default_or(args.keystore.as_deref())?;

    if args.verify_key.is_some() || args.verify_key_file.is_some() {
        let key_mat = load_key(args.verify_key.as_deref(), args.verify_key_file.as_deref())?;
        keystore.add_key("*", &key_mat.raw);
    }

    let policy = args.verify_policy;

    let token_file_env = args
        .token_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let resolved_token = dtnpm::resolve_bearer_token(args.token, args.token_file.as_deref())?;
    let (remote_bpa, _grpc_guard) = dtnpm_core::create_remote_bpa(
        &grpc_addr,
        resolved_token.as_deref(),
        args.socket.as_deref(),
    )?;
    let app = Arc::new(TriggerApp {
        sink: OnceCell::new(),
        local_eid: OnceCell::new(),
        grpc_port: port.clone(),
        socket: args.socket.clone(),
        token_file: token_file_env,
        verbose: args.verbose,
        print: args.print,
        command: args.command,
        keystore,
        policy,
        shutting_down: std::sync::atomic::AtomicBool::new(false),
        exec_semaphore: Arc::new(tokio::sync::Semaphore::new(8)),
    });

    let service_id = if let Ok(num) = args.endpoint.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(args.endpoint.clone().into())
    };

    let registered_eid = remote_bpa
        .register_service(service_id, app.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Service registration failed: {e}"))?;

    eprintln!("Listening for trigger events on: {}", registered_eid);

    // Wait for Ctrl+C to exit
    tokio::signal::ctrl_c().await?;
    app.shutting_down
        .store(true, std::sync::atomic::Ordering::Relaxed);
    eprintln!("\nShutting down trigger application...");

    if let Some(sink) = app.sink.get() {
        sink.unregister().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_temp_file() -> anyhow::Result<()> {
        let data = b"hello world";
        let temp = write_temp_file(data, true)?;

        let path = temp.path().to_path_buf();
        assert!(path.exists());

        let read_data = std::fs::read(&path)?;
        assert_eq!(read_data, data);

        // Dropping NamedTempFile deletes the file
        std::mem::drop(temp);
        assert!(!path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_execute_cmd_success() -> anyhow::Result<()> {
        let temp = write_temp_file(b"test data", false)?;
        // We will run a command like "true" which exits with 0
        let status = execute_cmd("true", temp, "dtn://source", &TriggerEnv::default(), false).await;
        assert!(status.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_execute_cmd_with_quotes() -> anyhow::Result<()> {
        let temp = write_temp_file(b"test data", false)?;
        let status = execute_cmd(
            "echo 'hello world'",
            temp,
            "dtn://source",
            &TriggerEnv::default(),
            false,
        )
        .await;
        assert!(status.is_ok());
        Ok(())
    }

    #[test]
    fn test_extract_trigger_env_derives_return_path_from_treb() {
        use dtnpm_core::mesh_blocks::{DEFAULT_BLOCK_TYPE_TREB, TrebData, TrebHopEntry};
        use hardy_bpv7::block;
        use hardy_bpv7::builder::Builder;
        use hardy_bpv7::creation_timestamp::CreationTimestamp;

        let src: Eid = "dtn://stata/client".parse().expect("src");
        let dst: Eid = "dtn://statd/fortune".parse().expect("dst");
        let treb = TrebData {
            hops: vec![
                TrebHopEntry {
                    node_eid: "dtn://stata/".to_string(),
                    arrival_time_ms: Some(1),
                    departure_time_ms: Some(2),
                    cla_info: None,
                },
                TrebHopEntry {
                    node_eid: "dtn://statb/".to_string(),
                    arrival_time_ms: Some(3),
                    departure_time_ms: Some(4),
                    cla_info: None,
                },
                TrebHopEntry {
                    node_eid: "dtn://statc/".to_string(),
                    arrival_time_ms: Some(5),
                    departure_time_ms: Some(6),
                    cla_info: None,
                },
            ],
        };
        let mut b = Builder::new(src, dst.clone()).with_payload(b"en".as_slice().into());
        b = dtnpm_core::mesh_blocks::apply_routing_extensions(
            b,
            Some(10),
            Some("STATB,STATC"),
            Some(8),
        )
        .expect("ext");
        b = b
            .add_extension_block(block::Type::Unrecognised(DEFAULT_BLOCK_TYPE_TREB))
            .expect("treb")
            .build(treb.encode_cbor().expect("encode").into_vec().into());
        let (_bundle, raw) = b.build(CreationTimestamp::now()).expect("build");
        let parsed = hardy_bpv7::parse::parse(raw.to_vec().into()).expect("parse");

        let env = extract_trigger_env(&parsed.bundle, &parsed.data, &dst, "35054", None, None);
        assert_eq!(env.destination, "dtn://statd/fortune");
        assert_eq!(
            env.return_path.as_deref(),
            Some("dtn://statc/,dtn://statb/")
        );
        assert_eq!(
            env.treb_trace.as_deref(),
            Some("dtn://stata/,dtn://statb/,dtn://statc/")
        );
        assert_eq!(env.aprs_path.as_deref(), Some("STATB,STATC"));
        assert_eq!(env.spray_quota, Some(8));
        assert_eq!(env.grpc_port, "35054");
    }

    #[tokio::test]
    async fn test_execute_cmd_exports_return_path_and_port_env() -> anyhow::Result<()> {
        let temp = write_temp_file(b"hello", false)?;
        let out_file = NamedTempFile::new()?;
        let out_path = out_file.path().display().to_string();
        let env = TriggerEnv {
            destination: "dtn://statd/fortune".to_string(),
            return_path: Some("dtn://statc/,dtn://statb/".to_string()),
            treb_trace: Some("dtn://stata/,dtn://statb/,dtn://statc/".to_string()),
            aprs_path: Some("STATB,STATC".to_string()),
            spray_quota: Some(8),
            grpc_port: "35054".to_string(),
            socket: Some("/tmp/test.sock".to_string()),
            token_file: Some("/etc/dtnpm/token.secret".to_string()),
        };
        let script = format!(
            "printf '%s|%s|%s|%s|%s|%s' \"$DTN_SOURCE\" \"$DTN_RETURN_PATH\" \"$DTN_GRPC_PORT\" \"$DTN_SPRAY_QUOTA\" \"$DTN_HDY_SOCKET\" \"$DTN_HDY_TOKEN_FILE\" > '{}'; #",
            out_path
        );
        execute_cmd(&script, temp, "dtn://stata/client", &env, false).await?;
        let recorded = std::fs::read_to_string(&out_path)?;
        assert_eq!(
            recorded,
            "dtn://stata/client|dtn://statc/,dtn://statb/|35054|8|/tmp/test.sock|/etc/dtnpm/token.secret"
        );
        Ok(())
    }
}
