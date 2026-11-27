mod events;
mod format;
pub mod geometry;
mod moments;
mod particles;
pub mod ring;
pub mod trail;

use std::ops::Range;

use crate::animation::Animation;
use crate::attach::PlayerLocator;
use crate::container::Bundle;
use crate::controller::{Anchor, ControllerSet, Effect};
use crate::cost::Features;
use crate::ir::Model;
use crate::issues::Report;
use crate::limits::EFFECT_EMITTER_RUNS;
use crate::math::Affine;
use crate::molang::Program;
use crate::reactions::{Reaction, ReactionOut};
use crate::wearer::{WearerDeclarations, WearerRecord};

pub use events::{effect_keys, last_step, Solo, PLAYBACK_SLOTS};
pub use format::{MANIFEST_KEYS, MOMENTS, VANILLA_PARTICLES, VANILLA_SOUNDS};
pub use moments::{moment_clip, ogg_opus, ogg_seconds};
pub use particles::{
    emitter_randoms, for_each_live, lane, size_at, spawn, spawn_hash, spawning, tint_at, uv_at,
    worst_case, Budget, Vars, VAR_COUNT, VAR_NAMES,
};

pub const SOURCE_KEYFRAME: u32 = 0;
pub const SOURCE_REACTION: u32 = 1;
pub const SOURCE_EVENT: u32 = 2;
pub const NO_CONTROLLER: u32 = u32::MAX;
pub const EMITTER_START: usize = 0;
pub const EMITTER_STOP: usize = 1;
pub const EMITTER_LOCATOR: usize = 2;
pub const EMITTER_ACTIVATION_TIMES: usize = 3;
pub const EMITTER_ACTIVATIONS: usize = 2;
pub const EMITTER_TIMES: usize = EMITTER_ACTIVATION_TIMES * EMITTER_ACTIVATIONS;

pub const QUAD_FLOATS: usize = 27;
pub const QUAD_TEXTURE: usize = 0;
pub const QUAD_BLEND: usize = 1;
pub const QUAD_COLOR: usize = 2;
pub const QUAD_CORNERS: usize = 6;
pub const CORNER_FLOATS: usize = 5;
pub const QUAD_FLAGS: usize = 26;
pub const QUAD_FLAG_EMISSIVE: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocatorRef {
    Model(u16),
    Player(PlayerLocator),
}

impl LocatorRef {
    pub fn encode(self) -> f32 {
        match self {
            LocatorRef::Model(i) => f32::from(i),
            LocatorRef::Player(p) => -1.0 - p.index() as f32,
        }
    }

    pub fn decode(value: f32) -> Option<LocatorRef> {
        if !value.is_finite() {
            return None;
        }
        if value >= 0.0 {
            return u16::try_from(value as u32).ok().map(LocatorRef::Model);
        }
        let index = (-1.0 - value) as usize;
        PlayerLocator::ALL
            .get(index)
            .copied()
            .map(LocatorRef::Player)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    #[default]
    Alpha,
    Additive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Facing {
    #[default]
    RotateXyz,
    RotateY,
    LookatXyz,
    LookatY,
    Direction,
    EmitterXy,
    EmitterXz,
    EmitterYz,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Point,
    Sphere {
        radius: f32,
        surface: bool,
    },
    Box {
        half: [f32; 3],
        surface: bool,
    },
    Disc {
        radius: f32,
        normal: [f32; 3],
        surface: bool,
    },
    EntityAabb {
        surface: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rate {
    Instant { count: u32 },
    Steady { per_second: f32, max: u32 },
    Manual { max: u32 },
}

#[derive(Clone, Debug)]
pub enum Lifetime {
    Once {
        seconds: f32,
    },
    Looping {
        active: f32,
        sleep: f32,
    },
    Expression {
        activation: Program,
        expiration: Program,
    },
}

#[derive(Clone, Debug)]
pub enum Direction {
    Outwards,
    Inwards,
    Vector(Box<[Program; 3]>),
    Velocity(Box<[Program; 3]>),
}

#[derive(Clone, Debug)]
pub enum Motion {
    Parametric {
        offset: [Program; 3],
    },
    Dynamic {
        speed: Program,
        direction: Direction,
        acceleration: Box<[Program; 3]>,
        drag: Program,
    },
}

#[derive(Clone, Debug)]
pub enum Tint {
    Fixed(u32),
    Slot {
        slot: u8,
        tone: Option<u8>,
    },
    Gradient {
        interpolant: Program,
        keys: Vec<(f32, u32)>,
    },
    Expression(Box<[Program; 4]>),
}

#[derive(Clone, Debug)]
pub enum Uv {
    Fixed {
        uv: [f32; 2],
        size: [f32; 2],
    },
    Flipbook {
        base: [f32; 2],
        size: [f32; 2],
        step: [f32; 2],
        frames: u32,
        fps: f32,
        stretch: bool,
        looping: bool,
    },
}

#[derive(Clone, Debug)]
pub struct ParticleDef {
    pub id: String,
    pub texture: String,
    pub blend: Blend,
    pub facing: Facing,
    pub size: [Program; 2],
    pub uv: Uv,
    pub tint: Tint,
    pub lifetime: Program,
    pub longest_life: f32,
    pub motion: Motion,
    pub emissive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundDef {
    pub id: String,
    pub file: Option<String>,
    pub vanilla: Option<String>,
    pub volume: f32,
    pub pitch: f32,
}

#[derive(Clone, Debug)]
pub struct EmitterDef {
    pub path: String,
    pub particle: Option<u16>,
    pub sound: Option<u16>,
    pub vanilla: Option<String>,
    pub locator: LocatorRef,
    pub offset: [f32; 3],
    pub shape: Shape,
    pub rate: Rate,
    pub lifetime: Lifetime,
    pub world: bool,
    pub ring: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct TrailDef {
    pub locator: LocatorRef,
    pub ring: u16,
    pub texture: String,
    pub blend: Blend,
    pub width: [f32; 2],
    pub tile: bool,
    pub tint: Tint,
    pub fade_seconds: f32,
    pub min_speed: f32,
    pub gates: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MomentPlan {
    pub name: String,
    pub clip: Option<u16>,
    pub seconds: f32,
}

#[derive(Clone, Debug, Default)]
pub struct EffectSet {
    pub particles: Vec<ParticleDef>,
    pub sounds: Vec<SoundDef>,
    pub emitters: Vec<EmitterDef>,
    pub trails: Vec<TrailDef>,
    pub named: Vec<(String, Range<u16>)>,
    pub ambient: Range<u16>,
    pub locators: Vec<String>,
    pub rings: Vec<LocatorRef>,
    pub ring_points: Vec<u16>,
    pub ring_turns: Vec<bool>,
    pub ring_gates: Vec<bool>,
    pub moments: Vec<MomentPlan>,
    pub playback_slots: u8,
    pub runs: Vec<(usize, usize)>,
    pub budget_runs: Vec<usize>,
    pub edges: Vec<(u16, u32, u32)>,
}

impl EffectSet {
    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
            && self.sounds.is_empty()
            && self.emitters.is_empty()
            && self.trails.is_empty()
    }

    pub fn has_moments(&self) -> bool {
        !self.named.is_empty() || !self.moments.is_empty()
    }

    pub fn runs(&self, emitter: usize) -> (usize, usize) {
        match self.runs.get(emitter) {
            Some(&slot) if self.runs.len() == self.emitters.len() => slot,
            _ => (emitter * EMITTER_TIMES, EMITTER_ACTIVATIONS),
        }
    }

    pub fn budget_runs(&self, emitter: usize) -> usize {
        let runs = self.runs(emitter).1;
        match self.budget_runs.get(emitter) {
            Some(&budget) if self.budget_runs.len() == self.emitters.len() => budget.min(runs),
            _ => runs,
        }
    }

    pub fn run_floats(&self) -> usize {
        match self.runs.last() {
            Some(&(at, runs)) if self.runs.len() == self.emitters.len() => {
                at + runs * EMITTER_ACTIVATION_TIMES
            }
            _ => self.emitters.len() * EMITTER_TIMES,
        }
    }

    pub fn set_runs(&mut self, runs: &[usize]) {
        let mut at = 0;
        self.runs = runs
            .iter()
            .map(|&count| {
                let slot = (at, count);
                at += count * EMITTER_ACTIVATION_TIMES;
                slot
            })
            .collect();
    }

    pub fn named(&self, name: &str) -> Option<Range<u16>> {
        self.named
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, range)| range.clone())
    }

    pub fn named_id(&self, name: &str) -> Option<u16> {
        let index = self.named.iter().position(|(n, _)| n == name)?;
        u16::try_from(index).ok()
    }

    pub fn sound(&self, name: &str) -> Option<u16> {
        let index = self.sounds.iter().position(|s| s.id == name)?;
        u16::try_from(index).ok()
    }

    pub fn locator(&self, name: &str) -> Option<LocatorRef> {
        match self.locators.iter().position(|l| l == name) {
            Some(index) => u16::try_from(index).ok().map(LocatorRef::Model),
            None => PlayerLocator::parse(name).map(LocatorRef::Player),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spawn {
    pub time: f32,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub acceleration: [f32; 3],
    pub drag: f32,
    pub randoms: [f32; 4],
    pub emitter: [f32; 4],
    pub emitter_age: f32,
    pub emitter_period: f32,
    pub emitter_lifetime: f32,
    pub life: f32,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    pub camera: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub origin_shift: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmitKind {
    Particle,
    Sound,
    VanillaParticle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emission {
    pub kind: EmitKind,
    pub controller: u32,
    pub source: u32,
    pub effect: u16,
    pub at: [f32; 3],
    pub volume: f32,
    pub pitch: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct EffectInput<'a> {
    pub model_locators: &'a [Affine],
    pub player_locators: &'a [Affine],
    pub root: &'a Affine,
    pub features: Features,
    pub triggers: &'a ReactionOut,
    pub seed: u64,
    pub seed_epoch: f64,
    pub fuel: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Playing<'a> {
    pub animation: &'a Animation,
    pub clip: usize,
    pub elapsed: f32,
    pub age: f32,
    pub from: Option<f32>,
    pub delays: [f32; 2],
    pub anchor: Option<Anchor>,
    pub late: bool,
    pub fresh: bool,
    pub first: bool,
    pub reaction: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Caught<'a> {
    pub animation: &'a Animation,
    pub clip: usize,
    pub age: f32,
    pub from: f32,
    pub to: f32,
    pub delays: [f32; 2],
    pub ago: [f32; 2],
    pub fresh: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ControllerFire<'a> {
    pub controller: u32,
    pub state: u32,
    pub effect: &'a Effect,
    pub ago: f32,
    pub emitter: Option<u16>,
    pub fresh: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sources<'a> {
    pub playing: &'a [Playing<'a>],
    pub caught: &'a [Caught<'a>],
    pub controller: &'a [ControllerFire<'a>],
    pub reactions: &'a [Reaction],
    pub named: &'a [u16],
}

pub fn parse(b: &mut Bundle<'_>, m: Option<&Model>, r: &mut Report) -> EffectSet {
    format::parse(b, m, r)
}

pub fn check_events(e: &EffectSet, anims: &[Animation], c: &ControllerSet, r: &mut Report) {
    format::check_events(e, anims, c, r)
}

pub fn plan_moments(
    e: &mut EffectSet,
    anims: &[Animation],
    sounds: &[Vec<u8>],
    set: bool,
    r: &mut Report,
) {
    moments::plan(e, anims, sounds, set, r)
}

pub fn bind_locators(e: &mut EffectSet, anims: &[Animation], c: &ControllerSet) {
    format::bind_locators(e, anims, c);
}

pub fn size_playbacks(
    e: &mut EffectSet,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
) {
    let still = ControllerSet::default();
    let c = if solo.controllers { c } else { &still };
    e.playback_slots = events::playback_slots(e, anims, c, rs);
    e.edges = events::edges(e, anims, c, rs);
    let needs = events::emitter_runs(e, anims, c, rs, solo);
    let runs: Vec<usize> = needs
        .iter()
        .map(|need| need.likely.min(EFFECT_EMITTER_RUNS as usize))
        .collect();
    e.budget_runs = needs
        .iter()
        .zip(&runs)
        .map(|(need, &runs)| need.budget.min(runs))
        .collect();
    e.set_runs(&runs);
}

pub fn size_catch_up(e: &EffectSet, anims: &[Animation], c: &mut ControllerSet) {
    c.size_catch_up(
        anims,
        |effect| events::catch_up_keep(e, effect),
        |effect| events::catch_up_gates(e, effect),
        |animation| events::clip_catch_up(e, animation),
    );
}

pub fn catch_up_live(e: &EffectSet, effect: &Effect, ago: f32) -> bool {
    let keep = events::catch_up_keep(e, effect);
    ago <= keep.seconds || ago < keep.spacing
}

pub fn catch_up_fires(e: &EffectSet) -> usize {
    events::catch_up_fires(e)
}

pub fn prune_fires(e: &EffectSet, fires: &mut Vec<ControllerFire<'_>>) {
    events::prune_fires(e, fires)
}

pub fn check_runs(
    e: &EffectSet,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
    r: &mut Report,
) {
    events::check_runs(e, anims, c, rs, solo, r)
}

pub fn check_plays(
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
    at: &str,
    r: &mut Report,
) {
    let still = ControllerSet::default();
    let c = if solo.controllers { c } else { &still };
    events::check_plays(anims, c, rs, at, r)
}

pub fn check_budgets(e: &EffectSet, path: &str, r: &mut Report) {
    particles::check(e, path, r)
}

pub fn declarations(e: &EffectSet) -> WearerDeclarations {
    WearerDeclarations {
        effect_times: if e.is_empty() { 0 } else { last_step(e) + 1 },
        rings: e.rings.len(),
        ring_points: (0..e.rings.len())
            .map(|ring| geometry::ring_points(e, ring))
            .sum(),
        ring_turns: (0..e.rings.len())
            .map(|ring| geometry::ring_turns(e, ring))
            .sum(),
        ring_gates: (0..e.rings.len())
            .map(|ring| geometry::ring_gates(e, ring))
            .sum(),
        ..WearerDeclarations::default()
    }
}

pub fn step(
    e: &EffectSet,
    rec: &mut WearerRecord<'_>,
    input: &EffectInput<'_>,
    out: &mut Vec<Emission>,
) -> u32 {
    events::step(e, rec, input, &Sources::default(), out)
}

pub fn step_with(
    e: &EffectSet,
    rec: &mut WearerRecord<'_>,
    input: &EffectInput<'_>,
    sources: &Sources<'_>,
    out: &mut Vec<Emission>,
) -> u32 {
    events::step(e, rec, input, sources, out)
}

pub fn particle_at(d: &ParticleDef, spawn: &Spawn, age: f32, budget: &mut Budget) -> [f32; 3] {
    particles::particle_at(d, spawn, age, budget)
}
