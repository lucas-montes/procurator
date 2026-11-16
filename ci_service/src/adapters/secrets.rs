//! [`SecretStore`] over a TOML file of `name = "value"` lines.
//! Why: repo configs name secrets (webhook URLs) and never hold them; the
//! file lives outside the Nix store, readable by the service only.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;

use crate::application::ports::{DeliveryError, SecretStore};

/// Secrets read from a file at each lookup, so edits apply without a
/// restart. No file means no secrets.
#[derive(Debug)]
pub struct FileSecretStore {
    path: Option<PathBuf>,
}

impl FileSecretStore {
    /// Secrets from `path`, or none.
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
}

impl SecretStore for FileSecretStore {
    fn secret(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<String>, DeliveryError>> + Send {
        async move {
            let Some(path) = &self.path else {
                return Ok(None);
            };
            let text = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| DeliveryError::new(format!("secrets file {}: {e}", path.display())))?;
            let mut secrets: HashMap<String, String> = toml::from_str(&text)
                .map_err(|e| DeliveryError::new(format!("secrets file {}: {e}", path.display())))?;
            Ok(secrets.remove(name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileSecretStore;
    use crate::application::ports::SecretStore as _;

    #[tokio::test]
    async fn reads_named_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        std::fs::write(&path, "slack = \"https://hooks.example/abc\"\n").unwrap();
        let store = FileSecretStore::new(Some(path));
        assert_eq!(
            store.secret("slack").await.unwrap().as_deref(),
            Some("https://hooks.example/abc")
        );
        assert_eq!(store.secret("other").await.unwrap(), None);
        assert_eq!(
            FileSecretStore::new(None).secret("slack").await.unwrap(),
            None
        );

        let broken = dir.path().join("broken.toml");
        std::fs::write(&broken, "slack = \n").unwrap();
        assert!(
            FileSecretStore::new(Some(broken))
                .secret("slack")
                .await
                .is_err()
        );
    }
}
