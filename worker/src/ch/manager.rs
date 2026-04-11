//! Cloud Hypervisor VMM backend implementation.
//! It does the requests to the CH API and translates between our internal config and the CH API.

use std::collections::HashMap;
use std::path::PathBuf;

use futures::stream::TryStreamExt;
use hyper::Uri;
use hyperlocal::{UnixClientExt, Uri as UnixUri};
use rtnetlink;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use super::{config::VmConfig, errors::Error};

pub struct Manager {
    /// Base HTTP client configured for unix socket communication
    base_client: hyper::Client<hyperlocal::UnixConnector>,
    socket_path: PathBuf,
}

impl Manager {
    /// Create a new Manager VMM instance
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        let base_client = hyper::Client::unix();

        Self {
            socket_path: socket_path.into(),
            base_client,
        }
    }
}
