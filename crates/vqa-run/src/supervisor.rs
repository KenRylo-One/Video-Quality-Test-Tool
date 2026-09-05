use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use vqa_backends::parse::stats_file::parse_stats_file;
use vqa_core::backend::{BufferSink, Invocation, ProcessRunner, Progress};
use vqa_core::capability::LaneKind;
use vqa_core::metric::MetricId;
use vqa_core::pooling::{Pooled, pool};
use vqa_core::set::FileId;

/// Runs a real process. This is the only real implementation of `ProcessRunner`,
/// because this is the one place that starts a real process.
///
/// It reads the child's error stream line by line rather than waiting for the whole
/// process, which is what gives the frame counter something to report and what gives
/// Cancel somewhere to act. FFmpeg writes `frame=N` there because of `-progress
/// pipe:2`, and FFVship writes an index and a score for each frame because of
/// `--live-score-output`.
#[derive(Default)]
pub struct RealProcessRunner {
    cancel: Arc<AtomicBool>,
}

impl RealProcessRunner {
    /// Builds a runner that stops when `cancel` is set.
    pub fn with_cancel(cancel: Arc<AtomicBool>) -> Self {
        Self { cancel }
    }
}

impl ProcessRunner for RealProcessRunner {
    fn run(
        &self,
        invocation: &Invocation,
        on_progress: &mut dyn FnMut(Progress),
    ) -> vqa_core::Result<vqa_core::backend::ExitReport> {
        let start_time = std::time::Instant::now();

        let mut command = std::process::Command::new(&invocation.program);
        command.args(&invocation.args);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if let Some(cwd) = &invocation.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &invocation.env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .map_err(|error| vqa_core::CoreError::parse("process", error.to_string()))?;

        // The same thread reads and kills, so the child needs no lock. Cancel lands at
        // the next progress line, which is at most half a second away.
        let mut cancelled = false;
        if let Some(stream) = child.stderr.take() {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if let Some(frame) = frame_of(&line) {
                    on_progress(Progress { frame });
                }
                if self.cancel.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    cancelled = true;
                    break;
                }
            }
        }

        let status = child
            .wait()
            .map_err(|error| vqa_core::CoreError::parse("process", error.to_string()))?;

        Ok(vqa_core::backend::ExitReport {
            succeeded: status.success() && !cancelled,
            wall_time_ms: start_time.elapsed().as_millis() as u64,
        })
    }
}

/// Reads a frame number out of one line of a back end's progress stream.
///
/// FFmpeg writes `frame=1234` because of `-progress`. FFVship writes an index and a
/// score, separated by a space, because of `--live-score-output`. Anything else is
/// error text, and gives no frame.
fn frame_of(line: &str) -> Option<u64> {
    let line = line.trim();
    if let Some(value) = line.strip_prefix("frame=") {
        return value.trim().parse().ok();
    }
    let (index, score) = line.split_once(char::is_whitespace)?;
    let index: u64 = index.parse().ok()?;
    score.trim().parse::<f64>().ok()?;
    Some(index)
}

pub enum SupervisorEvent {
    Progress {
        encode: FileId,
        metric: MetricId,
        frame: u64,
    },
    MetricReady {
        encode: FileId,
        metric: MetricId,
        pooled: Pooled,
        /// Every per-frame value, in frame order. The plot draws these, so they travel
        /// with the pooled record rather than being parsed a second time.
        series: Vec<f32>,
    },
    EncodeDone {
        encode: FileId,
    },
    Failed {
        encode: FileId,
        error: String,
    },
}

pub struct EncodeWork {
    pub encode: FileId,
    pub invocations: Vec<Invocation>,
}

struct WorkItem {
    encode: FileId,
    invocation: Invocation,
}

pub fn run_plan<R>(
    work: Vec<EncodeWork>,
    runner: Arc<R>,
    cpu_lane_permits: usize,
    gpu_lane_permits: usize,
) -> Receiver<SupervisorEvent>
where
    R: ProcessRunner + Send + Sync + 'static,
{
    run_plan_with_cancel(
        work,
        runner,
        cpu_lane_permits,
        gpu_lane_permits,
        Arc::new(AtomicBool::new(false)),
    )
}

/// Runs the plan, and stops taking new work once `cancel` is set.
///
/// A cancelled run drains its queues rather than tearing threads down, so every worker
/// leaves on its own and the channel closes exactly once.
pub fn run_plan_with_cancel<R>(
    work: Vec<EncodeWork>,
    runner: Arc<R>,
    cpu_lane_permits: usize,
    gpu_lane_permits: usize,
    cancel: Arc<AtomicBool>,
) -> Receiver<SupervisorEvent>
where
    R: ProcessRunner + Send + Sync + 'static,
{
    let (sender, receiver) = channel();

    let mut remaining_by_encode = HashMap::new();
    let mut cpu_queue = VecDeque::new();
    let mut gpu_queue = VecDeque::new();

    for encode_work in work {
        remaining_by_encode.insert(encode_work.encode, encode_work.invocations.len());
        for invocation in encode_work.invocations {
            let queue = match invocation.lane {
                LaneKind::Cpu => &mut cpu_queue,
                LaneKind::Gpu => &mut gpu_queue,
            };
            queue.push_back(WorkItem {
                encode: encode_work.encode,
                invocation,
            });
        }
    }

    let remaining_by_encode = Arc::new(Mutex::new(remaining_by_encode));
    let cpu_queue = Arc::new(Mutex::new(cpu_queue));
    let gpu_queue = Arc::new(Mutex::new(gpu_queue));

    for _ in 0..cpu_lane_permits.max(1) {
        spawn_worker(
            cpu_queue.clone(),
            remaining_by_encode.clone(),
            runner.clone(),
            sender.clone(),
            cancel.clone(),
        );
    }
    for _ in 0..gpu_lane_permits.max(1) {
        spawn_worker(
            gpu_queue.clone(),
            remaining_by_encode.clone(),
            runner.clone(),
            sender.clone(),
            cancel.clone(),
        );
    }

    receiver
}

fn spawn_worker<R>(
    queue: Arc<Mutex<VecDeque<WorkItem>>>,
    remaining_by_encode: Arc<Mutex<HashMap<FileId, usize>>>,
    runner: Arc<R>,
    sender: Sender<SupervisorEvent>,
    cancel: Arc<AtomicBool>,
) where
    R: ProcessRunner + Send + Sync + 'static,
{
    std::thread::spawn(move || {
        loop {
            let Some(item) = queue.lock().unwrap().pop_front() else {
                break;
            };
            if cancel.load(Ordering::Relaxed) {
                // Count the item off without running it, so the encode still reports
                // done and the interface never waits on work that will not happen.
                finish_item(&item, &remaining_by_encode, &sender);
                continue;
            }
            run_one(&item, runner.as_ref(), &sender);
            finish_item(&item, &remaining_by_encode, &sender);
        }
    });
}

/// Counts one invocation off its encode, and reports the encode once nothing is left.
fn finish_item(
    item: &WorkItem,
    remaining_by_encode: &Arc<Mutex<HashMap<FileId, usize>>>,
    sender: &Sender<SupervisorEvent>,
) {
    let done = {
        let mut remaining = remaining_by_encode.lock().unwrap();
        let count = remaining.entry(item.encode).or_insert(0);
        *count = count.saturating_sub(1);
        *count == 0
    };
    if done {
        let _ = sender.send(SupervisorEvent::EncodeDone {
            encode: item.encode,
        });
    }
}

fn run_one(item: &WorkItem, runner: &dyn ProcessRunner, sender: &Sender<SupervisorEvent>) {
    let encode = item.encode;
    let mut on_progress = |progress: Progress| {
        if let Some(metric) = item
            .invocation
            .expects
            .first()
            .and_then(|artifact| artifact.metrics.first())
        {
            let _ = sender.send(SupervisorEvent::Progress {
                encode,
                metric: *metric,
                frame: progress.frame,
            });
        }
    };

    let outcome = runner.run(&item.invocation, &mut on_progress);
    let exit_report = match outcome {
        Ok(report) if report.succeeded => report,
        Ok(_) => {
            let _ = sender.send(SupervisorEvent::Failed {
                encode,
                error: "the process exited with a failure".to_string(),
            });
            return;
        }
        Err(error) => {
            let _ = sender.send(SupervisorEvent::Failed {
                encode,
                error: error.to_string(),
            });
            return;
        }
    };
    let _ = exit_report;

    for artifact in &item.invocation.expects {
        for metric in artifact.metrics.iter().copied() {
            match read_pooled(artifact, metric) {
                Ok(Some((pooled, series))) => {
                    let _ = sender.send(SupervisorEvent::MetricReady {
                        encode,
                        metric,
                        pooled,
                        series,
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = sender.send(SupervisorEvent::Failed { encode, error });
                }
            }
        }
    }
}

fn read_pooled(
    artifact: &vqa_core::backend::LogArtifact,
    metric: MetricId,
) -> Result<Option<(Pooled, Vec<f32>)>, String> {
    let file = std::fs::File::open(&artifact.path).map_err(|error| error.to_string())?;
    let mut sink = BufferSink::default();
    parse_stats_file(BufReader::new(file), artifact.format, metric, &mut sink)
        .map_err(|error| error.to_string())?;
    Ok(pool(&sink.values, metric.def().harmonic_mean).map(|pooled| (pooled, sink.values)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use vqa_core::CoreError;
    use vqa_core::backend::{ExitReport, LogArtifact, LogFormat};
    use vqa_core::capability::BinaryId;

    struct FakeRunner {
        succeed: bool,
    }

    impl ProcessRunner for FakeRunner {
        fn run(
            &self,
            _invocation: &Invocation,
            on_progress: &mut dyn FnMut(Progress),
        ) -> vqa_core::Result<ExitReport> {
            on_progress(Progress { frame: 0 });
            if self.succeed {
                Ok(ExitReport {
                    succeeded: true,
                    wall_time_ms: 1,
                })
            } else {
                Err(CoreError::parse("test", "the fake runner was told to fail"))
            }
        }
    }

    fn write_fixture(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("vqa-supervisor-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn psnr_invocation(encode: FileId, stats_path: PathBuf) -> EncodeWork {
        EncodeWork {
            encode,
            invocations: vec![Invocation {
                program: PathBuf::from("ffmpeg"),
                args: Vec::new(),
                env: Vec::new(),
                cwd: None,
                expects: vec![LogArtifact {
                    path: stats_path,
                    format: LogFormat::PsnrStats,
                    metrics: vec![MetricId::PsnrY],
                }],
                lane: LaneKind::Cpu,
                binary: BinaryId::Ffmpeg,
            }],
        }
    }

    #[test]
    fn an_ffmpeg_progress_line_gives_its_frame_number() {
        assert_eq!(frame_of("frame=1234"), Some(1234));
        assert_eq!(frame_of("frame=0"), Some(0));
    }

    #[test]
    fn an_ffvship_index_and_score_line_gives_its_index() {
        assert_eq!(frame_of("41 87.512"), Some(41));
        assert_eq!(frame_of("0 -3.25"), Some(0));
    }

    #[test]
    fn error_text_and_the_other_progress_keys_give_no_frame() {
        assert_eq!(frame_of("fps=24.0"), None);
        assert_eq!(frame_of("out_time=00:00:02.26"), None);
        assert_eq!(frame_of("progress=continue"), None);
        assert_eq!(frame_of("Error opening input file"), None);
        assert_eq!(frame_of(""), None);
        assert_eq!(frame_of("frame=notanumber"), None);
    }

    #[test]
    fn a_cancelled_run_starts_nothing_and_still_reports_the_encode_done() {
        let first = write_fixture("cancel_a.log", "n:1 psnr_y:40.0\n");
        let second = write_fixture("cancel_b.log", "n:1 psnr_y:41.0\n");
        let work = vec![EncodeWork {
            encode: FileId(9),
            invocations: vec![
                psnr_invocation(FileId(9), first).invocations.remove(0),
                psnr_invocation(FileId(9), second).invocations.remove(0),
            ],
        }];

        let cancel = Arc::new(AtomicBool::new(true));
        let receiver =
            run_plan_with_cancel(work, Arc::new(FakeRunner { succeed: true }), 1, 1, cancel);

        let mut metric_ready = 0;
        let mut encode_done = 0;
        for event in receiver.iter() {
            match event {
                SupervisorEvent::MetricReady { .. } => metric_ready += 1,
                SupervisorEvent::EncodeDone { .. } => encode_done += 1,
                _ => {}
            }
        }
        assert_eq!(metric_ready, 0, "a cancelled run measures nothing");
        assert_eq!(encode_done, 1, "the interface must not wait forever");
    }

    #[test]
    fn a_successful_invocation_sends_a_metric_ready_event_and_then_encode_done() {
        let stats_path = write_fixture("psnr_ok.log", "n:1 psnr_y:40.0\nn:2 psnr_y:42.0\n");
        let work = vec![psnr_invocation(FileId(1), stats_path)];
        let receiver = run_plan(work, Arc::new(FakeRunner { succeed: true }), 1, 1);

        let mut saw_metric_ready = false;
        let mut saw_encode_done = false;
        for event in receiver.iter() {
            match event {
                SupervisorEvent::MetricReady { pooled, .. } => {
                    saw_metric_ready = true;
                    assert!((pooled.mean - 41.0).abs() < 0.01);
                }
                SupervisorEvent::EncodeDone { .. } => saw_encode_done = true,
                _ => {}
            }
        }
        assert!(saw_metric_ready);
        assert!(saw_encode_done);
    }

    #[test]
    fn a_failing_invocation_sends_failed_and_still_finishes_the_encode() {
        let stats_path = write_fixture("psnr_unused.log", "n:1 psnr_y:40.0\n");
        let work = vec![psnr_invocation(FileId(2), stats_path)];
        let receiver = run_plan(work, Arc::new(FakeRunner { succeed: false }), 1, 1);

        let mut saw_failed = false;
        let mut saw_encode_done = false;
        for event in receiver.iter() {
            match event {
                SupervisorEvent::Failed { .. } => saw_failed = true,
                SupervisorEvent::EncodeDone { .. } => saw_encode_done = true,
                _ => {}
            }
        }
        assert!(saw_failed);
        assert!(saw_encode_done);
    }

    #[test]
    fn two_invocations_for_one_encode_both_finish_before_encode_done() {
        let first_path = write_fixture("multi_a.log", "n:1 psnr_y:10.0\n");
        let second_path = write_fixture("multi_b.log", "n:1 psnr_y:20.0\n");
        let work = vec![EncodeWork {
            encode: FileId(3),
            invocations: vec![
                Invocation {
                    program: PathBuf::from("ffmpeg"),
                    args: Vec::new(),
                    env: Vec::new(),
                    cwd: None,
                    expects: vec![LogArtifact {
                        path: first_path,
                        format: LogFormat::PsnrStats,
                        metrics: vec![MetricId::PsnrY],
                    }],
                    lane: LaneKind::Cpu,
                    binary: BinaryId::Ffmpeg,
                },
                Invocation {
                    program: PathBuf::from("ffmpeg"),
                    args: Vec::new(),
                    env: Vec::new(),
                    cwd: None,
                    expects: vec![LogArtifact {
                        path: second_path,
                        format: LogFormat::PsnrStats,
                        metrics: vec![MetricId::PsnrY],
                    }],
                    lane: LaneKind::Cpu,
                    binary: BinaryId::Ffmpeg,
                },
            ],
        }];

        let receiver = run_plan(work, Arc::new(FakeRunner { succeed: true }), 2, 1);
        let mut metric_ready_count = 0;
        let mut encode_done_count = 0;
        for event in receiver.iter() {
            match event {
                SupervisorEvent::MetricReady { .. } => metric_ready_count += 1,
                SupervisorEvent::EncodeDone { .. } => encode_done_count += 1,
                _ => {}
            }
        }
        assert_eq!(metric_ready_count, 2);
        assert_eq!(encode_done_count, 1);
    }
}
