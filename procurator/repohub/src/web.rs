use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json,
};
use std::sync::Arc;

use crate::{
    database::Database,
    models::{CreateProjectRequest, CreateRepositoryRequest, CreateUserRequest, Project, Repository, User},
};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

// #[derive(Template)]
// #[template(path = "repo/infrastructure.html")]
// struct RepoInfrastructureTemplate {
//     username: String,
//     repo_name: String,
//     git_url: String,
//     active_tab: String,
//     infrastructure: Option<Infrastructure>,
// }

#[allow(dead_code)]
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

/// The main page of the app. We should show a list of the users in the database
async fn index(State(state): State<AppState>) -> impl IntoResponse {
    match state.db.list_users().await {
        Ok(users) => {
            let users: Vec<User> = users.into_iter().map(User::from).collect();
            (StatusCode::OK, Json(users)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list users: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list users: {}", e),
            )
                .into_response()
        }
    }
}

/// Create a new user
async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> impl IntoResponse {
    match state
        .db
        .create_user(&req.username, req.email.as_deref())
        .await
    {
        Ok(id) => {
            tracing::info!(user_id = id, username = req.username, "User created");
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create user: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create user: {}", e),
            )
                .into_response()
        }
    }
}

/// The main page of a given user. We'll fetch users by username and show projects related to that user
async fn user(State(state): State<AppState>, Path(username): Path<String>) -> impl IntoResponse {
    // Get user
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => User::from(user),
        Err(e) => {
            tracing::error!("Failed to get user '{}': {}", username, e);
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    // Get user's projects
    match state.db.list_projects_by_owner(user.id).await {
        Ok(projects) => {
            let projects: Vec<Project> = projects.into_iter().map(Project::from).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "user": user,
                    "projects": projects
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list projects for user '{}': {}", username, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list projects: {}", e),
            )
                .into_response()
        }
    }
}

/// Create a new project for a user
async fn create_project(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    // Get user first
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    match state
        .db
        .create_project(&req.name, user.id, req.description.as_deref())
        .await
    {
        Ok(id) => {
            tracing::info!(
                project_id = id,
                owner = username,
                project_name = req.name,
                "Project created"
            );
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create project: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create project: {}", e),
            )
                .into_response()
        }
    }
}

/// The main page of a given project. We'll drift from traditional repositories hub. Instead of having a per repo page, we are going to have a project.
/// A project can be one or many repositories, think of it like all the repos an org might have.
/// We should also find a config section in this project that would allow us to link all the repos together, add external dependencies (such as databases, proxies, etc...)
/// define the infrastructure, some configuration (CI/CD, linting, testing, environements, etc...), and have some utils to deploy, monitor, build, and manage the overall project.
/// We'll leverage nix flakes for that.
async fn project(
    State(state): State<AppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    // Get user
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    // Get project
    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => Project::from(project),
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Project not found: {}", e)).into_response();
        }
    };

    // Get repositories for this project
    match state.db.list_repositories_by_project(project.id).await {
        Ok(repos) => {
            let repos: Vec<Repository> = repos.into_iter().map(Repository::from).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "project": project,
                    "repositories": repos
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list repositories: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list repositories: {}", e),
            )
                .into_response()
        }
    }
}

/// Create a new repository in a project
async fn create_repository(
    State(state): State<AppState>,
    Path((username, project_name)): Path<(String, String)>,
    Json(req): Json<CreateRepositoryRequest>,
) -> impl IntoResponse {
    // Get user
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    // Get project
    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Project not found: {}", e)).into_response();
        }
    };

    match state
        .db
        .create_repository(project.id, &req.name, &req.git_url)
        .await
    {
        Ok(id) => {
            tracing::info!(
                repo_id = id,
                project = project_name,
                repo_name = req.name,
                "Repository created"
            );
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create repository: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create repository: {}", e),
            )
                .into_response()
        }
    }
}

/// TODO: find a better name
/// The idea for this view is to have a way to manage the configuration, infrastructure and everything needed to run all the services.
/// I would like to have a separate repo, which the user doesn't need to create, that will have all the settings together, maybe even the docs.
/// Cloning the repo would give an easy way to manage and run all the services both locally and remotely
async fn configuration(
    State(_state): State<AppState>,
    Path((_username, _project)): Path<(String, String)>,
) -> impl IntoResponse {
    // TODO: Implement configuration management
    (
        StatusCode::NOT_IMPLEMENTED,
        "Configuration management not yet implemented",
    )
}

/// A view allowing to create and define a testing strategy for the a project.
/// NOTE: For now redirect to somewhere else, well handle this logic in a different project
async fn testing(Path((username, project)): Path<(String, String)>) -> impl IntoResponse {
    Redirect::to(&format!("localhost:3002/{}/{}", username, project))
}

/// View of a given repo in a project. A regular viewer like github, gitlab or any other repo hosting provider
async fn repo(
    State(state): State<AppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    // Get user
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    // Get project
    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Project not found: {}", e)).into_response();
        }
    };

    // Get repository
    match state.db.get_repository(project.id, &repo_name).await {
        Ok(repo) => {
            let repo = Repository::from(repo);
            (StatusCode::OK, Json(repo)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to get repository: {}", e);
            (StatusCode::NOT_FOUND, format!("Repository not found: {}", e)).into_response()
        }
    }
}

/// The flake of the repo, with a description, information about the checks to run, dependencies and other information that could be useful
async fn repo_flake(
    State(_state): State<AppState>,
    Path((_username, _project, _repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
    // TODO: Parse and display flake.nix information
    (
        StatusCode::NOT_IMPLEMENTED,
        "Flake viewer not yet implemented",
    )
}

/// The builds or actions ran by a given repo. Like github actions.
async fn builds(
    State(_state): State<AppState>,
    Path((_username, _project, _repo, _id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
    // TODO: Integrate with CI service to show build information
    (StatusCode::NOT_IMPLEMENTED, "Builds viewer not yet implemented")
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/users", post(create_user))
        .route("/{username}", get(user))
        .route("/{username}/projects", post(create_project))
        .route("/{username}/{project}", get(project))
        .route("/{username}/{project}/repositories", post(create_repository))
        .route("/{username}/{project}/testing", get(testing))
        .route("/{username}/{project}/configuration", get(configuration))
        .route("/{username}/{project}/{repo}", get(repo))
        .route("/{username}/{project}/{repo}/builds/{id}", get(builds))
        .route("/{username}/{project}/{repo}/flake", get(repo_flake))
}
