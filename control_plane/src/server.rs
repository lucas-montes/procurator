//! Central point of communication. Talks to workers and receives requests from the cli.
use worker::server::{create as create_w, delete as delete_w};

pub fn create(name: String) {
    create_w(name);
}
pub fn delete(name: String) {
    delete_w(name);
}
