//! A CI run: one pushed commit whose flake checks are evaluated and built.
//! Why: the lifecycle rules (which status can follow which) live here, so
//! every store adapter applies the same ones.

use std::fmt;

/// Longest accepted `owner` or `name` segment of a repo name.
const MAX_SEGMENT: usize = 100;
/// Longest accepted git ref.
const MAX_REF: usize = 255;
/// Longest accepted pusher name.
const MAX_PUSHER: usize = 255;

/// Which value of a new run was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidRun {
    /// Not `owner/name` with safe characters.
    RepoName,
    /// Not a full SHA-1 or SHA-256 commit id.
    Commit,
    /// Not a `refs/...` name git would accept.
    GitRef,
    /// Empty, too long, or contains control characters.
    Pusher,
    /// A stored status string this version does not know.
    Status,
}

impl fmt::Display for InvalidRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RepoName => "repo must be `owner/name` using letters, digits, `.`, `_` or `-`",
            Self::Commit => "commit must be a full 40 or 64 character hex id",
            Self::GitRef => "ref must be a valid `refs/...` name",
            Self::Pusher => "pusher must be 1-255 printable characters",
            Self::Status => "unknown run status",
        })
    }
}

impl std::error::Error for InvalidRun {}

/// A repository identity, `owner/name`.
/// Why: it is the only repo identifier accepted from outside; the service
/// resolves it to a path itself, so no request can point at arbitrary paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoName {
    full: String,
    slash: usize,
}

impl RepoName {
    /// Validate an `owner/name` string.
    ///
    /// # Errors
    ///
    /// [`InvalidRun::RepoName`] unless it is exactly two segments of
    /// `[A-Za-z0-9._-]`, each 1-100 long and not starting with `.`.
    pub fn parse(full: String) -> Result<Self, InvalidRun> {
        let slash = full.find('/').ok_or(InvalidRun::RepoName)?;
        let (owner, name) = (&full[..slash], &full[slash + 1..]);
        if valid_segment(owner) && valid_segment(name) {
            Ok(Self { full, slash })
        } else {
            Err(InvalidRun::RepoName)
        }
    }

    /// The whole `owner/name`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.full
    }

    /// The owning user or organisation.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.full[..self.slash]
    }

    /// The repository's own name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.full[self.slash + 1..]
    }
}

/// Why: rejects `..`, hidden names and anything a shell or path could misread.
fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= MAX_SEGMENT
        && !segment.starts_with('.')
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A full commit id, lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitHash(String);

impl CommitHash {
    /// Validate a commit id (SHA-1 or SHA-256 repositories).
    ///
    /// # Errors
    ///
    /// [`InvalidRun::Commit`] unless it is 40 or 64 hex digits.
    pub fn parse(mut hex: String) -> Result<Self, InvalidRun> {
        let full_length = hex.len() == 40 || hex.len() == 64;
        if !full_length || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(InvalidRun::Commit);
        }
        hex.make_ascii_lowercase();
        Ok(Self(hex))
    }

    /// The commit id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A full git ref name, e.g. `refs/heads/main`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRef(String);

impl GitRef {
    /// Validate a ref name (a subset of `git check-ref-format`).
    ///
    /// # Errors
    ///
    /// [`InvalidRun::GitRef`] unless it starts with `refs/`, is at most 255
    /// bytes, and has no `..`, `//`, `@{`, control characters, spaces or
    /// any of `~^:?*[\`.
    // Why: git reserves exactly `.lock`; `.LOCK` is a legal ref suffix.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    pub fn parse(name: String) -> Result<Self, InvalidRun> {
        let forbidden = |b: u8| {
            b.is_ascii_control()
                || matches!(b, b' ' | b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        };
        let valid = name.len() <= MAX_REF
            && name.len() > "refs/".len()
            && name.starts_with("refs/")
            && !name.ends_with('/')
            && !name.ends_with(".lock")
            && !name.contains("..")
            && !name.contains("//")
            && !name.contains("@{")
            && !name.bytes().any(forbidden);
        if valid {
            Ok(Self(name))
        } else {
            Err(InvalidRun::GitRef)
        }
    }

    /// The full ref name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The branch name, when this ref is a branch.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.0.strip_prefix("refs/heads/")
    }
}

/// Who pushed, as reported by the git server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pusher(String);

impl Pusher {
    /// Validate a pusher name.
    ///
    /// # Errors
    ///
    /// [`InvalidRun::Pusher`] when empty, over 255 bytes, or containing
    /// control characters.
    pub fn parse(name: String) -> Result<Self, InvalidRun> {
        let valid =
            !name.is_empty() && name.len() <= MAX_PUSHER && !name.chars().any(char::is_control);
        if valid {
            Ok(Self(name))
        } else {
            Err(InvalidRun::Pusher)
        }
    }

    /// The pusher's name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A run's id, assigned by the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(i64);

impl RunId {
    /// Wrap a stored id.
    #[must_use]
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    /// The raw id.
    #[must_use]
    pub fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Where a run is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// Waiting for a worker.
    Queued,
    /// Claimed by a worker.
    Running,
    /// Every check passed, was cached, or was skipped.
    Passed,
    /// At least one check failed.
    Failed,
    /// The run hit its time limit.
    TimedOut,
    /// An infrastructure fault, not the repository's fault.
    Errored,
}

impl RunStatus {
    /// The stored and serialized spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Errored => "errored",
        }
    }

    /// Read a stored spelling.
    ///
    /// # Errors
    ///
    /// [`InvalidRun::Status`] for an unknown spelling.
    pub fn parse(stored: &str) -> Result<Self, InvalidRun> {
        match stored {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "timed_out" => Ok(Self::TimedOut),
            "errored" => Ok(Self::Errored),
            _ => Err(InvalidRun::Status),
        }
    }

    /// Whether a run in this status is over.
    #[must_use]
    pub fn is_final(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

/// What a push asks the service to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    repo: RepoName,
    commit: CommitHash,
    git_ref: GitRef,
    pusher: Option<Pusher>,
}

impl NewRun {
    /// Describe a requested run.
    #[must_use]
    pub fn new(
        repo: RepoName,
        commit: CommitHash,
        git_ref: GitRef,
        pusher: Option<Pusher>,
    ) -> Self {
        Self {
            repo,
            commit,
            git_ref,
            pusher,
        }
    }

    /// The repository.
    #[must_use]
    pub fn repo(&self) -> &RepoName {
        &self.repo
    }

    /// The commit to check.
    #[must_use]
    pub fn commit(&self) -> &CommitHash {
        &self.commit
    }

    /// The ref that was pushed.
    #[must_use]
    pub fn git_ref(&self) -> &GitRef {
        &self.git_ref
    }

    /// Who pushed, when known.
    #[must_use]
    pub fn pusher(&self) -> Option<&Pusher> {
        self.pusher.as_ref()
    }
}

/// A run's timestamps, unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunTimes {
    created: i64,
    started: Option<i64>,
    finished: Option<i64>,
}

impl RunTimes {
    /// Describe stored timestamps.
    #[must_use]
    pub fn new(created: i64, started: Option<i64>, finished: Option<i64>) -> Self {
        Self {
            created,
            started,
            finished,
        }
    }

    /// When the run was enqueued.
    #[must_use]
    pub fn created_at(self) -> i64 {
        self.created
    }

    /// When a worker last claimed it.
    #[must_use]
    pub fn started_at(self) -> Option<i64> {
        self.started
    }

    /// When it reached a final status.
    #[must_use]
    pub fn finished_at(self) -> Option<i64> {
        self.finished
    }
}

/// A stored run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    id: RunId,
    request: NewRun,
    status: RunStatus,
    times: RunTimes,
}

impl Run {
    /// A just-enqueued run.
    #[must_use]
    pub fn queued(id: RunId, request: NewRun, now: i64) -> Self {
        Self {
            id,
            request,
            status: RunStatus::Queued,
            times: RunTimes::new(now, None, None),
        }
    }

    /// Reassemble a stored run.
    #[must_use]
    pub fn from_parts(id: RunId, request: NewRun, status: RunStatus, times: RunTimes) -> Self {
        Self {
            id,
            request,
            status,
            times,
        }
    }

    /// The run's id.
    #[must_use]
    pub fn id(&self) -> RunId {
        self.id
    }

    /// What was pushed.
    #[must_use]
    pub fn request(&self) -> &NewRun {
        &self.request
    }

    /// Current status.
    #[must_use]
    pub fn status(&self) -> RunStatus {
        self.status
    }

    /// Timestamps.
    #[must_use]
    pub fn times(&self) -> RunTimes {
        self.times
    }

    /// Claim a queued run for a worker. `false` when it is not queued.
    pub fn start(&mut self, now: i64) -> bool {
        if self.status != RunStatus::Queued {
            return false;
        }
        self.status = RunStatus::Running;
        self.times.started = Some(now);
        true
    }

    /// End a running run. `false` when it is not running or `status` is not
    /// final.
    pub fn finish(&mut self, status: RunStatus, now: i64) -> bool {
        if self.status != RunStatus::Running || !status.is_final() {
            return false;
        }
        self.status = status;
        self.times.finished = Some(now);
        true
    }

    /// Put a running run back in the queue (its worker died). `false` when
    /// it is not running.
    pub fn requeue(&mut self) -> bool {
        if self.status != RunStatus::Running {
            return false;
        }
        self.status = RunStatus::Queued;
        self.times.started = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{CommitHash, GitRef, InvalidRun, NewRun, Pusher, RepoName, Run, RunId, RunStatus};

    fn request() -> NewRun {
        NewRun::new(
            RepoName::parse(String::from("lucas/dummy")).unwrap(),
            CommitHash::parse("a".repeat(40)).unwrap(),
            GitRef::parse(String::from("refs/heads/main")).unwrap(),
            None,
        )
    }

    #[test]
    fn repo_name_splits_owner_and_name() {
        let repo = RepoName::parse(String::from("acme/infra.nix")).unwrap();
        assert_eq!(repo.owner(), "acme");
        assert_eq!(repo.name(), "infra.nix");
        assert_eq!(repo.as_str(), "acme/infra.nix");
    }

    #[test]
    fn repo_name_rejects_paths_and_odd_characters() {
        for bad in [
            "dummy",
            "/dummy",
            "lucas/",
            "../etc",
            "lucas/..",
            "lucas/.git",
            "a/b/c",
            "lucas/du mmy",
            "lucas/dummy;rm",
            "/var/lib/git-server/lucas/dummy.git",
        ] {
            assert_eq!(
                RepoName::parse(String::from(bad)),
                Err(InvalidRun::RepoName),
                "{bad}"
            );
        }
    }

    #[test]
    fn commit_accepts_full_ids_only() {
        let upper = CommitHash::parse("ABCDEF0123".repeat(4)).unwrap();
        assert_eq!(upper.as_str(), "abcdef0123".repeat(4));
        assert!(CommitHash::parse("0".repeat(64)).is_ok());
        assert!(CommitHash::parse(String::from("abc123")).is_err());
        assert!(CommitHash::parse("g".repeat(40)).is_err());
    }

    #[test]
    fn git_ref_validation_and_branch() {
        let main = GitRef::parse(String::from("refs/heads/feature/x")).unwrap();
        assert_eq!(main.branch(), Some("feature/x"));
        let tag = GitRef::parse(String::from("refs/tags/v1")).unwrap();
        assert_eq!(tag.branch(), None);
        for bad in [
            "main",
            "refs/",
            "refs/heads/a..b",
            "refs/heads/a b",
            "refs/heads/x.lock",
            "refs/heads/",
            "refs/heads/a@{1}",
        ] {
            assert!(GitRef::parse(String::from(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn pusher_validation() {
        assert!(Pusher::parse(String::from("git")).is_ok());
        assert!(Pusher::parse(String::new()).is_err());
        assert!(Pusher::parse(String::from("a\nb")).is_err());
        assert!(Pusher::parse("x".repeat(256)).is_err());
    }

    #[test]
    fn status_round_trips() {
        for status in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Passed,
            RunStatus::Failed,
            RunStatus::TimedOut,
            RunStatus::Errored,
        ] {
            assert_eq!(RunStatus::parse(status.as_str()), Ok(status));
        }
        assert_eq!(RunStatus::parse("success"), Err(InvalidRun::Status));
    }

    #[test]
    fn lifecycle_only_allows_valid_transitions() {
        let mut run = Run::queued(RunId::new(1), request(), 10);
        assert!(!run.finish(RunStatus::Passed, 11), "queued cannot finish");
        assert!(!run.requeue(), "queued cannot requeue");

        assert!(run.start(12));
        assert!(!run.start(13), "running cannot start again");
        assert_eq!(run.times().started_at(), Some(12));

        assert!(run.requeue());
        assert_eq!(run.status(), RunStatus::Queued);
        assert_eq!(run.times().started_at(), None);

        assert!(run.start(14));
        assert!(!run.finish(RunStatus::Running, 15), "running is not final");
        assert!(run.finish(RunStatus::Failed, 16));
        assert_eq!(run.status(), RunStatus::Failed);
        assert_eq!(run.times().finished_at(), Some(16));
        assert!(!run.start(17), "finished cannot restart");
    }
}
