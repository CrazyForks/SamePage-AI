use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    commands::chat_input::{BUDDY_CHAT_IMAGE_MAX_BYTES, BUDDY_CHAT_IMAGE_MIME_TYPES},
    error::{BuddyError, BuddyResult},
    state::BuddyAppState,
    storage::{BuddyRegisteredAttachment, CreateBuddyRegisteredAttachmentRequest},
};

const BUDDY_CLIPBOARD_FILE_COUNT_LIMIT: usize = 16;
const BUDDY_CLIPBOARD_FILE_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyClipboardFile {
    pub(super) attachment_id: Option<String>,
    pub(super) kind: String,
    pub(super) name: String,
    pub(super) mime_type: String,
    pub(super) size_bytes: u64,
    pub(super) data_url: Option<String>,
    pub(super) preview_path: Option<String>,
    pub(super) text: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAttachmentPreview {
    path: String,
    mime_type: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyReleasedAttachments {
    released_attachment_ids: Vec<String>,
}

pub(crate) fn resolve_buddy_attachment_preview(
    state: &BuddyAppState,
    attachment_id: &str,
) -> BuddyResult<BuddyAttachmentPreview> {
    let attachment = state
        .find_attachment(attachment_id)?
        .ok_or_else(|| BuddyError::Validation("attachment does not exist".to_owned()))?;
    if attachment.kind != "image" {
        return Err(BuddyError::Validation(
            "attachment is not an image preview".to_owned(),
        ));
    }

    let attachments_dir = fs::canonicalize(state.attachments_dir_path())?;
    let path = fs::canonicalize(&attachment.path)?;
    if !path.starts_with(&attachments_dir) {
        return Err(BuddyError::Validation(
            "attachment preview is outside the managed directory".to_owned(),
        ));
    }

    Ok(BuddyAttachmentPreview {
        path: path.to_string_lossy().into_owned(),
        mime_type: attachment.mime_type,
    })
}

pub(crate) fn release_buddy_attachments(
    state: &BuddyAppState,
    attachment_ids: Vec<String>,
) -> BuddyResult<BuddyReleasedAttachments> {
    let attachments_dir = state.attachments_dir_path();
    let storage = state.storage_handle();
    let mut released_attachment_ids = Vec::new();
    for attachment_id in attachment_ids {
        let attachment_id = attachment_id.trim();
        if attachment_id.is_empty() || released_attachment_ids.iter().any(|id| id == attachment_id)
        {
            continue;
        }
        let Some(_released) =
            storage.release_unreferenced_attachment_file(&attachments_dir, attachment_id)?
        else {
            continue;
        };
        released_attachment_ids.push(attachment_id.to_owned());
    }

    Ok(BuddyReleasedAttachments {
        released_attachment_ids,
    })
}

pub(crate) fn cleanup_buddy_draft_attachments(
    state: &BuddyAppState,
    retained_attachment_ids: Vec<String>,
) -> BuddyResult<BuddyReleasedAttachments> {
    let released_attachment_ids = state
        .storage_handle()
        .cleanup_unreferenced_attachments_except(
            &state.attachments_dir_path(),
            &retained_attachment_ids,
        )?;

    Ok(BuddyReleasedAttachments {
        released_attachment_ids,
    })
}

pub(crate) fn create_buddy_clipboard_files_from_paths(
    state: &BuddyAppState,
    paths: &[PathBuf],
    source: &'static str,
) -> BuddyResult<Vec<BuddyClipboardFile>> {
    let mut files = Vec::new();
    for path in paths.iter().take(BUDDY_CLIPBOARD_FILE_COUNT_LIMIT) {
        if let Some(file) = create_buddy_clipboard_file_from_path(state, path, source)? {
            files.push(file);
        }
    }

    Ok(files)
}

pub(super) fn create_buddy_clipboard_file_from_path(
    state: &BuddyAppState,
    path: &Path,
    source: &'static str,
) -> BuddyResult<Option<BuddyClipboardFile>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(None),
        Err(error) => return Err(BuddyError::Io(error)),
    };
    if metadata.len() > BUDDY_CLIPBOARD_FILE_MAX_BYTES {
        return Err(BuddyError::Validation(format!(
            "clipboard file is too large: {} bytes",
            metadata.len()
        )));
    }

    let mime_type = guess_buddy_clipboard_file_mime_type(path);
    let kind = if BUDDY_CHAT_IMAGE_MIME_TYPES.contains(&mime_type.as_str()) {
        if metadata.len() > BUDDY_CHAT_IMAGE_MAX_BYTES {
            return Err(BuddyError::Validation(format!(
                "image attachment is too large: {} bytes",
                metadata.len()
            )));
        }
        "image"
    } else {
        let is_text_candidate = is_buddy_text_clipboard_file(path, &mime_type);
        let text_bytes = is_text_candidate.then(|| fs::read(path)).transpose()?;
        if text_bytes
            .as_ref()
            .is_some_and(|bytes| std::str::from_utf8(bytes).is_ok())
        {
            "text"
        } else {
            "binary"
        }
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| BuddyError::Validation("clipboard file name is not valid UTF-8".to_owned()))?
        .to_owned();
    let attachment = create_buddy_registered_attachment_from_path(
        state,
        path,
        &name,
        &mime_type,
        kind,
        source,
        metadata.len(),
    )?;
    let preview_path = (kind == "image").then(|| attachment.path.clone());

    Ok(Some(BuddyClipboardFile {
        attachment_id: Some(attachment.id),
        kind: kind.to_owned(),
        name,
        mime_type,
        size_bytes: metadata.len(),
        data_url: None,
        preview_path,
        text: None,
    }))
}

fn create_buddy_registered_attachment_from_path(
    state: &BuddyAppState,
    source_path: &Path,
    name: &str,
    mime_type: &str,
    kind: &str,
    source: &str,
    size_bytes: u64,
) -> BuddyResult<BuddyRegisteredAttachment> {
    let attachment_id = uuid::Uuid::new_v4().to_string();
    let path = resolve_buddy_registered_attachment_path(
        &state.attachments_dir_path(),
        &attachment_id,
        name,
    )?;
    fs::copy(source_path, &path)?;

    finalize_copied_attachment(&path, || {
        state.create_attachment(CreateBuddyRegisteredAttachmentRequest {
            id: attachment_id,
            kind: kind.to_owned(),
            mime_type: mime_type.to_owned(),
            name: name.to_owned(),
            path: path.to_string_lossy().into_owned(),
            size_bytes,
            source: source.to_owned(),
        })
    })
}

fn finalize_copied_attachment<T>(
    path: &Path,
    register: impl FnOnce() -> BuddyResult<T>,
) -> BuddyResult<T> {
    match register() {
        Ok(attachment) => Ok(attachment),
        Err(registration_error) => match crate::storage::remove_attachment_file(path) {
            Ok(()) => Err(registration_error),
            Err(cleanup_error) => Err(BuddyError::Runtime(format!(
                "{registration_error}; failed to remove unregistered attachment file: {cleanup_error}"
            ))),
        },
    }
}

fn resolve_buddy_registered_attachment_path(
    attachments_dir: &Path,
    attachment_id: &str,
    name: &str,
) -> BuddyResult<PathBuf> {
    fs::create_dir_all(attachments_dir)?;
    Ok(attachments_dir.join(format!(
        "{}-{}",
        attachment_id,
        sanitize_buddy_attachment_file_name(name)
    )))
}

fn sanitize_buddy_attachment_file_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| match character {
            '/' | '\\' | '\0' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect::<String>();
    let sanitized =
        sanitized.trim_matches(|character: char| character.is_whitespace() || character == '.');

    if sanitized.is_empty() {
        "attachment".to_owned()
    } else {
        sanitized.to_owned()
    }
}

fn guess_buddy_clipboard_file_mime_type(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "txt" => "text/plain",
        "md" | "mdx" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" => "text/html",
        "css" => "text/css",
        "scss" => "text/x-scss",
        "ts" | "tsx" | "js" | "jsx" | "vue" | "rs" | "toml" | "yaml" | "yml" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_owned()
}

fn is_buddy_text_clipboard_file(path: &Path, mime_type: &str) -> bool {
    mime_type.starts_with("text/")
        || matches!(mime_type, "application/json" | "application/xml")
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "md" | "mdx"
                        | "txt"
                        | "csv"
                        | "ts"
                        | "tsx"
                        | "js"
                        | "jsx"
                        | "json"
                        | "vue"
                        | "rs"
                        | "toml"
                        | "yaml"
                        | "yml"
                        | "xml"
                        | "html"
                        | "css"
                        | "scss"
                )
            })
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use crate::{
        app_paths::BuddyAppPaths,
        state::BuddyAppState,
        storage::{
            AppendBuddyConversationMessageRequest, BuddyMessageAttachment,
            CreateBuddyConversationRequest,
        },
    };

    use super::{
        cleanup_buddy_draft_attachments, create_buddy_clipboard_file_from_path,
        create_buddy_clipboard_files_from_paths, finalize_copied_attachment,
        release_buddy_attachments, resolve_buddy_attachment_preview,
    };

    #[test]
    fn removes_copied_file_when_attachment_registration_fails() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let copied_path = dir.join("copied.txt");
        fs::write(&copied_path, "copied attachment").expect("write copied file");

        let error = finalize_copied_attachment(&copied_path, || {
            Err::<(), _>(crate::error::BuddyError::Runtime(
                "database unavailable".to_owned(),
            ))
        })
        .expect_err("registration must fail");

        assert!(error.to_string().contains("database unavailable"));
        assert!(!copied_path.exists());
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn restores_attachment_record_when_file_removal_fails() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        let attachment_id = uuid::Uuid::new_v4().to_string();
        let attachment_path = state.attachments_dir_path().join("not-a-file");
        fs::create_dir_all(&attachment_path).expect("create invalid attachment path");
        state
            .create_attachment(crate::storage::CreateBuddyRegisteredAttachmentRequest {
                id: attachment_id.clone(),
                kind: "binary".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                name: "not-a-file".to_owned(),
                path: attachment_path.to_string_lossy().into_owned(),
                size_bytes: 0,
                source: "file-picker".to_owned(),
            })
            .expect("register attachment");

        release_buddy_attachments(&state, vec![attachment_id.clone()])
            .expect_err("directory removal must fail");

        assert!(state
            .find_attachment(&attachment_id)
            .expect("find restored attachment")
            .is_some());
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn resolves_registered_images_only_inside_the_managed_attachment_directory() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        let source_path = dir.join("source.png");
        fs::write(&source_path, [137, 80, 78, 71]).expect("write image");

        let attachment = create_buddy_clipboard_file_from_path(&state, &source_path, "file-picker")
            .expect("register image")
            .expect("image attachment");
        let preview = resolve_buddy_attachment_preview(
            &state,
            attachment.attachment_id.as_deref().expect("attachment id"),
        )
        .expect("resolve preview");

        assert_eq!(preview.mime_type, "image/png");
        assert!(std::path::Path::new(&preview.path).starts_with(dir.join("attachments")));
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn rejects_non_image_attachment_previews() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        let source_path = dir.join("note.txt");
        fs::write(&source_path, "hello").expect("write text");

        let attachment = create_buddy_clipboard_file_from_path(&state, &source_path, "file-picker")
            .expect("register text")
            .expect("text attachment");
        let error = resolve_buddy_attachment_preview(
            &state,
            attachment.attachment_id.as_deref().expect("attachment id"),
        )
        .expect_err("text preview must fail");

        assert!(error.to_string().contains("not an image preview"));
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn rejects_oversized_clipboard_file_payloads_before_reading_bytes() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("large.txt");
        let file = fs::File::create(&path).expect("create large file");
        file.set_len(8 * 1024 * 1024 + 1)
            .expect("resize large file");

        let error = create_buddy_clipboard_file_from_path(&state, &path, "clipboard-file")
            .expect_err("large clipboard file should be rejected");

        assert!(error.to_string().contains("clipboard file is too large"));
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn limits_native_clipboard_file_payload_count() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        fs::create_dir_all(&dir).expect("create temp dir");
        let paths = (0..20)
            .map(|index| {
                let path = dir.join(format!("note-{index}.txt"));
                fs::write(&path, format!("note {index}")).expect("write text");
                path
            })
            .collect::<Vec<_>>();

        let files = create_buddy_clipboard_files_from_paths(&state, &paths, "clipboard-file")
            .expect("create payloads");

        assert_eq!(files.len(), 16);
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn unsupported_image_formats_are_registered_as_binary_files() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        let source_path = dir.join("vector.svg");
        fs::write(&source_path, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").expect("write svg");

        let attachment = create_buddy_clipboard_file_from_path(&state, &source_path, "file-picker")
            .expect("register file")
            .expect("file attachment");

        assert_eq!(attachment.kind, "binary");
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn releases_unreferenced_draft_attachments_and_their_files() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        let source_path = dir.join("draft.txt");
        fs::write(&source_path, "draft attachment").expect("write draft file");
        let attachment = create_buddy_clipboard_file_from_path(&state, &source_path, "file-picker")
            .expect("register attachment")
            .expect("attachment");
        let attachment_id = attachment.attachment_id.expect("attachment id");
        let registered = state
            .find_attachment(&attachment_id)
            .expect("find attachment")
            .expect("registered attachment");

        let released = release_buddy_attachments(&state, vec![attachment_id.clone()])
            .expect("release attachment");

        assert_eq!(
            released.released_attachment_ids,
            vec![attachment_id.clone()]
        );
        assert!(state
            .find_attachment(&attachment_id)
            .expect("find released attachment")
            .is_none());
        assert!(!Path::new(&registered.path).exists());
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn keeps_attachments_that_are_referenced_by_messages() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let state = BuddyAppState::initialize_with_paths(BuddyAppPaths::from_data_dir(dir.clone()))
            .expect("initialize state");
        let source_path = dir.join("sent.txt");
        fs::write(&source_path, "sent attachment").expect("write sent file");
        let attachment = create_buddy_clipboard_file_from_path(&state, &source_path, "file-picker")
            .expect("register attachment")
            .expect("attachment");
        let attachment_id = attachment.attachment_id.expect("attachment id");
        let registered = state
            .find_attachment(&attachment_id)
            .expect("find attachment")
            .expect("registered attachment");
        let storage = state.storage_handle();
        let conversation = storage
            .create_conversation(CreateBuddyConversationRequest {
                forked_from_message_id: None,
                project_root: None,
                scope: "global".to_owned(),
                source_conversation_id: None,
                source_run_id: None,
                title: None,
            })
            .expect("create conversation");
        storage
            .append_conversation_message(AppendBuddyConversationMessageRequest {
                attachments: vec![BuddyMessageAttachment {
                    attachment_id: Some(attachment_id.clone()),
                    data_url: None,
                    kind: "text".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    name: "sent.txt".to_owned(),
                    preview_path: None,
                    size_bytes: registered.size_bytes,
                }],
                branch_id: conversation.active_branch_id,
                content: "sent".to_owned(),
                conversation_id: conversation.id,
                parent_message_id: None,
                role: "user".to_owned(),
                run_id: None,
                version_group_id: None,
                version_index: 1,
                version_status: "active".to_owned(),
            })
            .expect("append message");

        let released = release_buddy_attachments(&state, vec![attachment_id.clone()])
            .expect("release attachment");

        assert!(released.released_attachment_ids.is_empty());
        assert!(state
            .find_attachment(&attachment_id)
            .expect("find referenced attachment")
            .is_some());
        assert!(Path::new(&registered.path).exists());
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn draft_cleanup_preserves_retained_attachments_and_removes_abandoned_ones() {
        let dir = std::env::temp_dir().join(format!("lexora-buddy-test-{}", uuid::Uuid::new_v4()));
        let paths = BuddyAppPaths::from_data_dir(dir.clone());
        let state = BuddyAppState::initialize_with_paths(paths.clone()).expect("initialize state");
        let retained_source_path = dir.join("retained.txt");
        let abandoned_source_path = dir.join("abandoned.txt");
        fs::write(&retained_source_path, "retained attachment").expect("write retained file");
        fs::write(&abandoned_source_path, "abandoned attachment").expect("write abandoned file");
        let retained =
            create_buddy_clipboard_file_from_path(&state, &retained_source_path, "file-picker")
                .expect("register retained attachment")
                .expect("retained attachment");
        let abandoned =
            create_buddy_clipboard_file_from_path(&state, &abandoned_source_path, "file-picker")
                .expect("register abandoned attachment")
                .expect("abandoned attachment");
        let retained_id = retained.attachment_id.expect("retained attachment id");
        let abandoned_id = abandoned.attachment_id.expect("abandoned attachment id");
        let retained_path = state
            .find_attachment(&retained_id)
            .expect("find retained attachment")
            .expect("registered retained attachment")
            .path;
        let abandoned_path = state
            .find_attachment(&abandoned_id)
            .expect("find abandoned attachment")
            .expect("registered abandoned attachment")
            .path;
        drop(state);

        let restarted = BuddyAppState::initialize_with_paths(paths).expect("restart state");
        cleanup_buddy_draft_attachments(&restarted, vec![retained_id.clone()])
            .expect("cleanup draft attachments");

        assert!(restarted
            .find_attachment(&retained_id)
            .expect("find retained attachment")
            .is_some());
        assert!(Path::new(&retained_path).exists());
        assert!(restarted
            .find_attachment(&abandoned_id)
            .expect("find abandoned attachment")
            .is_none());
        assert!(!Path::new(&abandoned_path).exists());
        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }
}
