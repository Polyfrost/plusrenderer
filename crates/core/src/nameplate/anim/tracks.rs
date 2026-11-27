use crate::limits::NAMEPLATE_OFFSET;
use crate::nameplate::types::{Ease, Track, Value, OPAQUE};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Frame,
    Position,
    Opacity,
    Visible,
}

impl Channel {
    pub fn clamp(self, value: i32) -> i32 {
        let limit = NAMEPLATE_OFFSET as i32;
        match self {
            Channel::Frame => value.max(0),
            Channel::Position => value.clamp(-limit, limit),
            Channel::Opacity => value.clamp(0, i32::from(OPAQUE)),
            Channel::Visible => i32::from(value != 0),
        }
    }

    pub fn convert(self, raw: f64) -> i32 {
        match self {
            Channel::Opacity => self.clamp(snap(raw * f64::from(OPAQUE))),
            Channel::Visible => i32::from(raw.is_finite() && raw != 0.0),
            _ => self.clamp(snap(raw)),
        }
    }
}

pub fn snap(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    libm::floor(value + 0.5).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

pub fn key_at(track: &Track, frame: i32) -> usize {
    track
        .keys
        .iter()
        .skip(1)
        .take_while(|k| i32::from(k.frame) <= frame)
        .count()
}

fn round_half_up(numerator: i128, denominator: i128) -> i32 {
    let rounded = (2 * numerator + denominator).div_euclid(2 * denominator);
    rounded.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32
}

pub fn blend(ease: Ease, from: i32, to: i32, elapsed: i32, span: i32) -> i32 {
    if span <= 0 {
        return to;
    }
    let (from, to) = (i128::from(from), i128::from(to));
    let span = i128::from(span);
    let elapsed = i128::from(elapsed).clamp(0, span);
    match ease {
        Ease::Step => {
            if elapsed >= span {
                to as i32
            } else {
                from as i32
            }
        }
        Ease::Linear => round_half_up(from * span + (to - from) * elapsed, span),
        Ease::Ease => {
            let cube = span * span * span;
            let shape = elapsed * elapsed * (3 * span - 2 * elapsed);
            round_half_up(from * cube + (to - from) * shape, cube)
        }
    }
}

fn value_of(value: Value, values: &dyn Fn(u32) -> i32) -> i32 {
    match value {
        Value::Fixed(v) => v,
        Value::Molang(program) => values(program),
    }
}

pub fn sample(track: &Track, frame: i32, values: &dyn Fn(u32) -> i32) -> i32 {
    let Some(first) = track.keys.first() else {
        return 0;
    };
    if frame < i32::from(first.frame) {
        return value_of(first.value, values);
    }
    let index = key_at(track, frame);
    let key = &track.keys[index];
    let from = value_of(key.value, values);
    match track.keys.get(index + 1) {
        Some(next) if next.ease != Ease::Step => blend(
            next.ease,
            from,
            value_of(next.value, values),
            frame - i32::from(key.frame),
            i32::from(next.frame) - i32::from(key.frame),
        ),
        _ => from,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::types::Key;

    #[test]
    fn tracks_step_round_linear_ties_up_and_ease_on_an_integer_smoothstep() {
        let samples = |keys: &[(u16, i32, Ease)], frames: std::ops::RangeInclusive<i32>| {
            let keys = keys.iter().map(|&(frame, v, ease)| Key { frame, value: if v == 99 { Value::Molang(7) } else { Value::Fixed(v) }, ease });
            let track = Track { keys: keys.collect() };
            frames.map(|f| sample(&track, f, &|p| if p == 7 { -4 } else { 0 })).collect::<Vec<_>>()
        };
        assert_eq!(samples(&[(0, 3, Ease::Step), (4, 7, Ease::Step)], -1..=5), [3, 3, 3, 3, 3, 7, 7]);
        assert_eq!(samples(&[(0, 0, Ease::Step), (8, 10, Ease::Linear)], 0..=8), [0, 1, 3, 4, 5, 6, 8, 9, 10]);
        assert_eq!(samples(&[(0, 0, Ease::Step), (8, -10, Ease::Linear)], 0..=8), [0, -1, -2, -4, -5, -6, -7, -9, -10]);
        assert_eq!(samples(&[(0, 0, Ease::Step), (8, 100, Ease::Ease)], 0..=8), [0, 4, 16, 32, 50, 68, 84, 96, 100]);
        assert_eq!(samples(&[(0, 0, Ease::Step), (2, -1, Ease::Ease)], 0..=2), [0, 0, -1]);
        assert_eq!(samples(&[(0, 99, Ease::Step), (4, 20, Ease::Linear)], 0..=4), [-4, 2, 8, 14, 20]);
        assert_eq!((blend(Ease::Linear, i32::MIN, i32::MAX, 1, 2), blend(Ease::Ease, 0, 255, 300, 600)), (0, 128));
        assert_eq!([snap(1.5), snap(-1.5), snap(f64::NAN), snap(1e300)], [2, -1, 0, i32::MAX]);
        let limit = NAMEPLATE_OFFSET as i32;
        assert_eq!([Channel::Position.convert(500.0), Channel::Opacity.convert(0.5), Channel::Visible.convert(0.2)], [limit, 128, 1]);
        assert_eq!([Channel::Frame.convert(-2.0), Channel::Opacity.convert(f64::INFINITY), Channel::Position.clamp(-90)], [0, 0, -limit]);
    }
}
