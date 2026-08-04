use crate::{
    app_paths::BuddyAppPaths, error::BuddyError, runtime_instance::BuddyRuntimeInstanceLock,
    storage::BuddyStorage,
};

use super::{
    admission::{ChoreographyAdmissionState, ChoreographyTriggerSource},
    affective::{AffectiveContextStore, ResolveContext},
    executor::{
        execute_ai_macro_demo_dev_fixture, execute_macro_intent_with_admission,
        execute_single_play_action_dev_fixture, ChoreographyStepExecutor,
        DevFixtureAdmissionExecutionRequest, DevFixtureExecutionContext, DevFixtureExecutionError,
        DevFixtureExecutionReport, DevFixtureKind, MacroIntentExecutionContext,
        MacroIntentExecutionRequest, NativePetChoreographyStepExecutor,
        TimelineAdmissionExecutionReport, TimelineExecutionError,
    },
    fixture::{AI_MACRO_DEMO_FIXTURE_NAME, SINGLE_PLAY_ACTION_FIXTURE_NAME},
    macro_plan::MacroIntent,
};

#[cfg(test)]
use super::executor::{
    execute_ai_macro_demo_dev_fixture_with_admission,
    execute_single_play_action_dev_fixture_with_admission, DevFixtureAdmissionExecutionReport,
};

pub(crate) const CHOREOGRAPHY_DEV_FIXTURE_ARG: &str = "--buddy-choreography-dev-fixture";
pub(crate) const SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME: &str = SINGLE_PLAY_ACTION_FIXTURE_NAME;
pub(crate) const AI_MACRO_DEMO_DEV_FIXTURE_NAME: &str = AI_MACRO_DEMO_FIXTURE_NAME;
const MACRO_INTENT_DEV_SOURCE_FIXTURE_NAME: &str = "macro-intent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChoreographyDevFixtureCommand {
    pub(crate) fixture_name: &'static str,
}

pub(crate) struct MacroIntentRunSource {
    pub(crate) source_ref: serde_json::Value,
    pub(crate) trigger_source: ChoreographyTriggerSource,
}

impl MacroIntentRunSource {
    pub(crate) fn user_requested_dev_fixture() -> Self {
        Self {
            source_ref: serde_json::json!({
                "kind": "devFixture",
                "fixtureName": MACRO_INTENT_DEV_SOURCE_FIXTURE_NAME,
            }),
            trigger_source: ChoreographyTriggerSource::UserRequested,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoreographyDevFixtureCommandError {
    exit_code: i32,
    message: String,
}

impl ChoreographyDevFixtureCommandError {
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

    fn startup(message: impl Into<String>) -> Self {
        Self {
            exit_code: 2,
            message: message.into(),
        }
    }

    fn action_log(error: BuddyError) -> Self {
        Self {
            exit_code: 4,
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

pub(crate) fn parse_choreography_dev_fixture_command<I, S>(
    args: I,
) -> Option<Result<ChoreographyDevFixtureCommand, ChoreographyDevFixtureCommandError>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let fixture_arg_index = args
        .iter()
        .position(|arg| arg == CHOREOGRAPHY_DEV_FIXTURE_ARG)?;
    let Some(fixture_name) = args.get(fixture_arg_index + 1) else {
        return Some(Err(ChoreographyDevFixtureCommandError::invalid_args(
            "--buddy-choreography-dev-fixture requires a fixture name",
        )));
    };
    let fixture_name = match fixture_name.as_str() {
        SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME => SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME,
        AI_MACRO_DEMO_DEV_FIXTURE_NAME => AI_MACRO_DEMO_DEV_FIXTURE_NAME,
        _ => {
            return Some(Err(ChoreographyDevFixtureCommandError::invalid_args(
                "unsupported buddy choreography dev fixture",
            )));
        }
    };

    Some(Ok(ChoreographyDevFixtureCommand { fixture_name }))
}

pub fn run_choreography_dev_fixture_command_from_env(
) -> Option<Result<(), ChoreographyDevFixtureCommandError>> {
    let command = parse_choreography_dev_fixture_command(std::env::args())?;

    Some(command.and_then(run_choreography_dev_fixture_command))
}

fn run_choreography_dev_fixture_command(
    command: ChoreographyDevFixtureCommand,
) -> Result<(), ChoreographyDevFixtureCommandError> {
    if !matches!(
        command.fixture_name,
        SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME | AI_MACRO_DEMO_DEV_FIXTURE_NAME
    ) {
        return Err(ChoreographyDevFixtureCommandError::invalid_args(
            "unsupported buddy choreography dev fixture",
        ));
    }

    let paths = BuddyAppPaths::from_default_buddy_home();
    let _runtime_instance_lock = BuddyRuntimeInstanceLock::acquire(&paths.data_dir_path())
        .map_err(|error| ChoreographyDevFixtureCommandError::startup(error.to_string()))?;
    paths
        .ensure_exists()
        .map_err(|error| ChoreographyDevFixtureCommandError::startup(error.to_string()))?;
    let storage = BuddyStorage::new_with_buddy_home(paths.database_path(), paths.data_dir_path());
    storage
        .initialize()
        .map_err(|error| ChoreographyDevFixtureCommandError::startup(error.to_string()))?;

    let resolve_context = AffectiveContextStore::from_buddy_home(paths.data_dir_path())
        .read_or_create_default_with_diagnostics(&storage)
        .map(ResolveContext::from_affective_snapshot)
        .map_err(|error| ChoreographyDevFixtureCommandError::startup(error.to_string()))?;
    let executor =
        NativePetChoreographyStepExecutor::spawn_sidecar_with_startup_health_diagnostics(&storage)
            .map_err(|error| ChoreographyDevFixtureCommandError::startup(error.to_string()))?;
    let report =
        execute_dev_fixture_command(command.fixture_name, storage, &executor, resolve_context)
            .map_err(|error| match error {
                DevFixtureExecutionError::ActionLog(error) => {
                    ChoreographyDevFixtureCommandError::action_log(error)
                }
                DevFixtureExecutionError::Execution(error) => {
                    ChoreographyDevFixtureCommandError::execution(error)
                }
            })?;
    let _completed_plan_id = report.plan_id;

    Ok(())
}

fn execute_dev_fixture_command(
    fixture_name: &str,
    storage: BuddyStorage,
    executor: &NativePetChoreographyStepExecutor,
    resolve_context: ResolveContext,
) -> Result<DevFixtureExecutionReport, DevFixtureExecutionError> {
    match fixture_name {
        SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME => execute_single_play_action_dev_fixture(
            storage,
            executor,
            DevFixtureExecutionContext::new(),
            resolve_context,
        ),
        AI_MACRO_DEMO_DEV_FIXTURE_NAME => execute_ai_macro_demo_dev_fixture(
            storage,
            executor,
            DevFixtureExecutionContext::new(),
            resolve_context,
        ),
        _ => Err(DevFixtureExecutionError::Execution(BuddyError::Validation(
            "unsupported buddy choreography dev fixture".to_owned(),
        ))),
    }
}

#[cfg(test)]
pub(crate) fn run_choreography_dev_fixture_with_admission(
    fixture_name: &str,
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    resolve_context: ResolveContext,
) -> Result<DevFixtureAdmissionExecutionReport, DevFixtureExecutionError> {
    match fixture_name {
        SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME => {
            execute_single_play_action_dev_fixture_with_admission(
                storage,
                executor,
                admission,
                DevFixtureExecutionContext::new(),
                resolve_context,
                ChoreographyTriggerSource::UserRequested,
            )
        }
        AI_MACRO_DEMO_DEV_FIXTURE_NAME => execute_ai_macro_demo_dev_fixture_with_admission(
            storage,
            executor,
            admission,
            DevFixtureExecutionContext::new(),
            resolve_context,
            ChoreographyTriggerSource::UserRequested,
        ),
        _ => Err(DevFixtureExecutionError::Execution(BuddyError::Validation(
            "unsupported buddy choreography dev fixture".to_owned(),
        ))),
    }
}

pub(crate) fn create_choreography_dev_fixture_admission_request(
    fixture_name: &str,
    storage: BuddyStorage,
    resolve_context: ResolveContext,
) -> Result<DevFixtureAdmissionExecutionRequest, DevFixtureExecutionError> {
    let fixture_kind = match fixture_name {
        SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME => DevFixtureKind::SinglePlayAction,
        AI_MACRO_DEMO_DEV_FIXTURE_NAME => DevFixtureKind::AiMacroDemo,
        _ => {
            return Err(DevFixtureExecutionError::Execution(BuddyError::Validation(
                "unsupported buddy choreography dev fixture".to_owned(),
            )));
        }
    };

    Ok(DevFixtureAdmissionExecutionRequest::new(
        storage,
        DevFixtureExecutionContext::new(),
        resolve_context,
        fixture_kind,
        ChoreographyTriggerSource::UserRequested,
    ))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn run_choreography_macro_intent_with_source_admission(
    intent: MacroIntent,
    storage: BuddyStorage,
    executor: &impl ChoreographyStepExecutor,
    admission: &mut ChoreographyAdmissionState,
    resolve_context: ResolveContext,
    source: MacroIntentRunSource,
) -> Result<TimelineAdmissionExecutionReport, TimelineExecutionError> {
    execute_macro_intent_with_admission(
        storage,
        executor,
        admission,
        &intent,
        MacroIntentExecutionRequest {
            context: MacroIntentExecutionContext::new(),
            source_ref: source.source_ref,
            resolve_context,
            trigger_source: source.trigger_source,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::choreography::{
        admission::{
            ChoreographyAdmissionDecision, ChoreographyAdmissionState, ChoreographyTriggerSource,
        },
        executor::ChoreographyStepExecutor,
        macro_plan::{DanceMacroParams, MacroIntent},
        registry::StepResolution,
        timeline::{MoveByPathStep, MoveToStep, PlayActionStep, WaitStep},
    };
    use crate::error::BuddyResult;

    use super::*;

    #[derive(Default)]
    struct FakeStepExecutor {
        played_animation_refs: RefCell<Vec<String>>,
        play_action_duration_ms: RefCell<Vec<Option<u64>>>,
    }

    impl ChoreographyStepExecutor for FakeStepExecutor {
        fn play_action_step(
            &self,
            step: &PlayActionStep,
            resolution: &StepResolution,
        ) -> BuddyResult<()> {
            self.played_animation_refs
                .borrow_mut()
                .push(resolution.animation_ref.clone());
            self.play_action_duration_ms
                .borrow_mut()
                .push(step.duration_ms);

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

    #[test]
    fn internal_dev_fixture_command_runs_fixed_fixture_through_admission() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();

        let report = run_choreography_dev_fixture_with_admission(
            SINGLE_PLAY_ACTION_DEV_FIXTURE_NAME,
            storage.clone(),
            &executor,
            &mut admission,
            ResolveContext::default(),
        )
        .expect("run internal fixture command");

        assert!(report.executed);
        assert!(matches!(
            report.decision,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert_eq!(
            executor.played_animation_refs.into_inner(),
            vec!["celebrate".to_owned()]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
    }

    #[test]
    fn internal_macro_intent_command_accepts_structured_macro_params() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();

        let report = run_choreography_macro_intent_with_source_admission(
            MacroIntent::Dance(DanceMacroParams { duration_ms: 2_500 }),
            storage.clone(),
            &executor,
            &mut admission,
            ResolveContext::default(),
            MacroIntentRunSource::user_requested_dev_fixture(),
        )
        .expect("run internal macro intent command");

        assert!(report.executed);
        assert!(matches!(
            report.decision,
            ChoreographyAdmissionDecision::Accepted { .. }
        ));
        assert_eq!(
            executor.played_animation_refs.into_inner(),
            vec!["celebrate".to_owned()]
        );
        assert_eq!(
            executor.play_action_duration_ms.into_inner(),
            vec![Some(2_500)]
        );
        assert_eq!(
            storage.action_log_event_types_for_test(&report.plan_id),
            vec![
                "executor.accepted",
                "plan.started",
                "step.resolved",
                "step.completed",
                "plan.completed",
            ]
        );
    }

    #[test]
    fn internal_macro_intent_command_uses_supplied_source_ref() {
        let storage = BuddyStorage::new_temporary_for_test().expect("create storage");
        let executor = FakeStepExecutor::default();
        let mut admission = ChoreographyAdmissionState::default();

        let report = run_choreography_macro_intent_with_source_admission(
            MacroIntent::Dance(DanceMacroParams { duration_ms: 2_500 }),
            storage.clone(),
            &executor,
            &mut admission,
            ResolveContext::default(),
            MacroIntentRunSource {
                source_ref: serde_json::json!({
                    "kind": "run",
                    "runId": "run_macro_intent",
                }),
                trigger_source: ChoreographyTriggerSource::AiChoreography,
            },
        )
        .expect("run internal macro intent command with source");

        let plan = storage
            .list_action_log_plans(crate::storage::ActionLogPlanListRequest {
                source_ref_kind: Some("run".to_owned()),
                ..crate::storage::ActionLogPlanListRequest::default()
            })
            .expect("list action log plans")
            .items
            .into_iter()
            .next()
            .expect("run sourced macro intent action log plan");

        assert_eq!(plan.plan_id, report.plan_id);
        assert_eq!(plan.source_ref_kind, "run");
        assert_eq!(plan.source_ref["kind"], "run");
        assert_eq!(plan.source_ref["runId"], "run_macro_intent");
    }
}
