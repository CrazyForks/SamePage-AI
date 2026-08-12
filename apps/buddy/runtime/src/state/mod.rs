#![allow(dead_code)]

use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use crate::choreography::admission::{ChoreographyAdmissionDecision, ChoreographyAdmissionRequest};
use crate::{
    app_paths::{BuddyAppPaths, BuddyAppPathsStatus},
    choreography::action_log::ActionLogSystemEvent,
    choreography::admission::{
        ChoreographyActiveStepUpdate, ChoreographyAdmissionRelease, ChoreographyAdmissionState,
        ChoreographyPlanPriority, ChoreographyTriggerSource,
    },
    choreography::executor::{
        admit_dev_fixture_with_pending_queue, admit_released_pending_dev_fixture,
        admit_released_pending_timeline_plan, admit_timeline_plan_with_pending_queue,
        DevFixtureAdmissionExecutionRequest, DevFixtureExecutionError,
        DevFixturePendingExecutionBody, PendingDevFixtureExecutionQueue,
        PendingTimelineExecutionQueue, ScheduledDevFixtureExecution, ScheduledTimelineExecution,
        TimelineAdmissionExecutionRequest, TimelineExecutionError, TimelinePendingExecutionBody,
    },
    choreography::readiness::{
        ChoreographyRuntimeReadinessSnapshot, ChoreographyRuntimeReadinessState,
    },
    choreography::replay_policy::{
        evaluate_startup_recoverable_replay_policy, StartupRecoverableReplayActionLogIndexStatus,
        StartupRecoverableReplayPolicyDecision, StartupRecoverableReplayPolicyInput,
        StartupRecoverableReplayPolicySummary,
    },
    domain::BuddyApprovalTerminalStatus,
    error::{BuddyError, BuddyResult},
    local_log::{parse_rfc3339_utc_seconds, LocalLogTimestamp},
    memory,
    native_pet::{
        query_native_pet_local_interaction_active, step_protocol::SidecarInterruptPolicy,
    },
    storage::{
        action_log_source_ref_kind, action_log_source_ref_primary_id,
        AppendBuddyConversationMessageRequest, BuddyApproval, BuddyConversation, BuddyMessage,
        BuddyProject, BuddyRegisteredAttachment, BuddyResolvedCodexAppServerRequestApproval,
        BuddyRun, BuddySession, BuddySetting, BuddyStorage, BuddyStorageStatus,
        ChoreographyActionLogIndexHealth, ChoreographyPendingExecutionBodyKind,
        CreateBuddyConversationRequest, CreateBuddyConversationRunRequest,
        CreateBuddyMessageRequest, CreateBuddyRegisteredAttachmentRequest,
        CreateBuddySessionRequest, RecoverableChoreographyPendingExecution,
        UpsertBuddyProjectRequest,
    },
};

#[derive(Clone)]
pub struct BuddyAppState {
    paths: BuddyAppPaths,
    storage: BuddyStorage,
    choreography_admission: Arc<Mutex<ChoreographyAdmissionState>>,
    choreography_pending_timeline: Arc<Mutex<PendingTimelineExecutionQueue>>,
    choreography_pending_dev_fixture: Arc<Mutex<PendingDevFixtureExecutionQueue>>,
    startup_recoverable_choreography_pending:
        Arc<Mutex<Vec<RecoverableChoreographyPendingExecution>>>,
    choreography_readiness: Arc<Mutex<ChoreographyRuntimeReadinessState>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyLocalStateStatus {
    paths: BuddyAppPathsStatus,
    storage: BuddyStorageStatus,
}

pub(crate) struct ChoreographyReleaseSchedule {
    pub(crate) release: ChoreographyAdmissionRelease,
    pub(crate) scheduled: Option<ScheduledChoreographyExecution>,
}

pub(crate) enum ScheduledChoreographyExecution {
    Timeline(ScheduledTimelineExecution),
    DevFixture(ScheduledDevFixtureExecution),
}

pub(crate) enum ChoreographyStepCompletionSchedule {
    Continue,
    RunPendingHandoffFinalizer { step_id: String },
    YieldToPendingPlan(Box<ScheduledChoreographyExecution>),
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartupRecoverableChoreographyPendingExecutionSummary {
    plan_id: String,
    source_ref_kind: String,
    source_ref_id: Option<String>,
    trigger_source: ChoreographyTriggerSource,
    priority: ChoreographyPlanPriority,
    reason_code: String,
    body_kind: ChoreographyPendingExecutionBodyKind,
    body_schema_version: u16,
    replay_policy: StartupRecoverableReplayPolicySummary,
    deferred_event_id: String,
    deferred_at: String,
    stored_event_id: String,
    stored_at: String,
}

impl StartupRecoverableChoreographyPendingExecutionSummary {
    pub(crate) fn plan_id(&self) -> &str {
        self.plan_id.as_str()
    }

    pub(crate) fn replay_policy(&self) -> &StartupRecoverableReplayPolicySummary {
        &self.replay_policy
    }
}

enum StartupRecoverablePendingExecutionSelector<'a> {
    PlanId(&'a str),
    NextEligible,
}

impl BuddyAppState {
    pub fn initialize_with_paths(paths: BuddyAppPaths) -> BuddyResult<Self> {
        paths.ensure_exists()?;
        memory::workspace::ensure_memory_workspace(&paths.memories_dir_path())?;

        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        storage.initialize()?;
        let mut startup_recoverable_choreography_pending = Vec::new();
        match storage.sync_choreography_action_log_index() {
            Ok(()) => {
                let stale_pending_body_cleanup =
                    rebuild_and_clear_stale_choreography_pending_bodies_after_startup(&storage)?;
                if stale_pending_body_cleanup.cleared_body_count > 0 {
                    if let Err(error) = append_stale_choreography_pending_bodies_cleared_diagnostic(
                        &storage,
                        &stale_pending_body_cleanup,
                    ) {
                        eprintln!(
                            "lexora buddy stale choreography pending body cleanup diagnostic failed: {error}"
                        );
                    }
                }
                startup_recoverable_choreography_pending =
                    stale_pending_body_cleanup.recoverable_pending_executions;
                if let Err(error) = storage
                    .reconcile_stale_choreography_action_log_plans_after_startup(
                        &LocalLogTimestamp::now_utc().to_rfc3339_millis(),
                    )
                {
                    eprintln!("lexora buddy stale action log plan reconcile failed: {error}");
                }
            }
            Err(error) => {
                storage.clear_choreography_pending_execution_bodies()?;
                eprintln!("lexora buddy action log index sync failed: {error}");
                if let Err(diagnostic_error) =
                    append_action_log_index_sync_failed_diagnostic(&storage, &error.to_string())
                {
                    eprintln!(
                        "lexora buddy action log index sync failure diagnostic failed: {diagnostic_error}"
                    );
                }
            }
        }

        Ok(Self {
            paths,
            storage,
            choreography_admission: Arc::new(Mutex::new(ChoreographyAdmissionState::default())),
            choreography_pending_timeline: Arc::new(Mutex::new(
                PendingTimelineExecutionQueue::default(),
            )),
            choreography_pending_dev_fixture: Arc::new(Mutex::new(
                PendingDevFixtureExecutionQueue::default(),
            )),
            startup_recoverable_choreography_pending: Arc::new(Mutex::new(
                startup_recoverable_choreography_pending,
            )),
            choreography_readiness: Arc::new(Mutex::new(
                ChoreographyRuntimeReadinessState::default(),
            )),
        })
    }

    pub fn local_state_status(&self) -> BuddyResult<BuddyLocalStateStatus> {
        Ok(BuddyLocalStateStatus {
            paths: self.paths.status(),
            storage: self.storage.status_snapshot()?,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn startup_recoverable_choreography_pending_count(&self) -> BuddyResult<usize> {
        let entries = self
            .startup_recoverable_choreography_pending
            .lock()
            .map_err(|_| {
                BuddyError::Runtime(
                    "startup recoverable choreography pending lock was poisoned".to_owned(),
                )
            })?;
        Ok(entries.len())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn startup_recoverable_choreography_pending_plan_ids(
        &self,
    ) -> BuddyResult<Vec<String>> {
        let entries = self
            .startup_recoverable_choreography_pending
            .lock()
            .map_err(|_| {
                BuddyError::Runtime(
                    "startup recoverable choreography pending lock was poisoned".to_owned(),
                )
            })?;
        Ok(entries
            .iter()
            .map(|entry| entry.admission.plan_id.clone())
            .collect())
    }

    pub(crate) fn startup_recoverable_choreography_pending_summaries_with_local_interaction_status(
        &self,
        local_interaction_is_active: bool,
    ) -> BuddyResult<Vec<StartupRecoverableChoreographyPendingExecutionSummary>> {
        let admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;
        let admission_is_idle = admission.active_plan_id().is_none();
        drop(admission);
        let runtime_accepting_choreography = self
            .choreography_runtime_readiness_snapshot()?
            .accepting_choreography;
        let action_log_index_status = startup_recoverable_replay_action_log_index_status(
            self.storage.choreography_action_log_index_health()?,
        );
        let current_unix_seconds = startup_recoverable_current_unix_seconds();

        let entries = self
            .startup_recoverable_choreography_pending
            .lock()
            .map_err(|_| {
                BuddyError::Runtime(
                    "startup recoverable choreography pending lock was poisoned".to_owned(),
                )
            })?;
        Ok(entries
            .iter()
            .map(|entry| {
                startup_recoverable_choreography_pending_execution_summary(
                    &self.storage,
                    entry,
                    admission_is_idle,
                    runtime_accepting_choreography,
                    action_log_index_status,
                    current_unix_seconds,
                    local_interaction_is_active,
                )
            })
            .collect())
    }

    pub(crate) fn schedule_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
        &self,
        plan_id: &str,
        local_interaction_is_active: bool,
    ) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
        self.schedule_startup_recoverable_choreography_pending_execution_with_selector(
            local_interaction_is_active,
            StartupRecoverablePendingExecutionSelector::PlanId(plan_id),
        )
    }

    pub(crate) fn schedule_next_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
        &self,
        local_interaction_is_active: bool,
    ) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
        self.schedule_startup_recoverable_choreography_pending_execution_with_selector(
            local_interaction_is_active,
            StartupRecoverablePendingExecutionSelector::NextEligible,
        )
    }

    fn schedule_startup_recoverable_choreography_pending_execution_with_selector(
        &self,
        local_interaction_is_active: bool,
        selector: StartupRecoverablePendingExecutionSelector<'_>,
    ) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
        self.ensure_choreography_runtime_accepting()?;

        if local_interaction_is_active {
            return Ok(None);
        }

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;
        if admission.active_plan_id().is_some() {
            return Ok(None);
        }

        let action_log_index_status = startup_recoverable_replay_action_log_index_status(
            self.storage.choreography_action_log_index_health()?,
        );
        let current_unix_seconds = startup_recoverable_current_unix_seconds();

        let mut pending_timeline = self.choreography_pending_timeline.lock().map_err(|_| {
            BuddyError::Runtime("choreography pending timeline queue lock was poisoned".to_owned())
        })?;
        let mut pending_dev_fixture =
            self.choreography_pending_dev_fixture.lock().map_err(|_| {
                BuddyError::Runtime(
                    "choreography pending dev fixture queue lock was poisoned".to_owned(),
                )
            })?;
        let mut entries = self
            .startup_recoverable_choreography_pending
            .lock()
            .map_err(|_| {
                BuddyError::Runtime(
                    "startup recoverable choreography pending lock was poisoned".to_owned(),
                )
            })?;
        let replay_policy_for_entry = |entry: &RecoverableChoreographyPendingExecution| {
            startup_recoverable_choreography_pending_execution_summary(
                &self.storage,
                entry,
                true,
                true,
                action_log_index_status,
                current_unix_seconds,
                false,
            )
            .replay_policy
        };
        let selection = match selector {
            StartupRecoverablePendingExecutionSelector::PlanId(plan_id) => entries
                .iter()
                .position(|entry| entry.admission.plan_id == plan_id)
                .map(|index| (index, replay_policy_for_entry(&entries[index]))),
            StartupRecoverablePendingExecutionSelector::NextEligible => {
                entries.iter().enumerate().find_map(|(index, entry)| {
                    let replay_policy = replay_policy_for_entry(entry);
                    match replay_policy.decision {
                        StartupRecoverableReplayPolicyDecision::Reject => None,
                        StartupRecoverableReplayPolicyDecision::Wait
                        | StartupRecoverableReplayPolicyDecision::Manual
                        | StartupRecoverableReplayPolicyDecision::Candidate => {
                            Some((index, replay_policy))
                        }
                    }
                })
            }
        };
        let Some((index, replay_policy)) = selection else {
            return Ok(None);
        };
        if !matches!(
            replay_policy.decision,
            StartupRecoverableReplayPolicyDecision::Manual
                | StartupRecoverableReplayPolicyDecision::Candidate
        ) {
            return Ok(None);
        }

        let scheduled = schedule_startup_recoverable_choreography_pending_execution_entry(
            &self.storage,
            &mut admission,
            &mut pending_timeline,
            &mut pending_dev_fixture,
            entries[index].clone(),
        )?;
        entries.remove(index);

        Ok(Some(scheduled))
    }

    pub fn global_runtime_cwd(&self) -> String {
        self.paths.data_dir_path().to_string_lossy().into_owned()
    }

    pub(crate) fn data_dir_path(&self) -> std::path::PathBuf {
        self.paths.data_dir_path()
    }

    pub fn attachments_dir_path(&self) -> std::path::PathBuf {
        self.paths.attachments_dir_path()
    }

    pub fn memories_dir_path(&self) -> std::path::PathBuf {
        self.paths.memories_dir_path()
    }

    pub fn storage_handle(&self) -> BuddyStorage {
        self.storage.clone()
    }

    #[cfg(test)]
    pub(crate) fn admit_choreography_plan(
        &self,
        request: ChoreographyAdmissionRequest,
    ) -> BuddyResult<ChoreographyAdmissionDecision> {
        self.ensure_choreography_runtime_accepting()?;

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;

        Ok(admission.admit(request))
    }

    #[cfg(test)]
    pub(crate) fn release_choreography_plan(
        &self,
        plan_id: &str,
    ) -> BuddyResult<ChoreographyAdmissionRelease> {
        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;

        Ok(admission.release_plan(plan_id))
    }

    pub(crate) fn release_choreography_plan_and_schedule_pending(
        &self,
        plan_id: &str,
    ) -> BuddyResult<ChoreographyReleaseSchedule> {
        self.ensure_choreography_runtime_accepting()?;

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;
        let mut pending_timeline = self.choreography_pending_timeline.lock().map_err(|_| {
            BuddyError::Runtime("choreography pending timeline queue lock was poisoned".to_owned())
        })?;
        let mut pending_dev_fixture =
            self.choreography_pending_dev_fixture.lock().map_err(|_| {
                BuddyError::Runtime(
                    "choreography pending dev fixture queue lock was poisoned".to_owned(),
                )
            })?;

        ensure_next_pending_choreography_plan_has_execution_body_for_active_plan(
            &self.storage,
            &admission,
            plan_id,
            &pending_timeline,
            &pending_dev_fixture,
            "release",
        )?;

        let release = admission.release_plan(plan_id);
        let scheduled = schedule_released_pending_choreography_plan(
            &mut admission,
            &mut pending_timeline,
            &mut pending_dev_fixture,
            release.clone(),
        )?;

        Ok(ChoreographyReleaseSchedule { release, scheduled })
    }

    pub(crate) fn release_choreography_plan_preserving_pending(
        &self,
        plan_id: &str,
    ) -> BuddyResult<ChoreographyAdmissionRelease> {
        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;

        Ok(admission.release_plan_preserving_pending(plan_id))
    }

    pub(crate) fn schedule_pending_choreography_plan_if_idle(
        &self,
        released_plan_id: &str,
    ) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
        self.ensure_choreography_runtime_accepting()?;

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;
        if let Some(active_plan_id) = admission.active_plan_id() {
            return Err(BuddyError::Runtime(format!(
                "cannot schedule pending choreography plan while plan {active_plan_id} is active"
            )));
        }
        let Some(pending_plan_id) = admission.next_pending_plan_id().map(str::to_owned) else {
            return Ok(None);
        };
        let mut pending_timeline = self.choreography_pending_timeline.lock().map_err(|_| {
            BuddyError::Runtime("choreography pending timeline queue lock was poisoned".to_owned())
        })?;
        let mut pending_dev_fixture =
            self.choreography_pending_dev_fixture.lock().map_err(|_| {
                BuddyError::Runtime(
                    "choreography pending dev fixture queue lock was poisoned".to_owned(),
                )
            })?;

        ensure_pending_choreography_plan_has_execution_body(
            &self.storage,
            released_plan_id,
            pending_plan_id.as_str(),
            &pending_timeline,
            &pending_dev_fixture,
            "failureRecovery",
        )?;

        let release = admission.release_next_pending_plan_if_idle(released_plan_id);
        let scheduled = schedule_released_pending_choreography_plan(
            &mut admission,
            &mut pending_timeline,
            &mut pending_dev_fixture,
            release,
        )?;

        scheduled.map_or_else(
            || {
                Err(BuddyError::Runtime(format!(
                    "pending choreography plan was released without a scheduled execution: {pending_plan_id}"
                )))
            },
            |scheduled| Ok(Some(scheduled)),
        )
    }

    pub(crate) fn schedule_pending_after_completed_choreography_step(
        &self,
        plan_id: &str,
        pending_handoff_finalizer_step_id: Option<&str>,
    ) -> BuddyResult<ChoreographyStepCompletionSchedule> {
        self.ensure_choreography_runtime_accepting()?;

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;
        if admission.active_plan_id() != Some(plan_id) {
            return Ok(ChoreographyStepCompletionSchedule::Continue);
        }
        let Some(pending_plan_id) = admission.next_pending_plan_id().map(str::to_owned) else {
            return Ok(ChoreographyStepCompletionSchedule::Continue);
        };
        if let Some(step_id) = pending_handoff_finalizer_step_id {
            return Ok(
                ChoreographyStepCompletionSchedule::RunPendingHandoffFinalizer {
                    step_id: step_id.to_owned(),
                },
            );
        }
        let mut pending_timeline = self.choreography_pending_timeline.lock().map_err(|_| {
            BuddyError::Runtime("choreography pending timeline queue lock was poisoned".to_owned())
        })?;
        let mut pending_dev_fixture =
            self.choreography_pending_dev_fixture.lock().map_err(|_| {
                BuddyError::Runtime(
                    "choreography pending dev fixture queue lock was poisoned".to_owned(),
                )
            })?;
        ensure_pending_choreography_plan_has_execution_body(
            &self.storage,
            plan_id,
            pending_plan_id.as_str(),
            &pending_timeline,
            &pending_dev_fixture,
            "stepCompletion",
        )?;

        let release = admission.release_plan(plan_id);

        let scheduled = schedule_released_pending_choreography_plan(
            &mut admission,
            &mut pending_timeline,
            &mut pending_dev_fixture,
            release,
        )?;

        let Some(scheduled) = scheduled else {
            return Err(BuddyError::Runtime(format!(
                "pending choreography plan was released without a scheduled execution: {pending_plan_id}"
            )));
        };

        Ok(ChoreographyStepCompletionSchedule::YieldToPendingPlan(
            Box::new(scheduled),
        ))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn refresh_choreography_plan_active_step(
        &self,
        plan_id: &str,
        step_id: &str,
        interrupt_policy: SidecarInterruptPolicy,
    ) -> BuddyResult<()> {
        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;

        match admission.update_active_step_with_policy(plan_id, step_id, interrupt_policy) {
            ChoreographyActiveStepUpdate::Updated { .. } => Ok(()),
            ChoreographyActiveStepUpdate::Stale {
                active_plan_id, ..
            } => Err(BuddyError::Runtime(format!(
                "choreography active step refresh ignored stale plan {plan_id}; active plan is {active_plan_id}"
            ))),
            ChoreographyActiveStepUpdate::NoActivePlan { .. } => Err(BuddyError::Runtime(
                format!("choreography active step refresh failed because plan {plan_id} is no longer active"),
            )),
        }
    }

    pub(crate) fn with_choreography_admission<T>(
        &self,
        operation: impl FnOnce(&mut ChoreographyAdmissionState) -> BuddyResult<T>,
    ) -> BuddyResult<T> {
        self.ensure_choreography_runtime_accepting()?;

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;

        operation(&mut admission)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_choreography_timeline_scheduler<T>(
        &self,
        operation: impl FnOnce(
            &mut ChoreographyAdmissionState,
            &mut PendingTimelineExecutionQueue,
        ) -> BuddyResult<T>,
    ) -> BuddyResult<T> {
        self.ensure_choreography_runtime_accepting()?;

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;
        let mut pending_timeline = self.choreography_pending_timeline.lock().map_err(|_| {
            BuddyError::Runtime("choreography pending timeline queue lock was poisoned".to_owned())
        })?;
        let mut pending_dev_fixture =
            self.choreography_pending_dev_fixture.lock().map_err(|_| {
                BuddyError::Runtime(
                    "choreography pending dev fixture queue lock was poisoned".to_owned(),
                )
            })?;

        let result = operation(&mut admission, &mut pending_timeline);
        remove_dev_fixture_bodies_replaced_by_timeline(&pending_timeline, &mut pending_dev_fixture);
        result
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_choreography_dev_fixture_scheduler<T>(
        &self,
        operation: impl FnOnce(
            &mut ChoreographyAdmissionState,
            &mut PendingDevFixtureExecutionQueue,
        ) -> BuddyResult<T>,
    ) -> BuddyResult<T> {
        self.ensure_choreography_runtime_accepting()?;

        let mut admission = self.choreography_admission.lock().map_err(|_| {
            BuddyError::Runtime("choreography admission state lock was poisoned".to_owned())
        })?;
        let mut pending_timeline = self.choreography_pending_timeline.lock().map_err(|_| {
            BuddyError::Runtime("choreography pending timeline queue lock was poisoned".to_owned())
        })?;
        let mut pending_dev_fixture =
            self.choreography_pending_dev_fixture.lock().map_err(|_| {
                BuddyError::Runtime(
                    "choreography pending dev fixture queue lock was poisoned".to_owned(),
                )
            })?;

        let result = operation(&mut admission, &mut pending_dev_fixture);
        remove_timeline_bodies_replaced_by_dev_fixture(&mut pending_timeline, &pending_dev_fixture);
        result
    }

    pub(crate) fn choreography_runtime_readiness_snapshot(
        &self,
    ) -> BuddyResult<ChoreographyRuntimeReadinessSnapshot> {
        let readiness = self.choreography_readiness.lock().map_err(|_| {
            BuddyError::Runtime("choreography readiness state lock was poisoned".to_owned())
        })?;

        Ok(readiness.snapshot())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn mark_choreography_runtime_degraded(
        &self,
        reason_code: impl Into<String>,
        degraded_at: impl Into<String>,
    ) -> BuddyResult<ChoreographyRuntimeReadinessSnapshot> {
        let mut readiness = self.choreography_readiness.lock().map_err(|_| {
            BuddyError::Runtime("choreography readiness state lock was poisoned".to_owned())
        })?;

        Ok(readiness.mark_degraded(reason_code, degraded_at))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn mark_choreography_runtime_ready(
        &self,
        recovered_at: impl Into<String>,
    ) -> BuddyResult<ChoreographyRuntimeReadinessSnapshot> {
        let mut readiness = self.choreography_readiness.lock().map_err(|_| {
            BuddyError::Runtime("choreography readiness state lock was poisoned".to_owned())
        })?;

        Ok(readiness.mark_ready(recovered_at))
    }

    fn ensure_choreography_runtime_accepting(&self) -> BuddyResult<()> {
        let snapshot = self.choreography_runtime_readiness_snapshot()?;
        if snapshot.accepting_choreography {
            return Ok(());
        }

        let reason = snapshot
            .reason_code
            .as_deref()
            .unwrap_or("runtime.degraded");
        Err(BuddyError::Runtime(format!(
            "choreography runtime is degraded: {reason}"
        )))
    }

    pub fn create_session(&self, request: CreateBuddySessionRequest) -> BuddyResult<BuddySession> {
        self.storage.create_session(request)
    }

    pub fn create_conversation(
        &self,
        request: CreateBuddyConversationRequest,
    ) -> BuddyResult<BuddyConversation> {
        self.storage.create_conversation(request)
    }

    pub fn find_conversation(&self, id: &str) -> BuddyResult<BuddyConversation> {
        self.storage.find_conversation(id)
    }

    pub fn delete_session(&self, id: String) -> BuddyResult<bool> {
        self.storage.delete_session(id)
    }

    pub fn create_message(&self, request: CreateBuddyMessageRequest) -> BuddyResult<BuddyMessage> {
        self.storage.create_message(request)
    }

    pub fn append_conversation_message(
        &self,
        request: AppendBuddyConversationMessageRequest,
    ) -> BuddyResult<BuddyMessage> {
        self.storage.append_conversation_message(request)
    }

    pub fn create_attachment(
        &self,
        request: CreateBuddyRegisteredAttachmentRequest,
    ) -> BuddyResult<BuddyRegisteredAttachment> {
        self.storage.create_attachment(request)
    }

    pub fn find_attachment(&self, id: &str) -> BuddyResult<Option<BuddyRegisteredAttachment>> {
        self.storage.find_attachment(id)
    }

    pub fn read_setting_json(&self, key: &str) -> BuddyResult<Option<BuddySetting>> {
        self.storage.read_setting_json(key)
    }

    pub fn upsert_project(&self, request: UpsertBuddyProjectRequest) -> BuddyResult<BuddyProject> {
        self.storage.upsert_project(request)
    }

    pub fn find_project(&self, root: &str) -> BuddyResult<Option<BuddyProject>> {
        self.storage.find_project(root)
    }

    pub fn create_conversation_run(
        &self,
        request: CreateBuddyConversationRunRequest,
    ) -> BuddyResult<BuddyRun> {
        self.storage.create_conversation_run(request)
    }

    pub fn find_approval(&self, approval_id: String) -> BuddyResult<BuddyApproval> {
        self.storage.find_approval(approval_id)
    }

    pub fn resolve_codex_app_server_request_approval(
        &self,
        approval_id: String,
        status: BuddyApprovalTerminalStatus,
    ) -> BuddyResult<BuddyResolvedCodexAppServerRequestApproval> {
        self.storage
            .resolve_codex_app_server_request_approval(approval_id, status)
    }
}

struct StaleChoreographyPendingBodiesCleanup {
    cleared_body_count: usize,
    recoverable_pending_executions: Vec<RecoverableChoreographyPendingExecution>,
}

impl StaleChoreographyPendingBodiesCleanup {
    fn recoverable_pending_admission_count(&self) -> usize {
        self.recoverable_pending_executions.len()
    }
}

fn rebuild_and_clear_stale_choreography_pending_bodies_after_startup(
    storage: &BuddyStorage,
) -> BuddyResult<StaleChoreographyPendingBodiesCleanup> {
    if let Err(error) = storage.rebuild_choreography_pending_execution_body_cache_from_action_log()
    {
        eprintln!("lexora buddy pending body cache rebuild failed: {error}");
    }

    let recoverable_pending_executions =
        match storage.list_recoverable_choreography_pending_executions_after_startup() {
            Ok(executions) => executions,
            Err(error) => {
                eprintln!("lexora buddy recoverable pending execution scan failed: {error}");
                Vec::new()
            }
        };
    let cleared_body_count = storage.clear_choreography_pending_execution_bodies()?;

    Ok(StaleChoreographyPendingBodiesCleanup {
        cleared_body_count,
        recoverable_pending_executions,
    })
}

fn append_action_log_index_sync_failed_diagnostic(
    storage: &BuddyStorage,
    error_message: &str,
) -> BuddyResult<()> {
    let event = ActionLogSystemEvent::action_log_index_sync_failed(
        format!("evt_{}", uuid::Uuid::now_v7()),
        error_message,
        LocalLogTimestamp::now_utc().to_rfc3339_millis(),
    );

    storage.append_choreography_action_log_unindexed_system_event(&event)
}

fn append_stale_choreography_pending_bodies_cleared_diagnostic(
    storage: &BuddyStorage,
    cleanup: &StaleChoreographyPendingBodiesCleanup,
) -> BuddyResult<()> {
    let event = ActionLogSystemEvent::choreography_scheduler_stale_pending_bodies_cleared(
        format!("evt_{}", uuid::Uuid::now_v7()),
        cleanup.cleared_body_count,
        cleanup.recoverable_pending_admission_count(),
        LocalLogTimestamp::now_utc().to_rfc3339_millis(),
    );

    storage.append_choreography_action_log_system_event(&event)
}

fn startup_recoverable_choreography_pending_execution_summary(
    storage: &BuddyStorage,
    entry: &RecoverableChoreographyPendingExecution,
    admission_is_idle: bool,
    runtime_accepting_choreography: bool,
    action_log_index_status: StartupRecoverableReplayActionLogIndexStatus,
    current_unix_seconds: u64,
    local_interaction_is_active: bool,
) -> StartupRecoverableChoreographyPendingExecutionSummary {
    let source_ref_kind = action_log_source_ref_kind(&entry.admission.source_ref);
    let source_ref_id = action_log_source_ref_primary_id(&entry.admission.source_ref);
    let recovery_source_plan_available = startup_recoverable_recovery_source_plan_available(
        storage,
        &source_ref_kind,
        &source_ref_id,
    );
    let replay_policy =
        evaluate_startup_recoverable_replay_policy(StartupRecoverableReplayPolicyInput {
            action_log_index_status,
            trigger_source: entry.admission.trigger_source,
            source_ref_kind: &source_ref_kind,
            source_ref_id: source_ref_id.as_deref(),
            recovery_source_plan_available,
            body_kind: entry.admission.body_kind,
            body_schema_version: entry.admission.body_schema_version,
            plan_age_seconds: startup_recoverable_plan_age_seconds(
                &entry.admission.deferred_at,
                current_unix_seconds,
            ),
            runtime_accepting_choreography,
            admission_is_idle,
            local_interaction_is_active,
        });

    StartupRecoverableChoreographyPendingExecutionSummary {
        plan_id: entry.admission.plan_id.clone(),
        source_ref_kind,
        source_ref_id,
        trigger_source: entry.admission.trigger_source,
        priority: entry.admission.priority,
        reason_code: entry.admission.reason_code.clone(),
        body_kind: entry.admission.body_kind,
        body_schema_version: entry.admission.body_schema_version,
        replay_policy,
        deferred_event_id: entry.admission.deferred_event_id.clone(),
        deferred_at: entry.admission.deferred_at.clone(),
        stored_event_id: entry.body.stored_event_id.clone(),
        stored_at: entry.body.stored_at.clone(),
    }
}

pub(crate) fn startup_recoverable_local_interaction_is_active() -> bool {
    query_native_pet_local_interaction_active()
        .ok()
        .flatten()
        .unwrap_or(false)
}

fn startup_recoverable_current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn startup_recoverable_plan_age_seconds(
    deferred_at: &str,
    current_unix_seconds: u64,
) -> Option<u64> {
    let deferred_unix_seconds = parse_rfc3339_utc_seconds(deferred_at)?;
    current_unix_seconds.checked_sub(deferred_unix_seconds)
}

fn startup_recoverable_replay_action_log_index_status(
    health: ChoreographyActionLogIndexHealth,
) -> StartupRecoverableReplayActionLogIndexStatus {
    match health {
        ChoreographyActionLogIndexHealth::Fresh => {
            StartupRecoverableReplayActionLogIndexStatus::Fresh
        }
        ChoreographyActionLogIndexHealth::Stale => {
            StartupRecoverableReplayActionLogIndexStatus::Stale
        }
        ChoreographyActionLogIndexHealth::Failed => {
            StartupRecoverableReplayActionLogIndexStatus::Failed
        }
    }
}

fn startup_recoverable_recovery_source_plan_available(
    storage: &BuddyStorage,
    source_ref_kind: &str,
    source_ref_id: &Option<String>,
) -> bool {
    if !matches!(source_ref_kind, "systemRecovery" | "macroFallback") {
        return true;
    }

    let Some(plan_id) = source_ref_id.as_deref() else {
        return false;
    };

    storage.get_action_log_plan_detail(plan_id).is_ok()
}

fn schedule_startup_recoverable_choreography_pending_execution_entry(
    storage: &BuddyStorage,
    admission: &mut ChoreographyAdmissionState,
    pending_timeline: &mut PendingTimelineExecutionQueue,
    pending_dev_fixture: &mut PendingDevFixtureExecutionQueue,
    entry: RecoverableChoreographyPendingExecution,
) -> BuddyResult<ScheduledChoreographyExecution> {
    match entry.body.body_kind {
        ChoreographyPendingExecutionBodyKind::Timeline => {
            let body = serde_json::from_value::<TimelinePendingExecutionBody>(entry.body.body)?;
            let request =
                TimelineAdmissionExecutionRequest::from_pending_body(storage.clone(), body)?;
            let scheduled =
                admit_timeline_plan_with_pending_queue(admission, pending_timeline, request)
                    .map_err(timeline_execution_error_to_buddy_error)?;
            Ok(ScheduledChoreographyExecution::Timeline(scheduled))
        }
        ChoreographyPendingExecutionBodyKind::DevFixture => {
            let body = serde_json::from_value::<DevFixturePendingExecutionBody>(entry.body.body)?;
            let request =
                DevFixtureAdmissionExecutionRequest::from_pending_body(storage.clone(), body)?;
            let scheduled =
                admit_dev_fixture_with_pending_queue(admission, pending_dev_fixture, request)
                    .map_err(dev_fixture_execution_error_to_buddy_error)?;
            Ok(ScheduledChoreographyExecution::DevFixture(scheduled))
        }
    }
}

fn ensure_next_pending_choreography_plan_has_execution_body_for_active_plan(
    storage: &BuddyStorage,
    admission: &ChoreographyAdmissionState,
    plan_id: &str,
    pending_timeline: &PendingTimelineExecutionQueue,
    pending_dev_fixture: &PendingDevFixtureExecutionQueue,
    phase: &str,
) -> BuddyResult<()> {
    if admission.active_plan_id() != Some(plan_id) {
        return Ok(());
    }

    let Some(pending_plan_id) = admission.next_pending_plan_id() else {
        return Ok(());
    };

    ensure_pending_choreography_plan_has_execution_body(
        storage,
        plan_id,
        pending_plan_id,
        pending_timeline,
        pending_dev_fixture,
        phase,
    )
}

fn ensure_pending_choreography_plan_has_execution_body(
    storage: &BuddyStorage,
    active_plan_id: &str,
    pending_plan_id: &str,
    pending_timeline: &PendingTimelineExecutionQueue,
    pending_dev_fixture: &PendingDevFixtureExecutionQueue,
    phase: &str,
) -> BuddyResult<()> {
    if pending_timeline.contains(pending_plan_id) || pending_dev_fixture.contains(pending_plan_id) {
        return Ok(());
    }

    append_pending_choreography_plan_execution_body_missing_diagnostic(
        storage,
        active_plan_id,
        pending_plan_id,
        phase,
    )?;

    Err(missing_pending_choreography_plan_execution_body_error(
        pending_plan_id,
    ))
}

fn remove_dev_fixture_bodies_replaced_by_timeline(
    pending_timeline: &PendingTimelineExecutionQueue,
    pending_dev_fixture: &mut PendingDevFixtureExecutionQueue,
) {
    for plan_id in pending_timeline.plan_ids() {
        pending_dev_fixture.remove_replaced(plan_id.as_str());
    }
}

fn remove_timeline_bodies_replaced_by_dev_fixture(
    pending_timeline: &mut PendingTimelineExecutionQueue,
    pending_dev_fixture: &PendingDevFixtureExecutionQueue,
) {
    for plan_id in pending_dev_fixture.plan_ids() {
        pending_timeline.remove_replaced(plan_id.as_str());
    }
}

fn append_pending_choreography_plan_execution_body_missing_diagnostic(
    storage: &BuddyStorage,
    active_plan_id: &str,
    pending_plan_id: &str,
    phase: &str,
) -> BuddyResult<()> {
    let event = ActionLogSystemEvent::choreography_scheduler_pending_body_missing(
        format!("evt_{}", uuid::Uuid::now_v7()),
        active_plan_id,
        pending_plan_id,
        phase,
        LocalLogTimestamp::now_utc().to_rfc3339_millis(),
    );

    storage.append_choreography_action_log_system_event(&event)
}

fn missing_pending_choreography_plan_execution_body_error(pending_plan_id: &str) -> BuddyError {
    BuddyError::Runtime(format!(
        "pending choreography plan {pending_plan_id} has no queued execution body"
    ))
}

fn schedule_released_pending_choreography_plan(
    admission: &mut ChoreographyAdmissionState,
    pending_timeline: &mut PendingTimelineExecutionQueue,
    pending_dev_fixture: &mut PendingDevFixtureExecutionQueue,
    release: ChoreographyAdmissionRelease,
) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
    let pending_plan_id = match &release {
        ChoreographyAdmissionRelease::ReleasedWithPending {
            pending_plan_id, ..
        } => pending_plan_id.clone(),
        _ => return Ok(None),
    };

    if let Some(scheduled) =
        admit_released_pending_timeline_plan(admission, pending_timeline, release.clone())
            .transpose()
            .map_err(timeline_execution_error_to_buddy_error)?
    {
        return Ok(Some(ScheduledChoreographyExecution::Timeline(scheduled)));
    }

    if let Some(scheduled) =
        admit_released_pending_dev_fixture(admission, pending_dev_fixture, release)
            .transpose()
            .map_err(dev_fixture_execution_error_to_buddy_error)?
    {
        return Ok(Some(ScheduledChoreographyExecution::DevFixture(scheduled)));
    }

    Err(missing_pending_choreography_plan_execution_body_error(
        pending_plan_id.as_str(),
    ))
}

fn timeline_execution_error_to_buddy_error(error: TimelineExecutionError) -> BuddyError {
    match error {
        TimelineExecutionError::ActionLog(error) | TimelineExecutionError::Execution(error) => {
            error
        }
    }
}

fn dev_fixture_execution_error_to_buddy_error(error: DevFixtureExecutionError) -> BuddyError {
    match error {
        DevFixtureExecutionError::ActionLog(error) | DevFixtureExecutionError::Execution(error) => {
            error
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::app_paths::BuddyAppPaths;
    use crate::choreography::action_log::ActionLogEvent;
    use crate::choreography::admission::{
        ChoreographyAdmissionDecision, ChoreographyAdmissionRequest, ChoreographyPlanPriority,
        ChoreographyTriggerSource,
    };
    use crate::choreography::affective::ResolveContext;
    use crate::choreography::executor::{
        admit_dev_fixture_with_pending_queue, execute_released_pending_timeline_plan,
        execute_timeline_plan_with_admission_and_pending_queue, ChoreographyStepExecutor,
        DevFixtureAdmissionExecutionRequest, DevFixtureExecutionContext, DevFixtureKind,
        TimelineAdmissionExecutionRequest, TimelineExecutionContext, TimelineExecutionError,
    };
    use crate::choreography::registry::StepResolution;
    use crate::choreography::replay_policy::StartupRecoverableReplayPolicyDecision;
    use crate::choreography::timeline::{
        MoveByPathStep, MoveTarget, MoveToStep, PlayActionStep, TimelineFailurePolicy,
        TimelinePlan, TimelineStep, WaitStep,
    };
    use crate::native_pet::step_protocol::SidecarInterruptPolicy;
    use crate::storage::{
        ActionLogPlanListRequest, ActionLogSystemEventQueryRequest,
        ChoreographyPendingExecutionBodyKind, UpsertChoreographyPendingExecutionBodyRequest,
    };

    #[derive(Default)]
    struct FakeStepExecutor {
        executed_targets: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for FakeStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target = match &step.target {
                MoveTarget::Center => "center",
                MoveTarget::Home => "home",
                _ => "other",
            };
            self.executed_targets.borrow_mut().push(target.to_owned());
            Ok(())
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, _step: &WaitStep) -> BuddyResult<()> {
            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    fn timeline_execution_error_to_buddy_error(error: TimelineExecutionError) -> BuddyError {
        match error {
            TimelineExecutionError::ActionLog(error) | TimelineExecutionError::Execution(error) => {
                error
            }
        }
    }

    #[test]
    fn initialize_creates_memory_workspace_files() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-memory-workspace-{}",
            uuid::Uuid::new_v4()
        ));

        BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
            .expect("initialize state");

        assert!(data_dir.join("memories/global/MEMORY.md").is_file());
        assert!(data_dir.join("memories/global/memory_summary.md").is_file());
        assert!(data_dir.join("memories/global/raw_memories.md").is_file());
        assert!(data_dir.join("memories/global/rollout_summaries").is_dir());
        assert!(data_dir.join("memories/projects").is_dir());

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_logs_action_log_index_sync_failure_without_advancing_watermark() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-action-log-index-sync-failed-{}",
            uuid::Uuid::new_v4()
        ));
        let action_log_dir = data_dir.join("action-log");
        std::fs::create_dir_all(&action_log_dir).expect("create action log dir");
        std::fs::write(
            action_log_dir.join("events.jsonl"),
            r#"{"eventId":"evt_state_index_sync_schema_v2","schemaVersion":2,"eventType":"plan.started","status":"started","reasonCode":"devFixture.started","planId":"plan_state_index_sync_schema_v2","stepId":null,"sourceRef":{"kind":"devFixture","fixtureName":"index-sync"},"triggerSource":"devFixture","payload":{},"createdAt":"2026-07-09T10:00:00.000Z"}"#,
        )
        .expect("write unsupported action log JSONL");

        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let system_events = state
            .storage_handle()
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("actionLogIndex.syncFailed".to_owned()),
                source_ref_kind: Some("actionLogIndex".to_owned()),
                reason_code: Some("actionLogIndex.syncFailed".to_owned()),
                status: Some("degraded".to_owned()),
                limit: Some(10),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query action log index sync failure event");

        assert!(system_events.index_stale);
        assert_eq!(system_events.index_status, "failed");
        assert_eq!(system_events.last_indexed_at, None);
        assert_eq!(system_events.items.len(), 1);
        assert_eq!(
            system_events.items[0].event_type,
            "actionLogIndex.syncFailed"
        );
        assert_eq!(system_events.items[0].source_ref.kind, "actionLogIndex");
        assert_eq!(system_events.items[0].trigger_source, "actionLogIndex");
        assert_eq!(system_events.items[0].plan_id, None);
        assert_eq!(system_events.items[0].step_id, None);

        let action_log_content =
            std::fs::read_to_string(action_log_dir.join("events.jsonl")).expect("read action log");
        let lines = action_log_content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let diagnostic_json =
            serde_json::from_str::<serde_json::Value>(lines[1]).expect("parse diagnostic line");
        assert_eq!(
            diagnostic_json.get("eventType"),
            Some(&serde_json::json!("actionLogIndex.syncFailed"))
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_interrupts_stale_running_action_log_plan_after_restart() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-action-log-stale-running-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let action_log_dir = data_dir.join("action-log");
        std::fs::create_dir_all(&action_log_dir).expect("create action log dir");
        std::fs::write(
            action_log_dir.join("events.jsonl"),
            r#"{"eventId":"evt_state_stale_running_plan_started","schemaVersion":1,"eventType":"plan.started","status":"started","reasonCode":"timeline.started","planId":"plan_state_stale_running_after_restart","stepId":null,"sourceRef":{"kind":"devFixture","fixtureName":"stale-running"},"triggerSource":"userRequested","payload":{"sourceRef":{"kind":"devFixture","fixtureName":"stale-running"},"failurePolicy":"abort","stepCount":1},"createdAt":"2026-07-09T10:00:00.000Z"}"#,
        )
        .expect("write running action log JSONL");

        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let plans = state
            .storage_handle()
            .list_action_log_plans(ActionLogPlanListRequest {
                plan_id: Some("plan_state_stale_running_after_restart".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list reconciled action log plans");
        let plan = plans.items.first().expect("reconciled plan summary");

        assert_eq!(plan.status, "interrupted");
        assert_eq!(plan.result_kind, "interrupted");
        assert_eq!(plan.last_event_type, "plan.interrupted");
        assert_eq!(plan.last_reason_code, "runtime.restarted");
        assert_eq!(plan.detail_status, "interrupted");
        assert_eq!(plan.detail_reason_code, "runtime.restarted");
        assert!(plan.completed_at.is_some());

        let action_log_content =
            std::fs::read_to_string(action_log_dir.join("events.jsonl")).expect("read action log");
        let lines = action_log_content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let recovery_json =
            serde_json::from_str::<serde_json::Value>(lines[1]).expect("parse recovery line");
        assert_eq!(
            recovery_json.get("eventType"),
            Some(&serde_json::json!("plan.interrupted"))
        );
        assert_eq!(
            recovery_json.get("reasonCode"),
            Some(&serde_json::json!("runtime.restarted"))
        );
        assert_eq!(
            recovery_json.get("triggerSource"),
            Some(&serde_json::json!("startupSystem"))
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_interrupts_stale_running_action_log_steps_after_restart() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-action-log-stale-running-step-{}",
            uuid::Uuid::new_v4()
        ));
        let action_log_dir = data_dir.join("action-log");
        std::fs::create_dir_all(&action_log_dir).expect("create action log dir");
        std::fs::write(
            action_log_dir.join("events.jsonl"),
            [
                r#"{"eventId":"evt_state_stale_running_step_plan_started","schemaVersion":1,"eventType":"plan.started","status":"started","reasonCode":"timeline.started","planId":"plan_state_stale_running_step_after_restart","stepId":null,"sourceRef":{"kind":"devFixture","fixtureName":"stale-running-step"},"triggerSource":"userRequested","payload":{"sourceRef":{"kind":"devFixture","fixtureName":"stale-running-step"},"failurePolicy":"abort","stepCount":1},"createdAt":"2026-07-09T10:00:00.000Z"}"#,
                r#"{"eventId":"evt_state_stale_running_step_resolved","schemaVersion":1,"eventType":"step.resolved","status":"resolved","reasonCode":"timeline.stepResolved","planId":"plan_state_stale_running_step_after_restart","stepId":"step_state_stale_running_after_restart","sourceRef":{"kind":"devFixture","fixtureName":"stale-running-step"},"triggerSource":"userRequested","payload":{"stepKind":"playAction","actionId":"celebrate","animationRef":"celebrate"},"createdAt":"2026-07-09T10:00:01.000Z"}"#,
            ]
            .join("\n"),
        )
        .expect("write running action log JSONL");

        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let detail = state
            .storage_handle()
            .get_action_log_plan_detail("plan_state_stale_running_step_after_restart")
            .expect("get reconciled plan detail");

        assert_eq!(detail.plan.status, "interrupted");
        assert_eq!(detail.steps.len(), 1);
        assert_eq!(
            detail.steps[0].step_id,
            "step_state_stale_running_after_restart"
        );
        assert_eq!(detail.steps[0].status, "interrupted");
        assert_eq!(detail.steps[0].reason_code, "runtime.restarted");
        assert_eq!(detail.steps[0].event_count, 2);

        let action_log_content =
            std::fs::read_to_string(action_log_dir.join("events.jsonl")).expect("read action log");
        let lines = action_log_content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 4);
        let step_json =
            serde_json::from_str::<serde_json::Value>(lines[2]).expect("parse step recovery line");
        assert_eq!(
            step_json.get("eventType"),
            Some(&serde_json::json!("step.interrupted"))
        );
        assert_eq!(
            step_json.get("stepId"),
            Some(&serde_json::json!("step_state_stale_running_after_restart"))
        );
        assert_eq!(
            step_json.get("reasonCode"),
            Some(&serde_json::json!("runtime.restarted"))
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_interrupts_stale_action_log_steps_in_event_order_after_restart() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-action-log-stale-step-order-{}",
            uuid::Uuid::new_v4()
        ));
        let action_log_dir = data_dir.join("action-log");
        std::fs::create_dir_all(&action_log_dir).expect("create action log dir");
        std::fs::write(
            action_log_dir.join("events.jsonl"),
            [
                r#"{"eventId":"evt_state_stale_step_order_plan_started","schemaVersion":1,"eventType":"plan.started","status":"started","reasonCode":"timeline.started","planId":"plan_state_stale_step_order_after_restart","stepId":null,"sourceRef":{"kind":"devFixture","fixtureName":"stale-step-order"},"triggerSource":"userRequested","payload":{"sourceRef":{"kind":"devFixture","fixtureName":"stale-step-order"},"failurePolicy":"abort","stepCount":2},"createdAt":"2026-07-09T10:00:00.000Z"}"#,
                r#"{"eventId":"evt_state_stale_step_order_z_resolved","schemaVersion":1,"eventType":"step.resolved","status":"resolved","reasonCode":"timeline.stepResolved","planId":"plan_state_stale_step_order_after_restart","stepId":"step_z_event_order_first","sourceRef":{"kind":"devFixture","fixtureName":"stale-step-order"},"triggerSource":"userRequested","payload":{"stepKind":"playAction","actionId":"celebrate","animationRef":"celebrate"},"createdAt":"2026-07-09T10:00:01.000Z"}"#,
                r#"{"eventId":"evt_state_stale_step_order_a_resolved","schemaVersion":1,"eventType":"step.resolved","status":"resolved","reasonCode":"timeline.stepResolved","planId":"plan_state_stale_step_order_after_restart","stepId":"step_a_event_order_second","sourceRef":{"kind":"devFixture","fixtureName":"stale-step-order"},"triggerSource":"userRequested","payload":{"stepKind":"playAction","actionId":"happy_idle","animationRef":"happy_idle"},"createdAt":"2026-07-09T10:00:02.000Z"}"#,
            ]
            .join("\n"),
        )
        .expect("write running action log JSONL");

        let _state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");

        let action_log_content =
            std::fs::read_to_string(action_log_dir.join("events.jsonl")).expect("read action log");
        let lines = action_log_content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 6);
        let first_step_json = serde_json::from_str::<serde_json::Value>(lines[3])
            .expect("parse first step recovery line");
        let second_step_json = serde_json::from_str::<serde_json::Value>(lines[4])
            .expect("parse second step recovery line");
        let plan_json =
            serde_json::from_str::<serde_json::Value>(lines[5]).expect("parse plan recovery line");

        assert_eq!(
            first_step_json.get("stepId"),
            Some(&serde_json::json!("step_z_event_order_first"))
        );
        assert_eq!(
            second_step_json.get("stepId"),
            Some(&serde_json::json!("step_a_event_order_second"))
        );
        assert_eq!(
            plan_json.get("eventType"),
            Some(&serde_json::json!("plan.interrupted"))
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_clears_stale_choreography_pending_execution_bodies_after_restart() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-stale-pending-body-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_stale_pending_body_after_restart";
        storage.initialize().expect("initialize storage");
        storage
            .upsert_choreography_pending_execution_body(
                UpsertChoreographyPendingExecutionBodyRequest {
                    plan_id: plan_id.to_owned(),
                    body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                    schema_version: 1,
                    body: serde_json::json!({
                        "schemaVersion": 1,
                        "plan": {
                            "planId": plan_id,
                            "sourceRef": {
                                "kind": "devFixture",
                                "fixtureName": "stale-pending-body"
                            },
                            "failurePolicy": "abort",
                            "steps": [],
                            "createdAt": "2026-07-12T00:00:00.000Z"
                        },
                        "context": TimelineExecutionContext::fixed_for_test(),
                        "resolveContext": ResolveContext::default(),
                        "triggerSource": "userRequested"
                    }),
                },
            )
            .expect("store stale pending body");

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");

        assert!(state
            .storage_handle()
            .find_choreography_pending_execution_body(plan_id)
            .expect("find stale pending body after startup")
            .is_none());

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_logs_stale_choreography_pending_execution_bodies_cleared_after_restart() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-stale-pending-body-cleared-event-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_stale_pending_body_cleared_event";
        storage.initialize().expect("initialize storage");
        storage
            .upsert_choreography_pending_execution_body(
                UpsertChoreographyPendingExecutionBodyRequest {
                    plan_id: plan_id.to_owned(),
                    body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                    schema_version: 1,
                    body: serde_json::json!({
                        "schemaVersion": 1,
                        "plan": {
                            "planId": plan_id,
                            "sourceRef": {
                                "kind": "devFixture",
                                "fixtureName": "stale-pending-body-cleared-event"
                            },
                            "failurePolicy": "abort",
                            "steps": [],
                            "createdAt": "2026-07-12T00:00:00.000Z"
                        },
                        "context": TimelineExecutionContext::fixed_for_test(),
                        "resolveContext": ResolveContext::default(),
                        "triggerSource": "userRequested"
                    }),
                },
            )
            .expect("store stale pending body");

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let system_events = state
            .storage_handle()
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("choreographyScheduler.stalePendingBodiesCleared".to_owned()),
                source_ref_kind: Some("choreographyScheduler".to_owned()),
                reason_code: Some("runtime.restarted".to_owned()),
                status: Some("completed".to_owned()),
                limit: Some(10),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query stale pending body cleared system event");

        assert_eq!(system_events.items.len(), 1);
        assert_eq!(
            system_events.items[0].event_type,
            "choreographyScheduler.stalePendingBodiesCleared"
        );
        assert_eq!(
            system_events.items[0].source_ref.kind,
            "choreographyScheduler"
        );
        assert_eq!(
            system_events.items[0].trigger_source,
            "choreographyScheduler"
        );
        assert_eq!(system_events.items[0].plan_id, None);
        assert_eq!(system_events.items[0].step_id, None);

        let action_log_content =
            std::fs::read_to_string(data_dir.join("action-log").join("events.jsonl"))
                .expect("read action log");
        let lines = action_log_content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let event_json =
            serde_json::from_str::<serde_json::Value>(lines[1]).expect("parse system event line");
        assert_eq!(
            event_json.get("eventType"),
            Some(&serde_json::json!(
                "choreographyScheduler.stalePendingBodiesCleared"
            ))
        );
        assert_eq!(
            event_json
                .get("payload")
                .and_then(|payload| payload.get("clearedBodyCount")),
            Some(&serde_json::json!(1))
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_rebuilds_pending_body_cache_from_jsonl_before_stale_cleanup() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-rebuild-pending-body-before-cleanup-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_rebuild_pending_body_before_cleanup";
        storage.initialize().expect("initialize storage");
        storage
            .upsert_choreography_pending_execution_body(
                UpsertChoreographyPendingExecutionBodyRequest {
                    plan_id: plan_id.to_owned(),
                    body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                    schema_version: 1,
                    body: serde_json::json!({
                        "schemaVersion": 1,
                        "plan": {
                            "planId": plan_id,
                            "sourceRef": {
                                "kind": "devFixture",
                                "fixtureName": "rebuild-pending-body-before-cleanup"
                            },
                            "failurePolicy": "abort",
                            "steps": [],
                            "createdAt": "2026-07-13T00:00:00.000Z"
                        },
                        "context": TimelineExecutionContext::fixed_for_test(),
                        "resolveContext": ResolveContext::default(),
                        "triggerSource": "userRequested"
                    }),
                },
            )
            .expect("store pending body fact");
        assert_eq!(
            storage
                .clear_choreography_pending_execution_bodies()
                .expect("drop sqlite cache before restart"),
            1
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");

        assert!(state
            .storage_handle()
            .find_choreography_pending_execution_body(plan_id)
            .expect("find stale pending body after cleanup")
            .is_none());
        let system_events = state
            .storage_handle()
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("choreographyScheduler.stalePendingBodiesCleared".to_owned()),
                source_ref_kind: Some("choreographyScheduler".to_owned()),
                reason_code: Some("runtime.restarted".to_owned()),
                status: Some("completed".to_owned()),
                limit: Some(10),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query stale pending body cleared system event");

        assert_eq!(system_events.items.len(), 1);
        let action_log_content =
            std::fs::read_to_string(data_dir.join("action-log").join("events.jsonl"))
                .expect("read action log");
        let lines = action_log_content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let event_json =
            serde_json::from_str::<serde_json::Value>(lines[1]).expect("parse system event line");
        assert_eq!(
            event_json
                .get("payload")
                .and_then(|payload| payload.get("clearedBodyCount")),
            Some(&serde_json::json!(1))
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn initialize_logs_recoverable_pending_admission_count_before_stale_cleanup() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-admission-cleanup-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_pending_admission_cleanup";
        let active_plan_id = "plan_state_recoverable_active_plan";
        let active_step_id = "step_state_recoverable_active_step";
        let source_ref = serde_json::json!({
            "kind": "devFixture",
            "fixtureName": "recoverable-pending-admission-cleanup"
        });
        storage.initialize().expect("initialize storage");
        storage
            .upsert_choreography_pending_execution_body(
                UpsertChoreographyPendingExecutionBodyRequest {
                    plan_id: plan_id.to_owned(),
                    body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                    schema_version: 1,
                    body: serde_json::json!({
                        "schemaVersion": 1,
                        "plan": {
                            "planId": plan_id,
                            "sourceRef": source_ref,
                            "failurePolicy": "abort",
                            "steps": [],
                            "createdAt": "2026-07-13T00:00:00.000Z"
                        },
                        "context": TimelineExecutionContext::fixed_for_test(),
                        "resolveContext": ResolveContext::default(),
                        "triggerSource": "userRequested"
                    }),
                },
            )
            .expect("store pending body fact");
        storage
            .append_choreography_action_log_event(
                &ActionLogEvent::executor_admission_decision_for_source(
                    "evt_state_recoverable_pending_admission",
                    plan_id,
                    &source_ref,
                    ChoreographyTriggerSource::UserRequested.action_log_value(),
                    &ChoreographyAdmissionDecision::Deferred {
                        plan_id: plan_id.to_owned(),
                        trigger_source: ChoreographyTriggerSource::UserRequested,
                        priority: ChoreographyPlanPriority::UserRequested,
                        active_plan_id: active_plan_id.to_owned(),
                        active_step_id: Some(active_step_id.to_owned()),
                        active_priority: ChoreographyPlanPriority::AiChoreography,
                        active_step_interrupt_policy: SidecarInterruptPolicy::FinishStep,
                        reason_code: "admission.waitingForActiveStepToFinish".to_owned(),
                    },
                    "2026-07-13T00:05:00.000Z",
                ),
            )
            .expect("append deferred admission event");
        assert_eq!(
            storage
                .clear_choreography_pending_execution_bodies()
                .expect("drop sqlite cache before restart"),
            1
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");

        assert!(state
            .storage_handle()
            .find_choreography_pending_execution_body(plan_id)
            .expect("find stale pending body after cleanup")
            .is_none());
        let action_log_content =
            std::fs::read_to_string(data_dir.join("action-log").join("events.jsonl"))
                .expect("read action log");
        let cleanup_event_json = action_log_content
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|event| {
                event.get("eventType")
                    == Some(&serde_json::json!(
                        "choreographyScheduler.stalePendingBodiesCleared"
                    ))
            })
            .expect("find stale pending cleanup action log event");

        assert_eq!(
            cleanup_event_json
                .get("payload")
                .and_then(|payload| payload.get("recoverablePendingAdmissionCount")),
            Some(&serde_json::json!(1))
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    fn seed_recoverable_pending_timeline_execution(
        storage: &BuddyStorage,
        plan_id: &str,
        active_plan_id: &str,
        fixture_name: &str,
        deferred_event_id: &str,
        active_step_id: &str,
    ) {
        let source_ref = serde_json::json!({
            "kind": "devFixture",
            "fixtureName": fixture_name
        });
        seed_recoverable_pending_timeline_execution_with_source_ref(
            storage,
            plan_id,
            active_plan_id,
            &source_ref,
            deferred_event_id,
            active_step_id,
        )
    }

    fn seed_recoverable_pending_timeline_execution_with_source_ref(
        storage: &BuddyStorage,
        plan_id: &str,
        active_plan_id: &str,
        source_ref: &serde_json::Value,
        deferred_event_id: &str,
        active_step_id: &str,
    ) {
        seed_recoverable_pending_timeline_execution_with_source_ref_and_deferred_at(
            storage,
            plan_id,
            active_plan_id,
            source_ref,
            deferred_event_id,
            active_step_id,
            &LocalLogTimestamp::now_utc().to_rfc3339_millis(),
        )
    }

    fn seed_recoverable_pending_timeline_execution_with_source_ref_and_deferred_at(
        storage: &BuddyStorage,
        plan_id: &str,
        active_plan_id: &str,
        source_ref: &serde_json::Value,
        deferred_event_id: &str,
        active_step_id: &str,
        deferred_at: &str,
    ) {
        storage
            .upsert_choreography_pending_execution_body(
                UpsertChoreographyPendingExecutionBodyRequest {
                    plan_id: plan_id.to_owned(),
                    body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                    schema_version: 1,
                    body: serde_json::json!({
                        "schemaVersion": 1,
                        "plan": {
                            "planId": plan_id,
                            "sourceRef": source_ref,
                            "failurePolicy": "abort",
                            "steps": [],
                            "createdAt": "2026-07-13T00:00:00.000Z"
                        },
                        "context": TimelineExecutionContext::fixed_for_test(),
                        "resolveContext": ResolveContext::default(),
                        "triggerSource": "userRequested"
                    }),
                },
            )
            .expect("store pending body fact");
        storage
            .append_choreography_action_log_event(
                &ActionLogEvent::executor_admission_decision_for_source(
                    deferred_event_id,
                    plan_id,
                    source_ref,
                    ChoreographyTriggerSource::UserRequested.action_log_value(),
                    &ChoreographyAdmissionDecision::Deferred {
                        plan_id: plan_id.to_owned(),
                        trigger_source: ChoreographyTriggerSource::UserRequested,
                        priority: ChoreographyPlanPriority::UserRequested,
                        active_plan_id: active_plan_id.to_owned(),
                        active_step_id: Some(active_step_id.to_owned()),
                        active_priority: ChoreographyPlanPriority::AiChoreography,
                        active_step_interrupt_policy: SidecarInterruptPolicy::FinishStep,
                        reason_code: "admission.waitingForActiveStepToFinish".to_owned(),
                    },
                    deferred_at,
                ),
            )
            .expect("append deferred admission event");
        storage
            .clear_choreography_pending_execution_bodies()
            .expect("drop sqlite cache before restart");
    }

    #[test]
    fn initialize_keeps_recoverable_pending_execution_isolated_from_live_admission() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-isolated-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_pending_execution";
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution(
            &storage,
            plan_id,
            "plan_state_recoverable_active_execution",
            "recoverable-pending-execution-isolated",
            "evt_state_recoverable_pending_execution",
            "step_state_recoverable_active_execution",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");

        assert_eq!(
            state
                .startup_recoverable_choreography_pending_count()
                .expect("read startup recoverable count"),
            1
        );
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![plan_id.to_owned()]
        );
        assert!(state
            .storage_handle()
            .find_choreography_pending_execution_body(plan_id)
            .expect("find live pending body after startup")
            .is_none());
        assert!(matches!(
            state
                .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                    "plan_state_new_after_recoverable_startup",
                    ChoreographyTriggerSource::UserRequested,
                ))
                .expect("admit new plan after startup"),
            ChoreographyAdmissionDecision::Accepted { .. }
        ));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_summary_uses_approval_id_as_source_ref_id() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-approval-source-ref-id-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_approval_source_ref_id";
        let approval_id = "approval_state_recoverable_source_ref_id";
        let source_ref = serde_json::json!({
            "kind": "approval",
            "approvalId": approval_id
        });
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution_with_source_ref(
            &storage,
            plan_id,
            "plan_state_recoverable_approval_active",
            &source_ref,
            "evt_state_recoverable_approval_source_ref_id",
            "step_state_recoverable_approval_active",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let summaries = state
            .startup_recoverable_choreography_pending_summaries_with_local_interaction_status(false)
            .expect("list startup recoverable summaries");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].source_ref_kind, "approval");
        assert_eq!(summaries[0].source_ref_id.as_deref(), Some(approval_id));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_summary_uses_preset_behavior_id_as_source_ref_id() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-preset-source-ref-id-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_preset_source_ref_id";
        let preset_behavior_id = "throw_after_drag";
        let source_ref = serde_json::json!({
            "kind": "presetBehavior",
            "presetBehaviorId": preset_behavior_id,
            "interactionId": "interaction_state_recoverable_preset"
        });
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution_with_source_ref(
            &storage,
            plan_id,
            "plan_state_recoverable_preset_active",
            &source_ref,
            "evt_state_recoverable_preset_source_ref_id",
            "step_state_recoverable_preset_active",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let summaries = state
            .startup_recoverable_choreography_pending_summaries_with_local_interaction_status(false)
            .expect("list startup recoverable summaries");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].source_ref_kind, "presetBehavior");
        assert_eq!(
            summaries[0].source_ref_id.as_deref(),
            Some(preset_behavior_id)
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_summary_uses_system_recovery_triggered_plan_as_source_ref_id() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-system-recovery-source-ref-id-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_system_recovery_source_ref_id";
        let triggered_by_plan_id = "plan_state_recoverable_system_recovery_trigger";
        let source_ref = serde_json::json!({
            "kind": "systemRecovery",
            "triggeredByPlanId": triggered_by_plan_id,
            "triggerReason": "runtime.systemRecoveryFailed"
        });
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution_with_source_ref(
            &storage,
            plan_id,
            "plan_state_recoverable_system_recovery_active",
            &source_ref,
            "evt_state_recoverable_system_recovery_source_ref_id",
            "step_state_recoverable_system_recovery_active",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let summaries = state
            .startup_recoverable_choreography_pending_summaries_with_local_interaction_status(false)
            .expect("list startup recoverable summaries");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].source_ref_kind, "systemRecovery");
        assert_eq!(
            summaries[0].source_ref_id.as_deref(),
            Some(triggered_by_plan_id)
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_summary_uses_macro_fallback_triggered_plan_as_source_ref_id() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-macro-fallback-source-ref-id-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_macro_fallback_source_ref_id";
        let triggered_by_plan_id = "plan_state_recoverable_macro_fallback_trigger";
        let source_ref = serde_json::json!({
            "kind": "macroFallback",
            "triggeredByPlanId": triggered_by_plan_id,
            "triggeredByStepId": "step_state_recoverable_macro_fallback_trigger",
            "triggerReason": "semanticFallback.windowAnchorTargetUnavailable",
            "originalMacroId": "peekBehindWindow",
            "fallbackMacroId": "peekFromEdge"
        });
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution_with_source_ref(
            &storage,
            plan_id,
            "plan_state_recoverable_macro_fallback_active",
            &source_ref,
            "evt_state_recoverable_macro_fallback_source_ref_id",
            "step_state_recoverable_macro_fallback_active",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let summaries = state
            .startup_recoverable_choreography_pending_summaries_with_local_interaction_status(false)
            .expect("list startup recoverable summaries");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].source_ref_kind, "macroFallback");
        assert_eq!(
            summaries[0].source_ref_id.as_deref(),
            Some(triggered_by_plan_id)
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_summary_rejects_recovery_source_without_indexed_original_plan() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-missing-recovery-source-plan-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let source_ref = serde_json::json!({
            "kind": "systemRecovery",
            "triggeredByPlanId": "plan_state_recoverable_missing_recovery_source",
            "triggerReason": "runtime.systemRecoveryFailed"
        });
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution_with_source_ref(
            &storage,
            "plan_state_recoverable_recovery_source_unavailable",
            "plan_state_recoverable_recovery_source_unavailable_active",
            &source_ref,
            "evt_state_recoverable_recovery_source_unavailable",
            "step_state_recoverable_recovery_source_unavailable_active",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let summaries = state
            .startup_recoverable_choreography_pending_summaries_with_local_interaction_status(false)
            .expect("list startup recoverable summaries");

        assert_eq!(
            summaries[0].replay_policy.decision,
            StartupRecoverableReplayPolicyDecision::Reject
        );
        assert_eq!(
            summaries[0].replay_policy.reason_code,
            "replay.recoverySourcePlanUnavailable"
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_pending_execution_can_be_scheduled_when_idle() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-schedule-idle-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_pending_execution_schedule_idle";
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution(
            &storage,
            plan_id,
            "plan_state_recoverable_active_execution_schedule_idle",
            "recoverable-pending-execution-schedule-idle",
            "evt_state_recoverable_pending_execution_schedule_idle",
            "step_state_recoverable_active_execution_schedule_idle",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let scheduled = state
            .schedule_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
                plan_id, false,
            )
            .expect("schedule startup recoverable execution")
            .expect("recoverable execution should be scheduled");

        match scheduled {
            ScheduledChoreographyExecution::Timeline(scheduled) => {
                assert_eq!(scheduled.plan_id, plan_id);
                assert!(matches!(
                    scheduled.decision,
                    ChoreographyAdmissionDecision::Accepted { .. }
                ));
                assert!(scheduled.execution.is_some());
            }
            ScheduledChoreographyExecution::DevFixture(_) => {
                panic!("timeline recoverable execution should not schedule a dev fixture")
            }
        }
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_count()
                .expect("read startup recoverable count"),
            0
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_pending_execution_waits_when_local_interaction_is_active() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-local-interaction-active-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_pending_execution_local_interaction_active";
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution(
            &storage,
            plan_id,
            "plan_state_recoverable_active_execution_local_interaction_active",
            "recoverable-pending-execution-local-interaction-active",
            "evt_state_recoverable_pending_execution_local_interaction_active",
            "step_state_recoverable_active_execution_local_interaction_active",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let scheduled = state
            .schedule_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
                plan_id,
                true,
            )
            .expect("try schedule startup recoverable execution during local interaction");

        assert!(scheduled.is_none());
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_pending_execution_rejects_policy_rejected_entry_without_consuming_it() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-policy-rejected-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_pending_execution_policy_rejected";
        let source_ref = serde_json::json!({
            "kind": "conversationMessage",
            "conversationId": "conversation_state_recoverable_policy_rejected",
            "messageId": "message_state_recoverable_policy_rejected"
        });
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution_with_source_ref_and_deferred_at(
            &storage,
            plan_id,
            "plan_state_recoverable_active_execution_policy_rejected",
            &source_ref,
            "evt_state_recoverable_pending_execution_policy_rejected",
            "step_state_recoverable_active_execution_policy_rejected",
            "1970-01-01T00:00:00.000Z",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let scheduled = state
            .schedule_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
                plan_id,
                false,
            )
            .expect("try schedule policy-rejected startup recoverable execution");

        assert!(scheduled.is_none());
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn next_startup_recoverable_pending_execution_schedules_first_entry_when_idle() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-next-idle-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let first_plan_id = "plan_state_recoverable_pending_execution_next_first";
        let second_plan_id = "plan_state_recoverable_pending_execution_next_second";
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution(
            &storage,
            first_plan_id,
            "plan_state_recoverable_active_execution_next_first",
            "recoverable-pending-execution-next-first",
            "evt_state_recoverable_pending_execution_next_first",
            "step_state_recoverable_active_execution_next_first",
        );
        seed_recoverable_pending_timeline_execution(
            &storage,
            second_plan_id,
            "plan_state_recoverable_active_execution_next_second",
            "recoverable-pending-execution-next-second",
            "evt_state_recoverable_pending_execution_next_second",
            "step_state_recoverable_active_execution_next_second",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let scheduled = state
            .schedule_next_startup_recoverable_choreography_pending_execution_with_local_interaction_status(false)
            .expect("schedule next startup recoverable execution")
            .expect("next recoverable execution should be scheduled");

        match scheduled {
            ScheduledChoreographyExecution::Timeline(scheduled) => {
                assert_eq!(scheduled.plan_id, first_plan_id);
                assert!(matches!(
                    scheduled.decision,
                    ChoreographyAdmissionDecision::Accepted { .. }
                ));
                assert!(scheduled.execution.is_some());
            }
            ScheduledChoreographyExecution::DevFixture(_) => {
                panic!("timeline recoverable execution should not schedule a dev fixture")
            }
        }
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![second_plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn next_startup_recoverable_pending_execution_skips_rejected_entry_without_consuming_it() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-next-skips-rejected-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let rejected_plan_id = "plan_state_recoverable_pending_execution_next_rejected_first";
        let eligible_plan_id = "plan_state_recoverable_pending_execution_next_eligible_second";
        let rejected_source_ref = serde_json::json!({
            "kind": "conversationMessage",
            "conversationId": "conversation_state_recoverable_next_rejected",
            "messageId": "message_state_recoverable_next_rejected"
        });
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution_with_source_ref_and_deferred_at(
            &storage,
            rejected_plan_id,
            "plan_state_recoverable_active_execution_next_rejected",
            &rejected_source_ref,
            "evt_state_recoverable_pending_execution_next_rejected",
            "step_state_recoverable_active_execution_next_rejected",
            "1970-01-01T00:00:00.000Z",
        );
        seed_recoverable_pending_timeline_execution(
            &storage,
            eligible_plan_id,
            "plan_state_recoverable_active_execution_next_eligible",
            "recoverable-pending-execution-next-eligible",
            "evt_state_recoverable_pending_execution_next_eligible",
            "step_state_recoverable_active_execution_next_eligible",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        let scheduled = state
            .schedule_next_startup_recoverable_choreography_pending_execution_with_local_interaction_status(false)
            .expect("schedule next eligible startup recoverable execution")
            .expect("eligible recoverable execution should be scheduled");

        match scheduled {
            ScheduledChoreographyExecution::Timeline(scheduled) => {
                assert_eq!(scheduled.plan_id, eligible_plan_id);
            }
            ScheduledChoreographyExecution::DevFixture(_) => {
                panic!("timeline recoverable execution should not schedule a dev fixture")
            }
        }
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![rejected_plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn next_startup_recoverable_pending_execution_waits_when_runtime_is_busy() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-next-busy-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_pending_execution_next_busy";
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution(
            &storage,
            plan_id,
            "plan_state_recoverable_active_execution_next_busy",
            "recoverable-pending-execution-next-busy",
            "evt_state_recoverable_pending_execution_next_busy",
            "step_state_recoverable_active_execution_next_busy",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_state_busy_before_next_recoverable_startup",
                ChoreographyTriggerSource::AiChoreography,
            ))
            .expect("admit active plan after startup");

        let scheduled = state
            .schedule_next_startup_recoverable_choreography_pending_execution_with_local_interaction_status(false)
            .expect("try schedule next startup recoverable execution while busy");

        assert!(scheduled.is_none());
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_pending_execution_waits_when_runtime_is_busy() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-recoverable-pending-execution-waits-busy-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let plan_id = "plan_state_recoverable_pending_execution_waits_busy";
        storage.initialize().expect("initialize storage");
        seed_recoverable_pending_timeline_execution(
            &storage,
            plan_id,
            "plan_state_recoverable_active_execution_waits_busy",
            "recoverable-pending-execution-waits-busy",
            "evt_state_recoverable_pending_execution_waits_busy",
            "step_state_recoverable_active_execution_waits_busy",
        );

        let state =
            BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart");
        state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_state_busy_after_recoverable_startup",
                ChoreographyTriggerSource::AiChoreography,
            ))
            .expect("admit active plan after startup");

        let scheduled = state
            .schedule_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
                plan_id, false,
            )
            .expect("try schedule startup recoverable execution while busy");

        assert!(scheduled.is_none());
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn app_state_clones_share_choreography_admission_state() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-choreography-admission-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let cloned_state = state.clone();

        let accepted = state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_active_ai",
                ChoreographyTriggerSource::AiChoreography,
            ))
            .expect("admit active plan");
        let rejected = cloned_state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_next_ai",
                ChoreographyTriggerSource::AiChoreography,
            ))
            .expect("admit competing plan");

        assert!(matches!(
            accepted,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert!(matches!(
            rejected,
            ChoreographyAdmissionDecision::Rejected { .. }
        ));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn app_state_choreography_readiness_blocks_admission_until_health_recovers() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-choreography-readiness-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let cloned_state = state.clone();

        state
            .mark_choreography_runtime_degraded(
                "runtime.systemRecoveryFailed",
                "2026-07-09T10:00:00.000Z",
            )
            .expect("mark runtime degraded");
        let degraded = cloned_state
            .choreography_runtime_readiness_snapshot()
            .expect("read runtime readiness");
        let rejected = cloned_state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_blocked_while_degraded",
                ChoreographyTriggerSource::AiChoreography,
            ))
            .expect_err("degraded runtime should reject admission");

        assert_eq!(degraded.status.as_str(), "degraded");
        assert!(!degraded.accepting_choreography);
        assert_eq!(
            degraded.reason_code.as_deref(),
            Some("runtime.systemRecoveryFailed")
        );
        assert!(rejected
            .to_string()
            .contains("choreography runtime is degraded"));

        cloned_state
            .mark_choreography_runtime_ready("2026-07-09T10:00:05.000Z")
            .expect("mark runtime ready");
        let ready = state
            .choreography_runtime_readiness_snapshot()
            .expect("read recovered runtime readiness");
        let accepted = state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_after_recovery",
                ChoreographyTriggerSource::AiChoreography,
            ))
            .expect("admit after recovery");

        assert_eq!(ready.status.as_str(), "ready");
        assert!(ready.accepting_choreography);
        assert_eq!(ready.reason_code, None);
        assert!(matches!(
            accepted,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn app_state_choreography_scheduler_flushes_released_pending_timeline_plan() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-choreography-scheduler-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let executor = FakeStepExecutor::default();
        let storage = state.storage_handle();
        let active_plan_id = "plan_state_active_019f5b00-0000-7000-8000-000000000101";
        let pending_plan_id = "plan_state_pending_019f5b00-0000-7000-8000-000000000102";
        state
            .admit_choreography_plan(
                ChoreographyAdmissionRequest::new(
                    active_plan_id,
                    ChoreographyTriggerSource::AiChoreography,
                )
                .with_active_step(
                    "step_state_active_019f5b00-0000-7000-8000-000000000201",
                    SidecarInterruptPolicy::FinishStep,
                ),
            )
            .expect("admit active plan");
        let pending_plan = TimelinePlan {
            plan_id: pending_plan_id.to_owned(),
            source_ref: serde_json::json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5b00-0000-7000-8000-000000000202",
                "messageId": "message_019f5b00-0000-7000-8000-000000000302",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                "step_state_pending_019f5b00-0000-7000-8000-000000000501",
                30_000,
            ))],
            created_at: "2026-07-12T00:00:00.000Z".to_owned(),
        };

        let queued = state
            .with_choreography_timeline_scheduler(|admission, pending_queue| {
                execute_timeline_plan_with_admission_and_pending_queue(
                    &executor,
                    admission,
                    pending_queue,
                    TimelineAdmissionExecutionRequest::new(
                        storage.clone(),
                        pending_plan,
                        TimelineExecutionContext::fixed_for_test(),
                        ResolveContext::default(),
                        ChoreographyTriggerSource::UserRequested,
                    ),
                )
                .map_err(timeline_execution_error_to_buddy_error)
            })
            .expect("queue pending plan");
        let stored_pending_body = storage
            .find_choreography_pending_execution_body(pending_plan_id)
            .expect("find stored pending body")
            .expect("pending body should be stored");
        assert_eq!(
            stored_pending_body.body_kind,
            ChoreographyPendingExecutionBodyKind::Timeline
        );
        assert_eq!(stored_pending_body.schema_version, 1);

        let release = state
            .release_choreography_plan(active_plan_id)
            .expect("release active plan");
        let promoted = state
            .with_choreography_timeline_scheduler(|admission, pending_queue| {
                execute_released_pending_timeline_plan(&executor, admission, pending_queue, release)
                    .expect("pending plan should exist")
                    .map_err(timeline_execution_error_to_buddy_error)
            })
            .expect("flush pending plan");

        assert!(!queued.executed);
        assert!(promoted.executed);
        assert_eq!(
            executor.executed_targets.into_inner(),
            vec!["center".to_owned()]
        );
        assert!(storage
            .find_choreography_pending_execution_body(pending_plan_id)
            .expect("find consumed pending body")
            .is_none());

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn app_state_choreography_scheduler_promotes_pending_during_release_before_new_admission() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-choreography-release-promote-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let executor = FakeStepExecutor::default();
        let storage = state.storage_handle();
        let active_plan_id = "plan_state_active_019f5b00-0000-7000-8000-000000000111";
        let pending_plan_id = "plan_state_pending_019f5b00-0000-7000-8000-000000000112";
        state
            .admit_choreography_plan(
                ChoreographyAdmissionRequest::new(
                    active_plan_id,
                    ChoreographyTriggerSource::AiChoreography,
                )
                .with_active_step(
                    "step_state_active_019f5b00-0000-7000-8000-000000000211",
                    SidecarInterruptPolicy::FinishStep,
                ),
            )
            .expect("admit active plan");
        let pending_plan = TimelinePlan {
            plan_id: pending_plan_id.to_owned(),
            source_ref: serde_json::json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5b00-0000-7000-8000-000000000212",
                "messageId": "message_019f5b00-0000-7000-8000-000000000312",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                "step_state_pending_019f5b00-0000-7000-8000-000000000511",
                30_000,
            ))],
            created_at: "2026-07-12T00:00:00.000Z".to_owned(),
        };

        let queued = state
            .with_choreography_timeline_scheduler(|admission, pending_queue| {
                execute_timeline_plan_with_admission_and_pending_queue(
                    &executor,
                    admission,
                    pending_queue,
                    TimelineAdmissionExecutionRequest::new(
                        storage.clone(),
                        pending_plan,
                        TimelineExecutionContext::fixed_for_test(),
                        ResolveContext::default(),
                        ChoreographyTriggerSource::UserRequested,
                    ),
                )
                .map_err(timeline_execution_error_to_buddy_error)
            })
            .expect("queue pending plan");
        let release = state
            .release_choreography_plan_and_schedule_pending(active_plan_id)
            .expect("release active plan and schedule pending");
        let competing = state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_state_competing_019f5b00-0000-7000-8000-000000000113",
                ChoreographyTriggerSource::UserRequested,
            ))
            .expect("admit competing plan after release");

        assert!(!queued.executed);
        match release.scheduled {
            Some(ScheduledChoreographyExecution::Timeline(scheduled)) => {
                assert_eq!(scheduled.plan_id, pending_plan_id);
                assert!(scheduled.execution.is_some());
            }
            Some(ScheduledChoreographyExecution::DevFixture(_)) => {
                panic!("pending timeline plan should not schedule dev fixture")
            }
            None => panic!("pending plan should be scheduled while releasing active plan"),
        }
        assert!(matches!(
            competing,
            ChoreographyAdmissionDecision::Rejected { active_plan_id, .. }
                if active_plan_id == pending_plan_id
        ));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn app_state_choreography_scheduler_replaces_stale_timeline_body_when_pending_type_changes() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-choreography-replace-pending-body-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let executor = FakeStepExecutor::default();
        let storage = state.storage_handle();
        let active_plan_id = "plan_state_active_019f5b00-0000-7000-8000-000000000141";
        let pending_plan_id = "plan_state_pending_019f5b00-0000-7000-8000-000000000142";
        state
            .admit_choreography_plan(
                ChoreographyAdmissionRequest::new(
                    active_plan_id,
                    ChoreographyTriggerSource::AiChoreography,
                )
                .with_active_step(
                    "step_state_active_019f5b00-0000-7000-8000-000000000241",
                    SidecarInterruptPolicy::FinishStep,
                ),
            )
            .expect("admit active plan");
        let pending_timeline_plan = TimelinePlan {
            plan_id: pending_plan_id.to_owned(),
            source_ref: serde_json::json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5b00-0000-7000-8000-000000000242",
                "messageId": "message_019f5b00-0000-7000-8000-000000000342",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                "step_state_stale_timeline_019f5b00-0000-7000-8000-000000000541",
                30_000,
            ))],
            created_at: "2026-07-12T00:00:00.000Z".to_owned(),
        };

        let queued_timeline = state
            .with_choreography_timeline_scheduler(|admission, pending_queue| {
                execute_timeline_plan_with_admission_and_pending_queue(
                    &executor,
                    admission,
                    pending_queue,
                    TimelineAdmissionExecutionRequest::new(
                        storage.clone(),
                        pending_timeline_plan,
                        TimelineExecutionContext::fixed_for_test(),
                        ResolveContext::default(),
                        ChoreographyTriggerSource::UserRequested,
                    ),
                )
                .map_err(timeline_execution_error_to_buddy_error)
            })
            .expect("queue pending timeline");
        let mut pending_dev_fixture_context = DevFixtureExecutionContext::fixed_for_test();
        pending_dev_fixture_context.plan_id = pending_plan_id.to_owned();
        let queued_dev_fixture = state
            .with_choreography_dev_fixture_scheduler(|admission, pending_queue| {
                admit_dev_fixture_with_pending_queue(
                    admission,
                    pending_queue,
                    DevFixtureAdmissionExecutionRequest::new(
                        storage.clone(),
                        pending_dev_fixture_context,
                        ResolveContext::default(),
                        DevFixtureKind::SinglePlayAction,
                        ChoreographyTriggerSource::UserRequested,
                    ),
                )
                .map_err(dev_fixture_execution_error_to_buddy_error)
            })
            .expect("replace pending body with dev fixture");
        let replaced_body = storage
            .find_choreography_pending_execution_body(pending_plan_id)
            .expect("find replaced pending body")
            .expect("replaced dev fixture body should stay durable");
        let replayable_replaced_body = storage
            .find_replayable_choreography_pending_execution_body_from_action_log(pending_plan_id)
            .expect("find replayable replaced pending body")
            .expect("replaced dev fixture body should remain replayable");
        let release = state
            .release_choreography_plan_and_schedule_pending(active_plan_id)
            .expect("release active plan and schedule replaced pending body");

        assert!(!queued_timeline.executed);
        assert!(queued_dev_fixture.execution.is_none());
        assert_eq!(
            replaced_body.body_kind,
            ChoreographyPendingExecutionBodyKind::DevFixture
        );
        assert_eq!(
            replayable_replaced_body.body_kind,
            ChoreographyPendingExecutionBodyKind::DevFixture
        );
        match release.scheduled {
            Some(ScheduledChoreographyExecution::DevFixture(scheduled)) => {
                assert_eq!(scheduled.plan_id, pending_plan_id);
                assert!(scheduled.execution.is_some());
            }
            Some(ScheduledChoreographyExecution::Timeline(_)) => {
                panic!("stale timeline body must not win after dev fixture replacement")
            }
            None => panic!("replaced pending dev fixture should be scheduled"),
        }

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn app_state_choreography_scheduler_rejects_release_when_pending_body_is_missing() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-choreography-missing-pending-body-release-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let active_plan_id = "plan_state_active_019f5b00-0000-7000-8000-000000000121";
        let pending_plan_id = "plan_state_pending_019f5b00-0000-7000-8000-000000000122";

        state
            .admit_choreography_plan(
                ChoreographyAdmissionRequest::new(
                    active_plan_id,
                    ChoreographyTriggerSource::AiChoreography,
                )
                .with_active_step(
                    "step_state_active_019f5b00-0000-7000-8000-000000000221",
                    SidecarInterruptPolicy::FinishStep,
                ),
            )
            .expect("admit active plan");
        let deferred = state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                pending_plan_id,
                ChoreographyTriggerSource::UserRequested,
            ))
            .expect("defer pending plan");

        let error = match state.release_choreography_plan_and_schedule_pending(active_plan_id) {
            Ok(_) => panic!("missing pending body should fail release scheduling"),
            Err(error) => error,
        };
        let (active_after_error, pending_after_error) = state
            .with_choreography_admission(|admission| {
                Ok((
                    admission.active_plan_id().map(str::to_owned),
                    admission.next_pending_plan_id().map(str::to_owned),
                ))
            })
            .expect("read admission state");

        assert!(matches!(
            deferred,
            ChoreographyAdmissionDecision::Deferred { plan_id, .. } if plan_id == pending_plan_id
        ));
        assert!(error.to_string().contains(
            "pending choreography plan plan_state_pending_019f5b00-0000-7000-8000-000000000122 has no queued execution body"
        ));
        assert_eq!(active_after_error.as_deref(), Some(active_plan_id));
        assert_eq!(pending_after_error.as_deref(), Some(pending_plan_id));
        let system_events = state
            .storage_handle()
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("choreographyScheduler.pendingBodyMissing".to_owned()),
                source_ref_kind: Some("choreographyScheduler".to_owned()),
                reason_code: Some("choreographyScheduler.pendingBodyMissing".to_owned()),
                status: Some("degraded".to_owned()),
                limit: Some(10),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query missing pending body system event");

        assert_eq!(system_events.items.len(), 1);
        assert_eq!(
            system_events.items[0].event_type,
            "choreographyScheduler.pendingBodyMissing"
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn app_state_choreography_scheduler_rejects_step_completion_when_pending_body_is_missing() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-state-choreography-missing-pending-body-step-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let active_plan_id = "plan_state_active_019f5b00-0000-7000-8000-000000000131";
        let pending_plan_id = "plan_state_pending_019f5b00-0000-7000-8000-000000000132";

        state
            .admit_choreography_plan(
                ChoreographyAdmissionRequest::new(
                    active_plan_id,
                    ChoreographyTriggerSource::AiChoreography,
                )
                .with_active_step(
                    "step_state_active_019f5b00-0000-7000-8000-000000000231",
                    SidecarInterruptPolicy::FinishStep,
                ),
            )
            .expect("admit active plan");
        let deferred = state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                pending_plan_id,
                ChoreographyTriggerSource::UserRequested,
            ))
            .expect("defer pending plan");

        let error =
            match state.schedule_pending_after_completed_choreography_step(active_plan_id, None) {
                Ok(_) => panic!("missing pending body should fail step-completion scheduling"),
                Err(error) => error,
            };
        let (active_after_error, pending_after_error) = state
            .with_choreography_admission(|admission| {
                Ok((
                    admission.active_plan_id().map(str::to_owned),
                    admission.next_pending_plan_id().map(str::to_owned),
                ))
            })
            .expect("read admission state");

        assert!(matches!(
            deferred,
            ChoreographyAdmissionDecision::Deferred { plan_id, .. } if plan_id == pending_plan_id
        ));
        assert!(error.to_string().contains(
            "pending choreography plan plan_state_pending_019f5b00-0000-7000-8000-000000000132 has no queued execution body"
        ));
        assert_eq!(active_after_error.as_deref(), Some(active_plan_id));
        assert_eq!(pending_after_error.as_deref(), Some(pending_plan_id));
        let system_events = state
            .storage_handle()
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("choreographyScheduler.pendingBodyMissing".to_owned()),
                source_ref_kind: Some("choreographyScheduler".to_owned()),
                reason_code: Some("choreographyScheduler.pendingBodyMissing".to_owned()),
                status: Some("degraded".to_owned()),
                limit: Some(10),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query missing pending body system event");

        assert_eq!(system_events.items.len(), 1);
        assert_eq!(
            system_events.items[0].event_type,
            "choreographyScheduler.pendingBodyMissing"
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }
}
