#[derive(Debug)]
pub enum Error {
    Communication(String),
    OperationFailed(String),
    Serialization(serde_json::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Communication(msg) => write!(f, "Communication error: {msg}"),
            Error::OperationFailed(msg) => write!(f, "Operation failed: {msg}"),
            Error::Serialization(err) => write!(f, "Serialization error: {err}"),
            Error::Io(err) => write!(f, "IO error: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Serialization(err) => Some(err),
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Serialization(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}
