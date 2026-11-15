#![cfg(all(feature = "node", feature = "mod-rego-routing", feature = "web-core"))]

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

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

async fn update_routing_policy(web_port: u16, routing_rego: &str) {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{web_port}/api/rego_routing/v1/policy");
    let resp = client
        .put(&url)
        .json(&serde_json::json!({
            "custom_routing_rego": routing_rego
        }))
        .send()
        .await
        .expect("PUT /api/rego_routing/v1/policy");
    assert!(
        resp.status().is_success(),
        "policy update failed: {}",
        resp.status()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_seven_dtnpm_mixed_aprs_and_spray_and_wait() {
    // 7-station topology:
    //   A -> B1 -> C1
    //   A -> B2 -> C2 -> D1
    //   A -> B3 -> C2
    let grpc_a = alloc_port();
    let web_a = alloc_port();
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
    let a_phase1_b2 = combined_policy(
        r#"{"dtn://d1/**"}"#,
        r#"{"dtn://b2/"}"#,
        r#"{"dtn://d1/**"}"#,
        r#"{"dtn://b2/", "dtn://b3/"}"#,
    );
    let a_phase2_b3 = combined_policy(
        r#"{"dtn://d1/**"}"#,
        r#"{"dtn://b3/"}"#,
        r#"{"dtn://d1/**"}"#,
        r#"{"dtn://b2/", "dtn://b3/"}"#,
    );
    let a_phase3_snw = combined_policy("set()", "set()", r#"{"dtn://d1/**"}"#, r#"{"dtn://b2/"}"#);

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
        Some(web_a),
        tcp_a,
        vec![
            format!("127.0.0.1:{tcp_b1}"),
            format!("127.0.0.1:{tcp_b2}"),
            format!("127.0.0.1:{tcp_b3}"),
        ],
        Some(&a_phase1_b2),
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

    let mut dtnprint_child = Command::new(env!("CARGO_BIN_EXE_dtnprint"))
        .args(["-p", &grpc_d1.to_string(), "-s", "incoming", "-v"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtnprint");

    tokio::time::sleep(Duration::from_millis(400)).await;

    // 1. Bundle 1: APRS via B2 -> C2 -> D1
    let p1 = "Bundle 1 APRS via B2";
    send_bundle(grpc_a, &["--aprs-path", "dtn://b2/,dtn://c2/"], p1);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 2. Bundle 2: APRS via B3 -> C2 -> D1
    update_routing_policy(web_a, &a_phase2_b3).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let p2 = "Bundle 2 APRS via B3";
    send_bundle(grpc_a, &["--aprs-path", "dtn://b3/,dtn://c2/"], p2);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3. Bundle 3: Spray-and-Wait with L=8 (8 -> 4 -> 2 across A -> B2 -> C2 -> D1)
    update_routing_policy(web_a, &a_phase3_snw).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let p3 = "Bundle 3 Spray-and-Wait L=8";
    send_bundle(grpc_a, &["--spray-copies", "8"], p3);
    tokio::time::sleep(Duration::from_millis(800)).await;

    let _ = dtnprint_child.kill();
    let out = dtnprint_child
        .wait_with_output()
        .expect("wait dtnprint output");
    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let stderr_str = String::from_utf8_lossy(&out.stderr);

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
    assert!(
        stdout_str.contains("dtn://b2/") && stdout_str.contains("dtn://b3/"),
        "expected TREB traces through both B2 and B3: stdout={stdout_str:?}"
    );
    assert!(
        stdout_str.contains("Spray-Permit: 2"),
        "expected Spray-Permit: 2 on D1: stdout={stdout_str:?}"
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
