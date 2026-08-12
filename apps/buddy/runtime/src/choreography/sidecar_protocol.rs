use crate::{
    error::{BuddyError, BuddyResult},
    native_pet::step_protocol::{
        execute_step_request, ExecuteStepPayload, ExecuteStepPlayback, ExecuteStepRequest,
        SidecarInterruptPolicy,
    },
};

use super::{
    affective::ResolveContext,
    registry::{ActionRegistry, StepResolution},
    step_resolution::{
        resolve_move_by_path_after_action, resolve_move_to_after_action, resolve_play_action_step,
    },
    timeline::TimelineStep,
};

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn compile_execute_step_request(
    registry: &ActionRegistry,
    resolve_context: &ResolveContext,
    step: &TimelineStep,
) -> BuddyResult<ExecuteStepRequest> {
    match step {
        TimelineStep::PlayAction(step) => {
            let resolution = resolve_play_action_step(registry, resolve_context, step)?;
            let playback = execute_step_playback(&resolution);
            Ok(execute_step_request(
                step.step_id.clone(),
                ExecuteStepPayload::PlayAction {
                    animation: resolution.animation_ref,
                    playback,
                    interrupt_policy: resolution.interrupt_policy,
                    completion_behavior: step.completion_behavior,
                    timeout_ms: step.timeout_ms,
                },
            ))
        }
        TimelineStep::MoveTo(step) => Ok(execute_step_request(
            step.step_id.clone(),
            ExecuteStepPayload::MoveTo {
                target: serde_json::to_value(&step.target)?,
                after: resolve_move_to_after_action(registry, resolve_context, step)?
                    .map(|resolution| resolution.animation_ref),
                interrupt_policy: SidecarInterruptPolicy::Interruptible,
                timeout_ms: step.timeout_ms,
            },
        )),
        TimelineStep::MoveByPath(step) => Ok(execute_step_request(
            step.step_id.clone(),
            ExecuteStepPayload::MoveByPath {
                path: step
                    .path
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?,
                after: resolve_move_by_path_after_action(registry, resolve_context, step)?
                    .map(|resolution| resolution.animation_ref),
                interrupt_policy: SidecarInterruptPolicy::Interruptible,
                timeout_ms: step.timeout_ms,
            },
        )),
        TimelineStep::Wait(step) => Err(BuddyError::Validation(format!(
            "wait step is host-side and cannot compile to sidecar executeStep: {}",
            step.step_id
        ))),
        TimelineStep::Skip(step) => Err(BuddyError::Validation(format!(
            "skipStep is host-side and cannot compile to sidecar executeStep: {}",
            step.step_id
        ))),
        TimelineStep::Retry(step) => Err(BuddyError::Validation(format!(
            "retry step is host-side and cannot compile to sidecar executeStep: {}",
            step.step_id
        ))),
        TimelineStep::Replace(step) => Err(BuddyError::Validation(format!(
            "replaceStep is host-side and cannot compile to sidecar executeStep: {}",
            step.step_id
        ))),
        TimelineStep::Recover(step) => Err(BuddyError::Validation(format!(
            "recover step is host-side and cannot compile to sidecar executeStep: {}",
            step.step_id
        ))),
        TimelineStep::Repeat(_)
        | TimelineStep::Choose(_)
        | TimelineStep::SetFallback(_)
        | TimelineStep::Try(_)
        | TimelineStep::SnapshotPosition(_)
        | TimelineStep::RestorePosition(_) => Err(BuddyError::Validation(format!(
            "{} timeline step is planner-side and cannot compile to sidecar executeStep: {}",
            step.kind(),
            step.step_id()
        ))),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::compile_execute_step_request;
    use crate::choreography::{
        affective::ResolveContext,
        registry::ActionRegistry,
        timeline::{
            MoveByPathStep, MoveTarget, MoveToStep, PlayActionStep, RecoverStep, RepeatStep,
            ReplaceStep, RetryStep, SkipStep, TimelineSkipReason, TimelineStep, WaitStep,
        },
    };
    use crate::native_pet::step_protocol::{
        interrupt_step_request, motion_timeout_step_failed_response,
        parse_sidecar_state_snapshot_response, parse_sidecar_step_response,
        protocol_error_response, protocol_error_response_with_code, query_state_request,
        state_snapshot_response, step_completed_response, step_interrupted_response,
        SidecarInterruptReasonCode, SidecarPlayActionCompletionBehavior, SidecarStepErrorCode,
        SidecarStepResponse,
    };

    #[test]
    fn compile_execute_step_request_resolves_play_action_without_raw_action_id() {
        let registry = ActionRegistry::load_bundled().expect("load bundled registry");
        let mut step = PlayActionStep::once(
            "step_019f4800-0000-7000-8000-000000000001",
            "celebrate",
            5_000,
        );
        step.completion_behavior = SidecarPlayActionCompletionBehavior::HoldLastFrame;
        let step = TimelineStep::PlayAction(step);

        let request = compile_execute_step_request(&registry, &ResolveContext::default(), &step)
            .expect("compile executeStep request");

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4800-0000-7000-8000-000000000001",
                "type": "executeStep",
                "stepId": "step_019f4800-0000-7000-8000-000000000001",
                "step": {
                    "kind": "playAction",
                    "animation": "celebrate",
                    "playback": {
                        "kind": "once",
                        "durationMs": 1720
                    },
                    "interruptPolicy": "finishStep",
                    "completionBehavior": "holdLastFrame",
                    "timeoutMs": 5000
                }
            })
        );
    }

    #[test]
    fn compile_execute_step_request_resolves_loop_for_duration_play_action() {
        let registry = ActionRegistry::load_bundled().expect("load bundled registry");
        let step = TimelineStep::PlayAction(PlayActionStep::loop_for_duration(
            "step_019f4800-0000-7000-8000-000000000004",
            "celebrate",
            10_000,
            11_000,
        ));

        let request = compile_execute_step_request(&registry, &ResolveContext::default(), &step)
            .expect("compile executeStep request");

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4800-0000-7000-8000-000000000004",
                "type": "executeStep",
                "stepId": "step_019f4800-0000-7000-8000-000000000004",
                "step": {
                    "kind": "playAction",
                    "animation": "celebrate",
                    "playback": {
                        "kind": "loopForDuration",
                        "durationMs": 10000,
                        "clipDurationMs": 1720
                    },
                    "interruptPolicy": "finishStep",
                    "timeoutMs": 11000
                }
            })
        );
    }

    #[test]
    fn compile_execute_step_request_resolves_move_after_action_to_animation() {
        let registry = ActionRegistry::load_bundled().expect("load bundled registry");
        let step = TimelineStep::MoveTo(MoveToStep::home_with_after_action(
            "step_019f4800-0000-7000-8000-000000000002",
            "sleep",
            15_000,
        ));

        let request = compile_execute_step_request(&registry, &ResolveContext::default(), &step)
            .expect("compile executeStep request");

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4800-0000-7000-8000-000000000002",
                "type": "executeStep",
                "stepId": "step_019f4800-0000-7000-8000-000000000002",
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
    fn compile_execute_step_request_uses_move_after_action_fallback_when_primary_is_unknown() {
        let registry = ActionRegistry::load_bundled().expect("load bundled registry");
        let mut step = MoveToStep::home_with_after_action(
            "step_019f4800-0000-7000-8000-000000000012",
            "missing.after_action",
            15_000,
        );
        step.fallback_after_action_id = Some("sleep".to_owned());
        let step = TimelineStep::MoveTo(step);

        let request = compile_execute_step_request(&registry, &ResolveContext::default(), &step)
            .expect("compile executeStep request with fallback after action");

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4800-0000-7000-8000-000000000012",
                "type": "executeStep",
                "stepId": "step_019f4800-0000-7000-8000-000000000012",
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
    fn compile_execute_step_request_resolves_move_by_path_after_action_to_animation() {
        let registry = ActionRegistry::load_bundled().expect("load bundled registry");
        let mut step = MoveByPathStep::new(
            "step_019f4800-0000-7000-8000-000000000008",
            vec![
                MoveTarget::Edge {
                    edge: crate::choreography::timeline::MoveEdge::Left,
                },
                MoveTarget::Center,
            ],
            30_000,
        );
        step.after_action_id = Some("sleep".to_owned());
        let step = TimelineStep::MoveByPath(step);

        let request = compile_execute_step_request(&registry, &ResolveContext::default(), &step)
            .expect("compile executeStep request");

        assert_eq!(
            serde_json::to_value(request).expect("serialize executeStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4800-0000-7000-8000-000000000008",
                "type": "executeStep",
                "stepId": "step_019f4800-0000-7000-8000-000000000008",
                "step": {
                    "kind": "moveByPath",
                    "path": [
                        { "kind": "edge", "edge": "left" },
                        { "kind": "center" }
                    ],
                    "after": "sleep",
                    "interruptPolicy": "interruptible",
                    "timeoutMs": 30000
                }
            })
        );
    }

    #[test]
    fn compile_execute_step_request_rejects_host_and_planner_side_steps() {
        let registry = ActionRegistry::load_bundled().expect("load bundled registry");
        let cases = [
            TimelineStep::Wait(WaitStep::new(
                "step_019f4800-0000-7000-8000-000000000009",
                250,
                1_000,
            )),
            TimelineStep::Skip(SkipStep {
                step_id: "step_019f4800-0000-7000-8000-000000000012".to_owned(),
                kind: "skipStep".to_owned(),
                reason: TimelineSkipReason::BranchNotRequired,
            }),
            TimelineStep::Retry(RetryStep {
                step_id: "step_019f4800-0000-7000-8000-000000000013".to_owned(),
                kind: "retry".to_owned(),
                max_attempts: 2,
                steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                    "step_019f4800-0000-7000-8000-000000000014",
                    "celebrate",
                    5_000,
                ))],
            }),
            TimelineStep::Replace(ReplaceStep {
                step_id: "step_019f4800-0000-7000-8000-000000000015".to_owned(),
                kind: "replaceStep".to_owned(),
                steps: vec![],
                replacement_steps: vec![],
            }),
            TimelineStep::Recover(RecoverStep {
                step_id: "step_019f4800-0000-7000-8000-000000000018".to_owned(),
                kind: "recover".to_owned(),
                steps: vec![],
                recovery_steps: vec![],
            }),
            TimelineStep::Repeat(RepeatStep {
                step_id: "step_019f4800-0000-7000-8000-000000000010".to_owned(),
                kind: "repeat".to_owned(),
                times: 2,
                steps: vec![],
            }),
        ];

        for step in cases {
            let expected_kind = step.kind().to_owned();
            let error = compile_execute_step_request(&registry, &ResolveContext::default(), &step)
                .expect_err("host and planner steps must not compile to sidecar executeStep");

            assert!(error.to_string().contains(&expected_kind));
        }
    }

    #[test]
    fn protocol_error_response_serializes_stable_json_line_payload() {
        let response = protocol_error_response_with_code(
            Some("step_019f4800-0000-7000-8000-000000000003"),
            SidecarStepErrorCode::InvalidStepProtocol,
            "executeStep step.kind is unsupported",
        );

        assert_eq!(
            serde_json::to_value(response).expect("serialize protocolError"),
            json!({
                "protocolVersion": 1,
                "correlationId": "message_019f4800-0000-7000-8000-000000000003",
                "type": "protocolError",
                "stepId": "step_019f4800-0000-7000-8000-000000000003",
                "code": "invalidStepProtocol",
                "message": "executeStep step.kind is unsupported"
            })
        );
    }

    #[test]
    fn interrupt_step_request_serializes_stable_json_line_payload() {
        let request = interrupt_step_request(
            "step_019f4800-0000-7000-8000-000000000005",
            SidecarInterruptReasonCode::AdmissionPreemptedByHigherPriorityPlan,
        );

        assert_eq!(
            serde_json::to_value(request).expect("serialize interruptStep"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f4800-0000-7000-8000-000000000005",
                "type": "interruptStep",
                "stepId": "step_019f4800-0000-7000-8000-000000000005",
                "reasonCode": "admission.preemptedByHigherPriorityPlan"
            })
        );
    }

    #[test]
    fn query_state_request_serializes_stable_json_line_payload() {
        let request = query_state_request("state_019f5500-0000-7000-8000-000000000001");

        assert_eq!(
            serde_json::to_value(request).expect("serialize queryState"),
            json!({
                "protocolVersion": 1,
                "messageId": "message_019f5500-0000-7000-8000-000000000001",
                "type": "queryState",
                "requestId": "state_019f5500-0000-7000-8000-000000000001"
            })
        );
    }

    #[test]
    fn state_snapshot_response_serializes_stable_json_line_payload() {
        let response =
            state_snapshot_response("state_019f5500-0000-7000-8000-000000000002", 120, 640);

        assert_eq!(
            serde_json::to_value(response).expect("serialize stateSnapshot"),
            json!({
                "protocolVersion": 1,
                "correlationId": "message_019f5500-0000-7000-8000-000000000002",
                "type": "stateSnapshot",
                "requestId": "state_019f5500-0000-7000-8000-000000000002",
                "position": {
                    "x": 120,
                    "y": 640
                }
            })
        );
    }

    #[test]
    fn step_completed_response_serializes_stable_json_line_payload() {
        let response = step_completed_response("step_019f4800-0000-7000-8000-000000000006", 1_720);

        assert_eq!(
            serde_json::to_value(response).expect("serialize stepCompleted"),
            json!({
                "protocolVersion": 1,
                "correlationId": "message_019f4800-0000-7000-8000-000000000006",
                "type": "stepCompleted",
                "stepId": "step_019f4800-0000-7000-8000-000000000006",
                "elapsedMs": 1720
            })
        );
    }

    #[test]
    fn step_failed_response_serializes_stable_json_line_payload() {
        let response = motion_timeout_step_failed_response(
            "step_019f4800-0000-7000-8000-000000000007",
            Some(15_000),
        );

        assert_eq!(
            serde_json::to_value(response).expect("serialize stepFailed"),
            json!({
                "protocolVersion": 1,
                "correlationId": "message_019f4800-0000-7000-8000-000000000007",
                "type": "stepFailed",
                "stepId": "step_019f4800-0000-7000-8000-000000000007",
                "code": "motionTimeout",
                "message": "native pet motion did not settle before timeout",
                "elapsedMs": 15000
            })
        );
    }

    #[test]
    fn step_interrupted_response_serializes_stable_json_line_payload() {
        let response = step_interrupted_response(
            "step_019f4800-0000-7000-8000-000000000008",
            SidecarInterruptReasonCode::AdmissionPreemptedByHigherPriorityPlan,
            None,
        );

        assert_eq!(
            serde_json::to_value(response).expect("serialize stepInterrupted"),
            json!({
                "protocolVersion": 1,
                "correlationId": "message_019f4800-0000-7000-8000-000000000008",
                "type": "stepInterrupted",
                "stepId": "step_019f4800-0000-7000-8000-000000000008",
                "reasonCode": "admission.preemptedByHigherPriorityPlan"
            })
        );
    }

    #[test]
    fn parse_sidecar_step_response_reads_step_completed_json_line() {
        let response = parse_sidecar_step_response(
            r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000009","type":"stepCompleted","stepId":"step_019f4800-0000-7000-8000-000000000009","elapsedMs":1720}"#,
        )
        .expect("parse stepCompleted response");

        assert_eq!(
            response,
            SidecarStepResponse::StepCompleted(step_completed_response(
                "step_019f4800-0000-7000-8000-000000000009",
                1_720,
            ))
        );
    }

    #[test]
    fn parse_sidecar_state_snapshot_response_reads_state_snapshot_json_line() {
        let response = parse_sidecar_state_snapshot_response(
            r#"{"protocolVersion":1,"correlationId":"message_019f5500-0000-7000-8000-000000000003","type":"stateSnapshot","requestId":"state_019f5500-0000-7000-8000-000000000003","position":{"x":120,"y":640}}"#,
        )
        .expect("parse stateSnapshot response");

        assert_eq!(
            response,
            state_snapshot_response("state_019f5500-0000-7000-8000-000000000003", 120, 640)
        );
    }

    #[test]
    fn parse_sidecar_state_snapshot_response_rejects_unknown_fields() {
        let error = parse_sidecar_state_snapshot_response(
            r#"{"protocolVersion":1,"correlationId":"message_019f5500-0000-7000-8000-000000000004","type":"stateSnapshot","requestId":"state_019f5500-0000-7000-8000-000000000004","position":{"x":120,"y":640},"debug":true}"#,
        )
        .expect_err("stateSnapshot extra field should be rejected");

        assert_eq!(
            error.to_string(),
            "buddy state validation failed: unsupported sidecar stateSnapshot field: debug"
        );
    }

    #[test]
    fn parse_sidecar_step_response_reads_protocol_error_json_line() {
        let response = parse_sidecar_step_response(
            r#"{"protocolVersion":1,"type":"protocolError","stepId":null,"code":"invalidStepProtocol","message":"missing type"}"#,
        )
        .expect("parse protocolError response");

        assert_eq!(
            response,
            SidecarStepResponse::ProtocolError(protocol_error_response(
                None,
                SidecarStepErrorCode::InvalidStepProtocol,
                "missing type",
            ))
        );
    }

    #[test]
    fn parse_sidecar_step_response_reads_step_failed_json_line() {
        let response = parse_sidecar_step_response(
            r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000011","type":"stepFailed","stepId":"step_019f4800-0000-7000-8000-000000000011","code":"motionTimeout","message":"native pet motion did not settle before timeout","elapsedMs":15000}"#,
        )
        .expect("parse stepFailed response");

        assert_eq!(
            response,
            SidecarStepResponse::StepFailed(motion_timeout_step_failed_response(
                "step_019f4800-0000-7000-8000-000000000011",
                Some(15_000),
            ))
        );
    }

    #[test]
    fn parse_sidecar_step_response_reads_interrupt_rejected_step_failed_json_line() {
        let response = parse_sidecar_step_response(
            r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000024","type":"stepFailed","stepId":"step_019f4800-0000-7000-8000-000000000024","code":"interruptRejected","message":"native pet step rejected interrupt due to interrupt policy","elapsedMs":90}"#,
        )
        .expect("parse interruptRejected stepFailed response");

        let SidecarStepResponse::StepFailed(response) = response else {
            panic!("interruptRejected response should be stepFailed");
        };

        assert_eq!(
            serde_json::to_value(response).expect("serialize response"),
            json!({
                "protocolVersion": 1,
                "correlationId": "message_019f4800-0000-7000-8000-000000000024",
                "type": "stepFailed",
                "stepId": "step_019f4800-0000-7000-8000-000000000024",
                "code": "interruptRejected",
                "message": "native pet step rejected interrupt due to interrupt policy",
                "elapsedMs": 90
            })
        );
    }

    #[test]
    fn parse_sidecar_step_response_rejects_unknown_fields() {
        let cases = [
            (
                r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000025","type":"stepCompleted","stepId":"step_019f4800-0000-7000-8000-000000000025","elapsedMs":1720,"debug":true}"#,
                "stepCompleted",
            ),
            (
                r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000026","type":"stepFailed","stepId":"step_019f4800-0000-7000-8000-000000000026","code":"motionTimeout","message":"native pet motion did not settle before timeout","debug":true}"#,
                "stepFailed",
            ),
            (
                r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000027","type":"stepInterrupted","stepId":"step_019f4800-0000-7000-8000-000000000027","reasonCode":"admission.preemptedByHigherPriorityPlan","debug":true}"#,
                "stepInterrupted",
            ),
            (
                r#"{"protocolVersion":1,"type":"protocolError","stepId":null,"code":"invalidStepProtocol","message":"missing type","debug":true}"#,
                "protocolError",
            ),
        ];

        for (line, message_type) in cases {
            let error = parse_sidecar_step_response(line)
                .expect_err("sidecar step response extra field should be rejected");

            assert_eq!(
                error.to_string(),
                format!("buddy state validation failed: unsupported sidecar {message_type} field: debug")
            );
        }
    }

    #[test]
    fn parse_sidecar_step_response_reads_step_interrupted_json_line() {
        let response = parse_sidecar_step_response(
            r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000012","type":"stepInterrupted","stepId":"step_019f4800-0000-7000-8000-000000000012","reasonCode":"admission.preemptedByHigherPriorityPlan"}"#,
        )
        .expect("parse stepInterrupted response");

        assert_eq!(
            response,
            SidecarStepResponse::StepInterrupted(step_interrupted_response(
                "step_019f4800-0000-7000-8000-000000000012",
                SidecarInterruptReasonCode::AdmissionPreemptedByHigherPriorityPlan,
                None,
            ))
        );
    }

    #[test]
    fn parse_sidecar_step_response_rejects_unknown_protocol_values() {
        let cases = [
            (
                r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000021","type":"stepFailed","stepId":"step_019f4800-0000-7000-8000-000000000021","code":"futureFailure","message":"future failure"}"#,
                "buddy state validation failed: unsupported sidecar step error code: futureFailure",
            ),
            (
                r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000022","type":"protocolError","stepId":"step_019f4800-0000-7000-8000-000000000022","code":"futureProtocolError","message":"future protocol error"}"#,
                "buddy state validation failed: unsupported sidecar step error code: futureProtocolError",
            ),
            (
                r#"{"protocolVersion":1,"correlationId":"message_019f4800-0000-7000-8000-000000000023","type":"stepInterrupted","stepId":"step_019f4800-0000-7000-8000-000000000023","reasonCode":"futureInterrupt"}"#,
                "buddy state validation failed: unsupported sidecar interrupt reason code: futureInterrupt",
            ),
            (
                r#"{"protocolVersion":1,"type":"ready","stepId":"step_019f4800-0000-7000-8000-000000000010"}"#,
                "buddy state validation failed: unsupported sidecar step response type: ready",
            ),
            (
                r#"{"protocolVersion":2,"type":"stepCompleted","stepId":"step_019f4800-0000-7000-8000-000000000013","elapsedMs":1720}"#,
                "buddy state validation failed: unsupported sidecar protocol version: 2",
            ),
        ];

        for (line, expected_error) in cases {
            let error = parse_sidecar_step_response(line)
                .expect_err("unknown sidecar protocol value should be rejected");

            assert_eq!(error.to_string(), expected_error);
        }
    }
}
