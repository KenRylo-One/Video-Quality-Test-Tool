//! Adapters for the back-end programs.
//!
//! Every back end runs as a separate process. Nothing here is linked into the tool, and
//! the tool ships no binary.

pub mod capabilities;
pub mod discovery;
pub mod error;
pub mod ffmpeg;
pub mod ffprobe;
pub mod hash;
pub mod parse;
pub mod vmaf_model;
pub mod vship;

pub use discovery::{Discovery, discover, find_path, probe_binary};
pub use error::BackendError;
pub use ffprobe::FfprobeProbe;
