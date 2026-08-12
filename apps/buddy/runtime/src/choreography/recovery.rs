use serde::Serialize;

use super::{
    macro_plan::MOVE_TO_STEP_TIMEOUT_MS,
    timeline::{MoveToStep, TimelineStep},
};

const RUNTIME_SAFE_FALLBACK_AFTER_ACTION_ID: &str = "sleep";
const RUNTIME_SAFE_FALLBACK_AFTER_ACTION_FALLBACK_ID: &str = "idle";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSafeFallbackPlan {
    pub(crate) plan_id: String,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) posture: RuntimeSafeFallbackPosture,
    pub(crate) steps: Vec<TimelineStep>,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeSafeFallbackPosture {
    HomeSleep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeSafeFallbackReason {
    MotionTimeout,
    StepFailed,
    StepInterrupted,
    ProtocolError,
    UnsupportedStepCapability,
    ExecutorError,
    MacroPlanningFailed,
}

impl RuntimeSafeFallbackReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::MotionTimeout => "sidecar.motionTimeout",
            Self::StepFailed => "sidecar.stepFailed",
            Self::StepInterrupted => "sidecar.stepInterrupted",
            Self::ProtocolError => "sidecar.protocolError",
            Self::UnsupportedStepCapability => "sidecar.unsupportedStepCapability",
            Self::ExecutorError => "executor.error",
            Self::MacroPlanningFailed => "macro.planningFailed",
        }
    }
}

pub(crate) struct RuntimeSafeFallbackPlanContext<'a> {
    pub(crate) plan_id: &'a str,
    pub(crate) step_id: &'a str,
    pub(crate) triggered_by_plan_id: &'a str,
    pub(crate) triggered_by_step_id: Option<&'a str>,
    pub(crate) trigger_reason: RuntimeSafeFallbackReason,
    pub(crate) created_at: &'a str,
}

pub(crate) fn create_runtime_safe_fallback_plan(
    context: RuntimeSafeFallbackPlanContext<'_>,
) -> RuntimeSafeFallbackPlan {
    let mut source_ref = serde_json::json!({
        "kind": "systemRecovery",
        "triggeredByPlanId": context.triggered_by_plan_id,
        "triggerReason": context.trigger_reason.as_str(),
    });
    if let Some(triggered_by_step_id) = context.triggered_by_step_id {
        source_ref["triggeredByStepId"] =
            serde_json::Value::String(triggered_by_step_id.to_owned());
    }

    RuntimeSafeFallbackPlan {
        plan_id: context.plan_id.to_owned(),
        source_ref,
        posture: RuntimeSafeFallbackPosture::HomeSleep,
        steps: vec![TimelineStep::MoveTo(
            MoveToStep::home_with_after_action_fallback(
                context.step_id,
                RUNTIME_SAFE_FALLBACK_AFTER_ACTION_ID,
                RUNTIME_SAFE_FALLBACK_AFTER_ACTION_FALLBACK_ID,
                MOVE_TO_STEP_TIMEOUT_MS,
            ),
        )],
        created_at: context.created_at.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeSafeFallbackReason;

    #[test]
    fn runtime_safe_fallback_reasons_expose_stable_trigger_reason_values() {
        assert_eq!(
            RuntimeSafeFallbackReason::MotionTimeout.as_str(),
            "sidecar.motionTimeout"
        );
        assert_eq!(
            RuntimeSafeFallbackReason::StepFailed.as_str(),
            "sidecar.stepFailed"
        );
        assert_eq!(
            RuntimeSafeFallbackReason::StepInterrupted.as_str(),
            "sidecar.stepInterrupted"
        );
        assert_eq!(
            RuntimeSafeFallbackReason::ProtocolError.as_str(),
            "sidecar.protocolError"
        );
        assert_eq!(
            RuntimeSafeFallbackReason::UnsupportedStepCapability.as_str(),
            "sidecar.unsupportedStepCapability"
        );
        assert_eq!(
            RuntimeSafeFallbackReason::ExecutorError.as_str(),
            "executor.error"
        );
        assert_eq!(
            RuntimeSafeFallbackReason::MacroPlanningFailed.as_str(),
            "macro.planningFailed"
        );
    }
}
