use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use serde::Deserialize;
use serde_json::Value;

use crate::{
    app_paths::{CONVERSATIONS_DIR_NAME, RUNS_DIR_NAME},
    error::{BuddyError, BuddyResult},
    local_log::{
        append_jsonl_event, conversation_index_path, conversation_log_path, run_index_path,
        run_log_path, LocalLogEvent, LocalLogTimestamp,
    },
};

const CONVERSATION_INDEX_EVENT_TYPE: &str = "conversation.indexed";
const CONVERSATION_DELETED_EVENT_TYPE: &str = "conversation.deleted";
const RUN_INDEX_EVENT_TYPE: &str = "run.indexed";
const RUN_DELETED_EVENT_TYPE: &str = "run.deleted";

#[derive(Clone)]
pub struct LocalLogRuntime {
    buddy_home: PathBuf,
    event_ids: LocalLogEventIdSource,
    timestamps: LocalLogTimestampSource,
}

#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
enum LocalLogEventIdSource {
    Uuid,
    Counter {
        next: Arc<AtomicU64>,
        prefix: String,
    },
}

#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
enum LocalLogTimestampSource {
    System,
    Fixed(LocalLogTimestamp),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalLogIndexLine {
    schema_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    payload: LocalLogIndexPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalLogIndexPayload {
    log_path: String,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct LocalLogDiscovery {
    pub(crate) active_log_paths: Vec<String>,
    pub(crate) deleted_entity_ids: Vec<String>,
}

impl LocalLogRuntime {
    pub fn new(buddy_home: PathBuf) -> Self {
        Self {
            buddy_home,
            event_ids: LocalLogEventIdSource::Uuid,
            timestamps: LocalLogTimestampSource::System,
        }
    }

    #[cfg(test)]
    pub fn fixed_for_test(buddy_home: PathBuf, timestamp: LocalLogTimestamp) -> Self {
        Self {
            buddy_home,
            event_ids: LocalLogEventIdSource::Counter {
                next: Arc::new(AtomicU64::new(1)),
                prefix: "test-event".to_owned(),
            },
            timestamps: LocalLogTimestampSource::Fixed(timestamp),
        }
    }

    pub fn conversation_log_path(&self, conversation_id: &str) -> PathBuf {
        conversation_log_path(&self.buddy_home, self.timestamp(), conversation_id)
    }

    pub fn run_log_path(&self, run_id: &str) -> PathBuf {
        run_log_path(&self.buddy_home, self.timestamp(), run_id)
    }

    pub fn relative_path(&self, path: &Path) -> BuddyResult<String> {
        let relative_path = path.strip_prefix(&self.buddy_home).map_err(|_| {
            BuddyError::Validation("local log path must stay under Buddy home".to_owned())
        })?;

        Ok(relative_path.to_string_lossy().into_owned())
    }

    pub fn absolute_path(&self, relative_path: &str) -> PathBuf {
        self.buddy_home.join(relative_path)
    }

    pub fn checked_absolute_path(&self, relative_path: &str) -> BuddyResult<PathBuf> {
        let path = Path::new(relative_path);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(BuddyError::Validation(
                "local log path must be relative to Buddy home".to_owned(),
            ));
        }

        Ok(self.absolute_path(relative_path))
    }

    pub fn discover_conversation_logs(&self) -> BuddyResult<Vec<String>> {
        Ok(self.discover_conversation_log_state()?.active_log_paths)
    }

    pub(crate) fn discover_conversation_log_state(&self) -> BuddyResult<LocalLogDiscovery> {
        if let Some(discovery) = self.discover_indexed_log_state(
            conversation_index_path(&self.buddy_home),
            CONVERSATION_INDEX_EVENT_TYPE,
            CONVERSATION_DELETED_EVENT_TYPE,
            validate_indexed_conversation_log_path,
            |payload| payload.conversation_id.as_deref(),
        )? {
            return Ok(discovery);
        }

        Ok(LocalLogDiscovery {
            active_log_paths: self.discover_jsonl_logs(CONVERSATIONS_DIR_NAME)?,
            deleted_entity_ids: Vec::new(),
        })
    }

    pub fn discover_run_logs(&self) -> BuddyResult<Vec<String>> {
        Ok(self.discover_run_log_state()?.active_log_paths)
    }

    pub(crate) fn discover_run_log_state(&self) -> BuddyResult<LocalLogDiscovery> {
        if let Some(discovery) = self.discover_indexed_log_state(
            run_index_path(&self.buddy_home),
            RUN_INDEX_EVENT_TYPE,
            RUN_DELETED_EVENT_TYPE,
            validate_indexed_run_log_path,
            |payload| payload.run_id.as_deref(),
        )? {
            return Ok(discovery);
        }

        Ok(LocalLogDiscovery {
            active_log_paths: self.discover_jsonl_logs(RUNS_DIR_NAME)?,
            deleted_entity_ids: Vec::new(),
        })
    }

    pub fn append_event(
        &self,
        path: &Path,
        event_type: impl Into<String>,
        payload: Value,
    ) -> BuddyResult<LocalLogEvent> {
        let event = LocalLogEvent {
            event_id: self.next_event_id(),
            event_type: event_type.into(),
            payload,
            timestamp: self.timestamp().to_rfc3339_millis(),
        };
        append_jsonl_event(path, &event)?;

        Ok(event)
    }

    pub fn append_conversation_index_entry(
        &self,
        conversation_id: &str,
        log_path: &str,
    ) -> BuddyResult<LocalLogEvent> {
        let path = conversation_index_path(&self.buddy_home);

        self.append_event(
            &path,
            CONVERSATION_INDEX_EVENT_TYPE,
            serde_json::json!({
                "conversationId": conversation_id,
                "logPath": log_path,
            }),
        )
    }

    pub fn append_run_index_entry(
        &self,
        run_id: &str,
        log_path: &str,
    ) -> BuddyResult<LocalLogEvent> {
        let path = run_index_path(&self.buddy_home);

        self.append_event(
            &path,
            RUN_INDEX_EVENT_TYPE,
            serde_json::json!({
                "logPath": log_path,
                "runId": run_id,
            }),
        )
    }

    pub fn append_conversation_deleted_index_entry(
        &self,
        conversation_id: &str,
        log_path: &str,
    ) -> BuddyResult<LocalLogEvent> {
        self.append_event(
            &conversation_index_path(&self.buddy_home),
            CONVERSATION_DELETED_EVENT_TYPE,
            serde_json::json!({
                "conversationId": conversation_id,
                "logPath": log_path,
            }),
        )
    }

    pub fn append_run_deleted_index_entry(
        &self,
        run_id: &str,
        log_path: &str,
    ) -> BuddyResult<LocalLogEvent> {
        self.append_event(
            &run_index_path(&self.buddy_home),
            RUN_DELETED_EVENT_TYPE,
            serde_json::json!({
                "logPath": log_path,
                "runId": run_id,
            }),
        )
    }

    pub fn remove_log_file(&self, relative_path: &str) -> BuddyResult<()> {
        let path = self.checked_absolute_path(relative_path)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(test)]
    pub fn absolute_path_for_test(&self, relative_path: &str) -> PathBuf {
        self.absolute_path(relative_path)
    }

    fn next_event_id(&self) -> String {
        match &self.event_ids {
            LocalLogEventIdSource::Uuid => uuid::Uuid::new_v4().to_string(),
            LocalLogEventIdSource::Counter { next, prefix } => {
                let id = next.fetch_add(1, Ordering::SeqCst);
                format!("{prefix}-{id}")
            }
        }
    }

    fn timestamp(&self) -> LocalLogTimestamp {
        match self.timestamps {
            LocalLogTimestampSource::System => LocalLogTimestamp::now_utc(),
            LocalLogTimestampSource::Fixed(timestamp) => timestamp,
        }
    }

    fn discover_jsonl_logs(&self, root_name: &str) -> BuddyResult<Vec<String>> {
        let root = self.buddy_home.join(root_name);
        if !root.is_dir() {
            return Ok(Vec::new());
        }

        let mut logs = Vec::new();
        collect_jsonl_logs(&self.buddy_home, &root, &mut logs)?;
        logs.sort();

        Ok(logs)
    }

    fn discover_indexed_log_state(
        &self,
        index_path: PathBuf,
        indexed_event_type: &str,
        deleted_event_type: &str,
        validate_log_path: fn(&str) -> BuddyResult<String>,
        entity_id: fn(&LocalLogIndexPayload) -> Option<&str>,
    ) -> BuddyResult<Option<LocalLogDiscovery>> {
        if !index_path.is_file() {
            return Ok(None);
        }

        let content = fs::read_to_string(index_path)?;
        let mut entity_logs = BTreeMap::new();
        let mut recognized_event = false;
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed: LocalLogIndexLine = serde_json::from_str(line).map_err(|error| {
                BuddyError::Validation(format!(
                    "local log index line {} is not valid JSON: {}",
                    index + 1,
                    error
                ))
            })?;
            if parsed.schema_version != 1 {
                return Err(BuddyError::Validation(format!(
                    "unsupported local log index schema version {}",
                    parsed.schema_version
                )));
            }
            if parsed.event_type != indexed_event_type && parsed.event_type != deleted_event_type {
                continue;
            }
            recognized_event = true;
            let log_path = validate_log_path(&parsed.payload.log_path)?;
            let entity_id = entity_id(&parsed.payload)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    BuddyError::Validation("local log index entity id is required".to_owned())
                })?;
            entity_logs.insert(
                entity_id.to_owned(),
                (parsed.event_type == indexed_event_type).then_some(log_path),
            );
        }
        if !recognized_event {
            return Ok(None);
        }

        let mut active_log_paths = entity_logs
            .values()
            .filter_map(Clone::clone)
            .collect::<Vec<_>>();
        active_log_paths.sort();
        active_log_paths.dedup();
        let deleted_entity_ids = entity_logs
            .into_iter()
            .filter_map(|(entity_id, log_path)| log_path.is_none().then_some(entity_id))
            .collect();

        Ok(Some(LocalLogDiscovery {
            active_log_paths,
            deleted_entity_ids,
        }))
    }
}

fn validate_indexed_conversation_log_path(log_path: &str) -> BuddyResult<String> {
    validate_indexed_local_log_path(CONVERSATIONS_DIR_NAME, log_path)
}

fn validate_indexed_run_log_path(log_path: &str) -> BuddyResult<String> {
    validate_indexed_local_log_path(RUNS_DIR_NAME, log_path)
}

fn validate_indexed_local_log_path(root_name: &str, log_path: &str) -> BuddyResult<String> {
    let log_path = log_path.trim();
    if log_path.is_empty() {
        return Err(BuddyError::Validation(
            "local log index logPath is required".to_owned(),
        ));
    }
    let path = Path::new(log_path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(BuddyError::Validation(
            "local log index logPath must stay under Buddy home".to_owned(),
        ));
    }
    if !log_path.starts_with(&format!("{root_name}/")) || !log_path.ends_with(".jsonl") {
        return Err(BuddyError::Validation(
            "local log index logPath must point to the expected JSONL root".to_owned(),
        ));
    }

    Ok(log_path.to_owned())
}

fn collect_jsonl_logs(
    buddy_home: &Path,
    directory: &Path,
    logs: &mut Vec<String>,
) -> BuddyResult<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_jsonl_logs(buddy_home, &path, logs)?;
            continue;
        }
        if !file_type.is_file()
            || path
                .extension()
                .and_then(|value| value.to_str())
                .is_none_or(|extension| extension != "jsonl")
        {
            continue;
        }
        let relative_path = path.strip_prefix(buddy_home).map_err(|_| {
            BuddyError::Validation("local log path must stay under Buddy home".to_owned())
        })?;
        logs.push(relative_path.to_string_lossy().into_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::LocalLogRuntime;
    use crate::local_log::LocalLogTimestamp;

    #[test]
    fn checked_absolute_path_rejects_paths_outside_buddy_home() {
        let runtime = LocalLogRuntime::fixed_for_test(
            PathBuf::from("/tmp/lexora-buddy"),
            LocalLogTimestamp::new(2026, 7, 6, 9, 8, 7),
        );

        assert!(runtime.checked_absolute_path("/tmp/run.jsonl").is_err());
        assert!(runtime
            .checked_absolute_path("../outside/run.jsonl")
            .is_err());
        assert_eq!(
            runtime
                .checked_absolute_path("runs/2026/07/06/run.jsonl")
                .expect("relative path should be accepted"),
            PathBuf::from("/tmp/lexora-buddy/runs/2026/07/06/run.jsonl")
        );
    }

    #[test]
    fn discovers_jsonl_logs_under_stable_buddy_subdirectories() {
        let buddy_home = std::env::temp_dir().join(format!(
            "lexora-buddy-discover-log-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(buddy_home.join("conversations/2026/07/06"))
            .expect("create conversation log dir");
        std::fs::create_dir_all(buddy_home.join("runs/2026/07/06")).expect("create run log dir");
        std::fs::write(
            buddy_home.join("conversations/2026/07/06/conversation-a.jsonl"),
            "",
        )
        .expect("write conversation log");
        std::fs::write(buddy_home.join("runs/2026/07/06/run-a.jsonl"), "").expect("write run log");
        std::fs::write(buddy_home.join("runs/2026/07/06/run-a.txt"), "")
            .expect("write ignored file");
        let runtime = LocalLogRuntime::fixed_for_test(
            buddy_home.clone(),
            LocalLogTimestamp::new(2026, 7, 6, 9, 8, 7),
        );

        assert_eq!(
            runtime
                .discover_conversation_logs()
                .expect("discover conversation logs"),
            vec!["conversations/2026/07/06/conversation-a.jsonl"]
        );
        assert_eq!(
            runtime.discover_run_logs().expect("discover run logs"),
            vec!["runs/2026/07/06/run-a.jsonl"]
        );

        std::fs::remove_dir_all(buddy_home).expect("cleanup");
    }
}
