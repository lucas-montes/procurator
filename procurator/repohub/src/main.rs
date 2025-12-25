// //! Procurator CI Service
// //!
// //! A lightweight CI/CD service that integrates with Git repositories via post-receive hooks.
// //!
// //! ## Architecture
// //!
// //! - **Git Service Integration**: Bare Git repositories trigger CI via post-receive hooks
// //! - **Build Queue**: SQLite-based queue for managing pending, running, and completed builds
// //! - **Worker**: Background task that polls the queue and executes builds using Nix
// //! - **Web UI**: Simple SPA for viewing build history and logs
// //! - **REST API**: HTTP endpoints for creating builds and retrieving results
// //!
// //! ## Data Flow
// //!
// //! 1. User pushes to a monitored branch
// //! 2. Post-receive hook calls `POST /api/builds` with commit/push details
// //! 3. Build is enqueued in SQLite
// //! 4. Worker polls queue and picks up the build
// //! 5. Worker runs `nix flake check` on the specified commit
// //! 6. Build status and logs are stored in database
// //! 7. Web UI displays results in real-time via SSE

// mod api;
// mod config;
// mod database;
// mod domain;
// mod job_queue;
// mod repo_manager;
// mod web;
// mod worker;

// use axum::{routing::get, Router};
// use std::sync::Arc;
// use tracing::info;

// #[derive(Debug)]
// enum Error {
//     Database(String),
//     Network(String),
// }

// impl std::fmt::Display for Error {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         match self {
//             Error::Database(msg) => write!(f, "Database error: {}", msg),
//             Error::Network(msg) => write!(f, "Network error: {}", msg),
//         }
//     }
// }

// impl std::error::Error for Error {}

// type Result<T> = std::result::Result<T, Error>;

// #[derive(Clone)]
// pub struct AppState {
//     queue: Arc<job_queue::JobQueue>,
//     repo_store: Arc<repo_manager::RepositoryStore>,
//     git_manager: Arc<repo_manager::RepoManager>,
// }

// #[tokio::main]
// async fn main() -> Result<()> {
//     tracing_subscriber::fmt()
//         .with_env_filter(
//             tracing_subscriber::EnvFilter::try_from_default_env()
//                 .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
//         )
//         .init();

//     // Initialize config singleton
//     let config = config::Config::init();

//     info!(
//         database = config.database_url.as_str(),
//         bind_address = config.bind_address.as_str(),
//         repos_path = config.repos_base_path.as_str(),
//         "Starting Procurator CI Service"
//     );

//     // Initialize database
//     let database = database::Database::new(&config.database_url)
//         .await
//         .map_err(|e| Error::Database(e.to_string()))?;

//     // Initialize job queue and repository store
//     let queue = Arc::new(job_queue::JobQueue::new(database.clone()));
//     let repo_store = Arc::new(repo_manager::RepositoryStore::new(database));

//     // Initialize repository manager with repository store access
//     let git_manager = repo_manager::RepoManager::new(&config.repos_base_path)
//         .with_repo_store(repo_store.clone());

//     let state = AppState {
//         queue,
//         repo_store,
//         git_manager: Arc::new(git_manager),
//     };

//     // Start build worker in background
//     let worker_state = state.clone();
//     tokio::spawn(async move {
//         let worker = worker::Worker::new(worker_state.queue.clone());
//         if let Err(e) = worker.run().await {
//             tracing::error!(error = %e, "Worker task exited with error");
//         }
//     });

//     info!(target: "procurator::main", "Build worker spawned");

//     // Build API by merging routes from different modules
//     let app = Router::new()
//         .merge(web::routes())
//         .nest("/api",api::routes())
//         // Health check
//         .route("/health", get(|| async { "OK" }))
//         .with_state(state);

//     // Start server
//     info!(target: "procurator::main", bind_address = config.bind_address.as_str(), "Starting HTTP server");

//     let listener = tokio::net::TcpListener::bind(&config.bind_address)
//         .await
//         .map_err(|e| Error::Network(e.to_string()))?;

//     axum::serve(listener, app)
//         .await
//         .map_err(|e| Error::Network(e.to_string()))?;

//     Ok(())
// }

fn main(){}
