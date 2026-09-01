use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use vqa_backends::ffmpeg;
use vqa_core::backend::{JobInput, MeasureJob};
use vqa_core::metric::MetricId;
use vqa_core::pooling::Pooled;
use vqa_core::set::FileId;
use vqa_run::{EncodeWork, RealProcessRunner, Session, SupervisorEvent, run_plan};

pub struct RunState {
    receiver: Receiver<SupervisorEvent>,
    encodes_remaining: usize,
    pub results: HashMap<(FileId, MetricId), Pooled>,
    pub failures: Vec<String>,
}

impl RunState {
    /// Builds one measurement job for each encode, and starts the supervisor.
    ///
    /// Returns nothing when there is no reference, or no encode, or no runnable metric.
    pub fn start(session: &Session) -> Option<Self> {
        let reference = session.files.reference()?;
        let metrics: std::collections::BTreeSet<MetricId> =
            session.runnable_metrics().into_iter().collect();
        if metrics.is_empty() {
            return None;
        }

        let frame_range = if session.selection.whole_file {
            None
        } else {
            Some((session.selection.first_frame, session.selection.last_frame))
        };

        let run_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let work_dir = std::env::temp_dir()
            .join("vqa-run")
            .join(run_id.to_string());

        let mut work = Vec::new();
        for encode in session.files.encodes() {
            let encode_work_dir = work_dir.join(encode.id.0.to_string());
            std::fs::create_dir_all(&encode_work_dir).ok()?;

            let job = MeasureJob {
                reference: JobInput {
                    path: reference.info.path.clone(),
                    info: reference.info.clone(),
                },
                encode: JobInput {
                    path: encode.info.path.clone(),
                    info: encode.info.clone(),
                },
                metrics: metrics.clone(),
                frame_range,
                fused_passes: session.settings.fused_passes,
                work_dir: encode_work_dir,
            };

            let invocations = ffmpeg::plan(&job).ok()?;
            if invocations.is_empty() {
                continue;
            }
            work.push(EncodeWork {
                encode: encode.id,
                invocations,
            });
        }

        if work.is_empty() {
            return None;
        }

        let encodes_remaining = work.len();
        let cpu_lane_permits = session.settings.cpu_lane_permits as usize;
        let gpu_lane_permits = session.settings.gpu_lane_permits as usize;
        let receiver = run_plan(
            work,
            Arc::new(RealProcessRunner),
            cpu_lane_permits,
            gpu_lane_permits,
        );

        Some(Self {
            receiver,
            encodes_remaining,
            results: HashMap::new(),
            failures: Vec::new(),
        })
    }

    /// Reads every event that is ready without waiting. Returns true when a run is
    /// still in progress, so the caller knows whether to poll again.
    pub fn poll(&mut self) -> bool {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                SupervisorEvent::Progress { .. } => {}
                SupervisorEvent::MetricReady {
                    encode,
                    metric,
                    pooled,
                } => {
                    self.results.insert((encode, metric), pooled);
                }
                SupervisorEvent::EncodeDone { .. } => {
                    self.encodes_remaining = self.encodes_remaining.saturating_sub(1);
                }
                SupervisorEvent::Failed { error, .. } => {
                    self.failures.push(error);
                }
            }
        }
        self.encodes_remaining > 0
    }
}
