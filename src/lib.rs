pub mod storage;
pub mod sync;
pub mod types;
pub mod utils;

pub use storage::Storage;
pub use types::*;

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Find the database path using the standard search order
pub fn find_database_path() -> Result<PathBuf> {
    utils::find_database_path()
}

/// Find the JSONL path for a given database
pub fn find_jsonl_path(db_path: &Path) -> PathBuf {
    utils::find_jsonl_path(db_path)
}

/// Open a storage backend at the given path
pub fn open_storage(path: &PathBuf) -> Result<Box<dyn Storage>> {
    Ok(Box::new(storage::sqlite::SqliteStorage::new(path)?))
}

/// Get the current actor name
pub fn get_actor() -> String {
    utils::get_actor()
}
