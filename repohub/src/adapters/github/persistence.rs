use crate::{
    adapters::shared::database::Database,
    application::github::{GithubError, GithubPort, PortFuture},
};

#[derive(Clone)]
pub struct SqliteGithubRepository {
    pub db: Database,
}

impl SqliteGithubRepository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    fn map_db_error(error: crate::adapters::shared::database::DatabaseError) -> GithubError {
        match error {
            crate::adapters::shared::database::DatabaseError::NotFound(message) => {
                GithubError::NotFound(message)
            }
            other => GithubError::Storage(other.to_string()),
        }
    }
}

impl GithubPort for SqliteGithubRepository {
    fn list_users<'a>(
        &'a self,
    ) -> PortFuture<'a, Result<Vec<crate::adapters::shared::database::UserRow>, GithubError>> {
        Box::pin(async move { self.db.list_users().await.map_err(Self::map_db_error) })
    }

    fn create_user<'a>(
        &'a self,
        username: &'a str,
        email: Option<&'a str>,
    ) -> PortFuture<'a, Result<i64, GithubError>> {
        Box::pin(async move {
            self.db
                .create_user(username, email)
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn get_user_by_username<'a>(
        &'a self,
        username: &'a str,
    ) -> PortFuture<'a, Result<crate::adapters::shared::database::UserRow, GithubError>> {
        Box::pin(async move {
            self.db
                .get_user_by_username(username)
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn list_projects_by_owner<'a>(
        &'a self,
        owner_id: i64,
    ) -> PortFuture<'a, Result<Vec<crate::adapters::shared::database::ProjectRow>, GithubError>> {
        Box::pin(async move {
            self.db
                .list_projects_by_owner(owner_id)
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn create_project<'a>(
        &'a self,
        name: &'a str,
        owner_id: i64,
        description: Option<&'a str>,
    ) -> PortFuture<'a, Result<i64, GithubError>> {
        Box::pin(async move {
            self.db
                .create_project(name, owner_id, description)
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn get_project<'a>(
        &'a self,
        owner_id: i64,
        project_name: &'a str,
    ) -> PortFuture<'a, Result<crate::adapters::shared::database::ProjectRow, GithubError>> {
        Box::pin(async move {
            self.db
                .get_project(owner_id, project_name)
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn list_repositories_by_project<'a>(
        &'a self,
        project_id: i64,
    ) -> PortFuture<
        'a,
        Result<Vec<crate::adapters::shared::database::RepositoryRow>, GithubError>,
    > {
        Box::pin(async move {
            self.db
                .list_repositories_by_project(project_id)
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn create_repository<'a>(
        &'a self,
        project_id: i64,
        name: &'a str,
        git_url: &'a str,
    ) -> PortFuture<'a, Result<i64, GithubError>> {
        Box::pin(async move {
            self.db
                .create_repository(project_id, name, git_url)
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn get_repository<'a>(
        &'a self,
        project_id: i64,
        repo_name: &'a str,
    ) -> PortFuture<'a, Result<crate::adapters::shared::database::RepositoryRow, GithubError>> {
        Box::pin(async move {
            self.db
                .get_repository(project_id, repo_name)
                .await
                .map_err(Self::map_db_error)
        })
    }
}
