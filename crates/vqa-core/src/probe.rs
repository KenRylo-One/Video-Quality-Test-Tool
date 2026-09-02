//! The media probe trait.

use crate::media::{FrameSample, LumaExtremes, MediaInfo};
use std::path::Path;

/// Reads what a media file is.
///
/// The trait lives here so that `vqa-run` depends on the trait and never on `ffprobe`.
/// A test gives a fake implementation and needs no binary.
pub trait MediaProbe {
    /// The error type of this implementation.
    type Error;

    /// Reads the stream and format record of one file.
    fn probe(&self, path: &Path) -> Result<MediaInfo, Self::Error>;

    /// Reads the real luma minimum and maximum from a sample of frames.
    ///
    /// The color range flag can disagree with the data. The tool reports that, and
    /// corrects nothing, since it cannot tell a wrong flag from low-contrast content.
    fn luma_extremes(&self, path: &Path, sample: FrameSample) -> Result<LumaExtremes, Self::Error>;
}
