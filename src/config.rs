//! Service configuration loaded from environment variables.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_MCP_LISTEN_ADDR: &str = "0.0.0.0:8787";
const DEFAULT_DATABASE_PATH: &str = "/data/reading-list.db";

/// All settings the service needs. Secrets are redacted in `Debug` output.
pub struct Config {
    pub telegram_token: String,
    pub allowed_user_id: u64,
    pub openrouter_api_key: String,
    pub openrouter_model: String,
    pub openrouter_base_url: String,
    pub mcp_bearer_token: String,
    pub mcp_listen_addr: SocketAddr,
    pub database_path: PathBuf,
}

impl Config {
    /// Reads configuration from the process environment.
    pub fn from_env() -> Result<Config> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads configuration through `lookup`, so tests never touch the
    /// process environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Config> {
        let required = |key: &str| {
            lookup(key)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| anyhow!("missing required environment variable {key}"))
        };
        let optional = |key: &str, default: &str| lookup(key).unwrap_or_else(|| default.to_owned());

        let allowed_user_id = required("TELEGRAM_ALLOWED_USER_ID")?
            .parse()
            .context("TELEGRAM_ALLOWED_USER_ID must be a numeric Telegram user ID")?;
        let mcp_listen_addr = optional("MCP_LISTEN_ADDR", DEFAULT_MCP_LISTEN_ADDR)
            .parse()
            .context("MCP_LISTEN_ADDR must be host:port, e.g. 0.0.0.0:8787")?;

        Ok(Config {
            telegram_token: required("TELEGRAM_BOT_TOKEN")?,
            allowed_user_id,
            openrouter_api_key: required("OPENROUTER_API_KEY")?,
            openrouter_model: required("OPENROUTER_MODEL")?,
            openrouter_base_url: optional("OPENROUTER_BASE_URL", DEFAULT_OPENROUTER_BASE_URL),
            mcp_bearer_token: required("MCP_BEARER_TOKEN")?,
            mcp_listen_addr,
            database_path: optional("DATABASE_PATH", DEFAULT_DATABASE_PATH).into(),
        })
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const HIDDEN: &str = "<redacted>";
        f.debug_struct("Config")
            .field("telegram_token", &HIDDEN)
            .field("allowed_user_id", &self.allowed_user_id)
            .field("openrouter_api_key", &HIDDEN)
            .field("openrouter_model", &self.openrouter_model)
            .field("openrouter_base_url", &self.openrouter_base_url)
            .field("mcp_bearer_token", &HIDDEN)
            .field("mcp_listen_addr", &self.mcp_listen_addr)
            .field("database_path", &self.database_path)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    const REQUIRED: &[(&str, &str)] = &[
        ("TELEGRAM_BOT_TOKEN", "secret-token"),
        ("TELEGRAM_ALLOWED_USER_ID", "42"),
        ("OPENROUTER_API_KEY", "secret-key"),
        ("OPENROUTER_MODEL", "deepseek/deepseek-flash"),
        ("MCP_BEARER_TOKEN", "secret-bearer"),
    ];

    #[test]
    fn config_missing_token_errors() {
        let without_token: Vec<_> = REQUIRED
            .iter()
            .copied()
            .filter(|(k, _)| *k != "TELEGRAM_BOT_TOKEN")
            .collect();
        let err = Config::from_lookup(lookup(&without_token)).unwrap_err();
        assert!(err.to_string().contains("TELEGRAM_BOT_TOKEN"), "{err}");
    }

    #[test]
    fn config_defaults_applied() {
        let cfg = Config::from_lookup(lookup(REQUIRED)).unwrap();
        assert_eq!(cfg.mcp_listen_addr, "0.0.0.0:8787".parse().unwrap());
        assert_eq!(cfg.openrouter_base_url, "https://openrouter.ai/api/v1");
        assert_eq!(cfg.database_path, PathBuf::from("/data/reading-list.db"));
        assert_eq!(cfg.allowed_user_id, 42);
    }

    #[test]
    fn config_debug_redacts() {
        let cfg = Config::from_lookup(lookup(REQUIRED)).unwrap();
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("secret-token"), "{shown}");
        assert!(!shown.contains("secret-key"), "{shown}");
        assert!(!shown.contains("secret-bearer"), "{shown}");
    }
}
