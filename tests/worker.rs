//! The processing worker with a scripted model, a recording notifier and
//! local pages.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use reading_list::bot::Notifier;
use reading_list::db::{Db, NewArticle};
use reading_list::llm::prompts::Assignment;
use reading_list::llm::{Llm, LlmError, Usage};
use reading_list::net::{Addresses, WebClient};
use reading_list::status::MAX_TRANSIENT_FAILURES;
use reading_list::worker::{Delays, Pause, Processed, QUOTA_MESSAGE, process_one, run};
use serde_json::{Value, json};
use sqlx::Row;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE: &str = include_str!("fixtures/article.html");

/// Answers from a script; when the script runs out, returns `fallback`.
struct FakeLlm {
    replies: Mutex<VecDeque<Result<Value, LlmError>>>,
    fallback: LlmError,
    prompts: Mutex<Vec<String>>,
}

impl FakeLlm {
    fn new(replies: Vec<Result<Value, LlmError>>) -> FakeLlm {
        FakeLlm {
            replies: Mutex::new(replies.into()),
            fallback: LlmError::BadResponse("script exhausted".into()),
            prompts: Mutex::new(Vec::new()),
        }
    }

    fn always(err: LlmError) -> FakeLlm {
        FakeLlm {
            fallback: err,
            ..FakeLlm::new(Vec::new())
        }
    }

    fn calls(&self) -> usize {
        self.prompts.lock().unwrap().len()
    }

    fn user_prompt(&self, n: usize) -> String {
        self.prompts.lock().unwrap()[n].clone()
    }
}

impl Llm for FakeLlm {
    // The guard is dropped before the function returns, so the future
    // holds no lock and stays `Send`.
    async fn chat_json(&self, _system: &str, user: &str) -> Result<(Value, Usage), LlmError> {
        self.prompts.lock().unwrap().push(user.to_owned());
        let next = self.replies.lock().unwrap().pop_front();
        next.unwrap_or_else(|| Err(self.fallback.clone()))
            .map(|v| (v, Usage::default()))
    }

    fn model(&self) -> &str {
        "fake-model"
    }
}

/// `run` owns its model; a reference lets the test keep reading `prompts`.
impl Llm for &FakeLlm {
    fn chat_json(
        &self,
        system: &str,
        user: &str,
    ) -> impl std::future::Future<Output = Result<(Value, Usage), LlmError>> + Send {
        (**self).chat_json(system, user)
    }

    fn model(&self) -> &str {
        (**self).model()
    }
}

#[derive(Default)]
struct FakeNotifier {
    sent: Mutex<Vec<String>>,
}

impl Notifier for &FakeNotifier {
    async fn notify(&self, text: &str) {
        self.sent.lock().unwrap().push(text.to_owned());
    }
}

fn new_story(title: &str) -> Result<Value, LlmError> {
    Ok(json!({"story_id": null,
              "new_story": {"title": title, "kind": "news", "summary": "first"}}))
}

fn existing(id: i64) -> Result<Value, LlmError> {
    Ok(json!({"story_id": id, "new_story": null}))
}

fn compared(adds: &[&str], summary: &str) -> Result<Value, LlmError> {
    Ok(json!({"adds": adds, "updated_summary": summary, "one_line_summary": "one line"}))
}

struct Env {
    db: Db,
    http: WebClient,
    server: MockServer,
}

impl Env {
    async fn new() -> Env {
        let server = MockServer::start().await;
        Mock::given(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path("/unavailable"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(wiremock::matchers::any())
            // Not `set_body_string`: it forces `text/plain`, overriding
            // any Content-Type header, and the fetcher rejects the page.
            .respond_with(ResponseTemplate::new(200).set_body_raw(FIXTURE, "text/html"))
            .mount(&server)
            .await;
        Env {
            db: Db::in_memory().await.unwrap(),
            http: WebClient::new(Addresses::AllowLoopback).unwrap(),
            server,
        }
    }

    async fn save(&self, page: &str) -> i64 {
        let url = format!("{}/{page}", self.server.uri());
        self.db
            .save_article(NewArticle {
                url: url.clone(),
                original_url: url,
                tags: vec!["esp32".into()],
                note: None,
                share_text: None,
            })
            .await
            .unwrap()
            .article_id
    }

    async fn process(&self, llm: &FakeLlm) -> Processed {
        process_one(&self.db, &self.http, llm).await.unwrap()
    }

    /// One column of one row, as text (NULL becomes "NULL").
    async fn get(&self, sql: &str, id: i64) -> String {
        // Test-only SQL built from literals in this file, never from input.
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
            .bind(id)
            .fetch_one(self.db.pool())
            .await
            .unwrap();
        let v: Option<String> = row.try_get(0).ok().flatten();
        v.unwrap_or_else(|| "NULL".into())
    }

    async fn set_story_status(&self, story_id: i64, status: &str) {
        sqlx::query("UPDATE story SET status = ? WHERE id = ?")
            .bind(status)
            .bind(story_id)
            .execute(self.db.pool())
            .await
            .unwrap();
    }

    /// Processes a first article into a new story; returns the story ID.
    async fn first_story(&self) -> i64 {
        let id = self.save("first").await;
        let llm = FakeLlm::new(vec![new_story("ESP32-C5"), compared(&["a"], "S1")]);
        assert_eq!(self.process(&llm).await, Processed::Done(id));
        self.get(
            "SELECT CAST(story_id AS TEXT) FROM article WHERE id = ?",
            id,
        )
        .await
        .parse()
        .unwrap()
    }
}

#[tokio::test]
async fn new_article_creates_story() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let llm = FakeLlm::new(vec![new_story("ESP32-C5"), compared(&["dual band"], "S1")]);

    assert_eq!(env.process(&llm).await, Processed::Done(id));

    let sid = "(SELECT story_id FROM article WHERE id = ?)";
    let story = |col: &str| format!("SELECT {col} FROM story WHERE id = {sid}");
    assert_eq!(env.get(&story("title"), id).await, "ESP32-C5");
    assert_eq!(env.get(&story("summary"), id).await, "S1");
    assert_eq!(env.get(&story("status"), id).await, "unread");
    assert_ne!(env.get(&story("last_new_info_at"), id).await, "NULL");
    let article = "SELECT process_status || ' ' || fetch_status FROM article WHERE id = ?";
    assert_eq!(env.get(article, id).await, "done ok");
    let calls = "SELECT CAST(count(*) AS TEXT) FROM model_call WHERE article_id = ?";
    assert_eq!(env.get(calls, id).await, "2");
    // The assign call saw the extracted text.
    assert!(llm.user_prompt(0).contains("RISC-V core"));
}

#[tokio::test]
async fn second_article_with_nothing_new_records_empty_addition() {
    let env = Env::new().await;
    let story_id = env.first_story().await;
    let id = env.save("second").await;
    let llm = FakeLlm::new(vec![existing(story_id), compared(&[], "S1")]);

    assert_eq!(env.process(&llm).await, Processed::Done(id));

    let added = "SELECT added_text FROM story_addition WHERE article_id = ?";
    assert_eq!(env.get(added, id).await, "");
    // The assign call listed the existing story.
    assert!(llm.user_prompt(0).contains(&format!("id={story_id}")));
}

#[tokio::test]
async fn read_story_becomes_unread_on_new_info() {
    let env = Env::new().await;
    let story_id = env.first_story().await;
    env.set_story_status(story_id, "read").await;
    env.save("second").await;
    let llm = FakeLlm::new(vec![existing(story_id), compared(&["new fact"], "S2")]);

    env.process(&llm).await;

    let status = "SELECT status FROM story WHERE id = ?";
    assert_eq!(env.get(status, story_id).await, "unread");
}

#[tokio::test]
async fn read_story_stays_read_without_new_info() {
    let env = Env::new().await;
    let story_id = env.first_story().await;
    env.set_story_status(story_id, "read").await;
    env.save("second").await;
    let llm = FakeLlm::new(vec![existing(story_id), compared(&[" "], "S1")]);

    env.process(&llm).await;

    let status = "SELECT status FROM story WHERE id = ?";
    assert_eq!(env.get(status, story_id).await, "read");
}

#[tokio::test]
async fn dropped_story_stays_dropped() {
    let env = Env::new().await;
    let story_id = env.first_story().await;
    env.set_story_status(story_id, "dropped").await;
    env.save("second").await;
    let llm = FakeLlm::new(vec![existing(story_id), compared(&["new fact"], "S2")]);

    env.process(&llm).await;

    let status = "SELECT status FROM story WHERE id = ?";
    assert_eq!(env.get(status, story_id).await, "dropped");
}

#[tokio::test]
async fn fetch_failure_still_assigned_to_story() {
    let env = Env::new().await;
    let id = env.save("missing").await;
    let llm = FakeLlm::new(vec![new_story("Unknown"), compared(&["a"], "S1")]);

    assert_eq!(env.process(&llm).await, Processed::Done(id));

    let article = "SELECT fetch_status || ' ' || process_status || ' ' || fetch_error \
                   FROM article WHERE id = ?";
    assert_eq!(env.get(article, id).await, "failed done HTTP 404");
    assert!(llm.user_prompt(0).contains("could not be fetched"));
}

#[tokio::test]
async fn quota_exhausted_keeps_pending_and_notifies_once() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let llm = FakeLlm::always(LlmError::QuotaExhausted);
    let notifier = FakeNotifier::default();
    let delays = Delays {
        idle: Duration::from_millis(5),
        retry: Duration::from_millis(5),
        quota: Duration::from_millis(5),
    };
    let shutdown = CancellationToken::new();

    let worker = run(
        env.db.clone(),
        env.http.clone(),
        &llm,
        &notifier,
        delays,
        shutdown.clone(),
    );
    // `calls()` counts inside the model call, before the worker releases
    // the article; cancelling then would leave it `processing` (by design,
    // see `run`). Wait until it is back in the queue.
    let status = "SELECT process_status FROM article WHERE id = ?";
    let stop_after_two_calls = async {
        while llm.calls() < 2 || env.get(status, id).await != "pending" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        shutdown.cancel();
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(worker, stop_after_two_calls)
    })
    .await
    .expect("worker did not stop");

    assert_eq!(*notifier.sent.lock().unwrap(), vec![QUOTA_MESSAGE]);
    let article =
        "SELECT process_status || ' ' || CAST(attempts AS TEXT) FROM article WHERE id = ?";
    assert_eq!(env.get(article, id).await, "pending 0");
}

#[tokio::test]
async fn bad_json_three_times_marks_failed() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let bad = || Ok(json!({"unexpected": true}));
    let llm = FakeLlm::new(vec![bad(), bad(), bad()]);

    assert_eq!(env.process(&llm).await, Processed::Retry(id));
    assert_eq!(env.process(&llm).await, Processed::Retry(id));
    assert_eq!(env.process(&llm).await, Processed::Failed(id));

    let article =
        "SELECT process_status || ' ' || CAST(attempts AS TEXT) FROM article WHERE id = ?";
    assert_eq!(env.get(article, id).await, "failed 3");
    assert_eq!(env.process(&llm).await, Processed::Idle);
}

#[tokio::test]
async fn unknown_story_id_counts_as_bad_response() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let llm = FakeLlm::new(vec![existing(999)]);

    assert_eq!(env.process(&llm).await, Processed::Retry(id));

    let article = "SELECT process_status || ' ' || CAST(attempts AS TEXT) || ' ' || error \
                   FROM article WHERE id = ?";
    assert_eq!(
        env.get(article, id).await,
        "pending 1 bad model response: unknown story_id 999"
    );
}

#[tokio::test]
async fn rate_limit_does_not_use_an_attempt() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let llm = FakeLlm::new(vec![Err(LlmError::RateLimited)]);

    assert_eq!(env.process(&llm).await, Processed::Deferred(id));

    let article =
        "SELECT process_status || ' ' || CAST(attempts AS TEXT) FROM article WHERE id = ?";
    assert_eq!(env.get(article, id).await, "pending 0");
}

#[tokio::test]
async fn stuck_processing_reset_on_startup() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let claimed = env.db.claim_next_pending().await.unwrap().unwrap();
    assert_eq!(claimed.id, id);
    assert!(env.db.claim_next_pending().await.unwrap().is_none());

    assert_eq!(env.db.reset_stuck_processing().await.unwrap(), 1);

    let status = "SELECT process_status FROM article WHERE id = ?";
    assert_eq!(env.get(status, id).await, "pending");
}

#[tokio::test]
async fn note_added_while_processing_requeues() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let claimed = env.db.claim_next_pending().await.unwrap().unwrap();
    env.db
        .add_note(id, "focus on power use", &[])
        .await
        .unwrap();

    let target = Assignment::New(reading_list::llm::prompts::NewStory {
        title: "T".into(),
        kind: reading_list::db::StoryKind::News,
        summary: "s".into(),
    });
    let result = reading_list::llm::prompts::CompareResult {
        adds: vec![],
        updated_summary: "s".into(),
        one_line_summary: "l".into(),
    };
    env.db
        .apply_comparison(&claimed, &target, &result)
        .await
        .unwrap();

    let status = "SELECT process_status FROM article WHERE id = ?";
    assert_eq!(env.get(status, id).await, "pending");
}

/// Runs the worker with short delays until `n` notifications were sent.
/// Each pause releases the article before notifying, so stopping right
/// after a notification never interrupts `process_one`.
async fn run_until_notified(env: &Env, llm: &FakeLlm, n: usize) -> Vec<String> {
    let notifier = FakeNotifier::default();
    let delays = Delays {
        idle: Duration::from_millis(5),
        retry: Duration::from_millis(5),
        quota: Duration::from_millis(5),
    };
    let shutdown = CancellationToken::new();
    let worker = run(
        env.db.clone(),
        env.http.clone(),
        llm,
        &notifier,
        delays,
        shutdown.clone(),
    );
    let stop = async {
        while notifier.sent.lock().unwrap().len() < n {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        shutdown.cancel();
    };
    tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(worker, stop) })
        .await
        .expect("worker did not stop");
    notifier.sent.into_inner().unwrap()
}

#[tokio::test]
async fn rejected_key_or_model_pauses_without_using_attempts() {
    for code in [401, 403, 404] {
        let env = Env::new().await;
        let id = env.save("page").await;
        let llm = FakeLlm::always(LlmError::Http(code, "rejected".into()));

        assert_eq!(
            env.process(&llm).await,
            Processed::Paused(Pause::Rejected(code))
        );
        let article =
            "SELECT process_status || ' ' || CAST(attempts AS TEXT) FROM article WHERE id = ?";
        assert_eq!(env.get(article, id).await, "pending 0", "HTTP {code}");
    }
}

#[tokio::test]
async fn bad_request_uses_an_attempt() {
    // 400 is usually about this article (too long, refused), not the setup.
    let env = Env::new().await;
    let id = env.save("page").await;
    let llm = FakeLlm::always(LlmError::Http(400, "context too long".into()));

    assert_eq!(env.process(&llm).await, Processed::Retry(id));
    let article =
        "SELECT process_status || ' ' || CAST(attempts AS TEXT) FROM article WHERE id = ?";
    assert_eq!(env.get(article, id).await, "pending 1");
}

#[tokio::test]
async fn new_pause_reason_notifies_again() {
    let env = Env::new().await;
    env.save("page").await;
    let llm = FakeLlm {
        fallback: LlmError::Http(401, "bad key".into()),
        ..FakeLlm::new(vec![Err(LlmError::QuotaExhausted)])
    };

    let sent = run_until_notified(&env, &llm, 2).await;
    assert_eq!(
        sent,
        vec![QUOTA_MESSAGE.to_owned(), Pause::Rejected(401).message()]
    );
}

#[tokio::test]
async fn quota_notifies_again_after_recovery() {
    let env = Env::new().await;
    env.save("page").await;
    env.save("other").await;
    // First article: quota, then processed on retry. Second article: quota.
    let llm = FakeLlm {
        fallback: LlmError::QuotaExhausted,
        ..FakeLlm::new(vec![
            Err(LlmError::QuotaExhausted),
            new_story("ESP32-C5"),
            compared(&["dual band"], "S1"),
        ])
    };

    let sent = run_until_notified(&env, &llm, 2).await;
    assert_eq!(sent, vec![QUOTA_MESSAGE, QUOTA_MESSAGE]);
}

impl Env {
    /// Runs test-only SQL built from literals in this file.
    async fn exec(&self, sql: &str, id: i64) {
        sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
            .bind(id)
            .execute(self.db.pool())
            .await
            .unwrap();
    }

    async fn near_cap(&self, id: i64) {
        let sql = format!(
            "UPDATE article SET transient_failures = {} WHERE id = ?",
            MAX_TRANSIENT_FAILURES - 1
        );
        self.exec(&sql, id).await;
    }
}

#[tokio::test]
async fn transient_model_error_defers_article_and_queue_moves_on() {
    let env = Env::new().await;
    let first = env.save("page").await;
    let second = env.save("other").await;
    let llm = FakeLlm::new(vec![
        Err(LlmError::Network("connection reset".into())),
        new_story("Second"),
        compared(&["news"], "S"),
    ]);

    assert_eq!(env.process(&llm).await, Processed::Deferred(first));
    // The first article waits; the one behind it is not blocked.
    assert_eq!(env.process(&llm).await, Processed::Done(second));
    assert_eq!(env.process(&llm).await, Processed::Idle);

    let q = "SELECT process_status || ' ' || attempts || ' ' || transient_failures || ' '
                    || (retry_at > datetime('now'))
             FROM article WHERE id = ?";
    assert_eq!(env.get(q, first).await, "pending 0 1 1");
}

#[tokio::test]
async fn deferred_article_is_claimed_when_its_time_comes() {
    let env = Env::new().await;
    let id = env.save("page").await;
    let llm = FakeLlm::new(vec![
        Err(LlmError::RateLimited),
        new_story("Later"),
        compared(&["news"], "S"),
    ]);

    assert_eq!(env.process(&llm).await, Processed::Deferred(id));
    env.exec(
        "UPDATE article SET retry_at = datetime('now', '-1 seconds') WHERE id = ?",
        id,
    )
    .await;
    assert_eq!(env.process(&llm).await, Processed::Done(id));
}

#[tokio::test]
async fn transient_failures_give_up_at_the_cap() {
    let env = Env::new().await;
    let id = env.save("page").await;
    env.near_cap(id).await;
    let llm = FakeLlm::always(LlmError::Http(503, "overloaded".into()));

    assert_eq!(env.process(&llm).await, Processed::Failed(id));
    let q = "SELECT process_status FROM article WHERE id = ?";
    assert_eq!(env.get(q, id).await, "failed");
}

#[tokio::test]
async fn transient_fetch_error_retries_the_page_without_calling_the_model() {
    let env = Env::new().await;
    let id = env.save("unavailable").await;
    let llm = FakeLlm::new(Vec::new());

    assert_eq!(env.process(&llm).await, Processed::Deferred(id));
    assert_eq!(llm.calls(), 0);
    let q = "SELECT fetch_status || ' ' || transient_failures || ' ' || error
             FROM article WHERE id = ?";
    assert_eq!(env.get(q, id).await, "pending 1 HTTP 503");
}

#[tokio::test]
async fn fetch_failing_at_the_cap_is_summarized_without_text() {
    let env = Env::new().await;
    let id = env.save("unavailable").await;
    env.near_cap(id).await;
    // Left by the previous transient failure.
    env.exec("UPDATE article SET error = 'HTTP 503' WHERE id = ?", id)
        .await;
    let llm = FakeLlm::new(vec![new_story("No text"), compared(&[], "S")]);

    assert_eq!(env.process(&llm).await, Processed::Done(id));
    let q = "SELECT fetch_status || ' ' || fetch_error FROM article WHERE id = ?";
    assert_eq!(env.get(q, id).await, "failed HTTP 503");
    // A done article has no processing error; the reason stays in fetch_error.
    let error = "SELECT error FROM article WHERE id = ?";
    assert_eq!(env.get(error, id).await, "NULL");
}

#[tokio::test]
async fn fetch_error_survives_a_model_failure() {
    let env = Env::new().await;
    let id = env.save("missing").await;
    let llm = FakeLlm::new(vec![Ok(json!({"unexpected": true}))]);

    assert_eq!(env.process(&llm).await, Processed::Retry(id));
    assert_eq!(
        env.get("SELECT fetch_error FROM article WHERE id = ?", id)
            .await,
        "HTTP 404"
    );
    let error = env.get("SELECT error FROM article WHERE id = ?", id).await;
    assert!(error.starts_with("bad model response"), "{error}");
}
