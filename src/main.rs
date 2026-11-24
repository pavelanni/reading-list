//! Wiring: config, database, clients, the three services, and shutdown.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use reading_list::bot::{self, TelegramNotifier};
use reading_list::config::Config;
use reading_list::db::Db;
use reading_list::llm::OpenRouterClient;
use reading_list::mcp;
use reading_list::net::{Addresses, WebClient};
use reading_list::worker::{self, Delays};
use teloxide::Bot;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::task::{Id, JoinError, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

/// How long the services get to finish after shutdown starts.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(8);
/// How long the runtime then waits for blocking tasks (readability
/// extraction cannot be cancelled). Drain plus this stays below the
/// Quadlet's 10 s `StopTimeout`, so we exit before Podman sends SIGKILL.
const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(1);

// No `#[tokio::main]`: that macro drops the runtime at the end, and
// dropping waits for `spawn_blocking` tasks with no time limit. Building
// the runtime by hand lets us bound that wait with `shutdown_timeout`.
fn main() -> Result<ExitCode> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let runtime = tokio::runtime::Runtime::new().context("starting tokio runtime")?;
    let code = runtime.block_on(run());
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
    code
}

async fn run() -> Result<ExitCode> {
    // First, before anything slow: in the container we are PID 1, and the
    // kernel drops signals PID 1 has no handler for. A SIGTERM during
    // startup is now kept until `supervise` reads it.
    let signals = Signals::install()?;

    // Errors before the services start return `Err`: Rust prints them and
    // exits with status 1.
    let cfg = Arc::new(Config::from_env()?);
    tracing::info!(?cfg, "starting");

    let db = Db::connect(&cfg.database_path).await?;
    let requeued = db.reset_stuck_processing().await?;
    if requeued > 0 {
        tracing::info!(
            requeued,
            "articles left processing by the last run are pending again"
        );
    }

    // Untrusted article URLs go through the guarded client; OpenRouter is a
    // fixed, trusted endpoint and gets a plain one.
    let web = WebClient::new(Addresses::PublicOnly).context("building web client")?;
    let openrouter_http = reqwest::Client::builder()
        .build()
        .context("building OpenRouter client")?;
    let llm = OpenRouterClient::new(
        openrouter_http,
        &cfg.openrouter_base_url,
        &cfg.openrouter_api_key,
        &cfg.openrouter_model,
    );
    let tg = Bot::new(&cfg.telegram_token);
    let notifier = TelegramNotifier::new(tg.clone(), cfg.allowed_user_id);

    let shutdown = CancellationToken::new();
    let mut services = Services::default();
    services.spawn(
        "bot",
        bot::run(tg, db.clone(), web.clone(), cfg.clone(), shutdown.clone()),
    );
    // `async move` takes ownership of what it uses, so it gets its own
    // clones; `db` itself is still needed below.
    let (worker_db, worker_shutdown) = (db.clone(), shutdown.clone());
    services.spawn("worker", async move {
        worker::run(
            worker_db,
            web,
            llm,
            notifier,
            Delays::default(),
            worker_shutdown,
        )
        .await;
        Ok(())
    });
    services.spawn("mcp", mcp::serve(db, cfg, shutdown.clone()));

    services.supervise(shutdown, signals).await
}

/// SIGTERM (Podman, systemd) and SIGINT (Ctrl-C) handlers.
struct Signals {
    term: Signal,
    int: Signal,
}

impl Signals {
    fn install() -> Result<Signals> {
        Ok(Signals {
            term: signal(SignalKind::terminate()).context("installing SIGTERM handler")?,
            int: signal(SignalKind::interrupt()).context("installing SIGINT handler")?,
        })
    }

    /// Waits for either signal and returns its name.
    async fn recv(&mut self) -> &'static str {
        tokio::select! {
            _ = self.term.recv() => "SIGTERM",
            _ = self.int.recv() => "SIGINT",
        }
    }
}

/// The running services, with names for log messages.
#[derive(Default)]
struct Services {
    tasks: JoinSet<Result<()>>,
    names: HashMap<Id, &'static str>,
}

impl Services {
    fn spawn(
        &mut self,
        name: &'static str,
        task: impl Future<Output = Result<()>> + Send + 'static,
    ) {
        let handle = self.tasks.spawn(task);
        self.names.insert(handle.id(), name);
    }

    /// Waits for a signal or for any service to stop, then stops the rest.
    async fn supervise(
        mut self,
        shutdown: CancellationToken,
        mut signals: Signals,
    ) -> Result<ExitCode> {
        let mut failed = false;

        tokio::select! {
            name = signals.recv() => tracing::info!(signal = name, "shutting down"),
            // `Some(..) =` disables this branch when no tasks are left.
            Some(joined) = self.tasks.join_next_with_id() => {
                let (name, result) = self.named(joined);
                failed |= exit_is_failure(name, result, false);
            }
        }
        shutdown.cancel();

        let drain = async {
            while let Some(joined) = self.tasks.join_next_with_id().await {
                let (name, result) = self.named(joined);
                failed |= exit_is_failure(name, result, true);
            }
        };
        if tokio::time::timeout(DRAIN_TIMEOUT, drain).await.is_err() {
            tracing::error!("services did not stop within {DRAIN_TIMEOUT:?}, aborting");
            failed = true;
        }
        // Dropping `self` aborts whatever is still running.
        Ok(if failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        })
    }

    /// Puts the service name next to a finished task's result. A panicked
    /// task has no return value, only a `JoinError` carrying its ID.
    fn named(
        &self,
        joined: Result<(Id, Result<()>), JoinError>,
    ) -> (&'static str, Result<Result<()>, JoinError>) {
        let id = match &joined {
            Ok((id, _)) => *id,
            Err(err) => err.id(),
        };
        let name = self.names.get(&id).copied().unwrap_or("unknown");
        (name, joined.map(|(_, result)| result))
    }
}

/// Logs how service `name` ended and says whether the process must exit
/// non-zero (so systemd treats it as a crash and restarts it).
///
/// `result` is nested: the outer `Result` is about the task (`Err` = it
/// panicked), the inner one is what the service itself returned.
/// `shutting_down` is true once we asked the services to stop.
fn exit_is_failure(name: &str, result: Result<Result<()>, JoinError>, shutting_down: bool) -> bool {
    match result {
        Ok(Ok(())) if shutting_down => {
            tracing::info!(service = name, "clean shutdown");
            false
        }
        Ok(Ok(())) => {
            tracing::error!(service = name, "service exited unexpectedly");
            true
        }
        Ok(Err(err)) => {
            tracing::error!(service = name, error = ?err, "service failed");
            true
        }
        Err(err) if err.is_panic() => {
            tracing::error!(service = name, error = %err, "service panicked");
            true
        }
        Err(err) => {
            tracing::error!(service = name, error = %err, "service task cancelled unexpectedly");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    async fn panicked() -> JoinError {
        tokio::spawn(async { panic!("boom") }).await.unwrap_err()
    }

    #[test]
    fn clean_stop_during_shutdown_is_success() {
        assert!(!exit_is_failure("bot", Ok(Ok(())), true));
    }

    #[test]
    fn stopping_on_its_own_is_failure() {
        // Services run until cancelled; returning early means something broke.
        assert!(exit_is_failure("bot", Ok(Ok(())), false));
    }

    #[test]
    fn service_error_is_failure_at_any_time() {
        assert!(exit_is_failure(
            "mcp",
            Ok(Err(anyhow!("bind failed"))),
            false
        ));
        assert!(exit_is_failure(
            "mcp",
            Ok(Err(anyhow!("bind failed"))),
            true
        ));
    }

    #[tokio::test]
    async fn panic_is_failure_at_any_time() {
        assert!(exit_is_failure("worker", Err(panicked().await), false));
        assert!(exit_is_failure("worker", Err(panicked().await), true));
    }
}
