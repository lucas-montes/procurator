//! Run logs: every line a run produced, tagged with where it came from.
//! Why: lines are keyed by derivation, not by check, so a dependency's build
//! log is stored once and can later be shown with every check that needs it.

/// Where a log line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOrigin<'a> {
    /// The run itself (evaluation, Nix messages).
    Run,
    /// A check, by attribute (its evaluation error, why it was skipped).
    Check(&'a str),
    /// A derivation's build output, by `.drv` path.
    Derivation(&'a str),
}

impl<'a> LogOrigin<'a> {
    /// One string per origin, for storage.
    /// Why: the three never collide — run is empty, derivations are absolute
    /// store paths, attributes never start with `/`.
    #[must_use]
    pub fn key(self) -> &'a str {
        match self {
            Self::Run => "",
            Self::Check(attr) => attr,
            Self::Derivation(path) => path,
        }
    }

    /// Read a stored key back.
    #[must_use]
    pub fn from_key(key: &'a str) -> Self {
        if key.is_empty() {
            Self::Run
        } else if key.starts_with('/') {
            Self::Derivation(key)
        } else {
            Self::Check(key)
        }
    }

    /// A short label for showing lines from many origins together:
    /// the derivation name without its hash, the attribute, or nothing.
    #[must_use]
    pub fn label(self) -> &'a str {
        match self {
            Self::Run => "",
            Self::Check(attr) => attr,
            Self::Derivation(path) => {
                let file = path.rsplit('/').next().unwrap_or(path);
                let name = file.split_once('-').map_or(file, |(_, name)| name);
                name.strip_suffix(".drv").unwrap_or(name)
            }
        }
    }
}

/// One line to store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewLogLine<'a> {
    origin: LogOrigin<'a>,
    text: &'a str,
}

impl<'a> NewLogLine<'a> {
    /// Describe a line.
    #[must_use]
    pub fn new(origin: LogOrigin<'a>, text: &'a str) -> Self {
        Self { origin, text }
    }

    /// Where it came from.
    #[must_use]
    pub fn origin(&self) -> LogOrigin<'a> {
        self.origin
    }

    /// The line, without a trailing newline.
    #[must_use]
    pub fn text(&self) -> &'a str {
        self.text
    }
}

/// A stored line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    origin: String,
    text: String,
}

impl LogLine {
    /// Reassemble a stored line from its origin key and text.
    #[must_use]
    pub fn new(origin: String, text: String) -> Self {
        Self { origin, text }
    }

    /// Where it came from.
    #[must_use]
    pub fn origin(&self) -> LogOrigin<'_> {
        LogOrigin::from_key(&self.origin)
    }

    /// The line.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use super::LogOrigin;

    #[test]
    fn keys_round_trip() {
        for origin in [
            LogOrigin::Run,
            LogOrigin::Check("checks.x86_64-linux.unit"),
            LogOrigin::Derivation("/nix/store/abc-dep.drv"),
        ] {
            assert_eq!(LogOrigin::from_key(origin.key()), origin);
        }
    }

    #[test]
    fn labels_are_short() {
        assert_eq!(
            LogOrigin::Derivation("/nix/store/ik38l896-shared-dep.drv").label(),
            "shared-dep"
        );
        assert_eq!(LogOrigin::Check("checks.x.a").label(), "checks.x.a");
        assert_eq!(LogOrigin::Run.label(), "");
    }
}
