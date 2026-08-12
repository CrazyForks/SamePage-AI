pub type BuddyResult<T> = Result<T, BuddyError>;

#[derive(Debug, thiserror::Error)]
pub enum BuddyError {
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "runtime")]
    #[error("sqlite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json operation failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("buddy state validation failed: {0}")]
    Validation(String),

    #[error("buddy state validation failed: unsupported {scope} capability: {capability}")]
    UnsupportedCapability { scope: String, capability: String },

    #[cfg(feature = "runtime")]
    #[error("codex runtime failed: {0}")]
    Codex(String),

    #[error("runtime failed: {0}")]
    Runtime(String),
}

#[cfg(feature = "runtime")]
impl BuddyError {
    pub(crate) fn public_code(&self) -> &'static str {
        match self {
            Self::Io(_) => "LOCAL_IO_FAILED",
            #[cfg(feature = "runtime")]
            Self::Sqlite(_) => "LOCAL_STORAGE_FAILED",
            Self::Json(_) => "LOCAL_DATA_INVALID",
            Self::Validation(_) => "VALIDATION_FAILED",
            Self::UnsupportedCapability { .. } => "UNSUPPORTED_CAPABILITY",
            #[cfg(feature = "runtime")]
            Self::Codex(_) => "CODEX_RUNTIME_FAILED",
            Self::Runtime(_) => "RUNTIME_EXECUTION_FAILED",
        }
    }

    pub(crate) fn public_message(&self) -> &'static str {
        match self {
            Self::Io(_) => "Local file operation failed",
            #[cfg(feature = "runtime")]
            Self::Sqlite(_) => "Local storage operation failed",
            Self::Json(_) => "Local data is invalid",
            Self::Validation(_) => "Request validation failed",
            Self::UnsupportedCapability { .. } => "Capability is not supported",
            #[cfg(feature = "runtime")]
            Self::Codex(_) => "Codex runtime operation failed",
            Self::Runtime(_) => "Runtime execution failed",
        }
    }

    pub(crate) fn retryable(&self) -> bool {
        match self {
            Self::Io(_) | Self::Runtime(_) => true,
            #[cfg(feature = "runtime")]
            Self::Codex(_) | Self::Sqlite(_) => true,
            Self::Json(_) | Self::Validation(_) | Self::UnsupportedCapability { .. } => false,
        }
    }
}
