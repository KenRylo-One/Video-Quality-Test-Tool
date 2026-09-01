//! Content presets.
//!
//! A preset is a button that ticks checkboxes. It is not a mode, and it never hides the
//! metric list. Any change to a checkbox clears the preset name.

use crate::metric::MetricId;

/// One content preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// The name in the dropdown.
    pub name: &'static str,
    /// What the preset ticks.
    pub metrics: &'static [MetricId],
    /// Why these metrics, in one sentence.
    pub reason: &'static str,
}

/// The six presets.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Live action, 24 to 30 fps",
        metrics: &[MetricId::Ssimulacra2, MetricId::Vmaf],
        reason: "Camera noise and skin tone are color faults, and SSIMULACRA 2 works in a color space.",
    },
    Preset {
        name: "Live action, 60 fps",
        metrics: &[MetricId::Vmaf, MetricId::Ssimulacra2],
        reason: "VMAF v1 corrects the 60 fps error of v0. The tool picks an hfr model from the frame rate.",
    },
    Preset {
        name: "Photorealistic game capture",
        metrics: &[MetricId::Vmaf, MetricId::Ssimulacra2, MetricId::XpsnrMin],
        reason: "High motion needs the v1 motion threshold. XPSNR adds speed.",
    },
    Preset {
        name: "Cel-shaded or anime",
        metrics: &[MetricId::Cambi, MetricId::Ssimulacra2, MetricId::Vmaf],
        reason: "Flat areas and slow gradients band. CAMBI is the only metric that targets banding.",
    },
    Preset {
        name: "Screen capture and text",
        metrics: &[MetricId::XpsnrMin, MetricId::Ssimulacra2, MetricId::PsnrY],
        reason: "No metric here targets text legibility. Measure, then look at the picture.",
    },
    Preset {
        name: "Transparency check",
        metrics: &[MetricId::ButteraugliMax, MetricId::Ssimulacra2],
        reason: "The maximum norm finds the one broken frame that a mean hides.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_guide_gives_six_presets() {
        assert_eq!(PRESETS.len(), 6);
    }

    #[test]
    fn every_preset_ticks_a_metric_that_the_registry_holds() {
        for preset in PRESETS {
            assert!(!preset.metrics.is_empty(), "{} ticks nothing", preset.name);
            for id in preset.metrics {
                assert_eq!(MetricId::from_key(id.key()), Some(*id));
            }
        }
    }

    #[test]
    fn every_preset_name_is_unique() {
        let names: BTreeSet<_> = PRESETS.iter().map(|preset| preset.name).collect();
        assert_eq!(names.len(), PRESETS.len());
    }
}
