use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json,
};

use crate::{
    database::Database,
    models::{
        CreateProjectRequest, CreateRepositoryRequest, CreateUserRequest,
        Project, Repository, User, SaveConfigurationRequest
    },
    config::Config,
    services::RepositoryService,
};
use repo_outils::nix::FlakeMetadata;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub repo_service: RepositoryService,
}

impl AppState {
    pub fn new(db: Database, config: &Config) -> Self {
        Self {
            db,
            repo_service: RepositoryService::new(config),
        }
    }
}

// Template rendering helper
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

// Templates
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    users: Vec<User>,
}

#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
    user: User,
    projects: Vec<Project>,
}

#[derive(Template)]
#[template(path = "project.html")]
struct ProjectTemplate {
    username: String,
    project: Project,
    repositories: Vec<Repository>,
}

#[derive(Template)]
#[template(path = "repository.html")]
struct RepositoryTemplate {
    username: String,
    project_name: String,
    repo: Repository,
}

#[derive(Template)]
#[template(path = "not_implemented.html")]
struct NotImplementedTemplate {
    feature: String,
    description: String,
    back_url: String,
}

#[derive(Template)]
#[template(path = "flake.html")]
struct FlakeTemplate {
    username: String,
    project_name: String,
    repo_name: String,
    flake_metadata: Option<FlakeMetadata>,
}

#[derive(Template)]
#[template(path = "configuration_v2.html")]
struct ConfigurationTemplate {
    username: String,
    project_name: String,
    repositories: Vec<Repository>,
    repositories_json: String,
}

/// The main page of the app. We should show a list of the users in the database
async fn index(State(state): State<AppState>) -> impl IntoResponse {
    match state.db.list_users().await {
        Ok(users) => {
            let users: Vec<User> = users.into_iter().map(User::from).collect();
            HtmlTemplate(IndexTemplate { users }).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list users: {}", e);
            HtmlTemplate(NotImplementedTemplate {
                feature: "Error".to_string(),
                description: format!("Failed to list users: {}", e),
                back_url: "/".to_string(),
            })
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
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // Get user's projects
    match state.db.list_projects_by_owner(user.id).await {
        Ok(projects) => {
            let projects: Vec<Project> = projects.into_iter().map(Project::from).collect();
            HtmlTemplate(UserTemplate { user, projects }).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list projects for user '{}': {}", username, e);
            HtmlTemplate(NotImplementedTemplate {
                feature: "Error".to_string(),
                description: format!("Failed to list projects: {}", e),
                back_url: "/".to_string(),
            })
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
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // Get project
    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => Project::from(project),
        Err(e) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    // Get repositories for this project
    match state.db.list_repositories_by_project(project.id).await {
        Ok(repos) => {
            let repositories: Vec<Repository> = repos.into_iter().map(Repository::from).collect();
            HtmlTemplate(ProjectTemplate {
                username,
                project,
                repositories,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list repositories: {}", e);
            HtmlTemplate(NotImplementedTemplate {
                feature: "Error".to_string(),
                description: format!("Failed to list repositories: {}", e),
                back_url: format!("/{}", username),
            })
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

    // Use the repository service to create or clone the repository
    let git_url = match state.repo_service.create_or_clone_repository(
        &username,
        &req.name,
        req.git_url.as_deref(),
    ) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to create/clone repository");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create repository: {}", e),
            )
            .into_response();
        }
    };

    // Persist repository in DB
    match state
        .db
        .create_repository(project.id, &req.name, &git_url)
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
    State(state): State<AppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    // Get user
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_e) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // Get project
    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_e) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    // Get repositories for this project
    let repositories = match state.db.list_repositories_by_project(project.id).await {
        Ok(repos) => repos.into_iter().map(Repository::from).collect::<Vec<_>>(),
        Err(_e) => Vec::new(),
    };

    // Serialize repositories to JSON for JavaScript
    let repositories_json = serde_json::to_string(&repositories).unwrap_or_else(|_| "[]".to_string());

    HtmlTemplate(ConfigurationTemplate {
        username,
        project_name,
        repositories: repositories.clone(),
        repositories_json,
    })
    .into_response()
}

/// Save project configuration
async fn save_configuration(
    State(_state): State<AppState>,
    Path((username, project_name)): Path<(String, String)>,
    Json(req): Json<SaveConfigurationRequest>,
) -> impl IntoResponse {
    tracing::info!(
        username = username,
        project = project_name,
        "Saving project configuration"
    );

    // TODO: Generate config.nix from req.configuration
    // TODO: Commit to project's procurator/ folder in git

    // For now, just log the configuration
    tracing::debug!(config = ?req.configuration, "Received configuration");

    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

/// A view allowing to create and define a testing strategy for the a project.
/// NOTE: For now redirect to somewhere else, well handle this logic in a different project
async fn testing(
    State(_state): State<AppState>,
    Path((username, project)): Path<(String, String)>,
) -> impl IntoResponse {
    HtmlTemplate(NotImplementedTemplate {
        feature: "E2E Testing & Monitoring".to_string(),
        description: "Define and run end-to-end tests across all services in this project. Monitor performance, measure service health, and validate integration points.".to_string(),
        back_url: format!("/{}/{}", username, project),
    })
}

/// The flake of the repo, with a description, information about the checks to run, dependencies and other information that could be useful
async fn repo_flake(
    State(state): State<AppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    // Parse flake metadata using the repository service
    let flake_metadata = state.repo_service.parse_flake_metadata(&username, &repo_name);

    HtmlTemplate(FlakeTemplate {
        username,
        project_name,
        repo_name,
        flake_metadata,
    })
}

/// The builds or actions ran by a given repo. Like github actions.
async fn builds(
    State(_state): State<AppState>,
    Path((username, project, repo, _id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
    HtmlTemplate(NotImplementedTemplate {
        feature: "Build Details".to_string(),
        description: "View build logs, status, and artifacts. This will integrate with the CI service to display build information.".to_string(),
        back_url: format!("/{}/{}/{}", username, project, repo),
    })
}

/// View of a given repo in a project. A regular viewer like github, gitlab or any other repo hosting provider
async fn repo(
    State(state): State<AppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    // Get user
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_e) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // Get project
    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_e) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    // Get repository
    match state.db.get_repository(project.id, &repo_name).await {
        Ok(repo) => {
            let repo = Repository::from(repo);
            HtmlTemplate(RepositoryTemplate {
                username,
                project_name,
                repo,
            })
            .into_response()
        }
        Err(_e) => {
            tracing::error!("Failed to get repository: {}", repo_name);
            HtmlTemplate(NotImplementedTemplate {
                feature: "Repository Not Found".to_string(),
                description: format!("Repository '{}' not found", repo_name),
                back_url: format!("/{}/{}", username, project_name),
            })
            .into_response()
        }
    }
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
        .route("/{username}/{project}/configuration", get(configuration).post(save_configuration))
        .route("/{username}/{project}/{repo}", get(repo))
        .route("/{username}/{project}/{repo}/builds/{id}", get(builds))
        .route("/{username}/{project}/{repo}/flake", get(repo_flake))
}
