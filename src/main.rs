//! CLI binary entry point for `dtnpm`.

use clap::{Parser, Subcommand};
use dtnpm::{HamConfig, InternalStorage, LogBuffer, LogBufferLayer};
use std::io::Write;
use std::path::{Path, PathBuf};
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

    /// Bearer token for authenticating to a remote gRPC BPA node
    #[arg(long, env = "DTN_HDY_TOKEN")]
    token: Option<String>,

    /// Path to a file containing the Bearer token for a remote gRPC BPA node (0600 recommended)
    #[arg(long, value_name = "FILE")]
    token_file: Option<PathBuf>,

    /// Unix Domain Socket path for connecting to a protected local gRPC BPA node
    #[arg(long, env = "DTN_HDY_SOCKET")]
    socket: Option<String>,

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
    /// Manage local WebUI operator/administrator accounts (Argon2id)
    User {
        #[command(subcommand)]
        action: UserCommands,
    },
    /// Manage gRPC / WebUI Bearer API tokens
    Token {
        #[command(subcommand)]
        action: TokenCommands,
    },
}

#[derive(Subcommand, Debug)]
enum UserCommands {
    /// Create or update an operator/administrator account
    Add {
        /// Username (typically callsign, e.g. f4abc)
        username: String,
        /// Assigned role (Admin or Operator)
        #[arg(long, default_value = "Operator")]
        role: String,
        /// Deprecated: plaintext password on CLI (visible via ps); prefer --password-stdin
        #[arg(long)]
        password: Option<String>,
        /// Read password cleanly from standard input
        #[arg(long = "password-stdin")]
        password_stdin: bool,
        /// Optional storage directory override
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Change the password of an existing local user
    Passwd {
        /// Username
        username: String,
        /// Deprecated: plaintext password on CLI (visible via ps); prefer --password-stdin
        #[arg(long)]
        password: Option<String>,
        /// Read new password cleanly from standard input
        #[arg(long = "password-stdin")]
        password_stdin: bool,
        /// Optional storage directory override
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// List configured local IAM accounts
    List {
        /// Optional storage directory override
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum TokenCommands {
    /// Issue a new persistent Bearer API token (prints raw secret once)
    Add {
        /// Human-readable description (e.g. ltp-kiss-cla)
        name: String,
        /// Assigned role (Admin or Operator)
        #[arg(long, default_value = "Admin")]
        role: String,
        /// Optional storage directory override
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// List active API tokens (metadata only; secrets are hashed at rest)
    List {
        /// Optional storage directory override
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Revoke an API token by its ID or name
    Revoke {
        /// Token ID or description/name
        id_or_name: String,
        /// Optional storage directory override
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

const EXAMPLE_CONFIG: &str = r#"# dtnpm: DTN Postmaster Station Configuration
# EID format can be DTN scheme (e.g. dtn://CALLSIGN/) or IPN scheme (e.g. ipn:NODE.0)
node-id: "dtn://n0call/"

# Storage settings (bundle storage directory & embedded SQLite database)
storage:
  data-dir: "~/.local/share/dtnpm"
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
    aliases: ["44NET"]   # Automatic 'TCPCL' tag is also added by default
    peers:
      - "44.27.131.233:4556"

  # 2. HQFBP over KISS (TNC via TCP or Serial)
  - name: kiss-tnc-vhf
    type: hqfbp
    callsign: "N0CALL-1"
    aliases: ["VHF", "2M"]
    transport:
      type: tcp # For Direwolf / soundmodem KISS server
      address: "127.0.0.1:8001"
      port: 0

  # 3. Licklider Transmission Protocol (LTP RFC 5326 over UDP or KISS)
  - name: ltp-udp-cislunar
    type: ltp
    engine-id: 1001
    aliases: ["QO100"]
    transport:
      type: udp
      bind-addr: "0.0.0.0:1113"

  # 4. File-based CLA (Sneakernet / USB drive / external software bridge; auto tag 'FILE')
  #    Dirs are created 0700; under systemd (ProtectHome=true), use /var/lib/dtnpm/spool/...
  # - name: file-usb
  #   type: file-cla
  #   aliases: ["USB"]
  #   max-bundle-size: "64MB"
  #   outbox: "~/dtn-spool/usb/outbox"
  #   peers:
  #     "dtn://f4usb/": "~/dtn-spool/usb/inbox"

# Unified OPA/Rego Routing & Built-in ANSRVR Group Service (ADR-0001)
rego-routing:
  record-treb: false
  ansrvr:
    enabled: true
    service-name: "ansrvr"
    default-ttl-secs: 86400
    cq-rate-limit-secs: 15

# Static routing rules (FALLBACK layer for plain bundles after RegoRib::decide)
# Direct peers, APRS strict/CLA/group aliases, and Spray-and-Wait are handled
# automatically by the embedded routing.rego policy without static wildcards.
routes: []

# RFC 9171 ingress validation policy toggles
rfc9171-validity:
  primary-block-integrity: false # Set false for interoperability with dtn-android-messenger / dtn7-rs
  bundle-age-required: false
"#;

#[cfg(feature = "web-core")]
#[allow(clippy::vec_init_then_push)]
async fn run_remote_webui(
    config: HamConfig,
    grpc_url: String,
    token: Option<String>,
    socket: Option<String>,
) -> anyhow::Result<()> {
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
        .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

    let internal_storage = if config.storage.in_memory {
        InternalStorage::new_in_memory_with_quota(config.application.quota.max_storage)?
    } else {
        let data_dir = config.storage.effective_data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        InternalStorage::new_disk_with_quota(&data_dir, config.application.quota.max_storage)?
    };

    let grpc_bpa = Arc::new(GrpcBpaClient::with_auth(
        grpc_url,
        token.as_deref(),
        socket.as_deref(),
    ));
    let bpa_reg: Arc<dyn BpaRegistration> = grpc_bpa.clone().as_registration();
    let bpa: Arc<dyn BpaClient> = grpc_bpa;
    let service_tracker = ServiceTracker::new(bpa_reg.clone());
    let injector = WebBundleInjector::new();
    bpa_reg
        .register_cla(
            "web-sender".to_string(),
            Arc::new(injector.clone()),
            None,
            ClaInit::default(),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to register web-sender CLA: {e}");
            anyhow::anyhow!("Failed to register web-sender CLA: {e}")
        })?;

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
    modules.push(Arc::new(dtnpm_mod_rego_routing::RegoRoutingModule::new()?));

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

    if let Some(Commands::User { action }) = cli.command {
        return handle_user_command(cli.config.as_deref(), action);
    }

    if let Some(Commands::Token { action }) = cli.command {
        return handle_token_command(cli.config.as_deref(), action);
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
    let resolved_token = dtnpm::resolve_bearer_token(cli.token, cli.token_file.as_deref())?;

    #[cfg(feature = "web-core")]
    if let Some(grpc_url) = cli.remote_bpa {
        return run_remote_webui(config, grpc_url, resolved_token, cli.socket).await;
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
        return run_remote_webui(config, grpc_url, resolved_token, cli.socket).await;
    }

    #[cfg(all(not(feature = "node"), not(feature = "web-core")))]
    {
        anyhow::bail!("Binary built without 'node' or 'web-core' features enabled");
    }
}

fn open_iam_storage(
    config_path: Option<&Path>,
    data_dir_override: Option<&Path>,
) -> anyhow::Result<InternalStorage> {
    let config = if let Some(found) = HamConfig::find_config(config_path) {
        HamConfig::load(Some(&found))?
    } else {
        HamConfig::default()
    };
    let dir = match data_dir_override {
        Some(p) => p.to_path_buf(),
        None => config.storage.effective_data_dir(),
    };
    std::fs::create_dir_all(&dir)?;
    InternalStorage::new_disk_with_quota(&dir, config.application.quota.max_storage)
}

fn read_password_arg(
    explicit: Option<String>,
    password_stdin: bool,
    prompt: &str,
) -> anyhow::Result<String> {
    if password_stdin {
        if explicit.is_some() {
            anyhow::bail!("Cannot specify both --password and --password-stdin");
        }
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_string();
        if trimmed.is_empty() {
            anyhow::bail!("Password read from stdin cannot be empty");
        }
        return Ok(trimmed);
    }
    if let Some(pw) = explicit {
        eprintln!(
            "WARNING: --password on the command line is deprecated and visible in process listings; prefer --password-stdin or interactive prompt."
        );
        if pw.is_empty() {
            anyhow::bail!("Password cannot be empty");
        }
        return Ok(pw);
    }
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_string();
    if trimmed.is_empty() {
        anyhow::bail!("Password cannot be empty (use --password-stdin or pipe via stdin)");
    }
    Ok(trimmed)
}

fn handle_user_command(config_path: Option<&Path>, action: UserCommands) -> anyhow::Result<()> {
    match action {
        UserCommands::Add {
            username,
            role,
            password,
            password_stdin,
            data_dir,
        } => {
            let storage = open_iam_storage(config_path, data_dir.as_deref())?;
            let pw = read_password_arg(
                password,
                password_stdin,
                &format!("Password for user '{username}': "),
            )?;
            let rec = storage.upsert_iam_user(&username, &pw, &role)?;
            println!("User '{}' saved with role '{}'", rec.username, rec.role);
        }
        UserCommands::Passwd {
            username,
            password,
            password_stdin,
            data_dir,
        } => {
            let storage = open_iam_storage(config_path, data_dir.as_deref())?;
            let pw = read_password_arg(
                password,
                password_stdin,
                &format!("New password for user '{username}': "),
            )?;
            if storage.update_iam_user_password(&username, &pw)? {
                println!("Password updated for user '{username}'");
            } else {
                anyhow::bail!("User '{username}' not found");
            }
        }
        UserCommands::List { data_dir } => {
            let storage = open_iam_storage(config_path, data_dir.as_deref())?;
            let users = storage.list_iam_users()?;
            if users.is_empty() {
                println!("No local IAM users registered.");
            } else {
                for u in users {
                    println!("{}\t{}", u.username, u.role);
                }
            }
        }
    }
    Ok(())
}

fn handle_token_command(config_path: Option<&Path>, action: TokenCommands) -> anyhow::Result<()> {
    match action {
        TokenCommands::Add {
            name,
            role,
            data_dir,
        } => {
            let storage = open_iam_storage(config_path, data_dir.as_deref())?;
            let (raw_secret, rec) = storage.create_iam_api_token(&name, &role)?;
            println!(
                "Created API token id='{}' name='{}' role='{}'",
                rec.id, rec.name, rec.role
            );
            println!("Token (save now — never shown again): {raw_secret}");
        }
        TokenCommands::List { data_dir } => {
            let storage = open_iam_storage(config_path, data_dir.as_deref())?;
            let tokens = storage.list_iam_api_tokens()?;
            if tokens.is_empty() {
                println!("No API tokens registered.");
            } else {
                for t in tokens {
                    println!("{}\t{}\t{}", t.id, t.name, t.role);
                }
            }
        }
        TokenCommands::Revoke {
            id_or_name,
            data_dir,
        } => {
            let storage = open_iam_storage(config_path, data_dir.as_deref())?;
            if storage.revoke_iam_api_token(&id_or_name)? {
                println!("Revoked token '{id_or_name}'");
            } else {
                anyhow::bail!("Token '{id_or_name}' not found");
            }
        }
    }
    Ok(())
}
