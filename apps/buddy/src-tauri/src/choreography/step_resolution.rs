use crate::error::{BuddyError, BuddyResult};

use super::{
    affective::ResolveContext,
    registry::{
        ActionRegistry, StepResolution, StepResolutionFallback,
        AFTER_ACTION_FALLBACK_RESOLUTION_REASON_CODE,
    },
    timeline::{MoveByPathStep, MoveToStep, PlayActionStep},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AfterActionResolution {
    pub(super) action_id: String,
    pub(super) animation_ref: String,
    pub(super) fallback: Option<StepResolutionFallback>,
}

impl AfterActionResolution {
    fn from_step_resolution(resolution: StepResolution) -> Self {
        Self {
            action_id: resolution.action_id,
            animation_ref: resolution.animation_ref,
            fallback: resolution.fallback,
        }
    }

    fn with_after_action_fallback(
        mut self,
        requested_action_id: impl Into<String>,
        fallback_action_id: impl Into<String>,
    ) -> Self {
        self.fallback = Some(StepResolutionFallback {
            requested_action_id: requested_action_id.into(),
            fallback_action_id: fallback_action_id.into(),
            reason_code: AFTER_ACTION_FALLBACK_RESOLUTION_REASON_CODE.to_owned(),
            unsupported_capability: None,
        });
        self
    }
}

pub(super) fn resolve_play_action_step(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    step: &PlayActionStep,
) -> BuddyResult<StepResolution> {
    let mut resolution = resolve_play_action_step_action(registry, resolve_context, step)?;
    match step.expected_playback.as_str() {
        "once" => Ok(resolution),
        "loopForDuration" => {
            let duration_ms = step.duration_ms.ok_or_else(|| {
                BuddyError::Validation("loopForDuration playAction requires durationMs".to_owned())
            })?;
            if duration_ms == 0 {
                return Err(BuddyError::Validation(
                    "loopForDuration playAction durationMs must be positive".to_owned(),
                ));
            }

            resolution.playback_kind = "loopForDuration".to_owned();
            resolution.duration_ms = duration_ms;
            resolution.loop_animation = true;
            Ok(resolution)
        }
        playback => Err(BuddyError::Validation(format!(
            "unsupported playAction expectedPlayback: {playback}"
        ))),
    }
}

fn resolve_play_action_step_action(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    step: &PlayActionStep,
) -> BuddyResult<StepResolution> {
    match registry.resolve_play_action(&step.action_id, resolve_context) {
        Ok(resolution) => Ok(resolution),
        Err(error) => {
            let Some(fallback_action_id) = step.fallback_action_id.as_deref() else {
                return Err(error);
            };

            registry
                .resolve_play_action(fallback_action_id, resolve_context)
                .map(|resolution| {
                    resolution.with_step_fallback(&step.action_id, fallback_action_id)
                })
        }
    }
}

pub(super) fn resolve_move_to_after_action(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    step: &MoveToStep,
) -> BuddyResult<Option<AfterActionResolution>> {
    resolve_after_action(
        registry,
        resolve_context,
        step.after_action_id.as_deref(),
        step.fallback_after_action_id.as_deref(),
        "moveTo",
    )
}

pub(super) fn resolve_move_by_path_after_action(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    step: &MoveByPathStep,
) -> BuddyResult<Option<AfterActionResolution>> {
    resolve_after_action(
        registry,
        resolve_context,
        step.after_action_id.as_deref(),
        step.fallback_after_action_id.as_deref(),
        "moveByPath",
    )
}

fn resolve_after_action(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    after_action_id: Option<&str>,
    fallback_after_action_id: Option<&str>,
    step_kind: &str,
) -> BuddyResult<Option<AfterActionResolution>> {
    let Some(after_action_id) = after_action_id else {
        if let Some(fallback_after_action_id) = fallback_after_action_id {
            return Err(BuddyError::Validation(format!(
                "{step_kind} fallbackAfterActionId requires afterActionId: {fallback_after_action_id}"
            )));
        }
        return Ok(None);
    };

    match resolve_stable_after_action(registry, resolve_context, after_action_id, step_kind) {
        Ok(resolution) => Ok(Some(resolution)),
        Err(error) => {
            let Some(fallback_after_action_id) = fallback_after_action_id else {
                return Err(error);
            };

            resolve_stable_after_action(
                registry,
                resolve_context,
                fallback_after_action_id,
                step_kind,
            )
            .map(|resolution| {
                Some(
                    resolution
                        .with_after_action_fallback(after_action_id, fallback_after_action_id),
                )
            })
        }
    }
}

fn resolve_stable_after_action(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    after_action_id: &str,
    step_kind: &str,
) -> BuddyResult<AfterActionResolution> {
    let resolution = registry.resolve_play_action(after_action_id, resolve_context)?;
    if resolution.playback_kind != "idleLoop" || !resolution.loop_animation {
        return Err(BuddyError::Validation(format!(
            "{step_kind} afterActionId must resolve to a stable loop action: {after_action_id}"
        )));
    }

    Ok(AfterActionResolution::from_step_resolution(resolution))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_action_step_uses_step_fallback_when_primary_action_is_unknown() {
        let registry = ActionRegistry::load_bundled().expect("load registry");
        let step = PlayActionStep {
            step_id: "step_019f4a00-0000-7000-8000-000000000001".to_owned(),
            kind: "playAction".to_owned(),
            action_id: "missing.action".to_owned(),
            fallback_action_id: Some("celebrate".to_owned()),
            expected_playback: "once".to_owned(),
            duration_ms: None,
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms: 5_000,
        };

        let resolution = resolve_play_action_step(&registry, &ResolveContext::default(), &step)
            .expect("resolve fallback action");
        let serialized_step = serde_json::to_value(&step).expect("serialize playAction step");

        assert_eq!(resolution.action_id, "celebrate");
        assert_eq!(resolution.animation_ref, "celebrate");
        assert_eq!(
            serde_json::to_value(&resolution.fallback).expect("serialize fallback"),
            serde_json::json!({
                "requestedActionId": "missing.action",
                "fallbackActionId": "celebrate",
                "reasonCode": "fallback.stepActionResolved"
            })
        );
        assert_eq!(serialized_step["fallbackActionId"], "celebrate");
    }

    #[test]
    fn play_action_step_without_fallback_keeps_unknown_action_error() {
        let registry = ActionRegistry::load_bundled().expect("load registry");
        let step = PlayActionStep {
            step_id: "step_019f4a00-0000-7000-8000-000000000002".to_owned(),
            kind: "playAction".to_owned(),
            action_id: "missing.action".to_owned(),
            fallback_action_id: None,
            expected_playback: "once".to_owned(),
            duration_ms: None,
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms: 5_000,
        };

        let error = resolve_play_action_step(&registry, &ResolveContext::default(), &step)
            .expect_err("missing action should still fail without fallback");
        let serialized_step = serde_json::to_value(&step).expect("serialize playAction step");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: unknown buddy actionId or selector: missing.action"
        );
        assert!(serialized_step.get("fallbackActionId").is_none());
    }
}
