//! `dtnpm`: DTN Postmaster modular assembler crate.
//!
//! Assembles `dtnpm-core`, optional convergence layers (`dtnpm-cla-*`),
//! optional `dtnpm-web-core`, and optional fullstack application modules (`dtnpm-mod-*`)
//! into cohesive deployment profiles (`emcom`, `amsat`, `node-headless`, `webui-remote`).

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
