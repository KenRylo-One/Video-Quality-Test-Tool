//! Writes one run to a folder.
//!
//! The three files of `06 - Data and Reports.md` travel together, and `run.json` names
//! its own CSV files by relative path. Version 1.0 has no run history view, so these
//! folders are the history.

use crate::csv_writer::{
    MetricColumn, SummaryRow, write_command_log, write_frame_csv, write_summary_csv,
};
use crate::graph_png::png_from_svg;
use crate::record::{RunOutcome, build, frame_csv_name, safe_name};
use crate::session::Session;
use std::path::{Path, PathBuf};
use vqa_core::media::Rational;
use vqa_core::metric::MetricId;
use vqa_core::plot::{PlotRequest, SeriesInput, build_scenes};
use vqa_core::plot_svg::to_svg;
use vqa_core::set::FileId;

/// The width and height a graph is exported at.
const GRAPH_SIZE: (f32, f32) = (1000.0, 420.0);

/// How much larger the PNG is than the SVG, so a raster graph stays readable when it is
/// pasted into a document.
const PNG_SCALE: f32 = 2.0;

/// Everything the export writes, for the message that follows it.
pub struct Exported {
    pub folder: PathBuf,
    pub files: Vec<String>,
}

/// Writes `run.json`, the two CSV shapes, the command log and a graph for each metric.
///
/// `fonts` are the faces the graph text is drawn with. With none, the PNG falls back to
/// whatever the rasterizer finds, and the SVG names the family for the reader to
/// resolve.
pub fn write_run(
    parent: &Path,
    session: &Session,
    outcome: &RunOutcome,
    fonts: &[&[u8]],
) -> vqa_core::Result<Exported> {
    let folder = parent.join(format!("vqa-{}", safe_name(&outcome.run_id)));
    std::fs::create_dir_all(&folder)
        .map_err(|error| vqa_core::CoreError::parse("export", error.to_string()))?;

    let mut files = Vec::new();

    let record = build(session, outcome);
    let json = serde_json::to_string_pretty(&record)
        .map_err(|error| vqa_core::CoreError::parse("export", error.to_string()))?;
    std::fs::write(folder.join("run.json"), json)
        .map_err(|error| vqa_core::CoreError::parse("export", error.to_string()))?;
    files.push("run.json".to_string());

    for encode in session.files.encodes() {
        let columns: Vec<MetricColumn> = outcome
            .metrics
            .iter()
            .filter_map(|metric| {
                outcome
                    .series
                    .get(&(encode.id, *metric))
                    .map(|values| MetricColumn {
                        metric: *metric,
                        values,
                    })
            })
            .collect();
        if columns.is_empty() {
            continue;
        }
        let name = frame_csv_name(&encode.label);
        write_frame_csv(&folder.join(&name), outcome.first_frame, &columns)?;
        files.push(name);
    }

    let rows = summary_rows(session, outcome);
    if !rows.is_empty() {
        write_summary_csv(&folder.join("summary.csv"), &outcome.run_id, &rows)?;
        files.push("summary.csv".to_string());
    }

    write_command_log(&folder.join("commands.txt"), outcome)?;
    files.push("commands.txt".to_string());

    files.extend(write_graphs(&folder, session, outcome, fonts));

    Ok(Exported { folder, files })
}

fn summary_rows(session: &Session, outcome: &RunOutcome) -> Vec<SummaryRow> {
    let mut rows = Vec::new();
    for encode in session.files.encodes() {
        for def in vqa_core::metric::REGISTRY.iter() {
            let Some(pooled) = outcome.results.get(&(encode.id, def.id)) else {
                continue;
            };
            rows.push(SummaryRow {
                encode: encode.label.clone(),
                bitrate_bps: encode.info.bit_rate,
                frames: outcome
                    .series
                    .get(&(encode.id, def.id))
                    .map_or(0, Vec::len),
                metric: def.id,
                pooled: *pooled,
            });
        }
    }
    rows
}

/// Draws one SVG and one PNG for each metric that has a result.
///
/// A graph that cannot be written is reported and skipped. An export that gives the
/// numbers is worth more than one that fails whole because a rasterizer complained.
fn write_graphs(
    folder: &Path,
    session: &Session,
    outcome: &RunOutcome,
    fonts: &[&[u8]],
) -> Vec<String> {
    let mut written = Vec::new();
    let frame_rate = session
        .files
        .reference()
        .map_or(Rational::ZERO, |file| file.info.frame_rate);

    for metric in &outcome.metrics {
        let names: Vec<(FileId, Option<usize>, String)> = session
            .files
            .encodes()
            .filter(|encode| outcome.series.contains_key(&(encode.id, *metric)))
            .map(|encode| {
                (
                    encode.id,
                    session.files.slot_of(encode.id),
                    encode.label.clone(),
                )
            })
            .collect();
        if names.is_empty() {
            continue;
        }

        let inputs: Vec<SeriesInput> = names
            .iter()
            .map(|(file, slot, label)| SeriesInput {
                file: *file,
                slot: *slot,
                name: label,
                values: outcome.series[&(*file, *metric)].as_slice(),
            })
            .collect();

        let request = PlotRequest {
            metric: *metric,
            series: &inputs,
            theme: outcome.theme,
            high_contrast: false,
            x_domain: (0.0, 1.0),
            first_frame: outcome.first_frame,
            frame_rate,
            size: GRAPH_SIZE,
            hover_frame: None,
        };
        let scenes = build_scenes(&request);
        let Some(scene) = scenes.first() else {
            continue;
        };

        let svg = to_svg(scene, outcome.theme);
        let stem = graph_stem(*metric);

        if std::fs::write(folder.join(format!("{stem}.svg")), &svg).is_ok() {
            written.push(format!("{stem}.svg"));
        }
        match png_from_svg(&svg, fonts, PNG_SCALE) {
            Ok(png) => {
                if std::fs::write(folder.join(format!("{stem}.png")), png).is_ok() {
                    written.push(format!("{stem}.png"));
                }
            }
            Err(error) => tracing::warn!(%error, metric = stem, "cannot rasterize the graph"),
        }
    }
    written
}

fn graph_stem(metric: MetricId) -> String {
    format!("graph-{}", metric.key())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CapabilityCache;
    use crate::record::InvocationRecord;
    use crate::settings::Settings;
    use std::collections::HashMap;
    use vqa_core::media::{ColorRange, MediaInfo};
    use vqa_core::palette::Theme;
    use vqa_core::pooling::pool;

    fn info(name: &str, bit_rate: Option<u64>) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from(name),
            bytes: 1024,
            codec: "h264".into(),
            profile: None,
            width: 1920,
            height: 1080,
            pix_fmt: "yuv420p".into(),
            bit_depth: 8,
            color_range: ColorRange::Tv,
            color_space: Some("bt709".into()),
            frame_rate: Rational { num: 60, den: 1 },
            nb_frames: Some(4),
            duration_s: Some(0.066),
            bit_rate,
            }
    }

    /// No binary scan: the export reads results the caller already holds, and hashing
    /// a full FFmpeg build would cost a minute for nothing.
    fn session_with_two_files() -> Session {
        let mut session = Session::new(Settings::default(), CapabilityCache::new());
        session.files.add(info("reference.mov", None));
        session.files.add(info("encode.mp4", Some(5_000_000)));
        session
    }

    fn outcome_for(session: &Session) -> RunOutcome {
        let encode = session.files.encodes().next().unwrap().id;
        let values = vec![40.0f32, 41.0, 39.0, 42.0];
        let mut results = HashMap::new();
        let mut series = HashMap::new();
        results.insert(
            (encode, MetricId::PsnrY),
            pool(&values, vqa_core::metric::HarmonicMean::Allowed).unwrap(),
        );
        series.insert((encode, MetricId::PsnrY), values);

        RunOutcome {
            run_id: "2026-09-05T01-02-03Z-abcd".into(),
            started: "2026-09-05T01:02:03Z".into(),
            finished: "2026-09-05T01:04:00Z".into(),
            metrics: vec![MetricId::PsnrY],
            frame_range: None,
            first_frame: 0,
            results,
            series,
            corrections: Vec::new(),
            notes: Vec::new(),
            invocations: vec![InvocationRecord {
                seq: 1,
                lane: "cpu",
                program: PathBuf::from("ffmpeg"),
                args: vec!["-i".into(), "encode.mp4".into()],
                cwd: None,
                exit_code: Some(0),
                wall_ms: 900,
            }],
            vmaf_model: None,
            theme: Theme::Dark,
        }
    }

    fn export_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("vqa-export-test").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_run_writes_the_record_both_csv_shapes_the_log_and_a_graph() {
        let session = session_with_two_files();
        let outcome = outcome_for(&session);

        let exported = write_run(&export_dir("full"), &session, &outcome, &[]).unwrap();

        assert!(exported.files.contains(&"run.json".to_string()));
        assert!(exported.files.contains(&"summary.csv".to_string()));
        assert!(exported.files.contains(&"commands.txt".to_string()));
        assert!(exported.files.contains(&"graph-psnr_y.svg".to_string()));
        assert!(exported.files.contains(&"graph-psnr_y.png".to_string()));
        assert!(
            exported
                .files
                .iter()
                .any(|name| name.starts_with("frames-") && name.ends_with(".csv"))
        );
        for name in &exported.files {
            assert!(exported.folder.join(name).exists(), "{name} is missing");
        }
    }

    #[test]
    fn the_record_reads_back_with_its_schema_and_names_its_own_csv_file() {
        let session = session_with_two_files();
        let outcome = outcome_for(&session);

        let exported = write_run(&export_dir("record"), &session, &outcome, &[]).unwrap();

        let text = std::fs::read_to_string(exported.folder.join("run.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(value["schema"], 1);
        assert_eq!(value["run_id"], "2026-09-05T01-02-03Z-abcd");
        assert_eq!(value["results"][0]["metric"], "psnr_y");
        assert_eq!(value["results"][0]["frames"], 4);
        let named = value["results"][0]["series"].as_str().unwrap().to_string();
        assert!(exported.folder.join(&named).exists());
        assert!(value["plan"]["measurement"]["width"].as_u64() == Some(1920));
        assert!(value["invocations"][0]["exit_code"] == 0);
    }

    #[test]
    fn a_folder_is_named_for_the_run_and_holds_everything_together() {
        let session = session_with_two_files();
        let outcome = outcome_for(&session);
        let parent = export_dir("named");

        let exported = write_run(&parent, &session, &outcome, &[]).unwrap();

        assert_eq!(
            exported.folder.file_name().unwrap(),
            "vqa-2026-09-05T01-02-03Z-abcd"
        );
        assert_eq!(exported.folder.parent().unwrap(), parent);
    }
}
