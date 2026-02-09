// The parser module looks for the configuration of a repository. Think of it as a mix between
// railpack and direnv.
use std::path::PathBuf;

use super::{analysis::Analysis,flake::Configuration, scan::Scan};

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
}


impl Parser<Analysis> {
    pub fn build<'write>(self) -> Parser<Configuration<'write>> {
        //TODO: maybe we want to pass an iterator? not sure that we want to do the merging in the iterator
        Parser(Configuration::from(self.0))
    }

}

impl Parser<Configuration<'_>> {
    pub fn generate(&self, output: &PathBuf) -> std::io::Result<()> {
        let flake = self.0.to_nix().expect("");
        std::fs::write(output, flake)
    }

    pub fn as_nix(&self, output: &PathBuf) -> std::io::Result<()> {
        let flake = ser_nix::to_string(&self.0).expect("");
        std::fs::write(output, flake)
    }

    pub fn as_json(&self, output: &PathBuf) -> std::io::Result<()> {
        let flake = serde_json::to_string_pretty(&self.0).expect("");
        std::fs::write(output, flake)
    }

    //     pub fn load(path: &Path) -> std::io::Result<Self> {
    //         let json = std::fs::read(path)?;
    //         let ir = serde_json::from_slice(&json)?;
    //         tracing::info!("Loaded configuration from {path:?}");
    //         Ok(Parser(ir))
    //     }
}
