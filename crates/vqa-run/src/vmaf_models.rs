//! Finds and loads the VMAF v1 model files.
//!
//! The standard and the high-frame-rate models live in two separate folders, each
//! holding the same four viewing-distance and display-height variants. The tool picks
//! the folder from the reference's own frame rate, then reads every model inside it.

use std::path::{Path, PathBuf};
use vqa_backends::vmaf_model::read_model_folder;
use vqa_core::media::Rational;
use vqa_core::vmaf_model::VmafModel;

/// Common install locations for the VMAF v1 model package, checked in order after the
/// repo-relative path CLAUDE.md names. The tool never assumes only one of these is
/// right, since a distro package and a hand-installed copy can both exist.
const COMMON_SYSTEM_LOCATIONS: [&str; 2] = ["/usr/share/model", "/usr/local/share/model"];

/// Finds the folder that holds `vmaf_v1.0.16` and `vmaf_v1.0.16_hfr`.
///
/// Checks the override first, then the repo-relative path, then a short list of
/// common system locations. Returns nothing when none of them hold both subfolders.
pub fn find_model_folder(override_path: Option<&Path>) -> Option<PathBuf> {
    let candidates = override_path
        .map(|path| vec![path.to_path_buf()])
        .unwrap_or_else(|| {
            let mut paths = vec![PathBuf::from("model")];
            paths.extend(COMMON_SYSTEM_LOCATIONS.iter().map(PathBuf::from));
            paths
        });

    candidates
        .into_iter()
        .find(|candidate| candidate.join("vmaf_v1.0.16").is_dir())
}

/// Loads every VMAF v1 model for the reference's frame rate bracket.
///
/// Returns an empty list when no model folder was found, or when the folder holds no
/// model files. A VMAF v1 metric ticked with an empty list simply does not run, and
/// the run reports why, the same way it reports any other missing back end.
pub fn load_models_for(model_folder: &Path, frame_rate: Rational) -> Vec<VmafModel> {
    let subfolder = if frame_rate.is_high_frame_rate() {
        "vmaf_v1.0.16_hfr"
    } else {
        "vmaf_v1.0.16"
    };
    read_model_folder(&model_folder.join(subfolder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_that_holds_no_model_folder_finds_nothing() {
        let temp = std::env::temp_dir().join("vqa-vmaf-model-search-test-empty");
        std::fs::create_dir_all(&temp).unwrap();
        assert!(find_model_folder(Some(&temp)).is_none());
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn an_override_that_holds_the_real_subfolder_is_found() {
        let temp = std::env::temp_dir().join("vqa-vmaf-model-search-test-real");
        std::fs::create_dir_all(temp.join("vmaf_v1.0.16")).unwrap();
        assert_eq!(find_model_folder(Some(&temp)), Some(temp.clone()));
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn loading_models_with_no_folder_gives_an_empty_list() {
        let missing = PathBuf::from("/does/not/exist/anywhere");
        assert!(load_models_for(&missing, Rational { num: 30, den: 1 }).is_empty());
    }

    #[test]
    fn a_high_frame_rate_reference_looks_in_the_hfr_subfolder() {
        let temp = std::env::temp_dir().join("vqa-vmaf-model-search-test-hfr");
        std::fs::create_dir_all(temp.join("vmaf_v1.0.16_hfr")).unwrap();
        std::fs::write(
            temp.join("vmaf_v1.0.16_hfr").join("model.json"),
            r#"{"model_dict":{"feature_names":["Cambi_feature_cambi_score"],"feature_opts_dicts":[{}]}}"#,
        )
        .unwrap();

        let models = load_models_for(
            &temp,
            Rational {
                num: 60000,
                den: 1001,
            },
        );
        assert_eq!(models.len(), 1);
        std::fs::remove_dir_all(&temp).ok();
    }
}
