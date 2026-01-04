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

use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = repohub::Config::default();

    info!(
        database = config.database_url.as_str(),
        bind_address = config.bind_address.as_str(),
        "Starting Repohub service"
    );

    let db = repohub::Database::new(&config.database_url).await?;
    let state = repohub::AppState::new(db, &config);
    let app = repohub::routes().with_state(state);

    info!("Listening on {}", config.bind_address);
    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
