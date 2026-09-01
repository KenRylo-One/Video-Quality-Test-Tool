//! Errors for the pure layer.

use thiserror::Error;

/// Every failure that the pure layer can report.
#[derive(Debug, Error)]
pub enum CoreError {
    /// A text or JSON input did not hold what the parser needs.
    #[error("cannot read {what}: {detail}")]
    Parse {
        /// The thing that the parser tried to read.
        what: &'static str,
        /// What was wrong with it.
        detail: String,
    },

    /// The file holds no video stream.
    #[error("the file holds no video stream: {0}")]
    NoVideoStream(String),

    /// A metric key does not appear in the registry.
    #[error("unknown metric key: {0}")]
    UnknownMetric(String),

    /// The comparison set does not hold that file.
    #[error("unknown file id: {0}")]
    UnknownFile(u64),
}

impl CoreError {
    /// Builds a parse error.
    pub fn parse(what: &'static str, detail: impl Into<String>) -> Self {
        Self::Parse {
            what,
            detail: detail.into(),
        }
    }
}

/// The result type of the pure layer.
pub type Result<T> = std::result::Result<T, CoreError>;
