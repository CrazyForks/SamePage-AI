use std::sync::Mutex;

use super::{append_buddy_host_action_events, execute_buddy_host_actions};
use crate::{
    app_paths::BuddyAppPaths,
    choreography::{
        executor::ChoreographyStepExecutor,
        registry::StepResolution,
        timeline::{MoveByPathStep, MoveToStep, PlayActionStep, WaitStep},
    },
    commands::{run_state::BuddyRunStateEventPublisher, runtime_events::CodexRuntimeOutput},
    error::BuddyResult,
    state::BuddyAppState,
    storage::{
        ActionLogPlanListRequest, BuddyStorage, CreateBuddyRunEventRequest, CreateBuddyRunRequest,
        CreateBuddySessionRequest,
    },
};

#[test]
fn appends_host_action_event_from_streamed_message_delta() {
    let storage = create_host_action_test_storage();
    let run = create_host_action_test_run(&storage);
    let message_delta = storage
        .append_run_event(CreateBuddyRunEventRequest::projected(
            run.id.clone(),
            "message.delta",
            serde_json::json!({
                "delta": "处理中 <lexora_buddy_host_action>{\"version\":1,\"action\":\"macroIntent\",\"intent\":{\"macroId\":\"dance\",\"params\":{\"durationMs\":2500}}}</lexora_buddy_host_action>",
                "itemId": "message-1",
                "protocol": "codex_app_server",
                "threadId": "thread-1",
                "turnId": "turn-1",
            }),
        ))
        .expect("append message delta");
    let mut events = vec![message_delta];
    let runtime_output = CodexRuntimeOutput {
        final_memory_citation: None,
        final_message: "已处理。".to_owned(),
        protocol: "codex_app_server",
        stdout_bytes: None,
        thread_id: Some("thread-1".to_owned()),
        turn_id: Some("turn-1".to_owned()),
    };

    append_buddy_host_action_events(
        &storage,
        &run.id,
        &mut events,
        None,
        &BuddyRunStateEventPublisher::disabled(),
        &runtime_output,
    )
    .expect("append host action");
    let stored_events = storage
        .list_run_events(run.id, None, 10)
        .expect("list events");
    let host_action_event = stored_events
        .iter()
        .find(|event| event.event_type == "host.action")
        .expect("host action event");

    assert_eq!(host_action_event.payload["action"], "macroIntent");
    assert_eq!(
        host_action_event.payload["source"],
        "buddy_builtin_host_skill"
    );
    assert_eq!(host_action_event.payload["intent"]["macroId"], "dance");
    assert_eq!(
        host_action_event.payload["intent"]["params"]["durationMs"],
        2500
    );
}

#[test]
fn executes_host_macro_intent_through_choreography_admission() {
    let data_dir = std::env::temp_dir().join(format!(
        "lexora-buddy-host-action-execution-{}",
        uuid::Uuid::new_v4()
    ));
    let state =
        BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
            .expect("initialize state");
    state
        .mark_choreography_runtime_ready("2026-08-05T00:00:00.000Z")
        .expect("mark choreography ready");
    let storage = state.storage_handle();
    let run = create_host_action_test_run(&storage);
    let mut events = Vec::new();
    let runtime_output = CodexRuntimeOutput {
        final_memory_citation: None,
        final_message: r#"已完成。<lexora_buddy_host_action>{"version":1,"action":"macroIntent","intent":{"macroId":"celebrate","params":{}},"reason":"task_completed"}</lexora_buddy_host_action>"#.to_owned(),
        protocol: "codex_app_server",
        stdout_bytes: None,
        thread_id: Some("thread-1".to_owned()),
        turn_id: Some("turn-1".to_owned()),
    };
    let executor = RecordingStepExecutor::default();

    let actions = append_buddy_host_action_events(
        &storage,
        &run.id,
        &mut events,
        None,
        &BuddyRunStateEventPublisher::disabled(),
        &runtime_output,
    )
    .expect("append host action");
    execute_buddy_host_actions(&state, &run.id, actions, &executor).expect("execute host actions");

    let plan = storage
        .list_action_log_plans(ActionLogPlanListRequest {
            source_ref_id: Some(run.id.clone()),
            source_ref_kind: Some("run".to_owned()),
            ..ActionLogPlanListRequest::default()
        })
        .expect("list host action plans")
        .items
        .into_iter()
        .next()
        .expect("host action plan");

    assert_eq!(plan.status, "completed");
    assert!(!executor.played_animation_refs().is_empty());
    std::fs::remove_dir_all(data_dir).expect("cleanup fixture");
}

#[derive(Default)]
struct RecordingStepExecutor {
    played_animation_refs: Mutex<Vec<String>>,
}

impl RecordingStepExecutor {
    fn played_animation_refs(&self) -> Vec<String> {
        self.played_animation_refs
            .lock()
            .expect("played animations lock")
            .clone()
    }
}

impl ChoreographyStepExecutor for RecordingStepExecutor {
    fn play_action_step(
        &self,
        _step: &PlayActionStep,
        resolution: &StepResolution,
    ) -> BuddyResult<()> {
        self.played_animation_refs
            .lock()
            .expect("played animations lock")
            .push(resolution.animation_ref.clone());
        Ok(())
    }

    fn move_to_step(
        &self,
        _step: &MoveToStep,
        _after_animation_ref: Option<&str>,
    ) -> BuddyResult<()> {
        Ok(())
    }

    fn move_by_path_step(
        &self,
        _step: &MoveByPathStep,
        _after_animation_ref: Option<&str>,
    ) -> BuddyResult<()> {
        Ok(())
    }

    fn wait_step(&self, _step: &WaitStep) -> BuddyResult<()> {
        Ok(())
    }

    fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
        Ok(())
    }

    fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
        Ok(None)
    }
}

fn create_host_action_test_storage() -> BuddyStorage {
    BuddyStorage::new_temporary_for_test().expect("create storage")
}

fn create_host_action_test_run(storage: &BuddyStorage) -> crate::storage::BuddyRun {
    let session = storage
        .create_session(CreateBuddySessionRequest {
            runtime: "codex".to_owned(),
            project_root: None,
            scope: "global".to_owned(),
            title: Some("Host action event".to_owned()),
        })
        .expect("create session");

    storage
        .create_run(CreateBuddyRunRequest {
            runtime: "codex".to_owned(),
            cwd: Some("/tmp/lexora-project".to_owned()),
            external_run_id: None,
            external_thread_id: None,
            session_id: session.id,
        })
        .expect("create run")
}
