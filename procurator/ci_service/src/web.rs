//! Web UI Handlers
//!
//! Serves the HTML interface for the CI service:
//! - `GET /` - Home page with list of users
//! - `GET /:username` - User's repositories
//! - `GET /:username/:repo` - Repository builds
//! - `GET /:username/:repo/builds/:id` - Build detail with logs
//! - `GET /:username/:repo/files` - Repository file listing
//! - `GET /:username/:repo/flake` - Flake metadata

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use std::process::Command;

use crate::config::Config;
use crate::domain::BuildStatus;
use crate::nix_parser::{FlakeMetadata, Infrastructure, };
use crate::AppState;

// ============================================================================
// Template Structs
// ============================================================================

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    users: Vec<UserInfo>,
}

struct UserInfo {
    username: String,
    repos_count: usize,
}

#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
    username: String,
    repos: Vec<RepoInfo>,
}

struct RepoInfo {
    name: String,
    builds_count: i64,
    last_build: Option<BuildInfo>,
}

#[derive(Clone)]
struct BuildInfo {
    id: i64,
    commit_short: String,
    branch: String,
    status: BuildStatus,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_seconds: Option<i64>,
    retry_count: i64,
    max_retries: i64,
    commit_hash: String,
}

impl From<crate::domain::Build> for BuildInfo {
    fn from(build: crate::domain::Build) -> Self {
        let commit_hash = build.commit_hash();
        let commit_short = if commit_hash.len() >= 8 {
            commit_hash[..8].to_string()
        } else {
            commit_hash.to_string()
        };

        let duration_seconds =
            if let (Some(started), Some(finished)) = (build.started_at(), build.finished_at()) {
                // Parse timestamps and calculate duration
                chrono::DateTime::parse_from_rfc3339(finished)
                    .ok()
                    .and_then(|f| {
                        chrono::DateTime::parse_from_rfc3339(started)
                            .ok()
                            .map(|s| (f - s).num_seconds())
                    })
            } else {
                None
            };

        BuildInfo {
            id: build.id(),
            commit_short,
            commit_hash: commit_hash.to_string(),
            branch: build.branch().to_string(),
            status: build.status(),
            created_at: build.created_at().to_string(),
            started_at: build.started_at().map(|s| s.to_string()),
            finished_at: build.finished_at().map(|s| s.to_string()),
            duration_seconds,
            retry_count: build.retry_count(),
            max_retries: build.max_retries(),
        }
    }
}

#[derive(Template)]
#[template(path = "repo/builds.html")]
struct RepoBuildsTemplate {
    username: String,
    repo_name: String,
    git_url: String,
    active_tab: String,
    builds: Vec<BuildInfo>,
    setup_instructions: SetupInstructions,
}

struct SetupInstructions {
    new_repo: Vec<String>,
    existing_repo: Vec<String>,
}

#[derive(Template)]
#[template(path = "repo/build_detail.html")]
struct BuildDetailTemplate {
    username: String,
    repo_name: String,
    git_url: String,
    active_tab: String,
    build: BuildInfo,
    logs: Option<String>,
}

#[derive(Template)]
#[template(path = "repo/files.html")]
struct RepoFilesTemplate {
    username: String,
    repo_name: String,
    git_url: String,
    active_tab: String,
    files: Vec<FileEntry>,
}

struct FileEntry {
    name: String,
    is_dir: bool,
}

#[derive(Template)]
#[template(path = "repo/flake.html")]
struct RepoFlakeTemplate {
    username: String,
    repo_name: String,
    git_url: String,
    active_tab: String,
    flake_metadata: Option<FlakeMetadata>,
}

#[derive(Template)]
#[template(path = "repo/infrastructure.html")]
struct RepoInfrastructureTemplate {
    username: String,
    repo_name: String,
    git_url: String,
    active_tab: String,
    infrastructure: Option<Infrastructure>,
}

// ============================================================================
// Template Response Helper
// ============================================================================

struct HtmlTemplate<T>(T);

impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => {
                tracing::error!("Template error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Template error: {}", err),
                )
                    .into_response()
            }
        }
    }
}

// ============================================================================
// Route Handlers
// ============================================================================

/// Home page - list of users
async fn index(State(state): State<AppState>) -> impl IntoResponse {
    // Get all users from the database by extracting unique usernames from repo paths
    let users = match state.repo_store.get_all_users_with_repo_count().await {
        Ok(user_data) => user_data
            .into_iter()
            .map(|(username, repos_count)| UserInfo {
                username,
                repos_count: repos_count as usize,
            })
            .collect(),
        Err(e) => {
            tracing::error!("Failed to get users from database: {}", e);
            Vec::new()
        }
    };

    HtmlTemplate(IndexTemplate { users })
}

/// User page - list of repositories for a user
async fn user_repos(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let repos = state
        .git_manager
        .list_repos(&username)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("User not found: {}", e)))?;

    let mut repo_infos = Vec::new();

    for repo_path in repos {
        let name = repo_path.repo_name().to_string();

        let builds_from_db = state.repo_store.list_repos_with_build_counts().await.ok().unwrap_or_default();
        let builds_count = builds_from_db
            .iter()
            .find(|(db_name, _, _)| db_name == &name)
            .map(|(_, _, count)| *count)
            .unwrap_or(0);

        let last_build = state
            .queue
            .get_latest_build_for_repo(&name)
            .await
            .ok()
            .flatten()
            .map(BuildInfo::from);

        repo_infos.push(RepoInfo {
            name,
            builds_count,
            last_build,
        });
    }

    Ok(HtmlTemplate(UserTemplate {
        username,
        repos: repo_infos,
    }))
}

/// Repository builds page
async fn repo_builds(
    State(state): State<AppState>,
    Path((username, repo)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let repos = state
        .git_manager
        .list_repos(&username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = repos
        .iter()
        .find(|r| r.repo_name() == repo)
        .ok_or((StatusCode::NOT_FOUND, format!("Repository '{}' not found", repo)))?;

    let all_builds = state
        .queue
        .list_all_builds()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_builds: Vec<BuildInfo> = all_builds
        .into_iter()
        .filter(|b| b.repo_name() == repo)
        .map(BuildInfo::from)
        .collect();

    let config = Config::init();
    let git_url = repo_path.to_ssh_url(&config.domain);

    Ok(HtmlTemplate(RepoBuildsTemplate {
        username: username.clone(),
        repo_name: repo.clone(),
        git_url: git_url.clone(),
        active_tab: "builds".to_string(),
        builds: repo_builds,
        setup_instructions: SetupInstructions {
            new_repo: vec![
                format!("echo \"# {}\" >> README.md", repo),
                "git init".to_string(),
                "git add .".to_string(),
                "git commit -m \"Initial commit\"".to_string(),
                format!("git remote add origin {}", git_url),
                "git push -u origin main".to_string(),
            ],
            existing_repo: vec![
                format!("git remote add origin {}", git_url),
                "git push -u origin main".to_string(),
            ],
        },
    }))
}

/// Build detail page with logs
async fn build_detail(
    State(state): State<AppState>,
    Path((username, repo, build_id)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let repos = state
        .git_manager
        .list_repos(&username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = repos
        .iter()
        .find(|r| r.repo_name() == repo)
        .ok_or((StatusCode::NOT_FOUND, format!("Repository '{}' not found", repo)))?;

    let build = state
        .queue
        .get_build(build_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("Build not found: {}", e)))?;

    let logs = state
        .queue
        .get_build_logs(build_id)
        .await
        .ok()
        .flatten();

    let config = Config::init();
    let git_url = repo_path.to_ssh_url(&config.domain);

    Ok(HtmlTemplate(BuildDetailTemplate {
        username,
        repo_name: repo,
        git_url,
        active_tab: "builds".to_string(),
        build: BuildInfo::from(build),
        logs,
    }))
}

/// Repository files page
async fn repo_files(
    State(state): State<AppState>,
    Path((username, repo)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let repos = state
        .git_manager
        .list_repos(&username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = repos
        .iter()
        .find(|r| r.repo_name() == repo)
        .ok_or((StatusCode::NOT_FOUND, format!("Repository '{}' not found", repo)))?;

    let config = Config::init();
    let git_url = repo_path.to_ssh_url(&config.domain);

    // Get file listing from bare repo using git ls-tree
    let bare_path = repo_path.bare_repo_path();
    let files = get_repo_files(&bare_path);

    Ok(HtmlTemplate(RepoFilesTemplate {
        username,
        repo_name: repo,
        git_url,
        active_tab: "files".to_string(),
        files,
    }))
}

/// Get file listing from a bare git repository
fn get_repo_files(bare_path: &std::path::Path) -> Vec<FileEntry> {
    let output = Command::new("git")
        .args(["--git-dir", &bare_path.to_string_lossy()])
        .args(["ls-tree", "--name-only", "HEAD"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter(|line| !line.is_empty())
                .map(|name| {
                    // Check if it's a directory by trying to ls-tree it
                    let is_dir = Command::new("git")
                        .args(["--git-dir", &bare_path.to_string_lossy()])
                        .args(["ls-tree", "HEAD", &format!("{}/", name)])
                        .output()
                        .map(|o| o.status.success() && !o.stdout.is_empty())
                        .unwrap_or(false);

                    FileEntry {
                        name: name.to_string(),
                        is_dir,
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Repository flake metadata page
async fn repo_flake(
    State(state): State<AppState>,
    Path((username, repo)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let repos = state
        .git_manager
        .list_repos(&username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = repos
        .iter()
        .find(|r| r.repo_name() == repo)
        .ok_or((StatusCode::NOT_FOUND, format!("Repository '{}' not found", repo)))?;

    let config = Config::init();
    let git_url = repo_path.to_ssh_url(&config.domain);

    // Try to get flake metadata
    let flake_metadata: Option<FlakeMetadata> = repo_path.try_into().ok();

    Ok(HtmlTemplate(RepoFlakeTemplate {
        username,
        repo_name: repo,
        git_url,
        active_tab: "flake".to_string(),
        flake_metadata,
    }))
}

/// Repository infrastructure configuration page
async fn repo_infrastructure(
    State(state): State<AppState>,
    Path((username, repo)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let repos = state
        .git_manager
        .list_repos(&username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = repos
        .iter()
        .find(|r| r.repo_name() == repo)
        .ok_or((StatusCode::NOT_FOUND, format!("Repository '{}' not found", repo)))?;

    let config = Config::init();
    let git_url = repo_path.to_ssh_url(&config.domain);

    // Parse infrastructure directly from the repository
    let infrastructure = repo_path.try_into().ok();

    Ok(HtmlTemplate(RepoInfrastructureTemplate {
        username,
        repo_name: repo,
        git_url,
        active_tab: "infrastructure".to_string(),
        infrastructure,
    }))
}

/// Build the web UI routes
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/{username}", get(user_repos))
        .route("/{username}/{repo}", get(repo_builds))
        .route("/{username}/{repo}/builds/{id}", get(build_detail))
        .route("/{username}/{repo}/files", get(repo_files))
        .route("/{username}/{repo}/flake", get(repo_flake))
        .route("/{username}/{repo}/infrastructure", get(repo_infrastructure))
}
