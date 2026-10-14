//! App data/log directories (see docs/ARCHITECTURE.md §3).
//!
//! Data dir from `dirs::data_dir()` (`~/<data dir>/synapse/`); project
//! folders default to `~/Synapse/<project-slug>/`.

use std::path::PathBuf;

/// `~/<data dir>/synapse` (e.g. `~/.local/share/synapse` on Linux).
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("synapse")
}

/// Rolling log files live under the data dir: `<data>/synapse/logs`.
pub fn log_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Default SQLite path: `<data>/synapse/synapse.db`.
pub fn db_path() -> PathBuf {
    data_dir().join("synapse.db")
}

/// Create the data + log dirs. Idempotent.
pub fn ensure_app_dirs() -> std::io::Result<AppDirs> {
    let dirs = AppDirs {
        data: data_dir(),
        logs: log_dir(),
        db: db_path(),
    };
    std::fs::create_dir_all(&dirs.data)?;
    std::fs::create_dir_all(&dirs.logs)?;
    if let Some(parent) = dirs.db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(dirs)
}

#[derive(Debug, Clone)]
pub struct AppDirs {
    pub data: PathBuf,
    pub logs: PathBuf,
    pub db: PathBuf,
}
