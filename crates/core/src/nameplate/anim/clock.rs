use crate::nameplate::types::Phase;

pub const FRAME_LIMIT: i64 = 1 << 23;

pub fn millis(clock: f64) -> i64 {
    if clock.is_finite() {
        libm::floor(clock * 1000.0) as i64
    } else {
        0
    }
}

pub fn absolute_frame(clock: f64, fps: u8) -> i64 {
    millis(clock)
        .saturating_mul(i64::from(fps))
        .div_euclid(1000)
}

pub fn wrap_period(loop_frames: u16) -> i64 {
    let period = i64::from(loop_frames.max(1));
    (FRAME_LIMIT - 1) / period * period
}

pub fn engine_frame(clock: f64, fps: u8, loop_frames: u16) -> u32 {
    absolute_frame(clock, fps).rem_euclid(wrap_period(loop_frames)) as u32
}

pub fn phase_offset(phase: Phase, seed: u64, loop_frames: u16) -> u32 {
    match phase {
        Phase::Global => 0,
        Phase::PerPlayer => (seed % u64::from(loop_frames.max(1))) as u32,
    }
}

pub fn loop_frame(engine: u32, offset: u32, loop_frames: u16) -> u16 {
    ((u64::from(engine) + u64::from(offset)) % u64::from(loop_frames.max(1))) as u16
}

pub fn cycle(engine: u32, loop_frames: u16) -> u32 {
    engine / u32::from(loop_frames.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_frames_match_a_date_now_clock_and_wrap_on_a_whole_loop() {
        let js = |ms: i64, fps: u8, loop_frames: u16| {
            let period = (FRAME_LIMIT - 1) / i64::from(loop_frames) * i64::from(loop_frames);
            ((ms * i64::from(fps)).div_euclid(1000) % period) as u32
        };
        for (ms, fps, loop_frames) in [(1_764_000_000_123_i64, 24_u8, 12_u16), (1_795_564_800_999, 1, 1), (41_666, 24, 12), (41_667, 24, 12)] {
            assert_eq!(engine_frame(ms as f64 / 1000.0, fps, loop_frames), js(ms, fps, loop_frames), "{ms}");
        }
        for ms in (0..20_000_i64).map(|step| 1_764_000_000_000 + step * 37) {
            for (fps, loop_frames) in [(24, 12), (13, 600), (7, 1), (1, 599)] {
                assert_eq!(engine_frame(ms as f64 / 1000.0, fps, loop_frames), js(ms, fps, loop_frames), "{ms} {fps}");
            }
        }
        let period = wrap_period(12);
        assert_eq!(period, 8_388_600);
        let at = |frames: i64| engine_frame((frames as f64 + 0.5) / 24.0, 24, 12);
        assert_eq!((at(period - 1), at(period), at(period + 5)), ((period - 1) as u32, 0, 5));
        assert_eq!((engine_frame(f64::NAN, 24, 12), engine_frame(-0.001, 24, 12)), (0, (period - 1) as u32));
        assert_eq!((phase_offset(Phase::PerPlayer, 12_345, 12), phase_offset(Phase::Global, 12_345, 12)), (12_345 % 12, 0));
        assert_eq!(loop_frame(10, 5, 12), 3);
    }
}
