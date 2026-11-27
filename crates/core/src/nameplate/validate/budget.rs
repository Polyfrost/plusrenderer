use std::collections::BTreeMap;

use crate::container::Kind;
use crate::cost::{check_emitters, quad_limit, record_bytes, Emitter as CostEmitter};
use crate::issues::{path, Issue, IssueCode, Mode, Report};
use crate::limits::{
    NAMEPLATE_BLEND_ROWS, NAMEPLATE_CACHED_STATES, NAMEPLATE_COST_CYCLE_TRACK,
    NAMEPLATE_COST_KEYS_TRACK, NAMEPLATE_COST_LETTER_FACTOR, NAMEPLATE_COST_MATERIAL_LAYER,
    NAMEPLATE_COST_PALETTE, NAMEPLATE_COST_PARTICLE, NAMEPLATE_COST_REPEAT_COPY,
    NAMEPLATE_COST_SPAWN_CHECK, NAMEPLATE_COST_WALKER, NAMEPLATE_CPU_UNITS,
    NAMEPLATE_DECORATION_MARGIN, NAMEPLATE_FALLBACK_BYTES, NAMEPLATE_IMAGE_BYTES, NAMEPLATE_OFFSET,
};

use super::super::anim::particles::{box_size, motion, worst_case_live};
use super::super::anim::tracks::{snap, Channel};
use super::super::anim::TagTime;
use super::super::geom::{BlockInput, Edge, Rect, CANVAS};
use super::super::image::{Axis, LayerImage, Painter};
use super::super::layout::{anchor_point, body_rect, reference_inputs};
use super::super::material::cpu::fallback_bytes;
use super::super::material::ir::{Lane, MaterialIr, Period};
use super::super::paint::layer_states;
use super::super::queries::MolangUse;
use super::super::types::{
    BadgePlacement, Clip, Fill, LayerContent, Mask, NameplateAsset, Of, PaletteTrack, SpawnShape,
    TagPose, Target, Track, Value,
};
use super::super::{Analysis, EffectAnalysis, LayerAnalysis};
use super::cover::{lanes, pairwise, sample};
use super::sim::{
    breakpoint_inputs, distinct_variants, resolved, shown, Eval, Scenario, Shot, SignalSample,
    Stage,
};
use super::{animation_path, layer_path, MANIFEST};

const STATE_CAP: u16 = 2 * NAMEPLATE_CACHED_STATES as u16;
const MARGIN_EVALS: u64 = 1 << 18;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub x0: i32,
    pub x1: i32,
    pub y0: i32,
    pub y1: i32,
}

impl Span {
    fn take(&mut self, x: bool, value: i32) {
        if x {
            self.x0 = self.x0.min(value);
            self.x1 = self.x1.max(value);
        } else {
            self.y0 = self.y0.min(value);
            self.y1 = self.y1.max(value);
        }
    }

    fn plus(self, other: Span) -> Span {
        Span {
            x0: self.x0 + other.x0,
            x1: self.x1 + other.x1,
            y0: self.y0 + other.y0,
            y1: self.y1 + other.y1,
        }
    }

    fn mirrored(self) -> Span {
        Span {
            x0: self.x0.min(-self.x1),
            x1: self.x1.max(-self.x0),
            ..self
        }
    }

    fn apply(self, rect: Rect) -> Rect {
        if rect.is_empty() {
            return rect;
        }
        Rect::new(
            rect.x0 + self.x0,
            rect.y0 + self.y0,
            rect.x1 + self.x1,
            rect.y1 + self.y1,
        )
    }

    pub fn reach(self) -> i32 {
        (-self.x0).max(self.x1).max(-self.y0).max(self.y1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reach {
    pub clip: String,
    pub target: Target,
    pub x: bool,
    pub raw: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Ranges {
    pub layers: Vec<Span>,
    pub groups: Vec<Span>,
    pub badge: Span,
    pub plate: Span,
    pub reaches: Vec<Reach>,
}

pub fn clips(asset: &NameplateAsset) -> Vec<(&Clip, i64)> {
    let a = &asset.animations;
    let looped = i64::from(asset.loop_frames.max(1));
    let mut out = vec![(&a.looping, looped)];
    for schedule in &a.schedules {
        for event in &schedule.events {
            out.push((&event.clip, i64::from(event.clip.length.max(1))));
        }
    }
    for reaction in &a.reactions {
        out.push((&reaction.clip, looped));
    }
    for shot in &a.one_shots {
        out.push((&shot.clip, i64::from(shot.clip.length.max(1))));
    }
    out
}

fn track_values(track: &Track, frames: i64, fps: u8, eval: &Eval<'_>, out: &mut Vec<i32>) {
    for key in &track.keys {
        match key.value {
            Value::Fixed(v) => out.push(v),
            Value::Molang(program) => {
                let (values, times) = eval.signals_read(program);
                if values.is_empty() && times.is_empty() {
                    for frame in 0..frames {
                        out.push(snap(eval.raw(program, Some(frame))));
                    }
                    continue;
                }
                let lanes = lanes(&values, &times);
                let sizes: Vec<usize> = lanes.iter().map(|l| l.samples.len()).collect();
                let most = MARGIN_EVALS / u64::try_from(frames.max(1)).unwrap_or(u64::MAX);
                let base = Scenario::idle(0, 0);
                for row in pairwise(&sizes, usize::try_from(most).unwrap_or(usize::MAX)) {
                    let scenario = Scenario {
                        sample: sample(&lanes, &row, &[]),
                        ..base.clone()
                    };
                    for frame in 0..frames {
                        let at = u32::try_from(frame).unwrap_or(u32::MAX);
                        eval.set(frame, &scenario.signals_at(at, fps));
                        out.push(snap(eval.raw(program, Some(frame))));
                    }
                }
                eval.set(0, &SignalSample::NEUTRAL);
            }
        }
    }
}

pub fn ranges(asset: &NameplateAsset, eval: &Eval<'_>) -> Ranges {
    let mut ranges = Ranges {
        layers: vec![Span::default(); asset.layers.len()],
        groups: vec![Span::default(); asset.groups.len()],
        ..Ranges::default()
    };
    let limit = NAMEPLATE_OFFSET as i32;
    let mut values = Vec::new();
    for (clip, frames) in clips(asset) {
        for target in &clip.targets {
            let tracks = [(&target.channels.x, true), (&target.channels.y, false)];
            for (track, x) in tracks {
                let Some(track) = track else {
                    continue;
                };
                values.clear();
                track_values(track, frames, asset.fps, eval, &mut values);
                let span = match target.target {
                    Target::Layer(l) => ranges.layers.get_mut(usize::from(l)),
                    Target::Group(g) => ranges.groups.get_mut(usize::from(g)),
                    Target::Badge => Some(&mut ranges.badge),
                    Target::Plate => Some(&mut ranges.plate),
                };
                let Some(span) = span else {
                    continue;
                };
                let mut worst = 0i32;
                for raw in &values {
                    span.take(x, Channel::Position.clamp(*raw));
                    if raw.abs() > worst.abs() {
                        worst = *raw;
                    }
                }
                if worst.abs() > limit {
                    ranges.reaches.push(Reach {
                        clip: clip.name.clone(),
                        target: target.target,
                        x,
                        raw: worst,
                    });
                }
            }
        }
    }
    ranges
}

fn group_chain(asset: &NameplateAsset, ranges: &Ranges, layer: usize) -> Span {
    let groups = &asset.groups;
    let mut total = Span::default();
    let mut owner = groups
        .iter()
        .position(|g| g.layers.iter().any(|l| usize::from(*l) == layer));
    let mut depth = 0;
    while let Some(index) = owner {
        if depth > groups.len() {
            break;
        }
        total = total.plus(ranges.groups.get(index).copied().unwrap_or_default());
        owner = groups[index].parent.map(usize::from);
        depth += 1;
    }
    total
}

fn own_chain(asset: &NameplateAsset, ranges: &Ranges, layer: usize) -> Span {
    ranges
        .layers
        .get(layer)
        .copied()
        .unwrap_or_default()
        .plus(group_chain(asset, ranges, layer))
}

fn follow_chain(asset: &NameplateAsset, ranges: &Ranges, of: Of) -> Span {
    match of {
        Of::Layer(t) => match asset.layers.get(usize::from(t)).map(|l| &l.content) {
            Some(LayerContent::Body(_)) => own_chain(asset, ranges, usize::from(t)),
            _ => Span::default(),
        },
        _ => Span::default(),
    }
}

pub fn layer_span(asset: &NameplateAsset, ranges: &Ranges, layer: usize) -> Span {
    let spec = &asset.layers[layer];
    let own = own_chain(asset, ranges, layer);
    let own = if spec.mirror.is_some() {
        own.mirrored()
    } else {
        own
    };
    own.plus(follow_chain(asset, ranges, spec.of))
        .plus(ranges.plate)
}

pub fn breach(extent: Rect, background: Rect) -> i32 {
    if extent.is_empty() {
        return 0;
    }
    (background.x0 - extent.x0)
        .max(extent.x1 - background.x1)
        .max(background.y0 - extent.y0)
        .max(extent.y1 - background.y1)
        .max(0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cpu {
    pub molang: u32,
    pub letters: u32,
    pub paths: u32,
    pub repeats: u32,
    pub particles: u32,
    pub palette: u32,
    pub materials: u32,
}

impl Cpu {
    pub fn total(&self) -> u32 {
        self.molang
            .saturating_add(self.letters)
            .saturating_add(self.paths)
            .saturating_add(self.repeats)
            .saturating_add(self.particles)
            .saturating_add(self.palette)
            .saturating_add(self.materials)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub variant: String,
    pub name_width: i32,
    pub lines: u8,
}

impl Place {
    fn of(stage: &Stage<'_>) -> Place {
        Place {
            variant: stage.variant.key.clone(),
            name_width: stage.placement.block.name_width,
            lines: stage.placement.block.line_count,
        }
    }

    fn tag(&self, issue: Issue) -> Issue {
        issue
            .value("name_width", f64::from(self.name_width))
            .value("lines", f64::from(self.lines))
            .name("variant", &self.variant)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Fallback {
    pub frame_bytes: u64,
    pub frames: u32,
    pub drift: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Measured {
    pub analysis: Analysis,
    pub quads: (u32, Place),
    pub cpu: (Cpu, Place),
    pub margins: Vec<(i32, Place)>,
    pub outside: Vec<Option<Place>>,
    pub badge: (i32, Place),
    pub plate: i32,
    pub reaches: Vec<(Reach, Place)>,
    pub blend_rows: Vec<i32>,
    pub fallbacks: Vec<Option<Fallback>>,
    pub extents: Vec<Vec<(BlockInput, u16, Rect)>>,
    pub build_ops: u64,
    pub effect_ops: Vec<u64>,
}

fn pixels(area: Rect) -> u64 {
    if area.is_empty() {
        return 0;
    }
    u64::from(area.width().unsigned_abs()) * u64::from(area.height().unsigned_abs())
}

fn material_frames(asset: &NameplateAsset, layer: usize, ir: &MaterialIr, drift: bool) -> u32 {
    if drift {
        return 1;
    }
    let step = u32::from(asset.layers[layer].step.max(1));
    let cycle = (u32::from(asset.loop_frames) / step).max(1);
    match ir.period {
        Period::Still => 1,
        Period::Frames(p) if p > 0 && cycle % p == 0 => p,
        _ => cycle,
    }
}

fn shot_states(asset: &NameplateAsset, layer: usize, ir: &MaterialIr) -> u32 {
    let step = u32::from(asset.layers[layer].step.max(1));
    asset
        .animations
        .one_shots
        .iter()
        .filter(|shot| {
            shot.clip
                .palette
                .iter()
                .any(|track| ir.tone_slots.contains(&track.slot))
        })
        .map(|shot| u32::from(shot.clip.length.max(1)).div_ceil(step))
        .fold(0u32, u32::saturating_add)
}

fn effects(asset: &NameplateAsset, measured: &Measured) -> Vec<EffectAnalysis> {
    let mut out = Vec::new();
    for (layer, spec) in asset.layers.iter().enumerate() {
        let LayerContent::Material(content) = &spec.content else {
            continue;
        };
        let Some(ir) = asset.materials.get(usize::from(content.material)) else {
            continue;
        };
        let mut reads: Vec<String> = Vec::new();
        for lane in &ir.lanes {
            if let Lane::Param(index) = lane {
                if let Some(param) = asset.params.get(usize::from(*index)) {
                    if !reads.contains(&param.id) {
                        reads.push(param.id.clone());
                    }
                }
            }
        }
        out.push(EffectAnalysis {
            layer: spec.id.clone(),
            states: material_frames(asset, layer, ir, content.drift),
            shot_states: shot_states(asset, layer, ir),
            tone_slots: u32::try_from(ir.tone_slots.len()).unwrap_or(u32::MAX),
            reads,
            build_ops: measured.effect_ops.get(layer).copied().unwrap_or(0),
        });
    }
    out
}

fn static_cpu(asset: &NameplateAsset) -> (u32, u32, u32) {
    let mut molang = 0u32;
    let mut text = 0u32;
    for program in &asset.molang.programs {
        if program.scope == MolangUse::Text {
            text = text.max(program.cost);
        } else {
            molang = molang.saturating_add(program.cost);
        }
    }
    let mut cycles = 0u32;
    let mut keys = 0u32;
    for (clip, _) in clips(asset) {
        for track in &clip.palette {
            match track.track {
                PaletteTrack::Cycle { .. } => cycles += 1,
                PaletteTrack::Keys(_) => keys += 1,
            }
        }
    }
    let palette = if cycles + keys == 0 {
        0
    } else {
        NAMEPLATE_COST_PALETTE
            + cycles * NAMEPLATE_COST_CYCLE_TRACK
            + keys * NAMEPLATE_COST_KEYS_TRACK
    };
    (
        molang,
        text.saturating_mul(NAMEPLATE_COST_LETTER_FACTOR),
        palette,
    )
}

struct Images {
    bounds: Rect,
    quads: u32,
}

fn layer_images(
    stage: &Stage<'_>,
    layer: usize,
    keys: Option<&mut BTreeMap<(usize, u16, u64), u32>>,
) -> Images {
    let asset = stage.asset;
    let id = u8::try_from(layer).unwrap_or(u8::MAX);
    let states = layer_states(asset, id).clamp(1, STATE_CAP);
    let mut bounds = Rect::EMPTY;
    let mut quads = 0;
    let mut keys = keys;
    for state in 0..states {
        let image: LayerImage =
            stage
                .painter()
                .paint(asset, id, state, &stage.placement, &stage.layout);
        bounds = bounds.union(image.bounds);
        quads = quads.max(image.quads());
        if let Some(keys) = keys.as_deref_mut() {
            keys.entry((layer, state, shape(&image)))
                .or_insert(image.bytes);
        }
    }
    Images { bounds, quads }
}

fn shape(image: &LayerImage) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut push = |value: i64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let (ox, oy) = (i64::from(image.bounds.x0), i64::from(image.bounds.y0));
    for rect in &image.rects {
        push(i64::from(rect.x0) - ox);
        push(i64::from(rect.y0) - oy);
        push(i64::from(rect.x1) - ox);
        push(i64::from(rect.y1) - oy);
        push(i64::from(rect.color.0));
    }
    push(-1);
    for strip in &image.strips {
        push(i64::from(strip.x0) - ox);
        push(i64::from(strip.y0) - oy);
        push(i64::from(strip.x1) - ox);
        push(i64::from(strip.y1) - oy);
        push(i64::from(strip.slot));
        push(i64::from(strip.phase));
        push(i64::from(strip.axis == Axis::Y));
    }
    hash
}

fn edge_rows(rect: Rect, edge: Edge) -> Rect {
    match edge {
        Edge::Top => Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + 1),
        Edge::Bottom => Rect::new(rect.x0, rect.y1 - 1, rect.x1, rect.y1),
        Edge::Left => Rect::new(rect.x0, rect.y0, rect.x0 + 1, rect.y1),
        Edge::Right => Rect::new(rect.x1 - 1, rect.y0, rect.x1, rect.y1),
    }
}

fn particle_extent(stage: &Stage<'_>, layer: usize, image: Rect) -> Rect {
    let asset = stage.asset;
    let LayerContent::Emitter(e) = &asset.layers[layer].content else {
        return Rect::EMPTY;
    };
    let layout = &stage.layout;
    let rect = layout.targets.get(layer).copied().unwrap_or(layout.bounds);
    let spawn = match e.spawn.shape {
        SpawnShape::Point => {
            let (x, y) = anchor_point(e.spawn.anchor, None, false, rect, &layout.block);
            Rect::new(x, y, x + 1, y + 1)
        }
        SpawnShape::Line => edge_rows(rect, e.spawn.edge),
        SpawnShape::Area => rect,
        SpawnShape::Border => {
            let [w, h] = box_size(e, &asset.sprites).map(i32::from);
            rect.grow(0, 0, w, h)
        }
    };
    if spawn.is_empty() || image.is_empty() {
        return Rect::EMPTY;
    }
    let longest = i64::from(e.lifetime[1].max(1));
    let mut mx = (i32::MAX, i32::MIN);
    let mut my = (i32::MAX, i32::MIN);
    for age in 0..longest {
        for v in e.velocity_x {
            let m = motion(v + e.drift[0], e.gravity[0], age);
            mx = (mx.0.min(m), mx.1.max(m));
        }
        for v in e.velocity_y {
            let m = motion(v + e.drift[1], e.gravity[1], age);
            my = (my.0.min(m), my.1.max(m));
        }
    }
    let (wx, wy) = e
        .wobble
        .map_or((0, 0), |w| (i32::from(w.x), i32::from(w.y)));
    let mut half = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for id in &e.sprites {
        let (w, h) = asset
            .sprites
            .get(usize::from(*id))
            .map_or((1, 1), |s| (i32::from(s.width), i32::from(s.height)));
        half = (
            half.0.min(-w / 2),
            half.1.max(-w / 2),
            half.2.min(-h / 2),
            half.3.max(-h / 2),
        );
    }
    if half.0 > half.1 {
        half = (0, 0, 0, 0);
    }
    let [ox, oy] = e.spawn.offset.map(i32::from);
    Rect::new(
        spawn.x0 + ox + half.0 + mx.0 - wx + image.x0,
        spawn.y0 + oy + half.2 + my.0 - wy + image.y0,
        spawn.x1 - 1 + ox + half.1 + mx.1 + wx + image.x1,
        spawn.y1 - 1 + oy + half.3 + my.1 + wy + image.y1,
    )
}

fn mirror_area(area: Rect, body: Rect) -> Rect {
    let axis = body.x0 + body.x1 - 1;
    Rect::new(axis - area.x1 + 1, area.y0, axis - area.x0 + 1, area.y1)
}

struct Layered {
    extent: Rect,
    raw: Rect,
    quads: u32,
    copies: u32,
}

#[allow(clippy::too_many_arguments)]
fn measure_stage(
    stage: &Stage<'_>,
    reference: bool,
    statics: (u32, u32, u32),
    keys: &mut BTreeMap<(usize, u16, u64), u32>,
    out: &mut Measured,
    per_layer: &mut [LayerAnalysis],
) {
    let asset = stage.asset;
    let variant = stage.variant;
    let place = Place::of(stage);
    let width = stage.placement.block.name_width;
    let background = stage.placement.block.background;
    let ranges = ranges(asset, &stage.eval);
    for reach in &ranges.reaches {
        if !out
            .reaches
            .iter()
            .any(|(r, _)| r.clip == reach.clip && r.target == reach.target && r.x == reach.x)
        {
            out.reaches.push((reach.clone(), place.clone()));
        }
    }
    let count = asset.layers.len();
    let mut slots: Vec<(usize, usize)> = Vec::new();
    for slot in 0..count.min(32) {
        if let Some(layer) = resolved(asset, variant, slot) {
            if shown(asset, slot, layer, width) {
                slots.push((slot, layer));
            }
        }
    }
    let is_path = |layer: usize| matches!(&asset.layers[layer].content, LayerContent::Sprite(s) if s.path.is_some());
    let mut hidden_static = 0u32;
    let mut hidden_paths = u32::MAX;
    for (slot, layer) in &slots {
        let bit = 1u32 << slot;
        match &asset.layers[*layer].content {
            LayerContent::Material(_) | LayerContent::Emitter(_) => hidden_static |= bit,
            LayerContent::Sprite(_) if is_path(*layer) => {
                hidden_static |= bit;
                hidden_paths &= !bit;
            }
            _ => {}
        }
    }
    let mut layered: Vec<Option<Layered>> = (0..count).map(|_| None).collect();
    let mut images: Vec<Option<Images>> = (0..count).map(|_| None).collect();
    let mut image_of = |layer: usize, keys: &mut BTreeMap<(usize, u16, u64), u32>| -> Rect {
        if images[layer].is_none() {
            images[layer] = Some(layer_images(stage, layer, reference.then_some(keys)));
        }
        images[layer].as_ref().map_or(Rect::EMPTY, |i| i.bounds)
    };
    let mut quads_of = vec![0u32; count];
    let identity = |frame: u32| Shot {
        time: TagTime::at(frame, asset.loop_frames, asset.phase, 0),
        pose: TagPose::default(),
        particles: Vec::new(),
        speeds: Vec::new(),
    };
    let add = |layered: &mut Vec<Option<Layered>>, layer: usize, rect: Rect, copies: u32| {
        let entry = layered[layer].get_or_insert(Layered {
            extent: Rect::EMPTY,
            raw: Rect::EMPTY,
            quads: 0,
            copies: 0,
        });
        entry.raw = entry.raw.union(rect);
        entry.copies = entry.copies.max(copies);
    };
    let list = stage.draws(&identity(0), hidden_static);
    let mut copies = vec![0u32; count];
    for draw in &list {
        if let Some(layer) = resolved(asset, variant, usize::from(draw.layer)) {
            copies[layer] += 1;
        }
    }
    for draw in &list {
        let Some(layer) = resolved(asset, variant, usize::from(draw.layer)) else {
            continue;
        };
        let bounds = image_of(layer, keys);
        add(&mut layered, layer, draw.place_rect(bounds), copies[layer]);
    }
    if hidden_paths != u32::MAX {
        for frame in 0..u32::from(asset.loop_frames.max(1)) {
            let list = stage.draws(&identity(frame), hidden_paths);
            let mut walkers = vec![0u32; count];
            for draw in &list {
                if let Some(layer) = resolved(asset, variant, usize::from(draw.layer)) {
                    walkers[layer] += 1;
                }
            }
            for draw in &list {
                let Some(layer) = resolved(asset, variant, usize::from(draw.layer)) else {
                    continue;
                };
                let bounds = image_of(layer, keys);
                add(&mut layered, layer, draw.place_rect(bounds), walkers[layer]);
            }
        }
        for (_, layer) in &slots {
            let layer = *layer;
            let LayerContent::Sprite(sprite) = &asset.layers[layer].content else {
                continue;
            };
            let Some(path) = &sprite.path else {
                continue;
            };
            if !stage.roams(layer) {
                continue;
            }
            let bounds = image_of(layer, keys);
            let walkers = u32::from(path.walkers.count.max(1));
            add(
                &mut layered,
                layer,
                stage.route_reach(layer, bounds),
                walkers,
            );
        }
    }
    let mut cpu = Cpu {
        molang: statics.0,
        letters: statics.1,
        palette: statics.2,
        ..Cpu::default()
    };
    let mut layer_cpu = vec![0u32; count];
    let mut fallback_frames = vec![None; count];
    let mut build_ops = 0u64;
    for (_, layer) in &slots {
        let layer = *layer;
        let spec = &asset.layers[layer];
        match &spec.content {
            LayerContent::Body(body) => {
                image_of(layer, keys);
                let rect = body_rect(body, spec.of, &stage.placement.block);
                let blends = body
                    .fill
                    .iter()
                    .chain(&body.rings)
                    .any(|f| matches!(f, Fill::Blend { .. }));
                if blends && rect.height() > NAMEPLATE_BLEND_ROWS as i32 {
                    out.blend_rows[layer] = out.blend_rows[layer].max(rect.height());
                }
            }
            LayerContent::Sprite(sprite) => {
                image_of(layer, keys);
                let n = layered[layer].as_ref().map_or(0, |l| l.copies);
                if let Some(path) = &sprite.path {
                    let walkers = u32::from(path.walkers.count.max(1));
                    cpu.paths += walkers * NAMEPLATE_COST_WALKER;
                    layer_cpu[layer] += walkers * NAMEPLATE_COST_WALKER;
                } else if sprite.repeat.is_some() {
                    cpu.repeats += n * NAMEPLATE_COST_REPEAT_COPY;
                    layer_cpu[layer] += n * NAMEPLATE_COST_REPEAT_COPY;
                }
            }
            LayerContent::Emitter(emitter) => {
                let bounds = image_of(layer, keys);
                let live = worst_case_live(asset, layer);
                let checks = emitter.rate.map_or(0, |rate| {
                    let every = u32::from(rate.every.max(1));
                    ((u32::from(emitter.lifetime[1]) + u32::from(rate.jitter)) / every + 1)
                        * u32::from(rate.count)
                });
                let units = live
                    .saturating_mul(NAMEPLATE_COST_PARTICLE)
                    .saturating_add(checks.saturating_mul(NAMEPLATE_COST_SPAWN_CHECK));
                cpu.particles = cpu.particles.saturating_add(units);
                layer_cpu[layer] += units;
                if live > 0 {
                    add(
                        &mut layered,
                        layer,
                        particle_extent(stage, layer, bounds),
                        live,
                    );
                }
            }
            LayerContent::Material(content) => {
                let Some(ir) = asset.materials.get(usize::from(content.material)) else {
                    continue;
                };
                let body = match spec.of {
                    Of::Block => stage.placement.block.background,
                    _ => stage.layout.targets.get(layer).copied().unwrap_or_default(),
                };
                let area = ir.area(body);
                let mirrored = spec.mirror.is_some();
                let n = 1 + u32::from(mirrored);
                let mut rect = area;
                if mirrored {
                    rect = rect.union(mirror_area(area, body));
                }
                add(&mut layered, layer, rect, n);
                cpu.materials += n * NAMEPLATE_COST_MATERIAL_LAYER;
                layer_cpu[layer] += n * NAMEPLATE_COST_MATERIAL_LAYER;
                let frames = material_frames(asset, layer, ir, content.drift);
                fallback_frames[layer] = Some(Fallback {
                    frame_bytes: u64::from(fallback_bytes(area)) * u64::from(n),
                    frames,
                    drift: content.drift,
                });
                let ops = pixels(area)
                    .saturating_mul(u64::from(n))
                    .saturating_mul(u64::from(ir.cost))
                    .saturating_mul(u64::from(frames));
                out.effect_ops[layer] = out.effect_ops[layer].max(ops);
                build_ops = build_ops.saturating_add(ops);
            }
        }
    }
    let mut quads = 0u32;
    for (_, layer) in &slots {
        let layer = *layer;
        let n = layered[layer].as_ref().map_or(0, |l| l.copies);
        let q = match &asset.layers[layer].content {
            LayerContent::Material(_) => n,
            _ => images[layer]
                .as_ref()
                .map_or(0, |i| i.quads)
                .saturating_mul(n),
        };
        quads_of[layer] = q;
        quads = quads.saturating_add(q);
    }
    for (layer, entry) in layered.iter_mut().enumerate() {
        let Some(entry) = entry else {
            continue;
        };
        entry.quads = quads_of[layer];
        entry.extent = layer_span(asset, &ranges, layer).apply(entry.raw);
    }
    let extents: Vec<Rect> = layered
        .iter()
        .map(|l| l.as_ref().map_or(Rect::EMPTY, |l| l.extent))
        .collect();
    for (layer, entry) in layered.iter().enumerate() {
        let Some(entry) = entry else {
            continue;
        };
        let clipped = match asset.layers[layer].mask {
            Some(Mask::Name) => entry.extent.intersect(stage.layout.name),
            Some(Mask::Layer { slot, .. }) => match resolved(asset, variant, usize::from(slot)) {
                Some(target) => entry.extent.intersect(extents[target]),
                None => Rect::EMPTY,
            },
            None => entry.extent,
        };
        let px = breach(clipped, background);
        if px > out.margins[layer].0 {
            out.margins[layer] = (px, place.clone());
        }
        let window = clipped.intersect(CANVAS);
        if window != clipped && out.outside[layer].is_none() {
            out.outside[layer] = Some(place.clone());
        }
        out.extents[layer].push((stage.input, stage.placement.variant, clipped));
    }
    if let Some(badge) = stage.layout.badge {
        let follow = match variant.badge.placement {
            BadgePlacement::Placed(placed) => follow_chain(asset, &ranges, placed.of),
            _ => Default::default(),
        };
        let span = ranges.badge.plus(follow).plus(ranges.plate);
        let px = breach(span.apply(badge), background);
        if px > out.badge.0 {
            out.badge = (px, place.clone());
        }
    }
    out.plate = out.plate.max(ranges.plate.reach());
    out.build_ops = out.build_ops.max(build_ops);
    let total = cpu.total();
    if total > out.cpu.0.total() || out.cpu.1.variant.is_empty() {
        out.cpu = (cpu, place.clone());
    }
    if quads > out.quads.0 || out.quads.1.variant.is_empty() {
        out.quads = (quads, place.clone());
    }
    for layer in 0..count {
        let entry = &mut per_layer[layer];
        entry.quads = entry.quads.max(quads_of[layer]);
        entry.cpu_units = entry.cpu_units.max(layer_cpu[layer]);
        if let Some(found) = fallback_frames[layer].take() {
            let keep = out.fallbacks[layer]
                .as_ref()
                .is_none_or(|f| f.frame_bytes < found.frame_bytes);
            if keep {
                out.fallbacks[layer] = Some(found);
            }
        }
    }
}

pub fn measure(asset: &NameplateAsset) -> Measured {
    let count = asset.layers.len();
    let mut out = Measured {
        margins: vec![(0, Place::default()); count],
        outside: vec![None; count],
        blend_rows: vec![0; count],
        fallbacks: vec![None; count],
        extents: vec![Vec::new(); count],
        effect_ops: vec![0; count],
        ..Measured::default()
    };
    let mut per_layer: Vec<LayerAnalysis> = asset
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| LayerAnalysis {
            id: l.id.clone(),
            states: u32::from(layer_states(asset, u8::try_from(i).unwrap_or(u8::MAX))),
            ..LayerAnalysis::default()
        })
        .collect();
    let statics = static_cpu(asset);
    let mut keys: BTreeMap<(usize, u16, u64), u32> = BTreeMap::new();
    let references = reference_inputs();
    let extra = breakpoint_inputs(asset);
    let variants = distinct_variants(asset);
    for (index, _) in asset.variants.iter().enumerate() {
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        let full = variants.contains(&index);
        for input in &references {
            let Some(stage) = Stage::new(asset, index, input) else {
                continue;
            };
            if full {
                measure_stage(&stage, true, statics, &mut keys, &mut out, &mut per_layer);
            } else {
                for layer in 0..count {
                    if !matches!(asset.layers[layer].content, LayerContent::Material(_)) {
                        layer_images(&stage, layer, Some(&mut keys));
                    }
                }
            }
        }
        if full {
            for input in &extra {
                if let Some(stage) = Stage::new(asset, index, input) {
                    measure_stage(&stage, false, statics, &mut keys, &mut out, &mut per_layer);
                }
            }
        }
    }
    let mut image_bytes = 0u32;
    for ((layer, _, _), bytes) in &keys {
        image_bytes = image_bytes.saturating_add(*bytes);
        let entry = &mut per_layer[*layer];
        entry.image_bytes = entry.image_bytes.saturating_add(*bytes);
    }
    let fallback: u64 = out
        .fallbacks
        .iter()
        .flatten()
        .map(|f| f.frame_bytes * u64::from(f.frames))
        .sum();
    for (entry, (px, _)) in per_layer.iter_mut().zip(&out.margins) {
        entry.margin = px.unsigned_abs();
    }
    let effects = effects(asset, &out);
    out.analysis = Analysis {
        requires: asset.requires.clone(),
        quads: out.quads.0,
        cpu_units: out.cpu.0.total(),
        image_bytes,
        fallback_bytes: u32::try_from(fallback).unwrap_or(u32::MAX),
        record_bytes: u32::try_from(record_bytes(asset.record)).unwrap_or(u32::MAX),
        max_out_words: asset.max_out_words,
        material_build_ops: out.build_ops,
        layers: per_layer,
        effects,
    };
    out
}

pub fn check(asset: &NameplateAsset, measured: &Measured, report: &mut Report) {
    let analysis = &measured.analysis;
    let quad_cap = quad_limit(Kind::Nameplate);
    if analysis.quads > quad_cap {
        report.push(
            measured.quads.1.tag(
                Issue::error(IssueCode::QuadsOverBudget, MANIFEST)
                    .value("quads", f64::from(analysis.quads))
                    .value("limit", f64::from(quad_cap)),
            ),
        );
    }
    let cpu = measured.cpu.0;
    if cpu.total() > NAMEPLATE_CPU_UNITS {
        report.push(
            measured.cpu.1.tag(
                Issue::error(IssueCode::OverCpu, MANIFEST)
                    .value("units", f64::from(cpu.total()))
                    .value("limit", f64::from(NAMEPLATE_CPU_UNITS))
                    .value("molang", f64::from(cpu.molang))
                    .value("letters", f64::from(cpu.letters))
                    .value("paths", f64::from(cpu.paths))
                    .value("repeats", f64::from(cpu.repeats))
                    .value("particles", f64::from(cpu.particles))
                    .value("palette", f64::from(cpu.palette))
                    .value("materials", f64::from(cpu.materials)),
            ),
        );
    }
    if analysis.image_bytes > NAMEPLATE_IMAGE_BYTES {
        report.push(
            Issue::error(IssueCode::OverCache, MANIFEST)
                .value("bytes", f64::from(analysis.image_bytes))
                .value("limit", f64::from(NAMEPLATE_IMAGE_BYTES)),
        );
    }
    for (layer, spec) in asset.layers.iter().enumerate() {
        let at = layer_path(&spec.id);
        let states = analysis.layers[layer].states;
        let checked = match &spec.content {
            LayerContent::Emitter(_) => spec.mask.is_some(),
            LayerContent::Material(_) => false,
            _ => true,
        };
        if checked && states > NAMEPLATE_CACHED_STATES {
            report.push(
                Issue::error(IssueCode::OverStates, at.clone())
                    .value("states", f64::from(states))
                    .value("limit", f64::from(NAMEPLATE_CACHED_STATES)),
            );
        }
        let (px, place) = &measured.margins[layer];
        if *px > NAMEPLATE_DECORATION_MARGIN as i32 {
            report.push(
                place.tag(
                    Issue::error(IssueCode::OverMargin, at.clone())
                        .value("px", f64::from(*px))
                        .value("limit", f64::from(NAMEPLATE_DECORATION_MARGIN)),
                ),
            );
        }
        if report.mode() == Mode::Authoring {
            if let Some(place) = &measured.outside[layer] {
                report.push(place.tag(Issue::error(IssueCode::ArtOutsideWindow, at.clone())));
            }
        }
        let rows = measured.blend_rows[layer];
        if rows > 0 {
            report.push(
                Issue::error(
                    IssueCode::Layer,
                    path(&format!("layers/{}.json", spec.id), "fill"),
                )
                .value("rows", f64::from(rows))
                .value("limit", f64::from(NAMEPLATE_BLEND_ROWS)),
            );
        }
    }
    let (px, place) = &measured.badge;
    if *px > NAMEPLATE_DECORATION_MARGIN as i32 {
        report.push(
            place.tag(
                Issue::error(IssueCode::OverMargin, path(MANIFEST, "badge"))
                    .value("px", f64::from(*px))
                    .value("limit", f64::from(NAMEPLATE_DECORATION_MARGIN)),
            ),
        );
    }
    if measured.plate > NAMEPLATE_DECORATION_MARGIN as i32 {
        let clip = clips(asset)
            .into_iter()
            .find(|(c, _)| c.targets.iter().any(|t| t.target == Target::Plate))
            .map_or_else(String::new, |(c, _)| c.name.clone());
        report.push(
            Issue::error(
                IssueCode::OverMargin,
                format!("{}.tracks.$plate", animation_path(&clip)),
            )
            .value("px", f64::from(measured.plate))
            .value("limit", f64::from(NAMEPLATE_DECORATION_MARGIN)),
        );
    }
    let emitters: Vec<CostEmitter> = asset
        .layers
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l.content, LayerContent::Emitter(_)))
        .map(|(i, l)| CostEmitter {
            path: layer_path(&l.id),
            rate: 0.0,
            longest_life: 0.0,
            bursts: vec![worst_case_live(asset, i)],
        })
        .collect();
    if !emitters.is_empty() {
        check_emitters(MANIFEST, &emitters, report);
    }
    fallbacks(asset, measured, report);
}

fn fallbacks(asset: &NameplateAsset, measured: &Measured, report: &mut Report) {
    let cap = u64::from(NAMEPLATE_FALLBACK_BYTES);
    let still: u64 = measured
        .fallbacks
        .iter()
        .flatten()
        .map(|f| f.frame_bytes)
        .sum();
    if still > cap {
        report.push(
            Issue::error(IssueCode::OverFallback, MANIFEST)
                .value("bytes", still as f64)
                .value("limit", cap as f64),
        );
    }
    for (layer, fallback) in measured.fallbacks.iter().enumerate() {
        let Some(fallback) = fallback else {
            continue;
        };
        let animated = fallback.frame_bytes * u64::from(fallback.frames);
        let code = if fallback.drift {
            IssueCode::FallbackRebuild
        } else if animated > cap {
            IssueCode::FallbackStill
        } else {
            IssueCode::FallbackCache
        };
        report.push(
            Issue::info(code, layer_path(&asset.layers[layer].id))
                .value("bytes", animated as f64)
                .value("frames", f64::from(fallback.frames))
                .value("frame_bytes", fallback.frame_bytes as f64)
                .value("limit", cap as f64),
        );
    }
}
