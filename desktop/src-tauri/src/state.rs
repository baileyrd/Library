use std::sync::Mutex;

use anyhow::Result;
use library_core::config::Config;
use library_core::db::Db;

/// `rusqlite::Connection` (inside `Db`) is `Send` but not `Sync`, so a single
/// connection is shared across Tauri's command threadpool behind a mutex
/// rather than opened per-call. `Config` is cheap to clone/reload but kept
/// alongside it so credential updates persist through the same lock.
pub struct AppState {
    pub db: Mutex<Db>,
    pub config: Mutex<Config>,
}

impl AppState {
    pub fn init() -> Result<Self> {
        let config = Config::load()?;
        let db = Db::open(&config.resolve_db_path())?;
        Ok(AppState {
            db: Mutex::new(db),
            config: Mutex::new(config),
        })
    }
}
