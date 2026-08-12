use crate::{
    error::{BuddyError, BuddyResult},
    local_log::LocalLogTimestamp,
    native_pet::NativePetPresetBehaviorEvent,
    storage::BuddyStorage,
};

use super::action_log::{ActionLogEvent, ActionLogSink};
use super::{
    affective::ResolveContext,
    registry::ActionRegistry,
    timeline::{PlayActionStep, TimelineFailurePolicy, TimelinePlan, TimelineStep},
};

const ACTION_LOG_SCHEMA_VERSION: u16 = 1;
const ACTION_LOG_TRIGGER_SOURCE_PRESET_BEHAVIOR: &str = "presetBehavior";
const PRESET_BEHAVIOR_STEP_TIMEOUT_MS: u64 = 5_000;
const PRESET_BEHAVIOR_STEP_FALLBACK_ACTION_ID: &str = "idle";

#[derive(Clone, Debug)]
pub(crate) struct NativePetPresetBehaviorLogContext {
    plan_id: String,
    step_id: String,
    plan_started_event_id: String,
    step_resolved_event_id: String,
    step_completed_event_id: String,
    plan_completed_event_id: String,
    created_at: String,
    resolved_at: String,
    completed_at: String,
}

impl NativePetPresetBehaviorLogContext {
    pub(crate) fn new() -> Self {
        let created_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let resolved_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
        let completed_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();

        Self {
            plan_id: prefixed_uuid_v7("plan"),
            step_id: prefixed_uuid_v7("step"),
            plan_started_event_id: prefixed_uuid_v7("evt"),
            step_resolved_event_id: prefixed_uuid_v7("evt"),
            step_completed_event_id: prefixed_uuid_v7("evt"),
            plan_completed_event_id: prefixed_uuid_v7("evt"),
            created_at,
            resolved_at,
            completed_at,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed_for_test() -> Self {
        Self {
            plan_id: "plan_019f4200-0000-7000-8000-000000000001".to_owned(),
            step_id: "step_019f4200-0000-7000-8000-000000000002".to_owned(),
            plan_started_event_id: "evt_019f4200-0000-7000-8000-000000000003".to_owned(),
            step_resolved_event_id: "evt_019f4200-0000-7000-8000-000000000004".to_owned(),
            step_completed_event_id: "evt_019f4200-0000-7000-8000-000000000005".to_owned(),
            plan_completed_event_id: "evt_019f4200-0000-7000-8000-000000000006".to_owned(),
            created_at: "2026-07-09T00:00:00.000Z".to_owned(),
            resolved_at: "2026-07-09T00:00:00.010Z".to_owned(),
            completed_at: "2026-07-09T00:00:00.020Z".to_owned(),
        }
    }
}

pub(crate) fn append_native_pet_preset_behavior_action_log(
    storage: &BuddyStorage,
    event: &NativePetPresetBehaviorEvent,
    context: NativePetPresetBehaviorLogContext,
) -> BuddyResult<()> {
    let action_selector = preset_behavior_action_selector(event);
    let resolve_context = ResolveContext::default();
    let registry = ActionRegistry::load_bundled()?;
    let timeline_plan = compile_native_pet_preset_behavior_timeline_plan(
        &registry,
        event,
        &context,
        &resolve_context,
    )?;
    let Some(timeline_step) = timeline_plan.steps.first() else {
        return Err(BuddyError::Runtime(
            "preset behavior compiled to empty timeline plan".to_owned(),
        ));
    };
    let TimelineStep::PlayAction(play_action_step) = timeline_step else {
        return Err(BuddyError::Runtime(
            "preset behavior compiled to non-playAction timeline step".to_owned(),
        ));
    };
    let source_ref = timeline_plan.source_ref.clone();
    let resolution = registry.resolve_play_action(&play_action_step.action_id, &resolve_context)?;
    let sink = ActionLogSink::new(storage.clone());

    for action_log_event in [
        preset_behavior_action_log_event(
            PresetBehaviorActionLogEnvelope {
                event_id: context.plan_started_event_id.as_str(),
                event_type: "plan.started",
                status: "started",
                reason_code: "presetBehavior.started",
                plan_id: context.plan_id.as_str(),
                step_id: None,
                source_ref: &source_ref,
                created_at: context.created_at.as_str(),
            },
            serde_json::json!({
                "sourceRef": source_ref,
                "presetBehaviorId": event.preset_behavior_id,
                "outcome": event.outcome,
                "actionSelector": action_selector.as_str(),
                "actionId": play_action_step.action_id.as_str(),
            }),
        ),
        preset_behavior_action_log_event(
            PresetBehaviorActionLogEnvelope {
                event_id: context.step_resolved_event_id.as_str(),
                event_type: "step.resolved",
                status: "resolved",
                reason_code: "presetBehavior.resolved",
                plan_id: context.plan_id.as_str(),
                step_id: Some(context.step_id.as_str()),
                source_ref: &source_ref,
                created_at: context.resolved_at.as_str(),
            },
            serde_json::json!({
                "resolution": &resolution,
                "timelineStep": &timeline_step,
                "actionSelector": action_selector.as_str(),
                "outcome": event.outcome,
            }),
        ),
        preset_behavior_action_log_event(
            PresetBehaviorActionLogEnvelope {
                event_id: context.step_completed_event_id.as_str(),
                event_type: "step.completed",
                status: "completed",
                reason_code: "presetBehavior.stepCompleted",
                plan_id: context.plan_id.as_str(),
                step_id: Some(context.step_id.as_str()),
                source_ref: &source_ref,
                created_at: context.completed_at.as_str(),
            },
            serde_json::json!({
                "actionId": resolution.action_id.as_str(),
                "animationRef": resolution.animation_ref.as_str(),
                "elapsedMs": resolution.duration_ms,
                "outcome": event.outcome,
            }),
        ),
        preset_behavior_action_log_event(
            PresetBehaviorActionLogEnvelope {
                event_id: context.plan_completed_event_id.as_str(),
                event_type: "plan.completed",
                status: "completed",
                reason_code: "presetBehavior.completed",
                plan_id: context.plan_id.as_str(),
                step_id: None,
                source_ref: &source_ref,
                created_at: context.completed_at.as_str(),
            },
            serde_json::json!({
                "status": "completed",
                "completedStepCount": 1,
                "durationMs": resolution.duration_ms,
                "outcome": event.outcome,
            }),
        ),
    ] {
        sink.append_event(&action_log_event)?;
    }

    Ok(())
}

pub(crate) fn compile_native_pet_preset_behavior_timeline_plan(
    registry: &ActionRegistry,
    event: &NativePetPresetBehaviorEvent,
    context: &NativePetPresetBehaviorLogContext,
    resolve_context: &ResolveContext,
) -> BuddyResult<TimelinePlan> {
    Ok(TimelinePlan {
        plan_id: context.plan_id.clone(),
        source_ref: preset_behavior_source_ref(event),
        failure_policy: TimelineFailurePolicy::Abort,
        steps: vec![compile_native_pet_preset_behavior_timeline_step(
            registry,
            event,
            context,
            resolve_context,
        )?],
        created_at: context.created_at.clone(),
    })
}

pub(crate) fn compile_native_pet_preset_behavior_timeline_step(
    registry: &ActionRegistry,
    event: &NativePetPresetBehaviorEvent,
    context: &NativePetPresetBehaviorLogContext,
    resolve_context: &ResolveContext,
) -> BuddyResult<TimelineStep> {
    let action_selector = preset_behavior_action_selector(event);
    let resolution = registry.resolve_action_for_animation(
        &action_selector,
        &event.animation,
        resolve_context,
    )?;
    let mut step = PlayActionStep::once(
        context.step_id.clone(),
        resolution.action_id,
        PRESET_BEHAVIOR_STEP_TIMEOUT_MS,
    );
    step.fallback_action_id = Some(PRESET_BEHAVIOR_STEP_FALLBACK_ACTION_ID.to_owned());

    Ok(TimelineStep::PlayAction(step))
}

fn preset_behavior_source_ref(event: &NativePetPresetBehaviorEvent) -> serde_json::Value {
    let mut source_ref = serde_json::json!({
        "kind": "presetBehavior",
        "presetBehaviorId": event.preset_behavior_id,
    });
    if let Some(interaction_id) = event.interaction_id.as_deref() {
        source_ref["interactionId"] = serde_json::json!(interaction_id);
    }

    source_ref
}

fn preset_behavior_action_selector(event: &NativePetPresetBehaviorEvent) -> String {
    format!("{}.{}", event.preset_behavior_id, event.outcome)
}

struct PresetBehaviorActionLogEnvelope<'a> {
    event_id: &'a str,
    event_type: &'a str,
    status: &'a str,
    reason_code: &'a str,
    plan_id: &'a str,
    step_id: Option<&'a str>,
    source_ref: &'a serde_json::Value,
    created_at: &'a str,
}

fn preset_behavior_action_log_event(
    envelope: PresetBehaviorActionLogEnvelope<'_>,
    payload: serde_json::Value,
) -> ActionLogEvent {
    ActionLogEvent {
        event_id: envelope.event_id.to_owned(),
        schema_version: ACTION_LOG_SCHEMA_VERSION,
        event_type: envelope.event_type.to_owned(),
        status: envelope.status.to_owned(),
        reason_code: envelope.reason_code.to_owned(),
        plan_id: envelope.plan_id.to_owned(),
        step_id: envelope.step_id.map(str::to_owned),
        source_ref: envelope.source_ref.clone(),
        trigger_source: ACTION_LOG_TRIGGER_SOURCE_PRESET_BEHAVIOR.to_owned(),
        payload,
        created_at: envelope.created_at.to_owned(),
    }
}

fn prefixed_uuid_v7(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::now_v7())
}

#[cfg(test)]
mod tests {
    use crate::{
        choreography::{
            affective::ResolveContext, registry::ActionRegistry,
            sidecar_protocol::compile_execute_step_request, timeline::TimelineStep,
        },
        native_pet::NativePetPresetBehaviorEvent,
        storage::BuddyStorage,
    };

    use super::{
        append_native_pet_preset_behavior_action_log,
        compile_native_pet_preset_behavior_timeline_plan,
        compile_native_pet_preset_behavior_timeline_step, NativePetPresetBehaviorLogContext,
    };

    #[test]
    fn preset_behavior_event_compiles_to_timeline_plan_with_stable_source_ref() {
        let registry = ActionRegistry::load_bundled().expect("load action registry");
        let event = NativePetPresetBehaviorEvent {
            preset_behavior_id: "throw_after_drag".to_owned(),
            interaction_id: Some("interaction_test".to_owned()),
            outcome: "stumble".to_owned(),
            animation: "stumble_recover_left".to_owned(),
        };
        let context = NativePetPresetBehaviorLogContext::fixed_for_test();

        let plan = compile_native_pet_preset_behavior_timeline_plan(
            &registry,
            &event,
            &context,
            &ResolveContext::default(),
        )
        .expect("compile preset behavior plan");

        assert_eq!(plan.plan_id, "plan_019f4200-0000-7000-8000-000000000001");
        assert_eq!(
            plan.source_ref,
            serde_json::json!({
                "kind": "presetBehavior",
                "presetBehaviorId": "throw_after_drag",
                "interactionId": "interaction_test"
            })
        );
        assert_eq!(plan.created_at, "2026-07-09T00:00:00.000Z");
        assert_eq!(plan.steps.len(), 1);
        let TimelineStep::PlayAction(step) = &plan.steps[0] else {
            panic!("expected playAction step");
        };
        assert_eq!(step.action_id, "throw_after_drag.stumble.left");
    }

    #[test]
    fn preset_behavior_fall_compiles_to_precise_directional_play_action_step() {
        let registry = ActionRegistry::load_bundled().expect("load action registry");
        let event = NativePetPresetBehaviorEvent {
            preset_behavior_id: "throw_after_drag".to_owned(),
            interaction_id: Some("interaction_test".to_owned()),
            outcome: "fall".to_owned(),
            animation: "trip_fall_right".to_owned(),
        };
        let context = NativePetPresetBehaviorLogContext::fixed_for_test();

        let step = compile_native_pet_preset_behavior_timeline_step(
            &registry,
            &event,
            &context,
            &ResolveContext::default(),
        )
        .expect("compile preset behavior step");

        let TimelineStep::PlayAction(step) = step else {
            panic!("expected playAction step");
        };
        assert_eq!(step.step_id, "step_019f4200-0000-7000-8000-000000000002");
        assert_eq!(step.action_id, "throw_after_drag.fall.right");
        assert_eq!(step.fallback_action_id.as_deref(), Some("idle"));
    }

    #[test]
    fn preset_behavior_stumble_compiles_to_precise_directional_action() {
        let registry = ActionRegistry::load_bundled().expect("load action registry");
        let event = NativePetPresetBehaviorEvent {
            preset_behavior_id: "throw_after_drag".to_owned(),
            interaction_id: Some("interaction_test".to_owned()),
            outcome: "stumble".to_owned(),
            animation: "stumble_recover_right".to_owned(),
        };
        let context = NativePetPresetBehaviorLogContext::fixed_for_test();

        let step = compile_native_pet_preset_behavior_timeline_step(
            &registry,
            &event,
            &context,
            &ResolveContext::default(),
        )
        .expect("compile stumble preset behavior step");

        let TimelineStep::PlayAction(step) = step else {
            panic!("expected playAction step");
        };
        assert_eq!(step.action_id, "throw_after_drag.stumble.right");
        assert_eq!(step.fallback_action_id.as_deref(), Some("idle"));
    }

    #[test]
    fn preset_behavior_get_up_compiles_to_sidecar_execute_step() {
        let registry = ActionRegistry::load_bundled().expect("load action registry");
        let event = NativePetPresetBehaviorEvent {
            preset_behavior_id: "throw_after_drag".to_owned(),
            interaction_id: Some("interaction_test".to_owned()),
            outcome: "get_up".to_owned(),
            animation: "fallen_get_up_left".to_owned(),
        };
        let context = NativePetPresetBehaviorLogContext::fixed_for_test();
        let step = compile_native_pet_preset_behavior_timeline_step(
            &registry,
            &event,
            &context,
            &ResolveContext::default(),
        )
        .expect("compile preset behavior step");

        let request = compile_execute_step_request(&registry, &ResolveContext::default(), &step)
            .expect("compile executeStep request");

        assert_eq!(request.step_id, "step_019f4200-0000-7000-8000-000000000002");
        assert_eq!(
            serde_json::to_value(request.step).expect("serialize step"),
            serde_json::json!({
                "kind": "playAction",
                "animation": "fallen_get_up_left",
                "playback": {
                    "kind": "once",
                    "durationMs": 1230
                },
                "interruptPolicy": "finishStep",
                "timeoutMs": 5000
            })
        );
    }

    #[test]
    fn preset_behavior_action_log_records_compiled_timeline_step() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        append_native_pet_preset_behavior_action_log(
            &storage,
            &NativePetPresetBehaviorEvent {
                preset_behavior_id: "throw_after_drag".to_owned(),
                interaction_id: Some("interaction_test".to_owned()),
                outcome: "fall".to_owned(),
                animation: "trip_fall_right".to_owned(),
            },
            NativePetPresetBehaviorLogContext::fixed_for_test(),
        )
        .expect("append preset behavior action log");

        let lines = storage.read_action_log_jsonl_lines_for_test();
        let resolved_event = lines
            .iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("jsonl event"))
            .find(|event| event["eventType"] == "step.resolved")
            .expect("step.resolved event");

        assert_eq!(
            resolved_event["payload"]["timelineStep"],
            serde_json::json!({
                "stepId": "step_019f4200-0000-7000-8000-000000000002",
                "kind": "playAction",
                "actionId": "throw_after_drag.fall.right",
                "fallbackActionId": "idle",
                "expectedPlayback": "once",
                "timeoutMs": 5000
            })
        );
    }
}
