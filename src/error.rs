//! Error model of the plugin.
//!
//! Errors are serialized as plain strings across the Tauri IPC boundary, which
//! keeps them predictable and readable on the JavaScript side.

use serde::Serialize;

/// Errors that the `power-monitor` plugin can produce.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The current platform has no power-monitoring backend.
    #[error("power monitoring is not supported on this platform")]
    UnsupportedPlatform,
    /// Querying the system power state failed.
    #[error("failed to query the system power state: {0}")]
    Query(String),
    /// Starting the native event listeners failed.
    #[error("failed to initialize the power event listeners: {0}")]
    ListenerInit(String),
    /// A Tauri error.
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
}

impl Error {
    pub(crate) fn query<E: std::fmt::Display>(err: E) -> Self {
        Error::Query(err.to_string())
    }

    pub(crate) fn listener<E: std::fmt::Display>(err: E) -> Self {
        Error::ListenerInit(err.to_string())
    }
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

/// Result type used throughout the plugin.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_serialize_to_their_display_string() {
        let err = Error::query("nope");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json,
            serde_json::json!("failed to query the system power state: nope")
        );

        let json = serde_json::to_value(Error::UnsupportedPlatform).unwrap();
        assert_eq!(
            json,
            serde_json::json!("power monitoring is not supported on this platform")
        );
    }
}
