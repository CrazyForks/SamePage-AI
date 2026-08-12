use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::error::BuddyResult;

pub const ATTACHMENTS_DIR_NAME: &str = "attachments";
pub const ARTIFACTS_DIR_NAME: &str = "artifacts";
pub const CONVERSATIONS_DIR_NAME: &str = "conversations";
pub const DATABASE_FILE_NAME: &str = "state.sqlite3";
pub const MEMORIES_DIR_NAME: &str = "memories";
pub const RUNS_DIR_NAME: &str = "runs";
pub const SQLITE_DIR_NAME: &str = "sqlite";

#[derive(Clone, Debug)]
pub struct BuddyAppPaths {
    data_dir: PathBuf,
    attachments_dir: PathBuf,
    artifacts_dir: PathBuf,
    cache_dir: PathBuf,
    conversations_dir: PathBuf,
    log_dir: PathBuf,
    memories_dir: PathBuf,
    runs_dir: PathBuf,
    sqlite_dir: PathBuf,
    database_path: PathBuf,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAppPathsStatus {
    data_dir: String,
    attachments_dir: String,
    artifacts_dir: String,
    cache_dir: String,
    conversations_dir: String,
    log_dir: String,
    memories_dir: String,
    runs_dir: String,
    sqlite_dir: String,
    database_path: String,
}

impl BuddyAppPaths {
    pub fn from_data_dir(data_dir: PathBuf) -> Self {
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);

        Self {
            attachments_dir: data_dir.join(ATTACHMENTS_DIR_NAME),
            artifacts_dir: data_dir.join(ARTIFACTS_DIR_NAME),
            cache_dir: data_dir.join("cache"),
            conversations_dir: data_dir.join(CONVERSATIONS_DIR_NAME),
            log_dir: data_dir.join("logs"),
            memories_dir: data_dir.join(MEMORIES_DIR_NAME),
            runs_dir: data_dir.join(RUNS_DIR_NAME),
            database_path: sqlite_dir.join(DATABASE_FILE_NAME),
            sqlite_dir,
            data_dir,
        }
    }

    pub fn from_default_buddy_home() -> Self {
        Self::from_data_dir(resolve_lexora_home().join("buddy"))
    }

    pub fn ensure_exists(&self) -> BuddyResult<()> {
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(&self.attachments_dir)?;
        fs::create_dir_all(&self.artifacts_dir)?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::create_dir_all(&self.conversations_dir)?;
        fs::create_dir_all(&self.log_dir)?;
        fs::create_dir_all(&self.memories_dir)?;
        fs::create_dir_all(&self.runs_dir)?;
        fs::create_dir_all(&self.sqlite_dir)?;

        Ok(())
    }

    pub fn database_path(&self) -> PathBuf {
        self.database_path.clone()
    }

    pub fn data_dir_path(&self) -> PathBuf {
        self.data_dir.clone()
    }

    pub fn attachments_dir_path(&self) -> PathBuf {
        self.attachments_dir.clone()
    }

    pub fn memories_dir_path(&self) -> PathBuf {
        self.memories_dir.clone()
    }

    pub fn status(&self) -> BuddyAppPathsStatus {
        BuddyAppPathsStatus {
            data_dir: path_to_string(&self.data_dir),
            attachments_dir: path_to_string(&self.attachments_dir),
            artifacts_dir: path_to_string(&self.artifacts_dir),
            cache_dir: path_to_string(&self.cache_dir),
            conversations_dir: path_to_string(&self.conversations_dir),
            log_dir: path_to_string(&self.log_dir),
            memories_dir: path_to_string(&self.memories_dir),
            runs_dir: path_to_string(&self.runs_dir),
            sqlite_dir: path_to_string(&self.sqlite_dir),
            database_path: path_to_string(&self.database_path),
        }
    }
}

fn resolve_lexora_home() -> PathBuf {
    std::env::var_os("LEXORA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".lexora"))
        })
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|home| home.join(".lexora"))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("lexora"))
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::BuddyAppPaths;

    #[test]
    fn creates_stable_buddy_home_subdirectories() {
        let data_dir =
            std::env::temp_dir().join(format!("lexora-buddy-paths-test-{}", uuid::Uuid::new_v4()));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());

        paths.ensure_exists().expect("ensure paths");

        for relative_path in [
            "attachments",
            "artifacts",
            "cache",
            "conversations",
            "logs",
            "memories",
            "runs",
            "sqlite",
        ] {
            assert!(
                fs::metadata(data_dir.join(relative_path))
                    .expect("subdirectory exists")
                    .is_dir(),
                "{relative_path} should be a directory"
            );
        }

        fs::remove_dir_all(data_dir).expect("cleanup");
    }

    #[test]
    fn stores_state_database_under_sqlite_directory() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-database-path-test-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());

        assert_eq!(
            paths.database_path(),
            data_dir.join("sqlite").join("state.sqlite3")
        );
    }
}
