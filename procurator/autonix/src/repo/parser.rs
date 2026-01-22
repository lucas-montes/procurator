// The parser module looks for the configuration of a repository. Think of it as a mix between
// railpack and direnv.
use std::path::PathBuf;

use super::{analysis::Analysis, scan::Scan};

#[derive(Debug)]
pub struct Parser<T = PathBuf>(T);

impl From<PathBuf> for Parser {
    fn from(path: PathBuf) -> Self {
        tracing::info!("Parsing repository: {path:?}");
        Self(path)
    }
}

impl Parser<PathBuf> {
    pub fn scan(self) -> Parser<Scan> {
        Parser(Scan::from(self.0))
    }
}

impl Parser<Scan> {
    pub fn analyse(self) -> Parser<Analysis> {
        Parser(Analysis::from(self.0.into_iter()))
    }

    //     pub fn save(&self, path: &Path) -> std::io::Result<()> {
    //         let json = serde_json::to_string_pretty(&self.0)?;
    //         std::fs::write(path, json)?;
    //         tracing::info!("Saved configuration to {path:?}");
    //         Ok(())
    //     }

    //     pub fn print(&self){
    //         tracing::info!(?self, "Intermediate Representation:");
    //         tracing::info!("Detected {} projects", self.0.projects.len());
    //         for config in &self.0.projects {
    //             tracing::info!(
    //                 "  - {} ({:?}, {:?})",
    //                 config.name,
    //                 config.toolchain.language,
    //                 config.toolchain.package_manager
    //             );
    //         }
    //     }

    //     pub fn load(path: &Path) -> std::io::Result<Self> {
    //         let json = std::fs::read(path)?;
    //         let ir = serde_json::from_slice(&json)?;
    //         tracing::info!("Loaded configuration from {path:?}");
    //         Ok(Parser(ir))
    //     }
}

struct Configuration;

impl Parser<Analysis> {
    fn represent(self) -> Parser<Configuration> {
        todo!()
    }
}
