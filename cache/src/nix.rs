//! Nix store introspection + narinfo helpers.
//! Why: fixes the old `nix-store -q` multi-flag bug with one `nix path-info --json` call.

use serde::Deserialize;

/// Concrete Nix errors. Why: no `anyhow`, no `Box<dyn>`.
#[derive(Debug)]
pub enum NixError {
    /// Hash part failed validation.
    BadHash(String),
    /// No store path matches the hash.
    NotFound(String),
    /// A Nix subprocess failed.
    Command(String),
    /// JSON or hash decoding failed.
    Decode(String),
}

impl std::fmt::Display for NixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadHash(m) => write!(f, "bad hash: {m}"),
            Self::NotFound(m) => write!(f, "not found: {m}"),
            Self::Command(m) => write!(f, "nix command: {m}"),
            Self::Decode(m) => write!(f, "decode: {m}"),
        }
    }
}

impl std::error::Error for NixError {}

/// Subset of `nix path-info --json` we need.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    /// SRI hash, e.g. `sha256-AbCd...=`.
    #[serde(default)]
    pub nar_hash: String,
    /// Uncompressed NAR size.
    #[serde(default)]
    pub nar_size: u64,
    /// Full store paths of references.
    #[serde(default)]
    pub references: Vec<String>,
    /// Full deriver path, if known.
    #[serde(default)]
    pub deriver: Option<String>,
    /// Existing `name:base64sig` entries.
    #[serde(default)]
    pub signatures: Vec<String>,
    /// Content address, e.g. `sha256:...` or object.
    #[serde(default)]
    pub ca: Option<serde_json::Value>,
}

/// Validate a 32-char Nix hash part.
pub fn check_hash(hash: &str) -> Result<(), NixError> {
    if hash.len() == 32
        && hash
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(NixError::BadHash(hash.to_string()))
    }
}

/// Resolve `<hash>` to a full store path by scanning `<store_dir>/<hash>-*`.
/// Why: narinfo filenames carry only the hash part, not `-name`.
pub async fn resolve_store_path(store_dir: &str, hash: &str) -> Result<String, NixError> {
    check_hash(hash)?;
    let prefix = format!("{hash}-");
    let mut dir = tokio::fs::read_dir(store_dir)
        .await
        .map_err(|e| NixError::Command(format!("read {store_dir}: {e}")))?;
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) {
            return Ok(format!("{store_dir}/{name}"));
        }
    }
    Err(NixError::NotFound(hash.to_string()))
}

/// Query one store path via `nix path-info --json`.
/// Why: single call replaces four conflicting `nix-store -q` flags.
pub async fn query_path_info(store_path: &str) -> Result<PathInfo, NixError> {
    let out = tokio::process::Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command",
            "path-info",
            "--json",
            "--",
            store_path,
        ])
        .output()
        .await
        .map_err(|e| NixError::Command(format!("spawn nix path-info: {e}")))?;
    if !out.status.success() {
        return Err(NixError::NotFound(store_path.to_string()));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| NixError::Decode(format!("path-info json: {e}")))?;
    let info: PathInfo = json
        .get(store_path)
        .cloned()
        .ok_or_else(|| NixError::NotFound(store_path.to_string()))
        .and_then(|v| {
            serde_json::from_value(v).map_err(|e| NixError::Decode(format!("info: {e}")))
        })?;
    Ok(info)
}

/// Convert SRI `sha256-<base64>` to narinfo `sha256:<nix32>`.
/// Why: `nix path-info` speaks SRI, narinfo files speak nix32.
pub fn sri_to_narinfo(sri: &str) -> Result<String, NixError> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let b64 = sri
        .strip_prefix("sha256-")
        .ok_or_else(|| NixError::Decode(format!("not sha256 SRI: {sri}")))?;
    let bytes = STANDARD
        .decode(b64)
        .map_err(|e| NixError::Decode(format!("base64: {e}")))?;
    if bytes.len() != 32 {
        return Err(NixError::Decode(format!(
            "want 32 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(format!("sha256:{}", nix32_encode(&bytes)))
}

/// Nix base32 alphabet (no `eotu`). Why: standard base32 would produce wrong hashes.
const NIX32: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Encode bytes as nix32 (Nix's base32). Why: port of `BaseNix32::encode`
/// from Nix `src/libutil/base-nix-32.cc` — groups count from the LSB end.
fn nix32_encode(bytes: &[u8]) -> String {
    let len = (bytes.len() * 8).div_ceil(5);
    let mut out = String::with_capacity(len);
    for n in (0..len).rev() {
        let b = n * 5;
        let i = b / 8;
        let j = b % 8;
        let lo = u32::from(bytes[i] >> j);
        let hi = if i + 1 < bytes.len() {
            u32::from(bytes[i + 1]) << (8 - j)
        } else {
            0
        };
        out.push(NIX32[(lo | hi) as usize & 0x1f] as char);
    }
    out
}

/// Basename of a store path. Why: narinfo `References`/`Deriver` use basenames.
#[must_use]
pub fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Fingerprint signed for `Sig:`. Why: exact Nix format or clients reject the signature.
#[must_use]
pub fn fingerprint(
    store_path: &str,
    nar_hash: &str,
    nar_size: u64,
    refs_basenames: &[String],
) -> String {
    format!(
        "1;{store_path};{nar_hash};{nar_size};{}",
        refs_basenames.join(",")
    )
}

/// Sign a fingerprint with one `SignKey`, returning `name:base64sig`.
pub fn sign_with(key_name: &str, seed: &[u8; 32], fingerprint: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer as _, SigningKey};
    let sig = SigningKey::from_bytes(seed).sign(fingerprint.as_bytes());
    format!("{key_name}:{}", STANDARD.encode(sig.to_bytes()))
}

/// Inputs for rendering a narinfo body. Why: one struct avoids 9-arg functions.
pub struct Narinfo<'a> {
    /// Full store path.
    pub store_path: &'a str,
    /// File hash (`sha256:<nix32>`).
    pub file_hash: &'a str,
    /// Compressed size (equals NAR size for `none`).
    pub file_size: u64,
    /// NAR hash (`sha256:<nix32>`).
    pub nar_hash: &'a str,
    /// Uncompressed NAR size.
    pub nar_size: u64,
    /// Reference basenames.
    pub refs: &'a [String],
    /// Deriver basename, if known.
    pub deriver: Option<&'a str>,
    /// `name:sig` entries.
    pub sigs: &'a [String],
    /// Content address, if known.
    pub ca: Option<&'a str>,
}

/// Render a narinfo body from path info + extra signatures.
/// Why: `FileHash/FileSize` equal NAR values for `Compression: none`.
/// Why: the URL embeds `<storehash>-` so `/nar/` can map back to the store
/// path without a reverse index (bare `<filehash>.nar` uploads serve from disk).
#[must_use]
pub fn render_narinfo(p: &Narinfo<'_>) -> String {
    use std::fmt::Write as _;
    let file_name = p
        .file_hash
        .split(':')
        .nth(1)
        .unwrap_or(p.file_hash)
        .replace('/', "-");
    let store_hash = p
        .store_path
        .rsplit('/')
        .next()
        .unwrap_or(p.store_path)
        .split('-')
        .next()
        .unwrap_or("");
    let mut out = format!(
        "StorePath: {}\nURL: nar/{store_hash}-{file_name}.nar\nCompression: none\nFileHash: {}\nFileSize: {}\nNarHash: {}\nNarSize: {}\n",
        p.store_path, p.file_hash, p.file_size, p.nar_hash, p.nar_size
    );
    if p.refs.is_empty() {
        out.push_str("References: \n");
    } else {
        let _ = writeln!(out, "References: {}", p.refs.join(" "));
    }
    let _ = writeln!(out, "Deriver: {}", p.deriver.unwrap_or("unknown-deriver"));
    for sig in p.sigs {
        let _ = writeln!(out, "Sig: {sig}");
    }
    if let Some(ca) = p.ca {
        let _ = writeln!(out, "CA: {ca}");
    }
    out
}

/// Minimal parsed narinfo for PUT validation.
#[derive(Debug, Default)]
pub struct ParsedNarinfo {
    /// Full `/nix/store/...` path.
    pub store_path: String,
    /// Relative `nar/...` URL.
    pub url: String,
    /// NAR hash in `sha256:<nix32>` form.
    pub nar_hash: String,
    /// NAR size.
    pub nar_size: u64,
    /// Reference basenames.
    pub references: Vec<String>,
}

/// Parse + validate an uploaded narinfo. Why: reject poisoned metadata early.
pub fn parse_narinfo(body: &str) -> Result<ParsedNarinfo, NixError> {
    let mut out = ParsedNarinfo::default();
    for line in body.lines() {
        let (k, v) = line
            .split_once(':')
            .ok_or_else(|| NixError::Decode(format!("bad line: {line}")))?;
        match k {
            "StorePath" => out.store_path = v.trim().to_string(),
            "URL" => out.url = v.trim().to_string(),
            "NarHash" => out.nar_hash = v.trim().to_string(),
            "NarSize" => {
                out.nar_size = v
                    .trim()
                    .parse()
                    .map_err(|_| NixError::Decode("bad NarSize".to_string()))?;
            }
            "References" => {
                out.references = v.split_whitespace().map(str::to_string).collect();
            }
            _ => {}
        }
    }
    if out.store_path.is_empty() || out.nar_hash.is_empty() || out.nar_size == 0 {
        return Err(NixError::Decode(
            "missing StorePath/NarHash/NarSize".to_string(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, nix32_encode, parse_narinfo, sri_to_narinfo};

    #[test]
    fn converts_known_hash() {
        // Why: `nix hash convert` says this base64 == this nix32.
        let narinfo =
            sri_to_narinfo("sha256-S0ymvFDCaeUfZSW1veq0gU12WoV80qZSXcgyllwCzZY=").unwrap();
        assert_eq!(
            narinfo,
            "sha256:15nd09f9ccn8bm9adlkwhmd7ckc1nkmbvd95clgyasf2a2yack2b"
        );
    }

    #[test]
    fn nix32_is_52_chars() {
        assert_eq!(nix32_encode(&[0_u8; 32]).len(), 52);
    }

    #[test]
    fn fingerprint_format() {
        let fp = fingerprint("/nix/store/h-foo", "sha256:abc", 10, &["a".to_string()]);
        assert_eq!(fp, "1;/nix/store/h-foo;sha256:abc;10;a");
    }

    #[test]
    fn parses_minimal_narinfo() {
        let body =
            "StorePath: /nix/store/abc-foo\nURL: nar/x.nar\nNarHash: sha256:abc\nNarSize: 10\n";
        let parsed = parse_narinfo(body).unwrap();
        assert_eq!(parsed.store_path, "/nix/store/abc-foo");
    }
}
