use askama::Template;
use axum::{
	Json, Router,
	extract::{Path, State},
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::{get, post},
};

use crate::{
	application::gerrit::{
		ChangeCommandPort, ChangeQueryPort, CreateChange, CreateChangeInput, PolicyPort, UploadPatchSet,
		UploadPatchSetInput, VoteOnChange, VoteOnChangeInput,
	},
	adapters::shared::views::HtmlTemplate,
	adapters::shared::database::Database,
	domain::{ChangeStatus, PatchSetKind, SubmitReadiness},
};

use super::dto::{
	ApprovalDto, ChangeDetailDto, ChangeDto, CreateChangeRequest, ReadinessCheckDto,
	UploadPatchSetRequest, VoteRequest,
};
use super::persistence::SqliteReviewRepository;

#[derive(Clone)]
pub struct GerritAppState {
	pub db: Database,
}

impl GerritAppState {
	pub fn new(db: Database) -> Self {
		Self { db }
	}
}

#[derive(Template)]
#[template(path = "gerrit_changes.html")]
struct GerritChangesTemplate {
	username: String,
	project_name: String,
	repo_name: String,
	changes: Vec<ChangeDto>,
}

#[derive(Template)]
#[template(path = "gerrit_change_detail.html")]
struct GerritChangeDetailTemplate {
	username: String,
	project_name: String,
	repo_name: String,
	detail: ChangeDetailDto,
}

async fn resolve_repo_for_user_project(
	state: &GerritAppState,
	username: &str,
	project_name: &str,
	repo_name: &str,
) -> Result<crate::adapters::shared::database::RepositoryRow, Response> {
	let user = state
		.db
		.get_user_by_username(username)
		.await
		.map_err(|_| {
			(
				StatusCode::NOT_FOUND,
				format!("User '{}' not found", username),
			)
				.into_response()
		})?;

	let project = state.db.get_project(user.id, project_name).await.map_err(|_| {
		(
			StatusCode::NOT_FOUND,
			format!("Project '{}' not found", project_name),
		)
			.into_response()
	})?;

	state
		.db
		.get_repository(project.id, repo_name)
		.await
		.map_err(|_| {
			(
				StatusCode::NOT_FOUND,
				format!("Repository '{}' not found", repo_name),
			)
				.into_response()
		})
}

fn parse_patch_kind(value: Option<&str>) -> PatchSetKind {
	match value {
		Some("web") | Some("web_upload") => PatchSetKind::WebUpload,
		_ => PatchSetKind::RefUpload,
	}
}

fn gen_change_key(owner_id: i64) -> String {
	let nanos = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|duration| duration.as_nanos())
		.unwrap_or_default();
	format!("I{:x}{:x}", owner_id, nanos)
}

async fn load_change_dtos(
	review_repo: &SqliteReviewRepository,
	repository_id: i64,
) -> Result<Vec<ChangeDto>, String> {
	review_repo
		.list_changes_by_repository(repository_id)
		.await
		.map(|changes| {
			changes
				.into_iter()
				.map(|change| ChangeDto {
					id: change.id,
					repository_id: change.repository_id,
					change_key: change.change_key,
					target_branch: change.target_branch,
					subject: change.subject,
					owner_user_id: change.owner_user_id,
					status: change.status,
					current_patch_set: change.current_patch_set,
					created_at: change.created_at,
					updated_at: change.updated_at,
				})
				.collect()
		})
		.map_err(|error| format!("Failed to list changes: {}", error))
}

fn map_change_status(status: ChangeStatus) -> String {
	match status {
		ChangeStatus::New => "New".to_string(),
		ChangeStatus::Merged => "Merged".to_string(),
		ChangeStatus::Abandoned => "Abandoned".to_string(),
	}
}

async fn compute_submit_readiness(
	review_repo: &SqliteReviewRepository,
	change_id: i64,
) -> Result<SubmitReadiness, String> {
	let change = review_repo
		.get_change(change_id)
		.await
		.map_err(|error| format!("Change not found: {}", error))?;

	let policy = review_repo
		.get_policy_for_repository(change.repository_id)
		.await
		.map_err(|error| format!("Failed to resolve policy: {}", error))?;

	let approvals = review_repo
		.list_approvals(change_id)
		.await
		.map_err(|error| format!("Failed to load approvals: {}", error))?;

	Ok(SubmitReadiness::evaluate(&policy, &approvals, true))
}

async fn load_change_detail(
	review_repo: &SqliteReviewRepository,
	repository_id: i64,
	change_id: i64,
) -> Result<ChangeDetailDto, String> {
	let change = review_repo
		.get_change(change_id)
		.await
		.map_err(|error| format!("Change not found: {}", error))?;

	if change.repository_id != repository_id {
		return Err("Change does not belong to repository".to_string());
	}

	let approvals = review_repo
		.list_approvals(change_id)
		.await
		.map_err(|error| format!("Failed to load approvals: {}", error))?;

	let readiness = compute_submit_readiness(review_repo, change_id).await?;

	let approval_dtos = approvals
		.into_iter()
		.map(|record| ApprovalDto {
			user_id: record.user_id,
			label: record.approval.label,
			value: record.approval.value,
		})
		.collect::<Vec<_>>();

	let readiness_checks = readiness
		.checks
		.into_iter()
		.map(|(name, passed)| ReadinessCheckDto { name, passed })
		.collect::<Vec<_>>();

	Ok(ChangeDetailDto {
		change: ChangeDto {
			id: change.id,
			repository_id: change.repository_id,
			change_key: change.change_key,
			target_branch: change.target_branch,
			subject: change.subject,
			owner_user_id: change.owner_user_id,
			status: map_change_status(change.status),
			current_patch_set: change.current_patch_set,
			created_at: "-".to_string(),
			updated_at: "-".to_string(),
		},
		approvals: approval_dtos,
		readiness_ready: readiness.ready,
		readiness_checks,
	})
}

async fn create_change(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name)): Path<(String, String, String)>,
	Json(req): Json<CreateChangeRequest>,
) -> impl IntoResponse {
	let repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let owner = match state.db.get_user_by_username(&username).await {
		Ok(user) => user,
		Err(error) => {
			return (
				StatusCode::NOT_FOUND,
				format!("Owner lookup failed: {}", error),
			)
				.into_response();
		}
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	let use_case = CreateChange::new(&review_repo);

	let input = CreateChangeInput {
		repository_id: repo.id,
		change_key: req.change_key.unwrap_or_else(|| gen_change_key(owner.id)),
		target_branch: req.target_branch,
		subject: req.subject,
		owner_user_id: owner.id,
		revision: req.revision,
		kind: parse_patch_kind(req.kind.as_deref()),
	};

	match use_case.execute(input).await {
		Ok(change) => (StatusCode::CREATED, Json(change)).into_response(),
		Err(error) => (
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("Failed to create change: {}", error),
		)
			.into_response(),
	}
}

async fn list_changes(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
	let repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	match load_change_dtos(&review_repo, repo.id).await {
		Ok(changes) => (StatusCode::OK, Json(changes)).into_response(),
		Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
	}
}

async fn changes_ui(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
	let repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	let changes = match load_change_dtos(&review_repo, repo.id).await {
		Ok(changes) => changes,
		Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
	};

	HtmlTemplate(GerritChangesTemplate {
		username,
		project_name,
		repo_name,
		changes,
	})
	.into_response()
}

async fn change_detail_ui(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name, change_id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
	let repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	let detail = match load_change_detail(&review_repo, repo.id, change_id).await {
		Ok(detail) => detail,
		Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
	};

	HtmlTemplate(GerritChangeDetailTemplate {
		username,
		project_name,
		repo_name,
		detail,
	})
	.into_response()
}

async fn upload_patch_set(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name, change_id)): Path<(String, String, String, i64)>,
	Json(req): Json<UploadPatchSetRequest>,
) -> impl IntoResponse {
	let repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let uploader_username = req.uploader_username.unwrap_or(username);
	let uploader = match state.db.get_user_by_username(&uploader_username).await {
		Ok(user) => user,
		Err(error) => {
			return (
				StatusCode::NOT_FOUND,
				format!("Uploader lookup failed: {}", error),
			)
				.into_response();
		}
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	let use_case = UploadPatchSet::new(&review_repo, &review_repo);

	let input = UploadPatchSetInput {
		change_id,
		revision: req.revision,
		uploader_user_id: uploader.id,
		kind: parse_patch_kind(req.kind.as_deref()),
	};

	match use_case.execute(input).await {
		Ok(()) => (StatusCode::CREATED, Json(serde_json::json!({ "ok": true, "repository_id": repo.id })))
			.into_response(),
		Err(error) => (
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("Failed to upload patch set: {}", error),
		)
			.into_response(),
	}
}

async fn vote_change(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name, change_id)): Path<(String, String, String, i64)>,
	Json(req): Json<VoteRequest>,
) -> impl IntoResponse {
	let repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let reviewer = match state.db.get_user_by_username(&req.reviewer_username).await {
		Ok(user) => user,
		Err(error) => {
			return (
				StatusCode::NOT_FOUND,
				format!("Reviewer lookup failed: {}", error),
			)
				.into_response();
		}
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	let use_case = VoteOnChange::new(&review_repo, &review_repo);

	let input = VoteOnChangeInput {
		change_id,
		repository_id: repo.id,
		user_id: reviewer.id,
		label: req.label,
		value: req.value,
	};

	match use_case.execute(input).await {
		Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
		Err(error) => (
			StatusCode::BAD_REQUEST,
			format!("Vote rejected: {}", error),
		)
			.into_response(),
	}
}

async fn submit_readiness(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name, change_id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
	let _repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	match compute_submit_readiness(&review_repo, change_id).await {
		Ok(readiness) => (StatusCode::OK, Json(readiness)).into_response(),
		Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
	}
}

async fn submit_change(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name, change_id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
	let _repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	let readiness = match compute_submit_readiness(&review_repo, change_id).await {
		Ok(readiness) => readiness,
		Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
	};
	if !readiness.ready {
		return (
			StatusCode::CONFLICT,
			Json(serde_json::json!({
				"ok": false,
				"reason": "submit requirements not met",
				"checks": readiness.checks,
			})),
		)
			.into_response();
	}

	match review_repo.update_change_status(change_id, "Merged").await {
		Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true, "status": "Merged" }))).into_response(),
		Err(error) => (
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("Failed to submit change: {}", error),
		)
			.into_response(),
	}
}

async fn abandon_change(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name, change_id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
	let _repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	match review_repo.update_change_status(change_id, "Abandoned").await {
		Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true, "status": "Abandoned" }))).into_response(),
		Err(error) => (
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("Failed to abandon change: {}", error),
		)
			.into_response(),
	}
}

async fn restore_change(
	State(state): State<GerritAppState>,
	Path((username, project_name, repo_name, change_id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
	let _repo = match resolve_repo_for_user_project(&state, &username, &project_name, &repo_name).await {
		Ok(repo) => repo,
		Err(response) => return response,
	};

	let review_repo = SqliteReviewRepository::new(state.db.clone());
	match review_repo.update_change_status(change_id, "New").await {
		Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true, "status": "New" }))).into_response(),
		Err(error) => (
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("Failed to restore change: {}", error),
		)
			.into_response(),
	}
}

pub fn routes() -> Router<GerritAppState> {
	Router::new()
		.route(
			"/{username}/{project}/{repo}/changes",
			get(list_changes).post(create_change),
		)
		.route("/{username}/{project}/{repo}/changes/ui", get(changes_ui))
		.route(
			"/{username}/{project}/{repo}/changes/{change_id}/ui",
			get(change_detail_ui),
		)
		.route(
			"/{username}/{project}/{repo}/changes/{change_id}/patchsets",
			post(upload_patch_set),
		)
		.route(
			"/{username}/{project}/{repo}/changes/{change_id}/votes",
			post(vote_change),
		)
		.route(
			"/{username}/{project}/{repo}/changes/{change_id}/submit-readiness",
			get(submit_readiness),
		)
		.route(
			"/{username}/{project}/{repo}/changes/{change_id}/submit",
			post(submit_change),
		)
		.route(
			"/{username}/{project}/{repo}/changes/{change_id}/abandon",
			post(abandon_change),
		)
		.route(
			"/{username}/{project}/{repo}/changes/{change_id}/restore",
			post(restore_change),
		)
}
