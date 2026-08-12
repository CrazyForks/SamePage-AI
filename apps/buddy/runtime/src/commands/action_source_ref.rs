use crate::{
    error::BuddyError,
    storage::{BuddyApproval, BuddyRun, BuddyStorage},
};

pub(in crate::commands) fn normalize_action_log_source_ref(
    storage: &BuddyStorage,
    source_ref: Option<serde_json::Value>,
) -> Result<Option<serde_json::Value>, BuddyError> {
    let Some(source_ref) = source_ref else {
        return Ok(None);
    };
    let object = source_ref
        .as_object()
        .ok_or_else(|| BuddyError::Validation("action sourceRef must be an object".to_owned()))?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .ok_or_else(|| BuddyError::Validation("action sourceRef requires kind".to_owned()))?;
    let normalized = match kind {
        "conversationMessage" => normalize_conversation_message_source_ref(object)?,
        "run" => normalize_run_source_ref(storage, object)?,
        "approval" => normalize_approval_source_ref(storage, object)?,
        "presetBehavior" => normalize_preset_behavior_source_ref(object)?,
        _ => {
            return Err(BuddyError::Validation(format!(
                "unsupported action sourceRef: {kind}"
            )));
        }
    };

    Ok(Some(normalized))
}

fn normalize_run_source_ref(
    storage: &BuddyStorage,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, BuddyError> {
    reject_action_source_ref_unknown_fields(object, "run", &["kind", "runId"])?;

    let run_id = required_action_source_ref_string(object, "runId")?.to_owned();
    let run = find_source_ref_run(storage, &run_id)?;

    if let Some(run) = run.as_ref() {
        if let (Some(conversation_id), Some(message_id)) = (
            run.conversation_id.as_deref(),
            run.triggering_message_id.as_deref(),
        ) {
            return Ok(serde_json::json!({
                "kind": "conversationMessage",
                "conversationId": conversation_id,
                "messageId": message_id,
                "runId": run_id,
            }));
        }
    }

    let mut normalized = serde_json::json!({
        "kind": "run",
        "runId": run_id,
    });
    if let Some(conversation_id) = run.and_then(|run| run.conversation_id) {
        normalized["conversationId"] = serde_json::json!(conversation_id);
    }

    Ok(normalized)
}

fn find_source_ref_run(
    storage: &BuddyStorage,
    run_id: &str,
) -> Result<Option<BuddyRun>, BuddyError> {
    match storage.find_run(run_id.to_owned()) {
        Ok(run) => Ok(Some(run)),
        Err(BuddyError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn normalize_conversation_message_source_ref(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, BuddyError> {
    reject_action_source_ref_unknown_fields(
        object,
        "conversationMessage",
        &["kind", "conversationId", "messageId", "runId"],
    )?;

    let conversation_id = required_action_source_ref_string(object, "conversationId")?;
    let message_id = required_action_source_ref_string(object, "messageId")?;
    let mut normalized = serde_json::json!({
        "kind": "conversationMessage",
        "conversationId": conversation_id,
        "messageId": message_id,
    });
    if let Some(run_id) = optional_action_source_ref_string(object, "runId") {
        normalized["runId"] = serde_json::json!(run_id);
    }

    Ok(normalized)
}

fn normalize_approval_source_ref(
    storage: &BuddyStorage,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, BuddyError> {
    reject_action_source_ref_unknown_fields(object, "approval", &["kind", "approvalId", "runId"])?;

    let approval_id = required_action_source_ref_string(object, "approvalId")?.to_owned();
    let mut normalized = serde_json::json!({
        "kind": "approval",
        "approvalId": approval_id,
    });
    if let Some(run_id) = optional_action_source_ref_string(object, "runId") {
        normalized["runId"] = serde_json::json!(run_id);
        return Ok(normalized);
    }
    if let Some(approval) = find_source_ref_approval(storage, &approval_id)? {
        if let Some(run_id) = approval.run_id {
            normalized["runId"] = serde_json::json!(run_id);
        }
    }

    Ok(normalized)
}

fn find_source_ref_approval(
    storage: &BuddyStorage,
    approval_id: &str,
) -> Result<Option<BuddyApproval>, BuddyError> {
    match storage.find_approval(approval_id.to_owned()) {
        Ok(approval) => Ok(Some(approval)),
        Err(BuddyError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn normalize_preset_behavior_source_ref(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, BuddyError> {
    reject_action_source_ref_unknown_fields(
        object,
        "presetBehavior",
        &["kind", "presetBehaviorId", "interactionId", "sessionId"],
    )?;

    let preset_behavior_id = required_action_source_ref_string(object, "presetBehaviorId")?;
    let mut normalized = serde_json::json!({
        "kind": "presetBehavior",
        "presetBehaviorId": preset_behavior_id,
    });
    if let Some(interaction_id) = optional_action_source_ref_string(object, "interactionId") {
        normalized["interactionId"] = serde_json::json!(interaction_id);
    }
    if let Some(session_id) = optional_action_source_ref_string(object, "sessionId") {
        normalized["sessionId"] = serde_json::json!(session_id);
    }

    Ok(normalized)
}

fn reject_action_source_ref_unknown_fields(
    object: &serde_json::Map<String, serde_json::Value>,
    kind: &str,
    allowed_fields: &[&str],
) -> Result<(), BuddyError> {
    for field in object.keys() {
        if !allowed_fields.contains(&field.as_str()) {
            return Err(BuddyError::Validation(format!(
                "action sourceRef kind={kind} field={field} is not allowed"
            )));
        }
    }

    Ok(())
}

fn required_action_source_ref_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, BuddyError> {
    optional_action_source_ref_string(object, key)
        .ok_or_else(|| BuddyError::Validation(format!("action sourceRef requires {key}")))
}

fn optional_action_source_ref_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use crate::storage::BuddyStorage;

    use super::normalize_action_log_source_ref;

    #[test]
    fn normalize_rejects_fields_outside_each_source_ref_schema() {
        let cases = [
            (
                serde_json::json!({
                    "kind": "conversationMessage",
                    "conversationId": "conversation_source_ref_strict",
                    "messageId": "message_source_ref_strict",
                    "content": "user message body must not ride along",
                }),
                "buddy state validation failed: action sourceRef kind=conversationMessage field=content is not allowed",
            ),
            (
                serde_json::json!({
                    "kind": "run",
                    "runId": "run_source_ref_strict",
                    "messageId": "message_source_ref_strict",
                }),
                "buddy state validation failed: action sourceRef kind=run field=messageId is not allowed",
            ),
            (
                serde_json::json!({
                    "kind": "approval",
                    "approvalId": "approval_019f",
                    "promptPreview": "approval prompt must not ride along",
                }),
                "buddy state validation failed: action sourceRef kind=approval field=promptPreview is not allowed",
            ),
            (
                serde_json::json!({
                    "kind": "presetBehavior",
                    "presetBehaviorId": "throw_after_drag",
                    "messageId": "message_source_ref_strict",
                }),
                "buddy state validation failed: action sourceRef kind=presetBehavior field=messageId is not allowed",
            ),
        ];

        for (source_ref, expected_error) in cases {
            let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
            let error = normalize_action_log_source_ref(&storage, Some(source_ref))
                .expect_err("sourceRef extra fields should be rejected");

            assert_eq!(error.to_string(), expected_error);
        }
    }

    #[test]
    fn normalize_accepts_preset_behavior_source_ref() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        let source_ref = normalize_action_log_source_ref(
            &storage,
            Some(serde_json::json!({
                "kind": "presetBehavior",
                "presetBehaviorId": "throw_after_drag",
                "interactionId": "interaction_019f",
                "sessionId": "session_019f",
            })),
        )
        .expect("presetBehavior sourceRef should normalize")
        .expect("sourceRef");

        assert_eq!(
            source_ref,
            serde_json::json!({
                "kind": "presetBehavior",
                "presetBehaviorId": "throw_after_drag",
                "interactionId": "interaction_019f",
                "sessionId": "session_019f",
            })
        );
    }

    #[test]
    fn normalize_accepts_approval_source_ref() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");

        let source_ref = normalize_action_log_source_ref(
            &storage,
            Some(serde_json::json!({
                "kind": "approval",
                "approvalId": "approval_019f",
                "runId": "run_019f",
            })),
        )
        .expect("approval sourceRef should normalize")
        .expect("sourceRef");

        assert_eq!(
            source_ref,
            serde_json::json!({
                "kind": "approval",
                "approvalId": "approval_019f",
                "runId": "run_019f",
            })
        );
    }
}
