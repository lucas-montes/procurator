use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};

#[derive(Clone, Debug)]
pub struct AppState;

// #[derive(Template)]
// #[template(path = "repo/infrastructure.html")]
// struct RepoInfrastructureTemplate {
//     username: String,
//     repo_name: String,
//     git_url: String,
//     active_tab: String,
//     infrastructure: Option<Infrastructure>,
// }

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
async fn index(
    State(state): State<AppState>,
    Path((username, repo)): Path<(String, String)>,
) -> impl IntoResponse {
}

/// The main page of a given user. We'll fetch users by username and show projects related to that use
async fn user(State(state): State<AppState>, Path(username): Path<String>) -> impl IntoResponse {}

/// The main page of a given project. We'll drift from traditional repositories hub. Instead of having a per repo page, we are going to have a project.
/// A project can be one or many repositories, think of it like all the repos an org might have.
/// We should also find a config section in this project that would allow us to link all the repos together, add external dependencies (such as databases, proxies, etc...)
/// define the infrastructure, some configuration (CI/CD, linting, testing, environements, etc...), and have some utils to deploy, monitor, build, and manage the overall project.
/// We'll leverage nix flakes for that.
async fn project(
    State(state): State<AppState>,
    Path((username, project)): Path<(String, String)>,
) -> impl IntoResponse {
}

/// TODO: find a better name
/// The idea for this view is to have a way to manage the configuration, infrastructure and everything needed to run all the services.
/// I would like to have a separate repo, which the user doesn't need to create, that will have all the settings together, maybe even the docs.
/// Cloning the repo would give an easy way to manage and run all the services both locally and remotely
async fn configuration(
    State(state): State<AppState>,
    Path((username, project)): Path<(String, String)>,
) -> impl IntoResponse {
}

/// A view allowing to create and define a testing strategy for the a project.
/// NOTE: Maybe it should also be available for a per repo. I need to think more about this and how to implement it.
async fn testing(
    State(state): State<AppState>,
    Path((username, project)): Path<(String, String)>,
) -> impl IntoResponse {
}

/// View of a given repo in a project. A regular viewer like github, gitlab or any other repo hosting provider
async fn repo(
    State(state): State<AppState>,
    Path((username, project, repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
}

/// The flake of the repo, with a description, information about the checks to run, dependencies and other information that could be useful
async fn repo_flake(
    State(state): State<AppState>,
    Path((username, project, repo)): Path<(String, String, String)>,
) -> impl IntoResponse {
}

/// The builds or actions ran by a given repo. Like github actions.
async fn builds(
    State(state): State<AppState>,
    Path((username, project, repo, id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/{username}", get(user))
        .route("/{username}/{project}", get(project))
        .route("/{username}/{project}/testing", get(testing))
        .route("/{username}/{project}/configuration", get(configuration))
        .route("/{username}/{project}/{repo}", get(repo))
        .route("/{username}/{project}/{repo}/builds/{id}", get(builds))
        .route("/{username}/{project}/{repo}/flake", get(repo_flake))
}
