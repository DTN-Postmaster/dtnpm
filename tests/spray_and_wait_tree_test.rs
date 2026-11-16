#![cfg(all(feature = "node", feature = "mod-rego-routing"))]

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command as AsyncCommand;

use dtnpm::config::cla::{ClaConfig, ClaType, Tcpclv4HamConfig};
use dtnpm::config::{GrpcConfig, HamConfig, RegoRoutingYamlConfig, StorageConfig};
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
        clas: vec![ClaConfig {
            name: "tcpcl-tree".to_string(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_five_dtnpm_tree_spray_and_wait_dtnsend_dtnprint() {
    // 5-node tree topology:
    //          node1 (root)
    //         /            \
    //      node2          node3
    //       /
    //    node4
    //     /
    //  node5 (final destination)
    let grpc1 = alloc_port();
    let grpc5 = alloc_port();
    let tcp1 = alloc_port();
    let tcp2 = alloc_port();
    let tcp3 = alloc_port();
    let tcp4 = alloc_port();
    let tcp5 = alloc_port();

    let node1_routing = r#"
package dtn.routing

import data.dtn.helpers

routes contains {
    "pattern": "dtn://node5/**",
    "action": "FORWARD_REPLICATE",
    "next_hop": peer.peer_eid,
    "priority": 90,
    "reason_code": 1,
} if {
    some peer in input.peers
    helpers.peer_allowed(peer, input.node)
    peer.peer_eid == "dtn://node2/"
}
"#;

    let node2_routing = r#"
package dtn.routing

import data.dtn.helpers

routes contains {
    "pattern": "dtn://node5/**",
    "action": "FORWARD_REPLICATE",
    "next_hop": peer.peer_eid,
    "priority": 90,
    "reason_code": 1,
} if {
    some peer in input.peers
    helpers.peer_allowed(peer, input.node)
    peer.peer_eid != "dtn://node1/"
}
"#;

    let cfg5 = make_node_config("dtn://node5/", Some(grpc5), tcp5, vec![], None);
    let cfg4 = make_node_config(
        "dtn://node4/",
        None,
        tcp4,
        vec![format!("127.0.0.1:{tcp5}")],
        None,
    );
    let cfg3 = make_node_config("dtn://node3/", None, tcp3, vec![], None);
    let cfg2 = make_node_config(
        "dtn://node2/",
        None,
        tcp2,
        vec![format!("127.0.0.1:{tcp4}")],
        Some(node2_routing),
    );
    let cfg1 = make_node_config(
        "dtn://node1/",
        Some(grpc1),
        tcp1,
        vec![format!("127.0.0.1:{tcp2}"), format!("127.0.0.1:{tcp3}")],
        Some(node1_routing),
    );

    let srv5 = HamServer::new(cfg5).await.expect("srv5");
    let bpa5 = srv5.bpa();
    let h5 = tokio::spawn(async move { srv5.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let srv3 = HamServer::new(cfg3).await.expect("srv3");
    let bpa3 = srv3.bpa();
    let h3 = tokio::spawn(async move { srv3.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let srv4 = HamServer::new(cfg4).await.expect("srv4");
    let bpa4 = srv4.bpa();
    let h4 = tokio::spawn(async move { srv4.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let srv2 = HamServer::new(cfg2).await.expect("srv2");
    let bpa2 = srv2.bpa();
    let h2 = tokio::spawn(async move { srv2.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let srv1 = HamServer::new(cfg1).await.expect("srv1");
    let bpa1 = srv1.bpa();
    let h1 = tokio::spawn(async move { srv1.run().await });

    // Wait for TCPCLv4 tree links and gRPC servers to settle.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Start dtnprint on final station (node5) and wait until it is actively listening.
    let mut dtnprint_child = AsyncCommand::new(env!("CARGO_BIN_EXE_dtnprint"))
        .args(["-p", &grpc5.to_string(), "-s", "incoming", "-v"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtnprint");

    let stdout_pipe = dtnprint_child.stdout.take().expect("dtnprint stdout");
    let stderr_pipe = dtnprint_child.stderr.take().expect("dtnprint stderr");
    let mut stderr_reader = BufReader::new(stderr_pipe);
    let mut stderr_str = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        let mut line = String::new();
        while !stderr_str.contains("Listening for bundles") {
            line.clear();
            let n = stderr_reader.read_line(&mut line).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            stderr_str.push_str(&line);
        }
    })
    .await;

    // Send bundle from node1 via dtnsend with --spray-copies 8
    let payload = "Hello Spray-and-Wait 5-node tree!";
    let mut dtnsend_child = Command::new(env!("CARGO_BIN_EXE_dtnsend"))
        .args([
            "-p",
            &grpc1.to_string(),
            "-s",
            "dtn://node1/sender",
            "-r",
            "dtn://node5/incoming",
            "--spray-copies",
            "8",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dtnsend");

    dtnsend_child
        .stdin
        .take()
        .expect("dtnsend stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");

    let dtnsend_out = dtnsend_child
        .wait_with_output()
        .expect("wait dtnsend output");
    assert!(
        dtnsend_out.status.success(),
        "dtnsend failed: {}",
        String::from_utf8_lossy(&dtnsend_out.stderr)
    );

    // Read dtnprint stdout until the bundle arrives across node1 -> node2 -> node4 -> node5 (bounded timeout).
    let mut stdout_reader = BufReader::new(stdout_pipe);
    let mut stdout_str = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(6), async {
        let mut line = String::new();
        while !(stdout_str.contains(payload) && stdout_str.contains("Spray-Permit:")) {
            line.clear();
            let n = stdout_reader.read_line(&mut line).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            stdout_str.push_str(&line);
        }
    })
    .await;

    let _ = dtnprint_child.kill().await;
    let _ = dtnprint_child.wait().await;
    let mut rest_stderr = String::new();
    let _ = tokio::time::timeout(
        Duration::from_millis(200),
        stderr_reader.read_to_string(&mut rest_stderr),
    )
    .await;
    stderr_str.push_str(&rest_stderr);

    assert!(
        stdout_str.contains("From: dtn://node1/sender"),
        "dtnprint stdout missing sender: stdout={stdout_str:?}, stderr={stderr_str:?}"
    );
    assert!(
        stdout_str.contains(payload),
        "dtnprint stdout missing payload: stdout={stdout_str:?}, stderr={stderr_str:?}"
    );
    assert!(
        stdout_str.contains("Spray-Permit: 2"),
        "expected Spray-Permit: 2 after 8 -> 4 -> 2 tree splits: stdout={stdout_str:?}"
    );
    assert!(
        stderr_str.contains("Spray-and-Wait remaining copies: 2"),
        "dtnprint stderr must report remaining potential copies: stderr={stderr_str:?}"
    );

    bpa1.shutdown().await;
    bpa2.shutdown().await;
    bpa3.shutdown().await;
    bpa4.shutdown().await;
    bpa5.shutdown().await;
    h1.abort();
    h2.abort();
    h3.abort();
    h4.abort();
    h5.abort();
}
