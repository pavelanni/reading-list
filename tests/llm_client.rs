//! OpenRouter client against a local mock server.

use reading_list::llm::{Llm, LlmError, OpenRouterClient};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL: &str = "test/model";

fn client(server: &MockServer) -> OpenRouterClient {
    OpenRouterClient::new(reqwest::Client::new(), &server.uri(), "test-key", MODEL)
}

/// A successful chat completion whose message content is `content`.
fn completion(content: &str) -> serde_json::Value {
    json!({
        "choices": [{ "message": { "role": "assistant", "content": content } }],
        "usage": { "prompt_tokens": 120, "completion_tokens": 30, "cost": 0.00042 }
    })
}

async fn respond(server: &MockServer, template: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(template)
        .mount(server)
        .await;
}

#[tokio::test]
async fn parses_json_content_and_usage() {
    let server = MockServer::start().await;
    respond(
        &server,
        ResponseTemplate::new(200).set_body_json(completion(r#"{"story_id": 7}"#)),
    )
    .await;

    let (value, usage) = client(&server).chat_json("sys", "user").await.unwrap();
    assert_eq!(value, json!({ "story_id": 7 }));
    assert_eq!(usage.prompt_tokens, 120);
    assert_eq!(usage.completion_tokens, 30);
    assert_eq!(usage.cost, Some(0.00042));
}

#[tokio::test]
async fn sends_bearer_and_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer test-key"))
        .and(body_partial_json(json!({
            "model": MODEL,
            "response_format": { "type": "json_object" },
            "usage": { "include": true },
            "messages": [
                { "role": "system", "content": "sys" },
                { "role": "user", "content": "user" }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("{}")))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).chat_json("sys", "user").await.unwrap();
    // `expect(1)` is verified when `server` is dropped.
}

#[tokio::test]
async fn maps_402_to_quota_exhausted() {
    let server = MockServer::start().await;
    respond(&server, ResponseTemplate::new(402)).await;
    let err = client(&server).chat_json("s", "u").await.unwrap_err();
    assert_eq!(err, LlmError::QuotaExhausted);
}

#[tokio::test]
async fn maps_429_to_rate_limited() {
    let server = MockServer::start().await;
    respond(&server, ResponseTemplate::new(429)).await;
    let err = client(&server).chat_json("s", "u").await.unwrap_err();
    assert_eq!(err, LlmError::RateLimited);
}

#[tokio::test]
async fn other_status_carries_api_message() {
    let server = MockServer::start().await;
    let body = json!({ "error": { "code": 401, "message": "No auth credentials found" } });
    respond(&server, ResponseTemplate::new(401).set_body_json(body)).await;
    let err = client(&server).chat_json("s", "u").await.unwrap_err();
    assert_eq!(err, LlmError::Http(401, "No auth credentials found".into()));
}

#[tokio::test]
async fn long_api_message_is_truncated() {
    // Provider messages can echo the prompt, which holds untrusted article text.
    let server = MockServer::start().await;
    let body = json!({ "error": { "message": "x".repeat(10_000) } });
    respond(&server, ResponseTemplate::new(400).set_body_json(body)).await;
    let err = client(&server).chat_json("s", "u").await.unwrap_err();
    let LlmError::Http(400, message) = err else {
        panic!("{err:?}")
    };
    assert_eq!(message.chars().count(), 300);
}

#[tokio::test]
async fn non_json_content_is_bad_response() {
    let server = MockServer::start().await;
    respond(
        &server,
        ResponseTemplate::new(200).set_body_json(completion("Sure! Here is the story.")),
    )
    .await;
    let err = client(&server).chat_json("s", "u").await.unwrap_err();
    assert!(matches!(err, LlmError::BadResponse(_)), "got {err:?}");
}

#[tokio::test]
async fn json_inside_code_fence_is_accepted() {
    let server = MockServer::start().await;
    let fenced = "```json\n{\"story_id\": 3}\n```";
    respond(
        &server,
        ResponseTemplate::new(200).set_body_json(completion(fenced)),
    )
    .await;
    let (value, _) = client(&server).chat_json("s", "u").await.unwrap();
    assert_eq!(value, json!({ "story_id": 3 }));
}

#[tokio::test]
async fn usage_without_token_counts_still_parses() {
    // The answer is valid and already billed; missing counts must not
    // turn it into a BadResponse and a second, billed attempt.
    let server = MockServer::start().await;
    let mut body = completion(r#"{"story_id": 7}"#);
    body["usage"] = json!({ "cost": 0.0001 });
    respond(&server, ResponseTemplate::new(200).set_body_json(body)).await;

    let (value, usage) = client(&server).chat_json("sys", "user").await.unwrap();
    assert_eq!(value, json!({ "story_id": 7 }));
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.cost, Some(0.0001));
}
