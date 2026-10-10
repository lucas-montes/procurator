//! Pluggable TLS providers with static dispatch (generics, no `dyn`).
//! Why: choose `file` now, `vault`/`step`/`spire` later without touching handlers.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Identity extracted from a verified client certificate.
/// Why: fingerprint always works; CN is best-effort (needs X.509 parse).
#[derive(Debug, Clone, Default)]
pub struct PeerIdentity {
    /// Lowercase hex SHA-256 over the leaf DER bytes.
    pub fingerprint_hex: String,
    /// Common name if the leaf parsed as X.509.
    pub common_name: Option<String>,
    /// True when the peer presented a chain rustls verified.
    pub verified: bool,
    /// True for loopback peers (set by the acceptor, not the cert).
    pub is_loopback: bool,
}

impl PeerIdentity {
    /// Anonymous caller (plain HTTP or no client cert).
    #[must_use]
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// Match against allowlist entries (`CN` or `sha256:<hex>` or bare hex).
    #[must_use]
    pub fn matches_allowlist(&self, allowlist: &[String]) -> bool {
        if allowlist.is_empty() {
            return self.verified;
        }
        allowlist.iter().any(|entry| {
            let e = entry.trim();
            if let Some(fp) = e.strip_prefix("sha256:") {
                return fp.eq_ignore_ascii_case(&self.fingerprint_hex);
            }
            if e.len() == 64 && e.chars().all(|c| c.is_ascii_hexdigit()) {
                return e.eq_ignore_ascii_case(&self.fingerprint_hex);
            }
            match &self.common_name {
                Some(cn) => cn == e,
                None => false,
            }
        })
    }
}

/// Concrete TLS errors. Why: no `anyhow`, matchable by callers.
#[derive(Debug)]
pub enum TlsError {
    /// Missing file or unreadable PEM.
    Io(String),
    /// No certificate found in the file.
    EmptyChain(String),
    /// Certificate/key pair mismatch.
    InvalidPair(String),
    /// Unknown provider name.
    UnknownProvider(String),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(m) => write!(f, "tls io: {m}"),
            Self::EmptyChain(m) => write!(f, "tls empty chain: {m}"),
            Self::InvalidPair(m) => write!(f, "tls invalid pair: {m}"),
            Self::UnknownProvider(m) => write!(f, "tls unknown provider: {m}"),
        }
    }
}

impl std::error::Error for TlsError {}

/// Provider trait with static dispatch. Why: no `dyn`, no `Send+Sync` bounds.
pub trait TlsProvider: Clone + 'static {
    /// Per-impl error type (no `Box<dyn>`).
    type Error: std::error::Error;
    /// Build the rustls server config (with mTLS verifier when a client CA is set).
    fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, Self::Error>;
    /// Reload material without restart (file provider re-reads PEMs).
    fn reload(&self) -> Result<(), Self::Error>;
}

/// File-backed provider: server chain+key plus optional client CA bundle.
/// Why: matches harmonia/nix-serve file deployment; vault/step impls come later.
#[derive(Debug, Clone)]
pub struct FilePki {
    /// Server certificate chain (PEM).
    pub cert: PathBuf,
    /// Server private key (PEM, PKCS#8/RSA/SEC1).
    pub key: PathBuf,
    /// Client CA bundle (PEM). `None` = no client auth.
    pub client_ca: Option<PathBuf>,
}

impl TlsProvider for FilePki {
    type Error = TlsError;

    fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, Self::Error> {
        // Why: explicit crypto provider (rustls 0.23 requires one).
        let _ = rustls::crypto::ring::default_provider().install_default();
        let chain = load_certs(&self.cert)?;
        let key = load_key(&self.key)?;
        let cfg = match &self.client_ca {
            None => rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .map_err(|e| TlsError::InvalidPair(e.to_string()))?,
            Some(ca) => {
                // Why: mTLS — only clients chaining to our CA pass the handshake.
                let mut roots = rustls::RootCertStore::empty();
                for cert in load_certs(ca)? {
                    roots
                        .add(cert)
                        .map_err(|e| TlsError::Io(format!("bad client CA: {e}")))?;
                }
                let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
                    .build()
                    .map_err(|e| TlsError::Io(format!("verifier: {e}")))?;
                rustls::ServerConfig::builder()
                    .with_client_cert_verifier(verifier)
                    .with_single_cert(chain, key)
                    .map_err(|e| TlsError::InvalidPair(e.to_string()))?
            }
        };
        Ok(Arc::new(cfg))
    }

    fn reload(&self) -> Result<(), Self::Error> {
        // Why: fail fast on rotation errors instead of serving stale certs.
        let _ = self.server_config()?;
        Ok(())
    }
}

/// SHA-256 fingerprint of a leaf DER cert, lowercase hex.
/// Why: stable identity even when X.509 parsing fails.
#[must_use]
pub fn fingerprint_hex(der: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    hex::encode(Sha256::digest(der))
}

/// Build identity from an optional verified leaf DER.
/// Why: handshake already verified the chain; we only label the peer.
#[must_use]
pub fn identity_from_leaf(leaf_der: Option<&[u8]>) -> PeerIdentity {
    match leaf_der {
        None => PeerIdentity::anonymous(),
        Some(der) => PeerIdentity {
            fingerprint_hex: fingerprint_hex(der),
            common_name: common_name_of(der),
            verified: true,
            is_loopback: false,
        },
    }
}

/// Best-effort CN extraction. Why: allowlists use human-readable CNs.
fn common_name_of(der: &[u8]) -> Option<String> {
    use x509_parser::prelude::{FromDer as _, X509Certificate};
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    let attr = cert.subject().iter_common_name().next()?;
    String::from_utf8(attr.as_slice().to_vec()).ok()
}

fn load_certs(path: &Path) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, TlsError> {
    let file =
        std::fs::File::open(path).map_err(|e| TlsError::Io(format!("{}: {e}", path.display())))?;
    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<_, _>>()
        .map_err(|e| TlsError::Io(format!("{}: {e}", path.display())))?;
    if certs.is_empty() {
        return Err(TlsError::EmptyChain(path.display().to_string()));
    }
    Ok(certs)
}

fn load_key(path: &Path) -> Result<rustls::pki_types::PrivateKeyDer<'static>, TlsError> {
    let file =
        std::fs::File::open(path).map_err(|e| TlsError::Io(format!("{}: {e}", path.display())))?;
    rustls_pemfile::private_key(&mut BufReader::new(file))
        .map_err(|e| TlsError::Io(format!("{}: {e}", path.display())))?
        .ok_or_else(|| TlsError::EmptyChain(format!("no key in {}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::PeerIdentity;

    #[test]
    fn allowlist_matches_cn_or_fingerprint() {
        let id = PeerIdentity {
            fingerprint_hex: "ab".repeat(32),
            common_name: Some("ci-writer".to_string()),
            verified: true,
            is_loopback: false,
        };
        assert!(id.matches_allowlist(&[]));
        assert!(id.matches_allowlist(&["ci-writer".to_string()]));
        assert!(id.matches_allowlist(&[format!("sha256:{}", "ab".repeat(32))]));
        assert!(!id.matches_allowlist(&["other".to_string()]));
    }

    #[test]
    fn anonymous_needs_explicit_empty_allowlist_and_verified() {
        let anon = PeerIdentity::anonymous();
        assert!(!anon.matches_allowlist(&[]));
    }
}
