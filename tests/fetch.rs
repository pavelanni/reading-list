//! `fetch_article` against a local mock server.

use reading_list::fetch::{FetchError, MAX_BODY_BYTES, fetch_article};
use url::Url;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE: &str = include_str!("fixtures/article.html");

/// Serves `body` with `content_type` at `/page` and returns that page's URL.
async fn serve(server: &MockServer, status: u16, content_type: &str, body: Vec<u8>) -> Url {
    Mock::given(path("/page"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("Content-Type", content_type)
                .set_body_bytes(body),
        )
        .mount(server)
        .await;
    Url::parse(&format!("{}/page", server.uri())).unwrap()
}

#[tokio::test]
async fn extracts_title_and_text() {
    let server = MockServer::start().await;
    let url = serve(&server, 200, "text/html; charset=utf-8", FIXTURE.into()).await;

    let fetched = fetch_article(&reqwest::Client::new(), &url).await.unwrap();
    let title = fetched.title.expect("title");
    assert!(
        title.contains("ESP32-C5 brings dual-band Wi-Fi 6"),
        "title: {title}"
    );
    assert!(fetched.text.contains("RISC-V core running at 240 MHz"));
    assert!(fetched.text.contains("Arduino support"));
}

#[tokio::test]
async fn pdf_is_not_html() {
    let server = MockServer::start().await;
    let url = serve(&server, 200, "application/pdf", b"%PDF-1.7".to_vec()).await;

    let err = fetch_article(&reqwest::Client::new(), &url)
        .await
        .unwrap_err();
    assert_eq!(err, FetchError::NotHtml("application/pdf".into()));
}

#[tokio::test]
async fn oversized_body_rejected() {
    let server = MockServer::start().await;
    let body = vec![b'a'; 6 * 1024 * 1024];
    assert!(body.len() > MAX_BODY_BYTES);
    let url = serve(&server, 200, "text/html", body).await;

    let err = fetch_article(&reqwest::Client::new(), &url)
        .await
        .unwrap_err();
    assert_eq!(err, FetchError::TooLarge);
}

#[tokio::test]
async fn http_404_reported() {
    let server = MockServer::start().await;
    let url = serve(&server, 404, "text/html", b"not found".to_vec()).await;

    let err = fetch_article(&reqwest::Client::new(), &url)
        .await
        .unwrap_err();
    assert_eq!(err, FetchError::Http(404));
    assert_eq!(err.to_string(), "HTTP 404");
}

#[tokio::test]
async fn page_without_article_is_empty() {
    let server = MockServer::start().await;
    let html = "<html><head><title>Login</title></head><body><p>Please sign in.</p></body></html>";
    let url = serve(&server, 200, "text/html", html.into()).await;

    let err = fetch_article(&reqwest::Client::new(), &url)
        .await
        .unwrap_err();
    assert_eq!(err, FetchError::Empty);
}

#[tokio::test]
async fn unreachable_server_is_network_error() {
    // Not a dropped MockServer: wiremock pools servers, so the port would
    // still answer (with 404). Take a free port from the OS and close it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = Url::parse(&format!("http://127.0.0.1:{port}/page")).unwrap();

    let err = fetch_article(&reqwest::Client::new(), &url)
        .await
        .unwrap_err();
    let FetchError::Network(msg) = err else {
        panic!("expected Network, got {err:?}");
    };
    // The cause from the source chain, not just "error sending request".
    assert!(msg.contains(": "), "no cause chain in: {msg}");
}
