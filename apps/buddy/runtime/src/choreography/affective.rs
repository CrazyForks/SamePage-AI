use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[cfg(unix)]
use std::{fs::File, os::fd::AsRawFd};

use serde::Serialize;

use crate::{
    app_paths::BuddyAppPaths, error::BuddyError, error::BuddyResult, local_log::LocalLogTimestamp,
    storage::BuddyStorage,
};

use super::action_log::{ActionLogSink, ActionLogSystemEvent};
pub(crate) use super::affective_types::{
    AffectiveContext, AffectiveContextSnapshot, AffectiveContextSource, AffectiveEnergy,
    AffectiveMood, ResolveContext,
};

pub(crate) const AFFECTIVE_STATE_FILE_NAME: &str = "pet-affective-state.json";
#[cfg(unix)]
const AFFECTIVE_STATE_LOCK_FILE_NAME: &str = ".pet-affective-state.lock";
const AFFECTIVE_STATE_ARG: &str = "--buddy-affective-state";
const AFFECTIVE_STATE_DATA_DIR_ARG: &str = "--buddy-affective-state-data-dir";
const AFFECTIVE_STATE_MOOD_ARG: &str = "--mood";
const AFFECTIVE_STATE_ENERGY_ARG: &str = "--energy";

static AFFECTIVE_CONTEXT_PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone)]
pub(crate) struct AffectiveContextStore {
    state_file_path: PathBuf,
}

impl AffectiveContextStore {
    pub(crate) fn from_buddy_home(buddy_home: PathBuf) -> Self {
        Self {
            state_file_path: buddy_home.join(AFFECTIVE_STATE_FILE_NAME),
        }
    }

    pub(crate) fn state_file_path(&self) -> &std::path::Path {
        &self.state_file_path
    }

    pub(crate) fn read_or_create_default(&self) -> BuddyResult<AffectiveContextSnapshot> {
        self.with_exclusive_lock(|| self.read_or_create_default_unlocked())
    }

    fn read_or_create_default_unlocked(&self) -> BuddyResult<AffectiveContextSnapshot> {
        if !self.state_file_path.exists() {
            self.write_context_unlocked(AffectiveContext::default())?;
            return Ok(AffectiveContextSnapshot {
                context: AffectiveContext::default(),
                source: AffectiveContextSource::DefaultCreated,
            });
        }

        let content = fs::read_to_string(&self.state_file_path)?;
        match serde_json::from_str::<AffectiveContext>(&content) {
            Ok(context) => Ok(AffectiveContextSnapshot {
                context,
                source: AffectiveContextSource::StateFile,
            }),
            Err(_) => Ok(AffectiveContextSnapshot {
                context: AffectiveContext::default(),
                source: AffectiveContextSource::InvalidFileFallback,
            }),
        }
    }

    pub(crate) fn read_or_create_default_with_diagnostics(
        &self,
        storage: &BuddyStorage,
    ) -> BuddyResult<AffectiveContextSnapshot> {
        let snapshot = self.read_or_create_default()?;
        if snapshot.source == AffectiveContextSource::InvalidFileFallback {
            let _ = self.append_invalid_state_file_system_event(storage);
        }

        Ok(snapshot)
    }

    fn append_invalid_state_file_system_event(&self, storage: &BuddyStorage) -> BuddyResult<()> {
        let event = ActionLogSystemEvent::affective_context_invalid_state_file(
            format!("evt_{}", uuid::Uuid::now_v7()),
            AFFECTIVE_STATE_FILE_NAME,
            LocalLogTimestamp::now_utc().to_rfc3339_millis(),
        );

        ActionLogSink::new(storage.clone()).append_system_event(&event)
    }

    fn update_context(
        &self,
        mood: Option<AffectiveMood>,
        energy: Option<AffectiveEnergy>,
    ) -> BuddyResult<AffectiveContext> {
        self.with_exclusive_lock(|| {
            let current = self.read_or_create_default_unlocked()?.context;
            let context = AffectiveContext {
                mood: mood.unwrap_or(current.mood),
                energy: energy.unwrap_or(current.energy),
            };
            self.write_context_unlocked(context)?;
            Ok(context)
        })
    }

    fn write_context_unlocked(&self, context: AffectiveContext) -> BuddyResult<()> {
        if let Some(parent) = self.state_file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        write_affective_context_atomically(
            &self.state_file_path,
            &serde_json::to_vec_pretty(&context)?,
        )?;

        Ok(())
    }

    fn with_exclusive_lock<T>(&self, operation: impl FnOnce() -> BuddyResult<T>) -> BuddyResult<T> {
        let process_lock = AFFECTIVE_CONTEXT_PROCESS_LOCK.get_or_init(|| Mutex::new(()));
        let _process_guard = process_lock.lock().map_err(|_| {
            BuddyError::Runtime("affective context process lock was poisoned".to_owned())
        })?;
        let _file_lock = AffectiveContextFileLock::acquire(&self.state_file_path)?;
        operation()
    }
}

struct AffectiveContextFileLock {
    #[cfg(unix)]
    file: File,
}

impl AffectiveContextFileLock {
    fn acquire(state_file_path: &Path) -> BuddyResult<Self> {
        let parent = state_file_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;

        #[cfg(unix)]
        {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .truncate(false)
                .write(true)
                .open(parent.join(AFFECTIVE_STATE_LOCK_FILE_NAME))?;
            // SAFETY: file owns a valid descriptor for the full lifetime of this lock guard.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }

            Ok(Self { file })
        }

        #[cfg(not(unix))]
        Ok(Self {})
    }
}

#[cfg(unix)]
impl Drop for AffectiveContextFileLock {
    fn drop(&mut self) {
        // SAFETY: file remains open until this drop implementation returns.
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn write_affective_context_atomically(path: &Path, content: &[u8]) -> BuddyResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp_path = parent.join(format!(
        ".{AFFECTIVE_STATE_FILE_NAME}.{}.tmp",
        uuid::Uuid::now_v7()
    ));
    let result: BuddyResult<()> = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;

        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }

    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AffectiveStateCommandConfig {
    data_dir: PathBuf,
    operation: AffectiveStateCommandOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AffectiveStateCommandOperation {
    Get,
    Set {
        mood: Option<AffectiveMood>,
        energy: Option<AffectiveEnergy>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AffectiveStateCommandReport {
    context: AffectiveContext,
    source: AffectiveContextSource,
    state_file_path: String,
}

pub fn run_affective_state_command_from_env() -> Option<BuddyResult<String>> {
    run_affective_state_command(std::env::args())
}

pub(crate) fn run_affective_state_command<I, S>(args: I) -> Option<BuddyResult<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let config = match parse_affective_state_command_config(args)? {
        Ok(config) => config,
        Err(error) => return Some(Err(error)),
    };

    Some(
        execute_affective_state_command(config)
            .and_then(|report| Ok(serde_json::to_string(&report)?)),
    )
}

fn parse_affective_state_command_config<I, S>(
    args: I,
) -> Option<BuddyResult<AffectiveStateCommandConfig>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let command_index = args.iter().position(|arg| arg == AFFECTIVE_STATE_ARG)?;
    let data_dir = argument_value(&args, AFFECTIVE_STATE_DATA_DIR_ARG)
        .map(PathBuf::from)
        .unwrap_or_else(|| BuddyAppPaths::from_default_buddy_home().data_dir_path());
    let Some(operation_name) = args.get(command_index + 1) else {
        return Some(Err(BuddyError::Validation(
            "--buddy-affective-state requires get or set".to_owned(),
        )));
    };

    let operation = match operation_name.as_str() {
        "get" => AffectiveStateCommandOperation::Get,
        "set" => {
            let mood = match optional_argument_value(&args, AFFECTIVE_STATE_MOOD_ARG) {
                Some(value) => match parse_affective_mood(value.as_str()) {
                    Ok(mood) => Some(mood),
                    Err(error) => return Some(Err(error)),
                },
                None => None,
            };
            let energy = match optional_argument_value(&args, AFFECTIVE_STATE_ENERGY_ARG) {
                Some(value) => match parse_affective_energy(value.as_str()) {
                    Ok(energy) => Some(energy),
                    Err(error) => return Some(Err(error)),
                },
                None => None,
            };
            if mood.is_none() && energy.is_none() {
                return Some(Err(BuddyError::Validation(
                    "--buddy-affective-state set requires --mood or --energy".to_owned(),
                )));
            }

            AffectiveStateCommandOperation::Set { mood, energy }
        }
        _ => {
            return Some(Err(BuddyError::Validation(
                "--buddy-affective-state only supports get or set".to_owned(),
            )));
        }
    };

    Some(Ok(AffectiveStateCommandConfig {
        data_dir,
        operation,
    }))
}

fn execute_affective_state_command(
    config: AffectiveStateCommandConfig,
) -> BuddyResult<AffectiveStateCommandReport> {
    let store = AffectiveContextStore::from_buddy_home(config.data_dir);
    let (context, source) = match config.operation {
        AffectiveStateCommandOperation::Get => {
            let snapshot = store.read_or_create_default()?;
            (snapshot.context, snapshot.source)
        }
        AffectiveStateCommandOperation::Set { mood, energy } => {
            let context = store.update_context(mood, energy)?;
            (context, AffectiveContextSource::StateFile)
        }
    };

    Ok(AffectiveStateCommandReport {
        context,
        source,
        state_file_path: store.state_file_path().to_string_lossy().into_owned(),
    })
}

fn argument_value(args: &[String], flag: &str) -> Option<String> {
    optional_argument_value(args, flag)
}

fn optional_argument_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

fn parse_affective_mood(value: &str) -> BuddyResult<AffectiveMood> {
    let mood = match value {
        "neutral" => AffectiveMood::Neutral,
        "happy" => AffectiveMood::Happy,
        "sad" => AffectiveMood::Sad,
        _ => {
            return Err(BuddyError::Validation(format!(
                "unsupported buddy affective mood: {value}"
            )));
        }
    };

    Ok(mood)
}

fn parse_affective_energy(value: &str) -> BuddyResult<AffectiveEnergy> {
    let energy = match value {
        "low" => AffectiveEnergy::Low,
        "medium" => AffectiveEnergy::Medium,
        "high" => AffectiveEnergy::High,
        _ => {
            return Err(BuddyError::Validation(format!(
                "unsupported buddy affective energy: {value}"
            )));
        }
    };

    Ok(energy)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Barrier},
        thread,
    };

    use serde_json::json;

    use super::{run_affective_state_command, AffectiveContext};

    #[test]
    fn affective_state_command_get_creates_default_state_file() {
        let data_dir = temp_affective_state_dir();

        let output = run_affective_state_command([
            "lexora-buddy",
            "--buddy-affective-state",
            "get",
            "--buddy-affective-state-data-dir",
            data_dir.to_string_lossy().as_ref(),
        ])
        .expect("affective state command should handle args")
        .expect("affective state get should pass");
        let value = serde_json::from_str::<serde_json::Value>(&output).expect("parse output");

        assert_eq!(
            value,
            json!({
                "context": {
                    "mood": "neutral",
                    "energy": "medium"
                },
                "source": "defaultCreated",
                "stateFilePath": data_dir
                    .join("pet-affective-state.json")
                    .to_string_lossy()
                    .into_owned()
            })
        );
        assert_eq!(
            serde_json::from_str::<AffectiveContext>(
                &fs::read_to_string(data_dir.join("pet-affective-state.json"))
                    .expect("read state file")
            )
            .expect("parse state file"),
            AffectiveContext::default()
        );
    }

    #[test]
    fn affective_state_command_set_updates_requested_fields() {
        let data_dir = temp_affective_state_dir();

        run_affective_state_command([
            "lexora-buddy",
            "--buddy-affective-state",
            "set",
            "--mood",
            "happy",
            "--buddy-affective-state-data-dir",
            data_dir.to_string_lossy().as_ref(),
        ])
        .expect("affective state command should handle set")
        .expect("affective state set should pass");
        let output = run_affective_state_command([
            "lexora-buddy",
            "--buddy-affective-state",
            "set",
            "--energy",
            "high",
            "--buddy-affective-state-data-dir",
            data_dir.to_string_lossy().as_ref(),
        ])
        .expect("affective state command should handle second set")
        .expect("affective state second set should pass");
        let value = serde_json::from_str::<serde_json::Value>(&output).expect("parse output");

        assert_eq!(
            value["context"],
            json!({
                "mood": "happy",
                "energy": "high"
            })
        );
        assert_eq!(value["source"], "stateFile");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &fs::read_to_string(data_dir.join("pet-affective-state.json"))
                    .expect("read state file")
            )
            .expect("parse state file"),
            json!({
                "mood": "happy",
                "energy": "high"
            })
        );
        assert!(fs::read_dir(&data_dir)
            .expect("read affective state directory")
            .all(|entry| !entry
                .expect("read affective state entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")));
    }

    #[test]
    fn concurrent_partial_affective_state_updates_do_not_lose_fields() {
        for _ in 0..32 {
            let data_dir = temp_affective_state_dir();
            fs::create_dir_all(&data_dir).expect("create affective state directory");
            fs::write(
                data_dir.join("pet-affective-state.json"),
                serde_json::to_vec_pretty(&AffectiveContext::default())
                    .expect("serialize default affective state"),
            )
            .expect("write default affective state");
            let start = Arc::new(Barrier::new(3));

            let mood_data_dir = data_dir.clone();
            let mood_start = Arc::clone(&start);
            let mood_writer = thread::spawn(move || {
                mood_start.wait();
                run_affective_state_command([
                    "lexora-buddy",
                    "--buddy-affective-state",
                    "set",
                    "--mood",
                    "happy",
                    "--buddy-affective-state-data-dir",
                    mood_data_dir.to_string_lossy().as_ref(),
                ])
                .expect("handle concurrent mood set")
                .expect("concurrent mood set");
            });
            let energy_data_dir = data_dir.clone();
            let energy_start = Arc::clone(&start);
            let energy_writer = thread::spawn(move || {
                energy_start.wait();
                run_affective_state_command([
                    "lexora-buddy",
                    "--buddy-affective-state",
                    "set",
                    "--energy",
                    "high",
                    "--buddy-affective-state-data-dir",
                    energy_data_dir.to_string_lossy().as_ref(),
                ])
                .expect("handle concurrent energy set")
                .expect("concurrent energy set");
            });

            start.wait();
            mood_writer.join().expect("join mood writer");
            energy_writer.join().expect("join energy writer");
            let context = serde_json::from_str::<AffectiveContext>(
                &fs::read_to_string(data_dir.join("pet-affective-state.json"))
                    .expect("read concurrent affective state"),
            )
            .expect("parse concurrent affective state");

            assert_eq!(
                context,
                AffectiveContext {
                    mood: super::AffectiveMood::Happy,
                    energy: super::AffectiveEnergy::High,
                }
            );
            fs::remove_dir_all(data_dir).expect("remove concurrent affective state directory");
        }
    }

    #[test]
    fn affective_state_command_rejects_set_without_fields() {
        let error = run_affective_state_command(["lexora-buddy", "--buddy-affective-state", "set"])
            .expect("affective state command should handle set")
            .expect_err("set without fields should fail");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: --buddy-affective-state set requires --mood or --energy"
        );
    }

    fn temp_affective_state_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "lexora-buddy-affective-state-command-{}",
            uuid::Uuid::new_v4()
        ))
    }
}
