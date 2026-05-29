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

use axum::Router;
use tracing::info;

use repohub::RefreshOrchestrator;
use repohub::application::dora::DoraAppState;
use repohub::application::ports::ForgeRepositoryTarget;

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

    // ── DORA: state, background task, and routes ─────────────────────────

    let dora_state = DoraAppState { db: db.clone() };

    // Spawn periodic background refresh for all GitHub-hosted repositories.
    // Each repo uses the project owner's GitHub Personal Access Token (PAT)
    // to fetch data. Repos whose owner has no token configured are skipped.
    let patterns = config.dora_incident_label_patterns.clone();
    let interval_secs = config.dora_interval_seconds;
    let db_clone = db.clone();

    tokio::spawn(async move {
        tracing::info!(
            interval_secs = interval_secs,
            "Starting DORA background refresh loop for all GitHub repositories"
        );

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
        // Tick immediately on startup so data is fetched ASAP.
        interval.tick().await;

        loop {
            interval.tick().await;

            // Fetch all repositories from the database.
            let repos = match db_clone.list_all_repositories().await {
                Ok(repos) => repos,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to list repositories for DORA refresh");
                    continue;
                }
            };

            for repo_row in &repos {
                // Parse GitHub owner and repo name from git_url.
                let (owner, repo_name) = match parse_github_url(&repo_row.git_url) {
                    Some(pair) => pair,
                    None => {
                        tracing::trace!(
                            git_url = %repo_row.git_url,
                            repo = %repo_row.name,
                            "Skipping non-GitHub repository in DORA refresh"
                        );
                        continue;
                    }
                };

                // Look up the project owner's PAT from the database.
                let token = match db_clone.get_github_token_for_repository(repo_row.id).await {
                    Ok(Some(t)) if !t.is_empty() => t,
                    _ => {
                        tracing::trace!(
                            repo = %repo_row.name,
                            owner = %owner,
                            "No GitHub token configured for repository owner; skipping"
                        );
                        continue;
                    }
                };

                let target = ForgeRepositoryTarget {
                    repository_id: repo_row.id,
                    owner: owner.to_string(),
                    name: repo_name.to_string(),
                };

                let auth = repohub::adapters::github::auth::GithubAuth::from_pat(token);
                let client = repohub::adapters::github::client::GithubClient::new(
                    auth,
                    db_clone.clone(),
                    owner.to_string(),
                    repo_name.to_string(),
                );
                let orchestrator = RefreshOrchestrator::new(Box::new(client), db_clone.clone());

                match orchestrator.trigger_refresh(&target, &patterns, "v1").await {
                    Ok(result) => {
                        info!(
                            repo = %repo_row.name,
                            owner = %owner,
                            weeks = result.weeks_covered,
                            prs = result.signal_counts.pull_requests,
                            reviews = result.signal_counts.reviews,
                            commits = result.signal_counts.commits,
                            deployments = result.signal_counts.deployments,
                            issues = result.signal_counts.issues,
                            "DORA background refresh completed"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            repo = %repo_row.name,
                            owner = %owner,
                            error = %e,
                            "DORA refresh failed for repository (will retry)"
                        );
                    }
                }
            }
        }
    });

    let dora_routes = repohub::dora_routes().with_state(dora_state);

    // ── Existing services ────────────────────────────────────────────────

    let github_state = repohub::GithubAppState::new(db.clone(), &config);
    let gerrit_state = repohub::GerritAppState::new(db);

    let github_app = repohub::github_routes().with_state(github_state);
    let gerrit_app =
        Router::new().nest("/gerrit", repohub::gerrit_routes().with_state(gerrit_state));

    // Merge all routers.
    let app = github_app.merge(gerrit_app).merge(dora_routes);

    info!("Listening on {}", config.bind_address);
    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Parse GitHub owner and repo name from a repository git URL.
///
/// Supports:
/// - `https://github.com/owner/repo.git`
/// - `https://github.com/owner/repo`
/// - `git@github.com:owner/repo.git`
/// - `git@github.com:owner/repo`
///
/// Returns `None` for non-GitHub URLs.
fn parse_github_url(url: &str) -> Option<(&str, &str)> {
    // Remove optional .git suffix
    let url = url.strip_suffix(".git").unwrap_or(url);

    if let Some(path) = url.strip_prefix("https://github.com/") {
        let mut parts = path.splitn(2, '/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        if !owner.is_empty() && !repo.is_empty() {
            return Some((owner, repo));
        }
    } else if let Some(path) = url.strip_prefix("git@github.com:") {
        let mut parts = path.splitn(2, '/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        if !owner.is_empty() && !repo.is_empty() {
            return Some((owner, repo));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url_https() {
        assert_eq!(
            parse_github_url("https://github.com/my-org/my-repo.git"),
            Some(("my-org", "my-repo"))
        );
    }

    #[test]
    fn test_parse_github_url_https_no_dot_git() {
        assert_eq!(
            parse_github_url("https://github.com/my-org/my-repo"),
            Some(("my-org", "my-repo"))
        );
    }

    #[test]
    fn test_parse_github_url_ssh() {
        assert_eq!(
            parse_github_url("git@github.com:my-org/my-repo.git"),
            Some(("my-org", "my-repo"))
        );
    }

    #[test]
    fn test_parse_github_url_non_github() {
        assert_eq!(parse_github_url("https://gitlab.com/owner/repo.git"), None);
        assert_eq!(parse_github_url(""), None);
    }

    #[test]
    fn test_parse_github_url_missing_parts() {
        assert_eq!(parse_github_url("https://github.com/"), None);
        assert_eq!(parse_github_url("https://github.com/owner/"), None);
    }
}
