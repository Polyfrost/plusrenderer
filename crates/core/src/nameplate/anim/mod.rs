pub mod channels;
pub mod clock;
pub mod particles;
pub mod path;
pub mod repeat;
pub mod schedule;
pub mod speed;
pub mod tracks;
pub mod triggers;
pub mod walker;

use crate::cost::{features, AnimationRate, Detail, Motion};
use crate::wearer::{seed, WearerRecord};

use super::palette;
use super::types::{Clip, NameplateAsset, PaletteTrack, PaletteValue, Phase, TagPose, Variant};
use tracks::Channel;
use triggers::Edges;

pub const SEED_PURPOSE: u64 = 0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TagTime {
    pub engine: u32,
    pub offset: u32,
    pub loop_frame: u16,
    pub cycle: u32,
    pub seed: u64,
}

impl TagTime {
    pub fn at(engine: u32, loop_frames: u16, phase: Phase, seed: u64) -> TagTime {
        let offset = clock::phase_offset(phase, seed, loop_frames);
        TagTime {
            engine,
            offset,
            loop_frame: clock::loop_frame(engine, offset, loop_frames),
            cycle: clock::cycle(engine, loop_frames),
            seed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Moment {
    pub tag: TagTime,
    pub absolute: i64,
    pub live: i64,
    pub millis: i64,
    pub fps: u8,
    pub loop_frames: u16,
}

impl Moment {
    pub fn new(
        clock: f64,
        rate: AnimationRate,
        fps: u8,
        loop_frames: u16,
        phase: Phase,
        seed: u64,
    ) -> Moment {
        let fps = fps.max(1);
        let wrap = clock::wrap_period(loop_frames);
        let shown = if clock.is_finite() {
            rate.time(clock)
        } else {
            0.0
        };
        let absolute = clock::absolute_frame(shown, fps);
        let engine = absolute.rem_euclid(wrap) as u32;
        Moment {
            tag: TagTime::at(engine, loop_frames, phase, seed),
            absolute,
            live: clock::absolute_frame(clock, fps),
            millis: clock::millis(clock),
            fps,
            loop_frames,
        }
    }

    pub fn of(asset: &NameplateAsset, wearer: u64, clock: f64, gates: &Gates) -> Moment {
        Moment::new(
            clock,
            gates.rate,
            asset.fps,
            asset.loop_frames,
            asset.phase,
            seed(wearer, asset.identity, SEED_PURPOSE),
        )
    }

    pub fn wrap(&self) -> i64 {
        clock::wrap_period(self.loop_frames)
    }

    pub fn live_engine(&self) -> u32 {
        self.live.rem_euclid(self.wrap()) as u32
    }

    pub fn unwrap(&self, engine: u32) -> i64 {
        self.live - (i64::from(self.live_engine()) - i64::from(engine)).rem_euclid(self.wrap())
    }

    fn frame_start_ms(&self, frame: i64) -> i64 {
        let fps = i64::from(self.fps.max(1));
        (frame.saturating_mul(1000) + fps - 1).div_euclid(fps)
    }

    pub fn stamp(&self, now: f32, frame: i64) -> f32 {
        let lo = self.frame_start_ms(frame);
        let hi = self.frame_start_ms(frame.saturating_add(1));
        let middle = lo + (hi - lo) / 2;
        now + ((middle - self.millis) as f64 / 1000.0) as f32
    }

    pub fn frame_of(&self, now: f32, stamp: f32) -> i64 {
        let offset = libm::floor(f64::from(stamp - now) * 1000.0 + 0.5);
        let offset = if offset.is_finite() { offset as i64 } else { 0 };
        self.millis
            .saturating_add(offset)
            .saturating_mul(i64::from(self.fps.max(1)))
            .div_euclid(1000)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipTime {
    pub clip: ClipRef,
    pub frame: i32,
    pub start: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tail {
    pub time: ClipTime,
    pub until: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipRef {
    Loop,
    Schedule { schedule: u16, event: u16 },
    Reaction(u16),
    OneShot(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Gates {
    pub rate: AnimationRate,
    pub first_frame: bool,
    pub hold_reactions: bool,
    pub palette: bool,
    pub plate: bool,
    pub letters: bool,
    pub particles: bool,
    pub paths: bool,
    pub effects: bool,
}

impl Gates {
    pub fn new(motion: Motion, detail: Detail) -> Gates {
        let f = features(detail, motion);
        let full = motion == Motion::Full;
        Gates {
            rate: f.animation,
            first_frame: f.animation == AnimationRate::FirstFrame,
            hold_reactions: !full,
            palette: f.palette_animation,
            plate: full,
            letters: f.letter_motion,
            particles: f.particles,
            paths: f.nameplate_paths && full,
            effects: f.effects,
        }
    }
}

impl Default for Gates {
    fn default() -> Gates {
        Gates::new(Motion::Full, Detail::Full)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scratch {
    pub playing: Vec<ClipTime>,
    pub shown: Vec<ClipTime>,
    pub tails: Vec<Tail>,
}

pub fn clip(asset: &NameplateAsset, clip: ClipRef) -> Option<&Clip> {
    palette::clip(asset, clip)
}

#[allow(clippy::too_many_arguments)]
pub fn pose(
    asset: &NameplateAsset,
    variant: u16,
    record: &mut WearerRecord<'_>,
    moment: &Moment,
    equipped: bool,
    gates: &Gates,
    values: &dyn Fn(u32, i32) -> f64,
    colors: &dyn Fn(u32, i32) -> u32,
    scratch: &mut Scratch,
    out: &mut TagPose,
) -> Edges {
    *out = TagPose::default();
    let index = if asset.variant(variant).is_some() {
        variant
    } else {
        0
    };
    let Some(chosen) = asset.variant(index) else {
        return Edges::default();
    };
    let equipped = triggers::variant_changed(asset, record, index) || equipped;
    let edges = triggers::update(asset, record, moment, equipped, &mut scratch.playing);
    triggers::tails(asset, record, moment, &mut scratch.tails);
    channels::shown(asset, &scratch.playing, gates, &mut scratch.shown);
    let sample =
        |program: u32, frame: i32, channel: Channel| channel.convert(values(program, frame));
    channels::pose_layers(
        asset,
        chosen,
        &scratch.shown,
        &sample,
        gates,
        record.disabled,
        out,
    );
    animate_palette(asset, chosen, &scratch.shown, colors, gates, out);
    edges
}

pub fn animate_palette(
    asset: &NameplateAsset,
    variant: &Variant,
    shown: &[ClipTime],
    colors: &dyn Fn(u32, i32) -> u32,
    gates: &Gates,
    out: &mut TagPose,
) {
    let frame_of = |program: u32| {
        shown
            .iter()
            .find(|t| {
                clip(asset, t.clip).is_some_and(|c| {
                    c.palette.iter().any(|track| match &track.track {
                        PaletteTrack::Keys(keys) => keys
                            .iter()
                            .any(|k| k.tones.contains(&PaletteValue::Molang(program))),
                        PaletteTrack::Cycle { .. } => false,
                    })
                })
            })
            .map_or(0, |t| t.frame)
    };
    let lookup = |program: u32| colors(program, frame_of(program));
    palette::animate(
        asset,
        variant,
        shown,
        &lookup,
        gates.palette,
        &mut out.palette,
    );
}

#[cfg(test)]
pub(crate) mod fixture {
    use crate::nameplate::geom::{Anchor, Edge};
    use crate::nameplate::types::{
        Animations, Body, Depth, Emitter, Layer, LayerContent, NameplateAsset, Of, ParticleFrames, Phase, Spawn, SpawnShape, Sprite, SpriteLayer, Trigger, Variant,
    };
    use crate::wearer::{WearerDeclarations, MAX_ONE_SHOTS};

    use super::triggers::tail_slots;

    pub fn body(id: &str) -> Layer {
        let content = LayerContent::Body(Body::default());
        Layer { id: id.into(), depth: Depth::Background, step: 1, mask: None, of: Of::Block, mirror: None, breakpoints: Vec::new(), content }
    }

    pub fn sprite(id: &str, sprite: u16, step: u16) -> Layer {
        let (anchor, pivot) = (Anchor::default(), Anchor::default());
        let layer = SpriteLayer { sprite, anchor, at: None, pivot, offset: [0, 0], timeline: Vec::new(), follow: None, path: None, repeat: None };
        Layer { step, content: LayerContent::Sprite(layer), ..body(id) }
    }

    pub fn emitter(id: &str, lifetime: u16) -> Layer {
        let spawn = Spawn { shape: SpawnShape::Point, anchor: Anchor::default(), edge: Edge::Top, offset: [0, 0], follow: None };
        let (velocity_x, velocity_y, gravity, drift) = ([0, 0], [0, 0], [0, 0], [0, 0]);
        let emitter = Emitter { sprites: vec![0], spawn, rate: None, lifetime: [lifetime; 2], velocity_x, velocity_y, gravity, drift, wobble: None, frames: ParticleFrames::Loop, fade: None };
        Layer { content: LayerContent::Emitter(emitter), ..body(id) }
    }

    pub fn frames(id: &str, count: usize) -> Sprite {
        Sprite { id: id.into(), width: 1, height: 1, colors: Vec::new(), frames: vec![vec![0]; count] }
    }

    pub fn asset(fps: u8, loop_frames: u16, layers: Vec<Layer>, sprites: Vec<Sprite>, animations: Animations) -> NameplateAsset {
        let clips = animations.reactions.iter().map(|r| &r.clip).chain(animations.one_shots.iter().map(|o| &o.clip));
        let speeds = clips.filter(|c| c.targets.iter().any(|t| t.channels.speed.is_some())).count();
        let equip = animations.one_shots.iter().any(|o| matches!(o.trigger, Trigger::Equip));
        let (one_shots, effect_times, effect_floats) = (animations.one_shots.len().min(MAX_ONE_SHOTS), tail_slots(&animations), 2 * speeds + usize::from(equip));
        let record = WearerDeclarations { one_shots, effect_times, effect_floats, channels: layers.len(), ..WearerDeclarations::default() };
        let variants = ["a", "b"].map(|key| Variant { key: key.into(), ..Variant::default() }).to_vec();
        NameplateAsset {
            id: "anim".into(), identity: 0x5eed, fps, loop_frames, phase: Phase::Global, params: Vec::new(), palette: Default::default(), variants, layers,
            groups: Vec::new(), sprites, flags: Vec::new(), materials: Vec::new(), textures: Vec::new(), animations, text: Default::default(),
            badge: Default::default(), molang: Default::default(), record, max_out_words: 0, requires: Vec::new(), fixed_colors: Vec::new(), breakpoint_edges: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_time_stamps_round_trip_across_rates_phases_and_the_frame_wrap() {
        for (fps, ms) in [1u8, 7, 24].into_iter().flat_map(|fps| [0i64, 41, 999, 1_764_000_000_123].map(|ms| (fps, ms))) {
            let time = Moment::new(ms as f64 / 1000.0, AnimationRate::EveryFrame, fps, 12, Phase::Global, 0);
            for (now, back) in [0.0f32, 17.25, 4095.5].into_iter().flat_map(|now| [0i64, 3, 600].map(|back| (now, back))) {
                assert_eq!(time.frame_of(now, time.stamp(now, time.live - back)), time.live - back, "{fps} {ms} {now} {back}");
            }
        }
        let at = |clock, rate, phase, seed| Moment::new(clock, rate, 24, 12, phase, seed);
        let (real, held) = (at(100.35, AnimationRate::EveryFrame, Phase::Global, 0), at(100.35, AnimationRate::Hz(10), Phase::Global, 0));
        assert_eq!((real.absolute, held.absolute, held.live), (2408, 2407, real.live));
        assert_eq!(at(100.35, AnimationRate::FirstFrame, Phase::Global, 0).absolute, 0);
        assert_eq!(at(100.35, AnimationRate::EveryFrame, Phase::PerPlayer, 7).tag.loop_frame, ((2408 + 7) % 12) as u16);
        let bad = at(f64::NAN, AnimationRate::EveryFrame, Phase::Global, 0);
        assert_eq!((bad.tag.engine, bad.absolute, bad.live), (0, 0, 0));
        let wrap = clock::wrap_period(12);
        let frame = |f: i64| at((f as f64 + 0.5) / 24.0, AnimationRate::EveryFrame, Phase::Global, 0);
        let (before, after) = (frame(wrap - 3), frame(wrap + 4));
        assert_eq!((after.tag.engine, after.unwrap(before.live_engine())), (4, wrap - 3));
    }

    #[test]
    fn gates_follow_motion_and_detail() {
        let gates = |motion, detail| {
            let g = Gates::new(motion, detail);
            [g.palette, g.plate, g.letters, g.particles, g.paths, g.first_frame, g.hold_reactions, g.effects]
        };
        assert_eq!(gates(Motion::Full, Detail::Full), [true, true, true, true, true, false, false, true]);
        assert_eq!(gates(Motion::Reduced, Detail::Full), [true, false, false, false, false, false, true, true]);
        assert_eq!(gates(Motion::Off, Detail::Full)[..2], [false, false]);
        assert!(gates(Motion::Off, Detail::Full)[5]);
        assert_eq!(gates(Motion::Full, Detail::Reduced), [true, true, true, false, false, false, false, true]);
        assert_eq!(gates(Motion::Full, Detail::Minimal)[7], false);
        assert_eq!((Gates::new(Motion::Full, Detail::Minimal).rate, Gates::default()), (AnimationRate::Hz(10), Gates::new(Motion::Full, Detail::Full)));
    }
}
