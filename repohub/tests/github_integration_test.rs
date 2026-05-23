#[cfg(test)]
mod tests {
    use repohub::adapters::github::auth::GithubAuth;
    use repohub::adapters::github::client::GithubClient;

    #[test]
    fn test_github_auth_can_be_created_with_pat() {
        // Test that we can create a GithubAuth instance with a PAT
        let _auth = GithubAuth::new("ghp_test_token".to_string());
        // Construction implies the struct is well-formed
    }

    #[tokio::test]
    async fn test_github_client_can_be_created_with_pat() {
        let db = repohub::Database::new("sqlite::memory:")
            .await
            .expect("Failed to create test database");

        let auth = GithubAuth::new("ghp_test_token".to_string());

        let _client =
            GithubClient::new(auth, db, "test-owner".to_string(), "test-repo".to_string());
        // Construction implies the client is well-formed
    }
}
