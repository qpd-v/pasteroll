//! Wheel accumulation.
//!
//! Windows reports wheel movement in multiples of `WHEEL_DELTA` (120) for a classic notch, but
//! high-resolution wheels and trackpads send smaller deltas. A step is emitted only once a whole
//! notch has accumulated, so one physical notch is one clip regardless of the device.

pub const WHEEL_DELTA: i32 = 120;

/// One carousel step. Wheel up (positive delta) is `Newer`, wheel down is `Older`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Newer,
    Older,
}

#[derive(Debug, Default)]
pub struct WheelAccumulator {
    remainder: i32,
}

impl WheelAccumulator {
    pub const fn new() -> Self {
        Self { remainder: 0 }
    }

    /// Feed one wheel event (`delta` is the signed high word of `mouseData`).
    /// Calls `emit` once per completed notch. Allocation-free: safe to call from a hook callback.
    pub fn push(&mut self, delta: i16, mut emit: impl FnMut(Step)) {
        self.remainder += i32::from(delta);
        while self.remainder.abs() >= WHEEL_DELTA {
            if self.remainder > 0 {
                self.remainder -= WHEEL_DELTA;
                emit(Step::Newer);
            } else {
                self.remainder += WHEEL_DELTA;
                emit(Step::Older);
            }
        }
    }

    pub fn reset(&mut self) {
        self.remainder = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(deltas: &[i16]) -> Vec<Step> {
        let mut acc = WheelAccumulator::new();
        let mut out = Vec::new();
        for &d in deltas {
            acc.push(d, |s| out.push(s));
        }
        out
    }

    #[test]
    fn one_classic_notch_down_is_one_older() {
        assert_eq!(run(&[-120]), vec![Step::Older]);
    }

    #[test]
    fn one_classic_notch_up_is_one_newer() {
        assert_eq!(run(&[120]), vec![Step::Newer]);
    }

    #[test]
    fn high_resolution_deltas_need_a_whole_notch() {
        assert_eq!(run(&[-30, -30, -30]), vec![]);
        assert_eq!(run(&[-30, -30, -30, -30]), vec![Step::Older]);
    }

    #[test]
    fn fast_flick_emits_several_steps() {
        assert_eq!(run(&[-360]), vec![Step::Older; 3]);
    }

    #[test]
    fn remainder_carries_over() {
        // 5 * 50 = 250 -> two notches, 10 left over
        assert_eq!(run(&[-50, -50, -50, -50, -50]), vec![Step::Older; 2]);
    }

    #[test]
    fn reversal_cancels_partial_travel() {
        assert_eq!(run(&[-60, 60, -60, 60]), vec![]);
    }

    #[test]
    fn reset_drops_the_remainder() {
        let mut acc = WheelAccumulator::new();
        let mut n = 0;
        acc.push(-100, |_| n += 1);
        acc.reset();
        acc.push(-100, |_| n += 1);
        assert_eq!(n, 0);
    }
}
