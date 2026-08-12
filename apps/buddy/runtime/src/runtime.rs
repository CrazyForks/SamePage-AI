use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    agents::{claude, codex, usage},
    app_paths::BuddyAppPaths,
    choreography::executor::NativePetChoreographyStepExecutor,
    choreography::preset_behavior::{
        append_native_pet_preset_behavior_action_log, NativePetPresetBehaviorLogContext,
    },
    commands::{
        self, agent_turn, approval, attachment, run_state::BuddyRunStateEventPublisher,
        BuddyCommandError, BuddyRunCancellationRegistry,
    },
    error::{BuddyError, BuddyResult},
    local_log::LocalLogTimestamp,
    native_pet::{self, NativePetSidecarEvent, NativePetSidecarProcess},
    protocol::{RpcOutput, RpcRequest, RUNTIME_PROTOCOL_VERSION},
    runtime_instance::BuddyRuntimeInstanceLock,
    server::{RpcRequestHandler, RuntimeNotificationSink},
    state::BuddyAppState,
    storage::UpsertBuddyProjectRequest,
};

pub struct RuntimeApplication {
    state: BuddyAppState,
    cancellations: BuddyRunCancellationRegistry,
    pet_process: Mutex<Option<Arc<NativePetSidecarProcess>>>,
    _instance_lock: Option<BuddyRuntimeInstanceLock>,
}

impl Drop for RuntimeApplication {
    fn drop(&mut self) {
        self.cancellations.shutdown();
        if let Ok(mut pet_process) = self.pet_process.lock() {
            pet_process.take();
        }
    }
}

impl RuntimeApplication {
    pub fn initialize() -> BuddyResult<Self> {
        let paths = BuddyAppPaths::from_default_buddy_home();
        let instance_lock = BuddyRuntimeInstanceLock::acquire(&paths.data_dir_path())?;
        let mut runtime = Self::initialize_with_paths(paths)?;
        runtime._instance_lock = Some(instance_lock);
        Ok(runtime)
    }

    pub(crate) fn initialize_with_paths(paths: BuddyAppPaths) -> BuddyResult<Self> {
        Ok(Self {
            state: BuddyAppState::initialize_with_paths(paths)?,
            cancellations: BuddyRunCancellationRegistry::default(),
            pet_process: Mutex::new(None),
            _instance_lock: None,
        })
    }

    fn event_publisher(
        &self,
        notifications: RuntimeNotificationSink,
    ) -> BuddyRunStateEventPublisher {
        BuddyRunStateEventPublisher::new(move |event| {
            notifications(json!({
                "jsonrpc": "2.0",
                "method": "run.event",
                "params": event,
            }));
        })
    }

    fn finish_native_pet_start(
        &self,
        pet_process: &mut Option<Arc<NativePetSidecarProcess>>,
        result: BuddyResult<NativePetSidecarProcess>,
        notifications: RuntimeNotificationSink,
    ) {
        match result {
            Ok(process) => *pet_process = Some(Arc::new(process)),
            Err(error) => {
                let _ = self.state.mark_choreography_runtime_degraded(
                    "runtime.nativePetSidecarUnavailable",
                    LocalLogTimestamp::now_utc().to_rfc3339_millis(),
                );
                eprintln!("Native pet startup failed: {error}");
                notifications(runtime_notification(
                    "pet.state",
                    json!({ "status": "offline" }),
                ));
            }
        }
    }

    fn host_action_executor(&self) -> Option<NativePetChoreographyStepExecutor> {
        self.pet_process
            .lock()
            .ok()
            .and_then(|process| process.as_ref().cloned())
            .map(NativePetChoreographyStepExecutor::from_shared_sidecar)
    }
}

impl RpcRequestHandler for RuntimeApplication {
    fn start(&self, notifications: RuntimeNotificationSink) -> Result<(), String> {
        let mut pet_process = self
            .pet_process
            .lock()
            .map_err(|_| "native pet process lock was poisoned".to_owned())?;
        if pet_process.is_some() {
            return Ok(());
        }

        let state = self.state.clone();
        let storage = state.storage_handle();
        let pet_notifications = Arc::clone(&notifications);
        let result = native_pet::spawn_native_pet_sidecar(move |event| match event {
            NativePetSidecarEvent::Ready => {
                let _ = state.mark_choreography_runtime_ready(
                    LocalLogTimestamp::now_utc().to_rfc3339_millis(),
                );
                pet_notifications(runtime_notification(
                    "pet.state",
                    json!({ "status": "ready" }),
                ));
            }
            NativePetSidecarEvent::Restarting => {
                let _ = state.mark_choreography_runtime_degraded(
                    "runtime.nativePetSidecarRestarting",
                    LocalLogTimestamp::now_utc().to_rfc3339_millis(),
                );
                pet_notifications(runtime_notification(
                    "pet.state",
                    json!({ "status": "restarting" }),
                ));
            }
            NativePetSidecarEvent::OpenChat => {
                pet_notifications(runtime_notification("desktop.open", json!({})));
            }
            NativePetSidecarEvent::PresetBehavior(event) => {
                let _ = append_native_pet_preset_behavior_action_log(
                    &storage,
                    &event,
                    NativePetPresetBehaviorLogContext::new(),
                );
            }
            NativePetSidecarEvent::StepResponse(_) | NativePetSidecarEvent::StateSnapshot(_) => {}
        });
        self.finish_native_pet_start(&mut pet_process, result, notifications);
        Ok(())
    }

    fn dispatch(&self, request: RpcRequest, notifications: RuntimeNotificationSink) -> RpcOutput {
        let (id, method, params) = request.into_parts();

        match method.as_str() {
            "runtime.status" => RpcOutput::response(
                id,
                serde_json::to_value(RuntimeStatus {
                    name: "lexora-buddy-runtime",
                    protocol_version: RUNTIME_PROTOCOL_VERSION,
                    ready: true,
                })
                .expect("runtime status must be serializable"),
            ),
            "runtime.localState" => response(id, self.state.local_state_status()),
            "runtime.shutdown" => RpcOutput::shutdown(id),
            "codex.status" => with_params::<EmptyParams, _>(id, params, |_| {
                Ok::<_, BuddyError>(codex::detect_codex_runtime_status())
            }),
            "claude.status" => with_params::<EmptyParams, _>(id, params, |_| {
                Ok::<_, BuddyError>(claude::detect_claude_runtime_status())
            }),
            "usage.snapshot" => with_params::<EmptyParams, _>(id, params, |_| {
                Ok::<_, BuddyError>(usage::load_buddy_usage_snapshot())
            }),
            "codex.listModels" => {
                with_params::<EmptyParams, _>(id, params, |_| codex::load_codex_model_options())
            }
            "codex.listContextOptions" => {
                with_params::<CodexContextParams, _>(id, params, |params| {
                    let (runtime_cwd, _) = commands::resolve_runtime_cwd(&self.state, params.cwd)?;
                    codex::load_codex_prompt_context_options(
                        &runtime_cwd,
                        params.file_query.as_deref(),
                    )
                })
            }
            "projects.authorize" => {
                with_params::<AuthorizeProjectParams, _>(id, params, |params| {
                    self.state.upsert_project(UpsertBuddyProjectRequest {
                        root: params.root,
                        name: params.name,
                    })
                })
            }
            "projects.list" => with_params::<LimitParams, _>(id, params, |params| {
                self.state
                    .storage_handle()
                    .list_projects(params.limit.unwrap_or(200))
            }),
            "conversations.list" => with_params::<LimitParams, _>(id, params, |params| {
                self.state
                    .storage_handle()
                    .list_conversations(params.limit.unwrap_or(50))
            }),
            "conversations.delete" => {
                with_params::<ConversationIdParams, _>(id, params, |params| {
                    let _reservation = self
                        .cancellations
                        .reserve_conversation(&params.conversation_id)?;
                    self.state
                        .storage_handle()
                        .delete_conversation(params.conversation_id)
                })
            }
            "conversations.listMessages" => {
                with_params::<ConversationMessagesParams, _>(id, params, |params| {
                    self.state
                        .storage_handle()
                        .list_active_conversation_messages(
                            params.conversation_id,
                            params.limit.unwrap_or(100),
                        )
                })
            }
            "runs.list" => with_params::<ListRunsParams, _>(id, params, |params| {
                if params.session_id.is_some() && params.conversation_id.is_some() {
                    return Err(BuddyError::Validation(
                        "run list scope must be a session or conversation, not both".to_owned(),
                    ));
                }
                let storage = self.state.storage_handle();
                match params.conversation_id {
                    Some(conversation_id) => {
                        storage.list_conversation_runs(conversation_id, params.limit.unwrap_or(100))
                    }
                    None => storage.list_runs(params.session_id, params.limit.unwrap_or(100)),
                }
            }),
            "runs.get" => with_params::<RunIdParams, _>(id, params, |params| {
                self.state.storage_handle().find_run(params.run_id)
            }),
            "runs.listChatEvents" => with_params::<RunEventsParams, _>(id, params, |params| {
                self.state.storage_handle().list_chat_run_events(
                    params.run_id,
                    params.after_id,
                    params.limit.unwrap_or(100),
                )
            }),
            "runs.listConversationChatEvents" => {
                with_params::<ConversationEventsParams, _>(id, params, |params| {
                    self.state
                        .storage_handle()
                        .list_chat_conversation_run_events(
                            params.conversation_id,
                            params.after_id,
                            params.run_limit.unwrap_or(40),
                            params.event_limit.unwrap_or(2_000),
                        )
                })
            }
            "approvals.list" => with_params::<ListApprovalsParams, _>(id, params, |params| {
                approval::list_buddy_approvals(&self.state, params.status, params.limit)
            }),
            "approvals.deny" => with_params::<ApprovalIdParams, _>(id, params, |params| {
                approval::deny_buddy_approval(&self.state, params.approval_id)
            }),
            "approvals.approveCodex" => with_params::<ApprovalIdParams, _>(id, params, |params| {
                approval::approve_buddy_codex_app_server_request_approval(
                    &self.state,
                    params.approval_id,
                )
            }),
            "workspaceState.read" => with_params::<SettingKeyParams, _>(id, params, |params| {
                self.state.read_setting_json(&params.key)
            }),
            "workspaceState.write" => with_params::<WriteSettingParams, _>(id, params, |params| {
                self.state
                    .storage_handle()
                    .write_setting_json(&params.key, params.value)
            }),
            "attachments.registerFiles" => {
                with_params::<RegisterFilesParams, _>(id, params, |params| {
                    let paths = params
                        .paths
                        .into_iter()
                        .map(PathBuf::from)
                        .collect::<Vec<_>>();
                    attachment::create_buddy_clipboard_files_from_paths(
                        &self.state,
                        &paths,
                        "file-picker",
                    )
                })
            }
            "attachments.resolvePreview" => {
                with_params::<AttachmentIdParams, _>(id, params, |params| {
                    attachment::resolve_buddy_attachment_preview(&self.state, &params.attachment_id)
                })
            }
            "attachments.release" => {
                with_params::<ReleaseAttachmentsParams, _>(id, params, |params| {
                    attachment::release_buddy_attachments(&self.state, params.attachment_ids)
                })
            }
            "attachments.cleanupDrafts" => {
                with_params::<RetainedAttachmentsParams, _>(id, params, |params| {
                    attachment::cleanup_buddy_draft_attachments(
                        &self.state,
                        params.retained_attachment_ids,
                    )
                })
            }
            "chat.startTurn" => {
                with_params::<agent_turn::StartBuddyAgentTurnRequest, _>(id, params, |request| {
                    agent_turn::start_buddy_agent_turn(
                        &self.state,
                        &self.cancellations,
                        self.event_publisher(Arc::clone(&notifications)),
                        self.host_action_executor(),
                        request,
                    )
                })
            }
            "chat.cancel" => with_params::<RunIdParams, _>(id, params, |params| {
                commands::cancel_buddy_chat_run(
                    &self.state,
                    &self.cancellations,
                    self.event_publisher(notifications),
                    params.run_id,
                )
            }),
            _ => RpcOutput::error(id, -32601, "Method not found"),
        }
    }

    fn shutdown(&self) {
        self.cancellations.shutdown();
        if let Ok(mut pet_process) = self.pet_process.lock() {
            pet_process.take();
        }
    }
}

fn runtime_notification(method: &'static str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

fn with_params<P, T>(id: String, params: Value, operation: impl FnOnce(P) -> T) -> RpcOutput
where
    P: DeserializeOwned,
    T: IntoRuntimeResult,
{
    let params = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(_) => return RpcOutput::error(id, -32602, "Invalid params"),
    };

    response(id, operation(params).into_runtime_result())
}

trait IntoRuntimeResult {
    type Output: Serialize;
    type Error: RuntimePublicError;

    fn into_runtime_result(self) -> Result<Self::Output, Self::Error>;
}

impl<T, E> IntoRuntimeResult for Result<T, E>
where
    T: Serialize,
    E: RuntimePublicError,
{
    type Output = T;
    type Error = E;

    fn into_runtime_result(self) -> Result<Self::Output, Self::Error> {
        self
    }
}

fn response<T, E>(id: String, result: Result<T, E>) -> RpcOutput
where
    T: Serialize,
    E: RuntimePublicError,
{
    match result {
        Ok(result) => match serde_json::to_value(result) {
            Ok(result) => RpcOutput::response(id, result),
            Err(error) => application_error(id, error),
        },
        Err(error) => application_error(id, error),
    }
}

trait RuntimePublicError: std::fmt::Display {
    fn public_code(&self) -> &'static str;
    fn public_message(&self) -> &'static str;
    fn retryable(&self) -> bool;
}

impl RuntimePublicError for BuddyError {
    fn public_code(&self) -> &'static str {
        self.public_code()
    }

    fn public_message(&self) -> &'static str {
        self.public_message()
    }

    fn retryable(&self) -> bool {
        self.retryable()
    }
}

impl RuntimePublicError for BuddyCommandError {
    fn public_code(&self) -> &'static str {
        self.public_code()
    }

    fn public_message(&self) -> &'static str {
        self.public_message()
    }

    fn retryable(&self) -> bool {
        self.retryable()
    }
}

impl RuntimePublicError for serde_json::Error {
    fn public_code(&self) -> &'static str {
        "RUNTIME_RESPONSE_INVALID"
    }

    fn public_message(&self) -> &'static str {
        "Runtime response is invalid"
    }

    fn retryable(&self) -> bool {
        false
    }
}

fn application_error(id: String, error: impl RuntimePublicError) -> RpcOutput {
    eprintln!("Runtime operation failed: {error}");
    RpcOutput::error_with_data(
        id,
        -32000,
        error.public_message(),
        Some(json!({
            "code": error.public_code(),
            "retryable": error.retryable(),
        })),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    name: &'static str,
    protocol_version: u32,
    ready: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitParams {
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodexContextParams {
    cwd: Option<String>,
    file_query: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizeProjectParams {
    root: String,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationIdParams {
    conversation_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationMessagesParams {
    conversation_id: String,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListRunsParams {
    session_id: Option<String>,
    conversation_id: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunIdParams {
    run_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunEventsParams {
    run_id: String,
    after_id: Option<i64>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationEventsParams {
    conversation_id: String,
    after_id: Option<i64>,
    run_limit: Option<i64>,
    event_limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListApprovalsParams {
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApprovalIdParams {
    approval_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingKeyParams {
    key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteSettingParams {
    key: String,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterFilesParams {
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttachmentIdParams {
    attachment_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseAttachmentsParams {
    attachment_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetainedAttachmentsParams {
    retained_attachment_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use crate::{
        app_paths::BuddyAppPaths,
        error::BuddyError,
        native_pet::NativePetSidecarProcess,
        protocol::parse_request,
        server::{RpcRequestHandler, RuntimeNotificationSink},
    };

    use super::{application_error, RuntimeApplication};

    #[test]
    fn application_errors_expose_only_a_stable_public_contract() {
        let output = application_error(
            "request-1".to_owned(),
            BuddyError::Validation(
                "database open failed at /home/alice/private/chat.sqlite".to_owned(),
            ),
        );
        let response = output.response_value();

        assert_eq!(
            response["error"]["message"],
            json!("Request validation failed")
        );
        assert_eq!(
            response["error"]["data"],
            json!({ "code": "VALIDATION_FAILED", "retryable": false })
        );
        assert!(!response.to_string().contains("/home/alice"));
    }

    #[test]
    fn authorizes_and_lists_a_project_through_the_runtime_protocol() {
        let root = std::env::temp_dir().join(format!(
            "lexora-buddy-runtime-protocol-test-{}",
            uuid::Uuid::new_v4()
        ));
        let data_dir = root.join("data");
        let project_dir = root.join("project");
        std::fs::create_dir_all(&project_dir).expect("create project fixture");
        let runtime =
            RuntimeApplication::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir))
                .expect("initialize runtime");
        let notifications: RuntimeNotificationSink = Arc::new(|_| {});

        let authorize = runtime.dispatch(
            parse_request(&format!(
                r#"{{"jsonrpc":"2.0","id":"authorize","method":"projects.authorize","params":{{"root":{},"name":"Demo"}}}}"#,
                serde_json::to_string(&project_dir).expect("encode project path"),
            ))
            .expect("parse authorize request"),
            Arc::clone(&notifications),
        );
        let list = runtime.dispatch(
            parse_request(
                r#"{"jsonrpc":"2.0","id":"list","method":"projects.list","params":{"limit":50}}"#,
            )
            .expect("parse list request"),
            notifications,
        );

        assert_eq!(authorize.response_value()["result"]["name"], json!("Demo"));
        assert_eq!(
            list.response_value()["result"][0]["root"],
            json!(project_dir.canonicalize().expect("canonical project path"))
        );

        std::fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn rejects_unknown_parameters_before_touching_state() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-runtime-invalid-params-test-{}",
            uuid::Uuid::new_v4()
        ));
        let runtime = RuntimeApplication::initialize_with_paths(BuddyAppPaths::from_data_dir(
            data_dir.clone(),
        ))
        .expect("initialize runtime");

        let output = runtime.dispatch(
            parse_request(
                r#"{"jsonrpc":"2.0","id":"list","method":"projects.list","params":{"limit":50,"token":"secret"}}"#,
            )
            .expect("parse envelope"),
            Arc::new(|_| {}),
        );

        assert_eq!(output.response_value()["error"]["code"], json!(-32602));
        std::fs::remove_dir_all(data_dir).expect("cleanup fixture");
    }

    #[test]
    fn native_pet_start_failure_degrades_choreography_without_failing_chat_runtime() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-runtime-pet-failure-test-{}",
            uuid::Uuid::new_v4()
        ));
        let runtime = RuntimeApplication::initialize_with_paths(BuddyAppPaths::from_data_dir(
            data_dir.clone(),
        ))
        .expect("initialize runtime");
        let notifications = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&notifications);
        let sink: RuntimeNotificationSink = Arc::new(move |value| {
            captured.lock().expect("notification lock").push(value);
        });
        let mut pet_process = None;

        runtime.finish_native_pet_start(
            &mut pet_process,
            Err::<NativePetSidecarProcess, _>(BuddyError::Runtime(
                "socket failed at /home/alice/private.sock".to_owned(),
            )),
            sink,
        );

        let readiness = runtime
            .state
            .choreography_runtime_readiness_snapshot()
            .expect("read choreography readiness");
        assert!(pet_process.is_none());
        assert_eq!(readiness.status.as_str(), "degraded");
        assert_eq!(
            readiness.reason_code.as_deref(),
            Some("runtime.nativePetSidecarUnavailable")
        );
        assert_eq!(
            notifications.lock().expect("notification lock").as_slice(),
            &[json!({
                "jsonrpc": "2.0",
                "method": "pet.state",
                "params": { "status": "offline" },
            })]
        );
        assert!(!notifications
            .lock()
            .expect("notification lock")
            .iter()
            .any(|value| value.to_string().contains("/home/alice")));

        std::fs::remove_dir_all(data_dir).expect("cleanup fixture");
    }
}
