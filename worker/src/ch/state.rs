use std::collections::HashMap;

use sqlx::SqlitePool;

pub struct State {
    persistent: SqlitePool,
    ephemeral: HashMap<String, String>,
}
