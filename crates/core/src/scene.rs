//! A player and its cosmetics: numbered slots (slot number = texture id), clocks and sheets.
//! Each frame: `pose`, then `build` (2D, back-to-front) or `build_mesh` (3D quads for the game).

use std::array;
use std::fmt;
use std::mem;
use std::sync::Arc;

use crate::animation::{
    self, channel_offsets, has_timeline, Animation, Disabled, Loop, KEPT_DELAY_FLOATS,
};
use crate::asset::{
    self, reaction_span, CosmeticAsset, EmoteRules, KeptDelays, Kind, KEPT_REACTION_FLOATS,
    PET_PLACE_FLOATS,
};
use crate::attach::{
    compose, compose_at, game_part_index, owner_signal, rig_bone, rig_parts, GamePart,
    PlayerLocator, RigKind, OWNER_PREFIX,
};
use crate::build::{Buffers, Camera, Instance, Light};
use crate::bundle;
use crate::container::json_fits;
use crate::controller::{
    fresh_play, run_timelines, Anchor, Caught, ClipProps, Clips, ControllerSet, EffectEvent,
    EffectKind, Played, Scratch, Slots, CATCH_UP_GRACE_SECONDS,
};
use crate::cosmetic::{self, BodySlot};
use crate::cost::{self, AnimationRate, Detail, Features, Motion};
use crate::effects::geometry::{self, Locators, QuadSink, WearerEffects};
use crate::effects::{
    self, Budget, Caught as CaughtKeys, ControllerFire, EffectInput, EffectSet, Emission, EmitKind,
    Playing, Sources, View,
};
use crate::issues::{Level, Mode, Report};
use crate::java;
use crate::limits::{EMOTE_BLEND_SECONDS, EVENT_EFFECT_SECONDS};
use crate::math::Affine;
use crate::mesh::{Mesh, Skin};
use crate::molang::{self, Interner, Rules, Runtime, FRAME_FUEL};
use crate::player::{self, Arms, ElytraPose};
use crate::pose::{
    held_channels, hold_channels, inverse, restore_channels, world_transforms,
    world_transforms_resolved, BonePose, Pose,
};
use crate::reactions::{self, Lead, ReactionOut, ReactionRecord, Speed};
use crate::roles::{self, Placement, RoleInput};
use crate::signals::{
    jump_query, signal_id, signal_time_id, SignalKind, SignalState, EVENT_ATTACK_SWING,
    EVENT_EQUIP, JUMP_QUERY, JUMP_SIGNAL, SIGNALS, SIGNAL_COUNT, TIME_SUFFIX,
};
use crate::springs::{self, SpringInput};
use crate::wearer::{
    self, CosmeticSlot, DeclarationError, WearerDeclarations, WearerRecord, WearerTable,
    ONE_SHOT_EMPTY, SPRING_CLOCK, TIME_NEVER,
};
use crate::Model;

/// As the in-game client: a cosmetic's sheet advances every 4 ticks.
const COSMETIC_FRAME_SECONDS: f32 = 0.2;
const ANIMATION_FILE: &str = "animation.json";

pub const PREVIEW_WEARER: u64 = u64::MAX;
pub const PART_FLOATS: usize = 12;
pub const WEARER_INPUT_EXTRA: usize = SIGNAL_COUNT + 2;
pub const EFFECT_EVENT_INTS: usize = 6;
pub const EFFECT_PARTICLE: u32 = 0;
pub const EFFECT_SOUND: u32 = 1;
pub const EFFECT_VANILLA_PARTICLE: u32 = 2;
pub const EFFECT_EVENT_FLOATS: usize = 5;
pub const EFFECT_NO_CONTROLLER: u32 = u32::MAX;
pub const ROOT_FLOATS: usize = 12;
pub const PLAY_FLOATS: usize = 2;
pub const ROLE_FLOATS: usize = 5;
pub const HOLD_FLOATS: usize = 1;
pub const LOCATOR_FLOATS: usize = 12;
pub const COLORWAY_FLOATS: usize = 1;
pub const PART_DELTA_FLOATS: usize = 10;
pub const NO_COLORWAY: f32 = -1.0;
pub const EMOTE_STATUS_FLOATS: usize = 1;
pub const EMOTE_POSING: u32 = 1 << 0;
pub const EMOTE_ENDED: u32 = 1 << 1;
pub const EMOTE_CANCEL: u32 = 1 << 2;
pub const EMOTE_STOPPING: u32 = 1 << 3;
pub const EMOTE_CANCEL_WALKING: u32 = 1 << 4;
pub const EMOTE_CANCEL_SPRINTING: u32 = 1 << 5;
pub const EMOTE_CANCEL_CROUCHING: u32 = 1 << 6;
pub const EMOTE_CANCEL_FALLING: u32 = 1 << 7;
pub const EMOTE_CANCEL_ELYTRA_FLYING: u32 = 1 << 8;
pub const EMOTE_CANCEL_SWIMMING: u32 = 1 << 9;
pub const EMOTE_CANCEL_SLEEPING: u32 = 1 << 10;
pub const EMOTE_CANCEL_SWINGING: u32 = 1 << 11;
pub const EMOTE_HELD: u32 = 1 << 12;
pub const EMOTE_ROLE_VARIABLE: &str = "emote_role";
pub const EMOTE_FALLBACK_VARIABLE: &str = "emote_fallback";

const TIMELINE_FIRST_FRAME_SECONDS: f32 = 0.05;
const ANCHOR_SLACK: f32 = 2.0e-3;
const MAX_PLAYING: usize = effects::PLAYBACK_SLOTS;
const MAX_FIRES: usize = 64;
const MAX_NAMED_FIRES: usize = 256;
const MAX_EFFECT_TONES: usize = 64;
const NAMED_FIRE_SECONDS: f64 = 1.0;

const CONTROLLER_EVENT_FLOATS: [f32; EFFECT_EVENT_FLOATS] = [0.0, 0.0, 0.0, 1.0, 1.0];
const REST_DELTA: [f32; PART_DELTA_FLOATS] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0];

const IS_SNEAKING: usize = 0;
const IS_SPRINTING: usize = 1;
const IS_SWIMMING: usize = 2;
const IN_WATER: usize = 3;
const IS_GLIDING: usize = 8;
const IS_FALLING: usize = 10;
const ON_GROUND: usize = 11;
const IS_SLEEPING: usize = 13;
const GROUND_SPEED: usize = 17;
const VERTICAL_SPEED: usize = 18;
const IS_MOVING: usize = 16;
const TIME_OF_DAY: usize = 19;

const EMOTE_CLIP: usize = 0;
const EMOTE_AGE: usize = 1;
const EMOTE_BASE_WEIGHT: usize = 2;
const EMOTE_DELAY: usize = 3;
const EMOTE_PLACED: usize = 4;
const EMOTE_MARK: usize = 5;
const EMOTE_YAW: usize = 8;
const EMOTE_DIRECT: usize = 9;
const EMOTE_FROZEN: usize = 10;
const EMOTE_FROZEN_AT: usize = 11;
const PLACE_MARK: usize = 12;
const PLACE_WEIGHT: usize = 12;
const PLACE_DELAY: usize = 13;
const PLACE_CLIP: usize = 14;
const PLACE_START: usize = 15;
const PLACE_START_MATCH: f64 = 1.0e-3;
const FROZEN_SEEDING: f32 = 1.0;
const FROZEN_KEPT: f32 = 2.0;
const EMOTE_RESTART_SLACK: f32 = 1.0e-3;
const CARRIED_BY_BODY: [&str; 3] = ["head", "rightArm", "leftArm"];

pub const NEUTRAL_SIGNALS: [f32; SIGNAL_COUNT] = {
    let mut signals = [0.0; SIGNAL_COUNT];
    signals[ON_GROUND] = 1.0;
    signals[TIME_OF_DAY] = 0.25;
    signals
};

pub fn detail_from_input(value: f32) -> Detail {
    if value >= 0.5 {
        Detail::from_value((value + 0.5) as u8).unwrap_or(Detail::Minimal)
    } else {
        Detail::Full
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Physics {
    #[default]
    On,
    Own(u64),
    Off,
}

impl Physics {
    fn allows(self, wearer: u64) -> bool {
        match self {
            Physics::On => true,
            Physics::Own(own) => own == wearer,
            Physics::Off => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameShape {
    pub parts: usize,
    pub queries: usize,
}

impl FrameShape {
    pub fn signals(&self) -> usize {
        self.parts
            .saturating_mul(PART_FLOATS)
            .saturating_add(self.queries)
    }

    pub fn events(&self) -> usize {
        self.signals().saturating_add(SIGNAL_COUNT)
    }

    pub fn detail(&self) -> usize {
        self.events().saturating_add(1)
    }

    pub fn stride(&self) -> usize {
        self.signals().saturating_add(WEARER_INPUT_EXTRA)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameSections {
    pub parts: usize,
    pub queries: usize,
    pub root: bool,
    pub variables: usize,
    pub play: bool,
    pub role: bool,
    pub colorway: bool,
    pub part_deltas: bool,
    pub hold: bool,
}

impl From<FrameShape> for FrameSections {
    fn from(shape: FrameShape) -> Self {
        FrameSections {
            parts: shape.parts,
            queries: shape.queries,
            ..FrameSections::default()
        }
    }
}

impl FrameSections {
    pub fn shape(&self) -> FrameShape {
        FrameShape {
            parts: self.parts,
            queries: self.queries,
        }
    }

    pub fn root_at(&self) -> usize {
        self.shape().stride()
    }

    pub fn variables_at(&self) -> usize {
        self.root_at()
            .saturating_add(if self.root { ROOT_FLOATS } else { 0 })
    }

    pub fn play_at(&self) -> usize {
        self.variables_at().saturating_add(self.variables)
    }

    pub fn role_at(&self) -> usize {
        self.play_at()
            .saturating_add(if self.play { PLAY_FLOATS } else { 0 })
    }

    pub fn hold_at(&self) -> usize {
        self.role_at()
            .saturating_add(if self.role { ROLE_FLOATS } else { 0 })
    }

    pub fn stride(&self) -> usize {
        self.hold_at()
            .saturating_add(if self.hold { HOLD_FLOATS } else { 0 })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputLayout {
    pub bones: usize,
    pub runs: usize,
    pub locators: usize,
    pub colorway: bool,
    pub part_deltas: usize,
    pub emote: bool,
}

impl OutputLayout {
    pub fn runs_at(&self) -> usize {
        self.bones.saturating_mul(PART_FLOATS)
    }

    pub fn locators_at(&self) -> usize {
        self.runs_at().saturating_add(self.runs)
    }

    pub fn colorway_at(&self) -> usize {
        self.locators_at()
            .saturating_add(self.locators.saturating_mul(LOCATOR_FLOATS))
    }

    pub fn part_deltas_at(&self) -> usize {
        self.colorway_at()
            .saturating_add(if self.colorway { COLORWAY_FLOATS } else { 0 })
    }

    pub fn emote_at(&self) -> usize {
        self.part_deltas_at()
            .saturating_add(self.part_deltas.saturating_mul(PART_DELTA_FLOATS))
    }

    pub fn stride(&self) -> usize {
        self.emote_at()
            .saturating_add(if self.emote { EMOTE_STATUS_FLOATS } else { 0 })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    Input { held: usize, needed: usize },
    Output { held: usize, needed: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Input { held, needed } => {
                write!(f, "input holds {held} floats, the wearers need {needed}")
            }
            FrameError::Output { held, needed } => {
                write!(f, "out holds {held} floats, the frame needs {needed}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryKind {
    Host,
    Signal(usize),
    Derived,
}

struct HostQuery {
    name: String,
    value: f32,
    kind: QueryKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayInput {
    clip: usize,
    age: f64,
}

fn moment_seconds(asset: &CosmeticAsset, clip: usize) -> f32 {
    asset
        .effects
        .moments
        .iter()
        .filter(|plan| plan.clip.is_some_and(|c| usize::from(c) == clip))
        .map(|plan| plan.seconds)
        .reduce(f32::max)
        .unwrap_or(EVENT_EFFECT_SECONDS)
}

fn play_input(floats: &[f32]) -> Option<PlayInput> {
    let [clip, age] = *floats else {
        return None;
    };
    if !clip.is_finite() || clip < 0.0 {
        return None;
    }
    Some(PlayInput {
        clip: clip as usize,
        age: if age.is_finite() { age as f64 } else { 0.0 },
    })
}

struct WearerInput<'a> {
    index: usize,
    wearer: u64,
    raw: &'a [f32; SIGNAL_COUNT],
    events: u32,
    detail: Detail,
    root: Affine,
    variables: &'a [f32],
    play: Option<PlayInput>,
    role: Option<RoleInput>,
    held: bool,
}

struct WearerPass<'a> {
    wearer: u64,
    seed: u64,
    clock: f64,
    life_time: f32,
    raw: &'a [f32; SIGNAL_COUNT],
    events: u32,
    detail: Detail,
    features: Features,
    hold: Option<i64>,
    queries: &'a [HostQuery],
    variable_names: &'a [String],
    variables: &'a [f32],
    role: Option<RoleInput>,
    held: bool,
}

impl WearerPass<'_> {
    fn first_frame(&self) -> bool {
        self.features.animation == AnimationRate::FirstFrame
    }
}

struct Slot {
    model: Option<Model>,
    visible: bool,
    cull: bool,
    /// Without controllers only the file's first animation plays, as in game. Controllers choose among all of them.
    animations: Vec<Animation>,
    binds: Vec<Vec<Option<usize>>>,
    offsets: Vec<usize>,
    controllers: Option<ControllerSet>,
    scratch: Scratch,
    /// Per slot, so one cosmetic's `variable.` writes can't leak into another's.
    interner: Interner,
    runtime: Runtime,
    anim_time: Option<u16>,
    life_time: Option<u16>,
    /// Scene time the animation started at. `None` holds the rest pose.
    started: Option<f64>,
    pose: Pose,
    transforms: Vec<Affine>,
    frames: u32,
    frame_seconds: f32,
    scale: f32,
    /// The last `pose` was mid-animation, so the next one may differ.
    moving: bool,
    cosmetic: Option<CosmeticSlot>,
    seeds: [u64; 2],
    signal_queries: Vec<(u16, usize, bool)>,
    host_queries: Vec<Option<u16>>,
    values: Vec<f32>,
    asset: Option<Arc<CosmeticAsset>>,
    host_variables: Vec<Option<u16>>,
    reaction_floats: usize,
    reaction: ReactionOut,
    active: bool,
    timeline_clock: Option<usize>,
    anim_clock: Option<usize>,
    anchor: Option<usize>,
    playing: Vec<Played>,
    caught: Vec<Caught>,
    fuel: u32,
    fired: Vec<EffectEvent>,
    emote_at: Option<(usize, usize)>,
    emote: EmoteFrame,
    place_at: Option<(usize, usize)>,
    role_play: Option<f32>,
    role_delay: Option<f32>,
    placed: Option<(Affine, f32)>,
    props: Option<Props>,
    prop_of: Option<usize>,
    delays: KeptDelays,
    held: Vec<u32>,
    emote_bones: EmoteBones,
    externals: Vec<Ext>,
    prop_externals: Vec<PropExt>,
    locator_binds: Vec<LocatorBind>,
}

#[derive(Clone, Copy, Debug, Default)]
struct EmoteBones {
    root: Option<usize>,
    body: Option<usize>,
    carried: [Option<usize>; CARRIED_BY_BODY.len()],
}

impl EmoteBones {
    fn of(model: &Model) -> EmoteBones {
        EmoteBones {
            root: model.bone("root"),
            body: model.bone("body"),
            carried: CARRIED_BY_BODY.map(|name| model.bone(name)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Ext {
    own: Option<usize>,
    rig: Option<usize>,
    lower: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PropExt {
    own: Option<usize>,
    rig: Option<usize>,
    bone: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LocatorBind {
    own: Option<usize>,
    rig: Option<usize>,
    player_bone: Option<usize>,
    prop_bone: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NameStamp {
    epoch: u32,
    rig: RigKind,
    parts: usize,
    slots: usize,
    player: Option<usize>,
}

#[derive(Default)]
struct Names {
    stamp: Option<NameStamp>,
    rig_parts: Vec<Option<usize>>,
    part_bones: Vec<Option<usize>>,
    carried_parts: Vec<bool>,
    body_part: Option<usize>,
    head_part: Option<usize>,
    capsules: Vec<Ext>,
    locators: Vec<Ext>,
    wings: Vec<Option<usize>>,
}

struct Props {
    model: Model,
    binds: Vec<Vec<Option<usize>>>,
    pose: Pose,
    transforms: Vec<Affine>,
}

impl Props {
    fn of(asset: &CosmeticAsset) -> Option<Props> {
        let model = asset.model.as_ref().and_then(prop_model)?;
        Some(Props {
            binds: asset.animations.iter().map(|a| a.bind(&model)).collect(),
            pose: Pose::rest(&model),
            transforms: Vec::new(),
            model,
        })
    }
}

#[derive(Clone, Debug, Default)]
struct EmoteFrame {
    live: bool,
    clips: Vec<usize>,
    weight: f32,
    prop_weight: f32,
    status: u32,
    placement: Option<Placement>,
    rules: EmoteRules,
    animated: Vec<bool>,
    placed_root: Option<usize>,
    carried: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EmoteCue {
    clip: usize,
    age: f32,
    placement: Option<Placement>,
    fade: Option<f32>,
    cancel: u32,
    direct: bool,
    restart: bool,
    freeze: Freeze,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Freeze {
    Off,
    Begin { live: bool },
    Kept,
}

#[derive(Clone, Copy)]
enum Posed<'a> {
    Clip,
    Driven(&'a [Played], Option<f32>),
    Kept,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EmoteSteer {
    role: Option<u16>,
    fallback: bool,
    direct: bool,
}

impl Slot {
    fn new() -> Slot {
        let interner = Interner::default();
        let runtime = Runtime::new(&interner);

        Slot {
            model: None,
            visible: true,
            cull: true,
            animations: Vec::new(),
            binds: Vec::new(),
            offsets: Vec::new(),
            controllers: None,
            scratch: Scratch::default(),
            interner,
            runtime,
            anim_time: None,
            life_time: None,
            started: None,
            pose: Pose(Vec::new()),
            transforms: Vec::new(),
            frames: 1,
            frame_seconds: COSMETIC_FRAME_SECONDS,
            scale: 1.0,
            moving: false,
            cosmetic: None,
            seeds: [0; 2],
            signal_queries: Vec::new(),
            host_queries: Vec::new(),
            values: Vec::new(),
            asset: None,
            host_variables: Vec::new(),
            reaction_floats: 0,
            reaction: ReactionOut::default(),
            active: false,
            timeline_clock: None,
            anim_clock: None,
            anchor: None,
            playing: Vec::new(),
            caught: Vec::new(),
            fuel: FRAME_FUEL,
            fired: Vec::new(),
            emote_at: None,
            emote: EmoteFrame::default(),
            place_at: None,
            role_play: None,
            role_delay: None,
            placed: None,
            props: None,
            prop_of: None,
            delays: KeptDelays::default(),
            held: Vec::new(),
            emote_bones: EmoteBones::default(),
            externals: Vec::new(),
            prop_externals: Vec::new(),
            locator_binds: Vec::new(),
        }
    }

    fn set_model(&mut self, model: Model, cull: bool, frames: u32, frame_seconds: f32) {
        self.pose = Pose::rest(&model);
        self.binds = self.animations.iter().map(|a| a.bind(&model)).collect();

        self.emote_bones = EmoteBones::of(&model);
        self.model = Some(model);
        self.cull = cull;
        self.frames = frames.max(1);
        self.frame_seconds = frame_seconds;
    }

    fn bones(&self) -> usize {
        self.model.as_ref().map_or(0, |model| model.bones.len())
    }

    fn refresh_held(&mut self) -> usize {
        let own = (&self.binds[..], 0);
        self.held = match &self.props {
            Some(props) => held_channels(&self.animations, &[own, (&props.binds, self.bones())]),
            None => held_channels(&self.animations, &[own]),
        };
        self.held.len()
    }

    fn has_springs(&self) -> bool {
        self.model.is_some() && self.asset.as_ref().is_some_and(|a| !a.springs.is_empty())
    }

    fn animate(
        &mut self,
        pass: &WearerPass<'_>,
        play: Option<PlayInput>,
        budget: u32,
        table: &mut WearerTable,
        events: &mut Vec<EffectEvent>,
    ) -> ReactionOut {
        let Slot {
            animations,
            binds,
            offsets,
            controllers,
            scratch,
            cosmetic,
            interner,
            runtime,
            anim_time,
            life_time,
            started,
            pose,
            moving,
            seeds,
            signal_queries,
            host_queries,
            values,
            asset,
            host_variables,
            reaction_floats,
            active,
            timeline_clock,
            anim_clock,
            anchor: base_anchor,
            playing: played,
            caught,
            fuel,
            emote_at,
            emote,
            place_at,
            role_play,
            role_delay,
            props,
            delays: kept_delays,
            held: held_map,
            ..
        } = self;
        *role_play = None;
        *role_delay = None;
        events.clear();
        played.clear();
        caught.clear();
        *fuel = budget;
        *active = false;
        if let Some(props) = props.as_mut() {
            props.pose.reset();
        }
        let first_frame = pass.first_frame();
        let emote_asset = asset
            .as_deref()
            .filter(|a| a.kind == Kind::Emote && emote_at.is_some());
        emote.live = false;
        emote.weight = 0.0;
        emote.status = 0;
        emote.placement = None;
        emote.rules = emote_asset.and_then(|a| a.rules).unwrap_or_default();
        let slot_started = *started;
        let moment = asset.as_deref().filter(|a| a.is_moment_set());
        let selected = moment.is_some();
        let role_clip = pass
            .role
            .and_then(|r| asset.as_ref()?.roles.get(r.role as usize))
            .map(|role| role.animation as usize);
        let pet_role = role_clip.is_some() && asset.as_deref().is_some_and(|a| a.kind == Kind::Pet);
        let play = play.filter(|play| {
            pet_role
                || moment
                    .is_none_or(|asset| play.age <= f64::from(moment_seconds(asset, play.clip)))
        });
        let dormant = selected && play.is_none();
        let (mut clip, mut started) = match play {
            Some(play) => (role_clip.unwrap_or(play.clip), Some(pass.clock - play.age)),
            None if selected => (0, None),
            None => (0, slot_started),
        };
        let shared = play.is_some() && pet_role;
        *role_play = play.filter(|_| shared).map(|play| play.age as f32);
        let role_start = started.filter(|_| shared);
        let playing =
            controllers.is_none() || selected || started.is_some() || emote_asset.is_some();
        let record = cosmetic
            .filter(|_| playing)
            .and_then(|cosmetic| table.draw(cosmetic, pass.wearer, pass.raw, pass.events));
        let Some(record) = record else {
            pose.reset();
            *moving = false;
            return ReactionOut::default();
        };
        let WearerRecord {
            wearer,
            appear,
            times_reset,
            now,
            epoch,
            shifted,
            signals,
            variables,
            controller_states,
            controller_entered,
            one_shot_ids,
            one_shot_starts,
            effect_times,
            effect_floats,
            detail,
            held,
            disabled,
            held_pose,
            ..
        } = record;

        let fits = kept_delays.at + kept_delays.len() <= effect_floats.len();
        let (effect_floats, kept) = if fits && !kept_delays.is_empty() {
            effect_floats.split_at_mut(kept_delays.at)
        } else {
            (effect_floats, &mut [][..])
        };
        let (base_kept, kept) = kept.split_at_mut(kept_delays.base.min(kept.len()));
        let (reaction_kept, kept) = kept.split_at_mut(kept_delays.reactions.min(kept.len()));
        let (controller_kept, anim_times) =
            kept.split_at_mut(kept_delays.controllers.min(kept.len()));
        let (effect_floats, place_floats) = match *place_at {
            Some((at, _)) if at <= effect_floats.len() => effect_floats.split_at_mut(at),
            _ => (effect_floats, &mut [][..]),
        };
        let known_delay = role_start.and_then(|start| kept_role_delay(place_floats, clip, start));
        let (effect_floats, emote_floats) = match *emote_at {
            Some((at, _)) if at <= effect_floats.len() => effect_floats.split_at_mut(at),
            _ => (effect_floats, &mut [][..]),
        };
        if shifted != 0.0 {
            if let Some(at) = emote_floats.get_mut(EMOTE_FROZEN_AT) {
                *at -= shifted;
            }
        }
        let steer = emote_asset
            .zip(play)
            .map(|(asset, play)| emote_steer(asset, play, pass.role));
        let cue = match (emote_asset, *emote_at) {
            (Some(asset), Some((_, time))) => {
                let source = emote_source(asset, play, pass.role, slot_started, pass.clock);
                let stop = effect_times.get_mut(time);
                let cancel = emote_cancel(&emote.rules, signals, pass.events);
                let direct = steer.is_some_and(|s| s.direct);
                let held = pass.held && play.is_some();
                emote_cue(source, emote_floats, stop, now, cancel, direct, held)
            }
            _ => None,
        };
        let direct = cue.is_some_and(|cue| cue.direct);
        if emote_asset.is_some() {
            clip = cue.map_or(clip, |cue| cue.clip);
            started = cue.map(|cue| pass.clock - f64::from(cue.age));
        }

        *detail = pass.detail;
        let prop_bones = props.as_ref().map_or(0, |p| p.pose.0.len());
        let driven = emote_asset.is_some() && controllers.is_some() && !direct;
        let declared = asset.as_deref().map_or(&[][..], |a| a.reactions.as_slice());
        let split = (*reaction_floats).min(effect_floats.len());
        let (own, kept) = effect_floats.split_at_mut(split);
        let mut reaction = ReactionRecord {
            wearer,
            now,
            epoch,
            shifted,
            appear,
            equip: pass.events & EVENT_EQUIP != 0,
            times_reset,
            signals,
            one_shot_ids,
            one_shot_starts,
            floats: own,
            delays: reaction_kept,
        };
        let freeze = cue.map_or(Freeze::Off, |cue| cue.freeze);
        let sized = held_pose.len() == held_map.len();
        let reuse = sized
            && match freeze {
                Freeze::Kept => true,
                Freeze::Begin { live } => live && driven,
                Freeze::Off => false,
            };
        let seeding = freeze != Freeze::Off && !reuse;
        if reuse {
            if let Some(frozen) = emote_floats.get_mut(EMOTE_FROZEN) {
                *frozen = FROZEN_KEPT;
            }
            if let Some(at) = emote_floats.get_mut(EMOTE_FROZEN_AT) {
                let shift = now - *at;
                if driven && shift.is_finite() {
                    controller_entered.iter_mut().for_each(|e| *e += shift);
                }
                *at = now;
            }
            *held = None;
            let props = props
                .as_mut()
                .map_or(&mut [][..], |p| p.pose.0.as_mut_slice());
            restore_channels(held_map, held_pose, &mut pose.0, props);
            if !first_frame {
                timeline_step(*timeline_clock, effect_times, now);
            }
            if let Some(clock) = anim_clock.and_then(|at| effect_times.get_mut(at)) {
                *clock = now;
            }
            *active = true;
            reactions::held(declared, &mut reaction);
            emote.clips.clear();
            match controllers.as_ref().filter(|_| driven) {
                Some(set) => set.state_clips(controller_states, &mut emote.clips),
                None => emote.clips.push(clip),
            }
            finish_emote(cue, None, animations, emote_floats, emote, Posed::Kept);
            return kept_reaction(kept);
        }
        let holds = !animations.is_empty() || pose.0.len() + prop_bones == 0;
        let role_known = role_start.is_none() || known_delay.is_some();
        if pass.hold.is_some()
            && !driven
            && !seeding
            && holds
            && role_known
            && *held == pass.hold
            && sized
        {
            let props = props
                .as_mut()
                .map_or(&mut [][..], |p| p.pose.0.as_mut_slice());
            restore_channels(held_map, held_pose, &mut pose.0, props);
            *role_delay = known_delay;
            *active = true;
            if dormant {
                reactions::idle(declared, &mut reaction);
            } else {
                reactions::held(declared, &mut reaction);
            }
            finish_emote(cue, None, animations, emote_floats, emote, Posed::Clip);
            if dormant {
                keep_reaction(kept, ReactionOut::default());
                return ReactionOut::default();
            }
            return kept_reaction(kept);
        }

        let interner: &Interner = match asset.as_deref() {
            Some(asset) => &asset.interner,
            None => interner,
        };
        if host_queries.len() != pass.queries.len() {
            host_queries.clear();
            host_queries.extend(pass.queries.iter().map(|q| match q.kind {
                QueryKind::Host => interner.query_slot(&q.name),
                _ => None,
            }));
        }
        if host_variables.len() != pass.variable_names.len() {
            host_variables.clear();
            host_variables.extend(
                pass.variable_names
                    .iter()
                    .map(|name| interner.variable_slot(name)),
            );
        }
        for (&value, slot) in pass.variables.iter().zip(host_variables.iter()) {
            if let Some(variable) = slot.and_then(|slot| variables.get_mut(slot as usize)) {
                *variable = value;
            }
        }
        if let Some(steer) = steer {
            let role = steer.role.map_or(-1.0, f32::from);
            let fallback = if steer.fallback { 1.0 } else { 0.0 };
            for (name, value) in [
                (EMOTE_ROLE_VARIABLE, role),
                (EMOTE_FALLBACK_VARIABLE, fallback),
            ] {
                let slot = interner.variable_slot(name);
                if let Some(variable) = slot.and_then(|slot| variables.get_mut(slot as usize)) {
                    *variable = value;
                }
            }
        }
        values.clear();
        values.resize(interner.query_count(), f32::NAN);
        if let Some(value) = life_time.and_then(|slot| values.get_mut(slot as usize)) {
            *value = pass.life_time;
        }
        for &(slot, id, time) in signal_queries.iter() {
            if let Some(value) = values.get_mut(slot as usize) {
                *value = if time {
                    signals.time(id, now)
                } else {
                    signals.value(id)
                };
            }
        }
        for (query, slot) in pass.queries.iter().zip(host_queries.iter()) {
            if let Some(value) = slot.and_then(|slot| values.get_mut(slot as usize)) {
                *value = query.value;
            }
        }
        if first_frame {
            if let Some(value) = life_time.and_then(|slot| values.get_mut(slot as usize)) {
                *value = 0.0;
            }
        }

        let seed = wearer::seed(pass.seed, seeds[0], seeds[1]);
        let mut scope = runtime.wearer_frame(variables, seed, pass.clock);
        scope.set_fuel(budget);
        scope.set_queries(values);
        if appear {
            for animation in animations.iter() {
                animation.initialize(&mut scope);
            }
        }

        let steering = controllers.as_ref().filter(|_| !direct && !selected);
        if let (Some(set), Some(started), false) = (steering, started, shared) {
            pose.reset();
            if cue.is_some_and(|cue| cue.restart) {
                set.restart(controller_states);
            }
            let lead;
            {
                let mut clips = Clips {
                    animations,
                    binds,
                    offsets,
                    disabled,
                    first_frame,
                    props: props.as_mut().map(|p| ClipProps {
                        binds: &p.binds,
                        pose: &mut p.pose,
                    }),
                };
                lead = reactions::lead(declared, &mut reaction, &mut scope, &clips);
                let slots = Slots {
                    states: &mut *controller_states,
                    entered: &mut *controller_entered,
                    delays: controller_kept,
                    anim_times,
                    anim_clock: anim_clock.and_then(|at| effect_times.get_mut(at)),
                    now,
                    epoch,
                    appear,
                    times_reset,
                    speed: lead.as_ref().map_or_else(Speed::default, Lead::speed),
                    timelines: !seeding && timeline_clock.is_some(),
                };
                let origin = play.map(|_| (started - epoch) as f32);
                set.step_from(slots, origin, &mut scope, &mut clips, scratch, pose, events);
            }
            let mut clips = Clips {
                animations,
                binds,
                offsets,
                disabled,
                first_frame,
                props: None,
            };
            if let Some(slot) = *anim_time {
                scope.set_query(slot, (pass.clock - started) as f32);
            }
            if !seeding {
                played.extend_from_slice(scratch.played());
                caught.extend_from_slice(scratch.caught());
            }
            let led = lead.as_ref().map(|lead| (lead.speed(), played.len()));
            *moving = true;
            *active = true;
            let out = match lead {
                Some(lead) => reactions::overlay_led(
                    declared,
                    &mut reaction,
                    &mut scope,
                    &mut clips,
                    pose,
                    played,
                    lead,
                ),
                None => reactions::overlay(
                    declared,
                    &mut reaction,
                    &mut scope,
                    &mut clips,
                    pose,
                    played,
                ),
            };
            if !first_frame {
                if let Some(clock) = timeline_step(*timeline_clock, effect_times, now) {
                    if !seeding {
                        run_timelines(clips.animations, played, led, clock, &mut scope);
                    }
                }
            }
            *fuel = scope.fuel();
            if seeding {
                events.clear();
                freeze_pose(
                    held,
                    (held_map, held_pose),
                    pose,
                    props.as_ref(),
                    (&mut *emote_floats, now),
                );
            } else {
                let keep = driven;
                if let Some(at) = emote_floats.get_mut(EMOTE_FROZEN_AT).filter(|_| keep) {
                    *at = now;
                }
                hold_pose(
                    pass,
                    held,
                    (held_map, held_pose),
                    pose,
                    props.as_ref(),
                    keep,
                );
            }
            keep_reaction(kept, out);
            let driven = Posed::Driven(scratch.posed(), scratch.remaining());
            finish_emote(cue, None, clips.animations, emote_floats, emote, driven);
            return out;
        }

        let mut sampled_delay = None;
        let lead = if dormant {
            reactions::idle(declared, &mut reaction);
            None
        } else {
            let clips = Clips {
                animations,
                binds,
                offsets,
                disabled,
                first_frame,
                props: None,
            };
            reactions::lead(declared, &mut reaction, &mut scope, &clips)
        };
        let paced = lead.as_ref().zip(started).and_then(|(lead, started)| {
            let elapsed = (pass.clock - started) as f32;
            reactions::pace(declared, &mut reaction, &lead.speed(), elapsed)
        });
        let cue = cue.map(|cue| match paced {
            Some(paced) if cue.fade.is_none() => EmoteCue {
                age: paced.time,
                ..cue
            },
            _ => cue,
        });

        let posed = 'base: {
            let Some(animation) = animations.get(clip) else {
                pose.reset();
                *moving = false;
                break 'base false;
            };
            let time = started.and_then(|started| {
                let delays = animation.delays_kept(clip, &mut scope, started, base_kept);
                sampled_delay = Some(delays[0]);
                let age = (pass.clock - started) as f32;
                let elapsed = paced.map_or(age, |p| p.time);
                if seeding {
                    return Some(animation.length);
                }
                if !animation.events.is_empty() {
                    let (anchor, first) =
                        anchor_base(*base_anchor, effect_times, (started - epoch) as f32)
                            .map_or((None, false), |(anchor, first)| (Some(anchor), first));
                    let anchor = anchor.filter(|_| paced.is_none());
                    let mut play =
                        Played::new(clip, elapsed, age, paced.and_then(|p| p.from), delays)
                            .anchored(anchor);
                    play.first = first && play.fresh;
                    played.push(play);
                }
                let time = animation.local_time(elapsed, delays);
                time.map(|time| if first_frame { 0.0 } else { time })
            });
            let Some(time) = time else {
                pose.reset();
                // Played but still in its start delay.
                *moving = started.is_some();
                *held = None;
                break 'base false;
            };

            *moving = !first_frame && !animation.holds_at(time);
            if let Some(slot) = *anim_time {
                scope.set_query(slot, time);
            }
            animation.pre_animation(&mut scope);
            let weight = animation.weight_in(&mut scope).value as f32;
            let bind = binds.get(clip).map_or(&[][..], Vec::as_slice);
            let first = offsets.get(clip).copied().unwrap_or(0);
            animation.pose(
                bind,
                time,
                weight,
                &mut scope,
                Disabled::new(disabled, first),
                pose,
            );
            if let Some(props) = props.as_mut() {
                let bind = props.binds.get(clip).map_or(&[][..], Vec::as_slice);
                animation.pose(
                    bind,
                    time,
                    weight,
                    &mut scope,
                    Disabled::new(disabled, first),
                    &mut props.pose,
                );
            }
            true
        };
        if let (Some(start), Some(delay)) = (role_start, sampled_delay) {
            *role_delay = Some(delay);
            keep_role_delay(place_floats, clip, start, delay);
        }
        let mut clips = Clips {
            animations,
            binds,
            offsets,
            disabled,
            first_frame,
            props: None,
        };
        let led = lead
            .as_ref()
            .zip(paced)
            .map(|(lead, _)| (lead.speed(), played.len()));
        let out = match lead {
            _ if dormant => ReactionOut::default(),
            Some(lead) => reactions::overlay_led(
                declared,
                &mut reaction,
                &mut scope,
                &mut clips,
                pose,
                played,
                lead,
            ),
            None => reactions::overlay(
                declared,
                &mut reaction,
                &mut scope,
                &mut clips,
                pose,
                played,
            ),
        };
        if !first_frame {
            if let Some(clock) = timeline_step(*timeline_clock, effect_times, now) {
                if !seeding {
                    run_timelines(clips.animations, played, led, clock, &mut scope);
                }
            }
        }
        *fuel = scope.fuel();
        let reacting = !dormant
            && (out.held != 0
                || out.fired != 0
                || reaction.one_shot_ids.iter().any(|&id| id != ONE_SHOT_EMPTY));
        if reacting {
            *moving = true;
        }
        if seeding {
            events.clear();
            freeze_pose(
                held,
                (held_map, held_pose),
                pose,
                props.as_ref(),
                (&mut *emote_floats, now),
            );
        }
        if posed || reacting {
            *active = true;
            if !seeding {
                hold_pose(
                    pass,
                    held,
                    (held_map, held_pose),
                    pose,
                    props.as_ref(),
                    false,
                );
            }
        }
        keep_reaction(kept, out);
        finish_emote(
            cue,
            sampled_delay,
            clips.animations,
            emote_floats,
            emote,
            Posed::Clip,
        );
        out
    }

    fn place_pet(
        &self,
        root: &Affine,
        role: Option<RoleInput>,
        wearer: u64,
        table: &mut WearerTable,
    ) -> Option<(Affine, f32)> {
        let (at, time) = self.place_at?;
        let asset = self.asset.as_deref()?;
        let record = table.record(self.cosmetic?, wearer)?;
        let now = record.now;
        let floats = record.effect_floats.get_mut(at..at + PET_PLACE_FLOATS)?;
        let stop = record.effect_times.get_mut(time)?;
        let playing = self
            .role_play
            .zip(role)
            .and_then(|(age, role)| Some((age, roles::select(&asset.roles, &role)?.placement)));
        if let Some((age, placement)) = playing {
            let scale = roles::uniform_scale(root)?;
            let mark = roles::mark_root(&placement, scale);
            let local = inverse(root)?.mul(&mark);
            let delay = self.role_delay.unwrap_or(floats[PLACE_DELAY]);
            let weight = ((age - delay) / EMOTE_BLEND_SECONDS).clamp(0.0, 1.0);
            floats[..PLACE_MARK].copy_from_slice(&moved(&mark, root, -1.0).0);
            floats[PLACE_WEIGHT] = weight;
            *stop = TIME_NEVER;
            return Some((local, weight));
        }
        let base = floats[PLACE_WEIGHT];
        if base.is_nan() || base <= 0.0 {
            return None;
        }
        if !stop.is_finite() {
            *stop = now;
        }
        let fade = 1.0 - (now - *stop) / EMOTE_BLEND_SECONDS;
        if fade.is_nan() || fade <= 0.0 {
            floats[PLACE_WEIGHT] = 0.0;
            return None;
        }
        let mark = moved(&Affine(floats[..PLACE_MARK].try_into().ok()?), root, 1.0);
        Some((inverse(root)?.mul(&mark), base * fade.min(1.0)))
    }

    fn plays_pet_role(&self, role: Option<RoleInput>) -> bool {
        let Some(role) = role else {
            return false;
        };
        self.asset
            .as_deref()
            .is_some_and(|a| a.kind == Kind::Pet && a.roles.get(usize::from(role.role)).is_some())
    }

    fn plays_moments(&self) -> bool {
        self.model.is_some()
            && self
                .asset
                .as_deref()
                .is_some_and(CosmeticAsset::is_moment_set)
    }

    fn is_emote(&self) -> bool {
        self.emote_at.is_some() && self.asset.as_ref().is_some_and(|a| a.kind == Kind::Emote)
    }

    fn pose_emote(&mut self) {
        let Slot {
            emote,
            pose,
            model,
            binds,
            moving,
            props,
            emote_bones,
            ..
        } = self;
        emote.animated.clear();
        emote.carried.clear();
        emote.placed_root = None;
        if let Some(props) = props.as_mut() {
            if emote.live && emote.weight > 0.0 {
                weigh(&mut props.pose.0, emote.prop_weight);
            } else {
                hide(&mut props.pose.0);
            }
        }
        let Some(model) = model.as_ref().filter(|_| emote.live) else {
            return;
        };
        *moving = true;
        let w = emote.weight;
        weigh(&mut pose.0, w);
        emote.animated.resize(model.bones.len(), false);
        for &clip in &emote.clips {
            for &bone in binds.get(clip).into_iter().flatten().flatten() {
                if let Some(animated) = emote.animated.get_mut(bone) {
                    *animated = true;
                }
            }
        }
        if let (Some(placement), Some(root)) = (emote.placement, emote_bones.root) {
            let at = placement.root_position();
            let bone = &mut pose.0[root];
            for (axis, offset) in at.iter().enumerate() {
                bone.position[axis] += offset * w;
            }
            bone.rotation[1] += placement.yaw * w;
            if !emote.animated[root] {
                emote.animated[root] = true;
                emote.placed_root = Some(root);
            }
        }
        let body = emote_bones.body.filter(|&b| w > 0.0 && emote.animated[b]);
        if let Some(body) = body {
            emote
                .carried
                .extend(emote_bones.carried.iter().flatten().map(|&bone| (bone, body)));
        }
    }

    fn frame(&self, clock: f64, motion: Motion) -> u32 {
        if motion == Motion::Off {
            return 0;
        }
        let elapsed = (clock - self.started.unwrap_or(0.0)) as f32;
        (elapsed.max(0.0) / self.frame_seconds) as u32 % self.frames
    }
}

fn hold_pose(
    pass: &WearerPass<'_>,
    held: &mut Option<i64>,
    (map, held_pose): (&[u32], &mut [[f32; 3]]),
    pose: &Pose,
    props: Option<&Props>,
    keep: bool,
) {
    *held = pass.hold;
    let props = props.map_or(&[][..], |p| p.pose.0.as_slice());
    if (pass.hold.is_some() || keep) && held_pose.len() == map.len() {
        hold_channels(map, &pose.0, props, held_pose);
    }
}

fn freeze_pose(
    held: &mut Option<i64>,
    (map, held_pose): (&[u32], &mut [[f32; 3]]),
    pose: &Pose,
    props: Option<&Props>,
    (floats, now): (&mut [f32], f32),
) {
    *held = None;
    if held_pose.len() != map.len() {
        return;
    }
    let props = props.map_or(&[][..], |p| p.pose.0.as_slice());
    hold_channels(map, &pose.0, props, held_pose);
    if let Some(frozen) = floats.get_mut(EMOTE_FROZEN) {
        *frozen = FROZEN_KEPT;
    }
    if let Some(at) = floats.get_mut(EMOTE_FROZEN_AT) {
        *at = now;
    }
}

fn weigh(bones: &mut [BonePose], w: f32) {
    for bone in bones {
        for axis in 0..3 {
            bone.rotation[axis] *= w;
            bone.position[axis] *= w;
            bone.scale[axis] = 1.0 + (bone.scale[axis] - 1.0) * w;
        }
    }
}

fn hide(bones: &mut [BonePose]) {
    for bone in bones {
        bone.scale = [0.0; 3];
    }
}

pub(crate) fn prop_model(model: &Model) -> Option<Model> {
    let bones = &model.bones;
    let mut has: Vec<bool> = bones
        .iter()
        .map(|b| !b.cubes.is_empty() || !b.locators.is_empty())
        .collect();
    for i in (0..bones.len()).rev() {
        if let Some(p) = bones[i].parent {
            has[p] |= has[i];
        }
    }
    let mut roots: Vec<(usize, &'static str)> = bones
        .iter()
        .enumerate()
        .filter(|&(i, _)| has[i])
        .filter_map(|(i, b)| {
            let parent = b
                .parent_name
                .as_deref()
                .or_else(|| Some(bones[b.parent?].name.as_str()))?;
            Some((i, cosmetic::player_bone(parent)?))
        })
        .collect();
    if roots.is_empty() {
        let top = bones
            .iter()
            .position(|b| b.parent.is_none() && b.parent_name.is_none())
            .or((!bones.is_empty()).then_some(0))?;
        if !has[top] {
            return None;
        }
        roots.push((top, "root"));
    }
    let mut index: Vec<Option<usize>> = vec![None; bones.len()];
    let mut kept = Vec::new();
    for (i, bone) in bones.iter().enumerate() {
        let target = roots.iter().find(|(r, _)| *r == i).map(|&(_, t)| t);
        let parent = bone.parent.and_then(|p| index[p]);
        if target.is_none() && parent.is_none() {
            continue;
        }
        let mut bone = bone.clone();
        match target {
            Some(target) => {
                bone.parent = None;
                bone.parent_name = Some(target.to_string());
            }
            None => bone.parent = parent,
        }
        index[i] = Some(kept.len());
        kept.push(bone);
    }
    Some(Model {
        identifier: model.identifier.clone(),
        texture_size: model.texture_size,
        bones: kept,
    })
}

fn prop_pair(slots: &mut [Slot], owner: usize, target: usize) -> Option<(&Slot, &mut Slot)> {
    if owner < target {
        let (low, high) = slots.split_at_mut(target);
        Some((low.get(owner)?, high.first_mut()?))
    } else if owner > target {
        let (low, high) = slots.split_at_mut(owner.min(slots.len()));
        Some((high.first()?, low.get_mut(target)?))
    } else {
        None
    }
}

fn emote_source(
    asset: &CosmeticAsset,
    play: Option<PlayInput>,
    role: Option<RoleInput>,
    started: Option<f64>,
    clock: f64,
) -> Option<(usize, f32, Option<Placement>)> {
    match play {
        Some(play) => {
            let selected = role.and_then(|r| roles::select(&asset.roles, &r));
            let clip = selected
                .and_then(|s| s.animation)
                .map_or(play.clip, usize::from);
            Some((clip, play.age as f32, selected.map(|s| s.placement)))
        }
        None => started.map(|started| (0, (clock - started) as f32, None)),
    }
}

fn emote_steer(asset: &CosmeticAsset, play: PlayInput, role: Option<RoleInput>) -> EmoteSteer {
    let role = role
        .map(|r| r.role)
        .filter(|&r| asset.roles.get(usize::from(r)).is_some());
    let fallback = role.is_none()
        && asset.animations.len() > 1
        && asset.fallback.is_some_and(|f| usize::from(f) == play.clip);
    let unread = |name: &str| {
        asset
            .interner
            .variable_slot(name)
            .is_none_or(|slot| !asset.controllers.reads_variable(slot))
    };
    let direct = !asset.controllers.controllers().is_empty()
        && ((role.is_some() && unread(EMOTE_ROLE_VARIABLE))
            || (fallback && unread(EMOTE_FALLBACK_VARIABLE)));
    EmoteSteer {
        role,
        fallback,
        direct,
    }
}

fn emote_cancel(rules: &EmoteRules, signals: &SignalState, events: u32) -> u32 {
    let on = |id: usize| signals.value(id) >= 0.5;
    let moving = on(IS_MOVING);
    let checks = [
        (!rules.allow_walking && moving, EMOTE_CANCEL_WALKING),
        (
            !rules.allow_sprinting && moving && on(IS_SPRINTING),
            EMOTE_CANCEL_SPRINTING,
        ),
        (
            !rules.allow_crouching && on(IS_SNEAKING),
            EMOTE_CANCEL_CROUCHING,
        ),
        (!rules.allow_falling && on(IS_FALLING), EMOTE_CANCEL_FALLING),
        (
            !rules.allow_elytra_flying && on(IS_GLIDING),
            EMOTE_CANCEL_ELYTRA_FLYING,
        ),
        (
            !rules.allow_swimming && on(IS_SWIMMING),
            EMOTE_CANCEL_SWIMMING,
        ),
        (on(IS_SLEEPING), EMOTE_CANCEL_SLEEPING),
        (events & EVENT_ATTACK_SWING != 0, EMOTE_CANCEL_SWINGING),
    ];
    let bits = checks
        .iter()
        .filter(|(hit, _)| *hit)
        .fold(0, |bits, (_, bit)| bits | bit);
    if bits != 0 {
        bits | EMOTE_CANCEL
    } else {
        0
    }
}

fn emote_cue(
    source: Option<(usize, f32, Option<Placement>)>,
    floats: &mut [f32],
    stop: Option<&mut f32>,
    now: f32,
    cancel: u32,
    direct: bool,
    held: bool,
) -> Option<EmoteCue> {
    let floats = floats.get_mut(..asset::EMOTE_RECORD_FLOATS)?;
    let stop = stop?;
    let frozen = floats[EMOTE_FROZEN] >= 0.5;
    if let Some((clip, age, placement)) = source {
        let same = floats[EMOTE_CLIP] == clip as f32 + 1.0;
        let back = age + EMOTE_RESTART_SLACK < floats[EMOTE_AGE];
        let restart = if frozen {
            !same || (!held && back)
        } else {
            stop.is_finite() || !same || back
        };
        if restart {
            floats[EMOTE_FROZEN] = 0.0;
        }
        let frozen = frozen && !restart;
        floats[EMOTE_CLIP] = clip as f32 + 1.0;
        floats[EMOTE_AGE] = age;
        let (placed, at) = match placement {
            Some(p) => (1.0, [p.offset[0], p.offset[1], p.offset[2], p.yaw]),
            None => (0.0, [0.0; 4]),
        };
        floats[EMOTE_PLACED] = placed;
        floats[EMOTE_MARK..=EMOTE_YAW].copy_from_slice(&at);
        floats[EMOTE_DIRECT] = if direct { 1.0 } else { 0.0 };
        if held || !frozen {
            *stop = TIME_NEVER;
            let freeze = match (held, floats[EMOTE_FROZEN] >= FROZEN_KEPT) {
                (false, _) => Freeze::Off,
                (true, true) => Freeze::Kept,
                (true, false) => {
                    floats[EMOTE_FROZEN] = FROZEN_SEEDING;
                    Freeze::Begin {
                        live: !restart && !frozen,
                    }
                }
            };
            return Some(EmoteCue {
                clip,
                age,
                placement,
                fade: None,
                cancel,
                direct,
                restart,
                freeze,
            });
        }
    }
    let base = floats[EMOTE_BASE_WEIGHT];
    if floats[EMOTE_CLIP] < 1.0 || base.is_nan() || base <= 0.0 {
        if source.is_none() {
            floats[EMOTE_FROZEN] = 0.0;
        }
        return None;
    }
    if !stop.is_finite() {
        *stop = now;
    }
    let fade = 1.0 - (now - *stop) / EMOTE_BLEND_SECONDS;
    if fade.is_nan() || fade <= 0.0 {
        floats[EMOTE_BASE_WEIGHT] = 0.0;
        if source.is_none() {
            floats[EMOTE_CLIP] = 0.0;
            floats[EMOTE_FROZEN] = 0.0;
        }
        return None;
    }
    let freeze = if !frozen {
        Freeze::Off
    } else if floats[EMOTE_FROZEN] >= FROZEN_KEPT {
        Freeze::Kept
    } else {
        Freeze::Begin { live: false }
    };
    let placement = (floats[EMOTE_PLACED] >= 0.5).then(|| Placement {
        offset: [
            floats[EMOTE_MARK],
            floats[EMOTE_MARK + 1],
            floats[EMOTE_MARK + 2],
        ],
        yaw: floats[EMOTE_YAW],
    });
    Some(EmoteCue {
        clip: (floats[EMOTE_CLIP] - 1.0) as usize,
        age: floats[EMOTE_AGE],
        placement,
        fade: Some(fade.min(1.0)),
        cancel: 0,
        direct: floats[EMOTE_DIRECT] >= 0.5,
        restart: false,
        freeze,
    })
}

fn emote_weight(animation: &Animation, age: f32, delay: f32) -> (f32, bool) {
    let t = age - delay;
    let fade_in = (t / EMOTE_BLEND_SECONDS).clamp(0.0, 1.0);
    if animation.loop_mode != Loop::Once {
        return (fade_in, false);
    }
    let left = animation.length + EMOTE_BLEND_SECONDS - t;
    let fade_out = (left / EMOTE_BLEND_SECONDS).clamp(0.0, 1.0);
    (fade_in.min(fade_out), t >= animation.length)
}

fn moved(mark: &Affine, root: &Affine, sign: f32) -> Affine {
    let mut out = *mark;
    for at in [3, 7, 11] {
        out.0[at] += sign * root.0[at];
    }
    out
}

fn kept_role_delay(floats: &[f32], clip: usize, start: f64) -> Option<f32> {
    let floats = floats.get(..PET_PLACE_FLOATS)?;
    let kept = f64::from(floats[PLACE_START]) + f64::from(floats[PLACE_START + 1]);
    let same = floats[PLACE_CLIP] == (clip + 1) as f32 && (kept - start).abs() < PLACE_START_MATCH;
    same.then_some(floats[PLACE_DELAY])
}

fn keep_role_delay(floats: &mut [f32], clip: usize, start: f64, delay: f32) {
    let Some(floats) = floats.get_mut(..PET_PLACE_FLOATS) else {
        return;
    };
    let high = start as f32;
    floats[PLACE_DELAY] = delay;
    floats[PLACE_CLIP] = (clip + 1) as f32;
    floats[PLACE_START] = high;
    floats[PLACE_START + 1] = (start - f64::from(high)) as f32;
}

fn driven_weight(remaining: Option<f32>, age: f32, delay: f32) -> (f32, bool) {
    let fade_in = ((age - delay) / EMOTE_BLEND_SECONDS).clamp(0.0, 1.0);
    let Some(remaining) = remaining.filter(|r| !r.is_nan()) else {
        return (fade_in, false);
    };
    let fade_out = ((remaining + EMOTE_BLEND_SECONDS) / EMOTE_BLEND_SECONDS).clamp(0.0, 1.0);
    (fade_in.min(fade_out), remaining <= 0.0)
}

fn finish_emote(
    cue: Option<EmoteCue>,
    delay: Option<f32>,
    animations: &[Animation],
    floats: &mut [f32],
    frame: &mut EmoteFrame,
    posed: Posed<'_>,
) {
    let Some(cue) = cue else {
        return;
    };
    let animation = animations.get(cue.clip);
    if animation.is_none() && matches!(posed, Posed::Clip) {
        return;
    }
    if let (Some(delay), Some(slot)) = (delay, floats.get_mut(EMOTE_DELAY)) {
        *slot = delay;
    }
    let delay = floats.get(EMOTE_DELAY).copied().unwrap_or(0.0);
    let (live, ended) = match (posed, animation) {
        (Posed::Driven(posed, remaining), _) => {
            frame.clips.clear();
            for played in posed {
                if !frame.clips.contains(&played.clip) {
                    frame.clips.push(played.clip);
                }
            }
            driven_weight(remaining, cue.age, delay)
        }
        (Posed::Clip, Some(animation)) => {
            frame.clips.clear();
            frame.clips.push(cue.clip);
            emote_weight(animation, cue.age, delay)
        }
        (Posed::Kept, _) => (1.0, false),
        (Posed::Clip, None) => return,
    };
    let frozen = cue.freeze != Freeze::Off;
    let (live, ended) = if frozen { (1.0, false) } else { (live, ended) };
    let fade_in = if frozen {
        1.0
    } else {
        ((cue.age - delay) / EMOTE_BLEND_SECONDS).clamp(0.0, 1.0)
    };
    let weight = match cue.fade {
        None => {
            if let Some(base) = floats.get_mut(EMOTE_BASE_WEIGHT) {
                *base = live;
            }
            live
        }
        Some(fade) => floats.get(EMOTE_BASE_WEIGHT).copied().unwrap_or(0.0) * fade,
    };
    let mut status = cue.cancel;
    if weight > 0.0 {
        status |= EMOTE_POSING;
    }
    if cue.fade.is_some() {
        status |= EMOTE_STOPPING;
    } else if frozen {
        status |= EMOTE_HELD;
    } else if ended {
        status |= EMOTE_ENDED;
    }
    frame.live = true;
    frame.weight = weight;
    frame.prop_weight = if cue.fade.is_some() { 1.0 } else { fade_in };
    frame.status = status;
    frame.placement = cue.placement;
}

fn part_matrix(part: &java::Part) -> Affine {
    let [x, y, z] = part.rotation;
    Affine::translation(part.offset)
        .mul(&Affine::rot_z(z))
        .mul(&Affine::rot_y(y))
        .mul(&Affine::rot_x(x))
        .mul(&Affine::scale(part.scale))
}

fn part_fields(m: &Affine) -> java::Part {
    let a = &m.0;
    let column = |c: usize| libm::sqrtf(a[c] * a[c] + a[4 + c] * a[4 + c] + a[8 + c] * a[8 + c]);
    let scale = [column(0), column(1), column(2)];
    let offset = [a[3], a[7], a[11]];
    if scale.iter().any(|s| s.is_nan() || *s <= 1.0e-6) {
        return java::Part {
            offset,
            rotation: [0.0; 3],
            scale,
        };
    }
    let r00 = a[0] / scale[0];
    let r10 = a[4] / scale[0];
    let r20 = a[8] / scale[0];
    let r01 = a[1] / scale[1];
    let r11 = a[5] / scale[1];
    let r02 = a[2] / scale[2];
    let r12 = a[6] / scale[2];
    let z = libm::atan2f(r10, r00);
    let y = libm::atan2f(-r20, libm::sqrtf(r00 * r00 + r10 * r10));
    let (sz, cz) = (libm::sinf(z), libm::cosf(z));
    let x = libm::atan2f(sz * r02 - cz * r12, cz * r11 - sz * r01);
    java::Part {
        offset,
        rotation: [x, y, z],
        scale,
    }
}

fn emote_part(
    rest: [f32; 3],
    vanilla: &java::Part,
    posed: &BonePose,
    weight: f32,
    absolute: bool,
) -> (java::Part, java::Part) {
    let delta = java::part_delta(posed);
    let base = if absolute {
        let keep = 1.0 - weight;
        java::Part {
            offset: array::from_fn(|i| vanilla.offset[i] * keep + rest[i] * weight),
            rotation: vanilla.rotation.map(|r| r * keep),
            scale: vanilla.scale.map(|s| s * keep + weight),
        }
    } else {
        *vanilla
    };
    let last = java::Part {
        offset: array::from_fn(|i| base.offset[i] + delta.offset[i]),
        rotation: array::from_fn(|i| base.rotation[i] + delta.rotation[i]),
        scale: array::from_fn(|i| {
            if absolute {
                base.scale[i] + delta.scale[i] - 1.0
            } else {
                base.scale[i] * delta.scale[i]
            }
        }),
    };
    (base, last)
}

fn emote_delta(vanilla: &java::Part, last: &java::Part, weight: f32) -> [f32; PART_DELTA_FLOATS] {
    let mut out = REST_DELTA;
    for i in 0..3 {
        out[i] = last.offset[i] - vanilla.offset[i];
        out[3 + i] = last.rotation[i] - vanilla.rotation[i];
        out[6 + i] = if vanilla.scale[i] != 0.0 {
            last.scale[i] / vanilla.scale[i]
        } else {
            1.0
        };
    }
    out[9] = weight;
    out
}

fn kept_reaction(kept: &[f32]) -> ReactionOut {
    let Some(&[colorway, low, high]) = kept
        .get(..KEPT_REACTION_FLOATS)
        .and_then(|kept| <&[f32; KEPT_REACTION_FLOATS]>::try_from(kept).ok())
    else {
        return ReactionOut::default();
    };
    ReactionOut {
        colorway: (colorway >= 0.0).then_some(colorway as u16),
        held: low as u32 | (high as u32) << 16,
        fired: 0,
    }
}

fn keep_reaction(kept: &mut [f32], out: ReactionOut) {
    if let Some(kept) = kept.get_mut(..KEPT_REACTION_FLOATS) {
        kept[0] = out.colorway.map_or(-1.0, f32::from);
        kept[1] = (out.held & 0xffff) as f32;
        kept[2] = (out.held >> 16) as f32;
    }
}

fn timeline_step(clock: Option<usize>, times: &mut [f32], now: f32) -> Option<[f32; 2]> {
    let last = times.get_mut(clock?)?;
    let from = if last.is_finite() {
        last.min(now)
    } else {
        now - TIMELINE_FIRST_FRAME_SECONDS
    };
    *last = now;
    Some([from, now])
}

fn anchor_base(slot: Option<usize>, times: &mut [f32], at: f32) -> Option<(Anchor, bool)> {
    let stored = times.get_mut(slot?)?;
    if !at.is_finite() {
        return None;
    }
    let kept = *stored;
    if kept.is_finite() && (kept - at).abs() <= ANCHOR_SLACK {
        return Some((Anchor::at(kept), false));
    }
    *stored = at;
    let before = if kept.is_finite() { kept.max(at) } else { at };
    let anchor = Anchor {
        base: 0.0,
        at,
        before,
    };
    Some((anchor, !kept.is_finite() || !fresh_play((kept - at).abs())))
}

fn timeline_clock(e: &EffectSet, animations: &[Animation]) -> Option<usize> {
    if !has_timeline(animations) {
        return None;
    }
    Some(if e.is_empty() {
        0
    } else {
        effects::last_step(e) + 1
    })
}

fn gather<T: Copy, const N: usize>(mut items: impl Iterator<Item = T>) -> Option<([T; N], usize)> {
    let first = items.next()?;
    let mut out = [first; N];
    let mut count = 1;
    for item in items.take(N - 1) {
        out[count] = item;
        count += 1;
    }
    Some((out, count))
}

fn hold_tick(wearer: u64, detail: Detail, clock: f64) -> Option<i64> {
    match detail.features().animation {
        AnimationRate::EveryFrame => None,
        AnimationRate::FirstFrame | AnimationRate::Hz(0) => Some(0),
        AnimationRate::Hz(hz) => {
            let phase = (wearer::seed(wearer, 0, 0) >> 11) as f64 / (1u64 << 53) as f64;
            Some(libm::floor(clock * hz as f64 + phase) as i64)
        }
    }
}

fn signal_queries(interner: &Interner) -> Vec<(u16, usize, bool)> {
    let mut queries = Vec::new();
    for prefix in ["", OWNER_PREFIX] {
        for (suffix, time) in [("", false), (TIME_SUFFIX, true)] {
            if let Some(slot) = interner.query_slot(&format!("{prefix}{JUMP_QUERY}{suffix}")) {
                queries.push((slot, JUMP_SIGNAL, time));
            }
        }
    }
    for (id, (name, kind)) in SIGNALS.iter().enumerate() {
        for prefix in ["", OWNER_PREFIX] {
            if let Some(slot) = interner.query_slot(&format!("{prefix}{name}")) {
                queries.push((slot, id, false));
            }
            if *kind == SignalKind::Bool {
                if let Some(slot) = interner.query_slot(&format!("{prefix}{name}{TIME_SUFFIX}")) {
                    queries.push((slot, id, true));
                }
            }
        }
    }
    queries
}

fn query_kind(name: &str) -> QueryKind {
    let name = owner_signal(name).unwrap_or(name);
    match signal_id(name) {
        Some(id) => QueryKind::Signal(id),
        None if signal_time_id(name).is_some() || jump_query(name).is_some() => QueryKind::Derived,
        None => QueryKind::Host,
    }
}

fn declaration_error(error: DeclarationError) -> animation::Error {
    let message = match error {
        DeclarationError::TooManyVariables { declared, limit } => {
            format!("{declared} `variable.` names, the limit is {limit}")
        }
        DeclarationError::TooManyOneShots { declared, limit } => {
            format!("{declared} one-shots, the limit is {limit}")
        }
        DeclarationError::OverBudget { bytes, budget } => {
            format!("a wearer record of {bytes} bytes, the budget is {budget}")
        }
    };
    animation::Error::Molang(molang::Error(message))
}

fn event_bits(value: f32) -> u32 {
    if value.is_finite() && value > 0.0 {
        value as u32
    } else {
        0
    }
}

fn controller_error(report: &Report) -> Option<animation::Error> {
    let issue = report.issues().iter().find(|i| i.level == Level::Error)?;
    let message = format!("{} at {}", issue.code.as_str(), issue.path);
    Some(animation::Error::Molang(molang::Error(message)))
}

fn shown(event: &EffectEvent, features: Features) -> bool {
    features.effects && (event.kind != EffectKind::Particle || features.particles)
}

#[derive(Default)]
pub struct Scene {
    slots: Vec<Slot>,
    /// The game's posed player parts. Cosmetics follow these ahead of any player slot.
    game_parts: Vec<GamePart>,
    part_values: Vec<Option<([f32; 3], java::Part)>>,
    part_deltas: Vec<[f32; PART_DELTA_FLOATS]>,
    emote_status: u32,
    queries: Vec<HostQuery>,
    clock: f64,
    life_origin: Option<f64>,
    layout: u32,
    skin_layout: Option<u32>,
    wearers: WearerTable,
    events: Vec<EffectEvent>,
    effect_events: Vec<u32>,
    effect_event_floats: Vec<f32>,
    emissions: Vec<Emission>,
    motion: Motion,
    physics: Physics,
    rig: RigKind,
    player_slot: Option<usize>,
    variable_names: Vec<String>,
    locators: Vec<SceneLocator>,
    locators_layout: Option<u32>,
    player_locators: Vec<Affine>,
    model_locators: Vec<Affine>,
    capsule_frames: Vec<Affine>,
    effect_draws: Vec<EffectDraw>,
    effect_budgets: Vec<Budget>,
    wearer_fuel: u32,
    effect_frames: Vec<Affine>,
    effect_rings_settled: bool,
    effect_tones: Vec<EffectTones>,
    effect_tones_layout: Option<u32>,
    named_fires: Vec<NamedFire>,
    elytra: Option<Model>,
    wing_frames: Vec<Affine>,
    effect_paints: Vec<WearerPaint>,
    names: Names,
    names_epoch: u32,
    pub light: Light,
    pub buffers: Buffers,
    pub mesh: Mesh,
    pub skin: Skin,
    pub skin_transforms: Vec<f32>,
    pub skin_offsets: Vec<f32>,
}

impl Scene {
    fn slot(&mut self, index: usize) -> &mut Slot {
        if self.slots.len() <= index {
            self.slots.resize_with(index + 1, Slot::new);
        }
        &mut self.slots[index]
    }

    fn reshape(&mut self, index: usize) -> &mut Slot {
        self.layout = self.layout.wrapping_add(1);
        self.renamed();
        self.slot(index)
    }

    fn renamed(&mut self) {
        self.names_epoch = self.names_epoch.wrapping_add(1);
    }

    fn remodel(&mut self, index: usize, model: Model, cull: bool, frames: u32, seconds: f32) {
        if self.player_slot == Some(index) {
            self.player_slot = None;
        }
        let slot = self.reshape(index);
        slot.set_model(model, cull, frames, seconds);
        slot.prop_of = None;
        let (cosmetic, channels) = (slot.cosmetic, slot.refresh_held());
        if let Some(cosmetic) = cosmetic {
            self.wearers.set_held_channels(cosmetic, channels);
        }
    }

    pub fn load_player(&mut self, slot: usize, arms: Arms) {
        self.remodel(slot, player::model(arms), true, 1, COSMETIC_FRAME_SECONDS);
        self.player_slot = Some(slot);
    }

    /// Prepared as the in-game client does (`cosmetic::prepare`), drawn two-sided.
    pub fn load_cosmetic(
        &mut self,
        slot: usize,
        mut model: Model,
        body: BodySlot,
        anchor: Option<&str>,
        image_size: Option<[u32; 2]>,
    ) {
        let frames = cosmetic::prepare(&mut model, body, anchor, image_size);
        self.remodel(slot, model, false, frames, COSMETIC_FRAME_SECONDS);
    }

    /// Multi-frame textures come from `bundle::cape_sheet`.
    pub fn load_cape(&mut self, slot: usize, frames: u32, frame_seconds: f32) {
        self.remodel(slot, player::cape(), true, frames, frame_seconds);
    }

    /// Takes the cape's texture and sheet, as vanilla draws a cape onto the elytra.
    pub fn load_elytra(&mut self, slot: usize, frames: u32, frame_seconds: f32) {
        self.remodel(slot, player::elytra(), true, frames, frame_seconds);
    }

    /// Poses an elytra slot's wings. Other slots are left alone.
    pub fn set_elytra_pose(&mut self, slot: usize, pose: ElytraPose) {
        if let Some(model) = &mut self.reshape(slot).model {
            player::pose_elytra(model, pose);
        }
    }

    /// Uses the file's first animation, which stays at rest until `play`.
    pub fn load_animation(&mut self, slot: usize, json: &str) -> Result<(), animation::Error> {
        self.load_controllers(slot, json, &[], &[])
    }

    pub fn load_controllers(
        &mut self,
        slot: usize,
        json: &str,
        files: &[(&str, &[u8])],
        declared: &[&str],
    ) -> Result<(), animation::Error> {
        let mut guard = Report::new(Mode::Client);
        json_fits(json.as_bytes(), ANIMATION_FILE, &mut guard);
        if let Some(error) = controller_error(&guard) {
            return Err(error);
        }
        let mut interner = Interner::default();
        let mut animations = animation::parse(json, &mut interner)?;
        let mut controllers = None;
        if !files.is_empty() {
            let mut report = Report::new(Mode::Client);
            let loaded = bundle::controllers(files, &mut interner, Rules::Bedrock, &mut report);
            let mut set = ControllerSet::select(loaded, declared, &mut report);
            set.link(&animations, &interner, &mut report);
            if let Some(error) = controller_error(&report) {
                return Err(error);
            }
            controllers = Some(set).filter(|set| !set.controllers().is_empty());
        }
        if controllers.is_none() {
            animations.truncate(1);
        }
        let timeline = timeline_clock(&EffectSet::default(), &animations);
        let (offsets, channels) = channel_offsets(&animations);
        let delays = KeptDelays {
            at: 0,
            base: if animations.is_empty() {
                0
            } else {
                KEPT_DELAY_FLOATS
            },
            reactions: 0,
            controllers: controllers
                .as_ref()
                .map_or(0, ControllerSet::kept_delay_floats),
            anim_times: controllers
                .as_ref()
                .map_or(0, ControllerSet::anim_time_floats),
        };
        let mut times = timeline.map_or(0, |clock| clock + 1);
        let anim_clock = controllers
            .as_ref()
            .is_some_and(ControllerSet::needs_clock)
            .then(|| {
                times += 1;
                times - 1
            });
        let anchor = asset::keyed(&animations).then(|| {
            times += 1;
            times - 1
        });
        let seed_name = match (animations.first(), &controllers) {
            (Some(animation), _) => Some(animation.name.as_str()),
            (None, Some(set)) => Some(set.controllers()[0].name.as_str()),
            (None, None) => None,
        };
        let cosmetic = match seed_name {
            Some(_) => Some(
                self.wearers
                    .add_cosmetic(WearerDeclarations {
                        variables: interner.variable_count(),
                        controllers: controllers.as_ref().map_or(0, ControllerSet::record_slots),
                        channels,
                        effect_times: times,
                        effect_floats: delays.len(),
                        ..WearerDeclarations::default()
                    })
                    .map_err(declaration_error)?,
            ),
            None => None,
        };
        let seeds = [
            seed_name.map_or(0, wearer::seed_key),
            wearer::seed_key("animation"),
        ];

        let s = self.slot(slot);
        let old = mem::replace(&mut s.cosmetic, cosmetic);
        s.runtime = Runtime::new(&interner);
        s.anim_time = interner.query_slot("anim_time");
        s.life_time = interner.query_slot("life_time");
        s.signal_queries = signal_queries(&interner);
        s.host_queries.clear();
        s.host_variables.clear();
        s.asset = None;
        s.reaction_floats = 0;
        s.delays = delays;
        s.timeline_clock = timeline;
        s.anim_clock = anim_clock;
        s.anchor = anchor;
        s.emote_at = None;
        s.emote = EmoteFrame::default();
        s.place_at = None;
        s.placed = None;
        s.props = None;
        s.prop_of = None;
        s.seeds = seeds;
        s.interner = interner;
        s.binds = match &s.model {
            Some(model) => animations.iter().map(|a| a.bind(model)).collect(),
            None => Vec::new(),
        };
        s.animations = animations;
        s.offsets = offsets;
        s.controllers = controllers;
        s.started = None;
        let channels = s.refresh_held();

        if let Some(cosmetic) = cosmetic {
            self.wearers.set_held_channels(cosmetic, channels);
        }
        if let Some(old) = old {
            self.wearers.remove_cosmetic(old);
        }
        self.renamed();

        Ok(())
    }

    pub fn set_motion(&mut self, motion: Motion) {
        self.motion = motion;
    }

    pub fn motion(&self) -> Motion {
        self.motion
    }

    pub fn set_physics(&mut self, physics: Physics) {
        self.physics = physics;
    }

    pub fn physics(&self) -> Physics {
        self.physics
    }

    pub fn play(&mut self, slot: usize, at: f64) {
        self.slot(slot).started = Some(at);
    }

    pub fn stop(&mut self, slot: usize) {
        self.slot(slot).started = None;
    }

    /// About the bone it hangs from, as the in-game client scales a shoulder pet.
    pub fn set_scale(&mut self, slot: usize, scale: f32) {
        self.slot(slot).scale = scale;
    }

    pub fn set_visible(&mut self, slot: usize, visible: bool) {
        let s = self.slot(slot);
        if s.visible != visible {
            s.visible = visible;
            self.layout = self.layout.wrapping_add(1);
        }
    }

    pub fn clear(&mut self, slot: usize) {
        self.layout = self.layout.wrapping_add(1);
        self.renamed();
        if self.player_slot == Some(slot) {
            self.player_slot = None;
        }
        if let Some(s) = self.slots.get_mut(slot) {
            if let Some(cosmetic) = s.cosmetic.take() {
                self.wearers.remove_cosmetic(cosmetic);
            }
            *s = Slot::new();
        }
    }

    /// Applies to every slot from the next `pose` and overrides the core's `life_time`.
    pub fn set_query(&mut self, name: &str, value: f32) {
        let id = self.query_id(name);
        self.queries[id].value = value;
    }

    pub fn query_id(&mut self, name: &str) -> usize {
        match self.queries.iter().position(|q| q.name == name) {
            Some(id) => id,
            None => {
                let kind = query_kind(name);
                self.queries.push(HostQuery {
                    name: name.to_string(),
                    value: if kind == QueryKind::Host {
                        0.0
                    } else {
                        f32::NAN
                    },
                    kind,
                });
                self.queries.len() - 1
            }
        }
    }

    pub fn set_query_at(&mut self, id: usize, value: f32) {
        if let Some(entry) = self.queries.get_mut(id) {
            entry.value = value;
        }
    }

    /// Hidden slots too: a hidden player can still drive an emote and its cosmetics.
    pub fn pose(&mut self, seconds: f32) {
        self.pose_at(f64::from(seconds));
    }

    pub fn pose_at(&mut self, clock: f64) {
        self.begin(clock);
        let mut raw = NEUTRAL_SIGNALS;
        for query in &self.queries {
            if let QueryKind::Signal(id) = query.kind {
                if !query.value.is_nan() {
                    raw[id] = query.value;
                }
            }
        }
        self.run_wearer(&WearerInput {
            index: 0,
            wearer: PREVIEW_WEARER,
            raw: &raw,
            events: 0,
            detail: Detail::Full,
            root: Affine::IDENTITY,
            variables: &[],
            play: None,
            role: None,
            held: false,
        });
    }

    fn begin(&mut self, clock: f64) {
        let rewound = self.wearers.begin_frame(clock);
        self.clock = self.wearers.clock();
        self.refresh_names();
        if rewound || self.life_origin.is_none() {
            self.life_origin = Some(self.clock);
        }
        let now = self.clock;
        self.named_fires.retain_mut(|f| {
            if f.at.is_nan() {
                f.at = now;
            }
            now - f.at <= NAMED_FIRE_SECONDS
        });
        self.effect_events.clear();
        self.effect_event_floats.clear();
        self.effect_draws.clear();
        self.effect_frames.clear();
        self.effect_rings_settled = false;
        self.resolve_locators();
    }

    fn run_wearer(&mut self, input: &WearerInput<'_>) -> Affine {
        self.refresh_names();
        let root = self.pose_wearer(input);
        let input = &WearerInput { root, ..*input };
        self.transform();
        if let Some(first) = self.settle_springs(input) {
            self.transform_from(first);
        }
        if !self.locators.is_empty() || self.runs_effects() {
            self.pose_wings(input.raw);
        }
        self.step_effects(input);
        root
    }

    fn pose_wearer(&mut self, input: &WearerInput<'_>) -> Affine {
        let features = cost::features(input.detail, self.motion);
        let seed = self.wearers.seed_wearer(input.wearer);
        let pass = WearerPass {
            wearer: input.wearer,
            seed,
            clock: self.clock,
            life_time: (self.clock - self.life_origin.unwrap_or(self.clock)) as f32,
            raw: input.raw,
            events: input.events,
            detail: input.detail,
            features,
            hold: match features.animation {
                AnimationRate::FirstFrame => None,
                _ => hold_tick(seed, input.detail, self.clock),
            },
            queries: &self.queries,
            variable_names: &self.variable_names,
            variables: input.variables,
            role: input.role,
            held: input.held,
        };
        let player_slot = self.player_slot;
        let emote = player_slot
            .and_then(|at| self.slots.get(at))
            .is_some_and(|slot| slot.asset.as_ref().is_some_and(|a| a.kind == Kind::Emote));
        let moments = !emote && self.slots.iter().any(Slot::plays_moments);
        let mut left = FRAME_FUEL;
        for (number, slot) in self.slots.iter_mut().enumerate() {
            if slot.model.is_none() {
                slot.active = false;
                slot.reaction = ReactionOut::default();
                slot.placed = None;
                continue;
            }
            let target = if moments {
                slot.plays_moments()
            } else {
                player_slot == Some(number)
            };
            let play = input
                .play
                .filter(|_| target || slot.plays_pet_role(input.role));
            let wearers = &mut self.wearers;
            slot.reaction = slot.animate(&pass, play, left, wearers, &mut self.events);
            slot.placed = slot.place_pet(&input.root, input.role, input.wearer, wearers);
            left = slot.fuel;
            slot.pose_emote();
            slot.fired.clear();
            if slot.asset.as_ref().is_some_and(|a| !a.effects.is_empty()) {
                slot.fired.append(&mut self.events);
                continue;
            }
            let fresh = |e: &EffectEvent| e.ago <= 0.0 || e.fresh;
            for event in self
                .events
                .drain(..)
                .filter(|e| shown(e, features) && fresh(e))
            {
                let kind = match event.kind {
                    EffectKind::Particle => EFFECT_PARTICLE,
                    EffectKind::Sound => EFFECT_SOUND,
                };
                self.effect_events.extend_from_slice(&[
                    input.index as u32,
                    number as u32,
                    event.controller as u32,
                    event.state as u32,
                    kind,
                    event.effect as u32,
                ]);
                self.effect_event_floats
                    .extend_from_slice(&CONTROLLER_EVENT_FLOATS);
            }
        }
        self.wearer_fuel = left;
        self.pose_props();
        self.pose_emote_parts();
        self.slots
            .iter()
            .find_map(|slot| slot.placed.filter(|(_, weight)| *weight > 0.0))
            .map_or(input.root, |(local, weight)| {
                input.root.mul(&roles::partway(&local, weight))
            })
    }

    fn pose_props(&mut self) {
        for target in 0..self.slots.len() {
            let Some(owner) = self.slots[target].prop_of else {
                continue;
            };
            match prop_pair(&mut self.slots, owner, target) {
                Some((source, slot)) => {
                    let props = source
                        .props
                        .as_ref()
                        .filter(|p| p.pose.0.len() == slot.pose.0.len());
                    match props {
                        Some(props) => {
                            slot.pose.0.copy_from_slice(&props.pose.0);
                            slot.moving = source.moving;
                            slot.active = source.active;
                        }
                        None => hide(&mut slot.pose.0),
                    }
                }
                None => hide(&mut self.slots[target].pose.0),
            }
        }
    }

    fn pose_emote_parts(&mut self) {
        self.refresh_names();
        self.emote_status = 0;
        self.part_deltas.clear();
        let Some(slot) = self
            .player_slot
            .and_then(|i| self.slots.get(i))
            .filter(|s| s.is_emote())
        else {
            return;
        };
        if slot.model.is_none() {
            return;
        }
        let emote = &slot.emote;
        self.emote_status = emote.status;
        let weight = if emote.live { emote.weight } else { 0.0 };
        let rules = &emote.rules;
        let values = &self.part_values;
        let part_bones = &self.names.part_bones;
        let head_part = self.names.head_part;
        let own = |id: usize| {
            let (rest, vanilla) = values.get(id).copied().flatten()?;
            let bone = part_bones.get(id).copied().flatten()?;
            if weight <= 0.0 || !emote.animated.get(bone).copied().unwrap_or(false) {
                return None;
            }
            let absolute = emote.placed_root != Some(bone)
                && (!rules.allow_vanilla_pose || (head_part == Some(id) && rules.absolute_head));
            Some(emote_part(
                rest,
                &vanilla,
                &slot.pose.0[bone],
                weight,
                absolute,
            ))
        };
        let carry = self
            .names
            .body_part
            .filter(|&id| self.game_parts[id].active)
            .filter(|_| !emote.carried.is_empty())
            .and_then(own)
            .and_then(|(base, last)| Some(part_matrix(&last).mul(&inverse(&part_matrix(&base))?)));
        self.part_deltas.resize(self.game_parts.len(), REST_DELTA);
        for (id, game) in self.game_parts.iter_mut().enumerate() {
            let Some((rest, vanilla)) = values.get(id).copied().flatten() else {
                continue;
            };
            if !game.active {
                continue;
            }
            let posed = own(id).map(|(_, last)| last);
            let carried = carry.filter(|_| self.names.carried_parts[id]);
            let last = match (posed, carried) {
                (posed, Some(carry)) => {
                    part_fields(&carry.mul(&part_matrix(&posed.unwrap_or(vanilla))))
                }
                (Some(posed), None) => posed,
                (None, None) => {
                    game.transform = java::part_transform(rest, &vanilla);
                    continue;
                }
            };
            game.transform = java::part_transform(rest, &last);
            self.part_deltas[id] = emote_delta(&vanilla, &last, weight);
        }
    }

    fn settle_springs(&mut self, input: &WearerInput<'_>) -> Option<usize> {
        let features = cost::features(input.detail, self.motion);
        let simulate =
            input.detail == Detail::Full && features.springs() && self.physics.allows(input.wearer);
        let capsules = springs::capsules(self.rig);
        if simulate && self.slots.iter().any(Slot::has_springs) {
            let mut frames = mem::take(&mut self.capsule_frames);
            frames.clear();
            let fresh = self.names_fresh();
            frames.extend(capsules.iter().enumerate().map(|(n, c)| {
                match self.names.capsules.get(n) {
                    Some(&ext) if fresh => self.ext_world(ext, c.bone),
                    _ => self.rig_bone(c.bone),
                }
            }));
            self.capsule_frames = frames;
        }
        let mut first = None;
        for (number, slot) in self.slots.iter_mut().enumerate() {
            if !slot.has_springs() {
                continue;
            }
            let Slot {
                asset,
                cosmetic,
                transforms,
                pose,
                ..
            } = slot;
            let (Some(asset), Some(cosmetic)) = (asset.as_deref(), *cosmetic) else {
                continue;
            };
            let Some(record) = self.wearers.record(cosmetic, input.wearer) else {
                continue;
            };
            if !simulate {
                if let Some(clock) = record.spring_header.get_mut(SPRING_CLOCK) {
                    *clock = TIME_NEVER;
                }
                continue;
            }
            let spring_input = SpringInput {
                world: transforms,
                root: &input.root,
                capsules,
                capsule_frames: &self.capsule_frames,
                in_water: record.signals.value(IN_WATER) >= 0.5,
                scale: features.spring_scale,
                now: record.now,
            };
            springs::settle(
                &asset.springs,
                record.spring_header,
                record.spring_bones,
                &spring_input,
                pose,
            );
            first.get_or_insert(number);
        }
        first
    }

    fn step_effects(&mut self, input: &WearerInput<'_>) {
        if !self.runs_effects() {
            return;
        }
        let features = cost::features(input.detail, self.motion);
        let seed_wearer = self.wearers.seed_wearer(input.wearer);
        let mut player_locators = mem::take(&mut self.player_locators);
        player_locators.clear();
        player_locators.extend(
            PlayerLocator::ALL
                .iter()
                .map(|&l| self.player_locator_world(l)),
        );
        self.player_locators = player_locators;
        let mut left = self.wearer_fuel;
        let group = match self.effect_draws.last() {
            Some(last) if last.wearer == input.wearer => last.budget,
            _ => self.effect_draws.len(),
        };
        for (number, slot) in self.slots.iter_mut().enumerate() {
            let Slot {
                asset,
                model,
                cosmetic,
                transforms,
                reaction,
                reaction_floats,
                seeds,
                animations,
                controllers,
                playing,
                caught,
                fired,
                props,
                locator_binds,
                ..
            } = slot;
            let (Some(asset), Some(model), Some(cosmetic)) = (asset.as_deref(), model, *cosmetic)
            else {
                continue;
            };
            if asset.effects.is_empty() {
                continue;
            }
            let Some(mut record) = self.wearers.record(cosmetic, input.wearer) else {
                continue;
            };
            let split = reaction_span(*reaction_floats).min(record.effect_floats.len());
            record.effect_floats = mem::take(&mut record.effect_floats).split_at_mut(split).1;
            self.model_locators.clear();
            match (props.as_ref(), asset.model.as_ref()) {
                (Some(props), Some(authored)) => {
                    let (rig, game, rig_parts) =
                        (self.rig, &self.game_parts, &self.names.rig_parts);
                    self.model_locators.extend(
                        authored.locators().enumerate().map(|(n, (_, locator))| {
                            let bind = locator_binds.get(n).copied().unwrap_or_default();
                            let player = bind.player_bone.and_then(|bone| {
                                compose_at(rig, game, rig_parts, bind.own, bind.rig)
                                    .map(|(tf, _)| tf)
                                    .or_else(|| transforms.get(bone).copied())
                            });
                            let prop = || props.transforms.get(bind.prop_bone?).copied();
                            player
                                .or_else(prop)
                                .map_or(Affine::IDENTITY, |tf| locator.world(&tf))
                        }),
                    );
                }
                _ => self
                    .model_locators
                    .extend(model.locators().map(|(bone, locator)| {
                        transforms
                            .get(bone)
                            .map_or(Affine::IDENTITY, |tf| locator.world(tf))
                    })),
            }
            let seed = wearer::seed(seed_wearer, seeds[0], wearer::seed_key("effects"));
            let seed_epoch = if seed_wearer == input.wearer {
                record.epoch
            } else {
                record.epoch - record.born
            };
            let effect_input = EffectInput {
                model_locators: &self.model_locators,
                player_locators: &self.player_locators,
                root: &input.root,
                features,
                triggers: reaction,
                seed,
                seed_epoch,
                fuel: left,
            };
            let clips = gather::<_, MAX_PLAYING>(playing.iter().filter_map(|p| {
                Some(Playing {
                    animation: animations.get(p.clip).filter(|a| effects::effect_keys(a))?,
                    clip: p.clip,
                    elapsed: p.elapsed,
                    age: p.age,
                    from: p.from,
                    delays: p.delays,
                    anchor: p.anchor,
                    late: p.late,
                    fresh: p.fresh,
                    first: p.first,
                    reaction: p.reaction,
                })
            }));
            let caught = gather::<_, MAX_PLAYING>(caught.iter().filter_map(|c| {
                Some(CaughtKeys {
                    animation: animations.get(c.clip).filter(|a| effects::effect_keys(a))?,
                    clip: c.clip,
                    age: c.age,
                    from: c.from,
                    to: c.to,
                    delays: c.delays,
                    ago: c.ago,
                    fresh: c.fresh,
                })
            }));
            let named = gather::<_, MAX_FIRES>(
                self.named_fires
                    .iter()
                    .filter(|f| f.wearer == input.wearer && f.slot == number)
                    .map(|f| f.id),
            );
            let named = named.as_ref().map_or(&[][..], |(n, count)| &n[..*count]);
            let live = |event: &&EffectEvent| {
                event.ago <= 0.0
                    || event.fresh
                    || controllers
                        .as_ref()
                        .and_then(|c| c.effect(event))
                        .is_some_and(|effect| {
                            effects::catch_up_live(&asset.effects, effect, event.ago)
                        })
            };
            let count = fired.iter().filter(live).count();
            let fire = |event: &EffectEvent| {
                Some(ControllerFire {
                    controller: event.controller as u32,
                    state: event.state as u32,
                    effect: controllers.as_ref()?.effect(event)?,
                    ago: event.ago,
                    emitter: event.emitter,
                    fresh: event.fresh,
                })
            };
            let few;
            let mut many: Vec<ControllerFire<'_>>;
            let fires = if count <= MAX_FIRES {
                few = gather::<_, MAX_FIRES>(fired.iter().filter(live).filter_map(fire));
                few.as_ref().map_or(&[][..], |(f, n)| &f[..*n])
            } else {
                many = fired.iter().filter(live).filter_map(fire).collect();
                let bound = effects::catch_up_fires(&asset.effects).max(MAX_FIRES);
                if many.len() > bound {
                    effects::prune_fires(&asset.effects, &mut many);
                    let older = many.len().saturating_sub(bound);
                    many.drain(..older);
                }
                &many[..]
            };
            let sources = Sources {
                playing: clips.as_ref().map_or(&[][..], |(c, n)| &c[..*n]),
                caught: caught.as_ref().map_or(&[][..], |(c, n)| &c[..*n]),
                controller: fires,
                reactions: &asset.reactions,
                named,
            };
            self.emissions.clear();
            left = effects::step_with(
                &asset.effects,
                &mut record,
                &effect_input,
                &sources,
                &mut self.emissions,
            );
            if !features.trails {
                geometry::clear_rings(&asset.effects, record.rings);
                *record.sampled = TIME_NEVER;
            }
            for emission in self.emissions.iter().filter(|e| emitted(e, features)) {
                let kind = match emission.kind {
                    EmitKind::Particle => EFFECT_PARTICLE,
                    EmitKind::Sound => EFFECT_SOUND,
                    EmitKind::VanillaParticle => EFFECT_VANILLA_PARTICLE,
                };
                self.effect_events.extend_from_slice(&[
                    input.index as u32,
                    number as u32,
                    emission.controller,
                    emission.source,
                    kind,
                    emission.effect as u32,
                ]);
                let [x, y, z] = emission.at;
                self.effect_event_floats.extend_from_slice(&[
                    x,
                    y,
                    z,
                    emission.volume,
                    emission.pitch,
                ]);
            }
            if !features.effects {
                continue;
            }
            let frames = self.effect_frames.len();
            let root = input.root;
            let paint = self
                .effect_paints
                .iter()
                .position(|p| p.wearer == input.wearer);
            self.effect_frames
                .extend(self.player_locators.iter().map(|tf| root.mul(tf)));
            self.effect_frames
                .extend(self.model_locators.iter().map(|tf| root.mul(tf)));
            self.effect_draws.push(EffectDraw {
                slot: number,
                wearer: input.wearer,
                cosmetic,
                seed,
                seed_epoch,
                fuel: left,
                budget: group,
                features,
                colorway: reaction
                    .colorway
                    .map(usize::from)
                    .or_else(|| paint.and_then(|i| self.effect_paints[i].colorway))
                    .filter(|&c| c < asset.palette.colorways.len()),
                paint,
                root,
                frames,
                model: self.model_locators.len(),
            });
        }
        self.named_fires.retain(|f| f.wearer != input.wearer);
    }

    pub fn set_effect_colors(
        &mut self,
        wearer: u64,
        colorway: Option<usize>,
        custom: &[(u8, u32)],
    ) {
        self.effect_paints.retain(|p| p.wearer != wearer);
        if colorway.is_some() || !custom.is_empty() {
            self.effect_paints.push(WearerPaint {
                wearer,
                colorway,
                custom: custom.to_vec(),
            });
        }
    }

    pub fn fire_named(&mut self, wearer: u64, name: &str) -> bool {
        let mut found = false;
        for (number, slot) in self.slots.iter().enumerate() {
            let Some(id) = slot.asset.as_ref().and_then(|a| a.effects.named_id(name)) else {
                continue;
            };
            found = true;
            if self.named_fires.len() < MAX_NAMED_FIRES {
                self.named_fires.push(NamedFire {
                    wearer,
                    slot: number,
                    id,
                    at: f64::NAN,
                });
            }
        }
        found
    }

    pub fn effect_events(&self) -> &[u32] {
        &self.effect_events
    }

    pub fn effect_event_floats(&self) -> &[f32] {
        &self.effect_event_floats
    }

    fn runs_effects(&self) -> bool {
        self.slots
            .iter()
            .any(|s| s.asset.as_ref().is_some_and(|a| !a.effects.is_empty()))
    }

    pub fn has_effects(&self) -> bool {
        self.slots.iter().any(|s| {
            s.asset
                .as_ref()
                .is_some_and(|a| !a.effects.is_empty() || a.effects.has_moments())
        })
    }

    pub fn effect_geometry(&mut self, view: &View, out: &mut [f32]) -> Result<usize, FrameError> {
        self.sample_effect_rings(view);
        self.refresh_effect_tones();
        let mut sink = QuadSink::new(out);
        let mut budgets = mem::take(&mut self.effect_budgets);
        budgets.clear();
        budgets.extend(self.effect_draws.iter().map(|d| Budget::new(d.fuel)));
        for draw in &self.effect_draws {
            budgets[draw.budget] = Budget::new(draw.fuel);
        }
        let mut base = 0usize;
        for (number, slot) in self.slots.iter().enumerate() {
            let Some(asset) = slot.asset.as_deref() else {
                continue;
            };
            let textures = geometry::texture_count(&asset.effects);
            for local in 0..textures {
                let texture = u32::try_from(base + local).unwrap_or(u32::MAX);
                for draw in self.effect_draws.iter().filter(|d| d.slot == number) {
                    let budget = &mut budgets[draw.budget];
                    let Some(record) = self.wearers.record(draw.cosmetic, draw.wearer) else {
                        continue;
                    };
                    let tones = draw_tones(&self.effect_tones, &self.effect_paints, draw);
                    let frames = &self.effect_frames[draw.frames..];
                    let (player, model) = frames.split_at(PlayerLocator::COUNT.min(frames.len()));
                    let w = WearerEffects {
                        locators: Locators {
                            model: &model[..draw.model.min(model.len())],
                            player,
                            root: draw.root,
                        },
                        times: record.effect_times,
                        rings: record.rings,
                        signals: record.signals,
                        now: record.now,
                        epoch: record.epoch,
                        seen: ((record.born - record.epoch) as f32 - CATCH_UP_GRACE_SECONDS)
                            .max(*record.sampled),
                        seed_epoch: draw.seed_epoch,
                        seed: draw.seed,
                        tones,
                        features: draw.features,
                    };
                    geometry::draw(&asset.effects, local, &w, view, texture, &mut sink, budget);
                }
            }
            base += textures;
        }
        self.effect_budgets = budgets;
        if sink.fits() {
            Ok(sink.count())
        } else {
            Err(FrameError::Output {
                held: sink.capacity(),
                needed: sink.needed(),
            })
        }
    }

    pub fn effect_texture(&self, id: usize) -> Option<(usize, &str)> {
        let mut base = 0usize;
        for (number, slot) in self.slots.iter().enumerate() {
            let Some(asset) = slot.asset.as_deref() else {
                continue;
            };
            let textures = geometry::texture_count(&asset.effects);
            if id < base + textures {
                return geometry::texture_path(&asset.effects, id - base).map(|p| (number, p));
            }
            base += textures;
        }
        None
    }

    pub fn effect_texture_count(&self) -> usize {
        self.slots
            .iter()
            .filter_map(|s| s.asset.as_deref())
            .map(|a| geometry::texture_count(&a.effects))
            .sum()
    }

    pub fn sample_effect_rings(&mut self, view: &View) {
        if mem::replace(&mut self.effect_rings_settled, true) {
            return;
        }
        let shift = view.origin_shift;
        if shift != [0.0; 3] && shift.iter().all(|s| s.is_finite()) {
            for slot in &self.slots {
                let (Some(asset), Some(cosmetic)) = (slot.asset.as_deref(), slot.cosmetic) else {
                    continue;
                };
                self.wearers.each_record(cosmetic, |record| {
                    geometry::rebase_rings(&asset.effects, record.rings, shift);
                });
            }
        }
        for draw in &self.effect_draws {
            let Some(asset) = self.slots.get(draw.slot).and_then(|s| s.asset.as_deref()) else {
                continue;
            };
            let Some(record) = self.wearers.record(draw.cosmetic, draw.wearer) else {
                continue;
            };
            if !(draw.features.trails || draw.features.particles) {
                continue;
            }
            let frames = &self.effect_frames[draw.frames..];
            let (player, model) = frames.split_at(PlayerLocator::COUNT.min(frames.len()));
            let locators = Locators {
                model: &model[..draw.model.min(model.len())],
                player,
                root: draw.root,
            };
            let restarted = geometry::sample_rings(
                &asset.effects,
                record.rings,
                &locators,
                record.signals,
                record.now,
            );
            if restarted {
                *record.sampled = record.now - CATCH_UP_GRACE_SECONDS;
            }
        }
    }

    fn refresh_effect_tones(&mut self) {
        if self.effect_tones_layout != Some(self.layout)
            || self.effect_tones.len() > MAX_EFFECT_TONES
        {
            self.effect_tones.clear();
            self.effect_tones_layout = Some(self.layout);
        }
        for draw in &self.effect_draws {
            let custom = paint_custom(&self.effect_paints, draw);
            if find_tones(&self.effect_tones, draw, custom).is_some() {
                continue;
            }
            let Some(asset) = self.slots.get(draw.slot).and_then(|s| s.asset.as_deref()) else {
                continue;
            };
            self.effect_tones.push(EffectTones {
                slot: draw.slot,
                colorway: draw.colorway,
                custom: custom.to_vec(),
                tones: geometry::palette_tones_with(&asset.palette, draw.colorway, custom),
            });
        }
    }

    pub fn effect_event_count(&self) -> usize {
        self.effect_events.len() / EFFECT_EVENT_INTS
    }

    pub fn controller_state(
        &mut self,
        slot: usize,
        wearer: u64,
        controller: usize,
    ) -> Option<(&str, f32)> {
        let s = self.slots.get(slot)?;
        let record = self.wearers.record(s.cosmetic?, wearer)?;
        s.controllers
            .as_ref()?
            .controller_state(&record, controller)
    }

    pub fn frame_wearers(
        &mut self,
        clock: f64,
        wearers: &[u64],
        shape: FrameShape,
        input: &[f32],
        to_host: &Affine,
        out: &mut [f32],
    ) -> Result<usize, FrameError> {
        let sections = FrameSections::from(shape);
        self.frame(clock, wearers, &sections, input, to_host, out, false)
    }

    pub fn frame_wearers_shaped(
        &mut self,
        clock: f64,
        wearers: &[u64],
        sections: &FrameSections,
        input: &[f32],
        to_host: &Affine,
        out: &mut [f32],
    ) -> Result<usize, FrameError> {
        self.frame(clock, wearers, sections, input, to_host, out, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn frame(
        &mut self,
        clock: f64,
        wearers: &[u64],
        sections: &FrameSections,
        input: &[f32],
        to_host: &Affine,
        out: &mut [f32],
        shaped: bool,
    ) -> Result<usize, FrameError> {
        let stride = sections.stride();
        let needed = stride.saturating_mul(wearers.len());
        if input.len() < needed {
            return Err(FrameError::Input {
                held: input.len(),
                needed,
            });
        }
        let layout = if shaped {
            self.output_layout(sections)
        } else {
            self.build_skin();
            OutputLayout {
                bones: self.skin.bone_count as usize,
                runs: self.skin.runs.len(),
                ..OutputLayout::default()
            }
        };
        let out_stride = layout.stride();
        let out_needed = out_stride.saturating_mul(wearers.len());
        if out.len() < out_needed {
            return Err(FrameError::Output {
                held: out.len(),
                needed: out_needed,
            });
        }

        self.begin(clock);
        for (i, &wearer) in wearers.iter().enumerate() {
            let row = &input[i * stride..(i + 1) * stride];
            let frame = &mut out[i * out_stride..(i + 1) * out_stride];
            let (parts, rest) = row.split_at(sections.parts * PART_FLOATS);
            let (queries, rest) = rest.split_at(sections.queries);
            let Some((raw, rest)) = rest.split_first_chunk::<SIGNAL_COUNT>() else {
                continue;
            };
            let (fixed, rest) = rest.split_at(2);
            let (root, rest) = rest.split_at(if sections.root { ROOT_FLOATS } else { 0 });
            let (variables, rest) = rest.split_at(sections.variables);
            let (play, rest) = rest.split_at(if sections.play { PLAY_FLOATS } else { 0 });
            let (role, rest) = rest.split_at(if sections.role { ROLE_FLOATS } else { 0 });
            let hold = &rest[..if sections.hold { HOLD_FLOATS } else { 0 }];
            self.set_game_parts_flat(parts);
            for (id, &value) in queries.iter().enumerate() {
                self.set_query_at(id, value);
            }
            let wearer = WearerInput {
                index: i,
                wearer,
                raw,
                events: event_bits(fixed[0]),
                detail: detail_from_input(fixed[1]),
                root: <[f32; ROOT_FLOATS]>::try_from(root).map_or(Affine::IDENTITY, Affine),
                variables,
                play: play_input(play),
                role: RoleInput::from_floats(role),
                held: hold.first().is_some_and(|&h| h >= 0.5),
            };
            let root = self.run_wearer(&wearer);
            let wearer = WearerInput { root, ..wearer };
            self.write_frame(to_host, &layout, &wearer, frame, shaped);
        }
        Ok(self.skin.bone_count as usize)
    }

    pub fn wearer_output_stride(&mut self) -> usize {
        self.build_skin();
        self.skin.bone_count as usize * PART_FLOATS + self.skin.runs.len()
    }

    pub fn output_layout(&mut self, sections: &FrameSections) -> OutputLayout {
        self.build_skin();
        self.resolve_locators();
        OutputLayout {
            bones: self.skin.bone_count as usize,
            runs: self.skin.runs.len(),
            locators: self.locators.len(),
            colorway: sections.colorway,
            part_deltas: if sections.part_deltas {
                sections.parts
            } else {
                0
            },
            emote: sections.play,
        }
    }

    fn write_frame(
        &self,
        to_host: &Affine,
        layout: &OutputLayout,
        input: &WearerInput<'_>,
        frame: &mut [f32],
        shaped: bool,
    ) {
        let host = if shaped && self.rig == RigKind::Actor {
            to_host.mul(&input.root)
        } else {
            *to_host
        };
        let (matrices, rest) = frame.split_at_mut(layout.runs_at().min(frame.len()));
        let (offsets, rest) = rest.split_at_mut(layout.runs.min(rest.len()));
        self.write_skin(&host, matrices, offsets);
        if !shaped {
            return;
        }
        let (locators, rest) =
            rest.split_at_mut((layout.locators * LOCATOR_FLOATS).min(rest.len()));
        for (out, locator) in locators
            .chunks_exact_mut(LOCATOR_FLOATS)
            .zip(&self.locators)
        {
            match locator.source.and_then(|source| self.locator_world(source)) {
                Some(tf) => out.copy_from_slice(&host.mul(&tf).0),
                None => out.fill(0.0),
            }
        }
        let (colorway, rest) = rest.split_at_mut(usize::from(layout.colorway).min(rest.len()));
        let (deltas, status) =
            rest.split_at_mut((layout.part_deltas * PART_DELTA_FLOATS).min(rest.len()));
        if let Some(out) = status.first_mut().filter(|_| layout.emote) {
            *out = self.emote_status as f32;
        }
        if let Some(out) = colorway.first_mut() {
            *out = self
                .slots
                .iter()
                .find_map(|s| s.reaction.colorway)
                .map_or(NO_COLORWAY, f32::from);
        }
        let player = self
            .player_slot
            .and_then(|i| self.slots.get(i))
            .filter(|s| s.active);
        let emoting = self
            .player_slot
            .and_then(|i| self.slots.get(i))
            .is_some_and(Slot::is_emote);
        if emoting {
            for (id, out) in deltas.chunks_exact_mut(PART_DELTA_FLOATS).enumerate() {
                out.copy_from_slice(self.part_deltas.get(id).unwrap_or(&REST_DELTA));
            }
            return;
        }
        let fresh = self.names_fresh();
        for (id, out) in deltas.chunks_exact_mut(PART_DELTA_FLOATS).enumerate() {
            let posed = player.zip(self.game_parts.get(id)).and_then(|(s, part)| {
                let bone = match self.names.part_bones.get(id) {
                    Some(&bone) if fresh => bone?,
                    _ => s.model.as_ref()?.bone(&part.bone)?,
                };
                s.pose.0.get(bone)
            });
            match posed {
                Some(pose) => {
                    let delta = java::part_delta(pose);
                    out[0..3].copy_from_slice(&delta.offset);
                    out[3..6].copy_from_slice(&delta.rotation);
                    out[6..9].copy_from_slice(&delta.scale);
                    out[9] = 1.0;
                }
                None => out.copy_from_slice(&REST_DELTA),
            }
        }
    }

    pub fn forget_wearer(&mut self, wearer: u64) -> bool {
        self.effect_paints.retain(|p| p.wearer != wearer);
        self.named_fires.retain(|f| f.wearer != wearer);
        self.wearers.forget_wearer(wearer)
    }

    pub fn break_rings(&mut self, wearer: u64) -> bool {
        let mut broke = false;
        for slot in &self.slots {
            let (Some(asset), Some(cosmetic)) = (slot.asset.as_deref(), slot.cosmetic) else {
                continue;
            };
            if let Some(record) = self.wearers.record(cosmetic, wearer) {
                broke |= geometry::break_rings(&asset.effects, record.rings);
            }
        }
        broke
    }

    pub fn set_seed_wearer(&mut self, wearer: u64, seed: u64) {
        self.wearers.set_seed_wearer(wearer, seed);
    }

    pub fn set_signal_override(&mut self, wearer: u64, signal: usize, value: f32) -> bool {
        self.wearers.set_signal_override(wearer, signal, value)
    }

    pub fn wearer_count(&self) -> usize {
        self.wearers.wearer_count()
    }

    pub fn wearer_bytes(&self) -> usize {
        self.wearers.wearer_bytes()
    }

    pub fn wearer_detail(&self, wearer: u64) -> Option<Detail> {
        self.wearers.detail(wearer)
    }

    /// Whether a later `pose` could differ from the last one.
    /// Hosts can skip frames while it is false.
    /// Host-side changes (camera, `set_query`, loads) are the host's to redraw for.
    pub fn is_animating(&self) -> bool {
        self.slots.iter().any(|s| {
            s.model.is_some() && (s.moving || (s.frames > 1 && self.motion != Motion::Off))
        })
    }

    /// `bone` is a core name (`head`, `rightArm`, ..., `root`).
    /// Cosmetics then follow the game's limbs.
    pub fn set_game_part(&mut self, bone: &str, rest: [f32; 3], part: &java::Part) {
        let id = self.part_id(bone);
        self.set_game_part_at(id, rest, part);
    }

    pub fn part_id(&mut self, bone: &str) -> usize {
        match game_part_index(&self.game_parts, bone) {
            Some(id) => id,
            None => {
                self.game_parts.push(GamePart {
                    bone: bone.to_string(),
                    transform: Affine::IDENTITY,
                    pivot: [0.0; 3],
                    active: false,
                });
                self.part_values.push(None);
                self.renamed();
                self.game_parts.len() - 1
            }
        }
    }

    pub fn set_rig(&mut self, rig: RigKind) {
        self.rig = rig;
        self.renamed();
    }

    pub fn rig(&self) -> RigKind {
        self.rig
    }

    pub fn host_variable_id(&mut self, name: &str) -> usize {
        let name = name
            .strip_prefix("variable.")
            .or_else(|| name.strip_prefix("v."))
            .unwrap_or(name);
        match self.variable_names.iter().position(|n| n == name) {
            Some(id) => id,
            None => {
                self.variable_names.push(name.to_string());
                self.variable_names.len() - 1
            }
        }
    }

    pub fn locator_id(&mut self, name: &str) -> Option<usize> {
        if let Some(id) = self.locators.iter().position(|l| l.name == name) {
            return Some(id);
        }
        let source = locator_source(&self.slots, name)?;
        self.locators.push(SceneLocator {
            name: name.to_string(),
            source: Some(source),
        });
        Some(self.locators.len() - 1)
    }

    fn name_stamp(&self) -> NameStamp {
        NameStamp {
            epoch: self.names_epoch,
            rig: self.rig,
            parts: self.game_parts.len(),
            slots: self.slots.len(),
            player: self.player_slot,
        }
    }

    fn names_fresh(&self) -> bool {
        self.names.stamp == Some(self.name_stamp())
    }

    fn refresh_names(&mut self) {
        let stamp = self.name_stamp();
        if self.names.stamp == Some(stamp) {
            return;
        }
        let (rig, game) = (self.rig, &self.game_parts);
        let names = &mut self.names;
        rig_parts(rig, game, &mut names.rig_parts);
        let player = self
            .player_slot
            .and_then(|i| self.slots.get(i))
            .and_then(|s| s.model.as_ref());
        names.part_bones.clear();
        names
            .part_bones
            .extend(game.iter().map(|p| player.and_then(|m| m.bone(&p.bone))));
        names.carried_parts.clear();
        names
            .carried_parts
            .extend(game.iter().map(|p| CARRIED_BY_BODY.contains(&p.bone.as_str())));
        names.body_part = game_part_index(game, "body");
        names.head_part = game_part_index(game, "head");
        names.capsules.clear();
        names.capsules.extend(
            springs::capsules(rig)
                .iter()
                .map(|c| resolve_ext(rig, game, &self.slots, c.bone)),
        );
        names.locators.clear();
        names.locators.extend(
            PlayerLocator::ALL
                .iter()
                .map(|l| resolve_ext(rig, game, &self.slots, l.bone())),
        );
        for i in 0..self.slots.len() {
            let (lower, rest) = self.slots.split_at_mut(i);
            let slot = &mut rest[0];
            slot.externals.clear();
            slot.prop_externals.clear();
            slot.locator_binds.clear();
            let Some(model) = &slot.model else {
                continue;
            };
            slot.externals.extend(model.bones.iter().map(|b| {
                b.parent_name
                    .as_deref()
                    .map_or(Ext::default(), |name| resolve_ext(rig, game, lower, name))
            }));
            let Some(props) = &slot.props else {
                continue;
            };
            slot.prop_externals.extend(props.model.bones.iter().map(|b| {
                b.parent_name
                    .as_deref()
                    .map_or(PropExt::default(), |name| PropExt {
                        own: game_part_index(game, name),
                        rig: rig_bone(rig, name),
                        bone: model.bone(name),
                    })
            }));
            if let Some(authored) = slot.asset.as_deref().and_then(|a| a.model.as_ref()) {
                slot.locator_binds
                    .extend(authored.locators().map(|(bone, _)| {
                        let name = authored.bones[bone].name.as_str();
                        LocatorBind {
                            own: game_part_index(game, name),
                            rig: rig_bone(rig, name),
                            player_bone: model.bone(name),
                            prop_bone: props.model.bone(name),
                        }
                    }));
            }
        }
        names.stamp = Some(stamp);
    }

    fn ext_world(&self, ext: Ext, name: &str) -> Affine {
        ext_bone(
            self.rig,
            &self.game_parts,
            &self.names.rig_parts,
            &self.slots,
            ext,
            name,
        )
        .map_or(Affine::IDENTITY, |(tf, _)| tf)
    }

    fn resolve_locators(&mut self) {
        if self.locators_layout == Some(self.layout) {
            return;
        }
        for locator in &mut self.locators {
            locator.source = locator_source(&self.slots, &locator.name);
        }
        self.locators_layout = Some(self.layout);
    }

    fn locator_world(&self, source: LocatorSource) -> Option<Affine> {
        match source {
            LocatorSource::Model {
                slot,
                bone,
                locator,
            } => {
                let s = self.slots.get(slot)?;
                let at = s.model.as_ref()?.bones.get(bone)?.locators.get(locator)?;
                Some(at.world(s.transforms.get(bone)?))
            }
            LocatorSource::Player(locator) => Some(self.player_locator_world(locator)),
        }
    }

    fn player_locator_world(&self, locator: PlayerLocator) -> Affine {
        let bone = match self.names.locators.get(locator.index()) {
            Some(&ext) if self.names_fresh() => self.ext_world(ext, locator.bone()),
            _ => self.rig_bone(locator.bone()),
        };
        let at = Affine::translation(locator.offset());
        let wing = locator.wing().and_then(|name| {
            let model = self.elytra.as_ref()?;
            let index = match self.names.wings.get(locator.index()) {
                Some(&index) => index,
                None => model.bone(name),
            };
            self.wing_frames.get(index?)
        });
        match wing {
            Some(wing) => bone.mul(wing).mul(&at),
            None => bone.mul(&at),
        }
    }

    fn pose_wings(&mut self, raw: &[f32; SIGNAL_COUNT]) {
        let pose = player::elytra_pose(
            raw[IS_GLIDING] >= 0.5,
            raw[IS_SNEAKING] >= 0.5,
            raw[GROUND_SPEED],
            raw[VERTICAL_SPEED],
        );
        let model = self.elytra.get_or_insert_with(player::elytra);
        if self.names.wings.is_empty() {
            self.names.wings.extend(
                PlayerLocator::ALL
                    .iter()
                    .map(|l| l.wing().and_then(|name| model.bone(name))),
            );
        }
        player::pose_elytra(model, pose);
        world_transforms(model, None, |_| None, &mut self.wing_frames);
    }

    fn rig_bone(&self, name: &str) -> Affine {
        compose(self.rig, &self.game_parts, name)
            .or_else(|| slot_bone(&self.slots, name))
            .map_or(Affine::IDENTITY, |(tf, _)| tf)
    }

    pub fn load_asset(
        &mut self,
        slot: usize,
        asset: Arc<CosmeticAsset>,
    ) -> Result<(), DeclarationError> {
        let keep = asset.kind == Kind::Emote && self.player_slot == Some(slot);
        self.install(slot, asset, keep, None)
    }

    pub fn load_emote(
        &mut self,
        slot: usize,
        props: Option<usize>,
        asset: Arc<CosmeticAsset>,
    ) -> Result<(), DeclarationError> {
        let props = props.filter(|&p| p != slot);
        if let Some(p) = props.filter(|&p| p < self.slots.len()) {
            self.clear(p);
        }
        if asset.kind != Kind::Emote {
            return self.load_asset(slot, asset);
        }
        let bound = Props::of(&asset);
        let model = props.and(bound.as_ref()).map(|b| b.model.clone());
        self.install(slot, asset, true, bound)?;
        if let (Some(p), Some(model)) = (props, model) {
            self.remodel(p, model, false, 1, COSMETIC_FRAME_SECONDS);
            self.slots[p].prop_of = Some(slot);
        }
        Ok(())
    }

    fn install(
        &mut self,
        slot: usize,
        asset: Arc<CosmeticAsset>,
        keep: bool,
        props: Option<Props>,
    ) -> Result<(), DeclarationError> {
        let animations = asset.animations.clone();
        let controllers = Some(asset.controllers.clone()).filter(|c| !c.controllers().is_empty());
        let (offsets, _) = channel_offsets(&animations);
        let declared = asset.record;
        let plan = asset::record_plan(&asset);
        let needed = plan.declarations;
        let declarations = WearerDeclarations {
            variables: declared.variables.max(needed.variables),
            controllers: declared.controllers.max(needed.controllers),
            one_shots: declared.one_shots.max(needed.one_shots),
            spring_bones: declared.spring_bones.max(needed.spring_bones),
            effect_times: declared.effect_times.max(needed.effect_times),
            effect_floats: declared.effect_floats.max(needed.effect_floats),
            channels: declared.channels.max(needed.channels),
            rings: needed.rings,
            ring_points: declared.ring_points.max(needed.ring_points),
            ring_turns: declared.ring_turns.max(needed.ring_turns),
            ring_gates: declared.ring_gates.max(needed.ring_gates),
            held_channels: declared.held_channels.max(needed.held_channels),
        };
        let stateful = !animations.is_empty()
            || controllers.is_some()
            || !asset.reactions.is_empty()
            || !asset.springs.is_empty()
            || !asset.effects.is_empty();
        let cosmetic = if stateful {
            Some(self.wearers.add_cosmetic(declarations)?)
        } else {
            None
        };
        let model = asset.model.clone().filter(|_| !keep);
        let props = match props {
            Some(props) => Some(props),
            None if keep && asset.kind == Kind::Emote => Props::of(&asset),
            None => None,
        };

        let s = self.slot(slot);
        let old = mem::replace(&mut s.cosmetic, cosmetic);
        s.runtime = Runtime::new(&asset.interner);
        s.anim_time = asset.interner.query_slot("anim_time");
        s.life_time = asset.interner.query_slot("life_time");
        s.signal_queries = signal_queries(&asset.interner);
        s.host_queries.clear();
        s.host_variables.clear();
        s.seeds = [wearer::seed_key(&asset.id), wearer::seed_key("animation")];
        s.interner = Interner::default();
        s.animations = animations;
        s.offsets = offsets;
        s.controllers = controllers;
        s.reaction_floats = plan.reaction_floats;
        s.delays = plan.delays;
        s.reaction = ReactionOut::default();
        s.timeline_clock = plan.timeline;
        s.anim_clock = plan.anim_clock;
        s.anchor = plan.anchor;
        s.emote_at = plan.emote_floats.zip(plan.emote_time);
        s.emote = EmoteFrame::default();
        s.place_at = plan.place_floats.zip(plan.place_time);
        s.placed = None;
        s.props = props;
        s.started = None;
        s.asset = Some(asset);
        let kept = s.model.is_some();
        let id = s.asset.as_ref().map_or(String::new(), |a| a.id.clone());
        if let Some(old) = old {
            self.wearers.remove_cosmetic(old);
        }
        match model {
            Some(model) => self.remodel(slot, model, false, 1, COSMETIC_FRAME_SECONDS),
            None if kept => {
                let Slot {
                    binds,
                    animations,
                    model,
                    ..
                } = &mut self.slots[slot];
                if let Some(model) = model {
                    *binds = animations.iter().map(|a| a.bind(model)).collect();
                }
                let channels = self.slots[slot].refresh_held();
                if let Some(cosmetic) = cosmetic {
                    self.wearers.set_held_channels(cosmetic, channels);
                }
                self.layout = self.layout.wrapping_add(1);
                self.renamed();
            }
            None => {
                let empty = Model {
                    identifier: id,
                    texture_size: [16.0, 16.0],
                    bones: Vec::new(),
                };
                self.remodel(slot, empty, false, 1, COSMETIC_FRAME_SECONDS);
            }
        }
        Ok(())
    }

    pub fn set_game_part_at(&mut self, id: usize, rest: [f32; 3], part: &java::Part) {
        if let Some(entry) = self.game_parts.get_mut(id) {
            entry.transform = java::part_transform(rest, part);
            entry.pivot = java::rest_pivot(rest);
            entry.active = true;
        }
        if let Some(values) = self.part_values.get_mut(id) {
            *values = Some((rest, *part));
        }
    }

    pub fn set_game_parts(&mut self, parts: &[([f32; 3], java::Part)]) {
        for (id, entry) in self.game_parts.iter_mut().enumerate() {
            match parts.get(id) {
                Some((rest, part)) => {
                    entry.transform = java::part_transform(*rest, part);
                    entry.pivot = java::rest_pivot(*rest);
                    entry.active = true;
                    if let Some(values) = self.part_values.get_mut(id) {
                        *values = Some((*rest, *part));
                    }
                }
                None => entry.active = false,
            }
        }
    }

    pub fn set_game_parts_flat(&mut self, parts: &[f32]) {
        let mut chunks = parts.chunks_exact(PART_FLOATS);
        for (entry, values) in self.game_parts.iter_mut().zip(&mut self.part_values) {
            match chunks.next() {
                Some(p) => {
                    let rest = [p[0], p[1], p[2]];
                    let part = java::Part {
                        offset: [p[3], p[4], p[5]],
                        rotation: [p[6], p[7], p[8]],
                        scale: [p[9], p[10], p[11]],
                    };
                    entry.transform = java::part_transform(rest, &part);
                    entry.pivot = java::rest_pivot(rest);
                    entry.active = true;
                    *values = Some((rest, part));
                }
                None => entry.active = false,
            }
        }
    }

    pub fn clear_game_parts(&mut self) {
        for part in &mut self.game_parts {
            part.active = false;
        }
    }

    /// A game host adds `java::part_delta` of this to the vanilla part to play the emote.
    pub fn bone_pose(&self, slot: usize, bone: &str) -> Option<BonePose> {
        let s = self.slots.get(slot)?;
        s.pose.0.get(s.model.as_ref()?.bone(bone)?).copied()
    }

    pub fn build(&mut self, camera: &Camera, viewport: [f32; 2]) -> &Buffers {
        self.transform();
        let instances = instances(&self.slots, self.clock, self.motion);
        self.buffers
            .build(&instances, camera, &self.light, viewport);
        &self.buffers
    }

    /// `to_host` is `java::MODEL` for the game.
    pub fn build_mesh(&mut self, to_host: &Affine) -> &Mesh {
        self.transform();
        let instances = instances(&self.slots, self.clock, self.motion);
        self.mesh.build(&instances, to_host);
        &self.mesh
    }

    pub fn build_skin(&mut self) -> &Skin {
        if self.skin_layout != Some(self.layout) {
            let instances = instances(&self.slots, self.clock, self.motion);
            self.skin.build(&instances);
            self.skin_layout = Some(self.layout);
        }
        &self.skin
    }

    pub fn layout(&self) -> u32 {
        self.layout
    }

    pub fn skin_frame(&mut self, to_host: &Affine) {
        self.build_skin();
        self.transform();
        let mut matrices = mem::take(&mut self.skin_transforms);
        let mut offsets = mem::take(&mut self.skin_offsets);
        matrices.resize(self.skin.bone_count as usize * PART_FLOATS, 0.0);
        offsets.resize(self.skin.runs.len(), 0.0);
        self.write_skin(to_host, &mut matrices, &mut offsets);
        self.skin_transforms = matrices;
        self.skin_offsets = offsets;
    }

    fn write_skin(&self, to_host: &Affine, matrices: &mut [f32], offsets: &mut [f32]) {
        let transforms = self
            .slots
            .iter()
            .filter(|s| s.visible && s.model.is_some())
            .flat_map(|s| s.transforms.iter());
        for (out, tf) in matrices.chunks_exact_mut(PART_FLOATS).zip(transforms) {
            out.copy_from_slice(&to_host.mul(tf).0);
        }
        for (out, run) in offsets.iter_mut().zip(&self.skin.runs) {
            *out = self.slots.get(run.texture as usize).map_or(0.0, |s| {
                s.frame(self.clock, self.motion) as f32 / s.frames.max(1) as f32
            });
        }
    }

    fn transform(&mut self) {
        self.transform_from(0);
    }

    fn transform_from(&mut self, first: usize) {
        self.refresh_names();
        let game = &self.game_parts;
        let rig_parts = &self.names.rig_parts;
        let rig = self.rig;

        for i in first..self.slots.len() {
            // Lower slots are done, so a cosmetic bone parented to `head` follows the player's.
            let (lower, rest) = self.slots.split_at_mut(i);
            let slot = &mut rest[0];
            let Some(model) = &slot.model else {
                continue;
            };

            let scale = Affine::scale([slot.scale; 3]);
            world_transforms_resolved(
                model,
                Some(&slot.pose),
                |bone| {
                    let name = model.bones[bone].parent_name.as_deref()?;
                    let ext = slot.externals.get(bone).copied()?;
                    let (host, pivot) = ext_bone(rig, game, rig_parts, lower, ext, name)?;
                    Some(host.mul(&Affine::about(pivot, &scale)))
                },
                &slot.emote.carried,
                &mut slot.transforms,
            );
            let Slot {
                model: Some(model),
                props: Some(props),
                transforms,
                prop_externals,
                ..
            } = slot
            else {
                continue;
            };
            world_transforms_resolved(
                &props.model,
                Some(&props.pose),
                |bone| {
                    let ext = prop_externals.get(bone).copied()?;
                    let (host, pivot) =
                        compose_at(rig, game, rig_parts, ext.own, ext.rig).or_else(|| {
                            let bone = ext.bone?;
                            Some((*transforms.get(bone)?, model.bones[bone].pivot))
                        })?;
                    Some(host.mul(&Affine::about(pivot, &scale)))
                },
                &[],
                &mut props.transforms,
            );
        }
    }
}

fn instances(slots: &[Slot], clock: f64, motion: Motion) -> Vec<Instance<'_>> {
    slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.visible)
        .filter_map(|(i, s)| {
            let mut instance = Instance::new(s.model.as_ref()?, &s.transforms, i as u32);
            instance.cull = s.cull;
            instance.frames = s.frames;
            instance.frame = s.frame(clock, motion);
            Some(instance)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocatorSource {
    Model {
        slot: usize,
        bone: usize,
        locator: usize,
    },
    Player(PlayerLocator),
}

#[derive(Clone, Copy, Debug)]
struct EffectDraw {
    slot: usize,
    wearer: u64,
    cosmetic: CosmeticSlot,
    seed: u64,
    seed_epoch: f64,
    fuel: u32,
    budget: usize,
    features: Features,
    colorway: Option<usize>,
    paint: Option<usize>,
    root: Affine,
    frames: usize,
    model: usize,
}

struct EffectTones {
    slot: usize,
    colorway: Option<usize>,
    custom: Vec<(u8, u32)>,
    tones: Vec<u32>,
}

struct NamedFire {
    wearer: u64,
    slot: usize,
    id: u16,
    at: f64,
}

struct WearerPaint {
    wearer: u64,
    colorway: Option<usize>,
    custom: Vec<(u8, u32)>,
}

fn paint_custom<'a>(paints: &'a [WearerPaint], draw: &EffectDraw) -> &'a [(u8, u32)] {
    draw.paint
        .and_then(|i| paints.get(i))
        .filter(|p| p.wearer == draw.wearer)
        .map_or(&[], |p| p.custom.as_slice())
}

fn find_tones<'a>(
    tones: &'a [EffectTones],
    draw: &EffectDraw,
    custom: &[(u8, u32)],
) -> Option<&'a EffectTones> {
    tones
        .iter()
        .find(|t| t.slot == draw.slot && t.colorway == draw.colorway && t.custom == custom)
}

fn draw_tones<'a>(
    tones: &'a [EffectTones],
    paints: &[WearerPaint],
    draw: &EffectDraw,
) -> &'a [u32] {
    find_tones(tones, draw, paint_custom(paints, draw)).map_or(&[], |t| t.tones.as_slice())
}

struct SceneLocator {
    name: String,
    source: Option<LocatorSource>,
}

fn locator_source(slots: &[Slot], name: &str) -> Option<LocatorSource> {
    let model = slots.iter().enumerate().find_map(|(slot, s)| {
        let model = s.model.as_ref()?;
        model.bones.iter().enumerate().find_map(|(bone, b)| {
            let locator = b.locators.iter().position(|l| l.name == name)?;
            Some(LocatorSource::Model {
                slot,
                bone,
                locator,
            })
        })
    });
    model.or_else(|| PlayerLocator::parse(name).map(LocatorSource::Player))
}

fn emitted(emission: &Emission, features: Features) -> bool {
    features.effects && (emission.kind == EmitKind::Sound || features.particles)
}

fn resolve_ext(rig: RigKind, parts: &[GamePart], lower: &[Slot], name: &str) -> Ext {
    Ext {
        own: game_part_index(parts, name),
        rig: rig_bone(rig, name),
        lower: lower
            .iter()
            .enumerate()
            .rev()
            .find_map(|(slot, s)| Some((slot, s.model.as_ref()?.bone(name)?))),
    }
}

fn ext_bone(
    rig: RigKind,
    parts: &[GamePart],
    rig_parts: &[Option<usize>],
    lower: &[Slot],
    ext: Ext,
    name: &str,
) -> Option<(Affine, [f32; 3])> {
    compose_at(rig, parts, rig_parts, ext.own, ext.rig).or_else(|| {
        let (at, bone) = ext.lower?;
        let slot = lower.get(at)?;
        match (slot.model.as_ref(), slot.transforms.get(bone)) {
            (Some(model), Some(tf)) => Some((*tf, model.bones[bone].pivot)),
            _ => slot_bone(lower, name),
        }
    })
}

fn slot_bone(lower: &[Slot], name: &str) -> Option<(Affine, [f32; 3])> {
    lower.iter().rev().find_map(|s| {
        let model = s.model.as_ref()?;
        let i = model.bone(name)?;
        Some((*s.transforms.get(i)?, model.bones[i].pivot))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{open_files, Kind, Opened, Options};
    use crate::controller::MAX_CONTROLLERS;
    use crate::effects::{MomentPlan, QUAD_CORNERS, QUAD_FLOATS, QUAD_TEXTURE};
    use crate::ir::Locator;
    use crate::palette::Colorway;
    use crate::reactions::{On, Reaction, EQUIP};
    use crate::roles::Role;
    use crate::signals::{signal_id, EVENT_EQUIP, EVENT_HURT, SIGNAL_COUNT};
    use serde_json::{json, Value};

    const HAT: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.hat","texture_width":16,"texture_height":16},
        "bones":[{"name":"brim","pivot":[0,24,0],"cubes":[{"origin":[-5,32,-5],"size":[10,1,10],"uv":[0,0]}]}]}]}"#;

    const NOD: &str = r#"{"animations":{"nod":{"loop":true,"animation_length":2,
        "bones":{"head":{"rotation":{"0.0":[0,0,0],"1.0":[30,0,0],"2.0":[0,0,0]}}}}}}"#;

    fn hat() -> Model {
        crate::bedrock::parse_one(HAT).unwrap()
    }

    fn draw(scene: &mut Scene, seconds: f32) -> &Buffers {
        scene.pose(seconds);
        scene.build(&Camera::orbit(180.0, 0.0, 62.0, [0.0, 18.0, 0.0]), [256.0, 256.0])
    }

    fn scene_with_hat() -> Scene {
        let mut scene = Scene::default();
        scene.load_player(0, Arms::Wide);
        scene.load_animation(0, NOD).unwrap();
        scene.load_cosmetic(1, hat(), BodySlot::Hat, None, None);
        scene
    }

    fn head(scene: &Scene) -> Affine {
        let player = &scene.slots[0];
        player.transforms[player.model.as_ref().unwrap().bone("head").unwrap()]
    }

    #[test]
    fn slots_follow_their_bone_from_their_own_start() {
        let mut scene = scene_with_hat();
        let runs = &draw(&mut scene, 7.0).runs;
        assert!(runs.iter().any(|r| r.texture == 0) && runs.iter().any(|r| r.texture == 1));
        assert_eq!(head(&scene), Affine::IDENTITY, "rest until played");
        scene.play(0, 100.0);
        scene.set_visible(0, false);
        let runs = &draw(&mut scene, 101.0).runs;
        assert!(runs.iter().all(|r| r.texture == 1), "a hidden player is not drawn");
        let nod = scene.bone_pose(0, "head").unwrap().rotation;
        assert_eq!(nod, [30.0, 0.0, 0.0], "but it still plays from its own start");
        assert_eq!(scene.slots[1].transforms[0], head(&scene), "and its hat follows");

        scene.stop(0);
        scene.set_scale(1, 2.0);
        draw(&mut scene, 101.0);
        let scaled = scene.slots[1].transforms[0];
        let points = [24.0, 25.0].map(|y| scaled.point([0.0, y, 0.0]));
        assert_eq!(points, [[0.0, 24.0, 0.0], [0.0, 26.0, 0.0]], "scaled about the pivot");

        scene.set_scale(1, 1.0);
        let part = |offset, rotation| java::Part { offset, rotation, scale: [1.0; 3] };
        let (crouch, nod) = (part([0.0, 3.2, 0.0], [0.0; 3]), part([0.0; 3], [0.5, 0.0, 0.0]));
        assert_eq!([scene.part_id("root"), scene.part_id("head")], [0, 1]);
        scene.set_game_parts(&[([0.0; 3], crouch), ([0.0; 3], nod)]);
        draw(&mut scene, 0.0);
        let tf = |p| java::part_transform([0.0; 3], p);
        assert_eq!(scene.slots[1].transforms[0], tf(&crouch).mul(&tf(&nod)));
        scene.clear_game_parts();
        draw(&mut scene, 0.0);
        assert_eq!(scene.slots[1].transforms[0], Affine::IDENTITY, "back on the rest pose");

        scene.clear(1);
        assert!(draw(&mut scene, 0.0).runs.iter().all(|r| r.texture == 0));
        let padded = format!("{NOD}{}", " ".repeat(256 * 1024));
        let error = scene.load_animation(0, &padded).unwrap_err();
        assert!(error.to_string().contains("json_too_large"), "{error}");
    }

    #[test]
    fn a_posed_skin_is_the_mesh() {
        let mut scene = scene_with_hat();
        scene.load_cape(2, 16, 0.1);
        scene.set_scale(1, 1.3);
        scene.play(0, 0.0);
        let crouch = java::Part { offset: [0.0, 3.2, 0.0], rotation: [0.4, 0.1, -0.2], scale: [1.0, 1.1, 0.9] };
        scene.set_game_part("root", [0.0; 3], &crouch);
        let runs = |runs: &[crate::mesh::MeshRun]| runs.iter().map(|r| (r.texture, r.start, r.count, r.cull)).collect::<Vec<_>>();
        for seconds in [0.0, 0.7, 1.35] {
            scene.pose(seconds);
            let mesh = scene.build_mesh(&java::MODEL);
            let want = [mesh.positions.clone(), mesh.normals.clone(), mesh.uvs.clone()];
            let mesh_runs = runs(&mesh.runs);
            scene.skin_frame(&java::MODEL);
            let skin = &scene.skin;
            assert_eq!((runs(&skin.runs), &skin.glow), (mesh_runs, &scene.mesh.glow));
            let (tf, offsets) = (&scene.skin_transforms, &scene.skin_offsets);
            let mut got = [Vec::new(), Vec::new(), Vec::new()];
            for (r, run) in skin.runs.iter().enumerate() {
                for v in run.start as usize * 4..(run.start + run.count) as usize * 4 {
                    let b = skin.bones[v] as usize;
                    let m = Affine(tf[b * 12..b * 12 + 12].try_into().unwrap());
                    let at = |a: &[f32]| [a[v * 3], a[v * 3 + 1], a[v * 3 + 2]];
                    got[0].extend_from_slice(&m.point(at(&skin.positions)));
                    got[1].extend_from_slice(&crate::math::normalize(m.vector(at(&skin.normals))));
                    got[2].extend_from_slice(&[skin.uvs[v * 2], skin.uvs[v * 2 + 1] + offsets[r]]);
                }
            }
            for (got, want) in got.iter().zip(&want) {
                assert_eq!(got.len(), want.len());
                assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-4), "at {seconds}");
            }
        }
    }

    const EMOTE_PARTS: [(&str, [f32; 3]); 4] =
        [("head", [0.0; 3]), ("body", [0.0; 3]), ("rightArm", [-5.0, 2.0, 0.0]), ("root", [0.0; 3])];
    const HEAD: usize = 0;
    const BODY: usize = 1;
    const ARM: usize = 2;

    #[derive(Clone, Copy)]
    struct Wearer {
        id: u64,
        vanilla: [java::Part; 4],
        raw: [f32; SIGNAL_COUNT],
        events: u32,
        detail: Detail,
        at: [f32; 3],
        play: [f32; 2],
        role: [f32; 5],
        hold: f32,
    }

    impl Wearer {
        fn on(mut self, signal: &str) -> Self {
            self.raw[signal_id(signal).unwrap()] = 1.0;
            self
        }
    }

    fn standing(id: u64, age: f32) -> Wearer {
        let vanilla = EMOTE_PARTS.map(|(_, rest)| java::Part { offset: rest, rotation: [0.0; 3], scale: [1.0; 3] });
        let (raw, detail, at, role) = (NEUTRAL_SIGNALS, Detail::Full, [0.0; 3], [-1.0, 0.0, 0.0, 0.0, 0.0]);
        Wearer { id, vanilla, raw, events: 0, detail, at, play: [0.0, age], role, hold: 0.0 }
    }

    fn idle(id: u64) -> Wearer {
        Wearer { play: [-1.0, 0.0], ..standing(id, 0.0) }
    }

    fn input(s: &FrameSections, rows: &[Wearer]) -> (Vec<u64>, Vec<f32>) {
        let mut input = Vec::new();
        for r in rows {
            for ((_, rest), part) in EMOTE_PARTS.iter().zip(&r.vanilla).take(s.parts) {
                input.extend([*rest, part.offset, part.rotation, part.scale].concat());
            }
            input.extend_from_slice(&r.raw);
            input.extend_from_slice(&[r.events as f32, r.detail.value() as f32]);
            if s.root {
                input.extend_from_slice(&Affine::translation(r.at).mul(&Affine::scale([1.0 / 16.0; 3])).0);
            }
            let tail = [(s.play, &r.play[..]), (s.role, &r.role[..]), (s.hold, &[r.hold][..])];
            tail.into_iter().filter(|(on, _)| *on).for_each(|(_, floats)| input.extend_from_slice(floats));
        }
        (rows.iter().map(|r| r.id).collect(), input)
    }

    fn wearer_scene(animation: &str) -> Scene {
        let mut scene = Scene::default();
        scene.load_cosmetic(0, hat(), BodySlot::Hat, None, None);
        scene.load_animation(0, animation).unwrap();
        scene.play(0, 0.0);
        scene
    }

    fn frame(scene: &mut Scene, clock: f64, rows: &[Wearer]) -> Vec<Vec<f32>> {
        let (ids, input) = input(&FrameSections::default(), rows);
        let stride = scene.wearer_output_stride();
        let mut out = vec![0.0; stride * rows.len()];
        let bones = scene.frame_wearers(clock, &ids, FrameShape::default(), &input, &Affine::IDENTITY, &mut out);
        assert_eq!(bones, Ok(scene.skin.bone_count as usize));
        out.chunks(stride).map(<[f32]>::to_vec).collect()
    }

    fn brim(scene: &Scene) -> [f32; 3] {
        scene.bone_pose(0, "brim").unwrap().rotation
    }

    fn pose_of(scene: &mut Scene, clock: f64, row: Wearer) -> [f32; 3] {
        frame(scene, clock, &[row]);
        brim(scene)
    }

    fn posed_row(rotation: [f32; 3]) -> Vec<f32> {
        let [x, y, z] = rotation;
        let json = format!(r#"{{"animations":{{"a":{{"loop":true,"bones":{{"brim":{{"rotation":[{x},{y},{z}]}}}}}}}}}}"#);
        frame(&mut wearer_scene(&json), 1.0, &[idle(99)]).remove(0)
    }

    fn variable(scene: &mut Scene, wearer: u64, name: &str) -> f32 {
        let slot = &scene.slots[0];
        let interner = slot.asset.as_ref().map_or(&slot.interner, |a| &a.interner);
        let (at, cosmetic) = (interner.variable_slot(name).unwrap() as usize, slot.cosmetic.unwrap());
        scene.wearers.record(cosmetic, wearer).unwrap().variables[at]
    }

    const SNEAK: &str = r#"{"animations":{"a":{"loop":true,"bones":{"brim":{"rotation":["q.is_sneaking * 20",0,0]}}}}}"#;

    const COUNTING: &str = r#"{"animations":{"a":{"loop":true,"initialize":"v.inits = v.inits + 1;",
        "pre_animation":["v.frames = v.frames + 1;","t.carry = t.carry + 1;"],
        "bones":{"brim":{"rotation":["v.frames * 10","v.inits","t.carry"]}}}}}"#;

    #[test]
    fn wearers_keep_their_own_signals_variables_and_records() {
        let mut scene = wearer_scene(SNEAK);
        let rows = frame(&mut scene, 1.0, &[idle(1).on("is_sneaking"), idle(2)]);
        let mut preview = wearer_scene(SNEAK);
        preview.set_query("is_sneaking", 1.0);
        preview.pose(1.0);
        preview.skin_frame(&Affine::IDENTITY);
        let single = [preview.skin_transforms.clone(), preview.skin_offsets.clone()].concat();
        assert_eq!(rows[0], single, "as one preview call with the same signals");
        assert_ne!(rows[0], rows[1]);
        assert!(scene.set_signal_override(2, signal_id("is_sneaking").unwrap(), 1.0));
        assert_eq!(pose_of(&mut scene, 1.1, idle(2)), [20.0, 0.0, 0.0], "an override forces a signal");

        let mut scene = wearer_scene(COUNTING);
        for n in 1..=3 {
            assert_eq!(pose_of(&mut scene, n as f64, idle(1)), [n as f32 * 10.0, 1.0, 1.0]);
        }
        assert_eq!(pose_of(&mut scene, 3.0, idle(2)), [10.0, 1.0, 1.0], "a record per wearer");
        assert_eq!(variable(&mut scene, 1, "inits"), 1.0, "initialize runs once per record");
        assert_eq!(scene.wearer_count(), 2);
        assert!(scene.forget_wearer(1) && !scene.forget_wearer(1));
        assert_eq!(pose_of(&mut scene, 4.0, idle(1)), [10.0, 1.0, 1.0], "a new record starts over");
        assert_eq!(pose_of(&mut scene, 40.0, idle(2)), [10.0, 1.0, 1.0], "dropped after 30 s unseen");
        assert_eq!(scene.wearer_count(), 1);
    }

    #[test]
    fn minimal_detail_evaluates_at_ten_hertz_and_holds_between() {
        let json = r#"{"animations":{"a":{"loop":true,"animation_length":100,
            "bones":{"brim":{"rotation":["q.anim_time * 100",0,0]}}}}}"#;
        let mut scene = wearer_scene(json);
        let far = Wearer { detail: Detail::Minimal, ..idle(7) };
        let tick = |c| hold_tick(7, Detail::Minimal, c);
        let start = (0..100).map(|i| 1.0 + i as f64 * 0.01).find(|&c| tick(c) == tick(c + 0.04)).unwrap();
        let held = pose_of(&mut scene, start, far);
        assert_eq!(pose_of(&mut scene, start + 0.04, far), held, "held");
        assert_eq!(scene.wearer_detail(7), Some(Detail::Minimal));
        assert_ne!(pose_of(&mut scene, start + 0.15, far), held, "the next tenth evaluates again");
        let moving = pose_of(&mut scene, start + 0.16, idle(7));
        assert_ne!(pose_of(&mut scene, start + 0.17, idle(7)), moving);
        assert_eq!(scene.wearer_detail(7), Some(Detail::Full));
    }

    #[test]
    fn frame_buffers_keep_their_wire_layout() {
        assert_eq!([Detail::Full, Detail::Reduced, Detail::Minimal].map(Detail::value), [0, 1, 2]);
        let read = [0.0, 1.0, 2.0, f32::NAN, 7.0].map(detail_from_input);
        assert_eq!(read, [Detail::Full, Detail::Reduced, Detail::Minimal, Detail::Full, Detail::Minimal]);
        let mut scene = wearer_scene(COUNTING);
        let shape = FrameShape { parts: 1, queries: 2 };
        let stride = shape.stride();
        assert_eq!(stride, 12 + 2 + SIGNAL_COUNT + 2);
        let out_stride = scene.wearer_output_stride();
        let (input, mut out, ids) = (vec![0.0; stride * 2], vec![0.0; out_stride * 2], [1, 2]);
        let short = scene.frame_wearers(1.0, &ids, shape, &input[1..], &Affine::IDENTITY, &mut out);
        assert_eq!(short, Err(FrameError::Input { held: stride * 2 - 1, needed: stride * 2 }));
        let short = scene.frame_wearers(1.0, &ids, shape, &input, &Affine::IDENTITY, &mut out[1..]);
        assert_eq!(short, Err(FrameError::Output { held: out_stride * 2 - 1, needed: out_stride * 2 }));
        assert_eq!(scene.wearer_count(), 0, "a short buffer fails before anything runs");
    }

    const FIN: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.fin","texture_width":16,"texture_height":16},
        "bones":[{"name":"fin","parent":"head","pivot":[0,32,0],"cubes":[{"origin":[-1,32,-1],"size":[2,4,2],"uv":[0,0]}]}]}]}"#;

    const DUET: &str = r#"{"animations":{
        "animation.duet.a":{"loop":true,"animation_length":2,"bones":{"head":{"rotation":{"0.0":[0,0,0],"1.0":[30,0,0],"2.0":[0,0,0]}}}},
        "animation.duet.b":{"loop":true,"animation_length":2,"bones":{"head":{"rotation":{"0.0":[0,0,0],"1.0":[0,40,0],"2.0":[0,0,0]}}}}}}"#;

    fn duet() -> CosmeticAsset {
        let mut asset = CosmeticAsset::new("duet", 9, Kind::Emote);
        asset.animations = animation::parse(DUET, &mut asset.interner).unwrap();
        let role = |i: u16| Role { id: i.to_string(), interaction: None, animation: i, offset: [0.0; 3], yaw: 0.0 };
        asset.roles = vec![role(0), role(1)];
        asset
    }

    const EVERY_SECTION: FrameSections = FrameSections {
        parts: 2, queries: 1, root: true, variables: 2, play: true, role: true, colorway: true, part_deltas: true, hold: true,
    };

    fn sectioned_row(root: Affine, pet_mode: f32, play: f32, role: f32, lift: f32) -> Vec<f32> {
        let mut input = vec![0.0, 0.0, 0.0, 0.0, lift, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        input.extend_from_slice(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0]);
        input.extend_from_slice(&[&NEUTRAL_SIGNALS[..], &[0.0, 0.0], &root.0].concat());
        input.extend_from_slice(&[pet_mode, 5.0, play, 1.0, role, 0.0, 0.0, 0.0, 0.0, 0.0]);
        input
    }

    #[test]
    fn shaped_frames_read_and_write_every_section() {
        let s = EVERY_SECTION;
        let base = 2 * PART_FLOATS + 1 + SIGNAL_COUNT + 2;
        let offsets = [s.shape().stride(), s.root_at(), s.variables_at(), s.play_at(), s.role_at(), s.hold_at(), s.stride()];
        assert_eq!(offsets, [base, base, base + 12, base + 14, base + 16, base + 21, base + 22]);

        let wave = r#"{"animations":{"animation.fin.wave":{"loop":true,"bones":{"fin":{"rotation":["v.pet_mode * 10",0,0]}}}}}"#;
        let mut scene = Scene::default();
        scene.load_player(0, Arms::Wide);
        scene.load_asset(0, Arc::new(duet())).unwrap();
        scene.load_asset(1, glow(wave, None, Vec::new(), None)).unwrap();
        scene.play(1, 0.0);
        assert_eq!([scene.part_id("root"), scene.part_id("head"), scene.query_id("is_moving")], [0, 1, 0]);
        assert_eq!(["pet_mode", "variable.unused", "v.pet_mode"].map(|n| scene.host_variable_id(n)), [0, 1, 0]);
        assert_eq!(["feet", "tip", "nowhere"].map(|n| scene.locator_id(n)), [Some(0), Some(1), None]);
        let layout = scene.output_layout(&s);
        let (bones, runs) = (scene.skin.bone_count as usize, scene.skin.runs.len());
        assert_eq!(layout, OutputLayout { bones, runs, locators: 2, colorway: true, part_deltas: 2, emote: true });
        let stride = bones * 12 + runs + 2 * LOCATOR_FLOATS + 1 + 2 * PART_DELTA_FLOATS + 1;
        assert_eq!(layout.stride(), stride);

        let root = Affine::translation([100.0, 64.0, -3.0]);
        let rest = sectioned_row(Affine::IDENTITY, 0.0, -1.0, -1.0, 0.0);
        let input = [rest, sectioned_row(root, 2.0, 0.0, 1.0, 2.0)].concat();
        let (mut out, ids) = (vec![f32::NAN; 2 * stride], [21, 22]);
        let framed = scene.frame_wearers_shaped(1.0, &ids, &s, &input, &Affine::IDENTITY, &mut out);
        assert_eq!(framed, Ok(bones));
        assert!(out.iter().all(|v| !v.is_nan()), "every float is written");
        let (rest, lifted) = out.split_at(stride);
        let fin = scene.bone_pose(1, "fin").unwrap().rotation;
        assert_eq!(fin, [20.0, 0.0, 0.0], "host variables reach the wearer's MoLang");
        let head = scene.bone_pose(0, "head").unwrap().rotation;
        assert!(head[0].abs() < 1e-4 && (head[1] - 40.0).abs() < 1e-3, "role b plays its clip: {head:?}");

        let at = layout.locators_at();
        let raised = java::Part { offset: [0.0, 2.0, 0.0], rotation: [0.0; 3], scale: [1.0; 3] };
        let feet = java::part_transform([0.0; 3], &raised);
        assert_eq!(lifted[at..at + 12], feet.0, "built-in locators follow the game's parts");
        let tip = scene.slots[1].transforms[0].mul(&Affine::translation([0.0, 36.0, 0.0]));
        assert_eq!(lifted[at + 12..layout.colorway_at()], tip.0, "model locators follow their bone");
        assert_eq!(lifted[layout.colorway_at()], NO_COLORWAY);
        let deltas = &lifted[layout.part_deltas_at()..layout.emote_at()];
        assert_eq!(deltas[..10], [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        assert!((deltas[14] - 40f32.to_radians()).abs() < 1e-4 && deltas[19] == 1.0, "{deltas:?}");
        assert_eq!(lifted[layout.emote_at()], EMOTE_POSING as f32);
        let resting = &rest[layout.part_deltas_at() + PART_DELTA_FLOATS..layout.emote_at()];
        assert_eq!(resting, REST_DELTA, "a wearer not playing keeps vanilla");
    }

    const TIPPING: &str = r#"{"animations":{
        "animation.hat.rest":{"loop":true,"bones":{"brim":{"rotation":[0,0,0]}}},
        "animation.hat.tip":{"loop":true,"bones":{"brim":{"rotation":[20,0,0]}}}}}"#;

    const TIP: &[u8] = br#"{"format_version":"1.10.0","animation_controllers":{
        "controller.animation.hat.tip":{"states":{
            "default":{"animations":["rest"],"transitions":[{"tipped":"q.is_sneaking"}]},
            "tipped":{"animations":["tip"],"particle_effects":[{"effect":"puff"}],"sound_effects":[{"effect":"ding"}],
                "transitions":[{"default":"!q.is_sneaking"}]}}},
        "controller.animation.hat.idle":{"states":{"default":{"animations":["tip"]}}}}}"#;

    const TIP_FILE: &str = "animation_controllers/hat.json";

    #[test]
    fn controllers_pose_each_wearer_from_its_own_state() {
        let mut scene = Scene::default();
        scene.load_cosmetic(0, hat(), BodySlot::Hat, None, None);
        scene.load_controllers(0, TIPPING, &[(TIP_FILE, TIP)], &["controller.animation.hat.tip"]).unwrap();
        scene.play(0, 0.0);
        let rows = frame(&mut scene, 1.0, &[idle(1).on("is_sneaking"), idle(2)]);
        assert_eq!(rows, [posed_row([20.0, 0.0, 0.0]), posed_row([0.0; 3])]);
        assert_eq!(scene.controller_state(0, 1, 0), Some(("tipped", 0.0)));
        assert_eq!(scene.controller_state(0, 2, 0), Some(("default", 0.0)));
        assert_eq!(scene.controller_state(0, 2, 1), None, "only the declared controller runs");
        assert_eq!(scene.effect_events(), [0, 0, 0, 1, EFFECT_PARTICLE, 0, 0, 0, 0, 1, EFFECT_SOUND, 1]);
        assert_eq!(scene.effect_event_floats(), [CONTROLLER_EVENT_FLOATS; 2].concat());
        frame(&mut scene, 1.5, &[idle(1).on("is_sneaking"), idle(2)]);
        assert_eq!(scene.effect_event_count(), 0, "effects fire on entry only");
        assert_eq!(scene.controller_state(0, 1, 0), Some(("tipped", 0.5)));

        let at = |id, detail| Wearer { detail, ..idle(id).on("is_sneaking") };
        frame(&mut scene, 2.0, &[at(3, Detail::Reduced), at(4, Detail::Minimal)]);
        let sound = [0, 0, 0, 1, EFFECT_SOUND, 1];
        assert_eq!(scene.effect_events(), sound, "reduced drops particles, minimal every effect");
        scene.set_motion(Motion::Reduced);
        frame(&mut scene, 2.0, &[at(5, Detail::Full)]);
        assert_eq!(scene.effect_events(), sound, "so does reduced motion");
        scene.set_motion(Motion::Off);
        frame(&mut scene, 2.0, &[at(6, Detail::Full)]);
        assert_eq!(scene.effect_event_count(), 0, "and motion off drops every effect");
        assert_eq!(scene.controller_state(0, 6, 0), Some(("tipped", 0.0)));

        scene.set_motion(Motion::Full);
        let missing = scene.load_controllers(0, TIPPING, &[(TIP_FILE, TIP)], &["controller.animation.hat.gone"]);
        assert!(missing.unwrap_err().to_string().contains("controller_missing"));
        assert!(scene.load_controllers(0, TIPPING, &[(TIP_FILE, b"{")], &[]).is_err());
        scene.set_query("is_sneaking", 1.0);
        scene.pose(1.0);
        assert_eq!(scene.controller_state(0, PREVIEW_WEARER, 0), Some(("tipped", 0.0)), "the old load stays");
        let empty: &[u8] = br#"{"animation_controllers":{}}"#;
        scene.load_controllers(0, TIPPING, &[(TIP_FILE, empty)], &[]).unwrap();
        scene.play(0, 0.0);
        scene.pose(1.0);
        assert_eq!(brim(&scene), [0.0; 3], "without controllers the first animation plays");
        assert_eq!(scene.controller_state(0, PREVIEW_WEARER, 0), None);
    }

    #[test]
    fn motion_off_shows_the_first_frame_and_reduced_keeps_moving() {
        let sway = r#"{"animations":{"a":{"loop":true,"animation_length":2,
            "bones":{"brim":{"rotation":{"0.0":[10,0,0],"1.0":[40,0,0],"2.0":[10,0,0]}}}}}}"#;
        let (mut full, mut reduced) = (wearer_scene(sway), wearer_scene(sway));
        reduced.set_motion(Motion::Reduced);
        assert_eq!(pose_of(&mut full, 0.5, idle(1)), [25.0, 0.0, 0.0]);
        for clock in [0.75, 1.0, 3.75] {
            assert_eq!(pose_of(&mut reduced, clock, idle(1)), pose_of(&mut full, clock, idle(1)));
        }
        full.set_motion(Motion::Off);
        for clock in [4.5, 5.3, 1_800_000_000.25] {
            assert_eq!(pose_of(&mut full, clock, idle(1)), [10.0, 0.0, 0.0], "at {clock}");
            full.pose(clock as f32);
            assert_eq!(brim(&full), [10.0, 0.0, 0.0], "preview at {clock}");
        }
        assert!(!full.is_animating(), "a still pose needs no redraw");
    }

    fn with_controllers(asset: &mut CosmeticAsset, text: &[u8]) {
        let mut report = Report::new(Mode::Client);
        let files = [("animation_controllers/c.json", text)];
        let loaded = bundle::controllers(&files, &mut asset.interner, Rules::Bedrock, &mut report);
        let mut set = ControllerSet::select(loaded, &[], &mut report);
        set.link(&asset.animations, &asset.interner, &mut report);
        assert!(controller_error(&report).is_none(), "{:?}", report.issues());
        asset.controllers = set;
    }

    fn effects_from(asset: &mut CosmeticAsset, id: &str, kind: &str, extra: Value, components: Value) {
        let mut manifest = json!({"format": 1, "kind": kind, "id": id, "particles": {"puff": {}}});
        manifest.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let puff = json!({"format_version": "1.10.0", "particle_effect": {"components": components,
            "description": {"identifier": format!("{id}:puff"),
                "basic_render_parameters": {"material": "particles_alpha", "texture": "textures/particle/puff"}}}});
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR\0\0\0\x10\0\0\0\x10\x08\x06\0\0\0");
        let files = [
            ("manifest.json".to_string(), serde_json::to_vec(&manifest).unwrap()),
            ("particles/puff.json".to_string(), serde_json::to_vec(&puff).unwrap()),
            ("textures/particle/puff.png".to_string(), png.clone()),
            ("textures/ribbon.png".to_string(), png),
        ];
        let Opened::Bundle(mut bundle) = open_files(&files, &Options::new(Mode::Client)) else {
            panic!("the bundle opens");
        };
        let mut report = Report::new(Mode::Client);
        asset.effects = effects::parse(&mut bundle, asset.model.as_ref(), &mut report);
        effects::bind_locators(&mut asset.effects, &asset.animations, &asset.controllers);
        let solo = effects::Solo::worn(asset.animations.len(), &asset.controllers);
        effects::size_playbacks(&mut asset.effects, &asset.animations, &asset.controllers, &[], &solo);
        effects::size_catch_up(&asset.effects, &asset.animations, &mut asset.controllers);
        assert!(report.issues().iter().all(|i| i.level != Level::Error), "{:?}", report.issues());
    }

    fn burst(lifetime: f32) -> Value {
        json!({"minecraft:emitter_local_space": {"position": true, "rotation": true},
            "minecraft:emitter_rate_instant": {"num_particles": 2},
            "minecraft:emitter_lifetime_once": {"active_time": lifetime},
            "minecraft:emitter_shape_point": {},
            "minecraft:particle_lifetime_expression": {"max_lifetime": 1},
            "minecraft:particle_appearance_billboard": {"size": [0.25, 0.25]}})
    }

    fn glow_effects(asset: &mut CosmeticAsset) {
        let extra = json!({"effects": {"burst": [{"particle": "puff", "locator": "tip"}]},
            "trails": [{"locator": "tip", "texture": "ribbon", "width": 0.5}]});
        effects_from(asset, "glow", "effect", extra, burst(0.5));
    }

    fn spark_effects(asset: &mut CosmeticAsset) {
        let extra = json!({"emitters": [{"particle": "puff", "locator": "tip"}],
            "effects": {"burst": [{"particle": "puff", "locator": "tip"}]},
            "trails": [{"locator": "tip", "texture": "ribbon", "width": 0.5}]});
        let steady = json!({"minecraft:emitter_rate_steady": {"spawn_rate": 10, "max_particles": 40},
            "minecraft:emitter_lifetime_looping": {"active_time": 1},
            "minecraft:emitter_shape_point": {},
            "minecraft:particle_lifetime_expression": {"max_lifetime": 3},
            "minecraft:particle_appearance_billboard": {"size": [0.25, 0.25]}});
        effects_from(asset, "glow", "effect", extra, steady);
    }

    const GLOW: &str = r#"{"format_version":"1.8.0","animations":{"animation.glow.idle":{
        "loop":true,"animation_length":1,"bones":{"fin":{"rotation":[0,0,0]}},
        "particle_effects":{"0.5":{"effect":"burst"}},"timeline":{"0.25":"v.flag = 1;"}}}}"#;

    fn glow(
        animations: &str,
        controller: Option<&[u8]>,
        reactions: Vec<Reaction>,
        effects: Option<fn(&mut CosmeticAsset)>,
    ) -> Arc<CosmeticAsset> {
        let mut asset = CosmeticAsset::new("glow", 11, Kind::Item);
        let mut model = crate::bedrock::parse_one(FIN).unwrap();
        model.bones[0].locators.push(Locator { name: "tip".to_string(), offset: [0.0, 36.0, 0.0], rotation: [0.0; 3] });
        asset.model = Some(model);
        asset.animations = animation::parse(animations, &mut asset.interner).unwrap();
        if let Some(controller) = controller {
            with_controllers(&mut asset, controller);
        }
        if let Some(effects) = effects {
            effects(&mut asset);
        }
        asset.reactions = reactions;
        if reactions::paces(&asset.reactions) {
            asset.controllers.pace();
        }
        asset.palette.colorways.push(Colorway { id: "lit".to_string(), overrides: Vec::new() });
        Arc::new(asset)
    }

    fn reaction(on: On, play: Option<u16>, palette: Option<u16>, effect: Option<u16>) -> Reaction {
        let signal = signal_id("is_sneaking").unwrap() as u8;
        Reaction { signal, on, play, restart: false, blend: 0.0, speed: None, palette, effect }
    }

    fn effect_frame(scene: &mut Scene, clock: f64, rows: &[Wearer]) {
        let sections = FrameSections { root: true, ..FrameSections::default() };
        let (ids, input) = input(&sections, rows);
        let mut out = vec![0.0; scene.output_layout(&sections).stride() * rows.len()];
        scene.frame_wearers_shaped(clock, &ids, &sections, &input, &Affine::IDENTITY, &mut out).unwrap();
    }

    const EYE: View = View { camera: [0.0, 0.0, 40.0], right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], origin_shift: [0.0; 3] };

    fn centers(scene: &mut Scene) -> Vec<(f32, [f32; 3])> {
        let mut out = vec![0.0; 512 * QUAD_FLOATS];
        let n = scene.effect_geometry(&EYE, &mut out).unwrap();
        let center = |q: &[f32]| [0, 1, 2].map(|i| (q[QUAD_CORNERS + i] + q[QUAD_CORNERS + 10 + i]) / 2.0);
        out.chunks_exact(QUAD_FLOATS).take(n).map(|q| (q[QUAD_TEXTURE], center(q))).collect()
    }

    fn events(scene: &Scene) -> Vec<(Vec<u32>, Vec<f32>)> {
        let ints = scene.effect_events().chunks(EFFECT_EVENT_INTS);
        let floats = scene.effect_event_floats().chunks(EFFECT_EVENT_FLOATS);
        ints.zip(floats).map(|(i, f)| (i.to_vec(), f.to_vec())).collect()
    }

    #[test]
    fn a_keyframe_particle_fires_once_and_draws_its_billboards() {
        let mut scene = Scene::default();
        scene.load_asset(0, glow(GLOW, None, Vec::new(), Some(glow_effects))).unwrap();
        scene.play(0, 0.0);
        assert_eq!((scene.has_effects(), scene.effect_texture_count(), scene.effect_texture(1).map(|t| t.0)), (true, 2, Some(0)));
        let mut fired = Vec::new();
        for step in 0..10 {
            effect_frame(&mut scene, step as f64 * 0.1, &[idle(1)]);
            fired.extend(events(&scene).into_iter().map(|e| (step, e)));
        }
        assert_eq!(variable(&mut scene, 1, "flag"), 1.0, "timeline keys run as they are crossed");
        assert_eq!(fired.len(), 1, "{fired:?}");
        let (step, (ints, floats)) = &fired[0];
        assert_eq!(*step, 5);
        assert_eq!(ints, &[0, 0, effects::NO_CONTROLLER, effects::SOURCE_KEYFRAME, EFFECT_PARTICLE, 0]);
        assert_eq!(&floats[3..], &[1.0, 1.0]);
        assert!(floats[1] > 1.0, "{floats:?}");
        let drawn = centers(&mut scene);
        assert_eq!(drawn.len(), 2, "two puffs, the still trail draws nothing");
        let want = [0, 1, 2].map(|i| floats[i] - EYE.camera[i]);
        for (texture, c) in &drawn {
            assert_eq!(*texture, 0.0);
            assert!((0..3).all(|i| (c[i] - want[i]).abs() < 1e-4), "{c:?} {want:?}");
        }
        let error = FrameError::Output { held: QUAD_FLOATS, needed: 2 * QUAD_FLOATS };
        assert_eq!(scene.effect_geometry(&EYE, &mut [0.0; QUAD_FLOATS]), Err(error));
    }

    #[test]
    fn reactions_and_named_fires_play_their_effect_once() {
        let burst = glow(GLOW, None, Vec::new(), Some(glow_effects)).effects.named_id("burst").unwrap();
        let start = reaction(On::Start, None, None, Some(burst));
        let equip = Reaction { signal: EQUIP, ..reaction(On::Start, None, None, Some(burst)) };
        let mut scene = Scene::default();
        scene.load_asset(0, glow(GLOW, None, vec![start, equip], Some(glow_effects))).unwrap();
        let sources = |scene: &Scene| events(scene).iter().map(|(i, _)| i[3]).collect::<Vec<_>>();
        let sneaking = idle(1).on("is_sneaking");
        let equipped = Wearer { events: EVENT_EQUIP, ..sneaking };
        let (reacted, fired) = (&[effects::SOURCE_REACTION][..], &[effects::SOURCE_EVENT][..]);
        let steps: [(&[Wearer], &[u32], &str); 8] = [
            (&[idle(1)], &[], "first sight is not an equip"),
            (&[sneaking], reacted, "a start reaction fires"),
            (&[sneaking], &[], "once"),
            (&[equipped, idle(2)], reacted, "the equip event fires equips"),
            (&[sneaking, idle(2)], &[], "once"),
            (&[sneaking], fired, "a named fire plays"),
            (&[sneaking], &[], "and drains"),
            (&[idle(3)], fired, "a fire waits for its wearer's frame"),
        ];
        assert!(!scene.fire_named(1, "missing"));
        for (n, (rows, want, why)) in steps.into_iter().enumerate() {
            if n == 5 {
                assert!(scene.fire_named(1, "burst") && scene.fire_named(3, "burst"));
            }
            effect_frame(&mut scene, 1.0 + n as f64 * 0.1, rows);
            assert_eq!(sources(&scene), want, "{why}");
        }
        assert_eq!(events(&scene)[0].0, [0, 0, effects::NO_CONTROLLER, effects::SOURCE_EVENT, EFFECT_PARTICLE, 0]);
    }

    const KEYED_ONCE: &str = r#"{"format_version":"1.8.0","animations":{
        "animation.glow.keyed":{"animation_length":2,"bones":{"fin":{"rotation":[0,0,0]}},
            "particle_effects":{"0.0":{"effect":"burst"},"0.5":{"effect":"burst"}},
            "timeline":{"0.0":"v.zero = v.zero + 1;","0.3":"v.third = v.third + 1;","0.5":"v.half = v.half + 1;"}}}}"#;

    const KEYED_LATE: &[u8] = br#"{"format_version":"1.10.0","animation_controllers":{
        "controller.animation.glow.keyed":{"states":{
            "default":{"transitions":[{"keyed":"q.anim_time >= 0.25"}]},"keyed":{"animations":["animation.glow.keyed"]}}}}}"#;

    const AWKWARD_CLOCKS: [(f64, f64); 6] =
        [(1_700_000_000.37, 0.1), (1_700_000_000.05, 0.05), (12_345.678_9, 0.1), (86_400.1, 1.0 / 60.0), (4_000.3, 0.1), (0.3, 0.1)];

    #[test]
    fn clip_keys_cross_once_whatever_the_clock() {
        let keys = ["zero", "third", "half"];
        for (start, step) in AWKWARD_CLOCKS {
            let frames = (0..libm::ceil(1.2 / step) as usize + 3).map(|k| start + step * (k as f64 - 3.0));
            for controller in [None, Some(KEYED_LATE)] {
                let mut scene = Scene::default();
                scene.load_asset(0, glow(KEYED_ONCE, controller, Vec::new(), Some(glow_effects))).unwrap();
                scene.play(0, start);
                let mut bursts = 0;
                for clock in frames.clone() {
                    effect_frame(&mut scene, clock, &[idle(1)]);
                    bursts += events(&scene).iter().filter(|(i, _)| i[3] == effects::SOURCE_KEYFRAME).count();
                }
                let counts = (keys.map(|n| variable(&mut scene, 1, n)), bursts);
                assert_eq!(counts, ([1.0; 3], 2), "{start} {step} {}", controller.is_some());
            }
            let mut asset = CosmeticAsset::new("moments", 4, Kind::Effect);
            asset.animations = animation::parse(&KEYED_ONCE.replace("fin", "head"), &mut asset.interner).unwrap();
            asset.effects.moments = vec![MomentPlan { name: "death".to_string(), clip: Some(0), seconds: 1.0 }];
            let mut scene = emote_scene(Arc::new(asset));
            for clock in frames {
                let play = if clock < start { -1.0 } else { 0.0 };
                one(&mut scene, clock, Wearer { play: [play, (clock - start) as f32], ..idle(1) });
            }
            assert_eq!(keys.map(|n| variable(&mut scene, 1, n)), [1.0; 3], "moment {start} {step}");
        }
    }

    const WAVE_ONCE: &str = r#"{"animations":{"animation.emote.wave":{"animation_length":1.0,
        "bones":{"head":{"rotation":[30,0,0]},"rightArm":{"rotation":[-90,0,0]}}}}}"#;

    const EMOTE_SECTIONS: FrameSections = FrameSections {
        parts: EMOTE_PARTS.len(), queries: 0, root: false, variables: 0, play: true, role: true, colorway: false,
        part_deltas: true, hold: true,
    };

    fn emote_asset(json: &str) -> CosmeticAsset {
        let mut asset = CosmeticAsset::new("wave", 3, Kind::Emote);
        asset.animations = animation::parse(json, &mut asset.interner).unwrap();
        asset.rules = Some(EmoteRules::default());
        asset
    }

    fn emote_scene(asset: Arc<CosmeticAsset>) -> Scene {
        let mut scene = Scene::default();
        scene.load_player(0, Arms::Wide);
        scene.load_asset(0, asset).unwrap();
        for (name, _) in EMOTE_PARTS {
            scene.part_id(name);
        }
        scene
    }

    struct EmoteOut {
        deltas: Vec<[f32; PART_DELTA_FLOATS]>,
        status: u32,
    }

    impl EmoteOut {
        fn weight(&self) -> f32 {
            self.deltas.iter().map(|d| d[9]).fold(0.0, f32::max)
        }
    }

    fn emote_frame(scene: &mut Scene, clock: f64, rows: &[Wearer]) -> Vec<EmoteOut> {
        let (ids, input) = input(&EMOTE_SECTIONS, rows);
        let layout = scene.output_layout(&EMOTE_SECTIONS);
        assert_eq!(layout.stride(), layout.emote_at() + EMOTE_STATUS_FLOATS);
        let mut out = vec![f32::NAN; layout.stride() * rows.len()];
        scene.frame_wearers_shaped(clock, &ids, &EMOTE_SECTIONS, &input, &java::MODEL, &mut out).unwrap();
        let deltas = |row: &[f32]| {
            let parts = row[layout.part_deltas_at()..layout.emote_at()].chunks(PART_DELTA_FLOATS);
            parts.map(|d| d.try_into().unwrap()).collect()
        };
        let rows = out.chunks(layout.stride());
        rows.map(|row| EmoteOut { deltas: deltas(row), status: row[layout.emote_at()] as u32 }).collect()
    }

    fn one(scene: &mut Scene, clock: f64, row: Wearer) -> EmoteOut {
        emote_frame(scene, clock, &[row]).remove(0)
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn emotes_blend_in_and_out_by_their_age_on_every_client() {
        let mut scene = emote_scene(Arc::new(emote_asset(WAVE_ONCE)));
        let arm = |out: &EmoteOut| out.deltas[ARM][3];
        let raised = (-90f32).to_radians();
        let start = one(&mut scene, 50.0, standing(1, 0.0));
        assert_eq!((start.weight(), start.status), (0.0, 0), "nothing shows at age zero");
        let half = one(&mut scene, 50.07, standing(1, 0.07));
        assert!(near(half.weight(), 0.5) && near(arm(&half), raised * 0.5) && half.status == EMOTE_POSING);
        let full = one(&mut scene, 50.5, standing(1, 0.5));
        assert!(full.weight() == 1.0 && near(arm(&full), raised));
        assert_eq!(full.deltas[BODY], REST_DELTA, "untouched parts stay vanilla");
        let mut late = emote_scene(Arc::new(emote_asset(WAVE_ONCE)));
        assert_eq!(one(&mut late, 7000.25, standing(1, 0.5)).deltas, full.deltas, "late clients agree");
        let ending = one(&mut scene, 51.07, standing(1, 1.07));
        assert!(near(ending.weight(), 0.5) && near(arm(&ending), raised * 0.5), "frozen on the last frame");
        assert_eq!(ending.status, EMOTE_POSING | EMOTE_ENDED);
        let done = one(&mut scene, 51.2, standing(1, 1.2));
        assert_eq!((done.weight(), done.status), (0.0, EMOTE_ENDED), "the host stops the emote");
        assert_eq!(one(&mut scene, 51.3, idle(1)).status, 0, "no fade after the end");

        one(&mut scene, 60.5, standing(1, 0.5));
        let stopped = one(&mut scene, 60.57, idle(1));
        assert!(near(stopped.weight(), 1.0) && stopped.status == EMOTE_POSING | EMOTE_STOPPING);
        let fading = one(&mut scene, 60.64, idle(1));
        assert!(near(fading.weight(), 0.5) && near(arm(&fading), raised * 0.5), "holds the pose it stopped on");
        let gone = one(&mut scene, 60.72, idle(1));
        assert_eq!((gone.weight(), gone.status), (0.0, 0));
        assert!(gone.deltas.iter().all(|d| *d == REST_DELTA));
    }

    #[test]
    fn held_emotes_freeze_until_released_and_late_joiners_see_the_end() {
        let rise = r#"{"animations":{"animation.emote.rise":{"animation_length":1.0,
            "bones":{"rightArm":{"rotation":{"0":[0,0,0],"1.0":[-90,0,0]}}},"timeline":{"0.9":"v.rises = v.rises + 1;"}}}}"#;
        let held = |age| Wearer { hold: 1.0, ..standing(1, age) };
        let mut scene = emote_scene(Arc::new(emote_asset(rise)));
        for age in [0.0f32, 0.5, 0.95] {
            one(&mut scene, 10.0 + f64::from(age), standing(1, age));
        }
        let first = one(&mut scene, 11.0, held(1.0));
        assert_eq!((first.weight(), first.status), (1.0, EMOTE_POSING | EMOTE_HELD));
        assert_eq!(one(&mut scene, 15.0, held(5.0)).deltas, first.deltas, "frozen while the clock runs");
        let mut late = emote_scene(Arc::new(emote_asset(rise)));
        assert_eq!(one(&mut late, 400.0, held(1.0)).deltas, first.deltas, "a late joiner sees the end");
        assert_eq!([variable(&mut scene, 1, "rises"), variable(&mut late, 1, "rises")], [1.0, 0.0], "keys fire once");
        let release = one(&mut scene, 15.5, idle(1));
        assert!(near(release.weight(), 1.0) && release.status == EMOTE_POSING | EMOTE_STOPPING);
    }

    type RuleCase = (&'static [&'static str], u32, fn(&mut EmoteRules));

    #[test]
    fn each_rule_cancels_the_emote_it_forbids() {
        let cases: [RuleCase; 8] = [
            (&["is_moving"], EMOTE_CANCEL_WALKING, |r| r.allow_walking = true),
            (&["is_moving", "is_sprinting"], EMOTE_CANCEL_SPRINTING, |r| r.allow_sprinting = true),
            (&["is_sneaking"], EMOTE_CANCEL_CROUCHING, |r| r.allow_crouching = true),
            (&["is_falling"], EMOTE_CANCEL_FALLING, |r| r.allow_falling = true),
            (&["is_gliding"], EMOTE_CANCEL_ELYTRA_FLYING, |r| r.allow_elytra_flying = true),
            (&["is_swimming"], EMOTE_CANCEL_SWIMMING, |r| r.allow_swimming = true),
            (&["is_sleeping"], EMOTE_CANCEL_SLEEPING, |_| {}),
            (&[], EMOTE_CANCEL_SWINGING, |_| {}),
        ];
        for (signals, bit, allow) in cases {
            let mut row = signals.iter().fold(standing(1, 0.5), |row, name| row.on(name));
            row.events = if bit == EMOTE_CANCEL_SWINGING { EVENT_ATTACK_SWING } else { 0 };
            let mut rules = EmoteRules { allow_falling: false, ..EmoteRules::default() };
            rules.allow_walking = bit == EMOTE_CANCEL_SPRINTING;
            for lenient in [false, true] {
                if lenient {
                    allow(&mut rules);
                }
                let mut asset = emote_asset(WAVE_ONCE);
                asset.rules = Some(rules);
                let status = one(&mut emote_scene(Arc::new(asset)), 1.0, row).status;
                let cancels = !lenient || bit == EMOTE_CANCEL_SLEEPING || bit == EMOTE_CANCEL_SWINGING;
                assert_eq!(status, EMOTE_POSING | if cancels { EMOTE_CANCEL | bit } else { 0 }, "{signals:?} {lenient}");
            }
        }
    }

    #[test]
    fn emote_props_load_beside_the_player_and_ride_its_posed_bones() {
        let props = r#"{"format_version":"1.12.0","minecraft:geometry":[{
            "description":{"identifier":"geometry.think","texture_width":16,"texture_height":16},
            "bones":[{"name":"head","pivot":[0,24,0]},{"name":"spare","pivot":[0,0,0]},
              {"name":"bubble","parent":"head","pivot":[0,32,0],"locators":{"tip":[0,34,0]},"cubes":[{"origin":[-1,32,-1],"size":[2,2,2],"uv":[0,0]}]}]}]}"#;
        let think = r#"{"animations":{"animation.think":{"loop":true,"animation_length":1.0,
            "bones":{"body":{"rotation":[20,0,0]},"head":{"rotation":[30,0,0]},"bubble":{"scale":[2,2,2]}}}}}"#;
        let mut asset = emote_asset(think);
        asset.model = Some(crate::bedrock::parse_one(props).unwrap());
        let mut scene = Scene::default();
        scene.load_player(0, Arms::Wide);
        scene.load_emote(0, Some(1), Arc::new(asset)).unwrap();
        for (name, _) in EMOTE_PARTS {
            scene.part_id(name);
        }
        assert_eq!(scene.player_slot, Some(0));
        let names: Vec<&str> = scene.slots[1].model.as_ref().unwrap().bones.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["bubble"], "only geometry that hangs from the player");
        let out = one(&mut scene, 3.0, standing(1, 0.5));
        assert!(near(out.deltas[HEAD][3], 50f32.to_radians()), "the head rides the body's bow");
        let (head, _) = compose(RigKind::Player, &scene.game_parts, "head").unwrap();
        let pivot = scene.slots[1].model.as_ref().unwrap().bones[0].pivot;
        let expected = head.mul(&Affine::about(pivot, &Affine::scale([2.0; 3])));
        let bubble = scene.slots[1].transforms[0];
        assert!(bubble.0.iter().zip(expected.0).all(|(a, b)| (a - b).abs() < 1e-4), "{bubble:?}");
    }

    fn joiner_asset(clips: &str, controllers: &str) -> Arc<CosmeticAsset> {
        let mut asset = emote_asset(&format!(r#"{{"format_version":"1.8.0","animations":{{{clips}}}}}"#));
        let controllers = format!(r#"{{"format_version":"1.10.0","animation_controllers":{{{controllers}}}}}"#);
        with_controllers(&mut asset, controllers.as_bytes());
        let extra = json!({"sounds": {"chime": {"vanilla": "minecraft:block.note_block.chime"}},
            "effects": {"burst": [{"particle": "puff", "locator": "right_hand"}, {"particle": "puff", "locator": "head"}]}});
        effects_from(&mut asset, "joiner", "emote", extra, burst(0.05));
        Arc::new(asset)
    }

    fn seen_by(scene: &mut Scene, variables: &[&str], clock: f64, wearer: u64, start: f64) -> String {
        one(scene, clock, standing(wearer, (clock - start) as f32));
        let count = scene.slots[0].controllers.as_ref().map_or(0, |c| c.controllers().len());
        let states: Vec<_> = (0..count).map(|c| scene.controller_state(0, wearer, c).unwrap().0.to_string()).collect();
        let variables: Vec<_> = variables.iter().map(|name| variable(scene, wearer, name)).collect();
        format!("{states:?} {variables:?} {} quads", centers(scene).len())
    }

    const FIXTURES: [(&str, &str, &[&str]); 7] = [
        (r#""animation.joiner.gate":{"animation_length":1,"bones":{"head":{"rotation":[5,0,0]}},
                "timeline":{"0.1":"v.off = 1;","0.2":"v.count = v.count + 1;"}}"#,
            r#""controller.animation.joiner":{"states":{"default":{"animations":[{"gate":"1 - v.off"}]}}}"#,
            &["off", "count"]),
        (r#""animation.joiner.zero":{"animation_length":0.5,"bones":{"head":{"rotation":[5,0,0]}},
                "timeline":{"0.0":"v.seen = v.seen + v.live;"},"particle_effects":{"0.0":{"effect":"burst"}}}"#,
            r#""controller.animation.joiner":{"states":{"default":{"transitions":[{"mid":"q.anim_time > 0.27"}]},
                "mid":{"on_entry":["v.live = 1;"],"on_exit":["v.live = 0;"],"animations":["zero"],"transitions":[{"end":"1"}]},
                "end":{"animations":["zero"],"transitions":[{"default":"q.anim_time > 0.62"}]}}}"#,
            &["live", "seen"]),
        (r#""animation.joiner.tick":{"loop":true,"animation_length":0.4,"bones":{"head":{"rotation":[5,0,0]}},
                "timeline":{"0.2":"v.ticks = v.ticks + 1;"},"particle_effects":{"0.3":{"effect":"burst"}}}"#,
            r#""controller.animation.joiner":{"states":{"default":{"animations":["tick"],"transitions":[{"rest":"v.ticks >= 3"}]},
                "rest":{"particle_effects":[{"effect":"burst"}],"on_entry":["v.rests = v.rests + 1;"]}}}"#,
            &["ticks", "rests"]),
        (r#""animation.joiner.flag":{"animation_length":1,"bones":{"head":{"rotation":[5,0,0]}},"timeline":{"0.35":"v.go = 1;"}},
                "animation.joiner.spark":{"animation_length":1,"bones":{"head":{"rotation":[0,5,0]}},
                "particle_effects":{"0.1":{"effect":"burst"}},"timeline":{"0.2":"v.sparks = v.sparks + 1;"}}"#,
            r#""controller.animation.joiner.a":{"states":{"default":{"animations":["flag"]}}},
                "controller.animation.joiner.b":{"states":{"default":{"transitions":[{"lit":"v.go"}]},
                "lit":{"animations":["spark"],"transitions":[{"done":"q.anim_time > 0.32"}]},"done":{}}}"#,
            &["go", "sparks"]),
        (r#""animation.joiner.wave":{"loop":true,"animation_length":0.3,"bones":{"head":{"rotation":[5,0,0]}},
                "particle_effects":{"0.15":{"effect":"burst"}},"timeline":{"0.15":"v.waves = v.waves + 1;"}}"#,
            r#""controller.animation.joiner":{"states":{"default":{"animations":["wave"],"blend_transition":0.4,
                "transitions":[{"calm":"q.anim_time > 0.52"}]},"calm":{"transitions":[{"default":"q.anim_time > 0.93"}]}}}"#,
            &["waves"]),
        (r#""animation.joiner.pace":{"loop":true,"animation_length":0.5,"bones":{"head":{"rotation":[5,0,0]}},
                "anim_time_update":"q.anim_time + q.delta_time * 1.5",
                "particle_effects":{"0.25":{"effect":"burst"}},"timeline":{"0.4":"v.paces = v.paces + 1;"}}"#,
            r#""controller.animation.joiner":{"states":{"default":{"animations":["pace"],"transitions":[{"stop":"v.paces >= 4"}]},"stop":{}}}"#,
            &["paces"]),
        (r#""animation.joiner.late":{"animation_length":0.5,"start_delay":0.33,"bones":{"head":{"rotation":[5,0,0]}},
                "particle_effects":{"0.0":{"effect":"burst"}},"timeline":{"0.0":"v.late = v.late + 1;","0.2":"v.after = v.late * 2;"}}"#,
            r#""controller.animation.joiner":{"states":{"default":{"animations":["late"],"transitions":[{"next":"q.all_animations_finished"}]},
                "next":{"transitions":[{"default":"q.anim_time > 0.22"}]}}}"#,
            &["late", "after"]),
    ];

    #[test]
    fn late_joiners_see_what_a_continuous_viewer_sees() {
        let mut wrong = Vec::new();
        let starts = [(20.0, &[0.12, 0.42, 0.83, 1.38, 2.21, 7.88, 30.17][..]), (AWKWARD_CLOCKS[0].0, &[0.27, 1.62, 3.4][..])];
        for (n, (clips, controllers, variables)) in FIXTURES.iter().enumerate() {
            for &(start, joins) in &starts {
                for &join in joins {
                    let mut scene = emote_scene(joiner_asset(clips, controllers));
                    let mut frame = 0u32;
                    while start + f64::from(frame) / 20.0 < start + join {
                        let clock = start + f64::from(frame) / 20.0;
                        one(&mut scene, clock, standing(1, (clock - start) as f32));
                        frame += 1;
                    }
                    let watched = seen_by(&mut scene, variables, start + join, 1, start);
                    let joined = seen_by(&mut scene, variables, start + join, 2, start);
                    if joined != watched {
                        wrong.push(format!("fixture {n} from {start} at {join}: {watched} vs {joined}"));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    fn heavy(name: &str, kind: Kind) -> CosmeticAsset {
        let mut asset = CosmeticAsset::new(name, 7, kind);
        let clips = r#"{"animations":{
            "animation.heavy.arm":{"loop":true,"animation_length":0.5,"bones":{"rightArm":{"rotation":[-90,0,0]}}},
            "animation.heavy.nod":{"loop":true,"animation_length":1.0,"bones":{"head":{"rotation":[30,0,0]}}}}}"#;
        asset.animations = animation::parse(clips, &mut asset.interner).unwrap();
        let terms = vec!["v.k"; 40].join(" + ");
        let controller = |n| {
            format!(
                r#""controller.animation.heavy.c{n}":{{"initial_state":"default","states":{{
                "default":{{"animations":["arm"],"transitions":[{{"done":"{terms} == 0 && q.anim_time > 30.0"}}]}},
                "done":{{"animations":["nod"]}}}}}}"#
            )
        };
        let all: Vec<String> = (0..MAX_CONTROLLERS).map(controller).collect();
        let text = format!(r#"{{"format_version":"1.10.0","animation_controllers":{{{}}}}}"#, all.join(","));
        with_controllers(&mut asset, text.as_bytes());
        assert_eq!(asset.controllers.controllers().len(), MAX_CONTROLLERS);
        asset
    }

    #[test]
    fn warm_crowd_frames_allocate_nothing() {
        let glow_controller: &[u8] = br#"{"format_version":"1.10.0","animation_controllers":{"controller.animation.glow":{"states":{
            "default":{"animations":["idle"],"transitions":[{"lit":"q.is_sneaking"}]},
            "lit":{"particle_effects":[{"effect":"burst"}],"transitions":[{"default":"!q.is_sneaking"}]}}}}}"#;
        let reactions = vec![reaction(On::Start, Some(0), Some(0), None)];
        let mut worn = glow(GLOW, Some(glow_controller), reactions, Some(spark_effects));
        let asset = Arc::get_mut(&mut worn).unwrap();
        let model = asset.model.as_mut().unwrap();
        model.bones[0].spring = Some(crate::ir::SpringParams::default());
        asset.springs = springs::build(model, &mut Report::new(Mode::Client));
        assert!(!asset.springs.is_empty());
        let mut pet = heavy("heavy_pet", Kind::Pet);
        (pet.model, pet.roles) = (Some(crate::bedrock::parse_one(FIN).unwrap()), duet().roles);
        let mut emote = heavy("heavy_emote", Kind::Emote);
        emote.rules = Some(EmoteRules::default());
        let mut scene = emote_scene(Arc::new(emote));
        scene.load_asset(1, worn).unwrap();
        scene.load_asset(3, Arc::new(pet)).unwrap();
        scene.play(1, 0.0);
        scene.set_effect_colors(2, Some(0), &[(0, 0xff_0000)]);

        let sections = FrameSections { root: true, ..EMOTE_SECTIONS };
        let ids: Vec<u64> = (1..=32).collect();
        let inputs: Vec<Vec<f32>> = (0..240)
            .map(|frame| {
                let t = frame as f32 / 60.0;
                let row = |(n, &id): (usize, &u64)| {
                    let mut row = if n % 3 == 0 { standing(id, t) } else { idle(id) };
                    row.raw[signal_id("is_sneaking").unwrap()] = ((frame / 40 + n) % 2) as f32;
                    row.events = if n % 3 == 2 && frame % 50 == n { EVENT_HURT } else { 0 };
                    row.detail = [Detail::Full, Detail::Reduced][usize::from(n % 7 == 6)];
                    row.at = [n as f32 * 2.0 + t * 3.0, 0.0, (n % 4) as f32];
                    row.role[0] = [-1.0, 0.0][usize::from(n % 3 == 1)];
                    row.hold = f32::from(n % 6 == 3 && frame > 60);
                    row
                };
                let rows: Vec<Wearer> = ids.iter().enumerate().map(row).collect();
                input(&sections, &rows).1
            })
            .collect();
        let layout = scene.output_layout(&sections);
        let (mut out, mut quads, mut drawn) = (vec![0.0; layout.stride() * ids.len()], vec![0.0; 8192 * QUAD_FLOATS], 0);
        let mut run = |scene: &mut Scene, frames: std::ops::Range<usize>| {
            for frame in frames {
                if frame % 20 == 0 {
                    scene.fire_named(1 + (frame % 32) as u64, "burst");
                }
                let clock = 10.0 + frame as f64 / 60.0;
                scene.frame_wearers_shaped(clock, &ids, &sections, &inputs[frame], &java::MODEL, &mut out).unwrap();
                drawn = scene.effect_geometry(&EYE, &mut quads).unwrap();
            }
        };
        run(&mut scene, 0..120);
        let count = crate::test_alloc::allocations_in(|| run(&mut scene, 120..240));
        assert!(drawn > 100, "{drawn}");
        assert!(out.chunks(layout.stride()).any(|row| row[layout.emote_at()] != 0.0));
        let record = scene.wearers.record(scene.slots[1].cosmetic.unwrap(), 5).unwrap();
        assert_ne!(record.spring_header[SPRING_CLOCK], TIME_NEVER);
        assert_eq!(count, 0);
    }
}
