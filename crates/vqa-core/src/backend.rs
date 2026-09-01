use crate::capability::LaneKind;
use crate::media::MediaInfo;
use crate::metric::MetricId;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;

pub struct JobInput {
    pub path: PathBuf,
    pub info: MediaInfo,
}

pub struct MeasureJob {
    pub reference: JobInput,
    pub encode: JobInput,
    pub metrics: BTreeSet<MetricId>,
    pub frame_range: Option<(u64, u64)>,
    pub fused_passes: bool,
    pub work_dir: PathBuf,
}

pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub expects: Vec<LogArtifact>,
    pub lane: LaneKind,
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
}

pub trait ProcessRunner {
    fn run(
        &self,
        invocation: &Invocation,
        on_progress: &mut dyn FnMut(Progress),
    ) -> crate::Result<ExitReport>;
}
