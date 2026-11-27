use libm::floor;

use crate::nameplate::geom::{Block, Rect};
use crate::nameplate::layout::anchor_point;
use crate::nameplate::route::{polyline, preset, Facing, Route, RouteKey};
use crate::nameplate::types::{Body, Path, PathEnd, PathShape, PathSpeed};

use super::speed::Profile;
use super::walker::{facing_frame, heading, lag, Lag};

pub const UNIT: i64 = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PathPose {
    pub x: i32,
    pub y: i32,
    pub facing: Option<u16>,
    pub flip: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Progress {
    pub lap: i64,
    pub along: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Leg {
    depart: i64,
    travel: i64,
    from: i64,
    to: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Timing {
    Speed,
    Laps(i64),
    Paused {
        legs: Vec<Leg>,
        period: i64,
        moving: i64,
        laps: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Walk {
    length: i64,
    end: PathEnd,
    reverse: bool,
    cycle: i64,
    base: Profile,
    timing: Timing,
}

pub fn path_time(frame: i64, step: u16) -> i64 {
    frame.div_euclid(i64::from(step.max(1)))
}

pub fn walk_gate(path: &Path, paths: bool, frame: i64, step: u16) -> (i64, u8) {
    if paths {
        (path_time(frame, step), path.walkers.count.max(1))
    } else {
        (0, 1)
    }
}

pub fn cycle(loop_frames: u16, step: u16) -> u32 {
    u32::from(loop_frames) / u32::from(step.max(1))
}

fn rounded(value: i64, over: i64) -> i64 {
    if over <= 0 {
        0
    } else {
        (2 * value + over).div_euclid(2 * over)
    }
}

fn checkpoints(path: &Path, route: &Route, length: i64) -> Vec<(i64, i64)> {
    let mut marks = Vec::new();
    if let PathShape::Points(points) = &path.shape {
        for (point, vertex) in points.iter().zip(&route.vertices) {
            if point.pause > 0 {
                let at = i64::from(*vertex).min(length);
                let at = if path.reverse { length - at } else { at };
                marks.push((at, i64::from(point.pause)));
            }
        }
    }
    for stop in &path.stops {
        let at = floor(stop.at * length as f64 + 0.5) as i64;
        marks.push((at.clamp(0, length), i64::from(stop.pause)));
    }
    if path.end == PathEnd::Bounce {
        let forward = marks.len();
        for i in 0..forward {
            let (at, pause) = marks[i];
            if at > 0 && at < length {
                marks.push((2 * length - at, pause));
            }
        }
    }
    marks.retain(|(_, pause)| *pause > 0);
    marks
}

fn legs(
    marks: &[(i64, i64)],
    sequence: i64,
    speed: i64,
    laps: Option<(i64, i64)>,
) -> (Vec<Leg>, i64, i64) {
    let mut stations: Vec<(i64, i64)> = vec![(0, 0), (sequence, 0)];
    for &(at, pause) in marks {
        match stations.iter_mut().find(|(s, _)| *s == at) {
            Some(station) => station.1 += pause,
            None => stations.push((at, pause)),
        }
    }
    stations.sort_unstable();
    let waits: i64 = stations.iter().map(|(_, pause)| pause).sum();
    let moving = match laps {
        Some((cycle, laps)) => (cycle / laps.max(1) - waits).max(1),
        None => 0,
    };
    let mut legs = Vec::with_capacity(stations.len());
    let mut time = stations[0].1;
    let mut arrived = 0;
    for pair in stations.windows(2) {
        let ((from, _), (to, pause)) = (pair[0], pair[1]);
        let travel = match laps {
            Some(_) => {
                let reached = rounded(to * moving, sequence);
                let travel = reached - arrived;
                arrived = reached;
                travel
            }
            None => ((to - from) * UNIT + speed - 1) / speed,
        };
        legs.push(Leg {
            depart: time,
            travel,
            from,
            to,
        });
        time += travel + pause;
    }
    (legs, time.max(1), moving)
}

impl Walk {
    pub fn new(path: &Path, route: &Route, cycle: u32) -> Walk {
        let length = i64::from(route.length());
        let cycle = i64::from(cycle.max(1));
        let sequence = if path.end == PathEnd::Bounce {
            2 * length
        } else {
            length
        };
        let laps = match path.speed {
            PathSpeed::Laps(laps) => Some(i64::from(laps.max(1))),
            PathSpeed::Track(_) => None,
        };
        let base = match &path.speed {
            PathSpeed::Track(track) => Profile::periodic(track, cycle as u32, 1),
            PathSpeed::Laps(_) => Profile::constant(laps.unwrap_or(1) * sequence * UNIT / cycle),
        };
        let marks = checkpoints(path, route, length);
        let timing = if !marks.is_empty() && sequence > 0 {
            let speed = base
                .constant_speed()
                .unwrap_or_else(|| base.sum(0, cycle) / cycle)
                .max(1);
            let (legs, period, moving) =
                legs(&marks, sequence, speed, laps.map(|laps| (cycle, laps)));
            Timing::Paused {
                legs,
                period,
                moving,
                laps: laps.is_some(),
            }
        } else if let Some(laps) = laps {
            Timing::Laps(laps)
        } else {
            Timing::Speed
        };
        Walk {
            length,
            end: path.end,
            reverse: path.reverse,
            cycle,
            base,
            timing,
        }
    }

    pub fn length(&self) -> i64 {
        self.length
    }

    pub fn end(&self) -> PathEnd {
        self.end
    }

    pub fn sequence(&self) -> i64 {
        if self.end == PathEnd::Bounce {
            2 * self.length
        } else {
            self.length
        }
    }

    pub fn span(&self) -> i64 {
        self.sequence() * UNIT
    }

    pub fn base(&self) -> &Profile {
        &self.base
    }

    fn speed(&self) -> i64 {
        self.base.constant_speed().unwrap_or(1).max(1)
    }

    pub fn modulus(&self) -> Option<i64> {
        if self.end == PathEnd::Stop {
            return None;
        }
        let modulus = match &self.timing {
            Timing::Paused { period, .. } => period * self.speed(),
            _ => self.span(),
        };
        (modulus > 0).then_some(modulus)
    }

    pub fn period(&self) -> i64 {
        match &self.timing {
            Timing::Paused { period, .. } => *period,
            Timing::Laps(laps) => self.cycle / laps,
            Timing::Speed => self.cycle,
        }
    }

    pub fn paused(&self) -> bool {
        matches!(self.timing, Timing::Paused { .. })
    }

    pub fn roams(&self) -> bool {
        self.end != PathEnd::Stop && self.base.sum(0, self.cycle) != 0
    }

    pub fn repeat_loops(&self) -> i64 {
        if !self.roams() {
            return 1;
        }
        let gcd = |mut a: i64, mut b: i64| {
            while b != 0 {
                (a, b) = (b, a.rem_euclid(b));
            }
            a.abs().max(1)
        };
        let loops = match &self.timing {
            Timing::Laps(_) => 1,
            Timing::Paused { period, .. } => period / gcd(*period, self.cycle),
            Timing::Speed if self.span() > 0 => {
                let span = self.span();
                span / gcd(self.base.sum(0, self.cycle).rem_euclid(span), span)
            }
            Timing::Speed => 1,
        };
        loops.max(1)
    }

    pub fn spacing_time(&self, pixels: i64) -> i64 {
        match &self.timing {
            Timing::Paused {
                moving, laps: true, ..
            } => rounded(pixels * UNIT * moving, self.span()),
            _ => rounded(pixels * UNIT, self.speed()),
        }
    }

    pub fn restarts_at(&self, t: i64) -> bool {
        self.end == PathEnd::Stop && t.rem_euclid(self.cycle) == 0
    }

    fn loop_time(&self, t: i64) -> i64 {
        if self.end == PathEnd::Stop {
            t.rem_euclid(self.cycle)
        } else {
            t
        }
    }

    fn distance(&self, t: i64) -> i64 {
        let t = self.loop_time(t);
        match self.timing {
            Timing::Laps(laps) => (t * laps * self.span()).div_euclid(self.cycle),
            _ => self.base.sum(0, t),
        }
    }

    fn timed(&self, legs: &[Leg], period: i64, laps: bool, t: i64) -> Progress {
        let span = self.span();
        let (lap, time) = match self.end {
            PathEnd::Stop if t < 0 => return Progress::default(),
            PathEnd::Stop if t >= period => {
                return Progress {
                    lap: 0,
                    along: span,
                }
            }
            PathEnd::Stop => (0, t),
            _ => (t.div_euclid(period), t.rem_euclid(period)),
        };
        let along = match legs.iter().rposition(|leg| leg.depart <= time) {
            None => legs.first().map_or(0, |leg| leg.from * UNIT),
            Some(index) => {
                let leg = legs[index];
                let local = time - leg.depart;
                let length = (leg.to - leg.from) * UNIT;
                if local >= leg.travel {
                    leg.to * UNIT
                } else if laps {
                    leg.from * UNIT + local * length / leg.travel
                } else {
                    leg.from * UNIT + (local * self.speed()).min(length)
                }
            }
        };
        Progress { lap, along }
    }

    pub fn progress(&self, t: i64, shift: i64, lag: Lag) -> Progress {
        let span = self.span();
        if span == 0 {
            return Progress::default();
        }
        if let Timing::Paused {
            legs, period, laps, ..
        } = &self.timing
        {
            let time = self.loop_time(t) - lag.time + shift.div_euclid(self.speed());
            return self.timed(legs, *period, *laps, time);
        }
        let distance = self.distance(t) + shift - lag.distance;
        match self.end {
            PathEnd::Stop => Progress {
                lap: 0,
                along: distance.clamp(0, span),
            },
            _ => Progress {
                lap: distance.div_euclid(span),
                along: distance.rem_euclid(span),
            },
        }
    }

    pub fn unwrapped(&self, progress: Progress) -> i64 {
        progress.lap * self.span() + progress.along
    }

    pub fn locate(&self, progress: Progress) -> i64 {
        let length = self.length;
        let q = progress.along.div_euclid(UNIT).clamp(0, self.sequence());
        let at = if self.end == PathEnd::Bounce && q > length {
            2 * length - q
        } else {
            q
        };
        if self.reverse {
            length - at
        } else {
            at
        }
    }

    pub fn seq_facing(&self, route: &Route, index: i64) -> Option<Facing> {
        let length = self.length;
        if length <= 0 || index < 0 || index >= self.sequence() {
            return None;
        }
        let (step, back) = if index >= length {
            (2 * length - 1 - index, true)
        } else {
            (index, false)
        };
        let facing = if self.reverse {
            route.facing(length - 1 - step).map(Facing::opposite)
        } else {
            route.facing(step)
        };
        if back {
            facing.map(Facing::opposite)
        } else {
            facing
        }
    }
}

pub fn route_for(
    path: &Path,
    rect: Rect,
    block: &Block,
    body: Option<&Body>,
    size: [u16; 2],
) -> Route {
    match &path.shape {
        PathShape::Preset(side) => preset(*side, rect, path.follow, body, size),
        PathShape::Points(points) => {
            let spots: Vec<(i32, i32)> = points
                .iter()
                .map(|point| {
                    let (x, y) = anchor_point(point.anchor, None, false, rect, block);
                    (
                        x + i32::from(point.offset[0]),
                        y + i32::from(point.offset[1]),
                    )
                })
                .collect();
            polyline(&spots, false)
        }
    }
}

pub fn route_key(
    layer: u8,
    path: &Path,
    rect: Rect,
    body: Option<&Body>,
    size: [u16; 2],
) -> Option<RouteKey> {
    match &path.shape {
        PathShape::Preset(side) => Some(RouteKey::new(
            layer,
            body,
            rect,
            path.follow,
            Some(*side),
            size,
        )),
        PathShape::Points(_) => None,
    }
}

pub fn walker_pose(
    walk: &Walk,
    route: &Route,
    path: &Path,
    index: u8,
    t: i64,
    shift: &dyn Fn(i64) -> i64,
) -> PathPose {
    let lag = lag(walk, &path.walkers, index);
    let at = |t: i64| walk.progress(t, shift(t), lag);
    let now = at(t);
    let moving = if walk.restarts_at(t) {
        heading(walk.unwrapped(now), walk.unwrapped(at(t + 1)))
    } else {
        heading(walk.unwrapped(at(t - 1)), walk.unwrapped(now))
    };
    let (x, y) = route.at(walk.locate(now));
    let (facing, flip) = facing_frame(walk, route, &path.facings, now, moving, t);
    PathPose { x, y, facing, flip }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::route::{polyline, preset};
    use crate::nameplate::types::{Ease, Facings, Key, PathShape, PathSpeed, PathStop, Preset, Track, Value, Walkers};

    fn top(speed: PathSpeed, end: PathEnd, stops: Vec<PathStop>) -> Path {
        let (shape, facings, walkers) = (PathShape::Preset(Preset::Top), Facings::default(), Walkers::default());
        Path { shape, follow: None, speed, end, reverse: false, stops, facings, walkers }
    }

    fn keyed(keys: &[(u16, i32)]) -> PathSpeed {
        PathSpeed::Track(Track { keys: keys.iter().map(|&(frame, v)| Key { frame, value: Value::Fixed(v), ease: Ease::Step }).collect() })
    }

    fn xs(path: &Path, length: i32, cycle: u32, frames: std::ops::Range<i64>) -> Vec<i32> {
        let route = polyline(&[(0, 0), (length, 0)], false);
        let walk = Walk::new(path, &route, cycle);
        frames.map(|t| walker_pose(&walk, &route, path, 0, t, &|_| 0).x).collect()
    }

    #[test]
    fn walkers_wrap_bounce_stop_pause_and_finish_whole_laps() {
        let pause = || vec![PathStop { at: 0.5, pause: 3 }];
        let one = keyed(&[(0, 256)]);
        assert_eq!(xs(&top(one.clone(), PathEnd::Wrap, vec![]), 4, 24, -3..6), [1, 2, 3, 0, 1, 2, 3, 0, 1]);
        assert_eq!(xs(&top(keyed(&[(0, 128)]), PathEnd::Wrap, vec![]), 4, 24, 0..9), [0, 0, 1, 1, 2, 2, 3, 3, 0]);
        assert_eq!(xs(&top(one.clone(), PathEnd::Bounce, vec![]), 4, 24, 0..10), [0, 1, 2, 3, 4, 3, 2, 1, 0, 1]);
        assert_eq!(xs(&top(one.clone(), PathEnd::Stop, vec![]), 4, 8, 0..10), [0, 1, 2, 3, 4, 4, 4, 4, 0, 1]);
        assert_eq!(xs(&top(one.clone(), PathEnd::Wrap, pause()), 4, 24, 0..8), [0, 1, 2, 2, 2, 2, 3, 0]);
        assert_eq!(xs(&top(one.clone(), PathEnd::Bounce, pause()), 4, 24, 7..15), [4, 3, 2, 2, 2, 2, 1, 0]);
        let mut reversed = top(one.clone(), PathEnd::Wrap, vec![]);
        reversed.reverse = true;
        assert_eq!(xs(&reversed, 4, 24, 0..6), [4, 3, 2, 1, 4, 3]);
        assert_eq!(xs(&top(keyed(&[(0, 256), (5, 512)]), PathEnd::Wrap, vec![]), 100, 10, 0..12), [0, 1, 2, 3, 4, 5, 7, 9, 11, 13, 15, 16]);
        let laps = xs(&top(PathSpeed::Laps(2), PathEnd::Wrap, vec![]), 10, 20, 0..40);
        assert_eq!((&laps[..10], &laps[10..20], &laps[20..]), (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9][..], &laps[..10], &laps[..20]));
        let paused = xs(&top(PathSpeed::Laps(2), PathEnd::Wrap, vec![PathStop { at: 0.5, pause: 5 }]), 10, 30, 0..30);
        assert_eq!((&paused[..15], &paused[15..]), (&[0, 1, 2, 3, 4, 5, 5, 5, 5, 5, 5, 6, 7, 8, 9][..], &paused[..15]));
        assert_eq!((path_time(11, 4), path_time(-1, 4), cycle(24, 4)), (2, -1, 6));
    }

    #[test]
    fn several_walkers_trail_shift_with_speed_and_stay_pure_functions_of_the_frame() {
        let route = polyline(&[(0, 0), (12, 0)], false);
        let mut path = top(keyed(&[(0, 256)]), PathEnd::Wrap, vec![]);
        path.walkers = Walkers { count: 3, spacing: 2, even: false };
        let walk = Walk::new(&path, &route, 24);
        let at = |index, t| walker_pose(&walk, &route, &path, index, t, &|_| 0).x;
        assert_eq!((at(0, 6), at(1, 6), at(2, 6), at(2, 1)), (6, 4, 2, 9));
        assert_eq!(walker_pose(&walk, &route, &path, 0, 2, &|t| if t >= 2 { 3 * 256 } else { 0 }).x, 5);
        let rect = Rect::new(0, 0, 40, 8);
        let route = preset(Preset::Perimeter, rect, None, None, [0, 0]);
        let mut path = top(keyed(&[(0, 300)]), PathEnd::Bounce, vec![PathStop { at: 0.25, pause: 7 }, PathStop { at: 0.8, pause: 2 }]);
        path.shape = PathShape::Preset(Preset::Perimeter);
        path.walkers = Walkers { count: 4, spacing: 5, even: false };
        let walk = Walk::new(&path, &route, 24);
        let poses = |order: &mut dyn Iterator<Item = i64>| {
            let mut all: Vec<_> = order.flat_map(|t| (0..4).map(move |i| (t, i))).map(|(t, i)| (t, i, walker_pose(&walk, &route, &path, i, t, &|_| 0))).collect();
            all.sort_by_key(|(t, i, _)| (*t, *i));
            all
        };
        assert_eq!(poses(&mut (0..300)), poses(&mut (0..300).rev()));
        assert_eq!(Walk::new(&path, &route, 24), walk);
    }
}
