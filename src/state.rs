use sea_orm::DatabaseConnection;

use crate::{config::Config, infrastructure::storage::Storage};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub redis: redis::Client,
    pub storage: Storage,
    pub config: Config,
}
