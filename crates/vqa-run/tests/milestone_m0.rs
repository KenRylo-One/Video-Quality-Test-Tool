//! The acceptance test of milestone M0.
//!
//! Each test skips when this machine has no `ffprobe` or no test media, because layer 4
//! runs locally.

use std::path::{Path, PathBuf};
use vqa_core::capability::BinaryId;
use vqa_core::media::ColorRange;
use vqa_core::metric::REGISTRY;
use vqa_run::{CapabilityCache, Session, Settings};

/// The folder that holds `TEST_A` and `TEST_B`.
fn media_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Test-Media")
}

/// A session that reads this machine, with the settings of a new user.
fn session() -> Session {
    Session::with_settings(Settings::default(), CapabilityCache::new())
}

/// True when this machine can read media information.
fn can_probe(session: &Session) -> bool {
    session.inventory.has(BinaryId::Ffprobe) && media_folder().is_dir()
}

#[test]
fn test_1_dropping_test_a_shows_color_range_pc() {
    let mut session = session();
    if !can_probe(&session) {
        return;
    }

    let id = session.add_file(&media_folder().join("TEST_A_full_range_flagged_pc.mp4"));
    let file = session.files.get(id).unwrap();
    assert_eq!(file.info.color_range, ColorRange::Pc);
    assert_eq!(file.info.color_range.tag(), "pc");
}

#[test]
fn test_2_dropping_test_b_shows_color_range_tv() {
    let mut session = session();
    if !can_probe(&session) {
        return;
    }

    let id = session.add_file(&media_folder().join("TEST_B_limited_range_flagged_tv.mp4"));
    let file = session.files.get(id).unwrap();
    assert_eq!(file.info.color_range, ColorRange::Tv);
    assert_eq!(file.info.color_range.tag(), "tv");
}

#[test]
fn test_3_four_files_give_four_rows_and_the_first_is_the_reference() {
    let mut session = session();
    if !can_probe(&session) {
        return;
    }

    let a = media_folder().join("TEST_A_full_range_flagged_pc.mp4");
    let b = media_folder().join("TEST_B_limited_range_flagged_tv.mp4");
    let first = session.add_file(&a);
    session.add_file(&b);
    session.add_file(&a);
    session.add_file(&b);

    assert_eq!(session.files.len(), 4);
    assert_eq!(session.files.encode_count(), 3);
    assert_eq!(session.files.reference_id(), Some(first));

    // The reference is TEST_A, so every TEST_B row carries a color range mark.
    let marks: Vec<bool> = session
        .files
        .encodes()
        .map(|file| session.files.diff_marks(file.id).color_range)
        .collect();
    assert_eq!(marks, vec![true, false, true]);
}

#[test]
fn test_4_the_metric_list_shows_every_metric_and_names_the_missing_back_end() {
    let session = session();

    for def in REGISTRY {
        let state = session.availability(def.id);
        if !state.is_available() {
            let reason = state
                .reason
                .expect("a disabled metric always gives a reason");
            assert!(!reason.is_empty(), "{} gave an empty reason", def.id.key());
        }
    }
}

#[test]
fn test_5_with_no_binaries_the_tool_opens_and_teaches() {
    // A new user has no binary and no settings file. Nothing here reports an error.
    let mut settings = Settings::default();
    for id in BinaryId::ALL {
        settings.set_binary_path(id, Some(PathBuf::from("/no/such/binary")));
    }
    let session = Session::with_settings(settings, CapabilityCache::new());

    assert!(session.inventory.is_empty());
    assert!(session.selection.metrics.is_empty());
    assert!(session.estimate().is_none());

    for id in BinaryId::ALL {
        assert!(
            !id.source().is_empty(),
            "{} names no source",
            id.display_name()
        );
        assert!(
            !id.provides().is_empty(),
            "{} names nothing that it gives",
            id.display_name()
        );
    }

    for def in REGISTRY {
        let state = session.availability(def.id);
        assert!(!state.is_available());
        assert!(state.reason.is_some());
    }
}

#[test]
fn a_file_that_ffprobe_cannot_read_still_gives_a_row() {
    let mut session = session();
    if !can_probe(&session) {
        return;
    }

    let id = session.add_file(&media_folder().join("check_levels.sh"));
    assert!(session.files.get(id).is_some());
    assert_eq!(session.files.len(), 1);
    assert!(!session.probe_problems.is_empty());
}
