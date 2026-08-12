use super::choreography_pending_bodies::ChoreographyPendingExecutionBody;
use super::*;
use crate::choreography::{
    action_log::ActionLogEvent,
    admission::{
        ChoreographyAdmissionDecision, ChoreographyPlanPriority, ChoreographyTriggerSource,
    },
};
use crate::domain::{
    BuddyApprovalTerminalStatus, BuddyRunEventType, BuddyRunStatus, BuddyRunTerminalStatus,
};
use crate::native_pet::step_protocol::SidecarInterruptPolicy;
use rusqlite::Connection;

#[test]
fn resets_stale_development_database_to_current_schema() {
    let buddy_home = create_storage_test_dir("lexora-buddy-stale-db");
    let database_path = buddy_home.join("sqlite").join("state.sqlite3");
    std::fs::create_dir_all(database_path.parent().expect("sqlite parent"))
        .expect("create sqlite dir");
    {
        let connection = Connection::open(&database_path).expect("open stale database");
        connection
            .execute_batch(
                r#"
                    CREATE TABLE stale_data(id TEXT PRIMARY KEY);
                    PRAGMA user_version = 1;
                    "#,
            )
            .expect("create stale database");
    }

    let storage = BuddyStorage::new_fixed_for_test(database_path, buddy_home.clone());
    let status = storage.initialize().expect("initialize storage");
    let connection = storage.open_connection().expect("open current database");
    let stale_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'stale_data')",
            [],
            |row| row.get(0),
        )
        .expect("read sqlite_master");

    assert_eq!(status.schema_version(), CURRENT_SCHEMA_VERSION);
    assert!(!stale_exists);
    std::fs::remove_dir_all(buddy_home).expect("cleanup buddy home");
}

#[test]
fn rebuilds_conversation_run_cycle_from_jsonl_into_empty_projection() {
    let buddy_home = create_storage_test_dir("lexora-buddy-jsonl-projection-rebuild");
    let database_path = buddy_home.join("sqlite").join("state.sqlite3");
    let storage = BuddyStorage::new_fixed_for_test(database_path.clone(), buddy_home.clone());
    storage.initialize().expect("initialize storage");
    let conversation = storage
        .create_conversation(CreateBuddyConversationRequest {
            forked_from_message_id: None,
            project_root: None,
            scope: "global".into(),
            source_conversation_id: None,
            source_run_id: None,
            title: Some("JSONL projection rebuild".into()),
        })
        .expect("create conversation");
    let user_message = storage
        .append_conversation_message(AppendBuddyConversationMessageRequest {
            attachments: Vec::new(),
            branch_id: conversation.active_branch_id.clone(),
            content: "rebuild this conversation".into(),
            conversation_id: conversation.id.clone(),
            parent_message_id: None,
            role: "user".into(),
            run_id: None,
            version_group_id: None,
            version_index: 1,
            version_status: "active".into(),
        })
        .expect("append user message");
    let run = storage
        .create_conversation_run(CreateBuddyConversationRunRequest {
            branch_id: conversation.active_branch_id.clone(),
            conversation_id: conversation.id.clone(),
            cwd: None,
            external_run_id: None,
            external_thread_id: None,
            intent: "buddy.agent.turn".into(),
            runtime: "codex".into(),
            triggering_message_id: user_message.id.clone(),
        })
        .expect("create conversation run");
    let assistant_message = storage
        .append_conversation_message(AppendBuddyConversationMessageRequest {
            attachments: Vec::new(),
            branch_id: conversation.active_branch_id.clone(),
            content: "projection restored".into(),
            conversation_id: conversation.id.clone(),
            parent_message_id: Some(user_message.id.clone()),
            role: "assistant".into(),
            run_id: Some(run.id.clone()),
            version_group_id: None,
            version_index: 1,
            version_status: "active".into(),
        })
        .expect("append assistant message");
    storage
        .finish_run(
            run.id.clone(),
            BuddyRunTerminalStatus::Completed,
            serde_json::json!({ "status": "ok" }),
        )
        .expect("finish run");
    storage
        .open_connection()
        .expect("open projection")
        .pragma_update(None, "user_version", 1)
        .expect("mark projection stale");

    let rebuilt = BuddyStorage::new_fixed_for_test(database_path, buddy_home.clone());
    rebuilt.initialize().expect("rebuild projection from JSONL");
    let messages = rebuilt
        .list_active_conversation_messages(conversation.id.clone(), 10)
        .expect("list rebuilt messages");
    let rebuilt_run = rebuilt.find_run(run.id.clone()).expect("find rebuilt run");

    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages
            .iter()
            .find(|message| message.id == assistant_message.id)
            .and_then(|message| message.run_id.as_deref()),
        Some(run.id.as_str())
    );
    assert_eq!(
        rebuilt_run.triggering_message_id.as_deref(),
        Some(user_message.id.as_str())
    );
    std::fs::remove_dir_all(buddy_home).expect("cleanup buddy home");
}

#[test]
fn startup_fails_interrupted_runs_and_persists_the_terminal_event() {
    let buddy_home = create_storage_test_dir("lexora-buddy-interrupted-runs");
    let database_path = buddy_home.join("sqlite").join("state.sqlite3");
    let storage = BuddyStorage::new_fixed_for_test(database_path.clone(), buddy_home.clone());
    storage.initialize().expect("initialize storage");
    let session = storage
        .create_session(CreateBuddySessionRequest {
            scope: "global".into(),
            runtime: "codex".into(),
            project_root: None,
            title: None,
        })
        .expect("create session");
    let queued_run = storage
        .create_run(CreateBuddyRunRequest {
            session_id: session.id.clone(),
            runtime: "codex".into(),
            cwd: None,
            external_thread_id: None,
            external_run_id: None,
        })
        .expect("create queued run");
    let running_run = storage
        .create_run(CreateBuddyRunRequest {
            session_id: session.id.clone(),
            runtime: "codex".into(),
            cwd: None,
            external_thread_id: None,
            external_run_id: None,
        })
        .expect("create running run");
    storage
        .update_run_status(running_run.id.clone(), BuddyRunStatus::Running)
        .expect("mark run running");
    let queued_approval = storage
        .create_approval(CreateBuddyApprovalRequest {
            run_id: Some(queued_run.id.clone()),
            kind: CODEX_APP_SERVER_REQUEST_APPROVAL_KIND.into(),
            payload: serde_json::json!({ "messageId": "queued-message" }),
        })
        .expect("create queued run approval");
    let running_approval = storage
        .create_approval(CreateBuddyApprovalRequest {
            run_id: Some(running_run.id.clone()),
            kind: CODEX_APP_SERVER_REQUEST_APPROVAL_KIND.into(),
            payload: serde_json::json!({ "method": "item/commandExecution/requestApproval" }),
        })
        .expect("create running run approval");

    let restarted = BuddyStorage::new_fixed_for_test(database_path.clone(), buddy_home.clone());
    restarted.initialize().expect("restart storage");

    for run_id in [&queued_run.id, &running_run.id] {
        let run = restarted
            .find_run(run_id.clone())
            .expect("find interrupted run");
        let events = restarted
            .list_run_events(run_id.clone(), None, 10)
            .expect("list interrupted run events");

        assert_eq!(run.status, "failed");
        assert!(run.completed_at.is_some());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "run.failed");
        assert_eq!(events[0].payload["reason"], "runtime_restarted");
    }
    for approval_id in [&queued_approval.id, &running_approval.id] {
        let approval = restarted
            .find_approval(approval_id.clone())
            .expect("find interrupted run approval");
        assert_eq!(approval.status, "cancelled");
        assert!(approval.resolved_at.is_some());
    }

    let restarted_again = BuddyStorage::new_fixed_for_test(database_path, buddy_home.clone());
    restarted_again.initialize().expect("restart storage again");
    for run_id in [&queued_run.id, &running_run.id] {
        let events = restarted_again
            .list_run_events(run_id.clone(), None, 10)
            .expect("list terminal events after second restart");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "run.failed");
    }

    std::fs::remove_dir_all(buddy_home).expect("cleanup buddy home");
}

#[test]
fn rejects_project_sessions_without_authorized_project() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let error = storage
        .create_session(CreateBuddySessionRequest {
            scope: "project".into(),
            runtime: "codex".into(),
            project_root: Some("/tmp/lexora".into()),
            title: Some("Project".into()),
        })
        .expect_err("project should require authorization");

    assert!(error.to_string().contains("project is not authorized yet"));
}

#[test]
fn rejects_project_authorization_for_missing_directory() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let missing_root = std::env::temp_dir()
        .join(format!("lexora-buddy-missing-{}", uuid::Uuid::new_v4()))
        .join("project");

    let error = storage
        .upsert_project(UpsertBuddyProjectRequest {
            root: missing_root.to_string_lossy().into_owned(),
            name: Some("Missing".into()),
        })
        .expect_err("missing project root should be rejected");

    assert!(error
        .to_string()
        .contains("project root must be an existing directory"));
}

#[test]
fn stores_replaces_and_deletes_choreography_pending_execution_body() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let plan_id = "plan_pending_body_storage_019f6000-0000-7000-8000-000000000001";
    let timeline_body = serde_json::json!({
        "schemaVersion": 1,
        "plan": {
            "planId": plan_id,
            "steps": []
        }
    });

    let stored = storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: timeline_body.clone(),
        })
        .expect("store pending body");

    assert_eq!(stored.plan_id, plan_id);
    assert_eq!(
        stored.body_kind,
        ChoreographyPendingExecutionBodyKind::Timeline
    );
    assert_eq!(stored.schema_version, 1);
    assert_eq!(stored.body, timeline_body);

    let fixture_body = serde_json::json!({
        "schemaVersion": 1,
        "fixtureKind": "aiMacroDemo"
    });
    let replaced = storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::DevFixture,
            schema_version: 1,
            body: fixture_body.clone(),
        })
        .expect("replace pending body");

    assert_eq!(
        replaced.body_kind,
        ChoreographyPendingExecutionBodyKind::DevFixture
    );
    assert_eq!(replaced.body, fixture_body);

    let found: ChoreographyPendingExecutionBody = storage
        .find_choreography_pending_execution_body(plan_id)
        .expect("find pending body")
        .expect("pending body exists");
    assert_eq!(found, replaced);

    assert!(storage
        .delete_choreography_pending_execution_body(plan_id)
        .expect("delete pending body"));
    assert!(storage
        .find_choreography_pending_execution_body(plan_id)
        .expect("find after delete")
        .is_none());
    assert!(!storage
        .delete_choreography_pending_execution_body(plan_id)
        .expect("delete missing pending body"));
}

#[test]
fn records_choreography_pending_execution_body_lifecycle_as_jsonl_facts() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let plan_id = "plan_pending_body_fact_019f6000-0000-7000-8000-000000000011";
    let timeline_body = serde_json::json!({
        "schemaVersion": 1,
        "plan": {
            "planId": plan_id,
            "steps": []
        }
    });
    let fixture_body = serde_json::json!({
        "schemaVersion": 1,
        "fixtureKind": "singlePlayAction"
    });

    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: timeline_body.clone(),
        })
        .expect("store timeline pending body");
    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::DevFixture,
            schema_version: 1,
            body: fixture_body.clone(),
        })
        .expect("replace with dev fixture pending body");
    assert!(storage
        .delete_choreography_pending_execution_body(plan_id)
        .expect("delete pending body"));
    assert!(!storage
        .delete_choreography_pending_execution_body(plan_id)
        .expect("delete missing pending body"));

    let system_events = storage
        .query_action_log_system_events(ActionLogSystemEventQueryRequest {
            source_ref_kind: Some("choreographyScheduler".to_owned()),
            limit: Some(10),
            ..ActionLogSystemEventQueryRequest::default()
        })
        .expect("query pending body system facts");
    let event_types = system_events
        .items
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            "choreographyScheduler.pendingBodyDeleted",
            "choreographyScheduler.pendingBodyStored",
            "choreographyScheduler.pendingBodyStored",
        ]
    );

    let jsonl_lines = storage.read_action_log_jsonl_lines_for_test();
    assert_eq!(jsonl_lines.len(), 3);
    let stored_timeline =
        serde_json::from_str::<serde_json::Value>(&jsonl_lines[0]).expect("parse timeline fact");
    let stored_dev_fixture =
        serde_json::from_str::<serde_json::Value>(&jsonl_lines[1]).expect("parse fixture fact");
    let deleted =
        serde_json::from_str::<serde_json::Value>(&jsonl_lines[2]).expect("parse deleted fact");

    assert_eq!(
        stored_timeline.get("eventType"),
        Some(&serde_json::json!(
            "choreographyScheduler.pendingBodyStored"
        ))
    );
    assert_eq!(
        stored_timeline
            .get("payload")
            .and_then(|payload| payload.get("body")),
        Some(&timeline_body)
    );
    assert_eq!(
        stored_dev_fixture
            .get("payload")
            .and_then(|payload| payload.get("bodyKind")),
        Some(&serde_json::json!("devFixture"))
    );
    assert_eq!(
        deleted.get("eventType"),
        Some(&serde_json::json!(
            "choreographyScheduler.pendingBodyDeleted"
        ))
    );
    assert_eq!(
        deleted
            .get("payload")
            .and_then(|payload| payload.get("planId")),
        Some(&serde_json::json!(plan_id))
    );
}

#[test]
fn rebuilds_choreography_pending_execution_body_cache_from_jsonl_facts() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let kept_plan_id = "plan_pending_body_rebuild_019f6000-0000-7000-8000-000000000021";
    let deleted_plan_id = "plan_pending_body_rebuild_019f6000-0000-7000-8000-000000000022";
    let initial_timeline_body = serde_json::json!({
        "schemaVersion": 1,
        "plan": {
            "planId": kept_plan_id,
            "steps": []
        }
    });
    let final_fixture_body = serde_json::json!({
        "schemaVersion": 1,
        "fixtureKind": "aiMacroDemo"
    });
    let deleted_timeline_body = serde_json::json!({
        "schemaVersion": 1,
        "plan": {
            "planId": deleted_plan_id,
            "steps": []
        }
    });

    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: kept_plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: initial_timeline_body,
        })
        .expect("store initial pending body");
    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: deleted_plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: deleted_timeline_body,
        })
        .expect("store deleted pending body");
    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: kept_plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::DevFixture,
            schema_version: 1,
            body: final_fixture_body.clone(),
        })
        .expect("replace kept pending body");
    assert!(storage
        .delete_choreography_pending_execution_body(deleted_plan_id)
        .expect("delete second pending body"));
    let jsonl_line_count = storage.read_action_log_jsonl_lines_for_test().len();

    assert_eq!(
        storage
            .clear_choreography_pending_execution_bodies()
            .expect("clear sqlite cache"),
        1
    );
    assert!(storage
        .find_choreography_pending_execution_body(kept_plan_id)
        .expect("find after clear")
        .is_none());

    let rebuilt_count = storage
        .rebuild_choreography_pending_execution_body_cache_from_action_log()
        .expect("rebuild pending body cache from JSONL facts");
    let rebuilt = storage
        .find_choreography_pending_execution_body(kept_plan_id)
        .expect("find rebuilt body")
        .expect("rebuilt pending body exists");

    assert_eq!(rebuilt_count, 1);
    assert_eq!(
        rebuilt.body_kind,
        ChoreographyPendingExecutionBodyKind::DevFixture
    );
    assert_eq!(rebuilt.body, final_fixture_body);
    assert!(storage
        .find_choreography_pending_execution_body(deleted_plan_id)
        .expect("find deleted body after rebuild")
        .is_none());
    assert_eq!(
        storage.read_action_log_jsonl_lines_for_test().len(),
        jsonl_line_count
    );
}

#[test]
fn rebuild_does_not_restore_pending_execution_body_after_terminal_plan_event() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let plan_id = "plan_pending_body_rebuild_019f6000-0000-7000-8000-000000000031";
    let pending_body = serde_json::json!({
        "schemaVersion": 1,
        "fixtureKind": "aiMacroDemo"
    });

    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::DevFixture,
            schema_version: 1,
            body: pending_body,
        })
        .expect("store pending body");
    storage
        .append_choreography_action_log_event(
            &ActionLogEvent::plan_interrupted_after_runtime_restart(
                "evt_pending_body_rebuild_terminal_plan",
                plan_id,
                serde_json::json!({
                    "kind": "devFixture",
                    "fixtureName": "terminal-pending-body",
                }),
                "deferred",
                "executor.deferred",
                "admission.waitingForActiveStepToFinish",
                "2026-07-13T00:00:00.000Z",
            ),
        )
        .expect("append terminal plan event");
    storage
        .clear_choreography_pending_execution_bodies()
        .expect("clear sqlite cache");

    let rebuilt_count = storage
        .rebuild_choreography_pending_execution_body_cache_from_action_log()
        .expect("rebuild pending body cache from JSONL facts");

    assert_eq!(rebuilt_count, 0);
    assert!(storage
        .find_choreography_pending_execution_body(plan_id)
        .expect("find terminal pending body")
        .is_none());
}

#[test]
fn finds_replayable_pending_execution_body_from_jsonl_after_restart_interruption() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let plan_id = "plan_pending_body_replay_019f6000-0000-7000-8000-000000000051";
    let pending_body = serde_json::json!({
        "schemaVersion": 1,
        "plan": {
            "planId": plan_id,
            "steps": []
        }
    });

    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: pending_body.clone(),
        })
        .expect("store pending body");
    storage
        .append_choreography_action_log_event(
            &ActionLogEvent::plan_interrupted_after_runtime_restart(
                "evt_pending_body_replay_terminal_plan",
                plan_id,
                serde_json::json!({
                    "kind": "devFixture",
                    "fixtureName": "replayable-pending-body",
                }),
                "deferred",
                "executor.deferred",
                "admission.waitingForActiveStepToFinish",
                "2026-07-13T00:00:00.000Z",
            ),
        )
        .expect("append terminal plan event");
    storage
        .clear_choreography_pending_execution_bodies()
        .expect("clear sqlite cache");
    assert_eq!(
        storage
            .rebuild_choreography_pending_execution_body_cache_from_action_log()
            .expect("rebuild pending body cache from JSONL facts"),
        0
    );

    let replayable: ReplayableChoreographyPendingExecutionBody = storage
        .find_replayable_choreography_pending_execution_body_from_action_log(plan_id)
        .expect("find replayable pending body")
        .expect("replayable pending body exists");

    assert_eq!(replayable.plan_id, plan_id);
    assert_eq!(
        replayable.body_kind,
        ChoreographyPendingExecutionBodyKind::Timeline
    );
    assert_eq!(replayable.schema_version, 1);
    assert_eq!(replayable.body, pending_body);
    assert!(replayable.stored_event_id.starts_with("evt_"));
    assert!(storage
        .find_choreography_pending_execution_body(plan_id)
        .expect("find live pending body after replay lookup")
        .is_none());
}

#[test]
fn accepted_admission_consumes_replayable_pending_execution_body() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let plan_id = "plan_pending_body_accepted_admission";
    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: serde_json::json!({
                "schemaVersion": 1,
                "plan": { "planId": plan_id, "steps": [] }
            }),
        })
        .expect("store pending body");
    storage
        .append_choreography_action_log_event(
            &ActionLogEvent::executor_admission_decision_for_source(
                "evt_pending_body_accepted_admission",
                plan_id,
                &serde_json::json!({
                    "kind": "devFixture",
                    "fixtureName": "pending-body-accepted-admission"
                }),
                ChoreographyTriggerSource::AiChoreography.action_log_value(),
                &ChoreographyAdmissionDecision::Accepted {
                    plan_id: plan_id.to_owned(),
                    trigger_source: ChoreographyTriggerSource::AiChoreography,
                    priority: ChoreographyPlanPriority::AiChoreography,
                },
                "2026-07-16T00:00:00.000Z",
            ),
        )
        .expect("append accepted admission");

    assert!(storage
        .find_choreography_pending_execution_body(plan_id)
        .expect("find cached pending body")
        .is_none());
    assert!(storage
        .find_replayable_choreography_pending_execution_body_from_action_log(plan_id)
        .expect("find replayable pending body")
        .is_none());
}

#[test]
fn lists_recoverable_pending_execution_entries_with_replayable_body() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let plan_id = "plan_pending_execution_entry_019f6000-0000-7000-8000-000000000061";
    let active_plan_id = "plan_active_execution_entry_019f6000-0000-7000-8000-000000000062";
    let active_step_id = "step_active_execution_entry_019f6000-0000-7000-8000-000000000063";
    let source_ref = serde_json::json!({
        "kind": "devFixture",
        "fixtureName": "recoverable-pending-execution-entry",
    });
    let pending_body = serde_json::json!({
        "schemaVersion": 1,
        "plan": {
            "planId": plan_id,
            "steps": []
        }
    });

    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: pending_body.clone(),
        })
        .expect("store pending body");
    storage
        .append_choreography_action_log_event(
            &ActionLogEvent::executor_admission_decision_for_source(
                "evt_recoverable_pending_execution_entry",
                plan_id,
                &source_ref,
                ChoreographyTriggerSource::UserRequested.action_log_value(),
                &ChoreographyAdmissionDecision::Deferred {
                    plan_id: plan_id.to_owned(),
                    trigger_source: ChoreographyTriggerSource::UserRequested,
                    priority: ChoreographyPlanPriority::UserRequested,
                    active_plan_id: active_plan_id.to_owned(),
                    active_step_id: Some(active_step_id.to_owned()),
                    active_priority: ChoreographyPlanPriority::AiChoreography,
                    active_step_interrupt_policy: SidecarInterruptPolicy::FinishStep,
                    reason_code: "admission.waitingForActiveStepToFinish".to_owned(),
                },
                "2026-07-13T00:10:00.000Z",
            ),
        )
        .expect("append deferred admission event");
    storage
        .clear_choreography_pending_execution_bodies()
        .expect("clear live pending body cache");
    assert_eq!(
        storage
            .rebuild_choreography_pending_execution_body_cache_from_action_log()
            .expect("rebuild pending body cache from JSONL facts"),
        1
    );

    let entries = storage
        .list_recoverable_choreography_pending_executions_after_startup()
        .expect("list recoverable pending execution entries");

    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.admission.plan_id, plan_id);
    assert_eq!(entry.admission.source_ref, source_ref);
    assert_eq!(entry.admission.active_plan_id, active_plan_id);
    assert_eq!(
        entry.admission.active_step_id.as_deref(),
        Some(active_step_id)
    );
    assert_eq!(
        entry.admission.active_step_interrupt_policy,
        SidecarInterruptPolicy::FinishStep
    );
    assert_eq!(entry.body.plan_id, plan_id);
    assert_eq!(
        entry.body.body_kind,
        ChoreographyPendingExecutionBodyKind::Timeline
    );
    assert_eq!(entry.body.schema_version, 1);
    assert_eq!(entry.body.body, pending_body);
    assert!(entry.body.stored_event_id.starts_with("evt_"));
}

#[test]
fn lists_recoverable_pending_admission_metadata_from_deferred_events_with_pending_body() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let plan_id = "plan_pending_admission_019f6000-0000-7000-8000-000000000041";
    let active_plan_id = "plan_active_admission_019f6000-0000-7000-8000-000000000042";
    let active_step_id = "step_active_admission_019f6000-0000-7000-8000-000000000043";
    let source_ref = serde_json::json!({
        "kind": "devFixture",
        "fixtureName": "recoverable-pending-admission",
    });
    let pending_body = serde_json::json!({
        "schemaVersion": 1,
        "plan": {
            "planId": plan_id,
            "steps": []
        }
    });

    storage
        .upsert_choreography_pending_execution_body(UpsertChoreographyPendingExecutionBodyRequest {
            plan_id: plan_id.to_owned(),
            body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
            schema_version: 1,
            body: pending_body,
        })
        .expect("store pending body");
    let decision = ChoreographyAdmissionDecision::Deferred {
        plan_id: plan_id.to_owned(),
        trigger_source: ChoreographyTriggerSource::UserRequested,
        priority: ChoreographyPlanPriority::UserRequested,
        active_plan_id: active_plan_id.to_owned(),
        active_step_id: Some(active_step_id.to_owned()),
        active_priority: ChoreographyPlanPriority::AiChoreography,
        active_step_interrupt_policy: SidecarInterruptPolicy::FinishStep,
        reason_code: "admission.waitingForActiveStepToFinish".to_owned(),
    };
    storage
        .append_choreography_action_log_event(
            &ActionLogEvent::executor_admission_decision_for_source(
                "evt_recoverable_pending_admission",
                plan_id,
                &source_ref,
                ChoreographyTriggerSource::UserRequested.action_log_value(),
                &decision,
                "2026-07-13T00:05:00.000Z",
            ),
        )
        .expect("append deferred admission event");

    let admissions = storage
        .list_recoverable_choreography_pending_admissions_after_startup()
        .expect("list recoverable pending admissions");

    assert_eq!(admissions.len(), 1);
    let admission = &admissions[0];
    assert_eq!(admission.plan_id, plan_id);
    assert_eq!(admission.source_ref, source_ref);
    assert_eq!(
        admission.trigger_source,
        ChoreographyTriggerSource::UserRequested
    );
    assert_eq!(admission.priority, ChoreographyPlanPriority::UserRequested);
    assert_eq!(
        admission.reason_code,
        "admission.waitingForActiveStepToFinish"
    );
    assert_eq!(admission.active_plan_id, active_plan_id);
    assert_eq!(admission.active_step_id.as_deref(), Some(active_step_id));
    assert_eq!(
        admission.active_priority,
        ChoreographyPlanPriority::AiChoreography
    );
    assert_eq!(
        admission.active_step_interrupt_policy,
        SidecarInterruptPolicy::FinishStep
    );
    assert_eq!(
        admission.body_kind,
        ChoreographyPendingExecutionBodyKind::Timeline
    );
    assert_eq!(admission.body_schema_version, 1);
    assert_eq!(
        admission.deferred_event_id,
        "evt_recoverable_pending_admission"
    );
    assert_eq!(admission.deferred_at, "2026-07-13T00:05:00.000Z");
}

#[test]
fn active_conversation_messages_exclude_superseded_versions() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let conversation = storage
        .create_conversation(CreateBuddyConversationRequest {
            forked_from_message_id: None,
            project_root: None,
            scope: "global".into(),
            source_conversation_id: None,
            source_run_id: None,
            title: None,
        })
        .expect("create conversation");

    storage
        .append_conversation_message(AppendBuddyConversationMessageRequest {
            attachments: Vec::new(),
            branch_id: conversation.active_branch_id.clone(),
            content: "older answer".into(),
            conversation_id: conversation.id.clone(),
            parent_message_id: None,
            role: "assistant".into(),
            run_id: None,
            version_group_id: Some("assistant-version".into()),
            version_index: 0,
            version_status: "superseded".into(),
        })
        .expect("append superseded message");
    let active = storage
        .append_conversation_message(AppendBuddyConversationMessageRequest {
            attachments: Vec::new(),
            branch_id: conversation.active_branch_id.clone(),
            content: "new answer".into(),
            conversation_id: conversation.id.clone(),
            parent_message_id: None,
            role: "assistant".into(),
            run_id: None,
            version_group_id: Some("assistant-version".into()),
            version_index: 1,
            version_status: "active".into(),
        })
        .expect("append active message");
    let conversation_id = conversation.id.clone();

    let messages = storage
        .list_active_conversation_messages(conversation_id.clone(), 20)
        .expect("list active messages");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, active.id);
    assert_eq!(
        messages[0].branch_id.as_deref(),
        Some(conversation.active_branch_id.as_str())
    );
    assert_eq!(messages[0].version_status.as_deref(), Some("active"));

    let log_lines = storage.read_local_log_lines_for_test(&conversation.log_path);
    assert_eq!(log_lines.len(), 3);
    let active_line: serde_json::Value =
        serde_json::from_str(&log_lines[2]).expect("message created jsonl");
    assert_eq!(active_line["type"], "message.created");
    assert_eq!(active_line["payload"]["messageId"], active.id);
    assert_eq!(active_line["payload"]["conversationId"], conversation_id);
    assert_eq!(
        active_line["payload"]["branchId"],
        conversation.active_branch_id
    );
}

#[test]
fn deleted_conversation_is_not_restored_from_local_logs_after_restart() {
    let buddy_home = create_storage_test_dir("lexora-buddy-deleted-conversation");
    let database_path = buddy_home.join("sqlite").join("state.sqlite3");
    let storage = BuddyStorage::new_fixed_for_test(database_path.clone(), buddy_home.clone());
    storage.initialize().expect("initialize storage");
    let conversation = storage
        .create_conversation(CreateBuddyConversationRequest {
            forked_from_message_id: None,
            project_root: None,
            scope: "global".into(),
            source_conversation_id: None,
            source_run_id: None,
            title: Some("delete me".into()),
        })
        .expect("create conversation");
    let message = storage
        .append_conversation_message(AppendBuddyConversationMessageRequest {
            attachments: Vec::new(),
            branch_id: conversation.active_branch_id.clone(),
            content: "temporary message".into(),
            conversation_id: conversation.id.clone(),
            parent_message_id: None,
            role: "user".into(),
            run_id: None,
            version_group_id: None,
            version_index: 1,
            version_status: "active".into(),
        })
        .expect("append message");
    storage
        .create_conversation_run(CreateBuddyConversationRunRequest {
            branch_id: conversation.active_branch_id,
            conversation_id: conversation.id.clone(),
            cwd: None,
            external_run_id: None,
            external_thread_id: None,
            intent: "buddy.agent.turn".into(),
            runtime: "codex".into(),
            triggering_message_id: message.id,
        })
        .expect("create conversation run");

    assert!(storage
        .delete_conversation(conversation.id)
        .expect("delete conversation"));

    let restarted = BuddyStorage::new_fixed_for_test(database_path, buddy_home.clone());
    restarted.initialize().expect("restart storage");

    assert!(restarted
        .list_conversations(10)
        .expect("list conversations")
        .is_empty());
    std::fs::remove_dir_all(buddy_home).expect("cleanup buddy home");
}

#[test]
fn conversation_delete_tombstone_removes_a_stale_sqlite_projection_on_restart() {
    let buddy_home = create_storage_test_dir("lexora-buddy-delete-tombstone");
    let database_path = buddy_home.join("sqlite").join("state.sqlite3");
    let storage = BuddyStorage::new_fixed_for_test(database_path.clone(), buddy_home.clone());
    storage.initialize().expect("initialize storage");
    let conversation = storage
        .create_conversation(CreateBuddyConversationRequest {
            forked_from_message_id: None,
            project_root: None,
            scope: "global".into(),
            source_conversation_id: None,
            source_run_id: None,
            title: Some("stale sqlite projection".into()),
        })
        .expect("create conversation");
    let message = storage
        .append_conversation_message(AppendBuddyConversationMessageRequest {
            attachments: Vec::new(),
            branch_id: conversation.active_branch_id.clone(),
            content: "stale run".into(),
            conversation_id: conversation.id.clone(),
            parent_message_id: None,
            role: "user".into(),
            run_id: None,
            version_group_id: None,
            version_index: 1,
            version_status: "active".into(),
        })
        .expect("append message");
    storage
        .create_conversation_run(CreateBuddyConversationRunRequest {
            branch_id: conversation.active_branch_id.clone(),
            conversation_id: conversation.id.clone(),
            cwd: None,
            external_run_id: None,
            external_thread_id: None,
            intent: "buddy.agent.turn".into(),
            runtime: "codex".into(),
            triggering_message_id: message.id,
        })
        .expect("create conversation run");
    storage
        .local_logs
        .append_conversation_deleted_index_entry(&conversation.id, &conversation.log_path)
        .expect("append delete tombstone");

    let restarted = BuddyStorage::new_fixed_for_test(database_path, buddy_home.clone());
    restarted.initialize().expect("restart storage");

    assert!(restarted
        .list_conversations(10)
        .expect("list conversations")
        .is_empty());
    assert!(restarted.list_runs(None, 10).expect("list runs").is_empty());
    std::fs::remove_dir_all(buddy_home).expect("cleanup buddy home");
}

#[test]
fn reconciles_run_index_and_events_from_jsonl_after_sqlite_rows_are_deleted() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let session = storage
        .create_session(CreateBuddySessionRequest {
            scope: "global".into(),
            runtime: "codex".into(),
            project_root: None,
            title: None,
        })
        .expect("create session");
    let run = storage
        .create_run(CreateBuddyRunRequest {
            session_id: session.id.clone(),
            runtime: "codex".into(),
            cwd: Some("/tmp/recoverable-project".into()),
            external_thread_id: Some("thread-1".into()),
            external_run_id: None,
        })
        .expect("create run");
    storage
        .append_run_event(CreateBuddyRunEventRequest::new(
            run.id.clone(),
            BuddyRunEventType::RunStarted,
            serde_json::json!({ "runtime": "codex" }),
        ))
        .expect("append started event");
    storage
        .finish_run(
            run.id.clone(),
            BuddyRunTerminalStatus::Completed,
            serde_json::json!({ "status": "ok" }),
        )
        .expect("finish run");
    let log_path = run.log_path.clone().expect("run should have log path");
    let connection = storage.open_connection().expect("open connection");
    connection
        .execute("DELETE FROM runs WHERE id = ?1", rusqlite::params![run.id])
        .expect("delete run index");

    assert!(storage
        .list_runs(Some(session.id.clone()), 10)
        .expect("list runs")
        .is_empty());

    let restored = storage
        .reconcile_run_log(&log_path)
        .expect("reconcile run log");
    let restored_events = storage
        .list_run_events(restored.id.clone(), None, 10)
        .expect("list restored run events");

    assert_eq!(restored.session_id.as_deref(), Some(session.id.as_str()));
    assert_eq!(restored.status, "completed");
    assert_eq!(restored.cwd.as_deref(), Some("/tmp/recoverable-project"));
    assert_eq!(restored.external_thread_id.as_deref(), Some("thread-1"));
    assert_eq!(restored.log_path.as_deref(), Some(log_path.as_str()));
    assert_eq!(restored_events.len(), 2);
    assert_eq!(restored_events[0].event_type, "run.started");
    assert_eq!(restored_events[0].payload["runtime"], "codex");
    assert_eq!(restored_events[1].event_type, "run.completed");
    assert_eq!(restored_events[1].payload["status"], "ok");
}

#[test]
fn replayed_memory_candidate_event_preserves_project_source() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let session = storage
        .create_session(CreateBuddySessionRequest {
            scope: "global".into(),
            runtime: "codex".into(),
            project_root: None,
            title: None,
        })
        .expect("create session");
    let run = storage
        .create_run(CreateBuddyRunRequest {
            session_id: session.id,
            runtime: "codex".into(),
            cwd: Some("/tmp/replay-project".into()),
            external_thread_id: None,
            external_run_id: None,
        })
        .expect("create run");
    let log_path = run.log_path.clone().expect("run should have log path");
    let source_event_id = format!("run:{}:memory_candidate:continuity.chat_turn", run.id);
    storage
        .append_run_event(CreateBuddyRunEventRequest::new(
            run.id.clone(),
            BuddyRunEventType::MemoryCandidateCreated,
            serde_json::json!({
                "candidateType": "continuity.chat_turn",
                "confidence": 0.82,
                "content": "用户希望 Lexora Buddy 从 JSONL replay 记忆写入。",
                "conversationId": null,
                "decision": "accepted",
                "eligibility": {
                    "candidateGeneration": true,
                    "durableWrite": true,
                    "retrieval": true
                },
                "projectId": "/tmp/replay-project",
                "reason": "eligible completed codex turn",
                "runId": run.id,
                "scope": "project-private",
                "sourceEventId": source_event_id,
                "sourceLogPath": log_path,
                "sourceRefs": [
                    {
                        "projectId": "/tmp/replay-project",
                        "scope": "project-private",
                        "sourceEventId": source_event_id,
                        "sourceKind": "run_log",
                        "sourceLogPath": log_path,
                        "sourceRunId": run.id
                    }
                ]
            }),
        ))
        .expect("append memory candidate event");

    assert!(storage
        .list_memory_candidates(None, 10)
        .expect("list candidates before reconcile")
        .is_empty());

    storage
        .reconcile_run_log(&log_path)
        .expect("reconcile run log");
    let candidates = storage
        .list_memory_candidates(Some("accepted".to_owned()), 10)
        .expect("list candidates after reconcile");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(candidates[0].candidate_type, "continuity.chat_turn");
    assert_eq!(
        candidates[0].source_event_id.as_deref(),
        Some(source_event_id.as_str())
    );
    assert_eq!(
        candidates[0].project_id.as_deref(),
        Some("/tmp/replay-project")
    );
    assert_eq!(candidates[0].source_refs[0]["sourceKind"], "run_log");
}

#[test]
fn lists_chat_run_events_as_compact_transcript_payloads() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let session = storage
        .create_session(CreateBuddySessionRequest {
            scope: "global".into(),
            runtime: "codex".into(),
            project_root: None,
            title: None,
        })
        .expect("create session");
    let run = storage
        .create_run(CreateBuddyRunRequest {
            session_id: session.id.clone(),
            runtime: "codex".into(),
            cwd: None,
            external_thread_id: None,
            external_run_id: None,
        })
        .expect("create run");
    let large_result = "A".repeat(20_000);
    let large_diff = format!(
        "diff --git a/src/chat.ts b/src/chat.ts\n+++ b/src/chat.ts\n{}",
        "B".repeat(20_000)
    );

    storage
        .append_run_event(CreateBuddyRunEventRequest::new(
            run.id.clone(),
            BuddyRunEventType::RunStarted,
            serde_json::json!({
                "userMessageId": "user-1",
                "unused": large_result.clone(),
            }),
        ))
        .expect("append run started");
    storage
        .append_run_event(CreateBuddyRunEventRequest::projected(
            run.id.clone(),
            "tool.finished",
            serde_json::json!({
                "itemId": "tool-1",
                "item": {
                    "result": {
                        "content": [
                            {
                                "text": large_result.clone(),
                                "type": "text",
                            }
                        ],
                    },
                    "status": "completed",
                    "tool": "read_mcp_resource",
                    "type": "mcpToolCall",
                },
            }),
        ))
        .expect("append tool finished");
    storage
        .append_run_event(CreateBuddyRunEventRequest::projected(
            run.id.clone(),
            "turn.diff.updated",
            serde_json::json!({
                "diff": large_diff.clone(),
                "itemId": "diff-1",
                "turnId": "turn-1",
            }),
        ))
        .expect("append diff updated");
    storage
        .append_run_event(CreateBuddyRunEventRequest::projected(
            run.id.clone(),
            "message.delta",
            serde_json::json!({
                "delta": "hello",
                "itemId": "message-1",
                "phase": "final_answer",
            }),
        ))
        .expect("append message delta");

    let chat_events = storage
        .list_chat_session_run_events(session.id, None, 40, 100)
        .expect("list chat events");
    let chat_events_json = serde_json::to_string(&chat_events).expect("serialize chat events");
    let tool_event = chat_events
        .iter()
        .find(|event| event.event_type == "tool.finished")
        .expect("find tool event");
    let diff_event = chat_events
        .iter()
        .find(|event| event.event_type == "turn.diff.updated")
        .expect("find diff event");
    let message_event = chat_events
        .iter()
        .find(|event| event.event_type == "message.delta")
        .expect("find message event");

    assert_eq!(chat_events.len(), 4);
    assert_eq!(tool_event.payload["item"]["tool"], "read_mcp_resource");
    assert_eq!(diff_event.payload["filePaths"][0], "src/chat.ts");
    assert!(diff_event.payload.get("diff").is_none());
    assert_eq!(message_event.payload["delta"], "hello");
    assert!(!chat_events_json.contains(&large_result));
    assert!(!chat_events_json.contains(&large_diff));
    assert!(chat_events_json.len() < 8_000);
}

#[test]
fn resolves_codex_app_server_approval_without_cancelling_the_run() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let session = storage
        .create_session(CreateBuddySessionRequest {
            scope: "global".into(),
            runtime: "codex".into(),
            project_root: None,
            title: None,
        })
        .expect("create session");
    let run = storage
        .create_run(CreateBuddyRunRequest {
            session_id: session.id.clone(),
            runtime: "codex".into(),
            cwd: Some("/tmp/lexora".into()),
            external_thread_id: None,
            external_run_id: None,
        })
        .expect("create run");
    let run = storage
        .update_run_status(run.id, BuddyRunStatus::Running)
        .expect("mark run running");
    let approval = storage
        .create_approval(CreateBuddyApprovalRequest {
            kind: CODEX_APP_SERVER_REQUEST_APPROVAL_KIND.to_owned(),
            payload: serde_json::json!({
                "runtime": "codex",
                "method": "item/commandExecution/requestApproval",
                "promptPreview": "pnpm test",
                "requestId": 41,
            }),
            run_id: Some(run.id.clone()),
        })
        .expect("create approval");

    let resolution = storage
        .resolve_codex_app_server_request_approval(approval.id, BuddyApprovalTerminalStatus::Denied)
        .expect("resolve approval");
    let runs = storage.list_runs(Some(session.id), 10).expect("list runs");
    let events = storage
        .list_run_events(run.id, None, 10)
        .expect("list events");

    assert_eq!(resolution.approval.status, "denied");
    assert_eq!(resolution.event.event_type, "approval.resolved");
    assert_eq!(
        resolution.event.payload["kind"],
        CODEX_APP_SERVER_REQUEST_APPROVAL_KIND
    );
    assert_eq!(runs[0].status, "running");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "approval.resolved");

    let log_lines = storage
        .read_local_log_lines_for_test(run.log_path.as_deref().expect("run should have log path"));
    let log_events = parse_jsonl_events(&log_lines);
    let approval_log_event = log_events
        .iter()
        .find(|event| event["type"] == "approval.resolved")
        .expect("approval resolution should be replayable");
    assert_eq!(
        approval_log_event["payload"]["eventType"],
        "approval.resolved"
    );
    assert_eq!(approval_log_event["payload"]["event"]["status"], "denied");
}

#[test]
fn rejects_project_fact_memory_candidate_without_project_scope() {
    let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
    let mut request = CreateBuddyMemoryCandidateRequest {
        candidate_type: "project.fact".to_owned(),
        confidence: 0.91,
        content: "项目事实：Buddy memory replay 要保留项目身份".to_owned(),
        conversation_id: None,
        decision: "accepted".to_owned(),
        eligibility: serde_json::json!({
            "candidateGeneration": true,
            "durableWrite": true,
            "retrieval": true
        }),
        project_id: None,
        reason: "project fact candidate".to_owned(),
        run_id: None,
        scope: "global".to_owned(),
        source_event_id: Some("project-fact-global".to_owned()),
        source_log_path: "runs/project-fact.jsonl".to_owned(),
        source_refs: serde_json::json!([]),
    };

    let global_error = storage
        .create_memory_candidate(request.clone())
        .expect_err("global project fact must be rejected");

    request.scope = "project-private".to_owned();
    request.source_event_id = Some("project-fact-missing-project".to_owned());
    let missing_project_error = storage
        .create_memory_candidate(request.clone())
        .expect_err("project fact without project id must be rejected");

    request.project_id = Some("/tmp/project-alpha".to_owned());
    request.source_event_id = Some("project-fact-valid".to_owned());
    let candidate = storage
        .create_memory_candidate(request)
        .expect("project fact with project id");

    assert!(global_error
        .to_string()
        .contains("project fact memory candidate requires project-private scope"));
    assert!(missing_project_error
        .to_string()
        .contains("project fact memory candidate requires project id"));
    assert_eq!(candidate.scope, "project-private");
    assert_eq!(candidate.project_id.as_deref(), Some("/tmp/project-alpha"));
}

fn parse_jsonl_events(lines: &[String]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("jsonl event"))
        .collect()
}

fn create_storage_test_dir(prefix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create storage test dir");
    dir
}
