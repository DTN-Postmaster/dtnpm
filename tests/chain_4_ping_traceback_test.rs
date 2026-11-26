#![cfg(all(feature = "node", feature = "mod-rego-routing", feature = "web-core"))]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use dtnpm::config::cla::{ClaConfig, ClaType, Tcpclv4HamConfig};
use dtnpm::config::{GrpcConfig, HamConfig, StorageConfig, WebConfig};
use dtnpm::server::HamServer;

fn alloc_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local_addr")
        .port()
}

fn make_route_free_chain_node(
    callsign: &str,
    node_id: &str,
    grpc_port: u16,
    web_port: u16,
    tcp_port: u16,
    peer_tcp_port: Option<u16>,
) -> HamConfig {
    HamConfig {
        callsign: Some(callsign.to_string()),
        node_id: node_id.to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: GrpcConfig {
            enabled: true,
            address: format!("127.0.0.1:{grpc_port}").parse().expect("grpc addr"),
            ..Default::default()
        },
        web: Some(WebConfig {
            address: format!("127.0.0.1:{web_port}").parse().expect("web addr"),
            ..Default::default()
        }),
        clas: vec![ClaConfig {
            name: format!("tcpcl-{}", callsign.to_ascii_lowercase()),
            cla_type: ClaType::TcpClv4(Tcpclv4HamConfig {
                address: Some(format!("127.0.0.1:{tcp_port}").parse().expect("tcp addr")),
                listeners: None,
                require_tls: Some(false),
                peers: peer_tcp_port.map(|p| vec![format!("127.0.0.1:{p}")]),
                segment_mru: None,
                transfer_mru: None,
            }),
            policy: None,
            schedule: None,
            aliases: Vec::new(),
        }],
        routes: vec![],
        ..Default::default()
    }
}

async fn wait_for_tcp(port: u16) {
    let addr = format!("127.0.0.1:{port}");
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {addr}");
}

fn run_dtnping(grpc_port: u16, temp_home: &std::path::Path, extra_args: &[&str]) -> String {
    let mut args = vec![
        "-p".to_string(),
        grpc_port.to_string(),
        "-c".to_string(),
        "1".to_string(),
        "-W".to_string(),
        "10s".to_string(),
        "dtn://statd/echo".to_string(),
    ];
    for a in extra_args {
        args.push((*a).to_string());
    }

    let out = Command::new(env!("CARGO_BIN_EXE_dtnping"))
        .env("HOME", temp_home)
        .args(&args)
        .output()
        .expect("run dtnping");

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "dtnping {:?} failed (status={:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        args,
        out.status.code()
    );
    assert!(
        stdout.contains("Reply from dtn://statd/echo: seq=0"),
        "expected reply from dtn://statd/echo in stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("1 received, 0.0% loss"),
        "expected 0.0% loss in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("  treb: dtn://statd/ -> dtn://statc/ -> dtn://statb/"),
        "expected TREB trace in dtnping stdout:\n{stdout}"
    );
    stdout
}

#[test]
fn example_chain_4_configs_are_valid_and_route_free() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("chain-4-stations");
    for (file, expected_id, expected_call) in [
        ("stata.yaml", "dtn://stata/", "STATA"),
        ("statb.yaml", "dtn://statb/", "STATB"),
        ("statc.yaml", "dtn://statc/", "STATC"),
        ("statd.yaml", "dtn://statd/", "STATD"),
    ] {
        let cfg = HamConfig::load(Some(&examples_dir.join(file))).expect("load example config");
        assert_eq!(cfg.node_id, expected_id);
        assert_eq!(cfg.callsign.as_deref(), Some(expected_call));
        assert!(
            cfg.routes.is_empty(),
            "{file} must remain route-free (zero static routes)"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_four_station_chain_route_free_ping_spray_and_aprs() {
    let grpc_a = alloc_port();
    let grpc_b = alloc_port();
    let grpc_c = alloc_port();
    let grpc_d = alloc_port();

    let web_a = alloc_port();
    let web_b = alloc_port();
    let web_c = alloc_port();
    let web_d = alloc_port();

    let tcp_a = alloc_port();
    let tcp_b = alloc_port();
    let tcp_c = alloc_port();
    let tcp_d = alloc_port();

    let cfg_d = make_route_free_chain_node("STATD", "dtn://statd/", grpc_d, web_d, tcp_d, None);
    let cfg_c =
        make_route_free_chain_node("STATC", "dtn://statc/", grpc_c, web_c, tcp_c, Some(tcp_d));
    let cfg_b =
        make_route_free_chain_node("STATB", "dtn://statb/", grpc_b, web_b, tcp_b, Some(tcp_c));
    let cfg_a =
        make_route_free_chain_node("STATA", "dtn://stata/", grpc_a, web_a, tcp_a, Some(tcp_b));

    let srv_d = HamServer::new(cfg_d).await.expect("srv_d");
    let srv_c = HamServer::new(cfg_c).await.expect("srv_c");
    let srv_b = HamServer::new(cfg_b).await.expect("srv_b");
    let srv_a = HamServer::new(cfg_a).await.expect("srv_a");

    let h_d = tokio::spawn(async move { srv_d.run().await });
    wait_for_tcp(tcp_d).await;
    let h_c = tokio::spawn(async move { srv_c.run().await });
    wait_for_tcp(tcp_c).await;
    let h_b = tokio::spawn(async move { srv_b.run().await });
    wait_for_tcp(tcp_b).await;
    let h_a = tokio::spawn(async move { srv_a.run().await });

    wait_for_tcp(grpc_a).await;
    wait_for_tcp(grpc_b).await;
    wait_for_tcp(grpc_c).await;
    wait_for_tcp(grpc_d).await;

    // Allow TCPCLv4 handshakes along the 4-station chain to settle
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let temp_home = tempfile::tempdir().expect("tempdir");

    // 1. Spray-and-Wait across 4 route-free stations (STATA -> STATB -> STATC -> STATD -> echo -> STATC -> STATB -> STATA)
    run_dtnping(grpc_a, temp_home.path(), &["--spray-quota", "8"]);

    // 2. APRS-like path listing transit relays
    run_dtnping(grpc_a, temp_home.path(), &["--aprs-path", "STATB,STATC"]);

    // 3. APRS-like path including final destination
    run_dtnping(
        grpc_a,
        temp_home.path(),
        &["--aprs-path", "STATB,STATC,STATD"],
    );

    h_a.abort();
    h_b.abort();
    h_c.abort();
    h_d.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_four_station_chain_route_free_dtntrigger_reply() {
    use std::io::Write;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command as AsyncCommand;

    let grpc_a = alloc_port();
    let grpc_b = alloc_port();
    let grpc_c = alloc_port();
    let grpc_d = alloc_port();

    let web_a = alloc_port();
    let web_b = alloc_port();
    let web_c = alloc_port();
    let web_d = alloc_port();

    let tcp_a = alloc_port();
    let tcp_b = alloc_port();
    let tcp_c = alloc_port();
    let tcp_d = alloc_port();

    let cfg_d = make_route_free_chain_node("STATD", "dtn://statd/", grpc_d, web_d, tcp_d, None);
    let cfg_c =
        make_route_free_chain_node("STATC", "dtn://statc/", grpc_c, web_c, tcp_c, Some(tcp_d));
    let cfg_b =
        make_route_free_chain_node("STATB", "dtn://statb/", grpc_b, web_b, tcp_b, Some(tcp_c));
    let cfg_a =
        make_route_free_chain_node("STATA", "dtn://stata/", grpc_a, web_a, tcp_a, Some(tcp_b));

    let srv_d = HamServer::new(cfg_d).await.expect("srv_d");
    let srv_c = HamServer::new(cfg_c).await.expect("srv_c");
    let srv_b = HamServer::new(cfg_b).await.expect("srv_b");
    let srv_a = HamServer::new(cfg_a).await.expect("srv_a");

    let h_d = tokio::spawn(async move { srv_d.run().await });
    wait_for_tcp(tcp_d).await;
    let h_c = tokio::spawn(async move { srv_c.run().await });
    wait_for_tcp(tcp_c).await;
    let h_b = tokio::spawn(async move { srv_b.run().await });
    wait_for_tcp(tcp_b).await;
    let h_a = tokio::spawn(async move { srv_a.run().await });

    wait_for_tcp(grpc_a).await;
    wait_for_tcp(grpc_b).await;
    wait_for_tcp(grpc_c).await;
    wait_for_tcp(grpc_d).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let temp_home = tempfile::tempdir().expect("tempdir");
    let script_path = temp_home.path().join("trigger_reply.sh");
    let dtnsend_bin = env!("CARGO_BIN_EXE_dtnsend");
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nset -eu\nprintf 'FORTUNE_OK:%s' \"$DTN_RETURN_PATH\" | \"{dtnsend_bin}\" -p \"$DTN_GRPC_PORT\" -s responder -r \"$1\" --aprs-path \"$DTN_RETURN_PATH\"\n"
        ),
    )
    .expect("write trigger script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod trigger script");
    }

    let mut trigger_child = AsyncCommand::new(env!("CARGO_BIN_EXE_dtntrigger"))
        .env("HOME", temp_home.path())
        .args([
            "-p",
            &grpc_d.to_string(),
            "-e",
            "responder",
            "-c",
            script_path.to_str().expect("utf8 script path"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtntrigger");

    let mut dtnprint_child = AsyncCommand::new(env!("CARGO_BIN_EXE_dtnprint"))
        .env("HOME", temp_home.path())
        .args(["-p", &grpc_a.to_string(), "-s", "client"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtnprint");

    tokio::time::sleep(Duration::from_millis(500)).await;

    for extra_flag in [["--spray-quota", "8"], ["--aprs-path", "STATB,STATC"]] {
        let mut child = Command::new(dtnsend_bin)
            .env("HOME", temp_home.path())
            .args([
                "-p",
                &grpc_a.to_string(),
                "-s",
                "client",
                "-r",
                "dtn://statd/responder",
                extra_flag[0],
                extra_flag[1],
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn dtnsend");
        child
            .stdin
            .take()
            .expect("dtnsend stdin")
            .write_all(b"en")
            .expect("write payload");
        let out = child.wait_with_output().expect("wait dtnsend");
        assert!(
            out.status.success(),
            "dtnsend {:?} failed: {}",
            extra_flag,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let stdout_pipe = dtnprint_child.stdout.take().expect("dtnprint stdout");
    let mut reader = BufReader::new(stdout_pipe);
    let mut captured = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(8), async {
        let mut line = String::new();
        while captured
            .matches("FORTUNE_OK:dtn://statc/,dtn://statb/")
            .count()
            < 2
        {
            line.clear();
            let n = reader.read_line(&mut line).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            captured.push_str(&line);
        }
    })
    .await;

    let _ = dtnprint_child.kill().await;
    let _ = trigger_child.kill().await;
    h_a.abort();
    h_b.abort();
    h_c.abort();
    h_d.abort();

    assert_eq!(
        captured
            .matches("FORTUNE_OK:dtn://statc/,dtn://statb/")
            .count(),
        2,
        "expected 2 route-free traceback replies at STATA dtnprint; got:\n{captured}"
    );
}
