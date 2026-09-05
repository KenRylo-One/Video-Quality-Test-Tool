use crate::capability::{BinaryId, LaneKind};
use crate::media::MediaInfo;
use crate::metric::MetricId;
use crate::vmaf_model::VmafModel;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone)]
pub struct JobInput {
    pub path: PathBuf,
    pub info: MediaInfo,
}

#[derive(Clone)]
pub struct MeasureJob {
    pub reference: JobInput,
    pub encode: JobInput,
    pub metrics: BTreeSet<MetricId>,
    pub frame_range: Option<(u64, u64)>,
    pub fused_passes: bool,
    pub work_dir: PathBuf,
    /// Every VMAF model available for the reference's frame rate bracket, already
    /// filtered to the standard or the high-frame-rate folder.
    pub vmaf_models: Vec<VmafModel>,
    /// The VMAF viewing distance, in picture heights, that chooses among those models.
    pub vmaf_viewing_distance: f32,
    /// The Butteraugli intensity target, in nits.
    pub butteraugli_intensity_nits: u32,
    /// FFVship's own `--gpu-threads` count.
    pub vship_gpu_threads: u32,
}

pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub expects: Vec<LogArtifact>,
    pub lane: LaneKind,
    /// Which binary this runs. The caller resolves `program` from this against
    /// whatever Settings or discovery actually found, since a backend's `plan()`
    /// cannot know where the user's copy of the binary lives.
    pub binary: BinaryId,
}

pub struct LogArtifact {
    pub path: PathBuf,
    pub format: LogFormat,
    pub metrics: Vec<MetricId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    PsnrStats,
    SsimStats,
    XpsnrStats,
    VmafCsv,
    VshipJson,
}

pub trait FrameSink {
    fn push(&mut self, metric: MetricId, frame: u64, value: f32);
}

/// Collects every value that a parser pushes, in the order the parser pushed them.
#[derive(Default)]
pub struct BufferSink {
    pub values: Vec<f32>,
}

impl FrameSink for BufferSink {
    fn push(&mut self, _metric: MetricId, _frame: u64, value: f32) {
        self.values.push(value);
    }
}

pub struct Progress {
    pub frame: u64,
}

pub struct ExitReport {
    pub succeeded: bool,
    pub wall_time_ms: u64,
    /// The code the process left with. A process that a signal stopped, or that the
    /// tool killed, reports nothing here.
    pub exit_code: Option<i32>,
    /// The last thing the process said that was not a frame count. A back end that
    /// cannot run usually explains itself, and those words are the only thing that
    /// tells the reader what to change.
    pub message: Option<String>,
}

pub trait ProcessRunner {
    fn run(
        &self,
        invocation: &Invocation,
        on_progress: &mut dyn FnMut(Progress),
    ) -> crate::Result<ExitReport>;
}
