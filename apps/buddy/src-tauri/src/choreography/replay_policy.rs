use crate::{
    choreography::admission::ChoreographyTriggerSource,
    storage::ChoreographyPendingExecutionBodyKind,
};

const STARTUP_RECOVERABLE_REPLAY_BODY_SCHEMA_VERSION: u16 = 1;
const STARTUP_RECOVERABLE_REPLAY_MAX_PLAN_AGE_SECONDS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StartupRecoverableReplayPolicyDecision {
    Candidate,
    Manual,
    Reject,
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupRecoverableReplayActionLogIndexStatus {
    Fresh,
    Stale,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartupRecoverableReplayPolicySummary {
    pub(crate) decision: StartupRecoverableReplayPolicyDecision,
    pub(crate) reason_code: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StartupRecoverableReplayPolicyInput<'a> {
    pub(crate) action_log_index_status: StartupRecoverableReplayActionLogIndexStatus,
    pub(crate) trigger_source: ChoreographyTriggerSource,
    pub(crate) source_ref_kind: &'a str,
    pub(crate) source_ref_id: Option<&'a str>,
    pub(crate) recovery_source_plan_available: bool,
    pub(crate) body_kind: ChoreographyPendingExecutionBodyKind,
    pub(crate) body_schema_version: u16,
    pub(crate) plan_age_seconds: Option<u64>,
    pub(crate) runtime_accepting_choreography: bool,
    pub(crate) admission_is_idle: bool,
    pub(crate) local_interaction_is_active: bool,
}

pub(crate) fn evaluate_startup_recoverable_replay_policy(
    input: StartupRecoverableReplayPolicyInput<'_>,
) -> StartupRecoverableReplayPolicySummary {
    match input.action_log_index_status {
        StartupRecoverableReplayActionLogIndexStatus::Fresh => {}
        StartupRecoverableReplayActionLogIndexStatus::Stale => {
            return reject("replay.actionLogIndexStale");
        }
        StartupRecoverableReplayActionLogIndexStatus::Failed => {
            return reject("replay.actionLogIndexFailed");
        }
    }
    if input.body_schema_version != STARTUP_RECOVERABLE_REPLAY_BODY_SCHEMA_VERSION {
        return reject("replay.unsupportedBodySchemaVersion");
    }
    if input.source_ref_id.is_none() {
        return reject("replay.missingSourceRefId");
    }
    if matches!(input.source_ref_kind, "systemRecovery" | "macroFallback")
        && !input.recovery_source_plan_available
    {
        return reject("replay.recoverySourcePlanUnavailable");
    }
    if !input.runtime_accepting_choreography {
        return reject("replay.runtimeNotReady");
    }
    let Some(plan_age_seconds) = input.plan_age_seconds else {
        return reject("replay.planAgeUnavailable");
    };
    if plan_age_seconds > STARTUP_RECOVERABLE_REPLAY_MAX_PLAN_AGE_SECONDS {
        return reject("replay.planTooOld");
    }
    if matches!(
        input.trigger_source,
        ChoreographyTriggerSource::IdleAutonomous
    ) {
        return reject("replay.idleAutonomousNotReplayable");
    }
    if !input.admission_is_idle {
        return wait("replay.waitingForIdle");
    }
    if input.local_interaction_is_active {
        return wait("replay.localInteractionActive");
    }

    match input.body_kind {
        ChoreographyPendingExecutionBodyKind::Timeline
        | ChoreographyPendingExecutionBodyKind::DevFixture => match input.trigger_source {
            ChoreographyTriggerSource::AttentionSystem
            | ChoreographyTriggerSource::CriticalInteraction => {
                candidate("replay.candidateRequiresRuntimeGates")
            }
            ChoreographyTriggerSource::AiChoreography
            | ChoreographyTriggerSource::SystemRecovery
            | ChoreographyTriggerSource::UserRequested => manual_internal_only(),
            ChoreographyTriggerSource::IdleAutonomous => {
                reject("replay.idleAutonomousNotReplayable")
            }
        },
    }
}

fn candidate(reason_code: &'static str) -> StartupRecoverableReplayPolicySummary {
    StartupRecoverableReplayPolicySummary {
        decision: StartupRecoverableReplayPolicyDecision::Candidate,
        reason_code,
    }
}

fn manual_internal_only() -> StartupRecoverableReplayPolicySummary {
    StartupRecoverableReplayPolicySummary {
        decision: StartupRecoverableReplayPolicyDecision::Manual,
        reason_code: "replay.manualInternalOnly",
    }
}

fn reject(reason_code: &'static str) -> StartupRecoverableReplayPolicySummary {
    StartupRecoverableReplayPolicySummary {
        decision: StartupRecoverableReplayPolicyDecision::Reject,
        reason_code,
    }
}

fn wait(reason_code: &'static str) -> StartupRecoverableReplayPolicySummary {
    StartupRecoverableReplayPolicySummary {
        decision: StartupRecoverableReplayPolicyDecision::Wait,
        reason_code,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_startup_recoverable_replay_policy, StartupRecoverableReplayActionLogIndexStatus,
        StartupRecoverableReplayPolicyDecision, StartupRecoverableReplayPolicyInput,
        STARTUP_RECOVERABLE_REPLAY_MAX_PLAN_AGE_SECONDS,
    };
    use crate::{
        choreography::admission::ChoreographyTriggerSource,
        storage::ChoreographyPendingExecutionBodyKind,
    };

    fn default_replay_policy_input() -> StartupRecoverableReplayPolicyInput<'static> {
        StartupRecoverableReplayPolicyInput {
            action_log_index_status: StartupRecoverableReplayActionLogIndexStatus::Fresh,
            trigger_source: ChoreographyTriggerSource::UserRequested,
            source_ref_kind: "conversationMessage",
            source_ref_id: Some("message_1"),
            recovery_source_plan_available: true,
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            body_schema_version: 1,
            plan_age_seconds: Some(0),
            runtime_accepting_choreography: true,
            admission_is_idle: true,
            local_interaction_is_active: false,
        }
    }

    #[test]
    fn startup_recoverable_replay_policy_returns_manual_internal_only_for_user_requested_timeline()
    {
        let summary = evaluate_startup_recoverable_replay_policy(default_replay_policy_input());

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Manual
        );
        assert_eq!(summary.reason_code, "replay.manualInternalOnly");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_missing_source_ref_id() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                source_ref_id: None,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.missingSourceRefId");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_unsupported_schema_version() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                body_schema_version: 2,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.unsupportedBodySchemaVersion");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_plan_older_than_threshold() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                plan_age_seconds: Some(STARTUP_RECOVERABLE_REPLAY_MAX_PLAN_AGE_SECONDS + 1),
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.planTooOld");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_unknown_plan_age() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                plan_age_seconds: None,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.planAgeUnavailable");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_idle_autonomous_source() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                trigger_source: ChoreographyTriggerSource::IdleAutonomous,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.idleAutonomousNotReplayable");
    }

    #[test]
    fn startup_recoverable_replay_policy_waits_when_admission_is_busy() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                admission_is_idle: false,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Wait
        );
        assert_eq!(summary.reason_code, "replay.waitingForIdle");
    }

    #[test]
    fn startup_recoverable_replay_policy_marks_attention_system_as_candidate() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                trigger_source: ChoreographyTriggerSource::AttentionSystem,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Candidate
        );
        assert_eq!(summary.reason_code, "replay.candidateRequiresRuntimeGates");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_stale_action_log_index() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                action_log_index_status: StartupRecoverableReplayActionLogIndexStatus::Stale,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.actionLogIndexStale");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_failed_action_log_index() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                action_log_index_status: StartupRecoverableReplayActionLogIndexStatus::Failed,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.actionLogIndexFailed");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_runtime_not_accepting_choreography() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                runtime_accepting_choreography: false,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(summary.reason_code, "replay.runtimeNotReady");
    }

    #[test]
    fn startup_recoverable_replay_policy_waits_when_local_interaction_is_active() {
        let summary =
            evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                local_interaction_is_active: true,
                ..default_replay_policy_input()
            });

        assert_eq!(
            summary.decision,
            StartupRecoverableReplayPolicyDecision::Wait
        );
        assert_eq!(summary.reason_code, "replay.localInteractionActive");
    }

    #[test]
    fn startup_recoverable_replay_policy_rejects_recovery_sources_without_original_plan() {
        for source_ref_kind in ["systemRecovery", "macroFallback"] {
            let summary =
                evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
                    source_ref_kind,
                    recovery_source_plan_available: false,
                    ..default_replay_policy_input()
                });

            assert_eq!(
                summary.decision,
                StartupRecoverableReplayPolicyDecision::Reject,
                "unexpected decision for {source_ref_kind}"
            );
            assert_eq!(
                summary.reason_code, "replay.recoverySourcePlanUnavailable",
                "unexpected reason for {source_ref_kind}"
            );
        }
    }
}
