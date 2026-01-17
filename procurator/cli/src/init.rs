// Parse a repository and initialize it's configurations
// TODO: We want to leverage direnv also so we need to check if .envrc exists and if it's configured
use std::{env, path::PathBuf};

use autonix::Parser;

pub fn init(path: Option<PathBuf>) {
    tracing::info!("Running autonix");
    let get_current_path = || env::current_dir().expect("Failed to get current directory");
    let path = path
        .unwrap_or_else(get_current_path)
        .canonicalize()
        .expect("Failed to canonicalize path");

    //TODO: we probably want to try to load config before creating it each time
    let config_path = path.join(".procurator").join("config.json");
    if let Err(err) = std::fs::create_dir_all(config_path.parent().expect("this is a batman dir")) {
        tracing::error!(?err, "Failed to create .procurator directory");
        return;
    };

    // let parser = Parser::from(path).advance().advance().advance();

    // println!("Generated configuration:");
    // parser.print();

    // parser
    //     .save(&config_path)
    //     .expect("Failed to save configuration");

    tracing::info!("Configuration saved to {}", config_path.display());
}
