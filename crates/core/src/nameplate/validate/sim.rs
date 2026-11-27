use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use crate::molang::Runtime;
use crate::signals::SIGNAL_COUNT;

use super::super::anim::channels::{pose_layers, shown as shown_clips};
use super::super::anim::particles::{
    emit_in, looped_starts, route_key, run_starts_until, SpawnArea,
};
use super::super::anim::path::Walk;
use super::super::anim::speed::{
    primary_layer, read_episode, rejoin_frame, speed_clips, speed_track, track, ClipClock, Episode,
    Profile, EPISODE_FLOATS,
};
use super::super::anim::tracks::Channel;
use super::super::anim::triggers::{burst_life, KEPT_RUNS};
use super::super::anim::{animate_palette, clip as anim_clip, ClipRef, ClipTime, Gates, TagTime};
use super::super::geom::{Align, BlockInput, LayoutResult, Placement, Rect, CANVAS};
use super::super::image::{LayerImage, Painter};
use super::super::layout::{build_block, resolve, MAX_BLOCK_WIDTH, REFERENCE_WIDTHS};
use super::super::paint::paint_layer;
use super::super::queries::{NameplateQuery, ParamQueries, QuerySlots};
use super::super::raster::{
    composite, composite_within, draws, layer_walk, Canvas, Colors, Draw, PathSpeed, Scene,
};
use super::super::route::build;
use super::super::text::run_letters;
use super::super::types::{
    Body, Breakpoint, Burst, LayerContent, LetterPose, NameplateAsset, Of, ParticlePose, TagPose,
    Target, TextMode, Variant,
};
use super::cover::lanes;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Loop,
    Reaction(u8),
    OneShot(u16),
    Schedule(u16),
    Everything,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Event {
    pub clip: ClipRef,
    pub start: i64,
    pub length: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    pub source: Source,
    pub signals: Vec<u8>,
    pub sample: SignalSample,
    pub events: Vec<Event>,
    pub restart: Option<u8>,
    pub flip: Option<i64>,
    pub frames: u32,
    pub seed: u64,
    pub ahead: u32,
}

impl Scenario {
    pub fn idle(frames: u32, seed: u64) -> Scenario {
        Scenario {
            source: Source::Loop,
            signals: Vec::new(),
            sample: SignalSample::NEUTRAL,
            events: Vec::new(),
            restart: None,
            flip: None,
            frames,
            seed,
            ahead: 0,
        }
    }

    pub fn time(&self, frame: u32, asset: &NameplateAsset) -> TagTime {
        let mut time = TagTime::at(frame, asset.loop_frames, asset.phase, self.seed);
        time.offset = time.offset.saturating_add(self.ahead);
        time
    }

    pub fn held(source: Source, signals: Vec<u8>, frames: u32, seed: u64) -> Scenario {
        Scenario {
            source,
            signals,
            ..Scenario::idle(frames, seed)
        }
    }

    pub fn signals_at(&self, frame: u32, fps: u8) -> SignalSample {
        let mut sample = self.sample;
        for signal in &self.signals {
            if let Some(value) = sample.values.get_mut(usize::from(*signal)) {
                *value = 1.0;
            }
        }
        let fps = f32::from(fps.max(1));
        let mut sample = sample.after(frame as f32 / fps);
        if let Some(signal) = self.restart {
            let own = match self.source {
                Source::OneShot(index) => Some(ClipRef::OneShot(index)),
                _ => None,
            };
            let latest = self
                .events
                .iter()
                .filter(|e| own.is_none_or(|clip| e.clip == clip))
                .map(|e| e.start)
                .filter(|start| *start <= i64::from(frame))
                .max();
            let index = usize::from(signal);
            if let (Some(start), true) = (latest, index < SIGNAL_COUNT) {
                let since = i64::from(frame) - start;
                sample.times[index] = since as f32 / fps;
                if let Some(after) = self.flip.filter(|after| since >= *after) {
                    let held = sample.values[index] != 0.0;
                    sample.values[index] = if held { 0.0 } else { 1.0 };
                    sample.times[index] = (since - after) as f32 / fps;
                }
            }
        }
        sample
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalSample {
    pub values: [f32; SIGNAL_COUNT],
    pub times: [f32; SIGNAL_COUNT],
}

impl SignalSample {
    pub const NEUTRAL: SignalSample = SignalSample {
        values: [0.0; SIGNAL_COUNT],
        times: [0.0; SIGNAL_COUNT],
    };

    pub fn after(&self, seconds: f32) -> SignalSample {
        let mut sample = *self;
        for time in &mut sample.times {
            *time += seconds;
        }
        sample
    }
}

const EXHAUSTIVE_CASES: usize = 64;

pub struct SignalCase {
    pub sample: SignalSample,
    pub changed: usize,
}

pub fn signal_cases(values: &[usize], times: &[usize]) -> Vec<SignalCase> {
    let lanes = lanes(values, times);
    let mut choices: Vec<Vec<usize>> = Vec::new();
    let total = lanes
        .iter()
        .try_fold(1usize, |n, lane| n.checked_mul(lane.samples.len()))
        .filter(|n| *n <= EXHAUSTIVE_CASES);
    if let Some(total) = total {
        for mut index in 0..total {
            let mut choice = Vec::with_capacity(lanes.len());
            for lane in &lanes {
                choice.push(index % lane.samples.len());
                index /= lane.samples.len();
            }
            choices.push(choice);
        }
        choices.sort_by_key(|choice| choice.iter().filter(|s| **s != 0).count());
    } else {
        let none = vec![0; lanes.len()];
        choices.push(none.clone());
        for (i, lane) in lanes.iter().enumerate() {
            for s in 1..lane.samples.len() {
                let mut choice = none.clone();
                choice[i] = s;
                choices.push(choice);
            }
        }
        choices.push(vec![1; lanes.len()]);
        for (i, first) in lanes.iter().enumerate() {
            for (j, second) in lanes.iter().enumerate().skip(i + 1) {
                for a in 1..first.samples.len() {
                    for b in 1..second.samples.len() {
                        let mut choice = none.clone();
                        choice[i] = a;
                        choice[j] = b;
                        choices.push(choice);
                    }
                }
            }
        }
    }
    choices
        .iter()
        .map(|choice| {
            let mut sample = SignalSample::NEUTRAL;
            for (lane, pick) in lanes.iter().zip(choice) {
                let into = if lane.time {
                    &mut sample.times
                } else {
                    &mut sample.values
                };
                into[lane.id] = lane.samples[*pick];
            }
            SignalCase {
                sample,
                changed: choice.iter().filter(|s| **s != 0).count(),
            }
        })
        .collect()
}

pub fn read_signals(asset: &NameplateAsset) -> (Vec<usize>, Vec<usize>) {
    let slots = QuerySlots::with_params(&asset.molang.interner, &asset.params);
    let mut values = Vec::new();
    let mut times = Vec::new();
    for entry in &asset.molang.programs {
        let reads = &entry.program.reads().queries;
        for id in 0..SIGNAL_COUNT {
            if slots.signal(id).is_some_and(|s| reads.contains(&s)) && !values.contains(&id) {
                values.push(id);
            }
            if slots.time(id).is_some_and(|s| reads.contains(&s)) && !times.contains(&id) {
                times.push(id);
            }
        }
    }
    values.sort_unstable();
    times.sort_unstable();
    (values, times)
}

pub fn one_line(width: i32) -> BlockInput {
    BlockInput {
        display: false,
        line_widths: [width, 0, 0],
        line_count: 1,
        name_line: 0,
        align: Align::Left,
        name_x: 0,
        name_width: width,
        prefix_width: 0,
        background: Rect::EMPTY,
        badge: false,
    }
}

pub fn breakpoint_inputs(asset: &NameplateAsset) -> Vec<BlockInput> {
    let mut edges: Vec<i32> = asset
        .breakpoint_edges
        .iter()
        .copied()
        .filter(|e| *e >= 0 && *e <= MAX_BLOCK_WIDTH)
        .collect();
    edges.sort_unstable();
    edges.dedup();
    if edges.is_empty() {
        return Vec::new();
    }
    let mut cells = vec![0];
    cells.extend(edges.iter().copied().filter(|e| *e > 0));
    let mut out = Vec::new();
    for (i, start) in cells.iter().enumerate() {
        let end = cells.get(i + 1).copied().unwrap_or(i32::MAX);
        let covered = REFERENCE_WIDTHS.iter().any(|w| *w >= *start && *w < end);
        if !covered {
            out.push(one_line(*start));
        }
    }
    out
}

pub fn distinct_variants(asset: &NameplateAsset) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for (index, variant) in asset.variants.iter().enumerate() {
        let same = out.iter().any(|seen| {
            let seen = &asset.variants[usize::from(*seen)];
            seen.tones == variant.tones
                && seen.stops == variant.stops
                && seen.slots == variant.slots
                && seen.flag == variant.flag
                && seen.params == variant.params
                && seen.colorway == variant.colorway
                && seen.badge.placement == variant.badge.placement
        });
        if !same {
            out.push(u16::try_from(index).unwrap_or(u16::MAX));
        }
    }
    out
}

pub fn resolved(asset: &NameplateAsset, variant: &Variant, slot: usize) -> Option<usize> {
    match variant.slots.get(slot) {
        Some(target) => target.map(usize::from),
        None => Some(slot),
    }
    .filter(|layer| *layer < asset.layers.len())
}

pub fn shown(asset: &NameplateAsset, slot: usize, layer: usize, width: i32) -> bool {
    let fits = |breakpoints: &[Breakpoint]| {
        breakpoints.is_empty() || breakpoints.iter().any(|b| b.contains(width))
    };
    if !fits(&asset.layers[layer].breakpoints) {
        return false;
    }
    let groups = &asset.groups;
    let mut owner = groups
        .iter()
        .position(|g| g.layers.iter().any(|l| usize::from(*l) == slot));
    let mut depth = 0;
    while let Some(index) = owner {
        let group = &groups[index];
        if !fits(&group.breakpoints) || depth > groups.len() {
            return false;
        }
        owner = group.parent.map(usize::from);
        depth += 1;
    }
    true
}

pub fn body_of(asset: &NameplateAsset, of: Of) -> Option<&Body> {
    match of {
        Of::Layer(l) => match asset.layers.get(usize::from(l)).map(|l| &l.content) {
            Some(LayerContent::Body(body)) => Some(body),
            _ => None,
        },
        _ => None,
    }
}

pub fn clip_length(asset: &NameplateAsset, clip: ClipRef) -> i64 {
    anim_clip(asset, clip).map_or(1, |c| i64::from(c.length.max(1)))
}

#[derive(Default)]
pub struct CachedPainter {
    images: RefCell<HashMap<(u8, u16), LayerImage>>,
}

impl Painter for CachedPainter {
    fn paint(
        &self,
        a: &NameplateAsset,
        layer: u8,
        state: u16,
        p: &Placement,
        g: &LayoutResult,
    ) -> LayerImage {
        if let Some(image) = self.images.borrow().get(&(layer, state)) {
            return image.clone();
        }
        let image = paint_layer(a, layer, state, p, g);
        self.images
            .borrow_mut()
            .insert((layer, state), image.clone());
        image
    }
}

pub struct Eval<'a> {
    asset: &'a NameplateAsset,
    pub params: Vec<f64>,
    slots: QuerySlots,
    runtime: RefCell<Runtime>,
    frame: Cell<i64>,
}

impl<'a> Eval<'a> {
    pub fn new(asset: &'a NameplateAsset, variant: &Variant, layout: &LayoutResult) -> Eval<'a> {
        let interner = &asset.molang.interner;
        let slots = QuerySlots::with_params(interner, &asset.params);
        let mut runtime = Runtime::new(interner);
        let block = &layout.block;
        let name = block.name_line_rect();
        let bg = block.background;
        let values = [
            (NameplateQuery::Width, layout.bounds.width()),
            (NameplateQuery::Height, layout.bounds.height()),
            (NameplateQuery::LineCount, i32::from(block.line_count)),
            (NameplateQuery::Frame, 0),
            (NameplateQuery::Loop, i32::from(asset.loop_frames)),
            (NameplateQuery::Fps, i32::from(asset.fps)),
            (NameplateQuery::NameX, block.name_x),
            (NameplateQuery::NameY, name.y0),
            (NameplateQuery::NameWidth, block.name_width),
            (NameplateQuery::PrefixWidth, block.prefix_width),
            (NameplateQuery::BgX0, bg.x0),
            (NameplateQuery::BgY0, bg.y0),
            (NameplateQuery::BgX1, bg.x1),
            (NameplateQuery::BgY1, bg.y1),
        ];
        for (query, value) in values {
            if let Some(slot) = slots.slot(query) {
                runtime.set_query(slot, value as f32);
            }
        }
        let params = asset
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| variant.params.get(i).copied().unwrap_or(p.value))
            .collect();
        Eval {
            asset,
            params,
            slots,
            runtime: RefCell::new(runtime),
            frame: Cell::new(0),
        }
    }

    pub fn set(&self, frame: i64, signals: &SignalSample) {
        self.frame.set(frame);
        let mut runtime = self.runtime.borrow_mut();
        if let Some(slot) = self.slots.slot(NameplateQuery::Frame) {
            runtime.set_query(slot, frame as f32);
        }
        for id in 0..SIGNAL_COUNT {
            if let Some(slot) = self.slots.signal(id) {
                runtime.set_query(slot, signals.values[id]);
            }
            if let Some(slot) = self.slots.time(id) {
                runtime.set_query(slot, signals.times[id]);
            }
        }
    }

    pub fn raw(&self, program: u32, frame: Option<i64>) -> f64 {
        let Some(entry) = self.asset.molang.programs.get(program as usize) else {
            return 0.0;
        };
        let slot = self.slots.slot(NameplateQuery::Frame);
        let mut runtime = self.runtime.borrow_mut();
        if let (Some(frame), Some(slot)) = (frame, slot) {
            runtime.set_query(slot, frame as f32);
        }
        runtime.clear_temps();
        let mut queries = ParamQueries {
            slots: &self.slots,
            values: &self.params,
        };
        let value = entry.program.run(&mut runtime, &mut queries);
        if let (Some(_), Some(slot)) = (frame, slot) {
            runtime.set_query(slot, self.frame.get() as f32);
        }
        value
    }

    pub fn value(&self, program: u32, frame: i32) -> f64 {
        let raw = self.raw(program, Some(i64::from(frame)));
        if raw.is_finite() {
            raw
        } else {
            0.0
        }
    }

    pub fn argb(&self, program: u32, frame: i32) -> u32 {
        let raw = self.raw(program, Some(i64::from(frame)));
        if !raw.is_finite() {
            return 0xff00_0000;
        }
        0xff00_0000 | libm::floor(raw.clamp(0.0, f64::from(0x00ff_ffff))) as u32
    }

    pub fn signals_read(&self, program: u32) -> (Vec<usize>, Vec<usize>) {
        let Some(entry) = self.asset.molang.programs.get(program as usize) else {
            return (Vec::new(), Vec::new());
        };
        let reads = &entry.program.reads().queries;
        let read = |slot: Option<u16>| slot.is_some_and(|slot| reads.contains(&slot));
        let values = (0..SIGNAL_COUNT)
            .filter(|&id| read(self.slots.signal(id)))
            .collect();
        let times = (0..SIGNAL_COUNT)
            .filter(|&id| read(self.slots.time(id)))
            .collect();
        (values, times)
    }

    pub fn letters(
        &self,
        program: u32,
        frame: i64,
        mode: TextMode,
        glyphs: &[f32],
        signals: &SignalSample,
        out: &mut [LetterPose],
    ) -> bool {
        self.set(frame, signals);
        let mut runtime = self.runtime.borrow_mut();
        let mut queries = ParamQueries {
            slots: &self.slots,
            values: &self.params,
        };
        run_letters(
            self.asset,
            program,
            mode,
            glyphs,
            &mut runtime.frame(),
            &mut queries,
            out,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    clip: ClipRef,
    start: i64,
    until: i64,
    seen: i64,
    gone: i64,
}

impl Run {
    fn kept(&self, frame: i64) -> bool {
        self.seen <= frame && frame < self.gone
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Timeline {
    runs: Vec<Run>,
    speeds: Vec<Vec<Option<Episode>>>,
}

static NO_RUNS: Timeline = Timeline {
    runs: Vec::new(),
    speeds: Vec::new(),
};

impl Timeline {
    pub fn new(asset: &NameplateAsset, scenario: &Scenario) -> Timeline {
        let mut runs = Vec::new();
        shot_runs(asset, scenario, &mut runs);
        reaction_runs(asset, scenario, &mut runs);
        Timeline {
            runs,
            speeds: Vec::new(),
        }
    }

    fn speed_edge(
        &self,
        asset: &NameplateAsset,
        scenario: &Scenario,
        clip: ClipRef,
        frame: u32,
    ) -> (bool, i64) {
        let now = i64::from(frame);
        match clip {
            ClipRef::Reaction(index) => {
                let Some(reaction) = asset.animations.reactions.get(usize::from(index)) else {
                    return (false, now);
                };
                let id = usize::from(reaction.signal);
                if id >= SIGNAL_COUNT {
                    return (false, now);
                }
                let fps = asset.fps.max(1);
                let sample = scenario.signals_at(frame, fps);
                let changed = now - libm::roundf(sample.times[id] * f32::from(fps)) as i64;
                (sample.values[id] >= 0.5, changed)
            }
            ClipRef::OneShot(index) => {
                let length = asset
                    .animations
                    .one_shots
                    .get(usize::from(index))
                    .map_or(0, |o| i64::from(o.clip.length));
                let mut ended = None;
                for run in self
                    .runs
                    .iter()
                    .filter(|r| r.clip == clip && r.start <= now)
                {
                    if now - run.start < run.until {
                        return (true, run.start);
                    }
                    ended = Some(run.start + run.until.min(length));
                }
                (false, ended.unwrap_or(now))
            }
            _ => (false, now),
        }
    }
}

fn shot_runs(asset: &NameplateAsset, scenario: &Scenario, out: &mut Vec<Run>) {
    let mut starts: Vec<(u16, i64)> = scenario
        .events
        .iter()
        .filter_map(|e| match e.clip {
            ClipRef::OneShot(index) => Some((index, e.start)),
            _ => None,
        })
        .collect();
    starts.sort_unstable();
    starts.dedup();
    for (index, shot) in asset.animations.one_shots.iter().enumerate() {
        let Ok(id) = u16::try_from(index) else {
            break;
        };
        let clip = ClipRef::OneShot(id);
        let length = clip_length(asset, clip);
        let first = out.len();
        for (_, start) in starts.iter().filter(|(i, _)| *i == id) {
            if let Some(last) = out[first..].last_mut() {
                if *start < last.start + last.until {
                    if !shot.restart {
                        continue;
                    }
                    last.until = start - last.start;
                }
            }
            out.push(Run {
                clip,
                start: *start,
                until: length,
                seen: *start,
                gone: i64::MAX,
            });
        }
        for i in first + 1..out.len() {
            out[i - 1].gone = out[i].start + out[i].until;
        }
    }
}

fn reaction_runs(asset: &NameplateAsset, scenario: &Scenario, out: &mut Vec<Run>) {
    let fps = asset.fps.max(1);
    let rate = f32::from(fps);
    for (index, reaction) in asset.animations.reactions.iter().enumerate() {
        let Ok(index) = u16::try_from(index) else {
            break;
        };
        let life = burst_life(asset, &reaction.clip);
        let id = usize::from(reaction.signal);
        if life == 0 || id >= SIGNAL_COUNT {
            continue;
        }
        let clip = ClipRef::Reaction(index);
        let mut running: Option<usize> = None;
        let mut ended: Vec<usize> = Vec::new();
        for frame in 0..scenario.frames {
            let now = i64::from(frame);
            let sample = scenario.signals_at(frame, fps);
            let changed = now - libm::roundf(sample.times[id] * rate) as i64;
            let mut stopped = None;
            if sample.values[id] >= 0.5 {
                if running.is_some_and(|r| out[r].start != changed) {
                    stopped = running.take();
                }
                if running.is_none() {
                    running = Some(out.len());
                    out.push(Run {
                        clip,
                        start: changed,
                        until: i64::MAX,
                        seen: now,
                        gone: i64::MAX,
                    });
                }
            } else {
                stopped = running.take();
            }
            if let Some(r) = stopped {
                out[r].until = changed - out[r].start;
                ended.insert(0, r);
            }
            let mut count = 0;
            ended.retain(|&r| {
                let run = &mut out[r];
                let keep = now - (run.start + run.until) < life && count < KEPT_RUNS;
                if keep {
                    count += 1;
                } else {
                    run.gone = now;
                }
                keep
            });
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Spawn {
    Looped(i64, Option<i64>),
    Run(i64, i64),
}

pub struct Shot {
    pub time: TagTime,
    pub pose: TagPose,
    pub particles: Vec<ParticlePose>,
    pub speeds: Vec<PathSpeed>,
}

pub struct Stage<'a> {
    pub asset: &'a NameplateAsset,
    pub variant: &'a Variant,
    pub input: BlockInput,
    pub placement: Placement,
    pub layout: LayoutResult,
    pub eval: Eval<'a>,
    painter: CachedPainter,
    timeline: RefCell<Option<(Scenario, Timeline)>>,
    speed: Vec<ClipRef>,
    walks: Vec<Option<Walk>>,
    profiles: RefCell<HashMap<(usize, usize, Option<i64>), Arc<Profile>>>,
}

impl<'a> Stage<'a> {
    pub fn new(asset: &'a NameplateAsset, variant: u16, input: &BlockInput) -> Option<Stage<'a>> {
        let chosen = asset.variant(variant)?;
        let block = build_block(input)?;
        let placement = Placement {
            block,
            variant,
            breakpoint: asset.breakpoint(block.name_width),
        };
        let layout = resolve(asset, &placement);
        let eval = Eval::new(asset, chosen, &layout);
        let walks = (0..asset.layers.len())
            .map(|layer| layer_walk(asset, &placement, &layout, layer))
            .collect();
        Some(Stage {
            asset,
            variant: chosen,
            input: *input,
            placement,
            layout,
            eval,
            painter: CachedPainter::default(),
            timeline: RefCell::new(None),
            speed: speed_clips(&asset.animations),
            walks,
            profiles: RefCell::new(HashMap::new()),
        })
    }

    fn profile(&self, index: usize, layer: usize, origin: Option<i64>) -> Option<Arc<Profile>> {
        let key = (index, layer, origin);
        if let Some(profile) = self.profiles.borrow().get(&key) {
            return Some(profile.clone());
        }
        let track = speed_track(self.asset, *self.speed.get(index)?, layer)?;
        let clock = origin.map_or(ClipClock::Loop, ClipClock::Local);
        let step = self.asset.layers[layer].step;
        let profile = Arc::new(Profile::reaction(
            track,
            clock,
            step,
            self.asset.loop_frames,
        ));
        self.profiles.borrow_mut().insert(key, profile.clone());
        Some(profile)
    }

    fn episodes(&self, scenario: &Scenario, timeline: &Timeline) -> Vec<Vec<Option<Episode>>> {
        let asset = self.asset;
        let offset = i64::from(scenario.time(0, asset).offset);
        let frames = scenario.frames as usize;
        let mut out = Vec::with_capacity(self.speed.len());
        for (index, clip) in self.speed.iter().enumerate() {
            let mut episodes = vec![None; frames];
            let walk = primary_layer(asset, *clip)
                .and_then(|layer| Some((layer, self.walks.get(layer)?.as_ref()?)));
            if let Some((layer, walk)) = walk {
                let step = asset.layers[layer].step;
                let one_shot = matches!(clip, ClipRef::OneShot(_));
                let mut floats = [0.0f32; EPISODE_FLOATS];
                for frame in 0..scenario.frames {
                    let now = i64::from(frame) + offset;
                    let (on, edge) = timeline.speed_edge(asset, scenario, *clip, frame);
                    let edge = edge + offset;
                    track(&mut floats, on, edge, now, |previous, e| {
                        let origin = one_shot.then_some(previous.start);
                        let profile = self.profile(index, layer, origin)?;
                        rejoin_frame(walk, &profile, e, step)
                    });
                    episodes[frame as usize] = read_episode(&floats);
                }
            }
            out.push(episodes);
        }
        out
    }

    fn speeds(&self, scenario: &Scenario, frame: u32) -> Vec<PathSpeed> {
        let episodes: Vec<Option<Episode>> = {
            let timeline = self.timeline(scenario);
            timeline
                .speeds
                .iter()
                .map(|e| e.get(frame as usize).copied().flatten())
                .collect()
        };
        let mut out = Vec::new();
        for (index, episode) in episodes.into_iter().enumerate() {
            let Some(episode) = episode else {
                continue;
            };
            let origin = matches!(self.speed[index], ClipRef::OneShot(_)).then_some(episode.start);
            for layer in 0..self.walks.len() {
                if self.walks[layer].is_none() {
                    continue;
                }
                if let Some(profile) = self.profile(index, layer, origin) {
                    out.push(PathSpeed {
                        layer: u8::try_from(layer).unwrap_or(u8::MAX),
                        profile,
                        episode,
                    });
                }
            }
        }
        out
    }

    pub fn painter(&self) -> &CachedPainter {
        &self.painter
    }

    pub fn roams(&self, layer: usize) -> bool {
        let steered = self
            .speed
            .iter()
            .any(|clip| speed_track(self.asset, *clip, layer).is_some());
        steered
            || self
                .walks
                .get(layer)
                .and_then(Option::as_ref)
                .is_some_and(Walk::roams)
    }

    pub fn route_loops(&self) -> i64 {
        self.walks
            .iter()
            .flatten()
            .map(Walk::repeat_loops)
            .max()
            .unwrap_or(1)
    }

    pub fn route_reach(&self, layer: usize, bounds: Rect) -> Rect {
        let pose = TagPose::default();
        let scene = Scene {
            asset: self.asset,
            variant: self.variant,
            placement: &self.placement,
            layout: &self.layout,
            pose: &pose,
            particles: &[],
            time: TagTime::default(),
            display: self.input.display,
            hidden: 0,
            paths: true,
            speeds: &[],
        };
        scene.route_reach(layer, bounds)
    }

    pub fn region(&self) -> Rect {
        let margin = 32;
        self.placement
            .block
            .background
            .grow(margin, margin, margin, margin)
            .intersect(CANVAS)
    }

    fn timeline(&self, scenario: &Scenario) -> Ref<'_, Timeline> {
        let stale = self
            .timeline
            .borrow()
            .as_ref()
            .is_none_or(|(seen, _)| seen != scenario);
        if stale {
            let mut fresh = Timeline::new(self.asset, scenario);
            fresh.speeds = self.episodes(scenario, &fresh);
            *self.timeline.borrow_mut() = Some((scenario.clone(), fresh));
        }
        Ref::map(self.timeline.borrow(), |cached| match cached {
            Some((_, timeline)) => timeline,
            None => &NO_RUNS,
        })
    }

    fn playing(
        &self,
        scenario: &Scenario,
        signals: &SignalSample,
        frame: u32,
    ) -> (TagTime, Vec<ClipTime>, Vec<(ClipRef, Spawn)>) {
        let asset = self.asset;
        let time = scenario.time(frame, asset);
        let engine = i64::from(time.engine);
        let looped = ClipTime {
            clip: ClipRef::Loop,
            frame: i32::from(time.loop_frame),
            start: engine - i64::from(time.loop_frame),
        };
        let at = |start: i64| i32::try_from(engine - start).unwrap_or(i32::MAX);
        let mut live = vec![looped];
        let mut spawns = vec![(ClipRef::Loop, Spawn::Looped(i64::MIN, None))];
        for event in &scenario.events {
            if !matches!(event.clip, ClipRef::Schedule { .. }) || event.start > engine {
                continue;
            }
            let until = if event.length < clip_length(asset, event.clip) {
                event.length
            } else {
                i64::MAX
            };
            spawns.push((event.clip, Spawn::Run(event.start, until)));
            if engine - event.start < event.length {
                live.push(ClipTime {
                    clip: event.clip,
                    frame: at(event.start),
                    start: event.start,
                });
            }
        }
        for (index, reaction) in asset.animations.reactions.iter().enumerate() {
            if signals
                .values
                .get(usize::from(reaction.signal))
                .is_some_and(|v| *v >= 0.5)
            {
                live.push(ClipTime {
                    clip: ClipRef::Reaction(u16::try_from(index).unwrap_or(u16::MAX)),
                    ..looped
                });
            }
        }
        for run in self.timeline(scenario).runs.iter() {
            if !run.kept(engine) {
                continue;
            }
            match run.clip {
                ClipRef::Reaction(_) => {
                    let until = (run.until != i64::MAX).then(|| run.start + run.until);
                    spawns.push((run.clip, Spawn::Looped(run.start, until)));
                }
                _ => {
                    spawns.push((run.clip, Spawn::Run(run.start, run.until)));
                    if engine - run.start < run.until {
                        live.push(ClipTime {
                            clip: run.clip,
                            frame: at(run.start),
                            start: run.start,
                        });
                    }
                }
            }
        }
        (time, live, spawns)
    }

    pub fn shot(&self, scenario: &Scenario, frame: u32) -> Shot {
        let asset = self.asset;
        let signals = scenario.signals_at(frame, asset.fps);
        let (time, live, spawns) = self.playing(scenario, &signals, frame);
        let molang_frame = live
            .iter()
            .filter(|t| matches!(t.clip, ClipRef::OneShot(_)))
            .max_by_key(|t| t.start)
            .map_or(i64::from(time.loop_frame), |t| i64::from(t.frame));
        self.eval.set(molang_frame, &signals);
        let gates = Gates::default();
        let mut kept = Vec::new();
        shown_clips(asset, &live, &gates, &mut kept);
        let sampler = |program: u32, frame: i32, channel: Channel| {
            channel.convert(self.eval.raw(program, Some(i64::from(frame))))
        };
        let colors = |program: u32, frame: i32| self.eval.argb(program, frame);
        let mut pose = TagPose::default();
        pose_layers(asset, self.variant, &kept, &sampler, &gates, &[], &mut pose);
        animate_palette(asset, self.variant, &kept, &colors, &gates, &mut pose);
        let particles = self.particles(&time, &spawns);
        let speeds = self.speeds(scenario, frame);
        Shot {
            time,
            pose,
            particles,
            speeds,
        }
    }

    fn particles(&self, time: &TagTime, spawns: &[(ClipRef, Spawn)]) -> Vec<ParticlePose> {
        let asset = self.asset;
        let layout = &self.layout;
        let offset = i64::from(time.offset);
        let frame = i64::from(time.engine) + offset;
        let mut out = Vec::new();
        for (index, layer) in asset.layers.iter().enumerate() {
            let LayerContent::Emitter(emitter) = &layer.content else {
                continue;
            };
            let id = u8::try_from(index).unwrap_or(u8::MAX);
            let rect = layout.targets.get(index).copied().unwrap_or(layout.bounds);
            let body = body_of(asset, layer.of);
            let route = route_key(id, emitter, &asset.sprites, rect, body).map(|k| build(&k));
            let area = SpawnArea::new(&emitter.spawn, rect, &layout.block, body, route.as_ref());
            let longest = emitter.lifetime[1];
            let mut bursts = Vec::new();
            for (clip, spawn) in spawns {
                let Some(clip) = anim_clip(asset, *clip) else {
                    continue;
                };
                let list: Vec<Burst> = clip
                    .targets
                    .iter()
                    .filter(|t| t.target == Target::Layer(id))
                    .flat_map(|t| t.channels.bursts.iter().copied())
                    .collect();
                if list.is_empty() {
                    continue;
                }
                match *spawn {
                    Spawn::Looped(from, until) => looped_starts(
                        &list,
                        asset.loop_frames,
                        frame,
                        longest,
                        (from.saturating_add(offset), until.map(|u| u + offset)),
                        &mut bursts,
                    ),
                    Spawn::Run(start, until) => {
                        run_starts_until(&list, start + offset, frame, longest, until, &mut bursts)
                    }
                }
            }
            bursts.sort_unstable_by_key(|b| (b.start, b.count));
            emit_in(
                emitter,
                &asset.sprites,
                id,
                &area,
                time.seed,
                frame,
                &bursts,
                &mut out,
            );
        }
        out
    }

    pub fn draws(&self, shot: &Shot, hidden: u32) -> Vec<Draw> {
        let scene = Scene {
            asset: self.asset,
            variant: self.variant,
            placement: &self.placement,
            layout: &self.layout,
            pose: &shot.pose,
            particles: &shot.particles,
            time: shot.time,
            display: self.input.display,
            hidden,
            paths: true,
            speeds: &shot.speeds,
        };

        let exprs = |program: u32, frame: i32| self.eval.value(program, frame);
        draws(&scene, &self.painter, &exprs)
    }

    pub fn plate(&self, shot: &Shot) -> (i32, i32) {
        if self.input.display {
            (0, 0)
        } else {
            (i32::from(shot.pose.plate.0), i32::from(shot.pose.plate.1))
        }
    }

    pub fn render(&self, shot: &Shot, list: &[Draw], canvas: &mut Canvas) {
        let colors = Colors::new(self.asset, self.variant, &shot.pose.palette);
        canvas.clear();
        composite(list, &colors, self.plate(shot), canvas);
    }

    pub fn render_within(&self, shot: &Shot, list: &[Draw], canvas: &mut Canvas, area: Rect) {
        let colors = Colors::new(self.asset, self.variant, &shot.pose.palette);
        canvas.clear_within(area);
        composite_within(list, &colors, self.plate(shot), canvas, area);
    }
}

pub fn sampled_frames(loop_frames: u16, most: u32) -> Vec<u32> {
    let frames = u32::from(loop_frames.max(1));
    let stride = frames.div_ceil(most.max(1)).max(1);
    (0..frames).step_by(stride as usize).collect()
}
