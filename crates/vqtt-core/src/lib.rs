//! Pure logic for the video quality analysis tool.
//!
//! This crate reads no file and starts no process. Every type here is a plain value,
//! so a test needs no binary and no video file.

pub mod backend;
pub mod capability;
pub mod corrections;
pub mod error;
pub mod estimate;
pub mod frames;
pub mod media;
pub mod metric;
pub mod palette;
pub mod plot;
pub mod plot_svg;
pub mod pooling;
pub mod preset;
pub mod probe;
pub mod set;
pub mod vmaf_model;

pub use backend::{
    BufferSink, ExitReport, FrameSink, Invocation, JobInput, LogArtifact, LogFormat, MeasureJob,
    ProcessRunner, Progress,
};
pub use capability::{BinaryCapabilities, BinaryId, FoundBinary, Inventory, LaneKind, Requirement};
pub use corrections::{
    Correction, CorrectionDetail, CorrectionId, DetectedCorrections, Note, NoteId, detect_all,
    detect_vship_gap_notes,
};
pub use error::{CoreError, Result};
pub use estimate::RunEstimate;
pub use frames::{FrameValue, worst_frames};
pub use media::{ColorRange, Fingerprint, FrameSample, LumaExtremes, MediaInfo, Rational};
pub use metric::{
    Availability, Direction, HarmonicMean, MetricDef, MetricGroup, MetricId, Percentile, Provider,
    REGISTRY, Unit,
};
pub use palette::{SeriesColor, Theme};
pub use plot::{
    Body, Canvas, Chrome, PlotRequest, Rgba, Scene, SeriesInput, TextAlign, build_scenes, draw,
};
pub use plot_svg::to_svg;
pub use pooling::{Pooled, pool};
pub use preset::{PRESETS, Preset};
pub use probe::MediaProbe;
pub use set::{ComparisonSet, DiffMarks, FileId, MediaFile};
