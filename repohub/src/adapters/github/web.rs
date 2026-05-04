use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};

use crate::{
    adapters::shared::database::Database,
    config::Config,
    domain::{Project, Repository, User},
    services::RepositoryService,
};
use repo_outils::nix::FlakeMetadata;

use super::dto::{
    CreateProjectRequest, CreateRepositoryRequest, CreateUserRequest, SaveConfigurationRequest,
};

#[derive(Clone)]
pub struct GithubAppState {
    pub db: Database,
    pub repo_service: RepositoryService,
}

impl GithubAppState {
    pub fn new(db: Database, config: &Config) -> Self {
        Self {
            db,
            repo_service: RepositoryService::new(config),
        }
    }
}

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

#[derive(Template)]
#[template(path = "agents.html")]
struct AgentsTemplate {
    username: String,
    project_name: String,
    agents: Vec<AgentView>,
    runs: Vec<RunView>,
}

#[derive(Template)]
#[template(path = "documentation.html")]
struct DocumentationTemplate {
    username: String,
    project_name: String,
    sources: Vec<DocSourceView>,
    documents: Vec<DocumentView>,
}

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    username: String,
    project_name: String,
    kpis: Vec<KpiView>,
    build_series: Vec<ChartPoint>,
    failure_series: Vec<ChartPoint>,
    recent_runs: Vec<RunView>,
    monitors: Vec<MonitorView>,
    monitor_options: Vec<MonitorOptionView>,
}

#[derive(Template)]
#[template(path = "milestones.html")]
struct MilestonesTemplate {
    username: String,
    project_name: String,
    milestones: Vec<MilestoneView>,
}

#[derive(Clone)]
struct AgentView {
    name: String,
    status: String,
    description: String,
    last_run: String,
    next_run: String,
}

#[derive(Clone)]
struct RunView {
    name: String,
    status: String,
    duration: String,
    started_at: String,
}

#[derive(Clone)]
struct DocSourceView {
    name: String,
    location: String,
    updated_at: String,
    status: String,
}

#[derive(Clone)]
struct DocumentView {
    title: String,
    repo: String,
    updated_at: String,
    status: String,
}

#[derive(Clone)]
struct KpiView {
    label: String,
    value: String,
}

#[derive(Clone)]
struct ChartPoint {
    label: String,
    value: u32,
    width_pct: u8,
}

#[derive(Clone)]
struct MonitorView {
    name: String,
    target: String,
    status: String,
    latency_ms: u32,
    uptime: String,
    last_check: String,
    ssl_expires: String,
    regions: String,
    alerting: String,
}

#[derive(Clone)]
struct MonitorOptionView {
    name: String,
    description: String,
}

#[derive(Clone)]
struct MilestoneView {
    title: String,
    due_date: String,
    status: String,
    progress: u8,
    description: String,
}

async fn index(State(state): State<GithubAppState>) -> impl IntoResponse {
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

async fn create_user(
    State(state): State<GithubAppState>,
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

async fn user(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
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

async fn create_project(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
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

async fn project(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(error) => {
            tracing::error!(username, project_name, %error, "Failed to get user");
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => Project::from(project),
        Err(error) => {
            tracing::error!(username, project_name, %error, "Failed to get project");
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

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
        Err(error) => {
            tracing::error!(username, project_name, %error, "Failed to list repositories");
            HtmlTemplate(NotImplementedTemplate {
                feature: "Error".to_string(),
                description: format!("Failed to list repositories: {}", error),
                back_url: format!("/{}", username),
            })
            .into_response()
        }
    }
}

async fn create_repository(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
    Json(req): Json<CreateRepositoryRequest>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Project not found: {}", e)).into_response();
        }
    };

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

async fn configuration(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let repositories = match state.db.list_repositories_by_project(project.id).await {
        Ok(repos) => repos.into_iter().map(Repository::from).collect::<Vec<_>>(),
        Err(_error) => Vec::new(),
    };

    let repositories_json =
        serde_json::to_string(&repositories).unwrap_or_else(|_| "[]".to_string());

    HtmlTemplate(ConfigurationTemplate {
        username,
        project_name,
        repositories: repositories.clone(),
        repositories_json,
    })
    .into_response()
}

async fn save_configuration(
    State(_state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
    Json(req): Json<SaveConfigurationRequest>,
) -> impl IntoResponse {
    tracing::info!(
        username = username,
        project = project_name,
        "Saving project configuration"
    );

    tracing::debug!(config = ?req.configuration, "Received configuration");

    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

async fn testing(
    State(_state): State<GithubAppState>,
    Path((username, project)): Path<(String, String)>,
) -> impl IntoResponse {
    HtmlTemplate(NotImplementedTemplate {
		feature: "E2E Testing & Monitoring".to_string(),
		description: "Define and run end-to-end tests across all services in this project. Monitor performance, measure service health, and validate integration points.".to_string(),
		back_url: format!("/{}/{}", username, project),
	})
}

async fn agents(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let agents = vec![
        AgentView {
            name: "Build Orchestrator".to_string(),
            status: "Healthy".to_string(),
            description: "Schedules CI builds and fan-out jobs".to_string(),
            last_run: "5 min ago".to_string(),
            next_run: "Every 10 min".to_string(),
        },
        AgentView {
            name: "Dependency Watcher".to_string(),
            status: "Paused".to_string(),
            description: "Tracks upstream updates across repos".to_string(),
            last_run: "Yesterday".to_string(),
            next_run: "Manual".to_string(),
        },
        AgentView {
            name: "Release Assistant".to_string(),
            status: "Healthy".to_string(),
            description: "Prepares release notes and changelog".to_string(),
            last_run: "2 hours ago".to_string(),
            next_run: "Daily".to_string(),
        },
    ];

    let runs = vec![
        RunView {
            name: "Build Orchestrator".to_string(),
            status: "Success".to_string(),
            duration: "2m 14s".to_string(),
            started_at: "2026-05-05 09:12".to_string(),
        },
        RunView {
            name: "Release Assistant".to_string(),
            status: "Success".to_string(),
            duration: "54s".to_string(),
            started_at: "2026-05-05 07:02".to_string(),
        },
        RunView {
            name: "Dependency Watcher".to_string(),
            status: "Skipped".to_string(),
            duration: "--".to_string(),
            started_at: "2026-05-04 20:20".to_string(),
        },
    ];

    HtmlTemplate(AgentsTemplate {
        username,
        project_name: project.name,
        agents,
        runs,
    })
    .into_response()
}

async fn documentation(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let sources = vec![
        DocSourceView {
            name: "Main Docs".to_string(),
            location: "repo/docs".to_string(),
            updated_at: "2026-05-05".to_string(),
            status: "Healthy".to_string(),
        },
        DocSourceView {
            name: "API Reference".to_string(),
            location: "repo/openapi".to_string(),
            updated_at: "2026-05-04".to_string(),
            status: "Syncing".to_string(),
        },
        DocSourceView {
            name: "Runbooks".to_string(),
            location: "repo/runbooks".to_string(),
            updated_at: "2026-05-01".to_string(),
            status: "Healthy".to_string(),
        },
    ];

    let documents = vec![
        DocumentView {
            title: "Architecture Overview".to_string(),
            repo: "core-services".to_string(),
            updated_at: "2026-05-05".to_string(),
            status: "Published".to_string(),
        },
        DocumentView {
            title: "CI Workflow".to_string(),
            repo: "ci-pipelines".to_string(),
            updated_at: "2026-05-04".to_string(),
            status: "Draft".to_string(),
        },
        DocumentView {
            title: "Ops Runbook".to_string(),
            repo: "runbooks".to_string(),
            updated_at: "2026-05-01".to_string(),
            status: "Published".to_string(),
        },
    ];

    HtmlTemplate(DocumentationTemplate {
        username,
        project_name: project.name,
        sources,
        documents,
    })
    .into_response()
}

async fn stats(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let kpis = vec![
        KpiView {
            label: "Build Success".to_string(),
            value: "93%".to_string(),
        },
        KpiView {
            label: "Avg Build Time".to_string(),
            value: "6m 42s".to_string(),
        },
        KpiView {
            label: "Queued Jobs".to_string(),
            value: "4".to_string(),
        },
        KpiView {
            label: "Active Repos".to_string(),
            value: "12".to_string(),
        },
    ];

    let build_series = vec![
        ChartPoint {
            label: "Mon".to_string(),
            value: 12,
            width_pct: 55,
        },
        ChartPoint {
            label: "Tue".to_string(),
            value: 18,
            width_pct: 85,
        },
        ChartPoint {
            label: "Wed".to_string(),
            value: 15,
            width_pct: 70,
        },
        ChartPoint {
            label: "Thu".to_string(),
            value: 21,
            width_pct: 100,
        },
        ChartPoint {
            label: "Fri".to_string(),
            value: 16,
            width_pct: 75,
        },
    ];

    let failure_series = vec![
        ChartPoint {
            label: "Infra".to_string(),
            value: 4,
            width_pct: 60,
        },
        ChartPoint {
            label: "Tests".to_string(),
            value: 2,
            width_pct: 30,
        },
        ChartPoint {
            label: "Deps".to_string(),
            value: 1,
            width_pct: 15,
        },
        ChartPoint {
            label: "Timeout".to_string(),
            value: 3,
            width_pct: 45,
        },
    ];

    let recent_runs = vec![
        RunView {
            name: "backend-service".to_string(),
            status: "Success".to_string(),
            duration: "5m 02s".to_string(),
            started_at: "2026-05-05 08:50".to_string(),
        },
        RunView {
            name: "worker".to_string(),
            status: "Failed".to_string(),
            duration: "7m 45s".to_string(),
            started_at: "2026-05-05 07:30".to_string(),
        },
        RunView {
            name: "frontend".to_string(),
            status: "Success".to_string(),
            duration: "4m 18s".to_string(),
            started_at: "2026-05-05 07:05".to_string(),
        },
    ];

    let monitors = vec![
        MonitorView {
            name: "Public API".to_string(),
            target: "https://api.repohub.local/health".to_string(),
            status: "Up".to_string(),
            latency_ms: 182,
            uptime: "99.98%".to_string(),
            last_check: "45s ago".to_string(),
            ssl_expires: "89 days".to_string(),
            regions: "6 regions".to_string(),
            alerting: "Slack + Email".to_string(),
        },
        MonitorView {
            name: "Docs Site".to_string(),
            target: "https://docs.repohub.local".to_string(),
            status: "Degraded".to_string(),
            latency_ms: 840,
            uptime: "99.62%".to_string(),
            last_check: "2m ago".to_string(),
            ssl_expires: "41 days".to_string(),
            regions: "4 regions".to_string(),
            alerting: "PagerDuty".to_string(),
        },
        MonitorView {
            name: "Internal CI".to_string(),
            target: "https://ci.repohub.local/".to_string(),
            status: "Down".to_string(),
            latency_ms: 0,
            uptime: "98.01%".to_string(),
            last_check: "now".to_string(),
            ssl_expires: "15 days".to_string(),
            regions: "3 regions".to_string(),
            alerting: "Slack".to_string(),
        },
    ];

    let monitor_options = vec![
        MonitorOptionView {
            name: "HTTP/HTTPS Checks".to_string(),
            description: "Status code, response time, and content validation".to_string(),
        },
        MonitorOptionView {
            name: "SSL Monitoring".to_string(),
            description: "Certificate expiry and chain validation alerts".to_string(),
        },
        MonitorOptionView {
            name: "Multi-Region Probes".to_string(),
            description: "Run checks from multiple regions for latency insights".to_string(),
        },
        MonitorOptionView {
            name: "Alerting Policies".to_string(),
            description: "Slack, email, PagerDuty, and webhook notifications".to_string(),
        },
        MonitorOptionView {
            name: "Status Pages".to_string(),
            description: "Public or private status visibility for stakeholders".to_string(),
        },
        MonitorOptionView {
            name: "Latency & Uptime SLOs".to_string(),
            description: "Targets and historical uptime reporting".to_string(),
        },
    ];

    HtmlTemplate(StatsTemplate {
        username,
        project_name: project.name,
        kpis,
        build_series,
        failure_series,
        recent_runs,
        monitors,
        monitor_options,
    })
    .into_response()
}

async fn milestones(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let milestones = vec![
        MilestoneView {
            title: "CI Stabilization".to_string(),
            due_date: "2026-05-18".to_string(),
            status: "In Progress".to_string(),
            progress: 65,
            description: "Reduce flaky tests and improve build times".to_string(),
        },
        MilestoneView {
            title: "Docs Revamp".to_string(),
            due_date: "2026-05-25".to_string(),
            status: "Planned".to_string(),
            progress: 20,
            description: "Consolidate runbooks and API reference".to_string(),
        },
        MilestoneView {
            title: "Release 2.3".to_string(),
            due_date: "2026-06-05".to_string(),
            status: "At Risk".to_string(),
            progress: 45,
            description: "Finalize features and prep release notes".to_string(),
        },
    ];

    HtmlTemplate(MilestonesTemplate {
        username,
        project_name: project.name,
        milestones,
    })
    .into_response()
}

async fn repo_flake(
    State(state): State<GithubAppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let flake_metadata = state
        .repo_service
        .parse_flake_metadata(&username, &repo_name);

    HtmlTemplate(FlakeTemplate {
        username,
        project_name,
        repo_name,
        flake_metadata,
    })
}

async fn builds(
    State(_state): State<GithubAppState>,
    Path((username, project, repo, _id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
    HtmlTemplate(NotImplementedTemplate {
		feature: "Build Details".to_string(),
		description: "View build logs, status, and artifacts. This will integrate with the CI service to display build information.".to_string(),
		back_url: format!("/{}/{}/{}", username, project, repo),
	})
}

async fn repo(
    State(state): State<GithubAppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

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
        Err(_error) => {
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

pub fn routes() -> Router<GithubAppState> {
    Router::new()
        .route("/", get(index))
        .route("/users", post(create_user))
        .route("/{username}", get(user))
        .route("/{username}/projects", post(create_project))
        .route("/{username}/{project}", get(project))
        .route(
            "/{username}/{project}/repositories",
            post(create_repository),
        )
        .route("/{username}/{project}/testing", get(testing))
        .route(
            "/{username}/{project}/configuration",
            get(configuration).post(save_configuration),
        )
        .route("/{username}/{project}/agents", get(agents))
        .route("/{username}/{project}/documentation", get(documentation))
        .route("/{username}/{project}/stats", get(stats))
        .route("/{username}/{project}/milestones", get(milestones))
        .route("/{username}/{project}/{repo}", get(repo))
        .route("/{username}/{project}/{repo}/builds/{id}", get(builds))
        .route("/{username}/{project}/{repo}/flake", get(repo_flake))
}
