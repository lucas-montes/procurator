//! Procurator artifact cache as an embeddable library.
//!
//! Hexagonal layout: [`domain`] holds the pure rules, [`application`] the use
//! cases and the ports they need, [`adapters`] the implementations (HTTP in,
//! disk/SQLite/memory out). The `cache` binary is one composition of these.

// Why: project rule — async functions return `impl Future + Send` explicitly,
// so the `Send` bound embedders rely on is part of every signature.
#![allow(clippy::manual_async_fn)]

pub mod adapters;
pub mod application;
pub mod config;
pub mod domain;
