// Enums mapping from files to their respective types in the Autonix system.

mod buildfiles;
mod lockfiles;
mod manifests;
mod languages;
mod outils;
mod containers;
mod cicdfiles;

pub use cicdfiles::{CiCdFile, CiJob, CiService, CiStep, ParsedCiCdFile};
pub use containers::{ContainerFile, ParsedContainerFile};
pub use languages::{Language, PackageManager};
pub use buildfiles::{BuildFile, BuildSystem, ParsedBuildFile};
pub use lockfiles::LockFile;
pub use manifests::{ManifestFile, ParsedManifest};
pub use outils::{ParseError, Parseable};
