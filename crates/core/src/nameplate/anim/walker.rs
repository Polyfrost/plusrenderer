use std::cmp::Ordering;

use crate::nameplate::route::{Facing, Route};
use crate::nameplate::types::{Facings, PathEnd, Walkers};

use super::path::{Progress, Walk, UNIT};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Lag {
    pub distance: i64,
    pub time: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Heading {
    Forward,
    Backward,
    Still,
}

pub fn lag(walk: &Walk, walkers: &Walkers, index: u8) -> Lag {
    let i = i64::from(index);
    if walkers.even {
        let count = i64::from(walkers.count.max(1));
        return Lag {
            distance: walk.span() * i / count,
            time: walk.period() * i / count,
        };
    }
    let spacing = i64::from(walkers.spacing);
    Lag {
        distance: i * spacing * UNIT,
        time: i * walk.spacing_time(spacing),
    }
}

pub fn heading(before: i64, now: i64) -> Heading {
    match now.cmp(&before) {
        Ordering::Greater => Heading::Forward,
        Ordering::Less => Heading::Backward,
        Ordering::Equal => Heading::Still,
    }
}

fn frames(facings: &Facings, facing: Facing) -> &[u16] {
    match facing {
        Facing::Right => &facings.right,
        Facing::Left => &facings.left,
        Facing::Up => &facings.up,
        Facing::Down => &facings.down,
    }
}

fn turned(facing: Option<Facing>, backward: bool) -> Option<Facing> {
    if backward {
        facing.map(Facing::opposite)
    } else {
        facing
    }
}

fn horizontal(facing: Option<Facing>) -> Option<Facing> {
    facing.filter(|f| matches!(f, Facing::Right | Facing::Left))
}

fn last_horizontal(walk: &Walk, route: &Route, index: i64, backward: bool) -> Facing {
    let n = walk.sequence();
    let wraps = walk.end() != PathEnd::Stop;
    for distance in 1..=n {
        let k = if backward {
            index + distance
        } else {
            index - distance
        };
        let k = if wraps {
            k.rem_euclid(n)
        } else if k < 0 || k >= n {
            break;
        } else {
            k
        };
        if let Some(found) = horizontal(turned(walk.seq_facing(route, k), backward)) {
            return found;
        }
    }
    Facing::Right
}

pub fn can_flip(facings: &Facings) -> bool {
    let framed = !(facings.right.is_empty()
        && facings.left.is_empty()
        && facings.up.is_empty()
        && facings.down.is_empty());
    framed && (facings.right.is_empty() || facings.left.is_empty())
}

pub fn facing_frame(
    walk: &Walk,
    route: &Route,
    facings: &Facings,
    progress: Progress,
    heading: Heading,
    t: i64,
) -> (Option<u16>, bool) {
    if facings.right.is_empty()
        && facings.left.is_empty()
        && facings.up.is_empty()
        && facings.down.is_empty()
    {
        return (None, false);
    }
    let n = walk.sequence();
    let wraps = walk.end() != PathEnd::Stop;
    let q = progress.along.div_euclid(UNIT).clamp(0, n);
    let (index, backward) = match heading {
        Heading::Forward => (q.min(n - 1), false),
        Heading::Backward if q > 0 => (q - 1, true),
        Heading::Backward => (if wraps { n - 1 } else { 0 }, true),
        Heading::Still if q > 0 => (q - 1, false),
        Heading::Still => (if wraps { n - 1 } else { 0 }, false),
    };
    let facing = match turned(walk.seq_facing(route, index), backward) {
        Some(f @ (Facing::Right | Facing::Left)) => f,
        Some(f) if !frames(facings, f).is_empty() => f,
        _ => last_horizontal(walk, route, index, backward),
    };
    let own = frames(facings, facing);
    let (shown, flip) = if own.is_empty() {
        (frames(facings, facing.opposite()), true)
    } else {
        (own, false)
    };
    let frame = match (heading, shown.len()) {
        (_, 0) => None,
        (Heading::Still, _) => shown.first().copied(),
        (_, len) => shown.get(t.rem_euclid(len as i64) as usize).copied(),
    };
    (frame, flip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::anim::path::{walker_pose, UNIT};
    use crate::nameplate::geom::Rect;
    use crate::nameplate::route::preset;
    use crate::nameplate::types::{Ease, Key, Path, PathEnd, PathShape, PathSpeed, Preset, Track, Value};

    #[test]
    fn walkers_lag_face_their_direction_and_keep_the_last_horizontal_facing() {
        let route = preset(Preset::Perimeter, Rect::new(0, 0, 4, 2), None, None, [0, 0]);
        let speed = PathSpeed::Track(Track { keys: vec![Key { frame: 0, value: Value::Fixed(256), ease: Ease::Step }] });
        let (shape, walkers) = (PathShape::Preset(Preset::Perimeter), Walkers::default());
        let path = |facings| Path { shape: shape.clone(), follow: None, speed: speed.clone(), end: PathEnd::Wrap, reverse: false, stops: Vec::new(), facings, walkers };
        let right = || Facings { right: vec![10, 11], ..Facings::default() };
        let facing = |path: &Path, t, offset: &dyn Fn(i64) -> i64| {
            let pose = walker_pose(&Walk::new(path, &route, 24), &route, path, 0, t, offset);
            (pose.facing, pose.flip)
        };
        let plain = path(right());
        let still = |_| 0;
        let at: Vec<_> = [0, 1, 5, 6, 9, 12].iter().map(|&t| facing(&plain, t, &still)).collect();
        assert_eq!(at, [(Some(10), false), (Some(11), false), (Some(11), false), (Some(10), true), (Some(11), true), (Some(10), false)]);
        let vertical = path(Facings { down: vec![20, 21], up: vec![30], ..right() });
        assert_eq!((facing(&vertical, 5, &still), facing(&vertical, 11, &still)), ((Some(21), false), (Some(30), false)));
        assert_eq!(facing(&plain, 1, &|t| -512 * t), (Some(11), true), "a reversed walker turns around");
        assert_eq!(facing(&path(Facings::default()), 6, &still), (None, false));
        let walk = Walk::new(&plain, &route, 24);
        let spaced = Walkers { count: 4, spacing: 3, even: false };
        assert_eq!((lag(&walk, &spaced, 0), lag(&walk, &spaced, 2).distance), (Lag::default(), 6 * UNIT));
        assert_eq!(lag(&walk, &Walkers { count: 4, spacing: 0, even: true }, 3).distance, 9 * UNIT);
        assert_eq!([heading(5, 9), heading(9, 5), heading(7, 7)], [Heading::Forward, Heading::Backward, Heading::Still]);
    }
}
