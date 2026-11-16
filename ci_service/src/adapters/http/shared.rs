//! The router's state: the service, shared by every request.

use std::sync::Arc;

use crate::application::Service;

/// A handle on the service that every handler receives.
pub struct Shared<S, C, B, R, N, A>(Arc<Service<S, C, B, R, N, A>>);

impl<S, C, B, R, N, A> Shared<S, C, B, R, N, A> {
    /// Share `service`.
    pub fn new(service: Arc<Service<S, C, B, R, N, A>>) -> Self {
        Self(service)
    }

    /// The service.
    pub fn service(&self) -> &Service<S, C, B, R, N, A> {
        &self.0
    }
}

/// Why: by hand, so the ports need not be `Clone`; only the `Arc` is.
impl<S, C, B, R, N, A> Clone for Shared<S, C, B, R, N, A> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
