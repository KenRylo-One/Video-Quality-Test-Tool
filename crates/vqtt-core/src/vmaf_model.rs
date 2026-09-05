//! The VMAF model registry.
//!
//! A VMAF model file names its own reference display height and its own normalized
//! viewing distance. The tool reads those two values out of the file. It never infers
//! them from the file name, because a renamed or a new Netflix model must still work.

use std::path::PathBuf;

/// What the tool needs from one VMAF model file.
#[derive(Debug, Clone, PartialEq)]
pub struct VmafModel {
    /// Where the file lives.
    pub path: PathBuf,
    /// True when the model already holds `adm_enhn_gain_limit` at 1.0.
    ///
    /// Every VMAF v1 model is already NEG. There is no v1 NEG model, and the tool
    /// never offers one.
    pub is_v1: bool,
    /// The reference display height the model was trained for, in pixels.
    pub reference_display_height: u32,
    /// The normalized viewing distance the model was trained for, in picture heights.
    pub normalized_viewing_distance: f32,
}

/// Picks the model whose reference display height and viewing distance are the
/// closest match to the measurement.
///
/// The measurement height is the reference file's own height, because the tool scales
/// every encode up to match the reference before it measures. `models` should already
/// hold only the models for the wanted frame rate bracket, since the high-frame-rate
/// and the standard model files live in two separate folders with no field of their
/// own to tell them apart.
pub fn choose_model(
    models: &[VmafModel],
    measurement_height: u32,
    viewing_distance: f32,
) -> Option<&VmafModel> {
    models.iter().min_by(|left, right| {
        score(left, measurement_height, viewing_distance).total_cmp(&score(
            right,
            measurement_height,
            viewing_distance,
        ))
    })
}

/// How far one model sits from the measurement. Lower is a better match.
///
/// The reference display height dominates the choice, since a model trained for the
/// wrong display size is the larger error. The viewing distance breaks a tie between
/// models that share a display height.
fn score(model: &VmafModel, measurement_height: u32, viewing_distance: f32) -> f64 {
    let height_gap =
        (f64::from(model.reference_display_height) - f64::from(measurement_height)).abs() * 1000.0;
    let distance_gap = f64::from(model.normalized_viewing_distance) - f64::from(viewing_distance);
    height_gap + distance_gap.abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(path: &str, is_v1: bool, height: u32, distance: f32) -> VmafModel {
        VmafModel {
            path: PathBuf::from(path),
            is_v1,
            reference_display_height: height,
            normalized_viewing_distance: distance,
        }
    }

    /// The real four standard-frame-rate v1.0.16 model files, exactly as their own
    /// JSON reports `adm_ref_display_height` and `adm_norm_view_dist`.
    fn real_v1_models() -> Vec<VmafModel> {
        vec![
            model("vmaf_v1.0.16_3d0h.json", true, 1080, 3.0),
            model("vmaf_v1.0.16_5d0h.json", true, 1080, 5.0),
            model("vmaf_v1.0.16_1d5h_2160.json", true, 2160, 1.5),
            model("vmaf_v1.0.16_3d0h_2160.json", true, 2160, 3.0),
        ]
    }

    #[test]
    fn a_1080p_measurement_at_the_default_distance_picks_the_1080p_3d0h_model() {
        let models = real_v1_models();
        let chosen = choose_model(&models, 1080, 3.0).unwrap();
        assert_eq!(chosen.path, PathBuf::from("vmaf_v1.0.16_3d0h.json"));
    }

    #[test]
    fn a_1080p_measurement_at_five_picture_heights_picks_the_5d0h_model() {
        let models = real_v1_models();
        let chosen = choose_model(&models, 1080, 5.0).unwrap();
        assert_eq!(chosen.path, PathBuf::from("vmaf_v1.0.16_5d0h.json"));
    }

    #[test]
    fn a_2160p_measurement_picks_a_2160p_model_and_not_the_closer_distance_at_1080p() {
        let models = real_v1_models();
        // 3.0 exists at 1080p exactly, and only 1.5 or 3.0 exist at 2160p. The display
        // height must win, so the answer is the 2160p 3.0 model, not the 1080p one.
        let chosen = choose_model(&models, 2160, 3.0).unwrap();
        assert_eq!(chosen.path, PathBuf::from("vmaf_v1.0.16_3d0h_2160.json"));
    }

    #[test]
    fn a_2160p_measurement_at_a_close_viewing_distance_picks_the_1d5h_model() {
        let models = real_v1_models();
        let chosen = choose_model(&models, 2160, 1.5).unwrap();
        assert_eq!(chosen.path, PathBuf::from("vmaf_v1.0.16_1d5h_2160.json"));
    }

    #[test]
    fn an_unmeasured_height_still_picks_the_nearest_display_height() {
        let models = real_v1_models();
        let chosen = choose_model(&models, 1440, 3.0).unwrap();
        assert_eq!(chosen.reference_display_height, 1080);
    }

    #[test]
    fn an_empty_model_list_chooses_nothing() {
        assert!(choose_model(&[], 1080, 3.0).is_none());
    }
}
