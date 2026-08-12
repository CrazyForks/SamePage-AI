mod app_server;

pub(super) use app_server::wait_for_codex_app_server_approval;

use crate::{
    domain::BuddyApprovalTerminalStatus,
    error::BuddyError,
    state::BuddyAppState,
    storage::{
        BuddyApproval, BuddyResolvedCodexAppServerRequestApproval,
        CODEX_APP_SERVER_REQUEST_APPROVAL_KIND,
    },
};

use super::BuddyCommandResult;

pub fn list_buddy_approvals(
    state: &BuddyAppState,
    status: Option<String>,
    limit: Option<i64>,
) -> BuddyCommandResult<Vec<BuddyApproval>> {
    Ok(state
        .storage_handle()
        .list_approvals(status, limit.unwrap_or(50))?)
}

pub fn deny_buddy_approval(
    state: &BuddyAppState,
    approval_id: String,
) -> BuddyCommandResult<serde_json::Value> {
    let approval = state.find_approval(approval_id.clone())?;
    if approval.kind == CODEX_APP_SERVER_REQUEST_APPROVAL_KIND {
        return Ok(
            serde_json::to_value(state.resolve_codex_app_server_request_approval(
                approval_id,
                BuddyApprovalTerminalStatus::Denied,
            )?)
            .map_err(BuddyError::from)?,
        );
    }

    Err(BuddyError::Validation("unsupported approval kind".to_owned()).into())
}

pub fn approve_buddy_codex_app_server_request_approval(
    state: &BuddyAppState,
    approval_id: String,
) -> BuddyCommandResult<BuddyResolvedCodexAppServerRequestApproval> {
    Ok(state.resolve_codex_app_server_request_approval(
        approval_id,
        BuddyApprovalTerminalStatus::Approved,
    )?)
}
