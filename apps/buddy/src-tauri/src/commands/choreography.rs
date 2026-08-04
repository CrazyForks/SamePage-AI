use std::{cell::RefCell, fs, path::PathBuf, sync::Arc};

use tauri::State;

use crate::{
    choreography::{
        action_log::ActionLogSystemEvent,
        admission::{
            ChoreographyAdmissionDecision, ChoreographyAdmissionRelease, ChoreographyTriggerSource,
        },
        affective::{AffectiveContextStore, ResolveContext},
        create_choreography_dev_fixture_admission_request,
        executor::{
            admit_dev_fixture_with_pending_queue, admit_timeline_plan_with_pending_queue,
            create_timeline_plan_from_macro_intent, execute_admitted_dev_fixture,
            execute_admitted_timeline_plan,
            trigger_admitted_macro_intent_timeline_failure_fallback,
            trigger_admitted_runtime_safe_fallback_after_dev_fixture_failure,
            trigger_admitted_runtime_safe_fallback_after_macro_planning_failure,
            trigger_admitted_runtime_safe_fallback_after_timeline_failure,
            AdmittedDevFixtureExecution, AdmittedTimelineExecution, ChoreographyRuntimeDegradation,
            ChoreographyStepExecutor, DevFixtureAdmissionExecutionReport, DevFixtureExecutionError,
            ExecutedDevFixtureAdmission, ExecutedTimelineAdmission, MacroIntentExecutionContext,
            MacroIntentTimelineFailureFallbackRequest, NativePetChoreographyStepExecutor,
            StepCompletionDecision, TimelineAdmissionExecutionReport,
            TimelineAdmissionExecutionRequest, TimelineExecutionError,
        },
        macro_plan::MacroIntent,
        replay_policy::{
            StartupRecoverableReplayPolicyDecision, StartupRecoverableReplayPolicySummary,
        },
        timeline::TimelineStep,
        MacroIntentRunSource,
    },
    error::{BuddyError, BuddyResult},
    local_log::LocalLogTimestamp,
    native_pet::NativePetSidecarProcess,
    runtime_instance::BuddyRuntimeInstanceLock,
    state::{
        startup_recoverable_local_interaction_is_active, BuddyAppState,
        ChoreographyStepCompletionSchedule, ScheduledChoreographyExecution,
        StartupRecoverableChoreographyPendingExecutionSummary,
    },
};

use super::{
    action_source_ref::normalize_action_log_source_ref, run_buddy_blocking, BuddyCommandResult,
};

pub const STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_ARG: &str = "--buddy-startup-recoverable-replay";
pub const STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_DATA_DIR_ARG: &str =
    "--buddy-startup-recoverable-replay-data-dir";
pub const STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_ARG: &str =
    "--buddy-startup-recoverable-replay-next";
pub const STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_DATA_DIR_ARG: &str =
    "--buddy-startup-recoverable-replay-next-data-dir";
pub const STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_ARG: &str = "--buddy-startup-recoverable-list";
pub const STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_DATA_DIR_ARG: &str =
    "--buddy-startup-recoverable-list-data-dir";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunChoreographyDevFixtureRequest {
    fixture_name: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunChoreographyDevFixtureResult {
    fixture_name: String,
    plan_id: String,
    executed: bool,
    admission_decision: &'static str,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunChoreographyMacroIntentRequest {
    intent: MacroIntent,
    source_ref: Option<serde_json::Value>,
    trigger_source: Option<RunChoreographyMacroIntentTriggerSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum RunChoreographyMacroIntentTriggerSource {
    AttentionSystem,
    CriticalInteraction,
}

impl RunChoreographyMacroIntentTriggerSource {
    fn choreography_trigger_source(self) -> ChoreographyTriggerSource {
        match self {
            Self::AttentionSystem => ChoreographyTriggerSource::AttentionSystem,
            Self::CriticalInteraction => ChoreographyTriggerSource::CriticalInteraction,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunChoreographyMacroIntentResult {
    macro_id: String,
    plan_id: String,
    executed: bool,
    admission_decision: &'static str,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayStartupRecoverableChoreographyPendingExecutionRequest {
    plan_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayStartupRecoverableChoreographyPendingExecutionResult {
    plan_id: String,
    replay_status: &'static str,
    executed: bool,
    admission_decision: Option<&'static str>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayNextStartupRecoverableChoreographyPendingExecutionResult {
    plan_id: Option<String>,
    replay_status: &'static str,
    executed: bool,
    admission_decision: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRecoverableChoreographyReplayCommand {
    pub(crate) plan_id: String,
    pub(crate) data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRecoverableChoreographyReplayNextCommand {
    pub(crate) data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRecoverableChoreographyListCommand {
    pub(crate) data_dir: Option<PathBuf>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseStartupRecoverableChoreographyReplayCandidatesResult {
    items: Vec<StartupRecoverableChoreographyPendingExecutionSummary>,
    total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRecoverableChoreographyReplayCommandError {
    exit_code: i32,
    message: String,
}

impl StartupRecoverableChoreographyReplayCommandError {
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn invalid_args(message: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            message: message.into(),
        }
    }

    fn startup(error: BuddyError) -> Self {
        Self {
            exit_code: 2,
            message: error.to_string(),
        }
    }

    fn execution(error: BuddyError) -> Self {
        Self {
            exit_code: 5,
            message: error.to_string(),
        }
    }
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn runChoreographyDevFixture(
    state: State<'_, BuddyAppState>,
    pet_process: State<'_, Arc<NativePetSidecarProcess>>,
    request: RunChoreographyDevFixtureRequest,
) -> BuddyCommandResult<RunChoreographyDevFixtureResult> {
    let state = state.inner().clone();
    let pet_process = Arc::clone(pet_process.inner());

    run_buddy_blocking("runChoreographyDevFixture", move || {
        run_choreography_dev_fixture_from_state_with_executor(&state, request, pet_process.as_ref())
    })
    .await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn runChoreographyMacroIntent(
    state: State<'_, BuddyAppState>,
    pet_process: State<'_, Arc<NativePetSidecarProcess>>,
    request: RunChoreographyMacroIntentRequest,
) -> BuddyCommandResult<RunChoreographyMacroIntentResult> {
    let state = state.inner().clone();
    let pet_process = Arc::clone(pet_process.inner());

    run_buddy_blocking("runChoreographyMacroIntent", move || {
        run_choreography_macro_intent_from_state_with_executor(
            &state,
            request,
            pet_process.as_ref(),
        )
    })
    .await
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn replayStartupRecoverableChoreographyPendingExecution(
    state: State<'_, BuddyAppState>,
    pet_process: State<'_, Arc<NativePetSidecarProcess>>,
    request: ReplayStartupRecoverableChoreographyPendingExecutionRequest,
) -> BuddyCommandResult<ReplayStartupRecoverableChoreographyPendingExecutionResult> {
    let state = state.inner().clone();
    let pet_process = Arc::clone(pet_process.inner());

    run_buddy_blocking(
        "replayStartupRecoverableChoreographyPendingExecution",
        move || {
            replay_startup_recoverable_choreography_pending_execution_from_state_with_executor(
                &state,
                request,
                pet_process.as_ref(),
            )
        },
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
#[allow(non_snake_case)]
pub async fn replayNextStartupRecoverableChoreographyPendingExecution(
    state: State<'_, BuddyAppState>,
    pet_process: State<'_, Arc<NativePetSidecarProcess>>,
) -> BuddyCommandResult<ReplayNextStartupRecoverableChoreographyPendingExecutionResult> {
    let state = state.inner().clone();
    let pet_process = Arc::clone(pet_process.inner());

    run_buddy_blocking(
        "replayNextStartupRecoverableChoreographyPendingExecution",
        move || {
            replay_next_startup_recoverable_choreography_pending_execution_from_state_with_executor(
                &state,
                pet_process.as_ref(),
            )
        },
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
#[allow(non_snake_case)]
pub async fn diagnoseStartupRecoverableChoreographyReplayCandidates(
    state: State<'_, BuddyAppState>,
) -> BuddyCommandResult<DiagnoseStartupRecoverableChoreographyReplayCandidatesResult> {
    let state = state.inner().clone();

    run_buddy_blocking(
        "diagnoseStartupRecoverableChoreographyReplayCandidates",
        move || diagnose_startup_recoverable_choreography_replay_candidates_from_state(&state),
    )
    .await
}

pub fn run_startup_recoverable_choreography_replay_command_from_env(
) -> Option<Result<String, StartupRecoverableChoreographyReplayCommandError>> {
    let command = parse_startup_recoverable_choreography_replay_command(std::env::args())?;

    Some(command.and_then(run_startup_recoverable_choreography_replay_command))
}

pub fn run_startup_recoverable_choreography_replay_next_command_from_env(
) -> Option<Result<String, StartupRecoverableChoreographyReplayCommandError>> {
    let command = parse_startup_recoverable_choreography_replay_next_command(std::env::args())?;

    Some(command.and_then(run_startup_recoverable_choreography_replay_next_command))
}

pub fn run_startup_recoverable_choreography_list_command_from_env(
) -> Option<Result<String, StartupRecoverableChoreographyReplayCommandError>> {
    let command = parse_startup_recoverable_choreography_list_command(std::env::args())?;

    Some(command.and_then(run_startup_recoverable_choreography_list_command))
}

pub(crate) fn parse_startup_recoverable_choreography_list_command<I, S>(
    args: I,
) -> Option<
    Result<
        StartupRecoverableChoreographyListCommand,
        StartupRecoverableChoreographyReplayCommandError,
    >,
>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    args.iter()
        .position(|arg| arg == STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_ARG)?;
    let data_dir = match optional_command_path_arg(
        &args,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_DATA_DIR_ARG,
    ) {
        Ok(data_dir) => data_dir,
        Err(error) => return Some(Err(error)),
    };

    Some(Ok(StartupRecoverableChoreographyListCommand { data_dir }))
}

pub(crate) fn run_startup_recoverable_choreography_list_command(
    command: StartupRecoverableChoreographyListCommand,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    run_startup_recoverable_choreography_list_command_with_local_interaction_status(
        command,
        startup_recoverable_local_interaction_is_active(),
    )
}

fn run_startup_recoverable_choreography_list_command_with_local_interaction_status(
    command: StartupRecoverableChoreographyListCommand,
    local_interaction_is_active: bool,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    let source_paths = command
        .data_dir
        .map(crate::app_paths::BuddyAppPaths::from_data_dir)
        .unwrap_or_else(crate::app_paths::BuddyAppPaths::from_default_buddy_home);
    let snapshot_paths = create_startup_recoverable_choreography_list_snapshot(&source_paths)
        .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;
    let snapshot_data_dir = snapshot_paths.data_dir_path();
    let result = (|| {
        let state = BuddyAppState::initialize_with_paths(snapshot_paths)
            .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;
        let output = diagnose_startup_recoverable_choreography_replay_candidates_from_state_with_local_interaction_status(
            &state,
            local_interaction_is_active,
        )
        .map_err(StartupRecoverableChoreographyReplayCommandError::execution)?;

        serde_json::to_string(&output).map_err(|error| {
            StartupRecoverableChoreographyReplayCommandError::execution(BuddyError::from(error))
        })
    })();
    let _ = fs::remove_dir_all(snapshot_data_dir);

    result
}

fn diagnose_startup_recoverable_choreography_replay_candidates_from_state(
    state: &BuddyAppState,
) -> BuddyResult<DiagnoseStartupRecoverableChoreographyReplayCandidatesResult> {
    diagnose_startup_recoverable_choreography_replay_candidates_from_state_with_local_interaction_status(
        state,
        startup_recoverable_local_interaction_is_active(),
    )
}

fn diagnose_startup_recoverable_choreography_replay_candidates_from_state_with_local_interaction_status(
    state: &BuddyAppState,
    local_interaction_is_active: bool,
) -> BuddyResult<DiagnoseStartupRecoverableChoreographyReplayCandidatesResult> {
    let items = state
        .startup_recoverable_choreography_pending_summaries_with_local_interaction_status(
            local_interaction_is_active,
        )?;

    Ok(
        DiagnoseStartupRecoverableChoreographyReplayCandidatesResult {
            total: items.len(),
            items,
        },
    )
}

fn create_startup_recoverable_choreography_list_snapshot(
    source_paths: &crate::app_paths::BuddyAppPaths,
) -> BuddyResult<crate::app_paths::BuddyAppPaths> {
    let snapshot_data_dir = std::env::temp_dir().join(format!(
        "lexora-buddy-startup-recoverable-list-snapshot-{}",
        uuid::Uuid::new_v4()
    ));
    let snapshot_paths = crate::app_paths::BuddyAppPaths::from_data_dir(snapshot_data_dir);
    snapshot_paths.ensure_exists()?;

    let source_action_log_path = source_paths
        .data_dir_path()
        .join("action-log")
        .join("events.jsonl");
    if source_action_log_path.exists() {
        let snapshot_action_log_path = snapshot_paths
            .data_dir_path()
            .join("action-log")
            .join("events.jsonl");
        if let Some(parent) = snapshot_action_log_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source_action_log_path, snapshot_action_log_path)?;
    }

    Ok(snapshot_paths)
}

pub(crate) fn parse_startup_recoverable_choreography_replay_command<I, S>(
    args: I,
) -> Option<
    Result<
        StartupRecoverableChoreographyReplayCommand,
        StartupRecoverableChoreographyReplayCommandError,
    >,
>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let replay_arg_index = args
        .iter()
        .position(|arg| arg == STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_ARG)?;
    let Some(plan_id) = args.get(replay_arg_index + 1) else {
        return Some(Err(
            StartupRecoverableChoreographyReplayCommandError::invalid_args(
                "--buddy-startup-recoverable-replay requires a plan id",
            ),
        ));
    };
    if plan_id.starts_with("--") {
        return Some(Err(
            StartupRecoverableChoreographyReplayCommandError::invalid_args(
                "--buddy-startup-recoverable-replay requires a plan id",
            ),
        ));
    }
    let plan_id =
        match normalize_startup_recoverable_choreography_pending_plan_id(plan_id.to_owned()) {
            Ok(plan_id) => plan_id,
            Err(error) => {
                return Some(Err(
                    StartupRecoverableChoreographyReplayCommandError::invalid_args(
                        error.to_string(),
                    ),
                ));
            }
        };
    let data_dir = match optional_command_path_arg(
        &args,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_DATA_DIR_ARG,
    ) {
        Ok(data_dir) => data_dir,
        Err(error) => return Some(Err(error)),
    };

    Some(Ok(StartupRecoverableChoreographyReplayCommand {
        plan_id,
        data_dir,
    }))
}

pub(crate) fn parse_startup_recoverable_choreography_replay_next_command<I, S>(
    args: I,
) -> Option<
    Result<
        StartupRecoverableChoreographyReplayNextCommand,
        StartupRecoverableChoreographyReplayCommandError,
    >,
>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    args.iter()
        .position(|arg| arg == STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_ARG)?;
    let data_dir = match optional_command_path_arg(
        &args,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_DATA_DIR_ARG,
    ) {
        Ok(data_dir) => data_dir,
        Err(error) => return Some(Err(error)),
    };

    Some(Ok(StartupRecoverableChoreographyReplayNextCommand {
        data_dir,
    }))
}

fn run_startup_recoverable_choreography_replay_command(
    command: StartupRecoverableChoreographyReplayCommand,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    let paths = startup_recoverable_choreography_command_paths(command.data_dir.as_ref());
    let _runtime_instance_lock = BuddyRuntimeInstanceLock::acquire(&paths.data_dir_path())
        .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;
    let executor = NativePetChoreographyStepExecutor::spawn_sidecar()
        .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;

    run_startup_recoverable_choreography_replay_command_with_executor(command, &executor)
}

pub(crate) fn run_startup_recoverable_choreography_replay_command_with_executor(
    command: StartupRecoverableChoreographyReplayCommand,
    executor: &impl ChoreographyStepExecutor,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    run_startup_recoverable_choreography_replay_command_with_executor_and_local_interaction_status(
        command,
        executor,
        startup_recoverable_local_interaction_is_active(),
    )
}

fn run_startup_recoverable_choreography_replay_command_with_executor_and_local_interaction_status(
    command: StartupRecoverableChoreographyReplayCommand,
    executor: &impl ChoreographyStepExecutor,
    local_interaction_is_active: bool,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    let paths = command
        .data_dir
        .map(crate::app_paths::BuddyAppPaths::from_data_dir)
        .unwrap_or_else(crate::app_paths::BuddyAppPaths::from_default_buddy_home);
    let state = BuddyAppState::initialize_with_paths(paths)
        .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;
    let result =
        replay_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
            &state,
            ReplayStartupRecoverableChoreographyPendingExecutionRequest {
                plan_id: command.plan_id,
            },
            executor,
            local_interaction_is_active,
        )
        .map_err(StartupRecoverableChoreographyReplayCommandError::execution)?;

    serde_json::to_string(&result).map_err(|error| {
        StartupRecoverableChoreographyReplayCommandError::execution(BuddyError::from(error))
    })
}

fn run_startup_recoverable_choreography_replay_next_command(
    command: StartupRecoverableChoreographyReplayNextCommand,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    let paths = startup_recoverable_choreography_command_paths(command.data_dir.as_ref());
    let _runtime_instance_lock = BuddyRuntimeInstanceLock::acquire(&paths.data_dir_path())
        .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;
    let executor = NativePetChoreographyStepExecutor::spawn_sidecar()
        .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;

    run_startup_recoverable_choreography_replay_next_command_with_executor(command, &executor)
}

pub(crate) fn run_startup_recoverable_choreography_replay_next_command_with_executor(
    command: StartupRecoverableChoreographyReplayNextCommand,
    executor: &impl ChoreographyStepExecutor,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    run_startup_recoverable_choreography_replay_next_command_with_executor_and_local_interaction_status(
        command,
        executor,
        startup_recoverable_local_interaction_is_active(),
    )
}

fn run_startup_recoverable_choreography_replay_next_command_with_executor_and_local_interaction_status(
    command: StartupRecoverableChoreographyReplayNextCommand,
    executor: &impl ChoreographyStepExecutor,
    local_interaction_is_active: bool,
) -> Result<String, StartupRecoverableChoreographyReplayCommandError> {
    let paths = command
        .data_dir
        .map(crate::app_paths::BuddyAppPaths::from_data_dir)
        .unwrap_or_else(crate::app_paths::BuddyAppPaths::from_default_buddy_home);
    let state = BuddyAppState::initialize_with_paths(paths)
        .map_err(StartupRecoverableChoreographyReplayCommandError::startup)?;
    let result =
        replay_next_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
            &state,
            executor,
            local_interaction_is_active,
        )
        .map_err(StartupRecoverableChoreographyReplayCommandError::execution)?;

    serde_json::to_string(&result).map_err(|error| {
        StartupRecoverableChoreographyReplayCommandError::execution(BuddyError::from(error))
    })
}

fn startup_recoverable_choreography_command_paths(
    data_dir: Option<&PathBuf>,
) -> crate::app_paths::BuddyAppPaths {
    data_dir
        .cloned()
        .map(crate::app_paths::BuddyAppPaths::from_data_dir)
        .unwrap_or_else(crate::app_paths::BuddyAppPaths::from_default_buddy_home)
}

fn optional_command_path_arg(
    args: &[String],
    name: &str,
) -> Result<Option<PathBuf>, StartupRecoverableChoreographyReplayCommandError> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    let Some(value) = args.get(index + 1) else {
        return Err(
            StartupRecoverableChoreographyReplayCommandError::invalid_args(format!(
                "{name} requires a path"
            )),
        );
    };
    if value.starts_with("--") {
        return Err(
            StartupRecoverableChoreographyReplayCommandError::invalid_args(format!(
                "{name} requires a path"
            )),
        );
    }

    Ok(Some(PathBuf::from(value)))
}

fn run_choreography_dev_fixture_from_state_with_executor(
    state: &BuddyAppState,
    request: RunChoreographyDevFixtureRequest,
    executor: &impl ChoreographyStepExecutor,
) -> BuddyResult<RunChoreographyDevFixtureResult> {
    let fixture_name = request.fixture_name.trim().to_owned();
    let storage = state.storage_handle();
    let resolve_context = AffectiveContextStore::from_buddy_home(state.data_dir_path())
        .read_or_create_default_with_diagnostics(&storage)
        .map(ResolveContext::from_affective_snapshot)?;

    run_choreography_dev_fixture_from_state_with_executor_and_context(
        state,
        fixture_name,
        storage,
        resolve_context,
        executor,
    )
}

fn run_choreography_dev_fixture_from_state_with_executor_and_context(
    state: &BuddyAppState,
    fixture_name: String,
    storage: crate::storage::BuddyStorage,
    resolve_context: ResolveContext,
    executor: &impl ChoreographyStepExecutor,
) -> BuddyResult<RunChoreographyDevFixtureResult> {
    let request = create_choreography_dev_fixture_admission_request(
        fixture_name.as_str(),
        storage.clone(),
        resolve_context,
    )
    .map_err(dev_fixture_execution_error_to_buddy_error)?;
    let scheduled = state.with_choreography_dev_fixture_scheduler(|admission, pending_queue| {
        admit_dev_fixture_with_pending_queue(admission, pending_queue, request)
            .map_err(dev_fixture_execution_error_to_buddy_error)
    })?;
    let report = match scheduled.execution {
        Some(execution) => {
            let executed =
                execute_dev_fixture_with_state_scheduler(state, storage, executor, execution)?;
            executed.report
        }
        None => DevFixtureAdmissionExecutionReport {
            plan_id: scheduled.plan_id,
            decision: scheduled.decision,
            executed: false,
        },
    };

    Ok(RunChoreographyDevFixtureResult {
        fixture_name,
        plan_id: report.plan_id,
        executed: report.executed,
        admission_decision: choreography_admission_decision_value(&report.decision),
    })
}

fn run_choreography_macro_intent_from_state_with_executor(
    state: &BuddyAppState,
    request: RunChoreographyMacroIntentRequest,
    executor: &impl ChoreographyStepExecutor,
) -> BuddyResult<RunChoreographyMacroIntentResult> {
    let macro_id = request.intent.macro_id().to_owned();
    let storage = state.storage_handle();
    let source =
        macro_intent_run_source_from_request(&storage, request.source_ref, request.trigger_source)?;
    let resolve_context = AffectiveContextStore::from_buddy_home(state.data_dir_path())
        .read_or_create_default_with_diagnostics(&storage)
        .map(ResolveContext::from_affective_snapshot)?;

    run_choreography_macro_intent_from_state_with_executor_and_context(
        state,
        request.intent,
        storage,
        resolve_context,
        source,
        executor,
        macro_id,
    )
}

fn macro_intent_run_source_from_request(
    storage: &crate::storage::BuddyStorage,
    source_ref: Option<serde_json::Value>,
    trigger_source: Option<RunChoreographyMacroIntentTriggerSource>,
) -> BuddyResult<MacroIntentRunSource> {
    let source_ref = normalize_action_log_source_ref(storage, source_ref)?;
    let Some(trigger_source) = trigger_source else {
        return Ok(source_ref
            .map(|source_ref| MacroIntentRunSource {
                source_ref,
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            })
            .unwrap_or_else(MacroIntentRunSource::user_requested_dev_fixture));
    };
    let Some(source_ref) = source_ref else {
        return Err(BuddyError::Validation(
            "macro intent triggerSource requires sourceRef".to_owned(),
        ));
    };

    Ok(MacroIntentRunSource {
        source_ref,
        trigger_source: trigger_source.choreography_trigger_source(),
    })
}

fn run_choreography_macro_intent_from_state_with_executor_and_context(
    state: &BuddyAppState,
    intent: MacroIntent,
    storage: crate::storage::BuddyStorage,
    resolve_context: ResolveContext,
    source: MacroIntentRunSource,
    executor: &impl ChoreographyStepExecutor,
    macro_id: String,
) -> BuddyResult<RunChoreographyMacroIntentResult> {
    let trigger_source = source.trigger_source;
    let context = MacroIntentExecutionContext::new();
    let plan = match create_timeline_plan_from_macro_intent(&intent, &context, source.source_ref) {
        Ok(plan) => plan,
        Err(error) => {
            let degradation = state.with_choreography_admission(|admission| {
                Ok(
                    trigger_admitted_runtime_safe_fallback_after_macro_planning_failure(
                        storage.clone(),
                        executor,
                        admission,
                        &context,
                        resolve_context,
                    ),
                )
            })?;
            mark_choreography_runtime_degraded_if_needed(state, degradation)?;
            return Err(error);
        }
    };
    recover_choreography_runtime_readiness_before_admission_if_allowed(
        state,
        trigger_source,
        executor,
    )?;
    let scheduled = state.with_choreography_timeline_scheduler(|admission, pending_queue| {
        admit_timeline_plan_with_pending_queue(
            admission,
            pending_queue,
            TimelineAdmissionExecutionRequest::new(
                storage.clone(),
                plan,
                context.timeline,
                resolve_context.clone(),
                trigger_source,
            ),
        )
        .map_err(timeline_execution_error_to_buddy_error)
    })?;
    let report = match scheduled.execution {
        Some(execution) => {
            let executed = execute_macro_timeline_with_state_scheduler(
                state,
                storage.clone(),
                executor,
                &intent,
                trigger_source,
                execution,
            )?;
            executed.report
        }
        None => TimelineAdmissionExecutionReport {
            plan_id: scheduled.plan_id,
            decision: scheduled.decision,
            executed: false,
        },
    };

    Ok(RunChoreographyMacroIntentResult {
        macro_id,
        plan_id: report.plan_id,
        executed: report.executed,
        admission_decision: choreography_admission_decision_value(&report.decision),
    })
}

fn replay_startup_recoverable_choreography_pending_execution_from_state_with_executor(
    state: &BuddyAppState,
    request: ReplayStartupRecoverableChoreographyPendingExecutionRequest,
    executor: &impl ChoreographyStepExecutor,
) -> BuddyResult<ReplayStartupRecoverableChoreographyPendingExecutionResult> {
    replay_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
        state,
        request,
        executor,
        startup_recoverable_local_interaction_is_active(),
    )
}

fn replay_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
    state: &BuddyAppState,
    request: ReplayStartupRecoverableChoreographyPendingExecutionRequest,
    executor: &impl ChoreographyStepExecutor,
    local_interaction_is_active: bool,
) -> BuddyResult<ReplayStartupRecoverableChoreographyPendingExecutionResult> {
    let plan_id = normalize_startup_recoverable_choreography_pending_plan_id(request.plan_id)?;
    let has_recoverable_entry = state
        .startup_recoverable_choreography_pending_plan_ids()?
        .iter()
        .any(|recoverable_plan_id| recoverable_plan_id == &plan_id);
    let Some(scheduled) = state
        .schedule_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
            plan_id.as_str(),
            local_interaction_is_active,
        )?
    else {
        let replay_status = if has_recoverable_entry {
            startup_recoverable_choreography_replay_status_for_unscheduled_entry_with_local_interaction_status(
                state,
                plan_id.as_str(),
                local_interaction_is_active,
            )?
        } else {
            "notFound"
        };
        return Ok(ReplayStartupRecoverableChoreographyPendingExecutionResult {
            plan_id,
            replay_status,
            executed: false,
            admission_decision: None,
        });
    };

    let admission_decision = scheduled_choreography_admission_decision_value(&scheduled);
    let executed = scheduled_choreography_has_execution(&scheduled);
    flush_scheduled_pending_choreography_plans(state, executor, Some(scheduled))?;

    Ok(ReplayStartupRecoverableChoreographyPendingExecutionResult {
        plan_id,
        replay_status: if executed { "executed" } else { "scheduled" },
        executed,
        admission_decision: Some(admission_decision),
    })
}

fn replay_next_startup_recoverable_choreography_pending_execution_from_state_with_executor(
    state: &BuddyAppState,
    executor: &impl ChoreographyStepExecutor,
) -> BuddyResult<ReplayNextStartupRecoverableChoreographyPendingExecutionResult> {
    replay_next_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
        state,
        executor,
        startup_recoverable_local_interaction_is_active(),
    )
}

fn replay_next_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
    state: &BuddyAppState,
    executor: &impl ChoreographyStepExecutor,
    local_interaction_is_active: bool,
) -> BuddyResult<ReplayNextStartupRecoverableChoreographyPendingExecutionResult> {
    let next_plan_id = state
        .startup_recoverable_choreography_pending_plan_ids()?
        .into_iter()
        .next();
    let Some(scheduled) = state
        .schedule_next_startup_recoverable_choreography_pending_execution_with_local_interaction_status(
            local_interaction_is_active,
        )?
    else {
        let replay_status = if let Some(plan_id) = next_plan_id.as_deref() {
            startup_recoverable_choreography_replay_status_for_unscheduled_entry_with_local_interaction_status(
                state,
                plan_id,
                local_interaction_is_active,
            )?
        } else {
            "notFound"
        };
        return Ok(
            ReplayNextStartupRecoverableChoreographyPendingExecutionResult {
                plan_id: next_plan_id,
                replay_status,
                executed: false,
                admission_decision: None,
            },
        );
    };

    let plan_id = scheduled_choreography_plan_id(&scheduled).to_owned();
    let admission_decision = scheduled_choreography_admission_decision_value(&scheduled);
    let executed = scheduled_choreography_has_execution(&scheduled);
    flush_scheduled_pending_choreography_plans(state, executor, Some(scheduled))?;

    Ok(
        ReplayNextStartupRecoverableChoreographyPendingExecutionResult {
            plan_id: Some(plan_id),
            replay_status: if executed { "executed" } else { "scheduled" },
            executed,
            admission_decision: Some(admission_decision),
        },
    )
}

fn startup_recoverable_choreography_replay_status_for_unscheduled_entry_with_local_interaction_status(
    state: &BuddyAppState,
    plan_id: &str,
    local_interaction_is_active: bool,
) -> BuddyResult<&'static str> {
    let summary = state
        .startup_recoverable_choreography_pending_summaries_with_local_interaction_status(
            local_interaction_is_active,
        )?
        .into_iter()
        .find(|summary| summary.plan_id() == plan_id);
    let Some(summary) = summary else {
        return Ok("waitingForIdle");
    };

    Ok(startup_recoverable_choreography_replay_status_for_policy(
        summary.replay_policy(),
    ))
}

fn startup_recoverable_choreography_replay_status_for_policy(
    replay_policy: &StartupRecoverableReplayPolicySummary,
) -> &'static str {
    match replay_policy.decision {
        StartupRecoverableReplayPolicyDecision::Reject => "rejectedByPolicy",
        StartupRecoverableReplayPolicyDecision::Wait => {
            if replay_policy.reason_code == "replay.localInteractionActive" {
                return "waitingForLocalInteraction";
            }

            "waitingForIdle"
        }
        StartupRecoverableReplayPolicyDecision::Candidate
        | StartupRecoverableReplayPolicyDecision::Manual => "waitingForIdle",
    }
}

fn execute_macro_timeline_with_state_scheduler(
    state: &BuddyAppState,
    storage: crate::storage::BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    intent: &MacroIntent,
    trigger_source: ChoreographyTriggerSource,
    execution: AdmittedTimelineExecution,
) -> BuddyResult<ExecutedTimelineAdmission> {
    let scheduled_pending = ScheduledChoreographyCapture::default();
    let executed = execute_admitted_timeline_plan(
        executor,
        execution,
        |plan_id, step, interrupt_policy| {
            state.refresh_choreography_plan_active_step(plan_id, step.step_id(), interrupt_policy)
        },
        |plan_id, step| scheduled_pending.capture_after_completed_step(state, plan_id, step),
        |plan_id| state.release_choreography_plan_preserving_pending(plan_id),
        |plan_id| scheduled_pending.capture_release(state, plan_id),
        |failed_plan, failed_step_id, error, resolve_context| {
            let degradation = state.with_choreography_admission(|admission| {
                trigger_admitted_macro_intent_timeline_failure_fallback(
                    storage.clone(),
                    executor,
                    admission,
                    MacroIntentTimelineFailureFallbackRequest {
                        original_intent: intent,
                        failed_plan,
                        failed_step_id,
                        error,
                        resolve_context,
                        trigger_source,
                    },
                )
            })?;
            if degradation.is_none() {
                scheduled_pending
                    .capture_pending_after_failed_recovery(state, failed_plan.plan_id.as_str())?;
            }
            mark_choreography_runtime_degraded_if_needed(state, degradation)
        },
    )
    .map_err(timeline_execution_error_to_buddy_error);

    flush_scheduled_pending_choreography_plans(
        state,
        executor,
        scheduled_pending.into_scheduled(),
    )?;

    executed
}

fn execute_dev_fixture_with_state_scheduler(
    state: &BuddyAppState,
    storage: crate::storage::BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    execution: AdmittedDevFixtureExecution,
) -> BuddyResult<ExecutedDevFixtureAdmission> {
    let scheduled_pending = ScheduledChoreographyCapture::default();
    let executed = execute_admitted_dev_fixture(
        executor,
        execution,
        |plan_id, step, interrupt_policy| {
            state.refresh_choreography_plan_active_step(plan_id, step.step_id(), interrupt_policy)
        },
        |plan_id, step| scheduled_pending.capture_after_completed_step(state, plan_id, step),
        |plan_id| state.release_choreography_plan_preserving_pending(plan_id),
        |plan_id| scheduled_pending.capture_release(state, plan_id),
        |failed_plan, failed_step_id, error, resolve_context| {
            let degradation = state.with_choreography_admission(|admission| {
                Ok(
                    trigger_admitted_runtime_safe_fallback_after_dev_fixture_failure(
                        storage.clone(),
                        executor,
                        admission,
                        failed_plan,
                        failed_step_id,
                        error,
                        resolve_context,
                    ),
                )
            })?;
            if degradation.is_none() {
                scheduled_pending
                    .capture_pending_after_failed_recovery(state, failed_plan.plan_id.as_str())?;
            }
            mark_choreography_runtime_degraded_if_needed(state, degradation)
        },
    )
    .map_err(dev_fixture_execution_error_to_buddy_error);

    flush_scheduled_pending_choreography_plans(
        state,
        executor,
        scheduled_pending.into_scheduled(),
    )?;

    executed
}

pub(super) fn flush_scheduled_pending_choreography_plans(
    state: &BuddyAppState,
    executor: &impl ChoreographyStepExecutor,
    initial_scheduled: Option<ScheduledChoreographyExecution>,
) -> BuddyResult<()> {
    let mut scheduled = initial_scheduled;
    while let Some(next) = scheduled {
        scheduled = execute_scheduled_pending_choreography_plan(state, executor, next)?;
    }

    Ok(())
}

fn execute_scheduled_pending_choreography_plan(
    state: &BuddyAppState,
    executor: &impl ChoreographyStepExecutor,
    scheduled: ScheduledChoreographyExecution,
) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
    match scheduled {
        ScheduledChoreographyExecution::Timeline(scheduled) => {
            execute_scheduled_pending_timeline_plan(state, executor, scheduled)
        }
        ScheduledChoreographyExecution::DevFixture(scheduled) => {
            execute_scheduled_pending_dev_fixture(state, executor, scheduled)
        }
    }
}

fn execute_scheduled_pending_timeline_plan(
    state: &BuddyAppState,
    executor: &impl ChoreographyStepExecutor,
    scheduled: crate::choreography::executor::ScheduledTimelineExecution,
) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
    let Some(execution) = scheduled.execution else {
        return Ok(None);
    };
    let scheduled_pending = ScheduledChoreographyCapture::default();

    let executed = execute_admitted_timeline_plan(
        executor,
        execution,
        |plan_id, step, interrupt_policy| {
            state.refresh_choreography_plan_active_step(plan_id, step.step_id(), interrupt_policy)
        },
        |plan_id, step| scheduled_pending.capture_after_completed_step(state, plan_id, step),
        |plan_id| state.release_choreography_plan_preserving_pending(plan_id),
        |plan_id| scheduled_pending.capture_release(state, plan_id),
        |failed_plan, failed_step_id, error, resolve_context| {
            let degradation = state.with_choreography_admission(|admission| {
                Ok(
                    trigger_admitted_runtime_safe_fallback_after_timeline_failure(
                        state.storage_handle(),
                        executor,
                        admission,
                        failed_plan,
                        failed_step_id,
                        error,
                        resolve_context,
                    ),
                )
            })?;
            if degradation.is_none() {
                scheduled_pending
                    .capture_pending_after_failed_recovery(state, failed_plan.plan_id.as_str())?;
            }
            mark_choreography_runtime_degraded_if_needed(state, degradation)
        },
    )
    .map_err(timeline_execution_error_to_buddy_error);

    let next_scheduled = scheduled_pending.into_scheduled();
    match executed {
        Ok(_) => Ok(next_scheduled),
        Err(error) => {
            flush_scheduled_pending_choreography_plans(state, executor, next_scheduled)?;
            Err(error)
        }
    }
}

fn execute_scheduled_pending_dev_fixture(
    state: &BuddyAppState,
    executor: &impl ChoreographyStepExecutor,
    scheduled: crate::choreography::executor::ScheduledDevFixtureExecution,
) -> BuddyResult<Option<ScheduledChoreographyExecution>> {
    let Some(execution) = scheduled.execution else {
        return Ok(None);
    };
    let scheduled_pending = ScheduledChoreographyCapture::default();

    let executed = execute_admitted_dev_fixture(
        executor,
        execution,
        |plan_id, step, interrupt_policy| {
            state.refresh_choreography_plan_active_step(plan_id, step.step_id(), interrupt_policy)
        },
        |plan_id, step| scheduled_pending.capture_after_completed_step(state, plan_id, step),
        |plan_id| state.release_choreography_plan_preserving_pending(plan_id),
        |plan_id| scheduled_pending.capture_release(state, plan_id),
        |failed_plan, failed_step_id, error, resolve_context| {
            let degradation = state.with_choreography_admission(|admission| {
                Ok(
                    trigger_admitted_runtime_safe_fallback_after_dev_fixture_failure(
                        state.storage_handle(),
                        executor,
                        admission,
                        failed_plan,
                        failed_step_id,
                        error,
                        resolve_context,
                    ),
                )
            })?;
            if degradation.is_none() {
                scheduled_pending
                    .capture_pending_after_failed_recovery(state, failed_plan.plan_id.as_str())?;
            }
            mark_choreography_runtime_degraded_if_needed(state, degradation)
        },
    )
    .map_err(dev_fixture_execution_error_to_buddy_error);

    let next_scheduled = scheduled_pending.into_scheduled();
    match executed {
        Ok(_) => Ok(next_scheduled),
        Err(error) => {
            flush_scheduled_pending_choreography_plans(state, executor, next_scheduled)?;
            Err(error)
        }
    }
}

#[derive(Default)]
struct ScheduledChoreographyCapture {
    scheduled: RefCell<Option<ScheduledChoreographyExecution>>,
}

impl ScheduledChoreographyCapture {
    fn capture_release(
        &self,
        state: &BuddyAppState,
        plan_id: &str,
    ) -> BuddyResult<ChoreographyAdmissionRelease> {
        let release = state.release_choreography_plan_and_schedule_pending(plan_id)?;
        if release.scheduled.is_some() {
            *self.scheduled.borrow_mut() = release.scheduled;
        }

        Ok(release.release)
    }

    fn capture_pending_after_failed_recovery(
        &self,
        state: &BuddyAppState,
        released_plan_id: &str,
    ) -> BuddyResult<()> {
        if let Some(scheduled) =
            state.schedule_pending_choreography_plan_if_idle(released_plan_id)?
        {
            *self.scheduled.borrow_mut() = Some(scheduled);
        }

        Ok(())
    }

    fn capture_after_completed_step(
        &self,
        state: &BuddyAppState,
        plan_id: &str,
        step: &TimelineStep,
    ) -> BuddyResult<StepCompletionDecision> {
        match state.schedule_pending_after_completed_choreography_step(
            plan_id,
            step.pending_handoff_finalizer_step_id(),
        )? {
            ChoreographyStepCompletionSchedule::Continue => Ok(StepCompletionDecision::Continue),
            ChoreographyStepCompletionSchedule::RunPendingHandoffFinalizer { step_id } => {
                Ok(StepCompletionDecision::RunPendingHandoffFinalizer { step_id })
            }
            ChoreographyStepCompletionSchedule::YieldToPendingPlan(scheduled) => {
                *self.scheduled.borrow_mut() = Some(*scheduled);
                Ok(StepCompletionDecision::YieldToPendingPlan)
            }
        }
    }

    fn into_scheduled(self) -> Option<ScheduledChoreographyExecution> {
        self.scheduled.into_inner()
    }
}

fn dev_fixture_execution_error_to_buddy_error(error: DevFixtureExecutionError) -> BuddyError {
    match error {
        DevFixtureExecutionError::ActionLog(error) | DevFixtureExecutionError::Execution(error) => {
            error
        }
    }
}

fn mark_choreography_runtime_degraded_if_needed(
    state: &BuddyAppState,
    degradation: Option<ChoreographyRuntimeDegradation>,
) -> BuddyResult<()> {
    let Some(degradation) = degradation else {
        return Ok(());
    };

    state.mark_choreography_runtime_degraded(degradation.reason_code, degradation.degraded_at)?;
    Ok(())
}

fn recover_choreography_runtime_readiness_before_admission_if_allowed(
    state: &BuddyAppState,
    trigger_source: ChoreographyTriggerSource,
    executor: &impl ChoreographyStepExecutor,
) -> BuddyResult<()> {
    let readiness = state.choreography_runtime_readiness_snapshot()?;
    if readiness.accepting_choreography {
        return Ok(());
    }
    if !choreography_trigger_source_allows_readiness_health_gate(trigger_source) {
        return Ok(());
    }

    let checked_at = LocalLogTimestamp::now_utc().to_rfc3339_millis();
    if let Err(error) = executor.query_state_position() {
        state
            .storage_handle()
            .append_choreography_action_log_system_event(
                &ActionLogSystemEvent::health_gate_failed(
                    format!("event_{}", uuid::Uuid::now_v7()),
                    trigger_source,
                    &error.to_string(),
                    checked_at,
                ),
            )?;
        return Err(error);
    }
    state
        .storage_handle()
        .append_choreography_action_log_system_event(&ActionLogSystemEvent::health_gate_passed(
            format!("event_{}", uuid::Uuid::now_v7()),
            trigger_source,
            checked_at.clone(),
        ))?;
    state.mark_choreography_runtime_ready(checked_at)?;
    Ok(())
}

fn choreography_trigger_source_allows_readiness_health_gate(
    trigger_source: ChoreographyTriggerSource,
) -> bool {
    matches!(
        trigger_source,
        ChoreographyTriggerSource::AttentionSystem | ChoreographyTriggerSource::CriticalInteraction
    )
}

fn timeline_execution_error_to_buddy_error(error: TimelineExecutionError) -> BuddyError {
    match error {
        TimelineExecutionError::ActionLog(error) | TimelineExecutionError::Execution(error) => {
            error
        }
    }
}

fn choreography_admission_decision_value(decision: &ChoreographyAdmissionDecision) -> &'static str {
    match decision {
        ChoreographyAdmissionDecision::Accepted { .. } => "accepted",
        ChoreographyAdmissionDecision::Preempted { .. } => "preempted",
        ChoreographyAdmissionDecision::Rejected { .. } => "rejected",
        ChoreographyAdmissionDecision::Deferred { .. } => "deferred",
        ChoreographyAdmissionDecision::Skipped { .. } => "skipped",
    }
}

fn scheduled_choreography_admission_decision_value(
    scheduled: &ScheduledChoreographyExecution,
) -> &'static str {
    match scheduled {
        ScheduledChoreographyExecution::Timeline(scheduled) => {
            choreography_admission_decision_value(&scheduled.decision)
        }
        ScheduledChoreographyExecution::DevFixture(scheduled) => {
            choreography_admission_decision_value(&scheduled.decision)
        }
    }
}

fn scheduled_choreography_plan_id(scheduled: &ScheduledChoreographyExecution) -> &str {
    match scheduled {
        ScheduledChoreographyExecution::Timeline(scheduled) => scheduled.plan_id.as_str(),
        ScheduledChoreographyExecution::DevFixture(scheduled) => scheduled.plan_id.as_str(),
    }
}

fn scheduled_choreography_has_execution(scheduled: &ScheduledChoreographyExecution) -> bool {
    match scheduled {
        ScheduledChoreographyExecution::Timeline(scheduled) => scheduled.execution.is_some(),
        ScheduledChoreographyExecution::DevFixture(scheduled) => scheduled.execution.is_some(),
    }
}

fn normalize_startup_recoverable_choreography_pending_plan_id(
    plan_id: String,
) -> BuddyResult<String> {
    let plan_id = plan_id.trim();
    if plan_id.is_empty() {
        return Err(BuddyError::Validation(
            "startup recoverable choreography pending plan id is required".to_owned(),
        ));
    }

    Ok(plan_id.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
        thread,
        time::Duration,
    };

    use crate::{
        app_paths::BuddyAppPaths,
        choreography::{
            action_log::ActionLogEvent,
            admission::{
                ChoreographyAdmissionDecision, ChoreographyAdmissionRequest,
                ChoreographyPlanPriority, ChoreographyTriggerSource,
            },
            affective::ResolveContext,
            executor::{ChoreographyStepExecutor, TimelineExecutionContext},
            macro_plan::MacroIntent,
            registry::StepResolution,
            timeline::{
                MoveByPathStep, MoveTarget, MoveToStep, PlayActionStep, TimelineFailurePolicy,
                TimelinePlan, TimelineStep, WaitStep,
            },
            MacroIntentRunSource,
        },
        error::{BuddyError, BuddyResult},
        local_log::LocalLogTimestamp,
        native_pet::step_protocol::SidecarInterruptPolicy,
        state::BuddyAppState,
        storage::{
            ActionLogSystemEventQueryRequest, AppendBuddyConversationMessageRequest, BuddyStorage,
            ChoreographyPendingExecutionBodyKind, CreateBuddyConversationRequest,
            CreateBuddyConversationRunRequest, UpsertChoreographyPendingExecutionBodyRequest,
        },
    };

    use super::{
        admit_timeline_plan_with_pending_queue,
        diagnose_startup_recoverable_choreography_replay_candidates_from_state_with_local_interaction_status,
        execute_admitted_timeline_plan, flush_scheduled_pending_choreography_plans,
        parse_startup_recoverable_choreography_list_command,
        parse_startup_recoverable_choreography_replay_command,
        parse_startup_recoverable_choreography_replay_next_command,
        replay_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status,
        run_choreography_macro_intent_from_state_with_executor,
        run_choreography_macro_intent_from_state_with_executor_and_context,
        run_startup_recoverable_choreography_list_command_with_local_interaction_status,
        run_startup_recoverable_choreography_replay_command_with_executor_and_local_interaction_status,
        run_startup_recoverable_choreography_replay_next_command_with_executor_and_local_interaction_status,
        timeline_execution_error_to_buddy_error, AdmittedTimelineExecution,
        ReplayStartupRecoverableChoreographyPendingExecutionRequest,
        RunChoreographyMacroIntentRequest, RunChoreographyMacroIntentTriggerSource,
        ScheduledChoreographyCapture, StartupRecoverableChoreographyListCommand,
        StartupRecoverableChoreographyReplayCommand,
        StartupRecoverableChoreographyReplayNextCommand, TimelineAdmissionExecutionReport,
        TimelineAdmissionExecutionRequest, STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_ARG,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_DATA_DIR_ARG,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_ARG,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_DATA_DIR_ARG,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_ARG,
        STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_DATA_DIR_ARG,
    };

    struct BlockingStepExecutor {
        step_started: mpsc::Sender<()>,
        release_step: mpsc::Receiver<()>,
    }

    impl ChoreographyStepExecutor for BlockingStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.step_started.send(()).expect("notify step start");
            self.release_step
                .recv_timeout(Duration::from_secs(2))
                .expect("release blocked step");
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

    struct BlockingActionStepExecutor {
        action_started: mpsc::Sender<String>,
        release_action: mpsc::Receiver<()>,
    }

    impl ChoreographyStepExecutor for BlockingActionStepExecutor {
        fn play_action_step(
            &self,
            step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.action_started
                .send(step.action_id.clone())
                .expect("notify action start");
            self.release_action
                .recv_timeout(Duration::from_secs(2))
                .expect("release blocked action");
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

    struct PanicStepExecutor;

    impl ChoreographyStepExecutor for PanicStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            panic!("deferred command should not execute its own step")
        }

        fn move_to_step(
            &self,
            _step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            panic!("deferred command should not execute its own move")
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            panic!("deferred command should not execute its own path")
        }

        fn wait_step(&self, _step: &WaitStep) -> BuddyResult<()> {
            panic!("deferred command should not execute its own wait")
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    struct FailingStepExecutor;

    impl ChoreographyStepExecutor for FailingStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn move_to_step(
            &self,
            _step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn wait_step(&self, _step: &WaitStep) -> BuddyResult<()> {
            Err(BuddyError::Runtime(
                "native pet control socket rejected host action: control_response_timeout"
                    .to_owned(),
            ))
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    struct FailFirstThenRecordStepExecutor {
        first_step_started: mpsc::Sender<()>,
        release_first_step: mpsc::Receiver<()>,
        play_attempts: AtomicUsize,
        operations: Arc<Mutex<Vec<String>>>,
    }

    impl ChoreographyStepExecutor for FailFirstThenRecordStepExecutor {
        fn play_action_step(
            &self,
            step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            if self.play_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                self.first_step_started
                    .send(())
                    .expect("notify first step start");
                self.release_first_step
                    .recv_timeout(Duration::from_secs(2))
                    .expect("release first step");
                return Err(BuddyError::Runtime("first action failed".to_owned()));
            }

            self.operations
                .lock()
                .expect("operations lock")
                .push(format!("play:{}", step.action_id));
            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target = match &step.target {
                MoveTarget::Center => "center",
                MoveTarget::Home => "home",
                _ => "other",
            };
            self.operations
                .lock()
                .expect("operations lock")
                .push(format!("moveTo:{target}"));
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

        fn interrupt_step(&self, step_id: &str, reason_code: &str) -> BuddyResult<()> {
            self.operations
                .lock()
                .expect("operations lock")
                .push(format!("interrupt:{step_id}:{reason_code}"));
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    struct HealthGateUnavailableStepExecutor;

    impl ChoreographyStepExecutor for HealthGateUnavailableStepExecutor {
        fn play_action_step(
            &self,
            _step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            panic!("health gate failure should block plan execution")
        }

        fn move_to_step(
            &self,
            _step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            panic!("health gate failure should block plan execution")
        }

        fn move_by_path_step(
            &self,
            _step: &MoveByPathStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            panic!("health gate failure should block plan execution")
        }

        fn wait_step(&self, _step: &WaitStep) -> BuddyResult<()> {
            panic!("health gate failure should block plan execution")
        }

        fn interrupt_step(&self, _step_id: &str, _reason_code: &str) -> BuddyResult<()> {
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Err(BuddyError::Runtime(
                "native pet sidecar queryState failed".to_owned(),
            ))
        }
    }

    #[derive(Clone, Default)]
    struct RecordingStepExecutor {
        operations: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingStepExecutor {
        fn operations(&self) -> Vec<String> {
            self.operations.lock().expect("operations lock").clone()
        }
    }

    impl ChoreographyStepExecutor for RecordingStepExecutor {
        fn play_action_step(
            &self,
            step: &PlayActionStep,
            _resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.operations
                .lock()
                .expect("operations lock")
                .push(format!("play:{}", step.action_id));
            Ok(())
        }

        fn move_to_step(
            &self,
            step: &MoveToStep,
            _after_animation_ref: Option<&str>,
        ) -> BuddyResult<()> {
            let target = match &step.target {
                MoveTarget::Center => "center",
                MoveTarget::Home => "home",
                _ => "other",
            };
            self.operations
                .lock()
                .expect("operations lock")
                .push(format!("moveTo:{target}"));
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

        fn interrupt_step(&self, step_id: &str, reason_code: &str) -> BuddyResult<()> {
            self.operations
                .lock()
                .expect("operations lock")
                .push(format!("interrupt:{step_id}:{reason_code}"));
            Ok(())
        }

        fn query_state_position(&self) -> BuddyResult<Option<(i32, i32)>> {
            Ok(None)
        }
    }

    fn execute_test_timeline_with_state_scheduler(
        state: &BuddyAppState,
        storage: BuddyStorage,
        executor: &impl ChoreographyStepExecutor,
        execution: AdmittedTimelineExecution,
        step_started: mpsc::Sender<String>,
    ) -> BuddyResult<TimelineAdmissionExecutionReport> {
        let scheduled_pending = ScheduledChoreographyCapture::default();
        let executed = execute_admitted_timeline_plan(
            executor,
            execution,
            |plan_id, step, interrupt_policy| {
                if step.kind() == "wait" {
                    step_started
                        .send(step.step_id().to_owned())
                        .expect("notify wait step start");
                }
                state.refresh_choreography_plan_active_step(
                    plan_id,
                    step.step_id(),
                    interrupt_policy,
                )
            },
            |plan_id, step| scheduled_pending.capture_after_completed_step(state, plan_id, step),
            |plan_id| state.release_choreography_plan_preserving_pending(plan_id),
            |plan_id| scheduled_pending.capture_release(state, plan_id),
            |failed_plan, failed_step_id, error, resolve_context| {
                let degradation = state.with_choreography_admission(|admission| {
                    Ok(
                        super::trigger_admitted_runtime_safe_fallback_after_timeline_failure(
                            storage.clone(),
                            executor,
                            admission,
                            failed_plan,
                            failed_step_id,
                            error,
                            resolve_context,
                        ),
                    )
                })?;
                if degradation.is_none() {
                    scheduled_pending.capture_pending_after_failed_recovery(
                        state,
                        failed_plan.plan_id.as_str(),
                    )?;
                }
                super::mark_choreography_runtime_degraded_if_needed(state, degradation)
            },
        )
        .map_err(timeline_execution_error_to_buddy_error);

        flush_scheduled_pending_choreography_plans(
            state,
            executor,
            scheduled_pending.into_scheduled(),
        )?;

        executed.map(|executed| executed.report)
    }

    fn seed_startup_recoverable_timeline_execution(
        data_dir: std::path::PathBuf,
        plan_id: &str,
    ) -> BuddyAppState {
        seed_startup_recoverable_timeline_execution_data(data_dir.clone(), plan_id);
        let paths = BuddyAppPaths::from_data_dir(data_dir);

        BuddyAppState::initialize_with_paths(paths).expect("initialize state after restart")
    }

    fn seed_startup_recoverable_timeline_execution_data(
        data_dir: std::path::PathBuf,
        plan_id: &str,
    ) {
        seed_startup_recoverable_timeline_execution_data_with_deferred_at(
            data_dir,
            plan_id,
            LocalLogTimestamp::now_utc().to_rfc3339_millis(),
        )
    }

    fn seed_startup_recoverable_timeline_execution_data_with_deferred_at(
        data_dir: std::path::PathBuf,
        plan_id: &str,
        deferred_at: String,
    ) {
        let paths = BuddyAppPaths::from_data_dir(data_dir);
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        let source_ref = json!({
            "kind": "devFixture",
            "fixtureName": "startup-recoverable-command-replay"
        });
        let plan = TimelinePlan {
            plan_id: plan_id.to_owned(),
            source_ref: source_ref.clone(),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![TimelineStep::PlayAction(PlayActionStep::once(
                "step_startup_recoverable_command_replay",
                "celebrate",
                5_000,
            ))],
            created_at: "2026-07-13T00:00:00.000Z".to_owned(),
        };
        let request = TimelineAdmissionExecutionRequest::new(
            storage.clone(),
            plan,
            TimelineExecutionContext::fixed_for_test(),
            ResolveContext::default(),
            ChoreographyTriggerSource::UserRequested,
        );

        storage.initialize().expect("initialize storage");
        storage
            .upsert_choreography_pending_execution_body(
                UpsertChoreographyPendingExecutionBodyRequest {
                    plan_id: plan_id.to_owned(),
                    body_kind: ChoreographyPendingExecutionBodyKind::Timeline,
                    schema_version: 1,
                    body: serde_json::to_value(request.pending_body())
                        .expect("serialize pending body"),
                },
            )
            .expect("store pending execution body fact");
        storage
            .append_choreography_action_log_event(
                &ActionLogEvent::executor_admission_decision_for_source(
                    format!("evt_startup_recoverable_command_deferred_{plan_id}"),
                    plan_id,
                    &source_ref,
                    ChoreographyTriggerSource::UserRequested.action_log_value(),
                    &ChoreographyAdmissionDecision::Deferred {
                        plan_id: plan_id.to_owned(),
                        trigger_source: ChoreographyTriggerSource::UserRequested,
                        priority: ChoreographyPlanPriority::UserRequested,
                        active_plan_id: "plan_startup_recoverable_command_active".to_owned(),
                        active_step_id: Some("step_startup_recoverable_command_active".to_owned()),
                        active_priority: ChoreographyPlanPriority::AiChoreography,
                        active_step_interrupt_policy: SidecarInterruptPolicy::FinishStep,
                        reason_code: "admission.waitingForActiveStepToFinish".to_owned(),
                    },
                    deferred_at,
                ),
            )
            .expect("append deferred admission event");
        storage
            .clear_choreography_pending_execution_bodies()
            .expect("drop sqlite cache before restart");
    }

    #[test]
    fn macro_intent_request_deserializes_structured_macro_params() {
        let request = serde_json::from_value::<RunChoreographyMacroIntentRequest>(json!({
            "intent": {
                "macroId": "dance",
                "params": {
                    "durationMs": 2500
                }
            }
        }))
        .expect("deserialize structured macro intent request");

        assert!(matches!(
            request.intent,
            MacroIntent::Dance(params) if params.duration_ms == 2500
        ));
    }

    #[test]
    fn macro_intent_request_accepts_optional_source_ref() {
        let request = serde_json::from_value::<RunChoreographyMacroIntentRequest>(json!({
            "intent": {
                "macroId": "dance",
                "params": {
                    "durationMs": 2500
                }
            },
            "sourceRef": {
                "kind": "run",
                "runId": "run_019f"
            }
        }))
        .expect("deserialize macro intent request sourceRef");

        assert_eq!(
            request.source_ref.as_ref().and_then(|source_ref| {
                source_ref.get("runId").and_then(serde_json::Value::as_str)
            }),
            Some("run_019f")
        );
    }

    #[test]
    fn macro_intent_request_accepts_critical_interaction_trigger_source() {
        let request = serde_json::from_value::<RunChoreographyMacroIntentRequest>(json!({
            "intent": {
                "macroId": "celebrate",
                "params": {}
            },
            "sourceRef": {
                "kind": "presetBehavior",
                "presetBehaviorId": "throw_after_drag",
                "interactionId": "interaction_019f"
            },
            "triggerSource": "criticalInteraction"
        }))
        .expect("deserialize critical interaction macro intent request");

        assert_eq!(
            request.trigger_source,
            Some(RunChoreographyMacroIntentTriggerSource::CriticalInteraction)
        );
    }

    #[test]
    fn macro_intent_request_rejects_unknown_top_level_fields() {
        let result = serde_json::from_value::<RunChoreographyMacroIntentRequest>(json!({
            "intent": {
                "macroId": "dance",
                "params": {
                    "durationMs": 2500
                }
            },
            "previewTimeline": true
        }));
        let Err(error) = result else {
            panic!("macro intent command request should reject fields outside the schema");
        };

        assert!(
            error.to_string().contains("unknown field"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn startup_recoverable_command_replays_idle_entry_and_consumes_it() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-command-idle-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_command_idle";
        let state = seed_startup_recoverable_timeline_execution(data_dir.clone(), plan_id);
        let executor = RecordingStepExecutor::default();

        let result =
            replay_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
                &state,
                ReplayStartupRecoverableChoreographyPendingExecutionRequest {
                    plan_id: plan_id.to_owned(),
                },
                &executor,
                false,
            )
            .expect("replay startup recoverable execution");

        assert_eq!(result.plan_id, plan_id);
        assert_eq!(result.replay_status, "executed");
        assert_eq!(result.admission_decision, Some("accepted"));
        assert!(result.executed);
        assert_eq!(executor.operations(), vec!["play:celebrate".to_owned()]);
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_count()
                .expect("read startup recoverable count"),
            0
        );
        assert_eq!(
            state
                .storage_handle()
                .action_log_plan_summary_for_test(plan_id)["status"],
            json!("completed")
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_command_waits_without_consuming_entry_when_busy() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-command-busy-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_command_busy";
        let state = seed_startup_recoverable_timeline_execution(data_dir.clone(), plan_id);
        let executor = RecordingStepExecutor::default();
        state
            .admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_startup_recoverable_command_current_active",
                ChoreographyTriggerSource::AiChoreography,
            ))
            .expect("admit active plan");

        let result =
            replay_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
                &state,
                ReplayStartupRecoverableChoreographyPendingExecutionRequest {
                    plan_id: plan_id.to_owned(),
                },
                &executor,
                false,
            )
            .expect("try replay startup recoverable execution while busy");

        assert_eq!(result.plan_id, plan_id);
        assert_eq!(result.replay_status, "waitingForIdle");
        assert_eq!(result.admission_decision, None);
        assert!(!result.executed);
        assert!(executor.operations().is_empty());
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_command_rejects_policy_rejected_entry_without_consuming_it() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-command-policy-rejected-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_command_policy_rejected";
        seed_startup_recoverable_timeline_execution_data_with_deferred_at(
            data_dir.clone(),
            plan_id,
            "1970-01-01T00:00:00.000Z".to_owned(),
        );
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state after restart");
        let executor = RecordingStepExecutor::default();

        let result =
            replay_startup_recoverable_choreography_pending_execution_from_state_with_executor_and_local_interaction_status(
                &state,
                ReplayStartupRecoverableChoreographyPendingExecutionRequest {
                    plan_id: plan_id.to_owned(),
                },
                &executor,
                false,
            )
            .expect("try replay policy-rejected startup recoverable execution");

        assert_eq!(result.plan_id, plan_id);
        assert_eq!(result.replay_status, "rejectedByPolicy");
        assert_eq!(result.admission_decision, None);
        assert!(!result.executed);
        assert!(executor.operations().is_empty());
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read startup recoverable plan ids"),
            vec![plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_replay_cli_parses_plan_id_and_data_dir() {
        let data_dir = std::env::temp_dir().join("lexora-buddy-startup-replay-cli");

        let command = parse_startup_recoverable_choreography_replay_command([
            "lexora-buddy",
            STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_ARG,
            "plan_startup_recoverable_cli",
            STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_DATA_DIR_ARG,
            data_dir.to_str().expect("temp path is utf8"),
        ])
        .expect("startup recoverable replay arg should be detected")
        .expect("startup recoverable replay cli args should parse");

        assert_eq!(command.plan_id, "plan_startup_recoverable_cli");
        assert_eq!(command.data_dir, Some(data_dir));
    }

    #[test]
    fn startup_recoverable_replay_cli_runs_replay_from_data_dir() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-cli-run-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_cli_run";
        seed_startup_recoverable_timeline_execution_data(data_dir.clone(), plan_id);
        let executor = RecordingStepExecutor::default();

        let output = run_startup_recoverable_choreography_replay_command_with_executor_and_local_interaction_status(
            StartupRecoverableChoreographyReplayCommand {
                plan_id: plan_id.to_owned(),
                data_dir: Some(data_dir.clone()),
            },
            &executor,
            false,
        )
        .expect("run startup recoverable replay cli command");
        let output =
            serde_json::from_str::<serde_json::Value>(&output).expect("replay cli output is json");

        assert_eq!(output["planId"], json!(plan_id));
        assert_eq!(output["replayStatus"], json!("executed"));
        assert_eq!(output["executed"], json!(true));
        assert_eq!(output["admissionDecision"], json!("accepted"));
        assert_eq!(executor.operations(), vec!["play:celebrate".to_owned()]);

        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        storage.initialize().expect("initialize storage");
        assert_eq!(
            storage.action_log_plan_summary_for_test(plan_id)["status"],
            json!("completed")
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_replay_next_cli_parses_data_dir() {
        let data_dir = std::env::temp_dir().join("lexora-buddy-startup-replay-next-cli");

        let command = parse_startup_recoverable_choreography_replay_next_command([
            "lexora-buddy",
            STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_ARG,
            STARTUP_RECOVERABLE_CHOREOGRAPHY_REPLAY_NEXT_DATA_DIR_ARG,
            data_dir.to_str().expect("temp path is utf8"),
        ])
        .expect("startup recoverable replay-next arg should be detected")
        .expect("startup recoverable replay-next cli args should parse");

        assert_eq!(command.data_dir, Some(data_dir));
    }

    #[test]
    fn startup_recoverable_replay_next_cli_runs_next_replay_from_data_dir() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-cli-next-run-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_cli_next_run";
        seed_startup_recoverable_timeline_execution_data(data_dir.clone(), plan_id);
        let executor = RecordingStepExecutor::default();

        let output = run_startup_recoverable_choreography_replay_next_command_with_executor_and_local_interaction_status(
            StartupRecoverableChoreographyReplayNextCommand {
                data_dir: Some(data_dir.clone()),
            },
            &executor,
            false,
        )
        .expect("run startup recoverable replay-next cli command");
        let output = serde_json::from_str::<serde_json::Value>(&output)
            .expect("replay-next cli output is json");

        assert_eq!(output["planId"], json!(plan_id));
        assert_eq!(output["replayStatus"], json!("executed"));
        assert_eq!(output["executed"], json!(true));
        assert_eq!(output["admissionDecision"], json!("accepted"));
        assert_eq!(executor.operations(), vec!["play:celebrate".to_owned()]);

        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        storage.initialize().expect("initialize storage");
        assert_eq!(
            storage.action_log_plan_summary_for_test(plan_id)["status"],
            json!("completed")
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_replay_next_cli_preserves_skipped_rejected_entry_across_restart() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-cli-next-skips-rejected-{}",
            uuid::Uuid::new_v4()
        ));
        let rejected_plan_id = "plan_startup_recoverable_cli_next_rejected_first";
        let eligible_plan_id = "plan_startup_recoverable_cli_next_eligible_second";
        seed_startup_recoverable_timeline_execution_data_with_deferred_at(
            data_dir.clone(),
            rejected_plan_id,
            "1970-01-01T00:00:00.000Z".to_owned(),
        );
        seed_startup_recoverable_timeline_execution_data(data_dir.clone(), eligible_plan_id);
        let executor = RecordingStepExecutor::default();

        let output = run_startup_recoverable_choreography_replay_next_command_with_executor_and_local_interaction_status(
            StartupRecoverableChoreographyReplayNextCommand {
                data_dir: Some(data_dir.clone()),
            },
            &executor,
            false,
        )
        .expect("run startup recoverable replay-next cli command");
        let output = serde_json::from_str::<serde_json::Value>(&output)
            .expect("replay-next cli output is json");

        assert_eq!(output["planId"], json!(eligible_plan_id));
        assert_eq!(output["replayStatus"], json!("executed"));
        assert_eq!(executor.operations(), vec!["play:celebrate".to_owned()]);

        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state after replay-next");
        assert_eq!(
            state
                .startup_recoverable_choreography_pending_plan_ids()
                .expect("read remaining startup recoverable plan ids"),
            vec![rejected_plan_id.to_owned()]
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_list_cli_parses_data_dir() {
        let data_dir = std::env::temp_dir().join("lexora-buddy-startup-list-cli");

        let command = parse_startup_recoverable_choreography_list_command([
            "lexora-buddy",
            STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_ARG,
            STARTUP_RECOVERABLE_CHOREOGRAPHY_LIST_DATA_DIR_ARG,
            data_dir.to_str().expect("temp path is utf8"),
        ])
        .expect("startup recoverable list arg should be detected")
        .expect("startup recoverable list cli args should parse");

        assert_eq!(command.data_dir, Some(data_dir));
    }

    #[test]
    fn startup_recoverable_list_cli_outputs_replayable_plan_summaries() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-list-run-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_list_cli";
        seed_startup_recoverable_timeline_execution_data(data_dir.clone(), plan_id);

        let output =
            run_startup_recoverable_choreography_list_command_with_local_interaction_status(
                StartupRecoverableChoreographyListCommand {
                    data_dir: Some(data_dir.clone()),
                },
                false,
            )
            .expect("list startup recoverable entries");
        let output = serde_json::from_str::<serde_json::Value>(&output)
            .expect("startup recoverable list output is json");

        assert_eq!(output["items"][0]["planId"], json!(plan_id));
        assert_eq!(output["items"][0]["sourceRefKind"], json!("devFixture"));
        assert_eq!(
            output["items"][0]["sourceRefId"],
            json!("startup-recoverable-command-replay")
        );
        assert_eq!(output["items"][0]["triggerSource"], json!("userRequested"));
        assert_eq!(output["items"][0]["priority"], json!("userRequested"));
        assert_eq!(
            output["items"][0]["reasonCode"],
            json!("admission.waitingForActiveStepToFinish")
        );
        assert_eq!(output["items"][0]["bodyKind"], json!("timeline"));
        assert_eq!(output["items"][0]["bodySchemaVersion"], json!(1));
        assert_eq!(
            output["items"][0]["replayPolicy"]["decision"],
            json!("manual")
        );
        assert_eq!(
            output["items"][0]["replayPolicy"]["reasonCode"],
            json!("replay.manualInternalOnly")
        );
        assert_eq!(output["items"][0]["body"], serde_json::Value::Null);
        assert_eq!(output["total"], json!(1));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_diagnose_command_returns_replay_policy_summaries() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-diagnose-run-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_diagnose";
        let state = seed_startup_recoverable_timeline_execution(data_dir.clone(), plan_id);

        let output = diagnose_startup_recoverable_choreography_replay_candidates_from_state_with_local_interaction_status(
            &state,
            false,
        )
        .expect("diagnose startup recoverable entries");
        let output = serde_json::to_value(output).expect("diagnose result is serializable");

        assert_eq!(output["items"][0]["planId"], json!(plan_id));
        assert_eq!(
            output["items"][0]["replayPolicy"]["decision"],
            json!("manual")
        );
        assert_eq!(
            output["items"][0]["replayPolicy"]["reasonCode"],
            json!("replay.manualInternalOnly")
        );
        assert_eq!(output["total"], json!(1));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn startup_recoverable_list_cli_does_not_mutate_source_startup_state() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-startup-recoverable-list-readonly-{}",
            uuid::Uuid::new_v4()
        ));
        let plan_id = "plan_startup_recoverable_list_readonly";
        seed_startup_recoverable_timeline_execution_data(data_dir.clone(), plan_id);
        let action_log_path = data_dir.join("action-log").join("events.jsonl");
        let before_events =
            std::fs::read_to_string(&action_log_path).expect("read action log before list");
        let paths = BuddyAppPaths::from_data_dir(data_dir.clone());
        let storage =
            BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
        storage
            .rebuild_choreography_pending_execution_body_cache_from_action_log()
            .expect("rebuild source pending body cache before list");
        assert!(
            storage
                .find_choreography_pending_execution_body(plan_id)
                .expect("read source pending body before list")
                .is_some(),
            "test setup must start with source pending body cache"
        );

        run_startup_recoverable_choreography_list_command_with_local_interaction_status(
            StartupRecoverableChoreographyListCommand {
                data_dir: Some(data_dir.clone()),
            },
            false,
        )
        .expect("list startup recoverable entries");

        let after_events =
            std::fs::read_to_string(&action_log_path).expect("read action log after list");

        assert_eq!(after_events, before_events);
        assert!(
            !after_events.contains("stalePendingBodiesCleared"),
            "list command must not append startup cleanup diagnostics to the source log"
        );
        assert!(
            storage
                .find_choreography_pending_execution_body(plan_id)
                .expect("read source pending body after list")
                .is_some(),
            "list command must not clear the source pending body cache"
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_command_does_not_hold_admission_lock_while_step_is_running() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-scheduler-lock-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let command_state = state.clone();
        let (step_started_tx, step_started_rx) = mpsc::channel();
        let (release_step_tx, release_step_rx) = mpsc::channel();
        let executor = BlockingStepExecutor {
            step_started: step_started_tx,
            release_step: release_step_rx,
        };
        let request = RunChoreographyMacroIntentRequest {
            intent: serde_json::from_value(json!({
                "macroId": "dance",
                "params": { "durationMs": 1000 }
            }))
            .expect("macro intent"),
            source_ref: None,
            trigger_source: None,
        };
        let command_thread = thread::spawn(move || {
            run_choreography_macro_intent_from_state_with_executor(
                &command_state,
                request,
                &executor,
            )
        });
        step_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("macro intent step should start");
        let probe_state = state.clone();
        let (probe_tx, probe_rx) = mpsc::channel();
        let probe_thread = thread::spawn(move || {
            let decision = probe_state.admit_choreography_plan(ChoreographyAdmissionRequest::new(
                "plan_probe_019f5b00-0000-7000-8000-000000000901",
                ChoreographyTriggerSource::AiChoreography,
            ));
            probe_tx.send(decision).expect("send probe decision");
        });

        let probe_decision = probe_rx.recv_timeout(Duration::from_millis(200));
        release_step_tx.send(()).expect("release command step");
        let command_result = command_thread.join().expect("join command thread");
        probe_thread.join().expect("join probe thread");

        let probe_decision = probe_decision
            .expect("admission probe should not block behind the running command")
            .expect("probe admission decision");
        assert!(matches!(
            probe_decision,
            ChoreographyAdmissionDecision::Skipped { .. }
                | ChoreographyAdmissionDecision::Rejected { .. }
        ));
        assert!(command_result.expect("command result").executed);

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_command_marks_runtime_readiness_degraded_when_recovery_fails() {
        let cases = [
            (
                "system-recovery",
                json!({ "macroId": "lieDown", "params": {} }),
            ),
            (
                "semantic-fallback",
                json!({ "macroId": "cast", "params": {} }),
            ),
        ];

        for (label, intent) in cases {
            let data_dir = std::env::temp_dir().join(format!(
                "lexora-buddy-command-runtime-degraded-{label}-{}",
                uuid::Uuid::new_v4()
            ));
            let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(
                data_dir.clone(),
            ))
            .expect("initialize state");

            let result = run_choreography_macro_intent_from_state_with_executor(
                &state,
                RunChoreographyMacroIntentRequest {
                    intent: serde_json::from_value(intent).expect("valid macro intent fixture"),
                    source_ref: Some(json!({
                        "kind": "conversationMessage",
                        "conversationId": "conversation_019f5b00-0000-7000-8000-000000000931",
                        "messageId": "message_019f5b00-0000-7000-8000-000000000932"
                    })),
                    trigger_source: None,
                },
                &FailingStepExecutor,
            );
            let readiness = state
                .choreography_runtime_readiness_snapshot()
                .expect("read runtime readiness");

            assert!(
                result.is_err(),
                "failed {label} macro should return the original execution error"
            );
            assert_eq!(readiness.status.as_str(), "degraded", "case: {label}");
            assert!(
                !readiness.accepting_choreography,
                "runtime should reject choreography after {label} failure"
            );
            assert_eq!(
                readiness.reason_code.as_deref(),
                Some("runtime.systemRecoveryFailed"),
                "case: {label}"
            );

            let _ = std::fs::remove_dir_all(data_dir);
        }
    }

    #[test]
    fn macro_intent_command_normalizes_run_source_ref_and_triggers_recovery_after_host_action_failure(
    ) {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-host-action-failure-recovery-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let storage = state.storage_handle();
        let conversation = storage
            .create_conversation(CreateBuddyConversationRequest {
                forked_from_message_id: None,
                project_root: None,
                scope: "global".to_owned(),
                source_conversation_id: None,
                source_run_id: None,
                title: Some("Host action failure source".to_owned()),
            })
            .expect("create conversation");
        let triggering_message = storage
            .append_conversation_message(AppendBuddyConversationMessageRequest {
                attachments: Vec::new(),
                branch_id: conversation.active_branch_id.clone(),
                content: "用户消息正文不应该进入动作日志 sourceRef".to_owned(),
                conversation_id: conversation.id.clone(),
                parent_message_id: None,
                role: "user".to_owned(),
                run_id: None,
                version_group_id: None,
                version_index: 1,
                version_status: "active".to_owned(),
            })
            .expect("append triggering message");
        let run = storage
            .create_conversation_run(CreateBuddyConversationRunRequest {
                branch_id: conversation.active_branch_id,
                conversation_id: conversation.id.clone(),
                cwd: Some("/tmp/lexora-project".to_owned()),
                external_run_id: None,
                external_thread_id: None,
                intent: "hostActionMacroIntent".to_owned(),
                runtime: "codex".to_owned(),
                triggering_message_id: triggering_message.id.clone(),
            })
            .expect("create conversation run");

        let result = run_choreography_macro_intent_from_state_with_executor(
            &state,
            RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "lieDown",
                    "params": {}
                }))
                .expect("lie down macro intent"),
                source_ref: Some(json!({
                    "kind": "run",
                    "runId": run.id,
                })),
                trigger_source: None,
            },
            &FailingStepExecutor,
        );
        assert!(
            result.is_err(),
            "failed host-action macro intent should return execution error"
        );

        let events = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .collect::<Vec<_>>();
        let original_plan_started = events
            .iter()
            .find(|event| {
                event["eventType"] == "plan.started"
                    && event["sourceRef"]["kind"] == "conversationMessage"
                    && event["sourceRef"]["conversationId"] == conversation.id
                    && event["sourceRef"]["messageId"] == triggering_message.id
                    && event["sourceRef"]["runId"] == run.id
            })
            .expect(
                "original host-action plan should use normalized conversationMessage sourceRef",
            );
        let original_plan_id = original_plan_started["planId"]
            .as_str()
            .expect("original plan id");
        let recovery_started = events
            .iter()
            .find(|event| {
                event["eventType"] == "plan.started"
                    && event["sourceRef"]["kind"] == "systemRecovery"
                    && event["sourceRef"]["triggeredByPlanId"] == original_plan_id
            })
            .expect("failed host-action plan should trigger system recovery");
        let recovery_plan_id = recovery_started["planId"]
            .as_str()
            .expect("recovery plan id");

        assert_eq!(
            storage.action_log_event_types_for_test(original_plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.failed",
                "step.resolved",
                "step.failed",
                "plan.failed",
            ]
        );
        let original_plan_events = events
            .iter()
            .filter(|event| event["planId"] == original_plan_id)
            .collect::<Vec<_>>();
        assert_eq!(original_plan_events[4]["payload"]["actionId"], "sleep");
        assert!(storage
            .action_log_event_types_for_test(recovery_plan_id)
            .contains(&"plan.failed".to_owned()));
        assert!(!events
            .iter()
            .any(|event| event.to_string().contains("用户消息正文不应该进入动作日志")));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_attention_trigger_recovers_degraded_runtime_after_sidecar_health_gate() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-health-gate-recovery-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        state
            .mark_choreography_runtime_degraded(
                "runtime.systemRecoveryFailed",
                "2026-07-14T01:00:00.000Z",
            )
            .expect("mark runtime degraded");
        let storage = state.storage_handle();
        let executor = RecordingStepExecutor::default();
        let intent: MacroIntent = serde_json::from_value(json!({
            "macroId": "celebrate",
            "params": {}
        }))
        .expect("macro intent");
        let source = MacroIntentRunSource {
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "attention-health-gate",
            }),
            trigger_source: ChoreographyTriggerSource::AttentionSystem,
        };

        let result = run_choreography_macro_intent_from_state_with_executor_and_context(
            &state,
            intent,
            storage,
            ResolveContext::default(),
            source,
            &executor,
            "celebrate".to_owned(),
        )
        .expect("attention health gate should recover and execute");
        let readiness = state
            .choreography_runtime_readiness_snapshot()
            .expect("read readiness");

        assert!(result.executed);
        assert_eq!(readiness.status.as_str(), "ready");
        assert!(readiness.accepting_choreography);
        assert_eq!(readiness.reason_code, None);
        assert_eq!(executor.operations(), vec!["play:celebrate"]);
        let health_events = state
            .storage_handle()
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("healthGate.passed".to_owned()),
                source_ref_kind: Some("healthGate".to_owned()),
                reason_code: Some("sidecar.available".to_owned()),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query health gate system events");

        assert_eq!(health_events.items.len(), 1);
        assert_eq!(health_events.items[0].event_type, "healthGate.passed");
        assert_eq!(health_events.items[0].status, "passed");
        assert_eq!(health_events.items[0].reason_code, "sidecar.available");
        assert_eq!(health_events.items[0].trigger_source, "healthGate");
        assert_eq!(health_events.items[0].source_ref.kind, "healthGate");

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_command_critical_interaction_trigger_recovers_degraded_runtime() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-critical-interaction-health-gate-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        state
            .mark_choreography_runtime_degraded(
                "runtime.systemRecoveryFailed",
                "2026-07-14T02:00:00.000Z",
            )
            .expect("mark runtime degraded");
        let executor = RecordingStepExecutor::default();

        let result = run_choreography_macro_intent_from_state_with_executor(
            &state,
            RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "celebrate",
                    "params": {}
                }))
                .expect("macro intent"),
                source_ref: Some(json!({
                    "kind": "presetBehavior",
                    "presetBehaviorId": "throw_after_drag",
                    "interactionId": "interaction_019f5b00_critical",
                })),
                trigger_source: Some(RunChoreographyMacroIntentTriggerSource::CriticalInteraction),
            },
            &executor,
        )
        .expect("critical interaction trigger should recover and execute");
        let readiness = state
            .choreography_runtime_readiness_snapshot()
            .expect("read readiness");
        let storage = state.storage_handle();
        let health_events = storage
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("healthGate.passed".to_owned()),
                source_ref_kind: Some("healthGate".to_owned()),
                reason_code: Some("sidecar.available".to_owned()),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query health gate system events");
        let plan_started = storage
            .read_action_log_jsonl_lines_for_test()
            .into_iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(&line).expect("parse event"))
            .find(|event| event["eventType"] == "plan.started" && event["planId"] == result.plan_id)
            .expect("critical interaction plan started event");

        assert!(result.executed);
        assert_eq!(result.admission_decision, "accepted");
        assert_eq!(readiness.status.as_str(), "ready");
        assert_eq!(executor.operations(), vec!["play:celebrate"]);
        assert_eq!(health_events.items.len(), 1);
        assert_eq!(plan_started["triggerSource"], json!("criticalInteraction"));
        assert_eq!(plan_started["sourceRef"]["kind"], json!("presetBehavior"));
        assert_eq!(
            plan_started["sourceRef"]["presetBehaviorId"],
            json!("throw_after_drag")
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_command_accepts_approval_source_ref() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-approval-source-ref-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let executor = RecordingStepExecutor::default();

        let result = run_choreography_macro_intent_from_state_with_executor(
            &state,
            RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "awaitApproval",
                    "params": {}
                }))
                .expect("macro intent"),
                source_ref: Some(json!({
                    "kind": "approval",
                    "approvalId": "approval_019f",
                    "runId": "run_019f",
                })),
                trigger_source: None,
            },
            &executor,
        )
        .expect("approval sourceRef should execute");
        let storage = state.storage_handle();
        let plan = storage
            .list_action_log_plans(crate::storage::ActionLogPlanListRequest {
                source_ref_kind: Some("approval".to_owned()),
                ..crate::storage::ActionLogPlanListRequest::default()
            })
            .expect("list approval action log plan")
            .items
            .into_iter()
            .next()
            .expect("approval action log plan");

        assert!(result.executed);
        assert_eq!(executor.operations(), vec!["play:approval"]);
        assert_eq!(plan.plan_id, result.plan_id);
        assert_eq!(plan.source_ref_kind, "approval");
        assert_eq!(plan.source_ref_id.as_deref(), Some("approval_019f"));
        assert_eq!(plan.source_ref["approvalId"], json!("approval_019f"));
        assert_eq!(plan.source_ref["runId"], json!("run_019f"));
        assert_eq!(plan.resolved_action_id.as_deref(), Some("approval"));
        assert_eq!(plan.resolved_animation_ref.as_deref(), Some("approval"));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_command_system_trigger_source_requires_source_ref() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-system-trigger-source-ref-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");

        let error = match run_choreography_macro_intent_from_state_with_executor(
            &state,
            RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "celebrate",
                    "params": {}
                }))
                .expect("macro intent"),
                source_ref: None,
                trigger_source: Some(RunChoreographyMacroIntentTriggerSource::CriticalInteraction),
            },
            &RecordingStepExecutor::default(),
        ) {
            Ok(_) => panic!("system trigger source should require stable sourceRef"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("macro intent triggerSource requires sourceRef"));

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_ai_trigger_does_not_recover_degraded_runtime_with_sidecar_health_gate() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-health-gate-ai-blocked-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        state
            .mark_choreography_runtime_degraded(
                "runtime.systemRecoveryFailed",
                "2026-07-14T01:00:00.000Z",
            )
            .expect("mark runtime degraded");
        let storage = state.storage_handle();
        let executor = RecordingStepExecutor::default();
        let intent: MacroIntent = serde_json::from_value(json!({
            "macroId": "celebrate",
            "params": {}
        }))
        .expect("macro intent");
        let source = MacroIntentRunSource {
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "ai-health-gate-blocked",
            }),
            trigger_source: ChoreographyTriggerSource::AiChoreography,
        };

        let error = match run_choreography_macro_intent_from_state_with_executor_and_context(
            &state,
            intent,
            storage,
            ResolveContext::default(),
            source,
            &executor,
            "celebrate".to_owned(),
        ) {
            Ok(_) => panic!("AI choreography should stay blocked while runtime is degraded"),
            Err(error) => error,
        };
        let readiness = state
            .choreography_runtime_readiness_snapshot()
            .expect("read readiness");

        assert!(error
            .to_string()
            .contains("choreography runtime is degraded"));
        assert_eq!(readiness.status.as_str(), "degraded");
        assert!(!readiness.accepting_choreography);
        assert_eq!(
            readiness.reason_code.as_deref(),
            Some("runtime.systemRecoveryFailed")
        );
        assert!(executor.operations().is_empty());

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_attention_trigger_records_health_gate_failed_when_sidecar_query_fails() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-health-gate-failed-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        state
            .mark_choreography_runtime_degraded(
                "runtime.systemRecoveryFailed",
                "2026-07-14T01:00:00.000Z",
            )
            .expect("mark runtime degraded");
        let storage = state.storage_handle();
        let executor = HealthGateUnavailableStepExecutor;
        let intent: MacroIntent = serde_json::from_value(json!({
            "macroId": "celebrate",
            "params": {}
        }))
        .expect("macro intent");
        let source = MacroIntentRunSource {
            source_ref: json!({
                "kind": "devFixture",
                "fixtureName": "attention-health-gate-failed",
            }),
            trigger_source: ChoreographyTriggerSource::AttentionSystem,
        };

        let error = match run_choreography_macro_intent_from_state_with_executor_and_context(
            &state,
            intent,
            storage,
            ResolveContext::default(),
            source,
            &executor,
            "celebrate".to_owned(),
        ) {
            Ok(_) => panic!("health gate failure should block plan execution"),
            Err(error) => error,
        };
        let readiness = state
            .choreography_runtime_readiness_snapshot()
            .expect("read readiness");
        let health_events = state
            .storage_handle()
            .query_action_log_system_events(ActionLogSystemEventQueryRequest {
                event_type: Some("healthGate.failed".to_owned()),
                source_ref_kind: Some("healthGate".to_owned()),
                reason_code: Some("sidecar.unavailable".to_owned()),
                ..ActionLogSystemEventQueryRequest::default()
            })
            .expect("query health gate system events");

        assert!(error.to_string().contains("queryState failed"));
        assert_eq!(readiness.status.as_str(), "degraded");
        assert!(!readiness.accepting_choreography);
        assert_eq!(health_events.items.len(), 1);
        assert_eq!(health_events.items[0].event_type, "healthGate.failed");
        assert_eq!(health_events.items[0].status, "failed");
        assert_eq!(health_events.items[0].reason_code, "sidecar.unavailable");
        assert_eq!(health_events.items[0].trigger_source, "healthGate");
        assert_eq!(health_events.items[0].source_ref.kind, "healthGate");

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_command_flushes_deferred_pending_plan_after_active_release() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-scheduler-flush-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let command_state = state.clone();
        let (step_started_tx, step_started_rx) = mpsc::channel();
        let (release_step_tx, release_step_rx) = mpsc::channel();
        let executor = BlockingStepExecutor {
            step_started: step_started_tx,
            release_step: release_step_rx,
        };
        let active_request = RunChoreographyMacroIntentRequest {
            intent: serde_json::from_value(json!({
                "macroId": "dance",
                "params": { "durationMs": 1000 }
            }))
            .expect("active macro intent"),
            source_ref: Some(json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5b00-0000-7000-8000-000000000911",
                "messageId": "message_019f5b00-0000-7000-8000-000000000912"
            })),
            trigger_source: None,
        };
        let command_thread = thread::spawn(move || {
            run_choreography_macro_intent_from_state_with_executor(
                &command_state,
                active_request,
                &executor,
            )
        });
        step_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("active macro intent step should start");

        let pending_state = state.clone();
        let (pending_tx, pending_rx) = mpsc::channel();
        let pending_thread = thread::spawn(move || {
            let pending_request = RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "dance",
                    "params": { "durationMs": 1000 }
                }))
                .expect("pending macro intent"),
                source_ref: None,
                trigger_source: None,
            };
            let result = run_choreography_macro_intent_from_state_with_executor(
                &pending_state,
                pending_request,
                &PanicStepExecutor,
            );
            pending_tx.send(result).expect("send pending result");
        });

        let pending_result = pending_rx
            .recv_timeout(Duration::from_millis(200))
            .expect("pending command should return deferred before active step completes")
            .expect("pending command result");
        assert_eq!(pending_result.admission_decision, "deferred");
        assert!(!pending_result.executed);

        release_step_tx.send(()).expect("release active step");
        step_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("promoted pending plan should execute after active release");
        release_step_tx
            .send(())
            .expect("release promoted pending step");

        let command_result = command_thread.join().expect("join command thread");
        pending_thread.join().expect("join pending thread");
        assert!(command_result.expect("command result").executed);

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn macro_intent_command_flushes_critical_pending_plan_after_active_failure() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-scheduler-failure-flush-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let command_state = state.clone();
        let (first_step_started_tx, first_step_started_rx) = mpsc::channel();
        let (release_first_step_tx, release_first_step_rx) = mpsc::channel();
        let operations = Arc::new(Mutex::new(Vec::new()));
        let executor = FailFirstThenRecordStepExecutor {
            first_step_started: first_step_started_tx,
            release_first_step: release_first_step_rx,
            play_attempts: AtomicUsize::new(0),
            operations: operations.clone(),
        };
        let active_request = RunChoreographyMacroIntentRequest {
            intent: serde_json::from_value(json!({
                "macroId": "dance",
                "params": { "durationMs": 1000 }
            }))
            .expect("active macro intent"),
            source_ref: Some(json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5b00-0000-7000-8000-000000000921",
                "messageId": "message_019f5b00-0000-7000-8000-000000000922"
            })),
            trigger_source: None,
        };
        let command_thread = thread::spawn(move || {
            run_choreography_macro_intent_from_state_with_executor(
                &command_state,
                active_request,
                &executor,
            )
        });
        first_step_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("active macro intent step should start");

        let pending_result = run_choreography_macro_intent_from_state_with_executor(
            &state,
            RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "celebrate",
                    "params": {}
                }))
                .expect("pending critical macro intent"),
                source_ref: Some(json!({
                    "kind": "presetBehavior",
                    "presetBehaviorId": "throw_after_drag",
                    "interactionId": "interaction_019f5b00_failure_flush"
                })),
                trigger_source: Some(RunChoreographyMacroIntentTriggerSource::CriticalInteraction),
            },
            &PanicStepExecutor,
        )
        .expect("queue pending critical macro intent");
        assert_eq!(pending_result.admission_decision, "deferred");
        assert!(!pending_result.executed);

        release_first_step_tx
            .send(())
            .expect("release failing active step");
        let active_error = match command_thread.join().expect("join command thread") {
            Ok(_) => panic!("active macro intent should return its execution failure"),
            Err(error) => error,
        };
        let (active_plan_id, pending_plan_id) = state
            .with_choreography_admission(|admission| {
                Ok((
                    admission.active_plan_id().map(str::to_owned),
                    admission.next_pending_plan_id().map(str::to_owned),
                ))
            })
            .expect("read admission state");
        let recorded_operations = operations.lock().expect("operations lock").clone();

        assert!(active_error.to_string().contains("first action failed"));
        assert!(
            recorded_operations
                .iter()
                .any(|operation| operation == "play:celebrate"),
            "critical pending plan should execute after recovery, got {recorded_operations:?}"
        );
        assert_eq!(active_plan_id, None);
        assert_eq!(pending_plan_id, None);
        assert!(state
            .storage_handle()
            .find_choreography_pending_execution_body(&pending_result.plan_id)
            .expect("find consumed pending body")
            .is_none());

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn timeline_command_yields_to_deferred_pending_plan_after_current_finish_step() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-step-boundary-yield-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let storage = state.storage_handle();
        let executor = RecordingStepExecutor::default();
        let active_plan_id = "plan_step_boundary_active_019f5b00-0000-7000-8000-000000000131";
        let active_plan = TimelinePlan {
            plan_id: active_plan_id.to_owned(),
            source_ref: json!({
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5b00-0000-7000-8000-000000000231",
                "messageId": "message_019f5b00-0000-7000-8000-000000000331"
            }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps: vec![
                TimelineStep::Wait(WaitStep::new(
                    "step_step_boundary_wait_019f5b00-0000-7000-8000-000000000431",
                    120,
                    500,
                )),
                TimelineStep::MoveTo(MoveToStep::center(
                    "step_step_boundary_move_019f5b00-0000-7000-8000-000000000432",
                    30_000,
                )),
            ],
            created_at: "2026-07-12T00:00:00.000Z".to_owned(),
        };
        let scheduled = state
            .with_choreography_timeline_scheduler(|admission, pending_queue| {
                admit_timeline_plan_with_pending_queue(
                    admission,
                    pending_queue,
                    TimelineAdmissionExecutionRequest::new(
                        storage.clone(),
                        active_plan,
                        TimelineExecutionContext::fixed_for_test(),
                        Default::default(),
                        ChoreographyTriggerSource::AiChoreography,
                    ),
                )
                .map_err(timeline_execution_error_to_buddy_error)
            })
            .expect("admit active timeline");
        let execution = scheduled.execution.expect("active timeline should execute");
        let active_state = state.clone();
        let active_storage = storage.clone();
        let active_executor = executor.clone();
        let (step_started_tx, step_started_rx) = mpsc::channel();
        let active_thread = thread::spawn(move || {
            execute_test_timeline_with_state_scheduler(
                &active_state,
                active_storage,
                &active_executor,
                execution,
                step_started_tx,
            )
        });

        step_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("wait step should start");
        let pending_result = run_choreography_macro_intent_from_state_with_executor(
            &state,
            RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "dance",
                    "params": { "durationMs": 1000 }
                }))
                .expect("pending dance macro"),
                source_ref: None,
                trigger_source: None,
            },
            &PanicStepExecutor,
        )
        .expect("queue pending macro intent");

        let active_report = active_thread
            .join()
            .expect("join active timeline")
            .expect("active timeline result");
        let operations = executor.operations();
        let active_summary = storage.action_log_plan_summary_for_test(active_plan_id);

        assert_eq!(pending_result.admission_decision, "deferred");
        assert!(!pending_result.executed);
        assert_eq!(active_report.plan_id, active_plan_id);
        assert!(
            operations
                .iter()
                .any(|operation| operation == "play:celebrate"),
            "promoted pending macro should execute after wait step, got {operations:?}"
        );
        assert!(
            !operations
                .iter()
                .any(|operation| operation == "moveTo:center"),
            "active timeline should yield before its next move step, got {operations:?}"
        );
        assert_eq!(
            active_summary,
            json!({
                "status": "interrupted",
                "lastEventType": "plan.interrupted",
                "lastReasonCode": "timeline.yieldedToPendingPlan",
                "resolvedActionId": null,
                "resolvedAnimationRef": null
            })
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn timeline_command_runs_pending_handoff_finalizer_before_deferred_plan() {
        let data_dir = std::env::temp_dir().join(format!(
            "lexora-buddy-command-pending-handoff-finalizer-{}",
            uuid::Uuid::new_v4()
        ));
        let state =
            BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(data_dir.clone()))
                .expect("initialize state");
        let storage = state.storage_handle();
        let active_plan_id = "plan_handoff_active_019f5b00-0000-7000-8000-000000000141";
        let active_plan: TimelinePlan = serde_json::from_value(json!({
            "planId": active_plan_id,
            "sourceRef": {
                "kind": "conversationMessage",
                "conversationId": "conversation_019f5b00-0000-7000-8000-000000000241",
                "messageId": "message_019f5b00-0000-7000-8000-000000000341"
            },
            "failurePolicy": "abort",
            "steps": [
                {
                    "stepId": "step_handoff_enter_019f5b00-0000-7000-8000-000000000441",
                    "kind": "playAction",
                    "actionId": "reassure",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000,
                    "pendingHandoffFinalizerStepId": "step_handoff_exit_019f5b00-0000-7000-8000-000000000443"
                },
                {
                    "stepId": "step_handoff_loop_019f5b00-0000-7000-8000-000000000442",
                    "kind": "playAction",
                    "actionId": "tap",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000,
                    "pendingHandoffFinalizerStepId": "step_handoff_exit_019f5b00-0000-7000-8000-000000000443"
                },
                {
                    "stepId": "step_handoff_exit_019f5b00-0000-7000-8000-000000000443",
                    "kind": "playAction",
                    "actionId": "curious",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                },
                {
                    "stepId": "step_handoff_after_019f5b00-0000-7000-8000-000000000444",
                    "kind": "playAction",
                    "actionId": "wake",
                    "expectedPlayback": "once",
                    "timeoutMs": 5000
                }
            ],
            "createdAt": "2026-07-16T00:00:00.000Z"
        }))
        .expect("active timeline plan");
        let scheduled = state
            .with_choreography_timeline_scheduler(|admission, pending_queue| {
                admit_timeline_plan_with_pending_queue(
                    admission,
                    pending_queue,
                    TimelineAdmissionExecutionRequest::new(
                        storage.clone(),
                        active_plan,
                        TimelineExecutionContext::fixed_for_test(),
                        Default::default(),
                        ChoreographyTriggerSource::AiChoreography,
                    ),
                )
                .map_err(timeline_execution_error_to_buddy_error)
            })
            .expect("admit active timeline");
        let execution = scheduled.execution.expect("active timeline should execute");
        let active_state = state.clone();
        let active_storage = storage.clone();
        let (action_started_tx, action_started_rx) = mpsc::channel();
        let (release_action_tx, release_action_rx) = mpsc::channel();
        let active_thread = thread::spawn(move || {
            execute_test_timeline_with_state_scheduler(
                &active_state,
                active_storage,
                &BlockingActionStepExecutor {
                    action_started: action_started_tx,
                    release_action: release_action_rx,
                },
                execution,
                mpsc::channel().0,
            )
        });

        assert_eq!(
            action_started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("enter action should start"),
            "reassure"
        );
        let pending_result = run_choreography_macro_intent_from_state_with_executor(
            &state,
            RunChoreographyMacroIntentRequest {
                intent: serde_json::from_value(json!({
                    "macroId": "dance",
                    "params": { "durationMs": 1000 }
                }))
                .expect("pending dance macro"),
                source_ref: None,
                trigger_source: None,
            },
            &PanicStepExecutor,
        )
        .expect("queue pending macro intent");
        release_action_tx.send(()).expect("release enter action");

        let next_action = action_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("handoff finalizer should start");
        release_action_tx
            .send(())
            .expect("release handoff finalizer");
        assert_eq!(next_action, "curious");

        let pending_action = action_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("promoted pending action should start");
        release_action_tx
            .send(())
            .expect("release promoted pending action");
        let active_report = active_thread
            .join()
            .expect("join active timeline")
            .expect("active timeline result");

        assert_eq!(pending_result.admission_decision, "deferred");
        assert!(!pending_result.executed);
        assert_eq!(active_report.plan_id, active_plan_id);
        assert_eq!(pending_action, "celebrate");

        let _ = std::fs::remove_dir_all(data_dir);
    }
}
