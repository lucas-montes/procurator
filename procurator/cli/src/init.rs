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

    let parser = Parser::from(path).scan().analyse().build();

    save_and_log(|p| parser.as_json(p), &config_path);
    save_and_log(|p| parser.as_nix(p), &config_path.with_extension("nix"));
    save_and_log(|p| parser.generate(p), &config_path.with_file_name("flake.nix"));

    // parser
    //     .as_json(&config_path)
    //     .expect("Failed to save configuration as json");

    // parser
    //     .as_nix(&config_path.with_extension("nix"))
    //     .expect("Failed to save configuration as nix");

    // parser
    //     .generate(&config_path.with_file_name("flake.nix"))
    //     .expect("Failed to save flake.nix");
}

fn save_and_log<F: Fn(&PathBuf) -> std::io::Result<()>>(f: F, p: &PathBuf) {
    f(p).expect("Failed to save configuration as json");
    tracing::info!("Configuration saved to {}", p.display());
}
