use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::iter::once;
use std::mem::{replace, take};
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::container::Image;
use crate::cost::{Detail, Motion};
use crate::limits::NAMEPLATE_FALLBACK_BYTES;
use crate::molang::Runtime;
use crate::palette::{MAX_SLOTS, MAX_TONES};
use crate::signals::SIGNAL_COUNT;
use crate::wearer::{CosmeticSlot, WearerRecord, WearerTable, WEARER_IDLE_SECONDS};

use super::abi::*;
use super::anim::channels::{letters as letter_program, pose_layers};
use super::anim::particles::{
    emit_in, looped_starts, route_key as particle_route_key, run_starts, run_starts_until,
    BurstStart, SpawnArea,
};
use super::anim::path::{cycle as path_cycle, route_for, walk_gate, walker_pose, Walk};
use super::anim::repeat::{positions, route_key as repeat_route_key};
use super::anim::speed::{
    advance, episode_floats, offset as speed_offset, primary_layer, read_episode, rejoin_frame,
    speed_clips, ClipClock, Profile,
};
use super::anim::tracks::Channel;
use super::anim::triggers::fire as fire_one_shot;
use super::anim::{
    animate_palette, clip as anim_clip, pose, ClipRef, ClipTime, Gates, Moment, Scratch, Tail,
};
use super::cache::{ImageCache, DEFAULT_CACHE_BYTES};
use super::format::MAX_CLIPS;
use super::geom::{Block, LayoutResult, Placement, Rect};
use super::image::{Axis, ImageKey, LayerImage};
use super::layout::{anchor_point, build_block, pivot_offset, resolve_with, scaled_corners};
use super::material::cpu::{evaluate_with, fallback_bytes, TextureSource};
use super::material::ir::{
    Blend, EdgeLane, Lane, MaterialFrame, MaterialIr, Period, TexRef, LANE_COUNT, MAX_SAMPLES,
};
use super::material::uniforms::write as write_uniforms;
use super::paint::{mask_state, paint_layer, placement_inputs};
use super::palette::{base_frame, custom_variant, resolve_ref};
use super::queries::{NameplateQuery, ParamQueries, QuerySlots};
use super::raster::material_time;
use super::route::{nearest_step, Route, RouteCache, RouteKey};
use super::text::{
    adjust, resolve_badge, resolve_text, run_letters, Backdrop, ResolvedBadge, ResolvedText,
};
use super::types::{
    BadgePose, Body, Breakpoint, Decoration, Formatting, LayerContent, LetterOverride, Mask,
    NameplateAsset, Of, Paint, PaletteFrame, PaletteTrack, PaletteValue, ParticlePose, Reduced,
    SpriteLayer, TagPose, Target, TextMode, Track, Value, Variant, MAX_LAYERS, MAX_LETTERS, OPAQUE,
};
use super::FrameError;

const BIT_PALETTE: u64 = 1 << 32;
const BIT_LETTERS: u64 = 1 << 33;
const BIT_BADGE: u64 = 1 << 34;
const BIT_PLATE: u64 = 1 << 35;
const LAYER_BITS: u64 = (1 << MAX_LAYERS) - 1;
const LAYOUT_ENTRIES: usize = 256;
const WALK_ENTRIES: usize = 64;
const PROFILE_ENTRIES: usize = 64;
const TEXT_ENTRIES: usize = 256;
const ADJUST_ENTRIES: usize = 512;
const CPU_ENTRIES: usize = 512;
const QUEUED_FALLBACKS: usize = 64;
const FALLBACK_WAIT_FRAMES: u64 = 240;
const BACKDROP_ENTRIES: usize = 8;
const STRIP_ENTRIES: usize = 4096;
const FRONT_IDLE_FRAMES: u64 = 240;
const FRONT_SWEEP_FRAMES: u64 = 64;
const EQUIP_SWEEP_SECONDS: f64 = 1.0;
const BACKDROP_FRAMES: usize = 2400;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(FNV_OFFSET)
    }

    fn push(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    fn rect(&mut self, r: Rect) {
        for v in [r.x0, r.y0, r.x1, r.y1] {
            self.push(v as u32 as u64);
        }
    }
}

fn multiply(a: u8, b: u8) -> u8 {
    ((u32::from(a) * u32::from(b) + 127) / 255) as u8
}

fn fade(argb: u32, opacity: u8) -> u32 {
    if opacity == OPAQUE {
        return argb;
    }
    let alpha = ((argb >> 24) * u32::from(opacity) + 127) / 255;
    (alpha << 24) | (argb & 0x00ff_ffff)
}

#[derive(Debug, Default)]
struct Texels {
    store: Vec<Vec<u32>>,
    free: Vec<u32>,
    strips: BTreeMap<u64, u32>,
    layout: u32,
    retired: Vec<u32>,
}

impl Texels {
    fn get(&self, id: u32) -> &[u32] {
        self.store
            .get(id as usize)
            .map_or(&[][..], |t| t.as_slice())
    }

    fn put(&mut self, texels: Vec<u32>) -> u32 {
        self.layout = self.layout.wrapping_add(1);
        match self.free.pop() {
            Some(id) => {
                self.store[id as usize] = texels;
                id
            }
            None => {
                self.store.push(texels);
                (self.store.len() - 1) as u32
            }
        }
    }

    fn release(&mut self, id: u32) {
        if (id as usize) < self.store.len() {
            self.retired.push(id);
        }
    }

    fn begin_frame(&mut self) {
        if self.retired.is_empty() {
            return;
        }
        for id in self.retired.drain(..) {
            self.store[id as usize] = Vec::new();
            self.free.push(id);
        }
        self.layout = self.layout.wrapping_add(1);
    }

    fn strip(&mut self, tones: &[u32]) -> u32 {
        let mut hash = Fnv::new();
        for tone in tones {
            hash.push(u64::from(*tone));
        }
        if let Some(&id) = self.strips.get(&hash.0) {
            if self.get(id) == tones {
                return id;
            }
        }
        if self.strips.len() >= STRIP_ENTRIES {
            let ids: Vec<u32> = self.strips.values().copied().collect();
            self.strips.clear();
            for id in ids {
                self.release(id);
            }
        }
        let id = self.put(tones.to_vec());
        self.strips.insert(hash.0, id);
        id
    }
}

struct HostTextures<'a>(&'a BTreeMap<String, Image>);

impl TextureSource for HostTextures<'_> {
    fn texture(&self, texture: &TexRef) -> Option<&Image> {
        match texture {
            TexRef::Bundle(_) => None,
            TexRef::Vanilla(name) => self.0.get(name.as_str()),
            TexRef::Atlas(id) => self.0.get(id.as_str()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SpeedClip {
    clip: ClipRef,
    primary: Option<u8>,
}

struct LayoutMemo {
    placement: Placement,
    result: LayoutResult,
    inputs: Vec<u64>,
    key: u32,
    backdrops: Vec<BackdropMemo>,
    used: u64,
}

struct BackdropMemo {
    palette: u64,
    under_name: Vec<u32>,
    under_badge: Vec<u32>,
    used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WalkKey {
    layer: u8,
    target: Rect,
    block: Block,
    size: [u16; 2],
}

struct WalkMemo {
    key: WalkKey,
    route: Route,
    walk: Walk,
    used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProfileKey {
    clip: usize,
    layer: u8,
    origin: Option<i64>,
}

struct ProfileMemo {
    key: ProfileKey,
    profile: Profile,
    used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextKey {
    layout: u32,
    palette: u64,
    external: Option<u32>,
    fingerprint: u64,
}

struct TextMemo {
    key: TextKey,
    text: ResolvedText,
    badge: ResolvedBadge,
    used: u64,
}

struct AdjustMemo {
    layout: u32,
    palette: u64,
    color: u32,
    adjusted: u32,
    used: u64,
}

struct CpuMemo {
    key: u64,
    stale: u64,
    id: u32,
    bytes: u32,
    used: u64,
    made: u64,
}

pub struct FallbackJob {
    asset: Arc<NameplateAsset>,
    token: Arc<()>,
    id: u32,
    key: u64,
    stale: u64,
    material: usize,
    area: Rect,
    body: Rect,
    clock: [i32; 4],
    lanes: [f64; LANE_COUNT],
    tones: Vec<u32>,
    cover: Option<Vec<Rect>>,
    mirror: Option<i32>,
    shift: (i32, i32),
    textures: Arc<BTreeMap<String, Image>>,
    bytes: u32,
}

pub struct FallbackImage {
    token: Arc<()>,
    id: u32,
    key: u64,
    stale: u64,
    pixels: Vec<u32>,
    bytes: u32,
}

impl FallbackJob {
    pub fn asset_id(&self) -> u32 {
        self.id
    }

    pub fn run(self) -> FallbackImage {
        let pixels = catch_unwind(AssertUnwindSafe(|| self.pixels())).unwrap_or_else(|_| {
            let area = self.area;
            vec![0; area.width().max(0) as usize * area.height().max(0) as usize]
        });
        FallbackImage {
            token: self.token,
            id: self.id,
            key: self.key,
            stale: self.stale,
            pixels,
            bytes: self.bytes,
        }
    }

    fn pixels(&self) -> Vec<u32> {
        let Some(ir) = self.asset.materials.get(self.material) else {
            return Vec::new();
        };
        let frame = MaterialFrame {
            area: self.area,
            body: self.body,
            clock: self.clock,
            lanes: self.lanes,
            tones: &self.tones,
            atlas: [[0.0; 4]; MAX_SAMPLES],
        };
        let host = HostTextures(&self.textures);
        let mut pixels = evaluate_with(ir, &frame, &self.asset.textures, &host);
        if let Some(cover) = &self.cover {
            let area = self.area;
            let width = area.width().max(1) as usize;
            for (i, pixel) in pixels.iter_mut().enumerate() {
                let x = area.x0 + (i % width) as i32;
                let y = area.y0 + (i / width) as i32;
                let px = self.mirror.map_or(x, |axis| axis - x) + self.shift.0;
                let py = y + self.shift.1;
                if !cover.iter().any(|r| r.contains(px, py)) {
                    *pixel = 0;
                }
            }
        }
        pixels
    }
}

impl FallbackImage {
    pub fn asset_id(&self) -> u32 {
        self.id
    }
}

struct Entry {
    asset: Arc<NameplateAsset>,
    token: Arc<()>,
    slot: CosmeticSlot,
    runtime: Runtime,
    slots: QuerySlots,
    owners: Vec<u64>,
    speed: Vec<SpeedClip>,
    counts: Vec<[u8; MAX_SLOTS]>,
    disabled: u64,
    failed: bool,
    reported: bool,
    routes: RouteCache,
    layouts: Vec<LayoutMemo>,
    next_layout: u32,
    front: BTreeMap<ImageKey, (Arc<LayerImage>, u64)>,
    walks: Vec<WalkMemo>,
    profiles: Vec<ProfileMemo>,
    texts: Vec<TextMemo>,
    adjusted: Vec<AdjustMemo>,
    cpu: Vec<CpuMemo>,
    cpu_bytes: u32,
    waiting: Vec<(u64, u64)>,
    palettes: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Equip {
    asset: u64,
    variant: u64,
    seen: f64,
}

struct Custom {
    colors: Vec<(u8, u32)>,
    resolved: Option<(u16, Arc<Variant>, u64)>,
}

impl Custom {
    fn variant(
        &mut self,
        asset: &NameplateAsset,
        index: u16,
        designed: &Variant,
    ) -> (Arc<Variant>, u64) {
        if let Some((at, variant, palette)) = &self.resolved {
            if *at == index {
                return (variant.clone(), *palette);
            }
        }
        let variant = Arc::new(custom_variant(asset, designed, &self.colors));
        let palette = palette_key(&variant);
        self.resolved = Some((index, variant.clone(), palette));
        (variant, palette)
    }
}

fn palette_key(variant: &Variant) -> u64 {
    let mut hash = Fnv::new();
    hash.push(variant.tones.len() as u64);
    for tone in &variant.tones {
        hash.push(u64::from(*tone));
    }
    for stops in &variant.stops {
        hash.push(stops.len() as u64);
        for stop in stops {
            hash.push(u64::from(*stop));
        }
    }
    hash.0
}

#[derive(Clone, Debug)]
enum Source {
    Image(Arc<LayerImage>),
    Material {
        clock: [i32; 4],
        lanes: [f64; LANE_COUNT],
    },
}

#[derive(Clone, Debug)]
struct Item {
    slot: u8,
    layer: u8,
    band: u8,
    source: Source,
    dx: i32,
    dy: i32,
    mirror: Option<i32>,
    opacity: u8,
}

impl Item {
    fn place_rect(&self, r: Rect) -> Rect {
        let (x0, x1) = match self.mirror {
            Some(axis) => (axis - r.x1 + 1, axis - r.x0 + 1),
            None => (r.x0, r.x1),
        };
        Rect::new(x0, r.y0, x1, r.y1).offset(self.dx, self.dy)
    }

    fn unplace_rect(&self, r: Rect) -> Rect {
        let r = r.offset(-self.dx, -self.dy);
        match self.mirror {
            Some(axis) => Rect::new(axis - r.x1 + 1, r.y0, axis - r.x0 + 1, r.y1),
            None => r,
        }
    }

    fn clipped(&self, src: Rect, clip: Option<Rect>) -> Option<(Rect, Rect)> {
        let src = match clip {
            Some(c) => src.intersect(self.unplace_rect(c)),
            None => src,
        };
        (!src.is_empty()).then(|| (src, self.place_rect(src)))
    }
}

#[derive(Default)]
struct Work {
    anim: Scratch,
    pose: Box<TagPose>,
    delayed: Box<TagPose>,
    delayed_shown: Vec<ClipTime>,
    items: Vec<Item>,
    particles: Vec<ParticlePose>,
    bursts: Vec<BurstStart>,
    spots: Vec<(i32, i32)>,
    cover: Vec<Rect>,
    materials: Vec<i32>,
    events: Vec<(i32, i32)>,
    failures: Vec<u32>,
    speeds: Vec<(usize, usize)>,
}

struct Writer<'o> {
    buf: &'o mut [i32],
    at: usize,
    overflow: bool,
}

impl Writer<'_> {
    fn push(&mut self, word: i32) {
        match self.buf.get_mut(self.at) {
            Some(slot) => {
                *slot = word;
                self.at += 1;
            }
            None => self.overflow = true,
        }
    }

    fn extend(&mut self, words: &[i32]) {
        for word in words {
            self.push(*word);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn op(&mut self, r: Rect, argb: u32, band: u8, slot: u8, kind: i32, aux0: i32, aux1: i32) {
        self.extend(&[
            r.x0,
            r.y0,
            r.x1,
            r.y1,
            argb as i32,
            pack_band_layer_kind(i32::from(band), i32::from(slot), kind),
            aux0,
            aux1,
        ]);
    }
}

pub struct NameplateRenderer {
    wearers: WearerTable,
    motion: Motion,
    assets: Vec<Option<Entry>>,
    images: ImageCache,
    texels: Texels,
    equips: BTreeMap<u64, Equip>,
    customs: BTreeMap<(u64, u32), Custom>,
    textures: Arc<BTreeMap<String, Image>>,
    texture_generation: u64,
    queue: VecDeque<FallbackJob>,
    made: u64,
    work: Work,
    tick: u64,
    swept: f64,
}

impl Default for NameplateRenderer {
    fn default() -> NameplateRenderer {
        NameplateRenderer::new()
    }
}

fn group_bits(asset: &NameplateAsset, group: u8) -> u64 {
    let mut bits = 0;
    for (index, candidate) in asset.groups.iter().enumerate() {
        let mut at = Some(index);
        let mut depth = 0;
        while let Some(current) = at {
            if current == usize::from(group) {
                for layer in &candidate.layers {
                    if usize::from(*layer) < MAX_LAYERS {
                        bits |= 1 << layer;
                    }
                }
                break;
            }
            if depth > asset.groups.len() {
                break;
            }
            at = asset.groups[current].parent.map(usize::from);
            depth += 1;
        }
    }
    bits
}

fn program_owners(asset: &NameplateAsset) -> Vec<u64> {
    let mut owners = vec![0u64; asset.molang.programs.len()];
    let mut mark = |program: u32, bits: u64| {
        if let Some(owner) = owners.get_mut(program as usize) {
            *owner |= bits;
        }
    };
    let animations = &asset.animations;
    let clips = once(&animations.looping)
        .chain(
            animations
                .schedules
                .iter()
                .flat_map(|s| s.events.iter().map(|e| &e.clip)),
        )
        .chain(animations.reactions.iter().map(|r| &r.clip))
        .chain(animations.one_shots.iter().map(|o| &o.clip));
    for clip in clips {
        for target in &clip.targets {
            let bits = match target.target {
                Target::Layer(layer) if usize::from(layer) < MAX_LAYERS => 1 << layer,
                Target::Layer(_) => 0,
                Target::Group(group) => group_bits(asset, group),
                Target::Badge => BIT_BADGE,
                Target::Plate => BIT_PLATE,
            };
            let c = &target.channels;
            let tracks: [&Option<Track>; 6] =
                [&c.frame, &c.x, &c.y, &c.opacity, &c.visible, &c.speed];
            for track in tracks.into_iter().flatten() {
                for key in &track.keys {
                    if let Value::Molang(program) = key.value {
                        mark(program, bits);
                    }
                }
            }
        }
        for track in &clip.palette {
            if let PaletteTrack::Keys(keys) = &track.track {
                for key in keys {
                    for tone in &key.tones {
                        if let PaletteValue::Molang(program) = tone {
                            mark(*program, BIT_PALETTE);
                        }
                    }
                }
            }
        }
        if let Some(LetterOverride::Program(program)) = clip.letters {
            mark(program, BIT_LETTERS);
        }
    }
    let styles = once(&asset.text).chain(asset.variants.iter().map(|v| &v.text));
    for style in styles {
        if let Some(program) = style.motion {
            mark(program, BIT_LETTERS);
        }
    }
    for (index, layer) in asset.layers.iter().enumerate().take(MAX_LAYERS) {
        if let LayerContent::Material(content) = &layer.content {
            if let Some(ir) = asset.materials.get(usize::from(content.material)) {
                for lane in &ir.lanes {
                    if let Lane::Expr(program) = lane {
                        mark(*program, 1 << index);
                    }
                }
            }
        }
    }
    owners
}

fn speed_layers(asset: &NameplateAsset) -> Vec<SpeedClip> {
    speed_clips(&asset.animations)
        .into_iter()
        .map(|clip| SpeedClip {
            clip,
            primary: primary_layer(asset, clip).and_then(|l| u8::try_from(l).ok()),
        })
        .collect()
}

fn tone_counts(asset: &NameplateAsset) -> Vec<[u8; MAX_SLOTS]> {
    asset
        .variants
        .iter()
        .map(|variant| {
            let mut counts = [0u8; MAX_SLOTS];
            let colorway = variant.colorway.map(usize::from);
            for (slot, count) in counts
                .iter_mut()
                .enumerate()
                .take(asset.palette.slots.len())
            {
                *count = asset.palette.tone_count(slot, colorway).min(MAX_TONES) as u8;
            }
            counts
        })
        .collect()
}

fn lru<T>(list: &[T], used: impl Fn(&T) -> u64) -> usize {
    list.iter()
        .enumerate()
        .min_by_key(|(_, item)| used(item))
        .map_or(0, |(i, _)| i)
}

impl Entry {
    fn layout(&mut self, placement: &Placement, tick: u64) -> usize {
        if let Some(index) = self.layouts.iter().position(|m| m.placement == *placement) {
            self.layouts[index].used = tick;
            return index;
        }
        let asset = &self.asset;
        let result = resolve_with(asset, placement, &mut self.routes);
        let inputs = (0..asset.layers.len())
            .map(|layer| placement_inputs(asset, layer as u8, placement))
            .collect();
        self.next_layout = self.next_layout.wrapping_add(1);
        let memo = LayoutMemo {
            placement: *placement,
            result,
            inputs,
            key: self.next_layout,
            backdrops: Vec::new(),
            used: tick,
        };
        if self.layouts.len() >= LAYOUT_ENTRIES {
            let index = lru(&self.layouts, |m| m.used);
            self.layouts[index] = memo;
            index
        } else {
            self.layouts.push(memo);
            self.layouts.len() - 1
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fetch(
    front: &mut BTreeMap<ImageKey, (Arc<LayerImage>, u64)>,
    images: &mut ImageCache,
    tick: u64,
    id: u32,
    asset: &NameplateAsset,
    placement: &Placement,
    layout: &LayoutResult,
    inputs: &[u64],
    layer: u8,
    state: u16,
) -> Arc<LayerImage> {
    let key = ImageKey {
        asset: id,
        layer,
        state,
        inputs: inputs.get(usize::from(layer)).copied().unwrap_or(0),
    };
    if let Some((image, used)) = front.get_mut(&key) {
        *used = tick;
        return image.clone();
    }
    let image = images.get_or_paint(key, || paint_layer(asset, layer, state, placement, layout));
    front.insert(key, (image.clone(), tick));
    image
}

struct View<'a> {
    asset: &'a NameplateAsset,
    variant: &'a Variant,
    placement: &'a Placement,
    layout: &'a LayoutResult,
}

impl View<'_> {
    fn resolved(&self, slot: usize) -> Option<usize> {
        match self.variant.slots.get(slot) {
            Some(target) => target.map(usize::from),
            None => Some(slot),
        }
        .filter(|layer| *layer < self.asset.layers.len())
    }

    fn shown(&self, slot: usize, layer: usize) -> bool {
        let width = self.placement.block.name_width;
        let fits = |breakpoints: &[Breakpoint]| {
            breakpoints.is_empty() || breakpoints.iter().any(|b| b.contains(width))
        };
        if !fits(&self.asset.layers[layer].breakpoints) {
            return false;
        }
        let groups = &self.asset.groups;
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

    fn target(&self, layer: usize) -> Rect {
        self.layout
            .targets
            .get(layer)
            .copied()
            .unwrap_or(self.layout.bounds)
    }

    fn body_of(&self, of: Of) -> Option<&Body> {
        body_of(self.asset, of)
    }

    fn material_body(&self, layer: usize) -> Rect {
        match self.asset.layers[layer].of {
            Of::Block => self.placement.block.background,
            _ => self.target(layer),
        }
    }

    fn sprite_size(&self, layer: usize, sprite: &SpriteLayer) -> (i32, i32) {
        let width = self.placement.block.name_width;
        let chosen = self.asset.layers[layer]
            .breakpoints
            .iter()
            .find(|b| b.contains(width))
            .and_then(|b| b.sprite)
            .unwrap_or(sprite.sprite);
        self.asset
            .sprites
            .get(usize::from(chosen))
            .map_or((0, 0), |s| (i32::from(s.width), i32::from(s.height)))
    }

    fn spot(
        &self,
        routes: &mut RouteCache,
        layer: usize,
        sprite: &SpriteLayer,
        mirrored: bool,
    ) -> (i32, i32) {
        let target = self.target(layer);
        let block = &self.placement.block;
        let (width, height) = self.sprite_size(layer, sprite);
        let (ax, ay) = anchor_point(sprite.anchor, sprite.at, mirrored, target, block);
        let (x, y) = match sprite.follow {
            Some(follow) => {
                let size = [width, height].map(|v| u16::try_from(v.max(0)).unwrap_or(u16::MAX));
                let key = RouteKey::new(
                    u8::try_from(layer).unwrap_or(u8::MAX),
                    self.body_of(self.asset.layers[layer].of),
                    target,
                    Some(follow),
                    None,
                    size,
                );
                let route = routes.route(key);
                route.at(i64::try_from(nearest_step(route, ax, ay)).unwrap_or(0))
            }
            None => {
                let pivot = if mirrored {
                    sprite.pivot.mirrored()
                } else {
                    sprite.pivot
                };
                let (px, py) = pivot_offset(pivot, width, height);
                (ax - px, ay - py)
            }
        };
        let [ox, oy] = sprite.offset.map(i32::from);
        (if mirrored { x - ox } else { x + ox }, y + oy)
    }

    fn state(&self, layer: usize, own: u16, pose: &TagPose) -> u16 {
        match self.asset.layers[layer].mask {
            Some(Mask::Layer { slot, .. }) => {
                let mask = self
                    .resolved(usize::from(slot))
                    .and_then(|l| pose.layers.get(l))
                    .map_or(0, |p| p.state);
                mask_state(self.asset, layer as u8, own, mask)
            }
            _ => own,
        }
    }

    fn edge(&self, of: Of, body: Rect) -> [f64; 5] {
        let Of::Layer(target) = of else {
            return [0.0; 5];
        };
        let Some(LayerContent::Body(target)) = self
            .asset
            .layers
            .get(usize::from(target))
            .map(|l| &l.content)
        else {
            return [0.0; 5];
        };
        let rings = target.rings.len() as f64;
        if target.art.is_some() {
            return [0.0, 0.0, 0.0, 0.0, rings];
        }
        let [tl, tr, br, bl] = scaled_corners(target.corners, body).map(f64::from);
        [tl, tr, br, bl, rings]
    }

    fn lanes(
        &self,
        ir: &MaterialIr,
        layer: usize,
        body: Rect,
        exprs: &dyn Fn(u32) -> f64,
    ) -> [f64; LANE_COUNT] {
        let mut lanes = [0.0; LANE_COUNT];
        let edge = self.edge(self.asset.layers[layer].of, body);
        for (lane, value) in ir.lanes.iter().zip(lanes.iter_mut()) {
            *value = match *lane {
                Lane::Param(index) => {
                    let index = usize::from(index);
                    self.variant
                        .params
                        .get(index)
                        .copied()
                        .or_else(|| self.asset.params.get(index).map(|p| p.value))
                        .unwrap_or(0.0)
                }
                Lane::Expr(program) => exprs(program),
                Lane::BodyEdge(which) => {
                    edge[match which {
                        EdgeLane::TopLeft => 0,
                        EdgeLane::TopRight => 1,
                        EdgeLane::BottomRight => 2,
                        EdgeLane::BottomLeft => 3,
                        EdgeLane::Rings => 4,
                    }]
                }
            };
        }
        lanes
    }
}

fn body_of(asset: &NameplateAsset, of: Of) -> Option<&Body> {
    match of {
        Of::Layer(l) => match asset.layers.get(usize::from(l)).map(|l| &l.content) {
            Some(LayerContent::Body(body)) => Some(body),
            _ => None,
        },
        _ => None,
    }
}

fn range_of(offset: i32, count: i32, width: usize, len: usize) -> Option<Range<usize>> {
    let offset = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let start = offset.checked_mul(width)?;
    let end = start.checked_add(count.checked_mul(width)?)?;
    (end <= len).then_some(start..end)
}

fn fingerprint(asset: &NameplateAsset, variant: &Variant, palette: &PaletteFrame) -> u64 {
    let mut hash = Fnv::new();
    let mut add = |c| {
        hash.push(u64::from(resolve_ref(
            c,
            palette,
            &asset.fixed_colors,
            &variant.stops,
        )))
    };
    let text = &variant.text;
    for paint in [&text.color, &text.shadow, &text.outline] {
        if let Paint::Color(c) = paint {
            add(*c);
        }
    }
    for c in text.candidates.iter().chain(&text.backdrop_hint) {
        add(*c);
    }
    let badge = &variant.badge;
    if let Some(c) = badge.tint {
        add(c);
    }
    if let Decoration::Shadow(Some(c)) | Decoration::Outline(Some(c)) = &badge.decoration {
        add(*c);
    }
    for c in &badge.candidates {
        add(*c);
    }
    hash.0
}

#[allow(clippy::too_many_arguments)]
fn sample_backdrop(
    asset: &NameplateAsset,
    variant: &Variant,
    view: &View<'_>,
    fetch: &mut dyn FnMut(u8, u16) -> Arc<LayerImage>,
    counts: &[u8; MAX_SLOTS],
) -> (Vec<u32>, Vec<u32>) {
    let name = view.layout.name;
    let badge = view.layout.badge;
    let mut under_name = Backdrop::new();
    let mut under_badge = Backdrop::new();
    let animations = &asset.animations;
    let looped = i32::from(asset.loop_frames.max(1));
    let mut runs: Vec<(Option<ClipRef>, i32)> = vec![(None, looped)];
    for index in 0..animations.reactions.len() {
        runs.push((Some(ClipRef::Reaction(index as u16)), looped));
    }
    for (index, shot) in animations.one_shots.iter().enumerate() {
        runs.push((
            Some(ClipRef::OneShot(index as u16)),
            i32::from(shot.clip.length.max(1)),
        ));
    }
    for (s, schedule) in animations.schedules.iter().enumerate() {
        for (e, event) in schedule.events.iter().enumerate() {
            let clip = ClipRef::Schedule {
                schedule: s as u16,
                event: e as u16,
            };
            runs.push((Some(clip), i32::from(event.clip.length.max(1))));
        }
    }
    let total: usize = runs.iter().map(|(_, n)| *n as usize).sum();
    let stride = total.div_ceil(BACKDROP_FRAMES).max(1);
    let gates = Gates::default();
    let mut pose = Box::<TagPose>::default();
    let mut shown = Vec::with_capacity(2);
    let sampler = |_: u32, _: i32, channel: Channel| channel.convert(0.0);
    for (clip, length) in runs {
        for frame in (0..length).step_by(stride) {
            shown.clear();
            shown.push(ClipTime {
                clip: ClipRef::Loop,
                frame: frame % looped,
                start: 0,
            });
            if let Some(clip) = clip {
                let local = match clip {
                    ClipRef::Reaction(_) => frame % looped,
                    _ => frame,
                };
                shown.push(ClipTime {
                    clip,
                    frame: local,
                    start: 0,
                });
            }
            pose_layers(asset, variant, &shown, &sampler, &gates, &[], &mut pose);
            animate_palette(
                asset,
                variant,
                &shown,
                &|_, _| 0xff00_0000,
                &gates,
                &mut pose,
            );
            let palette = &pose.palette;
            let mut add = |rect: Rect, argb: u32| {
                if rect.intersect(name) != Rect::EMPTY {
                    under_name.add(argb);
                }
                if let Some(badge) = badge {
                    if rect.intersect(badge) != Rect::EMPTY {
                        under_badge.add(argb);
                    }
                }
            };
            for slot in 0..asset.layers.len().min(MAX_LAYERS) {
                let Some(layer) = view.resolved(slot) else {
                    continue;
                };
                if !view.shown(slot, layer) {
                    continue;
                }
                let lp = pose.layers[layer];
                if !lp.visible {
                    continue;
                }
                let spec = &asset.layers[layer];
                let (dx, dy) = (i32::from(lp.dx), i32::from(lp.dy));
                match &spec.content {
                    LayerContent::Body(_)
                    | LayerContent::Sprite(SpriteLayer {
                        path: None,
                        repeat: None,
                        ..
                    }) => {
                        let image = fetch(layer as u8, view.state(layer, lp.state, &pose));
                        for rect in &image.rects {
                            let color = resolve_ref(
                                rect.color,
                                palette,
                                &asset.fixed_colors,
                                &variant.stops,
                            );
                            let r = Rect::new(
                                rect.x0.into(),
                                rect.y0.into(),
                                rect.x1.into(),
                                rect.y1.into(),
                            );
                            add(r.offset(dx, dy), fade(color, lp.opacity));
                        }
                        for strip in &image.strips {
                            let r = Rect::new(
                                strip.x0.into(),
                                strip.y0.into(),
                                strip.x1.into(),
                                strip.y1.into(),
                            );
                            let count =
                                usize::from(counts[usize::from(strip.slot) % MAX_SLOTS].max(1));
                            let base = usize::from(strip.slot) * MAX_TONES;
                            for tone in palette.tones.iter().skip(base).take(count) {
                                add(r.offset(dx, dy), fade(*tone, lp.opacity));
                            }
                        }
                    }
                    LayerContent::Material(content) => {
                        let Some(ir) = asset.materials.get(usize::from(content.material)) else {
                            continue;
                        };
                        let area = ir.area(view.material_body(layer)).offset(dx, dy);
                        for color in &ir.hint_colors {
                            add(area, fade(*color, lp.opacity));
                        }
                        for c in &ir.tone_hint {
                            let color =
                                resolve_ref(*c, palette, &asset.fixed_colors, &variant.stops);
                            add(area, fade(color, lp.opacity));
                        }
                        if ir.hint_colors.is_empty() && ir.tone_hint.is_empty() {
                            for slot in &ir.tone_slots {
                                let count =
                                    usize::from(counts[usize::from(*slot) % MAX_SLOTS].max(1));
                                let base = usize::from(*slot) * MAX_TONES;
                                for tone in palette.tones.iter().skip(base).take(count) {
                                    add(area, fade(*tone, lp.opacity));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    (under_name.colors(), under_badge.colors())
}

impl NameplateRenderer {
    pub fn new() -> NameplateRenderer {
        NameplateRenderer {
            wearers: WearerTable::new(),
            motion: Motion::Full,
            assets: Vec::new(),
            images: ImageCache::new(DEFAULT_CACHE_BYTES),
            texels: Texels::default(),
            equips: BTreeMap::new(),
            customs: BTreeMap::new(),
            textures: Arc::new(BTreeMap::new()),
            texture_generation: 0,
            queue: VecDeque::new(),
            made: 0,
            work: Work::default(),
            tick: 0,
            swept: f64::NEG_INFINITY,
        }
    }

    pub fn add_asset(&mut self, a: Arc<NameplateAsset>) -> u32 {
        let slot = match self.wearers.add_cosmetic(a.record) {
            Ok(slot) => slot,
            Err(_) => match self.wearers.add_cosmetic(Default::default()) {
                Ok(slot) => slot,
                Err(_) => return 0,
            },
        };
        let failed = self.wearers.layout(slot).map(|l| l.declarations()) != Some(a.record);
        let entry = Entry {
            palettes: a.variants.iter().map(palette_key).collect(),
            waiting: Vec::new(),
            runtime: Runtime::new(&a.molang.interner),
            slots: QuerySlots::with_params(&a.molang.interner, &a.params),
            owners: program_owners(&a),
            speed: speed_layers(&a),
            counts: tone_counts(&a),
            asset: a,
            token: Arc::new(()),
            slot,
            disabled: 0,
            failed,
            reported: false,
            routes: RouteCache::new(),
            layouts: Vec::new(),
            next_layout: 0,
            front: BTreeMap::new(),
            walks: Vec::new(),
            profiles: Vec::new(),
            texts: Vec::new(),
            adjusted: Vec::new(),
            cpu: Vec::new(),
            cpu_bytes: 0,
        };
        self.assets.push(Some(entry));
        self.assets.len() as u32
    }

    pub fn remove_asset(&mut self, id: u32) {
        let Some(index) = (id as usize).checked_sub(1) else {
            return;
        };
        let Some(entry) = self.assets.get_mut(index).and_then(Option::take) else {
            return;
        };
        self.wearers.remove_cosmetic(entry.slot);
        self.images.remove_asset(id);
        self.queue.retain(|job| job.id != id);
        self.customs.retain(|(_, asset), _| *asset != id);
        for memo in &entry.cpu {
            self.texels.release(memo.id);
        }
    }

    pub fn set_motion(&mut self, m: Motion) {
        self.motion = m;
    }

    pub fn motion(&self) -> Motion {
        self.motion
    }

    pub fn set_vanilla_texture(
        &mut self,
        id: &str,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> bool {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(rgba.len()) {
            return false;
        }
        let textures = Arc::make_mut(&mut self.textures);
        if rgba.is_empty() {
            textures.remove(id);
        } else {
            textures.insert(
                id.to_string(),
                Image {
                    width,
                    height,
                    rgba,
                },
            );
        }
        self.texture_generation = self.texture_generation.wrapping_add(1);
        true
    }

    fn entry(&self, id: i32) -> Option<&Entry> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.assets.get(index)?.as_ref()
    }

    pub fn needed_words(&self, tags: &[i32]) -> usize {
        let count = tags.len() / TAG_INTS;
        let mut needed = OUT_HEADER + count * DIR_WORDS;
        for row in tags.chunks_exact(TAG_INTS) {
            let colors = usize::try_from(row[TAG_COLOR_COUNT]).unwrap_or(0);
            let worst = self
                .entry(row[TAG_ASSET])
                .map_or(TAG_HEAD + EVENT_WORDS, |e| e.asset.max_out_words as usize);
            needed += worst.max(TAG_HEAD + EVENT_WORDS) + colors;
        }
        needed
    }

    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        clock: f64,
        wearers: &[u64],
        tags: &[i32],
        signals: &[f32],
        glyphs: &[f32],
        colors: &[i32],
        out: &mut [i32],
    ) -> Result<usize, FrameError> {
        if !tags.len().is_multiple_of(TAG_INTS) {
            return Err(FrameError::TagRows { ints: tags.len() });
        }
        let count = tags.len() / TAG_INTS;
        if wearers.len() != count {
            return Err(FrameError::Wearers {
                tags: count,
                wearers: wearers.len(),
            });
        }
        if signals.len() != count * SIGNAL_FLOATS {
            return Err(FrameError::Signals {
                expected: count * SIGNAL_FLOATS,
                got: signals.len(),
            });
        }
        let fixed = OUT_HEADER + count * DIR_WORDS;
        if out.len() < fixed {
            return Err(FrameError::OutTooSmall {
                needed: self.needed_words(tags),
            });
        }
        self.tick += 1;
        self.texels.begin_frame();
        if clock.is_finite() {
            self.wearers.begin_frame(clock);
        }
        let now = self.wearers.clock();
        if !(now - self.swept < EQUIP_SWEEP_SECONDS && now >= self.swept) {
            self.swept = now;
            self.equips
                .retain(|_, e| now - e.seen < WEARER_IDLE_SECONDS && e.seen <= now);
        }
        if self.tick.is_multiple_of(FRONT_SWEEP_FRAMES) {
            let tick = self.tick;
            for entry in self.assets.iter_mut().flatten() {
                entry
                    .front
                    .retain(|_, (_, used)| *used + FRONT_IDLE_FRAMES >= tick);
            }
        }
        let mut cursor = fixed;
        for i in 0..count {
            let row = &tags[i * TAG_INTS..(i + 1) * TAG_INTS];
            let sig = &signals[i * SIGNAL_FLOATS..(i + 1) * SIGNAL_FLOATS];
            let (status, len) = self.tag(now, wearers[i], row, sig, glyphs, colors, out, cursor);
            let dir = OUT_HEADER + i * DIR_WORDS;
            out[dir + DIR_OFFSET] = cursor as i32;
            out[dir + DIR_LEN] = len as i32;
            out[dir + DIR_STATUS] = status;
            cursor += len;
        }
        out[OUT_TAGS] = count as i32;
        out[OUT_WORDS] = cursor as i32;
        out[OUT_STRIP_LAYOUT] = self.texels.layout as i32;
        Ok(cursor)
    }

    #[allow(clippy::too_many_arguments)]
    fn tag(
        &mut self,
        now: f64,
        wearer: u64,
        row: &[i32],
        sig: &[f32],
        glyphs: &[f32],
        colors: &[i32],
        out: &mut [i32],
        at: usize,
    ) -> (i32, usize) {
        let Some(index) = usize::try_from(row[TAG_ASSET])
            .ok()
            .and_then(|id| id.checked_sub(1))
            .filter(|i| matches!(self.assets.get(*i), Some(Some(_))))
        else {
            return (STATUS_UNKNOWN_ASSET, 0);
        };
        let id = row[TAG_ASSET] as u32;
        if self.assets[index].as_ref().is_some_and(|e| e.failed) {
            return self.disabled(index, out, at);
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            let NameplateRenderer {
                wearers,
                motion,
                assets,
                images,
                texels,
                equips,
                customs,
                textures,
                texture_generation,
                queue,
                work,
                tick,
                ..
            } = self;
            let entry = assets[index].as_mut().expect("checked above");
            let mut ctx = Ctx {
                wearers,
                images,
                texels,
                equips,
                customs,
                textures,
                texture_generation: *texture_generation,
                queue,
                work,
                motion: *motion,
                now,
                tick: *tick,
            };
            draw_tag(
                &mut ctx, entry, id, wearer, row, sig, glyphs, colors, out, at,
            )
        }));
        match result {
            Ok(done) => done,
            Err(_) => {
                if let Some(entry) = self.assets[index].as_mut() {
                    entry.failed = true;
                    entry.reported = false;
                }
                self.disabled(index, out, at)
            }
        }
    }

    fn disabled(&mut self, index: usize, out: &mut [i32], at: usize) -> (i32, usize) {
        let Some(entry) = self.assets[index].as_mut() else {
            return (STATUS_UNKNOWN_ASSET, 0);
        };
        if entry.reported {
            return (STATUS_DISABLED, 0);
        }
        let len = TAG_HEAD + EVENT_WORDS;
        let Some(section) = out.get_mut(at..at + len) else {
            return (STATUS_DISABLED, 0);
        };
        section.fill(0);
        section[HEAD_EVENTS] = 1;
        section[TAG_HEAD + EVENT_CODE] = EVENT_ASSET_DISABLED;
        section[TAG_HEAD + EVENT_LAYER] = EVENT_TARGET_TAG;
        entry.reported = true;
        (STATUS_DISABLED, len)
    }

    pub fn forget_wearer(&mut self, w: u64) -> bool {
        self.equips.remove(&w);
        let customs = self.customs.len();
        self.customs.retain(|(wearer, _), _| *wearer != w);
        let dropped = self.customs.len() != customs;
        self.wearers.forget_wearer(w) || dropped
    }

    pub fn set_custom_colors(&mut self, w: u64, asset: u32, colors: &[(u8, u32)]) -> bool {
        let known = (asset as usize)
            .checked_sub(1)
            .and_then(|i| self.assets.get(i))
            .is_some_and(Option::is_some);
        if !known {
            return false;
        }
        if colors.is_empty() {
            self.customs.remove(&(w, asset));
            return true;
        }
        let colors: Vec<(u8, u32)> = colors
            .iter()
            .map(|(slot, rgb)| (*slot, rgb & 0x00ff_ffff))
            .collect();
        match self.customs.get_mut(&(w, asset)) {
            Some(custom) if custom.colors == colors => {}
            Some(custom) => {
                custom.colors = colors;
                custom.resolved = None;
            }
            None => {
                self.customs.insert(
                    (w, asset),
                    Custom {
                        colors,
                        resolved: None,
                    },
                );
            }
        }
        true
    }

    pub fn clear_custom_colors(&mut self, w: u64, asset: u32) -> bool {
        self.customs.remove(&(w, asset)).is_some()
    }

    pub fn custom_colors(&self, w: u64, asset: u32) -> Option<&[(u8, u32)]> {
        self.customs.get(&(w, asset)).map(|c| c.colors.as_slice())
    }

    pub fn set_signal_override(&mut self, w: u64, s: usize, v: f32) -> bool {
        self.wearers.set_signal_override(w, s, v)
    }

    pub fn fire(&mut self, w: u64, asset: u32, one_shot: u16) -> bool {
        let Some(entry) = (asset as usize)
            .checked_sub(1)
            .and_then(|i| self.assets.get(i))
            .and_then(Option::as_ref)
        else {
            return false;
        };
        if entry.failed || usize::from(one_shot) >= entry.asset.animations.one_shots.len() {
            return false;
        }
        let asset = entry.asset.clone();
        let slot = entry.slot;
        let gates = Gates::new(self.motion, Detail::Full);
        let clock = self.wearers.clock();
        let moment = Moment::of(&asset, w, clock, &gates);
        let Some(mut record) = self.wearers.record(slot, w) else {
            return false;
        };
        fire_one_shot(&asset, &mut record, &moment, one_shot)
    }

    pub fn strip_layout(&self) -> u32 {
        self.texels.layout
    }

    pub fn strip_texels(&self, id: u32) -> &[u32] {
        self.texels.get(id)
    }

    pub fn wearer_count(&self) -> usize {
        self.wearers.wearer_count()
    }

    pub fn wearer_bytes(&self) -> usize {
        self.wearers.wearer_bytes()
    }

    pub fn pending_fallbacks(&self) -> usize {
        self.queue.len()
    }

    pub fn take_fallback(&mut self) -> Option<FallbackJob> {
        self.queue.pop_front()
    }

    pub fn finish_fallback(&mut self, image: FallbackImage) -> bool {
        let Some(entry) = (image.id as usize)
            .checked_sub(1)
            .and_then(|i| self.assets.get_mut(i))
            .and_then(Option::as_mut)
        else {
            return false;
        };
        if !Arc::ptr_eq(&entry.token, &image.token) {
            return false;
        }
        entry.waiting.retain(|(key, _)| *key != image.key);
        if image.bytes > NAMEPLATE_FALLBACK_BYTES || entry.cpu.iter().any(|m| m.key == image.key) {
            return false;
        }
        while !entry.cpu.is_empty()
            && (entry.cpu_bytes + image.bytes > NAMEPLATE_FALLBACK_BYTES
                || entry.cpu.len() >= CPU_ENTRIES)
        {
            let index = lru(&entry.cpu, |m| m.used);
            let old = entry.cpu.swap_remove(index);
            entry.cpu_bytes -= old.bytes;
            self.texels.release(old.id);
        }
        self.made += 1;
        let id = self.texels.put(image.pixels);
        entry.cpu.push(CpuMemo {
            key: image.key,
            stale: image.stale,
            id,
            bytes: image.bytes,
            used: self.tick,
            made: self.made,
        });
        entry.cpu_bytes += image.bytes;
        true
    }

    pub fn run_fallbacks(&mut self, limit: usize) -> usize {
        for _ in 0..limit {
            let Some(job) = self.take_fallback() else {
                break;
            };
            self.finish_fallback(job.run());
        }
        self.queue.len()
    }
}

struct Ctx<'r> {
    wearers: &'r mut WearerTable,
    images: &'r mut ImageCache,
    texels: &'r mut Texels,
    equips: &'r mut BTreeMap<u64, Equip>,
    customs: &'r mut BTreeMap<(u64, u32), Custom>,
    textures: &'r Arc<BTreeMap<String, Image>>,
    texture_generation: u64,
    queue: &'r mut VecDeque<FallbackJob>,
    work: &'r mut Work,
    motion: Motion,
    now: f64,
    tick: u64,
}

fn disable(
    bits: u64,
    disabled: &mut u64,
    events: &mut Vec<(i32, i32)>,
    pose: &mut TagPose,
    variant: &Variant,
    over: bool,
) {
    for bit in 0..36 {
        if bits & (1 << bit) == 0 {
            continue;
        }
        let code = if over {
            EVENT_MOLANG_OVER_BUDGET
        } else {
            EVENT_LAYER_DISABLED
        };
        events.push((code, bit));
        if (bit as usize) < MAX_LAYERS {
            pose.layers[bit as usize].visible = false;
        }
    }
    if bits & BIT_PALETTE != 0 {
        base_frame(variant, &mut pose.palette);
    }
    if bits & BIT_PLATE != 0 {
        pose.plate = (0, 0);
    }
    *disabled |= bits;
}

fn signal_events(value: f32) -> u32 {
    if value.is_finite() && value >= 0.0 {
        value as u32
    } else {
        0
    }
}

fn episode_edge(
    asset: &NameplateAsset,
    clip: ClipRef,
    playing: &[ClipTime],
    tails: &[Tail],
    record: &WearerRecord<'_>,
    moment: &Moment,
    now: i64,
) -> (bool, i64) {
    let phased = |start: i64| now - (moment.absolute - start);
    if let Some(time) = playing.iter().find(|t| t.clip == clip) {
        return (true, phased(time.start));
    }
    match clip {
        ClipRef::Reaction(index) => {
            let Some(reaction) = asset.animations.reactions.get(usize::from(index)) else {
                return (false, now);
            };
            let changed = record
                .signals
                .changed
                .get(usize::from(reaction.signal))
                .copied()
                .unwrap_or(record.now);
            (false, phased(moment.frame_of(record.now, changed)))
        }
        ClipRef::OneShot(index) => {
            let length = asset
                .animations
                .one_shots
                .get(usize::from(index))
                .map_or(0, |o| i64::from(o.clip.length));
            match tails.iter().find(|t| t.time.clip == clip) {
                Some(tail) => (false, phased(tail.time.start) + tail.until.min(length)),
                None => (false, now),
            }
        }
        _ => (false, now),
    }
}

fn profile_index(
    profiles: &mut Vec<ProfileMemo>,
    key: ProfileKey,
    tick: u64,
    build: impl FnOnce(&mut Profile),
) -> usize {
    if let Some(index) = profiles.iter().position(|p| p.key == key) {
        profiles[index].used = tick;
        return index;
    }
    if profiles.len() >= PROFILE_ENTRIES {
        let index = lru(profiles, |p| p.used);
        let memo = &mut profiles[index];
        memo.key = key;
        memo.used = tick;
        build(&mut memo.profile);
        index
    } else {
        if profiles.capacity() == 0 {
            profiles.reserve_exact(PROFILE_ENTRIES);
        }
        let mut profile = Profile::empty();
        build(&mut profile);
        profiles.push(ProfileMemo {
            key,
            profile,
            used: tick,
        });
        profiles.len() - 1
    }
}

fn reaction_profile(
    profiles: &mut Vec<ProfileMemo>,
    tick: u64,
    key: ProfileKey,
    track: &Track,
    step: u16,
    loop_frames: u16,
) -> usize {
    let canonical = ProfileKey {
        origin: key.origin.map(|o| Profile::canonical_origin(o, step)),
        ..key
    };
    let index = profile_index(profiles, canonical, tick, |profile| {
        let clock = match canonical.origin {
            Some(origin) => ClipClock::Local(origin),
            None => ClipClock::Loop,
        };
        profile.reaction_into(track, clock, step, loop_frames);
    });
    if let Some(origin) = key.origin {
        profiles[index].profile.rebase(origin, step);
    }
    index
}

fn walk_index(
    walks: &mut Vec<WalkMemo>,
    key: WalkKey,
    tick: u64,
    build: impl FnOnce() -> (Route, Walk),
) -> usize {
    if let Some(index) = walks.iter().position(|w| w.key == key) {
        walks[index].used = tick;
        return index;
    }
    let (route, walk) = build();
    let memo = WalkMemo {
        key,
        route,
        walk,
        used: tick,
    };
    if walks.len() >= WALK_ENTRIES {
        let index = lru(walks, |w| w.used);
        walks[index] = memo;
        index
    } else {
        walks.push(memo);
        walks.len() - 1
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_tag(
    ctx: &mut Ctx<'_>,
    entry: &mut Entry,
    id: u32,
    wearer: u64,
    row: &[i32],
    sig: &[f32],
    glyphs: &[f32],
    colors: &[i32],
    out: &mut [i32],
    at: usize,
) -> (i32, usize) {
    let flags = row[TAG_FLAGS];
    if flags & !FLAGS_KNOWN != 0 {
        return (STATUS_BAD_ROW, 0);
    }
    let Some(input) = read_block(row) else {
        return (STATUS_BAD_ROW, 0);
    };
    let Some(detail) = u8::try_from(row[TAG_DETAIL])
        .ok()
        .and_then(Detail::from_value)
    else {
        return (STATUS_BAD_ROW, 0);
    };
    let Some(glyph_range) = range_of(
        row[TAG_GLYPH_OFFSET],
        row[TAG_GLYPH_COUNT],
        GLYPH_FLOATS,
        glyphs.len(),
    ) else {
        return (STATUS_BAD_ROW, 0);
    };
    let Some(color_range) = range_of(row[TAG_COLOR_OFFSET], row[TAG_COLOR_COUNT], 1, colors.len())
    else {
        return (STATUS_BAD_ROW, 0);
    };
    let Some(block) = build_block(&input) else {
        return (STATUS_VANILLA, 0);
    };
    let asset = entry.asset.clone();
    let token = entry.token.clone();
    let variant_index = u16::try_from(row[TAG_VARIANT])
        .ok()
        .filter(|v| asset.variant(*v).is_some())
        .unwrap_or(0);
    let Some(variant) = asset.variant(variant_index) else {
        return (STATUS_VANILLA, 0);
    };
    let custom = ctx
        .customs
        .get_mut(&(wearer, id))
        .map(|c| c.variant(&asset, variant_index, variant));
    let palette_id = match &custom {
        Some((_, key)) => *key,
        None => entry
            .palettes
            .get(usize::from(variant_index))
            .copied()
            .unwrap_or(0),
    };
    let custom = custom.map(|(variant, _)| variant);
    let variant = custom.as_deref().unwrap_or(variant);
    let display = input.display;
    let placement = Placement {
        block,
        variant: variant_index,
        breakpoint: asset.breakpoint(block.name_width),
    };
    let li = entry.layout(&placement, ctx.tick);
    let counts = entry
        .counts
        .get(usize::from(variant_index))
        .copied()
        .unwrap_or([0; MAX_SLOTS]);

    let Entry {
        slot,
        runtime,
        slots,
        owners,
        speed,
        disabled,
        routes,
        layouts,
        front,
        walks,
        profiles,
        texts,
        adjusted,
        cpu,
        waiting,
        ..
    } = entry;

    let mut raw = [0.0f32; SIGNAL_COUNT];
    raw.copy_from_slice(&sig[..SIGNAL_COUNT]);
    let Some(mut record) = ctx
        .wearers
        .draw(*slot, wearer, &raw, signal_events(sig[SIGNAL_EVENTS]))
    else {
        return (STATUS_DISABLED, 0);
    };
    *record.detail = detail;

    let mut gates = Gates::new(ctx.motion, detail);
    if flags & FLAG_EFFECTS_OFF != 0 {
        gates.effects = false;
    }
    if *disabled & BIT_PALETTE != 0 {
        gates.palette = false;
    }
    if *disabled & BIT_LETTERS != 0 || flags & FLAG_ANIMATED_NAMES_OFF != 0 {
        gates.letters = false;
    }
    if *disabled & BIT_PLATE != 0 {
        gates.plate = false;
    }
    let cpu_effects = flags & FLAG_EFFECTS_CPU != 0;
    let moment = Moment::of(&asset, wearer, ctx.now, &gates);
    let now_phased = i64::from(moment.tag.engine) + i64::from(moment.tag.offset);

    let identity = (asset.identity, variant.identity);
    let equipped = match ctx.equips.get_mut(&wearer) {
        Some(seen) => {
            let recent = ctx.now - seen.seen < WEARER_IDLE_SECONDS && seen.seen <= ctx.now;
            let changed = recent && (seen.asset, seen.variant) != identity;
            *seen = Equip {
                asset: identity.0,
                variant: identity.1,
                seen: ctx.now,
            };
            changed
        }
        None => {
            ctx.equips.insert(
                wearer,
                Equip {
                    asset: identity.0,
                    variant: identity.1,
                    seen: ctx.now,
                },
            );
            false
        }
    };
    if let Some(word) = record.disabled.first_mut() {
        *word = (*disabled & LAYER_BITS) as u32;
    }

    let Work {
        anim,
        pose: tag_pose,
        delayed,
        delayed_shown,
        items,
        particles,
        bursts,
        spots,
        cover,
        materials,
        events,
        failures,
        speeds,
    } = &mut *ctx.work;
    events.clear();
    failures.clear();
    items.clear();

    let memo = &layouts[li];
    let layout = &memo.result;
    let inputs = &memo.inputs;
    let view = View {
        asset: &asset,
        variant,
        placement: &memo.placement,
        layout,
    };

    let vars = take(&mut record.variables);
    let scope = RefCell::new(runtime.wearer_frame(&mut *vars, moment.tag.seed, ctx.now));
    {
        let mut s = scope.borrow_mut();
        let name = block.name_line_rect();
        let bg = block.background;
        let values = [
            (NameplateQuery::Width, layout.bounds.width()),
            (NameplateQuery::Height, layout.bounds.height()),
            (NameplateQuery::LineCount, i32::from(block.line_count)),
            (NameplateQuery::Frame, i32::from(moment.tag.loop_frame)),
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
            if let Some(q) = slots.slot(query) {
                s.set_query(q, value as f32);
            }
        }
        for signal in 0..SIGNAL_COUNT {
            if let Some(q) = slots.signal(signal) {
                s.set_query(q, record.signals.value(signal));
            }
            if let Some(q) = slots.time(signal) {
                s.set_query(q, record.signals.time(signal, record.now));
            }
        }
    }
    let frame_slot = slots.slot(NameplateQuery::Frame);
    let failures = RefCell::new(failures);
    let params = &variant.params;
    let slots_ref: &QuerySlots = slots;
    let run = |program: u32, frame: i32| -> f64 {
        let Some(entry) = asset.molang.programs.get(program as usize) else {
            return 0.0;
        };
        let mut s = scope.borrow_mut();
        if let Some(q) = frame_slot {
            s.set_query(q, frame as f32);
        }
        s.clear_temps();
        let mut queries = ParamQueries {
            slots: slots_ref,
            values: params,
        };
        let outcome = entry.program.evaluate(&mut s, &mut queries);
        if outcome.failed {
            let mut failed = failures.borrow_mut();
            if !failed.contains(&program) {
                failed.push(program);
            }
            return 0.0;
        }
        if outcome.value.is_finite() {
            outcome.value
        } else {
            0.0
        }
    };
    let values = |program: u32, frame: i32| run(program, frame);
    let colors_of = |program: u32, frame: i32| {
        let v = run(program, frame);
        0xff00_0000 | libm::floor(v.clamp(0.0, f64::from(0x00ff_ffff))) as u32
    };
    let posed = Gates {
        palette: gates.palette && custom.is_none(),
        ..gates
    };
    pose(
        &asset,
        variant_index,
        &mut record,
        &moment,
        equipped,
        &posed,
        &values,
        &colors_of,
        anim,
        tag_pose,
    );
    if custom.is_some() {
        animate_palette(&asset, variant, &anim.shown, &colors_of, &gates, tag_pose);
    }

    let mut fresh = 0u64;
    for program in failures.borrow().iter() {
        fresh |= owners.get(*program as usize).copied().unwrap_or(0);
    }
    fresh &= !*disabled;
    let over_budget = scope.borrow().fuel() == 0;
    if fresh != 0 {
        disable(fresh, disabled, events, tag_pose, variant, over_budget);
    }
    if *disabled & BIT_BADGE != 0 {
        tag_pose.badge = BadgePose::default();
    }

    let mode = if flags & FLAG_ANIMATED_NAMES_OFF != 0 {
        TextMode::Vanilla
    } else {
        variant.text.mode
    };
    let glyph_slice = &glyphs[glyph_range];
    let glyph_count = glyph_slice.len() / GLYPH_FLOATS;
    let mut letter_count = 0usize;
    if mode != TextMode::Vanilla && glyph_count <= MAX_LETTERS && gates.letters {
        if let Some((program, frame)) = letter_program(&asset, variant, &anim.shown, &gates) {
            let mut s = scope.borrow_mut();
            if let Some(q) = frame_slot {
                s.set_query(q, frame as f32);
            }
            let mut queries = ParamQueries {
                slots: slots_ref,
                values: params,
            };
            let ok = run_letters(
                &asset,
                program,
                mode,
                glyph_slice,
                &mut s,
                &mut queries,
                &mut tag_pose.letters[..glyph_count],
            );
            drop(s);
            if ok {
                letter_count = glyph_count;
            } else if *disabled & BIT_LETTERS == 0 {
                let over = scope.borrow().fuel() == 0;
                disable(BIT_LETTERS, disabled, events, tag_pose, variant, over);
            }
        }
    }

    let sneak = row[TAG_SNEAK_ALPHA].clamp(0, 255) as u8;
    let loop_frames = asset.loop_frames;

    if gates.paths {
        for (i, sc) in speed.iter().enumerate() {
            let Some(layer) = sc.primary.map(usize::from) else {
                continue;
            };
            let Some(spec) = asset.layers.get(layer) else {
                continue;
            };
            let LayerContent::Sprite(sprite) = &spec.content else {
                continue;
            };
            let Some(path) = &sprite.path else {
                continue;
            };
            let Some(track) = anim_clip(&asset, sc.clip).and_then(|clip| {
                clip.targets
                    .iter()
                    .find(|t| t.target == Target::Layer(layer as u8))
                    .and_then(|t| t.channels.speed.as_ref())
            }) else {
                continue;
            };
            let range = episode_floats(i);
            if record.effect_floats.get(range.clone()).is_none() {
                continue;
            }
            let (on, edge) = episode_edge(
                &asset,
                sc.clip,
                &anim.playing,
                &anim.tails,
                &record,
                &moment,
                now_phased,
            );
            let floats = &mut record.effect_floats[range];
            match read_episode(floats) {
                Some(previous) => {
                    let step = spec.step;
                    let (width, height) = view.sprite_size(layer, sprite);
                    let target = view.target(layer);
                    let size = [width, height].map(|v| u16::try_from(v.max(0)).unwrap_or(u16::MAX));
                    let key = WalkKey {
                        layer: layer as u8,
                        target,
                        block,
                        size,
                    };
                    let body = view.body_of(spec.of);
                    let wi = walk_index(walks, key, ctx.tick, || {
                        let route = route_for(path, target, &block, body, size);
                        let walk = Walk::new(path, &route, path_cycle(loop_frames, step));
                        (route, walk)
                    });
                    let origin = matches!(sc.clip, ClipRef::OneShot(_)).then_some(previous.start);
                    let key = ProfileKey {
                        clip: i,
                        layer: layer as u8,
                        origin,
                    };
                    let pi = reaction_profile(
                        profiles,
                        ctx.tick,
                        key,
                        track,
                        step,
                        loop_frames,
                    );
                    let walk = &walks[wi].walk;
                    let profile = &profiles[pi].profile;
                    advance(floats, on, edge, now_phased, |e| {
                        rejoin_frame(walk, profile, e, step)
                    });
                }
                None if on => advance(floats, on, edge, now_phased, |_| None),
                None => {}
            }
        }
    }

    for slot_index in 0..asset.layers.len().min(MAX_LAYERS) {
        let Some(layer) = view.resolved(slot_index) else {
            continue;
        };
        if !view.shown(slot_index, layer) {
            continue;
        }
        let lp = tag_pose.layers[layer];
        if !lp.visible {
            continue;
        }
        let spec = &asset.layers[layer];
        let base = Item {
            slot: slot_index as u8,
            layer: layer as u8,
            band: spec.depth.band(),
            source: Source::Material {
                clock: [0; 4],
                lanes: [0.0; LANE_COUNT],
            },
            dx: i32::from(lp.dx),
            dy: i32::from(lp.dy),
            mirror: None,
            opacity: lp.opacity,
        };
        let mut get = |state: u16| {
            fetch(
                front,
                ctx.images,
                ctx.tick,
                id,
                &asset,
                &memo.placement,
                layout,
                inputs,
                layer as u8,
                state,
            )
        };
        match &spec.content {
            LayerContent::Body(_) => {
                let image = get(view.state(layer, lp.state, tag_pose));
                items.push(Item {
                    source: Source::Image(image),
                    ..base
                });
            }
            LayerContent::Sprite(sprite) => {
                let (width, height) = view.sprite_size(layer, sprite);
                let target = view.target(layer);
                let size = [width, height].map(|v| u16::try_from(v.max(0)).unwrap_or(u16::MAX));
                let [ox, oy] = sprite.offset.map(i32::from);
                let (rx, ry) = view.spot(routes, layer, sprite, false);
                let place = |image: Arc<LayerImage>, x: i32, y: i32, flip: bool| Item {
                    source: Source::Image(image),
                    dx: base.dx + x - rx,
                    dy: base.dy + y - ry,
                    mirror: flip.then_some(2 * rx + width - 1),
                    ..base.clone()
                };
                if let Some(path) = &sprite.path {
                    let key = WalkKey {
                        layer: layer as u8,
                        target,
                        block,
                        size,
                    };
                    let body = view.body_of(spec.of);
                    let wi = walk_index(walks, key, ctx.tick, || {
                        let route = route_for(path, target, &block, body, size);
                        let walk = Walk::new(path, &route, path_cycle(loop_frames, spec.step));
                        (route, walk)
                    });
                    let step = spec.step;
                    speeds.clear();
                    if gates.paths {
                        for (i, sc) in speed.iter().enumerate() {
                            let Some(clip) = anim_clip(&asset, sc.clip) else {
                                continue;
                            };
                            let Some(track) = clip
                                .targets
                                .iter()
                                .find(|t| t.target == Target::Layer(layer as u8))
                                .and_then(|t| t.channels.speed.as_ref())
                            else {
                                continue;
                            };
                            let range = episode_floats(i);
                            if record.effect_floats.get(range.clone()).is_none() {
                                continue;
                            }
                            let one_shot = matches!(sc.clip, ClipRef::OneShot(_));
                            let Some(current) = read_episode(&record.effect_floats[range]) else {
                                continue;
                            };
                            let origin = one_shot.then_some(current.start);
                            let key = ProfileKey {
                                clip: i,
                                layer: layer as u8,
                                origin,
                            };
                            let pi = reaction_profile(
                                profiles,
                                ctx.tick,
                                key,
                                track,
                                step,
                                loop_frames,
                            );
                            speeds.push((i, pi));
                        }
                    }
                    let memo = &walks[wi];
                    let floats: &[f32] = &*record.effect_floats;
                    let profiles: &[ProfileMemo] = profiles;
                    let speeds: &[(usize, usize)] = speeds;
                    let shift = |t: i64| -> i64 {
                        speeds
                            .iter()
                            .map(|&(i, pi)| {
                                floats
                                    .get(episode_floats(i))
                                    .and_then(read_episode)
                                    .map_or(0, |e| {
                                        speed_offset(&memo.walk, &profiles[pi].profile, e, step, t)
                                    })
                            })
                            .sum()
                    };
                    let (t, walkers) = walk_gate(path, gates.paths, now_phased, step);
                    let (px, py) = match path.follow {
                        Some(_) => (0, 0),
                        None => pivot_offset(sprite.pivot, width, height),
                    };
                    for index in 0..walkers {
                        let pose_at = walker_pose(&memo.walk, &memo.route, path, index, t, &shift);
                        let own = pose_at.facing.unwrap_or(lp.state);
                        let image = get(view.state(layer, own, tag_pose));
                        items.push(place(
                            image,
                            pose_at.x - px + ox,
                            pose_at.y - py + oy,
                            pose_at.flip,
                        ));
                    }
                } else if let Some(repeat) = &sprite.repeat {
                    let key = repeat_route_key(
                        layer as u8,
                        repeat,
                        target,
                        sprite.follow,
                        view.body_of(spec.of),
                        size,
                    );
                    let route = key.map(|key| routes.route(key));
                    positions(repeat, target, width, height, sprite.pivot, route, spots);
                    let image = get(view.state(layer, lp.state, tag_pose));
                    for (x, y) in spots.iter() {
                        items.push(place(image.clone(), x + ox, y + oy, false));
                    }
                } else {
                    let image = get(view.state(layer, lp.state, tag_pose));
                    items.push(place(image, rx, ry, false));
                    if let Some(mirror) = spec.mirror {
                        let copy = if mirror.delay > 0 {
                            delayed_shown.clear();
                            let delay = i32::from(mirror.delay);
                            for time in &anim.shown {
                                let frame = match time.clip {
                                    ClipRef::Loop | ClipRef::Reaction(_) => (time.frame - delay)
                                        .rem_euclid(i32::from(loop_frames.max(1))),
                                    _ if time.frame < delay => continue,
                                    _ => time.frame - delay,
                                };
                                delayed_shown.push(ClipTime { frame, ..*time });
                            }
                            let sampler = |program: u32, frame: i32, channel: Channel| {
                                channel.convert(values(program, frame))
                            };
                            pose_layers(
                                &asset,
                                variant,
                                delayed_shown,
                                &sampler,
                                &gates,
                                &*record.disabled,
                                delayed,
                            );
                            delayed.layers[layer]
                        } else {
                            lp
                        };
                        if copy.visible {
                            let (x, y) = view.spot(routes, layer, sprite, true);
                            let image = get(view.state(layer, copy.state, tag_pose));
                            items.push(Item {
                                source: Source::Image(image),
                                dx: x - rx - i32::from(copy.dx),
                                dy: y - ry + i32::from(copy.dy),
                                mirror: Some(2 * rx + width - 1),
                                opacity: copy.opacity,
                                ..base.clone()
                            });
                        }
                    }
                }
            }
            LayerContent::Emitter(emitter) => {
                if !gates.particles {
                    continue;
                }
                let lid = layer as u8;
                let rect = view.target(layer);
                let body = view.body_of(spec.of);
                let longest = emitter.lifetime[1];
                bursts.clear();
                for time in anim.shown.iter() {
                    let Some(clip) = anim_clip(&asset, time.clip) else {
                        continue;
                    };
                    for target in clip
                        .targets
                        .iter()
                        .filter(|t| t.target == Target::Layer(lid))
                    {
                        let list = &target.channels.bursts;
                        match time.clip {
                            ClipRef::Loop => looped_starts(
                                list,
                                loop_frames,
                                now_phased,
                                longest,
                                (i64::MIN, None),
                                bursts,
                            ),
                            ClipRef::Reaction(_) => {
                                let from = now_phased - (moment.absolute - time.start);
                                looped_starts(
                                    list,
                                    loop_frames,
                                    now_phased,
                                    longest,
                                    (from, None),
                                    bursts,
                                )
                            }
                            ClipRef::Schedule { .. } | ClipRef::OneShot(_) => run_starts(
                                list,
                                now_phased - i64::from(time.frame),
                                now_phased,
                                longest,
                                bursts,
                            ),
                        }
                    }
                }
                if !gates.first_frame {
                    for tail in anim.tails.iter() {
                        let Some(clip) = anim_clip(&asset, tail.time.clip) else {
                            continue;
                        };
                        if let ClipRef::Reaction(index) = tail.time.clip {
                            let dropped = gates.hold_reactions
                                && asset
                                    .animations
                                    .reactions
                                    .get(usize::from(index))
                                    .is_some_and(|r| r.reduced == Reduced::Drop);
                            if dropped {
                                continue;
                            }
                        }
                        for target in clip
                            .targets
                            .iter()
                            .filter(|t| t.target == Target::Layer(lid))
                        {
                            let list = &target.channels.bursts;
                            match tail.time.clip {
                                ClipRef::Reaction(_) => {
                                    let from = now_phased - (moment.absolute - tail.time.start);
                                    looped_starts(
                                        list,
                                        loop_frames,
                                        now_phased,
                                        longest,
                                        (from, Some(from.saturating_add(tail.until))),
                                        bursts,
                                    )
                                }
                                _ => run_starts_until(
                                    list,
                                    now_phased - i64::from(tail.time.frame),
                                    now_phased,
                                    longest,
                                    tail.until,
                                    bursts,
                                ),
                            }
                        }
                    }
                }
                bursts.sort_unstable_by_key(|b| (b.start, b.count));
                let key = particle_route_key(lid, emitter, &asset.sprites, rect, body);
                let route = key.map(|key| routes.route(key));
                let area = SpawnArea::new(&emitter.spawn, rect, &block, body, route);
                particles.clear();
                emit_in(
                    emitter,
                    &asset.sprites,
                    lid,
                    &area,
                    moment.tag.seed,
                    now_phased,
                    bursts,
                    particles,
                );
                for particle in particles.iter().filter(|p| p.layer == lid) {
                    let image = get(particle.state);
                    items.push(Item {
                        source: Source::Image(image),
                        dx: base.dx + i32::from(particle.x),
                        dy: base.dy + i32::from(particle.y),
                        opacity: multiply(lp.opacity, particle.opacity),
                        ..base.clone()
                    });
                }
            }
            LayerContent::Material(content) => {
                if !gates.effects {
                    continue;
                }
                let Some(ir) = asset.materials.get(usize::from(content.material)) else {
                    continue;
                };
                let body = view.material_body(layer);
                let delays = once(0).chain(spec.mirror.map(|m| i64::from(m.delay)));
                for (copy, delay) in delays.enumerate() {
                    let (at, clock) = material_time(&asset, layer, &moment.tag, delay);
                    let lanes = view.lanes(ir, layer, body, &|program| values(program, at));
                    let source = Source::Material { clock, lanes };
                    if copy == 0 {
                        items.push(Item {
                            source,
                            ..base.clone()
                        });
                    } else {
                        items.push(Item {
                            source,
                            dx: -base.dx,
                            mirror: Some(body.x0 + body.x1 - 1),
                            ..base.clone()
                        });
                    }
                }
            }
        }
    }

    let mut late = 0u64;
    for program in failures.borrow().iter() {
        late |= owners.get(*program as usize).copied().unwrap_or(0);
    }
    late &= !*disabled;
    if late != 0 {
        let over = scope.borrow().fuel() == 0;
        disable(late, disabled, events, tag_pose, variant, over);
        items.retain(|item| late & (1u64 << item.layer) == 0);
    }

    let plate = if display {
        (0, 0)
    } else {
        (i32::from(tag_pose.plate.0), i32::from(tag_pose.plate.1))
    };
    let mut writer = Writer {
        buf: out,
        at: at + TAG_HEAD,
        overflow: false,
    };
    materials.clear();
    let mut op_count = 0i32;
    let mut material_count = 0i32;
    for band in 0..4u8 {
        for item in items.iter().filter(|i| i.band == band) {
            let layer = usize::from(item.layer);
            let spec = &asset.layers[layer];
            let opacity = multiply(item.opacity, sneak);
            match &item.source {
                Source::Image(image) => {
                    let clip = match spec.mask {
                        Some(Mask::Name) => Some(layout.name),
                        _ => None,
                    };
                    for rect in &image.rects {
                        let color = resolve_ref(
                            rect.color,
                            &tag_pose.palette,
                            &asset.fixed_colors,
                            &variant.stops,
                        );
                        let argb = fade(color, opacity);
                        if argb >> 24 == 0 {
                            continue;
                        }
                        let src = Rect::new(
                            rect.x0.into(),
                            rect.y0.into(),
                            rect.x1.into(),
                            rect.y1.into(),
                        );
                        let Some((_, placed)) = item.clipped(src, clip) else {
                            continue;
                        };
                        writer.op(
                            placed.offset(plate.0, plate.1),
                            argb,
                            band,
                            item.slot,
                            OP_KIND_RECT,
                            0,
                            0,
                        );
                        op_count += 1;
                    }
                    for strip in &image.strips {
                        let count = usize::from(counts[usize::from(strip.slot) % MAX_SLOTS].max(1));
                        let start = usize::from(strip.slot) * MAX_TONES;
                        let Some(tones) = tag_pose.palette.tones.get(start..start + count) else {
                            continue;
                        };
                        let texel = ctx.texels.strip(tones);
                        let src = Rect::new(
                            strip.x0.into(),
                            strip.y0.into(),
                            strip.x1.into(),
                            strip.y1.into(),
                        );
                        let Some((clipped, placed)) = item.clipped(src, clip) else {
                            continue;
                        };
                        let shift = match strip.axis {
                            Axis::X => clipped.x0 - src.x0,
                            Axis::Y => clipped.y0 - src.y0,
                        };
                        let phase = i32::from(strip.phase) + shift;
                        let mut aux = phase & AUX_LOW;
                        if strip.axis == Axis::Y {
                            aux |= AUX_AXIS_Y;
                        }
                        if item.mirror.is_some() {
                            aux |= AUX_MIRROR;
                        }
                        let argb = (u32::from(opacity) << 24) | 0x00ff_ffff;
                        writer.op(
                            placed.offset(plate.0, plate.1),
                            argb,
                            band,
                            item.slot,
                            OP_KIND_STRIP,
                            texel as i32,
                            aux,
                        );
                        op_count += 1;
                    }
                }
                Source::Material { clock, lanes } => {
                    let LayerContent::Material(content) = &spec.content else {
                        continue;
                    };
                    let Some(ir) = asset.materials.get(usize::from(content.material)) else {
                        continue;
                    };
                    let body = view.material_body(layer);
                    let area = ir.area(body);
                    let frame = MaterialFrame {
                        area,
                        body,
                        clock: *clock,
                        lanes: *lanes,
                        tones: &tag_pose.palette.tones,
                        atlas: [[0.0; 4]; MAX_SAMPLES],
                    };
                    cover.clear();
                    let mut covered = false;
                    match spec.mask {
                        Some(Mask::Name) => {
                            cover.push(layout.name);
                            covered = true;
                        }
                        Some(Mask::Layer { slot: target, fill }) => {
                            covered = true;
                            let limit = if fill {
                                view.resolved(usize::from(target))
                                    .and_then(|l| layout.bodies.get(l).copied().flatten())
                            } else {
                                None
                            };
                            if !fill || limit.is_some() {
                                for other in items.iter().filter(|i| i.slot == target) {
                                    let Source::Image(image) = &other.source else {
                                        continue;
                                    };
                                    let sources =
                                        image.rects.iter().map(|r| (r.x0, r.y0, r.x1, r.y1)).chain(
                                            image.strips.iter().map(|s| (s.x0, s.y0, s.x1, s.y1)),
                                        );
                                    for (x0, y0, x1, y1) in sources {
                                        let mut r =
                                            Rect::new(x0.into(), y0.into(), x1.into(), y1.into());
                                        if let Some(limit) = limit {
                                            r = r.intersect(limit);
                                        }
                                        if !r.is_empty() {
                                            cover.push(other.place_rect(r));
                                        }
                                    }
                                }
                            }
                        }
                        None => {}
                    }
                    if covered && cover.is_empty() {
                        continue;
                    }
                    let placed = item.place_rect(area);
                    if cpu_effects {
                        let mut stale = Fnv::new();
                        stale.push(u64::from(content.material));
                        stale.push(item.layer as u64);
                        stale.push(u64::from(item.mirror.is_some()));
                        if ir.textures.iter().any(|t| !matches!(t, TexRef::Bundle(_))) {
                            stale.push(ctx.texture_generation);
                        }
                        stale.rect(area);
                        stale.rect(body);
                        stale.rect(placed);
                        stale.push(u64::from(covered));
                        stale.push(cover.len() as u64);
                        for r in cover.iter() {
                            stale.rect(*r);
                        }
                        for (lane, v) in ir.lanes.iter().zip(&frame.lanes) {
                            if !matches!(lane, Lane::Expr(_)) {
                                stale.push(v.to_bits());
                            }
                        }
                        for slot in &ir.tone_slots {
                            let start = usize::from(*slot) * MAX_TONES;
                            for tone in tag_pose.palette.tones.iter().skip(start).take(MAX_TONES) {
                                stale.push(u64::from(*tone));
                            }
                        }
                        let stale = stale.0;
                        let mut key = Fnv(stale);
                        match ir.period {
                            Period::Still => {}
                            Period::Frames(period) if period > 0 => {
                                let state = i64::from(frame.clock[0]).rem_euclid(i64::from(period));
                                key.push(state as u64);
                                for v in &frame.clock[1..] {
                                    key.push(*v as u32 as u64);
                                }
                            }
                            _ => {
                                for v in frame.clock {
                                    key.push(v as u32 as u64);
                                }
                            }
                        }
                        for (lane, v) in ir.lanes.iter().zip(&frame.lanes) {
                            if matches!(lane, Lane::Expr(_)) {
                                key.push(v.to_bits());
                            }
                        }
                        let key = key.0;
                        let found = cpu.iter_mut().find(|m| m.key == key).map(|m| {
                            m.used = ctx.tick;
                            m.id
                        });
                        let texel = match found {
                            Some(texel) => Some(texel),
                            None => {
                                let bytes = fallback_bytes(area);
                                let tick = ctx.tick;
                                waiting.retain(|(_, since)| since + FALLBACK_WAIT_FRAMES >= tick);
                                if bytes <= NAMEPLATE_FALLBACK_BYTES
                                    && !waiting.iter().any(|(k, _)| *k == key)
                                {
                                    let job = FallbackJob {
                                        asset: asset.clone(),
                                        token: token.clone(),
                                        id,
                                        key,
                                        stale,
                                        material: usize::from(content.material),
                                        area,
                                        body,
                                        clock: frame.clock,
                                        lanes: frame.lanes,
                                        tones: tag_pose.palette.tones.to_vec(),
                                        cover: covered.then(|| cover.clone()),
                                        mirror: item.mirror,
                                        shift: (item.dx, item.dy),
                                        textures: ctx.textures.clone(),
                                        bytes,
                                    };
                                    let queued = ctx
                                        .queue
                                        .iter()
                                        .position(|j| j.id == id && j.stale == stale);
                                    match queued {
                                        Some(at) => {
                                            let old = replace(&mut ctx.queue[at], job);
                                            waiting.retain(|(k, _)| *k != old.key);
                                            waiting.push((key, tick));
                                        }
                                        None if ctx.queue.len() < QUEUED_FALLBACKS => {
                                            ctx.queue.push_back(job);
                                            waiting.push((key, tick));
                                        }
                                        None => {}
                                    }
                                }
                                cpu.iter_mut()
                                    .filter(|m| m.stale == stale)
                                    .max_by_key(|m| m.made)
                                    .map(|m| {
                                        m.used = tick;
                                        m.id
                                    })
                            }
                        };
                        let Some(texel) = texel else {
                            continue;
                        };
                        let mut aux = area.width() & AUX_LOW;
                        if item.mirror.is_some() {
                            aux |= AUX_MIRROR;
                        }
                        if ir.blend == Blend::Additive {
                            aux |= AUX_ADDITIVE;
                        }
                        let argb = (u32::from(opacity) << 24) | 0x00ff_ffff;
                        writer.op(
                            placed.offset(plate.0, plate.1),
                            argb,
                            band,
                            item.slot,
                            OP_KIND_IMAGE,
                            texel as i32,
                            aux,
                        );
                        op_count += 1;
                    } else {
                        if cover.len() > MAX_CLIP_RECTS {
                            let all = cover.iter().fold(Rect::EMPTY, |a, r| a.union(*r));
                            cover.clear();
                            cover.push(all);
                        }
                        let clips: &[Rect] = cover;
                        let uniform_words = ir.uniform_words() as usize;
                        materials.extend_from_slice(&[
                            i32::from(content.material),
                            pack_material(
                                i32::from(band),
                                i32::from(item.slot),
                                item.mirror.is_some(),
                                opacity,
                            ),
                            op_count,
                            placed.x0 + plate.0,
                            placed.y0 + plate.1,
                            placed.x1 + plate.0,
                            placed.y1 + plate.1,
                            clips.len() as i32,
                            uniform_words as i32,
                        ]);
                        for r in clips {
                            let r = r.offset(plate.0, plate.1);
                            materials.extend_from_slice(&[r.x0, r.y0, r.x1, r.y1]);
                        }
                        let start = materials.len();
                        materials.resize(start + uniform_words, 0);
                        write_uniforms(ir, &frame, &mut materials[start..]);
                        material_count += 1;
                    }
                }
            }
        }
    }
    writer.extend(materials);

    let memo_key = layouts[li].key;
    let found = layouts[li]
        .backdrops
        .iter()
        .position(|b| b.palette == palette_id);
    let bi = match found {
        Some(bi) => bi,
        None => {
            let memo = &layouts[li];
            let view = View {
                asset: &asset,
                variant,
                placement: &memo.placement,
                layout: &memo.result,
            };
            let mut get = |layer: u8, state: u16| {
                fetch(
                    front,
                    ctx.images,
                    ctx.tick,
                    id,
                    &asset,
                    &memo.placement,
                    &memo.result,
                    &memo.inputs,
                    layer,
                    state,
                )
            };
            let (under_name, under_badge) =
                sample_backdrop(&asset, variant, &view, &mut get, &counts);
            let sampled = BackdropMemo {
                palette: palette_id,
                under_name,
                under_badge,
                used: ctx.tick,
            };
            let backdrops = &mut layouts[li].backdrops;
            if backdrops.len() >= BACKDROP_ENTRIES {
                let index = lru(backdrops, |b| b.used);
                backdrops[index] = sampled;
                index
            } else {
                backdrops.push(sampled);
                backdrops.len() - 1
            }
        }
    };
    let backdrop = &mut layouts[li].backdrops[bi];
    backdrop.used = ctx.tick;
    let (under_name, under_badge) = (&backdrop.under_name, &backdrop.under_badge);
    let external = (flags & FLAG_EXTERNAL_COLOR != 0).then_some(row[TAG_EXTERNAL_COLOR] as u32);
    let key = TextKey {
        layout: memo_key,
        palette: palette_id,
        external,
        fingerprint: fingerprint(&asset, variant, &tag_pose.palette),
    };
    let (text, badge) = match texts.iter_mut().find(|t| t.key == key) {
        Some(found) => {
            found.used = ctx.tick;
            (found.text, found.badge)
        }
        None => {
            let text = resolve_text(
                &variant.text,
                &tag_pose.palette,
                &asset.fixed_colors,
                under_name,
                external,
            );
            let badge = resolve_badge(
                &variant.badge,
                &tag_pose.palette,
                &asset.fixed_colors,
                under_badge,
            );
            let memo = TextMemo {
                key,
                text,
                badge,
                used: ctx.tick,
            };
            if texts.len() >= TEXT_ENTRIES {
                let index = lru(texts, |t| t.used);
                texts[index] = memo;
            } else {
                texts.push(memo);
            }
            (text, badge)
        }
    };
    writer.extend(&[
        text.color as i32,
        text.shadow as i32,
        text.shadow_use.value(),
        text.outline as i32,
        letter_count as i32,
    ]);
    for letter in &tag_pose.letters[..letter_count] {
        writer.extend(&[
            i32::from(letter.dx),
            i32::from(letter.dy),
            letter.color as i32,
            letter.shadow as i32,
            i32::from(letter.opacity),
            pack_scale_rotation(letter.scale_q, letter.rotation),
            letter.outline as i32,
        ]);
    }
    for color in &colors[color_range] {
        let color = *color as u32;
        let adjusted_color = match variant.text.formatting {
            Formatting::Keep => color,
            Formatting::Override if text.color != 0 => {
                (color & 0xff00_0000) | (text.color & 0x00ff_ffff)
            }
            Formatting::Override => color,
            Formatting::Adjust => match adjusted
                .iter_mut()
                .find(|a| a.layout == memo_key && a.palette == palette_id && a.color == color)
            {
                Some(found) => {
                    found.used = ctx.tick;
                    found.adjusted
                }
                None => {
                    let value = adjust(color, under_name);
                    let memo = AdjustMemo {
                        layout: memo_key,
                        palette: palette_id,
                        color,
                        adjusted: value,
                        used: ctx.tick,
                    };
                    if adjusted.len() >= ADJUST_ENTRIES {
                        let index = lru(adjusted, |a| a.used);
                        adjusted[index] = memo;
                    } else {
                        adjusted.push(memo);
                    }
                    value
                }
            },
        };
        writer.push(adjusted_color as i32);
    }

    let badge_pose = tag_pose.badge;
    match layouts[li].result.badge {
        Some(rect) if badge_pose.visible => writer.extend(&[
            rect.x0 + i32::from(badge_pose.dx) + plate.0,
            rect.y0 + i32::from(badge_pose.dy) + plate.1,
            i32::from(multiply(badge_pose.opacity, sneak)),
            badge.tint as i32,
            pack_decoration(badge.decoration, badge.decoration_rgb),
        ]),
        _ => writer.extend(&[
            0,
            0,
            0,
            badge.tint as i32,
            pack_decoration(badge.decoration, badge.decoration_rgb),
        ]),
    }
    for (code, target) in events.iter() {
        writer.extend(&[*code, *target]);
    }

    let material_words = materials.len() as i32;
    let head = [
        i32::from(tag_pose.plate.0),
        i32::from(tag_pose.plate.1),
        i32::from(variant.owns_background),
        mode.value(),
        op_count,
        material_count,
        material_words,
        events.len() as i32,
    ];
    let end = writer.at;
    if writer.overflow {
        return (STATUS_NO_ROOM, 0);
    }
    out[at..at + TAG_HEAD].copy_from_slice(&head);
    (STATUS_OK, end - at)
}

const MAX_CLIP_RECTS: usize = MAX_CLIPS as usize;

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use serde_json::{json, Value as Json};

    use super::*;
    use crate::container::{open_files, Opened, Options as ContainerOptions, CAPABILITIES};
    use crate::issues::Mode;
    use crate::limits::NAMEPLATE_CAPS;
    use crate::molang::compile as compile_molang;
    use crate::nameplate::anim::SEED_PURPOSE;
    use crate::nameplate::format::compile;
    use crate::nameplate::geom::{BlockInput, CANVAS};
    use crate::nameplate::queries::MolangUse;
    use crate::nameplate::raster::{canvas_with, Canvas, RasterOptions, RasterTextures};
    use crate::nameplate::types::{Ease, Key, MolangProgram};
    use crate::nameplate::LoadOptions;
    use crate::signals::signal_id;
    use crate::test_alloc::allocations_in;
    use crate::wearer::seed;

    pub(crate) type Files = Vec<(String, Vec<u8>)>;

    pub(crate) fn kit(name: &str) -> Files {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/nameplate/format/testdata").join(name);
        let entries = |dir: &Path| std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect::<Vec<_>>();
        let paths = entries(&root).into_iter().flat_map(|p| if p.is_dir() { entries(&p) } else { vec![p] });
        let mut files: Files = paths.map(|p| (p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap())).collect();
        files.sort();
        files
    }

    pub(crate) fn patch(files: &mut Files, name: &str, edit: impl FnOnce(&mut Json)) {
        let bytes = &mut files.iter_mut().find(|(n, _)| n == name).unwrap().1;
        let mut json: Json = serde_json::from_slice(bytes).unwrap();
        edit(&mut json);
        *bytes = serde_json::to_vec(&json).unwrap();
    }

    pub(crate) fn compile_files(files: &Files, identity: Option<u64>) -> NameplateAsset {
        let options = ContainerOptions { mode: Mode::Client, legacy: NAMEPLATE_CAPS, capabilities: CAPABILITIES };
        let Opened::Bundle(mut bundle) = open_files(files, &options) else {
            panic!("a bundle");
        };
        let asset = compile(&mut bundle, &LoadOptions { identity, ..LoadOptions::default() });
        let issues = bundle.finish();
        asset.unwrap_or_else(|| panic!("compiles: {issues:?}"))
    }

    fn load(name: &str) -> Arc<NameplateAsset> {
        Arc::new(compile_files(&kit(name), None))
    }

    fn customizable() -> Arc<NameplateAsset> {
        let mut files = kit("test_plate");
        patch(&mut files, "palettes.json", |json| {
            for slot in json["slots"].as_array_mut().unwrap() {
                slot["customizable"] = json!(true);
            }
        });
        Arc::new(compile_files(&files, None))
    }

    fn row(asset: u32, width: i32) -> [i32; TAG_INTS] {
        let mut row = [0; TAG_INTS];
        row[TAG_ASSET] = asset as i32;
        row[TAG_SNEAK_ALPHA] = 255;
        let background = Rect::vanilla_background(width, 8);
        let block = BlockInput { line_widths: [width, 0, 0], line_count: 1, name_width: width, background, ..BlockInput::default() };
        assert!(write_block(&block, &mut row));
        row
    }

    pub(crate) struct Frame {
        out: Vec<i32>,
        words: usize,
    }

    impl Frame {
        fn status(&self, tag: usize) -> i32 {
            self.out[OUT_HEADER + tag * DIR_WORDS + DIR_STATUS]
        }

        pub(crate) fn section(&self, tag: usize) -> &[i32] {
            let dir = OUT_HEADER + tag * DIR_WORDS;
            let at = self.out[dir + DIR_OFFSET] as usize;
            &self.out[at..at + self.out[dir + DIR_LEN] as usize]
        }

        fn ops(&self, tag: usize) -> Vec<[i32; OP_WORDS]> {
            let section = self.section(tag);
            let ops = &section[TAG_HEAD..TAG_HEAD + section[HEAD_OPS] as usize * OP_WORDS];
            ops.chunks_exact(OP_WORDS).map(|op| op.try_into().unwrap()).collect()
        }

        fn events(&self, tag: usize) -> Vec<(i32, i32)> {
            let section = self.section(tag);
            let start = section.len() - section[HEAD_EVENTS] as usize * EVENT_WORDS;
            section[start..].chunks_exact(EVENT_WORDS).map(|e| (e[EVENT_CODE], e[EVENT_LAYER])).collect()
        }

        fn count(&self, kind: i32) -> usize {
            self.ops(0).iter().filter(|op| unpack_band_layer_kind(op[OP_BAND_LAYER_KIND]).2 == kind).count()
        }

        fn layers(&self) -> Vec<i32> {
            self.ops(0).iter().map(|op| unpack_band_layer_kind(op[OP_BAND_LAYER_KIND]).1).collect()
        }
    }

    pub(crate) fn draw_with(r: &mut NameplateRenderer, clock: f64, wearers: &[u64], tags: &[[i32; TAG_INTS]], signals: &[f32], glyphs: &[f32]) -> Frame {
        let flat: Vec<i32> = tags.iter().flatten().copied().collect();
        let mut out = vec![0; r.needed_words(&flat)];
        let words = r.frame(clock, wearers, &flat, signals, glyphs, &[], &mut out).expect("frame");
        Frame { out, words }
    }

    fn draw(r: &mut NameplateRenderer, clock: f64, wearers: &[u64], tags: &[[i32; TAG_INTS]]) -> Frame {
        draw_with(r, clock, wearers, tags, &vec![0.0f32; tags.len() * SIGNAL_FLOATS], &[])
    }

    pub(crate) fn clock_of(frame: u32, fps: u8) -> f64 {
        (f64::from(frame) + 0.5) / f64::from(fps)
    }

    pub(crate) fn paint(r: &NameplateRenderer, frame: &Frame, tag: usize) -> Canvas {
        let mut canvas = Canvas::new(CANVAS);
        for op in frame.ops(tag) {
            let (x0, y0, x1, y1) = (op[OP_X0], op[OP_Y0], op[OP_X1], op[OP_Y1]);
            let (argb, aux) = (op[OP_ARGB] as u32, op[OP_AUX1]);
            let kind = unpack_band_layer_kind(op[OP_BAND_LAYER_KIND]).2;
            let mirror = aux & AUX_MIRROR != 0;
            for (y, x) in (y0..y1).flat_map(|y| (x0..x1).map(move |x| (y, x))) {
                let (i, j) = (x - x0, y - y0);
                let texels = || r.strip_texels(op[OP_AUX0] as u32);
                let (color, blend) = match kind {
                    OP_KIND_RECT => (argb, Blend::Alpha),
                    OP_KIND_STRIP => {
                        let along = if aux & AUX_AXIS_Y != 0 { j } else if mirror { x1 - x0 - 1 - i } else { i };
                        let at = (along + ((aux << 16) >> 16)).rem_euclid(texels().len() as i32);
                        (fade(texels()[at as usize], (argb >> 24) as u8), Blend::Alpha)
                    }
                    OP_KIND_IMAGE => {
                        let width = aux & AUX_LOW;
                        let sx = if mirror { width - 1 - i } else { i };
                        let blend = if aux & AUX_ADDITIVE != 0 { Blend::Additive } else { Blend::Alpha };
                        (fade(texels()[(j * width + sx) as usize], (argb >> 24) as u8), blend)
                    }
                    _ => panic!("unknown op kind {kind}"),
                };
                canvas.put(x, y, color, blend);
            }
        }
        canvas
    }

    fn reference(asset: &NameplateAsset, tag: &[i32; TAG_INTS], wearer: u64, frame: u32, custom: &[(u8, u32)], host: &RasterTextures) -> Canvas {
        let (variant, block, seed) = (tag[TAG_VARIANT] as u16, read_block(tag).unwrap(), seed(wearer, asset.identity, SEED_PURPOSE));
        let options = RasterOptions { variant, block, frame, scale: 1, seed, custom: custom.to_vec(), ..RasterOptions::default() };
        canvas_with(asset, &options, host).expect("raster").0
    }

    pub(crate) fn same(a: &Canvas, b: &Canvas) -> bool {
        (CANVAS.y0..CANVAS.y1).all(|y| (CANVAS.x0..CANVAS.x1).all(|x| a.get(x, y) == b.get(x, y)))
    }

    #[test]
    fn frames_check_their_inputs_and_report_the_room_they_need() {
        let mut r = NameplateRenderer::new();
        let id = r.add_asset(load("test_plate"));
        let tag = row(id, 40);
        let signals = vec![0.0; SIGNAL_FLOATS];
        let short = Err(FrameError::Signals { expected: SIGNAL_FLOATS, got: SIGNAL_FLOATS - 1 });
        assert_eq!(r.frame(1.0, &[1], &tag, &signals[1..], &[], &[], &mut []), short);
        let needed = r.needed_words(&tag);
        let worst = r.assets[0].as_ref().unwrap().asset.max_out_words as usize;
        assert_eq!(needed, OUT_HEADER + DIR_WORDS + worst);
        let mut frame = |out: &mut [i32]| r.frame(1.0, &[1], &tag, &signals, &[], &[], out);
        assert_eq!(frame(&mut [0; 4]), Err(FrameError::OutTooSmall { needed }));
        let mut short = vec![0; OUT_HEADER + DIR_WORDS + 10];
        assert_eq!(frame(&mut short), Ok(OUT_HEADER + DIR_WORDS));
        assert_eq!(short[OUT_HEADER + DIR_STATUS], STATUS_NO_ROOM);
        let mut out = vec![0; needed];
        let words = frame(&mut out).unwrap();
        assert_eq!((out[OUT_TAGS], out[OUT_WORDS] as usize), (1, words));
        assert_eq!(out[OUT_HEADER + DIR_STATUS], STATUS_OK);
        let (mut unknown, mut bad, mut flags) = (tag, tag, tag);
        unknown[TAG_ASSET] = 99;
        bad[TAG_NAME_LINE] = 2;
        flags[TAG_FLAGS] = 1 << 12;
        let frame = draw(&mut r, 1.1, &[1, 2, 3, 4], &[unknown, bad, row(id, 600), flags]);
        let statuses: Vec<i32> = (0..4).map(|i| frame.status(i)).collect();
        assert_eq!(statuses, [STATUS_UNKNOWN_ASSET, STATUS_BAD_ROW, STATUS_VANILLA, STATUS_BAD_ROW]);
        assert_eq!(frame.words, OUT_HEADER + 4 * DIR_WORDS);
    }

    fn golden(asset: Arc<NameplateAsset>, widths: &[i32], variants: &[u16], frames: Range<u32>, flags: i32, textures: &[(&str, [u8; 4])]) {
        let (mut r, mut host) = (NameplateRenderer::new(), RasterTextures::default());
        for (name, rgba) in textures {
            assert!(r.set_vanilla_texture(name, 2, 2, rgba.repeat(4)) && host.set(name, 2, 2, rgba.repeat(4)));
        }
        let id = r.add_asset(asset.clone());
        let mut tags = Vec::new();
        for (&variant, &width) in variants.iter().flat_map(|v| widths.iter().map(move |w| (v, w))) {
            let mut tag = row(id, width);
            (tag[TAG_VARIANT], tag[TAG_FLAGS]) = (i32::from(variant), tag[TAG_FLAGS] | flags);
            tags.push(tag);
        }
        let wearers: Vec<u64> = (0..tags.len() as u64).map(|i| 0x5eed + i).collect();
        for frame in frames.start.saturating_sub(24)..frames.end {
            let clock = clock_of(frame, asset.fps);
            draw(&mut r, clock, &wearers, &tags);
            r.run_fallbacks(usize::MAX);
            if frame < frames.start {
                continue;
            }
            let drawn = draw(&mut r, clock, &wearers, &tags);
            for (index, tag) in tags.iter().enumerate() {
                assert_eq!(drawn.status(index), STATUS_OK, "{} frame {frame}", asset.id);
                let expected = reference(&asset, tag, wearers[index], frame, &[], &host);
                assert!(same(&paint(&r, &drawn, index), &expected), "{} tag {index} frame {frame}", asset.id);
            }
        }
    }

    #[test]
    fn frame_canvas_ops_paint_what_the_raster_paints() {
        golden(load("test_plate"), &[16, 40, 96], &[0], 120..132, 0, &[]);
        golden(load("wardrobe_plate"), &[24, 70], &[0, 1, 2], 240..244, 0, &[]);
        golden(load("dome_plate"), &[16, 50, 200], &[0, 1], 300..302, 0, &[]);
        golden(load("sample_plate"), &[20, 64], &[0], 400..432, 0, &[]);
        golden(load("material_plate"), &[30, 90], &[0], 500..512, FLAG_EFFECTS_CPU, &[]);
        golden(load("reaction_plate"), &[24, 80], &[0], 600..612, 0, &[]);
        let host = [("glint", [200, 40, 255, 255]), ("minecraft:block/oak_planks", [120, 90, 40, 255]), ("minecraft:block/prismarine", [60, 160, 150, 255])];
        golden(load("atlas_plate"), &[40], &[0], 700..702, FLAG_EFFECTS_CPU, &host);
        let mut files = kit("test_plate");
        patch(&mut files, "layers/body.json", |b| (b["fill"], b["rings"]) = (json!({ "sweep": "glow", "axis": "y" }), json!([{ "sweep": "gem", "axis": "x" }, { "gradient": "fill" }])));
        patch(&mut files, "manifest.json", |m| m["layers"][1]["mask"] = json!("body"));
        patch(&mut files, "layers/gem.json", |g| g["follow"] = json!("outside"));
        golden(Arc::new(compile_files(&files, None)), &[16, 40, 96], &[0], 120..126, 0, &[]);
        files.push(("sprites/frame.json".into(), br#"{"colors":{"a":"fill","b":"gem"},"rows":["abbba","a...a","abbba"]}"#.to_vec()));
        patch(&mut files, "layers/body.json", |b| b["art"] = json!({ "sprite": "frame", "left": 2, "right": 2 }));
        golden(Arc::new(compile_files(&files, None)), &[16, 40, 96], &[0], 120..126, 0, &[]);
    }

    #[test]
    fn custom_colours_paint_only_their_wearer_like_the_raster() {
        let (asset, none) = (customizable(), RasterTextures::default());
        let mut r = NameplateRenderer::new();
        let id = r.add_asset(asset.clone());
        let slot = |name| asset.palette.slot(name).unwrap() as u8;
        let custom = [(slot("fill"), 0x3f_a3_4d), (slot("gem"), 0xe0_a0_20), (slot("border"), 0x12_34_56)];
        assert!(!r.set_custom_colors(7, id + 1, &custom));
        assert!(r.set_custom_colors(7, id, &custom));
        let tags = [row(id, 40), row(id, 40)];
        (96..120).for_each(|f| _ = draw(&mut r, clock_of(f, asset.fps), &[7, 8], &tags));
        let check = |r: &mut NameplateRenderer, frame: u32, mine: &[(u8, u32)]| {
            let drawn = draw(r, clock_of(frame, asset.fps), &[7, 8], &tags);
            assert!(same(&paint(r, &drawn, 0), &reference(&asset, &tags[0], 7, frame, mine, &none)));
            assert!(same(&paint(r, &drawn, 1), &reference(&asset, &tags[1], 8, frame, &[], &none)));
            same(&paint(r, &drawn, 0), &reference(&asset, &tags[0], 7, frame, &[], &none))
        };
        assert!(!check(&mut r, 120, &custom));
        assert!(r.clear_custom_colors(7, id));
        assert!(check(&mut r, 121, &[]));
        assert!(r.set_custom_colors(7, id, &custom));
        r.remove_asset(id);
        assert!(r.customs.is_empty());
    }

    #[test]
    fn a_hundred_tags_draw_without_allocating_once_warm() {
        let mut r = NameplateRenderer::new();
        let plates = ["test_plate", "sample_plate", "dome_plate", "wardrobe_plate", "reaction_plate", "material_plate"];
        let mut ids: Vec<u32> = plates.iter().map(|p| r.add_asset(load(p))).collect();
        ids.push(r.add_asset(customizable()));
        let mut tags = Vec::new();
        let wearers: Vec<u64> = (1000..1100).collect();
        for (i, &wearer) in wearers.iter().enumerate() {
            let asset = ids[i % ids.len()];
            let mut tag = row(asset, 16 + (i as i32 % 7) * 13);
            (tag[TAG_VARIANT], tag[TAG_COLOR_OFFSET], tag[TAG_COLOR_COUNT]) = ((i % 3) as i32, (i % 3) as i32, 2);
            tag[TAG_FLAGS] |= if i % 4 == 0 { FLAG_EFFECTS_CPU } else { 0 };
            tags.extend_from_slice(&tag);
            if i % 3 == 1 {
                assert!(r.set_custom_colors(wearer, asset, &[(0, 0x3f_a3_4d), (1, 0xe0_a0_20)]));
            }
        }
        let mut signals = vec![0.0f32; 100 * SIGNAL_FLOATS];
        for tag in signals.chunks_exact_mut(SIGNAL_FLOATS).step_by(5) {
            tag[signal_id("is_on_fire").unwrap()] = 1.0;
        }
        let colors = [0xffff_5555u32 as i32, 0xff55_ff55u32 as i32, 0xff55_55ffu32 as i32, -1];
        let mut out = vec![0; r.needed_words(&tags)];
        let mut step = |r: &mut NameplateRenderer, f: u32| {
            if f % 7 == 0 {
                r.fire(1000 + u64::from(f % 100), ids[(f % 100) as usize % ids.len()], 0);
            }
            r.frame(50_000.0 + f64::from(f) / 24.0, &wearers, &tags, &signals, &[], &colors, &mut out).unwrap();
            (0..100).all(|i| out[OUT_HEADER + i * DIR_WORDS + DIR_STATUS] == STATUS_OK)
        };
        for f in 0..24 * 6 {
            assert!(step(&mut r, f));
            r.run_fallbacks(usize::MAX);
        }
        let layout = r.strip_layout();
        let count = allocations_in(|| (24 * 6..24 * 8).for_each(|f| assert!(step(&mut r, f))));
        assert_eq!(count, 0, "allocations in two warm seconds of 100 tags");
        assert_eq!(r.strip_layout(), layout, "warm strips and fallbacks stay put");
    }

    #[test]
    fn a_failing_program_disables_only_its_layer() {
        let mut asset = compile_files(&kit("test_plate"), None);
        let source = format!("({})", vec![vec!["v.a"; 400].join("+"); 3].join(")+("));
        let program = compile_molang(&source, &mut asset.molang.interner).expect("compiles");
        let index = asset.molang.programs.len() as u32;
        asset.molang.programs.push(MolangProgram { program, scope: MolangUse::Channel, path: "test".into(), cost: 0 });
        let gem = asset.layers.iter().position(|l| l.id == "gem").unwrap() as u8;
        let targets = &mut asset.animations.looping.targets;
        let tracks = targets.iter_mut().find(|t| t.target == Target::Layer(gem)).unwrap();
        let keys = vec![Key { frame: 0, value: Value::Molang(index), ease: Ease::Step }];
        tracks.channels.x = Some(Track { keys });
        let mut r = NameplateRenderer::new();
        let id = r.add_asset(Arc::new(asset));
        let first = draw(&mut r, 20.0, &[9], &[row(id, 40)]);
        assert_eq!(first.status(0), STATUS_OK);
        assert_eq!(first.events(0), [(EVENT_LAYER_DISABLED, i32::from(gem))]);
        assert!(!first.layers().contains(&i32::from(gem)) && first.layers().contains(&0));
        let other = draw(&mut r, 21.0, &[10], &[row(id, 40)]);
        assert!(other.events(0).is_empty(), "reported once");
        assert!(!other.layers().contains(&i32::from(gem)), "off for the whole bundle load");
    }

    #[test]
    fn fire_starts_an_equip_one_shot_only_for_new_uploads_and_drawn_wearers() {
        let manifest = json!({"format": 1, "kind": "nameplate", "id": "equip_plate",
            "requires": ["nameplate", "nameplate_one_shots"], "fps": 10, "loop": 10, "layers": [{"id": "body"}],
            "animations": {"hello": {"trigger": "equip", "length": 5, "tracks": {"body": {"position": [[0, [4, 0]]]}}}}});
        let files: Files = vec![
            ("manifest.json".into(), manifest.to_string().into_bytes()),
            ("palettes.json".into(), br##"{"slots":[{"id":"fill","name":"Fill","ramp":["#202020"]}]}"##.to_vec()),
            ("layers/body.json".into(), br#"{"kind":"body","fill":{"color":"fill"}}"#.to_vec()),
        ];
        let x = |r: &mut NameplateRenderer, clock: f64, id: u32| draw(r, clock, &[5], &[row(id, 40)]).ops(0)[0][OP_X0];
        let mut r = NameplateRenderer::new();
        let a = r.add_asset(Arc::new(compile_files(&files, Some(0x1111))));
        let b = r.add_asset(Arc::new(compile_files(&files, Some(0x2222))));
        let same_identity = r.add_asset(Arc::new(compile_files(&files, Some(0x2222))));
        let still = x(&mut r, 40.0, a);
        assert_eq!(x(&mut r, 40.1, a), still, "no equip on first sight");
        assert_eq!(x(&mut r, 40.2, b), still + 4, "a new upload equips");
        assert_eq!(x(&mut r, 41.5, same_identity), still, "same identity, no equip");
        assert!(!r.fire(5, b, 9) && !r.fire(7, b, 0), "unknown one-shot or never drawn");
        assert!(r.fire(5, same_identity, 0));
        assert_eq!(x(&mut r, 41.55, same_identity), still + 4);
    }

    #[test]
    fn cpu_effects_are_made_off_the_frame_and_drawn_once_ready() {
        let mut r = NameplateRenderer::new();
        let asset = load("material_plate");
        let id = r.add_asset(asset.clone());
        let mut tag = row(id, 40);
        tag[TAG_FLAGS] |= FLAG_EFFECTS_CPU;
        let first = draw(&mut r, clock_of(360, 6), &[1], &[tag]);
        assert_eq!(first.count(OP_KIND_IMAGE), 0, "nothing until made");
        draw(&mut r, clock_of(360, 6), &[1], &[tag]);
        assert_eq!(r.pending_fallbacks(), 3, "queued once");
        r.run_fallbacks(usize::MAX);
        assert_eq!(draw(&mut r, clock_of(360, 6), &[1], &[tag]).count(OP_KIND_IMAGE), 3);
        let animated = asset.materials.iter().filter(|m| m.period != Period::Still).count();
        assert!(animated > 0 && animated < 3);
        assert_eq!(draw(&mut r, clock_of(361, 6), &[1], &[tag]).count(OP_KIND_IMAGE), 3, "the last images stand in");
        assert_eq!(r.pending_fallbacks(), animated, "still materials ignore the clock");
        r.remove_asset(id);
        assert_eq!(r.pending_fallbacks(), 0);
    }
}
