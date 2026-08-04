use serde::{Deserialize, Serialize};

use crate::{error::BuddyResult, storage::BuddyStorage};

use super::{
    admission::{ChoreographyAdmissionDecision, ChoreographyTriggerSource},
    affective::{AffectiveContext, ResolveContext},
    fixture::DevFixturePlan,
    recovery::RuntimeSafeFallbackPlan,
    registry::StepResolution,
    step_resolution::AfterActionResolution,
    timeline::{
        MoveByPathStep, MoveToStep, RestorePositionStep, SkipStep, SnapshotPositionStep,
        TimelinePlan, WaitStep,
    },
};

const ACTION_LOG_SCHEMA_VERSION: u16 = 1;
const ACTION_LOG_RESULT_KIND_FALLBACK: &str = "fallback";
const ACTION_LOG_TRIGGER_SOURCE_ACTION_LOG_INDEX: &str = "actionLogIndex";
const ACTION_LOG_TRIGGER_SOURCE_AFFECTIVE_CONTEXT: &str = "affectiveContext";
const ACTION_LOG_TRIGGER_SOURCE_CHOREOGRAPHY_SCHEDULER: &str = "choreographyScheduler";
const ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE: &str = "devFixture";
const ACTION_LOG_TRIGGER_SOURCE_HEALTH_GATE: &str = "healthGate";
const ACTION_LOG_TRIGGER_SOURCE_STARTUP_HEALTH: &str = "startupHealth";
const ACTION_LOG_TRIGGER_SOURCE_STARTUP_SYSTEM: &str = "startupSystem";
const ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY: &str = "systemRecovery";

pub(crate) struct ActionLogEventIds<'a> {
    pub(crate) event_id: &'a str,
    pub(crate) plan_id: &'a str,
    pub(crate) step_id: Option<&'a str>,
}

pub(crate) struct ActionLogRestorePositionResolution<'a> {
    pub(crate) step: &'a RestorePositionStep,
    pub(crate) move_to_step: &'a MoveToStep,
    pub(crate) after_action_resolution: Option<&'a AfterActionResolution>,
    pub(crate) resolve_context: &'a ResolveContext,
}

pub(crate) struct ActionLogRuntimeRestartStepInterruption<'a> {
    pub(crate) event_id: String,
    pub(crate) plan_id: &'a str,
    pub(crate) step_id: &'a str,
    pub(crate) source_ref: &'a serde_json::Value,
    pub(crate) previous_status: &'a str,
    pub(crate) previous_event_type: &'a str,
    pub(crate) previous_reason_code: &'a str,
    pub(crate) created_at: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActionLogTimelinePlanStats {
    pub(crate) completed_step_count: u64,
    pub(crate) failed_step_count: u64,
    pub(crate) skipped_step_count: u64,
    pub(crate) duration_ms: u64,
}

#[derive(Clone)]
pub(crate) struct ActionLogSink {
    storage: BuddyStorage,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionLogEvent {
    pub(crate) event_id: String,
    pub(crate) schema_version: u16,
    pub(crate) event_type: String,
    pub(crate) status: String,
    pub(crate) reason_code: String,
    pub(crate) plan_id: String,
    pub(crate) step_id: Option<String>,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) trigger_source: String,
    pub(crate) payload: serde_json::Value,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionLogSystemEvent {
    pub(crate) event_id: String,
    pub(crate) schema_version: u16,
    pub(crate) event_type: String,
    pub(crate) status: String,
    pub(crate) reason_code: String,
    pub(crate) plan_id: Option<String>,
    pub(crate) step_id: Option<String>,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) trigger_source: String,
    pub(crate) payload: serde_json::Value,
    pub(crate) created_at: String,
}

impl ActionLogSink {
    pub(crate) fn new(storage: BuddyStorage) -> Self {
        Self { storage }
    }

    pub(crate) fn append_event(&self, event: &ActionLogEvent) -> BuddyResult<()> {
        self.storage.append_choreography_action_log_event(event)
    }

    pub(crate) fn append_system_event(&self, event: &ActionLogSystemEvent) -> BuddyResult<()> {
        self.storage
            .append_choreography_action_log_system_event(event)
    }
}

impl ActionLogEvent {
    pub(crate) fn plan_started(
        event_id: impl Into<String>,
        plan: &DevFixturePlan,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.started".to_owned(),
            status: "started".to_owned(),
            reason_code: "devFixture.started".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "sourceRef": plan.source_ref.clone(),
                "fixtureName": plan.fixture_name(),
                "stepCount": plan.step_count(),
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        resolution: &StepResolution,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        let payload = play_action_resolved_payload(resolution, resolve_context);

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "devFixture.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    pub(crate) fn move_to_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveToStep,
        after_action_resolution: Option<&AfterActionResolution>,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        let mut payload = serde_json::json!({
            "stepKind": "moveTo",
            "target": &step.target,
            "timeoutMs": step.timeout_ms,
            "resolveContext": resolve_context,
        });
        apply_after_action_payload(
            &mut payload,
            step.after_action_id.as_deref(),
            after_action_resolution,
        );

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "devFixture.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    pub(crate) fn move_by_path_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveByPathStep,
        after_action_resolution: Option<&AfterActionResolution>,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        let mut payload = serde_json::json!({
            "stepKind": "moveByPath",
            "target": format!("path:{}", step.path.len()),
            "path": &step.path,
            "timeoutMs": step.timeout_ms,
            "resolveContext": resolve_context,
        });
        apply_after_action_payload(
            &mut payload,
            step.after_action_id.as_deref(),
            after_action_resolution,
        );
        if let Some(after_action_resolution) = after_action_resolution {
            payload["animationRef"] = serde_json::json!(after_action_resolution.animation_ref);
        }

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "devFixture.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    pub(crate) fn wait_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &WaitStep,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "devFixture.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "stepKind": "wait",
                "durationMs": step.duration_ms,
                "timeoutMs": step.timeout_ms,
                "resolveContext": resolve_context,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        resolution: &StepResolution,
        elapsed_ms: u64,
        created_at: impl Into<String>,
    ) -> Self {
        let payload = play_action_completed_payload(resolution, elapsed_ms);

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "devFixture.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    pub(crate) fn move_to_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveToStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "devFixture.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveTo",
                "target": &step.target,
                "timeoutMs": step.timeout_ms,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn move_by_path_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveByPathStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "devFixture.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveByPath",
                "target": format!("path:{}", step.path.len()),
                "path": &step.path,
                "timeoutMs": step.timeout_ms,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn wait_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &WaitStep,
        elapsed_ms: u64,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "devFixture.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "stepKind": "wait",
                "durationMs": step.duration_ms,
                "timeoutMs": step.timeout_ms,
                "elapsedMs": elapsed_ms,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        resolution: &StepResolution,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        let payload = play_action_failed_payload(resolution, error_message);

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "devFixture.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    pub(crate) fn move_to_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveToStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "devFixture.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveTo",
                "target": &step.target,
                "timeoutMs": step.timeout_ms,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn move_by_path_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveByPathStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "devFixture.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveByPath",
                "target": format!("path:{}", step.path.len()),
                "path": &step.path,
                "timeoutMs": step.timeout_ms,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn wait_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &WaitStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "devFixture.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "stepKind": "wait",
                "durationMs": step.duration_ms,
                "timeoutMs": step.timeout_ms,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn plan_completed(
        event_id: impl Into<String>,
        plan: &DevFixturePlan,
        completed_step_count: u64,
        duration_ms: u64,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "devFixture.completed".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "status": "completed",
                "completedStepCount": completed_step_count,
                "durationMs": duration_ms,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn plan_failed(
        event_id: impl Into<String>,
        plan: &DevFixturePlan,
        error_message: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "devFixture.failed".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "status": "failed",
                "error": error_message.into(),
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn plan_interrupted(
        event_id: impl Into<String>,
        plan: &DevFixturePlan,
        completed_step_count: u64,
        duration_ms: u64,
        yielded_after_step_id: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.interrupted".to_owned(),
            status: "interrupted".to_owned(),
            reason_code: "devFixture.yieldedToPendingPlan".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE.to_owned(),
            payload: serde_json::json!({
                "status": "interrupted",
                "resultKind": "interrupted",
                "completedStepCount": completed_step_count,
                "durationMs": duration_ms,
                "yieldedAfterStepId": yielded_after_step_id,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_plan_started(
        event_id: impl Into<String>,
        plan: &TimelinePlan,
        trigger_source: ChoreographyTriggerSource,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.started".to_owned(),
            status: "started".to_owned(),
            reason_code: "timeline.started".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "sourceRef": plan.source_ref.clone(),
                "failurePolicy": plan.failure_policy,
                "stepCount": plan.step_count(),
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        resolution: &StepResolution,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        let payload = play_action_resolved_payload(resolution, resolve_context);

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "timeline.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_move_to_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &MoveToStep,
        after_action_resolution: Option<&AfterActionResolution>,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        let mut payload = serde_json::json!({
            "stepKind": "moveTo",
            "target": &step.target,
            "timeoutMs": step.timeout_ms,
            "resolveContext": resolve_context,
        });
        apply_after_action_payload(
            &mut payload,
            step.after_action_id.as_deref(),
            after_action_resolution,
        );

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "timeline.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_move_by_path_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &MoveByPathStep,
        after_action_resolution: Option<&AfterActionResolution>,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        let mut payload = serde_json::json!({
            "stepKind": "moveByPath",
            "target": format!("path:{}", step.path.len()),
            "path": &step.path,
            "timeoutMs": step.timeout_ms,
            "resolveContext": resolve_context,
        });
        apply_after_action_payload(
            &mut payload,
            step.after_action_id.as_deref(),
            after_action_resolution,
        );
        if let Some(after_action_resolution) = after_action_resolution {
            payload["animationRef"] = serde_json::json!(after_action_resolution.animation_ref);
        }

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "timeline.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_wait_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &WaitStep,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "timeline.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "wait",
                "durationMs": step.duration_ms,
                "timeoutMs": step.timeout_ms,
                "resolveContext": resolve_context,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_snapshot_position_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &SnapshotPositionStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "timeline.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "snapshotPosition",
                "snapshotId": step.snapshot_id,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_restore_position_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        resolution: ActionLogRestorePositionResolution<'_>,
        created_at: impl Into<String>,
    ) -> Self {
        let mut payload = serde_json::json!({
            "stepKind": "restorePosition",
            "snapshotId": resolution.step.snapshot_id,
            "target": &resolution.move_to_step.target,
            "timeoutMs": resolution.step.timeout_ms,
            "resolveContext": resolution.resolve_context,
        });
        apply_after_action_payload(
            &mut payload,
            resolution.step.after_action_id.as_deref(),
            resolution.after_action_resolution,
        );

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "timeline.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        resolution: &StepResolution,
        elapsed_ms: u64,
        created_at: impl Into<String>,
    ) -> Self {
        let payload = play_action_completed_payload(resolution, elapsed_ms);

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "timeline.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_move_to_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &MoveToStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "timeline.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveTo",
                "target": &step.target,
                "timeoutMs": step.timeout_ms,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_move_by_path_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &MoveByPathStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "timeline.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveByPath",
                "target": format!("path:{}", step.path.len()),
                "path": &step.path,
                "timeoutMs": step.timeout_ms,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_wait_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &WaitStep,
        elapsed_ms: u64,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "timeline.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "wait",
                "durationMs": step.duration_ms,
                "timeoutMs": step.timeout_ms,
                "elapsedMs": elapsed_ms,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_skip_step_skipped(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &SkipStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.skipped".to_owned(),
            status: "skipped".to_owned(),
            reason_code: "timeline.stepSkipped".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "skipStep",
                "skipReason": step.reason,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_snapshot_position_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &SnapshotPositionStep,
        position: (i32, i32),
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "timeline.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "snapshotPosition",
                "snapshotId": step.snapshot_id,
                "position": { "x": position.0, "y": position.1 },
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_restore_position_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &RestorePositionStep,
        move_to_step: &MoveToStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "timeline.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "restorePosition",
                "snapshotId": step.snapshot_id,
                "target": &move_to_step.target,
                "timeoutMs": step.timeout_ms,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        resolution: &StepResolution,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        let payload = play_action_failed_payload(resolution, error_message);

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "timeline.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_move_to_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &MoveToStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "timeline.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveTo",
                "target": &step.target,
                "timeoutMs": step.timeout_ms,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_move_by_path_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &MoveByPathStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "timeline.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveByPath",
                "target": format!("path:{}", step.path.len()),
                "path": &step.path,
                "timeoutMs": step.timeout_ms,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_wait_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &WaitStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "timeline.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "wait",
                "durationMs": step.duration_ms,
                "timeoutMs": step.timeout_ms,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_snapshot_position_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &SnapshotPositionStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "timeline.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "snapshotPosition",
                "snapshotId": step.snapshot_id,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_restore_position_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        trigger_source: ChoreographyTriggerSource,
        step: &RestorePositionStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "timeline.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "stepKind": "restorePosition",
                "snapshotId": step.snapshot_id,
                "error": error_message,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_plan_completed(
        event_id: impl Into<String>,
        plan: &TimelinePlan,
        trigger_source: ChoreographyTriggerSource,
        stats: ActionLogTimelinePlanStats,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "timeline.completed".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "status": "completed",
                "failurePolicy": plan.failure_policy,
                "completedStepCount": stats.completed_step_count,
                "failedStepCount": stats.failed_step_count,
                "skippedStepCount": stats.skipped_step_count,
                "durationMs": stats.duration_ms,
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_plan_failed(
        event_id: impl Into<String>,
        plan: &TimelinePlan,
        trigger_source: ChoreographyTriggerSource,
        stats: ActionLogTimelinePlanStats,
        error_message: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "timeline.failed".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "status": "failed",
                "failurePolicy": plan.failure_policy,
                "completedStepCount": stats.completed_step_count,
                "failedStepCount": stats.failed_step_count,
                "skippedStepCount": stats.skipped_step_count,
                "durationMs": stats.duration_ms,
                "error": error_message.into(),
            }),
            created_at: created_at.into(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_plan_interrupted(
        event_id: impl Into<String>,
        plan: &TimelinePlan,
        trigger_source: ChoreographyTriggerSource,
        stats: ActionLogTimelinePlanStats,
        yielded_after_step_id: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.interrupted".to_owned(),
            status: "interrupted".to_owned(),
            reason_code: "timeline.yieldedToPendingPlan".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: trigger_source.action_log_value().to_owned(),
            payload: serde_json::json!({
                "status": "interrupted",
                "resultKind": "interrupted",
                "failurePolicy": plan.failure_policy,
                "completedStepCount": stats.completed_step_count,
                "failedStepCount": stats.failed_step_count,
                "skippedStepCount": stats.skipped_step_count,
                "durationMs": stats.duration_ms,
                "yieldedAfterStepId": yielded_after_step_id,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn plan_interrupted_after_runtime_restart(
        event_id: impl Into<String>,
        plan_id: impl Into<String>,
        source_ref: serde_json::Value,
        previous_status: impl Into<String>,
        previous_event_type: impl Into<String>,
        previous_reason_code: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.interrupted".to_owned(),
            status: "interrupted".to_owned(),
            reason_code: "runtime.restarted".to_owned(),
            plan_id: plan_id.into(),
            step_id: None,
            source_ref,
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_STARTUP_SYSTEM.to_owned(),
            payload: serde_json::json!({
                "status": "interrupted",
                "resultKind": "interrupted",
                "previousStatus": previous_status.into(),
                "previousEventType": previous_event_type.into(),
                "previousReasonCode": previous_reason_code.into(),
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn step_interrupted_after_runtime_restart(
        interruption: ActionLogRuntimeRestartStepInterruption<'_>,
    ) -> Self {
        Self {
            event_id: interruption.event_id,
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.interrupted".to_owned(),
            status: "interrupted".to_owned(),
            reason_code: "runtime.restarted".to_owned(),
            plan_id: interruption.plan_id.to_owned(),
            step_id: Some(interruption.step_id.to_owned()),
            source_ref: interruption.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_STARTUP_SYSTEM.to_owned(),
            payload: serde_json::json!({
                "status": "interrupted",
                "resultKind": "interrupted",
                "previousStatus": interruption.previous_status,
                "previousEventType": interruption.previous_event_type,
                "previousReasonCode": interruption.previous_reason_code,
            }),
            created_at: interruption.created_at.to_owned(),
        }
    }

    pub(crate) fn system_recovery_plan_started(
        event_id: impl Into<String>,
        plan: &RuntimeSafeFallbackPlan,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.started".to_owned(),
            status: "started".to_owned(),
            reason_code: "systemRecovery.started".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY.to_owned(),
            payload: serde_json::json!({
                "sourceRef": plan.source_ref.clone(),
                "posture": plan.posture,
                "stepCount": plan.steps.len(),
                "resultKind": ACTION_LOG_RESULT_KIND_FALLBACK,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn system_recovery_move_to_step_resolved(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveToStep,
        after_action_resolution: Option<&AfterActionResolution>,
        resolve_context: &ResolveContext,
        created_at: impl Into<String>,
    ) -> Self {
        let mut payload = serde_json::json!({
            "stepKind": "moveTo",
            "target": &step.target,
            "timeoutMs": step.timeout_ms,
            "resolveContext": resolve_context,
            "resultKind": ACTION_LOG_RESULT_KIND_FALLBACK,
        });
        apply_after_action_payload(
            &mut payload,
            step.after_action_id.as_deref(),
            after_action_resolution,
        );
        if let Some(after_action_resolution) = after_action_resolution {
            payload["animationRef"] = serde_json::json!(after_action_resolution.animation_ref);
        }

        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.resolved".to_owned(),
            status: "resolved".to_owned(),
            reason_code: "systemRecovery.stepResolved".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY.to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    pub(crate) fn system_recovery_move_to_step_completed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveToStep,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "systemRecovery.stepCompleted".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY.to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveTo",
                "target": &step.target,
                "timeoutMs": step.timeout_ms,
                "resultKind": ACTION_LOG_RESULT_KIND_FALLBACK,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn system_recovery_move_to_step_failed(
        ids: ActionLogEventIds<'_>,
        source_ref: &serde_json::Value,
        step: &MoveToStep,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: ids.event_id.to_owned(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "step.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "systemRecovery.stepFailed".to_owned(),
            plan_id: ids.plan_id.to_owned(),
            step_id: ids.step_id.map(str::to_owned),
            source_ref: source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY.to_owned(),
            payload: serde_json::json!({
                "stepKind": "moveTo",
                "target": &step.target,
                "timeoutMs": step.timeout_ms,
                "error": error_message,
                "resultKind": ACTION_LOG_RESULT_KIND_FALLBACK,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn system_recovery_plan_completed(
        event_id: impl Into<String>,
        plan: &RuntimeSafeFallbackPlan,
        completed_step_count: u64,
        duration_ms: u64,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.completed".to_owned(),
            status: "completed".to_owned(),
            reason_code: "systemRecovery.completed".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY.to_owned(),
            payload: serde_json::json!({
                "status": "completed",
                "completedStepCount": completed_step_count,
                "durationMs": duration_ms,
                "resultKind": ACTION_LOG_RESULT_KIND_FALLBACK,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn system_recovery_plan_failed(
        event_id: impl Into<String>,
        plan: &RuntimeSafeFallbackPlan,
        error_message: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "plan.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "systemRecovery.failed".to_owned(),
            plan_id: plan.plan_id.clone(),
            step_id: None,
            source_ref: plan.source_ref.clone(),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY.to_owned(),
            payload: serde_json::json!({
                "status": "failed",
                "error": error_message.into(),
                "resultKind": ACTION_LOG_RESULT_KIND_FALLBACK,
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn executor_admission_decision(
        event_id: impl Into<String>,
        plan: &DevFixturePlan,
        decision: &ChoreographyAdmissionDecision,
        created_at: impl Into<String>,
    ) -> Self {
        Self::executor_admission_decision_for_source(
            event_id,
            plan.plan_id.as_str(),
            &plan.source_ref,
            ACTION_LOG_TRIGGER_SOURCE_DEV_FIXTURE,
            decision,
            created_at,
        )
    }

    pub(crate) fn system_recovery_executor_admission_decision(
        event_id: impl Into<String>,
        plan: &RuntimeSafeFallbackPlan,
        decision: &ChoreographyAdmissionDecision,
        created_at: impl Into<String>,
    ) -> Self {
        Self::executor_admission_decision_for_source(
            event_id,
            plan.plan_id.as_str(),
            &plan.source_ref,
            ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY,
            decision,
            created_at,
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn timeline_executor_admission_decision(
        event_id: impl Into<String>,
        plan: &TimelinePlan,
        trigger_source: ChoreographyTriggerSource,
        decision: &ChoreographyAdmissionDecision,
        created_at: impl Into<String>,
    ) -> Self {
        Self::executor_admission_decision_for_source(
            event_id,
            plan.plan_id.as_str(),
            &plan.source_ref,
            trigger_source.action_log_value(),
            decision,
            created_at,
        )
    }

    pub(crate) fn executor_admission_decision_for_source(
        event_id: impl Into<String>,
        plan_id: &str,
        source_ref: &serde_json::Value,
        trigger_source: &str,
        decision: &ChoreographyAdmissionDecision,
        created_at: impl Into<String>,
    ) -> Self {
        let (event_type, status, reason_code) = executor_admission_event_meta(decision);
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: event_type.to_owned(),
            status: status.to_owned(),
            reason_code: reason_code.to_owned(),
            plan_id: plan_id.to_owned(),
            step_id: None,
            source_ref: source_ref.clone(),
            trigger_source: trigger_source.to_owned(),
            payload: decision.action_log_payload(),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn resolved_action_id(&self) -> Option<&str> {
        self.payload
            .get("resolution")
            .and_then(|resolution| resolution.get("actionId"))
            .or_else(|| self.payload.get("actionId"))
            .or_else(|| self.payload.get("afterResolvedActionId"))
            .and_then(serde_json::Value::as_str)
    }

    pub(crate) fn resolved_animation_ref(&self) -> Option<&str> {
        self.payload
            .get("resolution")
            .and_then(|resolution| resolution.get("animationRef"))
            .or_else(|| self.payload.get("animationRef"))
            .or_else(|| self.payload.get("afterAnimationRef"))
            .and_then(serde_json::Value::as_str)
    }
}

fn executor_admission_event_meta(
    decision: &ChoreographyAdmissionDecision,
) -> (&'static str, &'static str, &str) {
    match decision {
        ChoreographyAdmissionDecision::Accepted { .. } => {
            ("executor.accepted", "running", "executor.accepted")
        }
        ChoreographyAdmissionDecision::Preempted { reason_code, .. } => {
            ("executor.preempted", "running", reason_code.as_str())
        }
        ChoreographyAdmissionDecision::Rejected { reason_code, .. } => {
            ("executor.rejected", "rejected", reason_code.as_str())
        }
        ChoreographyAdmissionDecision::Deferred { reason_code, .. } => {
            ("executor.deferred", "deferred", reason_code.as_str())
        }
        ChoreographyAdmissionDecision::Skipped { reason_code, .. } => {
            ("executor.skipped", "skipped", reason_code.as_str())
        }
    }
}

fn play_action_resolved_payload(
    resolution: &StepResolution,
    resolve_context: &ResolveContext,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "stepKind": "playAction",
        "actionId": &resolution.action_id,
        "animationRef": &resolution.animation_ref,
        "playbackKind": &resolution.playback_kind,
        "durationMs": resolution.duration_ms,
        "loop": resolution.loop_animation,
        "resolvedFromRegistryVersion": &resolution.resolved_from_registry_version,
        "resolveContext": resolve_context,
    });
    apply_resolution_fallback_payload(&mut payload, resolution);
    payload
}

fn play_action_completed_payload(
    resolution: &StepResolution,
    elapsed_ms: u64,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "stepKind": "playAction",
        "actionId": &resolution.action_id,
        "animationRef": &resolution.animation_ref,
        "elapsedMs": elapsed_ms,
    });
    apply_resolution_fallback_payload(&mut payload, resolution);
    payload
}

fn play_action_failed_payload(
    resolution: &StepResolution,
    error_message: &str,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "stepKind": "playAction",
        "actionId": &resolution.action_id,
        "animationRef": &resolution.animation_ref,
        "error": error_message,
    });
    apply_resolution_fallback_payload(&mut payload, resolution);
    payload
}

fn apply_resolution_fallback_payload(payload: &mut serde_json::Value, resolution: &StepResolution) {
    let Some(fallback) = resolution.fallback.as_ref() else {
        return;
    };

    apply_fallback_payload(payload, fallback);
}

fn apply_after_action_payload(
    payload: &mut serde_json::Value,
    after_action_id: Option<&str>,
    after_action_resolution: Option<&AfterActionResolution>,
) {
    if let Some(after_action_id) = after_action_id {
        payload["afterActionId"] = serde_json::json!(after_action_id);
    }
    let Some(after_action_resolution) = after_action_resolution else {
        return;
    };

    payload["afterResolvedActionId"] = serde_json::json!(after_action_resolution.action_id);
    payload["afterAnimationRef"] = serde_json::json!(after_action_resolution.animation_ref);
    if let Some(fallback) = after_action_resolution.fallback.as_ref() {
        apply_fallback_payload(payload, fallback);
    }
}

fn apply_fallback_payload(
    payload: &mut serde_json::Value,
    fallback: &super::registry::StepResolutionFallback,
) {
    payload["resultKind"] = serde_json::json!(ACTION_LOG_RESULT_KIND_FALLBACK);
    payload["detailReasonCode"] = serde_json::json!(fallback.reason_code.as_str());
    payload["fallback"] = serde_json::json!(fallback);
}

impl ActionLogSystemEvent {
    pub(crate) fn startup_health_failed(
        event_id: impl Into<String>,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "startupHealth.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "startupHealth.nativePetUnavailable".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "runtime" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_STARTUP_HEALTH.to_owned(),
            payload: serde_json::json!({
                "check": "nativePet.sidecar.ready",
                "detail": {
                    "message": "native pet sidecar startup failed",
                    "source": "startupHealth",
                    "items": [
                        {
                            "key": "error",
                            "value": error_message,
                        }
                    ],
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn health_gate_passed(
        event_id: impl Into<String>,
        recovery_trigger_source: ChoreographyTriggerSource,
        created_at: impl Into<String>,
    ) -> Self {
        let recovery_trigger_source = recovery_trigger_source.action_log_value();
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "healthGate.passed".to_owned(),
            status: "passed".to_owned(),
            reason_code: "sidecar.available".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "healthGate" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_HEALTH_GATE.to_owned(),
            payload: serde_json::json!({
                "recoveryTriggerSource": recovery_trigger_source,
                "check": "sidecar.queryState",
                "detail": {
                    "message": format!(
                        "choreography runtime readiness recovered after {recovery_trigger_source} health gate"
                    ),
                    "source": "choreographyRuntimeReadiness",
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn health_gate_failed(
        event_id: impl Into<String>,
        recovery_trigger_source: ChoreographyTriggerSource,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        let recovery_trigger_source = recovery_trigger_source.action_log_value();
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "healthGate.failed".to_owned(),
            status: "failed".to_owned(),
            reason_code: "sidecar.unavailable".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "healthGate" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_HEALTH_GATE.to_owned(),
            payload: serde_json::json!({
                "recoveryTriggerSource": recovery_trigger_source,
                "check": "sidecar.queryState",
                "error": error_message,
                "detail": {
                    "message": format!(
                        "choreography runtime readiness health gate failed for {recovery_trigger_source}"
                    ),
                    "source": "choreographyRuntimeReadiness",
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn choreography_scheduler_pending_body_stored(
        event_id: impl Into<String>,
        plan_id: &str,
        body_kind: &str,
        schema_version: u16,
        body: &serde_json::Value,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "choreographyScheduler.pendingBodyStored".to_owned(),
            status: "completed".to_owned(),
            reason_code: "choreographyScheduler.pendingBodyStored".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "choreographyScheduler" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_CHOREOGRAPHY_SCHEDULER.to_owned(),
            payload: serde_json::json!({
                "planId": plan_id,
                "bodyKind": body_kind,
                "schemaVersion": schema_version,
                "body": body,
                "detail": {
                    "message": format!("stored pending choreography execution body for plan {plan_id}"),
                    "source": "choreographyScheduler",
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn choreography_scheduler_pending_body_deleted(
        event_id: impl Into<String>,
        plan_id: &str,
        body_kind: &str,
        schema_version: u16,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "choreographyScheduler.pendingBodyDeleted".to_owned(),
            status: "completed".to_owned(),
            reason_code: "choreographyScheduler.pendingBodyDeleted".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "choreographyScheduler" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_CHOREOGRAPHY_SCHEDULER.to_owned(),
            payload: serde_json::json!({
                "planId": plan_id,
                "bodyKind": body_kind,
                "schemaVersion": schema_version,
                "detail": {
                    "message": format!("deleted pending choreography execution body for plan {plan_id}"),
                    "source": "choreographyScheduler",
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn choreography_scheduler_stale_pending_bodies_cleared(
        event_id: impl Into<String>,
        cleared_body_count: usize,
        recoverable_pending_admission_count: usize,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "choreographyScheduler.stalePendingBodiesCleared".to_owned(),
            status: "completed".to_owned(),
            reason_code: "runtime.restarted".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "choreographyScheduler" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_CHOREOGRAPHY_SCHEDULER.to_owned(),
            payload: serde_json::json!({
                "clearedBodyCount": cleared_body_count,
                "recoverablePendingAdmissionCount": recoverable_pending_admission_count,
                "detail": {
                    "message": format!(
                        "cleared {cleared_body_count} stale choreography pending execution body cache row(s) after runtime restart; {recoverable_pending_admission_count} recoverable pending admission(s) were discarded instead of replayed"
                    ),
                    "source": "choreographyScheduler",
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn choreography_scheduler_pending_body_missing(
        event_id: impl Into<String>,
        active_plan_id: &str,
        pending_plan_id: &str,
        phase: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "choreographyScheduler.pendingBodyMissing".to_owned(),
            status: "degraded".to_owned(),
            reason_code: "choreographyScheduler.pendingBodyMissing".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "choreographyScheduler" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_CHOREOGRAPHY_SCHEDULER.to_owned(),
            payload: serde_json::json!({
                "activePlanId": active_plan_id,
                "pendingPlanId": pending_plan_id,
                "phase": phase,
                "detail": {
                    "message": format!(
                        "pending choreography plan {pending_plan_id} has no queued execution body"
                    ),
                    "source": "choreographyScheduler",
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn action_log_index_sync_failed(
        event_id: impl Into<String>,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "actionLogIndex.syncFailed".to_owned(),
            status: "degraded".to_owned(),
            reason_code: "actionLogIndex.syncFailed".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "actionLogIndex" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_ACTION_LOG_INDEX.to_owned(),
            payload: serde_json::json!({
                "detail": {
                    "message": error_message,
                    "source": "sync_choreography_action_log_index",
                },
            }),
            created_at: created_at.into(),
        }
    }

    pub(crate) fn runtime_degraded_after_system_recovery_failed(
        event_id: impl Into<String>,
        plan: &RuntimeSafeFallbackPlan,
        error_message: &str,
        created_at: impl Into<String>,
    ) -> Self {
        let mut payload = serde_json::json!({
            "failedRecoveryPlanId": &plan.plan_id,
            "error": error_message,
        });
        if let Some(triggered_by_plan_id) = plan
            .source_ref
            .get("triggeredByPlanId")
            .and_then(serde_json::Value::as_str)
        {
            payload["triggeredByPlanId"] = serde_json::json!(triggered_by_plan_id);
        }
        if let Some(triggered_by_step_id) = plan
            .source_ref
            .get("triggeredByStepId")
            .and_then(serde_json::Value::as_str)
        {
            payload["triggeredByStepId"] = serde_json::json!(triggered_by_step_id);
        }
        if let Some(trigger_reason) = plan
            .source_ref
            .get("triggerReason")
            .and_then(serde_json::Value::as_str)
        {
            payload["triggerReason"] = serde_json::json!(trigger_reason);
        }

        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "runtime.degraded".to_owned(),
            status: "degraded".to_owned(),
            reason_code: "runtime.systemRecoveryFailed".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({ "kind": "runtime" }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_SYSTEM_RECOVERY.to_owned(),
            payload,
            created_at: created_at.into(),
        }
    }

    pub(crate) fn affective_context_invalid_state_file(
        event_id: impl Into<String>,
        state_file_name: &str,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            schema_version: ACTION_LOG_SCHEMA_VERSION,
            event_type: "affectiveContext.invalidStateFile".to_owned(),
            status: "degraded".to_owned(),
            reason_code: "affectiveContext.invalidStateFile".to_owned(),
            plan_id: None,
            step_id: None,
            source_ref: serde_json::json!({
                "kind": "affectiveContext",
                "stateFileName": state_file_name,
            }),
            trigger_source: ACTION_LOG_TRIGGER_SOURCE_AFFECTIVE_CONTEXT.to_owned(),
            payload: serde_json::json!({
                "stateFileName": state_file_name,
                "fallbackContext": AffectiveContext::default(),
            }),
            created_at: created_at.into(),
        }
    }
}
