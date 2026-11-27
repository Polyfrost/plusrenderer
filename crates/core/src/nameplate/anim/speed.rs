use std::ops::Range;

use crate::nameplate::types::{Animations, Clip, NameplateAsset, Target, Track};

use super::path::Walk;
use super::tracks::sample;
use super::{clip as anim_clip, ClipRef};

pub const EPISODE_FLOATS: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Episode {
    pub start: i64,
    pub stop: Option<i64>,
}

impl Episode {
    pub fn running(start: i64) -> Episode {
        Episode { start, stop: None }
    }

    pub fn ended(start: i64, stop: i64) -> Episode {
        Episode {
            start,
            stop: Some(stop),
        }
    }

    pub fn steps(&self, step: u16) -> (i64, Option<i64>) {
        let k = i64::from(step.max(1));
        let start = ceil_div(self.start, k);
        (
            start,
            self.stop.map(|stop| ceil_div(stop.max(self.start), k)),
        )
    }
}

fn ceil_div(value: i64, by: i64) -> i64 {
    -(-value).div_euclid(by)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipClock {
    Loop,
    Local(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    first: i64,
    prefix: Vec<i64>,
    periodic: bool,
    head: i64,
    tail: i64,
}

impl Profile {
    pub fn constant(speed: i64) -> Profile {
        Profile {
            first: 0,
            prefix: vec![0, speed],
            periodic: true,
            head: speed,
            tail: speed,
        }
    }

    pub fn empty() -> Profile {
        Profile {
            first: 0,
            prefix: Vec::new(),
            periodic: true,
            head: 0,
            tail: 0,
        }
    }

    fn periodic_into(&mut self, track: &Track, period: u32, scale: u16) {
        let scale = i64::from(scale.max(1));
        self.prefix.clear();
        self.prefix.push(0);
        let mut total = 0;
        let mut first_value = 0;
        let mut constant = true;
        for u in 0..i64::from(period.max(1)) {
            let value = speed_at(track, u * scale);
            if u == 0 {
                first_value = value;
            } else if value != first_value {
                constant = false;
            }
            total += value;
            self.prefix.push(total);
        }
        self.first = 0;
        self.periodic = true;
        if constant {
            self.prefix.clear();
            self.prefix.push(0);
            self.prefix.push(first_value);
            self.head = first_value;
            self.tail = first_value;
        } else {
            self.head = 0;
            self.tail = 0;
        }
    }

    fn local_into(&mut self, track: &Track, step: u16, origin: i64) {
        let k = i64::from(step.max(1));
        let first = ceil_div(origin, k);
        let last_key = track.keys.last().map_or(0, |key| i64::from(key.frame));
        let last = (origin + last_key).div_euclid(k).max(first);
        self.prefix.clear();
        self.prefix.push(0);
        let mut total = 0;
        for u in first..=last {
            total += speed_at(track, u * k - origin);
            self.prefix.push(total);
        }
        self.first = first;
        self.periodic = false;
        self.head = speed_at(track, (first - 1) * k - origin);
        self.tail = speed_at(track, last_key);
    }

    pub fn canonical_origin(origin: i64, step: u16) -> i64 {
        -(-origin).rem_euclid(i64::from(step.max(1)))
    }

    pub fn rebase(&mut self, origin: i64, step: u16) {
        if !self.periodic {
            self.first = ceil_div(origin, i64::from(step.max(1)));
        }
    }

    pub fn periodic(track: &Track, period: u32, scale: u16) -> Profile {
        let mut profile = Profile::empty();
        profile.periodic_into(track, period, scale);
        profile
    }

    pub fn local(track: &Track, step: u16, origin: i64) -> Profile {
        let mut profile = Profile::empty();
        profile.local_into(track, step, origin);
        profile
    }

    pub fn reaction_into(&mut self, track: &Track, clock: ClipClock, step: u16, loop_frames: u16) {
        match clock {
            ClipClock::Loop => {
                let cycle = u32::from(loop_frames) / u32::from(step.max(1));
                self.periodic_into(track, cycle, step);
            }
            ClipClock::Local(origin) => self.local_into(track, step, origin),
        }
    }

    pub fn reaction(track: &Track, clock: ClipClock, step: u16, loop_frames: u16) -> Profile {
        let mut profile = Profile::empty();
        profile.reaction_into(track, clock, step, loop_frames);
        profile
    }

    pub fn constant_speed(&self) -> Option<i64> {
        (self.periodic && self.prefix.len() == 2).then(|| self.prefix[1])
    }

    pub fn period(&self) -> i64 {
        i64::try_from(self.prefix.len().saturating_sub(1))
            .unwrap_or(1)
            .max(1)
    }

    fn total(&self) -> i64 {
        self.prefix.last().copied().unwrap_or(0)
    }

    fn cumulative(&self, x: i64) -> i64 {
        let n = self.period();
        let r = x - self.first;
        if self.periodic {
            r.div_euclid(n) * self.total() + self.prefix[r.rem_euclid(n) as usize]
        } else if r <= 0 {
            r * self.head
        } else if r >= n {
            self.total() + (r - n) * self.tail
        } else {
            self.prefix[r as usize]
        }
    }

    pub fn at(&self, step: i64) -> i64 {
        self.sum(step, step + 1)
    }

    pub fn sum(&self, from: i64, to: i64) -> i64 {
        self.cumulative(to) - self.cumulative(from)
    }
}

pub fn speed_at(track: &Track, frame: i64) -> i64 {
    let frame = frame.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    i64::from(sample(track, frame, &|_| 0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Catch {
    Done,
    Ahead(i64),
    Behind { lag: i64, speed: i64 },
}

fn catch_up(walk: &Walk, reaction: &Profile, start: i64, stop: i64) -> Catch {
    if stop <= start {
        return Catch::Done;
    }
    let raw = reaction.sum(start, stop) - walk.base().sum(start, stop);
    let delta = match walk.modulus() {
        Some(m) if m > 0 => {
            let r = raw.rem_euclid(m);
            if 2 * r > m {
                r - m
            } else {
                r
            }
        }
        _ => raw,
    };
    if delta > 0 {
        Catch::Ahead(delta)
    } else if delta < 0 {
        Catch::Behind {
            lag: -delta,
            speed: reaction.at(stop - 1),
        }
    } else {
        Catch::Done
    }
}

fn gain(base: i64, reaction: i64) -> i64 {
    (reaction - base).max(base)
}

fn base_sum(base: &Profile, from: i64, to: i64, f: impl Fn(i64) -> i64) -> i64 {
    if to <= from {
        return 0;
    }
    let n = base.period();
    let full = (to - from) / n;
    let whole = if full > 0 {
        full * (from..from + n).map(|u| f(base.at(u))).sum::<i64>()
    } else {
        0
    };
    whole + (from + full * n..to).map(|u| f(base.at(u))).sum::<i64>()
}

fn first_reach(base: &Profile, from: i64, target: i64, f: impl Fn(i64) -> i64) -> Option<i64> {
    if target <= 0 {
        return Some(from);
    }
    let n = base.period();
    let per_period: i64 = (from..from + n).map(|u| f(base.at(u))).sum();
    if per_period <= 0 {
        return None;
    }
    let skipped = (target - 1) / per_period;
    let mut reached = skipped * per_period;
    let at = from + skipped * n;
    for u in at..at + n {
        reached += f(base.at(u));
        if reached >= target {
            return Some(u + 1);
        }
    }
    None
}

pub fn offset(walk: &Walk, reaction: &Profile, episode: Episode, step: u16, t: i64) -> i64 {
    let (start, stop) = episode.steps(step);
    if t <= start {
        return 0;
    }
    let base = walk.base();
    let Some(stop) = stop.filter(|stop| *stop < t) else {
        return reaction.sum(start, t) - base.sum(start, t);
    };
    match catch_up(walk, reaction, start, stop) {
        Catch::Done => 0,
        Catch::Ahead(ahead) => (ahead - base.sum(stop, t).div_euclid(2)).max(0),
        Catch::Behind { lag, speed } => (base_sum(base, stop, t, |b| gain(b, speed)) - lag).min(0),
    }
}

pub fn rejoin(walk: &Walk, reaction: &Profile, episode: Episode, step: u16) -> Option<i64> {
    let (start, stop) = episode.steps(step);
    let stop = stop?;
    let base = walk.base();
    match catch_up(walk, reaction, start, stop) {
        Catch::Done => Some(stop.max(start)),
        Catch::Ahead(ahead) => first_reach(base, stop, 2 * ahead, |b| b),
        Catch::Behind { lag, speed } => first_reach(base, stop, lag, |b| gain(b, speed)),
    }
}

pub fn rejoin_frame(walk: &Walk, reaction: &Profile, episode: Episode, step: u16) -> Option<i64> {
    rejoin(walk, reaction, episode, step).map(|u| u * i64::from(step.max(1)))
}

fn has_speed(clip: &Clip) -> bool {
    clip.targets.iter().any(|t| t.channels.speed.is_some())
}

pub fn speed_clips(animations: &Animations) -> Vec<ClipRef> {
    let reactions = animations
        .reactions
        .iter()
        .enumerate()
        .filter(|(_, r)| has_speed(&r.clip))
        .filter_map(|(i, _)| u16::try_from(i).ok().map(ClipRef::Reaction));
    let one_shots = animations
        .one_shots
        .iter()
        .enumerate()
        .filter(|(_, o)| has_speed(&o.clip))
        .filter_map(|(i, _)| u16::try_from(i).ok().map(ClipRef::OneShot));
    reactions.chain(one_shots).collect()
}

pub fn speed_track(asset: &NameplateAsset, clip: ClipRef, layer: usize) -> Option<&Track> {
    let target = Target::Layer(u8::try_from(layer).ok()?);
    anim_clip(asset, clip)?
        .targets
        .iter()
        .find(|t| t.target == target)
        .and_then(|t| t.channels.speed.as_ref())
}

pub fn primary_layer(asset: &NameplateAsset, clip: ClipRef) -> Option<usize> {
    anim_clip(asset, clip)?
        .targets
        .iter()
        .find_map(|t| match t.target {
            Target::Layer(layer) if t.channels.speed.is_some() => Some(usize::from(layer)),
            _ => None,
        })
}

pub fn track(
    floats: &mut [f32],
    on: bool,
    edge: i64,
    now: i64,
    rejoin: impl Fn(Episode, Episode) -> Option<i64>,
) {
    match read_episode(floats) {
        Some(previous) => advance(floats, on, edge, now, |e| rejoin(previous, e)),
        None if on => advance(floats, on, edge, now, |_| None),
        None => {}
    }
}

pub fn replay(
    runs: &[(i64, Option<i64>)],
    now: i64,
    rejoin: impl Fn(Episode, Episode) -> Option<i64>,
) -> Option<Episode> {
    let mut floats = [0.0f32; EPISODE_FLOATS];
    for (index, &(start, stop)) in runs.iter().enumerate() {
        if start > now {
            break;
        }
        let next = runs.get(index + 1).map(|r| r.0).filter(|n| *n <= now);
        let stop = stop.filter(|s| *s <= now && next.is_none_or(|n| *s < n));
        if stop.is_some_and(|s| s <= start) {
            continue;
        }
        track(&mut floats, true, start, start, &rejoin);
        let last = match (stop, next) {
            (Some(stop), _) => stop - 1,
            (None, Some(next)) => next - 1,
            (None, None) => now,
        };
        if last > start {
            track(&mut floats, true, start, last, &rejoin);
        }
        if let Some(stop) = stop {
            track(&mut floats, false, stop, stop, &rejoin);
        }
    }
    read_episode(&floats)
}

pub fn episode_floats(index: usize) -> Range<usize> {
    EPISODE_FLOATS * index..EPISODE_FLOATS * (index + 1)
}

pub fn read_episode(floats: &[f32]) -> Option<Episode> {
    let (&start, &stop) = (floats.first()?, floats.get(1)?);
    if !start.is_finite() || stop.is_nan() || stop <= start {
        return None;
    }
    let stop = stop.is_finite().then_some(stop as i64);
    Some(Episode {
        start: start as i64,
        stop,
    })
}

pub fn write_episode(floats: &mut [f32], episode: Option<Episode>) {
    let [start, stop, ..] = floats else {
        return;
    };
    match episode {
        None => {
            *start = 0.0;
            *stop = 0.0;
        }
        Some(episode) => {
            *start = episode.start as f32;
            *stop = episode.stop.map_or(f32::INFINITY, |s| s as f32);
        }
    }
}

fn begin(
    previous: Option<Episode>,
    edge: i64,
    now: i64,
    rejoin: &impl Fn(Episode) -> Option<i64>,
) -> Option<Episode> {
    let earliest = match previous {
        Some(ended) => {
            let stop = ended.stop.unwrap_or(edge);
            edge.max(stop).max(rejoin(ended).unwrap_or(stop))
        }
        None => edge,
    };
    if earliest <= now {
        Some(Episode::running(earliest))
    } else {
        previous
    }
}

pub fn advance(
    floats: &mut [f32],
    on: bool,
    edge: i64,
    now: i64,
    rejoin: impl Fn(Episode) -> Option<i64>,
) {
    let current =
        read_episode(floats).filter(|e| e.start <= now && e.stop.is_none_or(|stop| stop <= now));
    let next = match (current, on) {
        (Some(Episode { start, stop: None }), true) if edge > start => begin(
            Some(Episode::ended(start, edge.min(now))),
            edge,
            now,
            &rejoin,
        ),
        (Some(Episode { start, stop: None }), false) => {
            Some(Episode::ended(start, edge.clamp(start, now)))
        }
        (Some(Episode { stop: None, .. }), true) => current,
        (previous, true) => begin(previous, edge, now, &rejoin),
        (other, false) => other,
    };
    write_episode(floats, next);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::route::polyline;
    use crate::nameplate::types::{Ease, Facings, Key, Path, PathEnd, PathShape, PathSpeed, Preset, Value, Walkers};

    fn track(keys: &[(u16, i32, Ease)]) -> Track {
        Track { keys: keys.iter().map(|&(frame, v, ease)| Key { frame, value: Value::Fixed(v), ease }).collect() }
    }

    fn line_walk(length: i32, speed: Track, end: PathEnd, cycle: u32) -> Walk {
        let (shape, facings, walkers) = (PathShape::Preset(Preset::Top), Facings::default(), Walkers::default());
        let path = Path { shape, follow: None, speed: PathSpeed::Track(speed), end, reverse: false, stops: Vec::new(), facings, walkers };
        Walk::new(&path, &polyline(&[(0, 0), (length, 0)], false), cycle)
    }

    #[test]
    fn profiles_sum_any_range_like_adding_each_step() {
        let t = track(&[(0, 100, Ease::Step), (6, 700, Ease::Linear), (14, -300, Ease::Ease), (20, 50, Ease::Step)]);
        for (profile, step, origin) in [(Profile::periodic(&t, 12, 2), 2, None), (Profile::local(&t, 3, 7), 3, Some(7)), (Profile::local(&t, 4, -9), 4, Some(-9))] {
            let brute = |u: i64| origin.map_or_else(|| speed_at(&t, u.rem_euclid(12) * step), |o| speed_at(&t, u * step - o));
            for a in -30..40 {
                assert_eq!(profile.at(a), brute(a), "{a}");
                (a..a + 25).for_each(|b| assert_eq!(profile.sum(a, b), (a..b).map(brute).sum::<i64>(), "{a} {b}"));
            }
        }
        assert_eq!(Profile::periodic(&track(&[(0, 300, Ease::Step)]), 12, 1), Profile::constant(300));
        assert_eq!(Profile::reaction(&t, ClipClock::Local(9), 3, 24), Profile::local(&t, 3, 9));
    }

    #[test]
    fn catch_up_never_moves_backwards_or_jumps_and_late_joiners_agree_once_it_ends() {
        let mut seed = 7u64;
        let mut next = |range: i64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((seed >> 33) % range.max(1) as u64) as i64
        };
        let eases = [Ease::Step, Ease::Linear, Ease::Ease];
        for case in 0..200 {
            let cycle = 1 + next(30) as u32;
            let base = track(&[(0, 1 + next(900) as i32, Ease::Step), ((cycle / 2) as u16, next(1200) as i32, eases[next(3) as usize])]);
            let walk = line_walk(3 + next(60) as i32, base, [PathEnd::Wrap, PathEnd::Bounce, PathEnd::Stop][next(3) as usize], cycle);
            let keys = [(0, next(8192) as i32 - 4096, Ease::Step), (next(24) as u16 + 1, next(8192) as i32 - 4096, eases[next(3) as usize])];
            let step = 1 + next(3) as u16;
            let reaction = Profile::reaction(&track(&keys), ClipClock::Loop, step, (cycle * u32::from(step)) as u16);
            let start = next(200) - 50;
            let episode = Episode::ended(start, start + next(150));
            let late = Episode::ended(start + next(40), episode.steps(1).1.unwrap());
            let (s, e) = episode.steps(step);
            let e = e.unwrap();
            let fastest = 2 * (0..i64::from(cycle)).map(|u| walk.base().at(u)).max().unwrap();
            let fastest = fastest.max(reaction.at(e - 1)) + 1;
            let (Some(back), Some(late_back)) = (rejoin(&walk, &reaction, episode, step), rejoin(&walk, &reaction, late, step)) else {
                continue;
            };
            let at = |t: i64| walk.base().sum(0, t) + offset(&walk, &reaction, episode, step, t);
            for t in s..e {
                let expected: i64 = (s..t + 1).map(|u| reaction.at(u) - walk.base().at(u)).sum();
                assert_eq!(offset(&walk, &reaction, episode, step, t + 1), expected);
            }
            for t in e + 1..back + 3 * i64::from(cycle) {
                let moved = walk.modulus().map_or(at(t) - at(t - 1), |m| (at(t) - at(t - 1)).rem_euclid(m));
                assert!((0..=fastest).contains(&moved), "{case} {t} moved by {moved}");
                assert_eq!(offset(&walk, &reaction, episode, step, t) == 0, t >= back, "{case} {t}");
            }
            let both = back.max(late_back);
            assert!((both..both + 100).all(|t| offset(&walk, &reaction, late, step, t) == 0), "{case}: late joiners rejoin");
        }
    }

    #[test]
    fn the_record_holds_only_the_episode_start_and_stop() {
        let walk = line_walk(40, track(&[(0, 256, Ease::Step)]), PathEnd::Wrap, 24);
        let still = Profile::constant(0);
        let rejoin = |e| rejoin_frame(&walk, &still, e, 1);
        let mut floats = [0.0_f32; 2];
        let steps = [(false, 3, 4, None), (true, 5, 7, Some(Episode::running(5))), (false, 12, 13, Some(Episode::ended(5, 12))),
            (true, 14, 18, Some(Episode::ended(5, 12))), (true, 14, 23, Some(Episode::running(19))), (true, 2, 3, Some(Episode::running(2))), (false, 1, 1, None)];
        for (on, at, now, expected) in steps {
            advance(&mut floats, on, at, now, &rejoin);
            assert_eq!(read_episode(&floats), expected, "{on} {at} {now}");
        }
        write_episode(&mut floats, Some(Episode::running(8_388_000)));
        assert_eq!(floats, [8_388_000.0, f32::INFINITY]);
        assert_eq!((episode_floats(2), EPISODE_FLOATS), (4..6, 2));
    }
}
