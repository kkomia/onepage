use std::sync::Arc;

use anyhow::Result;
use minijinja::Environment;

use crate::auth::LoginThrottle;
use crate::config::Config;
use crate::db::Database;
use crate::services::favicon;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub config: Arc<Config>,
    pub templates: Arc<Environment<'static>>,
    pub http: Arc<ureq::Agent>,
    pub throttle: Arc<LoginThrottle>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self> {
        let db = Database::open(&config.db_path())?;
        let templates = crate::templates::environment();
        let http = Arc::new(favicon::build_agent(&config));
        Ok(Self {
            db,
            config: Arc::new(config),
            templates,
            http,
            throttle: Arc::new(LoginThrottle::default()),
        })
    }

    pub fn refresh_icon_in_background(&self, link_id: i64) {
        let db = self.db.clone();
        let agent = self.http.clone();
        let config = self.config.clone();
        tokio::spawn(async move {
            favicon::spawn_refresh(db, agent, config, link_id).await;
        });
    }
}
