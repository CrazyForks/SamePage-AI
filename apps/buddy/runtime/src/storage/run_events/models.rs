use crate::domain::BuddyRunEventType;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBuddyRunEventRequest {
    pub(super) run_id: String,
    pub(super) event_type: String,
    pub(super) payload: serde_json::Value,
}

impl CreateBuddyRunEventRequest {
    pub(crate) fn new(
        run_id: impl Into<String>,
        event_type: BuddyRunEventType,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            event_type: event_type.as_str().to_owned(),
            payload,
        }
    }

    pub(crate) fn projected(
        run_id: impl Into<String>,
        event_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            event_type: event_type.into(),
            payload,
        }
    }

    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(crate) fn event_type(&self) -> &str {
        &self.event_type
    }

    pub(crate) fn payload(&self) -> &serde_json::Value {
        &self.payload
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyRunEvent {
    pub id: i64,
    pub run_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyChatRunEvent {
    pub id: i64,
    pub run_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyRunEventCount {
    pub run_id: String,
    pub event_count: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyRunEventSummary {
    pub id: i64,
    pub run_id: String,
    pub event_type: String,
    pub payload_preview: String,
    pub payload_chars: i64,
    pub created_at: String,
}
