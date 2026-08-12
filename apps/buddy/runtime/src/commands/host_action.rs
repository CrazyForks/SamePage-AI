mod payloads;
#[cfg(test)]
mod tests;

pub(super) use payloads::strip_buddy_host_action_blocks;

use crate::{
    choreography::{
        affective::{AffectiveContextStore, ResolveContext},
        command::{run_choreography_macro_intent_with_source_admission, MacroIntentRunSource},
        executor::{ChoreographyStepExecutor, TimelineExecutionError},
    },
    domain::BuddyRunEventType,
    error::BuddyError,
    state::BuddyAppState,
    storage::{BuddyRunEvent, BuddyStorage, CreateBuddyRunEventRequest},
};

use super::{run_state::BuddyRunStateEventPublisher, runtime_events::CodexRuntimeOutput};
use payloads::{collect_buddy_host_actions, BuddyHostAction};

pub(super) fn append_buddy_host_action_events(
    storage: &BuddyStorage,
    run_id: &str,
    events: &mut Vec<BuddyRunEvent>,
    session_id: Option<&str>,
    event_publisher: &BuddyRunStateEventPublisher,
    runtime_output: &CodexRuntimeOutput,
) -> Result<Vec<BuddyHostAction>, BuddyError> {
    let actions = collect_buddy_host_actions(runtime_output, events);
    for action in &actions {
        let event = storage.append_run_event(CreateBuddyRunEventRequest::new(
            run_id,
            BuddyRunEventType::HostAction,
            action.payload.clone(),
        ))?;
        event_publisher.emit_event(&event, session_id);
        events.push(event);
    }

    Ok(actions)
}

pub(super) fn execute_buddy_host_actions(
    state: &BuddyAppState,
    run_id: &str,
    actions: Vec<BuddyHostAction>,
    executor: &impl ChoreographyStepExecutor,
) -> Result<(), BuddyError> {
    for action in actions {
        let storage = state.storage_handle();
        let resolve_context = AffectiveContextStore::from_buddy_home(state.data_dir_path())
            .read_or_create_default_with_diagnostics(&storage)
            .map(ResolveContext::from_affective_snapshot)?;

        state.with_choreography_admission(|admission| {
            run_choreography_macro_intent_with_source_admission(
                action.intent,
                storage,
                executor,
                admission,
                resolve_context,
                MacroIntentRunSource {
                    source_ref: serde_json::json!({
                        "kind": "run",
                        "runId": run_id,
                    }),
                    trigger_source: action.trigger_source,
                },
            )
            .map(|_| ())
            .map_err(map_timeline_execution_error)
        })?;
    }

    Ok(())
}

fn map_timeline_execution_error(error: TimelineExecutionError) -> BuddyError {
    match error {
        TimelineExecutionError::ActionLog(error) | TimelineExecutionError::Execution(error) => {
            error
        }
    }
}
