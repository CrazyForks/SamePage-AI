use rusqlite::{params, types::Type, Connection, OptionalExtension};

use crate::{
    choreography::action_log::ActionLogSystemEvent,
    error::{BuddyError, BuddyResult},
    local_log::LocalLogTimestamp,
};

use super::BuddyStorage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChoreographyPendingExecutionBodyKind {
    Timeline,
    DevFixture,
}

impl ChoreographyPendingExecutionBodyKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Timeline => "timeline",
            Self::DevFixture => "devFixture",
        }
    }

    pub(crate) fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "timeline" => Ok(Self::Timeline),
            "devFixture" => Ok(Self::DevFixture),
            _ => Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                Type::Text,
                format!("invalid choreography pending execution body kind: {value}").into(),
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UpsertChoreographyPendingExecutionBodyRequest {
    pub plan_id: String,
    pub body_kind: ChoreographyPendingExecutionBodyKind,
    pub schema_version: u16,
    pub body: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChoreographyPendingExecutionBody {
    pub plan_id: String,
    pub body_kind: ChoreographyPendingExecutionBodyKind,
    pub schema_version: u16,
    pub body: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

impl BuddyStorage {
    pub fn upsert_choreography_pending_execution_body(
        &self,
        request: UpsertChoreographyPendingExecutionBodyRequest,
    ) -> BuddyResult<ChoreographyPendingExecutionBody> {
        let request = normalize_upsert_choreography_pending_execution_body_request(request)?;
        self.append_choreography_action_log_system_event(
            &ActionLogSystemEvent::choreography_scheduler_pending_body_stored(
                format!("evt_{}", uuid::Uuid::now_v7()),
                request.plan_id.as_str(),
                request.body_kind.as_str(),
                request.schema_version,
                &request.body,
                LocalLogTimestamp::now_utc().to_rfc3339_millis(),
            ),
        )?;

        self.find_choreography_pending_execution_body(request.plan_id.as_str())?
            .ok_or_else(|| BuddyError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn find_choreography_pending_execution_body(
        &self,
        plan_id: &str,
    ) -> BuddyResult<Option<ChoreographyPendingExecutionBody>> {
        self.with_connection("find_choreography_pending_execution_body", |connection| {
            self::find_choreography_pending_execution_body(connection, plan_id)
        })
    }

    pub fn delete_choreography_pending_execution_body(&self, plan_id: &str) -> BuddyResult<bool> {
        let Some(existing_body) = self.find_choreography_pending_execution_body(plan_id)? else {
            return Ok(false);
        };
        self.append_choreography_action_log_system_event(
            &ActionLogSystemEvent::choreography_scheduler_pending_body_deleted(
                format!("evt_{}", uuid::Uuid::now_v7()),
                existing_body.plan_id.as_str(),
                existing_body.body_kind.as_str(),
                existing_body.schema_version,
                LocalLogTimestamp::now_utc().to_rfc3339_millis(),
            ),
        )?;

        Ok(true)
    }

    pub fn clear_choreography_pending_execution_bodies(&self) -> BuddyResult<usize> {
        self.with_connection(
            "clear_choreography_pending_execution_bodies",
            self::clear_choreography_pending_execution_bodies,
        )
    }
}

pub fn upsert_choreography_pending_execution_body(
    connection: &Connection,
    request: UpsertChoreographyPendingExecutionBodyRequest,
) -> BuddyResult<ChoreographyPendingExecutionBody> {
    let plan_id = normalize_plan_id(&request.plan_id)?;
    if request.schema_version == 0 {
        return Err(BuddyError::Validation(
            "pending execution body schema version must be positive".to_owned(),
        ));
    }

    let body_json = serde_json::to_string(&request.body)?;
    connection.execute(
        r#"
        INSERT INTO choreography_pending_execution_bodies(
          plan_id,
          body_kind,
          schema_version,
          body_json
        )
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(plan_id) DO UPDATE SET
          body_kind = excluded.body_kind,
          schema_version = excluded.schema_version,
          body_json = excluded.body_json,
          updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
        params![
            plan_id,
            request.body_kind.as_str(),
            request.schema_version,
            body_json
        ],
    )?;

    find_choreography_pending_execution_body(connection, &plan_id)?
        .ok_or_else(|| BuddyError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
}

pub fn find_choreography_pending_execution_body(
    connection: &Connection,
    plan_id: &str,
) -> BuddyResult<Option<ChoreographyPendingExecutionBody>> {
    let Some(plan_id) = normalize_plan_id_for_lookup(plan_id) else {
        return Ok(None);
    };

    connection
        .query_row(
            r#"
            SELECT plan_id, body_kind, schema_version, body_json, created_at, updated_at
            FROM choreography_pending_execution_bodies
            WHERE plan_id = ?1
            "#,
            params![plan_id],
            map_choreography_pending_execution_body,
        )
        .optional()
        .map_err(Into::into)
}

pub fn delete_choreography_pending_execution_body(
    connection: &Connection,
    plan_id: &str,
) -> BuddyResult<bool> {
    let Some(plan_id) = normalize_plan_id_for_lookup(plan_id) else {
        return Ok(false);
    };

    let changed = connection.execute(
        "DELETE FROM choreography_pending_execution_bodies WHERE plan_id = ?1",
        params![plan_id],
    )?;
    Ok(changed > 0)
}

pub fn clear_choreography_pending_execution_bodies(connection: &Connection) -> BuddyResult<usize> {
    Ok(connection.execute("DELETE FROM choreography_pending_execution_bodies", [])?)
}

fn normalize_upsert_choreography_pending_execution_body_request(
    request: UpsertChoreographyPendingExecutionBodyRequest,
) -> BuddyResult<UpsertChoreographyPendingExecutionBodyRequest> {
    let plan_id = normalize_plan_id(&request.plan_id)?;
    if request.schema_version == 0 {
        return Err(BuddyError::Validation(
            "pending execution body schema version must be positive".to_owned(),
        ));
    }

    Ok(UpsertChoreographyPendingExecutionBodyRequest {
        plan_id,
        body_kind: request.body_kind,
        schema_version: request.schema_version,
        body: request.body,
    })
}

fn map_choreography_pending_execution_body(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ChoreographyPendingExecutionBody> {
    let body_kind_value: String = row.get(1)?;
    let schema_version: u16 = row.get(2)?;
    let body_json: String = row.get(3)?;
    let body = serde_json::from_str(&body_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
    })?;

    Ok(ChoreographyPendingExecutionBody {
        plan_id: row.get(0)?,
        body_kind: ChoreographyPendingExecutionBodyKind::parse(&body_kind_value)?,
        schema_version,
        body,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn normalize_plan_id(plan_id: &str) -> BuddyResult<String> {
    let normalized = plan_id.trim();
    if normalized.is_empty() {
        return Err(BuddyError::Validation(
            "pending execution body plan id is required".to_owned(),
        ));
    }

    Ok(normalized.to_owned())
}

fn normalize_plan_id_for_lookup(plan_id: &str) -> Option<String> {
    let normalized = plan_id.trim();
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_owned())
}
