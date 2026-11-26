//! File-based Convergence Layer Adapter (`file-cla`) configuration, path
//! validation, spool directory provisioning, and BPA registration.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use hardy_bpa::cla::{Cla, ClaInit};
use hardy_bpv7::eid::NodeId;
use serde::Deserialize;
use tracing::{info, warn};

use dtnpm_core::config::{HamConfig, cla::ClaConfig, cla::ClaType};

/// Default maximum bundle size (64 MiB) enforced on `file-cla` outbox ingestion
/// to prevent unbounded memory allocation on oversized files.
pub const DEFAULT_FILE_CLA_MAX_BUNDLE_SIZE: u64 = 64 * 1024 * 1024;

/// Typed configuration for a `file-cla` / `file` CLA instance.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct FileClaHamConfig {
    /// Directory watched for incoming raw BPv7 bundle files (deleted on dispatch).
    pub outbox: Option<PathBuf>,
    /// Map of peer Node IDs to their outgoing inbox directories.
    pub peers: HashMap<NodeId, PathBuf>,
    /// Optional maximum bundle size accepted from `outbox` (bytes or e.g. `"64MB"`).
    #[serde(
        alias = "max_bundle_size",
        deserialize_with = "deserialize_opt_byte_size"
    )]
    pub max_bundle_size: Option<u64>,
    /// Explicit opt-in allowing existing spool directories with group/world permissions (`mode & 0o077 != 0`).
    #[serde(default, alias = "allow_permissive_spool")]
    pub allow_permissive_spool: bool,
}

/// Parsed and validated `file-cla` configuration ready for provisioning or dry-run checks.
#[derive(Debug, Clone)]
pub struct ParsedFileClaConfig {
    /// Underlying `hardy-file-cla` configuration with tilde-expanded paths.
    pub hardy_cfg: hardy_file_cla::Config,
    /// Negotiated maximum bundle size cap enforced on `outbox` ingestion.
    pub max_bundle_size: Option<NonZeroU64>,
    /// Whether permissive (`mode & 0o077 != 0`) existing spool directories are allowed with a warning.
    pub allow_permissive_spool: bool,
}

fn deserialize_opt_byte_size<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptByteSizeVisitor;

    impl<'de> serde::de::Visitor<'de> for OptByteSizeVisitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str(
                "null, a non-negative integer, or a byte size string (e.g. '64MB', '1024')",
            )
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(value))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if value < 0 {
                return Err(E::custom("byte size cannot be negative"));
            }
            Ok(Some(value as u64))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            dtnpm_core::config::parse_byte_size(value)
                .map(Some)
                .map_err(E::custom)
        }
    }

    deserializer.deserialize_any(OptByteSizeVisitor)
}

/// Tracks canonicalized `file-cla` spool directories across all configured instances
/// to prevent duplicate watchers, forwarding loops, and quarantine of station files.
pub(crate) struct FileClaPathRegistry {
    pub(crate) outboxes: HashSet<PathBuf>,
    pub(crate) inboxes: HashSet<PathBuf>,
    pub(crate) refused_dirs: HashSet<PathBuf>,
    pub(crate) protected_dirs: HashSet<PathBuf>,
}

impl FileClaPathRegistry {
    /// Builds a registry seeded with protected station storage, config, home, and root directories.
    pub(crate) fn for_config(config: &HamConfig) -> Self {
        let mut protected_dirs = HashSet::new();
        if let Ok(canon) = std::fs::canonicalize(config.storage.effective_data_dir()) {
            protected_dirs.insert(canon);
        }
        if let Ok(canon) = std::fs::canonicalize(config.storage.effective_payload_dir()) {
            protected_dirs.insert(canon);
        }
        if let Some(cfg_path) = config.config_path.as_ref()
            && let Ok(canon_file) = std::fs::canonicalize(cfg_path)
            && let Some(cfg_dir) = canon_file.parent()
        {
            protected_dirs.insert(cfg_dir.to_path_buf());
        }
        if let Some(base) = directories::BaseDirs::new()
            && let Ok(canon_home) = std::fs::canonicalize(base.home_dir())
        {
            protected_dirs.insert(canon_home);
        }
        if let Ok(canon_root) = std::fs::canonicalize(Path::new("/")) {
            protected_dirs.insert(canon_root);
        }
        Self {
            outboxes: HashSet::new(),
            inboxes: HashSet::new(),
            refused_dirs: HashSet::new(),
            protected_dirs,
        }
    }
}

/// Parses and validates a `file-cla` entry into a [`ParsedFileClaConfig`].
pub fn parse_file_cla_config(cla_config: &ClaConfig) -> anyhow::Result<ParsedFileClaConfig> {
    let typed_cfg: FileClaHamConfig = match &cla_config.cla_type {
        ClaType::Other {
            config: serde_json::Value::Object(map),
            ..
        } => {
            if map.contains_key("max-bundle-size") && map.contains_key("max_bundle_size") {
                anyhow::bail!(
                    "File CLA '{}' specifies both 'max-bundle-size' and 'max_bundle_size'",
                    cla_config.name
                );
            }
            serde_json::from_value(serde_json::Value::Object(map.clone())).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to parse file-cla configuration for CLA '{}': {e}",
                    cla_config.name
                )
            })?
        }
        ClaType::Other {
            config: serde_json::Value::Null,
            ..
        } => FileClaHamConfig::default(),
        ClaType::Other { config: other, .. } => {
            anyhow::bail!(
                "File CLA '{}' configuration must be a mapping/object, got {}",
                cla_config.name,
                other
            );
        }
        _ => {
            anyhow::bail!(
                "CLA '{}' type mismatch while parsing file-cla configuration",
                cla_config.name
            );
        }
    };

    let parsed_max_bundle_size = match typed_cfg.max_bundle_size {
        Some(bytes) => {
            let nz = NonZeroU64::new(bytes).ok_or_else(|| {
                anyhow::anyhow!(
                    "File CLA '{}' 'max-bundle-size' must be greater than 0",
                    cla_config.name
                )
            })?;
            Some(nz)
        }
        None => None,
    };

    let max_bundle_size =
        parsed_max_bundle_size.or_else(|| NonZeroU64::new(DEFAULT_FILE_CLA_MAX_BUNDLE_SIZE));

    if cla_config.schedule.is_some() {
        warn!(
            "File CLA '{}' has 'schedule' configured, which is not supported by file-cla and will be ignored",
            cla_config.name
        );
    }
    if cla_config.policy.is_some() {
        warn!(
            "File CLA '{}' has 'policy' configured, which is not supported by file-cla and will be ignored",
            cla_config.name
        );
    }

    if let Some(ref outbox) = typed_cfg.outbox {
        if is_empty_or_whitespace_path(outbox) {
            anyhow::bail!("File CLA '{}' has an empty 'outbox' path", cla_config.name);
        }
        if is_dot_cwd_path(outbox) {
            anyhow::bail!(
                "File CLA '{}' outbox cannot be '.' (watching the working directory would quarantine station files into refused/)",
                cla_config.name
            );
        }
    }
    for (peer_eid, inbox) in &typed_cfg.peers {
        if is_empty_or_whitespace_path(inbox) {
            anyhow::bail!(
                "File CLA '{}' has an empty inbox path for peer '{}'",
                cla_config.name,
                peer_eid
            );
        }
    }

    let outbox = typed_cfg
        .outbox
        .as_deref()
        .map(dtnpm_core::config::expand_tilde);
    let peers: HashMap<NodeId, PathBuf> = typed_cfg
        .peers
        .into_iter()
        .map(|(eid, p)| (eid, dtnpm_core::config::expand_tilde(&p)))
        .collect();

    if outbox.is_none() && peers.is_empty() {
        warn!(
            "File CLA '{}' has neither 'outbox' nor 'peers' configured and will remain inert",
            cla_config.name
        );
    }

    Ok(ParsedFileClaConfig {
        hardy_cfg: hardy_file_cla::Config { outbox, peers },
        max_bundle_size,
        allow_permissive_spool: typed_cfg.allow_permissive_spool,
    })
}

/// Provisions spool directories (`0700` on Unix), validates path invariants across
/// all `file-cla` instances, and constructs the [`hardy_file_cla::Cla`] instance.
pub(crate) fn init_file_cla(
    cla_config: &ClaConfig,
    registry: &mut FileClaPathRegistry,
) -> anyhow::Result<(Arc<dyn Cla>, ClaInit)> {
    let parsed = parse_file_cla_config(cla_config)?;
    let file_cfg = parsed.hardy_cfg;
    let max_bundle_size = parsed.max_bundle_size;
    #[cfg_attr(not(unix), allow(unused_variables))]
    let allow_permissive = parsed.allow_permissive_spool;

    // Ensure spool directories exist with 0700 permissions and validate
    // before registering/starting hardy-file-cla; clean up newly created
    // directories if validation fails.
    let mut created_dirs = Vec::new();
    let validate_res: anyhow::Result<(Option<PathBuf>, Vec<PathBuf>)> = (|| {
        let canon_outbox = if let Some(ref outbox) = file_cfg.outbox {
            ensure_file_cla_dir(&cla_config.name, outbox, &mut created_dirs)?;
            let canon = std::fs::canonicalize(outbox).with_context(|| {
                format!(
                    "Failed to canonicalize outbox '{}' for File CLA '{}'",
                    outbox.display(),
                    cla_config.name
                )
            })?;
            if registry.inboxes.contains(&canon) {
                anyhow::bail!(
                    "File CLA '{}' shares directory '{}' as both outbox and peer inbox (would cause a forwarding loop)",
                    cla_config.name,
                    canon.display()
                );
            }
            if registry.outboxes.contains(&canon) {
                anyhow::bail!(
                    "File CLA '{}' outbox '{}' is already watched by another file-cla instance",
                    cla_config.name,
                    canon.display()
                );
            }
            if registry.protected_dirs.contains(&canon) {
                anyhow::bail!(
                    "File CLA '{}' outbox '{}' cannot point to a station storage, config, home, or root directory (would quarantine files into refused/)",
                    cla_config.name,
                    canon.display()
                );
            }
            if registry.refused_dirs.contains(&canon) {
                anyhow::bail!(
                    "File CLA '{}' outbox '{}' cannot watch another file-cla's refused/ quarantine directory",
                    cla_config.name,
                    canon.display()
                );
            }
            let refused_dir = canon.join("refused");
            let canon_refused = std::fs::canonicalize(&refused_dir).ok();
            if registry.outboxes.contains(&refused_dir)
                || registry.inboxes.contains(&refused_dir)
                || canon_refused
                    .as_ref()
                    .is_some_and(|r| registry.outboxes.contains(r) || registry.inboxes.contains(r))
            {
                anyhow::bail!(
                    "File CLA '{}' outbox '{}' has a refused/ subdirectory that overlaps another file-cla spool directory",
                    cla_config.name,
                    canon.display()
                );
            }
            #[cfg(unix)]
            check_spool_dir_permissions(&cla_config.name, "outbox", &canon, allow_permissive)?;
            Some(canon)
        } else {
            None
        };

        let same_cla_refused = canon_outbox.as_ref().map(|o| {
            let r = o.join("refused");
            std::fs::canonicalize(&r).unwrap_or(r)
        });

        let mut cla_inboxes = HashSet::new();
        let mut validated_inboxes = Vec::with_capacity(file_cfg.peers.len());
        for (peer_eid, inbox) in &file_cfg.peers {
            ensure_file_cla_dir(&cla_config.name, inbox, &mut created_dirs)?;
            let canon = std::fs::canonicalize(inbox).with_context(|| {
                format!(
                    "Failed to canonicalize inbox '{}' for File CLA '{}'",
                    inbox.display(),
                    cla_config.name
                )
            })?;
            #[cfg(unix)]
            check_spool_dir_permissions(&cla_config.name, "peer inbox", &canon, allow_permissive)?;
            if !cla_inboxes.insert(canon.clone()) {
                anyhow::bail!(
                    "File CLA '{}' maps multiple peers (including '{}') to the same inbox '{}'; each peer in a file-cla must have a distinct inbox directory",
                    cla_config.name,
                    peer_eid,
                    canon.display()
                );
            }
            if canon_outbox.as_ref() == Some(&canon) || registry.outboxes.contains(&canon) {
                anyhow::bail!(
                    "File CLA '{}' shares directory '{}' as both outbox and peer inbox (would cause a forwarding loop)",
                    cla_config.name,
                    canon.display()
                );
            }
            if registry.refused_dirs.contains(&canon) || same_cla_refused.as_ref() == Some(&canon) {
                anyhow::bail!(
                    "File CLA '{}' peer inbox '{}' cannot target a file-cla refused/ quarantine directory",
                    cla_config.name,
                    canon.display()
                );
            }
            validated_inboxes.push(canon);
        }
        Ok((canon_outbox, validated_inboxes))
    })();

    let (canon_outbox, validated_inboxes) = match validate_res {
        Ok(v) => v,
        Err(e) => {
            for dir in created_dirs.iter().rev() {
                let _ = std::fs::remove_dir(dir);
            }
            return Err(e);
        }
    };

    info!(
        "Initializing File CLA '{}' (outbox={:?}, peers={}, max_bundle_size={:?})",
        cla_config.name,
        file_cfg.outbox,
        file_cfg.peers.len(),
        max_bundle_size
    );
    let cla = hardy_file_cla::Cla::new(&file_cfg).map_err(|e| {
        for dir in created_dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
        anyhow::anyhow!("Failed to initialize File CLA '{}': {e}", cla_config.name)
    })?;

    if let Some(canon) = canon_outbox {
        registry.refused_dirs.insert(canon.join("refused"));
        if let Ok(canon_refused) = std::fs::canonicalize(canon.join("refused")) {
            registry.refused_dirs.insert(canon_refused);
        }
        registry.outboxes.insert(canon);
    }
    registry.inboxes.extend(validated_inboxes);

    let cla: Arc<dyn Cla> = Arc::new(cla);
    let init = ClaInit {
        max_bundle_size,
        ..ClaInit::default()
    };
    Ok((cla, init))
}

fn is_empty_or_whitespace_path(path: &Path) -> bool {
    path.as_os_str().is_empty() || path.to_str().is_some_and(|s| s.trim().is_empty())
}

fn is_dot_cwd_path(path: &Path) -> bool {
    path == Path::new(".")
        || path == Path::new("./")
        || path
            .to_str()
            .is_some_and(|s| matches!(s.trim(), "." | "./"))
}

fn ensure_file_cla_dir(
    cla_name: &str,
    path: &Path,
    created_dirs: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    if path.exists() {
        if !path.is_dir() {
            anyhow::bail!(
                "File CLA '{}' spool path '{}' exists and is not a directory",
                cla_name,
                path.display()
            );
        }
        return Ok(());
    }

    let mut missing = Vec::new();
    let mut cur = Some(path);
    while let Some(p) = cur {
        if p.as_os_str().is_empty() || p.exists() {
            break;
        }
        missing.push(p.to_path_buf());
        cur = p.parent();
    }
    missing.reverse();

    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).with_context(|| {
        format!(
            "Failed to create spool directory '{}' for File CLA '{}'",
            path.display(),
            cla_name
        )
    })?;

    created_dirs.extend(missing);
    Ok(())
}

#[cfg(unix)]
fn check_spool_dir_permissions(
    cla_name: &str,
    role: &str,
    path: &Path,
    allow_permissive: bool,
) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            if !allow_permissive {
                anyhow::bail!(
                    "File CLA '{}' {} '{}' has permissive mode {:04o}; restrict with 'chmod 0700' or explicitly opt in with 'allow-permissive-spool: true'",
                    cla_name,
                    role,
                    path.display(),
                    mode
                );
            }
            warn!(
                "File CLA '{}' {} '{}' has permissive mode {:04o}; proceeding only because 'allow-permissive-spool: true' is explicitly enabled",
                cla_name,
                role,
                path.display(),
                mode
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_cla(name: &str, config: serde_json::Value) -> ClaConfig {
        ClaConfig {
            name: name.to_string(),
            cla_type: ClaType::Other {
                cla_type: "file-cla".to_string(),
                config,
            },
            policy: None,
            schedule: None,
            aliases: Vec::new(),
        }
    }

    #[test]
    fn parse_valid_config_with_byte_size_variants_and_tilde() {
        let cla_str = make_cla(
            "usb-str",
            json!({
                "outbox": "~/spool/out",
                "max-bundle-size": "16MB",
                "peers": {
                    "dtn://f4usb/": "/tmp/usb/in"
                }
            }),
        );
        let parsed = parse_file_cla_config(&cla_str).unwrap();
        assert_eq!(parsed.max_bundle_size, NonZeroU64::new(16 * 1024 * 1024));
        assert!(
            !parsed
                .hardy_cfg
                .outbox
                .unwrap()
                .to_string_lossy()
                .starts_with("~/")
        );
        assert_eq!(parsed.hardy_cfg.peers.len(), 1);
        assert!(!parsed.allow_permissive_spool);

        let cla_int = make_cla("usb-int", json!({ "max_bundle_size": 4096 }));
        let parsed = parse_file_cla_config(&cla_int).unwrap();
        assert_eq!(parsed.max_bundle_size, NonZeroU64::new(4096));

        let cla_default = make_cla("usb-def", json!({}));
        let parsed = parse_file_cla_config(&cla_default).unwrap();
        assert_eq!(
            parsed.max_bundle_size,
            NonZeroU64::new(DEFAULT_FILE_CLA_MAX_BUNDLE_SIZE)
        );
    }

    #[test]
    fn parse_rejects_invalid_max_bundle_size_and_duplicates() {
        for bad in [json!(0), json!(-1), json!("not-a-size"), json!(true)] {
            let cla = make_cla("bad-cap", json!({ "max-bundle-size": bad }));
            assert!(
                parse_file_cla_config(&cla).is_err(),
                "expected error for max-bundle-size={bad}"
            );
        }

        let dup = make_cla(
            "dup-cap",
            json!({ "max-bundle-size": 1024, "max_bundle_size": 2048 }),
        );
        let err = parse_file_cla_config(&dup).unwrap_err();
        assert!(err.to_string().contains("dup-cap"));
    }

    #[test]
    fn parse_rejects_unknown_fields_non_object_and_dangerous_paths() {
        let unknown = make_cla("typo-cla", json!({ "inbox": "/tmp/x" }));
        let err = parse_file_cla_config(&unknown).unwrap_err().to_string();
        assert!(err.contains("typo-cla") && err.contains("inbox"), "{err}");

        let non_obj = make_cla("arr-cla", json!(["a", "b"]));
        let err = parse_file_cla_config(&non_obj).unwrap_err().to_string();
        assert!(
            err.contains("arr-cla") && err.contains("mapping/object"),
            "{err}"
        );

        let empty_out = make_cla("empty-out", json!({ "outbox": "   " }));
        assert!(parse_file_cla_config(&empty_out).is_err());

        let dot_out = make_cla("dot-out", json!({ "outbox": "./" }));
        assert!(parse_file_cla_config(&dot_out).is_err());

        let empty_peer = make_cla("empty-peer", json!({ "peers": { "dtn://f4peer/": "" } }));
        assert!(parse_file_cla_config(&empty_peer).is_err());
    }

    #[test]
    fn protected_dirs_include_bare_relative_config_parent_home_and_root() {
        let cfg = HamConfig {
            config_path: Some(PathBuf::from("Cargo.toml")),
            ..Default::default()
        };
        let registry = FileClaPathRegistry::for_config(&cfg);
        let cwd = std::fs::canonicalize(".").expect("canonicalize cwd");
        assert!(
            registry.protected_dirs.contains(&cwd),
            "expected cwd '{}' from bare relative config_path to be protected",
            cwd.display()
        );
        let root = std::fs::canonicalize("/").expect("canonicalize /");
        assert!(registry.protected_dirs.contains(&root));
        if let Some(base) = directories::BaseDirs::new()
            && let Ok(home) = std::fs::canonicalize(base.home_dir())
        {
            assert!(registry.protected_dirs.contains(&home));
        }
    }
}
