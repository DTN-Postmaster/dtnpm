#![cfg(all(feature = "node", feature = "mod-rego-routing", feature = "web-core"))]

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command as AsyncCommand;

use dtnpm::config::cla::{ClaConfig, ClaType, Tcpclv4HamConfig};
use dtnpm::config::{GrpcConfig, HamConfig, RegoRoutingYamlConfig, StorageConfig, WebConfig};
use dtnpm::server::HamServer;

fn alloc_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local_addr")
        .port()
}

fn make_node_config(
    node_id: &str,
    grpc_port: Option<u16>,
    web_port: Option<u16>,
    tcp_port: u16,
    peers: Vec<String>,
    custom_routing_rego: Option<&str>,
) -> HamConfig {
    HamConfig {
        node_id: node_id.to_string(),
        storage: StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: match grpc_port {
            Some(p) => GrpcConfig {
                enabled: true,
                address: format!("127.0.0.1:{p}").parse().expect("grpc addr"),
                ..Default::default()
            },
            None => GrpcConfig {
                enabled: false,
                ..Default::default()
            },
        },
        web: web_port.map(|p| WebConfig {
            address: format!("127.0.0.1:{p}").parse().expect("web addr"),
            ..Default::default()
        }),
        clas: vec![ClaConfig {
            name: "tcpcl-mixed".to_string(),
            cla_type: ClaType::TcpClv4(Tcpclv4HamConfig {
                address: Some(format!("127.0.0.1:{tcp_port}").parse().expect("tcp addr")),
                listeners: None,
                require_tls: Some(false),
                peers: (!peers.is_empty()).then_some(peers),
                segment_mru: None,
                transfer_mru: None,
            }),
            policy: None,
            schedule: None,
        }],
        routes: vec![],
        rego_routing: custom_routing_rego.map(|rego| RegoRoutingYamlConfig {
            custom_routing_rego: Some(rego.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn combined_policy(
    aprs_dests: &str,
    aprs_relays: &str,
    snw_dests: &str,
    snw_relays: &str,
) -> String {
    format!(
        r#"
package dtn.routing

import rego.v1
import data.dtn.helpers

aprs_destinations := {aprs_dests}
aprs_relays := {aprs_relays}
snw_destinations := {snw_dests}
snw_relays := {snw_relays}

routes contains {{
    "pattern": pattern,
    "action": "VIA",
    "next_hop": peer.peer_eid,
    "priority": 90,
}} if {{
    some peer in input.peers
    helpers.peer_allowed(peer, input.node)
    not helpers.is_local_destination(peer.peer_eid, input.node)
    peer.peer_eid in aprs_relays
    some pattern in aprs_destinations
}}

routes contains {{
    "pattern": pattern,
    "action": "FORWARD_REPLICATE",
    "next_hop": peer.peer_eid,
    "priority": 100,
}} if {{
    some peer in input.peers
    helpers.peer_allowed(peer, input.node)
    not helpers.is_local_destination(peer.peer_eid, input.node)
    peer.peer_eid in snw_relays
    some pattern in snw_destinations
}}
"#
    )
}

fn send_bundle(grpc_port: u16, extra_args: &[&str], payload: &str) {
    let mut args = vec![
        "-p".to_string(),
        grpc_port.to_string(),
        "-s".to_string(),
        "dtn://a/sender".to_string(),
        "-r".to_string(),
        "dtn://d1/incoming".to_string(),
    ];
    for a in extra_args {
        args.push((*a).to_string());
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_dtnsend"))
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
        .expect("write payload");

    let out = child.wait_with_output().expect("wait dtnsend");
    assert!(
        out.status.success(),
        "dtnsend failed for '{payload}': {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_seven_dtnpm_mixed_aprs_and_spray_and_wait() {
    // 7-station topology:
    //   A -> B1 -> C1
    //   A -> B2 -> C2 -> D1
    //   A -> B3 -> C2
    let grpc_a = alloc_port();
    let grpc_d1 = alloc_port();

    let tcp_a = alloc_port();
    let tcp_b1 = alloc_port();
    let tcp_b2 = alloc_port();
    let tcp_b3 = alloc_port();
    let tcp_c1 = alloc_port();
    let tcp_c2 = alloc_port();
    let tcp_d1 = alloc_port();

    let relay_to_c2 = combined_policy(
        r#"{"dtn://d1/**"}"#,
        r#"{"dtn://c2/"}"#,
        r#"{"dtn://d1/**"}"#,
        r#"{"dtn://c2/"}"#,
    );
    let a_concurrent_policy = r#"
package dtn.routing

import rego.v1
import data.dtn.helpers

# Per-bundle APRS routing via RegoRib::decide(&bundle):
# Bundle 1 (route_key = dtn://b2/) and Bundle 2 (route_key = dtn://b3/)
# are routed concurrently without any policy reload.
decide := {
    "action": "FORWARD",
    "next_hop": input.bundle.route_key,
    "retain": false,
} if {
    input.bundle.route_key != null
    some peer in input.peers
    peer.peer_eid == input.bundle.route_key
    helpers.peer_allowed(peer, input.node)
}

# Fallback for Bundle 3 (Spray-and-Wait L=8, no APRS route_key):
# DefaultRib installs FORWARD_REPLICATE toward dtn://b2/.
routes contains {
    "pattern": "dtn://d1/**",
    "action": "FORWARD_REPLICATE",
    "next_hop": peer.peer_eid,
    "priority": 100,
} if {
    some peer in input.peers
    helpers.peer_allowed(peer, input.node)
    peer.peer_eid == "dtn://b2/"
}
"#;

    let cfg_d1 = make_node_config("dtn://d1/", Some(grpc_d1), None, tcp_d1, vec![], None);
    let cfg_c1 = make_node_config("dtn://c1/", None, None, tcp_c1, vec![], None);
    let cfg_c2 = make_node_config(
        "dtn://c2/",
        None,
        None,
        tcp_c2,
        vec![format!("127.0.0.1:{tcp_d1}")],
        None,
    );
    let cfg_b1 = make_node_config(
        "dtn://b1/",
        None,
        None,
        tcp_b1,
        vec![format!("127.0.0.1:{tcp_c1}")],
        None,
    );
    let cfg_b2 = make_node_config(
        "dtn://b2/",
        None,
        None,
        tcp_b2,
        vec![format!("127.0.0.1:{tcp_c2}")],
        Some(&relay_to_c2),
    );
    let cfg_b3 = make_node_config(
        "dtn://b3/",
        None,
        None,
        tcp_b3,
        vec![format!("127.0.0.1:{tcp_c2}")],
        Some(&relay_to_c2),
    );
    let cfg_a = make_node_config(
        "dtn://a/",
        Some(grpc_a),
        None,
        tcp_a,
        vec![
            format!("127.0.0.1:{tcp_b1}"),
            format!("127.0.0.1:{tcp_b2}"),
            format!("127.0.0.1:{tcp_b3}"),
        ],
        Some(a_concurrent_policy),
    );

    let srv_d1 = HamServer::new(cfg_d1).await.expect("srv_d1");
    let bpa_d1 = srv_d1.bpa();
    let h_d1 = tokio::spawn(async move { srv_d1.run().await });

    let srv_c1 = HamServer::new(cfg_c1).await.expect("srv_c1");
    let bpa_c1 = srv_c1.bpa();
    let h_c1 = tokio::spawn(async move { srv_c1.run().await });

    let srv_c2 = HamServer::new(cfg_c2).await.expect("srv_c2");
    let bpa_c2 = srv_c2.bpa();
    let h_c2 = tokio::spawn(async move { srv_c2.run().await });

    let srv_b1 = HamServer::new(cfg_b1).await.expect("srv_b1");
    let bpa_b1 = srv_b1.bpa();
    let h_b1 = tokio::spawn(async move { srv_b1.run().await });

    let srv_b2 = HamServer::new(cfg_b2).await.expect("srv_b2");
    let bpa_b2 = srv_b2.bpa();
    let h_b2 = tokio::spawn(async move { srv_b2.run().await });

    let srv_b3 = HamServer::new(cfg_b3).await.expect("srv_b3");
    let bpa_b3 = srv_b3.bpa();
    let h_b3 = tokio::spawn(async move { srv_b3.run().await });

    let srv_a = HamServer::new(cfg_a).await.expect("srv_a");
    let bpa_a = srv_a.bpa();
    let h_a = tokio::spawn(async move { srv_a.run().await });

    // Wait for TCPCLv4 links, RegoRoutingAgent reconciliation, and gRPC/HTTP servers.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let mut dtnprint_child = AsyncCommand::new(env!("CARGO_BIN_EXE_dtnprint"))
        .args(["-p", &grpc_d1.to_string(), "-s", "incoming", "-v"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtnprint");

    let stdout_pipe = dtnprint_child.stdout.take().expect("dtnprint stdout");
    let mut stderr_pipe = dtnprint_child.stderr.take().expect("dtnprint stderr");

    tokio::time::sleep(Duration::from_millis(400)).await;

    // Send all 3 bundles concurrently under the SAME static policy on A!
    // 1. Bundle 1: APRS via B2 -> C2 -> D1
    let p1 = "Bundle 1 APRS via B2";
    send_bundle(grpc_a, &["--aprs-path", "dtn://b2/,dtn://c2/"], p1);

    // 2. Bundle 2: APRS via B3 -> C2 -> D1
    let p2 = "Bundle 2 APRS via B3";
    send_bundle(grpc_a, &["--aprs-path", "dtn://b3/,dtn://c2/"], p2);

    // 3. Bundle 3: Spray-and-Wait with L=8 (8 -> 4 -> 2 across A -> B2 -> C2 -> D1)
    let p3 = "Bundle 3 Spray-and-Wait L=8";
    send_bundle(grpc_a, &["--spray-copies", "8"], p3);

    let mut reader = BufReader::new(stdout_pipe);
    let mut stdout_str = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(6), async {
        let mut line = String::new();
        while !(stdout_str.contains(p1) && stdout_str.contains(p2) && stdout_str.contains(p3)) {
            line.clear();
            let n = reader.read_line(&mut line).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            stdout_str.push_str(&line);
        }
    })
    .await;

    let _ = dtnprint_child.kill().await;
    let _ = dtnprint_child.wait().await;
    let mut stderr_str = String::new();
    let _ = stderr_pipe.read_to_string(&mut stderr_str).await;

    assert!(
        stdout_str.contains(p1),
        "D1 missing Bundle 1: stdout={stdout_str:?}, stderr={stderr_str:?}"
    );
    assert!(
        stdout_str.contains(p2),
        "D1 missing Bundle 2: stdout={stdout_str:?}, stderr={stderr_str:?}"
    );
    assert!(
        stdout_str.contains(p3),
        "D1 missing Bundle 3: stdout={stdout_str:?}, stderr={stderr_str:?}"
    );

    // Verify per-bundle path attribution (not just aggregated across all bundles).
    let bundle_blocks: Vec<&str> = stdout_str
        .split("From: ")
        .filter(|s| !s.is_empty())
        .collect();
    let b1 = bundle_blocks
        .iter()
        .find(|b| b.contains(p1))
        .expect("Bundle 1 block");
    assert!(
        b1.contains("dtn://b2/") && !b1.contains("dtn://b3/"),
        "Bundle 1 must traverse B2 (not B3): {b1:?}"
    );
    let b2 = bundle_blocks
        .iter()
        .find(|b| b.contains(p2))
        .expect("Bundle 2 block");
    assert!(
        b2.contains("dtn://b3/") && !b2.contains("dtn://b2/"),
        "Bundle 2 must traverse B3 (not B2): {b2:?}"
    );
    let b3 = bundle_blocks
        .iter()
        .find(|b| b.contains(p3))
        .expect("Bundle 3 block");
    assert!(
        b3.contains("Spray-Permit: 2") && b3.contains("Mesh-Replication: quota=2"),
        "Bundle 3 must carry Spray-Permit: 2 (8 -> 4 -> 2): {b3:?}"
    );
    assert!(
        stderr_str.contains("Spray-and-Wait remaining copies: 2"),
        "expected remaining copies on dtnprint stderr: stderr={stderr_str:?}"
    );

    bpa_a.shutdown().await;
    bpa_b1.shutdown().await;
    bpa_b2.shutdown().await;
    bpa_b3.shutdown().await;
    bpa_c1.shutdown().await;
    bpa_c2.shutdown().await;
    bpa_d1.shutdown().await;
    h_a.abort();
    h_b1.abort();
    h_b2.abort();
    h_b3.abort();
    h_c1.abort();
    h_c2.abort();
    h_d1.abort();
}
