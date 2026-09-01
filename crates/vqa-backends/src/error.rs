//! Errors for the adapter layer.

use std::path::PathBuf;
use thiserror::Error;

/// Every failure that an adapter can report.
#[derive(Debug, Error)]
pub enum BackendError {
    /// The tool could not start the program.
    #[error("cannot start {program}: {source}")]
    Spawn {
        /// The program that did not start.
        program: String,
        /// What the operating system reported.
        #[source]
        source: std::io::Error,
    },

    /// The program ran and reported a failure.
    #[error("{program} exited with {code}: {stderr}")]
    Exit {
        /// The program.
        program: String,
        /// The exit code, or `signal` when a signal stopped it.
        code: String,
        /// The last lines of standard error.
        stderr: String,
    },

    /// The tool could not read a file.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system reported.
        #[source]
        source: std::io::Error,
    },

    /// The output of the program did not hold what the parser needs.
    #[error("cannot read the output of {program}: {detail}")]
    Parse {
        /// The program.
        program: String,
        /// What was wrong.
        detail: String,
    },

    /// The pure layer reported a failure.
    #[error(transparent)]
    Core(#[from] vqa_core::CoreError),
}

/// The result type of the adapter layer.
pub type Result<T> = std::result::Result<T, BackendError>;
