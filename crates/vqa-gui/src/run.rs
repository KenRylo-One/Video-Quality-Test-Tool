use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use vqa_backends::{ffmpeg, vship};
use vqa_core::backend::{JobInput, MeasureJob};
use vqa_core::corrections;
use vqa_core::metric::MetricId;
use vqa_core::pooling::Pooled;
use vqa_core::set::FileId;
use vqa_run::{EncodeWork, RealProcessRunner, Session, SupervisorEvent, run_plan};

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
    /// The absolute frame number that sample zero of every series holds. The plot's
    /// horizontal axis reads real frame numbers, not offsets into a clamped range.
    pub first_frame: u64,
    pub frame_rate: vqa_core::media::Rational,
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

        let vmaf_model_folder =
            vqa_run::vmaf_models::find_model_folder(session.settings.vmaf_model_folder.as_deref());
        let vmaf_models = vmaf_model_folder
            .as_deref()
            .map(|folder| vqa_run::vmaf_models::load_models_for(folder, reference.info.frame_rate))
            .unwrap_or_default();
        let vmaf_viewing_distance = session.settings.vmaf_viewing_distance;

        let mut work = Vec::new();
        let mut notes = Vec::new();

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
            notes.extend(
                corrections::detect_vship_gap_notes(&metrics, &detected.corrections)
                    .into_iter()
                    .map(|note| note.message),
            );

            work.push(EncodeWork {
                encode: encode.id,
                invocations,
            });
        }

        if work.is_empty() {
            return None;
        }

        for note in corrections::known_metric_fault_notes(&metrics) {
            notes.push(note.message);
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
            series: HashMap::new(),
            failures: Vec::new(),
            notes,
            first_frame: frame_range.map_or(0, |(first, _)| first),
            frame_rate: reference.info.frame_rate,
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

    #[cfg(test)]
    fn for_test(encodes_remaining: usize, receiver: Receiver<SupervisorEvent>) -> Self {
        Self {
            receiver,
            encodes_remaining,
            results: HashMap::new(),
            series: HashMap::new(),
            failures: Vec::new(),
            notes: Vec::new(),
            first_frame: 0,
            frame_rate: vqa_core::media::Rational { num: 25, den: 1 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vqa_core::metric::HarmonicMean;
    use vqa_core::pooling::pool;

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

        let mut session = vqa_run::Session::with_settings(
            vqa_run::Settings::default(),
            vqa_run::CapabilityCache::new(),
        );
        if !session.inventory.has(vqa_core::BinaryId::Ffmpeg)
            || !session.inventory.has(vqa_core::BinaryId::Ffprobe)
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

        let mut session = vqa_run::Session::with_settings(
            vqa_run::Settings::default(),
            vqa_run::CapabilityCache::new(),
        );
        if !session.inventory.has(vqa_core::BinaryId::Ffmpeg)
            || !session.inventory.has(vqa_core::BinaryId::Ffprobe)
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
    /// `VQA_FFVSHIP_PATH` before skipping. Proves both that a real number lands in
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

        let mut settings = vqa_run::Settings::default();
        if let Ok(path) = std::env::var("VQA_FFVSHIP_PATH") {
            settings.set_binary_path(
                vqa_core::BinaryId::Ffvship,
                Some(std::path::PathBuf::from(path)),
            );
        }
        let mut session =
            vqa_run::Session::with_settings(settings, vqa_run::CapabilityCache::new());
        if !session.inventory.has(vqa_core::BinaryId::Ffmpeg)
            || !session.inventory.has(vqa_core::BinaryId::Ffprobe)
            || !session.inventory.has(vqa_core::BinaryId::Ffvship)
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

        let encode_id = session.files.encodes().next().unwrap().id;
        while state.poll() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            state
                .results
                .contains_key(&(encode_id, MetricId::Ssimulacra2))
        );
    }
}
