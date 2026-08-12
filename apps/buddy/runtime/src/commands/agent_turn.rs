use std::{path::Path, thread};

use crate::{
    agents::codex,
    choreography::executor::NativePetChoreographyStepExecutor,
    context_pack,
    domain::{
        BuddyMessageRole, BuddyMessageVersionStatus, BuddyRunEventType, BuddyRunTerminalStatus,
        BuddyRuntime,
    },
    error::BuddyError,
    intent::{self, BuddyIntentClassificationInput, BuddyIntentDecision},
    state::BuddyAppState,
    storage::{
        AppendBuddyConversationMessageRequest, BuddyConversation, BuddyMessage, BuddyRun,
        BuddyStorage, CreateBuddyConversationRequest, CreateBuddyConversationRunRequest,
        CreateBuddyRunEventRequest,
    },
};

#[cfg(test)]
use crate::storage::BuddyRunEvent;

use super::{
    chat_input::{
        compose_buddy_chat_codex_inputs, compose_buddy_chat_runtime_content,
        compose_buddy_chat_user_message_content, create_buddy_builtin_host_skill_input,
        create_buddy_message_attachments, materialize_buddy_chat_attachments,
        BuddyChatAttachmentRequest, BuddyChatPromptContextItem,
    },
    codex_runtime::{execute_existing_codex_run, BuddyCodexRunTarget, ExistingCodexRunRequest},
    normalize_optional_string, resolve_runtime_cwd,
    run_state::{BuddyRunCancellationRegistry, BuddyRunStateEventPublisher},
    BuddyCommandResult,
};

const BUDDY_AGENT_TURN_INTENT: &str = "buddy.agent.turn";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartBuddyAgentTurnRequest {
    pub(crate) request_id: String,
    pub(crate) conversation_id: Option<String>,
    pub(crate) conversation_seed: Option<CreateBuddyConversationRequest>,
    pub(crate) content: String,
    pub(crate) cwd: Option<String>,
    pub(crate) model_selection: Option<BuddyChatModelSelection>,
    #[serde(default)]
    pub(crate) attachments: Vec<BuddyChatAttachmentRequest>,
    #[serde(default)]
    pub(crate) context_items: Vec<BuddyChatPromptContextItem>,
    #[serde(default)]
    pub(crate) inputs: Vec<codex::CodexUserInput>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BuddyChatModelSelection {
    pub(crate) runtime: String,
    pub(crate) model: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg(test)]
#[allow(dead_code)]
pub struct BuddyChatTurn {
    pub(crate) user_message: BuddyMessage,
    pub(crate) assistant_message: BuddyMessage,
    pub(crate) run: BuddyRun,
    pub(crate) events: Vec<BuddyRunEvent>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAgentTurnStart {
    assistant_message: Option<BuddyMessage>,
    conversation: BuddyConversation,
    intent: String,
    user_message: BuddyMessage,
    run: Option<BuddyRun>,
}

pub fn start_buddy_agent_turn(
    state: &BuddyAppState,
    cancellations: &BuddyRunCancellationRegistry,
    event_publisher: BuddyRunStateEventPublisher,
    host_action_executor: Option<NativePetChoreographyStepExecutor>,
    request: StartBuddyAgentTurnRequest,
) -> BuddyCommandResult<BuddyAgentTurnStart> {
    let request_id = normalize_request_id(&request.request_id)?;
    let storage = state.storage_handle();
    let request_preparation = storage.lock_agent_turn_request_preparation()?;
    if let Some(response) = storage.find_agent_turn_response(&request_id)? {
        return serde_json::from_value(response)
            .map_err(BuddyError::from)
            .map_err(Into::into);
    }

    let requested_conversation_id = request
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|conversation_id| !conversation_id.is_empty());
    let mut conversation_reservation = requested_conversation_id
        .map(|conversation_id| cancellations.reserve_conversation(conversation_id))
        .transpose()?;

    let plan = create_buddy_agent_turn_plan(state, request)?;
    let memory_root = state.memories_dir_path();
    let assistant_message = plan.assistant_message.clone();
    let conversation = plan.conversation.clone();
    let intent = plan.decision.intent.as_str().to_owned();
    let intent_decision = plan.decision.clone();
    let run = plan.run.clone();
    let user_message = plan.user_message.clone();
    let host_action_state = state.clone();
    let response = BuddyAgentTurnStart {
        assistant_message,
        conversation,
        intent,
        run,
        user_message: user_message.clone(),
    };
    let response_value = serde_json::to_value(&response).map_err(BuddyError::from)?;
    let stored_response = storage.store_agent_turn_response(
        &request_id,
        &response.conversation.id,
        &response_value,
    )?;
    let response = serde_json::from_value(stored_response).map_err(BuddyError::from)?;
    drop(request_preparation);

    if let Some(run_to_execute) = plan.run {
        let conversation_id = plan.conversation.id.clone();
        let user_message_id = user_message.id.clone();
        let reservation = match conversation_reservation.take() {
            Some(reservation) => reservation,
            None => cancellations.reserve_conversation(&conversation_id)?,
        };
        let cancellation = match reservation.register_run(&run_to_execute.id) {
            Ok(cancellation) => cancellation,
            Err(error) => {
                return Err(finalize_agent_turn_start_error(
                    &storage,
                    &run_to_execute.id,
                    &event_publisher,
                    error,
                )
                .into());
            }
        };
        let run_id = run_to_execute.id.clone();
        let cancellations = cancellations.clone();
        let cleanup_cancellations = cancellations.clone();
        let cleanup_run_id = run_id.clone();
        let start_failure_storage = storage.clone();
        let start_failure_event_publisher = event_publisher.clone();
        let spawn_result = thread::Builder::new()
            .name(format!("lexora-codex-run-{run_id}"))
            .spawn(move || {
                if let Err(error) = execute_existing_codex_run(ExistingCodexRunRequest {
                    approval_id: None,
                    attachment_count: plan.attachment_count,
                    runtime_cwd: plan.runtime_cwd,
                    content: plan.runtime_content,
                    input: plan.runtime_inputs,
                    intent_decision,
                    memory_root,
                    model_selection: plan.model_selection,
                    run: run_to_execute,
                    selected_context: plan.selected_context,
                    target: BuddyCodexRunTarget::conversation(
                        conversation_id,
                        plan.conversation.active_branch_id,
                        plan.user_message.id,
                    ),
                    source_project: plan.source_project,
                    storage,
                    cancellation: Some(cancellation),
                    event_publisher,
                    host_action_executor,
                    host_action_state,
                    user_message_id,
                }) {
                    eprintln!("lexora buddy agent turn failed: {error}");
                }
                cancellations.remove(&run_id);
            });
        let join_handle = spawn_result.map_err(|error| {
            cleanup_cancellations.remove(&cleanup_run_id);
            finalize_agent_turn_start_error(
                &start_failure_storage,
                &cleanup_run_id,
                &start_failure_event_publisher,
                BuddyError::Runtime(format!("failed to start Codex run worker: {error}")),
            )
        })?;
        cleanup_cancellations.attach_worker(&cleanup_run_id, join_handle);
    }

    Ok(response)
}

fn normalize_request_id(request_id: &str) -> Result<String, BuddyError> {
    let request_id = request_id.trim();
    if request_id.is_empty() || request_id.len() > 128 {
        return Err(BuddyError::Validation(
            "agent turn requestId must contain 1 to 128 bytes".to_owned(),
        ));
    }

    Ok(request_id.to_owned())
}

fn finalize_agent_turn_start_error(
    storage: &BuddyStorage,
    run_id: &str,
    event_publisher: &BuddyRunStateEventPublisher,
    error: BuddyError,
) -> BuddyError {
    match record_agent_turn_start_failure(storage, run_id, event_publisher, &error.to_string()) {
        Ok(()) => error,
        Err(record_error) => BuddyError::Runtime(format!(
            "{error}; failed to record agent turn start failure: {record_error}"
        )),
    }
}

fn record_agent_turn_start_failure(
    storage: &BuddyStorage,
    run_id: &str,
    event_publisher: &BuddyRunStateEventPublisher,
    message: &str,
) -> Result<(), BuddyError> {
    let finished_run = storage.finish_run(
        run_id.to_owned(),
        BuddyRunTerminalStatus::Failed,
        serde_json::json!({
            "message": message,
            "protocol": "worker_start",
        }),
    )?;
    event_publisher.emit_finished_run(&finished_run);
    Ok(())
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn run_buddy_agent_turn(
    state: &BuddyAppState,
    request: StartBuddyAgentTurnRequest,
) -> Result<BuddyChatTurn, BuddyError> {
    let plan = create_buddy_agent_turn_plan(state, request)?;
    let run = plan.run.ok_or_else(|| {
        BuddyError::Validation("agent turn did not require a runtime run".to_owned())
    })?;
    let conversation_id = plan.conversation.id.clone();
    let branch_id = plan.conversation.active_branch_id.clone();
    let user_message_id = plan.user_message.id.clone();
    let storage = state.storage_handle();
    let execution = execute_existing_codex_run(ExistingCodexRunRequest {
        approval_id: None,
        attachment_count: plan.attachment_count,
        runtime_cwd: plan.runtime_cwd,
        content: plan.runtime_content,
        input: plan.runtime_inputs,
        intent_decision: plan.decision,
        memory_root: state.memories_dir_path(),
        model_selection: plan.model_selection,
        run,
        selected_context: plan.selected_context,
        target: BuddyCodexRunTarget::conversation(
            conversation_id,
            branch_id,
            user_message_id.clone(),
        ),
        source_project: plan.source_project,
        storage,
        cancellation: None,
        event_publisher: BuddyRunStateEventPublisher::disabled(),
        host_action_executor: None,
        host_action_state: state.clone(),
        user_message_id,
    })?;

    Ok(BuddyChatTurn {
        assistant_message: execution.assistant_message,
        events: execution.events,
        run: execution.run,
        user_message: plan.user_message,
    })
}

struct BuddyAgentTurnPlan {
    attachment_count: usize,
    assistant_message: Option<BuddyMessage>,
    runtime_content: String,
    runtime_cwd: String,
    runtime_inputs: Vec<codex::CodexUserInput>,
    conversation: BuddyConversation,
    decision: BuddyIntentDecision,
    model_selection: Option<codex::CodexModelSelection>,
    run: Option<BuddyRun>,
    selected_context: Vec<context_pack::BuddyContextPackSelectedContextItem>,
    source_project: Option<String>,
    user_message: BuddyMessage,
}

fn create_buddy_agent_turn_plan(
    state: &BuddyAppState,
    request: StartBuddyAgentTurnRequest,
) -> Result<BuddyAgentTurnPlan, BuddyError> {
    let content = request.content.trim().to_owned();
    let attachments = materialize_buddy_chat_attachments(state, request.attachments)?;
    let context_items = request.context_items;
    let selected_context = create_context_pack_selected_context_items(&context_items);
    let inputs = request.inputs;
    let model_selection = create_codex_model_selection(request.model_selection);
    if content.is_empty() && attachments.is_empty() && inputs.is_empty() {
        return Err(BuddyError::Codex("message content is empty".to_owned()));
    }

    let conversation = resolve_agent_turn_conversation(
        state,
        normalize_optional_string(request.conversation_id),
        request.conversation_seed,
    )?;
    let (runtime_cwd, source_project) =
        resolve_agent_turn_runtime_cwd(state, &conversation, request.cwd.clone())?;
    let decision = intent::classify_buddy_intent(BuddyIntentClassificationInput {
        content: &content,
        cwd: Some(runtime_cwd.as_str()),
        has_attachments: !attachments.is_empty(),
        has_context_items: !context_items.is_empty(),
        has_project_scope: conversation.project_root.is_some() || source_project.is_some(),
        has_structured_inputs: !inputs.is_empty(),
    });
    let user_content =
        compose_buddy_chat_user_message_content(&content, &attachments, &context_items);
    let user_message =
        state.append_conversation_message(AppendBuddyConversationMessageRequest {
            attachments: create_buddy_message_attachments(&attachments),
            branch_id: conversation.active_branch_id.clone(),
            content: user_content,
            conversation_id: conversation.id.clone(),
            parent_message_id: None,
            role: BuddyMessageRole::User.as_str().to_owned(),
            run_id: None,
            version_group_id: None,
            version_index: 1,
            version_status: BuddyMessageVersionStatus::Active.as_str().to_owned(),
        })?;
    let router_decision_payload =
        create_router_decision_event_payload(&decision, &conversation, &user_message);
    state.storage_handle().append_conversation_event(
        conversation.id.clone(),
        BuddyRunEventType::RouterDecision.as_str().to_owned(),
        router_decision_payload.clone(),
    )?;
    if !decision.requires_runtime {
        let assistant_message =
            state.append_conversation_message(AppendBuddyConversationMessageRequest {
                attachments: Vec::new(),
                branch_id: conversation.active_branch_id.clone(),
                content: create_local_buddy_reply(decision.intent, &content),
                conversation_id: conversation.id.clone(),
                parent_message_id: Some(user_message.id.clone()),
                role: BuddyMessageRole::Assistant.as_str().to_owned(),
                run_id: None,
                version_group_id: None,
                version_index: 1,
                version_status: BuddyMessageVersionStatus::Active.as_str().to_owned(),
            })?;

        return Ok(BuddyAgentTurnPlan {
            attachment_count: attachments.len(),
            assistant_message: Some(assistant_message),
            runtime_content: String::new(),
            runtime_cwd,
            runtime_inputs: Vec::new(),
            conversation,
            decision,
            model_selection,
            run: None,
            selected_context,
            source_project,
            user_message,
        });
    }

    let runtime_content =
        compose_buddy_chat_runtime_content(&content, &attachments, &context_items);
    let builtin_host_skill =
        create_buddy_builtin_host_skill_input(Path::new(&state.global_runtime_cwd()))?;
    let runtime_inputs = compose_buddy_chat_codex_inputs(
        &runtime_content,
        inputs,
        &attachments,
        Some(builtin_host_skill),
    )?;
    let attachment_count = attachments.len();
    let external_thread_id = state
        .storage_handle()
        .find_conversation_runtime_binding(
            conversation.id.clone(),
            conversation.active_branch_id.clone(),
            BuddyRuntime::Codex.as_str().to_owned(),
            Some(runtime_cwd.clone()),
        )?
        .and_then(|binding| binding.external_thread_id);
    let run = state.create_conversation_run(CreateBuddyConversationRunRequest {
        runtime: BuddyRuntime::Codex.as_str().to_owned(),
        branch_id: conversation.active_branch_id.clone(),
        conversation_id: conversation.id.clone(),
        cwd: Some(runtime_cwd.clone()),
        external_run_id: None,
        external_thread_id,
        intent: BUDDY_AGENT_TURN_INTENT.to_owned(),
        triggering_message_id: user_message.id.clone(),
    })?;
    if let Err(error) = state
        .storage_handle()
        .append_run_event(CreateBuddyRunEventRequest::new(
            run.id.clone(),
            BuddyRunEventType::RouterDecision,
            router_decision_payload,
        ))
    {
        return Err(finalize_agent_turn_start_error(
            &state.storage_handle(),
            &run.id,
            &BuddyRunStateEventPublisher::disabled(),
            error,
        ));
    }

    Ok(BuddyAgentTurnPlan {
        attachment_count,
        assistant_message: None,
        runtime_content,
        runtime_cwd,
        runtime_inputs,
        conversation,
        decision,
        model_selection,
        run: Some(run),
        selected_context,
        source_project,
        user_message,
    })
}

fn create_context_pack_selected_context_items(
    context_items: &[BuddyChatPromptContextItem],
) -> Vec<context_pack::BuddyContextPackSelectedContextItem> {
    context_items
        .iter()
        .map(|item| context_pack::BuddyContextPackSelectedContextItem {
            description: item.description.clone(),
            kind: item.kind.clone(),
            label: item.label.clone(),
            path: item.path.clone(),
        })
        .collect()
}

fn create_router_decision_event_payload(
    decision: &BuddyIntentDecision,
    conversation: &BuddyConversation,
    user_message: &BuddyMessage,
) -> serde_json::Value {
    serde_json::json!({
        "runtime": decision.runtime.as_deref(),
        "branchId": conversation.active_branch_id.as_str(),
        "conversationId": conversation.id.as_str(),
        "cwd": decision.cwd.as_deref(),
        "intent": decision.intent.as_str(),
        "memoryEligibility": {
            "candidateGeneration": decision.memory_eligibility.candidate_generation,
            "durableWrite": decision.memory_eligibility.durable_write,
            "reasons": decision.memory_eligibility.reasons,
            "retrieval": decision.memory_eligibility.retrieval,
        },
        "memoryPolicy": decision.memory_policy.as_str(),
        "reason": decision.reason.as_str(),
        "requiresRuntime": decision.requires_runtime,
        "userMessageId": user_message.id.as_str(),
    })
}

fn create_local_buddy_reply(intent: intent::BuddyChatIntent, content: &str) -> String {
    match intent {
        intent::BuddyChatIntent::CompanionChat => create_companion_chat_reply(content),
        intent::BuddyChatIntent::DirectAnswer => create_direct_answer_reply(),
        _ => "我在。".to_owned(),
    }
}

fn create_companion_chat_reply(content: &str) -> String {
    let normalized = content.trim().trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '。' | '！' | '？' | '，' | '.' | '!' | '?' | ',' | '~' | '～'
            )
    });
    match normalized {
        "谢谢" | "谢了" | "多谢" | "辛苦了" => "不客气，我在。".to_owned(),
        "在吗" | "你在吗" => "我在。".to_owned(),
        _ => "你好，我在。".to_owned(),
    }
}

fn create_direct_answer_reply() -> String {
    "我是 Lexora Buddy，Lexora 的本地桌面伙伴。我会在自己的会话、运行事件和记忆边界内协助你；需要项目内容时，只按已授权的 cwd 处理。".to_owned()
}

fn resolve_agent_turn_conversation(
    state: &BuddyAppState,
    conversation_id: Option<String>,
    conversation_seed: Option<CreateBuddyConversationRequest>,
) -> Result<BuddyConversation, BuddyError> {
    match (conversation_id, conversation_seed) {
        (Some(conversation_id), None) => state.find_conversation(&conversation_id),
        (None, Some(conversation_seed)) => state.create_conversation(conversation_seed),
        (Some(_), Some(_)) => Err(BuddyError::Validation(
            "conversationId and conversationSeed cannot be provided together".to_owned(),
        )),
        (None, None) => Err(BuddyError::Validation(
            "conversationId or conversationSeed is required".to_owned(),
        )),
    }
}

fn resolve_agent_turn_runtime_cwd(
    state: &BuddyAppState,
    conversation: &BuddyConversation,
    requested_cwd: Option<String>,
) -> Result<(String, Option<String>), BuddyError> {
    let Some(requested_cwd) = normalize_optional_string(requested_cwd) else {
        return match conversation.project_root.as_ref() {
            Some(project_root) => Ok((project_root.clone(), Some(project_root.clone()))),
            None => Ok((state.global_runtime_cwd(), None)),
        };
    };
    let (runtime_cwd, source_project) = resolve_runtime_cwd(state, Some(requested_cwd))?;
    validate_agent_turn_conversation_project_scope(conversation, source_project.as_deref())?;

    Ok((runtime_cwd, source_project))
}

fn validate_agent_turn_conversation_project_scope(
    conversation: &BuddyConversation,
    source_project: Option<&str>,
) -> Result<(), BuddyError> {
    match (conversation.project_root.as_deref(), source_project) {
        (Some(conversation_project), Some(source_project))
            if conversation_project == source_project =>
        {
            Ok(())
        }
        (Some(_), Some(_)) => Err(BuddyError::Validation(
            "conversation project root does not match requested cwd".to_owned(),
        )),
        (Some(_), None) => Err(BuddyError::Validation(
            "project conversation requires its project cwd".to_owned(),
        )),
        (None, Some(_)) => Err(BuddyError::Validation(
            "global conversation cannot run in a project cwd".to_owned(),
        )),
        (None, None) => Ok(()),
    }
}

fn create_codex_model_selection(
    selection: Option<BuddyChatModelSelection>,
) -> Option<codex::CodexModelSelection> {
    let selection = selection?;
    if selection.runtime != BuddyRuntime::Codex.as_str() {
        return None;
    }

    let next = codex::CodexModelSelection {
        effort: normalize_optional_string(selection.effort),
        model: normalize_optional_string(selection.model),
        service_tier: normalize_optional_string(selection.service_tier),
    };
    if next.model.is_none() && next.service_tier.is_none() && next.effort.is_none() {
        return None;
    }

    Some(next)
}

#[cfg(test)]
mod tests;
