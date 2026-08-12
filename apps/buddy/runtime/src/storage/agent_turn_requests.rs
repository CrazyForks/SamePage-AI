use std::sync::MutexGuard;

use rusqlite::{params, types::Type, OptionalExtension};

use crate::error::{BuddyError, BuddyResult};

use super::BuddyStorage;

impl BuddyStorage {
    pub(crate) fn lock_agent_turn_request_preparation(&self) -> BuddyResult<MutexGuard<'_, ()>> {
        self.agent_turn_request_coordinator.lock().map_err(|_| {
            BuddyError::Runtime("agent turn request coordinator lock was poisoned".to_owned())
        })
    }

    pub fn find_agent_turn_response(
        &self,
        request_id: &str,
    ) -> BuddyResult<Option<serde_json::Value>> {
        self.with_connection("find_agent_turn_response", |connection| {
            let response_json = connection
                .query_row(
                    r#"
                    SELECT response_json
                    FROM agent_turn_requests
                    WHERE request_id = ?1
                    "#,
                    params![request_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            response_json
                .map(|response_json| {
                    serde_json::from_str(&response_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                            .into()
                    })
                })
                .transpose()
        })
    }

    pub fn store_agent_turn_response(
        &self,
        request_id: &str,
        conversation_id: &str,
        response: &serde_json::Value,
    ) -> BuddyResult<serde_json::Value> {
        self.with_connection("store_agent_turn_response", |connection| {
            let response_json = serde_json::to_string(response)?;
            connection.execute(
                r#"
                INSERT INTO agent_turn_requests(request_id, conversation_id, response_json)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(request_id) DO NOTHING
                "#,
                params![request_id, conversation_id, response_json],
            )?;

            let stored_json: String = connection.query_row(
                r#"
                SELECT response_json
                FROM agent_turn_requests
                WHERE request_id = ?1
                "#,
                params![request_id],
                |row| row.get(0),
            )?;
            serde_json::from_str(&stored_json).map_err(Into::into)
        })
    }
}
