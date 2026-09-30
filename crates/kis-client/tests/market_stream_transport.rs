//! Boundary tests for the market-only HTTP approval and WebSocket transport.
//! Every server here is a synthetic loopback server; no KIS host or protected
//! runtime path is touched.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use kis_client::clock::TestClock;
use kis_client::market_stream::{
    MarketStreamConfig, MarketStreamError, MarketStreamEvent, MarketSubscriptionOperation,
};
use kis_client::market_stream_approval::{ApprovalClient, ApprovalError};
use kis_client::market_stream_state::MarketStreamDomain;
use kis_client::market_stream_state::StateError;
use kis_client::market_stream_wire::MarketStreamSessionProof;
use kis_client::secret::Secret;
use sha1_smol::Sha1;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, oneshot};
use uuid::Uuid;

const APPROVAL_SENTINEL: &str = "SYNTHETIC_APPROVAL_KEY";
const APPKEY_SENTINEL: &str = "SYNTHETIC_APPKEY";
const SECRET_SENTINEL: &str = "SYNTHETIC_SECRET";

#[test]
fn synthetic_fixture_manifest_declares_source_and_wire_contract() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/market_stream_manifest.json")).unwrap();
    assert_eq!(manifest["synthetic"], true);
    assert_eq!(manifest["wire_version"], kis_client::WIRE_VERSION);
    assert_eq!(manifest["expected_field_count"], 47);
    assert_eq!(manifest["symbols"].as_array().unwrap().len(), 30);
    let packed = manifest["four_record_fixture"]["wire_payload"]
        .as_str()
        .unwrap();
    let fields = packed.splitn(4, '|').nth(3).unwrap().split('^').count();
    assert_eq!(manifest["four_record_fixture"]["record_count"], 4);
    assert_eq!(manifest["four_record_fixture"]["field_count"], 188);
    assert_eq!(fields, 188);
    assert_eq!(manifest["malformed_fixtures"].as_array().unwrap().len(), 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_http_and_fragmented_ws_transport_preserve_wire_counts_and_receipts() {
    let directory = tempfile::tempdir().expect("temporary test state");
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();

    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let (approval_body_tx, approval_body_rx) = oneshot::channel();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        approval_body_tx,
        APPROVAL_SENTINEL,
    ));

    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (command_tx, command_rx) = oneshot::channel();
    let ws_task = tokio::spawn(run_happy_ws_server(ws_listener, command_tx));

    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "credential-generation-7",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_ack_timeout(Duration::from_millis(500));
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();

    session.subscribe("005930").await.unwrap();
    let command = command_rx.await.unwrap();
    assert!(
        !command.is_empty(),
        "the command was captured as a WS text message"
    );
    assert!(command.contains(APPROVAL_SENTINEL));
    assert!(command.contains(r#""tr_type":"1""#));
    assert!(!command.contains(r#""tr_type":"0""#));
    assert!(command.contains(r#""tr_id":"H0STCNT0""#));

    let body = approval_body_rx.await.unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body_json["grant_type"], "client_credentials");
    assert_eq!(body_json["appkey"], APPKEY_SENTINEL);
    assert_eq!(body_json["secretkey"], SECRET_SENTINEL);
    assert_eq!(
        session.subscribed_symbols().collect::<Vec<_>>(),
        vec!["005930"]
    );

    assert!(matches!(
        session.next_event().await.unwrap(),
        MarketStreamEvent::ApplicationHeartbeat
    ));
    let event = session.next_event().await.unwrap();
    let MarketStreamEvent::Receipt(receipt) = event else {
        panic!("expected a receipt after fragmented/control frames")
    };
    assert_eq!(receipt.observation().symbol, "005930");
    assert_eq!(receipt.observation().base_price, None);
    assert_eq!(receipt.receive_ordinal(), 1);

    let debug = format!("{session:?}");
    assert!(!debug.contains(APPKEY_SENTINEL));
    assert!(!debug.contains(SECRET_SENTINEL));
    assert!(!format!("{:#?}", state).contains(APPKEY_SENTINEL));
    let state_bytes =
        std::fs::read(state.state_directory().join("approval-state-v1.json")).unwrap();
    assert!(
        !state_bytes
            .windows(APPKEY_SENTINEL.len())
            .any(|window| window == APPKEY_SENTINEL.as_bytes())
    );
    assert!(
        !state_bytes
            .windows(SECRET_SENTINEL.len())
            .any(|window| window == SECRET_SENTINEL.as_bytes())
    );

    session.close().await.unwrap();
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_ack_never_activates_a_subscription() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_wrong_ack_server(ws_listener));

    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "generation-1",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_ack_timeout(Duration::from_millis(500));
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    assert_eq!(
        session.subscribe("005930").await.unwrap_err(),
        MarketStreamError::AckMismatch
    );
    assert_eq!(session.subscribed_symbols().count(), 0);
    assert!(session.is_closed());
    assert!(!read_stream_state(&state)["pending_command"].is_null());
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_live_session_releases_the_lifetime_lock() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_idle_ws_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "generation-cancel",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap();
    let session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    drop(session);
    assert!(state.test_lock().is_ok(), "drop releases the lifetime lock");
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_http_rejection_does_not_follow_or_echo_provider_text() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_headers(&mut stream).await;
        let length = header_value(&request, "content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).await.unwrap();
        let leak = format!("{SECRET_SENTINEL}:{APPKEY_SENTINEL}");
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/{leak}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{leak}",
            leak.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let client = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "generation-rejected",
    )
    .unwrap();
    let error = client.test_key_for_connection().await.unwrap_err();
    assert_eq!(error, ApprovalError::HttpRejected { status: 302 });
    assert!(!error.to_string().contains(APPKEY_SENTINEL));
    assert!(!error.to_string().contains(SECRET_SENTINEL));
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_creates_a_new_epoch_and_reuses_cached_approval_without_resetting_budget() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(1_789_916_400_000));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let (approval_count_tx, approval_count_rx) = oneshot::channel();
    let approval_task = tokio::spawn(run_counting_approval_server(
        approval_listener,
        approval_count_tx,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_two_epoch_ws_server(ws_listener));

    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "generation-1",
    )
    .unwrap()
    .with_clock(clock.clone());
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_clock(clock.clone())
            .with_ack_timeout(Duration::from_millis(500));
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    let first_epoch = session.epoch();
    session.subscribe("005930").await.unwrap();
    assert!(matches!(
        session.next_event().await.unwrap(),
        MarketStreamEvent::Status { .. }
    ));
    clock.advance_ms(10_000);
    session.reconnect().await.unwrap();
    assert_ne!(first_epoch, session.epoch());
    assert!(matches!(
        session.next_event().await.unwrap(),
        MarketStreamEvent::Status {
            code: kis_client::StreamStatusCode::Gap,
            ..
        }
    ));
    session.subscribe("005930").await.unwrap();
    assert_eq!(session.subscribed_symbols().count(), 1);
    session.close().await.unwrap();

    assert_eq!(
        approval_count_rx.await.unwrap(),
        1,
        "cached approval is reused"
    );
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_process_lock_contention_and_restart_preserve_attempt_state() {
    if std::env::var_os("KIS_MARKET_STREAM_LOCK_CHILD").is_some() {
        let path = std::env::var_os("KIS_MARKET_STREAM_LOCK_PATH").unwrap();
        let slot = Uuid::parse_str(&std::env::var("KIS_MARKET_STREAM_LOCK_SLOT").unwrap()).unwrap();
        let store = MarketStreamDomain::for_test(path, slot).unwrap();
        store.test_append_command_attempt(777).unwrap();
        let _lock = store.test_lock().unwrap();
        println!("READY");
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(30));
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let slot = Uuid::new_v4();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("separate_process_lock_contention_and_restart_preserve_attempt_state")
        .arg("--nocapture")
        .env("KIS_MARKET_STREAM_LOCK_CHILD", "1")
        .env("KIS_MARKET_STREAM_LOCK_PATH", &path)
        .env("KIS_MARKET_STREAM_LOCK_SLOT", slot.to_string())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut ready = false;
    for _ in 0..4 {
        line.clear();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if line.contains("READY") {
            ready = true;
            break;
        }
    }
    assert!(ready, "child process did not report its lock readiness");
    let store = MarketStreamDomain::for_test(directory.path(), slot).unwrap();
    assert!(
        store.test_lock().is_err(),
        "the child process holds the lifetime inode"
    );
    child.kill().unwrap();
    let _ = child.wait();
    let state_json =
        std::fs::read_to_string(path.join("kis-market-stream/approval-state-v1.json")).unwrap();
    assert!(
        state_json.contains("777"),
        "restart retained the durable attempt history"
    );
    assert!(store.test_lock().is_ok(), "crash released the inode lock");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_held_state_anchor_is_promptly_busy_and_release_preserves_history() {
    if std::env::var_os("KIS_MARKET_STREAM_STATE_LOCK_CHILD").is_some() {
        let root = std::env::var_os("KIS_MARKET_STREAM_STATE_LOCK_ROOT").unwrap();
        let slot =
            Uuid::parse_str(&std::env::var("KIS_MARKET_STREAM_STATE_LOCK_SLOT").unwrap()).unwrap();
        let domain = MarketStreamDomain::for_test(root, slot).unwrap();
        let _lock = domain.test_state_lock().unwrap();
        println!("STATE_LOCK_READY");
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(30));
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    domain.test_append_command_attempt(700).unwrap();
    let state_path = domain.state_directory().join("approval-state-v1.json");
    let before = std::fs::read(&state_path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("child_held_state_anchor_is_promptly_busy_and_release_preserves_history")
        .arg("--nocapture")
        .env("KIS_MARKET_STREAM_STATE_LOCK_CHILD", "1")
        .env("KIS_MARKET_STREAM_STATE_LOCK_ROOT", root.path())
        .env("KIS_MARKET_STREAM_STATE_LOCK_SLOT", slot.to_string())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        if line.contains("STATE_LOCK_READY") {
            break;
        }
    }
    let started = Instant::now();
    assert_eq!(
        domain.test_append_command_attempt(701).unwrap_err(),
        StateError::LockBusy
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(before, std::fs::read(&state_path).unwrap());
    child.kill().unwrap();
    let _ = child.wait();
    domain.test_append_command_attempt(702).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(
        json["command_attempts_ms"].as_array().unwrap(),
        &[serde_json::json!(700), serde_json::json!(702)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clean_close_in_child_preserves_new_process_reconnect_spacing() {
    if std::env::var_os("KIS_MARKET_STREAM_CLEAN_CHILD").is_some() {
        let root = std::env::var_os("KIS_MARKET_STREAM_CLEAN_ROOT").unwrap();
        let slot =
            Uuid::parse_str(&std::env::var("KIS_MARKET_STREAM_CLEAN_SLOT").unwrap()).unwrap();
        let approval_url = std::env::var("KIS_MARKET_STREAM_CLEAN_APPROVAL").unwrap();
        let ws_url = std::env::var("KIS_MARKET_STREAM_CLEAN_WS").unwrap();
        let domain = MarketStreamDomain::for_test(root, slot).unwrap();
        let approval = ApprovalClient::for_loopback(
            &approval_url,
            Secret::new(APPKEY_SENTINEL.to_owned()),
            Secret::new(SECRET_SENTINEL.to_owned()),
            domain,
            "clean-child",
        )
        .unwrap();
        let session = kis_client::MarketStreamClient::new(
            MarketStreamConfig::loopback(approval, &ws_url).unwrap(),
        )
        .connect()
        .await
        .unwrap();
        session.close().await.unwrap();
        println!("CLEAN_CLOSED");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_idle_ws_server(ws_listener));
    let approval_url = format!("http://127.0.0.1:{approval_port}");
    let ws_url = format!("ws://127.0.0.1:{ws_port}/tryitout");
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("clean_close_in_child_preserves_new_process_reconnect_spacing")
        .arg("--nocapture")
        .env("KIS_MARKET_STREAM_CLEAN_CHILD", "1")
        .env("KIS_MARKET_STREAM_CLEAN_ROOT", root.path())
        .env("KIS_MARKET_STREAM_CLEAN_SLOT", slot.to_string())
        .env("KIS_MARKET_STREAM_CLEAN_APPROVAL", &approval_url)
        .env("KIS_MARKET_STREAM_CLEAN_WS", &ws_url)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("CLEAN_CLOSED"));

    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let approval = ApprovalClient::for_loopback(
        &approval_url,
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        domain,
        "clean-child",
    )
    .unwrap();
    let error = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &ws_url).unwrap(),
    )
    .connect()
    .await
    .unwrap_err();
    assert_eq!(
        error,
        MarketStreamError::State(StateError::ReconnectNotReady)
    );
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_approval_rejects_before_eof_at_the_eight_kib_bound() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_headers(&mut stream).await;
        let length = header_value(&request, "content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).await.unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2328\r\n",
            )
            .await
            .unwrap();
        stream.write_all(&vec![b'x'; 9_000]).await.unwrap();
        stream.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        stream.write_all(b"\r\n0\r\n\r\n").await.unwrap();
    });
    let client = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "approval-cap",
    )
    .unwrap();
    let started = Instant::now();
    let result = client.test_key_for_connection().await;
    assert_eq!(result.unwrap_err(), ApprovalError::ResponseInvalid);
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "the client rejected after crossing 8 KiB without waiting for EOF"
    );
    assert_eq!(
        client.test_key_for_connection().await.unwrap_err(),
        ApprovalError::OutcomeAmbiguous,
        "an HTTP-200 body that cannot be trusted must not be reissued automatically"
    );
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_dispatch_failure_is_ambiguous_and_never_automatically_reissued() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_headers(&mut stream).await;
        let length = header_value(&request, "content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    let client = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "ambiguous-approval",
    )
    .unwrap();
    assert_eq!(
        client.test_key_for_connection().await.unwrap_err(),
        ApprovalError::TransportAmbiguous
    );
    assert_eq!(
        client.test_key_for_connection().await.unwrap_err(),
        ApprovalError::OutcomeAmbiguous
    );
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_ack_closes_epoch_and_cannot_publish_later_data() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_duplicate_ack_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "duplicate-ack",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_ack_timeout(Duration::from_millis(500));
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    assert_eq!(
        session.subscribe("005930").await.unwrap_err(),
        MarketStreamError::DuplicateAck,
        "the final bounded scan must reject an ACK buffered behind the first ACK"
    );
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert!(
        !read_stream_state(&state)["pending_command"].is_null(),
        "same-write duplicate ACK prevents proof and retains the durable reservation"
    );
    assert!(session.is_closed());
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packed_receipts_never_escape_a_buffered_poison_message() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_packed_data_then_duplicate_ack_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "packed-poison",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_ack_timeout(Duration::from_millis(500));
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    session.subscribe("005930").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::DuplicateAck
    );
    assert!(session.is_closed());
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::Closed
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_drain_bound_fails_closed_before_a_ninth_complete_message() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_data_then_nine_controls_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "bounded-drain",
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::Protocol
    );
    assert!(session.is_closed());
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fragmented_message_limit_is_64_on_blocking_and_buffered_paths() {
    for frame_count in [64usize, 65] {
        let (mut session, directory, approval_task, ws_task) =
            session_with_fragmented_ack(frame_count).await;
        let result = tokio::time::timeout(Duration::from_secs(1), session.subscribe("005930"))
            .await
            .expect("fragment budget must bound blocking decoder work");
        if frame_count == 64 {
            result.expect("64-frame fragmented ACK must succeed");
            assert_eq!(
                session.subscribed_symbols().collect::<Vec<_>>(),
                vec!["005930"]
            );
        } else {
            assert_eq!(result.unwrap_err(), MarketStreamError::FrameInvalid);
            assert!(session.is_closed());
            let state: serde_json::Value = serde_json::from_slice(
                &std::fs::read(
                    directory
                        .path()
                        .join("kis-market-stream/approval-state-v1.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert!(!state["pending_command"].is_null());
        }
        drop(session);
        approval_task.await.unwrap();
        ws_task.await.unwrap();
    }

    for frame_count in [64usize, 65] {
        let (mut session, _directory, approval_task, ws_task) =
            session_with_buffered_fragmented_data(frame_count).await;
        let result = tokio::time::timeout(Duration::from_secs(1), session.next_event())
            .await
            .expect("fragment budget must bound nonblocking decoder work");
        if frame_count == 64 {
            let MarketStreamEvent::Receipt(first) = result.unwrap() else {
                panic!("first queued receipt must survive a 64-frame following message")
            };
            let MarketStreamEvent::Receipt(second) = session.next_event().await.unwrap() else {
                panic!("completed 64-frame following message must be queued")
            };
            assert_eq!(first.observation().trade_volume, 10);
            assert_eq!(second.observation().trade_volume, 11);
        } else {
            assert_eq!(result.unwrap_err(), MarketStreamError::FrameInvalid);
            assert!(session.is_closed());
            assert_eq!(
                session.next_event().await.unwrap_err(),
                MarketStreamError::Closed
            );
        }
        drop(session);
        approval_task.await.unwrap();
        ws_task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acked_symbol_data_survives_unrelated_pending_command_without_retrospective_admission() {
    const MIDNIGHT: i64 = 1_789_916_400_000;
    let clock = Arc::new(TestClock::at(MIDNIGHT + (10 * 3600 + 1) * 1_000));
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_two_symbol_pending_data_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "per-symbol-pending",
    )
    .unwrap()
    .with_clock(clock.clone());
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_clock(clock.clone()),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    clock.advance_ms(1_000);
    session.subscribe("000660").await.unwrap();

    let MarketStreamEvent::Receipt(first) = session.next_event().await.unwrap() else {
        panic!("already-ACKed symbol must survive unrelated pending command")
    };
    assert_eq!(first.observation().symbol, "005930");
    assert_eq!(first.receive_ordinal(), 1);
    assert!(matches!(
        session.next_event().await.unwrap(),
        MarketStreamEvent::Status {
            code: kis_client::market_stream::StreamStatusCode::DataBeforeAck,
            ..
        }
    ));
    let MarketStreamEvent::Receipt(second) = session.next_event().await.unwrap() else {
        panic!("post-ACK second-symbol data must be admitted")
    };
    assert_eq!(second.observation().symbol, "000660");
    assert_eq!(second.receive_ordinal(), 2);
    assert_eq!(second.observation().trade_volume, 12);
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_malformed_wrong_and_late_ack_variants_poison_before_receipts() {
    for (name, acknowledgement) in [
        ("buffered-malformed", br#"{"header":{}}"#.to_vec()),
        ("buffered-wrong-symbol", ack("000660", "SUBSCRIBE SUCCESS")),
        ("buffered-late", ack("005930", "SUBSCRIBE SUCCESS")),
    ] {
        assert_buffered_text_poison(acknowledgement, name).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_pingpong_and_incomplete_following_frame_preserve_prior_receipts() {
    let (mut session, _directory, approval_task, ws_task) =
        session_with_buffered_followup(br#"{"header":{"tr_id":"PINGPONG"}}"#.to_vec(), false).await;
    let first = session.next_event().await.unwrap();
    let second = session.next_event().await.unwrap();
    let heartbeat = session.next_event().await.unwrap();
    assert!(matches!(first, MarketStreamEvent::Receipt(_)));
    assert!(matches!(second, MarketStreamEvent::Receipt(_)));
    assert!(matches!(heartbeat, MarketStreamEvent::ApplicationHeartbeat));
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();

    let (mut session, _directory, approval_task, ws_task) =
        session_with_buffered_followup(Vec::new(), true).await;
    assert!(matches!(
        session.next_event().await.unwrap(),
        MarketStreamEvent::Receipt(_)
    ));
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_normalized_equality_only_suppresses_true_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_distinct_then_duplicate_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "normalized-equality",
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    let first = session.next_event().await.unwrap();
    let second = session.next_event().await.unwrap();
    let duplicate = session.next_event().await.unwrap();
    let regression = session.next_event().await.unwrap();
    let (MarketStreamEvent::Receipt(first), MarketStreamEvent::Receipt(second)) = (first, second)
    else {
        panic!("distinct normalized observations must both produce receipts")
    };
    assert_eq!(first.receive_ordinal(), 1);
    assert_eq!(second.receive_ordinal(), 2);
    assert_eq!(first.observation().trade_volume, 10);
    assert_eq!(second.observation().trade_volume, 999);
    assert!(matches!(
        duplicate,
        MarketStreamEvent::Status {
            code: kis_client::market_stream::StreamStatusCode::DuplicateObservation,
            ..
        }
    ));
    assert!(matches!(
        regression,
        MarketStreamEvent::Status {
            code: kis_client::market_stream::StreamStatusCode::StaleObservation,
            ..
        }
    ));
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_close_variants_poison_without_clearing_durable_epoch() {
    for (name, payload) in [
        ("one-byte", vec![0]),
        ("reserved-code", 1005u16.to_be_bytes().to_vec()),
        (
            "invalid-utf8",
            [1000u16.to_be_bytes().as_slice(), &[0xff]].concat(),
        ),
    ] {
        let (error, state) = run_close_case(payload, true, name).await;
        assert_eq!(error, MarketStreamError::FrameInvalid, "{name}");
        assert!(!state["current_epoch"].is_null(), "{name}");
    }
    let (error, state) = run_pending_malformed_close_case().await;
    assert_eq!(error, MarketStreamError::FrameInvalid);
    assert!(!state["current_epoch"].is_null());
    assert!(!state["pending_command"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_empty_and_normal_close_are_clean_but_fragmented_control_is_not() {
    for (name, payload) in [
        ("empty", Vec::new()),
        ("normal", 1000u16.to_be_bytes().to_vec()),
    ] {
        let (error, state) = run_close_case(payload, true, name).await;
        assert_eq!(error, MarketStreamError::Closed, "{name}");
        assert!(state["current_epoch"].is_null(), "{name}");
    }

    let (error, state) = run_close_case(1000u16.to_be_bytes().to_vec(), false, "fragmented").await;
    assert_eq!(error, MarketStreamError::FrameInvalid);
    assert!(!state["current_epoch"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_receipt_session_edges_and_kst_rollover_are_half_open() {
    const MIDNIGHT: i64 = 1_789_916_400_000;
    let before = MIDNIGHT + (15 * 3600 + 29 * 60 + 59) * 1_000 + 999;
    let at = MIDNIGHT + (15 * 3600 + 30 * 60) * 1_000;
    let accepted = run_session_edge_case(before, 90000, 153000, "152959").await;
    let MarketStreamEvent::Receipt(receipt) = accepted.unwrap() else {
        panic!("before-end observation must be a receipt")
    };
    assert_eq!(receipt.received_at_ms(), before);
    assert_eq!(
        run_session_edge_case(at, 90000, 153000, "152959")
            .await
            .unwrap_err(),
        MarketStreamError::Wire(kis_client::WireError::OutsideSessionWindow)
    );
    assert_eq!(
        run_session_edge_case(at + 1, 90000, 153000, "152959")
            .await
            .unwrap_err(),
        MarketStreamError::Wire(kis_client::WireError::OutsideSessionWindow)
    );
    assert_eq!(
        run_session_edge_case(MIDNIGHT + 86_400_000, 0, 240000, "235959")
            .await
            .unwrap_err(),
        MarketStreamError::Wire(kis_client::WireError::OutsideSessionWindow)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packed_receipts_keep_one_immutable_frame_timestamp() {
    const MIDNIGHT: i64 = 1_789_916_400_000;
    let received_at = MIDNIGHT + (10 * 3600 + 1) * 1_000;
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(received_at));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_packed_data_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "packed-timestamp",
    )
    .unwrap()
    .with_clock(clock.clone());
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_clock(clock.clone()),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    let mut frame_monotonic = None;
    for ordinal in 1..=4 {
        let MarketStreamEvent::Receipt(receipt) = session.next_event().await.unwrap() else {
            panic!("packed record {ordinal} must be a receipt")
        };
        assert_eq!(receipt.receive_ordinal(), ordinal);
        assert_eq!(receipt.received_at_ms(), received_at);
        if let Some(expected) = frame_monotonic {
            assert_eq!(receipt.received_monotonic(), expected);
        } else {
            frame_monotonic = Some(receipt.received_monotonic());
        }
        clock.advance_ms(60_000);
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_wire_rejection_matrix_rolls_back_whole_messages() {
    let mut legacy = market_fields(10);
    legacy.pop();
    let mut overfull = market_fields(10);
    overfull.push("x".into());
    let mut overlong = market_fields(10);
    overlong[6] = "x".repeat(65);
    let mut malformed_second = market_fields(11);
    malformed_second[2] = "0".into();
    let mut packed = market_fields(10);
    packed.extend(malformed_second);
    let cases = vec![
        (
            "legacy-46",
            format!("0|H0STCNT0|001|{}", legacy.join("^")).into_bytes(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::FieldCountMismatch),
        ),
        (
            "overfull-48",
            format!("0|H0STCNT0|001|{}", overfull.join("^")).into_bytes(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::FieldCountMismatch),
        ),
        (
            "bad-count",
            String::from_utf8(market_message())
                .unwrap()
                .replacen("|001|", "|002|", 1)
                .into_bytes(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::FieldCountMismatch),
        ),
        (
            "extra-pipe",
            [market_message(), b"|extra".to_vec()].concat(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::UnsupportedMessage),
        ),
        (
            "newline",
            [b"\n".as_slice(), market_message().as_slice()].concat(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::ControlCharacter),
        ),
        (
            "invalid-utf8",
            vec![0xff],
            0x1,
            MarketStreamError::Wire(kis_client::WireError::InvalidUtf8),
        ),
        (
            "overlong-field",
            format!("0|H0STCNT0|001|{}", overlong.join("^")).into_bytes(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::FieldTooLarge),
        ),
        (
            "whole-message-rollback",
            format!("0|H0STCNT0|002|{}", packed.join("^")).into_bytes(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::NumericInvalid),
        ),
        ("binary", market_message(), 0x2, MarketStreamError::Protocol),
        (
            "encrypted-prefix",
            String::from_utf8(market_message())
                .unwrap()
                .replacen("0|", "1|", 1)
                .into_bytes(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::UnsupportedMessage),
        ),
        (
            "account-tr",
            String::from_utf8(market_message())
                .unwrap()
                .replacen("H0STCNT0", "H0STCNI0", 1)
                .into_bytes(),
            0x1,
            MarketStreamError::Wire(kis_client::WireError::UnsupportedMessage),
        ),
        (
            "oversized-control",
            vec![b'x'; 126],
            0x9,
            MarketStreamError::FrameInvalid,
        ),
    ];
    for (name, payload, opcode, expected) in cases {
        let error = run_wire_case(payload, opcode, false, name).await;
        assert_eq!(error, expected, "{name}");
    }

    let oversized = vec![b'x'; 256 * 1024 + 1];
    assert_eq!(
        run_wire_case(oversized, 0x1, true, "fragmented-oversize").await,
        MarketStreamError::FrameInvalid
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_wrong_operation_missing_and_late_ack_poison_without_retry() {
    let (malformed, malformed_state) = run_ack_case(
        Some(br#"{"header":{}}"#.to_vec()),
        Duration::ZERO,
        "malformed-ack",
    )
    .await;
    assert_eq!(malformed, MarketStreamError::AckMismatch);
    assert!(!malformed_state["pending_command"].is_null());

    let (wrong_operation, wrong_operation_state) = run_ack_case(
        Some(ack("005930", "UNSUBSCRIBE SUCCESS")),
        Duration::ZERO,
        "wrong-operation-ack",
    )
    .await;
    assert_eq!(wrong_operation, MarketStreamError::AckMismatch);
    assert!(!wrong_operation_state["pending_command"].is_null());

    let (rejected, rejected_state) =
        run_ack_case(Some(rejected_ack("005930")), Duration::ZERO, "rejected-ack").await;
    assert_eq!(rejected, MarketStreamError::CommandRejected);
    assert!(!rejected_state["pending_command"].is_null());
    let (missing, missing_state) = run_ack_case(None, Duration::ZERO, "missing-ack").await;
    assert_eq!(missing, MarketStreamError::AckTimeout);
    assert!(!missing_state["pending_command"].is_null());
    let (late, late_state) = run_ack_case(
        Some(ack("005930", "SUBSCRIBE SUCCESS")),
        Duration::from_millis(75),
        "late-ack",
    )
    .await;
    assert_eq!(late, MarketStreamError::AckTimeout);
    assert!(!late_state["pending_command"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_numeric_message_is_rejected_before_any_receipt_effect() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_malformed_numeric_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "numeric-bound",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap();
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    assert_eq!(
        session.subscribe("005930").await.unwrap_err(),
        MarketStreamError::Wire(kis_client::WireError::NumericInvalid)
    );
    assert!(session.is_closed());
    assert!(
        !read_stream_state(&state)["pending_command"].is_null(),
        "poison found in the post-ACK scan retains command ambiguity"
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn masked_server_frame_and_cancelled_write_poison_the_real_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_masked_ack_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "masked-frame",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_ack_timeout(Duration::from_millis(500));
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    assert_eq!(
        session.subscribe("005930").await.unwrap_err(),
        MarketStreamError::FrameInvalid
    );
    assert!(session.is_closed());
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();

    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_command_sink_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "cancelled-write",
    )
    .unwrap();
    let barrier = Arc::new(Notify::new());
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_ack_timeout(Duration::from_secs(5))
            .with_test_write_barrier(barrier);
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    let timed_out = tokio::time::timeout(Duration::from_millis(100), session.subscribe("005930"))
        .await
        .is_err();
    assert!(
        timed_out,
        "the test barrier cancelled an in-flight socket write"
    );
    assert!(session.is_closed(), "cancelled write poisoned the epoch");
    assert!(
        !read_stream_state(&state)["pending_command"].is_null(),
        "cancelled guarded write retains durable ambiguity"
    );
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_default_requires_typed_session_proof_and_exact30_rejects_unknown_code() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval = ApprovalClient::for_loopback(
        "http://127.0.0.1:1",
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "proof-required",
    )
    .unwrap();
    let error = kis_client::MarketStreamClient::new(MarketStreamConfig::new(approval).unwrap())
        .connect()
        .await
        .unwrap_err();
    assert_eq!(error, MarketStreamError::SessionProofRequired);

    // The exact30 check is exercised at the public command boundary by an
    // actual connected loopback session in the existing idle-server fixture.
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_idle_ws_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "exact30",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap();
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    assert_eq!(
        session.subscribe("123456").await.unwrap_err(),
        MarketStreamError::SymbolNotAllowed
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forbidden_websocket_endpoint_is_rejected_before_any_network_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let approval = ApprovalClient::for_loopback(
        "http://127.0.0.1:1",
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "forbidden-endpoint",
    )
    .unwrap();
    assert_eq!(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{port}/account"))
            .unwrap_err(),
        MarketStreamError::EndpointNotAllowed
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err(),
        "a forbidden endpoint must be rejected before TCP bytes"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_child_kill_leaves_uncertain_epoch_and_blocks_direct_restart() {
    if std::env::var_os("KIS_MARKET_STREAM_CONNECTED_CHILD").is_some() {
        let path = std::env::var_os("KIS_MARKET_STREAM_CHILD_STATE").unwrap();
        let slot =
            Uuid::parse_str(&std::env::var("KIS_MARKET_STREAM_CHILD_SLOT").unwrap()).unwrap();
        let approval_url = std::env::var("KIS_MARKET_STREAM_CHILD_APPROVAL").unwrap();
        let ws_url = std::env::var("KIS_MARKET_STREAM_CHILD_WS").unwrap();
        let state = MarketStreamDomain::for_test(path, slot).unwrap();
        let approval = ApprovalClient::for_loopback(
            &approval_url,
            Secret::new(APPKEY_SENTINEL.to_owned()),
            Secret::new(SECRET_SENTINEL.to_owned()),
            state.clone(),
            "connected-child",
        )
        .unwrap();
        let config = MarketStreamConfig::loopback(approval, &ws_url).unwrap();
        let mut session = kis_client::MarketStreamClient::new(config)
            .connect()
            .await
            .unwrap();
        println!("SOCKET_CONNECTED");
        std::io::stdout().flush().unwrap();
        tokio::spawn(async move {
            let _ = session.subscribe("005930").await;
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        println!("COMMAND_PENDING");
        std::io::stdout().flush().unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let slot = Uuid::new_v4();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_hold_ws_server(ws_listener));
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("connected_child_kill_leaves_uncertain_epoch_and_blocks_direct_restart")
        .arg("--nocapture")
        .env("KIS_MARKET_STREAM_CONNECTED_CHILD", "1")
        .env("KIS_MARKET_STREAM_CHILD_STATE", &path)
        .env("KIS_MARKET_STREAM_CHILD_SLOT", slot.to_string())
        .env(
            "KIS_MARKET_STREAM_CHILD_APPROVAL",
            format!("http://127.0.0.1:{approval_port}"),
        )
        .env(
            "KIS_MARKET_STREAM_CHILD_WS",
            format!("ws://127.0.0.1:{ws_port}/tryitout"),
        )
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut connected = false;
    let mut command_pending = false;
    for _ in 0..12 {
        line.clear();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if line.contains("SOCKET_CONNECTED") {
            connected = true;
        }
        if line.contains("COMMAND_PENDING") {
            command_pending = true;
            break;
        }
    }
    assert!(connected, "child did not own a connected loopback socket");
    assert!(command_pending, "child did not persist a pending command");
    child.kill().unwrap();
    let _ = child.wait();

    let state = MarketStreamDomain::for_test(directory.path(), slot).unwrap();
    let durable_bytes =
        std::fs::read(state.state_directory().join("approval-state-v1.json")).unwrap();
    assert!(String::from_utf8_lossy(&durable_bytes).contains("pending_command"));
    assert!(String::from_utf8_lossy(&durable_bytes).contains("005930"));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "connected-child",
    )
    .unwrap();
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap();
    let error = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MarketStreamError::State(StateError::PriorSessionUncertain)
    ));
    assert!(
        state.test_lock().is_ok(),
        "child crash released the inode lock"
    );
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribe_uses_tr_type_two_and_forbids_same_epoch_resubscribe() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(1_000_000));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_subscribe_unsubscribe_server(ws_listener));
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "unsubscribe",
    )
    .unwrap()
    .with_clock(clock.clone());
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_clock(clock.clone());
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    let subscribe = session.prepare_subscribe("005930").unwrap().unwrap();
    let subscribe_ack = session.send_prepared(subscribe).await.unwrap();
    assert_eq!(
        subscribe_ack.operation(),
        MarketSubscriptionOperation::Subscribe
    );
    assert_eq!(subscribe_ack.ordinal(), 1);
    clock.advance_ms(1_000);
    let unsubscribe = session.prepare_unsubscribe("005930").unwrap().unwrap();
    let unsubscribe_ack = session.send_prepared(unsubscribe).await.unwrap();
    assert_eq!(
        unsubscribe_ack.operation(),
        MarketSubscriptionOperation::Unsubscribe
    );
    assert_eq!(unsubscribe_ack.ordinal(), 2);
    assert_eq!(session.subscribed_symbols().count(), 0);
    assert_eq!(
        session.subscribe("005930").await.unwrap_err(),
        MarketStreamError::ResubscribeRequiresNewEpoch
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_unsubscribe_ack_suppresses_target_and_keeps_unrelated_receipt() {
    const FIXTURE_NOW: i64 = 1_789_916_400_000 + 10 * 60 * 60 * 1_000;
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(FIXTURE_NOW));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(run_unsubscribe_with_packed_data_server(ws_listener));
    let client = synthetic_client(
        state,
        approval_port,
        ws_port,
        clock.clone(),
        Duration::from_millis(500),
    );
    let mut session = client.connect().await.unwrap();

    for (ordinal, symbol) in [(1, "005930"), (2, "000660")] {
        let prepared = session.prepare_subscribe(symbol).unwrap().unwrap();
        let ack = session.send_prepared(prepared).await.unwrap();
        assert_eq!(ack.operation(), MarketSubscriptionOperation::Subscribe);
        assert_eq!(ack.symbol(), symbol);
        assert_eq!(ack.ordinal(), ordinal);
        clock.advance_ms(1_000);
    }

    let prepared = session.prepare_unsubscribe("005930").unwrap().unwrap();
    let unsubscribe_ack = session.send_prepared(prepared).await.unwrap();
    assert_eq!(
        unsubscribe_ack.operation(),
        MarketSubscriptionOperation::Unsubscribe
    );
    assert_eq!(unsubscribe_ack.symbol(), "005930");
    assert_eq!(unsubscribe_ack.ordinal(), 3);
    assert_eq!(
        session.subscribed_symbols().collect::<Vec<_>>(),
        vec!["000660"]
    );

    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), session.next_event())
            .await
            .unwrap()
            .unwrap(),
        MarketStreamEvent::Status {
            code: kis_client::market_stream::StreamStatusCode::DataBeforeAck,
            ..
        }
    ));
    let MarketStreamEvent::Receipt(unrelated) =
        tokio::time::timeout(Duration::from_secs(1), session.next_event())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("already-ACKed unrelated data should remain admitted after unsubscribe")
    };
    assert_eq!(unrelated.observation().symbol, "000660");
    assert_eq!(unrelated.receive_ordinal(), 1);
    assert_eq!(unrelated.received_at_ms(), FIXTURE_NOW + 2_000);
    assert!(unrelated.received_monotonic() >= unsubscribe_ack.ack_received_monotonic());

    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_command_persists_identity_before_wire_and_returns_opaque_ack() {
    const FIXTURE_NOW: i64 = 1_789_916_400_000 + 10 * 60 * 60 * 1_000;
    let directory = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let state = MarketStreamDomain::for_test(directory.path(), slot).unwrap();
    let clock = Arc::new(TestClock::at(FIXTURE_NOW));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (quiet_tx, quiet_rx) = oneshot::channel();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let mut byte = [0u8; 1];
        let result = tokio::time::timeout(Duration::from_millis(60), stream.read(&mut byte)).await;
        let no_early_bytes = matches!(result, Err(_) | Ok(Ok(0)));
        quiet_tx.send(no_early_bytes).unwrap();
        let (opcode, command) = read_server_frame(&mut stream).await;
        assert_eq!(opcode, 0x1);
        let value: serde_json::Value = serde_json::from_slice(&command).unwrap();
        assert_eq!(value["header"]["tr_type"], "1");
        write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
        tokio::time::sleep(Duration::from_millis(120)).await;
    });
    let client = synthetic_client(
        state.clone(),
        approval_port,
        ws_port,
        clock,
        Duration::from_millis(500),
    );
    let mut session = client.connect().await.unwrap();
    let before = read_stream_state(&state);
    assert!(session.prepare_unsubscribe("005930").unwrap().is_none());
    assert_eq!(read_stream_state(&state), before, "no-op consumes no state");

    let prepared = session.prepare_subscribe("005930").unwrap().unwrap();
    assert_eq!(prepared.credential_slot_id(), slot);
    assert_eq!(prepared.epoch(), session.epoch());
    assert_eq!(prepared.symbol(), "005930");
    assert_eq!(prepared.operation(), MarketSubscriptionOperation::Subscribe);
    assert_eq!(prepared.ordinal(), 1);
    assert_eq!(prepared.reserved_at_ms(), FIXTURE_NOW);
    assert_eq!(prepared.deadline_at_ms(), FIXTURE_NOW + 500);
    assert!(prepared.deadline_monotonic() > prepared.reserved_at_monotonic());
    let reserved_state = read_stream_state(&state);
    assert_eq!(
        reserved_state["command_attempts_ms"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(reserved_state["next_command_ordinal"], 1);
    assert_eq!(reserved_state["pending_command"]["ordinal"], 1);
    assert_eq!(reserved_state["pending_command"]["sent_at_ms"], FIXTURE_NOW);
    assert_eq!(
        reserved_state["pending_command"]["deadline_ms"],
        FIXTURE_NOW + 500
    );
    assert!(
        quiet_rx.await.unwrap(),
        "prepare wrote no subscription bytes"
    );

    let ack = session.send_prepared(prepared).await.unwrap();
    assert_eq!(ack.credential_slot_id(), slot);
    assert_eq!(ack.epoch(), session.epoch());
    assert_eq!(ack.symbol(), "005930");
    assert_eq!(ack.operation(), MarketSubscriptionOperation::Subscribe);
    assert_eq!(ack.ordinal(), 1);
    assert_eq!(ack.reserved_at_ms(), FIXTURE_NOW);
    assert_eq!(ack.deadline_at_ms(), FIXTURE_NOW + 500);
    assert!(ack.ack_received_at_ms() >= ack.reserved_at_ms());
    assert!(ack.ack_received_at_ms() < ack.deadline_at_ms());
    assert!(ack.ack_received_monotonic() >= ack.reserved_at_monotonic());
    assert!(ack.ack_received_monotonic() < ack.deadline_monotonic());
    let committed_state = read_stream_state(&state);
    assert!(committed_state["pending_command"].is_null());
    assert_eq!(
        committed_state["command_attempts_ms"],
        reserved_state["command_attempts_ms"]
    );
    assert_eq!(committed_state["next_command_ordinal"], 1);
    assert_eq!(
        session.subscribed_symbols().collect::<Vec<_>>(),
        vec!["005930"]
    );

    assert!(session.prepare_subscribe("005930").unwrap().is_none());
    assert_eq!(
        read_stream_state(&state),
        committed_state,
        "ACKed no-op is budget-neutral"
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_deadline_delay_fails_without_bytes_and_retains_restart_ambiguity() {
    const FIXTURE_NOW: i64 = 1_789_916_400_000 + 10 * 60 * 60 * 1_000;
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(FIXTURE_NOW));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (quiet_tx, quiet_rx) = oneshot::channel();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let mut byte = [0u8; 1];
        let result = tokio::time::timeout(Duration::from_millis(250), stream.read(&mut byte)).await;
        quiet_tx.send(matches!(result, Err(_) | Ok(Ok(0)))).unwrap();
    });
    let client = synthetic_client(
        state.clone(),
        approval_port,
        ws_port,
        clock.clone(),
        Duration::from_millis(30),
    );
    let mut session = client.connect().await.unwrap();
    let prepared = session.prepare_subscribe("005930").unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(45)).await;
    clock.advance_ms(45);
    assert_eq!(
        session.send_prepared(prepared).await.unwrap_err(),
        MarketStreamError::AckTimeout
    );
    assert!(
        quiet_rx.await.unwrap(),
        "expired preparation wrote no frame"
    );
    assert!(session.is_closed());
    let state_after = read_stream_state(&state);
    assert!(!state_after["pending_command"].is_null());
    assert_eq!(state_after["next_command_ordinal"], 1);
    drop(session);
    assert_eq!(
        client.connect().await.unwrap_err(),
        MarketStreamError::State(StateError::PriorSessionUncertain)
    );
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_token_cannot_cross_sessions_or_resume_after_abandonment() {
    let left_dir = tempfile::tempdir().unwrap();
    let right_dir = tempfile::tempdir().unwrap();
    let left_state = MarketStreamDomain::for_test(left_dir.path(), Uuid::new_v4()).unwrap();
    let right_state = MarketStreamDomain::for_test(right_dir.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(1_789_916_400_000 + 10 * 60 * 60 * 1_000));
    let (left_client, left_approval_task, left_ws_task, left_no_bytes_rx) =
        start_idle_loopback(left_state.clone(), clock.clone()).await;
    let (right_client, right_approval_task, right_ws_task, right_no_bytes_rx) =
        start_idle_loopback(right_state.clone(), clock.clone()).await;
    let mut left = left_client.connect().await.unwrap();
    let mut right = right_client.connect().await.unwrap();
    let prepared = left.prepare_subscribe("005930").unwrap().unwrap();
    assert_eq!(
        right.send_prepared(prepared).await.unwrap_err(),
        MarketStreamError::CommandCapabilityMismatch
    );
    assert_eq!(
        left.next_event().await.unwrap_err(),
        MarketStreamError::CommandPending,
        "the source loses the unique moved capability and cannot resume"
    );
    assert!(left.is_closed());
    assert_eq!(
        left.reconnect().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert_eq!(
        left.close().await.unwrap_err(),
        MarketStreamError::CommandPending
    );

    let abandoned = right.prepare_subscribe("005930").unwrap().unwrap();
    drop(abandoned);
    assert_eq!(
        right.close().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert!(!read_stream_state(&left_state)["pending_command"].is_null());
    assert!(!read_stream_state(&right_state)["pending_command"].is_null());
    assert!(left_no_bytes_rx.await.unwrap());
    assert!(right_no_bytes_rx.await.unwrap());
    assert_eq!(
        left_client.connect().await.unwrap_err(),
        MarketStreamError::State(StateError::PriorSessionUncertain)
    );
    left_approval_task.await.unwrap();
    left_ws_task.await.unwrap();
    right_approval_task.await.unwrap();
    right_ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_state_loss_and_invalid_state_poison_without_repair_or_resume() {
    const FIXTURE_NOW: i64 = 1_789_916_400_000 + 10 * 60 * 60 * 1_000;
    const INVALID_STATE_BYTES: &[u8] = b"invalid synthetic market-stream state fixture";

    for state_is_invalid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let clock = Arc::new(TestClock::at(FIXTURE_NOW));
        let (client, approval_task, ws_task, no_bytes_rx) =
            start_idle_loopback(state.clone(), clock).await;
        let mut session = client.connect().await.unwrap();
        let state_path = state.state_directory().join("approval-state-v1.json");

        if state_is_invalid {
            std::fs::write(&state_path, INVALID_STATE_BYTES).unwrap();
        } else {
            std::fs::remove_file(&state_path).unwrap();
        }

        let prepared = session.prepare_subscribe("005930");
        let expected_error = if state_is_invalid {
            MarketStreamError::State(StateError::InvalidState)
        } else {
            MarketStreamError::State(StateError::PriorSessionUncertain)
        };
        assert_eq!(prepared.unwrap_err(), expected_error);
        assert!(
            session.is_closed(),
            "durable state uncertainty closes the socket"
        );
        assert_eq!(
            session.next_event().await.unwrap_err(),
            MarketStreamError::CommandPending,
            "an uncertain prepare cannot resume through event reading"
        );
        assert_eq!(
            session.prepare_subscribe("000660").unwrap_err(),
            MarketStreamError::CommandPending,
            "an uncertain prepare cannot mint another capability"
        );
        assert_eq!(
            session.reconnect().await.unwrap_err(),
            MarketStreamError::CommandPending,
            "an uncertain prepare cannot reconnect or clear its state"
        );
        assert_eq!(
            session.close().await.unwrap_err(),
            MarketStreamError::CommandPending,
            "graceful close cannot clear an uncertain prepare"
        );

        if state_is_invalid {
            assert_eq!(
                std::fs::read(&state_path).unwrap(),
                INVALID_STATE_BYTES,
                "the invalid temporary fixture is not repaired or rewritten"
            );
        } else {
            assert!(
                !state_path.exists(),
                "the missing temporary fixture is not recreated"
            );
        }
        assert!(
            no_bytes_rx.await.unwrap(),
            "failed prepare writes no socket bytes"
        );
        approval_task.await.unwrap();
        ws_task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_command_budget_exhaustion_does_not_poison_or_mutate_state() {
    const FIXTURE_NOW: i64 = 1_789_916_400_000 + 10 * 60 * 60 * 1_000;
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(FIXTURE_NOW));
    state.test_append_command_attempt(FIXTURE_NOW).unwrap();
    let (client, approval_task, ws_task, no_bytes_rx) =
        start_idle_loopback(state.clone(), clock).await;
    let mut session = client.connect().await.unwrap();
    let state_path = state.state_directory().join("approval-state-v1.json");
    let before = std::fs::read(&state_path).unwrap();

    for symbol in ["005930", "000660"] {
        assert!(matches!(
            session.prepare_subscribe(symbol).unwrap_err(),
            MarketStreamError::CommandBudget(_)
        ));
        assert!(
            !session.is_closed(),
            "ordinary budget rejection leaves the idle transport usable"
        );
        assert_eq!(std::fs::read(&state_path).unwrap(), before);
    }

    assert!(
        no_bytes_rx.await.unwrap(),
        "budget rejection writes no socket bytes"
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_reservation_boundary_error_poisons_after_budget_reservation() {
    const FIXTURE_NOW: i64 = 1_789_916_400_000 + 10 * 60 * 60 * 1_000;
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(FIXTURE_NOW));
    let (client, approval_task, ws_task, no_bytes_rx) =
        start_idle_loopback(state.clone(), clock).await;
    let mut session = client.connect().await.unwrap();
    let state_path = state.state_directory().join("approval-state-v1.json");

    let mut fixture = read_stream_state(&state);
    fixture["next_command_ordinal"] = serde_json::json!(u64::MAX - 1);
    std::fs::write(&state_path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    assert_eq!(
        read_stream_state(&state)["next_command_ordinal"].as_u64(),
        Some(u64::MAX - 1),
        "the existing valid upper-bound fixture is accepted before preparation"
    );

    assert_eq!(
        session.prepare_subscribe("005930").unwrap_err(),
        MarketStreamError::State(StateError::InvalidState),
        "incrementing the valid maximum ordinal fails durable validation"
    );
    assert!(session.is_closed());
    let after_reservation_error = std::fs::read(&state_path).unwrap();
    let durable = serde_json::from_slice::<serde_json::Value>(&after_reservation_error).unwrap();
    assert_eq!(durable["next_command_ordinal"].as_u64(), Some(u64::MAX - 1));
    assert!(durable["pending_command"].is_null());
    assert_eq!(
        durable["command_attempts_ms"].as_array().unwrap().len(),
        1,
        "the preceding successful budget reservation remains durable"
    );

    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert_eq!(
        session.prepare_unsubscribe("005930").unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert_eq!(
        session.reconnect().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert_eq!(
        session.close().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert_eq!(std::fs::read(&state_path).unwrap(), after_reservation_error);
    assert!(
        no_bytes_rx.await.unwrap(),
        "reservation error writes no socket bytes"
    );
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn already_buffered_ack_before_write_poisons_without_command_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(1_789_916_400_000 + 10 * 60 * 60 * 1_000));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (ack_sent_tx, ack_sent_rx) = oneshot::channel();
    let (no_bytes_tx, no_bytes_rx) = oneshot::channel();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
        ack_sent_tx.send(()).unwrap();
        let mut byte = [0u8; 1];
        let result = tokio::time::timeout(Duration::from_millis(250), stream.read(&mut byte)).await;
        no_bytes_tx
            .send(matches!(result, Err(_) | Ok(Ok(0))))
            .unwrap();
    });
    let client = synthetic_client(
        state.clone(),
        approval_port,
        ws_port,
        clock,
        Duration::from_millis(500),
    );
    let mut session = client.connect().await.unwrap();
    let prepared = session.prepare_subscribe("005930").unwrap().unwrap();
    ack_sent_rx.await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(
        session.send_prepared(prepared).await.unwrap_err(),
        MarketStreamError::DuplicateAck
    );
    assert!(no_bytes_rx.await.unwrap());
    assert!(session.is_closed());
    assert!(!read_stream_state(&state)["pending_command"].is_null());
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_ack_provisional_admission_keeps_one_frame_capture_until_proof_return() {
    const FIXTURE_NOW: i64 = 1_789_916_400_000 + 10 * 60 * 60 * 1_000;
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(FIXTURE_NOW));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        let mut bytes = server_frame_bytes(0x1, &market_message(), true);
        bytes.extend(server_frame_bytes(
            0x1,
            &ack("005930", "SUBSCRIBE SUCCESS"),
            true,
        ));
        bytes.extend(server_frame_bytes(
            0x1,
            &packed_market_message(&[10, 11]),
            true,
        ));
        stream.write_all(&bytes).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
    });
    let client = synthetic_client(
        state.clone(),
        approval_port,
        ws_port,
        clock,
        Duration::from_millis(500),
    );
    let mut session = client.connect().await.unwrap();
    let prepared = session.prepare_subscribe("005930").unwrap().unwrap();
    let ack = session.send_prepared(prepared).await.unwrap();
    assert_eq!(ack.operation(), MarketSubscriptionOperation::Subscribe);
    assert_eq!(
        session.subscribed_symbols().collect::<Vec<_>>(),
        vec!["005930"]
    );
    assert!(read_stream_state(&state)["pending_command"].is_null());
    assert!(matches!(
        session.next_event().await.unwrap(),
        MarketStreamEvent::Status {
            code: kis_client::market_stream::StreamStatusCode::DataBeforeAck,
            ..
        }
    ));
    let MarketStreamEvent::Receipt(first) = session.next_event().await.unwrap() else {
        panic!("post-ACK target observation should be released after proof return")
    };
    let MarketStreamEvent::Receipt(second) = session.next_event().await.unwrap() else {
        panic!("packed post-ACK observation should preserve its second record")
    };
    assert_eq!(first.receive_ordinal(), 1);
    assert_eq!(second.receive_ordinal(), 2);
    assert_eq!(first.observation().trade_volume, 10);
    assert_eq!(second.observation().trade_volume, 11);
    assert_eq!(first.received_at_ms(), second.received_at_ms());
    assert_eq!(first.received_at_ms(), FIXTURE_NOW);
    assert!(first.received_monotonic() >= ack.ack_received_monotonic());
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_in_pre_send_drain_and_ack_wait_keeps_pending_and_mints_no_ack() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(1_789_916_400_000 + 10 * 60 * 60 * 1_000));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (trigger_tx, trigger_rx) = oneshot::channel();
    let (heartbeat_sent_tx, heartbeat_sent_rx) = oneshot::channel();
    let (pong_seen_tx, pong_seen_rx) = oneshot::channel();
    let (no_command_tx, no_command_rx) = oneshot::channel();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        trigger_rx.await.unwrap();
        write_server_frame(
            &mut stream,
            0x1,
            br#"{"header":{"tr_id":"PINGPONG"}}"#,
            true,
        )
        .await;
        heartbeat_sent_tx.send(()).unwrap();
        let (opcode, payload) = read_server_frame(&mut stream).await;
        assert_eq!(opcode, 0xA, "pre-send application heartbeat gets a Pong");
        assert!(String::from_utf8_lossy(&payload).contains("PINGPONG"));
        pong_seen_tx.send(()).unwrap();
        let mut byte = [0u8; 1];
        let result = tokio::time::timeout(Duration::from_millis(250), stream.read(&mut byte)).await;
        no_command_tx
            .send(matches!(result, Err(_) | Ok(Ok(0))))
            .unwrap();
    });
    let barrier = Arc::new(Notify::new());
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "cancel-pre-send-drain",
    )
    .unwrap()
    .with_clock(clock.clone());
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_clock(clock)
            .with_ack_timeout(Duration::from_secs(2))
            .with_test_write_barrier(barrier);
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    let prepared = session.prepare_subscribe("005930").unwrap().unwrap();
    trigger_tx.send(()).unwrap();
    heartbeat_sent_rx.await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut send_future = Box::pin(session.send_prepared(prepared));
    tokio::select! {
        _ = pong_seen_rx => {}
        result = &mut send_future => panic!("pre-send drain unexpectedly completed: {result:?}"),
    }
    drop(send_future);
    assert!(
        session.is_closed(),
        "cancelled Pong write leaves socket poisoned"
    );
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert!(!read_stream_state(&state)["pending_command"].is_null());
    assert!(
        no_command_rx.await.unwrap(),
        "no subscription frame preceded the drain"
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();

    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(1_789_916_400_000 + 10 * 60 * 60 * 1_000));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (command_seen_tx, command_seen_rx) = oneshot::channel();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let (opcode, _) = read_server_frame(&mut stream).await;
        assert_eq!(opcode, 0x1);
        command_seen_tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
    });
    let client = synthetic_client(
        state.clone(),
        approval_port,
        ws_port,
        clock,
        Duration::from_secs(2),
    );
    let mut session = client.connect().await.unwrap();
    let prepared = session.prepare_subscribe("005930").unwrap().unwrap();
    let mut send_future = Box::pin(session.send_prepared(prepared));
    tokio::select! {
        _ = command_seen_rx => {}
        result = &mut send_future => panic!("ACK wait unexpectedly completed: {result:?}"),
    }
    drop(send_future);
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert!(!read_stream_state(&state)["pending_command"].is_null());
    drop(session);
    assert_eq!(
        client.connect().await.unwrap_err(),
        MarketStreamError::State(StateError::PriorSessionUncertain)
    );
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_in_post_ack_scan_retains_pending_after_valid_ack_capture() {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let clock = Arc::new(TestClock::at(1_789_916_400_000 + 10 * 60 * 60 * 1_000));
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (command_seen_tx, command_seen_rx) = oneshot::channel();
    let (pong_seen_tx, pong_seen_rx) = oneshot::channel();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let (opcode, _) = read_server_frame(&mut stream).await;
        assert_eq!(opcode, 0x1);
        command_seen_tx.send(()).unwrap();
        let mut bytes = server_frame_bytes(0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true);
        bytes.extend(server_frame_bytes(
            0x1,
            br#"{"header":{"tr_id":"PINGPONG"}}"#,
            true,
        ));
        stream.write_all(&bytes).await.unwrap();
        let (opcode, payload) = read_server_frame(&mut stream).await;
        assert_eq!(opcode, 0xA);
        assert!(String::from_utf8_lossy(&payload).contains("PINGPONG"));
        pong_seen_tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let barrier = Arc::new(Notify::new());
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "cancel-post-ack-scan",
    )
    .unwrap()
    .with_clock(clock.clone());
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_clock(clock)
            .with_ack_timeout(Duration::from_secs(2))
            .with_test_write_barrier(barrier.clone());
    let mut session = kis_client::MarketStreamClient::new(config)
        .connect()
        .await
        .unwrap();
    let prepared = session.prepare_subscribe("005930").unwrap().unwrap();
    let mut send_future = Box::pin(session.send_prepared(prepared));
    tokio::select! {
        _ = command_seen_rx => {}
        result = &mut send_future => panic!("command write unexpectedly completed: {result:?}"),
    }
    barrier.notify_one();
    tokio::select! {
        _ = pong_seen_rx => {}
        result = &mut send_future => panic!("post-ACK scan unexpectedly completed: {result:?}"),
    }
    drop(send_future);
    assert!(
        session.is_closed(),
        "cancelled buffered Pong write poisons the socket"
    );
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::CommandPending
    );
    assert!(!read_stream_state(&state)["pending_command"].is_null());
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

async fn run_approval_server(
    listener: TcpListener,
    body_tx: oneshot::Sender<String>,
    approval_key: &'static str,
) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let request = read_http_headers(&mut stream).await;
    let content_length = header_value(&request, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body).await.unwrap();
    let _ = body_tx.send(String::from_utf8(body).unwrap());
    let response_body = format!(r#"{{"approval_key":"{approval_key}"}}"#);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

fn synthetic_client(
    state: MarketStreamDomain,
    approval_port: u16,
    ws_port: u16,
    clock: Arc<TestClock>,
    ack_timeout: Duration,
) -> kis_client::MarketStreamClient {
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "c2-transport-proof",
    )
    .unwrap()
    .with_clock(clock.clone());
    let config =
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_clock(clock)
            .with_ack_timeout(ack_timeout);
    kis_client::MarketStreamClient::new(config)
}

async fn start_idle_loopback(
    state: MarketStreamDomain,
    clock: Arc<TestClock>,
) -> (
    kis_client::MarketStreamClient,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
    oneshot::Receiver<bool>,
) {
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let (no_bytes_tx, no_bytes_rx) = oneshot::channel();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let mut byte = [0u8; 1];
        let result = tokio::time::timeout(Duration::from_millis(250), stream.read(&mut byte)).await;
        no_bytes_tx
            .send(matches!(result, Err(_) | Ok(Ok(0))))
            .unwrap();
    });
    (
        synthetic_client(
            state,
            approval_port,
            ws_port,
            clock,
            Duration::from_millis(500),
        ),
        approval_task,
        ws_task,
        no_bytes_rx,
    )
}

fn read_stream_state(state: &MarketStreamDomain) -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(state.state_directory().join("approval-state-v1.json")).unwrap(),
    )
    .unwrap()
}

async fn run_counting_approval_server(listener: TcpListener, count_tx: oneshot::Sender<usize>) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let request = read_http_headers(&mut stream).await;
    let length = header_value(&request, "content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.unwrap();
    let response_body = r#"{"approval_key":"SYNTHETIC_APPROVAL_KEY"}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    let _ = count_tx.send(1);
}

async fn run_happy_ws_server(listener: TcpListener, command_tx: oneshot::Sender<String>) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let (opcode, command) = read_server_frame(&mut stream).await;
    assert_eq!(opcode, 0x1);
    command_tx
        .send(String::from_utf8(command).unwrap())
        .unwrap();
    let ack = ack("005930", "SUBSCRIBE SUCCESS");
    write_server_frame(&mut stream, 0x1, &ack[..ack.len() / 2], false).await;
    write_server_frame(&mut stream, 0x0, &ack[ack.len() / 2..], true).await;
    let heartbeat = br#"{"header":{"tr_id":"PINGPONG"}}"#;
    write_server_frame(&mut stream, 0x1, heartbeat, true).await;
    let (opcode, _) = read_server_frame(&mut stream).await;
    assert_eq!(opcode, 0xA, "application heartbeat must receive a Pong");
    write_server_frame(&mut stream, 0x9, b"control", true).await;
    let data = market_message();
    write_server_frame_in_partial_reads(&mut stream, 0x1, &data[..data.len() / 2], false).await;
    write_server_frame(&mut stream, 0x0, &data[data.len() / 2..], true).await;
    let (opcode, _) = read_server_frame(&mut stream).await;
    assert_eq!(opcode, 0xA, "control Ping must receive a Pong");
    let _ = tokio::time::timeout(Duration::from_secs(1), read_server_frame(&mut stream)).await;
}

async fn run_wrong_ack_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    let wrong = ack("000660", "SUBSCRIBE SUCCESS");
    write_server_frame(&mut stream, 0x1, &wrong, true).await;
}

async fn run_idle_ws_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let mut byte = [0u8; 1];
    let _ = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte)).await;
}

async fn run_duplicate_ack_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    let ack = ack("005930", "SUBSCRIBE SUCCESS");
    let mut bytes = server_frame_bytes(0x1, &ack, true);
    bytes.extend(server_frame_bytes(0x1, &ack, true));
    bytes.extend(server_frame_bytes(0x1, &market_message(), true));
    stream.write_all(&bytes).await.unwrap();
}

async fn run_packed_data_then_duplicate_ack_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    let acknowledgement = ack("005930", "SUBSCRIBE SUCCESS");
    write_server_frame(&mut stream, 0x1, &acknowledgement, true).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    let data = packed_market_message(&[10, 11]);
    let mut bytes = server_frame_bytes(0x1, &data, true);
    bytes.extend(server_frame_bytes(0x1, &acknowledgement, true));
    stream.write_all(&bytes).await.unwrap();
}

async fn run_data_then_nine_controls_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    let mut bytes = server_frame_bytes(0x1, &market_message(), true);
    for index in 0..9u8 {
        bytes.extend(server_frame_bytes(0x9, &[index], true));
    }
    stream.write_all(&bytes).await.unwrap();
    let mut sink = [0u8; 512];
    let _ = tokio::time::timeout(Duration::from_millis(100), stream.read(&mut sink)).await;
}

async fn assert_buffered_text_poison(followup: Vec<u8>, generation: &'static str) {
    let (mut session, _directory, approval_task, ws_task) =
        session_with_buffered_followup(followup, false).await;
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::DuplicateAck,
        "{generation}"
    );
    assert!(session.is_closed(), "{generation}");
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
}

async fn session_with_fragmented_ack(
    frame_count: usize,
) -> (
    kis_client::market_stream::MarketStreamSession,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        write_fragmented_text_with_control(
            &mut stream,
            &ack("005930", "SUBSCRIBE SUCCESS"),
            frame_count,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "fragmented-ack-limit",
    )
    .unwrap();
    let session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    (session, directory, approval_task, ws_task)
}

async fn session_with_buffered_fragmented_data(
    frame_count: usize,
) -> (
    kis_client::market_stream::MarketStreamSession,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        let mut bytes = server_frame_bytes(0x1, &market_message_with_trade_volume(10), true);
        bytes.extend(fragmented_text_with_control_bytes(
            &market_message_with_trade_volume(11),
            frame_count,
        ));
        stream.write_all(&bytes).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "buffered-fragment-limit",
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    (session, directory, approval_task, ws_task)
}

async fn write_fragmented_text_with_control(
    stream: &mut TcpStream,
    payload: &[u8],
    frame_count: usize,
) {
    assert!(frame_count >= 3);
    let split = payload.len() / 2;
    write_server_frame_in_partial_reads(stream, 0x1, &payload[..split], false).await;
    write_server_frame_in_partial_reads(stream, 0x9, b"fragment-control", true).await;
    for _ in 0..frame_count - 3 {
        write_server_frame_in_partial_reads(stream, 0x0, &[], false).await;
    }
    write_server_frame_in_partial_reads(stream, 0x0, &payload[split..], true).await;
}

fn fragmented_text_with_control_bytes(payload: &[u8], frame_count: usize) -> Vec<u8> {
    assert!(frame_count >= 3);
    let split = payload.len() / 2;
    let mut bytes = server_frame_bytes(0x1, &payload[..split], false);
    bytes.extend(server_frame_bytes(0x9, b"fragment-control", true));
    for _ in 0..frame_count - 3 {
        bytes.extend(server_frame_bytes(0x0, &[], false));
    }
    bytes.extend(server_frame_bytes(0x0, &payload[split..], true));
    bytes
}

async fn session_with_buffered_followup(
    followup: Vec<u8>,
    incomplete_header: bool,
) -> (
    kis_client::market_stream::MarketStreamSession,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        let mut bytes = server_frame_bytes(0x1, &packed_market_message(&[10, 11]), true);
        if incomplete_header {
            bytes.push(0x81);
        } else {
            bytes.extend(server_frame_bytes(0x1, &followup, true));
        }
        stream.write_all(&bytes).await.unwrap();
        if incomplete_header {
            tokio::time::sleep(Duration::from_millis(200)).await;
        } else {
            let mut sink = [0u8; 512];
            let _ = tokio::time::timeout(Duration::from_millis(100), stream.read(&mut sink)).await;
        }
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "buffered-followup",
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    (session, directory, approval_task, ws_task)
}

async fn run_distinct_then_duplicate_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    let first = market_message_with_trade_volume(10);
    let second = market_message_with_trade_volume(999);
    let mut bytes = server_frame_bytes(0x1, &first, true);
    bytes.extend(server_frame_bytes(0x1, &second, true));
    bytes.extend(server_frame_bytes(0x1, &second, true));
    bytes.extend(server_frame_bytes(
        0x1,
        &market_message_at("095959", 1),
        true,
    ));
    stream.write_all(&bytes).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
}

async fn run_packed_data_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    write_server_frame(&mut stream, 0x1, &fixture_four_record_message(), true).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
}

async fn run_session_edge_case(
    received_at_ms: i64,
    session_start: u32,
    session_end: u32,
    provider_time: &'static str,
) -> Result<MarketStreamEvent, MarketStreamError> {
    const MIDNIGHT: i64 = 1_789_916_400_000;
    let provider_seconds = i64::from(provider_time[0..2].parse::<u32>().unwrap()) * 3600
        + i64::from(provider_time[2..4].parse::<u32>().unwrap()) * 60
        + i64::from(provider_time[4..6].parse::<u32>().unwrap());
    let initial_ms = MIDNIGHT + provider_seconds * 1_000;
    let clock = Arc::new(TestClock::at(initial_ms));
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        write_server_frame(
            &mut stream,
            0x1,
            &market_message_at(provider_time, 10),
            true,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        "session-edge",
    )
    .unwrap()
    .with_clock(clock.clone());
    let proof =
        MarketStreamSessionProof::new(20260921, session_start, session_end, MIDNIGHT).unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_session_proof(proof)
            .with_clock(clock.clone()),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    clock.advance_ms(received_at_ms - initial_ms);
    let result = session.next_event().await;
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
    result
}

async fn run_close_case(
    payload: Vec<u8>,
    fin: bool,
    generation: &'static str,
) -> (MarketStreamError, serde_json::Value) {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        let mut packed = server_frame_bytes(0x1, &market_message(), true);
        packed.extend(server_frame_bytes(0x8, &payload, fin));
        stream.write_all(&packed).await.unwrap();
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        generation,
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    let error = match session.next_event().await {
        Ok(MarketStreamEvent::Status {
            code: kis_client::market_stream::StreamStatusCode::Closed,
            ..
        }) => MarketStreamError::Closed,
        Ok(other) => panic!("unexpected close outcome: {other:?}"),
        Err(error) => error,
    };
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
    let bytes = std::fs::read(state.state_directory().join("approval-state-v1.json")).unwrap();
    (error, serde_json::from_slice(&bytes).unwrap())
}

async fn run_wire_case(
    payload: Vec<u8>,
    opcode: u8,
    fragmented: bool,
    generation: &'static str,
) -> MarketStreamError {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        if fragmented {
            let split = payload.len() / 2;
            let first = server_frame_bytes(opcode, &payload[..split], false);
            let second = server_frame_bytes(0x0, &payload[split..], true);
            let _ = stream.write_all(&first).await;
            let _ = stream.write_all(&second).await;
        } else {
            let _ = stream
                .write_all(&server_frame_bytes(opcode, &payload, true))
                .await;
        }
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state,
        generation,
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    session.subscribe("005930").await.unwrap();
    let error = session.next_event().await.unwrap_err();
    assert!(session.is_closed(), "{generation}");
    assert_eq!(
        session.next_event().await.unwrap_err(),
        MarketStreamError::Closed,
        "{generation} leaked a queued event after poison"
    );
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
    error
}

async fn run_ack_case(
    response: Option<Vec<u8>>,
    delay: Duration,
    generation: &'static str,
) -> (MarketStreamError, serde_json::Value) {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        tokio::time::sleep(delay).await;
        if let Some(response) = response {
            let _ = stream
                .write_all(&server_frame_bytes(0x1, &response, true))
                .await;
        } else {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        generation,
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap()
            .with_ack_timeout(Duration::from_millis(50)),
    )
    .connect()
    .await
    .unwrap();
    let error = session.subscribe("005930").await.unwrap_err();
    assert!(session.is_closed(), "{generation}");
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
    let bytes = std::fs::read(state.state_directory().join("approval-state-v1.json")).unwrap();
    (error, serde_json::from_slice(&bytes).unwrap())
}

async fn run_pending_malformed_close_case() -> (MarketStreamError, serde_json::Value) {
    let directory = tempfile::tempdir().unwrap();
    let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
    let approval_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let approval_port = approval_listener.local_addr().unwrap().port();
    let approval_task = tokio::spawn(run_approval_server(
        approval_listener,
        oneshot::channel().0,
        APPROVAL_SENTINEL,
    ));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_task = tokio::spawn(async move {
        let (mut stream, _) = ws_listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        let _ = stream.write_all(&server_frame_bytes(0x8, &[0], true)).await;
    });
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new(APPKEY_SENTINEL.to_owned()),
        Secret::new(SECRET_SENTINEL.to_owned()),
        state.clone(),
        "pending-malformed-close",
    )
    .unwrap();
    let mut session = kis_client::MarketStreamClient::new(
        MarketStreamConfig::loopback(approval, &format!("ws://127.0.0.1:{ws_port}/tryitout"))
            .unwrap(),
    )
    .connect()
    .await
    .unwrap();
    let error = session.subscribe("005930").await.unwrap_err();
    drop(session);
    approval_task.await.unwrap();
    ws_task.await.unwrap();
    let bytes = std::fs::read(state.state_directory().join("approval-state-v1.json")).unwrap();
    (error, serde_json::from_slice(&bytes).unwrap())
}

async fn run_malformed_numeric_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
    let malformed =
        String::from_utf8(market_message())
            .unwrap()
            .replacen("70000", "12345678901234567890", 1);
    write_server_frame(&mut stream, 0x1, malformed.as_bytes(), true).await;
}

async fn run_masked_ack_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    write_masked_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS")).await;
}

async fn run_command_sink_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
}

async fn run_hold_ws_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let mut byte = [0u8; 1];
    let _ = tokio::time::timeout(Duration::from_secs(30), stream.read(&mut byte)).await;
}

async fn run_unsubscribe_with_packed_data_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    for symbol in ["005930", "000660"] {
        let (_, command) = read_server_frame(&mut stream).await;
        let command: serde_json::Value = serde_json::from_slice(&command).unwrap();
        assert_eq!(command["header"]["tr_type"], "1");
        assert_eq!(command["body"]["input"]["tr_key"], symbol);
        write_server_frame(&mut stream, 0x1, &ack(symbol, "SUBSCRIBE SUCCESS"), true).await;
    }

    let (_, command) = read_server_frame(&mut stream).await;
    let command: serde_json::Value = serde_json::from_slice(&command).unwrap();
    assert_eq!(command["header"]["tr_type"], "2");
    assert_eq!(command["body"]["input"]["tr_key"], "005930");

    let mut fields = market_fields_for("005930", 10, 20);
    fields.extend(market_fields_for("000660", 11, 21));
    let packed_data = format!("0|H0STCNT0|002|{}", fields.join("^")).into_bytes();
    let mut buffered_write = server_frame_bytes(0x1, &ack("005930", "UNSUBSCRIBE SUCCESS"), true);
    buffered_write.extend(server_frame_bytes(0x1, &packed_data, true));
    stream.write_all(&buffered_write).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
}

async fn run_subscribe_unsubscribe_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let (_, first) = read_server_frame(&mut stream).await;
    let first_json: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(first_json["header"]["tr_type"], "1");
    write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
    let (_, second) = read_server_frame(&mut stream).await;
    let second_json: serde_json::Value = serde_json::from_slice(&second).unwrap();
    assert_eq!(second_json["header"]["tr_type"], "2");
    write_server_frame(
        &mut stream,
        0x1,
        &ack("005930", "UNSUBSCRIBE SUCCESS"),
        true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
}

async fn run_two_symbol_pending_data_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    websocket_handshake(&mut stream).await;
    let _ = read_server_frame(&mut stream).await;
    write_server_frame(&mut stream, 0x1, &ack("005930", "SUBSCRIBE SUCCESS"), true).await;
    let _ = read_server_frame(&mut stream).await;
    let first_symbol = market_fields_for("005930", 10, 20);
    let pending_symbol = market_fields_for("000660", 11, 21);
    let mut fields = first_symbol;
    fields.extend(pending_symbol);
    let mixed = format!("0|H0STCNT0|002|{}", fields.join("^")).into_bytes();
    let mut bytes = server_frame_bytes(0x1, &mixed, true);
    bytes.extend(server_frame_bytes(
        0x1,
        &ack("000660", "SUBSCRIBE SUCCESS"),
        true,
    ));
    bytes.extend(server_frame_bytes(
        0x1,
        &market_message_for("000660", 12, 22),
        true,
    ));
    stream.write_all(&bytes).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
}

async fn run_two_epoch_ws_server(listener: TcpListener) {
    for index in 0..2 {
        let (mut stream, _) = listener.accept().await.unwrap();
        websocket_handshake(&mut stream).await;
        let _ = read_server_frame(&mut stream).await;
        let response = ack("005930", "SUBSCRIBE SUCCESS");
        write_server_frame(&mut stream, 0x1, &response, true).await;
        if index == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            write_server_frame(&mut stream, 0x8, &[], true).await;
        } else {
            let _ =
                tokio::time::timeout(Duration::from_secs(1), read_server_frame(&mut stream)).await;
        }
    }
}

fn ack(symbol: &str, msg1: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "header": {"tr_id": "H0STCNT0", "tr_key": symbol, "encrypt": "N"},
        "body": {"rt_cd": "0", "msg_cd": "OPSP0000", "msg1": msg1}
    }))
    .unwrap()
}

fn rejected_ack(symbol: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "header": {"tr_id": "H0STCNT0", "tr_key": symbol, "encrypt": "N"},
        "body": {"rt_cd": "1", "msg_cd": "SYNTHETIC_REJECT", "msg1": "SYNTHETIC"}
    }))
    .unwrap()
}

fn market_message() -> Vec<u8> {
    market_message_with_trade_volume(10)
}

fn market_fields(trade_volume: u64) -> Vec<String> {
    market_fields_for("005930", trade_volume, 20)
}

fn market_fields_for(symbol: &str, trade_volume: u64, cumulative_volume: u64) -> Vec<String> {
    let mut fields = vec![String::new(); 47];
    fields[0] = symbol.into();
    fields[1] = "100001".into();
    fields[2] = "70000".into();
    fields[3] = "2".into();
    fields[4] = "100".into();
    fields[5] = "0.14".into();
    fields[12] = trade_volume.to_string();
    fields[13] = cumulative_volume.to_string();
    fields[33] = "20260921".into();
    fields[34] = "20".into();
    fields[35] = "N".into();
    fields[43] = "0".into();
    fields[46] = "2".into();
    fields
}

fn market_message_for(symbol: &str, trade_volume: u64, cumulative_volume: u64) -> Vec<u8> {
    format!(
        "0|H0STCNT0|001|{}",
        market_fields_for(symbol, trade_volume, cumulative_volume).join("^")
    )
    .into_bytes()
}

fn market_message_with_trade_volume(trade_volume: u64) -> Vec<u8> {
    format!("0|H0STCNT0|001|{}", market_fields(trade_volume).join("^")).into_bytes()
}

fn market_message_at(provider_time: &str, trade_volume: u64) -> Vec<u8> {
    let mut fields = market_fields(trade_volume);
    fields[1] = provider_time.to_owned();
    format!("0|H0STCNT0|001|{}", fields.join("^")).into_bytes()
}

fn packed_market_message(trade_volumes: &[u64]) -> Vec<u8> {
    let fields = trade_volumes
        .iter()
        .flat_map(|volume| market_fields(*volume))
        .collect::<Vec<_>>();
    format!("0|H0STCNT0|{:03}|{}", trade_volumes.len(), fields.join("^")).into_bytes()
}

fn fixture_four_record_message() -> Vec<u8> {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/market_stream_manifest.json")).unwrap();
    manifest["four_record_fixture"]["wire_payload"]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn server_frame_bytes(opcode: u8, payload: &[u8], fin: bool) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push((if fin { 0x80 } else { 0 }) | opcode);
    match payload.len() {
        0..=125 => frame.push(payload.len() as u8),
        126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => {
            frame.push(127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}

async fn websocket_handshake(stream: &mut TcpStream) {
    let request = read_http_headers(stream).await;
    let key = header_value(&request, "sec-websocket-key").unwrap();
    let mut digest = Sha1::new();
    digest.update(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    let accept = base64::engine::general_purpose::STANDARD.encode(digest.digest().bytes());
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

async fn read_server_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let mut length = usize::from(header[1] & 0x7f);
    if length == 126 {
        let mut extended = [0u8; 2];
        stream.read_exact(&mut extended).await.unwrap();
        length = usize::from(u16::from_be_bytes(extended));
    }
    let mut mask = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask).await.unwrap();
    }
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).await.unwrap();
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    (opcode, payload)
}

async fn write_server_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8], fin: bool) {
    let mut frame = vec![(if fin { 0x80 } else { 0 }) | opcode];
    match payload.len() {
        0..=125 => frame.push(payload.len() as u8),
        126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => panic!("synthetic payload too large"),
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame).await.unwrap();
}

async fn write_server_frame_in_partial_reads(
    stream: &mut TcpStream,
    opcode: u8,
    payload: &[u8],
    fin: bool,
) {
    let frame = server_frame_bytes(opcode, payload, fin);
    for chunk in frame.chunks(3) {
        stream.write_all(chunk).await.unwrap();
        tokio::task::yield_now().await;
    }
}

async fn write_masked_server_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) {
    let mask = [0x11u8, 0x22, 0x33, 0x44];
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        0..=125 => frame.push(0x80 | payload.len() as u8),
        126..=65_535 => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => panic!("synthetic payload too large"),
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame).await.unwrap();
}

async fn read_http_headers(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut one = [0u8; 1];
    while bytes.len() < 16 * 1024 {
        stream.read_exact(&mut one).await.unwrap();
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return bytes;
        }
    }
    panic!("synthetic headers too large")
}

fn header_value(request: &[u8], wanted: &str) -> Option<String> {
    let text = String::from_utf8_lossy(request);
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(wanted)
            .then(|| value.trim().to_owned())
    })
}
