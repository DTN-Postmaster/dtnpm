//! `dtnpm`: DTN Postmaster modular assembler crate.
//!
//! Assembles `dtnpm-core`, optional convergence layers (`dtnpm-cla-*`),
//! optional `dtnpm-web-core`, and optional fullstack application modules (`dtnpm-mod-*`)
//! into cohesive deployment profiles (`desktop`, `amsat`, `node-headless`, `webui-remote`).

pub use dtnpm_core::{
    BpaClient, GrpcBpaClient, LocalBpaClient, MockBpa, NoopSenderCla, ServiceTracker, basket,
    bundle_inspector, cla_manager, cli_security, config, coordinator, internal_storage, log_buffer,
    normalize_eid, pacing, resolve_grpc_port, routing, security,
};

pub use config::HamConfig;
pub use internal_storage::InternalStorage;
pub use log_buffer::{LogBuffer, LogBufferLayer, LogEntry};
pub use routing::RouteManager;

#[cfg(feature = "web-core")]
pub use dtnpm_web_core as web;

#[cfg(feature = "mod-chat")]
pub use dtnpm_mod_chat::{self as mod_chat, ChatManager, ChatModule, ChatService};

#[cfg(feature = "mod-files")]
pub use dtnpm_mod_files::{self as mod_files, FileBundleRecord, FilesModule, FilesService};

#[cfg(feature = "mod-rego-routing")]
pub use dtnpm_mod_rego_routing::{
    self as mod_rego_routing, RegoRoutingAgent, RegoRoutingConfig, RegoRoutingEngine,
    RegoRoutingModule,
};
#[cfg(feature = "mod-senml")]
pub use dtnpm_mod_senml::{
    self as mod_senml, SenmlManager, SenmlModule, SenmlService, SenmlStorage,
};

#[cfg(feature = "node")]
pub mod server;
#[cfg(feature = "node")]
pub use server::HamServer;

pub mod wizard;
pub use wizard::{run_interactive_wizard, run_wizard};

/// Resolves an optional gRPC/WebUI bearer token from an explicit CLI/env value or `--token-file`.
pub fn resolve_bearer_token(
    token: Option<String>,
    token_file: Option<&std::path::Path>,
) -> anyhow::Result<Option<String>> {
    if let Some(t) = token {
        let trimmed = t.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed));
        }
    }
    if let Some(path) = token_file {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read token file '{}': {e}", path.display()))?;
        let trimmed = content.trim().to_string();
        if trimmed.is_empty() {
            anyhow::bail!("Token file '{}' is empty", path.display());
        }
        return Ok(Some(trimmed));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::resolve_bearer_token;

    #[test]
    fn test_resolve_bearer_token_prefers_inline_else_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let tok_path = dir.path().join("token");
        std::fs::write(&tok_path, "  secret-file-token\n").unwrap();

        let from_file = resolve_bearer_token(None, Some(&tok_path)).unwrap();
        assert_eq!(from_file.as_deref(), Some("secret-file-token"));

        let inline_wins =
            resolve_bearer_token(Some("inline-tok".to_string()), Some(&tok_path)).unwrap();
        assert_eq!(inline_wins.as_deref(), Some("inline-tok"));

        let empty_path = dir.path().join("empty");
        std::fs::write(&empty_path, "   \n").unwrap();
        assert!(resolve_bearer_token(None, Some(&empty_path)).is_err());
    }
}
