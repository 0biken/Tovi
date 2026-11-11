/// Errors as Kotlin sees them: a `ToviException` subclass whose `message`
/// is meant to be shown to the user. Flat, so Kotlin gets the plain message
/// rather than a field dump.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum ToviError {
    /// The app passed something malformed: a bad device ID, address or path
    #[error("{message}")]
    InvalidArgument { message: String },
    /// The operation itself failed (network, peer declined, disk, ...)
    #[error("{message}")]
    Failed { message: String },
}

impl ToviError {
    pub(crate) fn invalid(e: impl std::fmt::Display) -> Self {
        Self::InvalidArgument {
            message: format!("{e:#}"),
        }
    }
}

impl From<anyhow::Error> for ToviError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed {
            message: format!("{e:#}"),
        }
    }
}
