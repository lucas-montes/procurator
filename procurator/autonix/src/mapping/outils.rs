use std::path::Path;

/// Trait for parsing manifest files
pub trait Parseable {
    type Output;

    fn parse(&self, path: &Path) -> Result<Self::Output, ParseError>;
}

/// Common error type for parsing
#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Toml(toml::de::Error),
    Json(serde_json::Error),
    Yaml(serde_yaml_ng::Error),
    InvalidFormat(String),
}

impl From<std::io::Error> for ParseError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<toml::de::Error> for ParseError {
    fn from(err: toml::de::Error) -> Self {
        Self::Toml(err)
    }
}

impl From<serde_json::Error> for ParseError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<serde_yaml_ng::Error> for ParseError {
    fn from(err: serde_yaml_ng::Error) -> Self {
        Self::Yaml(err)
    }
}

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Version(pub Option<String>);


impl From<Option<&str>> for Version {
    fn from(s: Option<&str>) -> Self {
        Self(s.map(|s| s.to_string()))
    }
}

impl Default for Version {
    fn default() -> Self {
        Self(None)
    }
}
