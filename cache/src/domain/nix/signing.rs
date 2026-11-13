//! Ed25519 cache signing keys in Nix's `name:base64` format.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer as _, SigningKey as Ed25519Key};

/// A cache signing key. Why: clients trust a cache by its public key.
pub struct SigningKey {
    name: String,
    key: Ed25519Key,
}

/// A key file that could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSigningKey {
    /// No `name:` prefix.
    MissingName,
    /// Payload is not base64.
    NotBase64,
    /// Payload is not 32 (seed) or 64 (seed + public) bytes.
    WrongLength,
}

impl fmt::Display for InvalidSigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingName => "signing key has no `name:` prefix",
            Self::NotBase64 => "signing key payload is not base64",
            Self::WrongLength => "signing key payload must be 32 or 64 bytes",
        })
    }
}

impl std::error::Error for InvalidSigningKey {}

impl SigningKey {
    /// Parse the output of `nix-store --generate-binary-cache-key`.
    ///
    /// # Errors
    /// Returns [`InvalidSigningKey`] when the text is not `name:base64` with a
    /// 32- or 64-byte payload.
    pub fn parse(text: &str) -> Result<Self, InvalidSigningKey> {
        let (name, payload) = text
            .trim()
            .split_once(':')
            .ok_or(InvalidSigningKey::MissingName)?;
        if name.is_empty() {
            return Err(InvalidSigningKey::MissingName);
        }
        let bytes = STANDARD
            .decode(payload.trim())
            .map_err(|_| InvalidSigningKey::NotBase64)?;
        // Why: Nix secret keys hold seed + public (64 bytes); the seed is first.
        let seed: [u8; 32] = match bytes.len() {
            32 | 64 => bytes[..32]
                .try_into()
                .map_err(|_| InvalidSigningKey::WrongLength)?,
            _ => return Err(InvalidSigningKey::WrongLength),
        };
        Ok(Self {
            name: String::from(name),
            key: Ed25519Key::from_bytes(&seed),
        })
    }

    /// The key name clients list in `trusted-public-keys`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sign a narinfo fingerprint, returning a `name:base64` signature.
    #[must_use]
    pub fn sign(&self, fingerprint: &str) -> String {
        let signature = self.key.sign(fingerprint.as_bytes());
        format!("{}:{}", self.name, STANDARD.encode(signature.to_bytes()))
    }

    /// The `name:base64` public key clients must trust.
    #[must_use]
    pub fn public_key(&self) -> String {
        format!(
            "{}:{}",
            self.name,
            STANDARD.encode(self.key.verifying_key().to_bytes())
        )
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Why: never print the seed.
        f.debug_struct("SigningKey")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};

    use super::{InvalidSigningKey, SigningKey};

    fn key() -> SigningKey {
        SigningKey::parse(&format!("test-1:{}", STANDARD.encode([9_u8; 32]))).unwrap()
    }

    #[test]
    fn signatures_verify_with_the_public_key() {
        let key = key();
        let signature = key.sign("1;/nix/store/x;sha256:y;1;");
        let (name, sig_b64) = signature.split_once(':').unwrap();
        assert_eq!(name, "test-1");
        let public = key.public_key();
        let pub_bytes: [u8; 32] = STANDARD
            .decode(public.split_once(':').unwrap().1)
            .unwrap()
            .try_into()
            .unwrap();
        let sig_bytes: [u8; 64] = STANDARD.decode(sig_b64).unwrap().try_into().unwrap();
        VerifyingKey::from_bytes(&pub_bytes)
            .unwrap()
            .verify(
                b"1;/nix/store/x;sha256:y;1;",
                &Signature::from_bytes(&sig_bytes),
            )
            .unwrap();
    }

    #[test]
    fn accepts_64_byte_nix_secret_keys() {
        let mut both = [9_u8; 64];
        both[32..].fill(1);
        let long = SigningKey::parse(&format!("test-1:{}\n", STANDARD.encode(both))).unwrap();
        assert_eq!(long.public_key(), key().public_key());
    }

    #[test]
    fn rejects_malformed_keys_and_hides_the_seed() {
        assert_eq!(
            SigningKey::parse("nocolon").unwrap_err(),
            InvalidSigningKey::MissingName
        );
        assert_eq!(
            SigningKey::parse("k:!!!").unwrap_err(),
            InvalidSigningKey::NotBase64
        );
        assert_eq!(
            SigningKey::parse("k:AAAA").unwrap_err(),
            InvalidSigningKey::WrongLength
        );
        assert!(!format!("{:?}", key()).contains("key:"));
    }
}
