use libm::{ceilf, fabsf, floorf, roundf, sqrtf};

use crate::limits::EFFECT_RING_HZ;
use crate::wearer::{
    reset_ring_clock, RING_BREAKS, RING_BREAK_COUNT, RING_CLOCK_FLOATS, RING_HEAD, RING_MARK_AT,
    RING_MARK_FLOATS, RING_MARK_TURN, RING_POINTS, RING_POINT_FLOATS, RING_SEEN, RING_SEGMENTS,
    RING_SEGMENT_BITS, RING_TIME,
};

pub const RING_STEP_SECONDS: f32 = 1.0 / EFFECT_RING_HZ as f32;
pub const RING_SPAN_SECONDS: f32 = RING_POINTS as f32 * RING_STEP_SECONDS;
pub const NO_TURN: Turn = [0.0, 0.0, 0.0, 1.0];

const STEP_SLACK: f32 = 1e-3;
const JUMP_SPEED: f32 = 100.0;
const JUMP_BLOCKS: f32 = 24.0;
const TURN_LEVELS: u32 = 161;
const TURN_SPAN: f32 = core::f32::consts::FRAC_1_SQRT_2;

pub type Turn = [f32; 4];

#[derive(Clone, Copy, Debug, Default)]
pub struct Ring<'a> {
    pub clock: &'a [f32],
    pub points: &'a [f32],
    pub turns: &'a [f32],
    pub gates: &'a [f32],
}

#[derive(Debug)]
pub struct RingMut<'a> {
    pub clock: &'a mut [f32],
    pub points: &'a mut [f32],
    pub turns: &'a mut [f32],
    pub gates: &'a mut [f32],
}

enum Spot {
    Head,
    Ahead(f32),
    Between(usize, f32),
    Back(usize),
    Leaving(usize, Mark, f32),
}

#[derive(Clone, Copy, Debug)]
struct Mark {
    time: f32,
    at: [f32; 3],
    turn: f32,
}

fn steps(seconds: f32) -> usize {
    if seconds > 0.0 {
        ceilf(seconds * EFFECT_RING_HZ as f32) as usize
    } else {
        0
    }
}

pub fn particle_points(life: f32) -> usize {
    steps(life).saturating_add(2).min(RING_POINTS)
}

pub fn trail_points(fade: f32) -> usize {
    steps(fade).saturating_add(1).min(RING_POINTS)
}

impl Ring<'_> {
    pub fn capacity(&self) -> usize {
        if self.clock.len() < RING_CLOCK_FLOATS {
            return 0;
        }
        (self.points.len() / RING_POINT_FLOATS).min(RING_POINTS)
    }

    fn count(&self) -> usize {
        let capacity = self.capacity();
        match self.clock.get(RING_HEAD) {
            Some(&head) if capacity > 0 && head.is_finite() && head >= 1.0 => {
                (head as usize).min(2 * capacity - 1)
            }
            _ => 0,
        }
    }

    pub fn len(&self) -> usize {
        self.count().min(self.capacity())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn newest_time(&self) -> Option<f32> {
        let time = self.clock.get(RING_TIME).copied()?;
        (!self.is_empty() && time.is_finite()).then_some(time)
    }

    fn slot(&self, back: usize) -> usize {
        (self.count() - 1 - back) % self.capacity()
    }

    pub fn point(&self, back: usize) -> Option<[f32; 3]> {
        if back >= self.len() {
            return None;
        }
        let index = self.slot(back) * RING_POINT_FLOATS;
        Some([
            self.points[index],
            self.points[index + 1],
            self.points[index + 2],
        ])
    }

    pub fn turn(&self, back: usize) -> Option<Turn> {
        if back >= self.len() || self.turns.len() < self.capacity() {
            return None;
        }
        Some(unpack_turn(self.turns[self.slot(back)]))
    }

    pub fn gate(&self, back: usize) -> Option<u32> {
        if back >= self.len() || self.gates.len() < self.capacity() {
            return None;
        }
        Some(self.gates[self.slot(back)] as u32)
    }

    fn mark(&self, at: usize) -> Option<Mark> {
        let floats = self.clock.get(at..at + RING_MARK_FLOATS)?;
        let place = &floats[RING_MARK_AT..RING_MARK_AT + 3];
        floats[0].is_finite().then(|| Mark {
            time: floats[0],
            at: [place[0], place[1], place[2]],
            turn: floats[RING_MARK_TURN],
        })
    }

    fn departure(&self, newer: usize) -> Option<Mark> {
        if newer >= RING_BREAK_COUNT {
            return None;
        }
        self.mark(RING_BREAKS + newer * RING_MARK_FLOATS)
    }

    fn pending(&self) -> Option<Mark> {
        let newest = self.newest_time()?;
        self.departure(0).filter(|mark| mark.time >= newest)
    }

    pub fn breaks_before_head(&self) -> bool {
        self.pending().is_some()
    }

    pub fn starts_segment(&self, back: usize) -> bool {
        if back + 1 >= self.len() {
            return false;
        }
        let slot = self.slot(back);
        let word = self.clock[RING_SEGMENTS + slot / RING_SEGMENT_BITS] as u32;
        (word >> (slot % RING_SEGMENT_BITS)) & 1 != 0
    }

    pub fn jumps_to(&self, at: [f32; 3], now: f32, block: f32) -> bool {
        let Some((time, newest)) = self.newest_time().zip(self.point(0)) else {
            return false;
        };
        if !(now >= time && now - time <= RING_SPAN_SECONDS) {
            return false;
        }
        let block = if block.is_finite() {
            block.max(1.0)
        } else {
            1.0
        };
        let reach = (JUMP_SPEED * (now - time).max(RING_STEP_SECONDS)).min(JUMP_BLOCKS) * block;
        let moved = sqrtf(
            (0..3)
                .map(|i| (at[i] - newest[i]) * (at[i] - newest[i]))
                .sum(),
        );
        moved > reach
    }

    fn spot(&self, now: f32, time: f32) -> Spot {
        let Some(newest_at) = self.newest_time() else {
            return Spot::Head;
        };
        if time >= newest_at {
            return match self.pending() {
                Some(left) if time > left.time => Spot::Head,
                Some(left) => Spot::Leaving(0, left, toward(newest_at, left.time, time)),
                None if now <= newest_at => Spot::Back(0),
                None => Spot::Ahead(((time - newest_at) / (now - newest_at)).clamp(0.0, 1.0)),
            };
        }
        let back = (newest_at - time) * EFFECT_RING_HZ as f32;
        let whole = floorf(back);
        let j = whole as usize;
        let oldest = self.len() - 1;
        if j >= oldest {
            return Spot::Back(oldest);
        }
        if !self.starts_segment(j) {
            return Spot::Between(j, back - whole);
        }
        let newer = usize::from(self.breaks_before_head())
            + (0..j).filter(|&k| self.starts_segment(k)).count();
        let older = newest_at - (j + 1) as f32 * RING_STEP_SECONDS;
        match self.departure(newer) {
            Some(left) if time > left.time => Spot::Back(j),
            Some(left) => Spot::Leaving(j + 1, left, toward(older, left.time, time)),
            None if back - whole < 0.5 => Spot::Back(j),
            None => Spot::Back(j + 1),
        }
    }

    pub fn position_at(&self, head: [f32; 3], now: f32, time: f32) -> [f32; 3] {
        match self.spot(now, time) {
            Spot::Head => head,
            Spot::Ahead(f) => self.point(0).map_or(head, |newest| lerp(newest, head, f)),
            Spot::Between(j, f) => match self.point(j).zip(self.point(j + 1)) {
                Some((a, b)) => lerp(a, b, f),
                None => head,
            },
            Spot::Back(j) => self.point(j).unwrap_or(head),
            Spot::Leaving(j, left, f) => self.point(j).map_or(left.at, |a| lerp(a, left.at, f)),
        }
    }

    pub fn turn_at(&self, head: Turn, now: f32, time: f32) -> Turn {
        match self.spot(now, time) {
            Spot::Head => head,
            Spot::Ahead(f) => self.turn(0).map_or(head, |newest| nlerp(newest, head, f)),
            Spot::Between(j, f) => match self.turn(j).zip(self.turn(j + 1)) {
                Some((a, b)) => nlerp(a, b, f),
                None => head,
            },
            Spot::Back(j) => self.turn(j).unwrap_or(head),
            Spot::Leaving(j, left, f) => self
                .turn(j)
                .map_or(head, |a| nlerp(a, unpack_turn(left.turn), f)),
        }
    }
}

impl RingMut<'_> {
    pub fn as_ring(&self) -> Ring<'_> {
        Ring {
            clock: self.clock,
            points: self.points,
            turns: self.turns,
            gates: self.gates,
        }
    }

    pub fn clear(&mut self) {
        reset_ring_clock(self.clock);
    }

    fn push(&mut self, at: [f32; 3], turn: Turn, gates: u32, starts: bool) {
        let ring = self.as_ring();
        let (capacity, written) = (ring.capacity(), ring.count());
        let slot = written % capacity;
        let index = slot * RING_POINT_FLOATS;
        self.points[index..index + 3].copy_from_slice(&at);
        if let Some(stored) = self.turns.get_mut(slot) {
            *stored = pack_turn(turn);
        }
        if let Some(stored) = self.gates.get_mut(slot) {
            *stored = gates as f32;
        }
        let word = RING_SEGMENTS + slot / RING_SEGMENT_BITS;
        let bit = 1u32 << (slot % RING_SEGMENT_BITS);
        let bits = self.clock[word] as u32;
        self.clock[word] = if starts { bits | bit } else { bits & !bit } as f32;
        let next = written + 1;
        let next = if next >= 2 * capacity {
            next - capacity
        } else {
            next
        };
        self.clock[RING_HEAD] = next as f32;
    }

    fn write_mark(&mut self, at: usize, mark: Mark) {
        let floats = &mut self.clock[at..at + RING_MARK_FLOATS];
        floats[0] = mark.time;
        floats[RING_MARK_AT..RING_MARK_AT + 3].copy_from_slice(&mark.at);
        floats[RING_MARK_TURN] = mark.turn;
    }

    fn see(&mut self, at: [f32; 3], turn: Turn, now: f32) {
        let turn = pack_turn(turn);
        self.write_mark(
            RING_SEEN,
            Mark {
                time: now,
                at,
                turn,
            },
        );
    }

    pub fn sample(&mut self, at: [f32; 3], now: f32) -> usize {
        self.sample_turned(at, NO_TURN, now)
    }

    pub fn sample_turned(&mut self, at: [f32; 3], turn: Turn, now: f32) -> usize {
        self.sample_gated(at, turn, |_| 0, now)
    }

    pub fn sample_gated(
        &mut self,
        at: [f32; 3],
        turn: Turn,
        gates: impl Fn(f32) -> u32,
        now: f32,
    ) -> usize {
        if self.as_ring().capacity() == 0 || !now.is_finite() {
            return 0;
        }
        let ring = self.as_ring();
        let last = ring.newest_time().zip(ring.point(0));
        let Some((time, newest)) = last.filter(|(time, _)| now - time <= RING_SPAN_SECONDS) else {
            self.clear();
            self.push(at, turn, gates(now), false);
            self.clock[RING_TIME] = now;
            self.see(at, turn, now);
            return 1;
        };
        if now < time {
            return 0;
        }
        let newest_turn = ring.turn(0).unwrap_or(turn);
        let left = ring.pending();
        let span = now - time;
        let steps = floorf(span * EFFECT_RING_HZ as f32 + STEP_SLACK) as usize;
        let mut crossed = false;
        for k in 1..=steps {
            let sampled = (time + k as f32 * RING_STEP_SECONDS).min(now);
            let (point, facing, starts) = match left {
                Some(left) if sampled <= left.time => {
                    let f = toward(time, left.time, sampled);
                    let facing = nlerp(newest_turn, unpack_turn(left.turn), f);
                    (lerp(newest, left.at, f), facing, false)
                }
                Some(_) => (at, turn, !core::mem::replace(&mut crossed, true)),
                None => {
                    let f = if span > 0.0 {
                        (k as f32 * RING_STEP_SECONDS / span).min(1.0)
                    } else {
                        1.0
                    };
                    (lerp(newest, at, f), nlerp(newest_turn, turn, f), false)
                }
            };
            self.push(point, facing, gates(sampled), starts);
        }
        if steps > 0 {
            self.clock[RING_TIME] = (time + steps as f32 * RING_STEP_SECONDS).min(now);
        }
        self.see(at, turn, now);
        steps
    }

    pub fn split(&mut self) -> bool {
        let ring = self.as_ring();
        let Some(newest) = ring.newest_time() else {
            return false;
        };
        if ring.breaks_before_head() {
            return true;
        }
        let seen = ring.mark(RING_SEEN).filter(|mark| mark.time >= newest);
        let last = ring.point(0).map(|at| Mark {
            time: newest,
            at,
            turn: pack_turn(ring.turn(0).unwrap_or(NO_TURN)),
        });
        let Some(mark) = seen.or(last) else {
            return false;
        };
        let kept = RING_BREAKS..RING_BREAKS + (RING_BREAK_COUNT - 1) * RING_MARK_FLOATS;
        self.clock.copy_within(kept, RING_BREAKS + RING_MARK_FLOATS);
        self.write_mark(RING_BREAKS, mark);
        true
    }

    pub fn rebase(&mut self, shift: [f32; 3]) {
        if shift == [0.0; 3] {
            return;
        }
        let written = self.as_ring().len();
        for point in self.points[..written * RING_POINT_FLOATS].chunks_exact_mut(RING_POINT_FLOATS)
        {
            for (value, by) in point.iter_mut().zip(shift) {
                *value -= by;
            }
        }
        if self.clock.len() < RING_CLOCK_FLOATS {
            return;
        }
        let breaks = (0..RING_BREAK_COUNT).map(|i| RING_BREAKS + i * RING_MARK_FLOATS);
        for mark in core::iter::once(RING_SEEN).chain(breaks) {
            let at = mark + RING_MARK_AT;
            for (value, by) in self.clock[at..at + 3].iter_mut().zip(shift) {
                *value -= by;
            }
        }
    }
}

fn toward(from: f32, to: f32, time: f32) -> f32 {
    if to > from {
        ((time - from) / (to - from)).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

pub fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn normalized(q: Turn) -> Turn {
    let len = sqrtf(q.iter().map(|c| c * c).sum());
    if len > 1e-6 && len.is_finite() {
        q.map(|c| c / len)
    } else {
        NO_TURN
    }
}

pub fn nlerp(a: Turn, b: Turn, t: f32) -> Turn {
    let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let b = if dot < 0.0 { b.map(|c| -c) } else { b };
    normalized([0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t))
}

pub fn rotate(q: Turn, v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let w = q[3];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = cross(u, v).map(|c| 2.0 * c);
    let ut = cross(u, t);
    [0, 1, 2].map(|i| v[i] + w * t[i] + ut[i])
}

pub fn pack_turn(q: Turn) -> f32 {
    let q = normalized(q);
    let mut big = 0;
    for i in 1..4 {
        if fabsf(q[i]) > fabsf(q[big]) {
            big = i;
        }
    }
    let sign = if q[big] < 0.0 { -1.0 } else { 1.0 };
    let top = (TURN_LEVELS - 1) as f32;
    let mut code = big as u32;
    for (i, &c) in q.iter().enumerate() {
        if i == big {
            continue;
        }
        let level = roundf((c * sign / TURN_SPAN + 1.0) * 0.5 * top).clamp(0.0, top);
        code = code * TURN_LEVELS + level as u32;
    }
    code as f32
}

pub fn unpack_turn(packed: f32) -> Turn {
    if !(packed.is_finite() && packed >= 0.0) {
        return NO_TURN;
    }
    let top = (TURN_LEVELS - 1) as f32;
    let mut code = packed as u32;
    let mut small = [0.0f32; 3];
    for value in small.iter_mut().rev() {
        let level = code % TURN_LEVELS;
        code /= TURN_LEVELS;
        *value = (level as f32 / top * 2.0 - 1.0) * TURN_SPAN;
    }
    let big = (code as usize).min(3);
    let rest = 1.0 - small.iter().map(|c| c * c).sum::<f32>();
    let mut q = [0.0f32; 4];
    let mut next = small.into_iter();
    for (i, value) in q.iter_mut().enumerate() {
        *value = if i == big {
            sqrtf(rest.max(0.0))
        } else {
            next.next().unwrap_or(0.0)
        };
    }
    normalized(q)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wearer::TIME_NEVER;

    pub(crate) struct Owned {
        pub clock: [f32; RING_CLOCK_FLOATS],
        pub points: Vec<f32>,
        pub turns: Vec<f32>,
        pub gates: Vec<f32>,
    }

    impl Owned {
        pub fn new(points: usize) -> Owned {
            let mut clock = [0.0; RING_CLOCK_FLOATS];
            clock[RING_TIME] = TIME_NEVER;
            Owned {
                clock,
                points: vec![0.0; points * RING_POINT_FLOATS],
                turns: Vec::new(),
                gates: Vec::new(),
            }
        }

        pub fn ring(&self) -> Ring<'_> {
            Ring {
                clock: &self.clock,
                points: &self.points,
                turns: &self.turns,
                gates: &self.gates,
            }
        }

        pub fn ring_mut(&mut self) -> RingMut<'_> {
            RingMut {
                clock: &mut self.clock,
                points: &mut self.points,
                turns: &mut self.turns,
                gates: &mut self.gates,
            }
        }

        pub fn sample(&mut self, at: [f32; 3], now: f32) -> usize {
            self.ring_mut().sample(at, now)
        }
    }

    fn fresh() -> Owned {
        Owned::new(RING_POINTS)
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    #[test]
    fn rings_sample_at_twenty_hertz_whatever_the_frame_rate() {
        let mut ring = fresh();
        assert_eq!(ring.ring().len(), 0);
        let mut added = 0;
        for frame in 0..=60 {
            let now = frame as f32 / 60.0;
            added += ring.sample([now, 0.0, 0.0], now);
        }
        let ring = ring.ring();
        assert_eq!(added, 21, "one sample at the start, then one per 50 ms");
        assert_eq!(ring.len(), 21);
        assert!((ring.newest_time().unwrap() - 1.0).abs() < 1e-4);
        assert!(close(ring.point(0).unwrap(), [1.0, 0.0, 0.0]));
        assert!(close(ring.point(20).unwrap(), [0.0, 0.0, 0.0]));
        assert!(close(ring.point(10).unwrap(), [0.5, 0.0, 0.0]));
    }

    fn about_y(degrees: f32) -> Turn {
        let half = degrees.to_radians() / 2.0;
        [0.0, libm::sinf(half), 0.0, libm::cosf(half)]
    }

    fn same_turn(a: Turn, b: Turn, tolerance: f32) -> bool {
        let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
        1.0 - dot.abs() < tolerance
    }

    #[test]
    fn rings_pack_a_turn_into_one_float_and_back() {
        assert_eq!(unpack_turn(pack_turn(NO_TURN)), NO_TURN);
        for (x, y, z, w) in [
            (0.3, -0.5, 0.1, 0.8),
            (-0.9, 0.1, 0.2, -0.1),
            (0.5, 0.5, 0.5, 0.5),
            (0.0, 0.0, -1.0, 0.0),
            (0.01, 0.7, -0.7, 0.02),
        ] {
            let q = normalized([x, y, z, w]);
            let packed = pack_turn(q);
            assert!(
                packed < 16_777_216.0 && packed == roundf(packed),
                "{packed}"
            );
            assert!(same_turn(unpack_turn(packed), q, 1e-4), "{q:?}");
        }
        let v = [1.0, 0.0, 0.0];
        let turned = rotate(about_y(90.0), v);
        assert!(close(turned, [0.0, 0.0, -1.0]), "{turned:?}");
        assert_eq!(unpack_turn(f32::NAN), NO_TURN);
    }
}
