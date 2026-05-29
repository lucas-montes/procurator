#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub database_url: String,
    pub bind_address: String,
    pub domain: String,
    pub repos_base_path: String,
    // DORA configuration
    pub dora_interval_seconds: u64,
    pub dora_incident_label_patterns: Vec<String>,
    // GitHub OAuth configuration
    pub github_oauth_client_id: String,
    pub github_oauth_client_secret: String,
    pub github_oauth_redirect_url: String,
    // GitHub App configuration
    pub github_app_id: Option<u64>,
    pub github_app_private_key_pem: Option<String>,
    pub github_webhook_secret: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "../repohub.db".to_string(),
            bind_address: "0.0.0.0:3001".to_string(),
            domain: "homelab".to_string(),
            repos_base_path: "git-server".to_string(),
            // DORA configuration (default values for development)
            dora_interval_seconds: 3600,
            dora_incident_label_patterns: vec![".*incident.*".to_string()],
            // GitHub OAuth configuration (empty defaults; configure explicitly when used)
            github_oauth_client_id: String::new(),
            github_oauth_client_secret: String::new(),
            github_oauth_redirect_url: String::new(),
            // GitHub App defaults
            github_app_id: None,
            github_app_private_key_pem: None,
            github_webhook_secret: None,
        }
    }
}
