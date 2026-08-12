use std::collections::BTreeSet;

use crate::{
    choreography::admission::ChoreographyTriggerSource,
    choreography::macro_plan::{is_public_macro_intent_params_valid, MacroIntent},
    storage::BuddyRunEvent,
};

use super::super::{read_json_string_field, runtime_events::CodexRuntimeOutput};

const BUDDY_ANIMATION_INTENT_START_TAG: &str = "<lexora_buddy_animation_intent>";
const BUDDY_ANIMATION_INTENT_END_TAG: &str = "</lexora_buddy_animation_intent>";
const BUDDY_HOST_ACTION_START_TAG: &str = "<lexora_buddy_host_action>";
const BUDDY_HOST_ACTION_END_TAG: &str = "</lexora_buddy_host_action>";
const BUDDY_HOST_ACTION_REASON_MAX_LEN: usize = 120;
const BUDDY_HOST_ACTION_SOURCE: &str = "buddy_builtin_host_skill";

pub(in crate::commands) struct BuddyHostAction {
    pub(super) intent: MacroIntent,
    pub(super) payload: serde_json::Value,
    pub(super) trigger_source: ChoreographyTriggerSource,
}

pub(super) fn collect_buddy_host_actions(
    runtime_output: &CodexRuntimeOutput,
    events: &[BuddyRunEvent],
) -> Vec<BuddyHostAction> {
    let mut seen = BTreeSet::new();
    let mut actions = Vec::new();
    let mut streamed_message = String::new();

    for event in events {
        if event.event_type != "message.delta" {
            continue;
        }
        if let Some(delta) = read_json_string_field(&event.payload, "delta") {
            streamed_message.push_str(&delta);
        }
    }

    for content in [&streamed_message, runtime_output.final_message.as_str()] {
        for action in extract_buddy_host_actions(content) {
            let key = serde_json::to_string(&action.payload).unwrap_or_default();
            if seen.insert(key) {
                actions.push(action);
            }
        }
    }

    actions
}

#[cfg(test)]
fn extract_buddy_host_action_payloads(content: &str) -> Vec<serde_json::Value> {
    extract_buddy_host_actions(content)
        .into_iter()
        .map(|action| action.payload)
        .collect()
}

fn extract_buddy_host_actions(content: &str) -> Vec<BuddyHostAction> {
    extract_buddy_json_tag_payloads(
        content,
        BUDDY_HOST_ACTION_START_TAG,
        BUDDY_HOST_ACTION_END_TAG,
    )
    .into_iter()
    .filter_map(normalize_buddy_host_action_payload)
    .collect()
}

fn extract_buddy_json_tag_payloads(
    content: &str,
    start_tag: &str,
    end_tag: &str,
) -> Vec<serde_json::Value> {
    let mut payloads = Vec::new();
    let mut remaining = content;

    while let Some(start_index) = remaining.find(start_tag) {
        let body_start = start_index + start_tag.len();
        let Some(end_offset) = remaining[body_start..].find(end_tag) else {
            break;
        };
        let body_end = body_start + end_offset;
        let body = remaining[body_start..body_end].trim();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            payloads.push(value);
        }
        let after_end = body_end + end_tag.len();
        remaining = &remaining[after_end..];
    }

    payloads
}

fn normalize_buddy_host_action_payload(value: serde_json::Value) -> Option<BuddyHostAction> {
    let object = value.as_object()?;
    if !buddy_json_object_protocol_version_is_supported(object) {
        return None;
    }

    match read_buddy_json_string(object, "action")? {
        "macroIntent" => normalize_buddy_host_macro_intent_action_payload(object),
        _ => None,
    }
}

fn normalize_buddy_host_macro_intent_action_payload(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<BuddyHostAction> {
    if !buddy_json_object_has_only_keys(
        object,
        &["action", "intent", "priority", "reason", "version"],
    ) {
        return None;
    }

    let intent = serde_json::from_value::<MacroIntent>(object.get("intent")?.clone()).ok()?;
    if !is_public_macro_intent_params_valid(&intent) {
        return None;
    }
    let mut payload = serde_json::json!({
        "version": 1,
        "action": "macroIntent",
        "intent": serde_json::to_value(&intent).ok()?,
        "source": BUDDY_HOST_ACTION_SOURCE,
    });
    append_buddy_host_common_fields(&mut payload, object)?;
    let trigger_source = match payload.get("priority").and_then(serde_json::Value::as_str) {
        Some("urgent") => ChoreographyTriggerSource::AttentionSystem,
        _ => ChoreographyTriggerSource::AiChoreography,
    };

    Some(BuddyHostAction {
        intent,
        payload,
        trigger_source,
    })
}

fn append_buddy_host_common_fields(
    payload: &mut serde_json::Value,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<()> {
    if let Some(priority_value) = object.get("priority") {
        let priority = priority_value.as_str()?;
        if !is_buddy_host_action_priority(priority) {
            return None;
        }
        payload["priority"] = serde_json::json!(priority);
    }
    if let Some(reason_value) = object.get("reason") {
        let reason = reason_value.as_str()?;
        if !is_buddy_host_action_reason(reason) {
            return None;
        }
        payload["reason"] = serde_json::json!(reason);
    }
    Some(())
}

fn read_buddy_json_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    object.get(key).and_then(serde_json::Value::as_str)
}

fn buddy_json_object_has_only_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed_keys: &[&str],
) -> bool {
    object
        .keys()
        .all(|key| allowed_keys.contains(&key.as_str()))
}

fn buddy_json_object_protocol_version_is_supported(
    object: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    object
        .get("version")
        .is_some_and(|version| version.as_u64() == Some(1))
}

fn is_buddy_host_action_priority(value: &str) -> bool {
    matches!(value, "background" | "normal" | "high" | "urgent")
}

fn is_buddy_host_action_reason(value: &str) -> bool {
    if value.is_empty() || value.len() > BUDDY_HOST_ACTION_REASON_MAX_LEN {
        return false;
    }

    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }

    let mut previous_was_underscore = false;
    for char in chars {
        if char == '_' {
            if previous_was_underscore {
                return false;
            }
            previous_was_underscore = true;
            continue;
        }
        if !char.is_ascii_lowercase() && !char.is_ascii_digit() {
            return false;
        }
        previous_was_underscore = false;
    }

    !previous_was_underscore
}

pub(in crate::commands) fn strip_buddy_host_action_blocks(content: &str) -> String {
    let stripped = strip_buddy_tagged_blocks(
        content,
        BUDDY_HOST_ACTION_START_TAG,
        BUDDY_HOST_ACTION_END_TAG,
    );
    strip_buddy_tagged_blocks(
        &stripped,
        BUDDY_ANIMATION_INTENT_START_TAG,
        BUDDY_ANIMATION_INTENT_END_TAG,
    )
}

fn strip_buddy_tagged_blocks(content: &str, start_tag: &str, end_tag: &str) -> String {
    let mut stripped = String::new();
    let mut remaining = content;

    loop {
        let Some(start_index) = remaining.find(start_tag) else {
            stripped.push_str(remaining);
            break;
        };
        stripped.push_str(&remaining[..start_index]);
        let body_start = start_index + start_tag.len();
        let Some(end_offset) = remaining[body_start..].find(end_tag) else {
            break;
        };
        let body_end = body_start + end_offset;
        let after_end = body_end + end_tag.len();
        remaining = &remaining[after_end..];
    }

    let stripped = stripped
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");

    collapse_consecutive_newlines(&stripped).trim().to_owned()
}

fn collapse_consecutive_newlines(value: &str) -> String {
    let mut output = String::new();
    let mut previous_was_newline = false;

    for char in value.chars() {
        if char == '\n' {
            if !previous_was_newline {
                output.push(char);
            }
            previous_was_newline = true;
        } else {
            output.push(char);
            previous_was_newline = false;
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::{extract_buddy_host_action_payloads, strip_buddy_host_action_blocks};

    #[test]
    fn ignores_direct_native_host_action_blocks_but_still_strips_them() {
        let content = r#"我会让桌宠庆祝。
<lexora_buddy_host_action>{"action":"sequence","steps":[{"type":"move","target":"center"},{"type":"animation","animation":"celebrate","durationMs":3000},{"type":"move","target":{"kind":"home"},"after":"sleep"}],"priority":"high","reason":"done"}</lexora_buddy_host_action>
继续保持安静。
<lexora_buddy_animation_intent>{"intent":"unknown","durationMs":3000}</lexora_buddy_animation_intent>"#;

        let payloads = extract_buddy_host_action_payloads(content);
        let stripped = strip_buddy_host_action_blocks(content);

        assert!(payloads.is_empty());
        assert_eq!(stripped, "我会让桌宠庆祝。\n继续保持安静。");
    }

    #[test]
    fn extracts_choreography_macro_intent_host_action() {
        let content = r#"<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"dance","params":{"durationMs":2500}},"reason":"user_requested_dance"}</lexora_buddy_host_action>"#;

        let payloads = extract_buddy_host_action_payloads(content);

        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["action"], "macroIntent");
        assert_eq!(payloads[0]["intent"]["macroId"], "dance");
        assert_eq!(payloads[0]["intent"]["params"]["durationMs"], 2500);
        assert_eq!(payloads[0]["reason"], "user_requested_dance");
        assert_eq!(payloads[0]["source"], "buddy_builtin_host_skill");
    }

    #[test]
    fn rejects_macro_intent_reason_outside_builtin_host_schema() {
        let invalid_reasons = vec![
            "User Requested".to_owned(),
            "user-requested".to_owned(),
            "user requested".to_owned(),
            "_user_requested".to_owned(),
            "user_requested_".to_owned(),
            "user__requested".to_owned(),
            " user_requested".to_owned(),
            "user_requested ".to_owned(),
            "a".repeat(121),
        ];

        for reason in invalid_reasons {
            let content = format!(
                r#"<lexora_buddy_host_action>{{"version":1,"action":"macroIntent","intent":{{"macroId":"dance","params":{{"durationMs":2500}}}},"reason":"{reason}"}}</lexora_buddy_host_action>"#
            );

            let payloads = extract_buddy_host_action_payloads(&content);

            assert!(payloads.is_empty(), "reason should be rejected: {reason}");
        }
    }

    #[test]
    fn rejects_macro_intent_priority_outside_builtin_host_schema() {
        let invalid_priorities = vec![
            serde_json::json!(""),
            serde_json::json!("urgent_now"),
            serde_json::json!("HIGH"),
            serde_json::json!(" high"),
            serde_json::json!("high "),
            serde_json::json!(1),
        ];

        for priority in invalid_priorities {
            let content = format!(
                r#"<lexora_buddy_host_action>{{"version":1,"action":"macroIntent","intent":{{"macroId":"dance","params":{{"durationMs":2500}}}},"priority":{priority}}}</lexora_buddy_host_action>"#
            );

            let payloads = extract_buddy_host_action_payloads(&content);

            assert!(
                payloads.is_empty(),
                "priority should be rejected: {priority}"
            );
        }
    }

    #[test]
    fn rejects_macro_intent_action_outside_exact_wire_schema() {
        let invalid_actions = [" macroIntent", "macroIntent ", "MacroIntent"];

        for action in invalid_actions {
            let content = format!(
                r#"<lexora_buddy_host_action>{{"version":1,"action":"{action}","intent":{{"macroId":"dance","params":{{"durationMs":2500}}}}}}</lexora_buddy_host_action>"#
            );

            let payloads = extract_buddy_host_action_payloads(&content);

            assert!(payloads.is_empty(), "action should be rejected: {action}");
        }
    }

    #[test]
    fn rejects_macro_intent_payloads_outside_public_wire_schema() {
        let cases = [
            (
                "missing version",
                r#"<lexora_buddy_host_action>{"action":"macroIntent","intent":{"macroId":"dance","params":{"durationMs":2500}}}</lexora_buddy_host_action>"#,
            ),
            (
                "extra top-level field",
                r#"<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"dance","params":{"durationMs":2500}},"previewTimeline":true}</lexora_buddy_host_action>"#,
            ),
            (
                "unsupported version",
                r#"<lexora_buddy_host_action>{"action":"macroIntent","intent":{"macroId":"dance","params":{"durationMs":2500}},"version":2}</lexora_buddy_host_action>"#,
            ),
            (
                "dance duration below minimum",
                r#"<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"dance","params":{"durationMs":0}}}</lexora_buddy_host_action>"#,
            ),
            (
                "patrol loops below minimum",
                r#"<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"patrolAroundScreen","params":{"loops":0}}}</lexora_buddy_host_action>"#,
            ),
            (
                "peek duration above maximum",
                r#"<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"peekBehindWindow","params":{"windowSelector":{"kind":"activeWindow"},"edge":"left","reveal":"head","durationMs":30000}}}</lexora_buddy_host_action>"#,
            ),
        ];

        for (label, content) in cases {
            assert!(
                extract_buddy_host_action_payloads(content).is_empty(),
                "payload should be rejected: {label}"
            );
        }
    }
}
