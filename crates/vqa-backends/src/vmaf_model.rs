//! Reads a VMAF model file.
//!
//! A model's own JSON carries the display height and the viewing distance it was
//! trained for, inside `model_dict.feature_opts_dicts`, at the same position as the
//! `VMAF_integer_feature` entry in `model_dict.feature_names`. The tool reads that
//! position instead of matching a file name, because a renamed or a new Netflix model
//! must still work.

use crate::error::{BackendError, Result};
use serde_json::Value;
use std::path::Path;
use vqa_core::vmaf_model::VmafModel;

/// Reads one model file from disk.
pub fn read_model_file(path: &Path) -> Result<VmafModel> {
    let text = std::fs::read_to_string(path).map_err(|error| BackendError::Parse {
        program: "vmaf model".into(),
        detail: format!("{}: {error}", path.display()),
    })?;
    parse_model_json(&text, path)
}

/// Reads every `.json` model file in one folder. A file that fails to parse is left
/// out, and never stops the rest of the folder from loading.
pub fn read_model_folder(folder: &Path) -> Vec<VmafModel> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| read_model_file(&path).ok())
        .collect()
}

/// Reads the model fields out of one file's already-loaded JSON text. Pure, and used
/// directly by the tests, since it needs no file on disk.
pub fn parse_model_json(json: &str, path: &Path) -> Result<VmafModel> {
    let root: Value = serde_json::from_str(json).map_err(|error| BackendError::Parse {
        program: "vmaf model".into(),
        detail: error.to_string(),
    })?;

    let model_dict = root.get("model_dict").ok_or_else(|| BackendError::Parse {
        program: "vmaf model".into(),
        detail: format!("{}: no model_dict", path.display()),
    })?;

    let feature_names = model_dict
        .get("feature_names")
        .and_then(Value::as_array)
        .ok_or_else(|| BackendError::Parse {
            program: "vmaf model".into(),
            detail: format!("{}: no feature_names", path.display()),
        })?;

    // Only a VMAF v1 model runs CAMBI as one of its own features, and that is the one
    // structural fact that tells a v1 model apart from a v0 model or a v0 NEG model,
    // both of which can carry the same `adm_enhn_gain_limit: 1.0` a v1 model carries.
    let is_v1 = feature_names.iter().any(|name| {
        name.as_str()
            .is_some_and(|name| name.starts_with("Cambi_feature"))
    });

    let adm_opts = model_dict
        .get("feature_opts_dicts")
        .and_then(Value::as_array)
        .and_then(|feature_opts| {
            feature_names
                .iter()
                .zip(feature_opts.iter())
                .find_map(|(name, opts)| match name.as_str() {
                    Some(name) if name.starts_with("VMAF_integer_feature") => Some(opts),
                    _ => None,
                })
        });

    let reference_display_height = adm_opts
        .and_then(|opts| opts.get("adm_ref_display_height"))
        .and_then(Value::as_u64)
        .unwrap_or(1080) as u32;
    let normalized_viewing_distance = adm_opts
        .and_then(|opts| opts.get("adm_norm_view_dist"))
        .and_then(Value::as_f64)
        .unwrap_or(3.0) as f32;

    Ok(VmafModel {
        path: path.to_path_buf(),
        is_v1,
        reference_display_height,
        normalized_viewing_distance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real shape of a VMAF v1.0.16 model file, trimmed of the SVM weight blob,
    /// which this parser never reads. Captured from `vmaf_v1.0.16_3d0h.json` on a real
    /// machine with the model package installed.
    const REAL_V1_MODEL_SHAPE: &str = r#"{
        "param_dict": {
            "norm_type": "clip_0to1",
            "score_clip": null,
            "gamma": 0.035,
            "C": 4.0,
            "nu": 0.9
        },
        "model_dict": {
            "model_type": "LIBSVMNUSVR",
            "feature_names": [
                "Cambi_feature_cambi_score",
                "Speed_chroma_feature_speed_chroma_uv_score",
                "VMAF_integer_feature_adm3_score",
                "VMAF_integer_feature_motion3_score"
            ],
            "feature_opts_dicts": [
                {
                    "cambi_high_res_speedup": 1080,
                    "cambi_vis_lum_threshold": 0.06,
                    "cambi_max_val": 17.0
                },
                {
                    "speed_kernelscale": 1.0,
                    "speed_max_val": 45.0
                },
                {
                    "adm_dlm_weight": 0.7,
                    "adm_enhn_gain_limit": 1.0,
                    "adm_noise_weight": 0.02,
                    "adm_norm_view_dist": 3.0,
                    "adm_ref_display_height": 1080,
                    "adm_min_val": 0.5,
                    "adm_csf_mode": 2
                },
                {
                    "motion_max_val": 18.0
                }
            ]
        }
    }"#;

    /// The real shape of `vmaf_v0.6.1.json`: no `Cambi_feature` entry, and no
    /// `feature_opts_dicts` key at all. This model predates the viewing-distance-aware
    /// ADM feature entirely.
    const REAL_V0_MODEL_SHAPE: &str = r#"{
        "model_dict": {
            "feature_names": [
                "VMAF_integer_feature_adm2_score",
                "VMAF_integer_feature_motion2_score",
                "VMAF_integer_feature_vif_scale0_score",
                "VMAF_integer_feature_vif_scale1_score",
                "VMAF_integer_feature_vif_scale2_score",
                "VMAF_integer_feature_vif_scale3_score"
            ]
        }
    }"#;

    /// The real shape of `vmaf_v0.6.1neg.json`: `adm_enhn_gain_limit` already at 1.0,
    /// the same value a v1 model carries, but still no `Cambi_feature` entry. This is
    /// the real case that rules out reading `adm_enhn_gain_limit` alone to decide v1.
    const REAL_V0_NEG_MODEL_SHAPE: &str = r#"{
        "model_dict": {
            "feature_names": [
                "VMAF_integer_feature_adm2_score",
                "VMAF_integer_feature_motion2_score",
                "VMAF_integer_feature_vif_scale0_score",
                "VMAF_integer_feature_vif_scale1_score",
                "VMAF_integer_feature_vif_scale2_score",
                "VMAF_integer_feature_vif_scale3_score"
            ],
            "feature_opts_dicts": [
                { "adm_enhn_gain_limit": 1.0 },
                {},
                { "vif_enhn_gain_limit": 1.0 },
                { "vif_enhn_gain_limit": 1.0 },
                { "vif_enhn_gain_limit": 1.0 },
                { "vif_enhn_gain_limit": 1.0 }
            ]
        }
    }"#;

    #[test]
    fn a_real_v1_model_reads_its_own_display_height_and_viewing_distance() {
        let model =
            parse_model_json(REAL_V1_MODEL_SHAPE, Path::new("vmaf_v1.0.16_3d0h.json")).unwrap();
        assert_eq!(model.reference_display_height, 1080);
        assert_eq!(model.normalized_viewing_distance, 3.0);
        assert!(model.is_v1);
    }

    #[test]
    fn a_v0_model_with_no_feature_opts_falls_back_to_the_default_size_and_is_not_v1() {
        let model = parse_model_json(REAL_V0_MODEL_SHAPE, Path::new("vmaf_v0.6.1.json")).unwrap();
        assert!(!model.is_v1);
        assert_eq!(model.reference_display_height, 1080);
        assert_eq!(model.normalized_viewing_distance, 3.0);
    }

    #[test]
    fn a_v0_neg_model_shares_v1_gain_limit_but_is_still_not_v1() {
        let model =
            parse_model_json(REAL_V0_NEG_MODEL_SHAPE, Path::new("vmaf_v0.6.1neg.json")).unwrap();
        assert!(!model.is_v1);
    }

    #[test]
    fn a_file_with_no_model_dict_is_a_parse_error() {
        let result = parse_model_json("{}", Path::new("empty.json"));
        assert!(result.is_err());
    }
}
