use crate::limits::PARTICLES_PER_EMITTER;
use crate::nameplate::geom::{Anchor, Block, Edge, Rect};
use crate::nameplate::layout::{anchor_point, row_insets};
use crate::nameplate::route::{Route, RouteKey};
use crate::nameplate::types::{
    Animations, Body, Burst, Clip, Emitter, Fade, Follow, LayerContent, NameplateAsset,
    ParticleFrames, ParticlePose, Preset, Spawn, SpawnShape, Sprite, Target, OPAQUE,
};
use crate::wearer::seed as mix;

pub const MOTION_UNIT: i64 = 256;

const SALT_TIME: u64 = 1;
const SALT_LIFE: u64 = 2;
const SALT_X: u64 = 3;
const SALT_Y: u64 = 4;
const SALT_VX: u64 = 5;
const SALT_VY: u64 = 6;
const SALT_SPRITE: u64 = 7;
const SALT_WOBBLE: u64 = 8;
const SALT_RATE: u64 = 9;
const SALT_BURST: u64 = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BurstStart {
    pub start: i64,
    pub count: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnArea<'a> {
    pub rect: Rect,
    pub point: (i32, i32),
    pub corners: [u8; 4],
    pub route: Option<&'a Route>,
}

impl<'a> SpawnArea<'a> {
    pub fn of_rect(spawn: &Spawn, rect: Rect) -> SpawnArea<'a> {
        let anchor = match spawn.anchor {
            Anchor::NameLeft => Anchor::Left,
            Anchor::NameRight => Anchor::Right,
            other => other,
        };
        SpawnArea {
            rect,
            point: anchor_point(anchor, None, false, rect, &Block::default()),
            corners: [0; 4],
            route: None,
        }
    }

    pub fn new(
        spawn: &Spawn,
        rect: Rect,
        block: &Block,
        body: Option<&Body>,
        route: Option<&'a Route>,
    ) -> SpawnArea<'a> {
        let corners = match (spawn.shape, spawn.follow, body) {
            (SpawnShape::Area, Some(Follow::Inside), Some(body)) if body.art.is_none() => {
                body.corners
            }
            _ => [0; 4],
        };
        let traced = spawn.shape == SpawnShape::Border && spawn.follow.is_some();
        SpawnArea {
            rect,
            point: anchor_point(spawn.anchor, None, false, rect, block),
            corners,
            route: route.filter(|_| traced),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScheduledBursts {
    pub window: u16,
    pub jitter: u16,
    pub events: Vec<Vec<Burst>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BurstSources {
    pub looped: Vec<Burst>,
    pub scheduled: Vec<ScheduledBursts>,
    pub one_shots: Vec<(Vec<Burst>, u32)>,
}

fn pick(hash: u64, salt: u64, range: i64) -> i64 {
    if range <= 1 {
        0
    } else {
        (mix(hash, salt, 0) % range as u64) as i64
    }
}

fn layer_salt(layer: u8, salt: u64) -> u64 {
    (u64::from(layer) << 8) | salt
}

fn sprite_shape(sprites: &[Sprite], id: u16) -> (i32, i32, i64) {
    sprites.get(usize::from(id)).map_or((1, 1, 1), |s| {
        (
            i32::from(s.width),
            i32::from(s.height),
            (s.frames.len() as i64).max(1),
        )
    })
}

pub fn box_size(emitter: &Emitter, sprites: &[Sprite]) -> [u16; 2] {
    emitter.sprites.iter().fold([0, 0], |[w, h], id| {
        let (sw, sh, _) = sprite_shape(sprites, *id);
        [
            w.max(u16::try_from(sw).unwrap_or(0)),
            h.max(u16::try_from(sh).unwrap_or(0)),
        ]
    })
}

pub fn route_key(
    layer: u8,
    emitter: &Emitter,
    sprites: &[Sprite],
    rect: Rect,
    body: Option<&Body>,
) -> Option<RouteKey> {
    let follow = emitter.spawn.follow?;
    (emitter.spawn.shape == SpawnShape::Border).then(|| {
        RouteKey::new(
            layer,
            body,
            rect,
            Some(follow),
            Some(Preset::Perimeter),
            box_size(emitter, sprites),
        )
    })
}

pub fn motion(velocity: i32, gravity: i32, age: i64) -> i32 {
    let doubled = 2 * i64::from(velocity) * age + i64::from(gravity) * age * age;
    let moved = (doubled + MOTION_UNIT).div_euclid(2 * MOTION_UNIT);
    moved.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub fn wave(amplitude: i32, phase: i64, period: u16) -> i32 {
    let period = i64::from(period.max(1));
    let u = phase.rem_euclid(period);
    let v = (2 * u - period).abs();
    (i64::from(amplitude) * (period - 2 * v) / period) as i32
}

pub fn fade_opacity(fade: Option<&Fade>, age: i64, life: i64) -> u8 {
    let Some(fade) = fade else {
        return OPAQUE;
    };
    let steps = i64::from(fade.steps.max(1));
    let start = libm::floor(fade.after * life as f64) as i64;
    if age < start {
        return OPAQUE;
    }
    let level = ((age - start + 1) * steps / (life - start).max(1)).clamp(1, steps);
    let drop = (i64::from(OPAQUE) - i64::from(fade.to)) * level / steps;
    (i64::from(OPAQUE) - drop).clamp(0, i64::from(OPAQUE)) as u8
}

pub fn particle_sprite(emitter: &Emitter, sprites: &[Sprite], state: u16) -> Option<(u16, u16)> {
    let mut rest = i64::from(state);
    for id in &emitter.sprites {
        let (_, _, frames) = sprite_shape(sprites, *id);
        if rest < frames {
            return Some((*id, u16::try_from(rest).ok()?));
        }
        rest -= frames;
    }
    None
}

pub fn looped_starts(
    bursts: &[Burst],
    loop_frames: u16,
    frame: i64,
    longest: u16,
    window: (i64, Option<i64>),
    out: &mut Vec<BurstStart>,
) {
    let period = i64::from(loop_frames.max(1));
    let longest = i64::from(longest.max(1));
    let (from, until) = window;
    for burst in bursts {
        let offset = i64::from(burst.frame);
        let first = (frame - longest - offset).div_euclid(period) + 1;
        let last = (frame - offset).div_euclid(period);
        for lap in first..=last {
            let start = lap * period + offset;
            if start >= from && until.is_none_or(|until| start < until) {
                out.push(BurstStart {
                    start,
                    count: burst.count,
                });
            }
        }
    }
}

pub fn run_starts(
    bursts: &[Burst],
    origin: i64,
    frame: i64,
    longest: u16,
    out: &mut Vec<BurstStart>,
) {
    run_starts_until(bursts, origin, frame, longest, i64::MAX, out);
}

pub fn run_starts_until(
    bursts: &[Burst],
    origin: i64,
    frame: i64,
    longest: u16,
    until: i64,
    out: &mut Vec<BurstStart>,
) {
    let longest = i64::from(longest.max(1));
    for burst in bursts.iter().filter(|b| i64::from(b.frame) < until) {
        let start = origin + i64::from(burst.frame);
        let age = frame - start;
        if (0..longest).contains(&age) {
            out.push(BurstStart {
                start,
                count: burst.count,
            });
        }
    }
}

fn edge_point(edge: Edge, rect: Rect, along: i64) -> (i32, i32) {
    let along = along as i32;
    match edge {
        Edge::Top => (rect.x0 + along, rect.y0),
        Edge::Bottom => (rect.x0 + along, rect.y1),
        Edge::Left => (rect.x0, rect.y0 + along),
        Edge::Right => (rect.x1, rect.y0 + along),
    }
}

fn border_point(rect: Rect, at: i64) -> (i32, i32) {
    let (w, h) = (i64::from(rect.width()), i64::from(rect.height()));
    let at32 = |v: i64| v as i32;
    if at < w {
        (rect.x0 + at32(at), rect.y0)
    } else if at < w + h {
        (rect.x1, rect.y0 + at32(at - w))
    } else if at < 2 * w + h {
        (rect.x1 - at32(at - w - h), rect.y1)
    } else {
        (rect.x0, rect.y1 - at32(at - 2 * w - h))
    }
}

struct Emission<'a> {
    emitter: &'a Emitter,
    sprites: &'a [Sprite],
    layer: u8,
    area: &'a SpawnArea<'a>,
    boxed: [u16; 2],
}

impl Emission<'_> {
    fn spot(&self, hash: u64) -> (i32, i32) {
        let rect = self.area.rect;
        let spawn = &self.emitter.spawn;
        match spawn.shape {
            SpawnShape::Point => self.area.point,
            SpawnShape::Line => {
                let length = match spawn.edge {
                    Edge::Top | Edge::Bottom => rect.width(),
                    Edge::Left | Edge::Right => rect.height(),
                };
                edge_point(spawn.edge, rect, pick(hash, SALT_X, i64::from(length)))
            }
            SpawnShape::Area => {
                let y = rect.y0 + pick(hash, SALT_Y, i64::from(rect.height())) as i32;
                let (left, right) = if self.area.corners == [0; 4] {
                    (0, 0)
                } else {
                    row_insets(self.area.corners, rect, y)
                };
                let room = i64::from(rect.width() - left - right);
                (rect.x0 + left + pick(hash, SALT_X, room) as i32, y)
            }
            SpawnShape::Border => match self.area.route {
                Some(route) => {
                    let (x, y) = route.at(pick(hash, SALT_X, route.steps.len() as i64));
                    let [bw, bh] = self.boxed.map(i32::from);
                    (x + bw / 2, y + bh / 2)
                }
                None => {
                    let around = 2 * (i64::from(rect.width()) + i64::from(rect.height()));
                    border_point(rect, pick(hash, SALT_X, around))
                }
            },
        }
    }

    fn particle(&self, hash: u64, age: i64) -> Option<ParticlePose> {
        let e = self.emitter;
        let [shortest, longest] = e.lifetime.map(i64::from);
        let life = shortest + pick(hash, SALT_LIFE, longest - shortest + 1);
        if age < 0 || age >= life {
            return None;
        }
        let range = |r: [i32; 2], salt| {
            i64::from(r[0]) + pick(hash, salt, i64::from(r[1]) - i64::from(r[0]) + 1)
        };
        let vx = (range(e.velocity_x, SALT_VX) + i64::from(e.drift[0])) as i32;
        let vy = (range(e.velocity_y, SALT_VY) + i64::from(e.drift[1])) as i32;
        let pool = pick(hash, SALT_SPRITE, e.sprites.len() as i64) as usize;
        let mut state = 0;
        for id in &e.sprites[..pool.min(e.sprites.len())] {
            state += sprite_shape(self.sprites, *id).2;
        }
        let (w, h, frames) = e
            .sprites
            .get(pool)
            .map_or((1, 1, 1), |id| sprite_shape(self.sprites, *id));
        let frame = match e.frames {
            ParticleFrames::Loop => age % frames,
            ParticleFrames::Life => age * frames / life,
        };
        let (sx, sy) = self.spot(hash);
        let [ox, oy] = e.spawn.offset.map(i32::from);
        let mut x = sx + ox - w / 2 + motion(vx, e.gravity[0], age);
        let mut y = sy + oy - h / 2 + motion(vy, e.gravity[1], age);
        if let Some(wobble) = e.wobble {
            let phase = age + pick(hash, SALT_WOBBLE, i64::from(wobble.period));
            x += wave(i32::from(wobble.x), phase, wobble.period);
            y += wave(i32::from(wobble.y), phase, wobble.period);
        }
        let narrow = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        Some(ParticlePose {
            x: narrow(x),
            y: narrow(y),
            layer: self.layer,
            state: u16::try_from(state + frame).unwrap_or(u16::MAX),
            opacity: fade_opacity(e.fade.as_ref(), age, life),
        })
    }
}

pub fn emit(
    emitter: &Emitter,
    layer: u8,
    area: Rect,
    seed: u64,
    frame: i64,
    bursts: &[BurstStart],
    out: &mut Vec<ParticlePose>,
) {
    let area = SpawnArea::of_rect(&emitter.spawn, area);
    emit_in(emitter, &[], layer, &area, seed, frame, bursts, out);
}

#[allow(clippy::too_many_arguments)]
pub fn emit_in(
    emitter: &Emitter,
    sprites: &[Sprite],
    layer: u8,
    area: &SpawnArea<'_>,
    seed: u64,
    frame: i64,
    bursts: &[BurstStart],
    out: &mut Vec<ParticlePose>,
) {
    let emission = Emission {
        emitter,
        sprites,
        layer,
        area,
        boxed: box_size(emitter, sprites),
    };
    let cap = PARTICLES_PER_EMITTER as usize;
    let longest = i64::from(emitter.lifetime[1].max(1));
    let mut made = 0;
    let mut push = |hash: u64, age: i64, out: &mut Vec<ParticlePose>| {
        if made >= cap {
            return false;
        }
        if let Some(pose) = emission.particle(hash, age) {
            out.push(pose);
            made += 1;
        }
        true
    };
    if let Some(rate) = emitter.rate {
        let every = i64::from(rate.every.max(1));
        let jitter = i64::from(rate.jitter);
        let first = (frame - longest - jitter).div_euclid(every) + 1;
        for k in first..=frame.div_euclid(every) {
            let spawn = mix(seed, layer_salt(layer, SALT_RATE), k as u64);
            for j in 0..rate.count {
                let hash = mix(spawn, u64::from(j), 0);
                let at = k * every + pick(hash, SALT_TIME, jitter + 1);
                if !push(hash, frame - at, out) {
                    return;
                }
            }
        }
    }
    for (index, burst) in bursts.iter().enumerate() {
        let age = frame - burst.start;
        if !(0..longest).contains(&age) {
            continue;
        }
        let twin = bursts[..index]
            .iter()
            .filter(|b| b.start == burst.start)
            .count() as u64;
        let run = mix(seed, layer_salt(layer, SALT_BURST), burst.start as u64);
        for j in 0..burst.count {
            if !push(mix(run, twin, u64::from(j)), age, out) {
                return;
            }
        }
    }
}

fn targeted(clip: &Clip, layer: u8) -> Vec<Burst> {
    clip.targets
        .iter()
        .filter(|t| t.target == Target::Layer(layer))
        .flat_map(|t| t.channels.bursts.iter().copied())
        .collect()
}

pub fn burst_sources(animations: &Animations, layer: u8) -> BurstSources {
    let mut looped = targeted(&animations.looping, layer);
    for reaction in &animations.reactions {
        looped.extend(targeted(&reaction.clip, layer));
    }
    let scheduled = animations
        .schedules
        .iter()
        .map(|schedule| ScheduledBursts {
            window: schedule.window,
            jitter: schedule.jitter,
            events: schedule
                .events
                .iter()
                .map(|event| targeted(&event.clip, layer))
                .collect(),
        })
        .filter(|s| s.events.iter().any(|e| !e.is_empty()))
        .collect();
    let one_shots = animations
        .one_shots
        .iter()
        .map(|shot| (targeted(&shot.clip, layer), 2))
        .filter(|(bursts, _)| !bursts.is_empty())
        .collect();
    BurstSources {
        looped,
        scheduled,
        one_shots,
    }
}

fn ceil_div(value: i64, by: i64) -> i64 {
    -(-value).div_euclid(by.max(1))
}

fn firing(bursts: &[Burst], longest: i64) -> i64 {
    let mut sorted = bursts.to_vec();
    sorted.sort_by_key(|b| b.frame);
    (0..sorted.len())
        .map(|i| {
            sorted[i..]
                .iter()
                .take_while(|b| i64::from(b.frame) - i64::from(sorted[i].frame) < longest)
                .map(|b| i64::from(b.count))
                .sum::<i64>()
        })
        .max()
        .unwrap_or(0)
}

pub fn live_bound(emitter: &Emitter, loop_frames: u16, sources: &BurstSources) -> u32 {
    let longest = i64::from(emitter.lifetime[1].max(1));
    let mut live = 0;
    if let Some(rate) = emitter.rate {
        live += i64::from(rate.count)
            * ceil_div(longest + i64::from(rate.jitter), i64::from(rate.every));
    }
    if !sources.looped.is_empty() {
        let period = i64::from(loop_frames.max(1));
        live += (0..period)
            .map(|t| {
                sources
                    .looped
                    .iter()
                    .map(|b| {
                        let since = t - i64::from(b.frame);
                        let laps = since.div_euclid(period) - (since - longest).div_euclid(period);
                        laps * i64::from(b.count)
                    })
                    .sum::<i64>()
            })
            .max()
            .unwrap_or(0);
    }
    for schedule in &sources.scheduled {
        let frames = schedule.events.iter().flatten().map(|b| i64::from(b.frame));
        let (Some(first), Some(last)) = (frames.clone().min(), frames.max()) else {
            continue;
        };
        let windows = ceil_div(
            longest + i64::from(schedule.jitter) + last - first,
            i64::from(schedule.window),
        );
        let most = schedule
            .events
            .iter()
            .map(|e| e.iter().map(|b| i64::from(b.count)).sum::<i64>())
            .max()
            .unwrap_or(0);
        live += windows * most;
    }
    for (bursts, copies) in &sources.one_shots {
        live += i64::from(*copies) * firing(bursts, longest);
    }
    u32::try_from(live).unwrap_or(u32::MAX)
}

pub fn worst_case_live(asset: &NameplateAsset, layer: usize) -> u32 {
    match (asset.layers.get(layer), u8::try_from(layer)) {
        (Some(found), Ok(index)) => match &found.content {
            LayerContent::Emitter(emitter) => live_bound(
                emitter,
                asset.loop_frames,
                &burst_sources(&asset.animations, index),
            ),
            _ => 0,
        },
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::geom::{Anchor, Edge};
    use crate::nameplate::types::{ParticleFrames, Rate};

    const BAR: Rect = Rect::new(0, 0, 40, 8);
    const AREA: SpawnArea<'static> = SpawnArea { rect: BAR, point: (0, 0), corners: [0; 4], route: None };

    fn dot(frames: usize) -> Sprite {
        Sprite { id: "dot".into(), width: 1, height: 1, colors: Vec::new(), frames: vec![vec![0]; frames] }
    }

    fn emitter(lifetime: [u16; 2], rate: Option<Rate>) -> Emitter {
        let spawn = Spawn { shape: SpawnShape::Area, anchor: Anchor::TopLeft, edge: Edge::Top, offset: [0, 0], follow: None };
        let (velocity_x, velocity_y, gravity, drift) = ([0, 0], [0, 0], [0, 0], [0, 0]);
        Emitter { sprites: vec![0], spawn, rate, lifetime, velocity_x, velocity_y, gravity, drift, wobble: None, frames: ParticleFrames::Loop, fade: None }
    }

    fn run(e: &Emitter, sprites: &[Sprite], seed: u64, frame: i64, bursts: &[BurstStart]) -> Vec<ParticlePose> {
        let mut out = Vec::new();
        emit_in(e, sprites, 2, &AREA, seed, frame, bursts, &mut out);
        out
    }

    #[test]
    fn particle_motion_fades_and_wobble_are_exact_integer_steps() {
        for (age, v, g) in (0..50i64).flat_map(|a| [-700, -1, 0, 129, 900].into_iter().flat_map(move |v| [-64, 0, 17, 1024].map(|g| (a, v, g)))) {
            assert_eq!(i64::from(motion(v, g, age)), (2 * i64::from(v) * age + i64::from(g) * age * age + 256).div_euclid(512));
        }
        let fade = Fade { after: 0.5, to: 0, steps: 4 };
        let levels: Vec<u8> = (0..10).map(|age| fade_opacity(Some(&fade), age, 10)).collect();
        assert_eq!(levels, [255, 255, 255, 255, 255, 192, 192, 128, 64, 0]);
        assert_eq!((-2..10).map(|p| wave(4, p, 8)).collect::<Vec<_>>(), [0, -2, -4, -2, 0, 2, 4, 2, 0, -2, -4, -2]);
    }

    #[test]
    fn emission_is_a_pure_function_of_seed_frame_and_starts() {
        let sprites = [dot(2), dot(3)];
        let mut e = emitter([5, 30], Some(Rate { every: 3, count: 2, jitter: 2 }));
        (e.sprites, e.velocity_x, e.velocity_y, e.gravity, e.drift) = (vec![0, 1], [-300, 300], [-200, 50], [0, 20], [30, 0]);
        e.fade = Some(Fade { after: 0.3, to: 10, steps: 5 });
        let bursts = [BurstStart { start: 40, count: 6 }, BurstStart { start: 52, count: 6 }];
        for shape in [SpawnShape::Point, SpawnShape::Line, SpawnShape::Border, SpawnShape::Area] {
            e.spawn.shape = shape;
            let all: Vec<_> = (0..120).map(|f| run(&e, &sprites, 0xfeed, f, &bursts)).collect();
            assert!((0..120).rev().all(|f| run(&e, &sprites, 0xfeed, f, &bursts) == all[f as usize]), "{shape:?}");
        }
        let all: Vec<_> = (0..120).map(|f| run(&e, &sprites, 0xfeed, f, &bursts)).collect();
        assert_ne!(run(&e, &sprites, 0xbeef, 60, &bursts), all[60]);
        for p in all.iter().flatten() {
            let (sprite, frame) = particle_sprite(&e, &sprites, p.state).unwrap();
            assert!(usize::from(frame) < sprites[usize::from(sprite)].frames.len() && p.layer == 2);
        }
        let plain = emitter([10, 10], None);
        let earlier = run(&plain, &[dot(1)], 0xfeed, 15, &[BurstStart { start: 10, count: 5 }]);
        let both = run(&plain, &[dot(1)], 0xfeed, 15, &[BurstStart { start: 10, count: 5 }, BurstStart { start: 10, count: 5 }]);
        assert_eq!((both.len(), &both[..5]), (10, &earlier[..]), "a restarted burst keeps its earlier run");
        assert_ne!(both[5..], earlier[..], "bursts sharing a frame spawn different particles");
        assert!(run(&plain, &[dot(1)], 0xfeed, 20, &[BurstStart { start: 10, count: 5 }]).is_empty());
    }

    #[test]
    fn the_worst_case_covers_the_most_particles_ever_alive() {
        let b = |frame, count| Burst { frame, count };
        let simulate = |e: &Emitter, looped: &[Burst], runs: &[(Vec<Burst>, i64)]| {
            let mut most = 0;
            for frame in 0..24 * 6 {
                let mut starts = Vec::new();
                looped_starts(looped, 24, frame, e.lifetime[1], (i64::MIN, None), &mut starts);
                runs.iter().for_each(|(bursts, origin)| run_starts(bursts, *origin, frame, e.lifetime[1], &mut starts));
                most = most.max(run(e, &[dot(1)], 0xfeed, frame, &starts).len());
            }
            most
        };
        let steady = emitter([20, 20], Some(Rate { every: 5, count: 2, jitter: 0 }));
        assert_eq!((live_bound(&steady, 24, &BurstSources::default()), simulate(&steady, &[], &[])), (8, 8));
        let jittered = emitter([20, 20], Some(Rate { every: 5, count: 2, jitter: 3 }));
        assert!(simulate(&jittered, &[], &[]) as u32 <= live_bound(&jittered, 24, &BurstSources::default()));
        let bursty = emitter([20, 20], None);
        let looped = vec![b(0, 5), b(10, 3), b(12, 4)];
        assert_eq!(live_bound(&bursty, 24, &BurstSources { looped: looped.clone(), ..BurstSources::default() }), 12);
        assert_eq!(simulate(&bursty, &looped, &[]), 12);
        let shot = vec![b(0, 6)];
        assert_eq!(live_bound(&bursty, 24, &BurstSources { one_shots: vec![(shot.clone(), 2)], ..BurstSources::default() }), 12);
        assert_eq!(simulate(&bursty, &[], &[(shot.clone(), 30), (shot, 33)]), 12, "a refire meets its own tail");
    }
}
