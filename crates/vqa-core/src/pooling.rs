use crate::metric::HarmonicMean;

pub struct Pooled {
    pub mean: f32,
    pub harmonic_mean: Option<f32>,
    pub median: f32,
    pub p1: f32,
    pub p5: f32,
    pub p10: f32,
    pub p25: f32,
    pub p75: f32,
    pub p90: f32,
    pub p95: f32,
    /// Serves the "Worst 1%" column for a lower-is-better metric, the way `p1` already
    /// serves it for a higher-is-better metric.
    pub p99: f32,
    pub min: f32,
    pub max: f32,
    pub stdev: f32,
}

pub fn pool(values: &[f32], harmonic_mean: HarmonicMean) -> Option<Pooled> {
    if values.is_empty() {
        return None;
    }

    let mut sorted_values: Vec<f32> = values.to_vec();
    sorted_values.sort_by(|left, right| left.total_cmp(right));

    let count = sorted_values.len() as f32;
    let mean = sorted_values.iter().sum::<f32>() / count;
    let variance = sorted_values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / count;

    Some(Pooled {
        mean,
        harmonic_mean: harmonic_mean_of(&sorted_values, harmonic_mean, mean),
        median: percentile(&sorted_values, 50.0),
        p1: percentile(&sorted_values, 1.0),
        p5: percentile(&sorted_values, 5.0),
        p10: percentile(&sorted_values, 10.0),
        p25: percentile(&sorted_values, 25.0),
        p75: percentile(&sorted_values, 75.0),
        p90: percentile(&sorted_values, 90.0),
        p95: percentile(&sorted_values, 95.0),
        p99: percentile(&sorted_values, 99.0),
        min: sorted_values[0],
        max: sorted_values[sorted_values.len() - 1],
        stdev: variance.sqrt(),
    })
}

fn harmonic_mean_of(sorted_values: &[f32], rule: HarmonicMean, mean: f32) -> Option<f32> {
    match rule {
        HarmonicMean::Blocked(_) => None,
        HarmonicMean::AllowedAboveZero if sorted_values[0] <= 0.0 => None,
        HarmonicMean::Allowed | HarmonicMean::AllowedAboveZero => {
            if mean == 0.0 {
                return Some(0.0);
            }
            let reciprocal_sum: f32 = sorted_values.iter().map(|value| 1.0 / value).sum();
            Some(sorted_values.len() as f32 / reciprocal_sum)
        }
    }
}

// Linear interpolation between the two closest ranks, the same method a spreadsheet's
// PERCENTILE.INC function uses. `sorted_values` must already be sorted.
fn percentile(sorted_values: &[f32], target: f64) -> f32 {
    let last_index = sorted_values.len() - 1;
    let rank = target / 100.0 * last_index as f64;
    let lower_index = rank.floor() as usize;
    let upper_index = rank.ceil() as usize;

    if lower_index == upper_index {
        return sorted_values[lower_index];
    }

    let weight = (rank - lower_index as f64) as f32;
    let lower_value = sorted_values[lower_index];
    let upper_value = sorted_values[upper_index];
    lower_value + weight * (upper_value - lower_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_series_gives_no_pooled_value() {
        assert!(pool(&[], HarmonicMean::Allowed).is_none());
    }

    #[test]
    fn a_known_series_gives_spreadsheet_percentiles() {
        let values: Vec<f32> = (1..=100).map(|value| value as f32).collect();
        let pooled = pool(&values, HarmonicMean::Allowed).unwrap();

        assert!((pooled.mean - 50.5).abs() < 0.01);
        assert!((pooled.median - 50.5).abs() < 0.01);
        assert!((pooled.p1 - 1.99).abs() < 0.01);
        assert!((pooled.p5 - 5.95).abs() < 0.01);
        assert!((pooled.p95 - 95.05).abs() < 0.01);
        assert_eq!(pooled.min, 1.0);
        assert_eq!(pooled.max, 100.0);
    }

    #[test]
    fn one_value_gives_that_value_for_every_percentile() {
        let pooled = pool(&[42.0], HarmonicMean::Allowed).unwrap();
        assert_eq!(pooled.mean, 42.0);
        assert_eq!(pooled.median, 42.0);
        assert_eq!(pooled.p95, 42.0);
        assert_eq!(pooled.stdev, 0.0);
    }

    #[test]
    fn the_harmonic_mean_is_blocked_when_the_registry_says_so() {
        let pooled = pool(&[1.0, 2.0, 3.0], HarmonicMean::Blocked("test reason")).unwrap();
        assert_eq!(pooled.harmonic_mean, None);
    }

    #[test]
    fn the_harmonic_mean_is_blocked_above_zero_when_a_value_reaches_zero_or_below() {
        let pooled = pool(&[-4.0, 10.0, 20.0], HarmonicMean::AllowedAboveZero).unwrap();
        assert_eq!(pooled.harmonic_mean, None);
    }

    #[test]
    fn the_harmonic_mean_is_computed_when_every_value_is_positive() {
        let pooled = pool(&[1.0, 2.0, 4.0], HarmonicMean::Allowed).unwrap();
        let expected = 3.0 / (1.0 / 1.0 + 1.0 / 2.0 + 1.0 / 4.0);
        assert!((pooled.harmonic_mean.unwrap() - expected).abs() < 0.001);
    }

    #[test]
    fn a_constant_series_gives_a_standard_deviation_of_zero() {
        let pooled = pool(&[7.0, 7.0, 7.0, 7.0], HarmonicMean::Allowed).unwrap();
        assert_eq!(pooled.stdev, 0.0);
    }
}
