//! A [`Builder`] and a [`ConfigSource`] that replay a script. Why: use-case
//! and HTTP tests run the whole pipeline without git or Nix.

use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::application::ports::{
    Access, AccessError, BuildEvent, Builder, BuilderError, ConfigSource, DeliveryError, Evaluated,
    Notifier, RepoConfigError,
};
use crate::domain::access::{Credential, Principal};
use crate::domain::check::{Derivation, Systems};
use crate::domain::event::{EventPayload, Target};
use crate::domain::repo_config::{CheckSelection, RepoSettings, RunSettings, Timeouts};
use crate::domain::run::{NewRun, RepoName};

/// What the builder answers, and what it was asked.
#[derive(Debug, Default)]
pub struct ScriptedBuilder {
    systems: Vec<String>,
    evaluation: Mutex<Option<Result<Vec<Evaluated>, BuilderError>>>,
    events: Vec<BuildEvent>,
    /// `.drv` paths and the try (1-based) from which their build passes.
    passing: Vec<(String, usize)>,
    /// Builds never finish. Why: tests of the run timeout.
    hang: bool,
    built: Mutex<Vec<String>>,
    asked: Mutex<Vec<(CheckSelection, Option<Timeouts>)>>,
    pushed: Mutex<Vec<(String, Vec<String>)>>,
    /// `.drv` paths and the derivations they need.
    requisites: Vec<(String, Vec<String>)>,
    /// Store paths and their contents, for `read_output`.
    files: Vec<(String, String)>,
    /// Whether each evaluation asked for the deployment.
    deployments: Mutex<Vec<bool>>,
}

impl ScriptedBuilder {
    /// A builder for `systems` that evaluates to `evaluation` once.
    #[must_use]
    pub fn new(systems: &[&str], evaluation: Result<Vec<Evaluated>, BuilderError>) -> Self {
        Self {
            systems: systems.iter().map(|&system| String::from(system)).collect(),
            evaluation: Mutex::new(Some(evaluation)),
            ..Self::default()
        }
    }

    /// Send these events during every build.
    #[must_use]
    pub fn with_events(mut self, events: Vec<BuildEvent>) -> Self {
        self.events = events;
        self
    }

    /// Builds of these `.drv` paths pass; every other build fails.
    #[must_use]
    pub fn passing(mut self, drvs: &[&str]) -> Self {
        self.passing
            .extend(drvs.iter().map(|&drv| (String::from(drv), 1)));
        self
    }

    /// Builds of `drv` fail until try `from`, then pass.
    #[must_use]
    pub fn passing_from(mut self, drv: &str, from: usize) -> Self {
        self.passing.push((String::from(drv), from));
        self
    }

    /// `drv` needs `deps` built first.
    #[must_use]
    pub fn needing(mut self, drv: &str, deps: &[&str]) -> Self {
        let deps = deps.iter().map(|&dep| String::from(dep)).collect();
        self.requisites.push((String::from(drv), deps));
        self
    }

    /// The store path `path` holds `contents`.
    #[must_use]
    pub fn with_file(mut self, path: &str, contents: &str) -> Self {
        self.files
            .push((String::from(path), String::from(contents)));
        self
    }

    /// Per evaluation, whether it asked for the deployment.
    pub fn deployments_asked(&self) -> Vec<bool> {
        lock(&self.deployments).clone()
    }

    /// Builds never finish.
    #[must_use]
    pub fn hanging(mut self) -> Self {
        self.hang = true;
        self
    }

    /// What the next evaluation answers. Why: tests of a second run.
    pub fn evaluation_slot(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<Result<Vec<Evaluated>, BuilderError>>> {
        lock(&self.evaluation)
    }

    /// The `.drv` paths asked to be built, in order, one per try.
    pub fn built(&self) -> Vec<String> {
        lock(&self.built).clone()
    }

    /// Each push: the cache and the derivations.
    pub fn pushed(&self) -> Vec<(String, Vec<String>)> {
        lock(&self.pushed).clone()
    }

    /// The selection evaluated and the timeouts of each build, in order.
    pub fn asked(&self) -> Vec<(CheckSelection, Option<Timeouts>)> {
        lock(&self.asked).clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A derivation check result, for scripts.
#[must_use]
pub fn derivation(attr: &str, system: &str, drv: &str, cached: bool) -> Evaluated {
    Evaluated::Derivation {
        attr: String::from(attr),
        system: String::from(system),
        derivation: Derivation::new(String::from(drv), vec![format!("{drv}-out")]),
        cached,
    }
}

impl Builder for ScriptedBuilder {
    fn systems(&self) -> impl Future<Output = Result<Systems, BuilderError>> + Send {
        std::future::ready(Ok(Systems::new(self.systems.clone())))
    }

    fn evaluate(
        &self,
        _run: &NewRun,
        selection: &CheckSelection,
        deployment: bool,
    ) -> impl Future<Output = Result<Vec<Evaluated>, BuilderError>> + Send {
        lock(&self.asked).push((selection.clone(), None));
        lock(&self.deployments).push(deployment);
        let evaluation = lock(&self.evaluation)
            .take()
            .unwrap_or_else(|| Err(BuilderError::Unavailable(String::from("script over"))));
        std::future::ready(evaluation)
    }

    fn read_output(
        &self,
        path: &str,
        limit: u64,
    ) -> impl Future<Output = Result<String, BuilderError>> + Send {
        let found = self
            .files
            .iter()
            .find(|(known, _)| known == path)
            .map(|(_, contents)| contents.clone());
        let answer = match found {
            None => Err(BuilderError::Unavailable(format!("{path}: no such path"))),
            Some(contents) if u64::try_from(contents.len()).unwrap_or(u64::MAX) > limit => Err(
                BuilderError::Flake(format!("{path}: larger than {limit} bytes")),
            ),
            Some(contents) => Ok(contents),
        };
        std::future::ready(answer)
    }

    fn requisites(
        &self,
        drv: &Derivation,
    ) -> impl Future<Output = Result<Vec<String>, BuilderError>> + Send {
        let deps = self
            .requisites
            .iter()
            .find(|(path, _)| path == drv.path())
            .map(|(_, deps)| deps.clone())
            .unwrap_or_default();
        std::future::ready(Ok(deps))
    }

    fn build(
        &self,
        targets: &[&Derivation],
        timeouts: Timeouts,
        events: mpsc::Sender<BuildEvent>,
    ) -> impl Future<Output = Result<Vec<bool>, BuilderError>> + Send {
        lock(&self.asked).push((CheckSelection::All, Some(timeouts)));
        let mut built = lock(&self.built);
        built.extend(targets.iter().map(|drv| String::from(drv.path())));
        let results = targets
            .iter()
            .map(|drv| {
                let tries = built.iter().filter(|path| *path == drv.path()).count();
                self.passing
                    .iter()
                    .any(|(passing, from)| passing == drv.path() && tries >= *from)
            })
            .collect();
        drop(built);
        let script = self.events.clone();
        let hang = self.hang;
        async move {
            for event in script {
                events.send(event).await.unwrap();
            }
            if hang {
                std::future::pending::<()>().await;
            }
            Ok(results)
        }
    }

    fn push(
        &self,
        drvs: &[&str],
        cache: &str,
    ) -> impl Future<Output = Result<usize, BuilderError>> + Send {
        let drvs: Vec<String> = drvs.iter().map(|&drv| String::from(drv)).collect();
        let count = drvs.len();
        lock(&self.pushed).push((String::from(cache), drvs));
        std::future::ready(Ok(count))
    }
}

/// A [`Notifier`] that records deliveries and fails for chosen targets.
#[derive(Debug, Default)]
pub struct RecordingNotifier {
    down: Vec<Target>,
    delivered: Mutex<Vec<(Target, EventPayload)>>,
}

impl RecordingNotifier {
    /// Deliveries to `down` always fail.
    #[must_use]
    pub fn failing_for(down: Vec<Target>) -> Self {
        Self {
            down,
            ..Self::default()
        }
    }

    /// What arrived, in order.
    pub fn delivered(&self) -> Vec<(Target, EventPayload)> {
        lock(&self.delivered).clone()
    }
}

impl Notifier for RecordingNotifier {
    fn deliver(
        &self,
        target: &Target,
        payload: &EventPayload,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        let result = if self.down.contains(target) {
            Err(DeliveryError::new("connection refused"))
        } else {
            lock(&self.delivered).push((target.clone(), payload.clone()));
            Ok(())
        };
        std::future::ready(result)
    }

    /// Two retries, a minute apart.
    fn retry_delay(&self, _target: &Target, attempts: u32) -> Option<Duration> {
        (attempts < 3).then_some(Duration::from_secs(60))
    }
}

/// An [`Access`] where each token is one user, and each user reads only
/// the repos listed for them. Anonymous callers read nothing.
#[derive(Debug, Default)]
pub struct TokenAccess {
    users: Vec<(String, String, Vec<String>)>,
}

impl TokenAccess {
    /// `token` is `user`, who may read `repos` (`owner/name`).
    #[must_use]
    pub fn with_user(mut self, token: &str, user: &str, repos: &[&str]) -> Self {
        let repos = repos.iter().map(|&repo| String::from(repo)).collect();
        self.users
            .push((String::from(token), String::from(user), repos));
        self
    }
}

impl Access for TokenAccess {
    fn authenticate(
        &self,
        credential: Credential<'_>,
    ) -> impl Future<Output = Result<Principal, AccessError>> + Send {
        let principal = match credential {
            Credential::None => Ok(Principal::Anonymous),
            Credential::Bearer(token) => self
                .users
                .iter()
                .find(|(known, _, _)| known == token)
                .map(|(_, user, _)| Principal::User(user.clone()))
                .ok_or(AccessError::Unauthenticated),
        };
        std::future::ready(principal)
    }

    fn can_read(
        &self,
        principal: &Principal,
        repo: &RepoName,
    ) -> impl Future<Output = Result<bool, AccessError>> + Send {
        let allowed = match principal {
            Principal::Anonymous => false,
            Principal::User(name) => self.users.iter().any(|(_, user, repos)| {
                user == name && repos.iter().any(|allowed| allowed == repo.as_str())
            }),
        };
        std::future::ready(Ok(allowed))
    }
}

/// A [`ConfigSource`] that always answers the same.
#[derive(Debug)]
pub struct FixedConfig(Result<RunSettings, String>, RepoSettings);

impl FixedConfig {
    /// Every run gets `settings`.
    #[must_use]
    pub fn settings(settings: RunSettings) -> Self {
        Self(Ok(settings), RepoSettings::default())
    }

    /// Every run's `procurator.nix` is invalid for `reason`.
    #[must_use]
    pub fn invalid(reason: &str) -> Self {
        Self(Err(String::from(reason)), RepoSettings::default())
    }

    /// Every repo's default branch has these repo-level settings.
    #[must_use]
    pub fn with_repo(mut self, repo: RepoSettings) -> Self {
        self.1 = repo;
        self
    }
}

impl Default for FixedConfig {
    fn default() -> Self {
        Self::settings(RunSettings::default())
    }
}

impl ConfigSource for FixedConfig {
    fn run_settings(
        &self,
        _run: &NewRun,
    ) -> impl Future<Output = Result<RunSettings, RepoConfigError>> + Send {
        let answer = match &self.0 {
            Ok(settings) => Ok(settings.clone()),
            Err(reason) => Err(RepoConfigError::Invalid(reason.clone())),
        };
        std::future::ready(answer)
    }

    fn repo_settings(
        &self,
        _repo: &RepoName,
    ) -> impl Future<Output = Result<RepoSettings, RepoConfigError>> + Send {
        std::future::ready(Ok(self.1.clone()))
    }
}
