use serde::Serialize;

use crate::error::{BuddyError, BuddyResult};

use super::macro_plan::{
    compile_beat_plan_to_timeline_steps, compile_macro_intent_to_beat_plan, BeatAction, BeatKind,
    BeatPlan, BeatPlanBeat, BeatPlanBeatBody, BeatPlanBuildContext, BeatTarget, BeatWait,
    LieDownMacroParams, MacroIntent, PatrolAroundScreenMacroParams, MOVE_TO_STEP_TIMEOUT_MS,
};
use super::timeline::{MoveTarget, PlayActionStep, TimelineStep};

pub(crate) const SINGLE_PLAY_ACTION_FIXTURE_NAME: &str = "single-play-action";
pub(crate) const AI_MACRO_DEMO_FIXTURE_NAME: &str = "ai-macro-demo";
const SINGLE_PLAY_ACTION_ID: &str = "celebrate";
const SINGLE_PLAY_ACTION_TIMEOUT_MS: u64 = 5_000;
const AI_MACRO_DEMO_CAST_ACTION_ID: &str = "cast";
const AI_MACRO_DEMO_CAST_TIMEOUT_MS: u64 = 5_000;
const AI_MACRO_DEMO_WAIT_DURATION_MS: u64 = 500;
const AI_MACRO_DEMO_WAIT_TIMEOUT_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DevFixturePlan {
    pub(crate) plan_id: String,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) steps: Vec<TimelineStep>,
    pub(crate) created_at: String,
}

impl DevFixturePlan {
    pub(crate) fn fixture_name(&self) -> &str {
        self.source_ref
            .get("fixtureName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
    }

    pub(crate) fn step_count(&self) -> usize {
        self.steps.len()
    }

    #[cfg(test)]
    pub(crate) fn only_play_action_step(&self) -> Option<&PlayActionStep> {
        match self.steps.as_slice() {
            [TimelineStep::PlayAction(step)] => Some(step),
            _ => None,
        }
    }
}

pub(crate) fn create_single_play_action_dev_fixture_plan(
    plan_id: impl Into<String>,
    step_id: impl Into<String>,
    created_at: impl Into<String>,
) -> DevFixturePlan {
    DevFixturePlan {
        plan_id: plan_id.into(),
        source_ref: serde_json::json!({
            "kind": "devFixture",
            "fixtureName": SINGLE_PLAY_ACTION_FIXTURE_NAME,
        }),
        steps: vec![TimelineStep::PlayAction(PlayActionStep {
            step_id: step_id.into(),
            kind: "playAction".to_owned(),
            action_id: SINGLE_PLAY_ACTION_ID.to_owned(),
            fallback_action_id: None,
            expected_playback: "once".to_owned(),
            duration_ms: None,
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms: SINGLE_PLAY_ACTION_TIMEOUT_MS,
        })],
        created_at: created_at.into(),
    }
}

pub(crate) fn create_ai_macro_demo_dev_fixture_plan(
    plan_id: impl Into<String>,
    beat_id: impl Into<String>,
    step_id: impl Into<String>,
    created_at: impl Into<String>,
) -> BuddyResult<DevFixturePlan> {
    let plan_id = plan_id.into();
    let beat_id = beat_id.into();
    let step_id = step_id.into();
    let created_at = created_at.into();
    let source_ref = serde_json::json!({
        "kind": "devFixture",
        "fixtureName": AI_MACRO_DEMO_FIXTURE_NAME,
    });
    let intent = MacroIntent::PatrolAroundScreen(PatrolAroundScreenMacroParams { loops: 2 });
    let patrol_plan = compile_macro_intent_to_beat_plan(
        &intent,
        BeatPlanBuildContext {
            plan_id: &plan_id,
            beat_id: &beat_id,
            step_id: &step_id,
            source_ref: source_ref.clone(),
            created_at: &created_at,
        },
    )?;
    let mut steps = compile_beat_plan_to_timeline_steps(&patrol_plan)?;
    let center_step_index = patrol_plan.beats.len();
    let settle_step_index = center_step_index + 1;
    let cast_step_index = settle_step_index + 1;
    let performance_plan = BeatPlan {
        plan_id: plan_id.clone(),
        source_ref: source_ref.clone(),
        macro_id: "aiMacroDemoPerformance".to_owned(),
        fallback_action_id: None,
        recovery: None,
        beats: vec![
            BeatPlanBeat {
                beat_id: indexed_id(&beat_id, center_step_index),
                kind: BeatKind::Approach,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Target {
                    target: BeatTarget::move_to(
                        indexed_id(&step_id, center_step_index),
                        MoveTarget::Center,
                        MOVE_TO_STEP_TIMEOUT_MS,
                    ),
                },
            },
            BeatPlanBeat {
                beat_id: indexed_id(&beat_id, settle_step_index),
                kind: BeatKind::Settle,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Wait {
                    wait: BeatWait::new(
                        indexed_id(&step_id, settle_step_index),
                        AI_MACRO_DEMO_WAIT_DURATION_MS,
                        AI_MACRO_DEMO_WAIT_TIMEOUT_MS,
                    ),
                },
            },
            BeatPlanBeat {
                beat_id: indexed_id(&beat_id, cast_step_index),
                kind: BeatKind::Perform,
                fallback_action_id: None,
                body: BeatPlanBeatBody::Action {
                    action: BeatAction::once(
                        indexed_id(&step_id, cast_step_index),
                        AI_MACRO_DEMO_CAST_ACTION_ID,
                        AI_MACRO_DEMO_CAST_TIMEOUT_MS,
                    ),
                },
            },
        ],
        created_at: created_at.clone(),
    };
    steps.extend(compile_beat_plan_to_timeline_steps(&performance_plan)?);

    let lie_down_step_index = cast_step_index + 1;
    let lie_down_plan = compile_macro_intent_to_beat_plan(
        &MacroIntent::LieDown(LieDownMacroParams::default()),
        BeatPlanBuildContext {
            plan_id: &plan_id,
            beat_id: &indexed_id(&beat_id, lie_down_step_index),
            step_id: &indexed_id(&step_id, lie_down_step_index),
            source_ref: source_ref.clone(),
            created_at: &created_at,
        },
    )?;
    steps.extend(compile_beat_plan_to_timeline_steps(&lie_down_plan)?);
    if steps.is_empty() {
        return Err(BuddyError::Validation(
            "ai macro demo fixture must compile to at least one timeline step".to_owned(),
        ));
    }

    Ok(DevFixturePlan {
        plan_id,
        source_ref,
        steps,
        created_at,
    })
}

fn indexed_id(base_id: &str, zero_based_index: usize) -> String {
    if zero_based_index == 0 {
        return base_id.to_owned();
    }

    format!("{base_id}.{}", zero_based_index + 1)
}
