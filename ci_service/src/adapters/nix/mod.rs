//! Nix adapter: the [`crate::application::ports::Builder`] over the local
//! Nix daemon and its remote builders.

mod builder;
mod internal_json;
mod systems;

pub use builder::NixBuilder;
