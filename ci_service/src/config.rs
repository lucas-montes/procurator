#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_address: String,
    pub domain: String,
    pub repos_base_path: String,
    pub max_retries: i64,
    pub worker_poll_interval_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "../ci.db".to_string(),
            bind_address: "0.0.0.0:3000".to_string(),
            domain: "homelab".to_string(),
            repos_base_path: "/var/lib/git-server".to_string(),
            max_retries: 3,
            worker_poll_interval_ms: 1000,
        }
    }
}
