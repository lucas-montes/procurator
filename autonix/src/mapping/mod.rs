// Enums mapping from files to their respective types in the Autonix system.

mod cicdfiles;
mod containers;
mod languages;
mod lockfiles;
mod manifests;
mod outils;
mod tasks;
mod version;

pub use cicdfiles::{CiCdFile, CiJob, CiService, CiStep, ParsedCiCdFile};
pub use containers::{ContainerFile, ContainerService, ParsedContainerFile};
pub use languages::{Language, PackageManager};
pub use lockfiles::LockFile;
pub use manifests::{ManifestFile, ParsedManifest};
pub use outils::{ParseError, Parseable};
pub use tasks::{BuildSystem, ParsedTaskFile, TaskFile};
pub use version::{SemVerParser, Version};
