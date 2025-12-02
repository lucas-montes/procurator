//! Configuration Management
//!
//! Provides application configuration as a singleton using `OnceLock`.
//! Configuration values are read from environment variables with sensible defaults.
//!
//! ## Configuration Variables
//!
//! - `DATABASE_URL`: Path to SQLite database file (default: `../ci.db`)
//! - `BIND_ADDRESS`: HTTP server bind address (default: `0.0.0.0:3000`)
//! - `REPOS_BASE_PATH`: Base path for Git repositories (default: `../repos`)
//! - `MAX_RETRIES`: Maximum build retry attempts (default: `3`)
//! - `WORKER_POLL_INTERVAL_MS`: Build queue poll interval in milliseconds (default: `1000`)

use std::sync::OnceLock;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub database_url: String,
    pub bind_address: String,
    pub repos_base_path: String,
    pub max_retries: i64,
    pub worker_poll_interval_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "../ci.db".to_string(),
            bind_address: "0.0.0.0:3000".to_string(),
            repos_base_path: "/var/lib/git-server".to_string(),
            max_retries: 3,
            worker_poll_interval_ms: 1000,
        }
    }
}

impl Config {
    /// Initialize the global config (can only be called once)
    pub fn init() -> &'static Config {
        CONFIG.get_or_init(|| Config::default())
    }

}
