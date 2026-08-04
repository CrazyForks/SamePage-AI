use serde::{Deserialize, Serialize};

use crate::error::{BuddyError, BuddyResult};

use super::timeline::{
    MoveByPathStep, MoveEdge, MoveTarget, MoveToStep, PlayActionStep, RecoverStep, TimelineStep,
    TryStep, WaitStep, WindowAnchorEdge, WindowAnchorReveal, WindowAnchorSelector,
};

const CELEBRATE_ACTION_ID: &str = "celebrate";
const IDLE_FALLBACK_ACTION_ID: &str = "idle";
const CELEBRATE_STEP_TIMEOUT_MS: u64 = 5_000;
const DANCE_CURRENT_ACTION_ID: &str = "celebrate";
const DANCE_STEP_TIMEOUT_PADDING_MS: u64 = 1_000;
const LIE_DOWN_AFTER_ACTION_ID: &str = "sleep";
const LIE_DOWN_RECOVERY_SLEEP_TIMEOUT_MS: u64 = 5_000;
pub(crate) const MOVE_TO_STEP_TIMEOUT_MS: u64 = 15_000;
const REASSURE_ACTION_ID: &str = "reassure";
const REASSURE_STEP_TIMEOUT_MS: u64 = 5_000;
const SAD_ACTION_ID: &str = "sad";
const SAD_STEP_TIMEOUT_MS: u64 = 5_000;
const THINKING_ACTION_ID: &str = "thinking";
const THINKING_STEP_TIMEOUT_MS: u64 = 5_000;
const WORKING_ACTION_ID: &str = "working";
const WORKING_STEP_TIMEOUT_MS: u64 = 5_000;
const CURIOUS_ACTION_ID: &str = "curious";
const CURIOUS_STEP_TIMEOUT_MS: u64 = 5_000;
const APPROVAL_ACTION_ID: &str = "approval";
const APPROVAL_FALLBACK_ACTION_ID: &str = THINKING_ACTION_ID;
const APPROVAL_STEP_TIMEOUT_MS: u64 = 5_000;
const GET_UP_STEP_TIMEOUT_MS: u64 = 5_000;
const PEEK_FROM_EDGE_AFTER_ACTION_ID: &str = "peek_from_edge";
const PEEK_FROM_EDGE_DURATION_MS: u64 = 1_500;
const CAST_ACTION_ID: &str = "cast";
const CAST_FALLBACK_ACTION_ID: &str = "celebrate";
const CAST_STEP_TIMEOUT_MS: u64 = 5_000;

pub(crate) const PUBLIC_DANCE_DURATION_MS_MIN: u64 = 1_000;
pub(crate) const PUBLIC_DANCE_DURATION_MS_MAX: u64 = 30_000;
pub(crate) const PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MIN: u8 = 1;
pub(crate) const PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MAX: u8 = 4;
pub(crate) const PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MIN: u64 = 500;
pub(crate) const PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MAX: u64 = 15_000;

pub(crate) const PUBLIC_MACRO_INTENT_IDS: &[&str] = &[
    "celebrate",
    "dance",
    "lieDown",
    "patrolAroundScreen",
    "reassure",
    "sad",
    "thinking",
    "working",
    "curious",
    "awaitApproval",
    "getUp",
    "peekFromEdge",
    "peekBehindWindow",
    "cast",
];

fn is_public_macro_intent_id(macro_id: &str) -> bool {
    PUBLIC_MACRO_INTENT_IDS.contains(&macro_id)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "macroId", content = "params", deny_unknown_fields)]
pub(crate) enum MacroIntent {
    #[serde(rename = "celebrate")]
    Celebrate(CelebrateMacroParams),
    #[serde(rename = "dance")]
    Dance(DanceMacroParams),
    #[serde(rename = "lieDown")]
    LieDown(LieDownMacroParams),
    #[serde(rename = "patrolAroundScreen")]
    PatrolAroundScreen(PatrolAroundScreenMacroParams),
    #[serde(rename = "reassure")]
    Reassure(ReassureMacroParams),
    #[serde(rename = "sad")]
    Sad(SadMacroParams),
    #[serde(rename = "thinking")]
    Thinking(ThinkingMacroParams),
    #[serde(rename = "working")]
    Working(WorkingMacroParams),
    #[serde(rename = "curious")]
    Curious(CuriousMacroParams),
    #[serde(rename = "awaitApproval")]
    AwaitApproval(AwaitApprovalMacroParams),
    #[serde(rename = "getUp")]
    GetUp(GetUpMacroParams),
    #[serde(rename = "peekFromEdge")]
    PeekFromEdge(PeekFromEdgeMacroParams),
    #[serde(rename = "peekBehindWindow")]
    PeekBehindWindow(PeekBehindWindowMacroParams),
    #[serde(rename = "cast")]
    Cast(CastMacroParams),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CelebrateMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DanceMacroParams {
    pub(crate) duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LieDownMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PatrolAroundScreenMacroParams {
    pub(crate) loops: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReassureMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SadMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThinkingMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkingMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CuriousMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AwaitApprovalMacroParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GetUpMacroParams {
    pub(crate) side: GetUpSide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PeekFromEdgeMacroParams {
    pub(crate) edge: MoveEdge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PeekBehindWindowMacroParams {
    pub(crate) window_selector: WindowAnchorSelector,
    pub(crate) edge: WindowAnchorEdge,
    pub(crate) reveal: WindowAnchorReveal,
    pub(crate) duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CastMacroParams {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GetUpSide {
    Left,
    Right,
}

impl GetUpSide {
    fn action_id(self) -> &'static str {
        match self {
            Self::Left => "throw_after_drag.get_up.left",
            Self::Right => "throw_after_drag.get_up.right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MacroFallbackPolicy {
    pub(crate) macro_id: &'static str,
    pub(crate) action_fallback_id: Option<&'static str>,
    pub(crate) timeline_failure_fallback: MacroTimelineFailureFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacroTimelineFailureFallback {
    SystemRecovery,
    Semantic(MacroSemanticFallback),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacroSemanticFallback {
    WindowAnchorTargetUnavailableToPeekFromEdge,
    AwaitApprovalActionFailedToThinking,
    CelebrateActionFailedToReassure,
    CastActionFailedToCelebrate,
    CuriousActionFailedToThinking,
    DanceActionFailedToCelebrate,
    GetUpActionFailedToReassure,
    ReassureActionFailedToLieDown,
    SadActionFailedToReassure,
    ThinkingActionFailedToCurious,
    WorkingActionFailedToThinking,
}

impl MacroSemanticFallback {
    pub(crate) fn trigger_reason_code(self) -> &'static str {
        match self {
            Self::WindowAnchorTargetUnavailableToPeekFromEdge => {
                "semanticFallback.windowAnchorTargetUnavailable"
            }
            Self::AwaitApprovalActionFailedToThinking => {
                "semanticFallback.awaitApprovalActionFailed"
            }
            Self::CelebrateActionFailedToReassure => "semanticFallback.celebrateActionFailed",
            Self::CastActionFailedToCelebrate => "semanticFallback.castActionFailed",
            Self::CuriousActionFailedToThinking => "semanticFallback.curiousActionFailed",
            Self::DanceActionFailedToCelebrate => "semanticFallback.danceActionFailed",
            Self::GetUpActionFailedToReassure => "semanticFallback.getUpActionFailed",
            Self::ReassureActionFailedToLieDown => "semanticFallback.reassureActionFailed",
            Self::SadActionFailedToReassure => "semanticFallback.sadActionFailed",
            Self::ThinkingActionFailedToCurious => "semanticFallback.thinkingActionFailed",
            Self::WorkingActionFailedToThinking => "semanticFallback.workingActionFailed",
        }
    }

    pub(crate) fn fallback_macro_id(self) -> &'static str {
        match self {
            Self::WindowAnchorTargetUnavailableToPeekFromEdge => "peekFromEdge",
            Self::AwaitApprovalActionFailedToThinking => "thinking",
            Self::CelebrateActionFailedToReassure => "reassure",
            Self::CastActionFailedToCelebrate => "celebrate",
            Self::CuriousActionFailedToThinking => "thinking",
            Self::DanceActionFailedToCelebrate => "celebrate",
            Self::GetUpActionFailedToReassure => "reassure",
            Self::ReassureActionFailedToLieDown => "lieDown",
            Self::SadActionFailedToReassure => "reassure",
            Self::ThinkingActionFailedToCurious => "curious",
            Self::WorkingActionFailedToThinking => "thinking",
        }
    }
}

pub(crate) struct BeatPlanBuildContext<'a> {
    pub(crate) plan_id: &'a str,
    pub(crate) beat_id: &'a str,
    pub(crate) step_id: &'a str,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) created_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatPlan {
    pub(crate) plan_id: String,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) macro_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recovery: Option<BeatPlanRecovery>,
    pub(crate) beats: Vec<BeatPlanBeat>,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatPlanRecovery {
    pub(crate) step_id: String,
    pub(crate) recovery_steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatPlanBeat {
    pub(crate) beat_id: String,
    pub(crate) kind: BeatKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_action_id: Option<String>,
    #[serde(flatten)]
    pub(crate) body: BeatPlanBeatBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum BeatPlanBeatBody {
    Action {
        action: BeatAction,
    },
    Target {
        target: BeatTarget,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    Path {
        path: BeatPath,
    },
    Wait {
        wait: BeatWait,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    Group {
        group: BeatGroup,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    FailureBranch {
        #[serde(rename = "failureBranch")]
        failure_branch: BeatFailureBranch,
    },
    #[cfg(test)]
    Step {
        step: TimelineStep,
    },
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatGroup {
    pub(crate) beats: Vec<BeatPlanBeat>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl BeatGroup {
    pub(crate) fn new(beats: Vec<BeatPlanBeat>) -> Self {
        Self { beats }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatFailureBranch {
    pub(crate) step_id: String,
    pub(crate) primary_beats: Vec<BeatPlanBeat>,
    pub(crate) fallback_beats: Vec<BeatPlanBeat>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl BeatFailureBranch {
    pub(crate) fn new(
        step_id: impl Into<String>,
        primary_beats: Vec<BeatPlanBeat>,
        fallback_beats: Vec<BeatPlanBeat>,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            primary_beats,
            fallback_beats,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatPath {
    pub(crate) step_id: String,
    pub(crate) path: Vec<MoveTarget>,
    pub(crate) after_action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_after_action_id: Option<String>,
    pub(crate) timeout_ms: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl BeatPath {
    pub(crate) fn move_by_path(
        step_id: impl Into<String>,
        path: Vec<MoveTarget>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            path,
            after_action_id: None,
            fallback_after_action_id: None,
            timeout_ms,
        }
    }

    fn to_move_by_path_step(&self, beat_id: &str) -> BuddyResult<MoveByPathStep> {
        if self.path.is_empty() {
            return Err(BuddyError::Validation(format!(
                "path beat {beat_id} must contain at least one target"
            )));
        }

        let mut step = MoveByPathStep::new(&self.step_id, self.path.clone(), self.timeout_ms);
        step.after_action_id = self.after_action_id.clone();
        step.fallback_after_action_id = self.fallback_after_action_id.clone();
        Ok(step)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatAction {
    pub(crate) step_id: String,
    pub(crate) action_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_action_id: Option<String>,
    pub(crate) expected_playback: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pending_handoff_finalizer_step_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::is_restore_idle"
    )]
    pub(crate) completion_behavior:
        crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior,
    pub(crate) timeout_ms: u64,
}

impl BeatAction {
    pub(crate) fn once(
        step_id: impl Into<String>,
        action_id: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            action_id: action_id.into(),
            fallback_action_id: None,
            expected_playback: "once".to_owned(),
            duration_ms: None,
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms,
        }
    }

    pub(crate) fn loop_for_duration(
        step_id: impl Into<String>,
        action_id: impl Into<String>,
        duration_ms: u64,
        timeout_ms: u64,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            action_id: action_id.into(),
            fallback_action_id: None,
            expected_playback: "loopForDuration".to_owned(),
            duration_ms: Some(duration_ms),
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms,
        }
    }

    fn to_play_action_step(&self) -> PlayActionStep {
        PlayActionStep {
            step_id: self.step_id.clone(),
            kind: "playAction".to_owned(),
            action_id: self.action_id.clone(),
            fallback_action_id: self.fallback_action_id.clone(),
            expected_playback: self.expected_playback.clone(),
            duration_ms: self.duration_ms,
            pending_handoff_finalizer_step_id: self.pending_handoff_finalizer_step_id.clone(),
            completion_behavior: self.completion_behavior,
            timeout_ms: self.timeout_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatTarget {
    pub(crate) step_id: String,
    pub(crate) target: MoveTarget,
    pub(crate) after_action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_after_action_id: Option<String>,
    pub(crate) timeout_ms: u64,
}

impl BeatTarget {
    pub(crate) fn move_to(step_id: impl Into<String>, target: MoveTarget, timeout_ms: u64) -> Self {
        Self {
            step_id: step_id.into(),
            target,
            after_action_id: None,
            fallback_after_action_id: None,
            timeout_ms,
        }
    }

    pub(crate) fn move_to_with_after_action_fallback(
        step_id: impl Into<String>,
        target: MoveTarget,
        after_action_id: impl Into<String>,
        fallback_after_action_id: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        let mut beat_target = Self::move_to(step_id, target, timeout_ms);
        beat_target.after_action_id = Some(after_action_id.into());
        beat_target.fallback_after_action_id = Some(fallback_after_action_id.into());
        beat_target
    }

    fn to_move_to_step(&self) -> MoveToStep {
        let mut step = MoveToStep::target(&self.step_id, self.target.clone(), self.timeout_ms);
        step.after_action_id = self.after_action_id.clone();
        step.fallback_after_action_id = self.fallback_after_action_id.clone();
        step
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BeatWait {
    pub(crate) step_id: String,
    pub(crate) duration_ms: u64,
    pub(crate) timeout_ms: u64,
}

impl BeatWait {
    pub(crate) fn new(step_id: impl Into<String>, duration_ms: u64, timeout_ms: u64) -> Self {
        Self {
            step_id: step_id.into(),
            duration_ms,
            timeout_ms,
        }
    }

    fn to_wait_step(&self) -> WaitStep {
        WaitStep::new(&self.step_id, self.duration_ms, self.timeout_ms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BeatKind {
    Approach,
    Perform,
    Recover,
    Rest,
    Settle,
}

impl MacroIntent {
    pub(crate) fn macro_id(&self) -> &'static str {
        match self {
            Self::Celebrate(_) => "celebrate",
            Self::Dance(_) => "dance",
            Self::LieDown(_) => "lieDown",
            Self::PatrolAroundScreen(_) => "patrolAroundScreen",
            Self::Reassure(_) => "reassure",
            Self::Sad(_) => "sad",
            Self::Thinking(_) => "thinking",
            Self::Working(_) => "working",
            Self::Curious(_) => "curious",
            Self::AwaitApproval(_) => "awaitApproval",
            Self::GetUp(_) => "getUp",
            Self::PeekFromEdge(_) => "peekFromEdge",
            Self::PeekBehindWindow(_) => "peekBehindWindow",
            Self::Cast(_) => "cast",
        }
    }
}

pub(crate) fn is_public_macro_intent_params_valid(intent: &MacroIntent) -> bool {
    if !is_public_macro_intent_id(intent.macro_id()) {
        return false;
    }

    match intent {
        MacroIntent::Celebrate(_)
        | MacroIntent::LieDown(_)
        | MacroIntent::Reassure(_)
        | MacroIntent::Sad(_)
        | MacroIntent::Thinking(_)
        | MacroIntent::Working(_)
        | MacroIntent::Curious(_)
        | MacroIntent::AwaitApproval(_)
        | MacroIntent::GetUp(_)
        | MacroIntent::PeekFromEdge(_)
        | MacroIntent::Cast(_) => true,
        MacroIntent::Dance(params) => (PUBLIC_DANCE_DURATION_MS_MIN..=PUBLIC_DANCE_DURATION_MS_MAX)
            .contains(&params.duration_ms),
        MacroIntent::PatrolAroundScreen(params) => (PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MIN
            ..=PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MAX)
            .contains(&params.loops),
        MacroIntent::PeekBehindWindow(params) => (PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MIN
            ..=PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MAX)
            .contains(&params.duration_ms),
    }
}

pub(crate) fn macro_fallback_policy(intent: &MacroIntent) -> MacroFallbackPolicy {
    match intent {
        MacroIntent::Celebrate(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::CelebrateActionFailedToReassure,
            ),
        },
        MacroIntent::Dance(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::DanceActionFailedToCelebrate,
            ),
        },
        MacroIntent::LieDown(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: None,
            timeline_failure_fallback: MacroTimelineFailureFallback::SystemRecovery,
        },
        MacroIntent::PatrolAroundScreen(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: None,
            timeline_failure_fallback: MacroTimelineFailureFallback::SystemRecovery,
        },
        MacroIntent::Reassure(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::ReassureActionFailedToLieDown,
            ),
        },
        MacroIntent::Sad(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::SadActionFailedToReassure,
            ),
        },
        MacroIntent::Thinking(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::ThinkingActionFailedToCurious,
            ),
        },
        MacroIntent::Working(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::WorkingActionFailedToThinking,
            ),
        },
        MacroIntent::Curious(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::CuriousActionFailedToThinking,
            ),
        },
        MacroIntent::AwaitApproval(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(APPROVAL_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::AwaitApprovalActionFailedToThinking,
            ),
        },
        MacroIntent::GetUp(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(IDLE_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::GetUpActionFailedToReassure,
            ),
        },
        MacroIntent::PeekFromEdge(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: None,
            timeline_failure_fallback: MacroTimelineFailureFallback::SystemRecovery,
        },
        MacroIntent::PeekBehindWindow(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: None,
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::WindowAnchorTargetUnavailableToPeekFromEdge,
            ),
        },
        MacroIntent::Cast(_) => MacroFallbackPolicy {
            macro_id: intent.macro_id(),
            action_fallback_id: Some(CAST_FALLBACK_ACTION_ID),
            timeline_failure_fallback: MacroTimelineFailureFallback::Semantic(
                MacroSemanticFallback::CastActionFailedToCelebrate,
            ),
        },
    }
}

pub(crate) fn compile_macro_intent_to_beat_plan(
    intent: &MacroIntent,
    context: BeatPlanBuildContext<'_>,
) -> BuddyResult<BeatPlan> {
    let fallback_policy = macro_fallback_policy(intent);

    match intent {
        MacroIntent::Celebrate(_) => Ok(compile_direct_action_macro(
            intent,
            fallback_policy,
            context,
            CELEBRATE_ACTION_ID,
            CELEBRATE_STEP_TIMEOUT_MS,
        )),
        MacroIntent::Dance(params) => compile_dance_macro(params, fallback_policy, context),
        MacroIntent::LieDown(_) => Ok(BeatPlan {
            plan_id: context.plan_id.to_owned(),
            source_ref: context.source_ref,
            macro_id: intent.macro_id().to_owned(),
            fallback_action_id: None,
            recovery: Some(in_place_sleep_recovery(context.step_id)),
            beats: vec![BeatPlanBeat {
                beat_id: context.beat_id.to_owned(),
                kind: BeatKind::Rest,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Target {
                    target: BeatTarget::move_to_with_after_action_fallback(
                        context.step_id,
                        MoveTarget::Home,
                        LIE_DOWN_AFTER_ACTION_ID,
                        IDLE_FALLBACK_ACTION_ID,
                        MOVE_TO_STEP_TIMEOUT_MS,
                    ),
                },
            }],
            created_at: context.created_at.to_owned(),
        }),
        MacroIntent::PatrolAroundScreen(params) => {
            compile_patrol_around_screen_macro(params, context)
        }
        MacroIntent::Reassure(_) => Ok(compile_direct_action_macro(
            intent,
            fallback_policy,
            context,
            REASSURE_ACTION_ID,
            REASSURE_STEP_TIMEOUT_MS,
        )),
        MacroIntent::Sad(_) => Ok(compile_direct_action_macro(
            intent,
            fallback_policy,
            context,
            SAD_ACTION_ID,
            SAD_STEP_TIMEOUT_MS,
        )),
        MacroIntent::Thinking(_) => Ok(compile_direct_action_macro(
            intent,
            fallback_policy,
            context,
            THINKING_ACTION_ID,
            THINKING_STEP_TIMEOUT_MS,
        )),
        MacroIntent::Working(_) => Ok(compile_direct_action_macro(
            intent,
            fallback_policy,
            context,
            WORKING_ACTION_ID,
            WORKING_STEP_TIMEOUT_MS,
        )),
        MacroIntent::Curious(_) => Ok(compile_direct_action_macro(
            intent,
            fallback_policy,
            context,
            CURIOUS_ACTION_ID,
            CURIOUS_STEP_TIMEOUT_MS,
        )),
        MacroIntent::AwaitApproval(_) => Ok(compile_direct_action_macro(
            intent,
            fallback_policy,
            context,
            APPROVAL_ACTION_ID,
            APPROVAL_STEP_TIMEOUT_MS,
        )),
        MacroIntent::GetUp(params) => Ok(BeatPlan {
            plan_id: context.plan_id.to_owned(),
            source_ref: context.source_ref,
            macro_id: intent.macro_id().to_owned(),
            fallback_action_id: fallback_policy.action_fallback_id.map(str::to_owned),
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: context.beat_id.to_owned(),
                kind: BeatKind::Recover,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Action {
                    action: BeatAction::once(
                        context.step_id,
                        params.side.action_id(),
                        GET_UP_STEP_TIMEOUT_MS,
                    ),
                },
            }],
            created_at: context.created_at.to_owned(),
        }),
        MacroIntent::PeekFromEdge(params) => compile_peek_from_edge_macro(params, context),
        MacroIntent::PeekBehindWindow(params) => compile_peek_behind_window_macro(params, context),
        MacroIntent::Cast(_) => Ok(BeatPlan {
            plan_id: context.plan_id.to_owned(),
            source_ref: context.source_ref,
            macro_id: intent.macro_id().to_owned(),
            fallback_action_id: fallback_policy.action_fallback_id.map(str::to_owned),
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: context.beat_id.to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Action {
                    action: BeatAction::once(context.step_id, CAST_ACTION_ID, CAST_STEP_TIMEOUT_MS),
                },
            }],
            created_at: context.created_at.to_owned(),
        }),
    }
}

fn compile_direct_action_macro(
    intent: &MacroIntent,
    fallback_policy: MacroFallbackPolicy,
    context: BeatPlanBuildContext<'_>,
    action_id: &'static str,
    timeout_ms: u64,
) -> BeatPlan {
    BeatPlan {
        plan_id: context.plan_id.to_owned(),
        source_ref: context.source_ref,
        macro_id: intent.macro_id().to_owned(),
        fallback_action_id: fallback_policy.action_fallback_id.map(str::to_owned),
        recovery: None,
        beats: vec![BeatPlanBeat {
            beat_id: context.beat_id.to_owned(),
            kind: BeatKind::Perform,
            fallback_action_id: None,
            body: BeatPlanBeatBody::Action {
                action: BeatAction::once(context.step_id, action_id, timeout_ms),
            },
        }],
        created_at: context.created_at.to_owned(),
    }
}

pub(crate) fn compile_beat_plan_to_timeline_steps(
    beat_plan: &BeatPlan,
) -> BuddyResult<Vec<TimelineStep>> {
    let mut used_macro_fallback = false;
    let mut steps = Vec::with_capacity(beat_plan.beats.len());

    for beat in &beat_plan.beats {
        let compiled =
            compile_beat_to_timeline_steps(beat, beat_plan.fallback_action_id.as_deref())?;
        used_macro_fallback |= compiled.used_inherited_fallback;
        steps.extend(compiled.steps);
    }

    if beat_plan.fallback_action_id.is_some() && !used_macro_fallback {
        return Err(BuddyError::Validation(format!(
            "fallbackActionId on beat plan {} must apply to at least one playAction timeline step",
            beat_plan.plan_id
        )));
    }

    if let Some(recovery) = &beat_plan.recovery {
        if steps.is_empty() {
            return Err(BuddyError::Validation(format!(
                "recovery on beat plan {} must wrap at least one timeline step",
                beat_plan.plan_id
            )));
        }
        if recovery.recovery_steps.is_empty() {
            return Err(BuddyError::Validation(format!(
                "recovery on beat plan {} must contain at least one recovery step",
                beat_plan.plan_id
            )));
        }

        return Ok(vec![TimelineStep::Recover(RecoverStep {
            step_id: recovery.step_id.clone(),
            kind: "recover".to_owned(),
            steps,
            recovery_steps: recovery.recovery_steps.clone(),
        })]);
    }

    Ok(steps)
}

struct CompiledBeatTimelineSteps {
    steps: Vec<TimelineStep>,
    used_inherited_fallback: bool,
}

fn compile_beat_to_timeline_steps(
    beat: &BeatPlanBeat,
    inherited_fallback_action_id: Option<&str>,
) -> BuddyResult<CompiledBeatTimelineSteps> {
    if let BeatPlanBeatBody::Group { group } = &beat.body {
        return compile_group_beat_to_timeline_steps(beat, group, inherited_fallback_action_id);
    }
    if let BeatPlanBeatBody::FailureBranch { failure_branch } = &beat.body {
        return compile_failure_branch_beat_to_timeline_steps(
            beat,
            failure_branch,
            inherited_fallback_action_id,
        );
    }

    let mut step = match &beat.body {
        BeatPlanBeatBody::Action { action } => {
            TimelineStep::PlayAction(action.to_play_action_step())
        }
        BeatPlanBeatBody::Target { target } => TimelineStep::MoveTo(target.to_move_to_step()),
        BeatPlanBeatBody::Path { path } => {
            TimelineStep::MoveByPath(path.to_move_by_path_step(&beat.beat_id)?)
        }
        BeatPlanBeatBody::Wait { wait } => TimelineStep::Wait(wait.to_wait_step()),
        BeatPlanBeatBody::Group { .. } => {
            unreachable!("group beats are compiled before scalar beats")
        }
        BeatPlanBeatBody::FailureBranch { .. } => {
            unreachable!("failure branch beats are compiled before scalar beats")
        }
        #[cfg(test)]
        BeatPlanBeatBody::Step { .. } => {
            return Err(BuddyError::Validation(format!(
                "raw timeline step beat {} is not supported; use action, target, path, wait, group, or failureBranch",
                beat.beat_id
            )));
        }
    };
    let mut used_inherited_fallback = false;

    match &mut step {
        TimelineStep::PlayAction(play_action_step) => {
            if play_action_step.fallback_action_id.is_none() {
                if let Some(fallback_action_id) = beat.fallback_action_id.as_deref() {
                    play_action_step.fallback_action_id = Some(fallback_action_id.to_owned());
                } else if let Some(fallback_action_id) = inherited_fallback_action_id {
                    play_action_step.fallback_action_id = Some(fallback_action_id.to_owned());
                    used_inherited_fallback = true;
                }
            }
        }
        _ if beat.fallback_action_id.is_some() => {
            return Err(BuddyError::Validation(format!(
                "fallbackActionId on beat {} can only apply to playAction timeline steps",
                beat.beat_id
            )));
        }
        _ => {}
    }

    Ok(CompiledBeatTimelineSteps {
        steps: vec![step],
        used_inherited_fallback,
    })
}

fn compile_group_beat_to_timeline_steps(
    beat: &BeatPlanBeat,
    group: &BeatGroup,
    inherited_fallback_action_id: Option<&str>,
) -> BuddyResult<CompiledBeatTimelineSteps> {
    if group.beats.is_empty() {
        return Err(BuddyError::Validation(format!(
            "group beat {} must contain at least one nested beat",
            beat.beat_id
        )));
    }

    let group_fallback_action_id = beat.fallback_action_id.as_deref();
    let nested_fallback_action_id = group_fallback_action_id.or(inherited_fallback_action_id);
    let compiled = compile_nested_beats_to_timeline_steps(&group.beats, nested_fallback_action_id)?;

    if group_fallback_action_id.is_some() && !compiled.used_inherited_fallback {
        return Err(BuddyError::Validation(format!(
            "fallbackActionId on group beat {} must apply to at least one nested playAction timeline step",
            beat.beat_id
        )));
    }

    Ok(CompiledBeatTimelineSteps {
        steps: compiled.steps,
        used_inherited_fallback: group_fallback_action_id.is_none()
            && compiled.used_inherited_fallback,
    })
}

fn compile_failure_branch_beat_to_timeline_steps(
    beat: &BeatPlanBeat,
    failure_branch: &BeatFailureBranch,
    inherited_fallback_action_id: Option<&str>,
) -> BuddyResult<CompiledBeatTimelineSteps> {
    if failure_branch.primary_beats.is_empty() {
        return Err(BuddyError::Validation(format!(
            "failure branch beat {} must contain at least one primary nested beat",
            beat.beat_id
        )));
    }
    if failure_branch.fallback_beats.is_empty() {
        return Err(BuddyError::Validation(format!(
            "failure branch beat {} must contain at least one fallback nested beat",
            beat.beat_id
        )));
    }

    let branch_fallback_action_id = beat.fallback_action_id.as_deref();
    let nested_fallback_action_id = branch_fallback_action_id.or(inherited_fallback_action_id);
    let primary = compile_nested_beats_to_timeline_steps(
        &failure_branch.primary_beats,
        nested_fallback_action_id,
    )?;
    let fallback = compile_nested_beats_to_timeline_steps(
        &failure_branch.fallback_beats,
        nested_fallback_action_id,
    )?;
    let used_nested_fallback = primary.used_inherited_fallback || fallback.used_inherited_fallback;

    if branch_fallback_action_id.is_some() && !used_nested_fallback {
        return Err(BuddyError::Validation(format!(
            "fallbackActionId on failure branch beat {} must apply to at least one nested playAction timeline step",
            beat.beat_id
        )));
    }

    Ok(CompiledBeatTimelineSteps {
        steps: vec![TimelineStep::Try(TryStep {
            step_id: failure_branch.step_id.clone(),
            kind: "try".to_owned(),
            steps: primary.steps,
            fallback_steps: fallback.steps,
        })],
        used_inherited_fallback: branch_fallback_action_id.is_none() && used_nested_fallback,
    })
}

fn compile_nested_beats_to_timeline_steps(
    beats: &[BeatPlanBeat],
    inherited_fallback_action_id: Option<&str>,
) -> BuddyResult<CompiledBeatTimelineSteps> {
    let mut steps = Vec::with_capacity(beats.len());
    let mut used_inherited_fallback = false;

    for beat in beats {
        let compiled = compile_beat_to_timeline_steps(beat, inherited_fallback_action_id)?;
        used_inherited_fallback |= compiled.used_inherited_fallback;
        steps.extend(compiled.steps);
    }

    Ok(CompiledBeatTimelineSteps {
        steps,
        used_inherited_fallback,
    })
}

fn compile_peek_behind_window_macro(
    params: &PeekBehindWindowMacroParams,
    context: BeatPlanBuildContext<'_>,
) -> BuddyResult<BeatPlan> {
    if !(PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MIN..=PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MAX)
        .contains(&params.duration_ms)
    {
        return Err(BuddyError::Validation(format!(
            "peekBehindWindow durationMs must be between {PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MIN} and {PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MAX}"
        )));
    }
    let timeout_ms = MOVE_TO_STEP_TIMEOUT_MS
        .checked_add(params.duration_ms)
        .ok_or_else(|| {
            BuddyError::Validation("peekBehindWindow timeoutMs overflowed".to_owned())
        })?;

    Ok(BeatPlan {
        plan_id: context.plan_id.to_owned(),
        source_ref: context.source_ref,
        macro_id: "peekBehindWindow".to_owned(),
        fallback_action_id: None,
        recovery: None,
        beats: vec![BeatPlanBeat {
            beat_id: context.beat_id.to_owned(),
            kind: BeatKind::Approach,
            fallback_action_id: None,
            body: BeatPlanBeatBody::Target {
                target: BeatTarget::move_to(
                    context.step_id,
                    MoveTarget::WindowAnchor {
                        selector: params.window_selector,
                        edge: params.edge,
                        reveal: params.reveal,
                        duration_ms: params.duration_ms,
                    },
                    timeout_ms,
                ),
            },
        }],
        created_at: context.created_at.to_owned(),
    })
}

fn compile_peek_from_edge_macro(
    params: &PeekFromEdgeMacroParams,
    context: BeatPlanBuildContext<'_>,
) -> BuddyResult<BeatPlan> {
    Ok(BeatPlan {
        plan_id: context.plan_id.to_owned(),
        source_ref: context.source_ref,
        macro_id: "peekFromEdge".to_owned(),
        fallback_action_id: None,
        recovery: Some(home_sleep_recovery(context.step_id)),
        beats: vec![BeatPlanBeat {
            beat_id: context.beat_id.to_owned(),
            kind: BeatKind::Approach,
            fallback_action_id: None,
            body: BeatPlanBeatBody::Target {
                target: BeatTarget::move_to_with_after_action_fallback(
                    primary_step_id(context.step_id),
                    MoveTarget::EdgeAnchor {
                        edge: params.edge,
                        reveal: WindowAnchorReveal::Head,
                        duration_ms: PEEK_FROM_EDGE_DURATION_MS,
                    },
                    PEEK_FROM_EDGE_AFTER_ACTION_ID,
                    IDLE_FALLBACK_ACTION_ID,
                    MOVE_TO_STEP_TIMEOUT_MS + PEEK_FROM_EDGE_DURATION_MS,
                ),
            },
        }],
        created_at: context.created_at.to_owned(),
    })
}

fn compile_patrol_around_screen_macro(
    params: &PatrolAroundScreenMacroParams,
    context: BeatPlanBuildContext<'_>,
) -> BuddyResult<BeatPlan> {
    if !(PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MIN..=PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MAX)
        .contains(&params.loops)
    {
        return Err(BuddyError::Validation(format!(
            "patrolAroundScreen loops must be between {PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MIN} and {PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MAX}"
        )));
    }

    let primary_step_id = primary_step_id(context.step_id);
    let mut beats = Vec::with_capacity(params.loops as usize * PATROL_EDGE_ORDER.len());
    for loop_index in 0..params.loops {
        for (edge_index, edge) in PATROL_EDGE_ORDER.iter().copied().enumerate() {
            let index = loop_index as usize * PATROL_EDGE_ORDER.len() + edge_index;
            beats.push(BeatPlanBeat {
                beat_id: indexed_id(context.beat_id, index),
                kind: BeatKind::Approach,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Target {
                    target: BeatTarget::move_to(
                        indexed_id(&primary_step_id, index),
                        MoveTarget::Edge { edge },
                        MOVE_TO_STEP_TIMEOUT_MS,
                    ),
                },
            });
        }
    }

    Ok(BeatPlan {
        plan_id: context.plan_id.to_owned(),
        source_ref: context.source_ref,
        macro_id: "patrolAroundScreen".to_owned(),
        fallback_action_id: None,
        recovery: Some(home_sleep_recovery(context.step_id)),
        beats,
        created_at: context.created_at.to_owned(),
    })
}

fn compile_dance_macro(
    params: &DanceMacroParams,
    fallback_policy: MacroFallbackPolicy,
    context: BeatPlanBuildContext<'_>,
) -> BuddyResult<BeatPlan> {
    if !(PUBLIC_DANCE_DURATION_MS_MIN..=PUBLIC_DANCE_DURATION_MS_MAX).contains(&params.duration_ms)
    {
        return Err(BuddyError::Validation(format!(
            "dance durationMs must be between {PUBLIC_DANCE_DURATION_MS_MIN} and {PUBLIC_DANCE_DURATION_MS_MAX}"
        )));
    }
    let timeout_ms = params
        .duration_ms
        .checked_add(DANCE_STEP_TIMEOUT_PADDING_MS)
        .ok_or_else(|| BuddyError::Validation("dance timeoutMs overflowed".to_owned()))?;

    Ok(BeatPlan {
        plan_id: context.plan_id.to_owned(),
        source_ref: context.source_ref,
        macro_id: "dance".to_owned(),
        fallback_action_id: fallback_policy.action_fallback_id.map(str::to_owned),
        recovery: None,
        beats: vec![BeatPlanBeat {
            beat_id: context.beat_id.to_owned(),
            kind: BeatKind::Perform,
            fallback_action_id: None,
            body: BeatPlanBeatBody::Action {
                action: BeatAction::loop_for_duration(
                    context.step_id,
                    DANCE_CURRENT_ACTION_ID,
                    params.duration_ms,
                    timeout_ms,
                ),
            },
        }],
        created_at: context.created_at.to_owned(),
    })
}

const PATROL_EDGE_ORDER: [MoveEdge; 4] = [
    MoveEdge::Left,
    MoveEdge::Top,
    MoveEdge::Right,
    MoveEdge::Bottom,
];

fn primary_step_id(step_id: &str) -> String {
    format!("{step_id}.primary")
}

fn recovery_step_id(step_id: &str) -> String {
    format!("{step_id}.recovery")
}

fn recovery_primary_step_id(step_id: &str) -> String {
    format!("{}.primary", recovery_step_id(step_id))
}

fn recovery_fallback_step_id(step_id: &str) -> String {
    format!("{}.fallback", recovery_step_id(step_id))
}

fn home_sleep_recovery(step_id: &str) -> BeatPlanRecovery {
    BeatPlanRecovery {
        step_id: step_id.to_owned(),
        recovery_steps: vec![TimelineStep::Recover(RecoverStep {
            step_id: recovery_step_id(step_id),
            kind: "recover".to_owned(),
            steps: vec![TimelineStep::MoveTo(
                MoveToStep::home_with_after_action_fallback(
                    recovery_primary_step_id(step_id),
                    LIE_DOWN_AFTER_ACTION_ID,
                    IDLE_FALLBACK_ACTION_ID,
                    MOVE_TO_STEP_TIMEOUT_MS,
                ),
            )],
            recovery_steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                recovery_fallback_step_id(step_id),
                LIE_DOWN_AFTER_ACTION_ID,
                LIE_DOWN_RECOVERY_SLEEP_TIMEOUT_MS,
            ))],
        })],
    }
}

fn in_place_sleep_recovery(step_id: &str) -> BeatPlanRecovery {
    BeatPlanRecovery {
        step_id: step_id.to_owned(),
        recovery_steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
            recovery_step_id(step_id),
            LIE_DOWN_AFTER_ACTION_ID,
            LIE_DOWN_RECOVERY_SLEEP_TIMEOUT_MS,
        ))],
    }
}

fn indexed_id(base_id: &str, zero_based_index: usize) -> String {
    if zero_based_index == 0 {
        return base_id.to_owned();
    }

    format!("{base_id}.{}", zero_based_index + 1)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        choreography::{affective::ResolveContext, registry::ActionRegistry},
        error::BuddyResult,
    };

    use super::{
        compile_beat_plan_to_timeline_steps, compile_macro_intent_to_beat_plan,
        is_public_macro_intent_id, macro_fallback_policy, BeatAction, BeatFailureBranch, BeatGroup,
        BeatKind, BeatPath, BeatPlan, BeatPlanBeat, BeatPlanBeatBody, BeatPlanBuildContext,
        BeatTarget, BeatWait, MacroIntent, MacroSemanticFallback, MacroTimelineFailureFallback,
    };
    use crate::choreography::timeline::{MoveEdge, MoveTarget, TimelineStep, WaitStep};

    fn fixed_context<'a>() -> BeatPlanBuildContext<'a> {
        BeatPlanBuildContext {
            plan_id: "plan_019f5100-0000-7000-8000-000000000001",
            beat_id: "beat_019f5100-0000-7000-8000-000000000002",
            step_id: "step_019f5100-0000-7000-8000-000000000003",
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "ai-macro-demo"
            }),
            created_at: "2026-07-09T12:00:00.000Z",
        }
    }

    fn beat_action_with_fallback(
        step_id: &str,
        action_id: &str,
        fallback_action_id: &str,
    ) -> BeatAction {
        let mut action = BeatAction::once(step_id, action_id, 5_000);
        action.fallback_action_id = Some(fallback_action_id.to_owned());
        action
    }

    #[test]
    fn public_macro_intent_id_gate_uses_exact_registered_ids() {
        assert!(is_public_macro_intent_id("dance"));
        assert!(!is_public_macro_intent_id("Dance"));
        assert!(!is_public_macro_intent_id("dance "));
        assert!(!is_public_macro_intent_id("beatPlan"));
    }

    #[test]
    fn macro_fallback_policy_covers_every_macro_intent() -> BuddyResult<()> {
        let cases = [
            (
                json!({ "macroId": "celebrate", "params": {} }),
                "celebrate",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::CelebrateActionFailedToReassure,
                ),
            ),
            (
                json!({ "macroId": "dance", "params": { "durationMs": 1000 } }),
                "dance",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::DanceActionFailedToCelebrate,
                ),
            ),
            (
                json!({ "macroId": "lieDown", "params": {} }),
                "lieDown",
                None,
                MacroTimelineFailureFallback::SystemRecovery,
            ),
            (
                json!({ "macroId": "patrolAroundScreen", "params": { "loops": 1 } }),
                "patrolAroundScreen",
                None,
                MacroTimelineFailureFallback::SystemRecovery,
            ),
            (
                json!({ "macroId": "reassure", "params": {} }),
                "reassure",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::ReassureActionFailedToLieDown,
                ),
            ),
            (
                json!({ "macroId": "sad", "params": {} }),
                "sad",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::SadActionFailedToReassure,
                ),
            ),
            (
                json!({ "macroId": "thinking", "params": {} }),
                "thinking",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::ThinkingActionFailedToCurious,
                ),
            ),
            (
                json!({ "macroId": "working", "params": {} }),
                "working",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::WorkingActionFailedToThinking,
                ),
            ),
            (
                json!({ "macroId": "curious", "params": {} }),
                "curious",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::CuriousActionFailedToThinking,
                ),
            ),
            (
                json!({ "macroId": "awaitApproval", "params": {} }),
                "awaitApproval",
                Some("thinking"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::AwaitApprovalActionFailedToThinking,
                ),
            ),
            (
                json!({ "macroId": "getUp", "params": { "side": "left" } }),
                "getUp",
                Some("idle"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::GetUpActionFailedToReassure,
                ),
            ),
            (
                json!({ "macroId": "peekFromEdge", "params": { "edge": "left" } }),
                "peekFromEdge",
                None,
                MacroTimelineFailureFallback::SystemRecovery,
            ),
            (
                json!({
                    "macroId": "peekBehindWindow",
                    "params": {
                        "windowSelector": { "kind": "activeWindow" },
                        "edge": "left",
                        "reveal": "head",
                        "durationMs": 1500
                    }
                }),
                "peekBehindWindow",
                None,
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::WindowAnchorTargetUnavailableToPeekFromEdge,
                ),
            ),
            (
                json!({ "macroId": "cast", "params": {} }),
                "cast",
                Some("celebrate"),
                MacroTimelineFailureFallback::Semantic(
                    MacroSemanticFallback::CastActionFailedToCelebrate,
                ),
            ),
        ];

        for (intent_json, macro_id, action_fallback_id, timeline_failure_fallback) in cases {
            let intent = serde_json::from_value::<MacroIntent>(intent_json)?;
            let policy = macro_fallback_policy(&intent);

            assert_eq!(policy.macro_id, macro_id);
            assert_eq!(policy.action_fallback_id, action_fallback_id);
            assert_eq!(policy.timeline_failure_fallback, timeline_failure_fallback);
        }

        Ok(())
    }

    #[test]
    fn macro_semantic_fallbacks_have_stable_trigger_reason_codes() {
        let cases = [
            (
                MacroSemanticFallback::WindowAnchorTargetUnavailableToPeekFromEdge,
                "semanticFallback.windowAnchorTargetUnavailable",
                "peekFromEdge",
            ),
            (
                MacroSemanticFallback::AwaitApprovalActionFailedToThinking,
                "semanticFallback.awaitApprovalActionFailed",
                "thinking",
            ),
            (
                MacroSemanticFallback::CelebrateActionFailedToReassure,
                "semanticFallback.celebrateActionFailed",
                "reassure",
            ),
            (
                MacroSemanticFallback::CastActionFailedToCelebrate,
                "semanticFallback.castActionFailed",
                "celebrate",
            ),
            (
                MacroSemanticFallback::CuriousActionFailedToThinking,
                "semanticFallback.curiousActionFailed",
                "thinking",
            ),
            (
                MacroSemanticFallback::DanceActionFailedToCelebrate,
                "semanticFallback.danceActionFailed",
                "celebrate",
            ),
            (
                MacroSemanticFallback::GetUpActionFailedToReassure,
                "semanticFallback.getUpActionFailed",
                "reassure",
            ),
            (
                MacroSemanticFallback::ReassureActionFailedToLieDown,
                "semanticFallback.reassureActionFailed",
                "lieDown",
            ),
            (
                MacroSemanticFallback::SadActionFailedToReassure,
                "semanticFallback.sadActionFailed",
                "reassure",
            ),
            (
                MacroSemanticFallback::ThinkingActionFailedToCurious,
                "semanticFallback.thinkingActionFailed",
                "curious",
            ),
            (
                MacroSemanticFallback::WorkingActionFailedToThinking,
                "semanticFallback.workingActionFailed",
                "thinking",
            ),
        ];

        for (semantic_fallback, trigger_reason_code, fallback_macro_id) in cases {
            assert_eq!(semantic_fallback.trigger_reason_code(), trigger_reason_code);
            assert_eq!(semantic_fallback.fallback_macro_id(), fallback_macro_id);
        }
    }

    #[test]
    fn macro_intent_rejects_unknown_top_level_fields() {
        let result = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "dance",
            "params": { "durationMs": 2500 },
            "debugTimeline": true
        }));
        let Err(error) = result else {
            panic!("macro intent should reject fields outside the public schema");
        };
        let error_message = error.to_string();

        assert!(
            error_message.contains("macroId") && error_message.contains("params"),
            "unexpected error: {error_message}"
        );
    }

    #[test]
    fn macro_intent_get_up_compiles_to_directional_recovery_action() -> BuddyResult<()> {
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "getUp",
            "params": { "side": "right" }
        }))?;

        let beat_plan = compile_macro_intent_to_beat_plan(&intent, fixed_context())?;
        let steps = compile_beat_plan_to_timeline_steps(&beat_plan)?;

        assert_eq!(
            serde_json::to_value(&steps)?,
            json!([
                {
                    "stepId": "step_019f5100-0000-7000-8000-000000000003",
                    "kind": "playAction",
                    "actionId": "throw_after_drag.get_up.right",
                    "fallbackActionId": "idle",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn play_action_macros_compile_with_stable_fallback_actions() -> BuddyResult<()> {
        let registry = ActionRegistry::load_bundled()?;
        let macro_expectations = [
            (
                json!({ "macroId": "celebrate", "params": {} }),
                "celebrate",
                "idle",
                "celebrate",
            ),
            (
                json!({ "macroId": "dance", "params": { "durationMs": 1000 } }),
                "celebrate",
                "idle",
                "celebrate",
            ),
            (
                json!({ "macroId": "reassure", "params": {} }),
                "reassure",
                "idle",
                "reassure",
            ),
            (
                json!({ "macroId": "sad", "params": {} }),
                "sad",
                "idle",
                "sad",
            ),
            (
                json!({ "macroId": "thinking", "params": {} }),
                "thinking",
                "idle",
                "thinking",
            ),
            (
                json!({ "macroId": "working", "params": {} }),
                "working",
                "idle",
                "working",
            ),
            (
                json!({ "macroId": "curious", "params": {} }),
                "curious",
                "idle",
                "curious",
            ),
            (
                json!({ "macroId": "awaitApproval", "params": {} }),
                "approval",
                "thinking",
                "approval",
            ),
            (
                json!({ "macroId": "getUp", "params": { "side": "left" } }),
                "throw_after_drag.get_up.left",
                "idle",
                "fallen_get_up_left",
            ),
            (
                json!({ "macroId": "cast", "params": {} }),
                "cast",
                "celebrate",
                "cast",
            ),
        ];

        for (intent_json, action_id, fallback_action_id, animation_ref) in macro_expectations {
            let intent = serde_json::from_value::<MacroIntent>(intent_json)?;
            let beat_plan = compile_macro_intent_to_beat_plan(&intent, fixed_context())?;
            let steps = compile_beat_plan_to_timeline_steps(&beat_plan)?;
            let [TimelineStep::PlayAction(step)] = steps.as_slice() else {
                panic!(
                    "expected single playAction step for macro {}",
                    beat_plan.macro_id
                );
            };

            assert_eq!(step.action_id, action_id);
            assert_eq!(
                step.fallback_action_id.as_deref(),
                Some(fallback_action_id),
                "unexpected fallback for macro {}",
                beat_plan.macro_id
            );
            assert_eq!(
                registry
                    .resolve_play_action(action_id, &ResolveContext::default())?
                    .animation_ref,
                animation_ref,
                "unexpected animation for macro {}",
                beat_plan.macro_id
            );
        }

        Ok(())
    }

    #[test]
    fn macro_intent_patrol_around_screen_compiles_with_plan_internal_recovery() -> BuddyResult<()> {
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "patrolAroundScreen",
            "params": { "loops": 1 }
        }))?;

        let beat_plan = compile_macro_intent_to_beat_plan(&intent, fixed_context())?;

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5100-0000-7000-8000-000000000003",
                    "kind": "recover",
                    "steps": [
                        {
                            "stepId": "step_019f5100-0000-7000-8000-000000000003.primary",
                            "kind": "moveTo",
                            "target": {
                                "kind": "edge",
                                "edge": "left"
                            },
                            "afterActionId": null,
                            "timeoutMs": 15000
                        },
                        {
                            "stepId": "step_019f5100-0000-7000-8000-000000000003.primary.2",
                            "kind": "moveTo",
                            "target": {
                                "kind": "edge",
                                "edge": "top"
                            },
                            "afterActionId": null,
                            "timeoutMs": 15000
                        },
                        {
                            "stepId": "step_019f5100-0000-7000-8000-000000000003.primary.3",
                            "kind": "moveTo",
                            "target": {
                                "kind": "edge",
                                "edge": "right"
                            },
                            "afterActionId": null,
                            "timeoutMs": 15000
                        },
                        {
                            "stepId": "step_019f5100-0000-7000-8000-000000000003.primary.4",
                            "kind": "moveTo",
                            "target": {
                                "kind": "edge",
                                "edge": "bottom"
                            },
                            "afterActionId": null,
                            "timeoutMs": 15000
                        }
                    ],
                    "recoverySteps": [
                        {
                            "stepId": "step_019f5100-0000-7000-8000-000000000003.recovery",
                            "kind": "recover",
                            "steps": [
                                {
                                    "stepId": "step_019f5100-0000-7000-8000-000000000003.recovery.primary",
                                    "kind": "moveTo",
                                    "target": {
                                        "kind": "home"
                                    },
                                    "afterActionId": "sleep",
                                    "fallbackAfterActionId": "idle",
                                    "timeoutMs": 15000
                                }
                            ],
                            "recoverySteps": [
                                {
                                    "stepId": "step_019f5100-0000-7000-8000-000000000003.recovery.fallback",
                                    "kind": "playAction",
                                    "actionId": "sleep",
                                    "expectedPlayback": "once",
                                    "timeoutMs": 5000
                                }
                            ]
                        }
                    ]
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn macro_intent_peek_from_edge_compiles_with_plan_internal_recovery() -> BuddyResult<()> {
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekFromEdge",
            "params": { "edge": "left" }
        }))?;

        let beat_plan = compile_macro_intent_to_beat_plan(&intent, fixed_context())?;
        let steps = compile_beat_plan_to_timeline_steps(&beat_plan)?;

        assert_eq!(
            serde_json::to_value(&steps)?,
            json!([
                {
                    "stepId": "step_019f5100-0000-7000-8000-000000000003",
                    "kind": "recover",
                    "steps": [
                        {
                            "stepId": "step_019f5100-0000-7000-8000-000000000003.primary",
                            "kind": "moveTo",
                            "target": {
                                "kind": "edgeAnchor",
                                "edge": "left",
                                "reveal": "head",
                                "durationMs": 1500
                            },
                            "afterActionId": "peek_from_edge",
                            "fallbackAfterActionId": "idle",
                            "timeoutMs": 16500
                        }
                    ],
                    "recoverySteps": [
                        {
                            "stepId": "step_019f5100-0000-7000-8000-000000000003.recovery",
                            "kind": "recover",
                            "steps": [
                                {
                                    "stepId": "step_019f5100-0000-7000-8000-000000000003.recovery.primary",
                                    "kind": "moveTo",
                                    "target": {
                                        "kind": "home"
                                    },
                                    "afterActionId": "sleep",
                                    "fallbackAfterActionId": "idle",
                                    "timeoutMs": 15000
                                }
                            ],
                            "recoverySteps": [
                                {
                                    "stepId": "step_019f5100-0000-7000-8000-000000000003.recovery.fallback",
                                    "kind": "playAction",
                                    "actionId": "sleep",
                                    "expectedPlayback": "once",
                                    "timeoutMs": 5000
                                }
                            ]
                        }
                    ]
                }
            ])
        );
        assert_eq!(beat_plan.beats[0].kind, BeatKind::Approach);
        Ok(())
    }

    #[test]
    fn vertical_edge_peek_uses_the_generic_proxy() -> BuddyResult<()> {
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekFromEdge",
            "params": { "edge": "top" }
        }))?;

        let beat_plan = compile_macro_intent_to_beat_plan(&intent, fixed_context())?;
        let timeline = serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?;
        let primary_steps = timeline[0]["steps"]
            .as_array()
            .expect("peek recovery primary steps");

        assert_eq!(primary_steps.len(), 1);
        assert_eq!(primary_steps[0]["target"]["edge"], json!("top"));
        assert_eq!(primary_steps[0]["target"]["durationMs"], json!(1500));
        assert_eq!(primary_steps[0]["afterActionId"], json!("peek_from_edge"));
        Ok(())
    }

    #[test]
    fn macro_intent_peek_behind_window_keeps_semantic_fallback_unwrapped() -> BuddyResult<()> {
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekBehindWindow",
            "params": {
                "windowSelector": { "kind": "activeWindow" },
                "edge": "right",
                "reveal": "head",
                "durationMs": 1500
            }
        }))?;

        let beat_plan = compile_macro_intent_to_beat_plan(&intent, fixed_context())?;
        let steps = compile_beat_plan_to_timeline_steps(&beat_plan)?;

        assert_eq!(
            serde_json::to_value(&steps)?,
            json!([
                {
                    "stepId": "step_019f5100-0000-7000-8000-000000000003",
                    "kind": "moveTo",
                    "target": {
                        "kind": "windowAnchor",
                        "selector": { "kind": "activeWindow" },
                        "edge": "right",
                        "reveal": "head",
                        "durationMs": 1500
                    },
                    "afterActionId": null,
                    "timeoutMs": 16500
                }
            ])
        );
        assert!(beat_plan.recovery.is_none());
        Ok(())
    }

    #[test]
    fn macro_intent_peek_behind_window_accepts_auto_edge() -> BuddyResult<()> {
        let intent = serde_json::from_value::<MacroIntent>(json!({
            "macroId": "peekBehindWindow",
            "params": {
                "windowSelector": { "kind": "activeWindow" },
                "edge": "auto",
                "reveal": "head",
                "durationMs": 1500
            }
        }))?;

        let beat_plan = compile_macro_intent_to_beat_plan(&intent, fixed_context())?;
        let steps = compile_beat_plan_to_timeline_steps(&beat_plan)?;

        assert_eq!(
            serde_json::to_value(&steps)?,
            json!([
                {
                    "stepId": "step_019f5100-0000-7000-8000-000000000003",
                    "kind": "moveTo",
                    "target": {
                        "kind": "windowAnchor",
                        "selector": { "kind": "activeWindow" },
                        "edge": "auto",
                        "reveal": "head",
                        "durationMs": 1500
                    },
                    "afterActionId": null,
                    "timeoutMs": 16500
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_applies_action_beat_macro_fallback_priority(
    ) -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000001".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "fallback-priority"
            }),
            macro_id: "fallbackPriority".to_owned(),
            fallback_action_id: Some("macro.fallback".to_owned()),
            recovery: None,
            beats: vec![
                BeatPlanBeat {
                    beat_id: "beat_019f5200-0000-7000-8000-000000000101".to_owned(),
                    kind: BeatKind::Perform,
                    fallback_action_id: None,
                    body: BeatPlanBeatBody::Action {
                        action: BeatAction::once(
                            "step_019f5200-0000-7000-8000-000000000101",
                            "primary.macro",
                            5_000,
                        ),
                    },
                },
                BeatPlanBeat {
                    beat_id: "beat_019f5200-0000-7000-8000-000000000102".to_owned(),
                    kind: BeatKind::Perform,
                    fallback_action_id: Some("beat.fallback".to_owned()),
                    body: BeatPlanBeatBody::Action {
                        action: BeatAction::once(
                            "step_019f5200-0000-7000-8000-000000000102",
                            "primary.beat",
                            5_000,
                        ),
                    },
                },
                BeatPlanBeat {
                    beat_id: "beat_019f5200-0000-7000-8000-000000000103".to_owned(),
                    kind: BeatKind::Perform,
                    fallback_action_id: Some("ignored.beat.fallback".to_owned()),
                    body: BeatPlanBeatBody::Action {
                        action: beat_action_with_fallback(
                            "step_019f5200-0000-7000-8000-000000000103",
                            "primary.action",
                            "action.fallback",
                        ),
                    },
                },
            ],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000101",
                    "kind": "playAction",
                    "actionId": "primary.macro",
                    "fallbackActionId": "macro.fallback",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                },
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000102",
                    "kind": "playAction",
                    "actionId": "primary.beat",
                    "fallbackActionId": "beat.fallback",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                },
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000103",
                    "kind": "playAction",
                    "actionId": "primary.action",
                    "fallbackActionId": "action.fallback",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_preserves_hold_and_settle_wait_steps() -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000201".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "wait-beats"
            }),
            macro_id: "waitBeats".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![
                BeatPlanBeat {
                    beat_id: "beat_019f5200-0000-7000-8000-000000000201".to_owned(),
                    kind: BeatKind::Perform,
                    fallback_action_id: None,
                    body: BeatPlanBeatBody::Wait {
                        wait: BeatWait::new(
                            "step_019f5200-0000-7000-8000-000000000201",
                            750,
                            1_000,
                        ),
                    },
                },
                BeatPlanBeat {
                    beat_id: "beat_019f5200-0000-7000-8000-000000000202".to_owned(),
                    kind: BeatKind::Settle,
                    fallback_action_id: None,
                    body: BeatPlanBeatBody::Wait {
                        wait: BeatWait::new(
                            "step_019f5200-0000-7000-8000-000000000202",
                            250,
                            1_000,
                        ),
                    },
                },
            ],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000201",
                    "kind": "wait",
                    "durationMs": 750,
                    "timeoutMs": 1000
                },
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000202",
                    "kind": "wait",
                    "durationMs": 250,
                    "timeoutMs": 1000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_beat_fallback_on_wait_beat() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000211".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "invalid-wait-fallback"
            }),
            macro_id: "invalidWaitFallback".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000211".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: Some("idle".to_owned()),
                body: BeatPlanBeatBody::Wait {
                    wait: BeatWait::new("step_019f5200-0000-7000-8000-000000000211", 750, 1_000),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("wait beat cannot consume beat-level fallbackActionId");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: fallbackActionId on beat beat_019f5200-0000-7000-8000-000000000211 can only apply to playAction timeline steps"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_beat_fallback_on_target_beat() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000212".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "invalid-target-fallback"
            }),
            macro_id: "invalidTargetFallback".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000212".to_owned(),
                kind: BeatKind::Approach,
                fallback_action_id: Some("idle".to_owned()),
                body: BeatPlanBeatBody::Target {
                    target: BeatTarget::move_to(
                        "step_019f5200-0000-7000-8000-000000000212",
                        MoveTarget::Edge {
                            edge: MoveEdge::Left,
                        },
                        15_000,
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("target beat cannot consume beat-level fallbackActionId");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: fallbackActionId on beat beat_019f5200-0000-7000-8000-000000000212 can only apply to playAction timeline steps"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_unused_macro_fallback_on_wait_only_plan() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000213".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "invalid-macro-fallback"
            }),
            macro_id: "invalidMacroFallback".to_owned(),
            fallback_action_id: Some("idle".to_owned()),
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000213".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Wait {
                    wait: BeatWait::new("step_019f5200-0000-7000-8000-000000000213", 750, 1_000),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("wait-only beat plan cannot consume plan-level fallbackActionId");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: fallbackActionId on beat plan plan_019f5200-0000-7000-8000-000000000213 must apply to at least one playAction timeline step"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_raw_step_escape_hatch() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000701".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "raw-step"
            }),
            macro_id: "rawStep".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000701".to_owned(),
                kind: BeatKind::Settle,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Step {
                    step: TimelineStep::Wait(WaitStep::new(
                        "step_019f5200-0000-7000-8000-000000000701",
                        125,
                        500,
                    )),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("raw timeline step beat must not compile");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: raw timeline step beat beat_019f5200-0000-7000-8000-000000000701 is not supported; use action, target, path, wait, group, or failureBranch"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_compiles_group_beat_to_multiple_steps() -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000901".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-group"
            }),
            macro_id: "groupBeat".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000901".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Group {
                    group: BeatGroup::new(vec![
                        BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000000902".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Action {
                                action: BeatAction::once(
                                    "step_019f5200-0000-7000-8000-000000000901",
                                    "celebrate",
                                    5_000,
                                ),
                            },
                        },
                        BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000000903".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Wait {
                                wait: BeatWait::new(
                                    "step_019f5200-0000-7000-8000-000000000902",
                                    250,
                                    1_000,
                                ),
                            },
                        },
                    ]),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000901",
                    "kind": "playAction",
                    "actionId": "celebrate",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                },
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000902",
                    "kind": "wait",
                    "durationMs": 250,
                    "timeoutMs": 1000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_applies_group_fallback_to_nested_play_action(
    ) -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000904".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-group-fallback"
            }),
            macro_id: "groupFallback".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000904".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: Some("group.fallback".to_owned()),
                body: BeatPlanBeatBody::Group {
                    group: BeatGroup::new(vec![
                        BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000000905".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Action {
                                action: BeatAction::once(
                                    "step_019f5200-0000-7000-8000-000000000904",
                                    "celebrate",
                                    5_000,
                                ),
                            },
                        },
                        BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000000906".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Wait {
                                wait: BeatWait::new(
                                    "step_019f5200-0000-7000-8000-000000000905",
                                    250,
                                    1_000,
                                ),
                            },
                        },
                    ]),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000904",
                    "kind": "playAction",
                    "actionId": "celebrate",
                    "fallbackActionId": "group.fallback",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                },
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000905",
                    "kind": "wait",
                    "durationMs": 250,
                    "timeoutMs": 1000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_empty_group_beat() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000907".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "empty-group-beat"
            }),
            macro_id: "emptyGroupBeat".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000907".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Group {
                    group: BeatGroup::new(Vec::new()),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("group beat must contain at least one nested beat");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: group beat beat_019f5200-0000-7000-8000-000000000907 must contain at least one nested beat"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_group_fallback_without_nested_play_action() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000908".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "invalid-group-fallback"
            }),
            macro_id: "invalidGroupFallback".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000908".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: Some("idle".to_owned()),
                body: BeatPlanBeatBody::Group {
                    group: BeatGroup::new(vec![BeatPlanBeat {
                        beat_id: "beat_019f5200-0000-7000-8000-000000000909".to_owned(),
                        kind: BeatKind::Perform,
                        fallback_action_id: None,
                        body: BeatPlanBeatBody::Wait {
                            wait: BeatWait::new(
                                "step_019f5200-0000-7000-8000-000000000908",
                                250,
                                1_000,
                            ),
                        },
                    }]),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("group fallbackActionId must be consumed by nested playAction");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: fallbackActionId on group beat beat_019f5200-0000-7000-8000-000000000908 must apply to at least one nested playAction timeline step"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_compiles_failure_branch_to_try_step() -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000001001".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-failure-branch"
            }),
            macro_id: "failureBranchBeat".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000001001".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::FailureBranch {
                    failure_branch: BeatFailureBranch::new(
                        "step_019f5200-0000-7000-8000-000000001001",
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001002".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Action {
                                action: BeatAction::once(
                                    "step_019f5200-0000-7000-8000-000000001002",
                                    "cast",
                                    5_000,
                                ),
                            },
                        }],
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001003".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Action {
                                action: BeatAction::once(
                                    "step_019f5200-0000-7000-8000-000000001003",
                                    "celebrate",
                                    5_000,
                                ),
                            },
                        }],
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000001001",
                    "kind": "try",
                    "steps": [
                        {
                            "stepId": "step_019f5200-0000-7000-8000-000000001002",
                            "kind": "playAction",
                            "actionId": "cast",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ],
                    "fallbackSteps": [
                        {
                            "stepId": "step_019f5200-0000-7000-8000-000000001003",
                            "kind": "playAction",
                            "actionId": "celebrate",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ]
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_applies_failure_branch_fallback_to_nested_play_actions(
    ) -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000001004".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-failure-branch-fallback"
            }),
            macro_id: "failureBranchFallback".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000001004".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: Some("branch.fallback".to_owned()),
                body: BeatPlanBeatBody::FailureBranch {
                    failure_branch: BeatFailureBranch::new(
                        "step_019f5200-0000-7000-8000-000000001004",
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001005".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Action {
                                action: BeatAction::once(
                                    "step_019f5200-0000-7000-8000-000000001005",
                                    "cast",
                                    5_000,
                                ),
                            },
                        }],
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001006".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Action {
                                action: BeatAction::once(
                                    "step_019f5200-0000-7000-8000-000000001006",
                                    "celebrate",
                                    5_000,
                                ),
                            },
                        }],
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000001004",
                    "kind": "try",
                    "steps": [
                        {
                            "stepId": "step_019f5200-0000-7000-8000-000000001005",
                            "kind": "playAction",
                            "actionId": "cast",
                            "fallbackActionId": "branch.fallback",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ],
                    "fallbackSteps": [
                        {
                            "stepId": "step_019f5200-0000-7000-8000-000000001006",
                            "kind": "playAction",
                            "actionId": "celebrate",
                            "fallbackActionId": "branch.fallback",
                            "expectedPlayback": "once",
                            "timeoutMs": 5000
                        }
                    ]
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_empty_failure_branch_primary() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000001007".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "empty-failure-branch-primary"
            }),
            macro_id: "emptyFailureBranchPrimary".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000001007".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::FailureBranch {
                    failure_branch: BeatFailureBranch::new(
                        "step_019f5200-0000-7000-8000-000000001007",
                        Vec::new(),
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001008".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Wait {
                                wait: BeatWait::new(
                                    "step_019f5200-0000-7000-8000-000000001008",
                                    250,
                                    1_000,
                                ),
                            },
                        }],
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("failure branch primary beats must be non-empty");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: failure branch beat beat_019f5200-0000-7000-8000-000000001007 must contain at least one primary nested beat"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_empty_failure_branch_fallback() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000001009".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "empty-failure-branch-fallback"
            }),
            macro_id: "emptyFailureBranchFallback".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000001009".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::FailureBranch {
                    failure_branch: BeatFailureBranch::new(
                        "step_019f5200-0000-7000-8000-000000001009",
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001010".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Wait {
                                wait: BeatWait::new(
                                    "step_019f5200-0000-7000-8000-000000001010",
                                    250,
                                    1_000,
                                ),
                            },
                        }],
                        Vec::new(),
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("failure branch fallback beats must be non-empty");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: failure branch beat beat_019f5200-0000-7000-8000-000000001009 must contain at least one fallback nested beat"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_failure_branch_fallback_without_nested_play_action() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000001011".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "invalid-failure-branch-fallback"
            }),
            macro_id: "invalidFailureBranchFallback".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000001011".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: Some("idle".to_owned()),
                body: BeatPlanBeatBody::FailureBranch {
                    failure_branch: BeatFailureBranch::new(
                        "step_019f5200-0000-7000-8000-000000001011",
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001012".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Wait {
                                wait: BeatWait::new(
                                    "step_019f5200-0000-7000-8000-000000001012",
                                    250,
                                    1_000,
                                ),
                            },
                        }],
                        vec![BeatPlanBeat {
                            beat_id: "beat_019f5200-0000-7000-8000-000000001013".to_owned(),
                            kind: BeatKind::Perform,
                            fallback_action_id: None,
                            body: BeatPlanBeatBody::Wait {
                                wait: BeatWait::new(
                                    "step_019f5200-0000-7000-8000-000000001013",
                                    250,
                                    1_000,
                                ),
                            },
                        }],
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("failure branch fallbackActionId must be consumed by nested playAction");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: fallbackActionId on failure branch beat beat_019f5200-0000-7000-8000-000000001011 must apply to at least one nested playAction timeline step"
        );
    }

    #[test]
    fn beat_plan_timeline_compilation_compiles_target_only_beat_to_move_to_step() -> BuddyResult<()>
    {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000301".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-target"
            }),
            macro_id: "targetBeat".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000301".to_owned(),
                kind: BeatKind::Approach,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Target {
                    target: BeatTarget::move_to(
                        "step_019f5200-0000-7000-8000-000000000301",
                        MoveTarget::Edge {
                            edge: MoveEdge::Left,
                        },
                        15_000,
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000301",
                    "kind": "moveTo",
                    "target": {
                        "kind": "edge",
                        "edge": "left"
                    },
                    "afterActionId": null,
                    "timeoutMs": 15000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_compiles_action_only_beat_to_play_action_step(
    ) -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000501".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-action"
            }),
            macro_id: "actionBeat".to_owned(),
            fallback_action_id: Some("macro.fallback".to_owned()),
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000501".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Action {
                    action: BeatAction::once(
                        "step_019f5200-0000-7000-8000-000000000501",
                        "celebrate",
                        5_000,
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000501",
                    "kind": "playAction",
                    "actionId": "celebrate",
                    "fallbackActionId": "macro.fallback",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_compiles_wait_only_beat_to_wait_step() -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000601".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-wait"
            }),
            macro_id: "waitBeat".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000601".to_owned(),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Wait {
                    wait: BeatWait::new("step_019f5200-0000-7000-8000-000000000601", 750, 1_000),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000601",
                    "kind": "wait",
                    "durationMs": 750,
                    "timeoutMs": 1000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_compiles_path_beat_to_move_by_path_step() -> BuddyResult<()> {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000801".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "beat-path"
            }),
            macro_id: "pathBeat".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000801".to_owned(),
                kind: BeatKind::Approach,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Path {
                    path: BeatPath::move_by_path(
                        "step_019f5200-0000-7000-8000-000000000801",
                        vec![
                            MoveTarget::Edge {
                                edge: MoveEdge::Left,
                            },
                            MoveTarget::Center,
                            MoveTarget::Home,
                        ],
                        30_000,
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(compile_beat_plan_to_timeline_steps(&beat_plan)?)?,
            json!([
                {
                    "stepId": "step_019f5200-0000-7000-8000-000000000801",
                    "kind": "moveByPath",
                    "path": [
                        {
                            "kind": "edge",
                            "edge": "left"
                        },
                        {
                            "kind": "center"
                        },
                        {
                            "kind": "home"
                        }
                    ],
                    "afterActionId": null,
                    "timeoutMs": 30000
                }
            ])
        );
        Ok(())
    }

    #[test]
    fn beat_plan_timeline_compilation_rejects_empty_path_beat() {
        let beat_plan = BeatPlan {
            plan_id: "plan_019f5200-0000-7000-8000-000000000802".to_owned(),
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "empty-path-beat"
            }),
            macro_id: "emptyPathBeat".to_owned(),
            fallback_action_id: None,
            recovery: None,
            beats: vec![BeatPlanBeat {
                beat_id: "beat_019f5200-0000-7000-8000-000000000802".to_owned(),
                kind: BeatKind::Approach,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Path {
                    path: BeatPath::move_by_path(
                        "step_019f5200-0000-7000-8000-000000000802",
                        Vec::new(),
                        30_000,
                    ),
                },
            }],
            created_at: "2026-07-09T12:00:00.000Z".to_owned(),
        };

        let error = compile_beat_plan_to_timeline_steps(&beat_plan)
            .expect_err("path beat must contain at least one target");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: path beat beat_019f5200-0000-7000-8000-000000000802 must contain at least one target"
        );
    }
}
