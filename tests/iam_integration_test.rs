#![cfg(all(feature = "node", feature = "cli"))]

use std::process::Command;

use dtnpm::{HamConfig, HamServer, InternalStorage, NoopSenderCla};
use dtnpm_core::config::{GrpcAuthConfig, GrpcTokenEntry};
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::cla::ClaInit;
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_cli_user_and_token_management() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("station.toml");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let cfg_toml = format!(
        r#"callsign = "F4CLI"
node-id = "dtn://f4cli/"

[storage]
in-memory = false
data-dir = "{}"

[grpc]
enabled = false
"#,
        data_dir.display()
    );
    std::fs::write(&cfg_path, cfg_toml).unwrap();

    let bin = env!("CARGO_BIN_EXE_dtnpm");

    // 1. Add user via CLI
    let out = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "user",
            "add",
            "f4abc",
            "--role",
            "Operator",
            "--password",
            "RadioPass1!",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "user add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 2. Update password via CLI
    let out = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "user",
            "passwd",
            "f4abc",
            "--password",
            "RadioPass2!",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "user passwd failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 3. List users
    let out = Command::new(bin)
        .args(["--config", cfg_path.to_str().unwrap(), "user", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("f4abc\tOperator"), "stdout={stdout}");

    // 4. Create API token
    let out = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "token",
            "add",
            "ltp-kiss-daemon",
            "--role",
            "Admin",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let raw_token = stdout
        .lines()
        .find_map(|l| l.split("never shown again): ").nth(1))
        .expect("token output must contain raw token")
        .trim()
        .to_string();
    assert!(raw_token.starts_with("dtnpm_"));

    // Verify password & token directly against SQLite InternalStorage
    let storage = InternalStorage::new_disk_with_quota(&data_dir, 1024 * 1024).unwrap();
    assert!(
        storage
            .verify_iam_user_password("f4abc", "RadioPass2!")
            .unwrap()
            .is_some()
    );
    let tok_rec = storage
        .verify_iam_api_token(&raw_token)
        .unwrap()
        .expect("CLI-issued token must verify");
    assert_eq!(tok_rec.role, "Admin");

    // 5. Revoke token and verify rejection
    let out = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "token",
            "revoke",
            "ltp-kiss-daemon",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(storage.verify_iam_api_token(&raw_token).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_ham_server_grpc_gateway_and_uds_registration() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("dtnpm-grpc.sock");
    let grpc_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };

    let cfg = HamConfig {
        callsign: Some("F4GW".to_string()),
        node_id: "dtn://f4gw/".to_string(),
        storage: dtnpm_core::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        web: None,
        grpc: dtnpm_core::config::GrpcConfig {
            enabled: true,
            address: format!("127.0.0.1:{grpc_port}").parse().unwrap(),
            auth: GrpcAuthConfig {
                enabled: true,
                socket_path: Some(sock_path.clone()),
                socket_group: None,
                socket_mode: "0600".to_string(),
                tokens: vec![
                    GrpcTokenEntry {
                        name: "admin-cli".to_string(),
                        token: "super-secret-admin-tok".to_string(),
                        role: "Admin".to_string(),
                    },
                    GrpcTokenEntry {
                        name: "readonly-op".to_string(),
                        token: "super-secret-op-tok".to_string(),
                        role: "Operator".to_string(),
                    },
                ],
            },
        },
        ..Default::default()
    };

    let server = HamServer::new(cfg).await.unwrap();
    let server_task = tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Wait briefly for gateway TCP + UDS socket initialization
    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    for _ in 0..50 {
        if sock_path.exists()
            && tokio::net::TcpStream::connect(("127.0.0.1", grpc_port))
                .await
                .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(sock_path.exists(), "UDS socket must exist");

    // 1. No token over TCP -> rejected
    let (unauth_bpa, _g1) = dtnpm_core::create_remote_bpa(&endpoint, None, None).unwrap();
    let res = unauth_bpa
        .register_cla(
            "unauth-cla".to_string(),
            Arc::new(NoopSenderCla::default()),
            None,
            ClaInit::default(),
        )
        .await;
    assert!(res.is_err(), "unauthenticated register_cla must fail");

    // 2. Operator token over TCP -> rejected by Rego PBAC for CLA registration
    let (op_bpa, _g2) =
        dtnpm_core::create_remote_bpa(&endpoint, Some("super-secret-op-tok"), None).unwrap();
    let res = op_bpa
        .register_cla(
            "op-cla".to_string(),
            Arc::new(NoopSenderCla::default()),
            None,
            ClaInit::default(),
        )
        .await;
    assert!(res.is_err(), "Operator cannot register admin CLA");

    // 3. Admin token over TCP -> allowed
    let (admin_bpa, _g3) =
        dtnpm_core::create_remote_bpa(&endpoint, Some("super-secret-admin-tok"), None).unwrap();
    let node_ids = admin_bpa
        .register_cla(
            "admin-cla".to_string(),
            Arc::new(NoopSenderCla::default()),
            None,
            ClaInit::default(),
        )
        .await
        .expect("Admin token register_cla must succeed");
    assert!(!node_ids.is_empty());

    // 4. Unix Domain Socket -> implicitly local Admin without token
    let sock_str = sock_path.to_str().unwrap();
    let (uds_bpa, _g4) = dtnpm_core::create_remote_bpa(&endpoint, None, Some(sock_str)).unwrap();
    let node_ids_uds = uds_bpa
        .register_cla(
            "uds-cla".to_string(),
            Arc::new(NoopSenderCla::default()),
            None,
            ClaInit::default(),
        )
        .await
        .expect("UDS register_cla must succeed without token");
    assert!(!node_ids_uds.is_empty());

    server_task.abort();
}
