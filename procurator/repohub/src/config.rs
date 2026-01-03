#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub database_url: String,
    pub bind_address: String,
    pub domain: String,
    pub repos_base_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "../repohub.db".to_string(),
            bind_address: "0.0.0.0:3001".to_string(),
            domain: "homelab".to_string(),
            repos_base_path: "/var/lib/git-server".to_string(),
        }
    }
}
