//! Shortener resolution against a local mock server.
//!
//! The mock server's host (127.0.0.1) stands in for `t.co` and friends.

use reading_list::ingest::normalize::resolve_shortener_with_hosts;
use reading_list::net::{Addresses, WebClient};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MOCK_HOSTS: &[&str] = &["127.0.0.1"];

fn client() -> WebClient {
    WebClient::new(Addresses::AllowLoopback).unwrap()
}

#[tokio::test]
async fn shortener_followed() {
    let server = MockServer::start().await;
    let target = format!("{}/final", server.uri());
    Mock::given(method("HEAD"))
        .and(path("/abc"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", target.as_str()))
        .mount(&server)
        .await;
    Mock::given(path("/final"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let short = Url::parse(&format!("{}/abc", server.uri())).unwrap();
    let resolved = resolve_shortener_with_hosts(&client(), &short, MOCK_HOSTS).await;
    assert_eq!(resolved.as_str(), target);
}

#[tokio::test]
async fn falls_back_to_get_when_head_not_allowed() {
    let server = MockServer::start().await;
    let target = format!("{}/final", server.uri());
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/abc"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", target.as_str()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/final"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let short = Url::parse(&format!("{}/abc", server.uri())).unwrap();
    let resolved = resolve_shortener_with_hosts(&client(), &short, MOCK_HOSTS).await;
    assert_eq!(resolved.as_str(), target);
}

#[tokio::test]
async fn shortener_error_returns_input() {
    let server = MockServer::start().await;
    Mock::given(path("/abc"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let short = Url::parse(&format!("{}/abc", server.uri())).unwrap();
    let resolved = resolve_shortener_with_hosts(&client(), &short, MOCK_HOSTS).await;
    assert_eq!(resolved, short);
}

#[tokio::test]
async fn non_shortener_is_not_requested() {
    let server = MockServer::start().await;
    Mock::given(path("/abc"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", "http://elsewhere/"))
        .expect(0)
        .mount(&server)
        .await;

    let url = Url::parse(&format!("{}/abc", server.uri())).unwrap();
    let resolved = resolve_shortener_with_hosts(&client(), &url, &["t.co"]).await;
    assert_eq!(resolved, url);
}
