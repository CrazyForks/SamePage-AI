use std::{cell::RefCell, collections::HashMap, sync::Arc, thread, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    error::{BuddyError, BuddyResult},
    local_log::LocalLogTimestamp,
    native_pet::{
        spawn_native_pet_sidecar,
        step_protocol::{
            execute_step_request, ExecuteStepPayload, ExecuteStepPlayback, ExecuteStepRequest,
            SidecarInterruptPolicy, SidecarStepErrorCode, SidecarStepResponse,
        },
        NativePetSidecarProcess,
    },
    storage::{
        BuddyStorage, ChoreographyPendingExecutionBodyKind,
        UpsertChoreographyPendingExecutionBodyRequest,
    },
};

use super::{
    action_log::{
        ActionLogEvent, ActionLogEventIds, ActionLogRestorePositionResolution, ActionLogSink,
        ActionLogSystemEvent, ActionLogTimelinePlanStats,
    },
    admission::{
        ChoreographyActiveStepUpdate, ChoreographyAdmissionDecision, ChoreographyAdmissionRelease,
        ChoreographyAdmissionRequest, ChoreographyAdmissionState, ChoreographyTriggerSource,
    },
    affective::ResolveContext,
    fixture::{
        create_ai_macro_demo_dev_fixture_plan, create_single_play_action_dev_fixture_plan,
        DevFixturePlan,
    },
    macro_plan::{
        compile_beat_plan_to_timeline_steps, compile_macro_intent_to_beat_plan,
        macro_fallback_policy, BeatPlanBuildContext, CelebrateMacroParams, CuriousMacroParams,
        LieDownMacroParams, MacroIntent, MacroSemanticFallback, MacroTimelineFailureFallback,
        PeekFromEdgeMacroParams, ReassureMacroParams, ThinkingMacroParams,
    },
    recovery::{
        create_runtime_safe_fallback_plan, RuntimeSafeFallbackPlan, RuntimeSafeFallbackPlanContext,
        RuntimeSafeFallbackReason,
    },
    registry::{ActionRegistry, StepResolution},
    step_resolution::{
        resolve_move_by_path_after_action, resolve_move_to_after_action, resolve_play_action_step,
    },
    timeline::{
        expand_planner_timeline_plan, MoveByPathStep, MoveTarget, MoveToStep, PlayActionStep,
        RecoverStep, ReplaceStep, RestorePositionStep, RetryStep, SnapshotPositionStep,
        TimelineFailurePolicy, TimelinePlan, TimelineStep, TryStep, WaitStep,
    },
};

pub(crate) trait ChoreographyStepExecutor {
    fn play_action_step(
        &self,
        step: &PlayActionStep,
        resolution: &StepResolution,
    ) -> BuddyResult<()>;
    fn move_to_step(&self, step: &MoveToStep, after_animation_ref: Option<&str>)
        -> BuddyResult<()>;
    fn move_by_path_step(
        &self,
        step: &MoveByPathStep,
        after_animation_ref: Option<&str>,
    ) -> BuddyResult<()>;
    fn wait_step(&self, step: &WaitStep) -> BuddyResult<()>;
    fn interrupt_step(&self, step_id: &str, reason_code: &str) -> BuddyResult<()>;
    fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>>;
}

#[derive(Clone)]
pub(crate) struct NativePetChoreographyStepExecutor {
    sidecar: Arc<NativePetSidecarProcess>,
}

impl NativePetChoreographyStepExecutor {
    pub(crate) fn from_shared_sidecar(sidecar: Arc<NativePetSidecarProcess>) -> Self {
        Self { sidecar }
    }

    pub(crate) fn spawn_sidecar() -> BuddyResult<Self> {
        spawn_native_pet_sidecar(|_| {})
            .map(Arc::new)
            .map(Self::from_shared_sidecar)
    }

    pub(crate) fn spawn_sidecar_with_startup_health_diagnostics(
        storage: &BuddyStorage,
    ) -> BuddyResult<Self> {
        spawn_sidecar_with_startup_health_diagnostics(storage, Self::spawn_sidecar)
    }
}

fn spawn_sidecar_with_startup_health_diagnostics<T>(
    storage: &BuddyStorage,
    spawn: impl FnOnce() -> BuddyResult<T>,
) -> BuddyResult<T> {
    match spawn() {
        Ok(executor) => Ok(executor),
        Err(error) => {
            append_startup_health_failed_diagnostic(storage, &error);
            Err(error)
        }
    }
}

fn append_startup_health_failed_diagnostic(storage: &BuddyStorage, error: &BuddyError) {
    let error_message = error.to_string();
    let event = ActionLogSystemEvent::startup_health_failed(
        prefixed_uuid_v7("evt"),
        error_message.as_str(),
        LocalLogTimestamp::now_utc().to_rfc3339_millis(),
    );
    if let Err(error) = storage.append_choreography_action_log_system_event(&event) {
        eprintln!("lexora buddy startup health diagnostic failed: {error}");
    }
}

impl ChoreographyStepExecutor for NativePetChoreographyStepExecutor {
    fn play_action_step(
        &self,
        step: &PlayActionStep,
        resolution: &StepResolution,
    ) -> BuddyResult<()> {
        self.sidecar.play_action_step(step, resolution)
    }

    fn move_to_step(
        &self,
        step: &MoveToStep,
        after_animation_ref: Option<&str>,
    ) -> BuddyResult<()> {
        self.sidecar.move_to_step(step, after_animation_ref)
    }

    fn move_by_path_step(
        &self,
        step: &MoveByPathStep,
        after_animation_ref: Option<&str>,
    ) -> BuddyResult<()> {
        self.sidecar.move_by_path_step(step, after_animation_ref)
    }

    fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
        execute_wait_step(step)
    }

    fn interrupt_step(&self, step_id: &str, reason_code: &str) -> BuddyResult<()> {
        self.sidecar.interrupt_step(step_id, reason_code)
    }

    fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
        self.sidecar.query_state_position()
    }
}

impl ChoreographyStepExecutor for NativePetSidecarProcess {
    fn play_action_step(
        &self,
        step: &PlayActionStep,
        resolution: &StepResolution,
    ) -> BuddyResult<()> {
        let response = self.execute_step(&play_action_execute_step_request(step, resolution))?;
        ensure_sidecar_step_completed(response)
    }

    fn move_to_step(
        &self,
        step: &MoveToStep,
        after_animation_ref: Option<&str>,
    ) -> BuddyResult<()> {
        let response =
            self.execute_step(&move_to_execute_step_request(step, after_animation_ref)?)?;
        ensure_sidecar_step_completed(response)
    }

    fn move_by_path_step(
        &self,
        step: &MoveByPathStep,
        after_animation_ref: Option<&str>,
    ) -> BuddyResult<()> {
        let response = self.execute_step(&move_by_path_execute_step_request(
            step,
            after_animation_ref,
        )?)?;
        ensure_sidecar_step_completed(response)
    }

    fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
        execute_wait_step(step)
    }

    fn interrupt_step(&self, step_id: &str, reason_code: &str) -> BuddyResult<()> {
        NativePetSidecarProcess::interrupt_step(self, step_id, reason_code)
    }

    fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
        let snapshot = self.query_state_snapshot()?;
        Ok(Some((snapshot.position.x, snapshot.position.y)))
    }
}

#[cfg(test)]
pub(crate) fn execute_timeline_step(
    executor: &impl ChoreographyStepExecutor,
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    step: &TimelineStep,
) -> BuddyResult<()> {
    let mut runtime_state = TimelineRuntimeState::default();
    execute_timeline_step_with_runtime_state(
        executor,
        registry,
        resolve_context,
        step,
        &mut runtime_state,
    )
}

#[cfg(test)]
fn execute_timeline_step_with_runtime_state(
    executor: &impl ChoreographyStepExecutor,
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    step: &TimelineStep,
    runtime_state: &mut TimelineRuntimeState,
) -> BuddyResult<()> {
    match step {
        TimelineStep::PlayAction(step) => {
            let resolution = resolve_play_action_step(registry, resolve_context, step)?;
            executor.play_action_step(step, &resolution)
        }
        TimelineStep::MoveTo(step) => {
            let after_action_resolution =
                resolve_move_to_after_action(registry, resolve_context, step)?;
            let after_animation_ref = after_action_resolution
                .as_ref()
                .map(|resolution| resolution.animation_ref.as_str());
            executor.move_to_step(step, after_animation_ref)
        }
        TimelineStep::MoveByPath(step) => {
            let after_action_resolution =
                resolve_move_by_path_after_action(registry, resolve_context, step)?;
            let after_animation_ref = after_action_resolution
                .as_ref()
                .map(|resolution| resolution.animation_ref.as_str());
            executor.move_by_path_step(step, after_animation_ref)
        }
        TimelineStep::Wait(step) => {
            validate_wait_step(step)?;
            executor.wait_step(step)
        }
        TimelineStep::Skip(_) => Ok(()),
        TimelineStep::SnapshotPosition(step) => {
            let position = executor
                .query_state_position()?
                .ok_or_else(|| snapshot_position_unavailable_error(step))?;
            runtime_state.save_position_snapshot(step.snapshot_id.as_str(), position);
            Ok(())
        }
        TimelineStep::RestorePosition(step) => {
            let move_to_step = runtime_state.restore_position_step(step)?;
            let after_action_resolution =
                resolve_move_to_after_action(registry, resolve_context, &move_to_step)?;
            let after_animation_ref = after_action_resolution
                .as_ref()
                .map(|resolution| resolution.animation_ref.as_str());
            executor.move_to_step(&move_to_step, after_animation_ref)
        }
        TimelineStep::Retry(step) => {
            let mut last_error = None;
            for _attempt_index in 0..step.max_attempts {
                let attempt_result = step.steps.iter().try_for_each(|step| {
                    execute_timeline_step_with_runtime_state(
                        executor,
                        registry,
                        resolve_context,
                        step,
                        runtime_state,
                    )
                });
                match attempt_result {
                    Ok(()) => return Ok(()),
                    Err(error) => last_error = Some(error),
                }
            }

            Err(last_error.unwrap_or_else(|| {
                BuddyError::Validation(format!(
                    "retry timeline step must contain at least one attempt: {}",
                    step.step_id
                ))
            }))
        }
        TimelineStep::Replace(step) => {
            if step
                .steps
                .iter()
                .try_for_each(|step| {
                    execute_timeline_step_with_runtime_state(
                        executor,
                        registry,
                        resolve_context,
                        step,
                        runtime_state,
                    )
                })
                .is_ok()
            {
                return Ok(());
            }

            for step in &step.replacement_steps {
                execute_timeline_step_with_runtime_state(
                    executor,
                    registry,
                    resolve_context,
                    step,
                    runtime_state,
                )?;
            }
            Ok(())
        }
        TimelineStep::Recover(step) => {
            if step
                .steps
                .iter()
                .try_for_each(|step| {
                    execute_timeline_step_with_runtime_state(
                        executor,
                        registry,
                        resolve_context,
                        step,
                        runtime_state,
                    )
                })
                .is_ok()
            {
                return Ok(());
            }

            for step in &step.recovery_steps {
                execute_timeline_step_with_runtime_state(
                    executor,
                    registry,
                    resolve_context,
                    step,
                    runtime_state,
                )?;
            }
            Ok(())
        }
        TimelineStep::Try(step) => {
            let primary_result = step.steps.iter().try_for_each(|step| {
                execute_timeline_step_with_runtime_state(
                    executor,
                    registry,
                    resolve_context,
                    step,
                    runtime_state,
                )
            });
            if primary_result.is_ok() {
                return Ok(());
            }

            for step in &step.fallback_steps {
                execute_timeline_step_with_runtime_state(
                    executor,
                    registry,
                    resolve_context,
                    step,
                    runtime_state,
                )?;
            }
            Ok(())
        }
        TimelineStep::Repeat(_) | TimelineStep::Choose(_) | TimelineStep::SetFallback(_) => {
            Err(planner_side_timeline_step_error(step))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct TimelineExecutionReport {
    pub(crate) completed_step_count: usize,
}

#[cfg(test)]
pub(crate) fn execute_timeline_steps(
    executor: &impl ChoreographyStepExecutor,
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    steps: &[TimelineStep],
) -> BuddyResult<TimelineExecutionReport> {
    let steps = super::timeline::expand_planner_timeline_steps(steps)?;
    let mut completed_step_count = 0;
    let mut runtime_state = TimelineRuntimeState::default();
    for step in &steps {
        execute_timeline_step_with_runtime_state(
            executor,
            registry,
            resolve_context,
            step,
            &mut runtime_state,
        )?;
        completed_step_count += 1;
    }

    Ok(TimelineExecutionReport {
        completed_step_count,
    })
}

fn move_to_execute_step_request(
    step: &MoveToStep,
    after_animation_ref: Option<&str>,
) -> BuddyResult<ExecuteStepRequest> {
    Ok(execute_step_request(
        step.step_id.clone(),
        ExecuteStepPayload::MoveTo {
            target: serde_json::to_value(&step.target)?,
            after: resolved_move_to_after_animation_ref(step, after_animation_ref)?,
            interrupt_policy: SidecarInterruptPolicy::Interruptible,
            timeout_ms: step.timeout_ms,
        },
    ))
}

fn move_by_path_execute_step_request(
    step: &MoveByPathStep,
    after_animation_ref: Option<&str>,
) -> BuddyResult<ExecuteStepRequest> {
    if step.path.is_empty() {
        return Err(BuddyError::Validation(
            "moveByPath requires at least one path target".to_owned(),
        ));
    }

    Ok(execute_step_request(
        step.step_id.clone(),
        ExecuteStepPayload::MoveByPath {
            path: step
                .path
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?,
            after: resolved_after_animation_ref(
                "moveByPath",
                step.after_action_id.as_deref(),
                after_animation_ref,
            )?,
            interrupt_policy: SidecarInterruptPolicy::Interruptible,
            timeout_ms: step.timeout_ms,
        },
    ))
}

fn play_action_execute_step_request(
    step: &PlayActionStep,
    resolution: &StepResolution,
) -> ExecuteStepRequest {
    execute_step_request(
        step.step_id.clone(),
        ExecuteStepPayload::PlayAction {
            animation: resolution.animation_ref.clone(),
            playback: execute_step_playback(resolution),
            interrupt_policy: resolution.interrupt_policy,
            completion_behavior: step.completion_behavior,
            timeout_ms: step.timeout_ms,
        },
    )
}

fn execute_step_playback(resolution: &StepResolution) -> ExecuteStepPlayback {
    if resolution.playback_kind == "loopForDuration" {
        return ExecuteStepPlayback::LoopForDuration {
            duration_ms: resolution.duration_ms,
            clip_duration_ms: resolution.clip_duration_ms,
        };
    }

    ExecuteStepPlayback::Once {
        duration_ms: resolution.duration_ms,
    }
}

fn resolved_move_to_after_animation_ref(
    step: &MoveToStep,
    after_animation_ref: Option<&str>,
) -> BuddyResult<Option<String>> {
    resolved_after_animation_ref(
        "moveTo",
        step.after_action_id.as_deref(),
        after_animation_ref,
    )
}

fn resolved_after_animation_ref(
    step_kind: &str,
    after_action_id: Option<&str>,
    after_animation_ref: Option<&str>,
) -> BuddyResult<Option<String>> {
    match (after_action_id, after_animation_ref) {
        (Some(_), Some(animation_ref)) => Ok(Some(animation_ref.to_owned())),
        (Some(after_action_id), None) => Err(BuddyError::Validation(format!(
            "{step_kind} afterActionId requires registry resolution before native dispatch: {after_action_id}"
        ))),
        (None, Some(_)) => Err(BuddyError::Validation(format!(
            "{step_kind} after animation requires afterActionId"
        ))),
        (None, None) => Ok(None),
    }
}

fn execute_wait_step(step: &WaitStep) -> BuddyResult<()> {
    validate_wait_step(step)?;
    thread::sleep(Duration::from_millis(step.duration_ms));
    Ok(())
}

fn validate_wait_step(step: &WaitStep) -> BuddyResult<()> {
    if step.duration_ms == 0 {
        return Err(BuddyError::Validation(
            "wait durationMs must be greater than 0".to_owned(),
        ));
    }

    if step.timeout_ms < step.duration_ms {
        return Err(BuddyError::Validation(
            "wait timeoutMs must be greater than or equal to durationMs".to_owned(),
        ));
    }

    Ok(())
}

fn planner_side_timeline_step_error(step: &TimelineStep) -> BuddyError {
    BuddyError::Validation(format!(
        "{} timeline step is planner-side and cannot execute directly: {}",
        step.kind(),
        step.step_id()
    ))
}

fn snapshot_position_unavailable_error(step: &SnapshotPositionStep) -> BuddyError {
    BuddyError::Runtime(format!(
        "timeline position snapshot is unavailable: {}",
        step.step_id
    ))
}

fn ensure_sidecar_step_completed(response: SidecarStepResponse) -> BuddyResult<()> {
    match response {
        SidecarStepResponse::StepCompleted(_) => Ok(()),
        SidecarStepResponse::StepFailed(response) => Err(BuddyError::Runtime(format!(
            "native pet step failed: {}: {}",
            response.code, response.message
        ))),
        SidecarStepResponse::StepInterrupted(response) => Err(BuddyError::Runtime(format!(
            "native pet step interrupted: {}",
            response.reason_code
        ))),
        SidecarStepResponse::ProtocolError(response)
            if response.code == SidecarStepErrorCode::UnsupportedStepCapability =>
        {
            Err(BuddyError::UnsupportedCapability {
                scope: "native pet executeStep".to_owned(),
                capability: unsupported_step_capability_from_message(response.message.as_str()),
            })
        }
        SidecarStepResponse::ProtocolError(response) => Err(BuddyError::Runtime(format!(
            "native pet step protocol error: {}: {}",
            response.code, response.message
        ))),
    }
}

fn unsupported_step_capability_from_message(message: &str) -> String {
    if message.contains("windowAnchor") {
        "windowAnchor".to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn runtime_safe_fallback_reason_for_execution_error(
    error: &BuddyError,
) -> RuntimeSafeFallbackReason {
    match error {
        BuddyError::UnsupportedCapability { .. } => {
            RuntimeSafeFallbackReason::UnsupportedStepCapability
        }
        BuddyError::Runtime(message) if message.contains("motionTimeout") => {
            RuntimeSafeFallbackReason::MotionTimeout
        }
        BuddyError::Runtime(message) if message.starts_with("native pet step interrupted") => {
            RuntimeSafeFallbackReason::StepInterrupted
        }
        BuddyError::Runtime(message) if message.starts_with("native pet step protocol error") => {
            RuntimeSafeFallbackReason::ProtocolError
        }
        BuddyError::Runtime(message) if message.starts_with("native pet step failed") => {
            RuntimeSafeFallbackReason::StepFailed
        }
        _ => RuntimeSafeFallbackReason::ExecutorError,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use serde_json::json;

    use super::{
        admit_timeline_plan_with_pending_queue, ensure_sidecar_step_completed,
        execute_admitted_timeline_plan, macro_semantic_fallback_intent,
        move_to_execute_step_request, play_action_execute_step_request,
        validate_pending_handoff_finalizer_interrupt_policies, ChoreographyStepExecutor,
        DevFixtureAdmissionExecutionRequest, DevFixtureExecutionContext, DevFixtureKind,
        MoveByPathStep, MoveToStep, PendingDevFixtureExecutionQueue, PendingTimelineExecutionQueue,
        PlayActionStep, StepResolution, TimelineAdmissionExecutionRequest,
        TimelineExecutionContext, TimelinePendingExecutionBody,
    };
    use crate::{
        app_paths::BuddyAppPaths,
        choreography::admission::{
            ChoreographyAdmissionRequest, ChoreographyAdmissionState, ChoreographyTriggerSource,
        },
        choreography::affective::ResolveContext,
        choreography::macro_plan::MacroIntent,
        choreography::registry::ActionRegistry,
        choreography::timeline::{
            RecoverStep, TimelineFailurePolicy, TimelinePlan, TimelineStep, WaitStep,
        },
        error::{BuddyError, BuddyResult},
        native_pet::step_protocol::{
            protocol_error_response_with_code, SidecarInterruptPolicy, SidecarStepErrorCode,
            SidecarStepResponse,
        },
        storage::{ActionLogSystemEventQueryRequest, BuddyStorage},
    };

    fn resolution(playback_kind: &str, duration_ms: u64, clip_duration_ms: u64) -> StepResolution {
        StepResolution {
            action_id: "celebrate".to_owned(),
            animation_ref: "celebrate".to_owned(),
            playback_kind: playback_kind.to_owned(),
            duration_ms,
            loop_animation: playback_kind == "loopForDuration",
            interrupt_policy: SidecarInterruptPolicy::Interruptible,
            resolved_from_registry_version: "test".to_owned(),
            fallback: None,
            clip_duration_ms,
        }
    }

    #[test]
    fn maps_play_action_runtime_failures_to_semantic_fallback_intents() {
        let failed_step_id = "failed-action";
        let failed_plan = TimelinePlan {
            plan_id: "failed-plan".to_owned(),
            source_ref: json!({ "kind": "conversationMessage" }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                failed_step_id,
                "celebrate",
                5_000,
            ))],
            created_at: "2026-07-16T00:00:00.000Z".to_owned(),
        };
        let error = BuddyError::Runtime("native pet step failed".to_owned());
        let cases = [
            (
                json!({ "macroId": "awaitApproval", "params": {} }),
                "thinking",
            ),
            (json!({ "macroId": "celebrate", "params": {} }), "reassure"),
            (json!({ "macroId": "cast", "params": {} }), "celebrate"),
            (json!({ "macroId": "curious", "params": {} }), "thinking"),
            (
                json!({ "macroId": "dance", "params": { "durationMs": 2_500 } }),
                "celebrate",
            ),
            (
                json!({ "macroId": "getUp", "params": { "side": "left" } }),
                "reassure",
            ),
            (json!({ "macroId": "reassure", "params": {} }), "lieDown"),
            (json!({ "macroId": "sad", "params": {} }), "reassure"),
            (json!({ "macroId": "thinking", "params": {} }), "curious"),
            (json!({ "macroId": "working", "params": {} }), "thinking"),
        ];

        for (intent_json, expected_fallback_macro_id) in cases {
            let intent =
                serde_json::from_value::<MacroIntent>(intent_json).expect("parse macro intent");
            let fallback =
                macro_semantic_fallback_intent(&intent, &failed_plan, failed_step_id, &error)
                    .expect("resolve semantic fallback");

            assert_eq!(fallback.intent.macro_id(), expected_fallback_macro_id);
            assert_eq!(
                fallback.semantic_fallback.fallback_macro_id(),
                expected_fallback_macro_id
            );
        }
    }

    #[test]
    fn pending_handoff_source_action_must_finish_before_interruption() {
        let registry = ActionRegistry::load_bundled().expect("load action registry");
        let mut source = PlayActionStep::once("source", "explain", 5_000);
        source.pending_handoff_finalizer_step_id = Some("finalizer".to_owned());
        let plan = TimelinePlan {
            plan_id: "plan_interruptible_handoff_source".to_owned(),
            source_ref: json!({ "kind": "devFixture" }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::PlayAction(source),
                TimelineStep::PlayAction(PlayActionStep::once("finalizer", "curious", 5_000)),
            ],
            created_at: "2026-07-16T00:00:00.000Z".to_owned(),
        };

        let error = validate_pending_handoff_finalizer_interrupt_policies(
            &registry,
            &ResolveContext::default(),
            &plan,
        )
        .expect_err("interruptible source should fail");

        assert!(error
            .to_string()
            .contains("pending handoff source action must finish before interruption: source"));
    }

    #[test]
    fn nested_pending_handoff_source_action_must_finish_before_interruption() {
        let registry = ActionRegistry::load_bundled().expect("load action registry");
        let mut source = PlayActionStep::once("source", "explain", 5_000);
        source.pending_handoff_finalizer_step_id = Some("finalizer".to_owned());
        let plan = TimelinePlan {
            plan_id: "plan_nested_interruptible_handoff_source".to_owned(),
            source_ref: json!({ "kind": "devFixture" }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::Recover(RecoverStep {
                step_id: "recover".to_owned(),
                kind: "recover".to_owned(),
                steps: vec![
                    TimelineStep::PlayAction(source),
                    TimelineStep::PlayAction(PlayActionStep::once("finalizer", "curious", 5_000)),
                ],
                recovery_steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "recover-idle",
                    "idle",
                    5_000,
                ))],
            })],
            created_at: "2026-07-16T00:00:00.000Z".to_owned(),
        };

        let error = validate_pending_handoff_finalizer_interrupt_policies(
            &registry,
            &ResolveContext::default(),
            &plan,
        )
        .expect_err("nested interruptible source should fail");

        assert!(error
            .to_string()
            .contains("pending handoff source action must finish before interruption: source"));
    }

    #[test]
    fn pending_handoff_finalizer_action_must_finish_before_interruption() {
        let registry = ActionRegistry::load_bundled().expect("load action registry");
        let mut source = PlayActionStep::once("source", "reassure", 5_000);
        source.pending_handoff_finalizer_step_id = Some("finalizer".to_owned());
        let plan = TimelinePlan {
            plan_id: "plan_interruptible_handoff_finalizer".to_owned(),
            source_ref: json!({ "kind": "devFixture" }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::PlayAction(source),
                TimelineStep::PlayAction(PlayActionStep::once("finalizer", "idle", 5_000)),
            ],
            created_at: "2026-07-16T00:00:00.000Z".to_owned(),
        };

        let error = validate_pending_handoff_finalizer_interrupt_policies(
            &registry,
            &ResolveContext::default(),
            &plan,
        )
        .expect_err("interruptible finalizer should fail");

        assert!(error.to_string().contains(
            "pending handoff finalizer action must finish before interruption: finalizer"
        ));
    }

    #[test]
    fn records_startup_health_failed_when_sidecar_spawn_fails() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        let error = super::spawn_sidecar_with_startup_health_diagnostics(&storage, || {
            Err::<(), _>(BuddyError::Runtime(
                "native pet sidecar ready timed out".to_owned(),
            ))
        })
        .expect_err("startup failure should return original error");

        assert!(
            matches!(error, BuddyError::Runtime(message) if message == "native pet sidecar ready timed out")
        );

        let events = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("startupHealth.failed".to_owned()),
                source_ref_kind: Some("runtime".to_owned()),
                reason_code: Some("startupHealth.nativePetUnavailable".to_owned()),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query startup health system events");

        assert_eq!(events.items.len(), 1);
        assert_eq!(events.items[0].event_type, "startupHealth.failed");
        assert_eq!(events.items[0].status, "failed");
        assert_eq!(
            events.items[0].reason_code,
            "startupHealth.nativePetUnavailable"
        );
        assert_eq!(events.items[0].trigger_source, "startupHealth");
        assert_eq!(events.items[0].source_ref.kind, "runtime");
    }

    #[test]
    fn play_action_execute_step_request_serializes_resolved_animation_for_sidecar_protocol() {
        let step = PlayActionStep::once(
            "step_019f4500-0000-7000-8000-000000000037",
            "celebrate",
            5_000,
        );

        let request = play_action_execute_step_request(&step, &resolution("once", 1_720, 1_720));

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4500-0000-7000-8000-000000000037",
                "type": "executeStep",
                "stepId": "step_019f4500-0000-7000-8000-000000000037",
                "step": {
                    "kind": "playAction",
                    "animation": "celebrate",
                    "playback": {
                        "kind": "once",
                        "durationMs": 1720
                    },
                    "interruptPolicy": "interruptible",
                    "timeoutMs": 5000
                }
            })
        );
    }

    #[test]
    fn move_to_execute_step_request_serializes_resolved_target_for_sidecar_protocol() {
        let mut step = MoveToStep::home("step_019f4500-0000-7000-8000-000000000036", 15_000);
        step.after_action_id = Some("sleep".to_owned());

        let request = move_to_execute_step_request(&step, Some("sleep"))
            .expect("serialize moveTo executeStep request");

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4500-0000-7000-8000-000000000036",
                "type": "executeStep",
                "stepId": "step_019f4500-0000-7000-8000-000000000036",
                "step": {
                    "kind": "moveTo",
                    "target": { "kind": "home" },
                    "after": "sleep",
                    "interruptPolicy": "interruptible",
                    "timeoutMs": 15000
                }
            })
        );
    }

    #[test]
    fn move_by_path_execute_step_request_serializes_path_for_sidecar_protocol() {
        let mut step = MoveByPathStep::new(
            "step_019f4500-0000-7000-8000-000000000038",
            vec![
                super::super::timeline::MoveTarget::Edge {
                    edge: super::super::timeline::MoveEdge::Left,
                },
                super::super::timeline::MoveTarget::Center,
                super::super::timeline::MoveTarget::Position { x: 320, y: 640 },
            ],
            30_000,
        );
        step.after_action_id = Some("sleep".to_owned());

        let request = super::move_by_path_execute_step_request(&step, Some("sleep"))
            .expect("serialize moveByPath executeStep request");

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4500-0000-7000-8000-000000000038",
                "type": "executeStep",
                "stepId": "step_019f4500-0000-7000-8000-000000000038",
                "step": {
                    "kind": "moveByPath",
                    "path": [
                        { "kind": "edge", "edge": "left" },
                        { "kind": "center" },
                        { "kind": "position", "x": 320, "y": 640 }
                    ],
                    "after": "sleep",
                    "interruptPolicy": "interruptible",
                    "timeoutMs": 30000
                }
            })
        );
    }

    #[test]
    fn ensure_sidecar_step_completed_preserves_unsupported_step_capability() {
        let error = ensure_sidecar_step_completed(SidecarStepResponse::ProtocolError(
            protocol_error_response_with_code(
                Some("step_019f4500-0000-7000-8000-000000000039"),
                SidecarStepErrorCode::UnsupportedStepCapability,
                "windowAnchor is unsupported",
            ),
        ))
        .expect_err("unsupported capability should remain typed");

        match error {
            BuddyError::UnsupportedCapability { scope, capability } => {
                assert_eq!(scope, "native pet executeStep");
                assert_eq!(capability, "windowAnchor");
            }
            error => panic!("expected unsupported capability error, got {error}"),
        }
    }

    #[test]
    fn pending_timeline_execution_body_round_trips_without_runtime_handles() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-pending-timeline-body-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let mut move_step = MoveByPathStep::new(
            "step_pending_body_path",
            vec![
                super::super::timeline::MoveTarget::Edge {
                    edge: super::super::timeline::MoveEdge::Left,
                },
                super::super::timeline::MoveTarget::WindowAnchor {
                    selector: super::super::timeline::WindowAnchorSelector {
                        kind: super::super::timeline::WindowAnchorSelectorKind::ActiveWindow,
                    },
                    edge: super::super::timeline::WindowAnchorEdge::Auto,
                    reveal: super::super::timeline::WindowAnchorReveal::Head,
                    duration_ms: 1_500,
                },
            ],
            30_000,
        );
        move_step.after_action_id = Some("sleep".to_owned());
        move_step.fallback_after_action_id = Some("idle".to_owned());
        let plan =
            super::super::timeline::TimelinePlan {
                plan_id: "plan_pending_body".to_owned(),
                source_ref: serde_json::json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_pending_body",
                    "messageId": "message_pending_body"
                }),
                failure_policy: super::super::timeline::TimelineFailurePolicy::Abort,
                steps: vec![
                    super::super::timeline::TimelineStep::MoveByPath(move_step),
                    super::super::timeline::TimelineStep::Wait(
                        super::super::timeline::WaitStep::new("step_pending_body_wait", 500, 1_000),
                    ),
                ],
                created_at: "2026-07-12T00:00:00.000Z".to_owned(),
            };
        let context = TimelineExecutionContext::fixed_for_test();
        let resolve_context = ResolveContext::default().with_unsupported_capability("windowAnchor");
        let request = TimelineAdmissionExecutionRequest::new(
            storage.clone(),
            plan.clone(),
            context.clone(),
            resolve_context.clone(),
            ChoreographyTriggerSource::AiChoreography,
        );

        let value =
            serde_json::to_value(request.pending_body()).expect("serialize pending body snapshot");
        assert!(value.get("storage").is_none());
        assert_eq!(value.get("schemaVersion"), Some(&serde_json::json!(1)));
        assert_eq!(
            value.get("triggerSource"),
            Some(&serde_json::json!("aiChoreography"))
        );
        assert_eq!(
            value.pointer("/context/admissionEventId"),
            Some(&serde_json::json!(
                "evt_timeline_019f4000-0000-7000-8000-000000000000"
            ))
        );
        assert!(value.pointer("/context/admission_event_id").is_none());
        let decoded = serde_json::from_value::<TimelinePendingExecutionBody>(value)
            .expect("deserialize pending body snapshot");
        let restored = TimelineAdmissionExecutionRequest::from_pending_body(storage, decoded)
            .expect("restore pending request");

        assert_eq!(restored.plan, plan);
        assert_eq!(restored.context, context);
        assert_eq!(restored.resolve_context, resolve_context);
        assert_eq!(
            restored.trigger_source,
            ChoreographyTriggerSource::AiChoreography
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn taking_pending_execution_requests_keeps_bodies_until_admission_is_recorded() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let timeline_plan_id = "plan_pending_timeline_handoff";
        let timeline_request = TimelineAdmissionExecutionRequest::new(
            storage.clone(),
            TimelinePlan {
                plan_id: timeline_plan_id.to_owned(),
                source_ref: json!({ "kind": "test" }),
                failure_policy: TimelineFailurePolicy::Abort,
                steps: vec![],
                created_at: "2026-07-16T00:00:00.000Z".to_owned(),
            },
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        );
        let mut timeline_queue = PendingTimelineExecutionQueue::default();
        timeline_queue
            .enqueue(timeline_request)
            .expect("enqueue timeline request");

        assert!(timeline_queue
            .take(timeline_plan_id)
            .expect("take timeline request")
            .is_some());
        assert!(storage
            .find_replayable_choreography_pending_execution_body_from_action_log(timeline_plan_id)
            .expect("find replayable timeline body")
            .is_some());

        let fixture_context = DevFixtureExecutionContext::fixed_for_test();
        let fixture_plan_id = fixture_context.plan_id.clone();
        let fixture_request = DevFixtureAdmissionExecutionRequest::new(
            storage.clone(),
            fixture_context,
            ResolveContext::default(),
            DevFixtureKind::AiMacroDemo,
            ChoreographyTriggerSource::UserRequested,
        );
        let mut fixture_queue = PendingDevFixtureExecutionQueue::default();
        fixture_queue
            .enqueue(fixture_request)
            .expect("enqueue fixture request");

        assert!(fixture_queue
            .take(fixture_plan_id.as_str())
            .expect("take fixture request")
            .is_some());
        assert!(storage
            .find_replayable_choreography_pending_execution_body_from_action_log(
                fixture_plan_id.as_str(),
            )
            .expect("find replayable fixture body")
            .is_some());
    }

    #[test]
    fn deferred_admission_is_recorded_after_its_replayable_body() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut admission = ChoreographyAdmissionState::default();
        let active_decision = admission.admit(
            ChoreographyAdmissionRequest::new(
                "plan_pending_body_write_ahead_active",
                ChoreographyTriggerSource::AiChoreography,
            )
            .with_active_step(
                "step_pending_body_write_ahead_active",
                SidecarInterruptPolicy::FinishStep,
            ),
        );
        assert!(matches!(
            active_decision,
            crate::choreography::admission::ChoreographyAdmissionDecision::Accepted { .. }
        ));
        let plan_id = "plan_pending_body_write_ahead_deferred";
        let request = TimelineAdmissionExecutionRequest::new(
            storage.clone(),
            TimelinePlan {
                plan_id: plan_id.to_owned(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_pending_body_write_ahead",
                    "messageId": "message_pending_body_write_ahead"
                }),
                failure_policy: TimelineFailurePolicy::Abort,
                steps: vec![],
                created_at: "2026-07-16T00:00:00.000Z".to_owned(),
            },
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::UserRequested,
        );
        let mut pending_queue = PendingTimelineExecutionQueue::default();

        let scheduled =
            admit_timeline_plan_with_pending_queue(&mut admission, &mut pending_queue, request)
                .expect("defer timeline request");
        assert!(scheduled.execution.is_none());

        let records = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|record| serde_json::from_str::<serde_json::Value>(&record).expect("parse record"))
            .collect::<Vec<_>>();
        let stored_index = records
            .iter()
            .position(|record| {
                record["eventType"] == "choreographyScheduler.pendingBodyStored"
                    && record["payload"]["planId"] == plan_id
            })
            .expect("pending body stored fact");
        let deferred_index = records
            .iter()
            .position(|record| record["eventType"] == "executor.deferred")
            .expect("deferred admission event");

        assert!(stored_index < deferred_index);
    }

    struct InterruptFailingStepExecutor;

    impl ChoreographyStepExecutor for InterruptFailingStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn move_to_step(
            &self,
            _step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
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
            Err(BuddyError::Runtime("interrupt failed".to_owned()))
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[test]
    fn preempt_interrupt_failure_records_plan_failure_and_uses_failure_recovery() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new(
                "plan_preempt_interrupt_active",
                ChoreographyTriggerSource::AiChoreography,
            )
            .with_active_step(
                "step_preempt_interrupt_active",
                SidecarInterruptPolicy::Interruptible,
            ),
        );
        let plan_id = "plan_preempt_interrupt_failure";
        let mut pending_queue = PendingTimelineExecutionQueue::default();
        let scheduled = admit_timeline_plan_with_pending_queue(
            &mut admission,
            &mut pending_queue,
            TimelineAdmissionExecutionRequest::new(
                storage.clone(),
                TimelinePlan {
                    plan_id: plan_id.to_owned(),
                    source_ref: json!({
                        "kind": "conversationMessage",
                        "conversationId": "conversation_preempt_interrupt_failure",
                        "messageId": "message_preempt_interrupt_failure"
                    }),
                    failure_policy: TimelineFailurePolicy::Abort,
                    steps: vec![TimelineStep::Wait(WaitStep::new(
                        "step_preempt_interrupt_failure",
                        1,
                        10,
                    ))],
                    created_at: "2026-07-16T00:00:00.000Z".to_owned(),
                },
                TimelineExecutionContext::fixed_for_test(),
                ResolveContext::default(),
                ChoreographyTriggerSource::UserRequested,
            ),
        )
        .expect("admit preempting plan");
        let execution = scheduled.execution.expect("preempting plan executes");
        let failure_release_calls = Cell::new(0);
        let normal_release_calls = Cell::new(0);
        let recovery_calls = Cell::new(0);

        let result = execute_admitted_timeline_plan(
            &InterruptFailingStepExecutor,
            execution,
            |_, _, _| Ok(()),
            |_, _| Ok(super::StepCompletionDecision::Continue),
            |_| {
                failure_release_calls.set(failure_release_calls.get() + 1);
                Ok(
                    crate::choreography::admission::ChoreographyAdmissionRelease::Released {
                        plan_id: plan_id.to_owned(),
                    },
                )
            },
            |_| {
                normal_release_calls.set(normal_release_calls.get() + 1);
                Ok(
                    crate::choreography::admission::ChoreographyAdmissionRelease::Released {
                        plan_id: plan_id.to_owned(),
                    },
                )
            },
            |_, _, _, _| {
                recovery_calls.set(recovery_calls.get() + 1);
                Ok(())
            },
        );

        assert!(result.is_err());
        assert_eq!(failure_release_calls.get(), 1);
        assert_eq!(normal_release_calls.get(), 0);
        assert_eq!(recovery_calls.get(), 1);
        assert_eq!(
            storage
                .get_action_log_plan_detail(plan_id)
                .expect("read failed plan detail")
                .plan
                .last_event_type,
            "plan.failed"
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DevFixtureExecutionError {
    #[error("action log write failed: {0}")]
    ActionLog(#[source] BuddyError),
    #[error("fixture execution failed: {0}")]
    Execution(#[source] BuddyError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DevFixtureExecutionContext {
    pub(crate) plan_id: String,
    pub(crate) beat_id: String,
    pub(crate) step_id: String,
    pub(crate) admission_event_id: String,
    pub(crate) plan_started_event_id: String,
    pub(crate) step_resolved_event_id: String,
    pub(crate) step_completed_event_id: String,
    pub(crate) step_failed_event_id: String,
    pub(crate) plan_completed_event_id: String,
    pub(crate) plan_failed_event_id: String,
    pub(crate) created_at: String,
    pub(crate) resolved_at: String,
    pub(crate) completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TimelineExecutionContext {
    pub(crate) admission_event_id: String,
    pub(crate) plan_started_event_id: String,
    pub(crate) step_resolved_event_id: String,
    pub(crate) step_completed_event_id: String,
    pub(crate) step_failed_event_id: String,
    pub(crate) plan_completed_event_id: String,
    pub(crate) plan_failed_event_id: String,
    pub(crate) created_at: String,
    pub(crate) resolved_at: String,
    pub(crate) completed_at: String,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct MacroIntentExecutionContext {
    pub(crate) plan_id: String,
    pub(crate) beat_id: String,
    pub(crate) step_id: String,
    pub(crate) timeline: TimelineExecutionContext,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct MacroIntentExecutionRequest {
    pub(crate) context: MacroIntentExecutionContext,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) resolve_context: ResolveContext,
    pub(crate) trigger_source: ChoreographyTriggerSource,
}

#[derive(Debug)]
pub(crate) struct DevFixtureExecutionReport {
    pub(crate) plan_id: String,
}

#[derive(Debug)]
pub(crate) struct DevFixtureAdmissionExecutionReport {
    pub(crate) plan_id: String,
    pub(crate) decision: ChoreographyAdmissionDecision,
    pub(crate) executed: bool,
}

const DEV_FIXTURE_PENDING_EXECUTION_BODY_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DevFixturePendingExecutionBody {
    schema_version: u16,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    fixture_kind: DevFixtureKind,
    trigger_source: ChoreographyTriggerSource,
}

pub(crate) struct DevFixtureAdmissionExecutionRequest {
    storage: BuddyStorage,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    fixture_kind: DevFixtureKind,
    trigger_source: ChoreographyTriggerSource,
}

impl DevFixtureAdmissionExecutionRequest {
    pub(crate) fn new(
        storage: BuddyStorage,
        context: DevFixtureExecutionContext,
        resolve_context: ResolveContext,
        fixture_kind: DevFixtureKind,
        trigger_source: ChoreographyTriggerSource,
    ) -> Self {
        Self {
            storage,
            context,
            resolve_context,
            fixture_kind,
            trigger_source,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn pending_body(&self) -> DevFixturePendingExecutionBody {
        DevFixturePendingExecutionBody {
            schema_version: DEV_FIXTURE_PENDING_EXECUTION_BODY_SCHEMA_VERSION,
            context: self.context.clone(),
            resolve_context: self.resolve_context.clone(),
            fixture_kind: self.fixture_kind,
            trigger_source: self.trigger_source,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn from_pending_body(
        storage: BuddyStorage,
        body: DevFixturePendingExecutionBody,
    ) -> BuddyResult<Self> {
        if body.schema_version != DEV_FIXTURE_PENDING_EXECUTION_BODY_SCHEMA_VERSION {
            return Err(BuddyError::Validation(format!(
                "unsupported pending dev fixture execution body schemaVersion={}",
                body.schema_version
            )));
        }

        Ok(Self::new(
            storage,
            body.context,
            body.resolve_context,
            body.fixture_kind,
            body.trigger_source,
        ))
    }
}

pub(crate) struct ScheduledDevFixtureExecution {
    pub(crate) plan_id: String,
    pub(crate) decision: ChoreographyAdmissionDecision,
    pub(crate) execution: Option<AdmittedDevFixtureExecution>,
}

pub(crate) struct AdmittedDevFixtureExecution {
    storage: BuddyStorage,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    fixture_kind: DevFixtureKind,
    decision: ChoreographyAdmissionDecision,
}

pub(crate) struct ExecutedDevFixtureAdmission {
    pub(crate) report: DevFixtureAdmissionExecutionReport,
}

#[derive(Debug, thiserror::Error)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum TimelineExecutionError {
    #[error("action log write failed: {0}")]
    ActionLog(#[source] BuddyError),
    #[error("timeline execution failed: {0}")]
    Execution(#[source] BuddyError),
}

#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TimelinePlanExecutionReport {
    pub(crate) plan_id: String,
}

#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TimelineAdmissionExecutionReport {
    pub(crate) plan_id: String,
    pub(crate) decision: ChoreographyAdmissionDecision,
    pub(crate) executed: bool,
}

pub(crate) struct ScheduledTimelineExecution {
    pub(crate) plan_id: String,
    pub(crate) decision: ChoreographyAdmissionDecision,
    pub(crate) execution: Option<AdmittedTimelineExecution>,
}

pub(crate) struct AdmittedTimelineExecution {
    storage: BuddyStorage,
    plan: TimelinePlan,
    context: TimelineExecutionContext,
    resolve_context: ResolveContext,
    trigger_source: ChoreographyTriggerSource,
    decision: ChoreographyAdmissionDecision,
}

pub(crate) struct ExecutedTimelineAdmission {
    pub(crate) report: TimelineAdmissionExecutionReport,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RuntimeSafeFallbackExecutionError {
    #[error("action log write failed: {0}")]
    ActionLog(#[source] BuddyError),
    #[error("runtime safe fallback execution failed: {0}")]
    Execution(#[source] BuddyError),
}

pub(crate) struct RuntimeSafeFallbackExecutionContext {
    pub(crate) plan_id: String,
    pub(crate) step_id: String,
    pub(crate) admission_event_id: String,
    pub(crate) plan_started_event_id: String,
    pub(crate) step_resolved_event_id: String,
    pub(crate) step_completed_event_id: String,
    pub(crate) step_failed_event_id: String,
    pub(crate) plan_completed_event_id: String,
    pub(crate) plan_failed_event_id: String,
    pub(crate) created_at: String,
    pub(crate) resolved_at: String,
    pub(crate) completed_at: String,
}

#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct RuntimeSafeFallbackExecutionReport {
    pub(crate) plan_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChoreographyRuntimeDegradation {
    pub(crate) reason_code: &'static str,
    pub(crate) degraded_at: String,
}

impl ChoreographyRuntimeDegradation {
    fn system_recovery_failed(degraded_at: impl Into<String>) -> Self {
        Self {
            reason_code: "runtime.systemRecoveryFailed",
            degraded_at: degraded_at.into(),
        }
    }
}

pub(crate) struct RuntimeSafeFallbackTrigger<'a> {
    pub(crate) triggered_by_plan_id: &'a str,
    pub(crate) triggered_by_step_id: Option<&'a str>,
    pub(crate) trigger_reason: RuntimeSafeFallbackReason,
}

#[cfg_attr(not(test), allow(dead_code))]
const TIMELINE_PENDING_EXECUTION_BODY_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TimelinePendingExecutionBody {
    schema_version: u16,
    plan: TimelinePlan,
    context: TimelineExecutionContext,
    resolve_context: ResolveContext,
    trigger_source: ChoreographyTriggerSource,
}

pub(crate) struct TimelineAdmissionExecutionRequest {
    storage: BuddyStorage,
    plan: TimelinePlan,
    context: TimelineExecutionContext,
    resolve_context: ResolveContext,
    trigger_source: ChoreographyTriggerSource,
}

impl TimelineAdmissionExecutionRequest {
    pub(crate) fn new(
        storage: BuddyStorage,
        plan: TimelinePlan,
        context: TimelineExecutionContext,
        resolve_context: ResolveContext,
        trigger_source: ChoreographyTriggerSource,
    ) -> Self {
        Self {
            storage,
            plan,
            context,
            resolve_context,
            trigger_source,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn pending_body(&self) -> TimelinePendingExecutionBody {
        TimelinePendingExecutionBody {
            schema_version: TIMELINE_PENDING_EXECUTION_BODY_SCHEMA_VERSION,
            plan: self.plan.clone(),
            context: self.context.clone(),
            resolve_context: self.resolve_context.clone(),
            trigger_source: self.trigger_source,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn from_pending_body(
        storage: BuddyStorage,
        body: TimelinePendingExecutionBody,
    ) -> BuddyResult<Self> {
        if body.schema_version != TIMELINE_PENDING_EXECUTION_BODY_SCHEMA_VERSION {
            return Err(BuddyError::Validation(format!(
                "unsupported pending timeline execution body schemaVersion={}",
                body.schema_version
            )));
        }

        Ok(Self::new(
            storage,
            body.plan,
            body.context,
            body.resolve_context,
            body.trigger_source,
        ))
    }
}

#[derive(Default)]
pub(crate) struct PendingTimelineExecutionQueue {
    requests: Vec<TimelineAdmissionExecutionRequest>,
}

impl PendingTimelineExecutionQueue {
    #[cfg(test)]
    fn enqueue(&mut self, request: TimelineAdmissionExecutionRequest) -> BuddyResult<()> {
        Self::persist(&request)?;
        self.enqueue_persisted(request);
        Ok(())
    }

    fn persist(request: &TimelineAdmissionExecutionRequest) -> BuddyResult<()> {
        request.storage.upsert_choreography_pending_execution_body(
            UpsertChoreographyPendingExecutionBodyRequest {
                plan_id: request.plan.plan_id.clone(),
                body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                schema_version: TIMELINE_PENDING_EXECUTION_BODY_SCHEMA_VERSION,
                body: serde_json::to_value(request.pending_body())?,
            },
        )?;
        Ok(())
    }

    fn enqueue_persisted(&mut self, request: TimelineAdmissionExecutionRequest) {
        self.requests
            .retain(|queued| queued.plan.plan_id != request.plan.plan_id);
        self.requests.push(request);
    }

    fn take(&mut self, plan_id: &str) -> BuddyResult<Option<TimelineAdmissionExecutionRequest>> {
        let index = self
            .requests
            .iter()
            .position(|request| request.plan.plan_id == plan_id);
        let Some(index) = index else {
            return Ok(None);
        };
        Ok(Some(self.requests.remove(index)))
    }

    pub(crate) fn remove_replaced(&mut self, plan_id: &str) -> bool {
        let Some(index) = self
            .requests
            .iter()
            .position(|request| request.plan.plan_id == plan_id)
        else {
            return false;
        };
        self.requests.remove(index);
        true
    }

    pub(crate) fn contains(&self, plan_id: &str) -> bool {
        self.requests
            .iter()
            .any(|request| request.plan.plan_id == plan_id)
    }

    pub(crate) fn plan_ids(&self) -> Vec<String> {
        self.requests
            .iter()
            .map(|request| request.plan.plan_id.clone())
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct PendingDevFixtureExecutionQueue {
    requests: Vec<DevFixtureAdmissionExecutionRequest>,
}

impl PendingDevFixtureExecutionQueue {
    #[cfg(test)]
    fn enqueue(&mut self, request: DevFixtureAdmissionExecutionRequest) -> BuddyResult<()> {
        Self::persist(&request)?;
        self.enqueue_persisted(request);
        Ok(())
    }

    fn persist(request: &DevFixtureAdmissionExecutionRequest) -> BuddyResult<()> {
        request.storage.upsert_choreography_pending_execution_body(
            UpsertChoreographyPendingExecutionBodyRequest {
                plan_id: request.context.plan_id.clone(),
                body_kind: ChoreographyPendingExecutionBodyKind::DevFixture,
                schema_version: DEV_FIXTURE_PENDING_EXECUTION_BODY_SCHEMA_VERSION,
                body: serde_json::to_value(request.pending_body())?,
            },
        )?;
        Ok(())
    }

    fn enqueue_persisted(&mut self, request: DevFixtureAdmissionExecutionRequest) {
        self.requests
            .retain(|queued| queued.context.plan_id != request.context.plan_id);
        self.requests.push(request);
    }

    fn take(&mut self, plan_id: &str) -> BuddyResult<Option<DevFixtureAdmissionExecutionRequest>> {
        let index = self
            .requests
            .iter()
            .position(|request| request.context.plan_id == plan_id);
        let Some(index) = index else {
            return Ok(None);
        };
        Ok(Some(self.requests.remove(index)))
    }

    pub(crate) fn remove_replaced(&mut self, plan_id: &str) -> bool {
        let Some(index) = self
            .requests
            .iter()
            .position(|request| request.context.plan_id == plan_id)
        else {
            return false;
        };
        self.requests.remove(index);
        true
    }

    pub(crate) fn contains(&self, plan_id: &str) -> bool {
        self.requests
            .iter()
            .any(|request| request.context.plan_id == plan_id)
    }

    pub(crate) fn plan_ids(&self) -> Vec<String> {
        self.requests
            .iter()
            .map(|request| request.context.plan_id.clone())
            .collect()
    }
}

pub(crate) struct MacroIntentTimelineFailureFallbackRequest<'a> {
    pub(crate) original_intent: &'a MacroIntent,
    pub(crate) failed_plan: &'a TimelinePlan,
    pub(crate) failed_step_id: &'a str,
    pub(crate) error: &'a BuddyError,
    pub(crate) resolve_context: ResolveContext,
    pub(crate) trigger_source: ChoreographyTriggerSource,
}

pub(crate) struct AdmittedRuntimeSafeFallback {
    pub(crate) plan: RuntimeSafeFallbackPlan,
    pub(crate) context: RuntimeSafeFallbackExecutionContext,
    pub(crate) decision: ChoreographyAdmissionDecision,
    pub(crate) should_execute: bool,
}

impl DevFixtureExecutionContext {
    pub(crate) fn new() -> Self {
        let created_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let resolved_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let completed_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();

        Self {
            plan_id: prefixed_uuid_v7("plan"),
            beat_id: prefixed_uuid_v7("beat"),
            step_id: prefixed_uuid_v7("step"),
            admission_event_id: prefixed_uuid_v7("evt"),
            plan_started_event_id: prefixed_uuid_v7("evt"),
            step_resolved_event_id: prefixed_uuid_v7("evt"),
            step_completed_event_id: prefixed_uuid_v7("evt"),
            step_failed_event_id: prefixed_uuid_v7("evt"),
            plan_completed_event_id: prefixed_uuid_v7("evt"),
            plan_failed_event_id: prefixed_uuid_v7("evt"),
            created_at,
            resolved_at,
            completed_at,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed_for_test() -> Self {
        Self {
            plan_id: "plan_019f4000-0000-7000-8000-000000000001".to_owned(),
            beat_id: "beat_019f4000-0000-7000-8000-000000000009".to_owned(),
            step_id: "step_019f4000-0000-7000-8000-000000000002".to_owned(),
            admission_event_id: "evt_019f4000-0000-7000-8000-000000000000".to_owned(),
            plan_started_event_id: "evt_019f4000-0000-7000-8000-000000000003".to_owned(),
            step_resolved_event_id: "evt_019f4000-0000-7000-8000-000000000004".to_owned(),
            step_completed_event_id: "evt_019f4000-0000-7000-8000-000000000005".to_owned(),
            plan_completed_event_id: "evt_019f4000-0000-7000-8000-000000000006".to_owned(),
            step_failed_event_id: "evt_019f4000-0000-7000-8000-000000000007".to_owned(),
            plan_failed_event_id: "evt_019f4000-0000-7000-8000-000000000008".to_owned(),
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
            resolved_at: "2026-07-08T00:00:00.010Z".to_owned(),
            completed_at: "2026-07-08T00:00:01.730Z".to_owned(),
        }
    }

    fn with_new_admission_event_id(mut self) -> Self {
        self.admission_event_id = prefixed_uuid_v7("evt");
        self
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl TimelineExecutionContext {
    pub(crate) fn new() -> Self {
        let created_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let resolved_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let completed_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();

        Self {
            admission_event_id: prefixed_uuid_v7("evt"),
            plan_started_event_id: prefixed_uuid_v7("evt"),
            step_resolved_event_id: prefixed_uuid_v7("evt"),
            step_completed_event_id: prefixed_uuid_v7("evt"),
            step_failed_event_id: prefixed_uuid_v7("evt"),
            plan_completed_event_id: prefixed_uuid_v7("evt"),
            plan_failed_event_id: prefixed_uuid_v7("evt"),
            created_at,
            resolved_at,
            completed_at,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed_for_test() -> Self {
        Self {
            admission_event_id: "evt_timeline_019f4000-0000-7000-8000-000000000000".to_owned(),
            plan_started_event_id: "evt_timeline_019f4000-0000-7000-8000-000000000003".to_owned(),
            step_resolved_event_id: "evt_timeline_019f4000-0000-7000-8000-000000000004".to_owned(),
            step_completed_event_id: "evt_timeline_019f4000-0000-7000-8000-000000000005".to_owned(),
            step_failed_event_id: "evt_timeline_019f4000-0000-7000-8000-000000000006".to_owned(),
            plan_completed_event_id: "evt_timeline_019f4000-0000-7000-8000-000000000007".to_owned(),
            plan_failed_event_id: "evt_timeline_019f4000-0000-7000-8000-000000000008".to_owned(),
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
            resolved_at: "2026-07-08T00:00:00.010Z".to_owned(),
            completed_at: "2026-07-08T00:00:01.730Z".to_owned(),
        }
    }

    fn with_new_admission_event_id(mut self) -> Self {
        self.admission_event_id = prefixed_uuid_v7("evt");
        self
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl MacroIntentExecutionContext {
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self {
            plan_id: prefixed_uuid_v7("plan"),
            beat_id: prefixed_uuid_v7("beat"),
            step_id: prefixed_uuid_v7("step"),
            timeline: TimelineExecutionContext::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed_for_test() -> Self {
        Self {
            plan_id: "plan_macro_019f4000-0000-7000-8000-000000000701".to_owned(),
            beat_id: "beat_macro_019f4000-0000-7000-8000-000000000702".to_owned(),
            step_id: "step_macro_019f4000-0000-7000-8000-000000000703".to_owned(),
            timeline: TimelineExecutionContext::fixed_for_test(),
        }
    }
}

impl RuntimeSafeFallbackExecutionContext {
    pub(crate) fn new() -> Self {
        let created_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let resolved_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let completed_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();

        Self {
            plan_id: prefixed_uuid_v7("plan"),
            step_id: prefixed_uuid_v7("step"),
            admission_event_id: prefixed_uuid_v7("evt"),
            plan_started_event_id: prefixed_uuid_v7("evt"),
            step_resolved_event_id: prefixed_uuid_v7("evt"),
            step_completed_event_id: prefixed_uuid_v7("evt"),
            step_failed_event_id: prefixed_uuid_v7("evt"),
            plan_completed_event_id: prefixed_uuid_v7("evt"),
            plan_failed_event_id: prefixed_uuid_v7("evt"),
            created_at,
            resolved_at,
            completed_at,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed_for_test() -> Self {
        Self {
            plan_id: "plan_recovery_019f4000-0000-7000-8000-000000000001".to_owned(),
            step_id: "step_recovery_019f4000-0000-7000-8000-000000000002".to_owned(),
            admission_event_id: "evt_recovery_019f4000-0000-7000-8000-000000000000".to_owned(),
            plan_started_event_id: "evt_recovery_019f4000-0000-7000-8000-000000000003".to_owned(),
            step_resolved_event_id: "evt_recovery_019f4000-0000-7000-8000-000000000004".to_owned(),
            step_completed_event_id: "evt_recovery_019f4000-0000-7000-8000-000000000005".to_owned(),
            step_failed_event_id: "evt_recovery_019f4000-0000-7000-8000-000000000006".to_owned(),
            plan_completed_event_id: "evt_recovery_019f4000-0000-7000-8000-000000000007".to_owned(),
            plan_failed_event_id: "evt_recovery_019f4000-0000-7000-8000-000000000008".to_owned(),
            created_at: "2026-07-08T00:00:02.000Z".to_owned(),
            resolved_at: "2026-07-08T00:00:02.010Z".to_owned(),
            completed_at: "2026-07-08T00:00:17.000Z".to_owned(),
        }
    }
}

pub(crate) fn execute_single_play_action_dev_fixture(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
) -> Result<DevFixtureExecutionReport, DevFixtureExecutionError> {
    execute_dev_fixture(
        storage,
        executor,
        context,
        resolve_context,
        DevFixtureKind::SinglePlayAction,
    )
}

#[cfg(test)]
pub(crate) fn execute_single_play_action_dev_fixture_with_admission(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    trigger_source: ChoreographyTriggerSource,
) -> Result<DevFixtureAdmissionExecutionReport, DevFixtureExecutionError> {
    execute_dev_fixture_with_admission(
        storage,
        executor,
        admission,
        context,
        resolve_context,
        DevFixtureKind::SinglePlayAction,
        trigger_source,
    )
}

#[cfg(test)]
pub(crate) fn execute_ai_macro_demo_dev_fixture_with_admission(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    trigger_source: ChoreographyTriggerSource,
) -> Result<DevFixtureAdmissionExecutionReport, DevFixtureExecutionError> {
    execute_dev_fixture_with_admission(
        storage,
        executor,
        admission,
        context,
        resolve_context,
        DevFixtureKind::AiMacroDemo,
        trigger_source,
    )
}

pub(crate) fn execute_ai_macro_demo_dev_fixture(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
) -> Result<DevFixtureExecutionReport, DevFixtureExecutionError> {
    execute_dev_fixture(
        storage,
        executor,
        context,
        resolve_context,
        DevFixtureKind::AiMacroDemo,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn execute_macro_intent_with_admission(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    intent: &MacroIntent,
    request: MacroIntentExecutionRequest,
) -> Result<TimelineAdmissionExecutionReport, TimelineExecutionError> {
    let MacroIntentExecutionRequest {
        context,
        source_ref,
        resolve_context,
        trigger_source,
    } = request;
    let plan = match create_timeline_plan_from_macro_intent(intent, &context, source_ref) {
        Ok(plan) => plan,
        Err(error) => {
            trigger_admitted_runtime_safe_fallback_after_macro_planning_failure(
                storage.clone(),
                executor,
                admission,
                &context,
                resolve_context,
            );
            return Err(TimelineExecutionError::Execution(error));
        }
    };

    let macro_failure_fallback_storage = storage.clone();
    execute_timeline_plan_with_admission_internal(
        executor,
        admission,
        None,
        TimelineAdmissionExecutionRequest::new(
            storage,
            plan,
            context.timeline,
            resolve_context,
            trigger_source,
        ),
        move |admission, failed_plan, failed_step_id, error, resolve_context| {
            trigger_admitted_macro_intent_timeline_failure_fallback(
                macro_failure_fallback_storage.clone(),
                executor,
                admission,
                MacroIntentTimelineFailureFallbackRequest {
                    original_intent: intent,
                    failed_plan,
                    failed_step_id,
                    error,
                    resolve_context,
                    trigger_source,
                },
            )
            .map(|_| ())
        },
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn execute_timeline_plan_with_admission(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    plan: TimelinePlan,
    context: TimelineExecutionContext,
    resolve_context: ResolveContext,
    trigger_source: ChoreographyTriggerSource,
) -> Result<TimelineAdmissionExecutionReport, TimelineExecutionError> {
    let recovery_storage = storage.clone();
    execute_timeline_plan_with_admission_internal(
        executor,
        admission,
        None,
        TimelineAdmissionExecutionRequest::new(
            storage,
            plan,
            context,
            resolve_context,
            trigger_source,
        ),
        move |admission, failed_plan, failed_step_id, error, resolve_context| {
            trigger_admitted_runtime_safe_fallback_after_timeline_failure(
                recovery_storage.clone(),
                executor,
                admission,
                failed_plan,
                failed_step_id,
                error,
                resolve_context,
            );
            Ok(())
        },
    )
}

pub(crate) fn admit_timeline_plan_with_pending_queue(
    admission: &mut ChoreographyAdmissionState,
    pending_queue: &mut PendingTimelineExecutionQueue,
    request: TimelineAdmissionExecutionRequest,
) -> Result<ScheduledTimelineExecution, TimelineExecutionError> {
    admit_timeline_plan_internal(admission, Some(pending_queue), request)
}

pub(crate) fn admit_released_pending_timeline_plan(
    admission: &mut ChoreographyAdmissionState,
    pending_queue: &mut PendingTimelineExecutionQueue,
    release: ChoreographyAdmissionRelease,
) -> Option<Result<ScheduledTimelineExecution, TimelineExecutionError>> {
    let ChoreographyAdmissionRelease::ReleasedWithPending {
        pending_plan_id, ..
    } = release
    else {
        return None;
    };
    let request = match pending_queue
        .take(pending_plan_id.as_str())
        .map_err(TimelineExecutionError::Execution)
    {
        Ok(Some(request)) => request,
        Ok(None) => return None,
        Err(error) => return Some(Err(error)),
    };

    Some(admit_timeline_plan_internal(
        admission,
        Some(pending_queue),
        request,
    ))
}

fn admit_timeline_plan_internal(
    admission: &mut ChoreographyAdmissionState,
    pending_queue: Option<&mut PendingTimelineExecutionQueue>,
    request: TimelineAdmissionExecutionRequest,
) -> Result<ScheduledTimelineExecution, TimelineExecutionError> {
    let TimelineAdmissionExecutionRequest {
        storage,
        plan,
        context,
        resolve_context,
        trigger_source,
    } = request;
    let plan = expand_planner_timeline_plan(plan).map_err(TimelineExecutionError::Execution)?;
    let plan_id = plan.plan_id.clone();
    let active_step = timeline_plan_active_step(&plan, &resolve_context);
    let decision = admission.admit(
        ChoreographyAdmissionRequest::new(plan.plan_id.clone(), trigger_source)
            .with_active_step(active_step.step_id, active_step.interrupt_policy),
    );
    let should_execute = should_execute_admission_decision(&decision);
    let deferred_request = if matches!(decision, ChoreographyAdmissionDecision::Deferred { .. })
        && pending_queue.is_some()
    {
        Some(TimelineAdmissionExecutionRequest {
            storage: storage.clone(),
            plan: plan.clone(),
            context: context.clone().with_new_admission_event_id(),
            resolve_context: resolve_context.clone(),
            trigger_source,
        })
    } else {
        None
    };
    if let Some(request) = deferred_request.as_ref() {
        if let Err(error) = PendingTimelineExecutionQueue::persist(request) {
            admission.discard_pending_plan(&plan.plan_id);
            return Err(TimelineExecutionError::Execution(error));
        }
    }

    let sink = ActionLogSink::new(storage.clone());
    if let Err(error) = sink.append_event(&ActionLogEvent::timeline_executor_admission_decision(
        context.admission_event_id.as_str(),
        &plan,
        trigger_source,
        &decision,
        context.created_at.as_str(),
    )) {
        if deferred_request.is_some() {
            admission.discard_pending_plan(&plan.plan_id);
            let _ = storage.delete_choreography_pending_execution_body(&plan.plan_id);
        }
        if should_execute {
            let _release = admission.release_plan_preserving_pending(&plan.plan_id);
        }
        return Err(TimelineExecutionError::ActionLog(error));
    }

    if !should_execute {
        if let (Some(pending_queue), Some(request)) = (pending_queue, deferred_request) {
            pending_queue.enqueue_persisted(request);
        }
        return Ok(ScheduledTimelineExecution {
            plan_id,
            decision,
            execution: None,
        });
    }

    Ok(ScheduledTimelineExecution {
        plan_id,
        decision: decision.clone(),
        execution: Some(AdmittedTimelineExecution {
            storage,
            plan,
            context,
            resolve_context,
            trigger_source,
            decision,
        }),
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn execute_timeline_plan_with_admission_and_pending_queue(
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    pending_queue: &mut PendingTimelineExecutionQueue,
    request: TimelineAdmissionExecutionRequest,
) -> Result<TimelineAdmissionExecutionReport, TimelineExecutionError> {
    let recovery_storage = request.storage.clone();
    execute_timeline_plan_with_admission_internal(
        executor,
        admission,
        Some(pending_queue),
        request,
        move |admission, failed_plan, failed_step_id, error, resolve_context| {
            trigger_admitted_runtime_safe_fallback_after_timeline_failure(
                recovery_storage.clone(),
                executor,
                admission,
                failed_plan,
                failed_step_id,
                error,
                resolve_context,
            );
            Ok(())
        },
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn execute_released_pending_timeline_plan(
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    pending_queue: &mut PendingTimelineExecutionQueue,
    release: ChoreographyAdmissionRelease,
) -> Option<Result<TimelineAdmissionExecutionReport, TimelineExecutionError>> {
    let ChoreographyAdmissionRelease::ReleasedWithPending {
        pending_plan_id, ..
    } = release
    else {
        return None;
    };
    let request = match pending_queue
        .take(pending_plan_id.as_str())
        .map_err(TimelineExecutionError::Execution)
    {
        Ok(Some(request)) => request,
        Ok(None) => return None,
        Err(error) => return Some(Err(error)),
    };
    let recovery_storage = request.storage.clone();

    Some(execute_timeline_plan_with_admission_internal(
        executor,
        admission,
        Some(pending_queue),
        request,
        move |admission, failed_plan, failed_step_id, error, resolve_context| {
            trigger_admitted_runtime_safe_fallback_after_timeline_failure(
                recovery_storage.clone(),
                executor,
                admission,
                failed_plan,
                failed_step_id,
                error,
                resolve_context,
            );
            Ok(())
        },
    ))
}

pub(crate) fn execute_admitted_timeline_plan(
    executor: &impl ChoreographyStepExecutor,
    admitted: AdmittedTimelineExecution,
    mut refresh_active_step: impl FnMut(&str, &TimelineStep, SidecarInterruptPolicy) -> BuddyResult<()>,
    mut after_completed_step: impl FnMut(&str, &TimelineStep) -> BuddyResult<StepCompletionDecision>,
    mut release_failed_plan: impl FnMut(&str) -> BuddyResult<ChoreographyAdmissionRelease>,
    mut release_plan: impl FnMut(&str) -> BuddyResult<ChoreographyAdmissionRelease>,
    mut on_execution_failed: impl FnMut(
        &TimelinePlan,
        &str,
        &BuddyError,
        ResolveContext,
    ) -> BuddyResult<()>,
) -> Result<ExecutedTimelineAdmission, TimelineExecutionError> {
    let AdmittedTimelineExecution {
        storage,
        plan,
        context,
        resolve_context,
        trigger_source,
        decision,
    } = admitted;

    if let Err(error) = interrupt_preempted_active_step(executor, &decision) {
        let failure_log_result = append_timeline_plan_failed(
            &ActionLogSink::new(storage.clone()),
            &plan,
            &context,
            trigger_source,
            ActionLogTimelinePlanStats {
                completed_step_count: 0,
                failed_step_count: 0,
                skipped_step_count: 0,
                duration_ms: 0,
            },
            error.to_string().as_str(),
        );
        let failed_step_id = plan
            .steps
            .first()
            .map(TimelineStep::step_id)
            .unwrap_or(plan.plan_id.as_str());
        let release_result = release_failed_plan(&plan.plan_id);
        let recovery_result =
            on_execution_failed(&plan, failed_step_id, &error, resolve_context.clone());
        release_result.map_err(TimelineExecutionError::Execution)?;
        recovery_result.map_err(TimelineExecutionError::Execution)?;
        failure_log_result?;
        return Err(TimelineExecutionError::Execution(error));
    }

    let plan_id = plan.plan_id.clone();
    let mut execution_failure_handled = false;
    let result = execute_timeline_plan_with_step_start(
        storage,
        executor,
        &plan,
        &context,
        resolve_context,
        trigger_source,
        |event| match event {
            TimelineExecutionEvent::StepStarting(step) => {
                refresh_active_step(plan_id.as_str(), step.step, step.interrupt_policy)?;
                Ok(StepCompletionDecision::Continue)
            }
            TimelineExecutionEvent::StepCompleted(step) => {
                after_completed_step(plan_id.as_str(), step.step)
            }
            TimelineExecutionEvent::ExecutionFailed {
                plan,
                failed_step_id,
                error,
                resolve_context,
                ..
            } => {
                let _release = release_failed_plan(&plan.plan_id)?;
                execution_failure_handled = true;
                on_execution_failed(plan, failed_step_id, error, resolve_context.clone())?;
                Ok(StepCompletionDecision::Continue)
            }
        },
    );
    if !execution_failure_handled {
        let _release = release_plan(&plan.plan_id).map_err(TimelineExecutionError::Execution)?;
    }

    result.map(|report| ExecutedTimelineAdmission {
        report: TimelineAdmissionExecutionReport {
            plan_id: report.plan_id,
            decision,
            executed: true,
        },
    })
}

fn execute_timeline_plan_with_admission_internal(
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    pending_queue: Option<&mut PendingTimelineExecutionQueue>,
    request: TimelineAdmissionExecutionRequest,
    mut on_execution_failed: impl FnMut(
        &mut ChoreographyAdmissionState,
        &TimelinePlan,
        &str,
        &BuddyError,
        ResolveContext,
    ) -> BuddyResult<()>,
) -> Result<TimelineAdmissionExecutionReport, TimelineExecutionError> {
    let TimelineAdmissionExecutionRequest {
        storage,
        plan,
        context,
        resolve_context,
        trigger_source,
    } = request;
    let plan = expand_planner_timeline_plan(plan).map_err(TimelineExecutionError::Execution)?;
    let plan_id = plan.plan_id.clone();
    let active_step = timeline_plan_active_step(&plan, &resolve_context);
    let decision = admission.admit(
        ChoreographyAdmissionRequest::new(plan.plan_id.clone(), trigger_source)
            .with_active_step(active_step.step_id, active_step.interrupt_policy),
    );
    let should_execute = should_execute_admission_decision(&decision);
    let deferred_request = if matches!(decision, ChoreographyAdmissionDecision::Deferred { .. })
        && pending_queue.is_some()
    {
        Some(TimelineAdmissionExecutionRequest {
            storage: storage.clone(),
            plan: plan.clone(),
            context: context.clone().with_new_admission_event_id(),
            resolve_context: resolve_context.clone(),
            trigger_source,
        })
    } else {
        None
    };
    if let Some(request) = deferred_request.as_ref() {
        if let Err(error) = PendingTimelineExecutionQueue::persist(request) {
            admission.discard_pending_plan(&plan.plan_id);
            return Err(TimelineExecutionError::Execution(error));
        }
    }

    let sink = ActionLogSink::new(storage.clone());
    if let Err(error) = sink.append_event(&ActionLogEvent::timeline_executor_admission_decision(
        context.admission_event_id.as_str(),
        &plan,
        trigger_source,
        &decision,
        context.created_at.as_str(),
    )) {
        if deferred_request.is_some() {
            admission.discard_pending_plan(&plan.plan_id);
            let _ = storage.delete_choreography_pending_execution_body(&plan.plan_id);
        }
        if should_execute {
            let _release = admission.release_plan_preserving_pending(&plan.plan_id);
        }
        return Err(TimelineExecutionError::ActionLog(error));
    }

    if !should_execute {
        if let (Some(pending_queue), Some(request)) = (pending_queue, deferred_request) {
            pending_queue.enqueue_persisted(request);
        }
        return Ok(TimelineAdmissionExecutionReport {
            plan_id,
            decision,
            executed: false,
        });
    }

    if let Err(error) = interrupt_preempted_active_step(executor, &decision) {
        let _release = admission.release_plan_preserving_pending(&plan.plan_id);
        return Err(TimelineExecutionError::Execution(error));
    }

    let plan_id = plan.plan_id.clone();
    let result = execute_timeline_plan_with_step_start(
        storage,
        executor,
        &plan,
        &context,
        resolve_context,
        trigger_source,
        |event| match event {
            TimelineExecutionEvent::StepStarting(step) => refresh_admitted_plan_active_step(
                admission,
                plan_id.as_str(),
                step.step,
                step.interrupt_policy,
            )
            .map(|()| StepCompletionDecision::Continue),
            TimelineExecutionEvent::StepCompleted(_) => Ok(StepCompletionDecision::Continue),
            TimelineExecutionEvent::ExecutionFailed {
                plan,
                failed_step_id,
                error,
                resolve_context,
                ..
            } => {
                let _release = admission.release_plan_preserving_pending(&plan.plan_id);
                on_execution_failed(
                    admission,
                    plan,
                    failed_step_id,
                    error,
                    resolve_context.clone(),
                )?;
                Ok(StepCompletionDecision::Continue)
            }
        },
    );
    let _release = admission.release_plan_preserving_pending(&plan.plan_id);

    result.map(|report| TimelineAdmissionExecutionReport {
        plan_id: report.plan_id,
        decision,
        executed: true,
    })
}

pub(crate) fn trigger_admitted_macro_intent_timeline_failure_fallback(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    request: MacroIntentTimelineFailureFallbackRequest<'_>,
) -> BuddyResult<Option<ChoreographyRuntimeDegradation>> {
    if let Some(degradation) = trigger_admitted_macro_semantic_fallback_after_timeline_failure(
        storage.clone(),
        executor,
        admission,
        &request,
    ) {
        return Ok(degradation);
    }

    Ok(
        trigger_admitted_runtime_safe_fallback_after_timeline_failure(
            storage,
            executor,
            admission,
            request.failed_plan,
            request.failed_step_id,
            request.error,
            request.resolve_context,
        ),
    )
}

fn trigger_admitted_macro_semantic_fallback_after_timeline_failure(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    request: &MacroIntentTimelineFailureFallbackRequest<'_>,
) -> Option<Option<ChoreographyRuntimeDegradation>> {
    let fallback = macro_semantic_fallback_intent(
        request.original_intent,
        request.failed_plan,
        request.failed_step_id,
        request.error,
    )?;

    let fallback_context = MacroIntentExecutionContext::new();
    let fallback_source_ref = serde_json::json!({
        "kind": "macroFallback",
        "triggeredByPlanId": request.failed_plan.plan_id,
        "triggeredByStepId": request.failed_step_id,
        "triggerReason": fallback.semantic_fallback.trigger_reason_code(),
        "originalMacroId": request.original_intent.macro_id(),
        "fallbackMacroId": fallback.semantic_fallback.fallback_macro_id(),
    });
    let Ok(plan) = create_timeline_plan_from_macro_intent(
        &fallback.intent,
        &fallback_context,
        fallback_source_ref,
    ) else {
        return None;
    };

    let recovery_storage = storage.clone();
    let degradation = RefCell::new(None);
    let _ = execute_timeline_plan_with_admission_internal(
        executor,
        admission,
        None,
        TimelineAdmissionExecutionRequest::new(
            storage,
            plan,
            fallback_context.timeline,
            request.resolve_context.clone(),
            request.trigger_source,
        ),
        |admission, failed_plan, failed_step_id, error, resolve_context| {
            *degradation.borrow_mut() =
                trigger_admitted_runtime_safe_fallback_after_timeline_failure(
                    recovery_storage.clone(),
                    executor,
                    admission,
                    failed_plan,
                    failed_step_id,
                    error,
                    resolve_context,
                );
            Ok(())
        },
    );
    Some(degradation.into_inner())
}

struct MacroSemanticFallbackIntent {
    intent: MacroIntent,
    semantic_fallback: MacroSemanticFallback,
}

fn macro_semantic_fallback_intent(
    intent: &MacroIntent,
    failed_plan: &TimelinePlan,
    failed_step_id: &str,
    error: &BuddyError,
) -> Option<MacroSemanticFallbackIntent> {
    let MacroTimelineFailureFallback::Semantic(semantic_fallback) =
        macro_fallback_policy(intent).timeline_failure_fallback
    else {
        return None;
    };

    match (semantic_fallback, intent) {
        (
            MacroSemanticFallback::AwaitApprovalActionFailedToThinking,
            MacroIntent::AwaitApproval(_),
        ) if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) => {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Thinking(ThinkingMacroParams::default()),
                semantic_fallback,
            })
        }
        (
            MacroSemanticFallback::WindowAnchorTargetUnavailableToPeekFromEdge,
            MacroIntent::PeekBehindWindow(params),
        ) if is_window_anchor_target_unavailable_error(error) => {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::PeekFromEdge(PeekFromEdgeMacroParams {
                    edge: params.edge.fallback_screen_edge(),
                }),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::CelebrateActionFailedToReassure, MacroIntent::Celebrate(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Reassure(ReassureMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::CastActionFailedToCelebrate, MacroIntent::Cast(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Celebrate(CelebrateMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::CuriousActionFailedToThinking, MacroIntent::Curious(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Thinking(ThinkingMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::DanceActionFailedToCelebrate, MacroIntent::Dance(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Celebrate(CelebrateMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::GetUpActionFailedToReassure, MacroIntent::GetUp(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Reassure(ReassureMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::ReassureActionFailedToLieDown, MacroIntent::Reassure(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::LieDown(LieDownMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::SadActionFailedToReassure, MacroIntent::Sad(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Reassure(ReassureMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::ThinkingActionFailedToCurious, MacroIntent::Thinking(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Curious(CuriousMacroParams::default()),
                semantic_fallback,
            })
        }
        (MacroSemanticFallback::WorkingActionFailedToThinking, MacroIntent::Working(_))
            if is_play_action_runtime_execution_error(failed_plan, failed_step_id, error) =>
        {
            Some(MacroSemanticFallbackIntent {
                intent: MacroIntent::Thinking(ThinkingMacroParams::default()),
                semantic_fallback,
            })
        }
        _ => None,
    }
}

fn is_window_anchor_target_unavailable_error(error: &BuddyError) -> bool {
    matches!(
        error,
        BuddyError::Runtime(message)
            if message.starts_with("native pet step failed: targetUnavailable:")
                && message.contains("active window rect is unavailable")
    )
}

fn is_play_action_runtime_execution_error(
    plan: &TimelinePlan,
    failed_step_id: &str,
    error: &BuddyError,
) -> bool {
    matches!(error, BuddyError::Runtime(_))
        && plan.steps.iter().any(|step| {
            step.step_id() == failed_step_id && matches!(step, TimelineStep::PlayAction(_))
        })
}

pub(crate) fn execute_runtime_safe_fallback_plan(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    plan: RuntimeSafeFallbackPlan,
    context: RuntimeSafeFallbackExecutionContext,
    resolve_context: ResolveContext,
) -> Result<RuntimeSafeFallbackExecutionReport, RuntimeSafeFallbackExecutionError> {
    let registry =
        ActionRegistry::load_bundled().map_err(RuntimeSafeFallbackExecutionError::Execution)?;
    let sink = ActionLogSink::new(storage);

    sink.append_event(&ActionLogEvent::system_recovery_plan_started(
        context.plan_started_event_id.as_str(),
        &plan,
        context.created_at.as_str(),
    ))
    .map_err(RuntimeSafeFallbackExecutionError::ActionLog)?;

    let mut completed_step_count = 0_u64;
    let mut duration_ms = 0_u64;
    let step_scope = RuntimeSafeFallbackStepExecutionScope {
        registry: &registry,
        resolve_context: &resolve_context,
        sink: &sink,
        plan: &plan,
        context: &context,
    };

    for (step_index, step) in plan.steps.iter().enumerate() {
        match execute_runtime_safe_fallback_timeline_step(executor, &step_scope, step, step_index) {
            Ok(step_duration_ms) => {
                completed_step_count += 1;
                duration_ms += step_duration_ms;
            }
            Err(RuntimeSafeFallbackExecutionError::Execution(error)) => {
                let error_message = error.to_string();
                append_runtime_safe_fallback_plan_failed(
                    &sink,
                    &plan,
                    &context,
                    error_message.as_str(),
                )?;
                append_runtime_degraded_after_system_recovery_failed(
                    &sink,
                    &plan,
                    error_message.as_str(),
                    context.completed_at.as_str(),
                )?;

                return Err(RuntimeSafeFallbackExecutionError::Execution(error));
            }
            Err(error) => return Err(error),
        }
    }

    sink.append_event(&ActionLogEvent::system_recovery_plan_completed(
        context.plan_completed_event_id,
        &plan,
        completed_step_count,
        duration_ms,
        context.completed_at,
    ))
    .map_err(RuntimeSafeFallbackExecutionError::ActionLog)?;

    Ok(RuntimeSafeFallbackExecutionReport {
        plan_id: plan.plan_id,
    })
}

pub(crate) fn admit_runtime_safe_fallback_plan(
    storage: BuddyStorage,
    admission: &mut ChoreographyAdmissionState,
    trigger: RuntimeSafeFallbackTrigger<'_>,
) -> BuddyResult<AdmittedRuntimeSafeFallback> {
    let context = RuntimeSafeFallbackExecutionContext::new();
    let plan = create_runtime_safe_fallback_plan_for_trigger(&context, trigger);
    let active_step = runtime_safe_fallback_plan_active_step(&plan, &ResolveContext::default());
    let decision = admission.admit(
        ChoreographyAdmissionRequest::new(
            plan.plan_id.clone(),
            ChoreographyTriggerSource::SystemRecovery,
        )
        .with_active_step(active_step.step_id, active_step.interrupt_policy),
    );
    let should_execute = should_execute_admission_decision(&decision);
    let sink = ActionLogSink::new(storage);

    if let Err(error) = sink.append_event(
        &ActionLogEvent::system_recovery_executor_admission_decision(
            context.admission_event_id.as_str(),
            &plan,
            &decision,
            context.created_at.as_str(),
        ),
    ) {
        if should_execute {
            let _release = admission.release_plan_preserving_pending(&plan.plan_id);
        }
        return Err(error);
    }

    Ok(AdmittedRuntimeSafeFallback {
        plan,
        context,
        decision,
        should_execute,
    })
}

pub(crate) fn admit_dev_fixture_with_pending_queue(
    admission: &mut ChoreographyAdmissionState,
    pending_queue: &mut PendingDevFixtureExecutionQueue,
    request: DevFixtureAdmissionExecutionRequest,
) -> Result<ScheduledDevFixtureExecution, DevFixtureExecutionError> {
    admit_dev_fixture_internal(admission, Some(pending_queue), request)
}

pub(crate) fn admit_released_pending_dev_fixture(
    admission: &mut ChoreographyAdmissionState,
    pending_queue: &mut PendingDevFixtureExecutionQueue,
    release: ChoreographyAdmissionRelease,
) -> Option<Result<ScheduledDevFixtureExecution, DevFixtureExecutionError>> {
    let ChoreographyAdmissionRelease::ReleasedWithPending {
        pending_plan_id, ..
    } = release
    else {
        return None;
    };
    let request = match pending_queue
        .take(pending_plan_id.as_str())
        .map_err(DevFixtureExecutionError::Execution)
    {
        Ok(Some(request)) => request,
        Ok(None) => return None,
        Err(error) => return Some(Err(error)),
    };

    Some(admit_dev_fixture_internal(
        admission,
        Some(pending_queue),
        request,
    ))
}

fn admit_dev_fixture_internal(
    admission: &mut ChoreographyAdmissionState,
    pending_queue: Option<&mut PendingDevFixtureExecutionQueue>,
    request: DevFixtureAdmissionExecutionRequest,
) -> Result<ScheduledDevFixtureExecution, DevFixtureExecutionError> {
    let DevFixtureAdmissionExecutionRequest {
        storage,
        context,
        resolve_context,
        fixture_kind,
        trigger_source,
    } = request;
    let plan = create_dev_fixture_plan(fixture_kind, &context)
        .map_err(DevFixtureExecutionError::Execution)?;
    let plan_id = plan.plan_id.clone();
    let active_step = dev_fixture_plan_active_step(&plan, &resolve_context);
    let decision = admission.admit(
        ChoreographyAdmissionRequest::new(plan.plan_id.clone(), trigger_source)
            .with_active_step(active_step.step_id, active_step.interrupt_policy),
    );
    let should_execute = should_execute_admission_decision(&decision);
    let deferred_request = if matches!(decision, ChoreographyAdmissionDecision::Deferred { .. })
        && pending_queue.is_some()
    {
        Some(DevFixtureAdmissionExecutionRequest {
            storage: storage.clone(),
            context: context.clone().with_new_admission_event_id(),
            resolve_context: resolve_context.clone(),
            fixture_kind,
            trigger_source,
        })
    } else {
        None
    };
    if let Some(request) = deferred_request.as_ref() {
        if let Err(error) = PendingDevFixtureExecutionQueue::persist(request) {
            admission.discard_pending_plan(&plan.plan_id);
            return Err(DevFixtureExecutionError::Execution(error));
        }
    }

    let sink = ActionLogSink::new(storage.clone());
    if let Err(error) = sink.append_event(&ActionLogEvent::executor_admission_decision(
        context.admission_event_id.as_str(),
        &plan,
        &decision,
        context.created_at.as_str(),
    )) {
        if deferred_request.is_some() {
            admission.discard_pending_plan(&plan.plan_id);
            let _ = storage.delete_choreography_pending_execution_body(&plan.plan_id);
        }
        if should_execute {
            let _release = admission.release_plan_preserving_pending(&plan.plan_id);
        }
        return Err(DevFixtureExecutionError::ActionLog(error));
    }

    if !should_execute {
        if let (Some(pending_queue), Some(request)) = (pending_queue, deferred_request) {
            pending_queue.enqueue_persisted(request);
        }
        return Ok(ScheduledDevFixtureExecution {
            plan_id,
            decision,
            execution: None,
        });
    }

    Ok(ScheduledDevFixtureExecution {
        plan_id,
        decision: decision.clone(),
        execution: Some(AdmittedDevFixtureExecution {
            storage,
            context,
            resolve_context,
            fixture_kind,
            decision,
        }),
    })
}

pub(crate) fn execute_admitted_dev_fixture(
    executor: &impl ChoreographyStepExecutor,
    admitted: AdmittedDevFixtureExecution,
    mut refresh_active_step: impl FnMut(&str, &TimelineStep, SidecarInterruptPolicy) -> BuddyResult<()>,
    mut after_completed_step: impl FnMut(&str, &TimelineStep) -> BuddyResult<StepCompletionDecision>,
    mut release_failed_plan: impl FnMut(&str) -> BuddyResult<ChoreographyAdmissionRelease>,
    mut release_plan: impl FnMut(&str) -> BuddyResult<ChoreographyAdmissionRelease>,
    mut on_execution_failed: impl FnMut(
        &DevFixturePlan,
        &str,
        &BuddyError,
        ResolveContext,
    ) -> BuddyResult<()>,
) -> Result<ExecutedDevFixtureAdmission, DevFixtureExecutionError> {
    let AdmittedDevFixtureExecution {
        storage,
        context,
        resolve_context,
        fixture_kind,
        decision,
    } = admitted;
    let plan = create_dev_fixture_plan(fixture_kind, &context)
        .map_err(DevFixtureExecutionError::Execution)?;

    if let Err(error) = interrupt_preempted_active_step(executor, &decision) {
        let failure_log_result = append_dev_fixture_plan_failed(
            &ActionLogSink::new(storage.clone()),
            &plan,
            &context,
            error.to_string().as_str(),
        );
        let failed_step_id = plan
            .steps
            .first()
            .map(TimelineStep::step_id)
            .unwrap_or(plan.plan_id.as_str());
        let release_result = release_failed_plan(&plan.plan_id);
        let recovery_result =
            on_execution_failed(&plan, failed_step_id, &error, resolve_context.clone());
        release_result.map_err(DevFixtureExecutionError::Execution)?;
        recovery_result.map_err(DevFixtureExecutionError::Execution)?;
        failure_log_result?;
        return Err(DevFixtureExecutionError::Execution(error));
    }

    let plan_id = plan.plan_id.clone();
    let mut execution_failure_handled = false;
    let result = execute_dev_fixture_with_step_start(
        storage,
        executor,
        context,
        resolve_context,
        fixture_kind,
        |event| match event {
            DevFixtureExecutionEvent::StepStarting(step) => {
                refresh_active_step(plan_id.as_str(), step.step, step.interrupt_policy)?;
                Ok(StepCompletionDecision::Continue)
            }
            DevFixtureExecutionEvent::StepCompleted(step) => {
                after_completed_step(plan_id.as_str(), step.step)
            }
            DevFixtureExecutionEvent::ExecutionFailed {
                plan,
                step,
                error,
                resolve_context,
            } => {
                let _release = release_failed_plan(&plan.plan_id)?;
                execution_failure_handled = true;
                on_execution_failed(plan, step.step_id(), error, resolve_context.clone())?;
                Ok(StepCompletionDecision::Continue)
            }
        },
    );
    if !execution_failure_handled {
        let _release = release_plan(&plan.plan_id).map_err(DevFixtureExecutionError::Execution)?;
    }

    result.map(|report| ExecutedDevFixtureAdmission {
        report: DevFixtureAdmissionExecutionReport {
            plan_id: report.plan_id,
            decision,
            executed: true,
        },
    })
}

#[cfg(test)]
fn execute_dev_fixture_with_admission(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    fixture_kind: DevFixtureKind,
    trigger_source: ChoreographyTriggerSource,
) -> Result<DevFixtureAdmissionExecutionReport, DevFixtureExecutionError> {
    let plan = create_dev_fixture_plan(fixture_kind, &context)
        .map_err(DevFixtureExecutionError::Execution)?;
    let active_step = dev_fixture_plan_active_step(&plan, &resolve_context);
    let decision = admission.admit(
        ChoreographyAdmissionRequest::new(plan.plan_id.clone(), trigger_source)
            .with_active_step(active_step.step_id, active_step.interrupt_policy),
    );
    let should_execute = should_execute_admission_decision(&decision);

    let sink = ActionLogSink::new(storage.clone());
    if let Err(error) = sink.append_event(&ActionLogEvent::executor_admission_decision(
        context.admission_event_id.as_str(),
        &plan,
        &decision,
        context.created_at.as_str(),
    )) {
        if should_execute {
            let _release = admission.release_plan_preserving_pending(&plan.plan_id);
        }
        return Err(DevFixtureExecutionError::ActionLog(error));
    }

    if !should_execute {
        return Ok(DevFixtureAdmissionExecutionReport {
            plan_id: plan.plan_id,
            decision,
            executed: false,
        });
    }

    if let Err(error) = interrupt_preempted_active_step(executor, &decision) {
        let _release = admission.release_plan_preserving_pending(&plan.plan_id);
        return Err(DevFixtureExecutionError::Execution(error));
    }

    let recovery_storage = storage.clone();
    let plan_id = plan.plan_id.clone();
    let result = execute_dev_fixture_with_step_start(
        storage,
        executor,
        context,
        resolve_context,
        fixture_kind,
        |event| match event {
            DevFixtureExecutionEvent::StepStarting(step) => refresh_admitted_plan_active_step(
                admission,
                plan_id.as_str(),
                step.step,
                step.interrupt_policy,
            )
            .map(|()| StepCompletionDecision::Continue),
            DevFixtureExecutionEvent::StepCompleted(_) => Ok(StepCompletionDecision::Continue),
            DevFixtureExecutionEvent::ExecutionFailed {
                plan,
                step,
                error,
                resolve_context,
            } => {
                let _release = admission.release_plan_preserving_pending(&plan.plan_id);
                trigger_admitted_runtime_safe_fallback_after_dev_fixture_failure(
                    recovery_storage.clone(),
                    executor,
                    admission,
                    plan,
                    step.step_id(),
                    error,
                    resolve_context.clone(),
                );
                Ok(StepCompletionDecision::Continue)
            }
        },
    );
    let _release = admission.release_plan_preserving_pending(&plan.plan_id);

    result.map(|report| DevFixtureAdmissionExecutionReport {
        plan_id: report.plan_id,
        decision,
        executed: true,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn create_timeline_plan_from_macro_intent(
    intent: &MacroIntent,
    context: &MacroIntentExecutionContext,
    source_ref: serde_json::Value,
) -> BuddyResult<TimelinePlan> {
    let beat_plan = compile_macro_intent_to_beat_plan(
        intent,
        BeatPlanBuildContext {
            plan_id: context.plan_id.as_str(),
            beat_id: context.beat_id.as_str(),
            step_id: context.step_id.as_str(),
            source_ref,
            created_at: context.timeline.created_at.as_str(),
        },
    )?;
    let steps = compile_beat_plan_to_timeline_steps(&beat_plan)?;
    if steps.is_empty() {
        return Err(BuddyError::Validation(
            "macro intent must compile to at least one timeline step".to_owned(),
        ));
    }

    Ok(TimelinePlan {
        plan_id: beat_plan.plan_id,
        source_ref: beat_plan.source_ref,
        failure_policy: TimelineFailurePolicy::Abort,
        steps,
        created_at: beat_plan.created_at,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
struct TimelineActiveStep {
    step_id: String,
    interrupt_policy: SidecarInterruptPolicy,
}

fn timeline_plan_active_step(
    plan: &TimelinePlan,
    resolve_context: &ResolveContext,
) -> TimelineActiveStep {
    timeline_steps_active_step(plan.plan_id.as_str(), &plan.steps, resolve_context)
}

fn dev_fixture_plan_active_step(
    plan: &DevFixturePlan,
    resolve_context: &ResolveContext,
) -> TimelineActiveStep {
    timeline_steps_active_step(plan.plan_id.as_str(), &plan.steps, resolve_context)
}

fn runtime_safe_fallback_plan_active_step(
    plan: &RuntimeSafeFallbackPlan,
    resolve_context: &ResolveContext,
) -> TimelineActiveStep {
    timeline_steps_active_step(plan.plan_id.as_str(), &plan.steps, resolve_context)
}

fn timeline_steps_active_step(
    fallback_step_id: &str,
    steps: &[TimelineStep],
    resolve_context: &ResolveContext,
) -> TimelineActiveStep {
    let registry = ActionRegistry::load_bundled().ok();
    let Some(step) = steps.first() else {
        return TimelineActiveStep {
            step_id: fallback_step_id.to_owned(),
            interrupt_policy: SidecarInterruptPolicy::Interruptible,
        };
    };

    TimelineActiveStep {
        step_id: step.step_id().to_owned(),
        interrupt_policy: timeline_step_interrupt_policy(registry.as_ref(), resolve_context, step),
    }
}

fn timeline_step_interrupt_policy(
    registry: Option<&ActionRegistry>,
    resolve_context: &ResolveContext,
    step: &TimelineStep,
) -> SidecarInterruptPolicy {
    match step {
        TimelineStep::PlayAction(step) => registry
            .and_then(|registry| {
                resolve_play_action_step(registry, resolve_context, step)
                    .map(|resolution| resolution.interrupt_policy)
                    .ok()
            })
            .unwrap_or(SidecarInterruptPolicy::FinishStep),
        TimelineStep::MoveTo(_)
        | TimelineStep::MoveByPath(_)
        | TimelineStep::RestorePosition(_) => SidecarInterruptPolicy::Interruptible,
        TimelineStep::Skip(_) => SidecarInterruptPolicy::Interruptible,
        TimelineStep::Wait(_)
        | TimelineStep::Repeat(_)
        | TimelineStep::Choose(_)
        | TimelineStep::SetFallback(_)
        | TimelineStep::Retry(_)
        | TimelineStep::Replace(_)
        | TimelineStep::Recover(_)
        | TimelineStep::Try(_)
        | TimelineStep::SnapshotPosition(_) => SidecarInterruptPolicy::FinishStep,
    }
}

fn interrupt_preempted_active_step(
    executor: &impl ChoreographyStepExecutor,
    decision: &ChoreographyAdmissionDecision,
) -> BuddyResult<()> {
    let ChoreographyAdmissionDecision::Preempted {
        interrupted_step_id: Some(step_id),
        reason_code,
        ..
    } = decision
    else {
        return Ok(());
    };

    executor.interrupt_step(step_id, reason_code)
}

fn should_execute_admission_decision(decision: &ChoreographyAdmissionDecision) -> bool {
    matches!(
        decision,
        ChoreographyAdmissionDecision::Accepted { .. }
            | ChoreographyAdmissionDecision::Preempted { .. }
    )
}

fn refresh_admitted_plan_active_step(
    admission: &mut ChoreographyAdmissionState,
    plan_id: &str,
    step: &TimelineStep,
    interrupt_policy: SidecarInterruptPolicy,
) -> BuddyResult<()> {
    match admission.update_active_step_with_policy(plan_id, step.step_id(), interrupt_policy) {
        ChoreographyActiveStepUpdate::Updated { .. } => Ok(()),
        ChoreographyActiveStepUpdate::Stale {
            active_plan_id, ..
        } => Err(BuddyError::Runtime(format!(
            "choreography active step refresh ignored stale plan {plan_id}; active plan is {active_plan_id}"
        ))),
        ChoreographyActiveStepUpdate::NoActivePlan { .. } => Err(BuddyError::Runtime(format!(
            "choreography active step refresh failed because plan {plan_id} is no longer active"
        ))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DevFixtureKind {
    SinglePlayAction,
    AiMacroDemo,
}

fn execute_dev_fixture(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    fixture_kind: DevFixtureKind,
) -> Result<DevFixtureExecutionReport, DevFixtureExecutionError> {
    let recovery_storage = storage.clone();
    execute_dev_fixture_with_step_start(
        storage,
        executor,
        context,
        resolve_context,
        fixture_kind,
        |event| match event {
            DevFixtureExecutionEvent::StepStarting(_) => Ok(StepCompletionDecision::Continue),
            DevFixtureExecutionEvent::StepCompleted(_) => Ok(StepCompletionDecision::Continue),
            DevFixtureExecutionEvent::ExecutionFailed {
                plan,
                step,
                error,
                resolve_context,
            } => {
                trigger_runtime_safe_fallback_after_dev_fixture_failure(
                    recovery_storage.clone(),
                    executor,
                    plan,
                    step.step_id(),
                    error,
                    resolve_context.clone(),
                );
                Ok(StepCompletionDecision::Continue)
            }
        },
    )
}

struct TimelineStepStartingEvent<'a> {
    step: &'a TimelineStep,
    interrupt_policy: SidecarInterruptPolicy,
}

struct TimelineStepCompletedEvent<'a> {
    step: &'a TimelineStep,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StepCompletionDecision {
    Continue,
    RunPendingHandoffFinalizer { step_id: String },
    YieldToPendingPlan,
}

enum DevFixtureExecutionEvent<'a> {
    StepStarting(TimelineStepStartingEvent<'a>),
    StepCompleted(TimelineStepCompletedEvent<'a>),
    ExecutionFailed {
        plan: &'a DevFixturePlan,
        step: &'a TimelineStep,
        error: &'a BuddyError,
        resolve_context: &'a ResolveContext,
    },
}

#[cfg_attr(not(test), allow(dead_code))]
enum TimelineExecutionEvent<'a> {
    StepStarting(TimelineStepStartingEvent<'a>),
    StepCompleted(TimelineStepCompletedEvent<'a>),
    ExecutionFailed {
        plan: &'a TimelinePlan,
        failed_step_id: &'a str,
        error: &'a BuddyError,
        resolve_context: &'a ResolveContext,
    },
}

type TimelineExecutionEventHandler<'a> = dyn for<'event> FnMut(TimelineExecutionEvent<'event>) -> BuddyResult<StepCompletionDecision>
    + 'a;

#[derive(Default)]
struct TimelineRuntimeState {
    position_snapshots: HashMap<String, (i32, i32)>,
}

#[derive(Default)]
struct TimelineStepExecutionReport {
    completed_step_count: u64,
    failed_step_count: u64,
    skipped_step_count: u64,
    duration_ms: u64,
    yielded_to_pending_plan_after_step_id: Option<String>,
}

impl TimelineStepExecutionReport {
    fn completed(duration_ms: u64) -> Self {
        Self {
            completed_step_count: 1,
            failed_step_count: 0,
            skipped_step_count: 0,
            duration_ms,
            yielded_to_pending_plan_after_step_id: None,
        }
    }

    fn failed() -> Self {
        Self {
            completed_step_count: 0,
            failed_step_count: 1,
            skipped_step_count: 0,
            duration_ms: 0,
            yielded_to_pending_plan_after_step_id: None,
        }
    }

    fn skipped() -> Self {
        Self {
            completed_step_count: 0,
            failed_step_count: 0,
            skipped_step_count: 1,
            duration_ms: 0,
            yielded_to_pending_plan_after_step_id: None,
        }
    }

    fn add(&mut self, report: Self) {
        let Self {
            completed_step_count,
            failed_step_count,
            skipped_step_count,
            duration_ms,
            yielded_to_pending_plan_after_step_id,
        } = report;
        self.completed_step_count += completed_step_count;
        self.failed_step_count += failed_step_count;
        self.skipped_step_count += skipped_step_count;
        self.duration_ms += duration_ms;
        if self.yielded_to_pending_plan_after_step_id.is_none() {
            self.yielded_to_pending_plan_after_step_id = yielded_to_pending_plan_after_step_id;
        }
    }
}

struct TimelineStepExecutionFailure {
    error: Box<TimelineExecutionError>,
    report: TimelineStepExecutionReport,
    failed_step_id: Option<String>,
}

struct TimelineBranchExecutionError {
    error: Box<TimelineExecutionError>,
    report: TimelineStepExecutionReport,
    failed_step_id: Option<String>,
}

impl TimelineStepExecutionFailure {
    fn action_log(error: BuddyError) -> Self {
        Self {
            error: Box::new(TimelineExecutionError::ActionLog(error)),
            report: TimelineStepExecutionReport::default(),
            failed_step_id: None,
        }
    }

    fn execution(error: BuddyError) -> Self {
        Self {
            error: Box::new(TimelineExecutionError::Execution(error)),
            report: TimelineStepExecutionReport::failed(),
            failed_step_id: None,
        }
    }

    fn from_parts(
        error: TimelineExecutionError,
        report: TimelineStepExecutionReport,
        failed_step_id: Option<String>,
    ) -> Self {
        Self {
            error: Box::new(error),
            report,
            failed_step_id,
        }
    }
}

impl TimelineRuntimeState {
    fn save_position_snapshot(&mut self, snapshot_id: &str, position: (i32, i32)) {
        self.position_snapshots
            .insert(snapshot_id.to_owned(), position);
    }

    fn restore_position_step(&self, step: &RestorePositionStep) -> BuddyResult<MoveToStep> {
        let Some(&(x, y)) = self.position_snapshots.get(&step.snapshot_id) else {
            return Err(BuddyError::Runtime(format!(
                "timeline position snapshot is missing: {}",
                step.snapshot_id
            )));
        };

        Ok(MoveToStep {
            step_id: step.step_id.clone(),
            kind: "moveTo".to_owned(),
            target: MoveTarget::Position { x, y },
            after_action_id: step.after_action_id.clone(),
            fallback_after_action_id: step.fallback_after_action_id.clone(),
            timeout_ms: step.timeout_ms,
        })
    }
}

struct DevFixtureStepExecutionScope<'a> {
    registry: &'a ActionRegistry,
    resolve_context: &'a ResolveContext,
    sink: &'a ActionLogSink,
    plan: &'a DevFixturePlan,
    context: &'a DevFixtureExecutionContext,
}

#[cfg_attr(not(test), allow(dead_code))]
struct TimelineStepExecutionScope<'a> {
    registry: &'a ActionRegistry,
    resolve_context: &'a ResolveContext,
    sink: &'a ActionLogSink,
    plan: &'a TimelinePlan,
    context: &'a TimelineExecutionContext,
    trigger_source: ChoreographyTriggerSource,
}

struct RuntimeSafeFallbackStepExecutionScope<'a> {
    registry: &'a ActionRegistry,
    resolve_context: &'a ResolveContext,
    sink: &'a ActionLogSink,
    plan: &'a RuntimeSafeFallbackPlan,
    context: &'a RuntimeSafeFallbackExecutionContext,
}

#[cfg_attr(not(test), allow(dead_code))]
fn execute_timeline_plan_with_step_start(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    plan: &TimelinePlan,
    context: &TimelineExecutionContext,
    resolve_context: ResolveContext,
    trigger_source: ChoreographyTriggerSource,
    mut on_execution_event: impl FnMut(
        TimelineExecutionEvent<'_>,
    ) -> BuddyResult<StepCompletionDecision>,
) -> Result<TimelinePlanExecutionReport, TimelineExecutionError> {
    let registry = ActionRegistry::load_bundled().map_err(TimelineExecutionError::Execution)?;
    validate_pending_handoff_finalizer_interrupt_policies(&registry, &resolve_context, plan)
        .map_err(TimelineExecutionError::Execution)?;
    let sink = ActionLogSink::new(storage);

    sink.append_event(&ActionLogEvent::timeline_plan_started(
        context.plan_started_event_id.as_str(),
        plan,
        trigger_source,
        context.created_at.as_str(),
    ))
    .map_err(TimelineExecutionError::ActionLog)?;

    let mut completed_step_count = 0_u64;
    let mut failed_step_count = 0_u64;
    let mut skipped_step_count = 0_u64;
    let mut duration_ms = 0_u64;
    let step_scope = TimelineStepExecutionScope {
        registry: &registry,
        resolve_context: &resolve_context,
        sink: &sink,
        plan,
        context,
        trigger_source,
    };

    let mut runtime_state = TimelineRuntimeState::default();
    let mut step_index = 0;
    while let Some(step) = plan.steps.get(step_index) {
        let interrupt_policy = timeline_step_interrupt_policy(
            Some(step_scope.registry),
            step_scope.resolve_context,
            step,
        );
        if let Err(error) = on_execution_event(TimelineExecutionEvent::StepStarting(
            TimelineStepStartingEvent {
                step,
                interrupt_policy,
            },
        )) {
            let error_message = error.to_string();
            append_timeline_plan_failed(
                &sink,
                plan,
                context,
                trigger_source,
                ActionLogTimelinePlanStats {
                    completed_step_count,
                    failed_step_count,
                    skipped_step_count,
                    duration_ms,
                },
                error_message.as_str(),
            )?;
            let _recovery = on_execution_event(TimelineExecutionEvent::ExecutionFailed {
                plan,
                failed_step_id: step.step_id(),
                error: &error,
                resolve_context: &resolve_context,
            });

            return Err(TimelineExecutionError::Execution(error));
        }

        match execute_timeline_plan_step(
            executor,
            &step_scope,
            step,
            step_index,
            &mut runtime_state,
            &mut on_execution_event,
        ) {
            Ok(report) => {
                let yielded_after_step_id = report.yielded_to_pending_plan_after_step_id.clone();
                completed_step_count += report.completed_step_count;
                failed_step_count += report.failed_step_count;
                skipped_step_count += report.skipped_step_count;
                duration_ms += report.duration_ms;
                if let Some(yielded_after_step_id) = yielded_after_step_id {
                    sink.append_event(&ActionLogEvent::timeline_plan_interrupted(
                        context.plan_failed_event_id.clone(),
                        plan,
                        trigger_source,
                        ActionLogTimelinePlanStats {
                            completed_step_count,
                            failed_step_count,
                            skipped_step_count,
                            duration_ms,
                        },
                        yielded_after_step_id.as_str(),
                        context.completed_at.as_str(),
                    ))
                    .map_err(TimelineExecutionError::ActionLog)?;

                    return Ok(TimelinePlanExecutionReport {
                        plan_id: plan.plan_id.clone(),
                    });
                }
                match on_execution_event(TimelineExecutionEvent::StepCompleted(
                    TimelineStepCompletedEvent { step },
                )) {
                    Ok(StepCompletionDecision::Continue) => {
                        step_index += 1;
                    }
                    Ok(StepCompletionDecision::RunPendingHandoffFinalizer { step_id }) => {
                        step_index = pending_handoff_finalizer_step_index(
                            &plan.steps,
                            step_index,
                            step_id.as_str(),
                        )
                        .map_err(TimelineExecutionError::Execution)?;
                    }
                    Ok(StepCompletionDecision::YieldToPendingPlan) => {
                        sink.append_event(&ActionLogEvent::timeline_plan_interrupted(
                            context.plan_failed_event_id.clone(),
                            plan,
                            trigger_source,
                            ActionLogTimelinePlanStats {
                                completed_step_count,
                                failed_step_count,
                                skipped_step_count,
                                duration_ms,
                            },
                            step.step_id(),
                            context.completed_at.as_str(),
                        ))
                        .map_err(TimelineExecutionError::ActionLog)?;

                        return Ok(TimelinePlanExecutionReport {
                            plan_id: plan.plan_id.clone(),
                        });
                    }
                    Err(error) => {
                        let error_message = error.to_string();
                        append_timeline_plan_failed(
                            &sink,
                            plan,
                            context,
                            trigger_source,
                            ActionLogTimelinePlanStats {
                                completed_step_count,
                                failed_step_count,
                                skipped_step_count,
                                duration_ms,
                            },
                            error_message.as_str(),
                        )?;
                        let _recovery =
                            on_execution_event(TimelineExecutionEvent::ExecutionFailed {
                                plan,
                                failed_step_id: step.step_id(),
                                error: &error,
                                resolve_context: &resolve_context,
                            });

                        return Err(TimelineExecutionError::Execution(error));
                    }
                }
            }
            Err(failure) => match *failure.error {
                TimelineExecutionError::Execution(error) => {
                    let failed_step_id = failure
                        .failed_step_id
                        .as_deref()
                        .unwrap_or_else(|| step.step_id());
                    completed_step_count += failure.report.completed_step_count;
                    failed_step_count += failure.report.failed_step_count;
                    skipped_step_count += failure.report.skipped_step_count;
                    duration_ms += failure.report.duration_ms;
                    if plan.failure_policy == TimelineFailurePolicy::Continue {
                        step_index += 1;
                        continue;
                    }

                    let error_message = error.to_string();
                    append_timeline_plan_failed(
                        &sink,
                        plan,
                        context,
                        trigger_source,
                        ActionLogTimelinePlanStats {
                            completed_step_count,
                            failed_step_count,
                            skipped_step_count,
                            duration_ms,
                        },
                        error_message.as_str(),
                    )?;
                    let _recovery = on_execution_event(TimelineExecutionEvent::ExecutionFailed {
                        plan,
                        failed_step_id,
                        error: &error,
                        resolve_context: &resolve_context,
                    });

                    return Err(TimelineExecutionError::Execution(error));
                }
                error => return Err(error),
            },
        }
    }

    sink.append_event(&ActionLogEvent::timeline_plan_completed(
        context.plan_completed_event_id.clone(),
        plan,
        trigger_source,
        ActionLogTimelinePlanStats {
            completed_step_count,
            failed_step_count,
            skipped_step_count,
            duration_ms,
        },
        context.completed_at.as_str(),
    ))
    .map_err(TimelineExecutionError::ActionLog)?;

    Ok(TimelinePlanExecutionReport {
        plan_id: plan.plan_id.clone(),
    })
}

fn pending_handoff_finalizer_step_index(
    steps: &[TimelineStep],
    completed_step_index: usize,
    finalizer_step_id: &str,
) -> BuddyResult<usize> {
    let Some((finalizer_index, finalizer_step)) = steps
        .iter()
        .enumerate()
        .find(|(_, step)| step.step_id() == finalizer_step_id)
    else {
        return Err(BuddyError::Validation(format!(
            "pending handoff finalizer step is missing: {finalizer_step_id}"
        )));
    };
    if finalizer_index <= completed_step_index {
        return Err(BuddyError::Validation(format!(
            "pending handoff finalizer must be after the completed step: {finalizer_step_id}"
        )));
    }
    if !matches!(finalizer_step, TimelineStep::PlayAction(_)) {
        return Err(BuddyError::Validation(format!(
            "pending handoff finalizer must be a playAction step: {finalizer_step_id}"
        )));
    }

    Ok(finalizer_index)
}

fn validate_pending_handoff_finalizer_interrupt_policies(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    plan: &TimelinePlan,
) -> BuddyResult<()> {
    validate_pending_handoff_finalizer_interrupt_policies_in_steps(
        registry,
        resolve_context,
        &plan.steps,
    )
}

fn validate_pending_handoff_finalizer_interrupt_policies_in_steps(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    steps: &[TimelineStep],
) -> BuddyResult<()> {
    for source_step in steps {
        let Some(finalizer_step_id) = source_step.pending_handoff_finalizer_step_id() else {
            continue;
        };
        let TimelineStep::PlayAction(source_action) = source_step else {
            continue;
        };
        let source_resolution = resolve_play_action_step(registry, resolve_context, source_action)?;
        if source_resolution.interrupt_policy.accepts_interrupt() {
            return Err(BuddyError::Validation(format!(
                "pending handoff source action must finish before interruption: {}",
                source_step.step_id()
            )));
        }

        let Some(TimelineStep::PlayAction(finalizer_action)) = steps
            .iter()
            .find(|step| step.step_id() == finalizer_step_id)
        else {
            return Err(BuddyError::Validation(format!(
                "pending handoff finalizer must be a playAction step: {finalizer_step_id}"
            )));
        };
        let finalizer_resolution =
            resolve_play_action_step(registry, resolve_context, finalizer_action)?;
        if finalizer_resolution.interrupt_policy.accepts_interrupt() {
            return Err(BuddyError::Validation(format!(
                "pending handoff finalizer action must finish before interruption: {finalizer_step_id}"
            )));
        }
    }

    for step in steps {
        match step {
            TimelineStep::Repeat(step) => {
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.steps,
                )?;
            }
            TimelineStep::Choose(step) => {
                for option in &step.options {
                    validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                        registry,
                        resolve_context,
                        &option.steps,
                    )?;
                }
            }
            TimelineStep::SetFallback(step) => {
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.steps,
                )?;
            }
            TimelineStep::Retry(step) => {
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.steps,
                )?;
            }
            TimelineStep::Replace(step) => {
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.steps,
                )?;
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.replacement_steps,
                )?;
            }
            TimelineStep::Recover(step) => {
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.steps,
                )?;
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.recovery_steps,
                )?;
            }
            TimelineStep::Try(step) => {
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.steps,
                )?;
                validate_pending_handoff_finalizer_interrupt_policies_in_steps(
                    registry,
                    resolve_context,
                    &step.fallback_steps,
                )?;
            }
            TimelineStep::PlayAction(_)
            | TimelineStep::MoveTo(_)
            | TimelineStep::MoveByPath(_)
            | TimelineStep::Wait(_)
            | TimelineStep::Skip(_)
            | TimelineStep::SnapshotPosition(_)
            | TimelineStep::RestorePosition(_) => {}
        }
    }

    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn append_timeline_plan_failed(
    sink: &ActionLogSink,
    plan: &TimelinePlan,
    context: &TimelineExecutionContext,
    trigger_source: ChoreographyTriggerSource,
    stats: ActionLogTimelinePlanStats,
    error_message: &str,
) -> Result<(), TimelineExecutionError> {
    sink.append_event(&ActionLogEvent::timeline_plan_failed(
        context.plan_failed_event_id.clone(),
        plan,
        trigger_source,
        stats,
        error_message,
        context.completed_at.as_str(),
    ))
    .map_err(TimelineExecutionError::ActionLog)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn trigger_admitted_runtime_safe_fallback_after_timeline_failure(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    failed_plan: &TimelinePlan,
    failed_step_id: &str,
    error: &BuddyError,
    resolve_context: ResolveContext,
) -> Option<ChoreographyRuntimeDegradation> {
    trigger_admitted_runtime_safe_fallback(
        storage,
        executor,
        admission,
        RuntimeSafeFallbackTrigger {
            triggered_by_plan_id: failed_plan.plan_id.as_str(),
            triggered_by_step_id: Some(failed_step_id),
            trigger_reason: runtime_safe_fallback_reason_for_execution_error(error),
        },
        resolve_context,
    )
}

fn execute_dev_fixture_with_step_start(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    context: DevFixtureExecutionContext,
    resolve_context: ResolveContext,
    fixture_kind: DevFixtureKind,
    mut on_execution_event: impl FnMut(
        DevFixtureExecutionEvent<'_>,
    ) -> BuddyResult<StepCompletionDecision>,
) -> Result<DevFixtureExecutionReport, DevFixtureExecutionError> {
    let registry = ActionRegistry::load_bundled().map_err(DevFixtureExecutionError::Execution)?;
    let sink = ActionLogSink::new(storage.clone());
    let plan = create_dev_fixture_plan(fixture_kind, &context)
        .map_err(DevFixtureExecutionError::Execution)?;

    sink.append_event(&ActionLogEvent::plan_started(
        context.plan_started_event_id.as_str(),
        &plan,
        context.created_at.as_str(),
    ))
    .map_err(DevFixtureExecutionError::ActionLog)?;

    let mut completed_step_count = 0_u64;
    let mut duration_ms = 0_u64;
    let step_scope = DevFixtureStepExecutionScope {
        registry: &registry,
        resolve_context: &resolve_context,
        sink: &sink,
        plan: &plan,
        context: &context,
    };

    let mut step_index = 0;
    while let Some(step) = plan.steps.get(step_index) {
        let interrupt_policy = timeline_step_interrupt_policy(
            Some(step_scope.registry),
            step_scope.resolve_context,
            step,
        );
        if let Err(error) = on_execution_event(DevFixtureExecutionEvent::StepStarting(
            TimelineStepStartingEvent {
                step,
                interrupt_policy,
            },
        )) {
            let error_message = error.to_string();
            append_dev_fixture_plan_failed(&sink, &plan, &context, error_message.as_str())?;
            let _recovery = on_execution_event(DevFixtureExecutionEvent::ExecutionFailed {
                plan: &plan,
                step,
                error: &error,
                resolve_context: &resolve_context,
            });

            return Err(DevFixtureExecutionError::Execution(error));
        }
        match execute_dev_fixture_timeline_step(executor, &step_scope, step, step_index) {
            Ok(step_report) => {
                completed_step_count += step_report.completed_step_count;
                duration_ms += step_report.duration_ms;
                match on_execution_event(DevFixtureExecutionEvent::StepCompleted(
                    TimelineStepCompletedEvent { step },
                )) {
                    Ok(StepCompletionDecision::Continue) => {
                        step_index += 1;
                    }
                    Ok(StepCompletionDecision::RunPendingHandoffFinalizer { step_id }) => {
                        step_index = pending_handoff_finalizer_step_index(
                            &plan.steps,
                            step_index,
                            step_id.as_str(),
                        )
                        .map_err(DevFixtureExecutionError::Execution)?;
                    }
                    Ok(StepCompletionDecision::YieldToPendingPlan) => {
                        sink.append_event(&ActionLogEvent::plan_interrupted(
                            context.plan_failed_event_id.clone(),
                            &plan,
                            completed_step_count,
                            duration_ms,
                            step.step_id(),
                            context.completed_at.as_str(),
                        ))
                        .map_err(DevFixtureExecutionError::ActionLog)?;

                        return Ok(DevFixtureExecutionReport {
                            plan_id: plan.plan_id,
                        });
                    }
                    Err(error) => {
                        let error_message = error.to_string();
                        append_dev_fixture_plan_failed(
                            &sink,
                            &plan,
                            &context,
                            error_message.as_str(),
                        )?;
                        let _recovery =
                            on_execution_event(DevFixtureExecutionEvent::ExecutionFailed {
                                plan: &plan,
                                step,
                                error: &error,
                                resolve_context: &resolve_context,
                            });

                        return Err(DevFixtureExecutionError::Execution(error));
                    }
                }
            }
            Err(DevFixtureExecutionError::Execution(error)) => {
                let error_message = error.to_string();
                append_dev_fixture_plan_failed(&sink, &plan, &context, error_message.as_str())?;
                let _recovery = on_execution_event(DevFixtureExecutionEvent::ExecutionFailed {
                    plan: &plan,
                    step,
                    error: &error,
                    resolve_context: &resolve_context,
                });

                return Err(DevFixtureExecutionError::Execution(error));
            }
            Err(error) => return Err(error),
        }
    }

    sink.append_event(&ActionLogEvent::plan_completed(
        context.plan_completed_event_id,
        &plan,
        completed_step_count,
        duration_ms,
        context.completed_at,
    ))
    .map_err(DevFixtureExecutionError::ActionLog)?;

    Ok(DevFixtureExecutionReport {
        plan_id: plan.plan_id,
    })
}

fn append_dev_fixture_plan_failed(
    sink: &ActionLogSink,
    plan: &DevFixturePlan,
    context: &DevFixtureExecutionContext,
    error_message: &str,
) -> Result<(), DevFixtureExecutionError> {
    sink.append_event(&ActionLogEvent::plan_failed(
        context.plan_failed_event_id.clone(),
        plan,
        error_message,
        context.completed_at.as_str(),
    ))
    .map_err(DevFixtureExecutionError::ActionLog)
}

fn trigger_runtime_safe_fallback_after_dev_fixture_failure(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    failed_plan: &DevFixturePlan,
    failed_step_id: &str,
    error: &BuddyError,
    resolve_context: ResolveContext,
) -> Option<ChoreographyRuntimeDegradation> {
    let context = RuntimeSafeFallbackExecutionContext::new();
    let plan = create_runtime_safe_fallback_plan_for_trigger(
        &context,
        RuntimeSafeFallbackTrigger {
            triggered_by_plan_id: failed_plan.plan_id.as_str(),
            triggered_by_step_id: Some(failed_step_id),
            trigger_reason: runtime_safe_fallback_reason_for_execution_error(error),
        },
    );

    let degraded_at = context.completed_at.clone();
    match execute_runtime_safe_fallback_plan(storage, executor, plan, context, resolve_context) {
        Err(RuntimeSafeFallbackExecutionError::Execution(_)) => Some(
            ChoreographyRuntimeDegradation::system_recovery_failed(degraded_at),
        ),
        Err(RuntimeSafeFallbackExecutionError::ActionLog(_)) | Ok(_) => None,
    }
}

pub(crate) fn trigger_admitted_runtime_safe_fallback_after_dev_fixture_failure(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    failed_plan: &DevFixturePlan,
    failed_step_id: &str,
    error: &BuddyError,
    resolve_context: ResolveContext,
) -> Option<ChoreographyRuntimeDegradation> {
    trigger_admitted_runtime_safe_fallback(
        storage,
        executor,
        admission,
        RuntimeSafeFallbackTrigger {
            triggered_by_plan_id: failed_plan.plan_id.as_str(),
            triggered_by_step_id: Some(failed_step_id),
            trigger_reason: runtime_safe_fallback_reason_for_execution_error(error),
        },
        resolve_context,
    )
}

pub(crate) fn trigger_admitted_runtime_safe_fallback_after_macro_planning_failure(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    context: &MacroIntentExecutionContext,
    resolve_context: ResolveContext,
) -> Option<ChoreographyRuntimeDegradation> {
    trigger_admitted_runtime_safe_fallback(
        storage,
        executor,
        admission,
        RuntimeSafeFallbackTrigger {
            triggered_by_plan_id: context.plan_id.as_str(),
            triggered_by_step_id: None,
            trigger_reason: RuntimeSafeFallbackReason::MacroPlanningFailed,
        },
        resolve_context,
    )
}

fn trigger_admitted_runtime_safe_fallback(
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    trigger: RuntimeSafeFallbackTrigger<'_>,
    resolve_context: ResolveContext,
) -> Option<ChoreographyRuntimeDegradation> {
    let admitted = match admit_runtime_safe_fallback_plan(storage.clone(), admission, trigger) {
        Ok(admitted) => admitted,
        Err(_) => return None,
    };

    if !admitted.should_execute {
        return None;
    }

    if interrupt_preempted_active_step(executor, &admitted.decision).is_err() {
        let _release = admission.release_plan_preserving_pending(&admitted.plan.plan_id);
        return None;
    }

    let plan_id = admitted.plan.plan_id.clone();
    let degraded_at = admitted.context.completed_at.clone();
    let result = execute_runtime_safe_fallback_plan(
        storage,
        executor,
        admitted.plan,
        admitted.context,
        resolve_context,
    );
    let _release = admission.release_plan_preserving_pending(&plan_id);

    match result {
        Err(RuntimeSafeFallbackExecutionError::Execution(_)) => Some(
            ChoreographyRuntimeDegradation::system_recovery_failed(degraded_at),
        ),
        Err(RuntimeSafeFallbackExecutionError::ActionLog(_)) | Ok(_) => None,
    }
}

fn create_runtime_safe_fallback_plan_for_trigger(
    context: &RuntimeSafeFallbackExecutionContext,
    trigger: RuntimeSafeFallbackTrigger<'_>,
) -> RuntimeSafeFallbackPlan {
    create_runtime_safe_fallback_plan(RuntimeSafeFallbackPlanContext {
        plan_id: context.plan_id.as_str(),
        step_id: context.step_id.as_str(),
        triggered_by_plan_id: trigger.triggered_by_plan_id,
        triggered_by_step_id: trigger.triggered_by_step_id,
        trigger_reason: trigger.trigger_reason,
        created_at: context.created_at.as_str(),
    })
}

fn append_runtime_safe_fallback_plan_failed(
    sink: &ActionLogSink,
    plan: &RuntimeSafeFallbackPlan,
    context: &RuntimeSafeFallbackExecutionContext,
    error_message: &str,
) -> Result<(), RuntimeSafeFallbackExecutionError> {
    sink.append_event(&ActionLogEvent::system_recovery_plan_failed(
        context.plan_failed_event_id.clone(),
        plan,
        error_message,
        context.completed_at.as_str(),
    ))
    .map_err(RuntimeSafeFallbackExecutionError::ActionLog)
}

fn append_runtime_degraded_after_system_recovery_failed(
    sink: &ActionLogSink,
    plan: &RuntimeSafeFallbackPlan,
    error_message: &str,
    created_at: &str,
) -> Result<(), RuntimeSafeFallbackExecutionError> {
    sink.append_system_event(
        &ActionLogSystemEvent::runtime_degraded_after_system_recovery_failed(
            prefixed_uuid_v7("evt"),
            plan,
            error_message,
            created_at,
        ),
    )
    .map_err(RuntimeSafeFallbackExecutionError::ActionLog)
}

#[cfg_attr(not(test), allow(dead_code))]
fn execute_timeline_plan_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    step: &TimelineStep,
    step_index: usize,
    runtime_state: &mut TimelineRuntimeState,
    on_execution_event: &mut TimelineExecutionEventHandler<'_>,
) -> Result<TimelineStepExecutionReport, TimelineStepExecutionFailure> {
    match step {
        TimelineStep::PlayAction(step) => {
            let resolution = resolve_play_action_step(scope.registry, scope.resolve_context, step)
                .map_err(TimelineStepExecutionFailure::execution)?;
            scope
                .sink
                .append_event(&ActionLogEvent::timeline_step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    &resolution,
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            if let Err(error) = executor.play_action_step(step, &resolution) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::timeline_step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        scope.trigger_source,
                        &resolution,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(TimelineStepExecutionFailure::action_log)?;

                return Err(TimelineStepExecutionFailure::execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::timeline_step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    &resolution,
                    resolution.duration_ms,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            Ok(TimelineStepExecutionReport::completed(
                resolution.duration_ms,
            ))
        }
        TimelineStep::MoveTo(step) => {
            let after_action_resolution =
                resolve_move_to_after_action(scope.registry, scope.resolve_context, step)
                    .map_err(TimelineStepExecutionFailure::execution)?;
            let after_animation_ref = after_action_resolution
                .as_ref()
                .map(|resolution| resolution.animation_ref.as_str());
            scope
                .sink
                .append_event(&ActionLogEvent::timeline_move_to_step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    after_action_resolution.as_ref(),
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            if let Err(error) = executor.move_to_step(step, after_animation_ref) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::timeline_move_to_step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        scope.trigger_source,
                        step,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(TimelineStepExecutionFailure::action_log)?;

                return Err(TimelineStepExecutionFailure::execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::timeline_move_to_step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            Ok(TimelineStepExecutionReport::completed(0))
        }
        TimelineStep::MoveByPath(step) => {
            let after_action_resolution =
                resolve_move_by_path_after_action(scope.registry, scope.resolve_context, step)
                    .map_err(TimelineStepExecutionFailure::execution)?;
            let after_animation_ref = after_action_resolution
                .as_ref()
                .map(|resolution| resolution.animation_ref.as_str());
            scope
                .sink
                .append_event(&ActionLogEvent::timeline_move_by_path_step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    after_action_resolution.as_ref(),
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            if let Err(error) = executor.move_by_path_step(step, after_animation_ref) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::timeline_move_by_path_step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        scope.trigger_source,
                        step,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(TimelineStepExecutionFailure::action_log)?;

                return Err(TimelineStepExecutionFailure::execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::timeline_move_by_path_step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            Ok(TimelineStepExecutionReport::completed(0))
        }
        TimelineStep::Wait(step) => {
            validate_wait_step(step).map_err(TimelineStepExecutionFailure::execution)?;
            scope
                .sink
                .append_event(&ActionLogEvent::timeline_wait_step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            if let Err(error) = executor.wait_step(step) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::timeline_wait_step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        scope.trigger_source,
                        step,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(TimelineStepExecutionFailure::action_log)?;

                return Err(TimelineStepExecutionFailure::execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::timeline_wait_step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    step.duration_ms,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            Ok(TimelineStepExecutionReport::completed(step.duration_ms))
        }
        TimelineStep::Skip(step) => {
            scope
                .sink
                .append_event(&ActionLogEvent::timeline_skip_step_skipped(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            Ok(TimelineStepExecutionReport::skipped())
        }
        TimelineStep::Retry(step) => execute_timeline_retry_step(
            executor,
            scope,
            step,
            step_index,
            runtime_state,
            on_execution_event,
        ),
        TimelineStep::Replace(step) => execute_timeline_replace_step(
            executor,
            scope,
            step,
            step_index,
            runtime_state,
            on_execution_event,
        ),
        TimelineStep::Recover(step) => execute_timeline_recover_step(
            executor,
            scope,
            step,
            step_index,
            runtime_state,
            on_execution_event,
        ),
        TimelineStep::Try(step) => execute_timeline_try_step(
            executor,
            scope,
            step,
            step_index,
            runtime_state,
            on_execution_event,
        ),
        TimelineStep::SnapshotPosition(step) => execute_timeline_snapshot_position_step(
            executor,
            scope,
            step,
            step_index,
            runtime_state,
        ),
        TimelineStep::RestorePosition(step) => {
            execute_timeline_restore_position_step(executor, scope, step, step_index, runtime_state)
        }
        TimelineStep::Repeat(_) | TimelineStep::Choose(_) | TimelineStep::SetFallback(_) => Err(
            TimelineStepExecutionFailure::execution(planner_side_timeline_step_error(step)),
        ),
    }
}

fn execute_timeline_replace_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    step: &ReplaceStep,
    step_index: usize,
    runtime_state: &mut TimelineRuntimeState,
    on_execution_event: &mut TimelineExecutionEventHandler<'_>,
) -> Result<TimelineStepExecutionReport, TimelineStepExecutionFailure> {
    match execute_timeline_branch_steps(
        executor,
        scope,
        &step.steps,
        step_index,
        0,
        runtime_state,
        on_execution_event,
    ) {
        Ok(report) => Ok(report),
        Err(primary_failure) => match *primary_failure.error {
            TimelineExecutionError::ActionLog(error) => {
                Err(TimelineStepExecutionFailure::from_parts(
                    TimelineExecutionError::ActionLog(error),
                    primary_failure.report,
                    primary_failure.failed_step_id,
                ))
            }
            TimelineExecutionError::Execution(_) => {
                let mut report = primary_failure.report;
                match execute_timeline_branch_steps(
                    executor,
                    scope,
                    &step.replacement_steps,
                    step_index,
                    1,
                    runtime_state,
                    on_execution_event,
                ) {
                    Ok(replacement_report) => {
                        report.add(replacement_report);
                        Ok(report)
                    }
                    Err(replacement_failure) => {
                        report.add(replacement_failure.report);
                        Err(TimelineStepExecutionFailure::from_parts(
                            *replacement_failure.error,
                            report,
                            replacement_failure.failed_step_id,
                        ))
                    }
                }
            }
        },
    }
}

fn execute_timeline_recover_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    step: &RecoverStep,
    step_index: usize,
    runtime_state: &mut TimelineRuntimeState,
    on_execution_event: &mut TimelineExecutionEventHandler<'_>,
) -> Result<TimelineStepExecutionReport, TimelineStepExecutionFailure> {
    match execute_timeline_branch_steps(
        executor,
        scope,
        &step.steps,
        step_index,
        0,
        runtime_state,
        on_execution_event,
    ) {
        Ok(report) => Ok(report),
        Err(primary_failure) => match *primary_failure.error {
            TimelineExecutionError::ActionLog(error) => {
                Err(TimelineStepExecutionFailure::from_parts(
                    TimelineExecutionError::ActionLog(error),
                    primary_failure.report,
                    primary_failure.failed_step_id,
                ))
            }
            TimelineExecutionError::Execution(_) => {
                let mut report = primary_failure.report;
                match execute_timeline_branch_steps(
                    executor,
                    scope,
                    &step.recovery_steps,
                    step_index,
                    1,
                    runtime_state,
                    on_execution_event,
                ) {
                    Ok(recovery_report) => {
                        report.add(recovery_report);
                        Ok(report)
                    }
                    Err(recovery_failure) => {
                        report.add(recovery_failure.report);
                        Err(TimelineStepExecutionFailure::from_parts(
                            *recovery_failure.error,
                            report,
                            recovery_failure.failed_step_id,
                        ))
                    }
                }
            }
        },
    }
}

fn execute_timeline_retry_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    step: &RetryStep,
    step_index: usize,
    runtime_state: &mut TimelineRuntimeState,
    on_execution_event: &mut TimelineExecutionEventHandler<'_>,
) -> Result<TimelineStepExecutionReport, TimelineStepExecutionFailure> {
    let mut report = TimelineStepExecutionReport::default();

    for attempt_index in 0..step.max_attempts {
        match execute_timeline_branch_steps(
            executor,
            scope,
            &step.steps,
            step_index,
            usize::from(attempt_index),
            runtime_state,
            on_execution_event,
        ) {
            Ok(attempt_report) => {
                report.add(attempt_report);
                return Ok(report);
            }
            Err(attempt_failure) => {
                report.add(attempt_failure.report);
                if matches!(
                    attempt_failure.error.as_ref(),
                    TimelineExecutionError::ActionLog(_)
                ) || attempt_index + 1 == step.max_attempts
                {
                    return Err(TimelineStepExecutionFailure::from_parts(
                        *attempt_failure.error,
                        report,
                        attempt_failure.failed_step_id,
                    ));
                }
            }
        }
    }

    Err(TimelineStepExecutionFailure::execution(
        BuddyError::Validation(format!(
            "retry timeline step must contain at least one attempt: {}",
            step.step_id
        )),
    ))
}

fn execute_timeline_try_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    step: &TryStep,
    step_index: usize,
    runtime_state: &mut TimelineRuntimeState,
    on_execution_event: &mut TimelineExecutionEventHandler<'_>,
) -> Result<TimelineStepExecutionReport, TimelineStepExecutionFailure> {
    match execute_timeline_branch_steps(
        executor,
        scope,
        &step.steps,
        step_index,
        0,
        runtime_state,
        on_execution_event,
    ) {
        Ok(report) => Ok(report),
        Err(primary_failure) => match *primary_failure.error {
            TimelineExecutionError::ActionLog(error) => {
                Err(TimelineStepExecutionFailure::from_parts(
                    TimelineExecutionError::ActionLog(error),
                    primary_failure.report,
                    primary_failure.failed_step_id,
                ))
            }
            TimelineExecutionError::Execution(_) => {
                let mut report = primary_failure.report;
                match execute_timeline_branch_steps(
                    executor,
                    scope,
                    &step.fallback_steps,
                    step_index,
                    1,
                    runtime_state,
                    on_execution_event,
                ) {
                    Ok(fallback_report) => {
                        report.add(fallback_report);
                        Ok(report)
                    }
                    Err(fallback_failure) => {
                        report.add(fallback_failure.report);
                        Err(TimelineStepExecutionFailure::from_parts(
                            *fallback_failure.error,
                            report,
                            fallback_failure.failed_step_id,
                        ))
                    }
                }
            }
        },
    }
}

fn execute_timeline_branch_steps(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    steps: &[TimelineStep],
    parent_step_index: usize,
    branch_index: usize,
    runtime_state: &mut TimelineRuntimeState,
    on_execution_event: &mut TimelineExecutionEventHandler<'_>,
) -> Result<TimelineStepExecutionReport, TimelineBranchExecutionError> {
    let mut report = TimelineStepExecutionReport::default();
    let mut nested_index = 0;
    while let Some(step) = steps.get(nested_index) {
        let nested_step_index =
            nested_timeline_step_event_index(parent_step_index, branch_index, nested_index);
        let interrupt_policy =
            timeline_step_interrupt_policy(Some(scope.registry), scope.resolve_context, step);
        if let Err(error) = on_execution_event(TimelineExecutionEvent::StepStarting(
            TimelineStepStartingEvent {
                step,
                interrupt_policy,
            },
        )) {
            return Err(TimelineBranchExecutionError {
                error: Box::new(TimelineExecutionError::Execution(error)),
                report,
                failed_step_id: Some(step.step_id().to_owned()),
            });
        }

        match execute_timeline_plan_step(
            executor,
            scope,
            step,
            nested_step_index,
            runtime_state,
            on_execution_event,
        ) {
            Ok(step_report) => {
                report.add(step_report);
                if report.yielded_to_pending_plan_after_step_id.is_some() {
                    return Ok(report);
                }

                let decision = on_execution_event(TimelineExecutionEvent::StepCompleted(
                    TimelineStepCompletedEvent { step },
                ));
                match decision {
                    Ok(StepCompletionDecision::Continue) => {
                        nested_index += 1;
                    }
                    Ok(StepCompletionDecision::RunPendingHandoffFinalizer { step_id }) => {
                        nested_index = match pending_handoff_finalizer_step_index(
                            steps,
                            nested_index,
                            step_id.as_str(),
                        ) {
                            Ok(finalizer_index) => finalizer_index,
                            Err(error) => {
                                return Err(TimelineBranchExecutionError {
                                    error: Box::new(TimelineExecutionError::Execution(error)),
                                    report,
                                    failed_step_id: Some(step.step_id().to_owned()),
                                });
                            }
                        };
                    }
                    Ok(StepCompletionDecision::YieldToPendingPlan) => {
                        report.yielded_to_pending_plan_after_step_id =
                            Some(step.step_id().to_owned());
                        return Ok(report);
                    }
                    Err(error) => {
                        return Err(TimelineBranchExecutionError {
                            error: Box::new(TimelineExecutionError::Execution(error)),
                            report,
                            failed_step_id: Some(step.step_id().to_owned()),
                        });
                    }
                }
            }
            Err(failure) => {
                let failed_step_id = failure
                    .failed_step_id
                    .unwrap_or_else(|| step.step_id().to_owned());
                report.add(failure.report);
                return Err(TimelineBranchExecutionError {
                    error: failure.error,
                    report,
                    failed_step_id: Some(failed_step_id),
                });
            }
        }
    }

    Ok(report)
}

fn nested_timeline_step_event_index(
    parent_step_index: usize,
    branch_index: usize,
    nested_index: usize,
) -> usize {
    ((parent_step_index + 1) * 1_000_000) + (branch_index * 100_000) + nested_index
}

fn execute_timeline_snapshot_position_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    step: &SnapshotPositionStep,
    step_index: usize,
    runtime_state: &mut TimelineRuntimeState,
) -> Result<TimelineStepExecutionReport, TimelineStepExecutionFailure> {
    scope
        .sink
        .append_event(&ActionLogEvent::timeline_snapshot_position_step_resolved(
            ActionLogEventIds {
                event_id: &indexed_event_id(&scope.context.step_resolved_event_id, step_index),
                plan_id: &scope.plan.plan_id,
                step_id: Some(&step.step_id),
            },
            &scope.plan.source_ref,
            scope.trigger_source,
            step,
            scope.context.resolved_at.as_str(),
        ))
        .map_err(TimelineStepExecutionFailure::action_log)?;

    let position = match executor.query_state_position() {
        Ok(Some(position)) => position,
        Ok(None) => {
            let error = snapshot_position_unavailable_error(step);
            let error_message = error.to_string();
            scope
                .sink
                .append_event(&ActionLogEvent::timeline_snapshot_position_step_failed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_failed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    error_message.as_str(),
                    scope.context.completed_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            return Err(TimelineStepExecutionFailure::execution(error));
        }
        Err(error) => {
            let error_message = error.to_string();
            scope
                .sink
                .append_event(&ActionLogEvent::timeline_snapshot_position_step_failed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_failed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    scope.trigger_source,
                    step,
                    error_message.as_str(),
                    scope.context.completed_at.as_str(),
                ))
                .map_err(TimelineStepExecutionFailure::action_log)?;

            return Err(TimelineStepExecutionFailure::execution(error));
        }
    };

    runtime_state.save_position_snapshot(step.snapshot_id.as_str(), position);
    scope
        .sink
        .append_event(&ActionLogEvent::timeline_snapshot_position_step_completed(
            ActionLogEventIds {
                event_id: &indexed_event_id(&scope.context.step_completed_event_id, step_index),
                plan_id: &scope.plan.plan_id,
                step_id: Some(&step.step_id),
            },
            &scope.plan.source_ref,
            scope.trigger_source,
            step,
            position,
            scope.context.completed_at.as_str(),
        ))
        .map_err(TimelineStepExecutionFailure::action_log)?;

    Ok(TimelineStepExecutionReport::completed(0))
}

fn execute_timeline_restore_position_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &TimelineStepExecutionScope<'_>,
    step: &RestorePositionStep,
    step_index: usize,
    runtime_state: &mut TimelineRuntimeState,
) -> Result<TimelineStepExecutionReport, TimelineStepExecutionFailure> {
    let move_to_step = runtime_state
        .restore_position_step(step)
        .map_err(TimelineStepExecutionFailure::execution)?;
    let after_action_resolution =
        resolve_move_to_after_action(scope.registry, scope.resolve_context, &move_to_step)
            .map_err(TimelineStepExecutionFailure::execution)?;
    let after_animation_ref = after_action_resolution
        .as_ref()
        .map(|resolution| resolution.animation_ref.as_str());

    scope
        .sink
        .append_event(&ActionLogEvent::timeline_restore_position_step_resolved(
            ActionLogEventIds {
                event_id: &indexed_event_id(&scope.context.step_resolved_event_id, step_index),
                plan_id: &scope.plan.plan_id,
                step_id: Some(&step.step_id),
            },
            &scope.plan.source_ref,
            scope.trigger_source,
            ActionLogRestorePositionResolution {
                step,
                move_to_step: &move_to_step,
                after_action_resolution: after_action_resolution.as_ref(),
                resolve_context: scope.resolve_context,
            },
            scope.context.resolved_at.as_str(),
        ))
        .map_err(TimelineStepExecutionFailure::action_log)?;

    if let Err(error) = executor.move_to_step(&move_to_step, after_animation_ref) {
        let error_message = error.to_string();
        scope
            .sink
            .append_event(&ActionLogEvent::timeline_restore_position_step_failed(
                ActionLogEventIds {
                    event_id: &indexed_event_id(&scope.context.step_failed_event_id, step_index),
                    plan_id: &scope.plan.plan_id,
                    step_id: Some(&step.step_id),
                },
                &scope.plan.source_ref,
                scope.trigger_source,
                step,
                error_message.as_str(),
                scope.context.completed_at.as_str(),
            ))
            .map_err(TimelineStepExecutionFailure::action_log)?;

        return Err(TimelineStepExecutionFailure::execution(error));
    }

    scope
        .sink
        .append_event(&ActionLogEvent::timeline_restore_position_step_completed(
            ActionLogEventIds {
                event_id: &indexed_event_id(&scope.context.step_completed_event_id, step_index),
                plan_id: &scope.plan.plan_id,
                step_id: Some(&step.step_id),
            },
            &scope.plan.source_ref,
            scope.trigger_source,
            step,
            &move_to_step,
            scope.context.completed_at.as_str(),
        ))
        .map_err(TimelineStepExecutionFailure::action_log)?;

    Ok(TimelineStepExecutionReport::completed(0))
}

fn execute_dev_fixture_timeline_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &DevFixtureStepExecutionScope<'_>,
    step: &TimelineStep,
    step_index: usize,
) -> Result<TimelineStepExecutionReport, DevFixtureExecutionError> {
    match step {
        TimelineStep::PlayAction(step) => {
            let resolution = resolve_play_action_step(scope.registry, scope.resolve_context, step)
                .map_err(DevFixtureExecutionError::Execution)?;
            scope
                .sink
                .append_event(&ActionLogEvent::step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    &resolution,
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            if let Err(error) = executor.play_action_step(step, &resolution) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        &resolution,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(DevFixtureExecutionError::ActionLog)?;

                return Err(DevFixtureExecutionError::Execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    &resolution,
                    resolution.duration_ms,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            Ok(TimelineStepExecutionReport::completed(
                resolution.duration_ms,
            ))
        }
        TimelineStep::MoveTo(step) => {
            let after_action_resolution =
                resolve_move_to_after_action(scope.registry, scope.resolve_context, step)
                    .map_err(DevFixtureExecutionError::Execution)?;
            let after_animation_ref = after_action_resolution
                .as_ref()
                .map(|resolution| resolution.animation_ref.as_str());
            scope
                .sink
                .append_event(&ActionLogEvent::move_to_step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    step,
                    after_action_resolution.as_ref(),
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            if let Err(error) = executor.move_to_step(step, after_animation_ref) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::move_to_step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        step,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(DevFixtureExecutionError::ActionLog)?;

                return Err(DevFixtureExecutionError::Execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::move_to_step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    step,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            Ok(TimelineStepExecutionReport::completed(0))
        }
        TimelineStep::MoveByPath(step) => {
            let after_action_resolution =
                resolve_move_by_path_after_action(scope.registry, scope.resolve_context, step)
                    .map_err(DevFixtureExecutionError::Execution)?;
            let after_animation_ref = after_action_resolution
                .as_ref()
                .map(|resolution| resolution.animation_ref.as_str());
            scope
                .sink
                .append_event(&ActionLogEvent::move_by_path_step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    step,
                    after_action_resolution.as_ref(),
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            if let Err(error) = executor.move_by_path_step(step, after_animation_ref) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::move_by_path_step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        step,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(DevFixtureExecutionError::ActionLog)?;

                return Err(DevFixtureExecutionError::Execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::move_by_path_step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    step,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            Ok(TimelineStepExecutionReport::completed(0))
        }
        TimelineStep::Wait(step) => {
            validate_wait_step(step).map_err(DevFixtureExecutionError::Execution)?;
            scope
                .sink
                .append_event(&ActionLogEvent::wait_step_resolved(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_resolved_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    step,
                    scope.resolve_context,
                    scope.context.resolved_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            if let Err(error) = executor.wait_step(step) {
                let error_message = error.to_string();
                scope
                    .sink
                    .append_event(&ActionLogEvent::wait_step_failed(
                        ActionLogEventIds {
                            event_id: &indexed_event_id(
                                &scope.context.step_failed_event_id,
                                step_index,
                            ),
                            plan_id: &scope.plan.plan_id,
                            step_id: Some(&step.step_id),
                        },
                        &scope.plan.source_ref,
                        step,
                        error_message.as_str(),
                        scope.context.completed_at.as_str(),
                    ))
                    .map_err(DevFixtureExecutionError::ActionLog)?;

                return Err(DevFixtureExecutionError::Execution(error));
            }

            scope
                .sink
                .append_event(&ActionLogEvent::wait_step_completed(
                    ActionLogEventIds {
                        event_id: &indexed_event_id(
                            &scope.context.step_completed_event_id,
                            step_index,
                        ),
                        plan_id: &scope.plan.plan_id,
                        step_id: Some(&step.step_id),
                    },
                    &scope.plan.source_ref,
                    step,
                    step.duration_ms,
                    scope.context.completed_at.as_str(),
                ))
                .map_err(DevFixtureExecutionError::ActionLog)?;

            Ok(TimelineStepExecutionReport::completed(step.duration_ms))
        }
        TimelineStep::Recover(step) => {
            execute_dev_fixture_recover_step(executor, scope, step, step_index)
        }
        TimelineStep::Repeat(_)
        | TimelineStep::Choose(_)
        | TimelineStep::SetFallback(_)
        | TimelineStep::Skip(_)
        | TimelineStep::Retry(_)
        | TimelineStep::Replace(_)
        | TimelineStep::Try(_)
        | TimelineStep::SnapshotPosition(_)
        | TimelineStep::RestorePosition(_) => Err(DevFixtureExecutionError::Execution(
            planner_side_timeline_step_error(step),
        )),
    }
}

struct DevFixtureTimelineBranchExecutionError {
    error: Box<DevFixtureExecutionError>,
    report: TimelineStepExecutionReport,
}

fn execute_dev_fixture_timeline_branch_steps(
    executor: &impl ChoreographyStepExecutor,
    scope: &DevFixtureStepExecutionScope<'_>,
    steps: &[TimelineStep],
    parent_step_index: usize,
    branch_index: usize,
) -> Result<TimelineStepExecutionReport, DevFixtureTimelineBranchExecutionError> {
    let mut report = TimelineStepExecutionReport::default();
    for (nested_index, step) in steps.iter().enumerate() {
        let nested_step_index =
            nested_timeline_step_event_index(parent_step_index, branch_index, nested_index);
        match execute_dev_fixture_timeline_step(executor, scope, step, nested_step_index) {
            Ok(step_report) => report.add(step_report),
            Err(error) => {
                report.add(TimelineStepExecutionReport::failed());
                return Err(DevFixtureTimelineBranchExecutionError {
                    error: Box::new(error),
                    report,
                });
            }
        }
    }

    Ok(report)
}

fn execute_dev_fixture_recover_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &DevFixtureStepExecutionScope<'_>,
    step: &RecoverStep,
    step_index: usize,
) -> Result<TimelineStepExecutionReport, DevFixtureExecutionError> {
    let primary_result =
        execute_dev_fixture_timeline_branch_steps(executor, scope, &step.steps, step_index, 0);
    let primary_failure = match primary_result {
        Ok(report) => return Ok(report),
        Err(error) => error,
    };

    let mut report = primary_failure.report;
    match execute_dev_fixture_timeline_branch_steps(
        executor,
        scope,
        &step.recovery_steps,
        step_index,
        1,
    ) {
        Ok(recovery_report) => {
            report.add(recovery_report);
            Ok(report)
        }
        Err(recovery_failure) => Err(*recovery_failure.error),
    }
}

fn execute_runtime_safe_fallback_timeline_step(
    executor: &impl ChoreographyStepExecutor,
    scope: &RuntimeSafeFallbackStepExecutionScope<'_>,
    step: &TimelineStep,
    step_index: usize,
) -> Result<u64, RuntimeSafeFallbackExecutionError> {
    let TimelineStep::MoveTo(step) = step else {
        return Err(RuntimeSafeFallbackExecutionError::Execution(
            BuddyError::Validation("runtime safe fallback only supports moveTo steps".to_owned()),
        ));
    };

    let after_action_resolution =
        resolve_move_to_after_action(scope.registry, scope.resolve_context, step)
            .map_err(RuntimeSafeFallbackExecutionError::Execution)?;
    let after_animation_ref = after_action_resolution
        .as_ref()
        .map(|resolution| resolution.animation_ref.as_str());
    scope
        .sink
        .append_event(&ActionLogEvent::system_recovery_move_to_step_resolved(
            ActionLogEventIds {
                event_id: &indexed_event_id(&scope.context.step_resolved_event_id, step_index),
                plan_id: &scope.plan.plan_id,
                step_id: Some(&step.step_id),
            },
            &scope.plan.source_ref,
            step,
            after_action_resolution.as_ref(),
            scope.resolve_context,
            scope.context.resolved_at.as_str(),
        ))
        .map_err(RuntimeSafeFallbackExecutionError::ActionLog)?;

    if let Err(error) = executor.move_to_step(step, after_animation_ref) {
        let error_message = error.to_string();
        scope
            .sink
            .append_event(&ActionLogEvent::system_recovery_move_to_step_failed(
                ActionLogEventIds {
                    event_id: &indexed_event_id(&scope.context.step_failed_event_id, step_index),
                    plan_id: &scope.plan.plan_id,
                    step_id: Some(&step.step_id),
                },
                &scope.plan.source_ref,
                step,
                error_message.as_str(),
                scope.context.completed_at.as_str(),
            ))
            .map_err(RuntimeSafeFallbackExecutionError::ActionLog)?;

        return Err(RuntimeSafeFallbackExecutionError::Execution(error));
    }

    scope
        .sink
        .append_event(&ActionLogEvent::system_recovery_move_to_step_completed(
            ActionLogEventIds {
                event_id: &indexed_event_id(&scope.context.step_completed_event_id, step_index),
                plan_id: &scope.plan.plan_id,
                step_id: Some(&step.step_id),
            },
            &scope.plan.source_ref,
            step,
            scope.context.completed_at.as_str(),
        ))
        .map_err(RuntimeSafeFallbackExecutionError::ActionLog)?;

    Ok(0)
}

fn create_dev_fixture_plan(
    fixture_kind: DevFixtureKind,
    context: &DevFixtureExecutionContext,
) -> BuddyResult<DevFixturePlan> {
    match fixture_kind {
        DevFixtureKind::SinglePlayAction => Ok(create_single_play_action_dev_fixture_plan(
            context.plan_id.as_str(),
            context.step_id.as_str(),
            context.created_at.as_str(),
        )),
        DevFixtureKind::AiMacroDemo => create_ai_macro_demo_dev_fixture_plan(
            context.plan_id.as_str(),
            context.beat_id.as_str(),
            context.step_id.as_str(),
            context.created_at.as_str(),
        ),
    }
}

fn prefixed_uuid_v7(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::now_v7())
}

fn indexed_event_id(base_id: &str, zero_based_index: usize) -> String {
    if zero_based_index == 0 {
        return base_id.to_owned();
    }

    format!("{base_id}.{}", zero_based_index + 1)
}
