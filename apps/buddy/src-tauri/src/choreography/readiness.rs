use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ChoreographyRuntimeReadinessStatus {
    Ready,
    #[cfg_attr(not(test), allow(dead_code))]
    Degraded,
}

impl ChoreographyRuntimeReadinessStatus {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChoreographyRuntimeReadinessSnapshot {
    pub(crate) status: ChoreographyRuntimeReadinessStatus,
    pub(crate) accepting_choreography: bool,
    pub(crate) reason_code: Option<String>,
    pub(crate) degraded_at: Option<String>,
    pub(crate) recovered_at: Option<String>,
    pub(crate) updated_at: Option<String>,
}

impl ChoreographyRuntimeReadinessSnapshot {
    fn ready() -> Self {
        Self {
            status: ChoreographyRuntimeReadinessStatus::Ready,
            accepting_choreography: true,
            reason_code: None,
            degraded_at: None,
            recovered_at: None,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChoreographyRuntimeReadinessState {
    snapshot: ChoreographyRuntimeReadinessSnapshot,
}

impl Default for ChoreographyRuntimeReadinessState {
    fn default() -> Self {
        Self {
            snapshot: ChoreographyRuntimeReadinessSnapshot::ready(),
        }
    }
}

impl ChoreographyRuntimeReadinessState {
    pub(crate) fn snapshot(&self) -> ChoreographyRuntimeReadinessSnapshot {
        self.snapshot.clone()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn mark_degraded(
        &mut self,
        reason_code: impl Into<String>,
        degraded_at: impl Into<String>,
    ) -> ChoreographyRuntimeReadinessSnapshot {
        let degraded_at = degraded_at.into();
        self.snapshot = ChoreographyRuntimeReadinessSnapshot {
            status: ChoreographyRuntimeReadinessStatus::Degraded,
            accepting_choreography: false,
            reason_code: Some(reason_code.into()),
            degraded_at: Some(degraded_at.clone()),
            recovered_at: None,
            updated_at: Some(degraded_at),
        };
        self.snapshot()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn mark_ready(
        &mut self,
        recovered_at: impl Into<String>,
    ) -> ChoreographyRuntimeReadinessSnapshot {
        let recovered_at = recovered_at.into();
        self.snapshot = ChoreographyRuntimeReadinessSnapshot {
            status: ChoreographyRuntimeReadinessStatus::Ready,
            accepting_choreography: true,
            reason_code: None,
            degraded_at: None,
            recovered_at: Some(recovered_at.clone()),
            updated_at: Some(recovered_at),
        };
        self.snapshot()
    }
}
