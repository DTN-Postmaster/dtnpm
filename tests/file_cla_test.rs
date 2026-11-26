#![cfg(all(feature = "node", feature = "cla-file", feature = "mod-rego-routing"))]

use std::path::{Path, PathBuf};
use std::time::Duration;

use dtnpm::HamServer;
use dtnpm::config::cla::{ClaConfig, ClaType};
use dtnpm::config::{GrpcConfig, HamConfig, StorageConfig};
use dtnpm_core::mesh_blocks::apply_routing_extensions;
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::Eid;
use serde_json::json;
use tempfile::tempdir;

fn build_bundle(src: &str, dst: &str, payload: &[u8], aprs_path: Option<&str>) -> Vec<u8> {
    let src_eid: Eid = src.parse().expect("valid src eid");
    let dst_eid: Eid = dst.parse().expect("valid dst eid");
    let builder = Builder::new(src_eid, dst_eid).with_payload(payload.to_vec().into());
    let builder =
        apply_routing_extensions(builder, Some(8), aprs_path, None).expect("apply extensions");
    let (_bundle, raw) = builder
        .build(CreationTimestamp::now())
        .expect("build bundle");
    raw.to_vec()
}

fn extract_payload_bytes(path: &Path) -> Vec<u8> {
    let raw = std::fs::read(path).expect("read forwarded bundle");
    let parsed = hardy_bpv7::parse::parse(raw.clone().into()).expect("valid BPv7 bundle");
    let p_block = parsed
        .bundle
        .blocks
        .get(&1)
        .expect("bundle has payload block");
    let r = p_block.payload_range();
    raw[r.start as usize..r.end as usize].to_vec()
}

fn list_regular_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

async fn wait_for_file_count(dir: &Path, expected: usize, timeout: Duration) -> Vec<PathBuf> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let files = list_regular_files(dir);
        if files.len() >= expected {
            return files;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {expected} file(s) in {} (found {})",
            dir.display(),
            files.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn test_multiple_file_clas_direct_and_outbox_routing() {
    let tmp = tempdir().expect("tempdir");
    let dir_a_out = tmp.path().join("usb/outbox");
    let dir_a_in = tmp.path().join("usb/inbox");
    let dir_b_out = tmp.path().join("ext/outbox");
    let dir_b_in = tmp.path().join("ext/inbox");

    let yaml = format!(
        r#"
callsign: "F4TEST"
node-id: "dtn://f4test/"
storage:
  in-memory: true
grpc:
  enabled: false
clas:
  - name: "usb-club"
    type: "file-cla"
    aliases: ["USB"]
    outbox: "{}"
    peers:
      "dtn://f4usb/": "{}"
  - name: "ext-bridge"
    type: "file"
    aliases: ["EXT"]
    outbox: "{}"
    peers:
      "dtn://f4ext/": "{}"
"#,
        dir_a_out.display(),
        dir_a_in.display(),
        dir_b_out.display(),
        dir_b_in.display(),
    );
    let cfg_path = tmp.path().join("config.yaml");
    std::fs::write(&cfg_path, yaml).expect("write config.yaml");

    let config = HamConfig::load(Some(&cfg_path)).expect("load config");
    assert_eq!(config.clas.len(), 2);
    assert_eq!(config.clas[0].effective_aliases(), vec!["FILE", "USB"]);
    assert_eq!(config.clas[1].effective_aliases(), vec!["EXT", "FILE"]);

    let server = HamServer::new(config).await.expect("server starts");
    server.bpa().start(false).await;

    // Give the directory watcher a moment to install before dropping the file.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let raw = build_bundle(
        "dtn://f4src/app",
        "dtn://f4ext/chat",
        b"hello-ext-cla",
        None,
    );
    let drop_path = dir_a_out.join("bundle_to_ext.bpv7");
    std::fs::write(&drop_path, &raw).expect("drop bundle into usb outbox");

    let inbox_files = wait_for_file_count(&dir_b_in, 1, Duration::from_secs(8)).await;
    assert_eq!(inbox_files.len(), 1);
    assert!(
        list_regular_files(&dir_a_in).is_empty(),
        "usb inbox should remain empty"
    );

    // Outbox file should be deleted after dispatch.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while drop_path.exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !drop_path.exists(),
        "outbox file should be deleted after dispatch"
    );

    let forwarded_bytes = std::fs::read(&inbox_files[0]).expect("read forwarded bundle");
    let parsed = hardy_bpv7::parse::parse(forwarded_bytes.into()).expect("valid BPv7 bundle");
    assert_eq!(
        parsed.bundle.primary.destination.to_string(),
        "dtn://f4ext/chat"
    );
    assert_eq!(extract_payload_bytes(&inbox_files[0]), b"hello-ext-cla");

    server.shutdown().await;
}

#[tokio::test]
async fn test_multiple_file_clas_aprs_alias_routing() {
    let tmp = tempdir().expect("tempdir");
    let usb_in = tmp.path().join("usb_inbox");
    let ext_in = tmp.path().join("ext_inbox");
    let ingest_out = tmp.path().join("ingest_outbox");

    let config = HamConfig {
        callsign: Some("F4APRS".to_string()),
        node_id: "dtn://f4aprs/".to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        clas: vec![
            ClaConfig {
                name: "usb-club".to_string(),
                cla_type: ClaType::Other {
                    cla_type: "file-cla".to_string(),
                    config: json!({
                        "peers": {
                            "dtn://f4usb/": usb_in
                        }
                    }),
                },
                policy: None,
                schedule: None,
                aliases: vec!["USB".to_string()],
            },
            ClaConfig {
                name: "ext-bridge".to_string(),
                cla_type: ClaType::Other {
                    cla_type: "file".to_string(),
                    config: json!({
                        "peers": {
                            "dtn://f4ext/": ext_in
                        }
                    }),
                },
                policy: None,
                schedule: None,
                aliases: vec!["EXT".to_string()],
            },
            ClaConfig {
                name: "ingest-spool".to_string(),
                cla_type: ClaType::Other {
                    cla_type: "file-cla".to_string(),
                    config: json!({
                        "outbox": ingest_out
                    }),
                },
                policy: None,
                schedule: None,
                aliases: Vec::new(),
            },
        ],
        ..Default::default()
    };

    let server = HamServer::new(config).await.expect("server starts");
    server.bpa().start(false).await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 1. USB1-1 -> only usb-club
    let b_usb = build_bundle(
        "dtn://f4src/app",
        "dtn://f4remote/chat",
        b"only-usb",
        Some("USB1-1"),
    );
    std::fs::write(ingest_out.join("01_usb.bpv7"), b_usb).expect("write usb bundle");
    wait_for_file_count(&usb_in, 1, Duration::from_secs(8)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        list_regular_files(&ext_in).len(),
        0,
        "USB1-1 must not leak to ext-bridge"
    );

    // 2. EXT1-1 -> only ext-bridge
    let b_ext = build_bundle(
        "dtn://f4src/app",
        "dtn://f4remote/chat",
        b"only-ext",
        Some("EXT1-1"),
    );
    std::fs::write(ingest_out.join("02_ext.bpv7"), b_ext).expect("write ext bundle");
    wait_for_file_count(&ext_in, 1, Duration::from_secs(8)).await;
    assert_eq!(
        list_regular_files(&usb_in).len(),
        1,
        "EXT1-1 must not leak to usb-club"
    );

    // 3. FILE1-1 -> automatic 'FILE' transport alias reaches BOTH file-cla instances
    let b_file = build_bundle(
        "dtn://f4src/app",
        "dtn://f4remote/chat",
        b"both-file",
        Some("FILE1-1"),
    );
    std::fs::write(ingest_out.join("03_file.bpv7"), b_file).expect("write file bundle");
    wait_for_file_count(&usb_in, 2, Duration::from_secs(8)).await;
    wait_for_file_count(&ext_in, 2, Duration::from_secs(8)).await;

    let mut usb_payloads: Vec<Vec<u8>> = list_regular_files(&usb_in)
        .iter()
        .map(|p| extract_payload_bytes(p))
        .collect();
    usb_payloads.sort();
    assert_eq!(
        usb_payloads,
        vec![b"both-file".to_vec(), b"only-usb".to_vec()],
        "usb_in should contain exactly only-usb and both-file"
    );

    let mut ext_payloads: Vec<Vec<u8>> = list_regular_files(&ext_in)
        .iter()
        .map(|p| extract_payload_bytes(p))
        .collect();
    ext_payloads.sort();
    assert_eq!(
        ext_payloads,
        vec![b"both-file".to_vec(), b"only-ext".to_vec()],
        "ext_in should contain exactly only-ext and both-file"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn test_file_cla_fails_startup_on_uncreatable_path() {
    let tmp = tempdir().expect("tempdir");
    let blocked = tmp.path().join("blocked");
    std::fs::write(&blocked, b"not a directory").expect("create blocking file");

    let config = HamConfig {
        callsign: Some("F4FAIL".to_string()),
        node_id: "dtn://f4fail/".to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        clas: vec![ClaConfig {
            name: "broken-usb".to_string(),
            cla_type: ClaType::Other {
                cla_type: "file-cla".to_string(),
                config: json!({
                    "outbox": blocked
                }),
            },
            policy: None,
            schedule: None,
            aliases: vec!["USB".to_string()],
        }],
        ..Default::default()
    };

    let err = match HamServer::new(config).await {
        Ok(_) => panic!("expected startup failure when file-cla outbox is a regular file"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("broken-usb"),
        "error should mention CLA name 'broken-usb', got: {msg}"
    );
}

#[tokio::test]
async fn test_file_cla_rejects_unknown_config_field() {
    let tmp = tempdir().expect("tempdir");
    let config = HamConfig {
        callsign: Some("F4TYPO".to_string()),
        node_id: "dtn://f4typo/".to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        clas: vec![ClaConfig {
            name: "typo-usb".to_string(),
            cla_type: ClaType::Other {
                cla_type: "file-cla".to_string(),
                config: json!({
                    "inbox": tmp.path().join("wrong_key")
                }),
            },
            policy: None,
            schedule: None,
            aliases: vec!["USB".to_string()],
        }],
        ..Default::default()
    };

    let err = match HamServer::new(config).await {
        Ok(_) => panic!("expected startup failure on unknown file-cla field 'inbox'"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("typo-usb") && msg.contains("inbox"),
        "error should mention CLA name and unknown field 'inbox', got: {msg}"
    );
}

#[tokio::test]
async fn test_file_cla_rejects_outbox_inbox_overlap_and_duplicate_outbox() {
    let tmp = tempdir().expect("tempdir");
    let shared = tmp.path().join("shared_dir");

    // 1. Same directory configured as both outbox and peer inbox (forwarding loop)
    let loop_cfg = HamConfig {
        callsign: Some("F4LOOP".to_string()),
        node_id: "dtn://f4loop/".to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        clas: vec![ClaConfig {
            name: "loop-cla".to_string(),
            cla_type: ClaType::Other {
                cla_type: "file-cla".to_string(),
                config: json!({
                    "outbox": shared,
                    "peers": {
                        "dtn://f4peer/": shared
                    }
                }),
            },
            policy: None,
            schedule: None,
            aliases: Vec::new(),
        }],
        ..Default::default()
    };
    let err = match HamServer::new(loop_cfg).await {
        Ok(_) => panic!("expected startup failure when outbox == inbox"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("loop"),
        "expected loop error, got: {err}"
    );

    // 2. Two file-cla instances watching the same outbox
    let dup_out = tmp.path().join("dup_outbox");
    let dup_cfg = HamConfig {
        callsign: Some("F4DUP".to_string()),
        node_id: "dtn://f4dup/".to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        clas: vec![
            ClaConfig {
                name: "usb-1".to_string(),
                cla_type: ClaType::Other {
                    cla_type: "file-cla".to_string(),
                    config: json!({ "outbox": dup_out }),
                },
                policy: None,
                schedule: None,
                aliases: Vec::new(),
            },
            ClaConfig {
                name: "usb-2".to_string(),
                cla_type: ClaType::Other {
                    cla_type: "file".to_string(),
                    config: json!({ "outbox": dup_out }),
                },
                policy: None,
                schedule: None,
                aliases: Vec::new(),
            },
        ],
        ..Default::default()
    };
    let err = match HamServer::new(dup_cfg).await {
        Ok(_) => panic!("expected startup failure on duplicate outbox"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("usb-2") && err.to_string().contains("already watched"),
        "expected duplicate outbox error, got: {err}"
    );
}

#[cfg(feature = "cli")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_crossover_stata_statb_bidirectional_dtnsend_dtnprint() {
    use std::io::Write;
    use std::net::TcpListener;
    use std::process::{Command, Stdio};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio::process::Command as AsyncCommand;

    let tmp = tempdir().expect("tempdir");
    let a_to_b = tmp.path().join("link/a_to_b");
    let b_to_a = tmp.path().join("link/b_to_a");

    let grpc_a = TcpListener::bind("127.0.0.1:0")
        .expect("bind grpc_a")
        .local_addr()
        .expect("local_addr a")
        .port();
    let grpc_b = TcpListener::bind("127.0.0.1:0")
        .expect("bind grpc_b")
        .local_addr()
        .expect("local_addr b")
        .port();

    let stata_yaml = format!(
        r#"callsign: "STATA"
node-id: "dtn://stata/"
storage:
  in-memory: true
grpc:
  enabled: true
  address: "127.0.0.1:{grpc_a}"
clas:
  - name: "file-link-statb"
    type: "file-cla"
    aliases: ["USB"]
    outbox: "{}"
    peers:
      "dtn://statb/": "{}"
"#,
        b_to_a.display(),
        a_to_b.display()
    );
    let statb_yaml = format!(
        r#"callsign: "STATB"
node-id: "dtn://statb/"
storage:
  in-memory: true
grpc:
  enabled: true
  address: "127.0.0.1:{grpc_b}"
clas:
  - name: "file-link-stata"
    type: "file-cla"
    aliases: ["USB"]
    outbox: "{}"
    peers:
      "dtn://stata/": "{}"
"#,
        a_to_b.display(),
        b_to_a.display()
    );

    let stata_cfg_path = tmp.path().join("stata-file-cla.yaml");
    let statb_cfg_path = tmp.path().join("statb-file-cla.yaml");
    std::fs::write(&stata_cfg_path, &stata_yaml).expect("write stata yaml");
    std::fs::write(&statb_cfg_path, &statb_yaml).expect("write statb yaml");

    let cfg_a = HamConfig::load(Some(&stata_cfg_path)).expect("load stata config");
    let cfg_b = HamConfig::load(Some(&statb_cfg_path)).expect("load statb config");

    let srv_a = HamServer::new(cfg_a).await.expect("srv_a");
    let bpa_a = srv_a.bpa();
    let h_a = tokio::spawn(async move { srv_a.run().await });

    let srv_b = HamServer::new(cfg_b).await.expect("srv_b");
    let bpa_b = srv_b.bpa();
    let h_b = tokio::spawn(async move { srv_b.run().await });

    // Wait for gRPC servers, file watchers, and RegoRoutingAgent reconciliation.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut print_b = AsyncCommand::new(env!("CARGO_BIN_EXE_dtnprint"))
        .env("HOME", tmp.path())
        .args(["-p", &grpc_b.to_string(), "-s", "incoming", "-v"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtnprint on STATB");
    let stdout_b = print_b.stdout.take().expect("dtnprint B stdout");
    let mut stderr_b = print_b.stderr.take().expect("dtnprint B stderr");

    let mut print_a = AsyncCommand::new(env!("CARGO_BIN_EXE_dtnprint"))
        .env("HOME", tmp.path())
        .args(["-p", &grpc_a.to_string(), "-s", "incoming", "-v"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtnprint on STATA");
    let stdout_a = print_a.stdout.take().expect("dtnprint A stdout");
    let mut stderr_a = print_a.stderr.take().expect("dtnprint A stderr");

    // Wait for both dtnprint services to register on their respective BPAs.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let send_via_dtnsend =
        |grpc_port: u16, sender: &str, receiver: &str, extra: &[&str], payload: &str| {
            let mut args = vec![
                "-p".to_string(),
                grpc_port.to_string(),
                "-s".to_string(),
                sender.to_string(),
                "-r".to_string(),
                receiver.to_string(),
            ];
            for e in extra {
                args.push((*e).to_string());
            }
            let mut child = Command::new(env!("CARGO_BIN_EXE_dtnsend"))
                .env("HOME", tmp.path())
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn dtnsend");
            child
                .stdin
                .take()
                .expect("dtnsend stdin")
                .write_all(payload.as_bytes())
                .expect("write dtnsend payload");
            let out = child.wait_with_output().expect("wait dtnsend");
            assert!(
                out.status.success(),
                "dtnsend failed for '{payload}': {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };

    let msg_ab_direct = "Hello STATB from STATA via file-cla";
    let msg_ab_usb = "APRS USB1-1 STATA -> STATB";
    let msg_ba_direct = "Hello STATA from STATB via file-cla";
    let msg_ba_file = "APRS FILE1-1 STATB -> STATA";

    // STATA -> STATB (direct + APRS USB1-1)
    send_via_dtnsend(
        grpc_a,
        "dtn://stata/cli",
        "dtn://statb/incoming",
        &[],
        msg_ab_direct,
    );
    send_via_dtnsend(
        grpc_a,
        "dtn://stata/cli",
        "dtn://statb/incoming",
        &["--aprs-path", "USB1-1"],
        msg_ab_usb,
    );

    // STATB -> STATA (direct + APRS FILE1-1)
    send_via_dtnsend(
        grpc_b,
        "dtn://statb/cli",
        "dtn://stata/incoming",
        &[],
        msg_ba_direct,
    );
    send_via_dtnsend(
        grpc_b,
        "dtn://statb/cli",
        "dtn://stata/incoming",
        &["--aprs-path", "FILE1-1"],
        msg_ba_file,
    );

    let read_until = |pipe: tokio::process::ChildStdout, expected: Vec<&'static str>| async move {
        let mut reader = BufReader::new(pipe);
        let mut collected = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(12), async {
            let mut line = String::new();
            while !expected.iter().all(|needle| collected.contains(needle)) {
                line.clear();
                let n = reader.read_line(&mut line).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                collected.push_str(&line);
            }
        })
        .await;
        collected
    };

    let (out_b, out_a) = tokio::join!(
        read_until(stdout_b, vec![msg_ab_direct, msg_ab_usb]),
        read_until(stdout_a, vec![msg_ba_direct, msg_ba_file]),
    );

    let _ = print_b.kill().await;
    let _ = print_b.wait().await;
    let _ = print_a.kill().await;
    let _ = print_a.wait().await;

    let mut err_b = String::new();
    let _ = stderr_b.read_to_string(&mut err_b).await;
    let mut err_a = String::new();
    let _ = stderr_a.read_to_string(&mut err_a).await;

    assert!(
        out_b.contains(msg_ab_direct) && out_b.contains(msg_ab_usb),
        "STATB dtnprint missing STATA bundles: stdout={out_b:?}, stderr={err_b:?}"
    );
    assert!(
        out_a.contains(msg_ba_direct) && out_a.contains(msg_ba_file),
        "STATA dtnprint missing STATB bundles: stdout={out_a:?}, stderr={err_a:?}"
    );

    bpa_a.shutdown().await;
    bpa_b.shutdown().await;
    h_a.abort();
    h_b.abort();
}

#[tokio::test]
async fn test_file_cla_rejects_shared_peer_inbox_within_same_cla() {
    let tmp = tempdir().expect("tempdir");
    let shared_inbox = tmp.path().join("shared_inbox");

    let cfg = HamConfig {
        callsign: Some("F4SHARE".to_string()),
        node_id: "dtn://f4share/".to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        clas: vec![ClaConfig {
            name: "shared-usb".to_string(),
            cla_type: ClaType::Other {
                cla_type: "file-cla".to_string(),
                config: json!({
                    "peers": {
                        "dtn://f4peer1/": shared_inbox,
                        "dtn://f4peer2/": shared_inbox
                    }
                }),
            },
            policy: None,
            schedule: None,
            aliases: Vec::new(),
        }],
        ..Default::default()
    };

    let err = match HamServer::new(cfg).await {
        Ok(_) => panic!("expected startup failure when two peers share an inbox directory"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("shared-usb") && msg.contains("distinct inbox"),
        "expected distinct inbox error mentioning 'shared-usb', got: {msg}"
    );
}
