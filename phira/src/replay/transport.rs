//! Replay timeline math. This module is deliberately independent from
//! rendering, audio, judgement, score submission, and ranking.

/// Supported playback rates (separate from the rate at which the chart was recorded).
pub const RATES: [f32; 8] = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0];

/// Resolve a one-frame step. Returns the target time and whether it came from
/// the recorded frame table. Sparse or exhausted tables use a 60 Hz fallback.
pub fn frame_step(frames: &[f64], current: f64, delta: i32, end: f64) -> (f64, bool) {
    let fallback = (current + delta as f64 / 60.0).clamp(0.0, end);
    if delta == 0 {
        return (current.clamp(0.0, end), false);
    }

    let index = if delta > 0 {
        frames.partition_point(|t| *t <= current + 1e-7) as i64 + delta as i64 - 1
    } else {
        frames.partition_point(|t| *t < current - 1e-7) as i64 + delta as i64
    };

    let Some(target) = usize::try_from(index).ok().and_then(|i| frames.get(i)).copied() else {
        return (fallback, false);
    };

    // A sparse frame must never turn a single step into a large seek.
    if (target - current).abs() > delta.unsigned_abs() as f64 * 0.25 {
        (fallback, false)
    } else {
        (target.clamp(0.0, end), true)
    }
}

/// Cycle the viewer playback rate without changing the replay's recorded chart speed.
pub fn next_rate(current: f64) -> f64 {
    let index = RATES.iter().position(|rate| (*rate as f64 - current).abs() < 1e-6).unwrap_or(4);
    RATES[(index + 1) % RATES.len()] as f64
}

/// Calculate the next A-B loop position while preserving overrun.
pub fn loop_target(current: f64, a: Option<f64>, b: Option<f64>, enabled: bool) -> Option<f64> {
    let (a, b) = (a?, b?);
    (enabled && b - a >= 0.05 && current >= b).then(|| a + (current - b).rem_euclid(b - a))
}

/// Locate the previous/next judgement error, preserving an explicit cursor
/// while the playback head is temporarily positioned before the selected error.
pub fn error_index(errors: &[f64], current: f64, next: bool, selected: Option<usize>) -> Option<usize> {
    if let Some(i) = selected.filter(|&i| {
        errors
            .get(i)
            .is_some_and(|t| current >= (t - 1.0).max(0.0) - 1e-7 && current <= *t + 1e-7)
    }) {
        return if next {
            (i + 1 < errors.len()).then_some(i + 1)
        } else {
            i.checked_sub(1)
        };
    }

    if next {
        let i = errors.partition_point(|t| *t <= current + 1e-7);
        (i < errors.len()).then_some(i)
    } else {
        errors.partition_point(|t| *t < current - 1e-7).checked_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_steps_use_recorded_frames_and_fallback_safely() {
        let frames = [0.0, 0.006, 0.017, 0.04, 0.041, 0.1];
        assert_eq!(frame_step(&frames, 0.017, 1, 2.0), (0.04, true));
        assert_eq!(frame_step(&frames, 0.017, -1, 2.0), (0.006, true));
        assert_eq!(frame_step(&frames, 0.02, -1, 2.0), (0.017, true));
        assert_eq!(frame_step(&frames, 0.0, -10, 2.0), (0.0, false));
        assert_eq!(frame_step(&[], 0.0, 1, 2.0), (1.0 / 60.0, false));
    }

    #[test]
    fn sparse_and_exhausted_frames_never_seek_to_song_boundaries() {
        for frames in [&[][..], &[0.0][..], &[0.0, 120.0][..]] {
            assert!((frame_step(frames, 30.0, -1, 120.0).0 - (30.0 - 1.0 / 60.0)).abs() < 1e-9);
            assert!((frame_step(frames, 30.0, 1, 120.0).0 - (30.0 + 1.0 / 60.0)).abs() < 1e-9);
        }
        assert_eq!(frame_step(&[0.0, 0.0, 0.01, 0.01, 0.02], 0.01, 1, 2.0), (0.02, true));
        assert_eq!(frame_step(&[0.0, 0.0, 0.01, 0.01, 0.02], 0.01, -1, 2.0), (0.0, true));
    }

    #[test]
    fn playback_rate_cycles_through_supported_values() {
        assert!((next_rate(1.0) - 1.5).abs() < 1e-6);
        assert!((next_rate(3.0) - 0.1).abs() < 1e-6);
        assert!((next_rate(99.0) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn loop_preserves_overrun_and_rejects_invalid_ranges() {
        assert!((loop_target(8.2, Some(3.0), Some(8.0), true).unwrap() - 3.2).abs() < 1e-12);
        assert_eq!(loop_target(8.0, Some(3.0), Some(8.0), true), Some(3.0));
        assert_eq!(loop_target(8.0, Some(8.0), Some(3.0), true), None);
        assert_eq!(loop_target(8.0, Some(3.0), Some(8.0), false), None);
        assert_eq!(loop_target(8.0, None, Some(8.0), true), None);
        assert_eq!(loop_target(8.0, Some(3.0), Some(3.04), true), None);
    }

    #[test]
    fn error_navigation_keeps_cursor_separate_from_preroll() {
        let errors = [0.1, 0.2, 3.0, 3.002, 8.0];
        assert_eq!(error_index(&errors, 0.0, true, None), Some(0));
        assert_eq!(error_index(&errors, 0.0, true, Some(0)), Some(1));
        assert_eq!(error_index(&errors, 0.0, false, Some(1)), Some(0));
        assert_eq!(error_index(&errors, 2.0, true, Some(2)), Some(3));
        assert_eq!(error_index(&errors, 2.002, false, Some(3)), Some(2));
        assert_eq!(error_index(&errors, 7.0, true, Some(4)), None);
        assert_eq!(error_index(&errors, 4.0, false, Some(2)), Some(3));
        assert_eq!(error_index(&[], 0.0, true, None), None);
    }
}
