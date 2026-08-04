use serde::{Deserialize, Serialize};

use crate::native_pet::step_protocol::SidecarInterruptPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ChoreographyPlanPriority {
    IdleAutonomous,
    AiChoreography,
    UserRequested,
    SystemRecovery,
    AttentionSystem,
    CriticalInteraction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum ChoreographyTriggerSource {
    IdleAutonomous,
    AiChoreography,
    UserRequested,
    AttentionSystem,
    SystemRecovery,
    CriticalInteraction,
}

impl ChoreographyTriggerSource {
    pub(crate) fn priority(self) -> ChoreographyPlanPriority {
        match self {
            Self::IdleAutonomous => ChoreographyPlanPriority::IdleAutonomous,
            Self::AiChoreography => ChoreographyPlanPriority::AiChoreography,
            Self::UserRequested => ChoreographyPlanPriority::UserRequested,
            Self::AttentionSystem => ChoreographyPlanPriority::AttentionSystem,
            Self::SystemRecovery => ChoreographyPlanPriority::SystemRecovery,
            Self::CriticalInteraction => ChoreographyPlanPriority::CriticalInteraction,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn action_log_value(self) -> &'static str {
        match self {
            Self::IdleAutonomous => "idleAutonomous",
            Self::AiChoreography => "aiChoreography",
            Self::UserRequested => "userRequested",
            Self::AttentionSystem => "attentionSystem",
            Self::SystemRecovery => "systemRecovery",
            Self::CriticalInteraction => "criticalInteraction",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChoreographyAdmissionRequest {
    plan_id: String,
    trigger_source: ChoreographyTriggerSource,
    active_step: Option<ActiveChoreographyStep>,
}

impl ChoreographyAdmissionRequest {
    pub(crate) fn new(
        plan_id: impl Into<String>,
        trigger_source: ChoreographyTriggerSource,
    ) -> Self {
        Self {
            plan_id: plan_id.into(),
            trigger_source,
            active_step: None,
        }
    }

    pub(crate) fn with_active_step(
        mut self,
        step_id: impl Into<String>,
        interrupt_policy: SidecarInterruptPolicy,
    ) -> Self {
        self.active_step = Some(ActiveChoreographyStep {
            step_id: step_id.into(),
            interrupt_policy,
        });
        self
    }

    fn priority(&self) -> ChoreographyPlanPriority {
        self.trigger_source.priority()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveChoreographyStep {
    step_id: String,
    interrupt_policy: SidecarInterruptPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveChoreographyPlan {
    plan_id: String,
    priority: ChoreographyPlanPriority,
    active_step: Option<ActiveChoreographyStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingChoreographyPlan {
    plan_id: String,
    trigger_source: ChoreographyTriggerSource,
    priority: ChoreographyPlanPriority,
    active_step: Option<ActiveChoreographyStep>,
}

impl PendingChoreographyPlan {
    fn from_request(
        request: &ChoreographyAdmissionRequest,
        priority: ChoreographyPlanPriority,
    ) -> Self {
        Self {
            plan_id: request.plan_id.clone(),
            trigger_source: request.trigger_source,
            priority,
            active_step: request.active_step.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ChoreographyAdmissionState {
    active_plan: Option<ActiveChoreographyPlan>,
    pending_plans: Vec<PendingChoreographyPlan>,
}

impl ChoreographyAdmissionState {
    pub(crate) fn admit(
        &mut self,
        request: ChoreographyAdmissionRequest,
    ) -> ChoreographyAdmissionDecision {
        let priority = request.priority();
        let Some(active_plan) = self.active_plan.as_ref() else {
            self.active_plan = Some(ActiveChoreographyPlan {
                plan_id: request.plan_id.clone(),
                priority,
                active_step: request.active_step.clone(),
            });
            return ChoreographyAdmissionDecision::Accepted {
                plan_id: request.plan_id,
                trigger_source: request.trigger_source,
                priority,
            };
        };

        if priority > active_plan.priority {
            if let Some(active_step) = active_plan
                .active_step
                .as_ref()
                .filter(|step| !step.interrupt_policy.accepts_interrupt())
            {
                let active_plan_id = active_plan.plan_id.clone();
                let active_step_id = active_step.step_id.clone();
                let active_priority = active_plan.priority;
                let active_step_interrupt_policy = active_step.interrupt_policy;
                self.queue_pending_plan(PendingChoreographyPlan::from_request(&request, priority));
                return ChoreographyAdmissionDecision::Deferred {
                    plan_id: request.plan_id,
                    trigger_source: request.trigger_source,
                    priority,
                    active_plan_id,
                    active_step_id: Some(active_step_id),
                    active_priority,
                    active_step_interrupt_policy,
                    reason_code: "admission.waitingForActiveStepToFinish".to_owned(),
                };
            }

            let interrupted_plan = active_plan.clone();
            self.active_plan = Some(ActiveChoreographyPlan {
                plan_id: request.plan_id.clone(),
                priority,
                active_step: request.active_step.clone(),
            });
            return ChoreographyAdmissionDecision::Preempted {
                plan_id: request.plan_id,
                trigger_source: request.trigger_source,
                priority,
                interrupted_plan_id: interrupted_plan.plan_id,
                interrupted_step_id: interrupted_plan
                    .active_step
                    .map(|active_step| active_step.step_id),
                interrupted_priority: interrupted_plan.priority,
                reason_code: "admission.preemptedByHigherPriorityPlan".to_owned(),
            };
        }

        if priority == active_plan.priority {
            return ChoreographyAdmissionDecision::Rejected {
                plan_id: request.plan_id,
                trigger_source: request.trigger_source,
                priority,
                active_plan_id: active_plan.plan_id.clone(),
                active_priority: active_plan.priority,
                reason_code: "executor.busy".to_owned(),
            };
        }

        ChoreographyAdmissionDecision::Skipped {
            plan_id: request.plan_id,
            trigger_source: request.trigger_source,
            priority,
            active_plan_id: active_plan.plan_id.clone(),
            active_priority: active_plan.priority,
            reason_code: "priority.tooLow".to_owned(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn active_plan_id(&self) -> Option<&str> {
        self.active_plan
            .as_ref()
            .map(|active_plan| active_plan.plan_id.as_str())
    }

    pub(crate) fn next_pending_plan_id(&self) -> Option<&str> {
        self.next_pending_plan_index()
            .map(|index| self.pending_plans[index].plan_id.as_str())
    }

    pub(crate) fn discard_pending_plan(&mut self, plan_id: &str) -> bool {
        let previous_len = self.pending_plans.len();
        self.pending_plans.retain(|plan| plan.plan_id != plan_id);
        self.pending_plans.len() != previous_len
    }

    #[cfg(test)]
    pub(crate) fn update_active_step(
        &mut self,
        plan_id: &str,
        step_id: impl Into<String>,
    ) -> ChoreographyActiveStepUpdate {
        self.update_active_step_with_policy(plan_id, step_id, SidecarInterruptPolicy::Interruptible)
    }

    pub(crate) fn update_active_step_with_policy(
        &mut self,
        plan_id: &str,
        step_id: impl Into<String>,
        interrupt_policy: SidecarInterruptPolicy,
    ) -> ChoreographyActiveStepUpdate {
        let Some(active_plan) = self.active_plan.as_mut() else {
            return ChoreographyActiveStepUpdate::NoActivePlan {
                plan_id: plan_id.to_owned(),
            };
        };

        if active_plan.plan_id != plan_id {
            return ChoreographyActiveStepUpdate::Stale {
                plan_id: plan_id.to_owned(),
                active_plan_id: active_plan.plan_id.clone(),
            };
        }

        let step_id = step_id.into();
        active_plan.active_step = Some(ActiveChoreographyStep {
            step_id: step_id.clone(),
            interrupt_policy,
        });
        ChoreographyActiveStepUpdate::Updated {
            plan_id: plan_id.to_owned(),
            step_id,
        }
    }

    pub(crate) fn release_plan(&mut self, plan_id: &str) -> ChoreographyAdmissionRelease {
        let release = self.release_plan_preserving_pending(plan_id);
        if !matches!(release, ChoreographyAdmissionRelease::Released { .. }) {
            return release;
        }

        self.release_next_pending_plan_if_idle(plan_id)
    }

    pub(crate) fn release_plan_preserving_pending(
        &mut self,
        plan_id: &str,
    ) -> ChoreographyAdmissionRelease {
        let Some(active_plan) = self.active_plan.as_ref() else {
            return ChoreographyAdmissionRelease::NoActivePlan {
                plan_id: plan_id.to_owned(),
            };
        };

        if active_plan.plan_id != plan_id {
            return ChoreographyAdmissionRelease::Stale {
                plan_id: plan_id.to_owned(),
                active_plan_id: active_plan.plan_id.clone(),
            };
        }

        self.active_plan = None;
        ChoreographyAdmissionRelease::Released {
            plan_id: plan_id.to_owned(),
        }
    }

    pub(crate) fn release_next_pending_plan_if_idle(
        &mut self,
        released_plan_id: &str,
    ) -> ChoreographyAdmissionRelease {
        if let Some(active_plan) = self.active_plan.as_ref() {
            return ChoreographyAdmissionRelease::Stale {
                plan_id: released_plan_id.to_owned(),
                active_plan_id: active_plan.plan_id.clone(),
            };
        }

        if let Some(pending_plan) = self.pop_next_pending_plan() {
            return ChoreographyAdmissionRelease::ReleasedWithPending {
                plan_id: released_plan_id.to_owned(),
                pending_plan_id: pending_plan.plan_id,
                pending_trigger_source: pending_plan.trigger_source,
                pending_priority: pending_plan.priority,
                pending_active_step_id: pending_plan
                    .active_step
                    .as_ref()
                    .map(|active_step| active_step.step_id.clone()),
                pending_active_step_interrupt_policy: pending_plan
                    .active_step
                    .map(|active_step| active_step.interrupt_policy),
            };
        }

        ChoreographyAdmissionRelease::Released {
            plan_id: released_plan_id.to_owned(),
        }
    }

    fn queue_pending_plan(&mut self, pending_plan: PendingChoreographyPlan) {
        self.pending_plans
            .retain(|plan| plan.plan_id != pending_plan.plan_id);
        self.pending_plans.push(pending_plan);
    }

    fn pop_next_pending_plan(&mut self) -> Option<PendingChoreographyPlan> {
        self.next_pending_plan_index()
            .map(|index| self.pending_plans.remove(index))
    }

    fn next_pending_plan_index(&self) -> Option<usize> {
        let mut next_index = None;
        for (index, plan) in self.pending_plans.iter().enumerate() {
            let Some(current_index) = next_index else {
                next_index = Some(index);
                continue;
            };
            if plan.priority > self.pending_plans[current_index].priority {
                next_index = Some(index);
            }
        }

        next_index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChoreographyActiveStepUpdate {
    Updated {
        plan_id: String,
        step_id: String,
    },
    Stale {
        plan_id: String,
        active_plan_id: String,
    },
    NoActivePlan {
        plan_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChoreographyAdmissionRelease {
    Released {
        plan_id: String,
    },
    ReleasedWithPending {
        plan_id: String,
        pending_plan_id: String,
        pending_trigger_source: ChoreographyTriggerSource,
        pending_priority: ChoreographyPlanPriority,
        pending_active_step_id: Option<String>,
        pending_active_step_interrupt_policy: Option<SidecarInterruptPolicy>,
    },
    Stale {
        plan_id: String,
        active_plan_id: String,
    },
    NoActivePlan {
        plan_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChoreographyAdmissionDecision {
    Accepted {
        plan_id: String,
        trigger_source: ChoreographyTriggerSource,
        priority: ChoreographyPlanPriority,
    },
    Preempted {
        plan_id: String,
        trigger_source: ChoreographyTriggerSource,
        priority: ChoreographyPlanPriority,
        interrupted_plan_id: String,
        interrupted_step_id: Option<String>,
        interrupted_priority: ChoreographyPlanPriority,
        reason_code: String,
    },
    Rejected {
        plan_id: String,
        trigger_source: ChoreographyTriggerSource,
        priority: ChoreographyPlanPriority,
        active_plan_id: String,
        active_priority: ChoreographyPlanPriority,
        reason_code: String,
    },
    Deferred {
        plan_id: String,
        trigger_source: ChoreographyTriggerSource,
        priority: ChoreographyPlanPriority,
        active_plan_id: String,
        active_step_id: Option<String>,
        active_priority: ChoreographyPlanPriority,
        active_step_interrupt_policy: SidecarInterruptPolicy,
        reason_code: String,
    },
    Skipped {
        plan_id: String,
        trigger_source: ChoreographyTriggerSource,
        priority: ChoreographyPlanPriority,
        active_plan_id: String,
        active_priority: ChoreographyPlanPriority,
        reason_code: String,
    },
}

impl ChoreographyAdmissionDecision {
    pub(crate) fn action_log_payload(&self) -> serde_json::Value {
        match self {
            Self::Accepted {
                plan_id,
                trigger_source,
                priority,
            } => serde_json::json!({
                "decision": "accepted",
                "planId": plan_id,
                "triggerSource": trigger_source,
                "priority": priority,
            }),
            Self::Preempted {
                plan_id,
                trigger_source,
                priority,
                interrupted_plan_id,
                interrupted_step_id,
                interrupted_priority,
                reason_code,
            } => {
                let interrupted_plan = match interrupted_step_id {
                    Some(step_id) => serde_json::json!({
                        "planId": interrupted_plan_id,
                        "stepId": step_id,
                        "priority": interrupted_priority,
                    }),
                    None => serde_json::json!({
                        "planId": interrupted_plan_id,
                        "priority": interrupted_priority,
                    }),
                };
                serde_json::json!({
                    "decision": "preempted",
                    "planId": plan_id,
                    "triggerSource": trigger_source,
                    "priority": priority,
                    "reasonCode": reason_code,
                    "interruptedPlan": interrupted_plan,
                })
            }
            Self::Deferred {
                plan_id,
                trigger_source,
                priority,
                active_plan_id,
                active_step_id,
                active_priority,
                active_step_interrupt_policy,
                reason_code,
            } => deferred_decision_payload(DeferredDecisionPayload {
                plan_id,
                trigger_source: *trigger_source,
                priority: *priority,
                reason_code,
                active_plan_id,
                active_step_id: active_step_id.as_deref(),
                active_priority: *active_priority,
                active_step_interrupt_policy: *active_step_interrupt_policy,
            }),
            Self::Rejected {
                plan_id,
                trigger_source,
                priority,
                active_plan_id,
                active_priority,
                reason_code,
            } => decision_with_active_plan_payload(
                "rejected",
                plan_id,
                *trigger_source,
                *priority,
                reason_code,
                active_plan_id,
                *active_priority,
            ),
            Self::Skipped {
                plan_id,
                trigger_source,
                priority,
                active_plan_id,
                active_priority,
                reason_code,
            } => decision_with_active_plan_payload(
                "skipped",
                plan_id,
                *trigger_source,
                *priority,
                reason_code,
                active_plan_id,
                *active_priority,
            ),
        }
    }
}

fn decision_with_active_plan_payload(
    decision: &str,
    plan_id: &str,
    trigger_source: ChoreographyTriggerSource,
    priority: ChoreographyPlanPriority,
    reason_code: &str,
    active_plan_id: &str,
    active_priority: ChoreographyPlanPriority,
) -> serde_json::Value {
    serde_json::json!({
        "decision": decision,
        "planId": plan_id,
        "triggerSource": trigger_source,
        "priority": priority,
        "reasonCode": reason_code,
        "activePlan": {
            "planId": active_plan_id,
            "priority": active_priority,
        },
    })
}

struct DeferredDecisionPayload<'a> {
    plan_id: &'a str,
    trigger_source: ChoreographyTriggerSource,
    priority: ChoreographyPlanPriority,
    reason_code: &'a str,
    active_plan_id: &'a str,
    active_step_id: Option<&'a str>,
    active_priority: ChoreographyPlanPriority,
    active_step_interrupt_policy: SidecarInterruptPolicy,
}

fn deferred_decision_payload(payload: DeferredDecisionPayload<'_>) -> serde_json::Value {
    let active_plan = match payload.active_step_id {
        Some(step_id) => serde_json::json!({
            "planId": payload.active_plan_id,
            "stepId": step_id,
            "priority": payload.active_priority,
            "interruptPolicy": payload.active_step_interrupt_policy,
        }),
        None => serde_json::json!({
            "planId": payload.active_plan_id,
            "priority": payload.active_priority,
            "interruptPolicy": payload.active_step_interrupt_policy,
        }),
    };

    serde_json::json!({
        "decision": "deferred",
        "planId": payload.plan_id,
        "triggerSource": payload.trigger_source,
        "priority": payload.priority,
        "reasonCode": payload.reason_code,
        "activePlan": active_plan,
    })
}
