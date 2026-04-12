use sqlx::SqlitePool;

#[derive(Clone)]
pub struct Database(SqlitePool);

impl Database {
    pub async fn new(path: &str) -> Self {
        let pool = SqlitePool::connect(path).await.unwrap();
        Self(pool)
    }
}
