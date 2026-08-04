use tauri::State;

use crate::{
    state::BuddyAppState,
    storage::{
        ActionLogPlanDetail, ActionLogPlanList, ActionLogPlanListRequest,
        ActionLogSystemEventQueryRequest, ActionLogSystemEventQueryResult,
    },
};

use super::{run_buddy_blocking, BuddyCommandResult};

#[tauri::command]
pub async fn list_buddy_action_log_plans(
    state: State<'_, BuddyAppState>,
    request: ActionLogPlanListRequest,
) -> BuddyCommandResult<ActionLogPlanList> {
    let storage = state.storage_handle();
    run_buddy_blocking("list_buddy_action_log_plans", move || {
        storage.list_action_log_plans(request)
    })
    .await
}

#[tauri::command]
pub async fn get_buddy_action_log_plan_detail(
    state: State<'_, BuddyAppState>,
    plan_id: String,
) -> BuddyCommandResult<ActionLogPlanDetail> {
    let storage = state.storage_handle();
    run_buddy_blocking("get_buddy_action_log_plan_detail", move || {
        storage.get_action_log_plan_detail(&plan_id)
    })
    .await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn queryActionLogSystemEvents(
    state: State<'_, BuddyAppState>,
    request: ActionLogSystemEventQueryRequest,
) -> BuddyCommandResult<ActionLogSystemEventQueryResult> {
    let storage = state.storage_handle();
    run_buddy_blocking("queryActionLogSystemEvents", move || {
        storage.query_action_log_system_events(request)
    })
    .await
}
