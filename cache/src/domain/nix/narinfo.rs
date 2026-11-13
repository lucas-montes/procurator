//! Narinfo parsing, validation, fingerprinting, and rendering.
//! Why: the cache stores a canonical narinfo it has validated and signed.

use std::fmt::{self, Write as _};

use super::signing::SigningKey;
use crate::domain::digest::Digest;

/// Length of a store path hash part.
const STORE_HASH_LEN: usize = 32;

/// Longest compression extension accepted in a NAR file name.
const MAX_EXT_LEN: usize = 8;

/// Why a narinfo, store hash, or NAR file name was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NixError {
    /// Hash part is not 32 lowercase alphanumerics.
    BadStoreHash,
    /// NAR file name is not `<nix32 sha256>.nar[.ext]`.
    BadNarFile,
    /// A line is not `Key: value`.
    BadLine,
    /// A required field is absent.
    MissingField(&'static str),
    /// A field holds a value the cache cannot accept.
    BadField(&'static str),
    /// `StorePath` is outside the cache's store directory.
    WrongStoreDir,
}

impl fmt::Display for NixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadStoreHash => f.write_str("store hash must be 32 lowercase alphanumerics"),
            Self::BadNarFile => f.write_str("NAR file must be named <nix32 sha256>.nar[.ext]"),
            Self::BadLine => f.write_str("narinfo line is not `Key: value`"),
            Self::MissingField(name) => write!(f, "narinfo is missing {name}"),
            Self::BadField(name) => write!(f, "narinfo has an invalid {name}"),
            Self::WrongStoreDir => f.write_str("StorePath is outside this cache's store"),
        }
    }
}

impl std::error::Error for NixError {}

/// The 32-character hash part of a store path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreHash<'a>(&'a str);

impl<'a> StoreHash<'a> {
    /// Validate a hash part.
    ///
    /// # Errors
    /// Returns [`NixError::BadStoreHash`] when `raw` is not a hash part.
    pub fn parse(raw: &'a str) -> Result<Self, NixError> {
        let valid = raw.len() == STORE_HASH_LEN
            && raw
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        valid.then_some(Self(raw)).ok_or(NixError::BadStoreHash)
    }

    /// The hash as text.
    #[must_use]
    pub fn as_str(&self) -> &'a str {
        self.0
    }
}

/// A NAR file name whose stem is the SHA-256 of the file's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NarFile<'a> {
    name: &'a str,
    digest: Digest,
}

impl<'a> NarFile<'a> {
    /// Validate `<nix32 sha256>.nar[.ext]`.
    ///
    /// # Errors
    /// Returns [`NixError::BadNarFile`] for any other shape.
    pub fn parse(name: &'a str) -> Result<Self, NixError> {
        let (stem, rest) = name.split_once('.').ok_or(NixError::BadNarFile)?;
        let ext_ok = match rest.strip_prefix("nar") {
            Some("") => true,
            Some(ext) => ext.strip_prefix('.').is_some_and(|e| {
                !e.is_empty()
                    && e.len() <= MAX_EXT_LEN
                    && e.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            }),
            None => false,
        };
        if !ext_ok {
            return Err(NixError::BadNarFile);
        }
        let digest = Digest::from_nix32(stem).map_err(|_| NixError::BadNarFile)?;
        Ok(Self { name, digest })
    }

    /// The full file name.
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// The digest the file's bytes must have.
    #[must_use]
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}

/// A parsed, validated narinfo borrowing from the uploaded text.
#[derive(Debug)]
pub struct Narinfo<'a> {
    store_path: &'a str,
    store_hash: StoreHash<'a>,
    nar_file: NarFile<'a>,
    compression: &'a str,
    file_hash: Option<&'a str>,
    file_size: Option<u64>,
    nar_hash: &'a str,
    nar_size: u64,
    references: &'a str,
    deriver: Option<&'a str>,
    sigs: Vec<&'a str>,
    ca: Option<&'a str>,
}

/// Parse a decimal size field.
fn size(value: &str, field: &'static str) -> Result<u64, NixError> {
    value.parse().map_err(|_| NixError::BadField(field))
}

/// Check a `sha256:<nix32>` hash field and return its digest.
fn sha256_field(value: &str, field: &'static str) -> Result<Digest, NixError> {
    value
        .strip_prefix("sha256:")
        .and_then(|h| Digest::from_nix32(h).ok())
        .ok_or(NixError::BadField(field))
}

/// A reference or deriver must be a bare `<hash>-<name>` base name.
fn is_base_name(value: &str) -> bool {
    value
        .split_once('-')
        .is_some_and(|(hash, name)| StoreHash::parse(hash).is_ok() && !name.is_empty())
        && !value.contains('/')
}

impl<'a> Narinfo<'a> {
    /// Parse and validate an uploaded narinfo for a cache serving `store_dir`.
    ///
    /// # Errors
    /// Returns [`NixError`] naming the first malformed or missing field.
    pub fn parse(body: &'a str, store_dir: &str) -> Result<Self, NixError> {
        let mut store_path = None;
        let mut url = None;
        let mut compression = None;
        let mut file_hash = None;
        let mut file_size = None;
        let mut nar_hash = None;
        let mut nar_size = None;
        let mut references = "";
        let mut deriver = None;
        let mut sigs = Vec::new();
        let mut ca = None;
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            let (key, value) = line.split_once(':').ok_or(NixError::BadLine)?;
            let value = value.trim();
            match key {
                "StorePath" => store_path = Some(value),
                "URL" => url = Some(value),
                "Compression" => compression = Some(value),
                "FileHash" => file_hash = Some(value),
                "FileSize" => file_size = Some(size(value, "FileSize")?),
                "NarHash" => nar_hash = Some(value),
                "NarSize" => nar_size = Some(size(value, "NarSize")?),
                "References" => references = value,
                "Deriver" => deriver = Some(value).filter(|d| *d != "unknown-deriver"),
                "Sig" => sigs.push(value),
                "CA" => ca = Some(value),
                // Why: unknown keys are dropped; the cache renders a canonical form.
                _ => {}
            }
        }
        let store_path = store_path.ok_or(NixError::MissingField("StorePath"))?;
        let base = store_path
            .strip_prefix(store_dir)
            .and_then(|rest| rest.strip_prefix('/'))
            .ok_or(NixError::WrongStoreDir)?;
        if !is_base_name(base) {
            return Err(NixError::BadField("StorePath"));
        }
        let store_hash = StoreHash::parse(&base[..STORE_HASH_LEN])?;
        let nar_file = url
            .ok_or(NixError::MissingField("URL"))?
            .strip_prefix("nar/")
            .ok_or(NixError::BadField("URL"))
            .and_then(NarFile::parse)?;
        let nar_hash = nar_hash.ok_or(NixError::MissingField("NarHash"))?;
        sha256_field(nar_hash, "NarHash")?;
        if let Some(hash) = file_hash {
            sha256_field(hash, "FileHash")?;
        }
        let nar_size = nar_size
            .filter(|n| *n > 0)
            .ok_or(NixError::MissingField("NarSize"))?;
        if !references.split_whitespace().all(is_base_name) {
            return Err(NixError::BadField("References"));
        }
        if deriver.is_some_and(|d| !is_base_name(d)) {
            return Err(NixError::BadField("Deriver"));
        }
        if sigs.iter().any(|s| !s.contains(':')) {
            return Err(NixError::BadField("Sig"));
        }
        Ok(Self {
            store_path,
            store_hash,
            nar_file,
            // Why: Nix treats an absent `Compression` as bzip2.
            compression: compression.unwrap_or("bzip2"),
            file_hash,
            file_size,
            nar_hash,
            nar_size,
            references,
            deriver,
            sigs,
            ca,
        })
    }

    /// The hash part of `StorePath`.
    #[must_use]
    pub fn store_hash(&self) -> StoreHash<'a> {
        self.store_hash
    }

    /// The NAR file `URL` points at.
    #[must_use]
    pub fn nar_file(&self) -> NarFile<'a> {
        self.nar_file
    }

    /// Check the narinfo against the NAR file actually stored.
    ///
    /// # Errors
    /// Returns [`NixError::BadField`] when a declared hash or size disagrees
    /// with the stored file.
    pub fn check_against_file(&self, digest: &Digest, size: u64) -> Result<(), NixError> {
        if self.file_size.is_some_and(|declared| declared != size) {
            return Err(NixError::BadField("FileSize"));
        }
        if let Some(hash) = self.file_hash
            && sha256_field(hash, "FileHash")? != *digest
        {
            return Err(NixError::BadField("FileHash"));
        }
        // Why: an uncompressed file IS the NAR, so `NarHash` is checkable too.
        if self.compression == "none" {
            if sha256_field(self.nar_hash, "NarHash")? != *digest {
                return Err(NixError::BadField("NarHash"));
            }
            if self.nar_size != size {
                return Err(NixError::BadField("NarSize"));
            }
        }
        Ok(())
    }

    /// The text Nix signs. Why: references are FULL store paths here, even
    /// though the `References:` field carries base names.
    fn fingerprint(&self, store_dir: &str) -> String {
        let mut out = format!("1;{};{};{};", self.store_path, self.nar_hash, self.nar_size);
        for (i, reference) in self.references.split_whitespace().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(out, "{store_dir}/{reference}");
        }
        out
    }

    /// Render the canonical narinfo, adding `key`'s signature unless a
    /// signature under that key name is already present.
    #[must_use]
    pub fn render_signed(&self, key: &SigningKey, store_dir: &str) -> String {
        let mut out = String::with_capacity(512);
        let _ = writeln!(out, "StorePath: {}", self.store_path);
        let _ = writeln!(out, "URL: nar/{}", self.nar_file.name());
        let _ = writeln!(out, "Compression: {}", self.compression);
        if let Some(hash) = self.file_hash {
            let _ = writeln!(out, "FileHash: {hash}");
        }
        if let Some(size) = self.file_size {
            let _ = writeln!(out, "FileSize: {size}");
        }
        let _ = writeln!(out, "NarHash: {}", self.nar_hash);
        let _ = writeln!(out, "NarSize: {}", self.nar_size);
        let _ = writeln!(out, "References: {}", self.references);
        if let Some(deriver) = self.deriver {
            let _ = writeln!(out, "Deriver: {deriver}");
        }
        let mut signed_by_key = false;
        for sig in &self.sigs {
            signed_by_key |= sig
                .split_once(':')
                .is_some_and(|(name, _)| name == key.name());
            let _ = writeln!(out, "Sig: {sig}");
        }
        if !signed_by_key {
            let _ = writeln!(out, "Sig: {}", key.sign(&self.fingerprint(store_dir)));
        }
        if let Some(ca) = self.ca {
            let _ = writeln!(out, "CA: {ca}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::{NarFile, Narinfo, NixError, StoreHash};
    use crate::domain::digest::Digest;
    use crate::domain::nix::SigningKey;

    const STORE: &str = "/nix/store";

    fn key() -> SigningKey {
        SigningKey::parse(&format!("cache-1:{}", STANDARD.encode([3_u8; 32]))).unwrap()
    }

    fn body(nar: &[u8], extra: &str) -> String {
        let hash = Digest::of(nar).to_nix32();
        format!(
            "StorePath: {STORE}/{h}-foo\nURL: nar/{hash}.nar\nCompression: none\n\
             FileHash: sha256:{hash}\nFileSize: {len}\nNarHash: sha256:{hash}\nNarSize: {len}\n\
             References: {h}-foo {d}-dep\n{extra}",
            h = "a".repeat(32),
            d = "b".repeat(32),
            len = nar.len(),
        )
    }

    #[test]
    fn store_hash_matches_nix_hash_parts() {
        assert!(StoreHash::parse(&"a".repeat(32)).is_ok());
        assert_eq!(StoreHash::parse("ABC"), Err(NixError::BadStoreHash));
        assert!(StoreHash::parse(&"a-".repeat(16)).is_err());
    }

    #[test]
    fn nar_file_requires_a_digest_stem() {
        let stem = Digest::of(b"x").to_nix32();
        let plain = format!("{stem}.nar");
        let xz = format!("{stem}.nar.xz");
        assert_eq!(*NarFile::parse(&plain).unwrap().digest(), Digest::of(b"x"));
        assert_eq!(NarFile::parse(&xz).unwrap().name(), xz);
        for bad in [
            String::from("x.nar"),
            format!("{stem}.tar"),
            format!("{stem}.nar."),
            format!("{stem}.nar.x/z"),
            format!("../{stem}.nar"),
            stem.clone(),
        ] {
            assert!(NarFile::parse(&bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn parses_and_checks_a_well_formed_narinfo() {
        let text = body(b"nar-bytes", "Deriver: unknown-deriver\n");
        let info = Narinfo::parse(&text, STORE).unwrap();
        assert_eq!(info.store_hash().as_str(), "a".repeat(32));
        assert_eq!(
            info.check_against_file(&Digest::of(b"nar-bytes"), 9),
            Ok(())
        );
        assert_eq!(
            info.check_against_file(&Digest::of(b"other"), 9),
            Err(NixError::BadField("FileHash"))
        );
        assert_eq!(
            info.check_against_file(&Digest::of(b"nar-bytes"), 10),
            Err(NixError::BadField("FileSize"))
        );
    }

    #[test]
    fn fingerprint_uses_full_reference_paths() {
        let text = body(b"n", "");
        let info = Narinfo::parse(&text, STORE).unwrap();
        let hash = Digest::of(b"n").to_nix32();
        assert_eq!(
            info.fingerprint(STORE),
            format!(
                "1;{STORE}/{a}-foo;sha256:{hash};1;{STORE}/{a}-foo,{STORE}/{b}-dep",
                a = "a".repeat(32),
                b = "b".repeat(32)
            )
        );
    }

    #[test]
    fn render_adds_the_cache_signature_exactly_once() {
        let key = key();
        let text = body(b"n", "Sig: upstream-1:AAAA\nUnknown: dropped\n");
        let rendered = Narinfo::parse(&text, STORE)
            .unwrap()
            .render_signed(&key, STORE);
        assert!(rendered.contains("Sig: upstream-1:AAAA\n"));
        assert_eq!(rendered.matches("Sig: cache-1:").count(), 1);
        assert!(!rendered.contains("Unknown"));
        // Why: re-uploading the stored form must not stack signatures.
        let again = Narinfo::parse(&rendered, STORE)
            .unwrap()
            .render_signed(&key, STORE);
        assert_eq!(again, rendered);
    }

    #[test]
    fn rejects_poisoned_metadata() {
        let good = body(b"n", "");
        let cases = [
            (
                good.replace("/nix/store/", "/other/store/"),
                NixError::WrongStoreDir,
            ),
            (
                good.replace("URL: nar/", "URL: ../"),
                NixError::BadField("URL"),
            ),
            (
                good.replace("NarHash: sha256:", "NarHash: md5:"),
                NixError::BadField("NarHash"),
            ),
            (
                good.replace("-dep", "/../dep"),
                NixError::BadField("References"),
            ),
            (
                good.replace("NarSize: 1", "NarSize: 0"),
                NixError::MissingField("NarSize"),
            ),
            (format!("{good}garbage\n"), NixError::BadLine),
            (format!("{good}Sig: nocolon\n"), NixError::BadField("Sig")),
        ];
        for (text, expected) in cases {
            assert_eq!(
                Narinfo::parse(&text, STORE).unwrap_err(),
                expected,
                "{text}"
            );
        }
    }
}
