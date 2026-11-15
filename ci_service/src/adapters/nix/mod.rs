//! Nix adapters: the [`crate::application::ports::Builder`] over the local
//! Nix daemon and its remote builders, and the
//! [`crate::application::ports::ConfigSource`] for `procurator.nix`.

mod builder;
mod config_source;
mod internal_json;
mod repos;
mod systems;

pub use builder::NixBuilder;
pub use config_source::GitConfigSource;
