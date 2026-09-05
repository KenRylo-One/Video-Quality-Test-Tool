use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use vqtt_backends::{ffmpeg, vship};
use vqtt_core::backend::{JobInput, MeasureJob};
use vqtt_core::corrections;
use vqtt_core::metric::MetricId;
use vqtt_core::pooling::Pooled;
use vqtt_core::set::FileId;
use vqtt_run::{EncodeWork, RealProcessRunner, Session, SupervisorEvent};

pub struct RunState {
    receiver: Receiver<SupervisorEvent>,
    encodes_remaining: usize,
    pub results: HashMap<(FileId, MetricId), Pooled>,
    /// Every per-frame value, for the plot.
    ///
    /// The worst realistic case is one hour at 60 fps, eight encodes and six metrics,
    /// which is about 41 MB. That sits well inside the 300 MB the design allows.
    pub series: HashMap<(FileId, MetricId), Vec<f32>>,
    pub failures: Vec<String>,
    /// Every correction and note for this run, as one flat, ready-to-show list.
    pub notes: Vec<String>,
    /// The same corrections and notes with their structure intact. The Notes section
    /// wants a sentence; the run record wants the id, the target and the detail.
    corrections: Vec<corrections::Correction>,
    structured_notes: Vec<corrections::Note>,
    /// Every command that ran, in the order the supervisor queued it.
    invocations: Vec<vqtt_run::InvocationRecord>,
    /// One job for each encode, kept so the frame viewer can extract a still through
    /// the same correction chain the measurement used.
    jobs: HashMap<FileId, MeasureJob>,
    /// The absolute frame number that sample zero of every series holds. The plot's
    /// horizontal axis reads real frame numbers, not offsets into a clamped range.
    pub first_frame: u64,
    pub frame_rate: vqtt_core::media::Rational,
    /// The measurements that are running now, and how far each has reached. A metric
    /// that already landed leaves this map, so a finished measurement never keeps the
    /// title bar and reads as the one holding the run up.
    running: BTreeMap<(FileId, MetricId), u64>,
    /// How many frames one metric covers, for the progress bar.
    pub total_frames: Option<u64>,
    /// The identity of this run, which names its export folder.
    pub run_id: String,
    pub started: String,
    /// The model that VMAF measured with, for the record.
    vmaf_model: Option<vqtt_core::vmaf_model::VmafModel>,
    metrics: Vec<MetricId>,
    frame_range: Option<(u64, u64)>,
    cancel: Arc<AtomicBool>,
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

        let now = std::time::SystemTime::now();
        let run_id = vqtt_run::record::run_id(now);
        let started = vqtt_run::record::timestamp(now);
        let work_dir = std::env::temp_dir()
            .join("vqtt-run")
            .join(vqtt_run::record::safe_name(&run_id));

        let vmaf_model_folder =
            vqtt_run::vmaf_models::find_model_folder(session.settings.vmaf_model_folder.as_deref());
        let vmaf_models = vmaf_model_folder
            .as_deref()
            .map(|folder| vqtt_run::vmaf_models::load_models_for(folder, reference.info.frame_rate))
            .unwrap_or_default();
        let vmaf_viewing_distance = session.settings.vmaf_viewing_distance;

        let mut work = Vec::new();
        let mut notes = Vec::new();
        let mut corrections_made = Vec::new();
        let mut structured_notes = Vec::new();
        let mut jobs: HashMap<FileId, MeasureJob> = HashMap::new();

        let wants_vmaf_v1 =
            metrics.contains(&MetricId::Vmaf) || metrics.contains(&MetricId::VmafV1Cambi);
        if wants_vmaf_v1 && vmaf_models.is_empty() {
            notes.push(
                "No VMAF v1 model was found. VMAF v1 and CAMBI in VMAF v1 did not run for this comparison."
                    .to_string(),
            );
        }

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
                vmaf_models: vmaf_models.clone(),
                vmaf_viewing_distance,
                butteraugli_intensity_nits: session.settings.butteraugli_intensity_nits,
                vship_gpu_threads: session.settings.vship_gpu_threads,
            };

            let mut invocations = ffmpeg::plan(&job).ok()?;
            invocations.extend(vship::plan(&job).ok()?);
            if invocations.is_empty() {
                continue;
            }
            for invocation in &mut invocations {
                if let Some(found) = session.inventory.get(invocation.binary) {
                    invocation.program = found.path.clone();
                }
            }

            let sample = session.luma_extremes(&encode.info.path);
            let detected = corrections::detect_all(
                &reference.info,
                &encode.info,
                &encode.label,
                sample,
                &metrics,
                &vmaf_models,
                vmaf_viewing_distance,
            );
            notes.extend(detected.display_lines());
            let gap_notes = corrections::detect_vship_gap_notes(&metrics, &detected.corrections);
            notes.extend(gap_notes.iter().map(|note| note.message.clone()));
            structured_notes.extend(detected.notes.iter().cloned());
            structured_notes.extend(gap_notes);
            corrections_made.extend(detected.corrections);

            jobs.insert(encode.id, job);
            work.push(EncodeWork {
                encode: encode.id,
                invocations,
            });
        }

        if work.is_empty() {
            return None;
        }

        for note in corrections::known_metric_fault_notes(&metrics) {
            notes.push(note.message.clone());
            structured_notes.push(note);
        }

        let encodes_remaining = work.len();
        let cpu_lane_permits = session.settings.cpu_lane_permits as usize;
        let gpu_lane_permits = session.settings.gpu_lane_permits as usize;
        let cancel = Arc::new(AtomicBool::new(false));
        let receiver = vqtt_run::run_plan_with_cancel(
            work,
            Arc::new(RealProcessRunner::with_cancel(cancel.clone())),
            cpu_lane_permits,
            gpu_lane_permits,
            cancel.clone(),
        );

        Some(Self {
            receiver,
            encodes_remaining,
            results: HashMap::new(),
            series: HashMap::new(),
            failures: Vec::new(),
            notes,
            corrections: corrections_made,
            structured_notes,
            invocations: Vec::new(),
            jobs,
            first_frame: frame_range.map_or(0, |(first, _)| first),
            frame_rate: reference.info.frame_rate,
            running: BTreeMap::new(),
            total_frames: measured_frame_count(reference.info.nb_frames, frame_range),
            run_id,
            started,
            vmaf_model: vqtt_core::vmaf_model::choose_model(
                &vmaf_models,
                reference.info.height,
                vmaf_viewing_distance,
            )
            .cloned(),
            metrics: metrics.iter().copied().collect(),
            frame_range,
            cancel,
        })
    }

    /// Reads every event that is ready without waiting. Returns true when a run is
    /// still in progress, so the caller knows whether to poll again.
    pub fn poll(&mut self) -> bool {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                SupervisorEvent::Started { encode, metric } => {
                    self.running.insert((encode, metric), 0);
                }
                SupervisorEvent::Progress {
                    encode,
                    metric,
                    frame,
                } => {
                    self.running.insert((encode, metric), frame);
                }
                SupervisorEvent::ItemDone { encode, metric } => {
                    self.running.remove(&(encode, metric));
                }
                SupervisorEvent::Ran { record, .. } => {
                    self.invocations.push(record);
                }
                SupervisorEvent::MetricReady {
                    encode,
                    metric,
                    pooled,
                    series,
                } => {
                    self.results.insert((encode, metric), pooled);
                    self.series.insert((encode, metric), series);
                }
                SupervisorEvent::EncodeDone { .. } => {
                    self.encodes_remaining = self.encodes_remaining.saturating_sub(1);
                }
                SupervisorEvent::Failed { error, .. } => {
                    self.failures.push(error);
                }
            }
        }
        self.is_running()
    }

    /// True until every encode has finished, one way or another.
    pub fn is_running(&self) -> bool {
        self.encodes_remaining > 0
    }

    /// Stops the run. Queued work is dropped, and every running process is killed
    /// within a fifth of a second, whether it is still saying anything or not.
    /// Whatever already finished stays on screen.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Every line the reader needs about this run: what the tool corrected, then every
    /// back end that gave no number and what it said about it.
    ///
    /// A failure that reaches nobody is the same as no failure at all. The reader is
    /// left comparing a table with a column missing and no reason for it.
    pub fn note_lines(&self) -> Vec<String> {
        let mut lines = self.notes.clone();
        for failure in &self.failures {
            if !lines.contains(failure) {
                lines.push(failure.clone());
            }
        }
        lines
    }

    /// True once the run has anything at all to show.
    pub fn has_something_to_say(&self) -> bool {
        !self.results.is_empty() || !self.failures.is_empty()
    }

    /// What the title bar names: the measurement holding the run up, how far it has
    /// reached, and how many others run beside it.
    ///
    /// The one that has reached the fewest frames is the honest answer to what the run
    /// is waiting on, and it is the one still there when every other lane has finished.
    pub fn current(&self) -> Option<(MetricId, u64, usize)> {
        let ((_, metric), frame) = self.running.iter().min_by_key(|(_, frame)| **frame)?;
        Some((*metric, *frame, self.running.len() - 1))
    }

    /// How far the run has reached, from 0.0 to 1.0, when the frame count is known.
    pub fn fraction(&self) -> Option<f32> {
        let (_, frame, _) = self.current()?;
        let total = self.total_frames?;
        if total == 0 {
            return None;
        }
        Some((frame as f32 / total as f32).clamp(0.0, 1.0))
    }

    /// The job of one encode, so the frame viewer can extract through the same
    /// correction chain the measurement used.
    pub fn job_for(&self, encode: FileId) -> Option<MeasureJob> {
        self.jobs.get(&encode).cloned()
    }

    /// The frames of one measurement, worst first.
    pub fn worst_frames(
        &self,
        encode: FileId,
        metric: MetricId,
    ) -> Vec<vqtt_core::frames::FrameValue> {
        let Some(values) = self.series.get(&(encode, metric)) else {
            return Vec::new();
        };
        vqtt_core::frames::worst_frames(values, self.first_frame, metric.def().direction)
    }

    /// Everything the export needs, taken from a run that has stopped.
    ///
    /// The invocation order is the order the supervisor queued the work, not the order
    /// the lanes happened to finish, so the log reads the way the plan does.
    pub fn outcome(&self, theme: vqtt_core::palette::Theme) -> vqtt_run::RunOutcome {
        let mut invocations = self.invocations.clone();
        invocations.sort_by_key(|record| record.seq);

        vqtt_run::RunOutcome {
            run_id: self.run_id.clone(),
            started: self.started.clone(),
            finished: vqtt_run::record::timestamp(std::time::SystemTime::now()),
            metrics: self.metrics.clone(),
            frame_range: self.frame_range,
            first_frame: self.first_frame,
            results: self.results.clone(),
            series: self.series.clone(),
            corrections: self.corrections.clone(),
            notes: self.structured_notes.clone(),
            invocations,
            vmaf_model: self.vmaf_model.clone(),
            theme,
        }
    }

    #[cfg(test)]
    fn for_test(encodes_remaining: usize, receiver: Receiver<SupervisorEvent>) -> Self {
        Self {
            receiver,
            encodes_remaining,
            results: HashMap::new(),
            series: HashMap::new(),
            failures: Vec::new(),
            notes: Vec::new(),
            corrections: Vec::new(),
            structured_notes: Vec::new(),
            invocations: Vec::new(),
            jobs: HashMap::new(),
            first_frame: 0,
            frame_rate: vqtt_core::media::Rational { num: 25, den: 1 },
            running: BTreeMap::new(),
            total_frames: None,
            run_id: "test-run".to_string(),
            started: "test".to_string(),
            vmaf_model: None,
            metrics: Vec::new(),
            frame_range: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// How many frames one metric actually measures, once the range selection applies.
fn measured_frame_count(nb_frames: Option<u64>, frame_range: Option<(u64, u64)>) -> Option<u64> {
    match frame_range {
        Some((first, last)) if last >= first => Some(last - first + 1),
        Some(_) => None,
        None => nb_frames,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vqtt_core::metric::HarmonicMean;
    use vqtt_core::pooling::pool;

    #[test]
    fn is_running_stays_true_while_an_encode_is_still_working() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut state = RunState::for_test(2, receiver);

        let pooled = pool(&[1.0], HarmonicMean::Allowed).unwrap();
        sender
            .send(SupervisorEvent::MetricReady {
                encode: FileId(1),
                metric: MetricId::PsnrY,
                pooled,
                series: vec![1.0],
            })
            .unwrap();
        sender
            .send(SupervisorEvent::EncodeDone { encode: FileId(1) })
            .unwrap();

        assert!(state.poll());
        assert!(state.is_running());
    }

    #[test]
    fn a_finished_metric_leaves_its_per_frame_series_behind_for_the_plot() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut state = RunState::for_test(1, receiver);

        let values = vec![40.0f32, 41.0, 42.0];
        let pooled = pool(&values, HarmonicMean::Allowed).unwrap();
        sender
            .send(SupervisorEvent::MetricReady {
                encode: FileId(7),
                metric: MetricId::PsnrY,
                pooled,
                series: values.clone(),
            })
            .unwrap();
        sender
            .send(SupervisorEvent::EncodeDone { encode: FileId(7) })
            .unwrap();
        state.poll();

        assert_eq!(
            state.series.get(&(FileId(7), MetricId::PsnrY)),
            Some(&values)
        );
    }

    /// The regression test for a title bar that named the wrong back end.
    ///
    /// A real run measured CAMBI to its last frame and then waited on FFVship, which
    /// had stalled. The bar still read "CAMBI · frame 150", so the metric that had
    /// already finished looked like the one that was stuck.
    #[test]
    fn a_finished_metric_stops_naming_the_title_bar() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut state = RunState::for_test(1, receiver);

        for event in [
            SupervisorEvent::Started {
                encode: FileId(1),
                metric: MetricId::Cambi,
            },
            SupervisorEvent::Started {
                encode: FileId(1),
                metric: MetricId::Ssimulacra2,
            },
            SupervisorEvent::Progress {
                encode: FileId(1),
                metric: MetricId::Cambi,
                frame: 150,
            },
        ] {
            sender.send(event).unwrap();
        }
        state.poll();
        assert_eq!(
            state.current(),
            Some((MetricId::Ssimulacra2, 0, 1)),
            "the metric with no frames yet is the one holding the run up"
        );

        sender
            .send(SupervisorEvent::ItemDone {
                encode: FileId(1),
                metric: MetricId::Cambi,
            })
            .unwrap();
        state.poll();
        assert_eq!(state.current(), Some((MetricId::Ssimulacra2, 0, 0)));
    }

    #[test]
    fn nothing_running_leaves_the_title_bar_with_no_metric_to_name() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut state = RunState::for_test(1, receiver);

        sender
            .send(SupervisorEvent::Started {
                encode: FileId(1),
                metric: MetricId::PsnrY,
            })
            .unwrap();
        sender
            .send(SupervisorEvent::ItemDone {
                encode: FileId(1),
                metric: MetricId::PsnrY,
            })
            .unwrap();
        state.poll();

        assert_eq!(state.current(), None);
        assert_eq!(state.fraction(), None);
    }

    #[test]
    fn is_running_turns_false_once_every_encode_is_done() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut state = RunState::for_test(2, receiver);

        sender
            .send(SupervisorEvent::EncodeDone { encode: FileId(1) })
            .unwrap();
        sender
            .send(SupervisorEvent::EncodeDone { encode: FileId(2) })
            .unwrap();

        assert!(!state.poll());
        assert!(!state.is_running());
    }

    #[test]
    fn a_failed_encode_still_counts_toward_encode_done() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut state = RunState::for_test(1, receiver);

        sender
            .send(SupervisorEvent::Failed {
                encode: FileId(1),
                error: "no ffmpeg".to_string(),
            })
            .unwrap();
        sender
            .send(SupervisorEvent::EncodeDone { encode: FileId(1) })
            .unwrap();

        assert!(!state.poll());
        assert_eq!(state.failures.len(), 1);
    }

    /// A real `Session`, real files, and a real `RunState`. Skips, and does not fail,
    /// when this machine has neither `ffmpeg` nor the test media.
    #[test]
    fn a_real_run_reports_the_color_range_note() {
        let media_folder =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Test-Media");
        let reference_path = media_folder.join("TEST_B_limited_range_flagged_tv.mp4");
        let encode_path = media_folder.join("TEST_A_full_range_flagged_pc.mp4");
        if !reference_path.is_file() || !encode_path.is_file() {
            return;
        }

        let mut session = vqtt_run::Session::with_settings(
            vqtt_run::Settings::default(),
            vqtt_run::CapabilityCache::new(),
        );
        if !session.inventory.has(vqtt_core::BinaryId::Ffmpeg)
            || !session.inventory.has(vqtt_core::BinaryId::Ffprobe)
        {
            return;
        }

        session.add_file(&reference_path);
        session.add_file(&encode_path);
        session.toggle_metric(MetricId::PsnrY, true);

        let Some(mut state) = RunState::start(&session) else {
            panic!("a reference, an encode, and a runnable metric are all present");
        };

        assert!(
            state
                .notes
                .iter()
                .any(|line| line.starts_with("Color range:")),
            "TEST_A against TEST_B must name the color range correction"
        );

        let encode_id = session.files.encodes().next().unwrap().id;
        while state.poll() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(state.results.contains_key(&(encode_id, MetricId::PsnrY)));
    }

    /// A real run of standalone CAMBI through `RunState`, the same layer the GUI
    /// calls. CAMBI needs no VMAF model file, so it reaches a result on every machine
    /// with a `libvmaf`-enabled `ffmpeg`, unlike VMAF v1.
    #[test]
    fn a_real_run_measures_cambi_with_no_model_folder_needed() {
        let media_folder =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Test-Media");
        let reference_path = media_folder.join("TEST_B_limited_range_flagged_tv.mp4");
        let encode_path = media_folder.join("TEST_A_full_range_flagged_pc.mp4");
        if !reference_path.is_file() || !encode_path.is_file() {
            return;
        }

        let mut session = vqtt_run::Session::with_settings(
            vqtt_run::Settings::default(),
            vqtt_run::CapabilityCache::new(),
        );
        if !session.inventory.has(vqtt_core::BinaryId::Ffmpeg)
            || !session.inventory.has(vqtt_core::BinaryId::Ffprobe)
        {
            return;
        }

        session.add_file(&reference_path);
        session.add_file(&encode_path);
        // Isolate this test to Cambi. `Vmaf` is a default tick too, and this machine's
        // real libvmaf build cannot load a real v1.0.16 model, which is a separate,
        // already-known environment gap and not what this test checks.
        session.selection.metrics.clear();
        session.toggle_metric(MetricId::Cambi, true);
        if session.runnable_metrics().is_empty() {
            return;
        }

        let Some(mut state) = RunState::start(&session) else {
            panic!("a reference, an encode, and a runnable metric are all present");
        };

        let encode_id = session.files.encodes().next().unwrap().id;
        while state.poll() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(state.results.contains_key(&(encode_id, MetricId::Cambi)));
        assert!(
            state.failures.is_empty(),
            "a real Cambi run must not fail: {:?}",
            state.failures
        );
    }

    /// A real run of SSIMULACRA 2 through FFVship, the same layer the GUI calls.
    /// FFVship is not on `PATH` on the development machine, so this also checks
    /// `VQTT_FFVSHIP_PATH` before skipping. Proves both that a real number lands in
    /// `state.results` and that the color range gap note names FFVship's own gap.
    #[test]
    fn a_real_run_measures_ssimulacra2_and_reports_the_vship_gap_note() {
        let media_folder =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Test-Media");
        let reference_path = media_folder.join("TEST_B_limited_range_flagged_tv.mp4");
        let encode_path = media_folder.join("TEST_A_full_range_flagged_pc.mp4");
        if !reference_path.is_file() || !encode_path.is_file() {
            return;
        }

        let mut settings = vqtt_run::Settings::default();
        if let Ok(path) = std::env::var("VQTT_FFVSHIP_PATH") {
            settings.set_binary_path(
                vqtt_core::BinaryId::Ffvship,
                Some(std::path::PathBuf::from(path)),
            );
        }
        let mut session =
            vqtt_run::Session::with_settings(settings, vqtt_run::CapabilityCache::new());
        if !session.inventory.has(vqtt_core::BinaryId::Ffmpeg)
            || !session.inventory.has(vqtt_core::BinaryId::Ffprobe)
            || !session.inventory.has(vqtt_core::BinaryId::Ffvship)
        {
            return;
        }

        session.add_file(&reference_path);
        session.add_file(&encode_path);
        session.selection.metrics.clear();
        session.toggle_metric(MetricId::Ssimulacra2, true);
        if session.runnable_metrics().is_empty() {
            return;
        }

        let Some(mut state) = RunState::start(&session) else {
            panic!("a reference, an encode, and a runnable metric are all present");
        };

        assert!(
            state
                .notes
                .iter()
                .any(|line| line.contains("FFVship measured the files as delivered")),
            "TEST_A against TEST_B must carry the FFVship color range gap note"
        );

        // FFVship 5.1.1 prints an out of video memory error and then never exits, on a
        // card that cannot hold the metric. This test asks it to stop rather than wait
        // for a process that has already decided not to finish.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let encode_id = session.files.encodes().next().unwrap().id;
        while state.poll() {
            if std::time::Instant::now() > deadline {
                state.cancel();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            state
                .results
                .contains_key(&(encode_id, MetricId::Ssimulacra2)),
            "FFVship gave no SSIMULACRA 2 value: {:?}",
            state.failures
        );
    }

    /// A real measurement, exported the way the Export button exports it, then read
    /// back. This is the only test that walks the whole path: measure, pool, record,
    /// write, and open again.
    #[test]
    fn a_real_run_exports_a_folder_that_reads_back() {
        let media_folder =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Test-Media");
        let reference_path = media_folder.join("TEST_B_limited_range_flagged_tv.mp4");
        let encode_path = media_folder.join("TEST_A_full_range_flagged_pc.mp4");
        if !reference_path.is_file() || !encode_path.is_file() {
            return;
        }

        let mut session = vqtt_run::Session::with_settings(
            vqtt_run::Settings::default(),
            vqtt_run::CapabilityCache::new(),
        );
        if !session.inventory.has(vqtt_core::BinaryId::Ffmpeg)
            || !session.inventory.has(vqtt_core::BinaryId::Ffprobe)
        {
            return;
        }

        session.add_file(&reference_path);
        session.add_file(&encode_path);
        session.toggle_metric(MetricId::PsnrY, true);
        session.toggle_metric(MetricId::SsimAll, true);

        let Some(mut state) = RunState::start(&session) else {
            panic!("a reference, an encode, and a runnable metric are all present");
        };
        while state.poll() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let parent = std::env::temp_dir().join("vqtt-export-real");
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();

        let outcome = state.outcome(vqtt_core::palette::Theme::Dark);
        let exported =
            vqtt_run::write_run(&parent, &session, &outcome, crate::fonts::FACES).unwrap();

        for name in &exported.files {
            let path = exported.folder.join(name);
            let bytes = std::fs::metadata(&path).map(|data| data.len()).unwrap_or(0);
            assert!(bytes > 0, "{name} was written empty");
        }

        let text = std::fs::read_to_string(exported.folder.join("run.json")).unwrap();
        let record: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(record["schema"], 1);
        assert!(
            record["backends"]
                .as_array()
                .is_some_and(|list| !list.is_empty()),
            "the record must name the binary that measured"
        );
        assert!(
            record["invocations"]
                .as_array()
                .is_some_and(|list| !list.is_empty()),
            "the record must hold every command that ran"
        );

        let frames = exported
            .files
            .iter()
            .find(|name| name.starts_with("frames-"))
            .expect("one per-frame file for the encode");
        let wide = std::fs::read_to_string(exported.folder.join(frames)).unwrap();
        let header = wide.lines().next().unwrap();
        assert!(header.starts_with("frame,"), "{header}");
        assert!(header.contains("psnr_y") && header.contains("ssim_all"));
        assert_eq!(
            wide.lines().count(),
            151,
            "150 frames and one header row, on one time axis"
        );

        let svg = std::fs::read_to_string(exported.folder.join("graph-psnr_y.svg")).unwrap();
        assert!(svg.starts_with("<svg "));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains("<polyline"), "the graph must draw its line");
    }
}
