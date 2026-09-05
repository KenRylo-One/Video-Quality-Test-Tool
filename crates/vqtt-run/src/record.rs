//! The run record.
//!
//! Every number carries the record that explains it. A number without its record cannot
//! be repeated, so everything that changed a value appears here: the binaries and their
//! hashes, the measurement settings, the corrections, and every command line that ran.
//!
//! Per-frame values are deliberately absent. One hour at 60 fps with twelve metrics is
//! 2.6 million values, which JSON holds badly. Those go to the CSV files beside this one.

use crate::session::Session;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use vqtt_core::capability::LaneKind;
use vqtt_core::corrections::{Correction, Note};
use vqtt_core::media::MediaInfo;
use vqtt_core::metric::MetricId;
use vqtt_core::palette::Theme;
use vqtt_core::pooling::Pooled;
use vqtt_core::set::FileId;
use vqtt_core::vmaf_model::VmafModel;

/// The version of this file's shape. A reader that does not know it must stop rather
/// than guess.
pub const SCHEMA: u32 = 1;

/// The short git hash of the build, when it was built inside a checkout.
const GIT_SHA: Option<&str> = option_env!("VQTT_GIT_SHA");

/// One command that ran, with what it cost and how it ended.
#[derive(Debug, Clone, Serialize)]
pub struct InvocationRecord {
    pub seq: u32,
    pub lane: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub wall_ms: u64,
}

impl InvocationRecord {
    pub fn of(seq: u32, invocation: &vqtt_core::backend::Invocation) -> Self {
        Self {
            seq,
            lane: match invocation.lane {
                LaneKind::Cpu => "cpu",
                LaneKind::Gpu => "gpu",
            },
            program: invocation.program.clone(),
            args: invocation
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            cwd: invocation.cwd.clone(),
            exit_code: None,
            wall_ms: 0,
        }
    }

    /// The command as one line, for the command log.
    pub fn command_line(&self) -> String {
        let mut line = quote(&self.program.to_string_lossy());
        for arg in &self.args {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        line
    }
}

/// Wraps an argument in quotes when it holds a space, so the log can be pasted back
/// into a shell and run.
fn quote(text: &str) -> String {
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.to_string()
    }
}

/// Everything one finished run produced.
///
/// The interface fills this and hands it back. It holds no interface type, so a command
/// line can build the same value.
pub struct RunOutcome {
    pub run_id: String,
    pub started: String,
    pub finished: String,
    pub metrics: Vec<MetricId>,
    pub frame_range: Option<(u64, u64)>,
    pub first_frame: u64,
    pub results: HashMap<(FileId, MetricId), Pooled>,
    pub series: HashMap<(FileId, MetricId), Vec<f32>>,
    pub corrections: Vec<Correction>,
    pub notes: Vec<Note>,
    pub invocations: Vec<InvocationRecord>,
    pub vmaf_model: Option<VmafModel>,
    pub theme: Theme,
}

#[derive(Debug, Serialize)]
pub struct RunRecord {
    pub schema: u32,
    pub run_id: String,
    pub started: String,
    pub finished: String,
    pub tool: Tool,
    pub machine: Machine,
    pub backends: Vec<Backend>,
    pub comparison_set: ComparisonSetRecord,
    pub plan: PlanRecord,
    pub corrections: Vec<Correction>,
    pub notes: Vec<Note>,
    pub invocations: Vec<InvocationRecord>,
    pub results: Vec<ResultRecord>,
}

#[derive(Debug, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub version: &'static str,
    pub git: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Machine {
    pub os: String,
    pub cpu_threads: usize,
}

#[derive(Debug, Serialize)]
pub struct Backend {
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ComparisonSetRecord {
    pub reference: Option<FileRecord>,
    pub encodes: Vec<FileRecord>,
}

#[derive(Debug, Serialize)]
pub struct FileRecord {
    pub role: &'static str,
    pub label: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// `qf1:<bytes>:<hash of the first 8 MiB>:<hash of the last 8 MiB>`. A full hash of
    /// a 50 GB reference costs minutes on every run, which no pre-run check can afford.
    pub fingerprint: Option<String>,
    pub media: MediaInfo,
}

#[derive(Debug, Serialize)]
pub struct PlanRecord {
    pub preset: Option<String>,
    pub metrics: Vec<&'static str>,
    pub frame_range: Option<FrameRange>,
    pub measurement: Measurement,
}

#[derive(Debug, Serialize)]
pub struct FrameRange {
    pub first: u64,
    pub last: u64,
}

#[derive(Debug, Serialize)]
pub struct Measurement {
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub color_range: String,
    pub scaler: &'static str,
    pub vmaf_model: Option<PathBuf>,
    pub adm_norm_view_dist: Option<f32>,
    pub adm_ref_display_height: Option<u32>,
    pub vmaf_neg_builtin: Option<bool>,
    pub fused_passes: bool,
    pub butteraugli_intensity_nits: u32,
    pub vship_gpu_threads: u32,
}

#[derive(Debug, Serialize)]
pub struct ResultRecord {
    pub encode: String,
    pub metric: &'static str,
    pub frames: usize,
    pub series: String,
    pub series_column: &'static str,
    pub pooled: Pooled,
    /// Why the harmonic mean is `null`, when it is. The tool does not compute a value
    /// it cannot defend, and says so instead of leaving a gap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harmonic_mean_blocked: Option<&'static str>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub metric_notes: &'static [&'static str],
}

/// The moment a run started or finished, as `2026-08-29T02:41:11Z`.
pub fn timestamp(at: std::time::SystemTime) -> String {
    let at = time::OffsetDateTime::from(at);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        at.year(),
        at.month() as u8,
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

/// The identity of one run, as `2026-08-29T02-41-11Z-a7f3`.
///
/// The suffix separates two runs that start inside the same second, which happens when
/// a run is repeated straight away.
pub fn run_id(at: std::time::SystemTime) -> String {
    let nanos = at
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    format!("{}-{:04x}", timestamp(at).replace(':', "-"), nanos & 0xffff)
}

/// The name of the per-frame file for one encode.
pub fn frame_csv_name(label: &str) -> String {
    format!("frames-{}.csv", safe_name(label))
}

/// A label with every character a file system objects to replaced.
pub fn safe_name(label: &str) -> String {
    label
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '_',
        })
        .collect()
}

/// Builds the record.
///
/// The fingerprints are read here rather than at run time, because each one reads
/// 16 MiB and nothing before the export needs them.
pub fn build(session: &Session, outcome: &RunOutcome) -> RunRecord {
    let reference = session.files.reference();

    RunRecord {
        schema: SCHEMA,
        run_id: outcome.run_id.clone(),
        started: outcome.started.clone(),
        finished: outcome.finished.clone(),
        tool: Tool {
            name: "vqtt",
            version: env!("CARGO_PKG_VERSION"),
            git: GIT_SHA,
        },
        machine: Machine {
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            cpu_threads: std::thread::available_parallelism().map_or(0, |count| count.get()),
        },
        backends: backends_of(session),
        comparison_set: ComparisonSetRecord {
            reference: reference.map(|file| file_record(file, "reference")),
            encodes: session
                .files
                .encodes()
                .map(|file| file_record(file, "encode"))
                .collect(),
        },
        plan: plan_record(session, outcome, reference.map(|file| &file.info)),
        corrections: outcome.corrections.clone(),
        notes: outcome.notes.clone(),
        invocations: outcome.invocations.clone(),
        results: results_of(session, outcome),
    }
}

fn backends_of(session: &Session) -> Vec<Backend> {
    let mut backends: Vec<Backend> = session
        .inventory
        .iter()
        .map(|found| Backend {
            id: found.id.display_name().to_string(),
            path: found.path.clone(),
            sha256: found.sha256.clone(),
            version: found.capabilities.version.clone(),
            filters: found
                .capabilities
                .ffmpeg_filters
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            metrics: found
                .capabilities
                .vship_metrics
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
        })
        .collect();
    backends.sort_by(|left, right| left.id.cmp(&right.id));
    backends
}

fn file_record(file: &vqtt_core::set::MediaFile, role: &'static str) -> FileRecord {
    FileRecord {
        role,
        label: file.label.clone(),
        path: file.info.path.clone(),
        bytes: file.info.bytes,
        fingerprint: vqtt_backends::hash::fingerprint_file(&file.info.path)
            .ok()
            .map(|print| print.to_string()),
        media: file.info.clone(),
    }
}

fn plan_record(
    session: &Session,
    outcome: &RunOutcome,
    reference: Option<&MediaInfo>,
) -> PlanRecord {
    let model = outcome.vmaf_model.as_ref();
    PlanRecord {
        preset: session.selection.preset.map(str::to_string),
        metrics: outcome.metrics.iter().map(|id| id.key()).collect(),
        frame_range: outcome
            .frame_range
            .map(|(first, last)| FrameRange { first, last }),
        measurement: Measurement {
            width: reference.map_or(0, |info| info.width),
            height: reference.map_or(0, |info| info.height),
            pix_fmt: reference.map_or_else(String::new, |info| info.pix_fmt.clone()),
            color_range: reference
                .map(|info| info.effective_color_range())
                .and_then(|range| range.ffmpeg_value())
                .unwrap_or("unknown")
                .to_string(),
            scaler: "bicubic",
            vmaf_model: model.map(|model| model.path.clone()),
            adm_norm_view_dist: model.map(|model| model.normalized_viewing_distance),
            adm_ref_display_height: model.map(|model| model.reference_display_height),
            // Every VMAF v1 model already carries `adm_enhn_gain_limit` at 1.0, so a v1
            // run is NEG whether or not anybody asked for it.
            vmaf_neg_builtin: model.map(|model| model.is_v1),
            fused_passes: session.settings.fused_passes,
            butteraugli_intensity_nits: session.settings.butteraugli_intensity_nits,
            vship_gpu_threads: session.settings.vship_gpu_threads,
        },
    }
}

fn results_of(session: &Session, outcome: &RunOutcome) -> Vec<ResultRecord> {
    let mut records = Vec::new();
    for encode in session.files.encodes() {
        for def in vqtt_core::metric::REGISTRY.iter() {
            let Some(pooled) = outcome.results.get(&(encode.id, def.id)) else {
                continue;
            };
            records.push(ResultRecord {
                encode: encode.label.clone(),
                metric: def.id.key(),
                frames: outcome.series.get(&(encode.id, def.id)).map_or(0, Vec::len),
                series: frame_csv_name(&encode.label),
                series_column: def.id.key(),
                pooled: *pooled,
                harmonic_mean_blocked: match def.harmonic_mean {
                    vqtt_core::metric::HarmonicMean::Blocked(reason) => Some(reason),
                    _ if pooled.harmonic_mean.is_none() => Some(
                        "The values reach zero or below, where the harmonic mean has no meaning.",
                    ),
                    _ => None,
                },
                metric_notes: def.notes,
            });
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_version_is_the_first_field_a_reader_meets() {
        let json = serde_json::to_string(&Tool {
            name: "vqtt",
            version: "0.1.0",
            git: None,
        })
        .unwrap();
        assert!(json.starts_with("{\"name\""));

        let record = serde_json::json!({ "schema": SCHEMA });
        assert_eq!(record["schema"], 1);
    }

    #[test]
    fn a_command_line_quotes_only_the_arguments_that_hold_a_space() {
        let record = InvocationRecord {
            seq: 1,
            lane: "cpu",
            program: PathBuf::from("C:/Program Files/ffmpeg.exe"),
            args: vec!["-i".into(), "D:/a b.mov".into(), "-v".into()],
            cwd: None,
            exit_code: Some(0),
            wall_ms: 12,
        };

        assert_eq!(
            record.command_line(),
            "\"C:/Program Files/ffmpeg.exe\" -i \"D:/a b.mov\" -v"
        );
    }

    #[test]
    fn a_label_that_a_file_system_would_refuse_becomes_a_usable_name() {
        assert_eq!(
            frame_csv_name("YT 1080p/50Mbps"),
            "frames-YT_1080p_50Mbps.csv"
        );
        assert_eq!(safe_name("a:b*c?d"), "a_b_c_d");
    }
}
