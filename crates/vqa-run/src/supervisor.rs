use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vqa_backends::parse::stats_file::parse_stats_file;
use vqa_core::backend::{BufferSink, Invocation, ProcessRunner, Progress};
use vqa_core::capability::LaneKind;
use vqa_core::metric::MetricId;
use vqa_core::pooling::{Pooled, pool};
use vqa_core::set::FileId;

/// How often the run loop looks at the cancel flag while a process says nothing.
///
/// Cancel must not wait on the next line of output. FFVship 5.1.1 prints an out of
/// video memory error and then never exits, so a loop that blocks on a read can never
/// reach the flag again, and the run has no way to end. Measured on the real binary.
const CANCEL_POLL: Duration = Duration::from_millis(200);

/// How many of the process's own lines to keep for a failure message.
const KEPT_LINES: usize = 4;

/// Runs a real process. This is the only real implementation of `ProcessRunner`,
/// because this is the one place that starts a real process.
///
/// It reads the child's streams on their own threads rather than waiting for the whole
/// process. That is what gives the frame counter something to report, what gives Cancel
/// somewhere to act, and what keeps a full pipe from stopping the child.
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
        // A back end must never wait for something to be typed at it.
        command.stdin(Stdio::null());
        if let Some(cwd) = &invocation.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &invocation.env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .map_err(|error| vqa_core::CoreError::parse("process", error.to_string()))?;

        // Both streams are read, and both carry progress. FFmpeg writes `frame=N` to
        // its error stream because of `-progress pipe:2`. FFVship writes an index and a
        // score for each frame to its output stream because of `--live-score-output`.
        // A stream that nobody reads also fills its pipe and stops the child, so each
        // one is drained whether or not it carries a number.
        let (sender, receiver) = channel();
        if let Some(stream) = child.stdout.take() {
            read_lines(stream, sender.clone());
        }
        if let Some(stream) = child.stderr.take() {
            read_lines(stream, sender.clone());
        }
        drop(sender);

        // The same thread reads and kills, so the child needs no lock.
        let mut cancelled = false;
        let mut said: VecDeque<String> = VecDeque::new();
        loop {
            match receiver.recv_timeout(CANCEL_POLL) {
                Ok(line) => match frame_of(&line) {
                    Some(frame) => on_progress(Progress { frame }),
                    None => keep_line(&mut said, line),
                },
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                cancelled = true;
                break;
            }
        }

        let status = child
            .wait()
            .map_err(|error| vqa_core::CoreError::parse("process", error.to_string()))?;

        Ok(vqa_core::backend::ExitReport {
            succeeded: status.success() && !cancelled,
            wall_time_ms: start_time.elapsed().as_millis() as u64,
            message: message_of(&said),
        })
    }
}

/// Sends every line of one stream, and leaves once the stream ends.
fn read_lines<S: std::io::Read + Send + 'static>(stream: S, sender: Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
}

/// Keeps the last few things a process said, and drops what it has no room for.
///
/// The end of the stream is the part worth keeping, because whatever banner the back
/// end opened with is not why it stopped. A line the process already said is dropped:
/// FFVship reports the same failure once for each of its own threads, and the repeats
/// push the first and clearest copy out of the window.
fn keep_line(said: &mut VecDeque<String>, line: String) {
    let line = line.trim();
    if line.is_empty() || said.iter().any(|kept| kept == line) {
        return;
    }
    said.push_back(line.to_string());
    while said.len() > KEPT_LINES {
        said.pop_front();
    }
}

/// Joins what a process said into one line for the reader.
///
/// Every kept line goes in, not just the final one. FFVship spreads one failure over
/// three lines and puts the source file last, so the final line alone names a file in
/// somebody else's build folder and never the reason. Measured on the real binary.
fn message_of(said: &VecDeque<String>) -> Option<String> {
    if said.is_empty() {
        return None;
    }
    Some(said.iter().map(String::as_str).collect::<Vec<_>>().join(" "))
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
    /// One invocation has begun. The interface names the metrics that are running now,
    /// so a measurement that already landed never keeps the label and reads as the
    /// stalled one.
    Started {
        encode: FileId,
        metric: MetricId,
    },
    Progress {
        encode: FileId,
        metric: MetricId,
        frame: u64,
    },
    /// One invocation has stopped, whether it gave a value or not.
    ItemDone {
        encode: FileId,
        metric: MetricId,
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

/// The metric that names an invocation while it runs. One invocation can measure
/// several metrics together, and the first is the one the interface names.
fn leading_metric(invocation: &Invocation) -> Option<MetricId> {
    invocation.expects.first()?.metrics.first().copied()
}

/// Counts one invocation off its encode, and reports the encode once nothing is left.
fn finish_item(
    item: &WorkItem,
    remaining_by_encode: &Arc<Mutex<HashMap<FileId, usize>>>,
    sender: &Sender<SupervisorEvent>,
) {
    if let Some(metric) = leading_metric(&item.invocation) {
        let _ = sender.send(SupervisorEvent::ItemDone {
            encode: item.encode,
            metric,
        });
    }

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
    let leading = leading_metric(&item.invocation);
    if let Some(metric) = leading {
        let _ = sender.send(SupervisorEvent::Started { encode, metric });
    }

    let mut on_progress = |progress: Progress| {
        if let Some(metric) = leading {
            let _ = sender.send(SupervisorEvent::Progress {
                encode,
                metric,
                frame: progress.frame,
            });
        }
    };

    let outcome = runner.run(&item.invocation, &mut on_progress);
    let exit_report = match outcome {
        Ok(report) if report.succeeded => report,
        Ok(report) => {
            let _ = sender.send(SupervisorEvent::Failed {
                encode,
                error: failure_text(item, report.message),
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

/// What a run that did not finish tells the reader.
///
/// The words of the back end come first when it left any, because a back end names the
/// thing to change and this tool cannot guess it.
fn failure_text(item: &WorkItem, message: Option<String>) -> String {
    let program = item.invocation.binary.display_name();
    match message {
        Some(said) => format!("{program} did not finish: {said}"),
        None => format!("{program} did not finish, and said nothing."),
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
                    message: None,
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

    /// A program that runs for a long time and says nothing at all, for the cancel
    /// test. `waitfor` waits for a signal that never comes; `sleep` just waits.
    fn quiet_long_program() -> (&'static str, Vec<&'static str>) {
        if cfg!(windows) {
            ("waitfor", vec!["/t", "20", "VqaCancelTest"])
        } else {
            ("sleep", vec!["20"])
        }
    }

    /// The regression test for a run that could not be stopped.
    ///
    /// FFVship 5.1.1 prints an out of video memory error and then never exits. Reading
    /// the stream line by line parked the loop inside a read, so the cancel flag was
    /// never looked at again and the run had no way to end. Cancel must not wait on a
    /// process that has stopped speaking.
    #[test]
    fn cancel_stops_a_process_that_says_nothing_and_does_not_exit() {
        let (program, args) = quiet_long_program();
        let invocation = Invocation {
            program: PathBuf::from(program),
            args: args.iter().map(Into::into).collect(),
            env: Vec::new(),
            cwd: None,
            expects: Vec::new(),
            lane: LaneKind::Cpu,
            binary: BinaryId::Ffmpeg,
        };

        let cancel = Arc::new(AtomicBool::new(true));
        let runner = RealProcessRunner::with_cancel(cancel);
        let start = std::time::Instant::now();
        let Ok(report) = runner.run(&invocation, &mut |_| {}) else {
            // This machine has no such program. The rule under test needs one.
            return;
        };

        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "cancel must not wait for the process to end on its own"
        );
        assert!(!report.succeeded);
    }

    #[test]
    fn the_last_words_of_a_process_reach_the_failure_message() {
        let mut said = VecDeque::new();
        for line in ["banner", "", "  second  ", "third", "fourth", "fifth"] {
            keep_line(&mut said, line.to_string());
        }
        assert_eq!(said.len(), KEPT_LINES);
        assert_eq!(
            message_of(&said).as_deref(),
            Some("second third fourth fifth")
        );
        assert!(
            !message_of(&said).unwrap().contains("banner"),
            "an opening banner is not the reason a back end failed"
        );
        assert_eq!(message_of(&VecDeque::new()), None);
    }

    /// What FFVship 5.1.1 really writes on a card that cannot hold the metric, read off
    /// the running binary. It says the same three lines once for each of its threads,
    /// and it puts the source file last, so neither the final line on its own nor a
    /// sliding window of repeats carries the reason.
    #[test]
    fn the_reason_a_back_end_gives_survives_repeats_and_a_trailing_source_line() {
        let mut said = VecDeque::new();
        for _ in 0..2 {
            for line in [
                "error: VshipException",
                "OutOfVRAM: Vship was not able to perform GPU memory allocation. (Advice) Reduce or Set numStream argument",
                " - At line 139 of C:\\Users\\Line\\Documents\\randomgit\\Vship\\src\\HIP\\ssimu2\\main.hpp",
            ] {
                keep_line(&mut said, line.to_string());
            }
        }

        let message = message_of(&said).expect("the process said something");
        assert!(
            message.starts_with("error: VshipException OutOfVRAM:"),
            "the reason must lead, not a source file left over from a repeat: {message}"
        );
        assert!(message.contains("Reduce or Set numStream argument"), "{message}");
        assert_eq!(
            message.matches("At line 139").count(),
            1,
            "one failure said twice is still one failure: {message}"
        );
    }

    #[test]
    fn a_failure_names_the_binary_and_repeats_what_it_said() {
        let stats_path = write_fixture("named_failure.log", "n:1 psnr_y:40.0\n");
        let mut work = psnr_invocation(FileId(11), stats_path);
        work.invocations[0].binary = BinaryId::Ffvship;
        let item = WorkItem {
            encode: FileId(11),
            invocation: work.invocations.remove(0),
        };

        let text = failure_text(&item, Some("OutOfVRAM: no GPU memory".to_string()));
        assert!(text.contains("FFVship"), "{text}");
        assert!(text.contains("OutOfVRAM: no GPU memory"), "{text}");

        let silent = failure_text(&item, None);
        assert!(silent.contains("FFVship"), "{silent}");
    }

    #[test]
    fn every_invocation_reports_that_it_started_and_that_it_stopped() {
        let stats_path = write_fixture("started_stopped.log", "n:1 psnr_y:40.0\n");
        let work = vec![psnr_invocation(FileId(5), stats_path)];
        let receiver = run_plan(work, Arc::new(FakeRunner { succeed: true }), 1, 1);

        let mut order = Vec::new();
        for event in receiver.iter() {
            match event {
                SupervisorEvent::Started { metric, .. } => order.push(("started", metric)),
                SupervisorEvent::ItemDone { metric, .. } => order.push(("done", metric)),
                _ => {}
            }
        }
        assert_eq!(
            order,
            vec![
                ("started", MetricId::PsnrY),
                ("done", MetricId::PsnrY),
            ]
        );
    }

    #[test]
    fn a_cancelled_item_that_never_ran_still_reports_that_it_stopped() {
        let stats_path = write_fixture("cancel_item_done.log", "n:1 psnr_y:40.0\n");
        let work = vec![psnr_invocation(FileId(6), stats_path)];
        let cancel = Arc::new(AtomicBool::new(true));
        let receiver =
            run_plan_with_cancel(work, Arc::new(FakeRunner { succeed: true }), 1, 1, cancel);

        let mut started = 0;
        let mut done = 0;
        for event in receiver.iter() {
            match event {
                SupervisorEvent::Started { .. } => started += 1,
                SupervisorEvent::ItemDone { .. } => done += 1,
                _ => {}
            }
        }
        assert_eq!(started, 0, "a cancelled run starts nothing");
        assert_eq!(done, 1, "the title bar must not keep naming it");
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
