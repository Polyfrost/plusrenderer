//! Bedrock `.animation.json`: keyframes, interpolation and blending into a `Pose`.
//! `relative_to` is ignored, not rejected. `anim_time_update` applies only when a controller plays the clip.

use std::fmt;

use serde_json::{Map, Value};

use crate::ir::Model;
use crate::issues::join;
use crate::molang::{
    self, compile_with, random_frame, Interner, NoQueries, Options, Outcome, Program, QueryScope,
    Reads, Rules, Runtime, Scope, Warnings,
};
use crate::pose::Pose;

#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    Molang(molang::Error),
    NoAnimations,
    BadValue(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(e) => write!(f, "{e}"),
            Error::Molang(e) => write!(f, "{e}"),
            Error::NoAnimations => f.write_str("no `animations` object"),
            Error::BadValue(what) => write!(f, "cannot read {what} as an animation value"),
        }
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Error {
        Error::Json(e)
    }
}

impl From<molang::Error> for Error {
    fn from(e: molang::Error) -> Error {
        Error::Molang(e)
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loop {
    /// Holds the last frame, as the client does for cosmetics. Unlike `Hold`, a controller restarts it when its state is entered again.
    Once,
    Repeat,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lerp {
    Linear,
    Step,
    CatmullRom,
    Bezier,
}

#[derive(Debug, Clone)]
struct Handles {
    left_time: [f32; 3],
    left_value: [f32; 3],
    right_time: [f32; 3],
    right_value: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Scalar,
    Axes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Plain(Option<Shape>),
    Split {
        pre: Option<Shape>,
        post: Option<Shape>,
    },
}

#[derive(Debug, Clone)]
struct Keyframe {
    time: f32,
    key: Option<Box<str>>,
    source: Source,
    pre: [Program; 3],
    post: [Program; 3],
    lerp: Lerp,
    handles: Option<Box<Handles>>,
}

/// Sorted by time. Empty means the channel is absent.
#[derive(Debug, Clone, Default)]
struct Channel(Vec<Keyframe>);

#[derive(Debug, Clone)]
pub struct BoneChannels {
    pub name: String,
    rotation: Channel,
    position: Channel,
    scale: Channel,
}

#[derive(Debug, Clone)]
struct Script {
    at: Option<usize>,
    program: Program,
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub time: f32,
    pub kind: KeyEventKind,
    at: Box<str>,
}

#[derive(Debug, Clone)]
pub enum KeyEventKind {
    Particle {
        effect: String,
        locator: Option<String>,
        pre_effect_script: Option<String>,
    },
    Sound {
        effect: String,
    },
    Timeline(Program),
}

#[derive(Debug, Clone)]
pub struct Animation {
    pub name: String,
    pub length: f32,
    pub loop_mode: Loop,
    start_delay: Program,
    loop_delay: Program,
    blend_weight: Program,
    anim_time_update: Option<Program>,
    initialize: Vec<Script>,
    pre_animation: Vec<Script>,
    pub bones: Vec<BoneChannels>,
    pub events: Vec<KeyEvent>,
}

pub struct Disabled<'a> {
    words: &'a mut [u32],
    first: usize,
}

impl<'a> Disabled<'a> {
    pub fn new(words: &'a mut [u32], first: usize) -> Disabled<'a> {
        Disabled { words, first }
    }

    pub fn none() -> Disabled<'static> {
        Disabled {
            words: &mut [],
            first: 0,
        }
    }

    pub fn get(&self, channel: usize) -> bool {
        let bit = self.first + channel;
        self.words
            .get(bit / 32)
            .is_some_and(|word| word & (1 << (bit % 32)) != 0)
    }

    pub fn set(&mut self, channel: usize) {
        let bit = self.first + channel;
        if let Some(word) = self.words.get_mut(bit / 32) {
            *word |= 1 << (bit % 32);
        }
    }
}

pub fn flag_words(channels: usize) -> usize {
    channels.div_ceil(32)
}

pub fn varies(reads: &Reads) -> bool {
    reads.random || !reads.queries.is_empty() || !reads.variables.is_empty()
}

pub fn has_timeline(animations: &[Animation]) -> bool {
    animations.iter().any(|a| {
        a.events
            .iter()
            .any(|e| matches!(e.kind, KeyEventKind::Timeline(_)))
    })
}

pub fn channel_offsets(animations: &[Animation]) -> (Vec<usize>, usize) {
    let mut total = 0;
    let offsets = animations
        .iter()
        .map(|animation| {
            let first = total;
            total += animation.channel_count();
            first
        })
        .collect();
    (offsets, total)
}

fn walk_scripts<'a>(out: &mut Vec<(String, &'a Program)>, key: &str, scripts: &'a [Script]) {
    for script in scripts {
        let at = match script.at {
            Some(index) => join(key, &index.to_string()),
            None => key.to_string(),
        };
        out.push((at, &script.program));
    }
}

fn walk_side<'a>(
    out: &mut Vec<(String, &'a Program)>,
    at: String,
    shape: Option<Shape>,
    programs: &'a [Program; 3],
) {
    match shape {
        Some(Shape::Scalar) => out.push((at, &programs[0])),
        Some(Shape::Axes) => {
            for (axis, program) in programs.iter().enumerate() {
                out.push((join(&at, &axis.to_string()), program));
            }
        }
        None => {}
    }
}

impl Animation {
    pub fn programs(&self) -> Vec<(String, &Program)> {
        let mut out = vec![
            ("start_delay".to_string(), &self.start_delay),
            ("loop_delay".to_string(), &self.loop_delay),
            ("blend_weight".to_string(), &self.blend_weight),
        ];
        if let Some(update) = &self.anim_time_update {
            out.push(("anim_time_update".to_string(), update));
        }
        walk_scripts(&mut out, "initialize", &self.initialize);
        walk_scripts(&mut out, "pre_animation", &self.pre_animation);
        for event in &self.events {
            if let KeyEventKind::Timeline(program) = &event.kind {
                out.push((join("timeline", &event.at), program));
            }
        }
        for bone in &self.bones {
            let base = join("bones", &bone.name);
            let channels = [
                ("rotation", &bone.rotation),
                ("position", &bone.position),
                ("scale", &bone.scale),
            ];
            for (name, channel) in channels {
                let at = join(&base, name);
                for frame in &channel.0 {
                    let at = match &frame.key {
                        Some(key) => join(&at, key),
                        None => at.clone(),
                    };
                    match frame.source {
                        Source::Plain(shape) => walk_side(&mut out, at, shape, &frame.pre),
                        Source::Split { pre, post } => {
                            walk_side(&mut out, join(&at, "pre"), pre, &frame.pre);
                            walk_side(&mut out, join(&at, "post"), post, &frame.post);
                        }
                    }
                }
            }
        }
        out.retain(|(_, program)| program.nodes() > 0);
        out
    }

    pub fn channel_count(&self) -> usize {
        self.bones.len() * CHANNELS_PER_BONE
    }

    pub fn keyed_channels<'a>(
        &'a self,
        bind: &'a [Option<usize>],
    ) -> impl Iterator<Item = usize> + 'a {
        self.bones
            .iter()
            .zip(bind)
            .filter_map(|(channels, bone)| Some((channels, (*bone)?)))
            .flat_map(|(channels, bone)| {
                [&channels.rotation, &channels.position, &channels.scale]
                    .into_iter()
                    .enumerate()
                    .filter(|(_, channel)| !channel.0.is_empty())
                    .map(move |(kind, _)| bone * CHANNELS_PER_BONE + kind)
            })
    }

    pub fn initialize(&self, scope: &mut Scope<'_>) -> bool {
        let frame = scope.random_frame();
        scope.set_random_frame(0);
        let failed = run_scripts(&self.initialize, scope);
        scope.set_random_frame(frame);
        failed
    }

    pub fn pre_animation(&self, scope: &mut Scope<'_>) -> bool {
        run_scripts(&self.pre_animation, scope)
    }

    pub fn constant_start_delay(&self) -> Option<f32> {
        let value = self.start_delay.literal()? as f32;
        value.is_finite().then_some(value.max(0.0))
    }

    pub fn longest_start_delay(&self) -> Option<f32> {
        let (_, high) = self.start_delay.bounds(&|_| None)?;
        let high = high as f32;
        high.is_finite().then_some(high.max(0.0))
    }

    pub fn shortest_loop_delay(&self) -> f32 {
        self.loop_delay
            .bounds(&|_| None)
            .map_or(0.0, |(low, _)| low as f32)
            .max(0.0)
    }

    pub fn steady_delays(&self) -> Option<[f32; 2]> {
        let fixed = |program: &Program| match program.bounds(&|_| None)? {
            (low, high) if low == high && low.is_finite() => Some((low as f32).max(0.0)),
            _ => None,
        };
        Some([fixed(&self.start_delay)?, fixed(&self.loop_delay)?])
    }

    pub fn steady_weight(&self) -> bool {
        matches!(
            self.blend_weight.bounds(&|_| None),
            Some((low, high)) if low == high && low > 0.0
        )
    }

    pub fn longest_loop_delay(&self) -> Option<f32> {
        let (_, high) = self.loop_delay.bounds(&|_| None)?;
        let high = high as f32;
        high.is_finite().then_some(high.max(0.0))
    }

    pub fn delays_in(&self, scope: &mut Scope<'_>) -> [f32; 2] {
        [
            self.start_delay.evaluate(scope, &mut NoQueries).value as f32,
            self.loop_delay.evaluate(scope, &mut NoQueries).value as f32,
        ]
    }

    pub fn delays_from(&self, scope: &mut Scope<'_>, started: f64) -> [f32; 2] {
        let frame = scope.random_frame();
        scope.set_random_frame(random_frame(started));
        let delays = self.delays_in(scope);
        scope.set_random_frame(frame);
        delays
    }

    pub fn delays_kept(
        &self,
        clip: usize,
        scope: &mut Scope<'_>,
        started: f64,
        kept: &mut [f32],
    ) -> [f32; 2] {
        let marker = (clip + 1) as f32;
        let mut free = None;
        let mut oldest = f64::INFINITY;
        for (i, entry) in kept.chunks_exact(KEPT_DELAY_FLOATS).enumerate() {
            if entry[2] == 0.0 {
                if oldest > f64::NEG_INFINITY {
                    free = Some(i);
                    oldest = f64::NEG_INFINITY;
                }
                continue;
            }
            let at = f64::from(entry[0]) + f64::from(entry[1]);
            if entry[2] == marker && (at - started).abs() < KEPT_DELAY_MATCH {
                return [entry[3], entry[4]];
            }
            if at < oldest {
                oldest = at;
                free = Some(i);
            }
        }
        let delays = self.delays_from(scope, started);
        if let Some(i) = free {
            let high = started as f32;
            let low = (started - f64::from(high)) as f32;
            kept[i * KEPT_DELAY_FLOATS..(i + 1) * KEPT_DELAY_FLOATS]
                .copy_from_slice(&[high, low, marker, delays[0], delays[1]]);
        }
        delays
    }

    pub fn anim_time_update(&self) -> Option<&Program> {
        self.anim_time_update.as_ref()
    }

    pub fn weight_varies(&self) -> bool {
        varies(self.blend_weight.reads())
    }

    pub fn weight_in(&self, scope: &mut Scope<'_>) -> Outcome {
        self.blend_weight.evaluate(scope, &mut NoQueries)
    }

    pub fn pose(
        &self,
        bind: &[Option<usize>],
        time: f32,
        weight: f32,
        scope: &mut Scope<'_>,
        mut disabled: Disabled<'_>,
        pose: &mut Pose,
    ) -> usize {
        pose.reset();
        let mut failures = 0;

        for (index, (channels, bone)) in self.bones.iter().zip(bind).enumerate() {
            let Some(target) = bone.and_then(|i| pose.0.get_mut(i)) else {
                continue;
            };

            let kinds = [&channels.rotation, &channels.position, &channels.scale];
            for (kind, channel) in kinds.into_iter().enumerate() {
                let flag = index * CHANNELS_PER_BONE + kind;
                if disabled.get(flag) {
                    continue;
                }
                let value = match sample(channel, time, scope) {
                    Sampled::Value(value) => value,
                    Sampled::Absent => continue,
                    Sampled::Failed => {
                        failures += 1;
                        disabled.set(flag);
                        continue;
                    }
                };
                match kind {
                    ROTATION => add_weighted(&mut target.rotation, value, weight),
                    POSITION => add_weighted(&mut target.position, value, weight),
                    _ => {
                        for (axis, v) in target.scale.iter_mut().zip(value) {
                            *axis *= 1.0 + (v - 1.0) * weight;
                        }
                    }
                }
            }
        }
        failures
    }

    /// Start and loop delay. Evaluate once per play, so a random delay holds for the whole run.
    pub fn delays(&self, rt: &mut Runtime) -> [f32; 2] {
        [self.start_delay.eval(rt), self.loop_delay.eval(rt)]
    }

    /// Time within the animation, or `None` before it starts. `delays` is from `delays`.
    pub fn local_time(&self, elapsed: f32, delays: [f32; 2]) -> Option<f32> {
        let [start_delay, loop_delay] = delays;
        let t = elapsed - start_delay;
        if t < 0.0 {
            return None;
        }

        match self.loop_mode {
            Loop::Once | Loop::Hold => Some(t.min(self.length)),
            Loop::Repeat => {
                let period = self.length + loop_delay.max(0.0);
                if period <= 0.0 {
                    return Some(0.0);
                }
                Some((t % period).min(self.length))
            }
        }
    }

    pub fn crossed(
        &self,
        from: Option<f32>,
        to: f32,
        delays: [f32; 2],
    ) -> impl Iterator<Item = &KeyEvent> + '_ {
        self.crossings(from, to, delays).map(|(event, _)| event)
    }

    pub fn crossings(
        &self,
        from: Option<f32>,
        to: f32,
        delays: [f32; 2],
    ) -> impl Iterator<Item = (&KeyEvent, f32)> + '_ {
        let events = &self.events[..];
        let through = |t: f32| events.partition_point(|e| e.time <= t);
        let lap_end = through(self.length);

        let (head, tail, lap) = match self.lap(to, delays) {
            None => (0..0, 0..0, 0),
            Some((lap_to, at_to)) => {
                let (lap_from, after) = match from.and_then(|f| self.lap(f, delays)) {
                    Some((lap, at)) => (lap, through(at)),
                    None => (0, 0),
                };
                match lap_to - lap_from {
                    0 => (after..through(at_to).max(after), 0..0, lap_to + 1),
                    1 => (after..lap_end.max(after), 0..through(at_to), lap_to),
                    gap if gap > 1 => {
                        let split = through(at_to);
                        (split..lap_end.max(split), 0..split, lap_to)
                    }
                    _ => (0..0, 0..0, 0),
                }
            }
        };
        let start = self.lap_start(delays);
        let head = events[head]
            .iter()
            .map(move |e| (e, start(lap - 1) + e.time));
        let tail = events[tail].iter().map(move |e| (e, start(lap) + e.time));
        head.chain(tail)
    }

    pub fn crossings_every_lap<'s>(
        &'s self,
        from: Option<f32>,
        to: f32,
        delays: [f32; 2],
        cap: usize,
        keep: impl Fn(&KeyEvent) -> bool + Copy + 's,
    ) -> impl Iterator<Item = (&'s KeyEvent, f32)> + 's {
        let events = &self.events[..];
        let through = |t: f32| events.partition_point(|e| e.time <= t);
        let lap_end = through(self.length);
        let (first, last, after, until) = match self.lap(to, delays) {
            None => (0, -1, 0, 0),
            Some((lap_to, at_to)) => {
                let (lap_from, after) = match from.and_then(|f| self.lap(f, delays)) {
                    Some((lap, at)) => (lap, through(at)),
                    None => (0, 0),
                };
                (lap_from, lap_to, after, through(at_to))
            }
        };
        let per_lap = events[..lap_end].iter().filter(|e| keep(e)).count();
        let kept = (cap / per_lap.max(1)) as i64 + 2;
        let start = first.max(last - kept);
        let span = move |lap: i64| {
            let lo = if lap == first { after } else { 0 };
            let hi = if lap == last { until } else { lap_end };
            lo..hi.max(lo)
        };
        let total: usize = (start..=last)
            .map(|lap| events[span(lap)].iter().filter(|e| keep(e)).count())
            .sum();
        let begin = self.lap_start(delays);
        (start..=last)
            .flat_map(move |lap| {
                events[span(lap)]
                    .iter()
                    .filter(move |e| keep(e))
                    .map(move |e| (e, begin(lap) + e.time))
            })
            .skip(total.saturating_sub(cap))
    }

    fn lap_start(&self, delays: [f32; 2]) -> impl Fn(i64) -> f32 + Copy {
        let [start_delay, loop_delay] = delays;
        let period = match self.loop_mode {
            Loop::Repeat => (self.length + loop_delay.max(0.0)).max(0.0),
            Loop::Once | Loop::Hold => 0.0,
        };
        move |lap| start_delay + lap as f32 * period
    }

    pub fn period(&self, delays: [f32; 2]) -> Option<f32> {
        let period = self.length + delays[1].max(0.0);
        (self.loop_mode == Loop::Repeat && period > 0.0).then_some(period)
    }

    fn lap(&self, elapsed: f32, delays: [f32; 2]) -> Option<(i64, f32)> {
        let [start_delay, loop_delay] = delays;
        let t = elapsed - start_delay;
        if t < 0.0 {
            return None;
        }
        match self.loop_mode {
            Loop::Once | Loop::Hold => Some((0, t.min(self.length))),
            Loop::Repeat => {
                let period = self.length + loop_delay.max(0.0);
                if period <= 0.0 {
                    return Some((0, 0.0));
                }
                let within = t % period;
                let lap = ((t - within) / period).round() as i64;
                Some((lap, within.min(self.length)))
            }
        }
    }

    /// A `Once` animation past its end only repeats its last frame.
    pub fn holds_at(&self, time: f32) -> bool {
        self.loop_mode != Loop::Repeat && time >= self.length
    }

    pub fn weight(&self, rt: &mut Runtime) -> f32 {
        self.blend_weight.eval(rt)
    }

    /// Player bones match as the client does, so `left_arm` drives `leftArm`.
    pub fn bind(&self, model: &Model) -> Vec<Option<usize>> {
        self.bones
            .iter()
            .map(|b| {
                model.bone(&b.name).or_else(|| {
                    crate::cosmetic::player_bone(&b.name).and_then(|name| model.bone(name))
                })
            })
            .collect()
    }
}

/// Blends over the rest pose: rotation and position scale by `weight`, scale eases towards 1.
pub fn evaluate(
    animation: &Animation,
    bind: &[Option<usize>],
    time: f32,
    weight: f32,
    rt: &mut Runtime,
    pose: &mut Pose,
) {
    let mut scope = rt.frame();
    animation.pre_animation(&mut scope);
    animation.pose(bind, time, weight, &mut scope, Disabled::none(), pose);
}

const CHANNELS_PER_BONE: usize = 3;
pub const KEPT_DELAY_FLOATS: usize = 5;
const KEPT_DELAY_MATCH: f64 = 1e-3;
const ROTATION: usize = 0;
const POSITION: usize = 1;

fn run_scripts(scripts: &[Script], scope: &mut Scope<'_>) -> bool {
    let mut failed = false;
    for script in scripts {
        failed |= script.program.evaluate(scope, &mut NoQueries).failed;
    }
    failed
}

fn add_weighted(target: &mut [f32; 3], value: [f32; 3], weight: f32) {
    for i in 0..3 {
        target[i] += value[i] * weight;
    }
}

enum Sampled {
    Value([f32; 3]),
    Absent,
    Failed,
}

fn sample(channel: &Channel, time: f32, scope: &mut Scope<'_>) -> Sampled {
    match try_sample(channel, time, scope) {
        Ok(Some(value)) => Sampled::Value(value),
        Ok(None) => Sampled::Absent,
        Err(Exhausted) => Sampled::Failed,
    }
}

struct Exhausted;

fn try_sample(
    channel: &Channel,
    time: f32,
    scope: &mut Scope<'_>,
) -> std::result::Result<Option<[f32; 3]>, Exhausted> {
    let frames = &channel.0;
    let Some(last) = frames.len().checked_sub(1) else {
        return Ok(None);
    };

    if time <= frames[0].time {
        return triple(&frames[0].pre, scope).map(Some);
    }
    if time >= frames[last].time {
        return triple(&frames[last].post, scope).map(Some);
    }

    let next = frames.partition_point(|k| k.time <= time);
    let (a, b) = (&frames[next - 1], &frames[next]);
    let span = b.time - a.time;
    let t = if span > 0.0 {
        (time - a.time) / span
    } else {
        0.0
    };

    let from = triple(&a.post, scope)?;
    let to = triple(&b.pre, scope)?;
    Ok(Some(match a.lerp {
        Lerp::Step => from,
        Lerp::Linear => lerp3(from, to, t),
        Lerp::CatmullRom => {
            let before = triple(&frames[next.saturating_sub(2)].post, scope)?;
            let after = triple(&frames[(next + 1).min(last)].pre, scope)?;
            std::array::from_fn(|i| catmull_rom(before[i], from[i], to[i], after[i], t))
        }
        Lerp::Bezier => std::array::from_fn(|i| {
            bezier(a, b, from[i], to[i], i, time).unwrap_or_else(|| from[i] + (to[i] - from[i]) * t)
        }),
    }))
}

fn triple(
    programs: &[Program; 3],
    scope: &mut Scope<'_>,
) -> std::result::Result<[f32; 3], Exhausted> {
    let mut out = [0.0; 3];
    for (slot, program) in out.iter_mut().zip(programs) {
        let outcome = program.evaluate(scope, &mut NoQueries);
        if outcome.failed {
            return Err(Exhausted);
        }
        *slot = outcome.value as f32;
    }
    Ok(out)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((2.0 * p1)
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Blockbench handles are time/value offsets from their key. Time is solved by bisection
/// for determinism.
fn bezier(a: &Keyframe, b: &Keyframe, from: f32, to: f32, axis: usize, time: f32) -> Option<f32> {
    let (ah, bh) = (a.handles.as_ref()?, b.handles.as_ref()?);
    let x = [
        a.time,
        a.time + ah.right_time[axis],
        b.time + bh.left_time[axis],
        b.time,
    ];
    let y = [
        from,
        from + ah.right_value[axis],
        to + bh.left_value[axis],
        to,
    ];

    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..16 {
        let mid = 0.5 * (lo + hi);
        if cubic(&x, mid) < time {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    Some(cubic(&y, 0.5 * (lo + hi)))
}

fn cubic(p: &[f32; 4], s: f32) -> f32 {
    let u = 1.0 - s;
    u * u * u * p[0] + 3.0 * u * u * s * p[1] + 3.0 * u * s * s * p[2] + s * s * s * p[3]
}

pub fn parse(json: &str, interner: &mut Interner) -> Result<Vec<Animation>> {
    parse_with(json, interner, Rules::Bedrock)
}

pub fn parse_with(json: &str, interner: &mut Interner, rules: Rules) -> Result<Vec<Animation>> {
    let root: Value = serde_json::from_str(json)?;
    let animations = root
        .get("animations")
        .and_then(Value::as_object)
        .ok_or(Error::NoAnimations)?;
    let mut cx = Compiler { interner, rules };
    animations
        .iter()
        .map(|(name, body)| parse_animation(name, body, &mut cx))
        .collect()
}

struct Compiler<'a> {
    interner: &'a mut Interner,
    rules: Rules,
}

impl Compiler<'_> {
    fn compile(&mut self, source: &str) -> Result<Program> {
        let options = Options {
            rules: self.rules,
            queries: QueryScope::Any,
        };
        Ok(compile_with(
            source,
            self.interner,
            &options,
            &mut Warnings::default(),
        )?)
    }
}

fn parse_animation(name: &str, body: &Value, cx: &mut Compiler<'_>) -> Result<Animation> {
    let loop_mode = match body.get("loop") {
        Some(Value::Bool(true)) => Loop::Repeat,
        Some(Value::String(mode)) if mode == "hold_on_last_frame" => Loop::Hold,
        _ => Loop::Once,
    };

    let bones = match body.get("bones").and_then(Value::as_object) {
        Some(map) => map
            .iter()
            .map(|(bone, channels)| parse_bone(bone, channels, cx))
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };

    let events = key_events(body, cx)?;

    let explicit = body.get("animation_length").and_then(Value::as_f64);
    let length = explicit.map(|v| v as f32).unwrap_or_else(|| {
        bones
            .iter()
            .flat_map(|b| [&b.rotation, &b.position, &b.scale])
            .filter_map(|c| c.0.last())
            .map(|k| k.time)
            .chain(events.last().map(|e| e.time))
            .fold(0.0f32, f32::max)
    });

    Ok(Animation {
        name: name.to_string(),
        length,
        loop_mode,
        start_delay: scalar(body.get("start_delay"), 0.0, cx)?,
        loop_delay: scalar(body.get("loop_delay"), 0.0, cx)?,
        blend_weight: scalar(body.get("blend_weight"), 1.0, cx)?,
        anim_time_update: match body.get("anim_time_update") {
            None | Some(Value::Null) => None,
            update => Some(scalar(update, 0.0, cx)?),
        },
        initialize: scripts(body.get("initialize"), cx)?,
        pre_animation: scripts(body.get("pre_animation"), cx)?,
        bones,
        events,
    })
}

fn timed_entries(section: Option<&Value>) -> Vec<(f32, String, &Value)> {
    let Some(map) = section.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, value) in map {
        let Some(time) = key.parse::<f32>().ok().filter(|t| t.is_finite()) else {
            continue;
        };
        match value {
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    out.push((time, join(key, &i.to_string()), item));
                }
            }
            other => out.push((time, key.clone(), other)),
        }
    }
    out
}

fn named(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn key_events(body: &Value, cx: &mut Compiler<'_>) -> Result<Vec<KeyEvent>> {
    let mut out = Vec::new();

    for (time, at, value) in timed_entries(body.get("particle_effects")) {
        if let Some(effect) = named(value, "effect") {
            out.push(KeyEvent {
                time,
                kind: KeyEventKind::Particle {
                    effect,
                    locator: named(value, "locator"),
                    pre_effect_script: named(value, "pre_effect_script"),
                },
                at: at.into(),
            });
        }
    }

    for (time, at, value) in timed_entries(body.get("sound_effects")) {
        if let Some(effect) = named(value, "effect") {
            out.push(KeyEvent {
                time,
                kind: KeyEventKind::Sound { effect },
                at: at.into(),
            });
        }
    }

    for (time, at, value) in timed_entries(body.get("timeline")) {
        let Some(source) = value.as_str().map(str::trim) else {
            continue;
        };
        if source.is_empty() || source.starts_with('/') || source.starts_with('@') {
            continue;
        }
        out.push(KeyEvent {
            time,
            kind: KeyEventKind::Timeline(cx.compile(source)?),
            at: at.into(),
        });
    }

    out.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(out)
}

fn scripts(value: Option<&Value>, cx: &mut Compiler<'_>) -> Result<Vec<Script>> {
    let sources: Vec<(Option<usize>, &str)> = match value {
        Some(Value::String(s)) => vec![(None, s.as_str())],
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| Some((Some(i), item.as_str()?)))
            .collect(),
        _ => Vec::new(),
    };
    sources
        .into_iter()
        .map(|(at, source)| {
            Ok(Script {
                at,
                program: cx.compile(source)?,
            })
        })
        .collect()
}

fn parse_bone(name: &str, channels: &Value, cx: &mut Compiler<'_>) -> Result<BoneChannels> {
    let mut channel = |key: &str| match channels.get(key) {
        Some(v) => parse_channel(v, cx),
        None => Ok(Channel::default()),
    };

    Ok(BoneChannels {
        name: name.to_string(),
        rotation: channel("rotation")?,
        position: channel("position")?,
        scale: channel("scale")?,
    })
}

fn parse_channel(value: &Value, cx: &mut Compiler<'_>) -> Result<Channel> {
    if let Some(map) = value.as_object() {
        if is_keyframe_map(map) {
            let mut frames: Vec<Keyframe> = map
                .iter()
                .map(|(key, v)| {
                    let time = key.parse::<f32>().unwrap_or(0.0);
                    let mut frame = parse_keyframe(time, v, cx)?;
                    frame.key = Some(key.as_str().into());
                    Ok(frame)
                })
                .collect::<Result<_>>()?;
            frames.sort_by(|a, b| a.time.total_cmp(&b.time));
            return Ok(Channel(frames));
        }
    }

    Ok(Channel(vec![parse_keyframe(0.0, value, cx)?]))
}

fn is_keyframe_map(map: &Map<String, Value>) -> bool {
    !map.is_empty() && map.keys().all(|k| k.parse::<f32>().is_ok())
}

fn parse_keyframe(time: f32, value: &Value, cx: &mut Compiler<'_>) -> Result<Keyframe> {
    let detailed = value
        .as_object()
        .filter(|m| !is_keyframe_map(m))
        .filter(|m| m.contains_key("pre") || m.contains_key("post") || m.contains_key("lerp_mode"));

    let Some(map) = detailed else {
        let (v, shape) = vector(value, cx)?;
        return Ok(Keyframe {
            time,
            key: None,
            source: Source::Plain(shape),
            pre: v.clone(),
            post: v,
            lerp: Lerp::Linear,
            handles: None,
        });
    };

    let (pre, pre_shape) = match map.get("pre") {
        Some(v) => vector(v, cx)?,
        None => (vector(map.get("post").unwrap_or(&Value::Null), cx)?.0, None),
    };
    let (post, post_shape) = match map.get("post") {
        Some(v) => vector(v, cx)?,
        None => (pre.clone(), None),
    };

    let lerp = match map.get("lerp_mode").and_then(Value::as_str) {
        Some("catmullrom") => Lerp::CatmullRom,
        Some("step") => Lerp::Step,
        Some("bezier") => Lerp::Bezier,
        _ => Lerp::Linear,
    };
    let handles = (lerp == Lerp::Bezier).then(|| {
        Box::new(Handles {
            left_time: numbers(map.get("bezier_left_time"), 0.0),
            left_value: numbers(map.get("bezier_left_value"), 0.0),
            right_time: numbers(map.get("bezier_right_time"), 0.0),
            right_value: numbers(map.get("bezier_right_value"), 0.0),
        })
    });

    Ok(Keyframe {
        time,
        key: None,
        source: Source::Split {
            pre: pre_shape,
            post: post_shape,
        },
        pre,
        post,
        lerp,
        handles,
    })
}

/// A scalar broadcasts to every axis.
fn vector(value: &Value, cx: &mut Compiler<'_>) -> Result<([Program; 3], Option<Shape>)> {
    match value {
        Value::Null => Ok((std::array::from_fn(|_| Program::number(0.0)), None)),
        Value::Array(items) if items.len() == 3 => {
            let x = scalar(Some(&items[0]), 0.0, cx)?;
            let y = scalar(Some(&items[1]), 0.0, cx)?;
            let z = scalar(Some(&items[2]), 0.0, cx)?;
            Ok(([x, y, z], Some(Shape::Axes)))
        }
        Value::Number(_) | Value::String(_) => {
            let p = scalar(Some(value), 0.0, cx)?;
            Ok(([p.clone(), p.clone(), p], Some(Shape::Scalar)))
        }
        _ => Err(Error::BadValue(value.to_string())),
    }
}

fn scalar(value: Option<&Value>, default: f32, cx: &mut Compiler<'_>) -> Result<Program> {
    Ok(match value {
        None | Some(Value::Null) => Program::number(default),
        Some(Value::Number(n)) => Program::number(n.as_f64().unwrap_or(0.0) as f32),
        Some(Value::String(s)) => cx.compile(s)?,
        Some(Value::Bool(b)) => Program::number(if *b { 1.0 } else { 0.0 }),
        Some(other) => return Err(Error::BadValue(other.to_string())),
    })
}

fn numbers(value: Option<&Value>, default: f32) -> [f32; 3] {
    let items = value.and_then(Value::as_array);
    std::array::from_fn(|i| {
        items
            .and_then(|a| a.get(i))
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .unwrap_or(default)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{self, Arms};
    use crate::signals::SIGNAL_COUNT;
    use crate::wearer::{seed, CosmeticSlot, WearerDeclarations, WearerTable};

    fn load(json: &str) -> (Vec<Animation>, Interner) {
        let mut interner = Interner::default();
        let animations = parse(json, &mut interner).expect("parse");
        (animations, interner)
    }

    fn arm(json: &str, time: f32) -> [f32; 3] {
        let (animations, interner) = load(json);
        let model = player::model(Arms::Wide);
        let bind = animations[0].bind(&model);
        let mut rt = Runtime::new(&interner);
        let mut pose = Pose::rest(&model);
        evaluate(&animations[0], &bind, time, 1.0, &mut rt, &mut pose);
        pose.0[model.bone("rightArm").unwrap()].rotation
    }

    fn rotation(keys: &str) -> String {
        format!(r#"{{"animations":{{"a":{{"animation_length":2.0,"bones":{{"rightArm":{{"rotation":{keys}}}}}}}}}}}"#)
    }

    #[test]
    fn keyframes_sample_by_their_lerp_mode() {
        let linear = rotation(r#"{"0.0":[0,0,0],"2.0":[90,0,0]}"#);
        let step = rotation(r#"{"0.0":{"post":[0,0,0],"lerp_mode":"step"},"2.0":[90,0,0]}"#);
        let jump = rotation(r#"{"0.0":[0,0,0],"1.0":{"pre":[10,0,0],"post":[90,0,0]},"2.0":[90,0,0]}"#);
        let cases: &[(&str, f32, f32)] = &[
            (&linear, 0.0, 0.0),
            (&linear, 1.0, 45.0),
            (&linear, 5.0, 90.0),
            (&step, 1.9, 0.0),
            (&step, 2.0, 90.0),
            (&jump, 0.5, 5.0),
            (&jump, 1.5, 90.0),
        ];
        for (json, time, want) in cases {
            assert_eq!(arm(json, *time)[0], *want, "{json} at {time}");
        }
        assert_eq!(arm(&rotation("30"), 0.0), [30.0; 3]);
        let scripted = r#"{"animations":{"a":{"pre_animation":["t.x = 30;","t.y = t.x + 1;"],
            "bones":{"rightArm":{"rotation":["t.x","t.y","t.x + t.y"]}}}}}"#;
        assert_eq!(arm(scripted, 0.0), [30.0, 31.0, 61.0]);

        let bezier = r#"{"animations":{"a":{"animation_length":1.0,"bones":{"rightArm":{"rotation":{
            "0.0":{"post":[0,0,0],"lerp_mode":"bezier","bezier_right_time":[0.3,0.3,0.3],"bezier_right_value":[0,0,0],
                   "bezier_left_time":[0,0,0],"bezier_left_value":[0,0,0]},
            "1.0":{"pre":[90,0,0],"lerp_mode":"bezier","bezier_left_time":[-0.3,-0.3,-0.3],"bezier_left_value":[0,0,0],
                   "bezier_right_time":[0,0,0],"bezier_right_value":[0,0,0]}}}}}}}"#;
        assert!((arm(bezier, 1.0)[0] - 90.0).abs() < 1e-3);
        assert!((arm(bezier, 0.5)[0] - 45.0).abs() < 1e-2);
        assert!(arm(bezier, 0.25)[0] < 22.5);
        let smooth = r#"{"animations":{"a":{"animation_length":3.0,"bones":{"rightArm":{"rotation":{"0.0":[0,0,0],
            "1.0":{"post":[10,0,0],"lerp_mode":"catmullrom"},"2.0":{"post":[90,0,0],"lerp_mode":"catmullrom"},
            "3.0":[100,0,0]}}}}}}"#;
        let value = arm(smooth, 1.25)[0];
        assert!((value - 30.0).abs() > 1.0 && (0.0..100.0).contains(&value), "{value}");
    }

    #[test]
    fn animations_parse_lengths_loops_and_file_order() {
        let (animations, interner) = load(
            r#"{"animations":{
                "held":{"loop":"hold_on_last_frame","animation_length":2.0,"bones":{}},
                "once":{"loop":false,"animation_length":2.0,"bones":{"tail":{"rotation":[45,0,0]}}},
                "repeat":{"loop":true,"animation_length":2.0,"bones":{}},
                "keyed":{"sound_effects":{"3.0":{"effect":"x"}},"bones":{"rightArm":{"rotation":{"0.0":0,"4.5":90}}}}}}"#,
        );
        let names: Vec<_> = animations.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["held", "once", "repeat", "keyed"]);
        let modes: Vec<_> = animations.iter().map(|a| a.loop_mode).collect();
        assert_eq!(modes, [Loop::Hold, Loop::Once, Loop::Repeat, Loop::Once]);
        assert_eq!(animations[3].length, 4.5);
        let mut rt = Runtime::new(&interner);
        let delays = animations[2].delays(&mut rt);
        assert_eq!(animations[2].local_time(5.0, delays), Some(1.0));
        assert!(!animations[2].holds_at(2.0));
        for held in &animations[..2] {
            assert_eq!(held.local_time(9.0, [0.0, 0.0]), Some(2.0));
            assert!(held.holds_at(2.0) && !held.holds_at(1.0));
        }
        assert_eq!(animations[1].bind(&player::model(Arms::Wide)), vec![None]);
    }

    struct Rig {
        animation: Animation,
        bind: Vec<Option<usize>>,
        bone: usize,
        table: WearerTable,
        slot: CosmeticSlot,
        rt: Runtime,
        pose: Pose,
        disabled: Vec<u32>,
        failures: usize,
    }

    impl Rig {
        fn new(json: &str) -> Rig {
            let (mut animations, interner) = load(json);
            let animation = animations.remove(0);
            let model = player::model(Arms::Wide);
            let mut table = WearerTable::new();
            let declarations = WearerDeclarations { variables: interner.variable_count(), ..WearerDeclarations::default() };
            let slot = table.add_cosmetic(declarations).unwrap();
            Rig {
                bind: animation.bind(&model),
                bone: model.bone("rightArm").unwrap(),
                disabled: vec![0; flag_words(animation.channel_count())],
                animation,
                table,
                slot,
                rt: Runtime::new(&interner),
                pose: Pose::rest(&model),
                failures: 0,
            }
        }

        fn draw(&mut self, clock: f64, wearer: u64) -> [f32; 3] {
            self.table.begin_frame(clock);
            let record = self.table.draw(self.slot, wearer, &[0.0; SIGNAL_COUNT], 0).unwrap();
            let mut scope = self.rt.wearer_frame(record.variables, seed(wearer, 0, 0), clock);
            if record.appear {
                assert!(!self.animation.initialize(&mut scope));
            }
            assert!(!self.animation.pre_animation(&mut scope));
            let disabled = Disabled::new(&mut self.disabled, 0);
            self.failures += self.animation.pose(&self.bind, 0.0, 1.0, &mut scope, disabled, &mut self.pose);
            self.pose.0[self.bone].rotation
        }
    }

    #[test]
    fn initialize_runs_once_per_record_and_pre_animation_every_frame() {
        let json = r#"{"animations":{"a":{"initialize":"v.inits = v.inits + 1; v.r = math.random(0, 100);",
            "pre_animation":["v.frames = v.frames + 1;","t.lift = v.frames * 10; t.carry = t.carry + 1;"],
            "bones":{"rightArm":{"rotation":["t.lift","v.inits + t.carry * 10","v.r"]}}}}}"#;
        let mut rig = Rig::new(json);
        let mut late = Rig::new(json);
        let roll = rig.draw(1.0, 1)[2];
        for frame in 2..=3 {
            assert_eq!(rig.draw(frame as f64, 1), [frame as f32 * 10.0, 11.0, roll]);
        }
        assert_eq!(late.draw(500.0, 1), [10.0, 11.0, roll]);
        assert_ne!(late.draw(500.0, 2)[2], roll);
        rig.table.forget(rig.slot, 1);
        assert_eq!(rig.draw(4.0, 1), [10.0, 11.0, roll]);
        assert_eq!(rig.failures, 0);
    }

    #[test]
    fn a_channel_over_budget_is_disabled_alone() {
        let mut rig = Rig::new(
            r#"{"animations":{"a":{"bones":{
            "leftArm":{"rotation":[5,0,0],"position":["math.die_roll(1000000000, 0, 1)",0,0]},
            "rightArm":{"rotation":[30,0,0]}}}}}"#,
        );
        let left = player::model(Arms::Wide).bone("leftArm").unwrap();
        assert_eq!(rig.draw(0.0, 1)[0], 30.0);
        assert_eq!((rig.failures, rig.disabled.as_slice()), (1, [0b10].as_slice()));
        assert_eq!((rig.pose.0[left].rotation[0], rig.pose.0[left].position), (5.0, [0.0; 3]));
        rig.draw(0.05, 1);
        assert_eq!(rig.failures, 1);

        let (animations, _) = load(
            r#"{"animations":{"a":{"bones":{"leftArm":{"rotation":[1,0,0]},"rightArm":{"rotation":[1,0,0]}}},
                "b":{"bones":{"head":{"rotation":[1,0,0]}}},"c":{"bones":{}}}}"#,
        );
        assert_eq!(channel_offsets(&animations), (vec![0, 6, 9], 9));
        assert_eq!((flag_words(32), flag_words(33)), (1, 2));
        let mut words = [0u32; 2];
        let mut flags = Disabled::new(&mut words, 30);
        flags.set(1);
        flags.set(3);
        assert!(flags.get(1) && !flags.get(2));
        assert_eq!(words, [1 << 31, 1 << 1]);
    }

    const STEPS: &str = r#"{"animations":{
        "loop":{"loop":true,"animation_length":1.0,
            "sound_effects":{"0.0":{"effect":"a"},"0.25":{"effect":"b"},"0.75":{"effect":"c"},"1.0":{"effect":"d"},"1.5":{"effect":"never"}},"bones":{}},
        "once":{"animation_length":1.0,
            "particle_effects":{"0.0":[{"effect":"a"},{"effect":""}],"0.25":{"effect":"b","locator":"tip","pre_effect_script":"v.size = 2;"},"late":{"effect":"x"}},
            "sound_effects":{"1.0":{"effect":"d"},"1.5":{"effect":"never"}},"timeline":{"0.5":"v.flag = 1;"},"bones":{}}}}"#;

    #[test]
    fn keyframe_events_parse_into_time_order() {
        let (animations, interner) = load(STEPS);
        let events: Vec<(f32, String)> = animations[1]
            .events
            .iter()
            .map(|e| {
                let label = match &e.kind {
                    KeyEventKind::Particle { effect, locator, .. } => format!("p:{effect}@{}", locator.as_deref().unwrap_or("-")),
                    KeyEventKind::Sound { effect } => format!("s:{effect}"),
                    KeyEventKind::Timeline(program) => format!("t:{}", program.nodes()),
                };
                (e.time, label)
            })
            .collect();
        let want = [(0.0, "p:a@-"), (0.25, "p:b@tip"), (0.5, "t:3"), (1.0, "s:d"), (1.5, "s:never")];
        assert_eq!(events, want.map(|(t, l)| (t, l.to_string())));
        assert!(interner.variable_slot("size").is_none());
        let flag = interner.variable_slot("flag").unwrap();
        let mut rt = Runtime::new(&interner);
        let mut scope = rt.frame();
        let KeyEventKind::Timeline(program) = &animations[1].events[2].kind else { panic!() };
        assert!(!program.evaluate(&mut scope, &mut NoQueries).failed);
        assert_eq!(scope.variable(flag), 1.0);
    }

    #[test]
    fn crossings_fire_each_key_once_per_lap() {
        let (animations, _) = load(STEPS);
        let (looping, once) = (&animations[0], &animations[1]);
        let crossed = |a: &Animation, from, to, delays| a.crossed(from, to, delays).map(|e| e.time).collect::<Vec<f32>>();
        let crossings =
            |a: &Animation, from, to, delays| a.crossings(from, to, delays).map(|(e, at)| (e.time, at)).collect::<Vec<_>>();
        let none = [0.0, 0.0];
        assert_eq!(crossed(once, None, 0.0, none), [0.0]);
        assert_eq!(crossed(once, Some(0.0), 0.1, none), Vec::<f32>::new());
        assert_eq!(crossed(once, Some(0.1), 0.25, none), [0.25]);
        assert_eq!(crossed(once, Some(1.4), 9.0, none), Vec::<f32>::new());
        assert_eq!(crossed(once, None, 9.0, none), [0.0, 0.25, 0.5, 1.0]);
        assert_eq!(crossed(once, Some(0.4), 0.5, [0.5, 0.0]), [0.0]);
        assert_eq!(crossed(looping, Some(0.6), 1.3, none), [0.75, 1.0, 0.0, 0.25]);
        assert_eq!(crossed(looping, Some(0.1), 3.5, none), [0.75, 1.0, 0.0, 0.25]);
        assert_eq!(crossed(looping, Some(0.8), 1.5, [0.0, 1.0]), [1.0]);
        assert_eq!(crossed(looping, Some(1.5), 2.1, [0.0, 1.0]), [0.0]);
        assert_eq!(crossings(looping, Some(0.1), 1.3, none), [(0.25, 0.25), (0.75, 0.75), (1.0, 1.0), (0.0, 1.0), (0.25, 1.25)]);
        assert_eq!(crossings(looping, Some(0.1), 3.5, none), [(0.75, 2.75), (1.0, 3.0), (0.0, 3.0), (0.25, 3.25)]);
        assert_eq!(crossings(looping, Some(2.4), 2.8, [0.5, 1.0]), [(0.0, 2.5), (0.25, 2.75)]);

        let every = |from, to, cap| {
            looping.crossings_every_lap(Some(from), to, none, cap, |_| true).map(|(e, at)| (e.time, at)).collect::<Vec<_>>()
        };
        assert_eq!(every(0.1, 2.3, 64).len(), 9);
        assert_eq!(every(0.6, 1.3, 64), crossings(looping, Some(0.6), 1.3, none));
        assert_eq!(every(0.1, 2.3, 3), [(1.0, 2.0), (0.0, 2.0), (0.25, 2.25)]);
        assert_eq!(every(0.1, 1.0e6 + 0.3, 5).len(), 5);
        assert_eq!(every(2.3, 0.1, 64), []);
        let c = |e: &KeyEvent| matches!(&e.kind, KeyEventKind::Sound { effect } if effect == "c");
        let kept: Vec<f32> = looping.crossings_every_lap(Some(0.1), 20.3, none, 16, c).map(|(_, at)| at).collect();
        assert_eq!((kept.len(), kept[0], kept[15]), (16, 4.75, 19.75));
    }
}
