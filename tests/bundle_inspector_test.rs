#![cfg(all(feature = "node", feature = "web-core"))]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dtnpm::{HamConfig, HamServer};
use tempfile::tempdir;
use tower::ServiceExt;

fn url_encode(input: &str) -> String {
    let mut encoded = String::new();
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(&mut encoded, "%{:02X}", byte);
            }
        }
    }
    encoded
}

#[tokio::test]
async fn test_bundle_inspector_endpoints_and_lifecycle() {
    let tmp = tempdir().expect("tempdir created");
    let data_dir = tmp.path().join("dtn_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let config = HamConfig {
        callsign: Some("F4JXQ".to_string()),
        node_id: "dtn://f4jxq/".to_string(),
        storage: dtnpm::config::StorageConfig {
            in_memory: false,
            data_dir: Some(data_dir.clone()),
            ..Default::default()
        },
        grpc: dtnpm::config::GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        web: Some(dtnpm::config::WebConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            static_dir: std::path::PathBuf::from("../dtnpm-web-core/ui/dist"),
        }),
        ..Default::default()
    };

    let server = HamServer::new(config.clone())
        .await
        .expect("HamServer should build with disk storage");
    server.bpa().start(false).await;

    let app = server
        .create_web_router()
        .await
        .expect("web router should build");

    // 1. Initial GET /api/v1/bundles - should be empty list
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/bundles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.len(), 0);

    // 2. Inject a text bundle via POST /api/v1/bundles
    let send_req = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/bundles")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"destination": "dtn://remote/inbox", "payload": "Antigravity ham test"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(send_req.status(), StatusCode::CREATED);
    let send_bytes = axum::body::to_bytes(send_req.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let send_json: serde_json::Value = serde_json::from_slice(&send_bytes).unwrap();
    let bundle_id = send_json["bundle_id"].as_str().unwrap().to_string();

    // 3. GET /api/v1/bundles - should now contain 1 bundle
    let res2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/bundles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res2.status(), StatusCode::OK);
    let bytes2 = axum::body::to_bytes(res2.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list2: Vec<serde_json::Value> = serde_json::from_slice(&bytes2).unwrap();
    assert_eq!(list2.len(), 1);
    assert_eq!(list2[0]["id"], bundle_id);
    assert_eq!(list2[0]["destination"], "dtn://remote/inbox");

    // 4. GET /api/v1/bundles?id=... - Inspect bundle details
    let inspect_uri = format!("/api/v1/bundles?id={}", url_encode(&bundle_id));
    let res3 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&inspect_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res3.status(), StatusCode::OK);
    let bytes3 = axum::body::to_bytes(res3.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let detail: serde_json::Value = serde_json::from_slice(&bytes3).unwrap();
    assert_eq!(detail["id"], bundle_id);
    assert_eq!(detail["mime_type"], "text/plain");
    assert_eq!(detail["is_binary"], false);
    assert_eq!(detail["payload_preview"], "Antigravity ham test");

    // 5. GET /api/v1/bundles/payload?id=... - Download raw payload
    let payload_uri = format!("/api/v1/bundles/payload?id={}", url_encode(&bundle_id));
    let res4 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&payload_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res4.status(), StatusCode::OK);
    assert_eq!(
        res4.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "text/plain"
    );
    let payload_bytes = axum::body::to_bytes(res4.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(payload_bytes.as_ref(), b"Antigravity ham test");

    // 6. DELETE /api/v1/bundles?id=... - Delete bundle from storage
    let delete_uri = format!("/api/v1/bundles?id={}", url_encode(&bundle_id));
    let res5 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&delete_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res5.status(), StatusCode::OK);
    let del_bytes = axum::body::to_bytes(res5.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let del_json: serde_json::Value = serde_json::from_slice(&del_bytes).unwrap();
    assert_eq!(del_json["status"], "deleted");

    // 7. Verify deletion: GET /api/v1/bundles?id=... should now be NOT_FOUND
    let res6 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&inspect_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res6.status(), StatusCode::NOT_FOUND);

    // 8. Verify bundle list is empty again
    let res7 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/bundles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res7.status(), StatusCode::OK);
    let bytes7 = axum::body::to_bytes(res7.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list7: Vec<serde_json::Value> = serde_json::from_slice(&bytes7).unwrap();
    assert_eq!(list7.len(), 0);
}

#[tokio::test]
async fn test_in_memory_bundle_inspector_and_connect_peer() {
    let mut config = HamConfig {
        callsign: Some("STATA".to_string()),
        node_id: "dtn://stata/".to_string(),
        storage: dtnpm::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: dtnpm::config::GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        web: Some(dtnpm::config::WebConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            static_dir: std::path::PathBuf::from("../dtnpm-web-core/ui/dist"),
        }),
        ..Default::default()
    };

    config.clas.push(dtnpm::config::cla::ClaConfig {
        name: "tcpcl-stata".to_string(),
        cla_type: dtnpm::config::cla::ClaType::TcpClv4(dtnpm::config::cla::Tcpclv4HamConfig {
            address: Some("127.0.0.1:0".parse().unwrap()),
            require_tls: Some(false),
            peers: Some(vec!["127.0.0.1:4559".to_string()]),
            ..Default::default()
        }),
        policy: None,
        schedule: None,
    });

    let server = HamServer::new(config.clone())
        .await
        .expect("HamServer should build with in-memory storage");
    server.bpa().start(false).await;

    let app = server
        .create_web_router()
        .await
        .expect("web router should build");

    // 1. Test POST /api/v1/clas/connect
    let connect_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/clas/connect")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"peer": "127.0.0.1:4559"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Since 127.0.0.1:4559 isn't listening, it will fail connection attempt but route succeeds in reaching handler
    assert!(connect_res.status().is_server_error());

    // 2. Inject a bundle in in-memory mode
    let send_req = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/bundles")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"destination": "dtn://remote/inbox", "payload": "In-memory test payload"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(send_req.status(), StatusCode::CREATED);

    // 3. GET /api/v1/bundles should now list the in-memory bundle!
    let list_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/bundles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(list_res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(list_res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["destination"], "dtn://remote/inbox");
}

#[cfg(feature = "cla-mercury")]
#[tokio::test]
async fn test_mercury_arq_peer_registered_in_cla_manager_and_connect_endpoint() {
    let mut config = HamConfig {
        callsign: Some("F4JXQ-2".to_string()),
        node_id: "dtn://f4jxq-2/".to_string(),
        storage: dtnpm::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: dtnpm::config::GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        web: Some(dtnpm::config::WebConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            static_dir: std::path::PathBuf::from("../dtnpm-web-core/ui/dist"),
        }),
        ..Default::default()
    };

    config.clas.push(dtnpm::config::cla::ClaConfig {
        name: "mercury-hf".to_string(),
        cla_type: dtnpm::config::cla::ClaType::MercuryArq(
            dtnpm::config::cla::MercuryArqHamConfig {
                my_callsign: Some("F4JXQ-2".to_string()),
                control_addr: "127.0.0.1:18300".parse().unwrap(),
                data_addr: "127.0.0.1:18301".parse().unwrap(),
                connect_timeout: Some(std::time::Duration::from_millis(200)),
                peers: vec![dtnpm::config::cla::MercuryPeerHamConfig {
                    callsign: "VA3NB".to_string(),
                    node_id: None,
                    schedule: None,
                }],
                ..Default::default()
            },
        ),
        policy: None,
        schedule: None,
    });

    let server = HamServer::new(config)
        .await
        .expect("HamServer should build with Mercury ARQ CLA");
    server.bpa().start(false).await;

    let statuses = server.cla_manager().list_peer_statuses().await;
    assert!(
        statuses.iter().any(|s| s.address == "VA3NB"),
        "Mercury peer VA3NB should be registered in ClaManager, got: {statuses:?}"
    );

    let app = server
        .create_web_router()
        .await
        .expect("web router should build");

    let connect_res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/clas/connect")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"peer": "VA3NB"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(connect_res.status().is_server_error());
    let body = axum::body::to_bytes(connect_res.into_body(), 1024 * 64)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        !body_str.contains("No configured"),
        "POST /api/v1/clas/connect should dispatch VA3NB to MercuryCla, got: {body_str}"
    );
}
