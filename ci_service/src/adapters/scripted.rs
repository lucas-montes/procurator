//! A [`Builder`] and a [`ConfigSource`] that replay a script. Why: use-case
//! and HTTP tests run the whole pipeline without git or Nix.

use std::future::Future;
use std::sync::{Mutex, PoisonError};

use tokio::sync::mpsc;

use crate::application::ports::{
    BuildEvent, Builder, BuilderError, ConfigSource, Evaluated, RepoConfigError,
};
use crate::domain::check::{Derivation, Systems};
use crate::domain::repo_config::{CheckSelection, RunSettings, Timeouts};
use crate::domain::run::NewRun;

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

    /// Builds never finish.
    #[must_use]
    pub fn hanging(mut self) -> Self {
        self.hang = true;
        self
    }

    /// The `.drv` paths asked to be built, in order, one per try.
    pub fn built(&self) -> Vec<String> {
        lock(&self.built).clone()
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
    ) -> impl Future<Output = Result<Vec<Evaluated>, BuilderError>> + Send {
        lock(&self.asked).push((selection.clone(), None));
        let evaluation = lock(&self.evaluation)
            .take()
            .unwrap_or_else(|| Err(BuilderError::Unavailable(String::from("script over"))));
        std::future::ready(evaluation)
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
}

/// A [`ConfigSource`] that always answers the same.
#[derive(Debug)]
pub struct FixedConfig(Result<RunSettings, String>);

impl FixedConfig {
    /// Every run gets `settings`.
    #[must_use]
    pub fn settings(settings: RunSettings) -> Self {
        Self(Ok(settings))
    }

    /// Every run's `procurator.nix` is invalid for `reason`.
    #[must_use]
    pub fn invalid(reason: &str) -> Self {
        Self(Err(String::from(reason)))
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
}
