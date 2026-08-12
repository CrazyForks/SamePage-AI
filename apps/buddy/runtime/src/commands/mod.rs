use std::fmt;

use crate::{
    error::{BuddyError, BuddyResult},
    state::BuddyAppState,
    storage::BuddyRun,
};

pub(crate) mod agent_turn;
pub(crate) mod approval;
pub(crate) mod attachment;
mod chat_input;
mod codex_runtime;
mod host_action;
pub(crate) mod run_state;
mod runtime_events;

pub use run_state::BuddyRunCancellationRegistry;

pub(crate) type BuddyCommandResult<T> = Result<T, BuddyCommandError>;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyCommandError {
    code: &'static str,
    message: String,
    public_message: &'static str,
    retryable: bool,
}

impl From<BuddyError> for BuddyCommandError {
    fn from(error: BuddyError) -> Self {
        Self {
            code: error.public_code(),
            message: error.to_string(),
            public_message: error.public_message(),
            retryable: error.retryable(),
        }
    }
}

impl BuddyCommandError {
    pub(crate) fn public_code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn public_message(&self) -> &'static str {
        self.public_message
    }

    pub(crate) fn retryable(&self) -> bool {
        self.retryable
    }
}

impl fmt::Display for BuddyCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BuddyCommandError {}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn read_json_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn resolve_runtime_cwd(
    state: &BuddyAppState,
    requested_cwd: Option<String>,
) -> BuddyResult<(String, Option<String>)> {
    let requested_cwd = requested_cwd
        .as_deref()
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty());
    if let Some(requested_cwd) = requested_cwd {
        let project = state
            .find_project(requested_cwd)?
            .ok_or_else(|| BuddyError::Validation("project is not authorized yet".to_owned()))?;

        return Ok((project.root.clone(), Some(project.root)));
    }

    Ok((state.global_runtime_cwd(), None))
}

pub(crate) fn cancel_buddy_chat_run(
    state: &BuddyAppState,
    cancellations: &BuddyRunCancellationRegistry,
    event_publisher: run_state::BuddyRunStateEventPublisher,
    run_id: String,
) -> BuddyCommandResult<BuddyRun> {
    let run = state.storage_handle().find_run(run_id.clone())?;
    if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
        return Ok(run);
    }
    if !cancellations.cancel(&run_id) {
        return Err(BuddyError::Validation("run can no longer be cancelled".to_owned()).into());
    }
    event_publisher.emit_run(&run);

    Ok(run)
}
