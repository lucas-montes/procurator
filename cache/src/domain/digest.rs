//! SHA-256 content digests and their text encodings.
//! Why: every blob is addressed by its digest; Nix names NAR files in nix32.

use std::fmt;

use sha2::{Digest as _, Sha256};

/// Nix base32 alphabet (no `e`, `o`, `t`, `u`).
const NIX32: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Length of a nix32-encoded SHA-256.
const NIX32_LEN: usize = 52;

/// A SHA-256 digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

/// A digest string that failed to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidDigest;

impl fmt::Display for InvalidDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid sha256 digest")
    }
}

impl std::error::Error for InvalidDigest {}

impl Digest {
    /// Hash `bytes` in one call.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Start hashing a stream of chunks.
    #[must_use]
    pub fn hasher() -> Hasher {
        Hasher(Sha256::new())
    }

    /// Decode 64 lowercase hex characters.
    ///
    /// # Errors
    /// Returns [`InvalidDigest`] when `text` is not 64 hex characters.
    pub fn from_hex(text: &str) -> Result<Self, InvalidDigest> {
        let mut out = [0_u8; 32];
        hex::decode_to_slice(text, &mut out).map_err(|_| InvalidDigest)?;
        Ok(Self(out))
    }

    /// Decode 52 nix32 characters.
    ///
    /// # Errors
    /// Returns [`InvalidDigest`] when `text` is not a nix32 SHA-256.
    pub fn from_nix32(text: &str) -> Result<Self, InvalidDigest> {
        let chars = text.as_bytes();
        if chars.len() != NIX32_LEN {
            return Err(InvalidDigest);
        }
        let mut out = [0_u8; 32];
        // Why: port of Nix `BaseNix32::decode` — groups count from the LSB end.
        for (n, c) in chars.iter().rev().enumerate() {
            let digit = NIX32.iter().position(|a| a == c).ok_or(InvalidDigest)?;
            let digit = u16::try_from(digit).map_err(|_| InvalidDigest)?;
            let bit = n * 5;
            let (i, j) = (bit / 8, bit % 8);
            let shifted = digit << j;
            out[i] |= shifted.to_le_bytes()[0];
            let carry = shifted.to_le_bytes()[1];
            if i + 1 < out.len() {
                out[i + 1] |= carry;
            } else if carry != 0 {
                return Err(InvalidDigest);
            }
        }
        Ok(Self(out))
    }

    /// Lowercase hex form (64 characters).
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Nix base32 form (52 characters).
    #[must_use]
    pub fn to_nix32(&self) -> String {
        let mut out = String::with_capacity(NIX32_LEN);
        for n in (0..NIX32_LEN).rev() {
            let bit = n * 5;
            let (i, j) = (bit / 8, bit % 8);
            let lo = u16::from(self.0[i]) >> j;
            let hi = self.0.get(i + 1).map_or(0, |b| u16::from(*b) << (8 - j));
            out.push(char::from(NIX32[usize::from((lo | hi) & 0x1f)]));
        }
        out
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({self})")
    }
}

/// Incremental SHA-256, for hashing uploads as they stream.
pub struct Hasher(Sha256);

impl Hasher {
    /// Feed one chunk.
    pub fn update(&mut self, chunk: &[u8]) {
        self.0.update(chunk);
    }

    /// Finish and return the digest.
    #[must_use]
    pub fn finish(self) -> Digest {
        Digest(self.0.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::Digest;

    #[test]
    fn nix32_matches_nix_hash_convert() {
        // Why: `nix hash convert` says this hex == this nix32.
        let digest =
            Digest::from_hex("4b4ca6bc50c269e51f6525b5bdeab4814d765a857cd2a6525dc832965c02cd96")
                .unwrap();
        assert_eq!(
            digest.to_nix32(),
            "15nd09f9ccn8bm9adlkwhmd7ckc1nkmbvd95clgyasf2a2yack2b"
        );
    }

    #[test]
    fn nix32_round_trips() {
        let digest = Digest::of(b"hello");
        assert_eq!(Digest::from_nix32(&digest.to_nix32()).unwrap(), digest);
        assert_eq!(Digest::from_hex(&digest.to_hex()).unwrap(), digest);
        assert_eq!(digest.to_string(), digest.to_hex());
    }

    #[test]
    fn rejects_malformed_text() {
        assert!(Digest::from_nix32("short").is_err());
        assert!(Digest::from_nix32(&"e".repeat(52)).is_err());
        // Why: 52 chars carry 260 bits; the top 4 must be zero.
        assert!(Digest::from_nix32(&"z".repeat(52)).is_err());
        assert!(Digest::from_hex("zz").is_err());
    }

    #[test]
    fn streaming_hash_equals_one_shot() {
        let mut hasher = Digest::hasher();
        hasher.update(b"hel");
        hasher.update(b"lo");
        assert_eq!(hasher.finish(), Digest::of(b"hello"));
    }
}
