use std::{collections::HashSet, fs, path::Path};

use rusqlite::{params, types::Type, OptionalExtension};

use crate::error::{BuddyError, BuddyResult};

use super::BuddyStorage;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyRegisteredAttachment {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub path: String,
    pub source: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct CreateBuddyRegisteredAttachmentRequest {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub path: String,
    pub source: String,
}

impl BuddyStorage {
    pub fn create_attachment(
        &self,
        request: CreateBuddyRegisteredAttachmentRequest,
    ) -> BuddyResult<BuddyRegisteredAttachment> {
        self.with_connection("create_attachment", |connection| {
            self::create_attachment(connection, request)
        })
    }

    pub fn find_attachment(&self, id: &str) -> BuddyResult<Option<BuddyRegisteredAttachment>> {
        self.with_connection("find_attachment", |connection| {
            self::find_attachment(connection, id)
        })
    }

    pub fn release_unreferenced_attachment(
        &self,
        id: &str,
    ) -> BuddyResult<Option<BuddyRegisteredAttachment>> {
        self.with_mut_connection("release_unreferenced_attachment", |connection| {
            let transaction = connection.transaction()?;
            let attachment = find_attachment(&transaction, id)?;
            let Some(attachment) = attachment else {
                return Ok(None);
            };
            if is_attachment_referenced(&transaction, id)? {
                return Ok(None);
            }

            transaction.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
            transaction.commit()?;
            Ok(Some(attachment))
        })
    }

    pub fn release_unreferenced_attachment_file(
        &self,
        attachments_dir: &Path,
        id: &str,
    ) -> BuddyResult<Option<BuddyRegisteredAttachment>> {
        let Some(attachment) = self.find_attachment(id)? else {
            return Ok(None);
        };
        validate_managed_attachment_path(attachments_dir, Path::new(&attachment.path))?;
        let Some(released) = self.release_unreferenced_attachment(id)? else {
            return Ok(None);
        };

        if let Err(remove_error) = remove_attachment_file(Path::new(&released.path)) {
            if let Err(restore_error) = self.restore_attachment(&released) {
                return Err(BuddyError::Runtime(format!(
                    "{remove_error}; failed to restore attachment record: {restore_error}"
                )));
            }
            return Err(remove_error);
        }

        Ok(Some(released))
    }

    fn restore_attachment(&self, attachment: &BuddyRegisteredAttachment) -> BuddyResult<()> {
        self.with_connection("restore_attachment", |connection| {
            connection.execute(
                r#"
                INSERT INTO attachments(
                  id, kind, name, mime_type, size_bytes, path, source, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    attachment.id,
                    attachment.kind,
                    attachment.name,
                    attachment.mime_type,
                    i64::try_from(attachment.size_bytes).map_err(|_| {
                        BuddyError::Validation("attachment size is too large".to_owned())
                    })?,
                    attachment.path,
                    attachment.source,
                    attachment.created_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn cleanup_unreferenced_attachments_except(
        &self,
        attachments_dir: &Path,
        retained_attachment_ids: &[String],
    ) -> BuddyResult<Vec<String>> {
        let retained_attachment_ids = retained_attachment_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let attachments = self.with_connection("list_unreferenced_attachments", |connection| {
            list_unreferenced_attachments(connection)
        })?;
        let mut released_attachment_ids = Vec::new();
        for attachment in attachments {
            if retained_attachment_ids.contains(attachment.id.as_str()) {
                continue;
            }
            let Some(released) =
                self.release_unreferenced_attachment_file(attachments_dir, &attachment.id)?
            else {
                continue;
            };
            released_attachment_ids.push(released.id);
        }

        Ok(released_attachment_ids)
    }
}

pub fn create_attachment(
    connection: &rusqlite::Connection,
    request: CreateBuddyRegisteredAttachmentRequest,
) -> BuddyResult<BuddyRegisteredAttachment> {
    let size_bytes = i64::try_from(request.size_bytes)
        .map_err(|_| BuddyError::Validation("attachment size is too large".to_owned()))?;

    connection.execute(
        r#"
        INSERT INTO attachments(id, kind, name, mime_type, size_bytes, path, source)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            request.id,
            request.kind,
            request.name,
            request.mime_type,
            size_bytes,
            request.path,
            request.source
        ],
    )?;

    find_attachment(connection, &request.id)?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

pub fn find_attachment(
    connection: &rusqlite::Connection,
    id: &str,
) -> BuddyResult<Option<BuddyRegisteredAttachment>> {
    Ok(connection
        .query_row(
            r#"
            SELECT id, kind, name, mime_type, size_bytes, path, source, created_at
            FROM attachments
            WHERE id = ?1
            "#,
            params![id],
            map_attachment,
        )
        .optional()?)
}

fn list_unreferenced_attachments(
    connection: &rusqlite::Connection,
) -> BuddyResult<Vec<BuddyRegisteredAttachment>> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, kind, name, mime_type, size_bytes, path, source, created_at
        FROM attachments
        WHERE NOT EXISTS (
          SELECT 1
          FROM messages, json_each(messages.attachments_json) AS message_attachment
          WHERE json_extract(message_attachment.value, '$.attachmentId') = attachments.id
        )
        ORDER BY rowid ASC
        "#,
    )?;
    let attachments = statement
        .query_map([], map_attachment)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(attachments)
}

fn is_attachment_referenced(connection: &rusqlite::Connection, id: &str) -> BuddyResult<bool> {
    Ok(connection.query_row(
        r#"
        SELECT EXISTS(
          SELECT 1
          FROM messages, json_each(messages.attachments_json) AS message_attachment
          WHERE json_extract(message_attachment.value, '$.attachmentId') = ?1
        )
        "#,
        params![id],
        |row| row.get(0),
    )?)
}

fn validate_managed_attachment_path(
    attachments_dir: &Path,
    attachment_path: &Path,
) -> BuddyResult<()> {
    let attachments_dir = fs::canonicalize(attachments_dir)?;
    match fs::canonicalize(attachment_path) {
        Ok(path) if path.starts_with(&attachments_dir) => Ok(()),
        Ok(_) => Err(BuddyError::Validation(
            "attachment is outside the managed directory".to_owned(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn remove_attachment_file(path: &Path) -> BuddyResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn map_attachment(row: &rusqlite::Row<'_>) -> rusqlite::Result<BuddyRegisteredAttachment> {
    let size_bytes: i64 = row.get(4)?;
    Ok(BuddyRegisteredAttachment {
        id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        mime_type: row.get(3)?,
        size_bytes: u64::try_from(size_bytes).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Integer, Box::new(error))
        })?,
        path: row.get(5)?,
        source: row.get(6)?,
        created_at: row.get(7)?,
    })
}
