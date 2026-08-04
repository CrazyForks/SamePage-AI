use serde::{Deserialize, Serialize};

use crate::error::{BuddyError, BuddyResult};

const MAX_EXPANDED_TIMELINE_STEPS: usize = 512;
const MAX_TIMELINE_RETRY_ATTEMPTS: u8 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TimelinePlan {
    pub(crate) plan_id: String,
    pub(crate) source_ref: serde_json::Value,
    pub(crate) failure_policy: TimelineFailurePolicy,
    pub(crate) steps: Vec<TimelineStep>,
    pub(crate) created_at: String,
}

#[cfg_attr(not(test), allow(dead_code))]
impl TimelinePlan {
    pub(crate) fn step_count(&self) -> usize {
        self.steps.len()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum TimelineFailurePolicy {
    #[default]
    Abort,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum TimelineStep {
    PlayAction(PlayActionStep),
    MoveTo(MoveToStep),
    MoveByPath(MoveByPathStep),
    Wait(WaitStep),
    Skip(SkipStep),
    Repeat(RepeatStep),
    Choose(ChooseStep),
    SetFallback(SetFallbackStep),
    Retry(RetryStep),
    Replace(ReplaceStep),
    Recover(RecoverStep),
    Try(TryStep),
    SnapshotPosition(SnapshotPositionStep),
    RestorePosition(RestorePositionStep),
}

impl TimelineStep {
    pub(crate) fn step_id(&self) -> &str {
        match self {
            Self::PlayAction(step) => step.step_id.as_str(),
            Self::MoveTo(step) => step.step_id.as_str(),
            Self::MoveByPath(step) => step.step_id.as_str(),
            Self::Wait(step) => step.step_id.as_str(),
            Self::Skip(step) => step.step_id.as_str(),
            Self::Repeat(step) => step.step_id.as_str(),
            Self::Choose(step) => step.step_id.as_str(),
            Self::SetFallback(step) => step.step_id.as_str(),
            Self::Retry(step) => step.step_id.as_str(),
            Self::Replace(step) => step.step_id.as_str(),
            Self::Recover(step) => step.step_id.as_str(),
            Self::Try(step) => step.step_id.as_str(),
            Self::SnapshotPosition(step) => step.step_id.as_str(),
            Self::RestorePosition(step) => step.step_id.as_str(),
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::PlayAction(_) => "playAction",
            Self::MoveTo(_) => "moveTo",
            Self::MoveByPath(_) => "moveByPath",
            Self::Wait(_) => "wait",
            Self::Skip(_) => "skipStep",
            Self::Repeat(_) => "repeat",
            Self::Choose(_) => "choose",
            Self::SetFallback(_) => "setFallback",
            Self::Retry(_) => "retry",
            Self::Replace(_) => "replaceStep",
            Self::Recover(_) => "recover",
            Self::Try(_) => "try",
            Self::SnapshotPosition(_) => "snapshotPosition",
            Self::RestorePosition(_) => "restorePosition",
        }
    }

    pub(crate) fn pending_handoff_finalizer_step_id(&self) -> Option<&str> {
        let Self::PlayAction(step) = self else {
            return None;
        };

        step.pending_handoff_finalizer_step_id.as_deref()
    }
}

pub(crate) fn expand_planner_timeline_plan(mut plan: TimelinePlan) -> BuddyResult<TimelinePlan> {
    plan.steps = expand_planner_timeline_steps(&plan.steps)?;
    validate_pending_handoff_finalizers(&plan.steps)?;
    Ok(plan)
}

pub(crate) fn expand_planner_timeline_steps(
    steps: &[TimelineStep],
) -> BuddyResult<Vec<TimelineStep>> {
    let mut expanded_steps = Vec::new();
    expand_planner_steps_into(steps, None, None, &mut expanded_steps)?;

    Ok(expanded_steps)
}

fn expand_planner_steps_into(
    steps: &[TimelineStep],
    inherited_fallback_action_id: Option<&str>,
    repeated_step_suffix: Option<&str>,
    expanded_steps: &mut Vec<TimelineStep>,
) -> BuddyResult<bool> {
    let mut consumed_inherited_fallback = false;

    for step in steps {
        consumed_inherited_fallback |= expand_planner_step_into(
            step,
            inherited_fallback_action_id,
            repeated_step_suffix,
            expanded_steps,
        )?;
    }

    Ok(consumed_inherited_fallback)
}

fn expand_planner_step_into(
    step: &TimelineStep,
    inherited_fallback_action_id: Option<&str>,
    repeated_step_suffix: Option<&str>,
    expanded_steps: &mut Vec<TimelineStep>,
) -> BuddyResult<bool> {
    match step {
        TimelineStep::PlayAction(step) => {
            let (step, consumed_inherited_fallback) = play_action_step_with_planner_context(
                step,
                inherited_fallback_action_id,
                repeated_step_suffix,
            );
            push_expanded_timeline_step(step, expanded_steps)?;
            Ok(consumed_inherited_fallback)
        }
        TimelineStep::MoveTo(step) => {
            push_expanded_timeline_step(
                repeated_timeline_step_id(TimelineStep::MoveTo(step.clone()), repeated_step_suffix),
                expanded_steps,
            )?;
            Ok(false)
        }
        TimelineStep::MoveByPath(step) => {
            push_expanded_timeline_step(
                repeated_timeline_step_id(
                    TimelineStep::MoveByPath(step.clone()),
                    repeated_step_suffix,
                ),
                expanded_steps,
            )?;
            Ok(false)
        }
        TimelineStep::Wait(step) => {
            push_expanded_timeline_step(
                repeated_timeline_step_id(TimelineStep::Wait(step.clone()), repeated_step_suffix),
                expanded_steps,
            )?;
            Ok(false)
        }
        TimelineStep::Skip(step) => {
            push_expanded_timeline_step(
                repeated_timeline_step_id(TimelineStep::Skip(step.clone()), repeated_step_suffix),
                expanded_steps,
            )?;
            Ok(false)
        }
        TimelineStep::Repeat(step) => {
            if step.times == 0 {
                return Err(BuddyError::Validation(format!(
                    "repeat timeline step times must be greater than zero: {}",
                    step.step_id
                )));
            }
            if step.steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "repeat timeline step must contain at least one nested step: {}",
                    step.step_id
                )));
            }

            let mut consumed_inherited_fallback = false;
            for iteration_index in 0..step.times {
                let iteration_suffix = repeated_iteration_suffix(
                    repeated_step_suffix,
                    usize::from(iteration_index) + 1,
                );
                consumed_inherited_fallback |= expand_planner_steps_into(
                    &step.steps,
                    inherited_fallback_action_id,
                    Some(iteration_suffix.as_str()),
                    expanded_steps,
                )?;
            }

            Ok(consumed_inherited_fallback)
        }
        TimelineStep::Choose(step) => {
            let selected_option = select_choose_option(step)?;
            expand_planner_steps_into(
                &selected_option.steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                expanded_steps,
            )
        }
        TimelineStep::SetFallback(step) => {
            let consumed_set_fallback = expand_planner_steps_into(
                &step.steps,
                Some(step.fallback_action_id.as_str()),
                repeated_step_suffix,
                expanded_steps,
            )?;
            if !consumed_set_fallback {
                return Err(BuddyError::Validation(format!(
                    "setFallback step {} must apply to at least one playAction timeline step",
                    step.step_id
                )));
            }

            Ok(false)
        }
        TimelineStep::Retry(step) => {
            if step.max_attempts < 2 {
                return Err(BuddyError::Validation(format!(
                    "retry timeline step maxAttempts must be at least two: {}",
                    step.step_id
                )));
            }
            if step.max_attempts > MAX_TIMELINE_RETRY_ATTEMPTS {
                return Err(BuddyError::Validation(format!(
                    "retry timeline step maxAttempts must be at most {MAX_TIMELINE_RETRY_ATTEMPTS}: {}",
                    step.step_id
                )));
            }
            if step.steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "retry timeline step must contain at least one nested step: {}",
                    step.step_id
                )));
            }

            let mut retry_steps = Vec::new();
            let consumed_fallback = expand_planner_steps_into(
                &step.steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                &mut retry_steps,
            )?;

            push_expanded_timeline_step(
                TimelineStep::Retry(RetryStep {
                    step_id: repeated_step_id_if_needed(&step.step_id, repeated_step_suffix),
                    kind: step.kind.clone(),
                    max_attempts: step.max_attempts,
                    steps: retry_steps,
                }),
                expanded_steps,
            )?;

            Ok(consumed_fallback)
        }
        TimelineStep::Replace(step) => {
            if step.steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "replaceStep timeline step must contain at least one primary step: {}",
                    step.step_id
                )));
            }
            if step.replacement_steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "replaceStep timeline step must contain at least one replacement step: {}",
                    step.step_id
                )));
            }

            let mut primary_steps = Vec::new();
            let consumed_primary_fallback = expand_planner_steps_into(
                &step.steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                &mut primary_steps,
            )?;
            let mut replacement_steps = Vec::new();
            let consumed_replacement_fallback = expand_planner_steps_into(
                &step.replacement_steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                &mut replacement_steps,
            )?;

            push_expanded_timeline_step(
                TimelineStep::Replace(ReplaceStep {
                    step_id: repeated_step_id_if_needed(&step.step_id, repeated_step_suffix),
                    kind: step.kind.clone(),
                    steps: primary_steps,
                    replacement_steps,
                }),
                expanded_steps,
            )?;

            Ok(consumed_primary_fallback || consumed_replacement_fallback)
        }
        TimelineStep::Recover(step) => {
            if step.steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "recover timeline step must contain at least one primary step: {}",
                    step.step_id
                )));
            }
            if step.recovery_steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "recover timeline step must contain at least one recovery step: {}",
                    step.step_id
                )));
            }

            let mut primary_steps = Vec::new();
            let consumed_primary_fallback = expand_planner_steps_into(
                &step.steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                &mut primary_steps,
            )?;
            let mut recovery_steps = Vec::new();
            let consumed_recovery_fallback = expand_planner_steps_into(
                &step.recovery_steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                &mut recovery_steps,
            )?;

            push_expanded_timeline_step(
                TimelineStep::Recover(RecoverStep {
                    step_id: repeated_step_id_if_needed(&step.step_id, repeated_step_suffix),
                    kind: step.kind.clone(),
                    steps: primary_steps,
                    recovery_steps,
                }),
                expanded_steps,
            )?;

            Ok(consumed_primary_fallback || consumed_recovery_fallback)
        }
        TimelineStep::Try(step) => {
            if step.steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "try timeline step must contain at least one primary step: {}",
                    step.step_id
                )));
            }
            if step.fallback_steps.is_empty() {
                return Err(BuddyError::Validation(format!(
                    "try timeline step must contain at least one fallback step: {}",
                    step.step_id
                )));
            }

            let mut primary_steps = Vec::new();
            let consumed_primary_fallback = expand_planner_steps_into(
                &step.steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                &mut primary_steps,
            )?;
            let mut fallback_steps = Vec::new();
            let consumed_branch_fallback = expand_planner_steps_into(
                &step.fallback_steps,
                inherited_fallback_action_id,
                repeated_step_suffix,
                &mut fallback_steps,
            )?;

            push_expanded_timeline_step(
                TimelineStep::Try(TryStep {
                    step_id: repeated_step_id_if_needed(&step.step_id, repeated_step_suffix),
                    kind: step.kind.clone(),
                    steps: primary_steps,
                    fallback_steps,
                }),
                expanded_steps,
            )?;

            Ok(consumed_primary_fallback || consumed_branch_fallback)
        }
        TimelineStep::SnapshotPosition(step) => {
            push_expanded_timeline_step(
                repeated_timeline_step_id(
                    TimelineStep::SnapshotPosition(step.clone()),
                    repeated_step_suffix,
                ),
                expanded_steps,
            )?;
            Ok(false)
        }
        TimelineStep::RestorePosition(step) => {
            push_expanded_timeline_step(
                repeated_timeline_step_id(
                    TimelineStep::RestorePosition(step.clone()),
                    repeated_step_suffix,
                ),
                expanded_steps,
            )?;
            Ok(false)
        }
    }
}

fn play_action_step_with_planner_context(
    step: &PlayActionStep,
    inherited_fallback_action_id: Option<&str>,
    repeated_step_suffix: Option<&str>,
) -> (TimelineStep, bool) {
    let mut step = step.clone();
    let mut consumed_inherited_fallback = false;
    if step.fallback_action_id.is_none() {
        step.fallback_action_id = inherited_fallback_action_id.map(str::to_owned);
        consumed_inherited_fallback = inherited_fallback_action_id.is_some();
    }

    (
        repeated_timeline_step_id(TimelineStep::PlayAction(step), repeated_step_suffix),
        consumed_inherited_fallback,
    )
}

fn repeated_timeline_step_id(
    mut step: TimelineStep,
    repeated_step_suffix: Option<&str>,
) -> TimelineStep {
    let Some(suffix) = repeated_step_suffix else {
        return step;
    };

    match &mut step {
        TimelineStep::PlayAction(step) => {
            step.step_id = repeated_step_id(&step.step_id, suffix);
            if let Some(finalizer_step_id) = &mut step.pending_handoff_finalizer_step_id {
                *finalizer_step_id = repeated_step_id(finalizer_step_id, suffix);
            }
        }
        TimelineStep::MoveTo(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::MoveByPath(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::Wait(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::Skip(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::SnapshotPosition(step) => {
            step.step_id = repeated_step_id(&step.step_id, suffix)
        }
        TimelineStep::RestorePosition(step) => {
            step.step_id = repeated_step_id(&step.step_id, suffix)
        }
        TimelineStep::Try(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::Retry(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::Replace(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::Recover(step) => step.step_id = repeated_step_id(&step.step_id, suffix),
        TimelineStep::Repeat(_) | TimelineStep::Choose(_) | TimelineStep::SetFallback(_) => {}
    }

    step
}

fn repeated_step_id(step_id: &str, suffix: &str) -> String {
    format!("{step_id}__{suffix}")
}

fn repeated_step_id_if_needed(step_id: &str, suffix: Option<&str>) -> String {
    suffix
        .map(|suffix| repeated_step_id(step_id, suffix))
        .unwrap_or_else(|| step_id.to_owned())
}

fn repeated_iteration_suffix(parent_suffix: Option<&str>, one_based_index: usize) -> String {
    let suffix = format!("repeat_{one_based_index:03}");
    parent_suffix
        .map(|parent| format!("{parent}__{suffix}"))
        .unwrap_or(suffix)
}

fn select_choose_option(step: &ChooseStep) -> BuddyResult<&ChooseOption> {
    if step.options.is_empty() {
        return Err(BuddyError::Validation(format!(
            "choose timeline step must contain at least one option: {}",
            step.step_id
        )));
    }
    for option in &step.options {
        if option.steps.is_empty() {
            return Err(BuddyError::Validation(format!(
                "choose timeline option {} must contain at least one nested step: {}",
                option.option_id, step.step_id
            )));
        }
    }

    match step.strategy.as_str() {
        "first" => Ok(&step.options[0]),
        "weighted" => step
            .options
            .iter()
            .min_by(|left, right| {
                right
                    .weight
                    .cmp(&left.weight)
                    .then_with(|| left.option_id.cmp(&right.option_id))
            })
            .ok_or_else(|| {
                BuddyError::Validation(format!(
                    "choose timeline step must contain at least one option: {}",
                    step.step_id
                ))
            }),
        strategy => Err(BuddyError::Validation(format!(
            "unsupported choose timeline strategy `{strategy}`: {}",
            step.step_id
        ))),
    }
}

fn push_expanded_timeline_step(
    step: TimelineStep,
    expanded_steps: &mut Vec<TimelineStep>,
) -> BuddyResult<()> {
    if expanded_steps.len() >= MAX_EXPANDED_TIMELINE_STEPS {
        return Err(BuddyError::Validation(format!(
            "expanded timeline step count exceeds {MAX_EXPANDED_TIMELINE_STEPS}"
        )));
    }

    expanded_steps.push(step);
    Ok(())
}

fn validate_pending_handoff_finalizers(steps: &[TimelineStep]) -> BuddyResult<()> {
    for (source_index, source_step) in steps.iter().enumerate() {
        let Some(finalizer_step_id) = source_step.pending_handoff_finalizer_step_id() else {
            continue;
        };
        let mut matches = steps
            .iter()
            .enumerate()
            .filter(|(_, step)| step.step_id() == finalizer_step_id);
        let Some((finalizer_index, finalizer_step)) = matches.next() else {
            return Err(BuddyError::Validation(format!(
                "pending handoff finalizer step is missing: {} -> {finalizer_step_id}",
                source_step.step_id()
            )));
        };
        if matches.next().is_some() {
            return Err(BuddyError::Validation(format!(
                "pending handoff finalizer step id is ambiguous: {} -> {finalizer_step_id}",
                source_step.step_id()
            )));
        }
        if finalizer_index <= source_index {
            return Err(BuddyError::Validation(format!(
                "pending handoff finalizer must be a later step: {} -> {finalizer_step_id}",
                source_step.step_id()
            )));
        }
        let TimelineStep::PlayAction(finalizer_step) = finalizer_step else {
            return Err(BuddyError::Validation(format!(
                "pending handoff finalizer must be a playAction step: {} -> {finalizer_step_id}",
                source_step.step_id()
            )));
        };
        if finalizer_step.pending_handoff_finalizer_step_id.is_some() {
            return Err(BuddyError::Validation(format!(
                "pending handoff finalizer cannot reference another finalizer: {finalizer_step_id}"
            )));
        }
    }

    for step in steps {
        match step {
            TimelineStep::Repeat(step) => validate_pending_handoff_finalizers(&step.steps)?,
            TimelineStep::Choose(step) => {
                for option in &step.options {
                    validate_pending_handoff_finalizers(&option.steps)?;
                }
            }
            TimelineStep::SetFallback(step) => {
                validate_pending_handoff_finalizers(&step.steps)?;
            }
            TimelineStep::Retry(step) => validate_pending_handoff_finalizers(&step.steps)?,
            TimelineStep::Replace(step) => {
                validate_pending_handoff_finalizers(&step.steps)?;
                validate_pending_handoff_finalizers(&step.replacement_steps)?;
            }
            TimelineStep::Recover(step) => {
                validate_pending_handoff_finalizers(&step.steps)?;
                validate_pending_handoff_finalizers(&step.recovery_steps)?;
            }
            TimelineStep::Try(step) => {
                validate_pending_handoff_finalizers(&step.steps)?;
                validate_pending_handoff_finalizers(&step.fallback_steps)?;
            }
            TimelineStep::PlayAction(_)
            | TimelineStep::MoveTo(_)
            | TimelineStep::MoveByPath(_)
            | TimelineStep::Wait(_)
            | TimelineStep::Skip(_)
            | TimelineStep::SnapshotPosition(_)
            | TimelineStep::RestorePosition(_) => {}
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct WaitStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) duration_ms: u64,
    pub(crate) timeout_ms: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl WaitStep {
    pub(crate) fn new(step_id: impl Into<String>, duration_ms: u64, timeout_ms: u64) -> Self {
        Self {
            step_id: step_id.into(),
            kind: "wait".to_owned(),
            duration_ms,
            timeout_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct RepeatStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) times: u16,
    pub(crate) steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ChooseStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) strategy: String,
    pub(crate) options: Vec<ChooseOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ChooseOption {
    pub(crate) option_id: String,
    pub(crate) weight: u16,
    pub(crate) steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SetFallbackStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) fallback_action_id: String,
    pub(crate) steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TryStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) steps: Vec<TimelineStep>,
    pub(crate) fallback_steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct RetryStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) max_attempts: u8,
    pub(crate) steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ReplaceStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) steps: Vec<TimelineStep>,
    pub(crate) replacement_steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct RecoverStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) steps: Vec<TimelineStep>,
    pub(crate) recovery_steps: Vec<TimelineStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum TimelineSkipReason {
    BranchNotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SkipStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) reason: TimelineSkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SnapshotPositionStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) snapshot_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct RestorePositionStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) snapshot_id: String,
    pub(crate) after_action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_after_action_id: Option<String>,
    pub(crate) timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlayActionStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) action_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_action_id: Option<String>,
    pub(crate) expected_playback: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pending_handoff_finalizer_step_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::is_restore_idle"
    )]
    pub(crate) completion_behavior:
        crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior,
    pub(crate) timeout_ms: u64,
}

impl PlayActionStep {
    pub(crate) fn once(
        step_id: impl Into<String>,
        action_id: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            kind: "playAction".to_owned(),
            action_id: action_id.into(),
            fallback_action_id: None,
            expected_playback: "once".to_owned(),
            duration_ms: None,
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms,
        }
    }

    #[cfg(test)]
    pub(crate) fn loop_for_duration(
        step_id: impl Into<String>,
        action_id: impl Into<String>,
        duration_ms: u64,
        timeout_ms: u64,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            kind: "playAction".to_owned(),
            action_id: action_id.into(),
            fallback_action_id: None,
            expected_playback: "loopForDuration".to_owned(),
            duration_ms: Some(duration_ms),
            pending_handoff_finalizer_step_id: None,
            completion_behavior:
                crate::native_pet::step_protocol::SidecarPlayActionCompletionBehavior::RestoreIdle,
            timeout_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MoveToStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) target: MoveTarget,
    pub(crate) after_action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_after_action_id: Option<String>,
    pub(crate) timeout_ms: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl MoveToStep {
    pub(crate) fn edge(step_id: impl Into<String>, edge: MoveEdge, timeout_ms: u64) -> Self {
        Self::new(step_id, MoveTarget::Edge { edge }, timeout_ms)
    }

    pub(crate) fn center(step_id: impl Into<String>, timeout_ms: u64) -> Self {
        Self::new(step_id, MoveTarget::Center, timeout_ms)
    }

    pub(crate) fn home(step_id: impl Into<String>, timeout_ms: u64) -> Self {
        Self::new(step_id, MoveTarget::Home, timeout_ms)
    }

    pub(crate) fn position(step_id: impl Into<String>, x: i32, y: i32, timeout_ms: u64) -> Self {
        Self::new(step_id, MoveTarget::Position { x, y }, timeout_ms)
    }

    pub(crate) fn x(step_id: impl Into<String>, x: i32, timeout_ms: u64) -> Self {
        Self::new(step_id, MoveTarget::X { x }, timeout_ms)
    }

    pub(crate) fn target(step_id: impl Into<String>, target: MoveTarget, timeout_ms: u64) -> Self {
        Self::new(step_id, target, timeout_ms)
    }

    pub(crate) fn window_anchor(
        step_id: impl Into<String>,
        selector: WindowAnchorSelector,
        edge: impl Into<WindowAnchorEdge>,
        reveal: WindowAnchorReveal,
        duration_ms: u64,
        timeout_ms: u64,
    ) -> Self {
        Self::new(
            step_id,
            MoveTarget::WindowAnchor {
                selector,
                edge: edge.into(),
                reveal,
                duration_ms,
            },
            timeout_ms,
        )
    }

    pub(crate) fn home_with_after_action(
        step_id: impl Into<String>,
        after_action_id: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        let mut step = Self::home(step_id, timeout_ms);
        step.after_action_id = Some(after_action_id.into());
        step
    }

    pub(crate) fn home_with_after_action_fallback(
        step_id: impl Into<String>,
        after_action_id: impl Into<String>,
        fallback_after_action_id: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        let mut step = Self::home_with_after_action(step_id, after_action_id, timeout_ms);
        step.fallback_after_action_id = Some(fallback_after_action_id.into());
        step
    }

    fn new(step_id: impl Into<String>, target: MoveTarget, timeout_ms: u64) -> Self {
        Self {
            step_id: step_id.into(),
            kind: "moveTo".to_owned(),
            target,
            after_action_id: None,
            fallback_after_action_id: None,
            timeout_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MoveByPathStep {
    pub(crate) step_id: String,
    pub(crate) kind: String,
    pub(crate) path: Vec<MoveTarget>,
    pub(crate) after_action_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fallback_after_action_id: Option<String>,
    pub(crate) timeout_ms: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl MoveByPathStep {
    pub(crate) fn new(step_id: impl Into<String>, path: Vec<MoveTarget>, timeout_ms: u64) -> Self {
        Self {
            step_id: step_id.into(),
            kind: "moveByPath".to_owned(),
            path,
            after_action_id: None,
            fallback_after_action_id: None,
            timeout_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum MoveTarget {
    Center,
    Home,
    Edge {
        edge: MoveEdge,
    },
    EdgeAnchor {
        edge: MoveEdge,
        reveal: WindowAnchorReveal,
        #[serde(rename = "durationMs")]
        duration_ms: u64,
    },
    Position {
        x: i32,
        y: i32,
    },
    X {
        x: i32,
    },
    WindowAnchor {
        selector: WindowAnchorSelector,
        edge: WindowAnchorEdge,
        reveal: WindowAnchorReveal,
        #[serde(rename = "durationMs")]
        duration_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MoveEdge {
    Left,
    Top,
    Right,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WindowAnchorEdge {
    Auto,
    Left,
    Top,
    Right,
    Bottom,
}

impl WindowAnchorEdge {
    pub(crate) fn fallback_screen_edge(self) -> MoveEdge {
        match self {
            Self::Auto | Self::Left => MoveEdge::Left,
            Self::Top => MoveEdge::Top,
            Self::Right => MoveEdge::Right,
            Self::Bottom => MoveEdge::Bottom,
        }
    }
}

impl From<MoveEdge> for WindowAnchorEdge {
    fn from(edge: MoveEdge) -> Self {
        match edge {
            MoveEdge::Left => Self::Left,
            MoveEdge::Top => Self::Top,
            MoveEdge::Right => Self::Right,
            MoveEdge::Bottom => Self::Bottom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WindowAnchorSelector {
    pub(crate) kind: WindowAnchorSelectorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WindowAnchorSelectorKind {
    ActiveWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WindowAnchorReveal {
    Head,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline_plan(steps: Vec<TimelineStep>) -> TimelinePlan {
        TimelinePlan {
            plan_id: "plan_pending_handoff_finalizer_test".to_owned(),
            source_ref: serde_json::json!({ "kind": "devFixture" }),
            failure_policy: TimelineFailurePolicy::Abort,
            steps,
            created_at: "2026-07-16T00:00:00.000Z".to_owned(),
        }
    }

    fn action_step(step_id: &str) -> PlayActionStep {
        PlayActionStep::once(step_id, "celebrate", 5_000)
    }

    fn action_step_with_finalizer(step_id: &str, finalizer_step_id: &str) -> PlayActionStep {
        let mut step = action_step(step_id);
        step.pending_handoff_finalizer_step_id = Some(finalizer_step_id.to_owned());
        step
    }

    #[test]
    fn planner_expansion_rejects_missing_pending_handoff_finalizer() {
        let error = expand_planner_timeline_plan(timeline_plan(vec![TimelineStep::PlayAction(
            action_step_with_finalizer("enter", "exit"),
        )]))
        .expect_err("missing finalizer should fail");

        assert!(error
            .to_string()
            .contains("pending handoff finalizer step is missing: enter -> exit"));
    }

    #[test]
    fn planner_expansion_rejects_missing_nested_pending_handoff_finalizer() {
        let error =
            expand_planner_timeline_plan(timeline_plan(vec![TimelineStep::Recover(RecoverStep {
                step_id: "recover".to_owned(),
                kind: "recover".to_owned(),
                steps: vec![TimelineStep::PlayAction(action_step_with_finalizer(
                    "enter", "exit",
                ))],
                recovery_steps: vec![TimelineStep::PlayAction(action_step("recover-idle"))],
            })]))
            .expect_err("missing nested finalizer should fail");

        assert!(error
            .to_string()
            .contains("pending handoff finalizer step is missing: enter -> exit"));
    }

    #[test]
    fn planner_expansion_rejects_backward_pending_handoff_finalizer() {
        let error = expand_planner_timeline_plan(timeline_plan(vec![
            TimelineStep::PlayAction(action_step("exit")),
            TimelineStep::PlayAction(action_step_with_finalizer("loop", "exit")),
        ]))
        .expect_err("backward finalizer should fail");

        assert!(error
            .to_string()
            .contains("pending handoff finalizer must be a later step: loop -> exit"));
    }

    #[test]
    fn planner_expansion_rejects_non_action_pending_handoff_finalizer() {
        let error = expand_planner_timeline_plan(timeline_plan(vec![
            TimelineStep::PlayAction(action_step_with_finalizer("enter", "wait")),
            TimelineStep::Wait(WaitStep::new("wait", 100, 100)),
        ]))
        .expect_err("non-action finalizer should fail");

        assert!(error
            .to_string()
            .contains("pending handoff finalizer must be a playAction step: enter -> wait"));
    }

    #[test]
    fn repeat_expansion_rewrites_pending_handoff_finalizer_references() {
        let expanded =
            expand_planner_timeline_plan(timeline_plan(vec![TimelineStep::Repeat(RepeatStep {
                step_id: "repeat".to_owned(),
                kind: "repeat".to_owned(),
                times: 2,
                steps: vec![
                    TimelineStep::PlayAction(action_step_with_finalizer("enter", "exit")),
                    TimelineStep::PlayAction(action_step("exit")),
                ],
            })]))
            .expect("repeat finalizer references should expand");
        let expanded_ids = expanded
            .steps
            .iter()
            .map(|step| {
                (
                    step.step_id().to_owned(),
                    step.pending_handoff_finalizer_step_id().map(str::to_owned),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            expanded_ids,
            vec![
                (
                    "enter__repeat_001".to_owned(),
                    Some("exit__repeat_001".to_owned())
                ),
                ("exit__repeat_001".to_owned(), None),
                (
                    "enter__repeat_002".to_owned(),
                    Some("exit__repeat_002".to_owned())
                ),
                ("exit__repeat_002".to_owned(), None),
            ]
        );
    }
}
