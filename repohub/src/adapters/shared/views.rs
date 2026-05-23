use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use repo_outils::nix::FlakeMetadata;

use crate::domain::{Project, Repository, User};

pub struct HtmlTemplate<T>(pub T);

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

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub users: Vec<User>,
}

#[derive(Template)]
#[template(path = "user.html")]
pub struct UserTemplate {
    pub user: User,
    pub projects: Vec<Project>,
}

#[derive(Template)]
#[template(path = "project.html")]
pub struct ProjectTemplate {
    pub username: String,
    pub project: Project,
    pub repositories: Vec<Repository>,
}

#[derive(Template)]
#[template(path = "repository.html")]
pub struct RepositoryTemplate {
    pub username: String,
    pub project_name: String,
    pub repo: Repository,
}

#[derive(Template)]
#[template(path = "not_implemented.html")]
pub struct NotImplementedTemplate {
    pub feature: String,
    pub description: String,
    pub back_url: String,
}

#[derive(Template)]
#[template(path = "flake.html")]
pub struct FlakeTemplate {
    pub username: String,
    pub project_name: String,
    pub repo_name: String,
    pub flake_metadata: Option<FlakeMetadata>,
}

#[derive(Template)]
#[template(path = "configuration_v2.html")]
pub struct ConfigurationTemplate {
    pub username: String,
    pub project_name: String,
    pub repositories: Vec<Repository>,
    pub repositories_json: String,
}

// ── DORA Dashboard ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WeekEntry {
    pub week_start: String,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct MetricItem {
    pub label: String,
    pub value: String,
}

#[derive(Template)]
#[template(path = "dora/dashboard.html")]
pub struct DoraDashboardTemplate {
    pub username: String,
    pub project_name: String,
    pub repo_name: String,
    pub weeks: Vec<WeekEntry>,
    pub has_data: bool,
    pub count_metrics: Vec<MetricItem>,
    pub cycle_metrics: Vec<MetricItem>,
    pub dora_metrics: Vec<MetricItem>,
    pub median_metrics: Vec<MetricItem>,
    /// Pre-serialized JSON for Chart.js (labels + datasets across all weeks)
    pub chart_data_json: String,
}
