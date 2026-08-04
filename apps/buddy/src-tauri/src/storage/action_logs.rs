use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::{
    agents::redaction::sanitize_process_stderr_line,
    choreography::{
        action_log::{
            ActionLogEvent, ActionLogRuntimeRestartStepInterruption, ActionLogSystemEvent,
        },
        admission::{ChoreographyPlanPriority, ChoreographyTriggerSource},
    },
    error::{BuddyError, BuddyResult},
    native_pet::step_protocol::SidecarInterruptPolicy,
};

use super::{
    choreography_pending_bodies::{
        self, ChoreographyPendingExecutionBodyKind, UpsertChoreographyPendingExecutionBodyRequest,
    },
    BuddyStorage, ReplayableChoreographyPendingExecutionBody,
};

const ACTION_LOG_JSONL_RELATIVE_PATH: &str = "action-log/events.jsonl";
const ACTION_LOG_SCHEMA_VERSION: u16 = 1;
const ACTION_LOG_PLAN_LIST_DEFAULT_LIMIT: i64 = 50;
const ACTION_LOG_PLAN_LIST_MAX_LIMIT: i64 = 100;
const ACTION_LOG_PLAN_LIST_CURSOR_PREFIX: &str = "v1:";
const ACTION_LOG_SYSTEM_EVENT_DEFAULT_LIMIT: i64 = 100;
const ACTION_LOG_SYSTEM_EVENT_MAX_LIMIT: i64 = 500;
const ACTION_LOG_SYSTEM_EVENT_FORBIDDEN_PREFIXES: &[&str] =
    &["plan.", "step.", "fallback.", "result.", "systemRecovery."];
const ACTION_LOG_SOURCE_REF_KIND_SYSTEM_RECOVERY: &str = "systemRecovery";
const ACTION_LOG_RESULT_KIND_NORMAL: &str = "normal";
const ACTION_LOG_RESULT_KIND_FALLBACK: &str = "fallback";
const ACTION_LOG_RESULT_KIND_DEGRADED: &str = "degraded";
const ACTION_LOG_RESULT_KIND_INTERRUPTED: &str = "interrupted";
const ACTION_LOG_DETAIL_STATUS_COMPLETED: &str = "completed";
const ACTION_LOG_DETAIL_STATUS_FAILED: &str = "failed";
const ACTION_LOG_DETAIL_STATUS_REJECTED: &str = "rejected";
const ACTION_LOG_DETAIL_STATUS_RUNNING: &str = "running";
const ACTION_LOG_DETAIL_STATUS_SKIPPED: &str = "skipped";
const ACTION_LOG_DETAIL_STATUS_DEFERRED: &str = "deferred";
const ACTION_LOG_INDEX_STATUS_FAILED: &str = "failed";
const ACTION_LOG_INDEX_STATUS_FRESH: &str = "fresh";
const ACTION_LOG_INDEX_STATUS_STALE: &str = "stale";
const ACTION_LOG_DIAGNOSTIC_DETAIL_ALLOWED_FIELDS: &[&str] =
    &["message", "rawCode", "source", "items", "truncated"];
const ACTION_LOG_DIAGNOSTIC_DETAIL_ITEM_ALLOWED_FIELDS: &[&str] = &["key", "value"];
const ACTION_LOG_DIAGNOSTIC_DETAIL_FIELD_MAX_CHARS: usize = 512;
const ACTION_LOG_DIAGNOSTIC_DETAIL_TOTAL_MAX_BYTES: usize = 2048;
const ACTION_LOG_DIAGNOSTIC_DETAIL_TOTAL_FIELD_FLOOR_CHARS: usize = 64;
const ACTION_LOG_DIAGNOSTIC_DETAIL_PATH_REDACTION: &str = "[path]";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogPlanListRequest {
    pub limit: Option<i64>,
    pub page_cursor: Option<String>,
    pub last_event_type: Option<String>,
    pub last_reason_code: Option<String>,
    pub plan_id: Option<String>,
    pub resolved_action_id: Option<String>,
    pub resolved_animation_ref: Option<String>,
    pub result_kind: Option<String>,
    pub source_ref_id: Option<String>,
    pub source_ref_kind: Option<String>,
    pub started_at_from: Option<String>,
    pub started_at_to: Option<String>,
    pub status: Option<String>,
    pub trigger_source: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogSystemEventQueryRequest {
    pub event_type: Option<String>,
    pub source_ref_kind: Option<String>,
    pub reason_code: Option<String>,
    pub status: Option<String>,
    pub created_at_from: Option<String>,
    pub created_at_to: Option<String>,
    pub limit: Option<i64>,
    pub plan_id: Option<String>,
    pub step_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogPlanList {
    pub items: Vec<ActionLogPlanSummary>,
    pub next_page_cursor: Option<String>,
    pub has_more: bool,
    pub index_stale: bool,
    pub index_status: &'static str,
    pub last_indexed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogSystemEventQueryResult {
    pub items: Vec<ActionLogSystemEventSummary>,
    pub limit: i64,
    pub has_more: bool,
    pub index_stale: bool,
    pub index_status: &'static str,
    pub last_indexed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogPlanSummary {
    pub plan_id: String,
    pub source_ref_kind: String,
    pub source_ref_id: Option<String>,
    pub source_ref: serde_json::Value,
    pub source_display: Option<ActionLogSourceDisplay>,
    pub status: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub last_event_type: String,
    pub last_reason_code: String,
    pub detail_status: String,
    pub detail_reason_code: String,
    pub resolved_action_id: Option<String>,
    pub resolved_animation_ref: Option<String>,
    pub result_kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogSourceDisplay {
    pub kind: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogSystemEventSummary {
    pub event_id: String,
    pub event_type: String,
    pub timestamp: String,
    pub source_ref: ActionLogSystemEventSourceRefSummary,
    pub trigger_source: String,
    pub status: String,
    pub reason_code: String,
    pub plan_id: Option<String>,
    pub step_id: Option<String>,
    pub index_status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogSystemEventSourceRefSummary {
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecoverableChoreographyPendingAdmission {
    pub(crate) plan_id: String,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) trigger_source: ChoreographyTriggerSource,
    pub(crate) priority: ChoreographyPlanPriority,
    pub(crate) reason_code: String,
    pub(crate) active_plan_id: String,
    pub(crate) active_step_id: Option<String>,
    pub(crate) active_priority: ChoreographyPlanPriority,
    pub(crate) active_step_interrupt_policy: SidecarInterruptPolicy,
    pub(crate) body_kind: ChoreographyPendingExecutionBodyKind,
    pub(crate) body_schema_version: u16,
    pub(crate) deferred_event_id: String,
    pub(crate) deferred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecoverableChoreographyPendingExecution {
    pub(crate) admission: RecoverableChoreographyPendingAdmission,
    pub(crate) body: ReplayableChoreographyPendingExecutionBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChoreographyActionLogIndexHealth {
    Fresh,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogPlanDetail {
    pub plan: ActionLogPlanSummary,
    pub steps: Vec<ActionLogStepDetail>,
    pub recovery_plans: Vec<ActionLogRelatedPlanDetail>,
    pub index_stale: bool,
    pub index_status: &'static str,
    pub last_indexed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogRelatedPlanDetail {
    pub plan: ActionLogPlanSummary,
    pub steps: Vec<ActionLogStepDetail>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogStepDetail {
    pub step_id: String,
    pub status: String,
    pub reason_code: String,
    pub step_kind: Option<String>,
    pub target_label: Option<String>,
    pub resolved_action_id: Option<String>,
    pub resolved_animation_ref: Option<String>,
    pub resolved_at: Option<String>,
    pub completed_at: Option<String>,
    pub failed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub event_count: u64,
}

#[derive(Debug, Clone)]
struct ActionLogPlanEventRecord {
    event_type: String,
    status: String,
    reason_code: String,
    step_id: Option<String>,
    payload: serde_json::Value,
    created_at: String,
}

struct ActionLogPlanListCursor {
    started_at: String,
    plan_id: String,
}

struct ActionLogJsonlAppendCursor {
    source_file: &'static str,
    byte_offset: i64,
    line_number: i64,
}

struct ActionLogJsonlAppendPosition {
    cursor: ActionLogJsonlAppendCursor,
    needs_leading_newline: bool,
}

struct ActionLogIndexTailInspection {
    append_position: Option<ActionLogJsonlAppendPosition>,
    last_indexed_at: Option<String>,
}

struct ActionLogIndexState {
    stale: bool,
    status: &'static str,
    last_indexed_at: Option<String>,
}

struct ActionLogIndexWatermark {
    byte_offset: i64,
    line_number: i64,
    event_id: String,
    updated_at: String,
}

enum ActionLogIndexSyncPlan {
    Fresh,
    Incremental { byte_offset: i64, line_number: i64 },
    Rebuild,
}

struct ActionLogJsonlReplayRecord {
    cursor: ActionLogJsonlAppendCursor,
    event: ActionLogJsonlReplayEvent,
}

struct StaleActionLogPlanAfterStartup {
    plan_id: String,
    source_ref: serde_json::Value,
    status: String,
    last_event_type: String,
    last_reason_code: String,
    stale_steps: Vec<StaleActionLogStepAfterStartup>,
}

struct StaleActionLogStepAfterStartup {
    last_event_rowid: i64,
    step_id: String,
    status: String,
    last_event_type: String,
    last_reason_code: String,
}

struct RecoverableChoreographyPendingAdmissionRow {
    deferred_event_rowid: i64,
    deferred_event_id: String,
    plan_id: String,
    source_ref: serde_json::Value,
    payload: serde_json::Value,
    reason_code: String,
    deferred_at: String,
}

struct RecoverableChoreographyPendingAdmissionCandidate {
    plan_id: String,
    source_ref: serde_json::Value,
    trigger_source: ChoreographyTriggerSource,
    priority: ChoreographyPlanPriority,
    reason_code: String,
    active_plan_id: String,
    active_step_id: Option<String>,
    active_priority: ChoreographyPlanPriority,
    active_step_interrupt_policy: SidecarInterruptPolicy,
    deferred_event_id: String,
    deferred_at: String,
}

impl RecoverableChoreographyPendingAdmissionCandidate {
    fn into_admission(
        self,
        body: &ReplayableChoreographyPendingExecutionBody,
    ) -> RecoverableChoreographyPendingAdmission {
        RecoverableChoreographyPendingAdmission {
            plan_id: self.plan_id,
            source_ref: self.source_ref,
            trigger_source: self.trigger_source,
            priority: self.priority,
            reason_code: self.reason_code,
            active_plan_id: self.active_plan_id,
            active_step_id: self.active_step_id,
            active_priority: self.active_priority,
            active_step_interrupt_policy: self.active_step_interrupt_policy,
            body_kind: body.body_kind,
            body_schema_version: body.schema_version,
            deferred_event_id: self.deferred_event_id,
            deferred_at: self.deferred_at,
        }
    }
}

enum ActionLogJsonlReplayEvent {
    Plan(ActionLogEvent),
    System(ActionLogSystemEvent),
}

struct ActionLogSystemSourceRefProjection<'a> {
    kind: &'a str,
    source_ref_id: Option<&'a str>,
}

struct ActionLogSystemSourceRefSchema {
    required_fields: &'static [&'static str],
    allowed_fields: &'static [&'static str],
}

impl BuddyStorage {
    pub(crate) fn append_choreography_action_log_event(
        &self,
        event: &ActionLogEvent,
    ) -> BuddyResult<()> {
        let event = sanitize_action_log_event(event)?;
        validate_action_log_event_schema(&event)?;
        let source_ref_projection = validate_action_log_source_ref(&event.source_ref)?;
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        for attempt in 0..2 {
            let appended =
                self.with_mut_connection("append_choreography_action_log_event", |connection| {
                    let transaction =
                        connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let inspection = inspect_action_log_index_tail(&transaction, &jsonl_path)?;
                    let Some(append_position) = inspection.append_position else {
                        return Ok(false);
                    };
                    ensure_action_log_index_event_id_is_new(&transaction, &event.event_id)?;
                    let append_cursor = append_action_log_jsonl_line_at_position(
                        jsonl_path.clone(),
                        &event,
                        append_position,
                    )?;
                    insert_action_log_event(&transaction, &event, &source_ref_projection)?;
                    upsert_action_log_plan_summary(&transaction, &event, &source_ref_projection)?;
                    project_choreography_pending_execution_body_cache_from_plan_event(
                        &transaction,
                        &event,
                    )?;
                    upsert_action_log_index_watermark(&transaction, &event, &append_cursor)?;
                    transaction.commit()?;

                    Ok(true)
                })?;
            if appended {
                return Ok(());
            }
            if attempt == 0 {
                self.sync_choreography_action_log_index_unlocked()?;
            }
        }

        Err(BuddyError::Runtime(
            "action log index is not contiguous after synchronization".to_owned(),
        ))
    }

    pub(crate) fn append_choreography_action_log_system_event(
        &self,
        event: &ActionLogSystemEvent,
    ) -> BuddyResult<()> {
        let event = sanitize_action_log_system_event(event)?;
        validate_action_log_system_event_schema(&event)?;
        let source_ref_projection = validate_action_log_system_source_ref(&event.source_ref)?;
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        for attempt in 0..2 {
            let appended = self.with_mut_connection(
                "append_choreography_action_log_system_event",
                |connection| {
                    let transaction =
                        connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let inspection = inspect_action_log_index_tail(&transaction, &jsonl_path)?;
                    let Some(append_position) = inspection.append_position else {
                        return Ok(false);
                    };
                    ensure_action_log_index_event_id_is_new(&transaction, &event.event_id)?;
                    let append_cursor = append_action_log_jsonl_line_at_position(
                        jsonl_path.clone(),
                        &event,
                        append_position,
                    )?;
                    insert_action_log_system_event(&transaction, &event, &source_ref_projection)?;
                    project_choreography_pending_execution_body_cache_from_system_event(
                        &transaction,
                        &event,
                    )?;
                    upsert_action_log_system_index_watermark(&transaction, &event, &append_cursor)?;
                    transaction.commit()?;

                    Ok(true)
                },
            )?;
            if appended {
                return Ok(());
            }
            if attempt == 0 {
                self.sync_choreography_action_log_index_unlocked()?;
            }
        }

        Err(BuddyError::Runtime(
            "action log index is not contiguous after synchronization".to_owned(),
        ))
    }

    pub(crate) fn append_choreography_action_log_unindexed_system_event(
        &self,
        event: &ActionLogSystemEvent,
    ) -> BuddyResult<()> {
        let event = sanitize_action_log_system_event(event)?;
        validate_action_log_system_event_schema(&event)?;
        let source_ref_projection = validate_action_log_system_source_ref(&event.source_ref)?;
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_mut_connection(
            "append_choreography_action_log_unindexed_system_event",
            |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                ensure_action_log_jsonl_event_id_is_new(&jsonl_path, &event.event_id)?;
                let append_position = next_action_log_jsonl_append_position(&jsonl_path)?;
                append_action_log_jsonl_line_at_position(jsonl_path, &event, append_position)?;
                insert_action_log_system_event(&transaction, &event, &source_ref_projection)?;
                project_choreography_pending_execution_body_cache_from_system_event(
                    &transaction,
                    &event,
                )?;
                transaction.commit()?;

                Ok(())
            },
        )
    }

    fn lock_action_log_writer(&self) -> BuddyResult<std::sync::MutexGuard<'_, ()>> {
        self.action_log_writer
            .lock()
            .map_err(|_| BuddyError::Runtime("action log writer lock was poisoned".to_owned()))
    }

    pub(crate) fn sync_choreography_action_log_index(&self) -> BuddyResult<()> {
        let _writer = self.lock_action_log_writer()?;
        self.sync_choreography_action_log_index_unlocked()
    }

    fn sync_choreography_action_log_index_unlocked(&self) -> BuddyResult<()> {
        let jsonl_path = self.action_log_jsonl_path();
        self.with_mut_connection("sync_choreography_action_log_index", |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let jsonl_bytes = read_action_log_jsonl_bytes(&jsonl_path)?;
            validate_action_log_jsonl_replay_input(&jsonl_bytes)?;
            let sync_plan = plan_action_log_index_sync(&transaction, &jsonl_bytes)?;
            match sync_plan {
                ActionLogIndexSyncPlan::Fresh => {}
                ActionLogIndexSyncPlan::Incremental {
                    byte_offset,
                    line_number,
                } => {
                    let records = read_action_log_jsonl_replay_records(
                        &jsonl_bytes,
                        byte_offset,
                        line_number,
                    )?;
                    for record in records {
                        index_action_log_jsonl_replay_record(&transaction, &record)?;
                    }
                }
                ActionLogIndexSyncPlan::Rebuild => {
                    reset_action_log_index_projection(&transaction)?;
                    choreography_pending_bodies::clear_choreography_pending_execution_bodies(
                        &transaction,
                    )?;
                    let records = read_action_log_jsonl_replay_records(&jsonl_bytes, 0, 1)?;
                    for record in records {
                        index_action_log_jsonl_replay_record(&transaction, &record)?;
                    }
                }
            }
            transaction.commit()?;

            Ok(())
        })
    }

    pub(crate) fn choreography_action_log_index_health(
        &self,
    ) -> BuddyResult<ChoreographyActionLogIndexHealth> {
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_connection("choreography_action_log_index_health", |connection| {
            Ok(choreography_action_log_index_health_from_state(
                read_action_log_index_state(connection, &jsonl_path),
            ))
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn rebuild_choreography_pending_execution_body_cache_from_action_log(
        &self,
    ) -> BuddyResult<usize> {
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_mut_connection(
            "rebuild_choreography_pending_execution_body_cache_from_action_log",
            |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let jsonl_bytes = read_action_log_jsonl_bytes(&jsonl_path)?;
                validate_action_log_jsonl_replay_input(&jsonl_bytes)?;
                let records = read_action_log_jsonl_replay_records(&jsonl_bytes, 0, 1)?;
                choreography_pending_bodies::clear_choreography_pending_execution_bodies(
                    &transaction,
                )?;
                for record in &records {
                    rebuild_choreography_pending_execution_body_cache_from_record(
                        &transaction,
                        record,
                    )?;
                }
                let rebuilt_count = count_choreography_pending_execution_bodies(&transaction)?;
                transaction.commit()?;

                Ok(rebuilt_count)
            },
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn list_recoverable_choreography_pending_admissions_after_startup(
        &self,
    ) -> BuddyResult<Vec<RecoverableChoreographyPendingAdmission>> {
        Ok(self
            .list_recoverable_choreography_pending_executions_after_startup()?
            .into_iter()
            .map(|execution| execution.admission)
            .collect())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn list_recoverable_choreography_pending_executions_after_startup(
        &self,
    ) -> BuddyResult<Vec<RecoverableChoreographyPendingExecution>> {
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_mut_connection(
            "list_recoverable_choreography_pending_executions_after_startup",
            |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let jsonl_bytes = read_action_log_jsonl_bytes(&jsonl_path)?;
                validate_action_log_jsonl_replay_input(&jsonl_bytes)?;
                let records = read_action_log_jsonl_replay_records(&jsonl_bytes, 0, 1)?;
                let candidates =
                    list_recoverable_choreography_pending_admission_candidates_after_startup(
                        &transaction,
                    )?;
                let mut executions = Vec::with_capacity(candidates.len());
                for candidate in candidates {
                    let Some(body) = replayable_pending_execution_body_from_records(
                        &candidate.plan_id,
                        &records,
                    )?
                    else {
                        continue;
                    };
                    let admission = candidate.into_admission(&body);
                    validate_recoverable_pending_execution_body_matches_admission(
                        &admission, &body,
                    )?;
                    executions.push(RecoverableChoreographyPendingExecution { admission, body });
                }
                transaction.commit()?;

                Ok(executions)
            },
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn find_replayable_choreography_pending_execution_body_from_action_log(
        &self,
        plan_id: &str,
    ) -> BuddyResult<Option<ReplayableChoreographyPendingExecutionBody>> {
        let plan_id = plan_id.trim();
        if plan_id.is_empty() {
            return Ok(None);
        }

        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_mut_connection(
            "find_replayable_choreography_pending_execution_body_from_action_log",
            |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let jsonl_bytes = read_action_log_jsonl_bytes(&jsonl_path)?;
                validate_action_log_jsonl_replay_input(&jsonl_bytes)?;
                let records = read_action_log_jsonl_replay_records(&jsonl_bytes, 0, 1)?;
                let body = replayable_pending_execution_body_from_records(plan_id, &records)?;
                transaction.commit()?;

                Ok(body)
            },
        )
    }

    pub(crate) fn reconcile_stale_choreography_action_log_plans_after_startup(
        &self,
        created_at: &str,
    ) -> BuddyResult<usize> {
        let stale_plans = self.with_connection(
            "list_stale_choreography_action_log_plans_after_startup",
            list_stale_choreography_action_log_plans_after_startup,
        )?;
        let count = stale_plans.len();
        for plan in stale_plans {
            for step in plan.stale_steps {
                let event = ActionLogEvent::step_interrupted_after_runtime_restart(
                    ActionLogRuntimeRestartStepInterruption {
                        event_id: format!("evt_{}", uuid::Uuid::now_v7()),
                        plan_id: plan.plan_id.as_str(),
                        step_id: step.step_id.as_str(),
                        source_ref: &plan.source_ref,
                        previous_status: step.status.as_str(),
                        previous_event_type: step.last_event_type.as_str(),
                        previous_reason_code: step.last_reason_code.as_str(),
                        created_at,
                    },
                );
                self.append_choreography_action_log_event(&event)?;
            }
            let event = ActionLogEvent::plan_interrupted_after_runtime_restart(
                format!("evt_{}", uuid::Uuid::now_v7()),
                plan.plan_id,
                plan.source_ref,
                plan.status,
                plan.last_event_type,
                plan.last_reason_code,
                created_at,
            );
            self.append_choreography_action_log_event(&event)?;
        }

        Ok(count)
    }

    fn action_log_jsonl_path(&self) -> PathBuf {
        self.local_logs
            .absolute_path(ACTION_LOG_JSONL_RELATIVE_PATH)
    }

    pub fn list_action_log_plans(
        &self,
        request: ActionLogPlanListRequest,
    ) -> BuddyResult<ActionLogPlanList> {
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_connection("list_action_log_plans", |connection| {
            let index_state = read_action_log_index_state(connection, &jsonl_path);
            list_action_log_plans(connection, request, index_state)
        })
    }

    pub fn query_action_log_system_events(
        &self,
        request: ActionLogSystemEventQueryRequest,
    ) -> BuddyResult<ActionLogSystemEventQueryResult> {
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_connection("query_action_log_system_events", |connection| {
            let index_state = read_action_log_index_state(connection, &jsonl_path);
            query_action_log_system_events(connection, request, index_state)
        })
    }

    pub fn get_action_log_plan_detail(&self, plan_id: &str) -> BuddyResult<ActionLogPlanDetail> {
        let _writer = self.lock_action_log_writer()?;
        let jsonl_path = self.action_log_jsonl_path();
        self.with_connection("get_action_log_plan_detail", |connection| {
            let index_state = read_action_log_index_state(connection, &jsonl_path);
            let plan = find_action_log_plan_summary(connection, plan_id)?;
            let events = list_action_log_plan_events(connection, plan_id)?;
            let steps = build_action_log_step_details(events);
            let recovery_plans = list_related_system_recovery_plan_details(connection, plan_id)?;

            Ok(ActionLogPlanDetail {
                plan,
                steps,
                recovery_plans,
                index_stale: index_state.stale,
                index_status: index_state.status,
                last_indexed_at: index_state.last_indexed_at,
            })
        })
    }

    #[cfg(test)]
    pub fn read_action_log_jsonl_lines_for_test(&self) -> Vec<String> {
        let content =
            std::fs::read_to_string(self.action_log_jsonl_path()).expect("read action log jsonl");

        content.lines().map(str::to_owned).collect()
    }

    #[cfg(test)]
    pub fn action_log_event_types_for_test(&self, plan_id: &str) -> Vec<String> {
        self.with_connection("action_log_event_types_for_test", |connection| {
            let mut statement = connection.prepare(
                r#"
                SELECT event_type
                FROM action_log_events
                WHERE plan_id = ?1
                ORDER BY rowid ASC
                "#,
            )?;
            let rows = statement.query_map(params![plan_id], |row| row.get::<_, String>(0))?;

            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .expect("read action log event types")
    }

    #[cfg(test)]
    pub fn action_log_plan_summary_for_test(&self, plan_id: &str) -> serde_json::Value {
        self.with_connection("action_log_plan_summary_for_test", |connection| {
            connection
                .query_row(
                    r#"
                    SELECT status, last_event_type, last_reason_code, resolved_action_id, resolved_animation_ref
                    FROM action_log_plan_summaries
                    WHERE plan_id = ?1
                    "#,
                    params![plan_id],
                    |row| {
                        Ok(serde_json::json!({
                            "status": row.get::<_, String>(0)?,
                            "lastEventType": row.get::<_, String>(1)?,
                            "lastReasonCode": row.get::<_, String>(2)?,
                            "resolvedActionId": row.get::<_, Option<String>>(3)?,
                            "resolvedAnimationRef": row.get::<_, Option<String>>(4)?,
                        }))
                    },
                )
                .map_err(Into::into)
        })
        .expect("read action log plan summary")
    }
}

fn list_action_log_plans(
    connection: &Connection,
    request: ActionLogPlanListRequest,
    index_state: ActionLogIndexState,
) -> BuddyResult<ActionLogPlanList> {
    let limit = normalize_action_log_plan_list_limit(request.limit);
    let cursor = request
        .page_cursor
        .as_deref()
        .map(parse_action_log_plan_list_cursor)
        .transpose()?;
    let cursor_started_at = cursor.as_ref().map(|cursor| cursor.started_at.as_str());
    let cursor_plan_id = cursor.as_ref().map(|cursor| cursor.plan_id.as_str());
    let include_system_recovery_plans = include_system_recovery_plans_in_list(&request);

    let mut statement = connection.prepare(
        r#"
        SELECT
          plan_id,
          source_ref_kind,
          source_ref_id,
          result_kind,
          source_ref_json,
          status,
          started_at,
          completed_at,
          last_event_type,
          last_reason_code,
          detail_status,
          detail_reason_code,
          resolved_action_id,
          resolved_animation_ref
        FROM action_log_plan_summaries AS plans
        WHERE (?1 IS NULL OR plans.plan_id = ?1)
          AND (?2 IS NULL OR plans.status = ?2)
          AND (?3 IS NULL OR plans.source_ref_kind = ?3)
          AND (?4 IS NULL OR plans.source_ref_id = ?4)
          AND (?5 IS NULL OR plans.result_kind = ?5)
          AND (?6 IS NULL OR plans.last_event_type = ?6)
          AND (?7 IS NULL OR plans.last_reason_code = ?7)
          AND (?8 IS NULL OR plans.resolved_action_id = ?8)
          AND (?9 IS NULL OR plans.resolved_animation_ref = ?9)
          AND (?10 IS NULL OR plans.started_at >= ?10)
          AND (?11 IS NULL OR plans.started_at <= ?11)
          AND (?12 OR plans.source_ref_kind != 'systemRecovery')
          AND (
            ?13 IS NULL
            OR EXISTS (
              SELECT 1
              FROM action_log_events AS events
              WHERE events.plan_id = plans.plan_id
                AND events.trigger_source = ?13
            )
          )
          AND (
            ?14 IS NULL
            OR plans.started_at < ?14
            OR (plans.started_at = ?14 AND plans.plan_id < ?15)
          )
        ORDER BY plans.started_at DESC, plans.plan_id DESC
        LIMIT ?16
        "#,
    )?;
    let rows = statement.query_map(
        params![
            request.plan_id.as_deref(),
            request.status.as_deref(),
            request.source_ref_kind.as_deref(),
            request.source_ref_id.as_deref(),
            request.result_kind.as_deref(),
            request.last_event_type.as_deref(),
            request.last_reason_code.as_deref(),
            request.resolved_action_id.as_deref(),
            request.resolved_animation_ref.as_deref(),
            request.started_at_from.as_deref(),
            request.started_at_to.as_deref(),
            include_system_recovery_plans,
            request.trigger_source.as_deref(),
            cursor_started_at,
            cursor_plan_id,
            limit + 1,
        ],
        |row| read_action_log_plan_summary(connection, row),
    )?;
    let mut items = rows.collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit as usize;
    if has_more {
        items.pop();
    }
    let next_page_cursor = if has_more {
        items.last().and_then(encode_action_log_plan_list_cursor)
    } else {
        None
    };

    Ok(ActionLogPlanList {
        items,
        next_page_cursor,
        has_more,
        index_stale: index_state.stale,
        index_status: index_state.status,
        last_indexed_at: index_state.last_indexed_at,
    })
}

fn list_stale_choreography_action_log_plans_after_startup(
    connection: &Connection,
) -> BuddyResult<Vec<StaleActionLogPlanAfterStartup>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
          plan_id,
          source_ref_json,
          status,
          last_event_type,
          last_reason_code
        FROM action_log_plan_summaries
        WHERE status IN ('running', 'deferred')
        ORDER BY COALESCE(started_at, completed_at, plan_id) ASC, plan_id ASC
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        let source_ref_json = row.get::<_, String>(1)?;
        let source_ref = serde_json::from_str(&source_ref_json).map_err(json_to_sqlite_error)?;
        Ok(StaleActionLogPlanAfterStartup {
            plan_id: row.get(0)?,
            source_ref,
            status: row.get(2)?,
            last_event_type: row.get(3)?,
            last_reason_code: row.get(4)?,
            stale_steps: Vec::new(),
        })
    })?;

    let mut plans = rows.collect::<Result<Vec<_>, _>>()?;
    for plan in &mut plans {
        plan.stale_steps =
            list_stale_choreography_action_log_steps_after_startup(connection, &plan.plan_id)?;
    }

    Ok(plans)
}

fn list_recoverable_choreography_pending_admission_candidates_after_startup(
    connection: &Connection,
) -> BuddyResult<Vec<RecoverableChoreographyPendingAdmissionCandidate>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
          events.rowid,
          events.event_id,
          events.plan_id,
          events.source_ref_json,
          events.payload_json,
          events.reason_code,
          events.created_at
        FROM action_log_plan_summaries AS plans
        JOIN action_log_events AS events
          ON events.plan_id = plans.plan_id
        WHERE (
            (plans.status = 'deferred' AND plans.last_event_type = 'executor.deferred')
            OR (
              plans.status = 'interrupted'
              AND plans.last_event_type = 'plan.interrupted'
              AND plans.last_reason_code = 'runtime.restarted'
            )
          )
          AND events.event_type = 'executor.deferred'
          AND events.rowid = (
            SELECT MAX(latest_events.rowid)
            FROM action_log_events AS latest_events
            WHERE latest_events.plan_id = plans.plan_id
              AND latest_events.event_type = 'executor.deferred'
          )
        "#,
    )?;
    let rows = statement.query_map([], map_recoverable_pending_admission_row)?;
    let mut admissions = Vec::new();
    for row in rows {
        let row = row?;
        let deferred_event_rowid = row.deferred_event_rowid;
        let candidate = recoverable_pending_admission_candidate_from_row(row)?;
        admissions.push((deferred_event_rowid, candidate));
    }
    admissions.sort_by(|left, right| {
        right
            .1
            .priority
            .cmp(&left.1.priority)
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.1.plan_id.cmp(&right.1.plan_id))
    });

    Ok(admissions
        .into_iter()
        .map(|(_, admission)| admission)
        .collect())
}

fn map_recoverable_pending_admission_row(
    row: &Row<'_>,
) -> rusqlite::Result<RecoverableChoreographyPendingAdmissionRow> {
    let source_ref_json: String = row.get(3)?;
    let payload_json: String = row.get(4)?;
    Ok(RecoverableChoreographyPendingAdmissionRow {
        deferred_event_rowid: row.get(0)?,
        deferred_event_id: row.get(1)?,
        plan_id: row.get(2)?,
        source_ref: serde_json::from_str(&source_ref_json).map_err(json_to_sqlite_error)?,
        payload: serde_json::from_str(&payload_json).map_err(json_to_sqlite_error)?,
        reason_code: row.get(5)?,
        deferred_at: row.get(6)?,
    })
}

fn recoverable_pending_admission_candidate_from_row(
    row: RecoverableChoreographyPendingAdmissionRow,
) -> BuddyResult<RecoverableChoreographyPendingAdmissionCandidate> {
    let event_type = "executor.deferred";
    let decision = required_action_log_payload_string(&row.payload, "decision", event_type)?;
    if decision != "deferred" {
        return Err(invalid_action_log_payload_for_event(
            event_type,
            "field=decision must be deferred",
        ));
    }
    let payload_plan_id = required_action_log_payload_string(&row.payload, "planId", event_type)?;
    if payload_plan_id != row.plan_id {
        return Err(invalid_action_log_payload_for_event(
            event_type,
            "field=planId must match event planId",
        ));
    }
    let reason_code = required_action_log_payload_string(&row.payload, "reasonCode", event_type)?;
    if reason_code != row.reason_code {
        return Err(invalid_action_log_payload_for_event(
            event_type,
            "field=reasonCode must match event reasonCode",
        ));
    }
    let trigger_source = parse_choreography_trigger_source_action_log_value(
        required_action_log_payload_string(&row.payload, "triggerSource", event_type)?,
        "triggerSource",
        event_type,
    )?;
    let priority = parse_choreography_plan_priority_action_log_value(
        required_action_log_payload_string(&row.payload, "priority", event_type)?,
        "priority",
        event_type,
    )?;
    let active_plan = required_action_log_payload_object(&row.payload, "activePlan", event_type)?;
    let active_plan_id = required_action_log_payload_object_string(
        active_plan,
        "planId",
        "activePlan.planId",
        event_type,
    )?
    .to_owned();
    let active_step_id = optional_action_log_payload_object_string(
        active_plan,
        "stepId",
        "activePlan.stepId",
        event_type,
    )?
    .map(str::to_owned);
    let active_priority = parse_choreography_plan_priority_action_log_value(
        required_action_log_payload_object_string(
            active_plan,
            "priority",
            "activePlan.priority",
            event_type,
        )?,
        "activePlan.priority",
        event_type,
    )?;
    let active_step_interrupt_policy =
        SidecarInterruptPolicy::parse(required_action_log_payload_object_string(
            active_plan,
            "interruptPolicy",
            "activePlan.interruptPolicy",
            event_type,
        )?)?;

    Ok(RecoverableChoreographyPendingAdmissionCandidate {
        plan_id: row.plan_id,
        source_ref: row.source_ref,
        trigger_source,
        priority,
        reason_code: reason_code.to_owned(),
        active_plan_id,
        active_step_id,
        active_priority,
        active_step_interrupt_policy,
        deferred_event_id: row.deferred_event_id,
        deferred_at: row.deferred_at,
    })
}

fn validate_recoverable_pending_execution_body_matches_admission(
    admission: &RecoverableChoreographyPendingAdmission,
    body: &ReplayableChoreographyPendingExecutionBody,
) -> BuddyResult<()> {
    if body.plan_id != admission.plan_id {
        return Err(invalid_action_log_payload_for_event(
            "choreographyScheduler.pendingBodyStored",
            "field=planId must match recoverable admission planId",
        ));
    }
    if body.body_kind != admission.body_kind {
        return Err(invalid_action_log_payload_for_event(
            "choreographyScheduler.pendingBodyStored",
            "field=bodyKind must match recoverable admission bodyKind",
        ));
    }
    if body.schema_version != admission.body_schema_version {
        return Err(invalid_action_log_payload_for_event(
            "choreographyScheduler.pendingBodyStored",
            "field=schemaVersion must match recoverable admission bodySchemaVersion",
        ));
    }

    Ok(())
}

fn list_stale_choreography_action_log_steps_after_startup(
    connection: &Connection,
    plan_id: &str,
) -> BuddyResult<Vec<StaleActionLogStepAfterStartup>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
          rowid,
          step_id,
          event_type,
          status,
          reason_code
        FROM action_log_events
        WHERE plan_id = ?1
          AND step_id IS NOT NULL
        ORDER BY rowid ASC
        "#,
    )?;
    let rows = statement.query_map(params![plan_id], |row| {
        Ok(StaleActionLogStepAfterStartup {
            last_event_rowid: row.get(0)?,
            step_id: row.get(1)?,
            last_event_type: row.get(2)?,
            status: row.get(3)?,
            last_reason_code: row.get(4)?,
        })
    })?;
    let mut latest_by_step = HashMap::<String, StaleActionLogStepAfterStartup>::new();
    for row in rows {
        let step = row?;
        latest_by_step.insert(step.step_id.clone(), step);
    }
    let mut steps = latest_by_step
        .into_values()
        .filter(|step| !action_log_step_status_is_terminal(&step.status))
        .collect::<Vec<_>>();
    steps.sort_by_key(|step| step.last_event_rowid);

    Ok(steps)
}

fn action_log_step_status_is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "interrupted" | "skipped")
}

fn include_system_recovery_plans_in_list(request: &ActionLogPlanListRequest) -> bool {
    request
        .plan_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
        || request.source_ref_kind.as_deref() == Some(ACTION_LOG_SOURCE_REF_KIND_SYSTEM_RECOVERY)
}

fn query_action_log_system_events(
    connection: &Connection,
    request: ActionLogSystemEventQueryRequest,
    index_state: ActionLogIndexState,
) -> BuddyResult<ActionLogSystemEventQueryResult> {
    let limit = validate_action_log_system_event_query(&request)?;
    let mut statement = connection.prepare(
        r#"
        SELECT
          event_id,
          event_type,
          created_at,
          source_ref_kind,
          trigger_source,
          status,
          reason_code,
          plan_id,
          step_id
        FROM action_log_events
        WHERE plan_id IS NULL
          AND (?1 IS NULL OR event_type = ?1)
          AND (?2 IS NULL OR source_ref_kind = ?2)
          AND (?3 IS NULL OR reason_code = ?3)
          AND (?4 IS NULL OR status = ?4)
          AND (?5 IS NULL OR created_at >= ?5)
          AND (?6 IS NULL OR created_at <= ?6)
        ORDER BY rowid DESC
        LIMIT ?7
        "#,
    )?;
    let rows = statement.query_map(
        params![
            request.event_type.as_deref(),
            request.source_ref_kind.as_deref(),
            request.reason_code.as_deref(),
            request.status.as_deref(),
            request.created_at_from.as_deref(),
            request.created_at_to.as_deref(),
            limit + 1,
        ],
        read_action_log_system_event_summary,
    )?;
    let mut items = rows.collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit as usize;
    if has_more {
        items.pop();
    }

    Ok(ActionLogSystemEventQueryResult {
        items,
        limit,
        has_more,
        index_stale: index_state.stale,
        index_status: index_state.status,
        last_indexed_at: index_state.last_indexed_at,
    })
}

fn find_action_log_plan_summary(
    connection: &Connection,
    plan_id: &str,
) -> BuddyResult<ActionLogPlanSummary> {
    connection
        .query_row(
            r#"
            SELECT
              plan_id,
              source_ref_kind,
              source_ref_id,
              result_kind,
              source_ref_json,
              status,
              started_at,
              completed_at,
              last_event_type,
              last_reason_code,
              detail_status,
              detail_reason_code,
              resolved_action_id,
              resolved_animation_ref
            FROM action_log_plan_summaries
            WHERE plan_id = ?1
            "#,
            params![plan_id],
            |row| read_action_log_plan_summary(connection, row),
        )
        .map_err(Into::into)
}

fn list_action_log_plan_events(
    connection: &Connection,
    plan_id: &str,
) -> BuddyResult<Vec<ActionLogPlanEventRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
          event_id,
          schema_version,
          event_type,
          status,
          reason_code,
          plan_id,
          step_id,
          source_ref_kind,
          source_ref_json,
          payload_json,
          created_at
        FROM action_log_events
        WHERE plan_id = ?1
        ORDER BY rowid ASC
        "#,
    )?;
    let rows = statement.query_map(params![plan_id], read_action_log_plan_event_summary)?;

    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn list_related_system_recovery_plan_details(
    connection: &Connection,
    triggered_plan_id: &str,
) -> BuddyResult<Vec<ActionLogRelatedPlanDetail>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
          plan_id,
          source_ref_kind,
          source_ref_id,
          result_kind,
          source_ref_json,
          status,
          started_at,
          completed_at,
          last_event_type,
          last_reason_code,
          detail_status,
          detail_reason_code,
          resolved_action_id,
          resolved_animation_ref
        FROM action_log_plan_summaries AS plans
        WHERE plans.source_ref_kind = ?1
          AND plans.source_ref_id = ?2
        ORDER BY plans.started_at ASC, plans.plan_id ASC
        "#,
    )?;
    let rows = statement.query_map(
        params![
            ACTION_LOG_SOURCE_REF_KIND_SYSTEM_RECOVERY,
            triggered_plan_id
        ],
        |row| read_action_log_plan_summary(connection, row),
    )?;
    let summaries = rows.collect::<Result<Vec<_>, _>>()?;

    summaries
        .into_iter()
        .map(|plan| {
            let events = list_action_log_plan_events(connection, &plan.plan_id)?;
            Ok(ActionLogRelatedPlanDetail {
                plan,
                steps: build_action_log_step_details(events),
            })
        })
        .collect::<BuddyResult<Vec<_>>>()
}

fn read_action_log_plan_summary(
    connection: &Connection,
    row: &Row<'_>,
) -> rusqlite::Result<ActionLogPlanSummary> {
    let source_ref_json = row.get::<_, String>(4)?;
    let source_ref = serde_json::from_str(&source_ref_json).map_err(json_to_sqlite_error)?;

    Ok(ActionLogPlanSummary {
        plan_id: row.get(0)?,
        source_ref_kind: row.get(1)?,
        source_ref_id: row.get(2)?,
        result_kind: row.get(3)?,
        source_display: resolve_action_log_source_display(connection, &source_ref)?,
        source_ref,
        status: row.get(5)?,
        started_at: row.get(6)?,
        completed_at: row.get(7)?,
        last_event_type: row.get(8)?,
        last_reason_code: row.get(9)?,
        detail_status: row.get(10)?,
        detail_reason_code: row.get(11)?,
        resolved_action_id: row.get(12)?,
        resolved_animation_ref: row.get(13)?,
    })
}

fn resolve_action_log_source_display(
    connection: &Connection,
    source_ref: &serde_json::Value,
) -> rusqlite::Result<Option<ActionLogSourceDisplay>> {
    let Some(kind) = action_log_source_ref_string(source_ref, "kind") else {
        return Ok(None);
    };

    let display = match kind.as_str() {
        "conversationMessage" => {
            resolve_conversation_message_source_display(connection, source_ref, kind)?
        }
        "run" => resolve_run_source_display(connection, source_ref, kind)?,
        "approval" => resolve_named_source_display(source_ref, kind, &["approvalId", "runId"]),
        "presetBehavior" => resolve_named_source_display(
            source_ref,
            kind,
            &[
                "presetBehaviorId",
                "interactionId",
                "sessionId",
                "behaviorId",
            ],
        ),
        "systemRecovery" => resolve_named_source_display(
            source_ref,
            kind,
            &["triggeredByPlanId", "triggerReason", "triggeredByStepId"],
        ),
        "macroFallback" => resolve_named_source_display(
            source_ref,
            kind,
            &[
                "fallbackMacroId",
                "originalMacroId",
                "triggerReason",
                "triggeredByPlanId",
            ],
        ),
        "startupSystem" => ActionLogSourceDisplay {
            kind,
            title: "startupSystem".to_owned(),
            subtitle: None,
            missing: false,
        },
        "devFixture" => resolve_named_source_display(source_ref, kind, &["fixtureName"]),
        _ => resolve_unknown_source_display(source_ref, kind),
    };

    Ok(Some(display))
}

fn resolve_conversation_message_source_display(
    connection: &Connection,
    source_ref: &serde_json::Value,
    kind: String,
) -> rusqlite::Result<ActionLogSourceDisplay> {
    let conversation_id = action_log_source_ref_string(source_ref, "conversationId");
    let message_id = action_log_source_ref_string(source_ref, "messageId");
    let conversation_title = conversation_id
        .as_deref()
        .map(|id| read_action_log_conversation_title(connection, id))
        .transpose()?
        .flatten();
    let message = message_id
        .as_deref()
        .map(|id| read_action_log_message_source_record(connection, id))
        .transpose()?
        .flatten();
    let title = conversation_title.unwrap_or_else(|| {
        action_log_source_title_from_id("conversationMessage", conversation_id.as_deref())
    });
    let subtitle = match (&message, message_id.as_deref()) {
        (Some(message), Some(message_id)) => {
            let index = read_action_log_message_index(connection, message, message_id)?;
            Some(match index {
                Some(index) => format!(
                    "{} #{} · {}",
                    message.role,
                    index,
                    short_source_id(message_id)
                ),
                None => format!("{} · {}", message.role, short_source_id(message_id)),
            })
        }
        (None, Some(message_id)) => {
            Some(format!("missing message {}", short_source_id(message_id)))
        }
        _ => conversation_id
            .as_deref()
            .map(|id| format!("conversation {}", short_source_id(id))),
    };

    Ok(ActionLogSourceDisplay {
        kind,
        title,
        subtitle,
        missing: message_id.is_some() && message.is_none(),
    })
}

fn resolve_run_source_display(
    connection: &Connection,
    source_ref: &serde_json::Value,
    kind: String,
) -> rusqlite::Result<ActionLogSourceDisplay> {
    let run_id = action_log_source_ref_string(source_ref, "runId");
    let run = run_id
        .as_deref()
        .map(|id| read_action_log_run_source_record(connection, id))
        .transpose()?
        .flatten();
    let conversation_title = run
        .as_ref()
        .and_then(|run| run.conversation_id.as_deref())
        .map(|id| read_action_log_conversation_title(connection, id))
        .transpose()?
        .flatten();
    let title = conversation_title
        .unwrap_or_else(|| action_log_source_title_from_id("run", run_id.as_deref()));
    let mut missing = run_id.is_some() && run.is_none();
    let triggering_message_locator = run
        .as_ref()
        .and_then(|run| run.triggering_message_id.as_deref())
        .map(|message_id| {
            resolve_action_log_message_locator(connection, message_id).map(|locator| {
                if locator.missing {
                    missing = true;
                }
                locator.label
            })
        })
        .transpose()?;
    let subtitle = match (&run, run_id.as_deref()) {
        (Some(run), Some(run_id)) => {
            let mut parts = vec![run.runtime.clone(), run.status.clone()];
            if let Some(locator) = triggering_message_locator {
                parts.push(locator);
            }
            parts.push(short_source_id(run_id));
            Some(parts.join(" · "))
        }
        (None, Some(run_id)) => Some(format!("missing run {}", short_source_id(run_id))),
        _ => None,
    };

    Ok(ActionLogSourceDisplay {
        kind,
        title,
        subtitle,
        missing,
    })
}

fn resolve_named_source_display(
    source_ref: &serde_json::Value,
    kind: String,
    title_keys: &[&str],
) -> ActionLogSourceDisplay {
    let title = title_keys
        .iter()
        .find_map(|key| action_log_source_ref_string(source_ref, key))
        .unwrap_or_else(|| kind.clone());

    ActionLogSourceDisplay {
        kind,
        title,
        subtitle: None,
        missing: false,
    }
}

fn resolve_unknown_source_display(
    source_ref: &serde_json::Value,
    kind: String,
) -> ActionLogSourceDisplay {
    let subtitle = ["id", "sourceId", "conversationId", "messageId", "runId"]
        .iter()
        .find_map(|key| action_log_source_ref_string(source_ref, key))
        .map(|id| short_source_id(&id));

    ActionLogSourceDisplay {
        title: kind.clone(),
        kind,
        subtitle,
        missing: false,
    }
}

struct ActionLogMessageSourceRecord {
    conversation_id: Option<String>,
    branch_id: Option<String>,
    created_at: String,
    role: String,
}

struct ActionLogRunSourceRecord {
    conversation_id: Option<String>,
    runtime: String,
    status: String,
    triggering_message_id: Option<String>,
}

struct ActionLogSourceLocator {
    label: String,
    missing: bool,
}

fn read_action_log_conversation_title(
    connection: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT title FROM conversations WHERE id = ?1",
            params![conversation_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
}

fn read_action_log_message_source_record(
    connection: &Connection,
    message_id: &str,
) -> rusqlite::Result<Option<ActionLogMessageSourceRecord>> {
    connection
        .query_row(
            r#"
            SELECT conversation_id, branch_id, created_at, role
            FROM messages
            WHERE id = ?1
            "#,
            params![message_id],
            |row| {
                Ok(ActionLogMessageSourceRecord {
                    conversation_id: row.get(0)?,
                    branch_id: row.get(1)?,
                    created_at: row.get(2)?,
                    role: row.get(3)?,
                })
            },
        )
        .optional()
}

fn read_action_log_message_index(
    connection: &Connection,
    message: &ActionLogMessageSourceRecord,
    message_id: &str,
) -> rusqlite::Result<Option<i64>> {
    let (Some(conversation_id), Some(branch_id)) = (
        message.conversation_id.as_deref(),
        message.branch_id.as_deref(),
    ) else {
        return Ok(None);
    };

    connection
        .query_row(
            r#"
            SELECT COUNT(*)
            FROM messages
            WHERE conversation_id = ?1
              AND branch_id = ?2
              AND version_status = 'active'
              AND (
                created_at < ?3
                OR (created_at = ?3 AND id <= ?4)
              )
            "#,
            params![conversation_id, branch_id, message.created_at, message_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
}

fn read_action_log_run_source_record(
    connection: &Connection,
    run_id: &str,
) -> rusqlite::Result<Option<ActionLogRunSourceRecord>> {
    connection
        .query_row(
            r#"
            SELECT conversation_id, runtime, status, triggering_message_id
            FROM runs
            WHERE id = ?1
            "#,
            params![run_id],
            |row| {
                Ok(ActionLogRunSourceRecord {
                    conversation_id: row.get(0)?,
                    runtime: row.get(1)?,
                    status: row.get(2)?,
                    triggering_message_id: row.get(3)?,
                })
            },
        )
        .optional()
}

fn resolve_action_log_message_locator(
    connection: &Connection,
    message_id: &str,
) -> rusqlite::Result<ActionLogSourceLocator> {
    let message = read_action_log_message_source_record(connection, message_id)?;
    let Some(message) = message else {
        return Ok(ActionLogSourceLocator {
            label: format!("missing message {}", short_source_id(message_id)),
            missing: true,
        });
    };
    let index = read_action_log_message_index(connection, &message, message_id)?;
    let label = match index {
        Some(index) => format!(
            "{} #{} · {}",
            message.role,
            index,
            short_source_id(message_id)
        ),
        None => format!("{} · {}", message.role, short_source_id(message_id)),
    };

    Ok(ActionLogSourceLocator {
        label,
        missing: false,
    })
}

fn action_log_source_ref_string(source_ref: &serde_json::Value, key: &str) -> Option<String> {
    source_ref
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

pub(crate) fn action_log_source_ref_kind(source_ref: &serde_json::Value) -> String {
    action_log_source_ref_string(source_ref, "kind").unwrap_or_else(|| "unknown".to_owned())
}

pub(crate) fn action_log_source_ref_primary_id(source_ref: &serde_json::Value) -> Option<String> {
    validate_action_log_source_ref(source_ref)
        .ok()
        .and_then(|projection| projection.source_ref_id.map(str::to_owned))
}

fn action_log_source_title_from_id(kind: &str, id: Option<&str>) -> String {
    id.map(|id| format!("{kind} {}", short_source_id(id)))
        .unwrap_or_else(|| kind.to_owned())
}

fn short_source_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn read_action_log_system_event_summary(
    row: &Row<'_>,
) -> rusqlite::Result<ActionLogSystemEventSummary> {
    Ok(ActionLogSystemEventSummary {
        event_id: row.get(0)?,
        event_type: row.get(1)?,
        timestamp: row.get(2)?,
        source_ref: ActionLogSystemEventSourceRefSummary { kind: row.get(3)? },
        trigger_source: row.get(4)?,
        status: row.get(5)?,
        reason_code: row.get(6)?,
        plan_id: row.get(7)?,
        step_id: row.get(8)?,
        index_status: "indexed",
    })
}

fn read_action_log_plan_event_summary(row: &Row<'_>) -> rusqlite::Result<ActionLogPlanEventRecord> {
    let payload_json = row.get::<_, String>(9)?;

    Ok(ActionLogPlanEventRecord {
        event_type: row.get(2)?,
        status: row.get(3)?,
        reason_code: row.get(4)?,
        step_id: row.get(6)?,
        payload: serde_json::from_str(&payload_json).map_err(json_to_sqlite_error)?,
        created_at: row.get(10)?,
    })
}

fn build_action_log_step_details(
    events: Vec<ActionLogPlanEventRecord>,
) -> Vec<ActionLogStepDetail> {
    let mut steps = Vec::new();
    let mut step_indexes = HashMap::<String, usize>::new();

    for event in events {
        let Some(step_id) = event.step_id.clone() else {
            continue;
        };
        let index = if let Some(index) = step_indexes.get(&step_id).copied() {
            index
        } else {
            steps.push(ActionLogStepDetail::new(step_id.clone()));
            let index = steps.len() - 1;
            step_indexes.insert(step_id, index);
            index
        };

        apply_action_log_step_event(&mut steps[index], &event);
    }

    steps
}

impl ActionLogStepDetail {
    fn new(step_id: String) -> Self {
        Self {
            step_id,
            status: "unknown".to_owned(),
            reason_code: "unknown".to_owned(),
            step_kind: None,
            target_label: None,
            resolved_action_id: None,
            resolved_animation_ref: None,
            resolved_at: None,
            completed_at: None,
            failed_at: None,
            duration_ms: None,
            elapsed_ms: None,
            event_count: 0,
        }
    }
}

fn apply_action_log_step_event(step: &mut ActionLogStepDetail, event: &ActionLogPlanEventRecord) {
    step.status.clone_from(&event.status);
    step.reason_code.clone_from(&event.reason_code);
    step.event_count += 1;

    if let Some(action_id) = action_log_payload_string(&event.payload, "actionId")
        .or_else(|| action_log_payload_string(&event.payload, "afterResolvedActionId"))
    {
        step.resolved_action_id = Some(action_id);
    }
    if let Some(step_kind) = action_log_payload_string(&event.payload, "stepKind") {
        step.step_kind = Some(step_kind);
    }
    if let Some(target_label) = action_log_step_target_label(&event.payload) {
        step.target_label = Some(target_label);
    }
    if let Some(animation_ref) = action_log_payload_string(&event.payload, "animationRef")
        .or_else(|| action_log_payload_string(&event.payload, "afterAnimationRef"))
    {
        step.resolved_animation_ref = Some(animation_ref);
    }
    if let Some(duration_ms) = action_log_payload_u64(&event.payload, "durationMs") {
        step.duration_ms = Some(duration_ms);
    }
    if let Some(elapsed_ms) = action_log_payload_u64(&event.payload, "elapsedMs") {
        step.elapsed_ms = Some(elapsed_ms);
    }

    match event.event_type.as_str() {
        "step.resolved" => step.resolved_at = Some(event.created_at.clone()),
        "step.completed" => step.completed_at = Some(event.created_at.clone()),
        "step.skipped" => step.completed_at = Some(event.created_at.clone()),
        "step.failed" => step.failed_at = Some(event.created_at.clone()),
        _ => {}
    }
}

fn action_log_step_target_label(payload: &serde_json::Value) -> Option<String> {
    let target = payload.get("target")?;
    if let Some(target) = target.as_str() {
        return Some(target.to_owned());
    }

    let kind = target.get("kind").and_then(serde_json::Value::as_str)?;
    match kind {
        "edge" => target
            .get("edge")
            .and_then(serde_json::Value::as_str)
            .map(|edge| format!("edge:{edge}")),
        "position" => {
            let x = target.get("x").and_then(serde_json::Value::as_i64)?;
            let y = target.get("y").and_then(serde_json::Value::as_i64)?;
            Some(format!("position:{x},{y}"))
        }
        "x" => {
            let x = target.get("x").and_then(serde_json::Value::as_i64)?;
            Some(format!("x:{x}"))
        }
        "windowAnchor" => {
            let selector = target
                .get("selector")
                .and_then(|selector| selector.get("kind"))
                .and_then(serde_json::Value::as_str)?;
            let edge = target.get("edge").and_then(serde_json::Value::as_str)?;
            let reveal = target.get("reveal").and_then(serde_json::Value::as_str)?;
            Some(format!("windowAnchor:{selector}:{edge}:{reveal}"))
        }
        _ => Some(kind.to_owned()),
    }
}

fn action_log_payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get("resolution")
        .and_then(|resolution| resolution.get(key))
        .or_else(|| payload.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn action_log_payload_u64(payload: &serde_json::Value, key: &str) -> Option<u64> {
    payload
        .get("resolution")
        .and_then(|resolution| resolution.get(key))
        .or_else(|| payload.get(key))
        .and_then(serde_json::Value::as_u64)
}

fn normalize_action_log_plan_list_limit(limit: Option<i64>) -> i64 {
    limit
        .unwrap_or(ACTION_LOG_PLAN_LIST_DEFAULT_LIMIT)
        .clamp(1, ACTION_LOG_PLAN_LIST_MAX_LIMIT)
}

fn validate_action_log_system_event_query(
    request: &ActionLogSystemEventQueryRequest,
) -> BuddyResult<i64> {
    if request.plan_id.is_some() {
        return Err(invalid_action_log_system_event_query_parameter(
            "planId",
            "must be omitted",
        ));
    }
    if request.step_id.is_some() {
        return Err(invalid_action_log_system_event_query_parameter(
            "stepId",
            "must be omitted",
        ));
    }
    if request
        .event_type
        .as_deref()
        .is_some_and(is_forbidden_action_log_system_event_type)
    {
        return Err(invalid_action_log_system_event_query_parameter(
            "eventType",
            "ordinary choreography event namespaces are not supported",
        ));
    }

    normalize_action_log_system_event_limit(request.limit)
}

fn normalize_action_log_system_event_limit(limit: Option<i64>) -> BuddyResult<i64> {
    match limit {
        None => Ok(ACTION_LOG_SYSTEM_EVENT_DEFAULT_LIMIT),
        Some(value) if (1..=ACTION_LOG_SYSTEM_EVENT_MAX_LIMIT).contains(&value) => Ok(value),
        Some(_) => Err(BuddyError::Validation(format!(
            "invalid action log system event query parameter: field=limit min=1 max={} default={}",
            ACTION_LOG_SYSTEM_EVENT_MAX_LIMIT, ACTION_LOG_SYSTEM_EVENT_DEFAULT_LIMIT
        ))),
    }
}

fn is_forbidden_action_log_system_event_type(event_type: &str) -> bool {
    ACTION_LOG_SYSTEM_EVENT_FORBIDDEN_PREFIXES
        .iter()
        .any(|prefix| event_type.starts_with(prefix))
}

fn invalid_action_log_system_event_query_parameter(field: &str, reason: &str) -> BuddyError {
    BuddyError::Validation(format!(
        "invalid action log system event query parameter: field={field} reason={reason}"
    ))
}

fn parse_action_log_plan_list_cursor(cursor: &str) -> BuddyResult<ActionLogPlanListCursor> {
    let Some(payload) = cursor.strip_prefix(ACTION_LOG_PLAN_LIST_CURSOR_PREFIX) else {
        return Err(BuddyError::Validation(
            "invalid action log plan page cursor".to_owned(),
        ));
    };
    let Some((started_at, plan_id)) = payload.split_once('|') else {
        return Err(BuddyError::Validation(
            "invalid action log plan page cursor".to_owned(),
        ));
    };
    if started_at.is_empty() || plan_id.is_empty() {
        return Err(BuddyError::Validation(
            "invalid action log plan page cursor".to_owned(),
        ));
    }

    Ok(ActionLogPlanListCursor {
        started_at: started_at.to_owned(),
        plan_id: plan_id.to_owned(),
    })
}

fn encode_action_log_plan_list_cursor(plan: &ActionLogPlanSummary) -> Option<String> {
    plan.started_at.as_ref().map(|started_at| {
        format!(
            "{ACTION_LOG_PLAN_LIST_CURSOR_PREFIX}{started_at}|{}",
            plan.plan_id
        )
    })
}

fn json_to_sqlite_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
fn append_action_log_jsonl_line<T>(
    path: PathBuf,
    event: &T,
) -> BuddyResult<ActionLogJsonlAppendCursor>
where
    T: Serialize,
{
    let append_position = next_action_log_jsonl_append_position(&path)?;
    append_action_log_jsonl_line_at_position(path, event, append_position)
}

fn append_action_log_jsonl_line_at_position<T>(
    path: PathBuf,
    event: &T,
    append_position: ActionLogJsonlAppendPosition,
) -> BuddyResult<ActionLogJsonlAppendCursor>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let path_existed = path.exists();

    let mut line = serde_json::to_vec(event)?;
    line.push(b'\n');
    let mut serialized =
        Vec::with_capacity(line.len() + usize::from(append_position.needs_leading_newline));
    if append_position.needs_leading_newline {
        serialized.push(b'\n');
    }
    serialized.extend_from_slice(&line);

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(&serialized)?;
    file.sync_data()?;
    drop(file);
    #[cfg(unix)]
    if !path_existed {
        File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    }

    Ok(append_position.cursor)
}

fn ensure_action_log_jsonl_event_id_is_new(path: &Path, event_id: &str) -> BuddyResult<()> {
    if !action_log_jsonl_contains_event_id(path, event_id)? {
        return Ok(());
    }

    Err(BuddyError::Validation(format!(
        "duplicate action log eventId={event_id}"
    )))
}

fn action_log_jsonl_contains_event_id(path: &Path, event_id: &str) -> BuddyResult<bool> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut byte_offset = 0;
    let mut line_number = 1;

    while byte_offset < bytes.len() {
        let remaining = &bytes[byte_offset..];
        let relative_line_end = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(remaining.len());
        let line_end = byte_offset + relative_line_end;
        let line = &bytes[byte_offset..line_end];
        if line.is_empty() {
            return Err(BuddyError::Validation(format!(
                "invalid action log JSONL: line {line_number} is empty"
            )));
        }
        let value = serde_json::from_slice::<serde_json::Value>(line).map_err(|error| {
            BuddyError::Validation(format!(
                "invalid action log JSONL: line {line_number} is not valid JSON: {error}"
            ))
        })?;
        if value.get("eventId").and_then(serde_json::Value::as_str) == Some(event_id) {
            return Ok(true);
        }

        byte_offset = if line_end < bytes.len() && bytes[line_end] == b'\n' {
            line_end + 1
        } else {
            line_end
        };
        line_number += 1;
    }

    Ok(false)
}

fn ensure_action_log_index_event_id_is_new(
    connection: &Connection,
    event_id: &str,
) -> BuddyResult<()> {
    let exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM action_log_events WHERE event_id = ?1)",
        params![event_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        return Ok(());
    }

    Err(BuddyError::Validation(format!(
        "duplicate action log eventId={event_id}"
    )))
}

fn next_action_log_jsonl_append_position(path: &Path) -> BuddyResult<ActionLogJsonlAppendPosition> {
    let existing_bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let needs_leading_newline = !existing_bytes.is_empty() && !existing_bytes.ends_with(b"\n");
    let event_byte_offset = existing_bytes
        .len()
        .checked_add(usize::from(needs_leading_newline))
        .ok_or_else(|| BuddyError::Validation("action log JSONL file is too large".to_owned()))?;
    let byte_offset = i64::try_from(event_byte_offset)
        .map_err(|_| BuddyError::Validation("action log JSONL file is too large".to_owned()))?;
    let existing_line_count = existing_bytes.iter().filter(|byte| **byte == b'\n').count();
    let event_line_index = if existing_bytes.is_empty() {
        1
    } else if needs_leading_newline {
        existing_line_count + 2
    } else {
        existing_line_count + 1
    };
    let line_number = i64::try_from(event_line_index)
        .map_err(|_| BuddyError::Validation("action log JSONL file is too large".to_owned()))?;

    Ok(ActionLogJsonlAppendPosition {
        cursor: ActionLogJsonlAppendCursor {
            source_file: ACTION_LOG_JSONL_RELATIVE_PATH,
            byte_offset,
            line_number,
        },
        needs_leading_newline,
    })
}

fn inspect_action_log_index_tail(
    connection: &Connection,
    jsonl_path: &Path,
) -> BuddyResult<ActionLogIndexTailInspection> {
    let watermark = read_action_log_index_watermark(connection)?;
    let projected_event_count = action_log_projection_event_count(connection)?;
    let metadata = match fs::metadata(jsonl_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let append_position = (watermark.is_none() && projected_event_count == 0).then_some({
                ActionLogJsonlAppendPosition {
                    cursor: ActionLogJsonlAppendCursor {
                        source_file: ACTION_LOG_JSONL_RELATIVE_PATH,
                        byte_offset: 0,
                        line_number: 1,
                    },
                    needs_leading_newline: false,
                }
            });
            return Ok(ActionLogIndexTailInspection {
                append_position,
                last_indexed_at: watermark.map(|watermark| watermark.updated_at),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let file_len = metadata.len();
    if file_len == 0 {
        let append_position = (watermark.is_none() && projected_event_count == 0).then_some({
            ActionLogJsonlAppendPosition {
                cursor: ActionLogJsonlAppendCursor {
                    source_file: ACTION_LOG_JSONL_RELATIVE_PATH,
                    byte_offset: 0,
                    line_number: 1,
                },
                needs_leading_newline: false,
            }
        });
        return Ok(ActionLogIndexTailInspection {
            append_position,
            last_indexed_at: watermark.map(|watermark| watermark.updated_at),
        });
    }

    let Some(watermark) = watermark else {
        return Ok(ActionLogIndexTailInspection {
            append_position: None,
            last_indexed_at: None,
        });
    };
    if projected_event_count != watermark.line_number {
        return Ok(ActionLogIndexTailInspection {
            append_position: None,
            last_indexed_at: Some(watermark.updated_at),
        });
    }
    let byte_offset = u64::try_from(watermark.byte_offset).map_err(|_| {
        BuddyError::Validation("action log watermark byte offset is invalid".to_owned())
    })?;
    if byte_offset >= file_len {
        return Ok(ActionLogIndexTailInspection {
            append_position: None,
            last_indexed_at: Some(watermark.updated_at),
        });
    }

    let mut file = File::open(jsonl_path)?;
    file.seek(SeekFrom::Start(byte_offset))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let needs_leading_newline = !tail.ends_with(b"\n");
    let line = tail.strip_suffix(b"\n").unwrap_or(tail.as_slice());
    if line.is_empty() || line.contains(&b'\n') {
        return Ok(ActionLogIndexTailInspection {
            append_position: None,
            last_indexed_at: Some(watermark.updated_at),
        });
    }
    let value = serde_json::from_slice::<serde_json::Value>(line).map_err(|error| {
        BuddyError::Validation(format!("invalid action log JSONL tail: {error}"))
    })?;
    if value.get("eventId").and_then(serde_json::Value::as_str) != Some(watermark.event_id.as_str())
    {
        return Ok(ActionLogIndexTailInspection {
            append_position: None,
            last_indexed_at: Some(watermark.updated_at),
        });
    }

    let event_byte_offset = file_len
        .checked_add(u64::from(needs_leading_newline))
        .ok_or_else(|| BuddyError::Validation("action log JSONL file is too large".to_owned()))?;
    let byte_offset = i64::try_from(event_byte_offset)
        .map_err(|_| BuddyError::Validation("action log JSONL file is too large".to_owned()))?;
    let line_number = watermark.line_number.checked_add(1).ok_or_else(|| {
        BuddyError::Validation("action log JSONL line number is too large".to_owned())
    })?;

    Ok(ActionLogIndexTailInspection {
        append_position: Some(ActionLogJsonlAppendPosition {
            cursor: ActionLogJsonlAppendCursor {
                source_file: ACTION_LOG_JSONL_RELATIVE_PATH,
                byte_offset,
                line_number,
            },
            needs_leading_newline,
        }),
        last_indexed_at: Some(watermark.updated_at),
    })
}

fn action_log_projection_event_count(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row("SELECT COUNT(*) FROM action_log_events", [], |row| {
        row.get(0)
    })
}

fn read_action_log_jsonl_bytes(jsonl_path: &Path) -> BuddyResult<Vec<u8>> {
    match fs::read(jsonl_path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn read_action_log_index_state(connection: &Connection, jsonl_path: &Path) -> ActionLogIndexState {
    let inspection = match inspect_action_log_index_tail(connection, jsonl_path) {
        Ok(inspection) => inspection,
        Err(_) => return ActionLogIndexState::failed(None),
    };
    if inspection.append_position.is_some() {
        return ActionLogIndexState::fresh(inspection.last_indexed_at);
    }

    let jsonl_bytes = match fs::read(jsonl_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ActionLogIndexState::stale(inspection.last_indexed_at);
        }
        Err(_) => return ActionLogIndexState::failed(inspection.last_indexed_at),
    };
    if validate_action_log_jsonl_replay_input(&jsonl_bytes).is_err() {
        return ActionLogIndexState::failed(inspection.last_indexed_at);
    }

    ActionLogIndexState::stale(inspection.last_indexed_at)
}

fn read_action_log_index_watermark(
    connection: &Connection,
) -> rusqlite::Result<Option<ActionLogIndexWatermark>> {
    connection
        .query_row(
            r#"
            SELECT byte_offset, line_number, event_id, updated_at
            FROM action_log_index_watermarks
            WHERE source_file = ?1
            "#,
            params![ACTION_LOG_JSONL_RELATIVE_PATH],
            |row| {
                Ok(ActionLogIndexWatermark {
                    byte_offset: row.get(0)?,
                    line_number: row.get(1)?,
                    event_id: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .optional()
}

fn plan_action_log_index_sync(
    connection: &Connection,
    jsonl_bytes: &[u8],
) -> BuddyResult<ActionLogIndexSyncPlan> {
    let watermark = read_action_log_index_watermark(connection)?;
    let projected_event_count = action_log_projection_event_count(connection)?;
    let Some(watermark) = watermark else {
        return if jsonl_bytes.is_empty() && projected_event_count == 0 {
            Ok(ActionLogIndexSyncPlan::Fresh)
        } else {
            Ok(ActionLogIndexSyncPlan::Rebuild)
        };
    };

    if jsonl_bytes.is_empty() {
        return Ok(ActionLogIndexSyncPlan::Rebuild);
    }

    if action_log_jsonl_event_id_at_offset(jsonl_bytes, watermark.byte_offset).as_deref()
        != Some(watermark.event_id.as_str())
    {
        return Ok(ActionLogIndexSyncPlan::Rebuild);
    }
    if projected_event_count != watermark.line_number {
        return Ok(ActionLogIndexSyncPlan::Rebuild);
    }

    let line_count = action_log_jsonl_line_count(jsonl_bytes)?;
    if line_count == watermark.line_number {
        return Ok(ActionLogIndexSyncPlan::Fresh);
    }
    if line_count < watermark.line_number {
        return Ok(ActionLogIndexSyncPlan::Rebuild);
    }

    Ok(ActionLogIndexSyncPlan::Incremental {
        byte_offset: action_log_jsonl_next_line_offset(jsonl_bytes, watermark.byte_offset)?,
        line_number: watermark.line_number + 1,
    })
}

fn action_log_jsonl_line_count(bytes: &[u8]) -> BuddyResult<i64> {
    let newline_count = bytes.iter().filter(|byte| **byte == b'\n').count();
    let trailing_line = usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
    i64::try_from(newline_count + trailing_line)
        .map_err(|_| BuddyError::Validation("action log JSONL file is too large".to_owned()))
}

fn action_log_jsonl_next_line_offset(bytes: &[u8], byte_offset: i64) -> BuddyResult<i64> {
    let offset = usize::try_from(byte_offset)
        .map_err(|_| BuddyError::Validation("invalid action log JSONL byte offset".to_owned()))?;
    let remaining = bytes
        .get(offset..)
        .ok_or_else(|| BuddyError::Validation("invalid action log JSONL byte offset".to_owned()))?;
    let line_end = remaining
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|line_end| offset + line_end + 1)
        .ok_or_else(|| {
            BuddyError::Validation("invalid action log JSONL watermark line".to_owned())
        })?;

    i64::try_from(line_end)
        .map_err(|_| BuddyError::Validation("action log JSONL file is too large".to_owned()))
}

fn action_log_jsonl_event_id_at_offset(bytes: &[u8], byte_offset: i64) -> Option<String> {
    let offset = usize::try_from(byte_offset).ok()?;
    let remaining = bytes.get(offset..)?;
    let line_end = remaining
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(remaining.len());
    let value = serde_json::from_slice::<serde_json::Value>(&remaining[..line_end]).ok()?;

    value
        .get("eventId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn validate_action_log_jsonl_unique_event_ids(bytes: &[u8]) -> BuddyResult<()> {
    let mut byte_offset = 0;
    let mut line_number = 1;
    let mut event_ids = HashSet::new();

    while byte_offset < bytes.len() {
        let remaining = &bytes[byte_offset..];
        let relative_line_end = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(remaining.len());
        let line_end = byte_offset + relative_line_end;
        let line = &bytes[byte_offset..line_end];
        if line.is_empty() {
            return Err(BuddyError::Validation(format!(
                "invalid action log JSONL: line {line_number} is empty"
            )));
        }

        let event_id = action_log_jsonl_event_id_in_line(line, line_number)?;
        if !event_ids.insert(event_id.to_owned()) {
            return Err(BuddyError::Validation(format!(
                "duplicate action log eventId={event_id}"
            )));
        }

        byte_offset = if line_end < bytes.len() && bytes[line_end] == b'\n' {
            line_end + 1
        } else {
            line_end
        };
        line_number += 1;
    }

    Ok(())
}

fn validate_action_log_jsonl_replay_input(bytes: &[u8]) -> BuddyResult<()> {
    validate_action_log_jsonl_unique_event_ids(bytes)?;

    for record in read_action_log_jsonl_replay_records(bytes, 0, 1)? {
        validate_action_log_jsonl_replay_event_schema(&record.event)?;
    }

    Ok(())
}

fn validate_action_log_jsonl_replay_event_schema(
    event: &ActionLogJsonlReplayEvent,
) -> BuddyResult<()> {
    match event {
        ActionLogJsonlReplayEvent::Plan(event) => {
            validate_action_log_event_schema(event)?;
            validate_action_log_source_ref(&event.source_ref)?;
        }
        ActionLogJsonlReplayEvent::System(event) => {
            validate_action_log_system_event_schema(event)?;
            validate_action_log_system_source_ref(&event.source_ref)?;
        }
    }

    Ok(())
}

fn action_log_jsonl_event_id_in_line(line: &[u8], line_number: i64) -> BuddyResult<String> {
    let value = serde_json::from_slice::<serde_json::Value>(line).map_err(|error| {
        BuddyError::Validation(format!(
            "invalid action log JSONL: line {line_number} is not valid JSON: {error}"
        ))
    })?;

    value
        .get("eventId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            BuddyError::Validation(format!(
                "invalid action log JSONL: line {line_number} missing eventId"
            ))
        })
}

fn read_action_log_jsonl_replay_records(
    bytes: &[u8],
    start_byte_offset: i64,
    start_line_number: i64,
) -> BuddyResult<Vec<ActionLogJsonlReplayRecord>> {
    let mut byte_offset = usize::try_from(start_byte_offset)
        .map_err(|_| BuddyError::Validation("invalid action log JSONL byte offset".to_owned()))?;
    if byte_offset > bytes.len() {
        return Err(BuddyError::Validation(
            "invalid action log JSONL byte offset".to_owned(),
        ));
    }

    let mut line_number = start_line_number;
    let mut records = Vec::new();
    let mut event_ids = HashSet::new();
    while byte_offset < bytes.len() {
        let remaining = &bytes[byte_offset..];
        let relative_line_end = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(remaining.len());
        let line_end = byte_offset + relative_line_end;
        let line = &bytes[byte_offset..line_end];
        if line.is_empty() {
            return Err(BuddyError::Validation(format!(
                "invalid action log JSONL: line {line_number} is empty"
            )));
        }

        let event = parse_action_log_jsonl_replay_event(line, line_number)?;
        let event_id = action_log_jsonl_replay_event_id(&event);
        if !event_ids.insert(event_id.to_owned()) {
            return Err(BuddyError::Validation(format!(
                "duplicate action log eventId={event_id}"
            )));
        }

        records.push(ActionLogJsonlReplayRecord {
            cursor: ActionLogJsonlAppendCursor {
                source_file: ACTION_LOG_JSONL_RELATIVE_PATH,
                byte_offset: i64::try_from(byte_offset).map_err(|_| {
                    BuddyError::Validation("action log JSONL file is too large".to_owned())
                })?,
                line_number,
            },
            event,
        });

        byte_offset = if line_end < bytes.len() && bytes[line_end] == b'\n' {
            line_end + 1
        } else {
            line_end
        };
        line_number += 1;
    }

    Ok(records)
}

fn action_log_jsonl_replay_event_id(event: &ActionLogJsonlReplayEvent) -> &str {
    match event {
        ActionLogJsonlReplayEvent::Plan(event) => &event.event_id,
        ActionLogJsonlReplayEvent::System(event) => &event.event_id,
    }
}

fn parse_action_log_jsonl_replay_event(
    line: &[u8],
    line_number: i64,
) -> BuddyResult<ActionLogJsonlReplayEvent> {
    let value = serde_json::from_slice::<serde_json::Value>(line).map_err(|error| {
        BuddyError::Validation(format!(
            "invalid action log JSONL: line {line_number} is not valid JSON: {error}"
        ))
    })?;
    if value.get("planId").is_some_and(|value| !value.is_null()) {
        let event = serde_json::from_value::<ActionLogEvent>(value)?;
        Ok(ActionLogJsonlReplayEvent::Plan(event))
    } else {
        let event = serde_json::from_value::<ActionLogSystemEvent>(value)?;
        Ok(ActionLogJsonlReplayEvent::System(event))
    }
}

fn index_action_log_jsonl_replay_record(
    connection: &Connection,
    record: &ActionLogJsonlReplayRecord,
) -> BuddyResult<()> {
    match &record.event {
        ActionLogJsonlReplayEvent::Plan(event) => {
            validate_action_log_event_schema(event)?;
            let source_ref_projection = validate_action_log_source_ref(&event.source_ref)?;
            delete_action_log_event_projection(connection, &event.event_id)?;
            insert_action_log_event(connection, event, &source_ref_projection)?;
            upsert_action_log_plan_summary(connection, event, &source_ref_projection)?;
            project_choreography_pending_execution_body_cache_from_plan_event(connection, event)?;
            upsert_action_log_index_watermark(connection, event, &record.cursor)
        }
        ActionLogJsonlReplayEvent::System(event) => {
            validate_action_log_system_event_schema(event)?;
            let source_ref_projection = validate_action_log_system_source_ref(&event.source_ref)?;
            delete_action_log_event_projection(connection, &event.event_id)?;
            insert_action_log_system_event(connection, event, &source_ref_projection)?;
            project_choreography_pending_execution_body_cache_from_system_event(connection, event)?;
            upsert_action_log_system_index_watermark(connection, event, &record.cursor)
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn rebuild_choreography_pending_execution_body_cache_from_record(
    connection: &Connection,
    record: &ActionLogJsonlReplayRecord,
) -> BuddyResult<()> {
    match &record.event {
        ActionLogJsonlReplayEvent::Plan(event) => {
            project_choreography_pending_execution_body_cache_from_plan_event(connection, event)?;
        }
        ActionLogJsonlReplayEvent::System(event) => {
            project_choreography_pending_execution_body_cache_from_system_event(connection, event)?;
        }
    }

    Ok(())
}

fn project_choreography_pending_execution_body_cache_from_plan_event(
    connection: &Connection,
    event: &ActionLogEvent,
) -> BuddyResult<()> {
    if action_log_plan_event_is_terminal(event)
        || action_log_plan_event_consumes_replayable_pending_execution_body(event)
    {
        choreography_pending_bodies::delete_choreography_pending_execution_body(
            connection,
            event.plan_id.as_str(),
        )?;
    }

    Ok(())
}

fn project_choreography_pending_execution_body_cache_from_system_event(
    connection: &Connection,
    event: &ActionLogSystemEvent,
) -> BuddyResult<()> {
    match event.event_type.as_str() {
        "choreographyScheduler.pendingBodyStored" => {
            let request = parse_pending_body_stored_fact_payload(&event.payload)?;
            choreography_pending_bodies::upsert_choreography_pending_execution_body(
                connection, request,
            )?;
        }
        "choreographyScheduler.pendingBodyDeleted" => {
            let plan_id = required_action_log_payload_string(
                &event.payload,
                "planId",
                event.event_type.as_str(),
            )?;
            choreography_pending_bodies::delete_choreography_pending_execution_body(
                connection, plan_id,
            )?;
        }
        _ => {}
    }

    Ok(())
}

fn replayable_pending_execution_body_from_records(
    plan_id: &str,
    records: &[ActionLogJsonlReplayRecord],
) -> BuddyResult<Option<ReplayableChoreographyPendingExecutionBody>> {
    let mut replayable_body = None;
    for record in records {
        match &record.event {
            ActionLogJsonlReplayEvent::Plan(event) => {
                if event.plan_id == plan_id
                    && action_log_plan_event_consumes_replayable_pending_execution_body(event)
                {
                    replayable_body = None;
                }
            }
            ActionLogJsonlReplayEvent::System(event) => match event.event_type.as_str() {
                "choreographyScheduler.pendingBodyStored" => {
                    let request = parse_pending_body_stored_fact_payload(&event.payload)?;
                    if request.plan_id == plan_id {
                        replayable_body = Some(ReplayableChoreographyPendingExecutionBody {
                            plan_id: request.plan_id,
                            body_kind: request.body_kind,
                            schema_version: request.schema_version,
                            body: request.body,
                            stored_event_id: event.event_id.clone(),
                            stored_at: event.created_at.clone(),
                        });
                    }
                }
                "choreographyScheduler.pendingBodyDeleted" => {
                    let deleted_plan_id = required_action_log_payload_string(
                        &event.payload,
                        "planId",
                        event.event_type.as_str(),
                    )?;
                    if deleted_plan_id == plan_id {
                        replayable_body = None;
                    }
                }
                _ => {}
            },
        }
    }

    Ok(replayable_body)
}

#[cfg_attr(not(test), allow(dead_code))]
fn action_log_plan_event_is_terminal(event: &ActionLogEvent) -> bool {
    matches!(
        event.event_type.as_str(),
        "plan.completed" | "plan.failed" | "plan.interrupted"
    )
}

fn action_log_plan_event_consumes_replayable_pending_execution_body(
    event: &ActionLogEvent,
) -> bool {
    matches!(
        event.event_type.as_str(),
        "executor.accepted"
            | "executor.preempted"
            | "executor.rejected"
            | "executor.skipped"
            | "plan.started"
            | "plan.completed"
            | "plan.failed"
    ) || (event.event_type == "plan.interrupted" && event.reason_code != "runtime.restarted")
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_pending_body_stored_fact_payload(
    payload: &serde_json::Value,
) -> BuddyResult<UpsertChoreographyPendingExecutionBodyRequest> {
    let event_type = "choreographyScheduler.pendingBodyStored";
    let plan_id = required_action_log_payload_string(payload, "planId", event_type)?;
    let body_kind = required_action_log_payload_string(payload, "bodyKind", event_type)?;
    let body_kind = ChoreographyPendingExecutionBodyKind::parse(body_kind)
        .map_err(|error| BuddyError::Validation(error.to_string()))?;
    let schema_version =
        required_action_log_payload_schema_version(payload, "schemaVersion", event_type)?;
    let body = payload.get("body").cloned().ok_or_else(|| {
        BuddyError::Validation(format!(
            "invalid action log payload for {event_type}: field=body is required"
        ))
    })?;

    Ok(UpsertChoreographyPendingExecutionBodyRequest {
        plan_id: plan_id.to_owned(),
        body_kind,
        schema_version,
        body,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn required_action_log_payload_string<'a>(
    payload: &'a serde_json::Value,
    field: &str,
    event_type: &str,
) -> BuddyResult<&'a str> {
    payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            BuddyError::Validation(format!(
                "invalid action log payload for {event_type}: field={field} is required"
            ))
        })
}

fn required_action_log_payload_object<'a>(
    payload: &'a serde_json::Value,
    field: &str,
    event_type: &str,
) -> BuddyResult<&'a serde_json::Map<String, serde_json::Value>> {
    payload
        .get(field)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            BuddyError::Validation(format!(
                "invalid action log payload for {event_type}: field={field} is required"
            ))
        })
}

fn required_action_log_payload_object_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
    field_path: &str,
    event_type: &str,
) -> BuddyResult<&'a str> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            BuddyError::Validation(format!(
                "invalid action log payload for {event_type}: field={field_path} is required"
            ))
        })
}

fn optional_action_log_payload_object_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
    field_path: &str,
    event_type: &str,
) -> BuddyResult<Option<&'a str>> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(Some)
        .ok_or_else(|| {
            BuddyError::Validation(format!(
                "invalid action log payload for {event_type}: field={field_path} must be a non-empty string"
            ))
        })
}

fn parse_choreography_trigger_source_action_log_value(
    value: &str,
    field_path: &str,
    event_type: &str,
) -> BuddyResult<ChoreographyTriggerSource> {
    match value {
        "idleAutonomous" => Ok(ChoreographyTriggerSource::IdleAutonomous),
        "aiChoreography" => Ok(ChoreographyTriggerSource::AiChoreography),
        "userRequested" => Ok(ChoreographyTriggerSource::UserRequested),
        "attentionSystem" => Ok(ChoreographyTriggerSource::AttentionSystem),
        "systemRecovery" => Ok(ChoreographyTriggerSource::SystemRecovery),
        "criticalInteraction" => Ok(ChoreographyTriggerSource::CriticalInteraction),
        _ => Err(invalid_action_log_payload_for_event(
            event_type,
            format!("field={field_path} is not supported"),
        )),
    }
}

fn parse_choreography_plan_priority_action_log_value(
    value: &str,
    field_path: &str,
    event_type: &str,
) -> BuddyResult<ChoreographyPlanPriority> {
    match value {
        "idleAutonomous" => Ok(ChoreographyPlanPriority::IdleAutonomous),
        "aiChoreography" => Ok(ChoreographyPlanPriority::AiChoreography),
        "userRequested" => Ok(ChoreographyPlanPriority::UserRequested),
        "systemRecovery" => Ok(ChoreographyPlanPriority::SystemRecovery),
        "attentionSystem" => Ok(ChoreographyPlanPriority::AttentionSystem),
        "criticalInteraction" => Ok(ChoreographyPlanPriority::CriticalInteraction),
        _ => Err(invalid_action_log_payload_for_event(
            event_type,
            format!("field={field_path} is not supported"),
        )),
    }
}

fn invalid_action_log_payload_for_event(event_type: &str, reason: impl Into<String>) -> BuddyError {
    BuddyError::Validation(format!(
        "invalid action log payload for {event_type}: {}",
        reason.into()
    ))
}

#[cfg_attr(not(test), allow(dead_code))]
fn required_action_log_payload_schema_version(
    payload: &serde_json::Value,
    field: &str,
    event_type: &str,
) -> BuddyResult<u16> {
    let Some(version) = payload.get(field).and_then(serde_json::Value::as_u64) else {
        return Err(BuddyError::Validation(format!(
            "invalid action log payload for {event_type}: field={field} is required"
        )));
    };
    let version = u16::try_from(version).map_err(|_| {
        BuddyError::Validation(format!(
            "invalid action log payload for {event_type}: field={field} is out of range"
        ))
    })?;
    if version == 0 {
        return Err(BuddyError::Validation(format!(
            "invalid action log payload for {event_type}: field={field} must be positive"
        )));
    }

    Ok(version)
}

#[cfg_attr(not(test), allow(dead_code))]
fn count_choreography_pending_execution_bodies(connection: &Connection) -> BuddyResult<usize> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM choreography_pending_execution_bodies",
        [],
        |row| row.get::<_, i64>(0),
    )?;

    usize::try_from(count).map_err(|_| {
        BuddyError::Validation("pending execution body count is out of range".to_owned())
    })
}

fn delete_action_log_event_projection(connection: &Connection, event_id: &str) -> BuddyResult<()> {
    connection.execute(
        r#"
        DELETE FROM action_log_events
        WHERE event_id = ?1
        "#,
        params![event_id],
    )?;

    Ok(())
}

fn reset_action_log_index_projection(connection: &Connection) -> BuddyResult<()> {
    connection.execute("DELETE FROM action_log_events", [])?;
    connection.execute("DELETE FROM action_log_plan_summaries", [])?;
    connection.execute(
        r#"
        DELETE FROM action_log_index_watermarks
        WHERE source_file = ?1
        "#,
        params![ACTION_LOG_JSONL_RELATIVE_PATH],
    )?;

    Ok(())
}

impl ActionLogIndexState {
    fn failed(last_indexed_at: Option<String>) -> Self {
        Self {
            stale: true,
            status: ACTION_LOG_INDEX_STATUS_FAILED,
            last_indexed_at,
        }
    }

    fn fresh(last_indexed_at: Option<String>) -> Self {
        Self {
            stale: false,
            status: ACTION_LOG_INDEX_STATUS_FRESH,
            last_indexed_at,
        }
    }

    fn stale(last_indexed_at: Option<String>) -> Self {
        Self {
            stale: true,
            status: ACTION_LOG_INDEX_STATUS_STALE,
            last_indexed_at,
        }
    }
}

fn choreography_action_log_index_health_from_state(
    state: ActionLogIndexState,
) -> ChoreographyActionLogIndexHealth {
    match state.status {
        ACTION_LOG_INDEX_STATUS_FRESH => ChoreographyActionLogIndexHealth::Fresh,
        ACTION_LOG_INDEX_STATUS_STALE => ChoreographyActionLogIndexHealth::Stale,
        ACTION_LOG_INDEX_STATUS_FAILED => ChoreographyActionLogIndexHealth::Failed,
        _ => ChoreographyActionLogIndexHealth::Failed,
    }
}

fn insert_action_log_event(
    connection: &Connection,
    event: &ActionLogEvent,
    source_ref_projection: &ActionLogSourceRefProjection<'_>,
) -> BuddyResult<()> {
    connection.execute(
        r#"
        INSERT INTO action_log_events(
          event_id,
          schema_version,
          event_type,
          status,
          reason_code,
          trigger_source,
          plan_id,
          step_id,
          source_ref_kind,
          source_ref_id,
          result_kind,
          source_ref_json,
          payload_json,
          created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
        params![
            event.event_id,
            event.schema_version,
            event.event_type,
            event.status,
            event.reason_code,
            event.trigger_source,
            event.plan_id,
            event.step_id,
            source_ref_projection.kind,
            source_ref_projection.source_ref_id,
            action_log_event_result_kind(event),
            serde_json::to_string(&event.source_ref)?,
            serde_json::to_string(&event.payload)?,
            event.created_at,
        ],
    )?;

    Ok(())
}

fn insert_action_log_system_event(
    connection: &Connection,
    event: &ActionLogSystemEvent,
    source_ref_projection: &ActionLogSystemSourceRefProjection<'_>,
) -> BuddyResult<()> {
    connection.execute(
        r#"
        INSERT INTO action_log_events(
          event_id,
          schema_version,
          event_type,
          status,
          reason_code,
          trigger_source,
          plan_id,
          step_id,
          source_ref_kind,
          source_ref_id,
          result_kind,
          source_ref_json,
          payload_json,
          created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            event.event_id,
            event.schema_version,
            event.event_type,
            event.status,
            event.reason_code,
            event.trigger_source,
            source_ref_projection.kind,
            source_ref_projection.source_ref_id,
            ACTION_LOG_RESULT_KIND_DEGRADED,
            serde_json::to_string(&event.source_ref)?,
            serde_json::to_string(&event.payload)?,
            event.created_at,
        ],
    )?;

    Ok(())
}

fn upsert_action_log_index_watermark(
    connection: &Connection,
    event: &ActionLogEvent,
    append_cursor: &ActionLogJsonlAppendCursor,
) -> BuddyResult<()> {
    upsert_action_log_index_watermark_record(
        connection,
        &event.event_id,
        &event.created_at,
        append_cursor,
    )
}

fn upsert_action_log_system_index_watermark(
    connection: &Connection,
    event: &ActionLogSystemEvent,
    append_cursor: &ActionLogJsonlAppendCursor,
) -> BuddyResult<()> {
    upsert_action_log_index_watermark_record(
        connection,
        &event.event_id,
        &event.created_at,
        append_cursor,
    )
}

fn upsert_action_log_index_watermark_record(
    connection: &Connection,
    event_id: &str,
    created_at: &str,
    append_cursor: &ActionLogJsonlAppendCursor,
) -> BuddyResult<()> {
    connection.execute(
        r#"
        INSERT INTO action_log_index_watermarks(
          source_file,
          byte_offset,
          line_number,
          event_id,
          updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(source_file) DO UPDATE SET
          byte_offset = excluded.byte_offset,
          line_number = excluded.line_number,
          event_id = excluded.event_id,
          updated_at = excluded.updated_at
        "#,
        params![
            append_cursor.source_file,
            append_cursor.byte_offset,
            append_cursor.line_number,
            event_id,
            created_at,
        ],
    )?;

    Ok(())
}

struct ActionLogSourceRefProjection<'a> {
    kind: &'a str,
    source_ref_id: Option<&'a str>,
}

struct ActionLogSourceRefSchema {
    required_fields: &'static [&'static str],
    allowed_fields: &'static [&'static str],
    primary_id_field: Option<&'static str>,
}

fn validate_action_log_source_ref(
    source_ref: &serde_json::Value,
) -> BuddyResult<ActionLogSourceRefProjection<'_>> {
    let object = source_ref
        .as_object()
        .ok_or_else(|| invalid_action_log_source_ref("sourceRef must be an object".to_owned()))?;
    let kind = required_action_log_source_ref_string(object, "kind", None)?;
    let schema = action_log_source_ref_schema(kind)
        .ok_or_else(|| invalid_action_log_source_ref(format!("kind={kind} is not supported")))?;

    for field in object.keys() {
        if !schema.allowed_fields.contains(&field.as_str()) {
            return Err(invalid_action_log_source_ref(format!(
                "kind={kind} field={field} is not allowed"
            )));
        }
    }
    for field in schema.required_fields {
        required_action_log_source_ref_string(object, field, Some(kind))?;
    }
    let source_ref_id = schema
        .primary_id_field
        .map(|field| required_action_log_source_ref_string(object, field, Some(kind)))
        .transpose()?;

    Ok(ActionLogSourceRefProjection {
        kind,
        source_ref_id,
    })
}

fn action_log_source_ref_schema(kind: &str) -> Option<ActionLogSourceRefSchema> {
    match kind {
        "conversationMessage" => Some(ActionLogSourceRefSchema {
            required_fields: &["conversationId", "messageId"],
            allowed_fields: &["kind", "conversationId", "messageId", "runId"],
            primary_id_field: Some("messageId"),
        }),
        "run" => Some(ActionLogSourceRefSchema {
            required_fields: &["runId"],
            allowed_fields: &["kind", "runId", "conversationId"],
            primary_id_field: Some("runId"),
        }),
        "approval" => Some(ActionLogSourceRefSchema {
            required_fields: &["approvalId"],
            allowed_fields: &["kind", "approvalId", "runId"],
            primary_id_field: Some("approvalId"),
        }),
        "presetBehavior" => Some(ActionLogSourceRefSchema {
            required_fields: &["presetBehaviorId"],
            allowed_fields: &["kind", "presetBehaviorId", "interactionId", "sessionId"],
            primary_id_field: Some("presetBehaviorId"),
        }),
        "systemRecovery" => Some(ActionLogSourceRefSchema {
            required_fields: &["triggeredByPlanId", "triggerReason"],
            allowed_fields: &[
                "kind",
                "triggeredByPlanId",
                "triggeredByStepId",
                "triggerReason",
            ],
            primary_id_field: Some("triggeredByPlanId"),
        }),
        "macroFallback" => Some(ActionLogSourceRefSchema {
            required_fields: &[
                "triggeredByPlanId",
                "triggeredByStepId",
                "triggerReason",
                "originalMacroId",
                "fallbackMacroId",
            ],
            allowed_fields: &[
                "kind",
                "triggeredByPlanId",
                "triggeredByStepId",
                "triggerReason",
                "originalMacroId",
                "fallbackMacroId",
            ],
            primary_id_field: Some("triggeredByPlanId"),
        }),
        "startupSystem" => Some(ActionLogSourceRefSchema {
            required_fields: &[],
            allowed_fields: &["kind"],
            primary_id_field: None,
        }),
        "devFixture" => Some(ActionLogSourceRefSchema {
            required_fields: &["fixtureName"],
            allowed_fields: &["kind", "fixtureName"],
            primary_id_field: Some("fixtureName"),
        }),
        _ => None,
    }
}

fn required_action_log_source_ref_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
    kind: Option<&str>,
) -> BuddyResult<&'a str> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            let prefix = kind.map(|kind| format!("kind={kind} ")).unwrap_or_default();
            invalid_action_log_source_ref(format!("{prefix}field={field} is required"))
        })
}

fn invalid_action_log_source_ref(reason: String) -> BuddyError {
    BuddyError::Validation(format!("invalid action log sourceRef: {reason}"))
}

fn validate_action_log_system_source_ref(
    source_ref: &serde_json::Value,
) -> BuddyResult<ActionLogSystemSourceRefProjection<'_>> {
    let object = source_ref.as_object().ok_or_else(|| {
        BuddyError::Validation("invalid action log system sourceRef: must be an object".to_owned())
    })?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            BuddyError::Validation(
                "invalid action log system sourceRef: field=kind is required".to_owned(),
            )
        })?;
    let schema = action_log_system_source_ref_schema(kind).ok_or_else(|| {
        invalid_action_log_system_source_ref(format!("kind={kind} is not supported"))
    })?;

    for field in object.keys() {
        if !schema.allowed_fields.contains(&field.as_str()) {
            return Err(invalid_action_log_system_source_ref(format!(
                "kind={kind} field={field} is not allowed"
            )));
        }
    }
    for field in schema.required_fields {
        required_action_log_system_source_ref_string(object, field, kind)?;
    }

    Ok(ActionLogSystemSourceRefProjection {
        kind,
        source_ref_id: None,
    })
}

fn action_log_system_source_ref_schema(kind: &str) -> Option<ActionLogSystemSourceRefSchema> {
    match kind {
        "actionLogIndex" => Some(ActionLogSystemSourceRefSchema {
            required_fields: &[],
            allowed_fields: &["kind"],
        }),
        "runtime" => Some(ActionLogSystemSourceRefSchema {
            required_fields: &[],
            allowed_fields: &["kind"],
        }),
        "healthGate" => Some(ActionLogSystemSourceRefSchema {
            required_fields: &[],
            allowed_fields: &["kind"],
        }),
        "choreographyScheduler" => Some(ActionLogSystemSourceRefSchema {
            required_fields: &[],
            allowed_fields: &["kind"],
        }),
        "affectiveContext" => Some(ActionLogSystemSourceRefSchema {
            required_fields: &["stateFileName"],
            allowed_fields: &["kind", "stateFileName"],
        }),
        _ => None,
    }
}

fn required_action_log_system_source_ref_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
    kind: &str,
) -> BuddyResult<&'a str> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            invalid_action_log_system_source_ref(format!("kind={kind} field={field} is required"))
        })
}

fn invalid_action_log_system_source_ref(reason: String) -> BuddyError {
    BuddyError::Validation(format!("invalid action log system sourceRef: {reason}"))
}

struct ActionLogEventSchema {
    statuses: &'static [&'static str],
    reason_codes: &'static [&'static str],
}

fn validate_action_log_event_schema(event: &ActionLogEvent) -> BuddyResult<()> {
    validate_action_log_schema_version("event", event.schema_version)?;
    validate_action_log_event_schema_parts(&event.event_type, &event.status, &event.reason_code)?;
    validate_action_log_payload_detail(&event.payload)
}

fn validate_action_log_system_event_schema(event: &ActionLogSystemEvent) -> BuddyResult<()> {
    validate_action_log_schema_version("system event", event.schema_version)?;
    if event.plan_id.is_some() || event.step_id.is_some() {
        return Err(BuddyError::Validation(
            "invalid action log system event schema: planId and stepId must be omitted".to_owned(),
        ));
    }

    validate_action_log_event_schema_parts(&event.event_type, &event.status, &event.reason_code)?;
    validate_action_log_payload_detail(&event.payload)
}

fn validate_action_log_schema_version(scope: &str, schema_version: u16) -> BuddyResult<()> {
    if schema_version == ACTION_LOG_SCHEMA_VERSION {
        return Ok(());
    }

    Err(BuddyError::Validation(format!(
        "invalid action log {scope} schema: schemaVersion={schema_version} is not supported"
    )))
}

fn validate_action_log_event_schema_parts(
    event_type: &str,
    status: &str,
    reason_code: &str,
) -> BuddyResult<()> {
    let Some(schema) = action_log_event_schema(event_type) else {
        return Err(BuddyError::Validation(format!(
            "invalid action log event schema: eventType={event_type} is not supported"
        )));
    };

    if !schema.statuses.contains(&status) {
        return Err(BuddyError::Validation(format!(
            "invalid action log event schema: eventType={event_type} status={status} is not allowed"
        )));
    }
    if !schema.reason_codes.contains(&reason_code) {
        return Err(BuddyError::Validation(format!(
            "invalid action log event schema: eventType={event_type} reasonCode={reason_code} is not allowed"
        )));
    }

    Ok(())
}

fn sanitize_action_log_event(event: &ActionLogEvent) -> BuddyResult<ActionLogEvent> {
    let mut event = event.clone();
    event.payload = sanitize_action_log_payload(&event.payload)?;
    Ok(event)
}

fn sanitize_action_log_system_event(
    event: &ActionLogSystemEvent,
) -> BuddyResult<ActionLogSystemEvent> {
    let mut event = event.clone();
    event.payload = sanitize_action_log_payload(&event.payload)?;
    Ok(event)
}

fn sanitize_action_log_payload(payload: &serde_json::Value) -> BuddyResult<serde_json::Value> {
    validate_action_log_payload_detail(payload)?;
    if payload.get("detail").is_none() {
        return Ok(payload.clone());
    }

    let mut payload = payload.clone();
    if let Some(object) = payload.as_object_mut() {
        if let Some(detail) = object.get("detail") {
            object.insert(
                "detail".to_owned(),
                sanitize_action_log_payload_detail(detail)?,
            );
        }
    }

    Ok(payload)
}

fn sanitize_action_log_payload_detail(
    detail: &serde_json::Value,
) -> BuddyResult<serde_json::Value> {
    let object = detail.as_object().ok_or_else(|| {
        BuddyError::Validation("invalid action log payload detail: must be an object".to_owned())
    })?;
    let mut sanitized = serde_json::Map::new();
    let mut changed = false;

    for (field, value) in object {
        match field.as_str() {
            "message" | "rawCode" | "source" => {
                let (value, field_changed) =
                    sanitize_action_log_payload_detail_string(value.as_str().ok_or_else(|| {
                        invalid_action_log_payload_detail(format!("field={field} must be a string"))
                    })?);
                sanitized.insert(field.to_owned(), serde_json::Value::String(value));
                changed |= field_changed;
            }
            "truncated" => {
                sanitized.insert(field.to_owned(), value.clone());
            }
            "items" => {
                let (items, items_changed) = sanitize_action_log_payload_detail_items(value)?;
                sanitized.insert(field.to_owned(), items);
                changed |= items_changed;
            }
            _ => {
                sanitized.insert(field.to_owned(), value.clone());
            }
        }
    }

    changed |= trim_action_log_payload_detail_to_total_limit(&mut sanitized)?;
    if changed {
        sanitized.insert("truncated".to_owned(), serde_json::json!(true));
        trim_action_log_payload_detail_to_total_limit(&mut sanitized)?;
    }

    Ok(serde_json::Value::Object(sanitized))
}

fn sanitize_action_log_payload_detail_items(
    items: &serde_json::Value,
) -> BuddyResult<(serde_json::Value, bool)> {
    let array = items.as_array().ok_or_else(|| {
        invalid_action_log_payload_detail("field=items must be an array".to_owned())
    })?;
    let mut changed = false;
    let mut sanitized_items = Vec::with_capacity(array.len());

    for item in array {
        if let Some(value) = item.as_str() {
            let (value, item_changed) = sanitize_action_log_payload_detail_string(value);
            sanitized_items.push(serde_json::Value::String(value));
            changed |= item_changed;
            continue;
        }

        let object = item.as_object().ok_or_else(|| {
            invalid_action_log_payload_detail(
                "items[] must be a string or key/value object".to_owned(),
            )
        })?;
        let mut sanitized_item = serde_json::Map::new();
        for field in ACTION_LOG_DIAGNOSTIC_DETAIL_ITEM_ALLOWED_FIELDS {
            let value = object
                .get(*field)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    invalid_action_log_payload_detail(format!("items[].{field} must be a string"))
                })?;
            let (value, item_changed) = sanitize_action_log_payload_detail_string(value);
            sanitized_item.insert((*field).to_owned(), serde_json::Value::String(value));
            changed |= item_changed;
        }
        sanitized_items.push(serde_json::Value::Object(sanitized_item));
    }

    Ok((serde_json::Value::Array(sanitized_items), changed))
}

fn sanitize_action_log_payload_detail_string(value: &str) -> (String, bool) {
    let redacted = redact_action_log_detail_paths(&redact_action_log_detail_secrets(
        &sanitize_process_stderr_line(value),
    ));
    let (limited, truncated) = truncate_action_log_detail_string(&redacted);
    let changed = value != limited || truncated;

    (limited, changed)
}

fn truncate_action_log_detail_string(value: &str) -> (String, bool) {
    if value.chars().count() <= ACTION_LOG_DIAGNOSTIC_DETAIL_FIELD_MAX_CHARS {
        return (value.to_owned(), false);
    }

    (
        value
            .chars()
            .take(ACTION_LOG_DIAGNOSTIC_DETAIL_FIELD_MAX_CHARS)
            .collect(),
        true,
    )
}

fn trim_action_log_payload_detail_to_total_limit(
    detail: &mut serde_json::Map<String, serde_json::Value>,
) -> BuddyResult<bool> {
    let mut changed = false;

    while serde_json::to_vec(detail)?.len() > ACTION_LOG_DIAGNOSTIC_DETAIL_TOTAL_MAX_BYTES {
        if let Some(items) = detail
            .get_mut("items")
            .and_then(serde_json::Value::as_array_mut)
        {
            if items.pop().is_some() {
                changed = true;
                continue;
            }
        }

        if truncate_longest_action_log_detail_string(detail) {
            changed = true;
            continue;
        }

        break;
    }

    Ok(changed)
}

fn truncate_longest_action_log_detail_string(
    detail: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let longest_field = ["message", "rawCode", "source"]
        .into_iter()
        .filter_map(|field| {
            detail
                .get(field)
                .and_then(serde_json::Value::as_str)
                .map(|value| (field, value.chars().count()))
        })
        .max_by_key(|(_, length)| *length);
    let Some((field, length)) = longest_field else {
        return false;
    };
    if length <= ACTION_LOG_DIAGNOSTIC_DETAIL_TOTAL_FIELD_FLOOR_CHARS {
        return false;
    }

    let target_length = (length / 2).max(ACTION_LOG_DIAGNOSTIC_DETAIL_TOTAL_FIELD_FLOOR_CHARS);
    let Some(value) = detail.get(field).and_then(serde_json::Value::as_str) else {
        return false;
    };
    let truncated = value.chars().take(target_length).collect::<String>();
    detail.insert(field.to_owned(), serde_json::Value::String(truncated));

    true
}

fn redact_action_log_detail_secrets(value: &str) -> String {
    [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "CODEX_API_KEY",
        "API_KEY",
        "HF_TOKEN",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "BUDDY_SESSION_TOKEN",
        "LEXORA_SESSION_TOKEN",
        "access_token",
        "refresh_token",
        "id_token",
        "auth_token",
        "client_secret",
        "secret_key",
        "password",
        "private_key",
        "cookie",
        "set-cookie",
    ]
    .into_iter()
    .fold(value.to_owned(), redact_action_log_detail_secret_assignment)
}

fn redact_action_log_detail_secret_assignment(value: String, marker: &str) -> String {
    let mut result = value;
    let mut search_start = 0;
    while search_start < result.len() {
        let Some(relative_index) = find_ascii_case_insensitive(&result[search_start..], marker)
        else {
            break;
        };
        let index = search_start + relative_index;
        if !action_log_detail_secret_marker_has_boundary(&result, index, marker.len()) {
            search_start = index + marker.len();
            continue;
        }
        let Some((value_start, value_end)) =
            action_log_detail_assignment_value_range(&result, index, marker.len())
        else {
            search_start = index + marker.len();
            continue;
        };

        let mut redacted = String::with_capacity(result.len());
        redacted.push_str(&result[..value_start]);
        redacted.push_str("[redacted]");
        redacted.push_str(&result[value_end..]);
        result = redacted;
        search_start = value_start + "[redacted]".len();
    }

    result
}

fn action_log_detail_secret_marker_has_boundary(
    value: &str,
    marker_start: usize,
    marker_len: usize,
) -> bool {
    let has_prefix_boundary = value[..marker_start]
        .chars()
        .next_back()
        .is_none_or(|character| !is_action_log_detail_secret_key_character(character));
    let marker_end = marker_start + marker_len;
    let has_suffix_boundary = value[marker_end..]
        .chars()
        .next()
        .is_none_or(|character| !is_action_log_detail_secret_key_character(character));

    has_prefix_boundary && has_suffix_boundary
}

fn is_action_log_detail_secret_key_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn action_log_detail_assignment_value_range(
    value: &str,
    marker_start: usize,
    marker_len: usize,
) -> Option<(usize, usize)> {
    let mut separator_start = marker_start + marker_len;
    for character in value[separator_start..].chars() {
        if character.is_ascii_whitespace() || matches!(character, '"' | '\'' | '`') {
            separator_start += character.len_utf8();
            continue;
        }
        break;
    }

    let separator = value[separator_start..].chars().next()?;
    if !matches!(separator, '=' | ':') {
        return None;
    }

    let mut value_start = separator_start + separator.len_utf8();
    for character in value[value_start..].chars() {
        if character.is_ascii_whitespace() {
            value_start += character.len_utf8();
            continue;
        }
        break;
    }

    let value_end = value[value_start..]
        .char_indices()
        .find_map(|(index, character)| {
            (character.is_ascii_whitespace()
                || matches!(character, '"' | '\'' | '`' | ',' | ';' | '}' | ']'))
            .then_some(value_start + index)
        })
        .unwrap_or(value.len());

    (value_end > value_start).then_some((value_start, value_end))
}

fn find_ascii_case_insensitive(value: &str, needle: &str) -> Option<usize> {
    value
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn redact_action_log_detail_paths(value: &str) -> String {
    let mut redacted = String::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let next = value[index..].chars().next().expect("valid char boundary");
        if next == '/' {
            let end = value[index..]
                .char_indices()
                .skip(1)
                .find_map(|(offset, character)| {
                    (character.is_ascii_whitespace()
                        || matches!(character, '"' | '\'' | '`' | ',' | ';' | ')' | ']' | '}'))
                    .then_some(index + offset)
                })
                .unwrap_or(value.len());
            let candidate = &value[index..end];
            if candidate.len() > 1 {
                redacted.push_str(ACTION_LOG_DIAGNOSTIC_DETAIL_PATH_REDACTION);
                index = end;
                continue;
            }
        }

        redacted.push(next);
        index += next.len_utf8();
    }

    redacted
}

fn validate_action_log_payload_detail(payload: &serde_json::Value) -> BuddyResult<()> {
    let Some(detail) = payload.get("detail") else {
        return Ok(());
    };
    let object = detail.as_object().ok_or_else(|| {
        BuddyError::Validation("invalid action log payload detail: must be an object".to_owned())
    })?;

    for field in object.keys() {
        if !ACTION_LOG_DIAGNOSTIC_DETAIL_ALLOWED_FIELDS.contains(&field.as_str()) {
            return Err(invalid_action_log_payload_detail(format!(
                "field={field} is not allowed"
            )));
        }
    }

    for (field, value) in object {
        validate_action_log_payload_detail_field(field, value)?;
    }

    Ok(())
}

fn validate_action_log_payload_detail_field(
    field: &str,
    value: &serde_json::Value,
) -> BuddyResult<()> {
    match field {
        "message" | "rawCode" | "source" => {
            if value.is_string() {
                Ok(())
            } else {
                Err(invalid_action_log_payload_detail(format!(
                    "field={field} must be a string"
                )))
            }
        }
        "truncated" => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(invalid_action_log_payload_detail(
                    "field=truncated must be a boolean".to_owned(),
                ))
            }
        }
        "items" => validate_action_log_payload_detail_items(value),
        _ => Ok(()),
    }
}

fn validate_action_log_payload_detail_items(items: &serde_json::Value) -> BuddyResult<()> {
    let array = items.as_array().ok_or_else(|| {
        invalid_action_log_payload_detail("field=items must be an array".to_owned())
    })?;

    for (index, item) in array.iter().enumerate() {
        validate_action_log_payload_detail_item(index, item)?;
    }

    Ok(())
}

fn validate_action_log_payload_detail_item(
    index: usize,
    item: &serde_json::Value,
) -> BuddyResult<()> {
    if item.is_string() {
        return Ok(());
    }

    let object = item.as_object().ok_or_else(|| {
        invalid_action_log_payload_detail(format!(
            "items[{index}] must be a string or key/value object"
        ))
    })?;

    for field in object.keys() {
        if !ACTION_LOG_DIAGNOSTIC_DETAIL_ITEM_ALLOWED_FIELDS.contains(&field.as_str()) {
            return Err(invalid_action_log_payload_detail(format!(
                "items[{index}].field={field} is not allowed"
            )));
        }
    }

    for field in ACTION_LOG_DIAGNOSTIC_DETAIL_ITEM_ALLOWED_FIELDS {
        let Some(value) = object.get(*field) else {
            return Err(invalid_action_log_payload_detail(format!(
                "items[{index}].{field} is required"
            )));
        };
        if !value.is_string() {
            return Err(invalid_action_log_payload_detail(format!(
                "items[{index}].{field} must be a string"
            )));
        }
    }

    Ok(())
}

fn invalid_action_log_payload_detail(reason: String) -> BuddyError {
    BuddyError::Validation(format!("invalid action log payload detail: {reason}"))
}

fn action_log_event_schema(event_type: &str) -> Option<ActionLogEventSchema> {
    match event_type {
        "plan.started" => Some(ActionLogEventSchema {
            statuses: &["started"],
            reason_codes: &[
                "devFixture.started",
                "presetBehavior.started",
                "run.hostAction.started",
                "systemRecovery.started",
                "timeline.started",
            ],
        }),
        "plan.completed" => Some(ActionLogEventSchema {
            statuses: &["completed"],
            reason_codes: &[
                "devFixture.completed",
                "presetBehavior.completed",
                "run.hostAction.completed",
                "systemRecovery.completed",
                "timeline.completed",
            ],
        }),
        "plan.failed" => Some(ActionLogEventSchema {
            statuses: &["failed"],
            reason_codes: &[
                "devFixture.failed",
                "run.hostAction.failed",
                "systemRecovery.failed",
                "timeline.failed",
            ],
        }),
        "plan.interrupted" => Some(ActionLogEventSchema {
            statuses: &["interrupted"],
            reason_codes: &[
                "devFixture.yieldedToPendingPlan",
                "runtime.restarted",
                "timeline.yieldedToPendingPlan",
            ],
        }),
        "step.resolved" => Some(ActionLogEventSchema {
            statuses: &["resolved"],
            reason_codes: &[
                "devFixture.stepResolved",
                "presetBehavior.resolved",
                "run.hostAction.resolved",
                "fallback.registrySelected",
                "systemRecovery.stepResolved",
                "timeline.stepResolved",
            ],
        }),
        "step.completed" => Some(ActionLogEventSchema {
            statuses: &["completed"],
            reason_codes: &[
                "devFixture.stepCompleted",
                "presetBehavior.stepCompleted",
                "run.hostAction.stepCompleted",
                "systemRecovery.stepCompleted",
                "timeline.stepCompleted",
            ],
        }),
        "step.failed" => Some(ActionLogEventSchema {
            statuses: &["failed"],
            reason_codes: &[
                "devFixture.stepFailed",
                "run.hostAction.stepFailed",
                "systemRecovery.stepFailed",
                "timeline.stepFailed",
            ],
        }),
        "step.skipped" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_SKIPPED],
            reason_codes: &["timeline.stepSkipped"],
        }),
        "step.interrupted" => Some(ActionLogEventSchema {
            statuses: &["interrupted"],
            reason_codes: &["runtime.restarted", "sidecar.stepInterrupted"],
        }),
        "fallback.registrySelected" => Some(ActionLogEventSchema {
            statuses: &["resolved"],
            reason_codes: &["fallback.registrySelected"],
        }),
        "executor.accepted" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_RUNNING],
            reason_codes: &["executor.accepted"],
        }),
        "executor.preempted" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_RUNNING],
            reason_codes: &["admission.preemptedByHigherPriorityPlan"],
        }),
        "executor.rejected" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_REJECTED],
            reason_codes: &["executor.busy"],
        }),
        "executor.deferred" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_DEFERRED],
            reason_codes: &["admission.waitingForActiveStepToFinish"],
        }),
        "executor.skipped" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_SKIPPED],
            reason_codes: &["priority.tooLow"],
        }),
        "affectiveContext.invalidStateFile" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_RESULT_KIND_DEGRADED],
            reason_codes: &["affectiveContext.invalidStateFile"],
        }),
        "actionLogIndex.syncFailed" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_RESULT_KIND_DEGRADED],
            reason_codes: &["actionLogIndex.syncFailed"],
        }),
        "choreographyScheduler.pendingBodyMissing" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_RESULT_KIND_DEGRADED],
            reason_codes: &["choreographyScheduler.pendingBodyMissing"],
        }),
        "choreographyScheduler.pendingBodyStored" => Some(ActionLogEventSchema {
            statuses: &["completed"],
            reason_codes: &["choreographyScheduler.pendingBodyStored"],
        }),
        "choreographyScheduler.pendingBodyDeleted" => Some(ActionLogEventSchema {
            statuses: &["completed"],
            reason_codes: &["choreographyScheduler.pendingBodyDeleted"],
        }),
        "choreographyScheduler.stalePendingBodiesCleared" => Some(ActionLogEventSchema {
            statuses: &["completed"],
            reason_codes: &["runtime.restarted"],
        }),
        "runtime.degraded" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_RESULT_KIND_DEGRADED],
            reason_codes: &["runtime.systemRecoveryFailed"],
        }),
        "startupHealth.failed" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_FAILED],
            reason_codes: &["startupHealth.nativePetUnavailable"],
        }),
        "healthGate.passed" => Some(ActionLogEventSchema {
            statuses: &["passed"],
            reason_codes: &["sidecar.available"],
        }),
        "healthGate.failed" => Some(ActionLogEventSchema {
            statuses: &[ACTION_LOG_DETAIL_STATUS_FAILED],
            reason_codes: &["sidecar.unavailable"],
        }),
        _ => None,
    }
}

fn action_log_event_result_kind(event: &ActionLogEvent) -> &'static str {
    if let Some(result_kind) = action_log_payload_result_kind(&event.payload) {
        return result_kind;
    }
    if event.status == ACTION_LOG_RESULT_KIND_INTERRUPTED
        || event.event_type.ends_with(".interrupted")
    {
        return ACTION_LOG_RESULT_KIND_INTERRUPTED;
    }
    if event.status == ACTION_LOG_RESULT_KIND_DEGRADED {
        return ACTION_LOG_RESULT_KIND_DEGRADED;
    }
    if event.event_type.starts_with("fallback.") || event.reason_code.starts_with("fallback.") {
        return ACTION_LOG_RESULT_KIND_FALLBACK;
    }

    ACTION_LOG_RESULT_KIND_NORMAL
}

fn action_log_payload_result_kind(payload: &serde_json::Value) -> Option<&'static str> {
    payload
        .get("resultKind")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            payload
                .get("result")
                .and_then(|result| result.get("kind"))
                .and_then(serde_json::Value::as_str)
        })
        .and_then(normalize_action_log_result_kind)
}

fn normalize_action_log_result_kind(value: &str) -> Option<&'static str> {
    match value {
        ACTION_LOG_RESULT_KIND_NORMAL => Some(ACTION_LOG_RESULT_KIND_NORMAL),
        ACTION_LOG_RESULT_KIND_FALLBACK => Some(ACTION_LOG_RESULT_KIND_FALLBACK),
        ACTION_LOG_RESULT_KIND_DEGRADED => Some(ACTION_LOG_RESULT_KIND_DEGRADED),
        ACTION_LOG_RESULT_KIND_INTERRUPTED => Some(ACTION_LOG_RESULT_KIND_INTERRUPTED),
        _ => None,
    }
}

fn action_log_event_detail_status(event: &ActionLogEvent, result_kind: &str) -> &'static str {
    if result_kind == ACTION_LOG_RESULT_KIND_INTERRUPTED {
        return ACTION_LOG_RESULT_KIND_INTERRUPTED;
    }
    if event.status == ACTION_LOG_DETAIL_STATUS_FAILED || event.event_type == "plan.failed" {
        return ACTION_LOG_DETAIL_STATUS_FAILED;
    }
    if event.status == ACTION_LOG_DETAIL_STATUS_REJECTED || event.event_type == "executor.rejected"
    {
        return ACTION_LOG_DETAIL_STATUS_REJECTED;
    }
    if event.status == ACTION_LOG_DETAIL_STATUS_DEFERRED || event.event_type == "executor.deferred"
    {
        return ACTION_LOG_DETAIL_STATUS_DEFERRED;
    }
    if event.status == ACTION_LOG_DETAIL_STATUS_SKIPPED || event.event_type == "executor.skipped" {
        return ACTION_LOG_DETAIL_STATUS_SKIPPED;
    }
    if result_kind == ACTION_LOG_RESULT_KIND_DEGRADED {
        return ACTION_LOG_RESULT_KIND_DEGRADED;
    }
    if result_kind == ACTION_LOG_RESULT_KIND_FALLBACK {
        return ACTION_LOG_RESULT_KIND_FALLBACK;
    }
    if event.event_type == "plan.completed" {
        return ACTION_LOG_DETAIL_STATUS_COMPLETED;
    }

    ACTION_LOG_DETAIL_STATUS_RUNNING
}

fn upsert_action_log_plan_summary(
    connection: &Connection,
    event: &ActionLogEvent,
    source_ref_projection: &ActionLogSourceRefProjection<'_>,
) -> BuddyResult<()> {
    let summary_status = match event.event_type.as_str() {
        "plan.completed" => "completed",
        "plan.failed" => "failed",
        "plan.interrupted" => ACTION_LOG_RESULT_KIND_INTERRUPTED,
        "executor.rejected" => ACTION_LOG_DETAIL_STATUS_REJECTED,
        "executor.deferred" => ACTION_LOG_DETAIL_STATUS_DEFERRED,
        "executor.skipped" => ACTION_LOG_DETAIL_STATUS_SKIPPED,
        _ => "running",
    };
    let started_at = if event.event_type == "plan.started" {
        Some(event.created_at.as_str())
    } else {
        None
    };
    let completed_at = if matches!(
        event.event_type.as_str(),
        "plan.completed"
            | "plan.failed"
            | "plan.interrupted"
            | "executor.rejected"
            | "executor.deferred"
            | "executor.skipped"
    ) {
        Some(event.created_at.as_str())
    } else {
        None
    };
    let resolved_action_id = event.resolved_action_id();
    let resolved_animation_ref = event.resolved_animation_ref();
    let result_kind = action_log_event_result_kind(event);
    let detail_status = action_log_event_detail_status(event, result_kind);
    let detail_reason_code = action_log_event_detail_reason_code(event);

    connection.execute(
        r#"
        INSERT INTO action_log_plan_summaries(
          plan_id,
          source_ref_kind,
          source_ref_id,
          result_kind,
          source_ref_json,
          status,
          started_at,
          completed_at,
          last_event_type,
          last_reason_code,
          detail_status,
          detail_reason_code,
          resolved_action_id,
          resolved_animation_ref
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        ON CONFLICT(plan_id) DO UPDATE SET
          source_ref_id = COALESCE(excluded.source_ref_id, action_log_plan_summaries.source_ref_id),
          result_kind = CASE
            WHEN action_log_plan_summaries.result_kind = 'interrupted' OR excluded.result_kind = 'interrupted' THEN 'interrupted'
            WHEN action_log_plan_summaries.result_kind = 'degraded' OR excluded.result_kind = 'degraded' THEN 'degraded'
            WHEN action_log_plan_summaries.result_kind = 'fallback' OR excluded.result_kind = 'fallback' THEN 'fallback'
            ELSE 'normal'
          END,
          status = excluded.status,
          completed_at = COALESCE(excluded.completed_at, action_log_plan_summaries.completed_at),
          last_event_type = excluded.last_event_type,
          last_reason_code = excluded.last_reason_code,
          detail_status = CASE
            WHEN excluded.detail_status = 'interrupted' THEN excluded.detail_status
            WHEN action_log_plan_summaries.detail_status = 'interrupted' THEN action_log_plan_summaries.detail_status
            WHEN excluded.detail_status = 'failed' THEN excluded.detail_status
            WHEN action_log_plan_summaries.detail_status = 'failed' THEN action_log_plan_summaries.detail_status
            WHEN excluded.detail_status = 'rejected' THEN excluded.detail_status
            WHEN action_log_plan_summaries.detail_status = 'rejected' THEN action_log_plan_summaries.detail_status
            WHEN excluded.detail_status = 'skipped' THEN excluded.detail_status
            WHEN action_log_plan_summaries.detail_status = 'skipped' THEN action_log_plan_summaries.detail_status
            WHEN excluded.detail_status = 'degraded' THEN excluded.detail_status
            WHEN action_log_plan_summaries.detail_status = 'degraded' THEN action_log_plan_summaries.detail_status
            WHEN excluded.detail_status = 'fallback' THEN excluded.detail_status
            WHEN action_log_plan_summaries.detail_status = 'fallback' THEN action_log_plan_summaries.detail_status
            WHEN excluded.detail_status = 'completed' THEN excluded.detail_status
            ELSE action_log_plan_summaries.detail_status
          END,
          detail_reason_code = CASE
            WHEN excluded.detail_status = 'interrupted' THEN excluded.detail_reason_code
            WHEN action_log_plan_summaries.detail_status = 'interrupted' THEN action_log_plan_summaries.detail_reason_code
            WHEN excluded.detail_status = 'failed' THEN excluded.detail_reason_code
            WHEN action_log_plan_summaries.detail_status = 'failed' THEN action_log_plan_summaries.detail_reason_code
            WHEN excluded.detail_status = 'rejected' THEN excluded.detail_reason_code
            WHEN action_log_plan_summaries.detail_status = 'rejected' THEN action_log_plan_summaries.detail_reason_code
            WHEN excluded.detail_status = 'skipped' THEN excluded.detail_reason_code
            WHEN action_log_plan_summaries.detail_status = 'skipped' THEN action_log_plan_summaries.detail_reason_code
            WHEN excluded.detail_status = 'degraded' THEN excluded.detail_reason_code
            WHEN action_log_plan_summaries.detail_status = 'degraded' THEN action_log_plan_summaries.detail_reason_code
            WHEN excluded.detail_status = 'fallback' THEN excluded.detail_reason_code
            WHEN action_log_plan_summaries.detail_status = 'fallback' THEN action_log_plan_summaries.detail_reason_code
            WHEN excluded.detail_status = 'completed' THEN excluded.detail_reason_code
            ELSE action_log_plan_summaries.detail_reason_code
          END,
          resolved_action_id = COALESCE(excluded.resolved_action_id, action_log_plan_summaries.resolved_action_id),
          resolved_animation_ref = COALESCE(excluded.resolved_animation_ref, action_log_plan_summaries.resolved_animation_ref)
        "#,
        params![
            event.plan_id,
            source_ref_projection.kind,
            source_ref_projection.source_ref_id,
            result_kind,
            serde_json::to_string(&event.source_ref)?,
            summary_status,
            started_at,
            completed_at,
            event.event_type,
            event.reason_code,
            detail_status,
            detail_reason_code,
            resolved_action_id,
            resolved_animation_ref,
        ],
    )?;

    Ok(())
}

fn action_log_event_detail_reason_code(event: &ActionLogEvent) -> &str {
    event
        .payload
        .get("detailReasonCode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(event.reason_code.as_str())
}

#[cfg(all(test, unix))]
mod action_log_jsonl_durability_tests {
    use std::{
        ffi::CString,
        fs::{self, File},
        io::Read,
        os::unix::ffi::OsStrExt,
        thread,
    };

    use super::*;

    #[test]
    fn append_does_not_acknowledge_when_the_durability_barrier_fails() {
        let directory = std::env::temp_dir().join(format!(
            "lexora-buddy-action-log-durability-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).expect("create durability test directory");
        let fifo_path = directory.join("action-log.jsonl");
        let fifo_path_c =
            CString::new(fifo_path.as_os_str().as_bytes()).expect("fifo path CString");
        // SAFETY: fifo_path_c is a valid null-terminated path and mode contains only permission bits.
        let result = unsafe { libc::mkfifo(fifo_path_c.as_ptr(), 0o600) };
        assert_eq!(
            result,
            0,
            "create FIFO: {}",
            std::io::Error::last_os_error()
        );

        let reader_path = fifo_path.clone();
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            File::open(reader_path)
                .expect("open FIFO reader")
                .read_to_end(&mut bytes)
                .expect("read FIFO payload");
            bytes
        });
        let append_position = ActionLogJsonlAppendPosition {
            cursor: ActionLogJsonlAppendCursor {
                source_file: ACTION_LOG_JSONL_RELATIVE_PATH,
                byte_offset: 0,
                line_number: 1,
            },
            needs_leading_newline: false,
        };

        let result = append_action_log_jsonl_line_at_position(
            fifo_path.clone(),
            &serde_json::json!({ "eventId": "evt_durability" }),
            append_position,
        );
        let error = match result {
            Ok(_) => panic!("append must fail when sync_data cannot establish durability"),
            Err(error) => error,
        };

        assert!(matches!(error, BuddyError::Io(_)));
        assert!(!reader.join().expect("join FIFO reader").is_empty());
        fs::remove_file(fifo_path).expect("remove FIFO");
        fs::remove_dir(directory).expect("remove durability test directory");
    }
}

#[cfg(test)]
mod action_log_event_schema_tests {
    use super::*;

    #[test]
    fn append_rejects_invalid_event_schema_before_jsonl_write() {
        let mut unsupported_schema_version =
            action_log_event_schema_test_event("plan.started", "started", "devFixture.started");
        unsupported_schema_version.schema_version = 2;
        let cases = [
            (
                action_log_event_schema_test_event(
                    "plan.teleported",
                    "started",
                    "devFixture.started",
                ),
                "buddy state validation failed: invalid action log event schema: eventType=plan.teleported is not supported",
            ),
            (
                unsupported_schema_version,
                "buddy state validation failed: invalid action log event schema: schemaVersion=2 is not supported",
            ),
            (
                action_log_event_schema_test_event(
                    "step.completed",
                    "started",
                    "devFixture.stepCompleted",
                ),
                "buddy state validation failed: invalid action log event schema: eventType=step.completed status=started is not allowed",
            ),
            (
                action_log_event_schema_test_event(
                    "plan.completed",
                    "completed",
                    "freeform.reason",
                ),
                "buddy state validation failed: invalid action log event schema: eventType=plan.completed reasonCode=freeform.reason is not allowed",
            ),
        ];

        for (event, expected_error) in cases {
            let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
            let error = storage
                .append_choreography_action_log_event(&event)
                .expect_err("invalid action log event schema should be rejected");

            assert_eq!(error.to_string(), expected_error);
            assert!(!storage.action_log_jsonl_path().exists());
        }
    }

    #[test]
    fn append_rejects_duplicate_event_id_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let event =
            action_log_event_schema_test_event("plan.started", "started", "devFixture.started");

        storage
            .append_choreography_action_log_event(&event)
            .expect("append first event");
        let error = storage
            .append_choreography_action_log_event(&event)
            .expect_err("duplicate event id should be rejected before JSONL append");

        assert_eq!(
            error.to_string(),
            format!(
                "buddy state validation failed: duplicate action log eventId={}",
                event.event_id
            )
        );
        assert_eq!(storage.read_action_log_jsonl_lines_for_test().len(), 1);
    }

    #[test]
    fn append_rejects_event_id_already_present_in_unindexed_jsonl() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let event =
            action_log_event_schema_test_event("plan.started", "started", "devFixture.started");

        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &event)
            .expect("append unindexed JSONL event");
        let error = storage
            .append_choreography_action_log_event(&event)
            .expect_err("duplicate JSONL event id should be rejected even when SQLite is stale");

        assert_eq!(
            error.to_string(),
            format!(
                "buddy state validation failed: duplicate action log eventId={}",
                event.event_id
            )
        );
        assert_eq!(storage.read_action_log_jsonl_lines_for_test().len(), 1);
    }

    #[test]
    fn append_rejects_payload_detail_fields_outside_diagnostic_schema_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event =
            action_log_event_schema_test_event("plan.failed", "failed", "devFixture.failed");
        event.payload = serde_json::json!({
            "detail": {
                "message": "runtime rejected action",
                "prompt": "raw prompt must not be logged",
            },
        });

        let error = storage
            .append_choreography_action_log_event(&event)
            .expect_err("payload.detail fields outside DiagnosticDetail should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log payload detail: field=prompt is not allowed"
        );
        assert!(!storage.action_log_jsonl_path().exists());
    }

    #[test]
    fn append_rejects_nested_payload_detail_items_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event =
            action_log_event_schema_test_event("plan.failed", "failed", "devFixture.failed");
        event.payload = serde_json::json!({
            "detail": {
                "message": "runtime rejected action",
                "items": [
                    "sidecar timeout",
                    {
                        "key": "nativeError",
                        "value": {
                            "message": "raw nested error",
                        },
                    },
                ],
            },
        });

        let error = storage
            .append_choreography_action_log_event(&event)
            .expect_err("payload.detail.items must stay bounded");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log payload detail: items[1].value must be a string"
        );
        assert!(!storage.action_log_jsonl_path().exists());
    }

    #[test]
    fn append_accepts_payload_detail_diagnostic_schema_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event =
            action_log_event_schema_test_event("plan.failed", "failed", "devFixture.failed");
        event.payload = serde_json::json!({
            "detail": {
                "message": "runtime rejected action",
                "rawCode": "sidecar.timeout",
                "source": "sidecar",
                "items": [
                    "step timed out",
                    {
                        "key": "phase",
                        "value": "executeStep",
                    },
                ],
                "truncated": true,
            },
        });

        storage
            .append_choreography_action_log_event(&event)
            .expect("DiagnosticDetail shape should be accepted");

        assert_eq!(storage.read_action_log_jsonl_lines_for_test().len(), 1);
    }

    #[test]
    fn append_sanitizes_payload_detail_sensitive_strings_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event =
            action_log_event_schema_test_event("plan.failed", "failed", "devFixture.failed");
        event.payload = serde_json::json!({
            "detail": {
                "message": "auth failed: OPENAI_API_KEY=sk-detail-secret-1234567890 Authorization: Bearer live-token at /home/user/.lexora/buddy/action-log/events.jsonl",
                "source": "sidecar",
                "items": [
                    "state path /home/user/project/private.txt",
                    {
                        "key": "env",
                        "value": "HF_TOKEN=hf-secret-1234567890abcdef",
                    },
                ],
            },
        });

        storage
            .append_choreography_action_log_event(&event)
            .expect("append sanitized detail");

        let payload = read_single_action_log_payload_for_test(&storage);
        let detail = payload.get("detail").expect("detail");
        let serialized = detail.to_string();
        assert!(serialized.contains("OPENAI_API_KEY=[redacted]"));
        assert!(serialized.contains("Authorization: Bearer [redacted]"));
        assert!(serialized.contains("[path]"));
        assert_eq!(detail.get("truncated"), Some(&serde_json::json!(true)));
        assert!(!serialized.contains("sk-detail-secret"));
        assert!(!serialized.contains("live-token"));
        assert!(!serialized.contains("hf-secret"));
        assert!(!serialized.contains("/home/user"));
    }

    #[test]
    fn append_truncates_payload_detail_fields_and_total_size_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event =
            action_log_event_schema_test_event("plan.failed", "failed", "devFixture.failed");
        event.payload = serde_json::json!({
            "detail": {
                "message": "x".repeat(800),
                "items": (0..60)
                    .map(|index| format!("diagnostic-item-{index}-{}", "y".repeat(80)))
                    .collect::<Vec<_>>(),
            },
        });

        storage
            .append_choreography_action_log_event(&event)
            .expect("append truncated detail");

        let payload = read_single_action_log_payload_for_test(&storage);
        let detail = payload.get("detail").expect("detail");
        let message_len = detail
            .get("message")
            .and_then(serde_json::Value::as_str)
            .expect("message")
            .chars()
            .count();
        let detail_bytes = serde_json::to_vec(detail).expect("serialize detail").len();
        assert!(message_len <= 512);
        assert!(detail_bytes <= 2048);
        assert_eq!(detail.get("truncated"), Some(&serde_json::json!(true)));
    }

    fn action_log_event_schema_test_event(
        event_type: &str,
        status: &str,
        reason_code: &str,
    ) -> ActionLogEvent {
        ActionLogEvent {
            event_id: format!("evt_event_schema_{event_type}_{status}_{reason_code}"),
            schema_version: 1,
            event_type: event_type.to_owned(),
            status: status.to_owned(),
            reason_code: reason_code.to_owned(),
            plan_id: "plan_event_schema".to_owned(),
            step_id: None,
            source_ref: serde_json::json!({
                "kind": "devFixture",
                "fixtureName": "event-schema",
            }),
            trigger_source: "devFixture".to_owned(),
            payload: serde_json::json!({}),
            created_at: "2026-07-09T10:00:00.000Z".to_owned(),
        }
    }

    fn read_single_action_log_payload_for_test(storage: &BuddyStorage) -> serde_json::Value {
        let lines = storage.read_action_log_jsonl_lines_for_test();
        assert_eq!(lines.len(), 1);
        serde_json::from_str::<serde_json::Value>(&lines[0])
            .expect("parse action log JSONL event")
            .get("payload")
            .expect("payload")
            .clone()
    }
}

#[cfg(test)]
mod action_log_system_event_schema_tests {
    use super::*;

    #[test]
    fn append_rejects_system_payload_detail_fields_outside_diagnostic_schema_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event = action_log_system_event_schema_test_event();
        event.payload = serde_json::json!({
            "detail": {
                "message": "affective state file is invalid",
                "rawJson": {
                    "state": "raw nested payload must not be logged",
                },
            },
        });

        let error = storage
            .append_choreography_action_log_system_event(&event)
            .expect_err("system event payload.detail should use DiagnosticDetail");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log payload detail: field=rawJson is not allowed"
        );
        assert!(!storage.action_log_jsonl_path().exists());
    }

    #[test]
    fn append_sanitizes_system_payload_detail_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event = action_log_system_event_schema_test_event();
        event.payload = serde_json::json!({
            "detail": {
                "message": "invalid state file /home/user/.lexora/buddy/pet-affective-state.json with access_token=state-secret-1234567890",
                "source": "affectiveContext",
            },
        });

        storage
            .append_choreography_action_log_system_event(&event)
            .expect("append sanitized system detail");

        let lines = storage.read_action_log_jsonl_lines_for_test();
        assert_eq!(lines.len(), 1);
        let event: serde_json::Value = serde_json::from_str(&lines[0]).expect("parse system event");
        let detail = event
            .get("payload")
            .and_then(|payload| payload.get("detail"))
            .expect("detail");
        let serialized = detail.to_string();
        assert!(serialized.contains("access_token=[redacted]"));
        assert!(serialized.contains("[path]"));
        assert_eq!(detail.get("truncated"), Some(&serde_json::json!(true)));
        assert!(!serialized.contains("state-secret"));
        assert!(!serialized.contains("/home/user"));
    }

    fn action_log_system_event_schema_test_event() -> ActionLogSystemEvent {
        ActionLogSystemEvent {
            event_id: "evt_system_event_schema_invalid".to_owned(),
            schema_version: 1,
            event_type: "affectiveContext.invalidStateFile".to_owned(),
            status: "degraded".to_owned(),
            reason_code: "affectiveContext.invalidStateFile".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({
                "kind": "affectiveContext",
                "stateFileName": "pet-affective-state.json",
            }),
            trigger_source: "affectiveContext".to_owned(),
            payload: serde_json::json!({}),
            created_at: "2026-07-09T10:00:00.000Z".to_owned(),
        }
    }
}

#[cfg(test)]
mod action_log_source_ref_schema_tests {
    use super::*;

    #[test]
    fn append_rejects_cross_variant_source_ref_fields_before_jsonl_write() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let event = ActionLogEvent {
            event_id: "evt_source_ref_schema_invalid".to_owned(),
            schema_version: 1,
            event_type: "plan.started".to_owned(),
            status: "started".to_owned(),
            reason_code: "run.hostAction.started".to_owned(),
            plan_id: "plan_source_ref_schema_invalid".to_owned(),
            step_id: None,
            source_ref: serde_json::json!({
                "kind": "run",
                "runId": "run_source_ref_schema",
                "messageId": "msg_should_not_be_here",
            }),
            trigger_source: "run".to_owned(),
            payload: serde_json::json!({}),
            created_at: "2026-07-09T10:00:00.000Z".to_owned(),
        };

        let error = storage
            .append_choreography_action_log_event(&event)
            .expect_err("cross-variant sourceRef should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log sourceRef: kind=run field=messageId is not allowed"
        );
        assert!(!storage.action_log_jsonl_path().exists());
    }

    #[test]
    fn append_rejects_invalid_system_source_refs_before_jsonl_write() {
        let cases = [
            (
                "extra field",
                serde_json::json!({
                    "kind": "affectiveContext",
                    "stateFileName": "pet-affective-state.json",
                    "content": "invalid JSON body must not ride along",
                }),
                "buddy state validation failed: invalid action log system sourceRef: kind=affectiveContext field=content is not allowed",
            ),
            (
                "ordinary action source",
                serde_json::json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_system_source_ref",
                    "messageId": "message_system_source_ref",
                }),
                "buddy state validation failed: invalid action log system sourceRef: kind=conversationMessage is not supported",
            ),
        ];

        for (label, source_ref, expected_error) in cases {
            let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
            let event = action_log_system_source_ref_schema_test_event(source_ref);

            let error = storage
                .append_choreography_action_log_system_event(&event)
                .expect_err("invalid system sourceRef should be rejected");

            assert_eq!(error.to_string(), expected_error, "case: {label}");
            assert!(
                !storage.action_log_jsonl_path().exists(),
                "invalid {label} should not create JSONL"
            );
        }
    }

    fn action_log_system_source_ref_schema_test_event(
        source_ref: serde_json::Value,
    ) -> ActionLogSystemEvent {
        ActionLogSystemEvent {
            event_id: "evt_system_source_ref_schema_invalid".to_owned(),
            schema_version: 1,
            event_type: "affectiveContext.invalidStateFile".to_owned(),
            status: "degraded".to_owned(),
            reason_code: "affectiveContext.invalidStateFile".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref,
            trigger_source: "affectiveContext".to_owned(),
            payload: serde_json::json!({}),
            created_at: "2026-07-09T10:00:00.000Z".to_owned(),
        }
    }
}

#[cfg(test)]
mod action_log_index_watermark_tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use super::*;

    #[test]
    fn append_repairs_a_failed_projection_before_advancing_the_watermark() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let first_event = action_log_index_watermark_test_event(
            "evt_projection_gap_started",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let second_event = action_log_index_watermark_test_event(
            "evt_projection_gap_completed",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );
        storage
            .with_connection("install_action_log_projection_failure", |connection| {
                connection.execute_batch(
                    r#"
                    CREATE TRIGGER fail_action_log_projection
                    BEFORE INSERT ON action_log_events
                    WHEN NEW.event_id = 'evt_projection_gap_started'
                    BEGIN
                      SELECT RAISE(ABORT, 'forced action log projection failure');
                    END;
                    "#,
                )?;
                Ok(())
            })
            .expect("install projection failure trigger");

        let error = storage
            .append_choreography_action_log_event(&first_event)
            .expect_err("projection failure must be visible to the caller");
        assert!(error
            .to_string()
            .contains("forced action log projection failure"));

        storage
            .with_connection("remove_action_log_projection_failure", |connection| {
                connection.execute_batch("DROP TRIGGER fail_action_log_projection")?;
                Ok(())
            })
            .expect("remove projection failure trigger");
        storage
            .append_choreography_action_log_event(&second_event)
            .expect("repair projection gap before appending the next event");

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list repaired action log plans");
        assert!(!list.index_stale);
        assert_eq!(list.items[0].status, "completed");
        assert_eq!(
            storage.action_log_event_types_for_test("plan_index_watermark"),
            vec!["plan.started", "plan.completed"]
        );
    }

    #[test]
    fn concurrent_appends_keep_jsonl_and_sqlite_projection_contiguous() {
        const EVENT_COUNT: usize = 64;

        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let barrier = Arc::new(Barrier::new(EVENT_COUNT));
        let handles = (0..EVENT_COUNT)
            .map(|index| {
                let storage = storage.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let event = action_log_index_watermark_test_event(
                        format!("evt_concurrent_append_{index}").as_str(),
                        "plan.started",
                        "started",
                        "devFixture.started",
                        "2026-07-09T10:00:00.000Z",
                    );
                    barrier.wait();
                    storage.append_choreography_action_log_event(&event)
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle
                .join()
                .expect("join action log append thread")
                .expect("append concurrent action log event");
        }

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list action log plans after concurrent appends");
        assert!(!list.index_stale);
        assert_eq!(
            storage.read_action_log_jsonl_lines_for_test().len(),
            EVENT_COUNT
        );
        assert_eq!(
            storage
                .action_log_event_types_for_test("plan_index_watermark")
                .len(),
            EVENT_COUNT
        );
    }

    #[test]
    fn independent_storage_instances_serialize_action_log_appends() {
        const EVENT_COUNT: usize = 32;

        let buddy_home = std::env::temp_dir().join(format!(
            "lexora-buddy-independent-action-log-writers-{}",
            uuid::Uuid::new_v4()
        ));
        let database_path = buddy_home.join("sqlite/state.sqlite3");
        let storage = BuddyStorage::new_with_buddy_home(database_path.clone(), buddy_home.clone());
        storage.initialize().expect("initialize storage");
        let barrier = Arc::new(Barrier::new(EVENT_COUNT));
        let handles = (0..EVENT_COUNT)
            .map(|index| {
                let database_path = database_path.clone();
                let buddy_home = buddy_home.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let storage = BuddyStorage::new_with_buddy_home(database_path, buddy_home);
                    let event = action_log_index_watermark_test_event(
                        format!("evt_independent_append_{index}").as_str(),
                        "plan.started",
                        "started",
                        "devFixture.started",
                        "2026-07-09T10:00:00.000Z",
                    );
                    barrier.wait();
                    storage.append_choreography_action_log_event(&event)
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle
                .join()
                .expect("join independent action log writer")
                .expect("append through independent storage");
        }

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list independently appended action log plans");
        assert!(!list.index_stale);
        assert_eq!(
            storage.read_action_log_jsonl_lines_for_test().len(),
            EVENT_COUNT
        );
        assert_eq!(
            storage
                .action_log_event_types_for_test("plan_index_watermark")
                .len(),
            EVENT_COUNT
        );

        let _ = std::fs::remove_dir_all(buddy_home);
    }

    #[test]
    fn independent_storage_instances_keep_pending_body_cache_in_jsonl_order() {
        const EVENT_COUNT: usize = 16;

        let buddy_home = std::env::temp_dir().join(format!(
            "lexora-buddy-independent-pending-body-writers-{}",
            uuid::Uuid::new_v4()
        ));
        let database_path = buddy_home.join("sqlite/state.sqlite3");
        let storage = BuddyStorage::new_with_buddy_home(database_path.clone(), buddy_home.clone());
        storage.initialize().expect("initialize storage");
        let plan_id = "plan_independent_pending_body";
        let barrier = Arc::new(Barrier::new(EVENT_COUNT));
        let handles = (0..EVENT_COUNT)
            .map(|index| {
                let database_path = database_path.clone();
                let buddy_home = buddy_home.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let storage = BuddyStorage::new_with_buddy_home(database_path, buddy_home);
                    barrier.wait();
                    storage.upsert_choreography_pending_execution_body(
                        UpsertChoreographyPendingExecutionBodyRequest {
                            plan_id: plan_id.to_owned(),
                            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                            schema_version: 1,
                            body: serde_json::json!({ "writer": index }),
                        },
                    )
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle
                .join()
                .expect("join independent pending body writer")
                .expect("store pending body through independent storage");
        }

        let last_jsonl_body = storage
            .read_action_log_jsonl_lines_for_test()
            .last()
            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .and_then(|event| event.pointer("/payload/body").cloned())
            .expect("last pending body fact");
        let cached_body = storage
            .find_choreography_pending_execution_body(plan_id)
            .expect("find pending body cache")
            .expect("pending body cache exists");

        assert_eq!(cached_body.body, last_jsonl_body);

        let _ = std::fs::remove_dir_all(buddy_home);
    }

    #[test]
    fn append_updates_index_watermark_to_last_indexed_jsonl_event() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let first_event = action_log_index_watermark_test_event(
            "evt_index_watermark_started",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let second_event = action_log_index_watermark_test_event(
            "evt_index_watermark_completed",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );

        storage
            .append_choreography_action_log_event(&first_event)
            .expect("append first action log event");
        storage
            .append_choreography_action_log_event(&second_event)
            .expect("append second action log event");

        let jsonl_lines = storage.read_action_log_jsonl_lines_for_test();
        let watermark = read_action_log_index_watermark_for_test(&storage);

        assert_eq!(
            watermark,
            serde_json::json!({
                "sourceFile": ACTION_LOG_JSONL_RELATIVE_PATH,
                "byteOffset": jsonl_lines[0].len() as i64 + 1,
                "lineNumber": 2,
                "eventId": second_event.event_id,
                "updatedAt": second_event.created_at,
            })
        );
    }

    #[test]
    fn queries_report_stale_index_state_when_jsonl_has_unindexed_events() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let indexed_event = action_log_index_watermark_test_event(
            "evt_index_state_started",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let unindexed_event = action_log_index_watermark_test_event(
            "evt_index_state_completed",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );

        storage
            .append_choreography_action_log_event(&indexed_event)
            .expect("append indexed action log event");
        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &unindexed_event)
            .expect("append unindexed JSONL event");

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list action log plans");
        let detail = storage
            .get_action_log_plan_detail("plan_index_watermark")
            .expect("get action log plan detail");
        let system_events = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest::default())
            .expect("query action log system events");

        assert_action_log_index_state_is_stale(serde_json::to_value(list).expect("serialize list"));
        assert_action_log_index_state_is_stale(
            serde_json::to_value(detail).expect("serialize detail"),
        );
        assert_action_log_index_state_is_stale(
            serde_json::to_value(system_events).expect("serialize system events"),
        );
    }

    #[test]
    fn sync_replays_unindexed_jsonl_events_into_sqlite_projection() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let indexed_event = action_log_index_watermark_test_event(
            "evt_index_sync_started",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let unindexed_event = action_log_index_watermark_test_event(
            "evt_index_sync_completed",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );

        storage
            .append_choreography_action_log_event(&indexed_event)
            .expect("append indexed action log event");
        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &unindexed_event)
            .expect("append unindexed JSONL event");

        let stale_list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list stale action log plans");
        assert_action_log_index_state_is_stale(
            serde_json::to_value(stale_list).expect("serialize stale list"),
        );

        storage
            .sync_choreography_action_log_index()
            .expect("sync action log index");

        let synced_list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list synced action log plans");
        assert!(!synced_list.index_stale);
        assert_eq!(synced_list.index_status, ACTION_LOG_INDEX_STATUS_FRESH);
        assert_eq!(synced_list.items.len(), 1);
        assert_eq!(synced_list.items[0].status, "completed");
        assert_eq!(synced_list.items[0].last_event_type, "plan.completed");

        let watermark = read_action_log_index_watermark_for_test(&storage);
        assert_eq!(watermark["eventId"], unindexed_event.event_id);
        assert_eq!(watermark["lineNumber"], serde_json::json!(2));
    }

    #[test]
    fn sync_rebuilds_sqlite_projection_when_watermark_is_missing() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let started_event = action_log_index_watermark_test_event(
            "evt_index_rebuild_started",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let completed_event = action_log_index_watermark_test_event(
            "evt_index_rebuild_completed",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );

        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &started_event)
            .expect("append started JSONL event");
        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &completed_event)
            .expect("append completed JSONL event");

        storage
            .sync_choreography_action_log_index()
            .expect("rebuild action log index");

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list rebuilt action log plans");
        assert!(!list.index_stale);
        assert_eq!(list.index_status, ACTION_LOG_INDEX_STATUS_FRESH);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].status, "completed");
        assert_eq!(list.items[0].last_event_type, "plan.completed");

        let watermark = read_action_log_index_watermark_for_test(&storage);
        assert_eq!(watermark["eventId"], completed_event.event_id);
        assert_eq!(watermark["lineNumber"], serde_json::json!(2));
    }

    #[test]
    fn sync_rebuilds_system_event_projection_from_jsonl() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let system_event = ActionLogSystemEvent {
            event_id: "evt_index_rebuild_system".to_owned(),
            schema_version: 1,
            event_type: "affectiveContext.invalidStateFile".to_owned(),
            status: "degraded".to_owned(),
            reason_code: "affectiveContext.invalidStateFile".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({
                "kind": "affectiveContext",
                "stateFileName": "pet-affective-state.json",
            }),
            trigger_source: "affectiveContext".to_owned(),
            payload: serde_json::json!({}),
            created_at: "2026-07-09T10:00:00.000Z".to_owned(),
        };

        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &system_event)
            .expect("append system JSONL event");

        storage
            .sync_choreography_action_log_index()
            .expect("rebuild action log system index");

        let result = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest::default())
            .expect("query rebuilt system action log events");
        assert!(!result.index_stale);
        assert_eq!(result.index_status, ACTION_LOG_INDEX_STATUS_FRESH);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].event_id, system_event.event_id);
        assert_eq!(result.items[0].event_type, system_event.event_type);
        assert_eq!(result.items[0].source_ref.kind, "affectiveContext");

        let watermark = read_action_log_index_watermark_for_test(&storage);
        assert_eq!(watermark["eventId"], system_event.event_id);
        assert_eq!(watermark["lineNumber"], serde_json::json!(1));
    }

    #[test]
    fn sync_rejects_jsonl_events_with_unknown_schema_version() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event = action_log_index_watermark_test_event(
            "evt_index_rebuild_schema_v2",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        event.schema_version = 2;

        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &event)
            .expect("append unsupported JSONL event");

        let error = storage
            .sync_choreography_action_log_index()
            .expect_err("unsupported replay schema version should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log event schema: schemaVersion=2 is not supported"
        );
        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list action log plans after failed sync");
        assert!(list.index_stale);
        assert_eq!(list.index_status, ACTION_LOG_INDEX_STATUS_FAILED);
        assert_eq!(list.last_indexed_at, None);
    }

    #[test]
    fn queries_report_failed_index_state_when_jsonl_has_unknown_schema_version() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut event = action_log_index_watermark_test_event(
            "evt_index_state_schema_v2",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        event.schema_version = 2;

        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &event)
            .expect("append unsupported JSONL event");

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list action log plans");

        assert!(list.index_stale);
        assert_eq!(list.index_status, ACTION_LOG_INDEX_STATUS_FAILED);
    }

    #[test]
    fn sync_rejects_duplicate_jsonl_event_ids_without_projecting_last_writer_wins() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let started_event = action_log_index_watermark_test_event(
            "evt_index_rebuild_duplicate",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let completed_event = action_log_index_watermark_test_event(
            "evt_index_rebuild_duplicate",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );

        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &started_event)
            .expect("append started JSONL event");
        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &completed_event)
            .expect("append duplicate JSONL event");

        let error = storage
            .sync_choreography_action_log_index()
            .expect_err("duplicate replay event id should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: duplicate action log eventId=evt_index_rebuild_duplicate"
        );
        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list action log plans after failed sync");
        assert!(list.items.is_empty());
        assert!(list.index_stale);
        assert_eq!(list.index_status, ACTION_LOG_INDEX_STATUS_FAILED);
    }

    #[test]
    fn sync_rejects_incremental_jsonl_event_id_duplicate_of_indexed_event() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let indexed_event = action_log_index_watermark_test_event(
            "evt_index_incremental_duplicate",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let duplicate_event = action_log_index_watermark_test_event(
            "evt_index_incremental_duplicate",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );

        storage
            .append_choreography_action_log_event(&indexed_event)
            .expect("append indexed event");
        append_action_log_jsonl_line(storage.action_log_jsonl_path(), &duplicate_event)
            .expect("append duplicate unindexed JSONL event");

        let error = storage
            .sync_choreography_action_log_index()
            .expect_err("incremental duplicate replay event id should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: duplicate action log eventId=evt_index_incremental_duplicate"
        );
        let detail = storage
            .get_action_log_plan_detail("plan_index_watermark")
            .expect("get original detail after failed sync");
        assert_eq!(detail.plan.status, "running");
        assert_eq!(detail.plan.last_event_type, "plan.started");
        assert!(detail.index_stale);
    }

    #[test]
    fn queries_report_failed_index_state_when_duplicate_jsonl_event_id_matches_watermark() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let indexed_event = action_log_index_watermark_test_event(
            "evt_index_state_duplicate_watermark",
            "plan.started",
            "started",
            "devFixture.started",
            "2026-07-09T10:00:00.000Z",
        );
        let duplicate_event = action_log_index_watermark_test_event(
            "evt_index_state_duplicate_watermark",
            "plan.completed",
            "completed",
            "devFixture.completed",
            "2026-07-09T10:00:01.000Z",
        );

        storage
            .append_choreography_action_log_event(&indexed_event)
            .expect("append indexed event");
        let duplicate_cursor =
            append_action_log_jsonl_line(storage.action_log_jsonl_path(), &duplicate_event)
                .expect("append duplicate unindexed JSONL event");
        storage
            .with_connection("force_duplicate_event_watermark_for_test", |connection| {
                upsert_action_log_index_watermark_record(
                    connection,
                    &duplicate_event.event_id,
                    &duplicate_event.created_at,
                    &duplicate_cursor,
                )
            })
            .expect("force duplicate event watermark");

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list action log plans");

        assert!(list.index_stale);
        assert_eq!(list.index_status, ACTION_LOG_INDEX_STATUS_FAILED);
    }

    fn action_log_index_watermark_test_event(
        event_id: &str,
        event_type: &str,
        status: &str,
        reason_code: &str,
        created_at: &str,
    ) -> ActionLogEvent {
        ActionLogEvent {
            event_id: event_id.to_owned(),
            schema_version: 1,
            event_type: event_type.to_owned(),
            status: status.to_owned(),
            reason_code: reason_code.to_owned(),
            plan_id: "plan_index_watermark".to_owned(),
            step_id: None,
            source_ref: serde_json::json!({
                "kind": "devFixture",
                "fixtureName": "index-watermark",
            }),
            trigger_source: "devFixture".to_owned(),
            payload: serde_json::json!({}),
            created_at: created_at.to_owned(),
        }
    }

    fn read_action_log_index_watermark_for_test(storage: &BuddyStorage) -> serde_json::Value {
        storage
            .with_connection("read_action_log_index_watermark_for_test", |connection| {
                connection
                    .query_row(
                        r#"
                        SELECT source_file, byte_offset, line_number, event_id, updated_at
                        FROM action_log_index_watermarks
                        WHERE source_file = ?1
                        "#,
                        params![ACTION_LOG_JSONL_RELATIVE_PATH],
                        |row| {
                            Ok(serde_json::json!({
                                "sourceFile": row.get::<_, String>(0)?,
                                "byteOffset": row.get::<_, i64>(1)?,
                                "lineNumber": row.get::<_, i64>(2)?,
                                "eventId": row.get::<_, String>(3)?,
                                "updatedAt": row.get::<_, String>(4)?,
                            }))
                        },
                    )
                    .map_err(Into::into)
            })
            .expect("read action log index watermark")
    }

    fn assert_action_log_index_state_is_stale(value: serde_json::Value) {
        assert_eq!(
            (
                value.get("indexStatus"),
                value.get("indexStale"),
                value.get("lastIndexedAt"),
            ),
            (
                Some(&serde_json::json!("stale")),
                Some(&serde_json::json!(true)),
                Some(&serde_json::json!("2026-07-09T10:00:00.000Z")),
            )
        );
    }
}

#[cfg(test)]
mod list_action_log_plans_tests {
    use super::*;

    #[test]
    fn filters_by_plan_summary_projection_whitelist() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        insert_action_log_plan_summary_row_for_filter_test(
            &storage,
            ActionLogPlanSummaryRowForFilterTest {
                plan_id: "plan_filter_match",
                source_ref_kind: "run",
                status: "completed",
                started_at: "2026-07-08T10:01:00.000Z",
                completed_at: "2026-07-08T10:01:01.720Z",
                last_event_type: "plan.completed",
                last_reason_code: "run.hostAction.completed",
                resolved_action_id: "celebrate",
                resolved_animation_ref: "celebrate",
                trigger_source: "run",
            },
        );
        insert_action_log_plan_summary_row_for_filter_test(
            &storage,
            ActionLogPlanSummaryRowForFilterTest {
                plan_id: "plan_filter_other_reason",
                source_ref_kind: "run",
                status: "completed",
                started_at: "2026-07-08T10:02:00.000Z",
                completed_at: "2026-07-08T10:02:01.720Z",
                last_event_type: "plan.completed",
                last_reason_code: "devFixture.completed",
                resolved_action_id: "celebrate",
                resolved_animation_ref: "celebrate",
                trigger_source: "run",
            },
        );
        insert_action_log_plan_summary_row_for_filter_test(
            &storage,
            ActionLogPlanSummaryRowForFilterTest {
                plan_id: "plan_filter_other_action",
                source_ref_kind: "run",
                status: "completed",
                started_at: "2026-07-08T10:03:00.000Z",
                completed_at: "2026-07-08T10:03:01.720Z",
                last_event_type: "plan.completed",
                last_reason_code: "run.hostAction.completed",
                resolved_action_id: "sleep",
                resolved_animation_ref: "sleep",
                trigger_source: "run",
            },
        );
        insert_action_log_plan_summary_row_for_filter_test(
            &storage,
            ActionLogPlanSummaryRowForFilterTest {
                plan_id: "plan_filter_outside_time_range",
                source_ref_kind: "run",
                status: "completed",
                started_at: "2026-07-08T09:59:59.000Z",
                completed_at: "2026-07-08T10:00:00.720Z",
                last_event_type: "plan.completed",
                last_reason_code: "run.hostAction.completed",
                resolved_action_id: "celebrate",
                resolved_animation_ref: "celebrate",
                trigger_source: "run",
            },
        );
        insert_action_log_plan_summary_row_for_filter_test(
            &storage,
            ActionLogPlanSummaryRowForFilterTest {
                plan_id: "plan_filter_other_trigger_source",
                source_ref_kind: "run",
                status: "completed",
                started_at: "2026-07-08T10:01:30.000Z",
                completed_at: "2026-07-08T10:01:31.720Z",
                last_event_type: "plan.completed",
                last_reason_code: "run.hostAction.completed",
                resolved_action_id: "celebrate",
                resolved_animation_ref: "celebrate",
                trigger_source: "devFixture",
            },
        );

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                plan_id: None,
                page_cursor: None,
                last_event_type: Some("plan.completed".to_owned()),
                last_reason_code: Some("run.hostAction.completed".to_owned()),
                resolved_action_id: Some("celebrate".to_owned()),
                resolved_animation_ref: Some("celebrate".to_owned()),
                result_kind: None,
                source_ref_id: None,
                source_ref_kind: Some("run".to_owned()),
                started_at_from: Some("2026-07-08T10:00:00.000Z".to_owned()),
                started_at_to: Some("2026-07-08T10:02:00.000Z".to_owned()),
                status: Some("completed".to_owned()),
                trigger_source: Some("run".to_owned()),
            })
            .expect("list action log plans");

        assert_eq!(
            list.items
                .iter()
                .map(|plan| plan.plan_id.as_str())
                .collect::<Vec<_>>(),
            vec!["plan_filter_match"]
        );
    }

    #[test]
    fn filters_by_exact_plan_id() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        insert_action_log_plan_summary_row_for_filter_test(
            &storage,
            ActionLogPlanSummaryRowForFilterTest {
                plan_id: "plan_filter_exact_match",
                source_ref_kind: "run",
                status: "completed",
                started_at: "2026-07-08T10:01:00.000Z",
                completed_at: "2026-07-08T10:01:01.720Z",
                last_event_type: "plan.completed",
                last_reason_code: "run.hostAction.completed",
                resolved_action_id: "celebrate",
                resolved_animation_ref: "celebrate",
                trigger_source: "run",
            },
        );
        insert_action_log_plan_summary_row_for_filter_test(
            &storage,
            ActionLogPlanSummaryRowForFilterTest {
                plan_id: "plan_filter_exact_other",
                source_ref_kind: "run",
                status: "completed",
                started_at: "2026-07-08T10:02:00.000Z",
                completed_at: "2026-07-08T10:02:01.720Z",
                last_event_type: "plan.completed",
                last_reason_code: "run.hostAction.completed",
                resolved_action_id: "celebrate",
                resolved_animation_ref: "celebrate",
                trigger_source: "run",
            },
        );

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                page_cursor: None,
                plan_id: Some("plan_filter_exact_match".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans");

        assert_eq!(
            list.items
                .iter()
                .map(|plan| plan.plan_id.as_str())
                .collect::<Vec<_>>(),
            vec!["plan_filter_exact_match"]
        );
    }

    #[test]
    fn filters_by_projected_source_ref_id() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_action_log_plan_started_event_for_source_ref_filter(
            &storage,
            "plan_source_ref_id_match",
            serde_json::json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_source_ref_filter",
                "messageId": "message_source_ref_filter_match",
            }),
            "2026-07-08T10:01:00.000Z",
        );
        append_action_log_plan_started_event_for_source_ref_filter(
            &storage,
            "plan_source_ref_id_other",
            serde_json::json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_source_ref_filter",
                "messageId": "message_source_ref_filter_other",
            }),
            "2026-07-08T10:02:00.000Z",
        );

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                source_ref_id: Some("message_source_ref_filter_match".to_owned()),
                source_ref_kind: Some("conversationMessage".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans");

        assert_eq!(
            list.items
                .iter()
                .map(|plan| { (plan.plan_id.as_str(), plan.source_ref_id.as_deref(),) })
                .collect::<Vec<_>>(),
            vec![(
                "plan_source_ref_id_match",
                Some("message_source_ref_filter_match")
            )]
        );
    }

    #[test]
    fn filters_by_projected_result_kind() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_result_kind_fallback",
            "plan.started",
            "started",
            "devFixture.started",
            serde_json::json!({}),
            "2026-07-08T10:01:00.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_result_kind_fallback",
            "fallback.registrySelected",
            "resolved",
            "fallback.registrySelected",
            serde_json::json!({}),
            "2026-07-08T10:01:01.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_result_kind_degraded",
            "plan.started",
            "started",
            "devFixture.started",
            serde_json::json!({}),
            "2026-07-08T10:02:00.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_result_kind_degraded",
            "step.completed",
            "completed",
            "devFixture.stepCompleted",
            serde_json::json!({ "resultKind": "degraded" }),
            "2026-07-08T10:02:01.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_result_kind_interrupted",
            "plan.started",
            "started",
            "devFixture.started",
            serde_json::json!({}),
            "2026-07-08T10:03:00.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_result_kind_interrupted",
            "step.interrupted",
            "interrupted",
            "sidecar.stepInterrupted",
            serde_json::json!({}),
            "2026-07-08T10:03:01.000Z",
        );

        let fallback = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                result_kind: Some("fallback".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list fallback plans");
        let degraded = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                result_kind: Some("degraded".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list degraded plans");
        let interrupted = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                result_kind: Some("interrupted".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list interrupted plans");

        assert_eq!(
            vec![
                result_kind_filter_summary(&fallback),
                result_kind_filter_summary(&degraded),
                result_kind_filter_summary(&interrupted),
            ],
            vec![
                vec![("plan_result_kind_fallback", "fallback")],
                vec![("plan_result_kind_degraded", "degraded")],
                vec![("plan_result_kind_interrupted", "interrupted")],
            ]
        );
    }

    #[test]
    fn keeps_detail_reason_when_later_terminal_event_updates_last_reason() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_detail_reason_fallback",
            "plan.started",
            "started",
            "devFixture.started",
            serde_json::json!({}),
            "2026-07-08T10:04:00.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_detail_reason_fallback",
            "fallback.registrySelected",
            "resolved",
            "fallback.registrySelected",
            serde_json::json!({}),
            "2026-07-08T10:04:01.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_detail_reason_fallback",
            "plan.completed",
            "completed",
            "devFixture.completed",
            serde_json::json!({}),
            "2026-07-08T10:04:02.000Z",
        );

        let plan = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                plan_id: Some("plan_detail_reason_fallback".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");

        assert_eq!(
            (
                plan.status.as_str(),
                plan.last_reason_code.as_str(),
                plan.result_kind.as_str(),
                plan.detail_status.as_str(),
                plan.detail_reason_code.as_str(),
            ),
            (
                "completed",
                "devFixture.completed",
                "fallback",
                "fallback",
                "fallback.registrySelected",
            )
        );
    }

    #[test]
    fn keeps_interrupted_detail_when_failed_event_arrives_later() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_detail_reason_interrupted",
            "plan.started",
            "started",
            "devFixture.started",
            serde_json::json!({}),
            "2026-07-08T10:05:00.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_detail_reason_interrupted",
            "step.interrupted",
            "interrupted",
            "sidecar.stepInterrupted",
            serde_json::json!({}),
            "2026-07-08T10:05:01.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_detail_reason_interrupted",
            "plan.failed",
            "failed",
            "devFixture.failed",
            serde_json::json!({}),
            "2026-07-08T10:05:02.000Z",
        );

        let plan = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                plan_id: Some("plan_detail_reason_interrupted".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");

        assert_eq!(
            (
                plan.status.as_str(),
                plan.last_reason_code.as_str(),
                plan.result_kind.as_str(),
                plan.detail_status.as_str(),
                plan.detail_reason_code.as_str(),
            ),
            (
                "failed",
                "devFixture.failed",
                "interrupted",
                "interrupted",
                "sidecar.stepInterrupted",
            )
        );
    }

    #[test]
    fn projects_plan_interrupted_as_terminal_interrupted_summary() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_yielded_to_pending",
            "plan.started",
            "started",
            "timeline.started",
            serde_json::json!({}),
            "2026-07-08T10:06:00.000Z",
        );
        append_action_log_event_for_result_kind_filter(
            &storage,
            "plan_yielded_to_pending",
            "plan.interrupted",
            "interrupted",
            "timeline.yieldedToPendingPlan",
            serde_json::json!({
                "status": "interrupted",
                "resultKind": "interrupted",
                "completedStepCount": 1
            }),
            "2026-07-08T10:06:01.000Z",
        );

        let plan = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                plan_id: Some("plan_yielded_to_pending".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");

        assert_eq!(
            (
                plan.status.as_str(),
                plan.last_event_type.as_str(),
                plan.last_reason_code.as_str(),
                plan.result_kind.as_str(),
                plan.detail_status.as_str(),
                plan.detail_reason_code.as_str(),
                plan.completed_at.as_deref(),
            ),
            (
                "interrupted",
                "plan.interrupted",
                "timeline.yieldedToPendingPlan",
                "interrupted",
                "interrupted",
                "timeline.yieldedToPendingPlan",
                Some("2026-07-08T10:06:01.000Z"),
            )
        );
    }

    struct ActionLogPlanSummaryRowForFilterTest<'a> {
        plan_id: &'a str,
        source_ref_kind: &'a str,
        status: &'a str,
        started_at: &'a str,
        completed_at: &'a str,
        last_event_type: &'a str,
        last_reason_code: &'a str,
        resolved_action_id: &'a str,
        resolved_animation_ref: &'a str,
        trigger_source: &'a str,
    }

    fn insert_action_log_plan_summary_row_for_filter_test(
        storage: &BuddyStorage,
        row: ActionLogPlanSummaryRowForFilterTest<'_>,
    ) {
        storage
            .with_connection(
                "insert_action_log_plan_summary_row_for_filter_test",
                |connection| {
                    let source_ref_json = serde_json::json!({
                        "kind": row.source_ref_kind,
                        "runId": format!("run_{}", row.plan_id),
                    })
                    .to_string();
                    connection.execute(
                        r#"
                        INSERT INTO action_log_plan_summaries(
                          plan_id,
                          source_ref_kind,
                          source_ref_json,
                          status,
                          started_at,
                          completed_at,
                          last_event_type,
                          last_reason_code,
                          resolved_action_id,
                          resolved_animation_ref
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                        "#,
                        params![
                            row.plan_id,
                            row.source_ref_kind,
                            source_ref_json,
                            row.status,
                            row.started_at,
                            row.completed_at,
                            row.last_event_type,
                            row.last_reason_code,
                            row.resolved_action_id,
                            row.resolved_animation_ref,
                        ],
                    )?;
                    connection.execute(
                        r#"
                        INSERT INTO action_log_events(
                          event_id,
                          schema_version,
                          event_type,
                          status,
                          reason_code,
                          trigger_source,
                          plan_id,
                          step_id,
                          source_ref_kind,
                          source_ref_json,
                          payload_json,
                          created_at
                        )
                        VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, '{}', ?9)
                        "#,
                        params![
                            format!("event_{}", row.plan_id),
                            row.last_event_type,
                            row.status,
                            row.last_reason_code,
                            row.trigger_source,
                            row.plan_id,
                            row.source_ref_kind,
                            serde_json::json!({
                                "kind": row.source_ref_kind,
                                "runId": format!("run_{}", row.plan_id),
                            })
                            .to_string(),
                            row.completed_at,
                        ],
                    )?;

                    Ok(())
                },
            )
            .expect("insert action log plan summary row");
    }

    fn append_action_log_plan_started_event_for_source_ref_filter(
        storage: &BuddyStorage,
        plan_id: &str,
        source_ref: serde_json::Value,
        created_at: &str,
    ) {
        storage
            .append_choreography_action_log_event(&ActionLogEvent {
                event_id: format!("evt_{plan_id}"),
                schema_version: 1,
                event_type: "plan.started".to_owned(),
                status: "started".to_owned(),
                reason_code: "devFixture.started".to_owned(),
                plan_id: plan_id.to_owned(),
                step_id: None,
                source_ref,
                trigger_source: "test".to_owned(),
                payload: serde_json::json!({}),
                created_at: created_at.to_owned(),
            })
            .expect("append action log plan started event");
    }

    fn append_action_log_event_for_result_kind_filter(
        storage: &BuddyStorage,
        plan_id: &str,
        event_type: &str,
        status: &str,
        reason_code: &str,
        payload: serde_json::Value,
        created_at: &str,
    ) {
        storage
            .append_choreography_action_log_event(&ActionLogEvent {
                event_id: format!("evt_{plan_id}_{event_type}"),
                schema_version: 1,
                event_type: event_type.to_owned(),
                status: status.to_owned(),
                reason_code: reason_code.to_owned(),
                plan_id: plan_id.to_owned(),
                step_id: Some(format!("step_{plan_id}")),
                source_ref: serde_json::json!({
                    "kind": "devFixture",
                    "fixtureName": "result-kind-filter",
                }),
                trigger_source: "test".to_owned(),
                payload,
                created_at: created_at.to_owned(),
            })
            .expect("append action log result kind event");
    }

    fn result_kind_filter_summary(plans: &ActionLogPlanList) -> Vec<(&str, &str)> {
        plans
            .items
            .iter()
            .map(|plan| (plan.plan_id.as_str(), plan.result_kind.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod query_action_log_system_events_tests {
    use super::*;

    #[test]
    fn returns_only_planless_system_events() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        insert_action_log_event_row_for_test(
            &storage,
            ActionLogEventRowForTest {
                event_id: "event_system_001",
                event_type: "startupHealth.failed",
                status: "failed",
                reason_code: "startupHealth.nativePetUnavailable",
                trigger_source: "startupHealth",
                plan_id: None,
                step_id: None,
                source_ref_kind: "runtime",
                created_at: "2026-07-08T12:00:00.000Z",
            },
        );
        insert_action_log_event_row_for_test(
            &storage,
            ActionLogEventRowForTest {
                event_id: "event_plan_001",
                event_type: "plan.failed",
                status: "failed",
                reason_code: "devFixture.failed",
                trigger_source: "devFixture",
                plan_id: Some("plan_001"),
                step_id: None,
                source_ref_kind: "devFixture",
                created_at: "2026-07-08T12:01:00.000Z",
            },
        );

        let result = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("startupHealth.failed".to_owned()),
                source_ref_kind: Some("runtime".to_owned()),
                reason_code: None,
                status: None,
                created_at_from: None,
                created_at_to: None,
                limit: Some(10),
                plan_id: None,
                step_id: None,
            })
            .expect("query system events");

        assert_eq!(
            serde_json::to_value(result).expect("serialize result"),
            serde_json::json!({
                "items": [
                    {
                        "eventId": "event_system_001",
                        "eventType": "startupHealth.failed",
                        "timestamp": "2026-07-08T12:00:00.000Z",
                        "sourceRef": {
                            "kind": "runtime"
                        },
                        "triggerSource": "startupHealth",
                        "status": "failed",
                        "reasonCode": "startupHealth.nativePetUnavailable",
                        "planId": null,
                        "stepId": null,
                        "indexStatus": "indexed"
                    }
                ],
                "limit": 10,
                "hasMore": false,
                "indexStale": true,
                "indexStatus": "stale",
                "lastIndexedAt": null
            })
        );
    }

    #[test]
    fn rejects_invalid_limit_without_writing_action_log_events() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        let error = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: None,
                source_ref_kind: None,
                reason_code: None,
                status: None,
                created_at_from: None,
                created_at_to: None,
                limit: Some(0),
                plan_id: None,
                step_id: None,
            })
            .expect_err("invalid limit should fail");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log system event query parameter: field=limit min=1 max=500 default=100"
        );
        assert_eq!(count_action_log_event_rows_for_test(&storage), 0);
    }

    #[test]
    fn rejects_plan_filter_for_system_event_query() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        let error = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: None,
                source_ref_kind: None,
                reason_code: None,
                status: None,
                created_at_from: None,
                created_at_to: None,
                limit: None,
                plan_id: Some("plan_001".to_owned()),
                step_id: None,
            })
            .expect_err("planId should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log system event query parameter: field=planId reason=must be omitted"
        );
    }

    #[test]
    fn rejects_ordinary_choreography_event_namespace() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        let error = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("step.completed".to_owned()),
                source_ref_kind: None,
                reason_code: None,
                status: None,
                created_at_from: None,
                created_at_to: None,
                limit: None,
                plan_id: None,
                step_id: None,
            })
            .expect_err("ordinary choreography event should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: invalid action log system event query parameter: field=eventType reason=ordinary choreography event namespaces are not supported"
        );
    }

    struct ActionLogEventRowForTest<'a> {
        event_id: &'a str,
        event_type: &'a str,
        status: &'a str,
        reason_code: &'a str,
        trigger_source: &'a str,
        plan_id: Option<&'a str>,
        step_id: Option<&'a str>,
        source_ref_kind: &'a str,
        created_at: &'a str,
    }

    fn insert_action_log_event_row_for_test(
        storage: &BuddyStorage,
        row: ActionLogEventRowForTest<'_>,
    ) {
        storage
            .with_connection("insert_action_log_event_row_for_test", |connection| {
                connection.execute(
                    r#"
                    INSERT INTO action_log_events(
                      event_id,
                      schema_version,
                      event_type,
                      status,
                      reason_code,
                      trigger_source,
                      plan_id,
                      step_id,
                      source_ref_kind,
                      source_ref_json,
                      payload_json,
                      created_at
                    )
                    VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', ?10)
                    "#,
                    params![
                        row.event_id,
                        row.event_type,
                        row.status,
                        row.reason_code,
                        row.trigger_source,
                        row.plan_id,
                        row.step_id,
                        row.source_ref_kind,
                        serde_json::json!({ "kind": row.source_ref_kind }).to_string(),
                        row.created_at,
                    ],
                )?;

                Ok(())
            })
            .expect("insert action log event row");
    }

    fn count_action_log_event_rows_for_test(storage: &BuddyStorage) -> i64 {
        storage
            .with_connection("count_action_log_event_rows_for_test", |connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM action_log_events", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(Into::into)
            })
            .expect("count action log event rows")
    }
}

#[cfg(test)]
mod action_log_source_display_tests {
    use super::*;
    use crate::storage::{
        AppendBuddyConversationMessageRequest, CreateBuddyConversationRequest,
        CreateBuddyConversationRunRequest,
    };

    #[test]
    fn list_action_log_plans_resolves_conversation_message_source_display_without_content() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let conversation = storage
            .create_conversation(CreateBuddyConversationRequest {
                forked_from_message_id: None,
                project_root: None,
                scope: "global".to_owned(),
                source_conversation_id: None,
                source_run_id: None,
                title: Some("动作来源对话".to_owned()),
            })
            .expect("create conversation");
        let message = storage
            .append_conversation_message(AppendBuddyConversationMessageRequest {
                attachments: Vec::new(),
                branch_id: conversation.active_branch_id.clone(),
                content: "用户提示正文不应进入动作日志来源展示".to_owned(),
                conversation_id: conversation.id.clone(),
                parent_message_id: None,
                role: "user".to_owned(),
                run_id: None,
                version_group_id: None,
                version_index: 1,
                version_status: "active".to_owned(),
            })
            .expect("append message");

        insert_action_log_plan_summary_row_for_test(
            &storage,
            "plan_source_display_001",
            serde_json::json!({
                "kind": "conversationMessage",
                "conversationId": conversation.id,
                "messageId": message.id,
            }),
        );

        let plan = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");
        let value = serde_json::to_value(plan).expect("serialize plan");

        assert_eq!(
            value["sourceDisplay"],
            serde_json::json!({
                "kind": "conversationMessage",
                "title": "动作来源对话",
                "subtitle": format!("user #1 · {}", &message.id[..8]),
                "missing": false,
            })
        );
        assert!(!value
            .to_string()
            .contains("用户提示正文不应进入动作日志来源展示"));
    }

    #[test]
    fn list_action_log_plans_resolves_run_source_display_with_triggering_message_locator() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let conversation = storage
            .create_conversation(CreateBuddyConversationRequest {
                forked_from_message_id: None,
                project_root: None,
                scope: "global".to_owned(),
                source_conversation_id: None,
                source_run_id: None,
                title: Some("Run 来源对话".to_owned()),
            })
            .expect("create conversation");
        let message = storage
            .append_conversation_message(AppendBuddyConversationMessageRequest {
                attachments: Vec::new(),
                branch_id: conversation.active_branch_id.clone(),
                content: "触发 run 的用户消息正文不应进入动作日志".to_owned(),
                conversation_id: conversation.id.clone(),
                parent_message_id: None,
                role: "user".to_owned(),
                run_id: None,
                version_group_id: None,
                version_index: 1,
                version_status: "active".to_owned(),
            })
            .expect("append message");
        let run = storage
            .create_conversation_run(CreateBuddyConversationRunRequest {
                branch_id: conversation.active_branch_id,
                conversation_id: conversation.id,
                cwd: Some("/tmp/lexora-project".to_owned()),
                external_run_id: None,
                external_thread_id: None,
                intent: "projectTask".to_owned(),
                runtime: "codex".to_owned(),
                triggering_message_id: message.id.clone(),
            })
            .expect("create conversation run");

        insert_action_log_plan_summary_row_for_test(
            &storage,
            "plan_run_source_display_001",
            serde_json::json!({
                "kind": "run",
                "runId": run.id,
            }),
        );

        let plan = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");
        let value = serde_json::to_value(plan).expect("serialize plan");

        assert_eq!(
            value["sourceDisplay"],
            serde_json::json!({
                "kind": "run",
                "title": "Run 来源对话",
                "subtitle": format!(
                    "codex · queued · user #1 · {} · {}",
                    &message.id[..8],
                    &run.id[..8],
                ),
                "missing": false,
            })
        );
        assert!(!value
            .to_string()
            .contains("触发 run 的用户消息正文不应进入动作日志"));
    }

    #[test]
    fn list_action_log_plans_resolves_preset_behavior_source_display_from_schema_id() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        insert_action_log_plan_summary_row_for_test(
            &storage,
            "plan_preset_behavior_source_display_001",
            serde_json::json!({
                "kind": "presetBehavior",
                "presetBehaviorId": "throw_after_drag",
                "interactionId": "interaction_drag_001",
            }),
        );

        let plan = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");
        let value = serde_json::to_value(plan).expect("serialize plan");

        assert_eq!(
            value["sourceDisplay"],
            serde_json::json!({
                "kind": "presetBehavior",
                "title": "throw_after_drag",
                "subtitle": null,
                "missing": false,
            })
        );
    }

    fn insert_action_log_plan_summary_row_for_test(
        storage: &BuddyStorage,
        plan_id: &str,
        source_ref: serde_json::Value,
    ) {
        let source_ref_kind = source_ref
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .expect("source ref kind");
        storage
            .with_connection(
                "insert_action_log_plan_summary_row_for_test",
                |connection| {
                    connection.execute(
                        r#"
                        INSERT INTO action_log_plan_summaries(
                          plan_id,
                          source_ref_kind,
                          source_ref_json,
                          status,
                          started_at,
                          completed_at,
                          last_event_type,
                          last_reason_code,
                          resolved_action_id,
                          resolved_animation_ref
                        )
                        VALUES (?1, ?2, ?3, 'completed', '2026-07-08T07:59:59.000Z', '2026-07-08T08:00:01.730Z', 'plan.completed', 'devFixture.completed', 'celebrate', 'celebrate')
                        "#,
                        params![plan_id, source_ref_kind, source_ref.to_string()],
                    )?;

                    Ok(())
                },
            )
            .expect("insert action log plan summary");
    }
}
