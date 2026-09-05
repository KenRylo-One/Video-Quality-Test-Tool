//! Which frames are worth looking at.
//!
//! Every metric has known faults, so a measurement ends with a person looking at the
//! pixels. This module decides the order that person walks the frames in.

use crate::metric::Direction;
use crate::plot::is_worse;

/// One frame and what it measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameValue {
    pub frame: u64,
    pub value: f32,
}

/// Every finite frame, worst first.
///
/// The direction decides what "worst" means. For CAMBI the worst frame carries the
/// highest value, and a list sorted the other way sends the reader to the cleanest
/// frame in the clip with nothing on screen to say so.
pub fn worst_frames(values: &[f32], first_frame: u64, direction: Direction) -> Vec<FrameValue> {
    let mut frames: Vec<FrameValue> = values
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, value)| value.is_finite())
        .map(|(offset, value)| FrameValue {
            frame: first_frame + offset as u64,
            value,
        })
        .collect();

    // The frame number breaks a tie, so the order is the same on every run.
    frames.sort_by(|left, right| match direction {
        Direction::LowerIsBetter => right
            .value
            .total_cmp(&left.value)
            .then(left.frame.cmp(&right.frame)),
        _ => left
            .value
            .total_cmp(&right.value)
            .then(left.frame.cmp(&right.frame)),
    });
    frames
}

/// The next frame to look at, walking `step` places through the worst-first order.
///
/// A positive `step` moves towards the better frames and a negative one towards the
/// worse. The walk stops at each end rather than wrapping, because wrapping from the
/// worst frame to the best reads as a bug.
pub fn step_from(order: &[FrameValue], current: u64, step: i64) -> Option<u64> {
    let at = order.iter().position(|entry| entry.frame == current)?;
    let wanted = (at as i64).saturating_add(step);
    let wanted = wanted.clamp(0, order.len() as i64 - 1) as usize;
    Some(order[wanted].frame)
}

/// Whether one value is the worse of two. Re-exported so a caller needs one import.
pub fn worse(value: f32, than: f32, direction: Direction) -> bool {
    is_worse(value, than, direction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_high_is_better_metric_puts_the_lowest_value_first() {
        let order = worst_frames(&[40.0, 30.0, 50.0], 0, Direction::HigherIsBetter);

        assert_eq!(order[0].frame, 1);
        assert_eq!(order[0].value, 30.0);
        assert_eq!(order.last().unwrap().frame, 2);
    }

    /// Acceptance test 3 of milestone M6. For CAMBI the worst frame is the highest.
    #[test]
    fn a_low_is_better_metric_puts_the_highest_value_first() {
        let order = worst_frames(&[0.0, 12.0, 3.0], 0, Direction::LowerIsBetter);

        assert_eq!(order[0].frame, 1);
        assert_eq!(order[0].value, 12.0);
        assert_eq!(order.last().unwrap().frame, 0);
    }

    #[test]
    fn the_order_reads_real_frame_numbers_when_a_run_started_part_way_in() {
        let order = worst_frames(&[40.0, 30.0], 1200, Direction::HigherIsBetter);

        assert_eq!(order[0].frame, 1201);
        assert_eq!(order[1].frame, 1200);
    }

    #[test]
    fn an_infinite_value_never_reaches_the_order() {
        let order = worst_frames(
            &[f32::INFINITY, 40.0, f32::NAN],
            0,
            Direction::HigherIsBetter,
        );

        assert_eq!(order.len(), 1);
        assert_eq!(order[0].frame, 1);
    }

    #[test]
    fn two_frames_of_the_same_value_keep_the_lower_frame_number_first() {
        let order = worst_frames(&[30.0, 30.0, 30.0], 0, Direction::HigherIsBetter);

        assert_eq!(
            order.iter().map(|entry| entry.frame).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn a_file_with_no_finite_value_gives_an_empty_order() {
        let order = worst_frames(&[f32::INFINITY; 4], 0, Direction::HigherIsBetter);
        assert!(order.is_empty());
        assert_eq!(step_from(&order, 0, -1), None);
    }

    #[test]
    fn stepping_walks_the_order_and_stops_at_each_end() {
        let order = worst_frames(&[40.0, 30.0, 50.0], 0, Direction::HigherIsBetter);

        assert_eq!(step_from(&order, 1, 1), Some(0));
        assert_eq!(step_from(&order, 0, 1), Some(2));
        assert_eq!(step_from(&order, 2, 1), Some(2), "the best end holds");
        assert_eq!(step_from(&order, 1, -1), Some(1), "the worst end holds");
        assert_eq!(step_from(&order, 2, -2), Some(1));
    }

    #[test]
    fn a_frame_that_is_not_in_the_order_gives_no_step() {
        let order = worst_frames(&[40.0], 0, Direction::HigherIsBetter);
        assert_eq!(step_from(&order, 99, -1), None);
    }
}
