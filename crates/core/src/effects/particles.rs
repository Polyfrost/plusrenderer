use libm::{cbrtf, ceilf, cosf, expf, floorf, sinf, sqrtf};

use super::{
    Direction, EffectSet, EmitterDef, Lifetime, Motion, ParticleDef, Rate, Shape, Spawn, Tint, Uv,
};
use crate::cost::{check_emitters, Emitter};
use crate::issues::Report;
use crate::molang::{
    compile_checked, CompileError, Interner, NoQueries, Options, Program, QueryScope, Rules,
    Runtime, Warnings, FRAME_FUEL,
};

pub const VAR_NAMES: [&str; 12] = [
    "particle_age",
    "particle_lifetime",
    "particle_random_1",
    "particle_random_2",
    "particle_random_3",
    "particle_random_4",
    "emitter_age",
    "emitter_lifetime",
    "emitter_random_1",
    "emitter_random_2",
    "emitter_random_3",
    "emitter_random_4",
];
pub const VAR_COUNT: usize = VAR_NAMES.len();
pub const PARTICLE_AGE: usize = 0;
pub const PARTICLE_LIFETIME: usize = 1;
pub const PARTICLE_RANDOM: usize = 2;
pub const EMITTER_AGE: usize = 6;
pub const EMITTER_LIFETIME: usize = 7;
pub const EMITTER_RANDOM: usize = 8;

const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
const LANE_SCALE: f32 = 16_777_216.0;
const EMITTER_INDEX: u32 = u32::MAX;
const SMALL_DRAG: f32 = 1e-3;
const ENTITY_HALF: [f32; 3] = [0.3, 0.9, 0.3];
const ENTITY_CENTER: [f32; 3] = [0.0, 0.9, 0.0];
const TAU: f32 = core::f32::consts::TAU;
pub const KEY_TICKS_PER_SECOND: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Activation,
    Loop,
    Spawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seeds {
    pub emitter: u64,
    pub particle: u64,
}

impl Seeds {
    pub fn both(seed: u64) -> Seeds {
        Seeds {
            emitter: seed,
            particle: seed,
        }
    }
}
const LIVE_SCAN_LIMIT: u32 = 4096;
const LOOP_SLACK: f64 = 1e-6;
const OFFSET_GRID: f64 = 1000.0;
const TIME_SLACK: f64 = 5e-4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    left: u32,
    starved: bool,
}

impl Budget {
    pub fn new(left: u32) -> Budget {
        Budget {
            left,
            starved: false,
        }
    }

    pub fn left(&self) -> u32 {
        self.left
    }

    pub fn starved(&self) -> bool {
        self.starved
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vars(pub [f32; VAR_COUNT]);

impl Vars {
    pub fn particle(spawn: &Spawn, age: f32) -> Vars {
        let mut v = [0.0; VAR_COUNT];
        v[PARTICLE_AGE] = age;
        v[PARTICLE_LIFETIME] = spawn.life;
        v[PARTICLE_RANDOM..PARTICLE_RANDOM + 4].copy_from_slice(&spawn.randoms);
        v[EMITTER_AGE] = wrap(spawn.emitter_age + age, spawn.emitter_period);
        v[EMITTER_LIFETIME] = spawn.emitter_lifetime;
        v[EMITTER_RANDOM..EMITTER_RANDOM + 4].copy_from_slice(&spawn.emitter);
        Vars(v)
    }

    pub fn emitter(randoms: [f32; 4], age: f32, lifetime: f32) -> Vars {
        let mut v = [0.0; VAR_COUNT];
        v[EMITTER_AGE] = age;
        v[EMITTER_LIFETIME] = lifetime;
        v[EMITTER_RANDOM..EMITTER_RANDOM + 4].copy_from_slice(&randoms);
        Vars(v)
    }

    pub fn eval(&self, p: &Program, seed: u64) -> f32 {
        self.eval_in(p, seed, &mut Budget::new(FRAME_FUEL))
    }

    pub fn eval_in(&self, p: &Program, seed: u64, budget: &mut Budget) -> f32 {
        let mut vars = self.0;
        let mut runtime = Runtime::new(&Interner::default());
        let mut scope = runtime.wearer_frame(&mut vars, seed, 0.0);
        scope.set_fuel(budget.left);
        let outcome = p.evaluate(&mut scope, &mut NoQueries);
        budget.left = scope.fuel();
        if outcome.failed && budget.left == 0 {
            budget.starved = true;
        }
        let value = outcome.value as f32;
        if outcome.failed || !value.is_finite() {
            0.0
        } else {
            value
        }
    }
}

pub(super) fn interner() -> Interner {
    let mut interner = Interner::default();
    let declare: String = VAR_NAMES
        .iter()
        .map(|name| format!("variable.{name};"))
        .collect();
    let _ = compile(&declare, &mut interner);
    interner
}

pub(super) fn compile(source: &str, interner: &mut Interner) -> Result<Program, CompileError> {
    let options = Options {
        rules: Rules::Strict,
        queries: QueryScope::Only(&[]),
    };
    compile_checked(source, interner, &options, &mut Warnings::default())
}

fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn spawn_hash(seed: u64, emitter: u16, index: u32) -> u64 {
    let site = ((emitter as u64) << 32) | index as u64;
    let h = mix64(seed.wrapping_add(GAMMA));
    mix64(h ^ site.wrapping_add(GAMMA.wrapping_mul(2)))
}

pub fn lane(hash: u64, lane: u32) -> f32 {
    let bits = mix64(hash ^ (lane as u64 + 1).wrapping_mul(GAMMA));
    (bits >> 40) as f32 / LANE_SCALE
}

pub fn keyed(seed: u64, domain: Key, key: i64) -> u64 {
    let h = mix64(seed ^ (domain as u64 + 1).wrapping_mul(GAMMA));
    mix64(h ^ (key as u64).wrapping_add(GAMMA.wrapping_mul(3)))
}

pub fn start_tick(epoch: f64, start: f32) -> i64 {
    libm::round((epoch + f64::from(start)) * KEY_TICKS_PER_SECOND) as i64
}

pub fn activation_seed(seed: u64, epoch: f64, start: f32) -> u64 {
    keyed(seed, Key::Activation, start_tick(epoch, start))
}

pub fn fold_loop(epoch: f64, start: f32, now: f32, back: f32, period: f32) -> f32 {
    let (number, offset) = loop_origin(epoch, start, period);
    let step = f64::from(period);
    let latest = epoch + f64::from(now) - f64::from(back.max(0.0)) - offset;
    let target = libm::floor(latest / step);
    if target - number < 1.0 {
        return start;
    }
    (target * step + offset - epoch) as f32
}

pub fn loop_origin(epoch: f64, start: f32, period: f32) -> (f64, f64) {
    let from = epoch + f64::from(start);
    let period = f64::from(period);
    let number = libm::round(from / period);
    let offset = libm::round((from - number * period) * OFFSET_GRID) / OFFSET_GRID;
    (number, offset)
}

pub fn loop_start(epoch: f64, now: f32, back: f32, period: f32) -> f32 {
    let period = f64::from(period);
    let at = libm::floor((epoch + f64::from(now) - f64::from(back.max(0.0))) / period) * period;
    (at - epoch) as f32
}

pub fn emitter_lifetime(lifetime: &Lifetime) -> f32 {
    match *lifetime {
        Lifetime::Once { seconds } => seconds,
        Lifetime::Looping { active, .. } => active,
        Lifetime::Expression { .. } => 0.0,
    }
}

fn wrap(since: f32, period: f32) -> f32 {
    if period > 0.0 {
        since - floorf(since / period) * period
    } else {
        since
    }
}

pub fn emitter_randoms(seed: u64, emitter: u16) -> [f32; 4] {
    let hash = spawn_hash(seed, emitter, EMITTER_INDEX);
    [0, 1, 2, 3].map(|i| lane(hash, i))
}

pub fn spawn(
    e: &EffectSet,
    emitter: u16,
    seeds: Seeds,
    index: u32,
    time: f32,
    since: f32,
    budget: &mut Budget,
) -> Option<Spawn> {
    let def = e.emitters.get(emitter as usize)?;
    let particle = e.particles.get(def.particle? as usize)?;
    let hash = spawn_hash(seeds.particle, emitter, index);
    let randoms = [0, 1, 2, 3].map(|i| lane(hash, i));
    let mut spawn = Spawn {
        time,
        position: [0.0; 3],
        velocity: [0.0; 3],
        acceleration: [0.0; 3],
        drag: 0.0,
        randoms,
        emitter: emitter_randoms(seeds.emitter, emitter),
        emitter_age: since,
        emitter_period: period(&def.lifetime).map_or(0.0, |(_, period)| period),
        emitter_lifetime: emitter_lifetime(&def.lifetime),
        life: 0.0,
        seed: hash,
    };
    let vars = Vars::particle(&spawn, 0.0);
    spawn.life = vars
        .eval_in(&particle.lifetime, hash, budget)
        .clamp(0.0, particle.longest_life);
    let vars = Vars::particle(&spawn, 0.0);
    let shape = [4, 5, 6, 7].map(|i| lane(hash, i));
    let local = sample_shape(def.shape, shape);
    spawn.position = add(def.offset, local);
    if let Motion::Dynamic {
        speed,
        direction,
        acceleration,
        drag,
    } = &particle.motion
    {
        let mut eval = |p: &Program| vars.eval_in(p, hash, budget);
        let outward = || {
            let toward = match def.shape {
                Shape::EntityAabb { .. } => sub(local, ENTITY_CENTER),
                _ => local,
            };
            if length(toward) > 0.0 {
                normalize(toward)
            } else {
                unit_sphere(lane(hash, 8), lane(hash, 9))
            }
        };
        spawn.velocity = match direction {
            Direction::Outwards => scale(outward(), eval(speed)),
            Direction::Inwards => scale(outward(), -eval(speed)),
            Direction::Vector(v) => {
                let toward = normalize(v.each_ref().map(&mut eval));
                scale(toward, eval(speed))
            }
            Direction::Velocity(v) => v.each_ref().map(&mut eval),
        };
        spawn.acceleration = acceleration.each_ref().map(&mut eval);
        spawn.drag = eval(drag).max(0.0);
    }
    Some(spawn)
}

pub fn particle_at(d: &ParticleDef, spawn: &Spawn, age: f32, budget: &mut Budget) -> [f32; 3] {
    match &d.motion {
        Motion::Parametric { offset } => {
            let vars = Vars::particle(spawn, age);
            add(
                spawn.position,
                offset
                    .each_ref()
                    .map(|p| vars.eval_in(p, spawn.seed, budget)),
            )
        }
        Motion::Dynamic { .. } => {
            let (g, h) = drift(spawn.drag, age);
            let mut out = spawn.position;
            for (i, value) in out.iter_mut().enumerate() {
                *value += spawn.velocity[i] * g + spawn.acceleration[i] * h;
            }
            out
        }
    }
}

pub fn particle_parts(
    d: &ParticleDef,
    spawn: &Spawn,
    age: f32,
    budget: &mut Budget,
) -> ([f32; 3], [f32; 3]) {
    match &d.motion {
        Motion::Parametric { .. } => (particle_at(d, spawn, age, budget), [0.0; 3]),
        Motion::Dynamic { .. } => {
            let (g, h) = drift(spawn.drag, age);
            let local = [0, 1, 2].map(|i| spawn.position[i] + spawn.velocity[i] * g);
            (local, spawn.acceleration.map(|a| a * h))
        }
    }
}

pub(super) fn drift(k: f32, t: f32) -> (f32, f32) {
    let t = t.max(0.0);
    if k * t < SMALL_DRAG {
        let kt = k * t;
        let g = t * (1.0 - kt / 2.0 + kt * kt / 6.0);
        let h = t * t * (0.5 - kt / 6.0 + kt * kt / 24.0);
        return (g, h);
    }
    let g = (1.0 - expf(-k * t)) / k;
    (g, (t - g) / k)
}

pub fn size_at(d: &ParticleDef, spawn: &Spawn, age: f32, budget: &mut Budget) -> [f32; 2] {
    let vars = Vars::particle(spawn, age);
    d.size
        .each_ref()
        .map(|p| vars.eval_in(p, spawn.seed, budget))
}

pub fn uv_at(d: &ParticleDef, spawn: &Spawn, age: f32) -> [f32; 4] {
    match d.uv {
        Uv::Fixed { uv, size } => [uv[0], uv[1], uv[0] + size[0], uv[1] + size[1]],
        Uv::Flipbook {
            base,
            size,
            step,
            frames,
            fps,
            stretch,
            looping,
        } => {
            let frames = frames.max(1);
            let raw = if stretch {
                if spawn.life > 0.0 {
                    age / spawn.life * frames as f32
                } else {
                    0.0
                }
            } else {
                age * fps
            };
            let raw = floorf(raw.max(0.0));
            let frame = if raw >= u32::MAX as f32 {
                u32::MAX
            } else {
                raw as u32
            };
            let frame = if looping {
                frame % frames
            } else {
                frame.min(frames - 1)
            } as f32;
            let u = base[0] + step[0] * frame;
            let v = base[1] + step[1] * frame;
            [u, v, u + size[0], v + size[1]]
        }
    }
}

pub fn tint_at(
    d: &ParticleDef,
    spawn: &Spawn,
    age: f32,
    slots: &[u32],
    budget: &mut Budget,
) -> u32 {
    match &d.tint {
        Tint::Fixed(color) => *color,
        Tint::Slot { slot, .. } => slots.get(*slot as usize).copied().unwrap_or(u32::MAX),
        Tint::Gradient { interpolant, keys } => {
            let x = Vars::particle(spawn, age).eval_in(interpolant, spawn.seed, budget);
            gradient(keys, x)
        }
        Tint::Expression(rgba) => {
            let vars = Vars::particle(spawn, age);
            let [r, g, b, a] = rgba.each_ref().map(|p| vars.eval_in(p, spawn.seed, budget));
            pack([r, g, b, a])
        }
    }
}

pub fn gradient(keys: &[(f32, u32)], x: f32) -> u32 {
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
        return u32::MAX;
    };
    if keys.len() < 2 || x.is_nan() || x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    let at = keys.partition_point(|k| k.0 <= x).clamp(1, keys.len() - 1);
    let (a, b) = (keys[at - 1], keys[at]);
    let span = b.0 - a.0;
    let t = if span > 0.0 { (x - a.0) / span } else { 0.0 };
    let (ca, cb) = (unpack(a.1), unpack(b.1));
    pack([0, 1, 2, 3].map(|i| ca[i] + (cb[i] - ca[i]) * t))
}

pub fn pack(rgba: [f32; 4]) -> u32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let [r, g, b, a] = rgba.map(byte);
    (a << 24) | (r << 16) | (g << 8) | b
}

pub fn unpack(argb: u32) -> [f32; 4] {
    let channel = |shift: u32| ((argb >> shift) & 0xff) as f32 / 255.0;
    [channel(16), channel(8), channel(0), channel(24)]
}

pub fn spawning(def: &EmitterDef, start: f32, stop: f32, now: f32) -> bool {
    if !start.is_finite() || now < start || now >= stop_time(stop) {
        return false;
    }
    match def.lifetime {
        Lifetime::Once { seconds } => now - start < seconds,
        Lifetime::Looping { .. } | Lifetime::Expression { .. } => true,
    }
}

fn stop_time(stop: f32) -> f32 {
    if stop.is_finite() {
        stop
    } else {
        f32::INFINITY
    }
}

pub fn period(lifetime: &Lifetime) -> Option<(f32, f32)> {
    match *lifetime {
        Lifetime::Looping { active, sleep } if active + sleep > 0.0 => {
            Some((active, active + sleep))
        }
        _ => None,
    }
}

fn in_window(lifetime: &Lifetime, since: f32) -> bool {
    match *lifetime {
        Lifetime::Once { seconds } => since < seconds,
        Lifetime::Looping { active, sleep } => {
            let period = active + sleep;
            period <= 0.0 || since - floorf(since / period) * period < active
        }
        Lifetime::Expression { .. } => true,
    }
}

pub fn effective_rate(per_second: f32, max: u32, longest: f32) -> f32 {
    if per_second.is_nan() || per_second <= 0.0 || per_second.is_infinite() {
        return 0.0;
    }
    if longest > 0.0 {
        per_second.min(max as f32 / longest)
    } else {
        per_second
    }
}

#[allow(clippy::too_many_arguments)]
pub fn for_each_live(
    e: &EffectSet,
    emitter: u16,
    start: f32,
    stop: f32,
    now: f32,
    epoch: f64,
    seed_epoch: f64,
    seed: u64,
    budget: &mut Budget,
    mut f: impl FnMut(u32, &Spawn, f32, &mut Budget),
) {
    let Some(def) = e.emitters.get(emitter as usize) else {
        return;
    };
    let Some(particle) = def.particle.and_then(|p| e.particles.get(p as usize)) else {
        return;
    };
    if !start.is_finite() || now < start {
        return;
    }
    let (from, at) = (f64::from(start), f64::from(now));
    let end = f64::from(stop_time(stop));
    if end < from {
        return;
    }
    let longest = f64::from(particle.longest_life);
    let opened = activation_seed(seed, seed_epoch, start);
    let mut visit = |index: u32, seeds: Seeds, time: f64, since: f64| {
        if budget.starved {
            return;
        }
        let spawned = spawn(e, emitter, seeds, index, time as f32, since as f32, budget);
        if let Some(spawn) = spawned {
            let age = (at - time).max(0.0);
            if age + TIME_SLACK < f64::from(spawn.life) && !budget.starved {
                f(index, &spawn, age as f32, budget);
            }
        }
    };
    let repeats = period(&def.lifetime);
    let rate = match def.rate {
        Rate::Steady { per_second, max } => {
            f64::from(effective_rate(per_second, max, particle.longest_life))
        }
        _ => 0.0,
    };
    match (def.rate, repeats) {
        (Rate::Manual { .. }, _) => visit(0, Seeds::both(opened), from, 0.0),
        (Rate::Instant { count }, None) => {
            for j in 0..count {
                visit(j, Seeds::both(opened), from, 0.0);
            }
        }
        (Rate::Instant { count }, Some((_, period))) => {
            let (first, offset) = loop_origin(epoch, start, period);
            let step = f64::from(period);
            let lap = |time: f64| (epoch + time - offset) / step;
            let low = libm::ceil(lap(at - longest) - LOOP_SLACK).max(first);
            let high = libm::floor(lap((at + TIME_SLACK).min(end)) + LOOP_SLACK);
            let high = high.min(low + f64::from(LIVE_SCAN_LIMIT));
            let mut number = low;
            while number <= high {
                let time = number * step + offset - epoch;
                let seeds = Seeds::both(keyed(seed, Key::Loop, number as i64));
                number += 1.0;
                if time > at + TIME_SLACK || (time >= end && number - 1.0 > first) {
                    continue;
                }
                for j in 0..count {
                    visit(j, seeds, time, 0.0);
                }
            }
        }
        (Rate::Steady { .. }, _) if rate <= 0.0 => {}
        (Rate::Steady { .. }, repeats) => {
            let origin = repeats.map(|(active, period)| {
                let (first, offset) = loop_origin(epoch, start, period);
                (first, offset, f64::from(active), f64::from(period))
            });
            let earliest = (at - longest).max(from);
            let low = libm::ceil((epoch + earliest) * rate);
            let high = libm::floor((epoch + (at + TIME_SLACK).min(end)) * rate);
            let high = high.min(low + f64::from(LIVE_SCAN_LIMIT));
            let mut number = low;
            while number <= high {
                let absolute = number / rate;
                let key = number as i64;
                number += 1.0;
                let time = absolute - epoch;
                if time < from - TIME_SLACK || time > at + TIME_SLACK || time >= end {
                    continue;
                }
                let (since, emitter_seed) = match origin {
                    Some((first, offset, active, period)) => {
                        let phase = (absolute - offset) / period;
                        let lap = libm::floor(phase + LOOP_SLACK);
                        let within = ((phase - lap) * period).max(0.0);
                        if lap < first || within >= active {
                            continue;
                        }
                        (within, keyed(seed, Key::Loop, lap as i64))
                    }
                    None => {
                        let since = (time - from).max(0.0);
                        if !in_window(&def.lifetime, since as f32) {
                            continue;
                        }
                        (since, opened)
                    }
                };
                let seeds = Seeds {
                    emitter: emitter_seed,
                    particle: keyed(seed, Key::Spawn, key),
                };
                visit(key as u32, seeds, time, since);
            }
        }
    }
}

pub fn worst_case(e: &EffectSet) -> Vec<Emitter> {
    let mut out = Vec::new();
    for (index, def) in e.emitters.iter().enumerate() {
        let Some(particle) = def.particle.and_then(|p| e.particles.get(p as usize)) else {
            continue;
        };
        let longest = particle.longest_life.max(0.0);
        let overlapping = match period(&def.lifetime) {
            Some((_, period)) => (ceilf(longest / period) as u32).max(1),
            None => 1,
        };
        let once = e.ambient.contains(&(index as u16))
            && !matches!(def.lifetime, Lifetime::Expression { .. });
        let copies = if once {
            1
        } else {
            u32::try_from(e.budget_runs(index)).unwrap_or(u32::MAX)
        };
        let (rate, bursts) = match def.rate {
            Rate::Instant { count } => (
                0.0,
                vec![count.saturating_mul(overlapping).saturating_mul(copies)],
            ),
            Rate::Manual { .. } => (0.0, vec![copies]),
            Rate::Steady { per_second, max } => (
                effective_rate(per_second, max, longest) as f64 * copies as f64,
                Vec::new(),
            ),
        };
        out.push(Emitter {
            path: def.path.clone(),
            rate,
            longest_life: longest as f64,
            bursts,
        });
    }
    out
}

pub fn check(e: &EffectSet, path: &str, r: &mut Report) {
    check_emitters(path, &worst_case(e), r);
}

fn sample_shape(shape: Shape, u: [f32; 4]) -> [f32; 3] {
    match shape {
        Shape::Point => [0.0; 3],
        Shape::Sphere { radius, surface } => {
            let dir = unit_sphere(u[0], u[1]);
            let r = if surface {
                radius
            } else {
                radius * cbrtf(u[2])
            };
            scale(dir, r)
        }
        Shape::Box { half, surface } => sample_box(half, surface, u),
        Shape::Disc {
            radius,
            normal,
            surface,
        } => {
            let n = normalize(normal);
            let helper = if n[1].abs() < 0.9 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let t1 = normalize(cross(helper, n));
            let t2 = cross(n, t1);
            let angle = TAU * u[0];
            let r = if surface {
                radius
            } else {
                radius * sqrtf(u[1])
            };
            add(scale(t1, cosf(angle) * r), scale(t2, sinf(angle) * r))
        }
        Shape::EntityAabb { surface } => add(ENTITY_CENTER, sample_box(ENTITY_HALF, surface, u)),
    }
}

fn sample_box(half: [f32; 3], surface: bool, u: [f32; 4]) -> [f32; 3] {
    let mut p = [0, 1, 2].map(|i| (2.0 * u[i] - 1.0) * half[i]);
    if surface {
        let axis = ((u[3] * 3.0) as usize).min(2);
        p[axis] = if u[axis] < 0.5 {
            -half[axis]
        } else {
            half[axis]
        };
    }
    p
}

fn unit_sphere(a: f32, b: f32) -> [f32; 3] {
    let y = 2.0 * a - 1.0;
    let ring = sqrtf((1.0 - y * y).max(0.0));
    let phi = TAU * b;
    [ring * cosf(phi), y, ring * sinf(phi)]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: [f32; 3]) -> f32 {
    sqrtf(a[0] * a[0] + a[1] * a[1] + a[2] * a[2])
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = length(a);
    if len > 0.0 {
        scale(a, 1.0 / len)
    } else {
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dynamic() -> ParticleDef {
        ParticleDef {
            id: "spark".to_string(),
            texture: "textures/spark.png".to_string(),
            blend: Default::default(),
            facing: Default::default(),
            size: [Program::number(0.1), Program::number(0.1)],
            uv: Uv::Fixed {
                uv: [0.0, 0.0],
                size: [1.0, 1.0],
            },
            tint: Tint::Fixed(u32::MAX),
            lifetime: Program::number(1.5),
            longest_life: 1.5,
            motion: Motion::Dynamic {
                speed: Program::number(2.0),
                direction: Direction::Outwards,
                acceleration: Box::new([0, 1, 2].map(|_| Program::number(0.0))),
                drag: Program::number(0.0),
            },
            emissive: true,
        }
    }

    fn at_rest(position: [f32; 3], velocity: [f32; 3], acceleration: [f32; 3], drag: f32) -> Spawn {
        Spawn {
            time: 0.0,
            position,
            velocity,
            acceleration,
            drag,
            randoms: [0.25; 4],
            emitter: [0.5; 4],
            emitter_age: 0.0,
            emitter_period: 0.0,
            emitter_lifetime: 0.0,
            life: 3.0,
            seed: 7,
        }
    }

    fn integrate(spawn: &Spawn, age: f32) -> [f32; 3] {
        let steps = (age / 1e-4).round() as usize;
        let mut p = spawn.position.map(f64::from);
        if steps == 0 {
            return p.map(|x| x as f32);
        }
        let dt = age as f64 / steps as f64;
        let mut v = spawn.velocity.map(f64::from);
        let a = spawn.acceleration.map(f64::from);
        let k = spawn.drag as f64;
        for _ in 0..steps {
            for i in 0..3 {
                let half = v[i] + 0.5 * dt * (a[i] - k * v[i]);
                p[i] += half * dt;
                v[i] += dt * (a[i] - k * half);
            }
        }
        p.map(|x| x as f32)
    }

    #[test]
    fn effects_closed_form_matches_a_stepped_reference() {
        let def = dynamic();
        for drag in [0.0, 0.0004, 0.5, 2.5] {
            let spawn = at_rest([1.0, 2.0, -1.0], [3.0, 4.0, -2.0], [0.0, -9.8, 0.5], drag);
            for age in [0.0, 0.05, 0.4, 1.0, 2.5] {
                let closed = particle_at(&def, &spawn, age, &mut Budget::new(FRAME_FUEL));
                let stepped = integrate(&spawn, age);
                for i in 0..3 {
                    assert!(
                        (closed[i] - stepped[i]).abs() < 2e-3,
                        "drag {drag} age {age}: {closed:?} vs {stepped:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn effects_spawn_hashes_are_pinned_and_integer_only() {
        assert_eq!(spawn_hash(0, 0, 0), 0xc073_7b7c_f89e_44ab);
        assert_eq!(
            spawn_hash(0x1234_5678_9abc_def0, 3, 17),
            0xce3f_ba12_16b6_fc4f
        );
        let hash = spawn_hash(42, 1, 9);
        assert_eq!(hash, 0x80b0_a521_27e5_4d6c);
        let lanes: Vec<u32> = (0..8)
            .map(|i| (lane(hash, i) * LANE_SCALE) as u32)
            .collect();
        assert_eq!(
            lanes,
            [1315213, 15745289, 7632109, 6628148, 6991565, 4435148, 9965430, 7013882]
        );
        for seed in [0u64, 1, u64::MAX, 0xdead_beef] {
            for index in [0u32, 1, 63, u32::MAX - 1] {
                let h = spawn_hash(seed, 5, index);
                for l in 0..10 {
                    let u = lane(h, l);
                    assert!((0.0..1.0).contains(&u));
                    let scaled = u * LANE_SCALE;
                    assert_eq!(
                        scaled,
                        scaled.trunc(),
                        "lanes are 24-bit integers over 2^24"
                    );
                }
            }
        }
        assert_ne!(spawn_hash(42, 1, 9), spawn_hash(42, 2, 9));
        assert_ne!(spawn_hash(42, 1, 9), spawn_hash(43, 1, 9));
        assert_eq!(emitter_randoms(42, 1), emitter_randoms(42, 1));
    }
}
