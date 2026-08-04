pub(crate) mod action_log;
pub(crate) mod admission;
pub(crate) mod affective;
mod command;
pub(crate) mod executor;
pub(crate) mod fixture;
pub(crate) mod macro_plan;
pub(crate) mod preset_behavior;
pub(crate) mod readiness;
pub(crate) mod recovery;
pub(crate) mod registry;
pub(crate) mod replay_policy;
pub(crate) mod sidecar_protocol;
mod step_resolution;
pub(crate) mod timeline;

pub use affective::run_affective_state_command_from_env;
pub use command::run_choreography_dev_fixture_command_from_env;
pub(crate) use command::{create_choreography_dev_fixture_admission_request, MacroIntentRunSource};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        action_log::{ActionLogEvent, ActionLogEventIds, ActionLogSink},
        admission::{
            ChoreographyAdmissionDecision, ChoreographyAdmissionRelease,
            ChoreographyAdmissionRequest, ChoreographyAdmissionState, ChoreographyPlanPriority,
            ChoreographyTriggerSource,
        },
        affective::{
            AffectiveContext, AffectiveContextSnapshot, AffectiveContextSource,
            AffectiveContextStore, ResolveContext,
        },
        executor::{
            execute_macro_intent_with_admission, execute_released_pending_timeline_plan,
            execute_runtime_safe_fallback_plan, execute_single_play_action_dev_fixture,
            execute_single_play_action_dev_fixture_with_admission,
            execute_timeline_plan_with_admission,
            execute_timeline_plan_with_admission_and_pending_queue, execute_timeline_step,
            execute_timeline_steps, ChoreographyStepExecutor, DevFixtureExecutionContext,
            DevFixtureExecutionError, MacroIntentExecutionContext, MacroIntentExecutionRequest,
            PendingTimelineExecutionQueue, RuntimeSafeFallbackExecutionContext,
            TimelineAdmissionExecutionRequest, TimelineExecutionContext, TimelineExecutionError,
        },
        fixture::{
            create_ai_macro_demo_dev_fixture_plan, create_single_play_action_dev_fixture_plan,
        },
        macro_plan::{
            compile_macro_intent_to_beat_plan, BeatPlanBuildContext, DanceMacroParams, MacroIntent,
        },
        preset_behavior::{
            append_native_pet_preset_behavior_action_log, NativePetPresetBehaviorLogContext,
        },
        recovery::{
            create_runtime_safe_fallback_plan, RuntimeSafeFallbackPlanContext,
            RuntimeSafeFallbackPosture, RuntimeSafeFallbackReason,
        },
        registry::{ActionRegistry, StepResolution},
        timeline::{
            expand_planner_timeline_steps, ChooseOption, ChooseStep, MoveByPathStep, MoveEdge,
            MoveTarget, MoveToStep, PlayActionStep, RecoverStep, RepeatStep, ReplaceStep,
            RestorePositionStep, RetryStep, SetFallbackStep, SkipStep, SnapshotPositionStep,
            TimelineFailurePolicy, TimelinePlan, TimelineSkipReason, TimelineStep, TryStep,
            WaitStep, WindowAnchorReveal, WindowAnchorSelector, WindowAnchorSelectorKind,
        },
    };
    use crate::error::{BuddyError, BuddyResult};
    use crate::native_pet::step_protocol::SidecarInterruptPolicy;
    use crate::storage::{
        ActionLogPlanListRequest, ActionLogSystemEventQueryRequest, BuddyStorage,
    };
    use std::{cell::RefCell, fs};

    #[derive(Default)]
    struct FakeStepExecutor {
        played_animation_refs: RefCell<Vec<String>>,
        played_durations_ms: RefCell<Vec<u64>>,
        played_playback_kinds: RefCell<Vec<String>>,
        moved_edges: RefCell<Vec<MoveEdge>>,
        moved_target_labels: RefCell<Vec<String>>,
        executed_step_kinds: RefCell<Vec<String>>,
        waited_durations_ms: RefCell<Vec<u64>>,
        interrupted_steps: RefCell<Vec<(String, String)>>,
        state_position: RefCell<Option<(i32, i32)>>,
    }

    impl ChoreographyStepExecutor for FakeStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.played_animation_refs
                .borrow_mut()
                .push(resolution.animation_ref.clone());
            self.played_durations_ms
                .borrow_mut()
                .push(resolution.duration_ms);
            self.played_playback_kinds
                .borrow_mut()
                .push(resolution.playback_kind.clone());
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            if let MoveTarget::Edge { edge } = &step.target {
                self.moved_edges.borrow_mut().push(*edge);
            }
            let target_label = move_target_label(&step.target);
            self.moved_target_labels
                .borrow_mut()
                .push(target_label.clone());
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            Ok(())
        }

        fn move_by_path_step(
            &self,
            step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            self.moved_target_labels
                .borrow_mut()
                .push(format!("path:{}", step.path.len()));
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveByPath:path:{}", step.path.len()));

            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.waited_durations_ms.borrow_mut().push(step.duration_ms);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, step_id: &str, reason_code: &str) -> BuddyResult<()> {
            self.interrupted_steps
                .borrow_mut()
                .push((step_id.to_owned(), reason_code.to_owned()));

            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(*self.state_position.borrow())
        }
    }

    fn move_target_label(target: &MoveTarget) -> String {
        match target {
            MoveTarget::Edge { edge } => format!("{edge:?}"),
            MoveTarget::EdgeAnchor {
                edge,
                reveal,
                duration_ms,
            } => format!(
                "edgeAnchor:{edge:?}:{}:{duration_ms}",
                window_anchor_reveal_label(*reveal)
            ),
            MoveTarget::Center => "center".to_owned(),
            MoveTarget::Home => "home".to_owned(),
            MoveTarget::Position { x, y } => format!("position:{x},{y}"),
            MoveTarget::X { x } => format!("x:{x}"),
            MoveTarget::WindowAnchor {
                selector,
                edge,
                reveal,
                ..
            } => format!(
                "windowAnchor:{}:{edge:?}:{}",
                window_anchor_selector_label(*selector),
                window_anchor_reveal_label(*reveal)
            ),
        }
    }

    fn window_anchor_selector_label(selector: WindowAnchorSelector) -> &'static str {
        match selector.kind {
            WindowAnchorSelectorKind::ActiveWindow => "activeWindow",
        }
    }

    fn window_anchor_reveal_label(reveal: WindowAnchorReveal) -> &'static str {
        match reveal {
            WindowAnchorReveal::Head => "head",
        }
    }

    struct FailingStepExecutor;

    impl ChoreographyStepExecutor for FailingStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn move_to_step(
            &self,
            _step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn wait_step(&self, _step: &WaitStep) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct PlayActionFailureRecoveryStepExecutor {
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for PlayActionFailureRecoveryStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            Ok(())
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    struct RetryPlayActionStepExecutor {
        remaining_play_action_failures: RefCell<u8>,
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl RetryPlayActionStepExecutor {
        fn fail_first_attempt() -> Self {
            Self {
                remaining_play_action_failures: RefCell::new(1),
                executed_step_kinds: RefCell::new(Vec::new()),
            }
        }
    }

    impl ChoreographyStepExecutor for RetryPlayActionStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            let mut remaining_failures = self.remaining_play_action_failures.borrow_mut();
            if *remaining_failures > 0 {
                *remaining_failures -= 1;
                return Err(BuddyError::Runtime(
                    "native pet control socket rejected host action: transient_motion_error"
                        .to_owned(),
                ));
            }

            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            Ok(())
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct ValidationPlayActionFailureRecoveryStepExecutor {
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for ValidationPlayActionFailureRecoveryStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            Err(BuddyError::Validation(
                "play action validation failed".to_owned(),
            ))
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            Ok(())
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct WindowAnchorFailureRecoveryStepExecutor {
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for WindowAnchorFailureRecoveryStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            if matches!(step.target, MoveTarget::WindowAnchor { .. }) {
                return Err(BuddyError::Runtime(
                    "native pet step failed: targetUnavailable: runtime failed: native pet active window rect is unavailable"
                        .to_owned(),
                ));
            }

            Ok(())
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct EdgeAnchorFailureRecoveryStepExecutor {
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for EdgeAnchorFailureRecoveryStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            if matches!(step.target, MoveTarget::EdgeAnchor { .. }) {
                return Err(BuddyError::Runtime(
                    "native pet step failed: motionTimeout: edge anchor target was not reached"
                        .to_owned(),
                ));
            }

            Ok(())
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct HomeMoveFailureRecoveryStepExecutor {
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for HomeMoveFailureRecoveryStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            if matches!(step.target, MoveTarget::Home) {
                return Err(BuddyError::Runtime(
                    "native pet step failed: motionTimeout: home target was not reached".to_owned(),
                ));
            }

            Ok(())
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct EdgeAnchorAndHomeFailureRecoveryStepExecutor {
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for EdgeAnchorAndHomeFailureRecoveryStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            match step.target {
                MoveTarget::EdgeAnchor { .. } => Err(BuddyError::Runtime(
                    "native pet step failed: motionTimeout: edge anchor target was not reached"
                        .to_owned(),
                )),
                MoveTarget::Home => Err(BuddyError::Runtime(
                    "native pet step failed: motionTimeout: home target was not reached".to_owned(),
                )),
                _ => Ok(()),
            }
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct EdgeAnchorHomeAndSleepFailureRecoveryStepExecutor {
        executed_step_kinds: RefCell<Vec<String>>,
    }

    impl ChoreographyStepExecutor for EdgeAnchorHomeAndSleepFailureRecoveryStepExecutor {
        fn play_action_step(
            &self,
            step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("playAction:{}", resolution.animation_ref));

            if step.step_id.ends_with(".recovery.fallback") {
                return Err(BuddyError::Runtime(
                    "native pet step failed: motionTimeout: sleep fallback did not settle"
                        .to_owned(),
                ));
            }

            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target_label = move_target_label(&step.target);
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("moveTo:{target_label}"));

            match step.target {
                MoveTarget::EdgeAnchor { .. } => Err(BuddyError::Runtime(
                    "native pet step failed: motionTimeout: edge anchor target was not reached"
                        .to_owned(),
                )),
                MoveTarget::Home if step.step_id.ends_with(".recovery.primary") => {
                    Err(BuddyError::Runtime(
                        "native pet step failed: motionTimeout: home target was not reached"
                            .to_owned(),
                    ))
                }
                _ => Ok(()),
            }
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Ok(())
        }

        fn wait_step(&self, step: &WaitStep) -> BuddyResult<()> {
            self.executed_step_kinds
                .borrow_mut()
                .push(format!("wait:{}", step.duration_ms));

            Ok(())
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    #[test]
    fn admission_priority_is_derived_from_runtime_trigger_source() {
        assert_eq!(
            ChoreographyTriggerSource::AiChoreography.priority(),
            ChoreographyPlanPriority::AiChoreography
        );
        assert_eq!(
            ChoreographyTriggerSource::CriticalInteraction.priority(),
            ChoreographyPlanPriority::CriticalInteraction
        );
        assert_eq!(
            ChoreographyTriggerSource::AttentionSystem.priority(),
            ChoreographyPlanPriority::AttentionSystem
        );
        assert_eq!(
            ChoreographyTriggerSource::SystemRecovery.priority(),
            ChoreographyPlanPriority::SystemRecovery
        );
        assert_eq!(
            serde_json::to_value(ChoreographyPlanPriority::UserRequested)
                .expect("serialize priority"),
            json!("userRequested")
        );
        assert_eq!(
            serde_json::to_value(ChoreographyTriggerSource::SystemRecovery)
                .expect("serialize trigger source"),
            json!("systemRecovery")
        );
    }

    #[test]
    fn admission_rejects_equal_priority_plan_while_active_plan_is_busy() {
        let mut admission = ChoreographyAdmissionState::default();
        let accepted = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_active_ai",
            ChoreographyTriggerSource::AiChoreography,
        ));
        let rejected = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_next_ai",
            ChoreographyTriggerSource::AiChoreography,
        ));

        assert!(matches!(
            accepted,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert_eq!(
            rejected,
            ChoreographyAdmissionDecision::Rejected {
                plan_id: "plan_next_ai".to_owned(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
                priority: ChoreographyPlanPriority::AiChoreography,
                active_plan_id: "plan_active_ai".to_owned(),
                active_priority: ChoreographyPlanPriority::AiChoreography,
                reason_code: "executor.busy".to_owned(),
            }
        );
        assert_eq!(admission.active_plan_id(), Some("plan_active_ai"));
        assert_eq!(
            rejected.action_log_payload(),
            json!({
                "decision": "rejected",
                "planId": "plan_next_ai",
                "triggerSource": "aiChoreography",
                "priority": "aiChoreography",
                "reasonCode": "executor.busy",
                "activePlan": {
                    "planId": "plan_active_ai",
                    "priority": "aiChoreography"
                }
            })
        );
    }

    #[test]
    fn admission_skips_lower_priority_plan_while_higher_priority_plan_is_active() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user",
            ChoreographyTriggerSource::UserRequested,
        ));

        let skipped = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_idle",
            ChoreographyTriggerSource::IdleAutonomous,
        ));

        assert_eq!(
            skipped,
            ChoreographyAdmissionDecision::Skipped {
                plan_id: "plan_idle".to_owned(),
                trigger_source: ChoreographyTriggerSource::IdleAutonomous,
                priority: ChoreographyPlanPriority::IdleAutonomous,
                active_plan_id: "plan_user".to_owned(),
                active_priority: ChoreographyPlanPriority::UserRequested,
                reason_code: "priority.tooLow".to_owned(),
            }
        );
        assert_eq!(admission.active_plan_id(), Some("plan_user"));
    }

    #[test]
    fn admission_preempts_lower_priority_active_plan() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new(
                "plan_idle",
                ChoreographyTriggerSource::IdleAutonomous,
            )
            .with_active_step("step_idle_active", SidecarInterruptPolicy::Interruptible),
        );

        let preempted = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user",
            ChoreographyTriggerSource::UserRequested,
        ));

        assert_eq!(
            preempted,
            ChoreographyAdmissionDecision::Preempted {
                plan_id: "plan_user".to_owned(),
                trigger_source: ChoreographyTriggerSource::UserRequested,
                priority: ChoreographyPlanPriority::UserRequested,
                interrupted_plan_id: "plan_idle".to_owned(),
                interrupted_step_id: Some("step_idle_active".to_owned()),
                interrupted_priority: ChoreographyPlanPriority::IdleAutonomous,
                reason_code: "admission.preemptedByHigherPriorityPlan".to_owned(),
            }
        );
        assert_eq!(admission.active_plan_id(), Some("plan_user"));
    }

    #[test]
    fn admission_defers_higher_priority_plan_when_active_step_must_finish() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new("plan_ai", ChoreographyTriggerSource::AiChoreography)
                .with_active_step("step_ai_finish", SidecarInterruptPolicy::FinishStep),
        );

        let deferred = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user",
            ChoreographyTriggerSource::UserRequested,
        ));

        assert_eq!(
            deferred,
            ChoreographyAdmissionDecision::Deferred {
                plan_id: "plan_user".to_owned(),
                trigger_source: ChoreographyTriggerSource::UserRequested,
                priority: ChoreographyPlanPriority::UserRequested,
                active_plan_id: "plan_ai".to_owned(),
                active_step_id: Some("step_ai_finish".to_owned()),
                active_priority: ChoreographyPlanPriority::AiChoreography,
                active_step_interrupt_policy: SidecarInterruptPolicy::FinishStep,
                reason_code: "admission.waitingForActiveStepToFinish".to_owned(),
            }
        );
        assert_eq!(admission.active_plan_id(), Some("plan_ai"));
        assert_eq!(
            deferred.action_log_payload(),
            json!({
                "decision": "deferred",
                "planId": "plan_user",
                "triggerSource": "userRequested",
                "priority": "userRequested",
                "reasonCode": "admission.waitingForActiveStepToFinish",
                "activePlan": {
                    "planId": "plan_ai",
                    "stepId": "step_ai_finish",
                    "priority": "aiChoreography",
                    "interruptPolicy": "finishStep"
                }
            })
        );
    }

    #[test]
    fn admission_release_returns_pending_promotion_after_deferred_finish_step_preemption() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new("plan_ai", ChoreographyTriggerSource::AiChoreography)
                .with_active_step("step_ai_finish", SidecarInterruptPolicy::FinishStep),
        );
        admission.admit(
            ChoreographyAdmissionRequest::new(
                "plan_user",
                ChoreographyTriggerSource::UserRequested,
            )
            .with_active_step("step_user_first", SidecarInterruptPolicy::Interruptible),
        );

        let release = admission.release_plan("plan_ai");

        assert_eq!(
            release,
            ChoreographyAdmissionRelease::ReleasedWithPending {
                plan_id: "plan_ai".to_owned(),
                pending_plan_id: "plan_user".to_owned(),
                pending_trigger_source: ChoreographyTriggerSource::UserRequested,
                pending_priority: ChoreographyPlanPriority::UserRequested,
                pending_active_step_id: Some("step_user_first".to_owned()),
                pending_active_step_interrupt_policy: Some(SidecarInterruptPolicy::Interruptible),
            }
        );
        assert_eq!(admission.active_plan_id(), None);
    }

    #[test]
    fn admission_pending_promotion_uses_priority_then_fifo_order() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new("plan_ai", ChoreographyTriggerSource::AiChoreography)
                .with_active_step("step_ai_finish", SidecarInterruptPolicy::FinishStep),
        );
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user_first",
            ChoreographyTriggerSource::UserRequested,
        ));
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_attention",
            ChoreographyTriggerSource::AttentionSystem,
        ));
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user_second",
            ChoreographyTriggerSource::UserRequested,
        ));

        let release = admission.release_plan("plan_ai");

        assert!(matches!(
            release,
            ChoreographyAdmissionRelease::ReleasedWithPending {
                pending_plan_id,
                pending_priority: ChoreographyPlanPriority::AttentionSystem,
                ..
            } if pending_plan_id == "plan_attention"
        ));
    }

    #[test]
    fn admission_active_step_refresh_changes_preempt_interrupt_target() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new("plan_ai", ChoreographyTriggerSource::AiChoreography)
                .with_active_step("step_first", SidecarInterruptPolicy::Interruptible),
        );

        admission.update_active_step("plan_ai", "step_second");
        let preempted = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user",
            ChoreographyTriggerSource::UserRequested,
        ));

        assert!(matches!(
            preempted,
            ChoreographyAdmissionDecision::Preempted {
                interrupted_step_id: Some(step_id),
                ..
            } if step_id == "step_second"
        ));
    }

    #[test]
    fn admission_active_step_refresh_changes_interrupt_policy_for_preemption() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new("plan_ai", ChoreographyTriggerSource::AiChoreography)
                .with_active_step("step_first", SidecarInterruptPolicy::Interruptible),
        );

        admission.update_active_step_with_policy(
            "plan_ai",
            "step_finish",
            SidecarInterruptPolicy::FinishStep,
        );
        let deferred = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user",
            ChoreographyTriggerSource::UserRequested,
        ));

        assert!(matches!(
            deferred,
            ChoreographyAdmissionDecision::Deferred {
                active_step_id: Some(step_id),
                active_step_interrupt_policy: SidecarInterruptPolicy::FinishStep,
                ..
            } if step_id == "step_finish"
        ));
        assert_eq!(admission.active_plan_id(), Some("plan_ai"));
    }

    #[test]
    fn admission_attention_system_preempts_active_system_recovery() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new(
                "plan_recovery",
                ChoreographyTriggerSource::SystemRecovery,
            )
            .with_active_step(
                "step_recovery_active",
                SidecarInterruptPolicy::Interruptible,
            ),
        );

        let preempted = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_attention",
            ChoreographyTriggerSource::AttentionSystem,
        ));

        assert_eq!(
            preempted,
            ChoreographyAdmissionDecision::Preempted {
                plan_id: "plan_attention".to_owned(),
                trigger_source: ChoreographyTriggerSource::AttentionSystem,
                priority: ChoreographyPlanPriority::AttentionSystem,
                interrupted_plan_id: "plan_recovery".to_owned(),
                interrupted_step_id: Some("step_recovery_active".to_owned()),
                interrupted_priority: ChoreographyPlanPriority::SystemRecovery,
                reason_code: "admission.preemptedByHigherPriorityPlan".to_owned(),
            }
        );
        assert_eq!(admission.active_plan_id(), Some("plan_attention"));
    }

    #[test]
    fn admission_releases_completed_active_plan_before_next_equal_priority_plan() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_active_ai",
            ChoreographyTriggerSource::AiChoreography,
        ));

        let release = admission.release_plan("plan_active_ai");
        let next = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_next_ai",
            ChoreographyTriggerSource::AiChoreography,
        ));

        assert_eq!(
            release,
            ChoreographyAdmissionRelease::Released {
                plan_id: "plan_active_ai".to_owned(),
            }
        );
        assert!(matches!(
            next,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert_eq!(admission.active_plan_id(), Some("plan_next_ai"));
    }

    #[test]
    fn admission_ignores_stale_release_from_preempted_plan() {
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_idle",
            ChoreographyTriggerSource::IdleAutonomous,
        ));
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_user",
            ChoreographyTriggerSource::UserRequested,
        ));

        let release = admission.release_plan("plan_idle");

        assert_eq!(
            release,
            ChoreographyAdmissionRelease::Stale {
                plan_id: "plan_idle".to_owned(),
                active_plan_id: "plan_user".to_owned(),
            }
        );
        assert_eq!(admission.active_plan_id(), Some("plan_user"));
    }

    #[test]
    fn admission_reports_no_active_plan_when_release_is_replayed() {
        let mut admission = ChoreographyAdmissionState::default();

        let release = admission.release_plan("plan_already_finished");

        assert_eq!(
            release,
            ChoreographyAdmissionRelease::NoActivePlan {
                plan_id: "plan_already_finished".to_owned(),
            }
        );
        assert_eq!(admission.active_plan_id(), None);
    }

    #[test]
    fn runtime_safe_fallback_plan_uses_home_sleep_system_recovery_posture() {
        let plan = create_runtime_safe_fallback_plan(RuntimeSafeFallbackPlanContext {
            plan_id: "plan_recovery_001",
            step_id: "step_recovery_001",
            triggered_by_plan_id: "plan_failed_001",
            triggered_by_step_id: Some("step_failed_001"),
            trigger_reason: RuntimeSafeFallbackReason::MotionTimeout,
            created_at: "2026-07-09T00:00:00.000Z",
        });

        assert_eq!(plan.posture, RuntimeSafeFallbackPosture::HomeSleep);
        assert_eq!(
            plan.source_ref,
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_failed_001",
                "triggeredByStepId": "step_failed_001",
                "triggerReason": "sidecar.motionTimeout"
            })
        );
        assert_eq!(
            plan.steps,
            vec![TimelineStep::MoveTo(
                MoveToStep::home_with_after_action_fallback(
                    "step_recovery_001",
                    "sleep",
                    "idle",
                    15_000,
                )
            )]
        );
    }

    #[test]
    fn runtime_safe_fallback_executor_logs_system_recovery_home_sleep_plan() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let context = RuntimeSafeFallbackExecutionContext::fixed_for_test();
        let plan = create_runtime_safe_fallback_plan(RuntimeSafeFallbackPlanContext {
            plan_id: &context.plan_id,
            step_id: &context.step_id,
            triggered_by_plan_id: "plan_019f4000-0000-7000-8000-000000000001",
            triggered_by_step_id: Some("step_019f4000-0000-7000-8000-000000000002"),
            trigger_reason: RuntimeSafeFallbackReason::MotionTimeout,
            created_at: context.created_at.as_str(),
        });

        let report = execute_runtime_safe_fallback_plan(
            storage.clone(),
            &executor,
            plan,
            context,
            ResolveContext::default(),
        )
        .expect("execute runtime safe fallback plan");

        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["moveTo:home".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            storage.action_log_plan_summary_for_test(&report.plan_id),
            json!({
                "status": "completed",
                "lastEventType": "plan.completed",
                "lastReasonCode": "systemRecovery.completed",
                "resolvedActionId": "sleep",
                "resolvedAnimationRef": "sleep"
            })
        );
        let lines = storage.read_action_log_jsonl_lines_for_test();
        let events = lines
            .iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("parse event"))
            .collect::<Vec<_>>();
        assert_eq!(events[0]["triggerSource"], "systemRecovery");
        assert_eq!(
            events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_019f4000-0000-7000-8000-000000000001",
                "triggeredByStepId": "step_019f4000-0000-7000-8000-000000000002",
                "triggerReason": "sidecar.motionTimeout"
            })
        );
        assert_eq!(events[3]["payload"]["resultKind"], "fallback");
    }

    #[test]
    fn executor_admission_rejected_event_projects_terminal_action_log_status() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let sink = ActionLogSink::new(storage.clone());
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_active_ai",
            ChoreographyTriggerSource::AiChoreography,
        ));
        let decision = admission.admit(ChoreographyAdmissionRequest::new(
            "plan_rejected_ai",
            ChoreographyTriggerSource::AiChoreography,
        ));
        let plan = create_single_play_action_dev_fixture_plan(
            "plan_rejected_ai",
            "step_rejected_ai",
            "2026-07-09T00:00:00.000Z",
        );
        let event = ActionLogEvent::executor_admission_decision(
            "evt_019f5000-0000-7000-8000-000000000001",
            &plan,
            &decision,
            "2026-07-09T00:00:00.010Z",
        );

        assert_eq!(
            serde_json::to_value(&event).expect("serialize executor rejected event"),
            json!({
                "eventId": "evt_019f5000-0000-7000-8000-000000000001",
                "schemaVersion": 1,
                "eventType": "executor.rejected",
                "status": "rejected",
                "reasonCode": "executor.busy",
                "planId": "plan_rejected_ai",
                "stepId": null,
                "sourceRef": {
                    "kind": "devFixture",
                    "fixtureName": "single-play-action"
                },
                "triggerSource": "devFixture",
                "payload": {
                    "decision": "rejected",
                    "planId": "plan_rejected_ai",
                    "triggerSource": "aiChoreography",
                    "priority": "aiChoreography",
                    "reasonCode": "executor.busy",
                    "activePlan": {
                        "planId": "plan_active_ai",
                        "priority": "aiChoreography"
                    }
                },
                "createdAt": "2026-07-09T00:00:00.010Z"
            })
        );

        sink.append_event(&event)
            .expect("append executor rejected event");
        let summary = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                plan_id: Some("plan_rejected_ai".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list rejected action log plan")
            .items
            .into_iter()
            .next()
            .expect("rejected plan summary");

        assert_eq!(summary.status, "rejected");
        assert_eq!(summary.detail_status, "rejected");
        assert_eq!(summary.last_event_type, "executor.rejected");
        assert_eq!(summary.last_reason_code, "executor.busy");
        assert_eq!(
            summary.completed_at.as_deref(),
            Some("2026-07-09T00:00:00.010Z")
        );
    }

    #[test]
    fn macro_intent_dance_rejects_invalid_duration() {
        let error = compile_macro_intent_to_beat_plan(
            &MacroIntent::Dance(DanceMacroParams { duration_ms: 0 }),
            BeatPlanBuildContext {
                plan_id: "plan_019f4300-0000-7000-8000-000000000021",
                beat_id: "beat_019f4300-0000-7000-8000-000000000022",
                step_id: "step_019f4300-0000-7000-8000-000000000023",
                source_ref: json!({
                    "kind": "devFixture",
                    "fixtureName": "ai-macro-demo"
                }),
                created_at: "2026-07-09T00:00:00.000Z",
            },
        )
        .expect_err("invalid dance duration should be rejected");

        assert!(error.to_string().contains("dance durationMs"));
    }

    #[test]
    fn macro_intent_peek_behind_window_rejects_unknown_selector_fields() {
        let error = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekBehindWindow",
            "params": {
                "windowSelector": {
                    "kind": "activeWindow",
                    "title": "do not smuggle free-form window matching yet"
                },
                "edge": "left",
                "reveal": "head",
                "durationMs": 3000
            }
        }))
        .expect_err("peekBehindWindow should keep selector schema fixed");

        assert!(
            error.to_string().contains("unknown field"),
            "unexpected serde error: {error}"
        );
    }

    #[test]
    fn macro_intent_rejects_unknown_macro_id() {
        let error = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "unknown",
            "params": {}
        }))
        .expect_err("unknown macro should be rejected");

        assert!(error.to_string().contains("unknown variant"));
    }

    #[test]
    fn macro_intent_patrol_around_screen_rejects_zero_loops() {
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "patrolAroundScreen",
            "params": { "loops": 0 }
        }))
        .expect("parse patrol macro intent");

        let error = compile_macro_intent_to_beat_plan(
            &intent,
            BeatPlanBuildContext {
                plan_id: "plan_019f4400-0000-7000-8000-000000000101",
                beat_id: "beat_019f4400-0000-7000-8000-000000000102",
                step_id: "step_019f4400-0000-7000-8000-000000000103",
                source_ref: json!({
                    "kind": "devFixture",
                    "fixtureName": "ai-macro-demo"
                }),
                created_at: "2026-07-09T01:00:00.000Z",
            },
        )
        .expect_err("zero patrol loops should be rejected");

        assert!(error.to_string().contains("patrolAroundScreen loops"));
    }

    #[test]
    fn timeline_executor_dispatches_move_to_edge_step() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = TimelineStep::MoveTo(MoveToStep::edge(
            "step_019f4500-0000-7000-8000-000000000001",
            MoveEdge::Left,
            15_000,
        ));

        execute_timeline_step(&executor, &registry, &ResolveContext::default(), &step)
            .expect("execute moveTo edge step");

        assert_eq!(
            executor.played_animation_refs.borrow().as_slice(),
            &[] as &[String]
        );
        assert_eq!(executor.moved_edges.borrow().as_slice(), &[MoveEdge::Left]);
        assert_eq!(
            executor.moved_target_labels.borrow().as_slice(),
            &["Left".to_owned()]
        );
    }

    #[test]
    fn timeline_executor_dispatches_move_to_center_step() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = TimelineStep::MoveTo(MoveToStep::center(
            "step_019f4500-0000-7000-8000-000000000011",
            15_000,
        ));

        execute_timeline_step(&executor, &registry, &ResolveContext::default(), &step)
            .expect("execute moveTo center step");

        assert_eq!(
            executor.moved_target_labels.borrow().as_slice(),
            &["center".to_owned()]
        );
    }

    #[test]
    fn timeline_executor_dispatches_move_by_path_step() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = TimelineStep::MoveByPath(MoveByPathStep::new(
            "step_019f4500-0000-7000-8000-000000000012",
            vec![
                MoveTarget::Edge {
                    edge: MoveEdge::Left,
                },
                MoveTarget::Center,
            ],
            30_000,
        ));

        execute_timeline_step(&executor, &registry, &ResolveContext::default(), &step)
            .expect("execute moveByPath step");

        assert_eq!(
            executor.executed_step_kinds.borrow().as_slice(),
            &["moveByPath:path:2".to_owned()]
        );
    }

    #[test]
    fn timeline_wait_step_serializes_as_host_side_primitive() {
        let step = TimelineStep::Wait(WaitStep::new(
            "step_019f4500-0000-7000-8000-000000000013",
            750,
            1_000,
        ));

        assert_eq!(
            serde_json::to_value(step).expect("serialize wait step"),
            json!({
                "stepId": "step_019f4500-0000-7000-8000-000000000013",
                "kind": "wait",
                "durationMs": 750,
                "timeoutMs": 1000
            })
        );
    }

    #[test]
    fn timeline_structural_primitives_serialize_as_planner_only_schema() {
        let steps = vec![
            TimelineStep::Repeat(RepeatStep {
                step_id: "step_019f4500-0000-7000-8000-000000000031".to_owned(),
                kind: "repeat".to_owned(),
                times: 2,
                steps: vec![TimelineStep::Wait(WaitStep::new(
                    "step_019f4500-0000-7000-8000-000000000032",
                    250,
                    1_000,
                ))],
            }),
            TimelineStep::Choose(ChooseStep {
                step_id: "step_019f4500-0000-7000-8000-000000000033".to_owned(),
                kind: "choose".to_owned(),
                strategy: "weighted".to_owned(),
                options: vec![ChooseOption {
                    option_id: "stumble".to_owned(),
                    weight: 70,
                    steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                        "step_019f4500-0000-7000-8000-000000000034",
                        "reassure",
                        5_000,
                    ))],
                }],
            }),
            TimelineStep::SetFallback(SetFallbackStep {
                step_id: "step_019f4500-0000-7000-8000-000000000035".to_owned(),
                kind: "setFallback".to_owned(),
                fallback_action_id: "idle".to_owned(),
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f4500-0000-7000-8000-000000000036",
                    "cast",
                    5_000,
                ))],
            }),
            TimelineStep::Retry(RetryStep {
                step_id: "step_019f4500-0000-7000-8000-000000000039".to_owned(),
                kind: "retry".to_owned(),
                max_attempts: 2,
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f4500-0000-7000-8000-000000000040",
                    "celebrate",
                    5_000,
                ))],
            }),
            TimelineStep::Replace(ReplaceStep {
                step_id: "step_019f4500-0000-7000-8000-000000000041".to_owned(),
                kind: "replaceStep".to_owned(),
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f4500-0000-7000-8000-000000000042",
                    "cast",
                    5_000,
                ))],
                replacement_steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f4500-0000-7000-8000-000000000043",
                    "celebrate",
                    5_000,
                ))],
            }),
            TimelineStep::Recover(RecoverStep {
                step_id: "step_019f4500-0000-7000-8000-000000000044".to_owned(),
                kind: "recover".to_owned(),
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f4500-0000-7000-8000-000000000045",
                    "cast",
                    5_000,
                ))],
                recovery_steps: vec![TimelineStep::MoveTo(MoveToStep::home(
                    "step_019f4500-0000-7000-8000-000000000046",
                    15_000,
                ))],
            }),
            TimelineStep::SnapshotPosition(SnapshotPositionStep {
                step_id: "step_019f4500-0000-7000-8000-000000000037".to_owned(),
                kind: "snapshotPosition".to_owned(),
                snapshot_id: "origin".to_owned(),
            }),
            TimelineStep::RestorePosition(RestorePositionStep {
                step_id: "step_019f4500-0000-7000-8000-000000000038".to_owned(),
                kind: "restorePosition".to_owned(),
                snapshot_id: "origin".to_owned(),
                after_action_id: Some("sleep".to_owned()),
                fallback_after_action_id: Some("idle".to_owned()),
                timeout_ms: 15_000,
            }),
        ];

        assert_eq!(
            serde_json::to_value(steps).expect("serialize structural timeline primitives"),
            json!([
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000031",
                    "kind": "repeat",
                    "times": 2,
                    "steps": [
                        {
                            "stepId": "step_019f4500-0000-7000-8000-000000000032",
                            "kind": "wait",
                            "durationMs": 250,
                            "timeoutMs": 1000
                        }
                    ]
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000033",
                    "kind": "choose",
                    "strategy": "weighted",
                    "options": [
                        {
                            "optionId": "stumble",
                            "weight": 70,
                            "steps": [
                                {
                                    "stepId": "step_019f4500-0000-7000-8000-000000000034",
                                    "kind": "playAction",
                                    "actionId": "reassure",
                                    "expectedPlayback": "once",
                                    "timeoutMs": 5000
                                }
                            ]
                        }
                    ]
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000035",
                    "kind": "setFallback",
                    "fallbackActionId": "idle",
                    "steps": [
                        {
                            "stepId": "step_019f4500-0000-7000-8000-000000000036",
                            "kind": "playAction",
                            "actionId": "cast",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ]
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000039",
                    "kind": "retry",
                    "maxAttempts": 2,
                    "steps": [
                        {
                            "stepId": "step_019f4500-0000-7000-8000-000000000040",
                            "kind": "playAction",
                            "actionId": "celebrate",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ]
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000041",
                    "kind": "replaceStep",
                    "steps": [
                        {
                            "stepId": "step_019f4500-0000-7000-8000-000000000042",
                            "kind": "playAction",
                            "actionId": "cast",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ],
                    "replacementSteps": [
                        {
                            "stepId": "step_019f4500-0000-7000-8000-000000000043",
                            "kind": "playAction",
                            "actionId": "celebrate",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ]
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000044",
                    "kind": "recover",
                    "steps": [
                        {
                            "stepId": "step_019f4500-0000-7000-8000-000000000045",
                            "kind": "playAction",
                            "actionId": "cast",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ],
                    "recoverySteps": [
                        {
                            "stepId": "step_019f4500-0000-7000-8000-000000000046",
                            "kind": "moveTo",
                            "target": { "kind": "home" },
                            "afterActionId": null,
                            "timeoutMs": 15000
                        }
                    ]
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000037",
                    "kind": "snapshotPosition",
                    "snapshotId": "origin"
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000038",
                    "kind": "restorePosition",
                    "snapshotId": "origin",
                    "afterActionId": "sleep",
                    "fallbackAfterActionId": "idle",
                    "timeoutMs": 15000
                }
            ])
        );
    }

    #[test]
    fn timeline_planner_expands_repeat_choose_and_set_fallback_deterministically() {
        let steps = vec![TimelineStep::SetFallback(SetFallbackStep {
            step_id: "step_019f5400-0000-7000-8000-000000000001".to_owned(),
            kind: "setFallback".to_owned(),
            fallback_action_id: "idle".to_owned(),
            steps: vec![TimelineStep::Choose(ChooseStep {
                step_id: "step_019f5400-0000-7000-8000-000000000002".to_owned(),
                kind: "choose".to_owned(),
                strategy: "weighted".to_owned(),
                options: vec![
                    ChooseOption {
                        option_id: "zulu".to_owned(),
                        weight: 70,
                        steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                            "step_019f5400-0000-7000-8000-000000000003",
                            "reassure",
                            5_000,
                        ))],
                    },
                    ChooseOption {
                        option_id: "alpha".to_owned(),
                        weight: 70,
                        steps: vec![TimelineStep::Repeat(RepeatStep {
                            step_id: "step_019f5400-0000-7000-8000-000000000004".to_owned(),
                            kind: "repeat".to_owned(),
                            times: 2,
                            steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                                "step_019f5400-0000-7000-8000-000000000005",
                                "cast",
                                5_000,
                            ))],
                        })],
                    },
                    ChooseOption {
                        option_id: "low".to_owned(),
                        weight: 10,
                        steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                            "step_019f5400-0000-7000-8000-000000000006",
                            "celebrate",
                            5_000,
                        ))],
                    },
                ],
            })],
        })];

        let expanded =
            expand_planner_timeline_steps(&steps).expect("expand planner-side timeline steps");

        assert_eq!(
            serde_json::to_value(expanded).expect("serialize expanded timeline steps"),
            json!([
                {
                    "stepId": "step_019f5400-0000-7000-8000-000000000005__repeat_001",
                    "kind": "playAction",
                    "actionId": "cast",
                    "fallbackActionId": "idle",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                },
                {
                    "stepId": "step_019f5400-0000-7000-8000-000000000005__repeat_002",
                    "kind": "playAction",
                    "actionId": "cast",
                    "fallbackActionId": "idle",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                }
            ])
        );
    }

    #[test]
    fn timeline_planner_rejects_choose_option_without_nested_steps() {
        let steps = vec![TimelineStep::Choose(ChooseStep {
            step_id: "step_019f5400-0000-7000-8000-000000000018".to_owned(),
            kind: "choose".to_owned(),
            strategy: "first".to_owned(),
            options: vec![ChooseOption {
                option_id: "empty".to_owned(),
                weight: 1,
                steps: Vec::new(),
            }],
        })];

        let error =
            expand_planner_timeline_steps(&steps).expect_err("choose option must not be empty");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: choose timeline option empty must contain at least one nested step: step_019f5400-0000-7000-8000-000000000018"
        );
    }

    #[test]
    fn timeline_planner_rejects_retry_with_single_attempt() {
        let steps = vec![TimelineStep::Retry(RetryStep {
            step_id: "step_019f5400-0000-7000-8000-000000000028".to_owned(),
            kind: "retry".to_owned(),
            max_attempts: 1,
            steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                "step_019f5400-0000-7000-8000-000000000029",
                "celebrate",
                5_000,
            ))],
        })];

        let error = expand_planner_timeline_steps(&steps)
            .expect_err("retry with one attempt should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: retry timeline step maxAttempts must be at least two: step_019f5400-0000-7000-8000-000000000028"
        );
    }

    #[test]
    fn timeline_planner_rejects_replace_step_without_replacement_steps() {
        let steps = vec![TimelineStep::Replace(ReplaceStep {
            step_id: "step_019f5400-0000-7000-8000-000000000030".to_owned(),
            kind: "replaceStep".to_owned(),
            steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                "step_019f5400-0000-7000-8000-000000000031",
                "celebrate",
                5_000,
            ))],
            replacement_steps: Vec::new(),
        })];

        let error = expand_planner_timeline_steps(&steps)
            .expect_err("replaceStep without replacement steps should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: replaceStep timeline step must contain at least one replacement step: step_019f5400-0000-7000-8000-000000000030"
        );
    }

    #[test]
    fn timeline_planner_rejects_recover_step_without_recovery_steps() {
        let steps = vec![TimelineStep::Recover(RecoverStep {
            step_id: "step_019f5400-0000-7000-8000-000000000032".to_owned(),
            kind: "recover".to_owned(),
            steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                "step_019f5400-0000-7000-8000-000000000033",
                "celebrate",
                5_000,
            ))],
            recovery_steps: Vec::new(),
        })];

        let error = expand_planner_timeline_steps(&steps)
            .expect_err("recover without recovery steps should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: recover timeline step must contain at least one recovery step: step_019f5400-0000-7000-8000-000000000032"
        );
    }

    #[test]
    fn timeline_planner_rejects_unused_set_fallback_on_wait_only_steps() {
        let steps = vec![TimelineStep::SetFallback(SetFallbackStep {
            step_id: "step_019f5400-0000-7000-8000-000000000011".to_owned(),
            kind: "setFallback".to_owned(),
            fallback_action_id: "idle".to_owned(),
            steps: vec![TimelineStep::Wait(WaitStep::new(
                "step_019f5400-0000-7000-8000-000000000012",
                250,
                1_000,
            ))],
        })];

        let error = expand_planner_timeline_steps(&steps)
            .expect_err("setFallback must apply to at least one playAction step");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: setFallback step step_019f5400-0000-7000-8000-000000000011 must apply to at least one playAction timeline step"
        );
    }

    #[test]
    fn timeline_planner_rejects_set_fallback_when_nested_play_actions_already_have_fallback() {
        let mut nested_action =
            PlayActionStep::once("step_019f5400-0000-7000-8000-000000000014", "cast", 5_000);
        nested_action.fallback_action_id = Some("celebrate".to_owned());
        let steps = vec![TimelineStep::SetFallback(SetFallbackStep {
            step_id: "step_019f5400-0000-7000-8000-000000000013".to_owned(),
            kind: "setFallback".to_owned(),
            fallback_action_id: "idle".to_owned(),
            steps: vec![TimelineStep::PlayAction(nested_action)],
        })];

        let error = expand_planner_timeline_steps(&steps)
            .expect_err("setFallback must be consumed by a playAction without explicit fallback");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: setFallback step step_019f5400-0000-7000-8000-000000000013 must apply to at least one playAction timeline step"
        );
    }

    #[test]
    fn timeline_planner_rejects_outer_set_fallback_when_only_inner_override_consumes_fallback() {
        let steps = vec![TimelineStep::SetFallback(SetFallbackStep {
            step_id: "step_019f5400-0000-7000-8000-000000000015".to_owned(),
            kind: "setFallback".to_owned(),
            fallback_action_id: "idle".to_owned(),
            steps: vec![TimelineStep::SetFallback(SetFallbackStep {
                step_id: "step_019f5400-0000-7000-8000-000000000016".to_owned(),
                kind: "setFallback".to_owned(),
                fallback_action_id: "celebrate".to_owned(),
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f5400-0000-7000-8000-000000000017",
                    "cast",
                    5_000,
                ))],
            })],
        })];

        let error = expand_planner_timeline_steps(&steps)
            .expect_err("outer setFallback must not count inner override as consumption");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: setFallback step step_019f5400-0000-7000-8000-000000000015 must apply to at least one playAction timeline step"
        );
    }

    #[test]
    fn timeline_executor_rejects_structural_primitives_before_dispatch() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = TimelineStep::Repeat(RepeatStep {
            step_id: "step_019f4500-0000-7000-8000-000000000039".to_owned(),
            kind: "repeat".to_owned(),
            times: 2,
            steps: vec![TimelineStep::Wait(WaitStep::new(
                "step_019f4500-0000-7000-8000-000000000040",
                250,
                1_000,
            ))],
        });

        let error = execute_timeline_step(&executor, &registry, &ResolveContext::default(), &step)
            .expect_err("structural step should not execute directly");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: repeat timeline step is planner-side and cannot execute directly: step_019f4500-0000-7000-8000-000000000039"
        );
        assert!(executor.executed_step_kinds.borrow().is_empty());
    }

    #[test]
    fn timeline_executor_expands_planner_steps_before_dispatch() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let steps = vec![TimelineStep::SetFallback(SetFallbackStep {
            step_id: "step_019f5400-0000-7000-8000-000000000011".to_owned(),
            kind: "setFallback".to_owned(),
            fallback_action_id: "idle".to_owned(),
            steps: vec![TimelineStep::Repeat(RepeatStep {
                step_id: "step_019f5400-0000-7000-8000-000000000012".to_owned(),
                kind: "repeat".to_owned(),
                times: 2,
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f5400-0000-7000-8000-000000000013",
                    "missing.action",
                    5_000,
                ))],
            })],
        })];

        let report =
            execute_timeline_steps(&executor, &registry, &ResolveContext::default(), &steps)
                .expect("execute expanded planner timeline");

        assert_eq!(report.completed_step_count, 2);
        assert_eq!(
            *executor.played_animation_refs.borrow(),
            vec!["idle".to_owned(), "idle".to_owned()]
        );
    }

    #[test]
    fn timeline_executor_dispatches_wait_step_without_sidecar_motion() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = TimelineStep::Wait(WaitStep::new(
            "step_019f4500-0000-7000-8000-000000000014",
            250,
            1_000,
        ));

        execute_timeline_step(&executor, &registry, &ResolveContext::default(), &step)
            .expect("execute wait step");

        assert_eq!(executor.waited_durations_ms.borrow().as_slice(), &[250]);
        assert_eq!(
            executor.executed_step_kinds.borrow().as_slice(),
            &["wait:250".to_owned()]
        );
        assert!(executor.moved_target_labels.borrow().is_empty());
        assert!(executor.played_animation_refs.borrow().is_empty());
    }

    #[test]
    fn timeline_move_to_named_targets_serialize_as_runtime_target_kind() {
        let steps = vec![
            TimelineStep::MoveTo(MoveToStep::center(
                "step_019f4500-0000-7000-8000-000000000021",
                15_000,
            )),
            TimelineStep::MoveTo(MoveToStep::home(
                "step_019f4500-0000-7000-8000-000000000022",
                15_000,
            )),
            TimelineStep::MoveTo(MoveToStep::position(
                "step_019f4500-0000-7000-8000-000000000023",
                120,
                640,
                15_000,
            )),
            TimelineStep::MoveTo(MoveToStep::x(
                "step_019f4500-0000-7000-8000-000000000024",
                320,
                15_000,
            )),
        ];

        assert_eq!(
            serde_json::to_value(steps).expect("serialize named target steps"),
            json!([
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000021",
                    "kind": "moveTo",
                    "target": { "kind": "center" },
                    "afterActionId": null,
                    "timeoutMs": 15000
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000022",
                    "kind": "moveTo",
                    "target": { "kind": "home" },
                    "afterActionId": null,
                    "timeoutMs": 15000
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000023",
                    "kind": "moveTo",
                    "target": { "kind": "position", "x": 120, "y": 640 },
                    "afterActionId": null,
                    "timeoutMs": 15000
                },
                {
                    "stepId": "step_019f4500-0000-7000-8000-000000000024",
                    "kind": "moveTo",
                    "target": { "kind": "x", "x": 320 },
                    "afterActionId": null,
                    "timeoutMs": 15000
                }
            ])
        );
    }

    #[test]
    fn timeline_executor_dispatches_move_to_position_and_x_steps() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let steps = [
            TimelineStep::MoveTo(MoveToStep::position(
                "step_019f4500-0000-7000-8000-000000000025",
                120,
                640,
                15_000,
            )),
            TimelineStep::MoveTo(MoveToStep::x(
                "step_019f4500-0000-7000-8000-000000000026",
                320,
                15_000,
            )),
        ];

        let report = execute_timeline_steps(
            &executor,
            &registry,
            &ResolveContext::default(),
            steps.as_slice(),
        )
        .expect("execute position moves");

        assert_eq!(report.completed_step_count, 2);
        assert_eq!(
            executor.moved_target_labels.borrow().as_slice(),
            &["position:120,640".to_owned(), "x:320".to_owned()]
        );
    }

    #[test]
    fn timeline_executor_snapshots_and_restores_current_position() {
        let executor = FakeStepExecutor::default();
        *executor.state_position.borrow_mut() = Some((120, 640));
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let steps = vec![
            TimelineStep::SnapshotPosition(SnapshotPositionStep {
                step_id: "step_019f4500-0000-7000-8000-000000000027".to_owned(),
                kind: "snapshotPosition".to_owned(),
                snapshot_id: "origin".to_owned(),
            }),
            TimelineStep::MoveTo(MoveToStep::center(
                "step_019f4500-0000-7000-8000-000000000028",
                15_000,
            )),
            TimelineStep::RestorePosition(RestorePositionStep {
                step_id: "step_019f4500-0000-7000-8000-000000000029".to_owned(),
                kind: "restorePosition".to_owned(),
                snapshot_id: "origin".to_owned(),
                after_action_id: Some("idle".to_owned()),
                fallback_after_action_id: None,
                timeout_ms: 15_000,
            }),
        ];

        let report = execute_timeline_steps(
            &executor,
            &registry,
            &ResolveContext::default(),
            steps.as_slice(),
        )
        .expect("execute snapshot restore timeline");

        assert_eq!(report.completed_step_count, 3);
        assert_eq!(
            executor.moved_target_labels.borrow().as_slice(),
            &["center".to_owned(), "position:120,640".to_owned()]
        );
    }

    #[test]
    fn timeline_executor_runs_plan_steps_in_order() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let steps = vec![
            TimelineStep::MoveTo(MoveToStep::edge(
                "step_019f4500-0000-7000-8000-000000000101",
                MoveEdge::Left,
                15_000,
            )),
            TimelineStep::MoveTo(MoveToStep::edge(
                "step_019f4500-0000-7000-8000-000000000102",
                MoveEdge::Top,
                15_000,
            )),
            TimelineStep::Wait(WaitStep::new(
                "step_019f4500-0000-7000-8000-000000000104",
                500,
                1_000,
            )),
            TimelineStep::PlayAction(super::timeline::PlayActionStep {
                step_id: "step_019f4500-0000-7000-8000-000000000103".to_owned(),
                kind: "playAction".to_owned(),
                action_id: "celebrate".to_owned(),
                fallback_action_id: None,
                expected_playback: "once".to_owned(),
                duration_ms: None,
                pending_handoff_finalizer_step_id: None,
                completion_behavior: crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
                timeout_ms: 5_000,
            }),
        ];

        let report = execute_timeline_steps(
            &executor,
            &registry,
            &ResolveContext::default(),
            steps.as_slice(),
        )
        .expect("execute timeline steps");

        assert_eq!(report.completed_step_count, 4);
        assert_eq!(
            executor.executed_step_kinds.borrow().as_slice(),
            &[
                "moveTo:Left".to_owned(),
                "moveTo:Top".to_owned(),
                "wait:500".to_owned(),
                "playAction:celebrate".to_owned(),
            ]
        );
    }

    #[test]
    fn timeline_executor_resolves_loop_for_duration_play_action_from_step_duration() {
        let executor = FakeStepExecutor::default();
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = TimelineStep::PlayAction(super::timeline::PlayActionStep {
            step_id: "step_019f4500-0000-7000-8000-000000000111".to_owned(),
            kind: "playAction".to_owned(),
            action_id: "celebrate".to_owned(),
            fallback_action_id: None,
            expected_playback: "loopForDuration".to_owned(),
            duration_ms: Some(10_000),
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms: 11_000,
        });

        execute_timeline_step(&executor, &registry, &ResolveContext::default(), &step)
            .expect("execute loopForDuration play action step");

        assert_eq!(
            executor.played_playback_kinds.borrow().as_slice(),
            &["loopForDuration".to_owned()]
        );
        assert_eq!(executor.played_durations_ms.borrow().as_slice(), &[10_000]);
    }

    #[test]
    fn move_to_step_resolved_event_serializes_stable_payload() {
        let plan = create_ai_macro_demo_dev_fixture_plan(
            "plan_019f4000-0000-7000-8000-000000000101",
            "beat_019f4000-0000-7000-8000-000000000102",
            "step_019f4000-0000-7000-8000-000000000103",
            "2026-07-08T00:00:00.000Z",
        )
        .expect("create ai macro demo plan");
        let step = MoveToStep::home_with_after_action(
            "step_019f4000-0000-7000-8000-000000000104",
            "sleep",
            15_000,
        );
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let after_action_resolution = super::step_resolution::resolve_move_to_after_action(
            &registry,
            &ResolveContext::default(),
            &step,
        )
        .expect("resolve after action");

        let event = ActionLogEvent::move_to_step_resolved(
            ActionLogEventIds {
                event_id: "evt_019f4000-0000-7000-8000-000000000105",
                plan_id: &plan.plan_id,
                step_id: Some(&step.step_id),
            },
            &plan.source_ref,
            &step,
            after_action_resolution.as_ref(),
            &ResolveContext::default(),
            "2026-07-08T00:00:00.010Z",
        );

        assert_eq!(
            serde_json::to_value(event).expect("serialize event"),
            json!({
                "eventId": "evt_019f4000-0000-7000-8000-000000000105",
                "schemaVersion": 1,
                "eventType": "step.resolved",
                "status": "resolved",
                "reasonCode": "devFixture.stepResolved",
                "planId": "plan_019f4000-0000-7000-8000-000000000101",
                "stepId": "step_019f4000-0000-7000-8000-000000000104",
                "sourceRef": {
                    "kind": "devFixture",
                    "fixtureName": "ai-macro-demo"
                },
                "triggerSource": "devFixture",
                "payload": {
                    "stepKind": "moveTo",
                    "target": { "kind": "home" },
                    "afterActionId": "sleep",
                    "afterResolvedActionId": "sleep",
                    "afterAnimationRef": "sleep",
                    "timeoutMs": 15000,
                    "resolveContext": {
                        "affectiveContext": {
                            "mood": "neutral",
                            "energy": "medium"
                        },
                        "affectiveContextSource": "defaultCreated"
                    }
                },
                "createdAt": "2026-07-08T00:00:00.010Z"
            })
        );
    }

    #[test]
    fn action_log_sink_writes_jsonl_and_sqlite_projection() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let sink = ActionLogSink::new(storage.clone());
        let plan = create_single_play_action_dev_fixture_plan(
            "plan_019f4000-0000-7000-8000-000000000001",
            "step_019f4000-0000-7000-8000-000000000002",
            "2026-07-08T00:00:00.000Z",
        );
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = plan
            .only_play_action_step()
            .expect("single play action fixture step");
        let resolution = registry
            .resolve_play_action(&step.action_id, &ResolveContext::default())
            .expect("resolve fixture action");

        for event in [
            ActionLogEvent::plan_started(
                "evt_019f4000-0000-7000-8000-000000000003",
                &plan,
                "2026-07-08T00:00:00.000Z",
            ),
            ActionLogEvent::step_resolved(
                ActionLogEventIds {
                    event_id: "evt_019f4000-0000-7000-8000-000000000004",
                    plan_id: &plan.plan_id,
                    step_id: Some(&step.step_id),
                },
                &plan.source_ref,
                &resolution,
                &ResolveContext::default(),
                "2026-07-08T00:00:00.010Z",
            ),
            ActionLogEvent::step_completed(
                ActionLogEventIds {
                    event_id: "evt_019f4000-0000-7000-8000-000000000005",
                    plan_id: &plan.plan_id,
                    step_id: Some(&step.step_id),
                },
                &plan.source_ref,
                &resolution,
                1720,
                "2026-07-08T00:00:01.730Z",
            ),
            ActionLogEvent::plan_completed(
                "evt_019f4000-0000-7000-8000-000000000006",
                &plan,
                1,
                1730,
                "2026-07-08T00:00:01.730Z",
            ),
        ] {
            sink.append_event(&event).expect("append action log event");
        }

        assert_eq!(storage.read_action_log_jsonl_lines_for_test().len(), 4);
        assert_eq!(
            storage.action_log_event_types_for_test(&plan.plan_id),
            vec![
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            storage.action_log_plan_summary_for_test(&plan.plan_id),
            json!({
                "status": "completed",
                "lastEventType": "plan.completed",
                "lastReasonCode": "devFixture.completed",
                "resolvedActionId": "celebrate",
                "resolvedAnimationRef": "celebrate"
            })
        );
    }

    #[test]
    fn preset_behavior_action_log_uses_stable_source_ref_and_interaction_id() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        append_native_pet_preset_behavior_action_log(
            &storage,
            &crate::native_pet::NativePetPresetBehaviorEvent {
                preset_behavior_id: "throw_after_drag".to_owned(),
                interaction_id: Some("interaction_019f4200".to_owned()),
                outcome: "fall".to_owned(),
                animation: "trip_fall_left".to_owned(),
            },
            NativePetPresetBehaviorLogContext::fixed_for_test(),
        )
        .expect("append preset behavior action log");
        let plan = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                source_ref_kind: Some("presetBehavior".to_owned()),
                source_ref_id: Some("throw_after_drag".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans")
            .items
            .into_iter()
            .next()
            .expect("preset behavior plan");

        assert_eq!(plan.source_ref_kind, "presetBehavior");
        assert_eq!(plan.source_ref["kind"], "presetBehavior");
        assert_eq!(plan.source_ref["presetBehaviorId"], "throw_after_drag");
        assert_eq!(plan.source_ref["interactionId"], "interaction_019f4200");
        assert_eq!(
            plan.resolved_action_id.as_deref(),
            Some("throw_after_drag.fall.left")
        );
        assert_eq!(
            plan.resolved_animation_ref.as_deref(),
            Some("trip_fall_left")
        );
        assert_eq!(plan.status, "completed");
        assert_eq!(plan.last_reason_code, "presetBehavior.completed");
    }

    #[test]
    fn action_log_plan_list_returns_newest_summaries_with_structured_filters() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_completed_action_log_fixture(
            &storage,
            CompletedActionLogFixture {
                plan_id: "plan_019f4000-0000-7000-8000-000000000101",
                step_id: "step_019f4000-0000-7000-8000-000000000102",
                event_ids: CompletedActionLogFixtureEventIds {
                    plan_started: "evt_019f4000-0000-7000-8000-000000000103",
                    step_resolved: "evt_019f4000-0000-7000-8000-000000000104",
                    step_completed: "evt_019f4000-0000-7000-8000-000000000105",
                    plan_completed: "evt_019f4000-0000-7000-8000-000000000106",
                },
                timestamps: CompletedActionLogFixtureTimestamps {
                    started_at: "2026-07-08T00:00:00.000Z",
                    resolved_at: "2026-07-08T00:00:00.010Z",
                    completed_at: "2026-07-08T00:00:01.730Z",
                },
            },
        );
        append_completed_action_log_fixture(
            &storage,
            CompletedActionLogFixture {
                plan_id: "plan_019f4000-0000-7000-8000-000000000201",
                step_id: "step_019f4000-0000-7000-8000-000000000202",
                event_ids: CompletedActionLogFixtureEventIds {
                    plan_started: "evt_019f4000-0000-7000-8000-000000000203",
                    step_resolved: "evt_019f4000-0000-7000-8000-000000000204",
                    step_completed: "evt_019f4000-0000-7000-8000-000000000205",
                    plan_completed: "evt_019f4000-0000-7000-8000-000000000206",
                },
                timestamps: CompletedActionLogFixtureTimestamps {
                    started_at: "2026-07-08T00:10:00.000Z",
                    resolved_at: "2026-07-08T00:10:00.010Z",
                    completed_at: "2026-07-08T00:10:01.730Z",
                },
            },
        );

        let list = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(10),
                source_ref_kind: Some("devFixture".to_owned()),
                status: Some("completed".to_owned()),
                page_cursor: None,
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans");

        assert_eq!(
            list.items
                .iter()
                .map(|plan| plan.plan_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "plan_019f4000-0000-7000-8000-000000000201",
                "plan_019f4000-0000-7000-8000-000000000101"
            ]
        );
        assert!(!list.has_more);
        assert!(list.next_page_cursor.is_none());

        let first_page = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(1),
                source_ref_kind: Some("devFixture".to_owned()),
                status: Some("completed".to_owned()),
                page_cursor: None,
                ..ActionLogPlanListRequest::default()
            })
            .expect("list first action log page");
        assert_eq!(
            first_page
                .items
                .iter()
                .map(|plan| plan.plan_id.as_str())
                .collect::<Vec<_>>(),
            vec!["plan_019f4000-0000-7000-8000-000000000201"]
        );
        assert!(first_page.has_more);
        let second_page = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                limit: Some(1),
                source_ref_kind: Some("devFixture".to_owned()),
                status: Some("completed".to_owned()),
                page_cursor: first_page.next_page_cursor,
                ..ActionLogPlanListRequest::default()
            })
            .expect("list second action log page");
        assert_eq!(
            second_page
                .items
                .iter()
                .map(|plan| plan.plan_id.as_str())
                .collect::<Vec<_>>(),
            vec!["plan_019f4000-0000-7000-8000-000000000101"]
        );
    }

    #[test]
    fn action_log_plan_detail_returns_sanitized_summary_and_ordered_steps() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_completed_action_log_fixture(
            &storage,
            CompletedActionLogFixture {
                plan_id: "plan_019f4000-0000-7000-8000-000000000301",
                step_id: "step_019f4000-0000-7000-8000-000000000302",
                event_ids: CompletedActionLogFixtureEventIds {
                    plan_started: "evt_019f4000-0000-7000-8000-000000000303",
                    step_resolved: "evt_019f4000-0000-7000-8000-000000000304",
                    step_completed: "evt_019f4000-0000-7000-8000-000000000305",
                    plan_completed: "evt_019f4000-0000-7000-8000-000000000306",
                },
                timestamps: CompletedActionLogFixtureTimestamps {
                    started_at: "2026-07-08T00:20:00.000Z",
                    resolved_at: "2026-07-08T00:20:00.010Z",
                    completed_at: "2026-07-08T00:20:01.730Z",
                },
            },
        );

        let detail = storage
            .get_action_log_plan_detail("plan_019f4000-0000-7000-8000-000000000301")
            .expect("get action log plan detail");

        assert_eq!(detail.plan.status, "completed");
        assert_eq!(
            detail
                .steps
                .iter()
                .map(|step| step.step_id.as_str())
                .collect::<Vec<_>>(),
            vec!["step_019f4000-0000-7000-8000-000000000302"]
        );
        assert_eq!(
            detail
                .steps
                .first()
                .and_then(|step| step.resolved_action_id.as_deref()),
            Some("celebrate")
        );
        let serialized = serde_json::to_string(&detail).expect("serialize detail");
        assert!(!serialized.contains("payload"));
        assert!(!serialized.contains("affectiveContext"));
    }

    #[test]
    fn action_log_plan_detail_projects_window_anchor_target_labels() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let sink = ActionLogSink::new(storage.clone());
        let plan = super::fixture::DevFixturePlan {
            plan_id: "plan_019f4000-0000-7000-8000-000000000411".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "ai-macro-demo"
            }),
            steps: vec![TimelineStep::MoveTo(MoveToStep::window_anchor(
                "step_019f4000-0000-7000-8000-000000000412",
                WindowAnchorSelector {
                    kind: WindowAnchorSelectorKind::ActiveWindow,
                },
                MoveEdge::Left,
                WindowAnchorReveal::Head,
                3_000,
                18_000,
            ))],
            created_at: "2026-07-08T00:35:00.000Z".to_owned(),
        };
        let step = match plan.steps.first().expect("move step") {
            TimelineStep::MoveTo(step) => step,
            TimelineStep::PlayAction(_)
            | TimelineStep::MoveByPath(_)
            | TimelineStep::Wait(_)
            | TimelineStep::Skip(_)
            | TimelineStep::Retry(_)
            | TimelineStep::Replace(_)
            | TimelineStep::Recover(_)
            | TimelineStep::Repeat(_)
            | TimelineStep::Choose(_)
            | TimelineStep::SetFallback(_)
            | TimelineStep::Try(_)
            | TimelineStep::SnapshotPosition(_)
            | TimelineStep::RestorePosition(_) => panic!("expected move step"),
        };

        for event in [
            ActionLogEvent::plan_started(
                "evt_019f4000-0000-7000-8000-000000000413",
                &plan,
                "2026-07-08T00:35:00.000Z",
            ),
            ActionLogEvent::move_to_step_completed(
                ActionLogEventIds {
                    event_id: "evt_019f4000-0000-7000-8000-000000000414",
                    plan_id: &plan.plan_id,
                    step_id: Some(&step.step_id),
                },
                &plan.source_ref,
                step,
                "2026-07-08T00:35:01.000Z",
            ),
        ] {
            sink.append_event(&event).expect("append action log event");
        }

        let detail = storage
            .get_action_log_plan_detail(&plan.plan_id)
            .expect("get action log plan detail");

        assert_eq!(
            detail
                .steps
                .first()
                .and_then(|step| step.target_label.as_deref()),
            Some("windowAnchor:activeWindow:left:head")
        );
    }

    #[test]
    fn affective_context_store_creates_default_state_file_when_missing() {
        let buddy_home = std::env::temp_dir().join(format!(
            "lexora-buddy-affective-context-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AffectiveContextStore::from_buddy_home(buddy_home);

        let snapshot = store.read_or_create_default().expect("read context");

        assert_eq!(
            snapshot,
            AffectiveContextSnapshot {
                context: AffectiveContext::default(),
                source: AffectiveContextSource::DefaultCreated,
            }
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &fs::read_to_string(store.state_file_path()).expect("read state file")
            )
            .expect("parse state file"),
            json!({ "mood": "neutral", "energy": "medium" })
        );
    }

    #[test]
    fn affective_context_store_preserves_invalid_file_and_uses_default_context() {
        let buddy_home = std::env::temp_dir().join(format!(
            "lexora-buddy-affective-context-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AffectiveContextStore::from_buddy_home(buddy_home);
        fs::create_dir_all(store.state_file_path().parent().expect("state file parent"))
            .expect("create state parent");
        fs::write(store.state_file_path(), "{broken").expect("write invalid state");

        let snapshot = store.read_or_create_default().expect("read context");

        assert_eq!(
            snapshot,
            AffectiveContextSnapshot {
                context: AffectiveContext::default(),
                source: AffectiveContextSource::InvalidFileFallback,
            }
        );
        assert_eq!(
            fs::read_to_string(store.state_file_path()).expect("read preserved state"),
            "{broken"
        );
    }

    #[test]
    fn affective_context_store_logs_invalid_file_system_event() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let buddy_home = std::env::temp_dir().join(format!(
            "lexora-buddy-affective-context-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AffectiveContextStore::from_buddy_home(buddy_home);
        fs::create_dir_all(store.state_file_path().parent().expect("state file parent"))
            .expect("create state parent");
        fs::write(store.state_file_path(), "{broken").expect("write invalid state");

        let snapshot = store
            .read_or_create_default_with_diagnostics(&storage)
            .expect("read context with diagnostics");

        assert_eq!(
            snapshot,
            AffectiveContextSnapshot {
                context: AffectiveContext::default(),
                source: AffectiveContextSource::InvalidFileFallback,
            }
        );

        let events = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("affectiveContext.invalidStateFile".to_owned()),
                source_ref_kind: Some("affectiveContext".to_owned()),
                reason_code: Some("affectiveContext.invalidStateFile".to_owned()),
                status: Some("degraded".to_owned()),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query affective context system events");

        assert_eq!(events.items.len(), 1);
        let event = events.items.first().expect("system event");
        assert_eq!(event.event_type, "affectiveContext.invalidStateFile");
        assert_eq!(event.source_ref.kind, "affectiveContext");
        assert_eq!(event.trigger_source, "affectiveContext");
        assert_eq!(event.status, "degraded");
        assert_eq!(event.reason_code, "affectiveContext.invalidStateFile");
        assert!(event.plan_id.is_none());
        assert!(event.step_id.is_none());
        assert!(storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list action log plans")
            .items
            .is_empty());
    }

    #[test]
    fn executor_logs_success_path_and_dispatches_resolved_animation_ref() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let report = execute_single_play_action_dev_fixture(
            storage.clone(),
            &executor,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
        )
        .expect("execute fixture");

        assert_eq!(
            executor.played_animation_refs.into_inner(),
            vec!["celebrate".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
    }

    #[test]
    fn executor_admission_wrapper_releases_active_plan_after_success() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();

        let report = execute_single_play_action_dev_fixture_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute fixture through admission");

        assert!(report.executed);
        assert!(matches!(
            report.decision,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
    }

    #[test]
    fn executor_admission_wrapper_rejects_busy_plan_without_dispatching_steps() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(ChoreographyAdmissionRequest::new(
            "plan_active_ai",
            ChoreographyTriggerSource::AiChoreography,
        ));

        let report = execute_single_play_action_dev_fixture_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("write rejected admission event");

        assert!(!report.executed);
        assert!(matches!(
            report.decision,
            ChoreographyAdmissionDecision::Rejected { .. }
        ));
        assert_eq!(admission.active_plan_id(), Some("plan_active_ai"));
        assert!(executor.played_animation_refs.into_inner().is_empty());
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec!["executor.rejected"]
        );
    }

    #[test]
    fn executor_admission_wrapper_interrupts_preempted_active_step_before_dispatch() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new(
                "plan_idle",
                ChoreographyTriggerSource::IdleAutonomous,
            )
            .with_active_step("step_idle_active", SidecarInterruptPolicy::Interruptible),
        );

        let report = execute_single_play_action_dev_fixture_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::UserRequested,
        )
        .expect("preempt and execute fixture");

        assert!(report.executed);
        assert!(matches!(
            report.decision,
            ChoreographyAdmissionDecision::Preempted { .. }
        ));
        assert_eq!(
            executor.interrupted_steps.into_inner(),
            vec![(
                "step_idle_active".to_owned(),
                "admission.preemptedByHigherPriorityPlan".to_owned()
            )]
        );
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.preempted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
    }

    #[test]
    fn executor_admission_wrapper_defers_finish_step_preemption_without_interrupting() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        admission.admit(
            ChoreographyAdmissionRequest::new("plan_ai", ChoreographyTriggerSource::AiChoreography)
                .with_active_step("step_ai_finish", SidecarInterruptPolicy::FinishStep),
        );

        let report = execute_single_play_action_dev_fixture_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::UserRequested,
        )
        .expect("defer fixture while finishStep active");

        assert!(!report.executed);
        assert!(matches!(
            report.decision,
            ChoreographyAdmissionDecision::Deferred { .. }
        ));
        assert_eq!(admission.active_plan_id(), Some("plan_ai"));
        assert!(executor.interrupted_steps.into_inner().is_empty());
        assert!(executor.played_animation_refs.into_inner().is_empty());
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec!["executor.deferred"]
        );
    }

    #[test]
    fn executor_records_affective_context_source_in_step_resolved_payload() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();

        let report = execute_single_play_action_dev_fixture(
            storage.clone(),
            &executor,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::from_affective_snapshot(AffectiveContextSnapshot {
                context: AffectiveContext::default(),
                source: AffectiveContextSource::StateFile,
            }),
        )
        .expect("execute fixture");

        let resolved_event = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .find(|event| event["eventType"] == "step.resolved")
            .expect("resolved event");

        assert_eq!(report.plan_id, "plan_019f4000-0000-7000-8000-000000000001");
        assert_eq!(
            resolved_event["payload"]["resolveContext"],
            json!({
                "affectiveContext": {
                    "mood": "neutral",
                    "energy": "medium"
                },
                "affectiveContextSource": "stateFile"
            })
        );
    }

    #[test]
    fn executor_logs_failure_path_and_marks_plan_failed() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let error = execute_single_play_action_dev_fixture(
            storage.clone(),
            &FailingStepExecutor,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
        )
        .expect_err("execution should fail");

        match error {
            DevFixtureExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            DevFixtureExecutionError::ActionLog(error) => {
                panic!("expected execution error, got action log error: {error}")
            }
        }
        assert_eq!(
            storage.action_log_event_types_for_test("plan_019f4000-0000-7000-8000-000000000001"),
            vec![
                "plan.started",
                "step.resolved",
                "step.failed",
                "plan.failed"
            ]
        );
        assert_eq!(
            storage.action_log_plan_summary_for_test("plan_019f4000-0000-7000-8000-000000000001"),
            json!({
                "status": "failed",
                "lastEventType": "plan.failed",
                "lastReasonCode": "devFixture.failed",
                "resolvedActionId": "celebrate",
                "resolvedAnimationRef": "celebrate"
            })
        );
    }

    #[test]
    fn executor_triggers_runtime_safe_fallback_after_step_execution_failure() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();

        let error = execute_single_play_action_dev_fixture(
            storage.clone(),
            &executor,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
        )
        .expect_err("original fixture execution should fail");

        match error {
            DevFixtureExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            DevFixtureExecutionError::ActionLog(error) => {
                panic!("expected original execution error, got action log error: {error}")
            }
        }
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["playAction:celebrate".to_owned(), "moveTo:home".to_owned()]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            recovery_events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_019f4000-0000-7000-8000-000000000001",
                "triggeredByStepId": "step_019f4000-0000-7000-8000-000000000002",
                "triggerReason": "executor.error"
            })
        );
        let recovery_plan_id = recovery_events[0]["planId"]
            .as_str()
            .expect("recovery plan id");
        assert_eq!(
            storage.action_log_plan_summary_for_test(recovery_plan_id),
            json!({
                "status": "completed",
                "lastEventType": "plan.completed",
                "lastReasonCode": "systemRecovery.completed",
                "resolvedActionId": "sleep",
                "resolvedAnimationRef": "sleep"
            })
        );

        let default_plan_ids = storage
            .list_action_log_plans(ActionLogPlanListRequest::default())
            .expect("list default action log plans")
            .items
            .into_iter()
            .map(|plan| plan.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            default_plan_ids,
            vec!["plan_019f4000-0000-7000-8000-000000000001".to_owned()]
        );

        let recovery_source_plan_ids = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                source_ref_kind: Some("systemRecovery".to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list system recovery action log plans")
            .items
            .into_iter()
            .map(|plan| plan.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(recovery_source_plan_ids, vec![recovery_plan_id.to_owned()]);

        let explicit_recovery_plan_ids = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                plan_id: Some(recovery_plan_id.to_owned()),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list explicit recovery action log plan")
            .items
            .into_iter()
            .map(|plan| plan.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            explicit_recovery_plan_ids,
            vec![recovery_plan_id.to_owned()]
        );

        let original_plan_detail = storage
            .get_action_log_plan_detail("plan_019f4000-0000-7000-8000-000000000001")
            .expect("read original plan detail");
        assert_eq!(original_plan_detail.recovery_plans.len(), 1);
        let folded_recovery = &original_plan_detail.recovery_plans[0];
        assert_eq!(folded_recovery.plan.plan_id, recovery_plan_id);
        assert_eq!(folded_recovery.plan.source_ref_kind, "systemRecovery");
        assert_eq!(
            folded_recovery
                .steps
                .iter()
                .map(|step| (
                    step.step_kind.as_deref(),
                    step.resolved_action_id.as_deref(),
                    step.resolved_animation_ref.as_deref(),
                    step.status.as_str(),
                ))
                .collect::<Vec<_>>(),
            vec![(Some("moveTo"), Some("sleep"), Some("sleep"), "completed")]
        );
    }

    #[test]
    fn executor_admission_wrapper_admits_runtime_safe_fallback_after_failure() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();

        let error = execute_single_play_action_dev_fixture_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            DevFixtureExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect_err("original admitted fixture execution should fail");

        match error {
            DevFixtureExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            DevFixtureExecutionError::ActionLog(error) => {
                panic!("expected original execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["playAction:celebrate".to_owned(), "moveTo:home".to_owned()]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            recovery_events[0]["payload"]["triggerSource"],
            json!("systemRecovery")
        );
        assert_eq!(
            recovery_events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_019f4000-0000-7000-8000-000000000001",
                "triggeredByStepId": "step_019f4000-0000-7000-8000-000000000002",
                "triggerReason": "executor.error"
            })
        );
    }

    #[test]
    fn production_timeline_executor_releases_active_plan_after_success() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_success_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000202",
                "messageId": "message_019f4000-0000-7000-8000-000000000302",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                "step_timeline_success_019f4000-0000-7000-8000-000000000501",
                30_000,
            ))],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::new(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute production timeline through admission");

        assert!(report.executed);
        assert!(matches!(
            report.decision,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert_eq!(report.plan_id, plan_id);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
        assert_eq!(
            storage.action_log_plan_summary_for_test(&report.plan_id),
            json!({
                "status": "completed",
                "lastEventType": "plan.completed",
                "lastReasonCode": "timeline.completed",
                "resolvedActionId": null,
                "resolvedAnimationRef": null
            })
        );
    }

    #[test]
    fn production_timeline_executor_executes_promoted_pending_plan_after_active_release() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let mut pending_queue = PendingTimelineExecutionQueue::default();
        let active_plan_id = "plan_timeline_active_019f5a00-0000-7000-8000-000000000101";
        admission.admit(
            ChoreographyAdmissionRequest::new(
                active_plan_id,
                ChoreographyTriggerSource::AiChoreography,
            )
            .with_active_step(
                "step_timeline_active_019f5a00-0000-7000-8000-000000000401",
                SidecarInterruptPolicy::FinishStep,
            ),
        );
        let pending_plan = TimelinePlan {
            plan_id: "plan_timeline_pending_019f5a00-0000-7000-8000-000000000102".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5a00-0000-7000-8000-000000000203",
                "messageId": "message_019f5a00-0000-7000-8000-000000000303",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                "step_timeline_pending_019f5a00-0000-7000-8000-000000000501",
                30_000,
            ))],
            created_at: "2026-07-08T00:00:00.050Z".to_owned(),
        };
        let pending_plan_id = pending_plan.plan_id.clone();

        let pending_report = execute_timeline_plan_with_admission_and_pending_queue(
            &executor,
            &mut admission,
            &mut pending_queue,
            TimelineAdmissionExecutionRequest::new(
                storage.clone(),
                pending_plan,
                TimelineExecutionContext::fixed_for_test(),
                ResolveContext::default(),
                ChoreographyTriggerSource::UserRequested,
            ),
        )
        .expect("queue pending timeline through admission");
        let release = admission.release_plan(active_plan_id);
        let promoted_report = execute_released_pending_timeline_plan(
            &executor,
            &mut admission,
            &mut pending_queue,
            release,
        )
        .expect("pending plan should be promoted")
        .expect("execute promoted pending timeline");

        assert!(!pending_report.executed);
        assert!(promoted_report.executed);
        assert!(matches!(
            promoted_report.decision,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["moveTo:center".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&pending_plan_id),
            vec![
                "executor.deferred",
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
    }

    #[test]
    fn production_timeline_executor_expands_planner_steps_before_logging() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_expand_019f5400-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5400-0000-7000-8000-000000000202",
                "messageId": "message_019f5400-0000-7000-8000-000000000302",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::SetFallback(SetFallbackStep {
                step_id: "step_timeline_expand_019f5400-0000-7000-8000-000000000401".to_owned(),
                kind: "setFallback".to_owned(),
                fallback_action_id: "idle".to_owned(),
                steps: vec![TimelineStep::Repeat(RepeatStep {
                    step_id: "step_timeline_expand_019f5400-0000-7000-8000-000000000402".to_owned(),
                    kind: "repeat".to_owned(),
                    times: 2,
                    steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                        "step_timeline_expand_019f5400-0000-7000-8000-000000000403",
                        "missing.action",
                        5_000,
                    ))],
                })],
            })],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute expanded production timeline through admission");

        assert!(report.executed);
        assert_eq!(
            *executor.played_animation_refs.borrow(),
            vec!["idle".to_owned(), "idle".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .filter(|event| event["planId"] == report.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(events[1]["payload"]["stepCount"], json!(2));
        assert_eq!(events[6]["payload"]["completedStepCount"], json!(2));
    }

    #[test]
    fn production_timeline_executor_logs_wait_step_duration() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_wait_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000202",
                "messageId": "message_019f4000-0000-7000-8000-000000000302",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::Wait(WaitStep::new(
                "step_timeline_wait_019f4000-0000-7000-8000-000000000501",
                250,
                1_000,
            ))],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute timeline wait step through admission");

        assert_eq!(
            executor.executed_step_kinds.borrow().as_slice(),
            &["wait:250".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let detail = storage
            .get_action_log_plan_detail(&plan_id)
            .expect("get action log plan detail");
        let step = detail.steps.first().expect("wait step detail");
        assert_eq!(step.step_kind.as_deref(), Some("wait"));
        assert_eq!(step.duration_ms, Some(250));
        assert_eq!(step.elapsed_ms, Some(250));
    }

    #[test]
    fn production_timeline_executor_skips_step_with_reason_without_dispatch() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_skip_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000202",
                "messageId": "message_019f4000-0000-7000-8000-000000000302",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::MoveTo(MoveToStep::center(
                    "step_timeline_skip_019f4000-0000-7000-8000-000000000501",
                    30_000,
                )),
                TimelineStep::Skip(SkipStep {
                    step_id: "step_timeline_skip_019f4000-0000-7000-8000-000000000502".to_owned(),
                    kind: "skipStep".to_owned(),
                    reason: TimelineSkipReason::BranchNotRequired,
                }),
                TimelineStep::MoveTo(MoveToStep::home(
                    "step_timeline_skip_019f4000-0000-7000-8000-000000000503",
                    30_000,
                )),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute timeline skip step through admission");

        assert!(report.executed);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["moveTo:center".to_owned(), "moveTo:home".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "step.skipped",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let lines = storage.read_action_log_jsonl_lines_for_test();
        let events = lines
            .iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("parse event"))
            .filter(|event| event["planId"] == report.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(events[4]["status"], json!("skipped"));
        assert_eq!(events[4]["reasonCode"], json!("timeline.stepSkipped"));
        assert_eq!(events[4]["payload"]["stepKind"], json!("skipStep"));
        assert_eq!(
            events[4]["payload"]["skipReason"],
            json!("branchNotRequired")
        );
        assert_eq!(events[7]["payload"]["completedStepCount"], json!(2));
        assert_eq!(events[7]["payload"]["failedStepCount"], json!(0));
        assert_eq!(events[7]["payload"]["skippedStepCount"], json!(1));
        assert_eq!(events[7]["payload"]["durationMs"], json!(0));

        let detail = storage
            .get_action_log_plan_detail(&plan_id)
            .expect("get action log plan detail");
        let skipped_step = detail.steps.get(1).expect("skip step detail");
        assert_eq!(skipped_step.status, "skipped");
        assert_eq!(skipped_step.reason_code, "timeline.stepSkipped");
        assert_eq!(skipped_step.step_kind.as_deref(), Some("skipStep"));
    }

    #[test]
    fn production_timeline_executor_logs_snapshot_restore_position_steps() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        *executor.state_position.borrow_mut() = Some((120, 640));
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_restore_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000202",
                "messageId": "message_019f4000-0000-7000-8000-000000000302",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::SnapshotPosition(SnapshotPositionStep {
                    step_id: "step_timeline_restore_019f4000-0000-7000-8000-000000000501"
                        .to_owned(),
                    kind: "snapshotPosition".to_owned(),
                    snapshot_id: "origin".to_owned(),
                }),
                TimelineStep::MoveTo(MoveToStep::center(
                    "step_timeline_restore_019f4000-0000-7000-8000-000000000502",
                    15_000,
                )),
                TimelineStep::RestorePosition(RestorePositionStep {
                    step_id: "step_timeline_restore_019f4000-0000-7000-8000-000000000503"
                        .to_owned(),
                    kind: "restorePosition".to_owned(),
                    snapshot_id: "origin".to_owned(),
                    after_action_id: Some("sleep".to_owned()),
                    fallback_after_action_id: Some("idle".to_owned()),
                    timeout_ms: 15_000,
                }),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute timeline snapshot restore through admission");

        assert_eq!(
            executor.moved_target_labels.borrow().as_slice(),
            &["center".to_owned(), "position:120,640".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let detail = storage
            .get_action_log_plan_detail(&plan_id)
            .expect("get action log plan detail");
        assert_eq!(detail.steps.len(), 3);
        assert_eq!(
            detail.steps[0].step_kind.as_deref(),
            Some("snapshotPosition")
        );
        assert_eq!(detail.steps[0].target_label, None);
        assert_eq!(
            detail.steps[2].step_kind.as_deref(),
            Some("restorePosition")
        );
        assert_eq!(
            detail.steps[2].target_label.as_deref(),
            Some("position:120,640")
        );
    }

    #[test]
    fn production_timeline_executor_projects_step_fallback_resolution() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_step_fallback_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000202",
                "messageId": "message_019f4000-0000-7000-8000-000000000302",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::PlayAction(PlayActionStep {
                step_id: "step_timeline_step_fallback_019f4000-0000-7000-8000-000000000501"
                    .to_owned(),
                kind: "playAction".to_owned(),
                action_id: "missing.action".to_owned(),
                fallback_action_id: Some("celebrate".to_owned()),
                expected_playback: "once".to_owned(),
                duration_ms: None,
                pending_handoff_finalizer_step_id: None,
                completion_behavior: crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
                timeout_ms: 5_000,
            })],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute timeline using step fallback");

        let resolved_event = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .find(|event| event["eventType"] == "step.resolved")
            .expect("resolved event");
        let plan_summary = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                plan_id: Some(plan_id.clone()),
                limit: Some(10),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");

        assert_eq!(
            executor.played_animation_refs.into_inner(),
            vec!["celebrate"]
        );
        assert_eq!(resolved_event["payload"]["actionId"], json!("celebrate"));
        assert_eq!(resolved_event["payload"]["resultKind"], json!("fallback"));
        assert_eq!(
            resolved_event["payload"]["detailReasonCode"],
            json!("fallback.stepActionResolved")
        );
        assert_eq!(
            resolved_event["payload"]["fallback"],
            json!({
                "requestedActionId": "missing.action",
                "fallbackActionId": "celebrate",
                "reasonCode": "fallback.stepActionResolved"
            })
        );
        assert_eq!(plan_summary.plan_id, plan_id);
        assert_eq!(plan_summary.status, "completed");
        assert_eq!(plan_summary.result_kind, "fallback");
        assert_eq!(plan_summary.detail_status, "fallback");
        assert_eq!(
            plan_summary.detail_reason_code,
            "fallback.stepActionResolved"
        );
    }

    #[test]
    fn production_timeline_executor_projects_after_action_fallback_resolution() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let mut step = MoveToStep::home_with_after_action(
            "step_timeline_after_fallback_019f4000-0000-7000-8000-000000000501",
            "missing.after_action",
            15_000,
        );
        step.fallback_after_action_id = Some("sleep".to_owned());
        let plan = TimelinePlan {
            plan_id: "plan_timeline_after_fallback_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000212",
                "messageId": "message_019f4000-0000-7000-8000-000000000312",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::MoveTo(step)],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("execute timeline using afterAction fallback");

        let resolved_event = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .find(|event| event["eventType"] == "step.resolved")
            .expect("resolved event");
        let plan_summary = storage
            .list_action_log_plans(ActionLogPlanListRequest {
                plan_id: Some(plan_id.clone()),
                limit: Some(10),
                ..ActionLogPlanListRequest::default()
            })
            .expect("list action log plans")
            .items
            .into_iter()
            .next()
            .expect("plan summary");

        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["moveTo:home"]
        );
        assert_eq!(
            resolved_event["payload"]["afterActionId"],
            json!("missing.after_action")
        );
        assert_eq!(
            resolved_event["payload"]["afterAnimationRef"],
            json!("sleep")
        );
        assert_eq!(
            resolved_event["payload"]["afterResolvedActionId"],
            json!("sleep")
        );
        assert_eq!(resolved_event["payload"]["resultKind"], json!("fallback"));
        assert_eq!(
            resolved_event["payload"]["detailReasonCode"],
            json!("fallback.afterActionResolved")
        );
        assert_eq!(
            resolved_event["payload"]["fallback"],
            json!({
                "requestedActionId": "missing.after_action",
                "fallbackActionId": "sleep",
                "reasonCode": "fallback.afterActionResolved"
            })
        );
        assert_eq!(plan_summary.plan_id, plan_id);
        assert_eq!(plan_summary.status, "completed");
        assert_eq!(plan_summary.result_kind, "fallback");
        assert_eq!(plan_summary.detail_status, "fallback");
        assert_eq!(
            plan_summary.detail_reason_code,
            "fallback.afterActionResolved"
        );
        assert_eq!(
            plan_summary.resolved_animation_ref.as_deref(),
            Some("sleep")
        );
        assert_eq!(plan_summary.resolved_action_id.as_deref(), Some("sleep"));
    }

    #[test]
    fn production_macro_intent_executor_compiles_and_runs_through_timeline_admission() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "patrolAroundScreen",
            "params": { "loops": 1 }
        }))
        .expect("parse patrol macro intent");

        let report = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000611",
                    "messageId": "message_019f4000-0000-7000-8000-000000000612",
                    "runId": "run_019f4000-0000-7000-8000-000000000613",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect("execute production macro intent through admission");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "moveTo:Left".to_owned(),
                "moveTo:Top".to_owned(),
                "moveTo:Right".to_owned(),
                "moveTo:Bottom".to_owned(),
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .filter(|event| event["planId"] == report.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .map(|event| event["reasonCode"].as_str().expect("reason code"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "timeline.started",
                "timeline.stepResolved",
                "timeline.stepCompleted",
                "timeline.stepResolved",
                "timeline.stepCompleted",
                "timeline.stepResolved",
                "timeline.stepCompleted",
                "timeline.stepResolved",
                "timeline.stepCompleted",
                "timeline.completed",
            ]
        );
        assert_eq!(
            events[1]["sourceRef"],
            json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000611",
                "messageId": "message_019f4000-0000-7000-8000-000000000612",
                "runId": "run_019f4000-0000-7000-8000-000000000613",
            })
        );
        assert_eq!(events[1]["triggerSource"], json!("aiChoreography"));
    }

    #[test]
    fn production_macro_intent_executor_admits_runtime_safe_fallback_after_planning_failure() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let context = MacroIntentExecutionContext::fixed_for_test();
        let original_plan_id = context.plan_id.clone();
        let intent = MacroIntent::Dance(DanceMacroParams { duration_ms: 0 });

        let error = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context,
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000611",
                    "messageId": "message_019f4000-0000-7000-8000-000000000612",
                    "runId": "run_019f4000-0000-7000-8000-000000000613",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect_err("macro planning should fail before original timeline admission");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("dance durationMs"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected planning execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["moveTo:home".to_owned()]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == original_plan_id)
            .collect::<Vec<_>>();
        assert!(original_events.is_empty());

        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            recovery_events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_macro_019f4000-0000-7000-8000-000000000701",
                "triggerReason": "macro.planningFailed"
            })
        );
    }

    #[test]
    fn production_macro_intent_executor_uses_edge_peek_fallback_when_window_anchor_target_is_unavailable(
    ) {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = WindowAnchorFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekBehindWindow",
            "params": {
                "windowSelector": { "kind": "activeWindow" },
                "edge": "left",
                "reveal": "head",
                "durationMs": 3000
            }
        }))
        .expect("parse peek behind window macro intent");

        let error = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000621",
                    "messageId": "message_019f4000-0000-7000-8000-000000000622",
                    "runId": "run_019f4000-0000-7000-8000-000000000623",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect_err("windowAnchor should fail when active window target is unavailable");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert_eq!(
                    error.to_string(),
                    "runtime failed: native pet step failed: targetUnavailable: runtime failed: native pet active window rect is unavailable"
                );
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected windowAnchor execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "moveTo:windowAnchor:activeWindow:Left:head".to_owned(),
                "moveTo:edgeAnchor:Left:head:1500".to_owned(),
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == "plan_macro_019f4000-0000-7000-8000-000000000701")
            .collect::<Vec<_>>();
        assert_eq!(
            original_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );
        assert_eq!(
            original_events[2]["payload"]["target"],
            json!({
                "kind": "windowAnchor",
                "selector": { "kind": "activeWindow" },
                "edge": "left",
                "reveal": "head",
                "durationMs": 3000
            })
        );

        let macro_fallback_events = events
            .iter()
            .filter(|event| event["sourceRef"]["kind"] == "macroFallback")
            .collect::<Vec<_>>();
        assert_eq!(
            macro_fallback_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            macro_fallback_events[0]["sourceRef"],
            json!({
                "kind": "macroFallback",
                "triggeredByPlanId": "plan_macro_019f4000-0000-7000-8000-000000000701",
                "triggeredByStepId": "step_macro_019f4000-0000-7000-8000-000000000703",
                "triggerReason": "semanticFallback.windowAnchorTargetUnavailable",
                "originalMacroId": "peekBehindWindow",
                "fallbackMacroId": "peekFromEdge"
            })
        );
        assert_eq!(
            macro_fallback_events[2]["payload"]["target"]["kind"],
            "edgeAnchor"
        );
    }

    #[test]
    fn production_macro_intent_executor_recovers_peek_from_edge_inside_original_plan() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = EdgeAnchorFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekFromEdge",
            "params": { "edge": "left" }
        }))
        .expect("parse peek from edge macro intent");

        let report = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000625",
                    "messageId": "message_019f4000-0000-7000-8000-000000000626",
                    "runId": "run_019f4000-0000-7000-8000-000000000627",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect("peekFromEdge should recover inside the original timeline plan");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "moveTo:edgeAnchor:Left:head:1500".to_owned(),
                "moveTo:home".to_owned(),
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == report.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
        assert_eq!(
            original_events[2]["payload"]["target"]["kind"],
            "edgeAnchor"
        );
        assert_eq!(original_events[4]["payload"]["target"]["kind"], "home");
        assert_eq!(
            original_events[6]["payload"]["completedStepCount"],
            json!(1)
        );
        assert_eq!(original_events[6]["payload"]["failedStepCount"], json!(1));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
        assert!(events
            .iter()
            .all(|event| event["sourceRef"]["kind"] != "macroFallback"));
    }

    #[test]
    fn production_macro_intent_executor_recovers_peek_from_edge_home_recovery_failure_with_in_place_sleep(
    ) {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = EdgeAnchorAndHomeFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekFromEdge",
            "params": { "edge": "left" }
        }))
        .expect("parse peek from edge macro intent");

        let report = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000641",
                    "messageId": "message_019f4000-0000-7000-8000-000000000642",
                    "runId": "run_019f4000-0000-7000-8000-000000000643",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect("peekFromEdge should recover in place when returning home also fails");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "moveTo:edgeAnchor:Left:head:1500".to_owned(),
                "moveTo:home".to_owned(),
                "playAction:sleep".to_owned(),
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == report.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
        assert_eq!(
            original_events[2]["payload"]["target"]["kind"],
            "edgeAnchor"
        );
        assert_eq!(original_events[4]["payload"]["target"]["kind"], "home");
        assert_eq!(original_events[6]["payload"]["actionId"], "sleep");
        assert_eq!(
            original_events[8]["payload"]["completedStepCount"],
            json!(1)
        );
        assert_eq!(original_events[8]["payload"]["failedStepCount"], json!(2));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
        assert!(events
            .iter()
            .all(|event| event["sourceRef"]["kind"] != "macroFallback"));
    }

    #[test]
    fn production_macro_intent_executor_reports_final_recovery_step_when_peek_from_edge_recovery_fails(
    ) {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = EdgeAnchorHomeAndSleepFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekFromEdge",
            "params": { "edge": "left" }
        }))
        .expect("parse peek from edge macro intent");

        let error = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000651",
                    "messageId": "message_019f4000-0000-7000-8000-000000000652",
                    "runId": "run_019f4000-0000-7000-8000-000000000653",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect_err("peekFromEdge should fail when both recovery levels fail");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("sleep fallback did not settle"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected peekFromEdge execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "moveTo:edgeAnchor:Left:head:1500".to_owned(),
                "moveTo:home".to_owned(),
                "playAction:sleep".to_owned(),
                "moveTo:home".to_owned(),
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["sourceRef"]["kind"] == "conversationMessage")
            .collect::<Vec<_>>();
        assert_eq!(
            original_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );
        let failed_step_id = original_events[7]["stepId"]
            .as_str()
            .expect("failed sleep recovery step id");
        assert!(failed_step_id.ends_with(".recovery.fallback"));

        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
        assert_eq!(
            recovery_events[0]["sourceRef"]["triggeredByStepId"],
            json!(failed_step_id)
        );
        assert!(events
            .iter()
            .all(|event| event["sourceRef"]["kind"] != "macroFallback"));
    }

    #[test]
    fn production_macro_intent_executor_recovers_lie_down_home_failure_with_in_place_sleep() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = HomeMoveFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "lieDown",
            "params": {}
        }))
        .expect("parse lie down macro intent");

        let report = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000628",
                    "messageId": "message_019f4000-0000-7000-8000-000000000629",
                    "runId": "run_019f4000-0000-7000-8000-000000000630",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect("lieDown should recover in place when returning home fails");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["moveTo:home".to_owned(), "playAction:sleep".to_owned()]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == report.plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
        assert_eq!(original_events[2]["payload"]["target"]["kind"], "home");
        assert_eq!(original_events[4]["payload"]["actionId"], "sleep");
        assert_eq!(
            original_events[6]["payload"]["completedStepCount"],
            json!(1)
        );
        assert_eq!(original_events[6]["payload"]["failedStepCount"], json!(1));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
        assert!(events
            .iter()
            .all(|event| event["sourceRef"]["kind"] != "macroFallback"));
    }

    #[test]
    fn production_macro_intent_executor_uses_celebrate_fallback_when_cast_timeline_fails() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = RetryPlayActionStepExecutor::fail_first_attempt();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "cast",
            "params": {}
        }))
        .expect("parse cast macro intent");

        let error = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000631",
                    "messageId": "message_019f4000-0000-7000-8000-000000000632",
                    "runId": "run_019f4000-0000-7000-8000-000000000633",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect_err("cast timeline should preserve the original execution failure");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("transient_motion_error"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected cast execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:cast".to_owned(),
                "playAction:celebrate".to_owned(),
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == "plan_macro_019f4000-0000-7000-8000-000000000701")
            .collect::<Vec<_>>();
        assert_eq!(
            original_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );
        assert_eq!(original_events[2]["payload"]["actionId"], "cast");
        assert_eq!(original_events[2]["payload"]["animationRef"], "cast");

        let macro_fallback_events = events
            .iter()
            .filter(|event| event["sourceRef"]["kind"] == "macroFallback")
            .collect::<Vec<_>>();
        assert_eq!(
            macro_fallback_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            macro_fallback_events[0]["sourceRef"],
            json!({
                "kind": "macroFallback",
                "triggeredByPlanId": "plan_macro_019f4000-0000-7000-8000-000000000701",
                "triggeredByStepId": "step_macro_019f4000-0000-7000-8000-000000000703",
                "triggerReason": "semanticFallback.castActionFailed",
                "originalMacroId": "cast",
                "fallbackMacroId": "celebrate"
            })
        );
        assert_eq!(macro_fallback_events[2]["payload"]["actionId"], "celebrate");
        assert_eq!(
            macro_fallback_events[2]["payload"]["animationRef"],
            "celebrate"
        );
    }

    #[test]
    fn production_macro_intent_executor_routes_dance_validation_failure_to_system_recovery() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = ValidationPlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "dance",
            "params": { "durationMs": 10_000 }
        }))
        .expect("parse dance macro intent");

        let error = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000651",
                    "messageId": "message_019f4000-0000-7000-8000-000000000652",
                    "runId": "run_019f4000-0000-7000-8000-000000000653",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect_err("dance validation failure should still be returned");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("play action validation failed"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected dance execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec!["playAction:celebrate".to_owned(), "moveTo:home".to_owned()]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        assert!(events
            .iter()
            .all(|event| event["sourceRef"]["kind"] != "macroFallback"));

        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
        assert_eq!(
            recovery_events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_macro_019f4000-0000-7000-8000-000000000701",
                "triggeredByStepId": "step_macro_019f4000-0000-7000-8000-000000000703",
                "triggerReason": "executor.error"
            })
        );
    }

    #[test]
    fn production_macro_intent_executor_admits_system_recovery_when_semantic_fallback_fails() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "cast",
            "params": {}
        }))
        .expect("parse cast macro intent");

        let error = execute_macro_intent_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            &intent,
            MacroIntentExecutionRequest {
                context: MacroIntentExecutionContext::fixed_for_test(),
                source_ref: json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_019f4000-0000-7000-8000-000000000641",
                    "messageId": "message_019f4000-0000-7000-8000-000000000642",
                    "runId": "run_019f4000-0000-7000-8000-000000000643",
                }),
                resolve_context: ResolveContext::default(),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect_err("original cast failure should still be returned");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected cast execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:cast".to_owned(),
                "playAction:celebrate".to_owned(),
                "moveTo:home".to_owned(),
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let macro_fallback_events = events
            .iter()
            .filter(|event| event["sourceRef"]["kind"] == "macroFallback")
            .collect::<Vec<_>>();
        assert_eq!(
            macro_fallback_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );
        assert_eq!(
            macro_fallback_events[0]["sourceRef"],
            json!({
                "kind": "macroFallback",
                "triggeredByPlanId": "plan_macro_019f4000-0000-7000-8000-000000000701",
                "triggeredByStepId": "step_macro_019f4000-0000-7000-8000-000000000703",
                "triggerReason": "semanticFallback.castActionFailed",
                "originalMacroId": "cast",
                "fallbackMacroId": "celebrate"
            })
        );
        let macro_fallback_plan_id = macro_fallback_events[0]["planId"]
            .as_str()
            .expect("macro fallback plan id");
        let macro_fallback_step_id = macro_fallback_events[3]["stepId"]
            .as_str()
            .expect("macro fallback failed step id");

        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
        assert_eq!(
            recovery_events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": macro_fallback_plan_id,
                "triggeredByStepId": macro_fallback_step_id,
                "triggerReason": "executor.error"
            })
        );
    }

    #[test]
    fn production_timeline_executor_admits_runtime_safe_fallback_after_failed_step() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000201",
                "messageId": "message_019f4000-0000-7000-8000-000000000301",
                "runId": "run_019f4000-0000-7000-8000-000000000401",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::MoveTo(MoveToStep::center(
                    "step_timeline_019f4000-0000-7000-8000-000000000501",
                    30_000,
                )),
                TimelineStep::PlayAction(PlayActionStep::once(
                    "step_timeline_019f4000-0000-7000-8000-000000000502",
                    "celebrate",
                    5_000,
                )),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let error = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect_err("production timeline should fail on the playAction step");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "moveTo:center".to_owned(),
                "playAction:celebrate".to_owned(),
                "moveTo:home".to_owned(),
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );
        assert_eq!(
            storage.action_log_plan_summary_for_test(&plan_id),
            json!({
                "status": "failed",
                "lastEventType": "plan.failed",
                "lastReasonCode": "timeline.failed",
                "resolvedActionId": "celebrate",
                "resolvedAnimationRef": "celebrate"
            })
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events
                .iter()
                .map(|event| event["reasonCode"].as_str().expect("reason code"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "timeline.started",
                "timeline.stepResolved",
                "timeline.stepCompleted",
                "timeline.stepResolved",
                "timeline.stepFailed",
                "timeline.failed",
            ]
        );
        assert_eq!(
            original_events[1]["sourceRef"],
            json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000201",
                "messageId": "message_019f4000-0000-7000-8000-000000000301",
                "runId": "run_019f4000-0000-7000-8000-000000000401",
            })
        );
        assert_eq!(original_events[1]["triggerSource"], json!("aiChoreography"));

        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
        assert_eq!(
            recovery_events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_timeline_019f4000-0000-7000-8000-000000000101",
                "triggeredByStepId": "step_timeline_019f4000-0000-7000-8000-000000000502",
                "triggerReason": "executor.error"
            })
        );
    }

    #[test]
    fn production_timeline_executor_continues_after_failed_step_when_policy_allows() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_continue_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000221",
                "messageId": "message_019f4000-0000-7000-8000-000000000321",
                "runId": "run_019f4000-0000-7000-8000-000000000421",
            }),
            failure_policy: TimelineFailurePolicy::Continue,
            steps: vec![
                TimelineStep::PlayAction(PlayActionStep::once(
                    "step_timeline_continue_019f4000-0000-7000-8000-000000000501",
                    "celebrate",
                    5_000,
                )),
                TimelineStep::MoveTo(MoveToStep::center(
                    "step_timeline_continue_019f4000-0000-7000-8000-000000000502",
                    30_000,
                )),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("continue failure policy should keep running later steps");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:celebrate".to_owned(),
                "moveTo:center".to_owned()
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events[1]["payload"]["failurePolicy"],
            json!("continue")
        );
        assert_eq!(
            original_events[6]["payload"]["failurePolicy"],
            json!("continue")
        );
        assert_eq!(
            original_events[6]["payload"]["completedStepCount"],
            json!(1)
        );
        assert_eq!(original_events[6]["payload"]["failedStepCount"], json!(1));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
    }

    #[test]
    fn production_timeline_executor_runs_try_fallback_steps_after_primary_failure() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_try_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000231",
                "messageId": "message_019f4000-0000-7000-8000-000000000331",
                "runId": "run_019f4000-0000-7000-8000-000000000431",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::Try(TryStep {
                    step_id: "step_timeline_try_019f4000-0000-7000-8000-000000000501".to_owned(),
                    kind: "try".to_owned(),
                    steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                        "step_timeline_try_019f4000-0000-7000-8000-000000000502",
                        "celebrate",
                        5_000,
                    ))],
                    fallback_steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                        "step_timeline_try_019f4000-0000-7000-8000-000000000503",
                        30_000,
                    ))],
                }),
                TimelineStep::MoveTo(MoveToStep::home(
                    "step_timeline_try_019f4000-0000-7000-8000-000000000504",
                    30_000,
                )),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("try fallback should recover the failed primary branch");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:celebrate".to_owned(),
                "moveTo:center".to_owned(),
                "moveTo:home".to_owned()
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events[8]["payload"]["completedStepCount"],
            json!(2)
        );
        assert_eq!(original_events[8]["payload"]["failedStepCount"], json!(1));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
    }

    #[test]
    fn production_timeline_executor_replaces_failed_step_without_recovery() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_replace_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000234",
                "messageId": "message_019f4000-0000-7000-8000-000000000334",
                "runId": "run_019f4000-0000-7000-8000-000000000434",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::Replace(ReplaceStep {
                    step_id: "step_timeline_replace_019f4000-0000-7000-8000-000000000501"
                        .to_owned(),
                    kind: "replaceStep".to_owned(),
                    steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                        "step_timeline_replace_019f4000-0000-7000-8000-000000000502",
                        "celebrate",
                        5_000,
                    ))],
                    replacement_steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                        "step_timeline_replace_019f4000-0000-7000-8000-000000000503",
                        30_000,
                    ))],
                }),
                TimelineStep::MoveTo(MoveToStep::home(
                    "step_timeline_replace_019f4000-0000-7000-8000-000000000504",
                    30_000,
                )),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("replacement steps should recover the failed original step");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:celebrate".to_owned(),
                "moveTo:center".to_owned(),
                "moveTo:home".to_owned()
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events[8]["payload"]["completedStepCount"],
            json!(2)
        );
        assert_eq!(original_events[8]["payload"]["failedStepCount"], json!(1));
        assert_eq!(original_events[8]["payload"]["skippedStepCount"], json!(0));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
    }

    #[test]
    fn production_timeline_executor_runs_recovery_steps_after_failed_primary_step() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_recover_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000235",
                "messageId": "message_019f4000-0000-7000-8000-000000000335",
                "runId": "run_019f4000-0000-7000-8000-000000000435",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::Recover(RecoverStep {
                    step_id: "step_timeline_recover_019f4000-0000-7000-8000-000000000501"
                        .to_owned(),
                    kind: "recover".to_owned(),
                    steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                        "step_timeline_recover_019f4000-0000-7000-8000-000000000502",
                        "celebrate",
                        5_000,
                    ))],
                    recovery_steps: vec![TimelineStep::MoveTo(MoveToStep::center(
                        "step_timeline_recover_019f4000-0000-7000-8000-000000000503",
                        30_000,
                    ))],
                }),
                TimelineStep::MoveTo(MoveToStep::home(
                    "step_timeline_recover_019f4000-0000-7000-8000-000000000504",
                    30_000,
                )),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("recover should compensate a failed primary step inside the same plan");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:celebrate".to_owned(),
                "moveTo:center".to_owned(),
                "moveTo:home".to_owned()
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events[8]["payload"]["completedStepCount"],
            json!(2)
        );
        assert_eq!(original_events[8]["payload"]["failedStepCount"], json!(1));
        assert_eq!(original_events[8]["payload"]["skippedStepCount"], json!(0));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
    }

    #[test]
    fn production_timeline_executor_triggers_system_recovery_when_recovery_steps_fail() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_recover_failed_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000236",
                "messageId": "message_019f4000-0000-7000-8000-000000000336",
                "runId": "run_019f4000-0000-7000-8000-000000000436",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::Recover(RecoverStep {
                step_id: "step_timeline_recover_failed_019f4000-0000-7000-8000-000000000501"
                    .to_owned(),
                kind: "recover".to_owned(),
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_timeline_recover_failed_019f4000-0000-7000-8000-000000000502",
                    "celebrate",
                    5_000,
                ))],
                recovery_steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_timeline_recover_failed_019f4000-0000-7000-8000-000000000503",
                    "reassure",
                    5_000,
                ))],
            })],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let error = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect_err("recover should fail when recovery steps fail");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:celebrate".to_owned(),
                "playAction:reassure".to_owned(),
                "moveTo:home".to_owned()
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events[6]["payload"]["completedStepCount"],
            json!(0)
        );
        assert_eq!(original_events[6]["payload"]["failedStepCount"], json!(2));
        assert_eq!(original_events[6]["payload"]["durationMs"], json!(0));

        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
    }

    #[test]
    fn production_timeline_executor_retries_steps_until_success_without_recovery() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = RetryPlayActionStepExecutor::fail_first_attempt();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_retry_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000233",
                "messageId": "message_019f4000-0000-7000-8000-000000000333",
                "runId": "run_019f4000-0000-7000-8000-000000000433",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::Retry(RetryStep {
                    step_id: "step_timeline_retry_019f4000-0000-7000-8000-000000000501".to_owned(),
                    kind: "retry".to_owned(),
                    max_attempts: 2,
                    steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                        "step_timeline_retry_019f4000-0000-7000-8000-000000000502",
                        "celebrate",
                        5_000,
                    ))],
                }),
                TimelineStep::MoveTo(MoveToStep::home(
                    "step_timeline_retry_019f4000-0000-7000-8000-000000000503",
                    30_000,
                )),
            ],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let report = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect("retry should recover a transient failed attempt");

        assert!(report.executed);
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:celebrate".to_owned(),
                "playAction:celebrate".to_owned(),
                "moveTo:home".to_owned()
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.completed",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events[8]["payload"]["completedStepCount"],
            json!(2)
        );
        assert_eq!(original_events[8]["payload"]["failedStepCount"], json!(1));
        assert_eq!(original_events[8]["payload"]["skippedStepCount"], json!(0));
        assert!(events
            .iter()
            .all(|event| event["triggerSource"] != "systemRecovery"));
    }

    #[test]
    fn production_timeline_executor_preserves_try_branch_counts_when_fallback_fails() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = PlayActionFailureRecoveryStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_try_failed_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000232",
                "messageId": "message_019f4000-0000-7000-8000-000000000332",
                "runId": "run_019f4000-0000-7000-8000-000000000432",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::Try(TryStep {
                step_id: "step_timeline_try_failed_019f4000-0000-7000-8000-000000000501".to_owned(),
                kind: "try".to_owned(),
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_timeline_try_failed_019f4000-0000-7000-8000-000000000502",
                    "celebrate",
                    5_000,
                ))],
                fallback_steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_timeline_try_failed_019f4000-0000-7000-8000-000000000503",
                    "reassure",
                    5_000,
                ))],
            })],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };
        let plan_id = plan.plan_id.clone();

        let error = execute_timeline_plan_with_admission(
            storage.clone(),
            &executor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect_err("try fallback should fail when fallback branch also fails");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);
        assert_eq!(
            executor.executed_step_kinds.into_inner(),
            vec![
                "playAction:celebrate".to_owned(),
                "playAction:reassure".to_owned(),
                "moveTo:home".to_owned()
            ]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_events = events
            .iter()
            .filter(|event| event["planId"] == plan_id)
            .collect::<Vec<_>>();
        assert_eq!(
            original_events[6]["payload"]["completedStepCount"],
            json!(0)
        );
        assert_eq!(original_events[6]["payload"]["failedStepCount"], json!(2));
        assert_eq!(original_events[6]["payload"]["durationMs"], json!(0));

        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed"
            ]
        );
    }

    #[test]
    fn production_timeline_executor_marks_runtime_degraded_when_system_recovery_fails() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let mut admission = ChoreographyAdmissionState::default();
        let plan = TimelinePlan {
            plan_id: "plan_timeline_degraded_019f4000-0000-7000-8000-000000000101".to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f4000-0000-7000-8000-000000000211",
                "messageId": "message_019f4000-0000-7000-8000-000000000311",
                "runId": "run_019f4000-0000-7000-8000-000000000411",
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                "step_timeline_degraded_019f4000-0000-7000-8000-000000000502",
                "celebrate",
                5_000,
            ))],
            created_at: "2026-07-08T00:00:00.000Z".to_owned(),
        };

        let error = execute_timeline_plan_with_admission(
            storage.clone(),
            &FailingStepExecutor,
            &mut admission,
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::AiChoreography,
        )
        .expect_err("production timeline should fail before degraded recovery");

        match error {
            TimelineExecutionError::Execution(error) => {
                assert!(error.to_string().contains("control_response_timeout"))
            }
            TimelineExecutionError::ActionLog(error) => {
                panic!("expected execution error, got action log error: {error}")
            }
        }
        assert_eq!(admission.active_plan_id(), None);

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let recovery_events = events
            .iter()
            .filter(|event| event["triggerSource"] == "systemRecovery")
            .filter(|event| event["planId"].as_str().is_some())
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_events
                .iter()
                .map(|event| event["eventType"].as_str().expect("event type"))
                .collect::<Vec<_>>(),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "plan.failed"
            ]
        );
        assert_eq!(
            recovery_events[0]["sourceRef"],
            json!({
                "kind": "systemRecovery",
                "triggeredByPlanId": "plan_timeline_degraded_019f4000-0000-7000-8000-000000000101",
                "triggeredByStepId": "step_timeline_degraded_019f4000-0000-7000-8000-000000000502",
                "triggerReason": "executor.error"
            })
        );
        let recovery_plan_id = recovery_events[0]["planId"]
            .as_str()
            .expect("recovery plan id");

        let degraded_events = events
            .iter()
            .filter(|event| event["eventType"] == "runtime.degraded")
            .collect::<Vec<_>>();
        assert_eq!(degraded_events.len(), 1);
        assert_eq!(
            degraded_events[0]["sourceRef"],
            json!({ "kind": "runtime" })
        );
        assert_eq!(degraded_events[0]["triggerSource"], json!("systemRecovery"));
        assert_eq!(degraded_events[0]["status"], json!("degraded"));
        assert_eq!(
            degraded_events[0]["reasonCode"],
            json!("runtime.systemRecoveryFailed")
        );
        assert_eq!(
            degraded_events[0]["payload"]["failedRecoveryPlanId"],
            json!(recovery_plan_id)
        );
        assert_eq!(
            degraded_events[0]["payload"]["triggeredByPlanId"],
            json!("plan_timeline_degraded_019f4000-0000-7000-8000-000000000101")
        );

        let system_events = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("runtime.degraded".to_owned()),
                source_ref_kind: Some("runtime".to_owned()),
                reason_code: Some("runtime.systemRecoveryFailed".to_owned()),
                status: Some("degraded".to_owned()),
                limit: Some(10),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query runtime degraded system event");
        assert_eq!(system_events.items.len(), 1);
    }

    struct CompletedActionLogFixture<'a> {
        plan_id: &'a str,
        step_id: &'a str,
        event_ids: CompletedActionLogFixtureEventIds<'a>,
        timestamps: CompletedActionLogFixtureTimestamps<'a>,
    }

    struct CompletedActionLogFixtureEventIds<'a> {
        plan_started: &'a str,
        step_resolved: &'a str,
        step_completed: &'a str,
        plan_completed: &'a str,
    }

    struct CompletedActionLogFixtureTimestamps<'a> {
        started_at: &'a str,
        resolved_at: &'a str,
        completed_at: &'a str,
    }

    fn append_completed_action_log_fixture(
        storage: &BuddyStorage,
        fixture: CompletedActionLogFixture<'_>,
    ) {
        let sink = ActionLogSink::new(storage.clone());
        let plan = create_single_play_action_dev_fixture_plan(
            fixture.plan_id,
            fixture.step_id,
            fixture.timestamps.started_at,
        );
        let registry = ActionRegistry::load_bundled().expect("load bundled action registry");
        let step = plan
            .only_play_action_step()
            .expect("single play action fixture step");
        let resolution = registry
            .resolve_play_action(&step.action_id, &ResolveContext::default())
            .expect("resolve fixture action");

        for event in [
            ActionLogEvent::plan_started(
                fixture.event_ids.plan_started,
                &plan,
                fixture.timestamps.started_at,
            ),
            ActionLogEvent::step_resolved(
                ActionLogEventIds {
                    event_id: fixture.event_ids.step_resolved,
                    plan_id: &plan.plan_id,
                    step_id: Some(&step.step_id),
                },
                &plan.source_ref,
                &resolution,
                &ResolveContext::default(),
                fixture.timestamps.resolved_at,
            ),
            ActionLogEvent::step_completed(
                ActionLogEventIds {
                    event_id: fixture.event_ids.step_completed,
                    plan_id: &plan.plan_id,
                    step_id: Some(&step.step_id),
                },
                &plan.source_ref,
                &resolution,
                1720,
                fixture.timestamps.completed_at,
            ),
            ActionLogEvent::plan_completed(
                fixture.event_ids.plan_completed,
                &plan,
                1,
                1730,
                fixture.timestamps.completed_at,
            ),
        ] {
            sink.append_event(&event).expect("append action log event");
        }
    }
}
