use std::sync::OnceLock;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Clone)]
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
            bind_address: "127.0.0.1:3000".to_string(),
            repos_base_path: "../repos".to_string(),
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
