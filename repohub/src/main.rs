//! Repohub - Git Repository Management Platform
//!
//! A platform for managing projects and their associated repositories.
//!
//! ## Architecture
//!
//! - **Users**: Can create and own multiple projects
//! - **Projects**: Collections of repositories (like an organization)
//! - **Repositories**: Individual Git repositories within a project
//! - **Collaboration**: Multiple users can collaborate on projects

use std::path::PathBuf;

use axum::Router;
use tracing::info;

use repohub::config::Config;

/// Resolve `--config <path>`. Why: CLI arg only — no env pointer, no fixed path.
fn config_path() -> Result<PathBuf, String> {
    const USAGE: &str = "usage: repohub --config <path.toml>";
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--config") => args
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| format!("{USAGE} (missing path after --config)")),
        Some(other) => Err(format!("{USAGE} (got {other:?})")),
        None => Err(USAGE.to_string()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Why: usage errors are distinct from config errors: they exit 2 with a
    // one-line usage message; config failures surface from `?` below.
    let path = match config_path() {
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
        Ok(p) => p,
    };
    // Fail-fast: unreadable, unparsable, or invalid bind/paths exits here.
    let config = Config::from_file(&path)?;

    info!(
        database = config.database_url.as_str(),
        bind_address = config.bind_address.as_str(),
        repos_path = config.repos_base_path.as_str(),
        path = %path.display(),
        "Starting Repohub service"
    );

    let db = repohub::Database::new(&config.database_url).await?;

    let github_state = repohub::GithubAppState::new(db.clone(), &config);
    let gerrit_state = repohub::GerritAppState::new(db);

    let github_app = repohub::github_routes().with_state(github_state);
    let gerrit_app =
        Router::new().nest("/gerrit", repohub::gerrit_routes().with_state(gerrit_state));
    let app = github_app.merge(gerrit_app);

    info!("Listening on {}", config.bind_address);
    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
