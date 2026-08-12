use std::fs;

use crate::{
    error::{BuddyError, BuddyResult},
    state::BuddyAppState,
    storage::BuddyMessageAttachment,
};

mod builtin_skill;
mod codex_inputs;
mod content;

pub(super) use builtin_skill::{create_buddy_builtin_host_skill_input, TrustedCodexSkillInput};
pub(super) use codex_inputs::compose_buddy_chat_codex_inputs;
pub(super) use content::{
    compose_buddy_chat_runtime_content, compose_buddy_chat_user_message_content,
    compose_buddy_runtime_instructions,
};

const BUDDY_CHAT_ATTACHMENT_MAX_BYTES: u64 = 8 * 1024 * 1024;
const BUDDY_CHAT_ATTACHMENT_COUNT_LIMIT: usize = 16;
const BUDDY_CHAT_ATTACHMENT_TOTAL_MAX_BYTES: u64 = 32 * 1024 * 1024;
pub(super) const BUDDY_CHAT_IMAGE_MAX_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const BUDDY_CHAT_IMAGE_MIME_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Clone, Debug)]
pub(crate) struct BuddyChatAttachment {
    pub(crate) attachment_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) data_url: Option<String>,
    pub(crate) preview_path: Option<String>,
    pub(crate) text: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BuddyChatAttachmentRequest {
    pub(crate) attachment_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BuddyChatPromptContextItem {
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) path: Option<String>,
    pub(crate) description: Option<String>,
}

pub(super) fn create_buddy_message_attachments(
    attachments: &[BuddyChatAttachment],
) -> Vec<BuddyMessageAttachment> {
    attachments
        .iter()
        .map(|attachment| BuddyMessageAttachment {
            attachment_id: attachment.attachment_id.clone(),
            data_url: if attachment.kind == "image" {
                attachment
                    .preview_path
                    .is_none()
                    .then(|| attachment.data_url.clone())
                    .flatten()
            } else {
                None
            },
            kind: attachment.kind.clone(),
            mime_type: attachment.mime_type.clone(),
            name: attachment.name.clone(),
            preview_path: attachment.preview_path.clone(),
            size_bytes: attachment.size_bytes,
        })
        .collect()
}

pub(super) fn materialize_buddy_chat_attachments(
    state: &BuddyAppState,
    attachments: Vec<BuddyChatAttachmentRequest>,
) -> BuddyResult<Vec<BuddyChatAttachment>> {
    if attachments.len() > BUDDY_CHAT_ATTACHMENT_COUNT_LIMIT {
        return Err(BuddyError::Validation(
            "a turn supports at most 16 attachments".to_owned(),
        ));
    }

    let mut total_bytes = 0_u64;
    let mut materialized = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let attachment = materialize_buddy_chat_attachment(state, attachment)?;
        total_bytes = total_bytes
            .checked_add(attachment.size_bytes)
            .ok_or_else(|| {
                BuddyError::Validation("attachment byte budget overflowed".to_owned())
            })?;
        if total_bytes > BUDDY_CHAT_ATTACHMENT_TOTAL_MAX_BYTES {
            return Err(BuddyError::Validation(
                "turn attachments must not exceed 32 MiB".to_owned(),
            ));
        }
        materialized.push(attachment);
    }

    Ok(materialized)
}

fn materialize_buddy_chat_attachment(
    state: &BuddyAppState,
    attachment: BuddyChatAttachmentRequest,
) -> BuddyResult<BuddyChatAttachment> {
    let attachment_id = attachment.attachment_id.trim();
    if attachment_id.is_empty() {
        return Err(BuddyError::Validation(
            "attachmentId is required".to_owned(),
        ));
    }

    let registered = state.find_attachment(attachment_id)?.ok_or_else(|| {
        BuddyError::Validation(format!("registered attachment not found: {attachment_id}"))
    })?;
    let attachments_dir = fs::canonicalize(state.attachments_dir_path())?;
    let registered_path = fs::canonicalize(&registered.path)?;
    if !registered_path.starts_with(&attachments_dir) {
        return Err(BuddyError::Validation(
            "registered attachment is outside the managed directory".to_owned(),
        ));
    }
    let metadata = fs::metadata(&registered_path)?;
    if !metadata.is_file() || metadata.len() > BUDDY_CHAT_ATTACHMENT_MAX_BYTES {
        return Err(BuddyError::Validation(
            "registered attachment is not a supported file".to_owned(),
        ));
    }
    if metadata.len() != registered.size_bytes {
        return Err(BuddyError::Validation(
            "registered attachment size no longer matches its record".to_owned(),
        ));
    }
    if registered.kind == "image"
        && (!BUDDY_CHAT_IMAGE_MIME_TYPES.contains(&registered.mime_type.as_str())
            || metadata.len() > BUDDY_CHAT_IMAGE_MAX_BYTES)
    {
        return Err(BuddyError::Validation(
            "registered image is not supported by the runtime".to_owned(),
        ));
    }

    let preview_path = if registered.kind == "image" {
        Some(registered_path.to_string_lossy().into_owned())
    } else {
        None
    };
    let text = if registered.kind == "text" {
        Some(fs::read_to_string(&registered_path)?)
    } else {
        None
    };

    Ok(BuddyChatAttachment {
        attachment_id: Some(registered.id),
        data_url: None,
        kind: registered.kind,
        mime_type: registered.mime_type,
        name: registered.name,
        preview_path,
        size_bytes: metadata.len(),
        text,
    })
}

#[cfg(test)]
mod tests;
