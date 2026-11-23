//! The MCP endpoint only answers requests with the right bearer token.

use reading_list::db::Db;
use reading_list::mcp;
use reqwest::StatusCode;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "test-token-0123456789";

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{
    "protocolVersion":"2025-06-18","capabilities":{},
    "clientInfo":{"name":"test","version":"0"}}}"#;

/// Starts the server on a free local port and returns its `/mcp` URL.
async fn start() -> (String, CancellationToken) {
    let db = Db::in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    tokio::spawn(mcp::serve_on(
        listener,
        db,
        TOKEN.to_owned(),
        shutdown.clone(),
    ));
    (format!("http://{addr}/mcp"), shutdown)
}

async fn post(url: &str, auth: Option<&str>) -> StatusCode {
    let mut req = reqwest::Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(INITIALIZE);
    if let Some(auth) = auth {
        req = req.header("Authorization", auth);
    }
    req.send().await.unwrap().status()
}

#[tokio::test]
async fn mcp_rejects_missing_token() {
    let (url, _shutdown) = start().await;
    assert_eq!(post(&url, None).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_rejects_wrong_token() {
    let (url, _shutdown) = start().await;
    assert_eq!(
        post(&url, Some("Bearer wrong-token")).await,
        StatusCode::UNAUTHORIZED
    );
    // The token alone, without the scheme, is not enough.
    assert_eq!(post(&url, Some(TOKEN)).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_accepts_valid_token() {
    let (url, _shutdown) = start().await;
    let status = post(&url, Some(&format!("Bearer {TOKEN}"))).await;
    assert!(status.is_success(), "got {status}");
}

#[tokio::test]
async fn mcp_accepts_lan_host_header() {
    // Clients on the LAN connect by the server's address, not localhost.
    let (url, _shutdown) = start().await;
    let status = reqwest::Client::new()
        .post(&url)
        .header("Host", "192.168.1.20:8787")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {TOKEN}"))
        .body(INITIALIZE)
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "got {status}");
}

#[tokio::test]
async fn serve_refuses_short_token() {
    let db = Db::in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let err = mcp::serve_on(listener, db, "short".into(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("MCP_BEARER_TOKEN"), "{err}");
}
