//! The router's state: the service, shared by every request.

use std::sync::Arc;

use crate::application::Service;

/// A handle on the service that every handler receives.
pub struct Shared<S, C, B, R, N>(Arc<Service<S, C, B, R, N>>);

impl<S, C, B, R, N> Shared<S, C, B, R, N> {
    /// Share `service`.
    pub fn new(service: Arc<Service<S, C, B, R, N>>) -> Self {
        Self(service)
    }

    /// The service.
    pub fn service(&self) -> &Service<S, C, B, R, N> {
        &self.0
    }
}

/// Why: by hand, so the ports need not be `Clone`; only the `Arc` is.
impl<S, C, B, R, N> Clone for Shared<S, C, B, R, N> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
