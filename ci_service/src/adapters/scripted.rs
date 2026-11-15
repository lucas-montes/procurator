//! A [`Builder`] that replays a script. Why: use-case and HTTP tests run the
//! whole pipeline without Nix.

use std::future::Future;
use std::sync::{Mutex, PoisonError};

use tokio::sync::mpsc;

use crate::application::ports::{BuildEvent, Builder, BuilderError, Evaluated};
use crate::domain::check::{Derivation, Systems};
use crate::domain::run::NewRun;

/// What the builder answers, and what it was asked.
#[derive(Debug, Default)]
pub struct ScriptedBuilder {
    systems: Vec<String>,
    evaluation: Mutex<Option<Result<Vec<Evaluated>, BuilderError>>>,
    events: Vec<BuildEvent>,
    /// `.drv` paths whose build passes.
    passing: Vec<String>,
    built: Mutex<Vec<String>>,
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

    /// Send these events during the build.
    #[must_use]
    pub fn with_events(mut self, events: Vec<BuildEvent>) -> Self {
        self.events = events;
        self
    }

    /// Builds of these `.drv` paths pass; every other build fails.
    #[must_use]
    pub fn passing(mut self, drvs: &[&str]) -> Self {
        self.passing = drvs.iter().map(|&drv| String::from(drv)).collect();
        self
    }

    /// The `.drv` paths asked to be built, in order.
    pub fn built(&self) -> Vec<String> {
        self.built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
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
    ) -> impl Future<Output = Result<Vec<Evaluated>, BuilderError>> + Send {
        let evaluation = self
            .evaluation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Err(BuilderError::Unavailable(String::from("script over"))));
        std::future::ready(evaluation)
    }

    fn build(
        &self,
        targets: &[&Derivation],
        events: mpsc::Sender<BuildEvent>,
    ) -> impl Future<Output = Result<Vec<bool>, BuilderError>> + Send {
        self.built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(targets.iter().map(|drv| String::from(drv.path())));
        let results = targets
            .iter()
            .map(|drv| self.passing.iter().any(|passing| passing == drv.path()))
            .collect();
        let script = self.events.clone();
        async move {
            for event in script {
                events.send(event).await.unwrap();
            }
            Ok(results)
        }
    }
}
