use std::cell::RefCell;
use std::collections::BTreeMap;
use std::slice;
use std::sync::Arc;

use crate::container::Image;
use crate::cost::{AnimationRate, Detail, Motion};
use crate::molang::Runtime;
use crate::palette::{MAX_SLOTS, MAX_TONES};
use crate::signals::SIGNAL_COUNT;

use super::abi::{
    pack_scale_rotation, GLYPH_FLOATS, LETTER_ARGB, LETTER_DX, LETTER_DY, LETTER_OPACITY,
    LETTER_OUTLINE, LETTER_SCALE_ROT, LETTER_SHADOW, LETTER_WORDS,
};
use super::anim::channels::{letters as letter_program, pose_layers, shown};
use super::anim::clock::absolute_frame;
use super::anim::particles::{
    emit_in, looped_starts, route_key as particle_route_key, run_starts, run_starts_until,
    SpawnArea,
};
use super::anim::path::{cycle as path_cycle, route_for, walk_gate, walker_pose, Walk};
use super::anim::repeat::{positions, route_key as repeat_route_key};
use super::anim::schedule::event_at;
use super::anim::speed::{
    offset as speed_offset, primary_layer, rejoin_frame, replay, speed_clips, speed_track,
    ClipClock, Episode, Profile,
};
use super::anim::tracks::Channel;
use super::anim::triggers::{burst_tail, schedule_tails};
use super::anim::walker::can_flip;
use super::anim::{animate_palette, clip as anim_clip, ClipRef, ClipTime, Gates, TagTime, Tail};
use super::geom::{BlockInput, LayoutResult, Placement, Rect, CANVAS};
use super::image::{Axis, LayerImage, Painter};
use super::layout::{anchor_point, build_block, pivot_offset, resolve, scaled_corners};
use super::material::cpu::{evaluate_with, NoTextures, TextureSource};
use super::material::ir::{Blend, EdgeLane, Lane, MaterialFrame, TexRef, LANE_COUNT, MAX_SAMPLES};
use super::paint::{mask_state, CanvasPainter};
use super::palette::{custom_variant, resolve_ref};
use super::queries::{NameplateQuery, ParamQueries, QuerySlots};
use super::route::{build, nearest_step, RouteKey};
use super::text::run_letters;
use super::types::{
    BadgePose, Body, Breakpoint, Burst, Depth, LayerContent, LetterPose, Mask, NameplateAsset, Of,
    PaletteFrame, ParticlePose, SpriteLayer, TagPose, Target, TextMode, Variant, MAX_LETTERS,
    OPAQUE,
};

pub const MAX_SCALE: u8 = 16;
const FOREVER: i64 = i64::MIN;

pub const INFO_WIDTH: usize = 0;
pub const INFO_HEIGHT: usize = 1;
pub const INFO_X: usize = 2;
pub const INFO_Y: usize = 3;
pub const INFO_PLATE_DX: usize = 4;
pub const INFO_PLATE_DY: usize = 5;
pub const INFO_BADGE_SHOWN: usize = 6;
pub const INFO_BADGE_X: usize = 7;
pub const INFO_BADGE_Y: usize = 8;
pub const INFO_BADGE_OPACITY: usize = 9;
pub const INFO_TEXT_MODE: usize = 10;
pub const INFO_LETTER_COUNT: usize = 11;
pub const INFO_LETTERS: usize = 12;
pub const INFO_WORDS: usize = INFO_LETTERS + MAX_LETTERS * LETTER_WORDS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RasterState {
    #[default]
    Idle,
    While(u8),
    OneShot(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RasterShot {
    pub index: u16,
    pub age: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RasterStop {
    pub signal: u8,
    pub held: u32,
    pub ago: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RasterBand {
    #[default]
    All,
    UnderText,
    OverText,
}

impl RasterBand {
    pub fn from_value(value: u32) -> Option<RasterBand> {
        match value {
            0 => Some(RasterBand::All),
            1 => Some(RasterBand::UnderText),
            2 => Some(RasterBand::OverText),
            _ => None,
        }
    }

    pub fn keeps(self, depth: Depth) -> bool {
        match self {
            RasterBand::All => true,
            RasterBand::UnderText => depth != Depth::OverText,
            RasterBand::OverText => depth == Depth::OverText,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RasterOptions {
    pub variant: u16,
    pub block: BlockInput,
    pub frame: u32,
    pub scale: u8,
    pub seed: u64,
    pub state: RasterState,
    pub hidden: u32,
    pub omit: u32,
    pub band: RasterBand,
    pub reactions: u32,
    pub held_for: Option<u32>,
    pub observed_for: Option<u32>,
    pub stopped: Vec<RasterStop>,
    pub one_shots: Vec<RasterShot>,
    pub glyphs: Vec<f32>,
    pub detail: Detail,
    pub motion: Motion,
    pub effects_off: bool,
    pub animated_names_off: bool,
    pub custom: Vec<(u8, u32)>,
}

impl Default for RasterOptions {
    fn default() -> RasterOptions {
        RasterOptions {
            variant: 0,
            block: BlockInput::default(),
            frame: 0,
            scale: 0,
            seed: 0,
            state: RasterState::Idle,
            hidden: 0,
            omit: 0,
            band: RasterBand::All,
            reactions: 0,
            held_for: None,
            observed_for: None,
            stopped: Vec::new(),
            one_shots: Vec::new(),
            glyphs: Vec::new(),
            detail: Detail::Full,
            motion: Motion::Full,
            effects_off: false,
            animated_names_off: false,
            custom: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RasterPose {
    pub plate: (i8, i8),
    pub badge: BadgePose,
    pub badge_at: Option<(i32, i32)>,
    pub text_mode: TextMode,
    pub letters: Vec<LetterPose>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub image: Image,
    pub origin: (i32, i32),
    pub pose: RasterPose,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerCanvas {
    pub slot: u8,
    pub depth: Depth,
    pub blend: Blend,
    pub canvas: Canvas,
    pub passes: Vec<Canvas>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterLayer {
    pub slot: u8,
    pub depth: Depth,
    pub blend: Blend,
    pub image: Image,
    pub passes: Vec<Image>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterLayers {
    pub size: (u32, u32),
    pub origin: (i32, i32),
    pub pose: RasterPose,
    pub layers: Vec<RasterLayer>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RasterTextures {
    images: BTreeMap<String, Image>,
}

impl RasterTextures {
    pub fn new() -> RasterTextures {
        RasterTextures::default()
    }

    pub fn set(&mut self, id: &str, width: u32, height: u32, rgba: Vec<u8>) -> bool {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(rgba.len()) {
            return false;
        }
        if rgba.is_empty() {
            self.images.remove(id);
        } else {
            self.images.insert(
                id.to_string(),
                Image {
                    width,
                    height,
                    rgba,
                },
            );
        }
        true
    }

    pub fn get(&self, id: &str) -> Option<&Image> {
        self.images.get(id)
    }

    pub fn len(&self) -> usize {
        self.images.len()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}

impl TextureSource for RasterTextures {
    fn texture(&self, texture: &TexRef) -> Option<&Image> {
        match texture {
            TexRef::Bundle(_) => None,
            TexRef::Vanilla(name) => self.images.get(name.as_str()),
            TexRef::Atlas(id) => self.images.get(id.as_str()),
        }
    }
}

pub fn over(source: u32, destination: u32) -> u32 {
    let sa = source >> 24;
    let da = destination >> 24;
    if sa == 255 || da == 0 {
        return source;
    }
    if sa == 0 {
        return destination;
    }
    let oa = sa + (da * (255 - sa) + 127) / 255;
    let mut out = oa << 24;
    for shift in [16, 8, 0] {
        let sc = (source >> shift) & 0xff;
        let dc = (destination >> shift) & 0xff;
        let blended = (sc * sa * 255 + dc * da * (255 - sa) + oa * 255 / 2) / (oa * 255);
        out |= blended << shift;
    }
    out
}

pub fn add(source: u32, destination: u32) -> u32 {
    let sa = source >> 24;
    let da = destination >> 24;
    let alpha = sa.max(da);
    let mut out = alpha << 24;
    if alpha == 0 {
        return out;
    }
    for shift in [16, 8, 0] {
        let sc = (source >> shift) & 0xff;
        let dc = (destination >> shift) & 0xff;
        let premul = dc * da / 255 + (sc * sa + 127) / 255;
        out |= ((premul * 255 + alpha / 2) / alpha).min(255) << shift;
    }
    out
}

pub fn fade(argb: u32, opacity: u8) -> u32 {
    if opacity == OPAQUE {
        return argb;
    }
    let alpha = ((argb >> 24) * u32::from(opacity) + 127) / 255;
    (alpha << 24) | (argb & 0x00ff_ffff)
}

fn multiply(a: u8, b: u8) -> u8 {
    ((u32::from(a) * u32::from(b) + 127) / 255) as u8
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    area: Rect,
    pixels: Vec<u32>,
}

impl Canvas {
    pub fn new(area: Rect) -> Canvas {
        let area = if area.is_empty() { Rect::EMPTY } else { area };
        let len = (area.width().max(0) as usize) * (area.height().max(0) as usize);
        Canvas {
            area,
            pixels: vec![0; len],
        }
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        self.area.contains(x, y).then(|| {
            (y - self.area.y0) as usize * self.area.width() as usize + (x - self.area.x0) as usize
        })
    }

    pub fn get(&self, x: i32, y: i32) -> u32 {
        self.index(x, y).map_or(0, |i| self.pixels[i])
    }

    pub fn put(&mut self, x: i32, y: i32, argb: u32, blend: Blend) {
        if argb >> 24 == 0 {
            return;
        }
        if let Some(i) = self.index(x, y) {
            let below = self.pixels[i];
            self.pixels[i] = match blend {
                Blend::Alpha => over(argb, below),
                Blend::Additive => add(argb, below),
            };
        }
    }

    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    pub fn clear_within(&mut self, area: Rect) {
        let area = area.intersect(self.area);
        let width = self.area.width().max(0) as usize;
        let span = area.width().max(0) as usize;
        for y in area.y0..area.y1 {
            let from = (y - self.area.y0) as usize * width + (area.x0 - self.area.x0) as usize;
            self.pixels[from..from + span].fill(0);
        }
    }

    pub fn painted(&self) -> Rect {
        let width = self.area.width().max(1) as usize;
        let mut bounds = Rect::EMPTY;
        for (i, pixel) in self.pixels.iter().enumerate() {
            if pixel >> 24 != 0 {
                let x = self.area.x0 + (i % width) as i32;
                let y = self.area.y0 + (i / width) as i32;
                bounds = bounds.union(Rect::new(x, y, x + 1, y + 1));
            }
        }
        bounds
    }

    pub fn crop(&self, area: Rect) -> Canvas {
        let mut out = Canvas::new(area);
        let width = out.area.width().max(0) as usize;
        for (i, pixel) in out.pixels.iter_mut().enumerate() {
            let x = out.area.x0 + (i % width) as i32;
            let y = out.area.y0 + (i / width) as i32;
            *pixel = self.get(x, y);
        }
        out
    }

    pub fn image(&self, scale: u8) -> Image {
        let scale = usize::from(scale.clamp(1, MAX_SCALE));
        let width = self.area.width().max(0) as usize;
        let height = self.area.height().max(0) as usize;
        let mut rgba = vec![0u8; width * height * scale * scale * 4];
        let row_bytes = width * scale * 4;
        for (i, argb) in self.pixels.iter().enumerate() {
            let (x, y) = (i % width, i / width);
            let bytes = [
                (argb >> 16) as u8,
                (argb >> 8) as u8,
                *argb as u8,
                (argb >> 24) as u8,
            ];
            for sy in 0..scale {
                let row = (y * scale + sy) * row_bytes;
                for sx in 0..scale {
                    let at = row + (x * scale + sx) * 4;
                    rgba[at..at + 4].copy_from_slice(&bytes);
                }
            }
        }
        Image {
            width: (width * scale) as u32,
            height: (height * scale) as u32,
            rgba,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cover {
    area: Rect,
    bits: Vec<bool>,
}

impl Cover {
    pub fn new(rects: &[Rect]) -> Cover {
        let area = rects.iter().fold(Rect::EMPTY, |all, r| all.union(*r));
        let width = area.width().max(0) as usize;
        let mut bits = vec![false; width * area.height().max(0) as usize];
        for rect in rects.iter().filter(|r| !r.is_empty()) {
            for y in rect.y0..rect.y1 {
                let row = (y - area.y0) as usize * width;
                for x in rect.x0..rect.x1 {
                    bits[row + (x - area.x0) as usize] = true;
                }
            }
        }
        Cover { area, bits }
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.area.contains(x, y)
            && self.bits[(y - self.area.y0) as usize * self.area.width() as usize
                + (x - self.area.x0) as usize]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Clip {
    #[default]
    None,
    Rect(Rect),
    Cover(Arc<Cover>),
}

impl Clip {
    pub fn allows(&self, x: i32, y: i32) -> bool {
        match self {
            Clip::None => true,
            Clip::Rect(rect) => rect.contains(x, y),
            Clip::Cover(cover) => cover.contains(x, y),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawSource {
    Image(Arc<LayerImage>),
    Pixels { area: Rect, pixels: Arc<[u32]> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draw {
    pub layer: u8,
    pub depth: Depth,
    pub source: DrawSource,
    pub dx: i32,
    pub dy: i32,
    pub mirror: Option<i32>,
    pub opacity: u8,
    pub blend: Blend,
    pub clip: Clip,
}

impl Draw {
    pub fn image(layer: u8, depth: Depth, image: Arc<LayerImage>) -> Draw {
        Draw {
            layer,
            depth,
            source: DrawSource::Image(image),
            dx: 0,
            dy: 0,
            mirror: None,
            opacity: OPAQUE,
            blend: Blend::Alpha,
            clip: Clip::None,
        }
    }

    pub fn place(&self, x: i32, y: i32) -> (i32, i32) {
        (
            self.mirror.map_or(x, |axis| axis - x) + self.dx,
            y + self.dy,
        )
    }

    pub fn place_rect(&self, rect: Rect) -> Rect {
        let (x0, x1) = match self.mirror {
            Some(axis) => (axis - rect.x1 + 1, axis - rect.x0 + 1),
            None => (rect.x0, rect.x1),
        };
        Rect::new(x0, rect.y0, x1, rect.y1).offset(self.dx, self.dy)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Colors<'a> {
    pub frame: &'a PaletteFrame,
    pub fixed: &'a [u32],
    pub stops: &'a [Vec<u32>],
    pub counts: [u8; MAX_SLOTS],
}

impl<'a> Colors<'a> {
    pub fn new(
        asset: &'a NameplateAsset,
        variant: &'a Variant,
        frame: &'a PaletteFrame,
    ) -> Colors<'a> {
        let mut counts = [0u8; MAX_SLOTS];
        let colorway = variant.colorway.map(usize::from);
        for (slot, count) in counts
            .iter_mut()
            .enumerate()
            .take(asset.palette.slots.len())
        {
            *count = asset.palette.tone_count(slot, colorway).min(MAX_TONES) as u8;
        }
        Colors {
            frame,
            fixed: &asset.fixed_colors,
            stops: &variant.stops,
            counts,
        }
    }

    pub fn strip(&self, slot: u8, along: i32) -> u32 {
        let count = i32::from(self.counts.get(usize::from(slot)).copied().unwrap_or(0)).max(1);
        let tone = along.rem_euclid(count) as usize;
        self.frame
            .tones
            .get(usize::from(slot) * MAX_TONES + tone)
            .copied()
            .unwrap_or(0)
    }
}

pub fn composite(draws: &[Draw], colors: &Colors<'_>, plate: (i32, i32), canvas: &mut Canvas) {
    let area = canvas.area();
    composite_within(draws, colors, plate, canvas, area);
}

fn source_window(draw: &Draw, placed: Rect) -> Rect {
    let (x0, x1) = match draw.mirror {
        Some(axis) => (
            axis + draw.dx - placed.x1 + 1,
            axis + draw.dx - placed.x0 + 1,
        ),
        None => (placed.x0 - draw.dx, placed.x1 - draw.dx),
    };
    Rect::new(x0, placed.y0 - draw.dy, x1, placed.y1 - draw.dy)
}

trait Sink {
    fn area(&self) -> Rect;
    fn put(&mut self, x: i32, y: i32, argb: u32, blend: Blend);
}

impl Sink for Canvas {
    fn area(&self) -> Rect {
        Canvas::area(self)
    }

    fn put(&mut self, x: i32, y: i32, argb: u32, blend: Blend) {
        Canvas::put(self, x, y, argb, blend);
    }
}

struct LayerPasses {
    combined: Canvas,
    writes: Vec<u32>,
    passes: Vec<Canvas>,
    used: usize,
}

impl LayerPasses {
    fn new(area: Rect) -> LayerPasses {
        let combined = Canvas::new(area);
        LayerPasses {
            writes: vec![0; combined.pixels.len()],
            combined,
            passes: Vec::new(),
            used: 0,
        }
    }

    fn exact_passes(&self, bounds: Rect) -> Vec<Canvas> {
        if self.used < 2 {
            return Vec::new();
        }
        self.passes[..self.used]
            .iter()
            .map(|pass| pass.crop(bounds))
            .collect()
    }

    fn clear_within(&mut self, bounds: Rect) {
        self.combined.clear_within(bounds);
        for pass in &mut self.passes[..self.used] {
            pass.clear_within(bounds);
        }
        let area = bounds.intersect(self.combined.area);
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                if let Some(i) = self.combined.index(x, y) {
                    self.writes[i] = 0;
                }
            }
        }
        self.used = 0;
    }
}

impl Sink for LayerPasses {
    fn area(&self) -> Rect {
        self.combined.area
    }

    fn put(&mut self, x: i32, y: i32, argb: u32, blend: Blend) {
        if argb >> 24 == 0 {
            return;
        }
        let Some(i) = self.combined.index(x, y) else {
            return;
        };
        self.combined.put(x, y, argb, blend);
        let pass = self.writes[i] as usize;
        self.writes[i] += 1;
        if pass == self.passes.len() {
            self.passes.push(Canvas::new(self.combined.area));
        }
        self.used = self.used.max(pass + 1);
        self.passes[pass].pixels[i] = argb;
    }
}

pub fn composite_within(
    draws: &[Draw],
    colors: &Colors<'_>,
    plate: (i32, i32),
    canvas: &mut Canvas,
    area: Rect,
) {
    blit_draws(draws, colors, plate, canvas, area);
}

fn blit_draws<S: Sink>(
    draws: &[Draw],
    colors: &Colors<'_>,
    plate: (i32, i32),
    canvas: &mut S,
    area: Rect,
) {
    let area = area.intersect(canvas.area());
    if area.is_empty() {
        return;
    }
    let mut order: Vec<usize> = (0..draws.len()).collect();
    order.sort_by_key(|i| draws[*i].depth.band());
    for draw in order.into_iter().map(|i| &draws[i]) {
        let window = source_window(draw, area.offset(-plate.0, -plate.1));
        let mut blit = |x: i32, y: i32, argb: u32| {
            let (px, py) = draw.place(x, y);
            if draw.clip.allows(px, py) {
                canvas.put(px + plate.0, py + plate.1, argb, draw.blend);
            }
        };
        match &draw.source {
            DrawSource::Image(image) => {
                for rect in &image.rects {
                    let color = resolve_ref(rect.color, colors.frame, colors.fixed, colors.stops);
                    let argb = fade(color, draw.opacity);
                    if argb >> 24 == 0 {
                        continue;
                    }
                    let seen = Rect::new(
                        i32::from(rect.x0),
                        i32::from(rect.y0),
                        i32::from(rect.x1),
                        i32::from(rect.y1),
                    )
                    .intersect(window);
                    for y in seen.y0..seen.y1 {
                        for x in seen.x0..seen.x1 {
                            blit(x, y, argb);
                        }
                    }
                }
                for strip in &image.strips {
                    let seen = Rect::new(
                        i32::from(strip.x0),
                        i32::from(strip.y0),
                        i32::from(strip.x1),
                        i32::from(strip.y1),
                    )
                    .intersect(window);
                    for y in seen.y0..seen.y1 {
                        for x in seen.x0..seen.x1 {
                            let along = match strip.axis {
                                Axis::X => x - i32::from(strip.x0),
                                Axis::Y => y - i32::from(strip.y0),
                            };
                            let color = colors.strip(strip.slot, along + i32::from(strip.phase));
                            blit(x, y, fade(color, draw.opacity));
                        }
                    }
                }
            }
            DrawSource::Pixels { area: from, pixels } => {
                let width = from.width().max(0) as usize;
                if width == 0 {
                    continue;
                }
                let rows = pixels.len().div_ceil(width);
                let full = Rect::new(from.x0, from.y0, from.x1, from.y0 + rows as i32);
                let seen = full.intersect(window);
                for y in seen.y0..seen.y1 {
                    let row = (y - from.y0) as usize * width;
                    for x in seen.x0..seen.x1 {
                        let Some(argb) = pixels.get(row + (x - from.x0) as usize) else {
                            break;
                        };
                        blit(x, y, fade(*argb, draw.opacity));
                    }
                }
            }
        }
    }
}

pub struct Scene<'a> {
    pub asset: &'a NameplateAsset,
    pub variant: &'a Variant,
    pub placement: &'a Placement,
    pub layout: &'a LayoutResult,
    pub pose: &'a TagPose,
    pub particles: &'a [ParticlePose],
    pub time: TagTime,
    pub display: bool,
    pub hidden: u32,
    pub paths: bool,
    pub speeds: &'a [PathSpeed],
}

#[derive(Clone, Debug)]
pub struct PathSpeed {
    pub layer: u8,
    pub profile: Arc<Profile>,
    pub episode: Episode,
}

pub(super) fn layer_target(layout: &LayoutResult, layer: usize) -> Rect {
    layout.targets.get(layer).copied().unwrap_or(layout.bounds)
}

pub(super) fn layer_sprite_size(
    asset: &NameplateAsset,
    placement: &Placement,
    layer: usize,
    sprite: &SpriteLayer,
) -> (i32, i32) {
    let width = placement.block.name_width;
    let chosen = asset.layers[layer]
        .breakpoints
        .iter()
        .find(|b| b.contains(width))
        .and_then(|b| b.sprite)
        .unwrap_or(sprite.sprite);
    asset
        .sprites
        .get(usize::from(chosen))
        .map_or((0, 0), |s| (i32::from(s.width), i32::from(s.height)))
}

impl Scene<'_> {
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
        layer_target(self.layout, layer)
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
        layer_sprite_size(self.asset, self.placement, layer, sprite)
    }

    fn spot(&self, layer: usize, sprite: &SpriteLayer, mirrored: bool) -> (i32, i32) {
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
                let route = build(&key);
                route.at(i64::try_from(nearest_step(&route, ax, ay)).unwrap_or(0))
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

    pub fn route_reach(&self, layer: usize, bounds: Rect) -> Rect {
        let spec = &self.asset.layers[layer];
        let LayerContent::Sprite(sprite) = &spec.content else {
            return Rect::EMPTY;
        };
        let Some(path) = &sprite.path else {
            return Rect::EMPTY;
        };
        if bounds.is_empty() {
            return Rect::EMPTY;
        }
        let pose = self.pose.layers.get(layer).copied().unwrap_or_default();
        let (width, height) = self.sprite_size(layer, sprite);
        let size = [width, height].map(|v| u16::try_from(v.max(0)).unwrap_or(u16::MAX));
        let block = &self.placement.block;
        let route = route_for(path, self.target(layer), block, self.body_of(spec.of), size);
        let [ox, oy] = sprite.offset.map(i32::from);
        let (rx, ry) = self.spot(layer, sprite, false);
        let (px, py) = match path.follow {
            Some(_) => (0, 0),
            None => pivot_offset(sprite.pivot, width, height),
        };
        let image = if can_flip(&path.facings) {
            let axis = 2 * rx + width - 1;
            bounds.union(Rect::new(
                axis - bounds.x1 + 1,
                bounds.y0,
                axis - bounds.x0 + 1,
                bounds.y1,
            ))
        } else {
            bounds
        };
        let mut steps = route
            .steps
            .iter()
            .map(|(x, y)| (i32::from(*x), i32::from(*y)));
        let first = steps.next().unwrap_or((0, 0));
        let (low, high) = steps.fold((first, first), |(low, high), (x, y)| {
            ((low.0.min(x), low.1.min(y)), (high.0.max(x), high.1.max(y)))
        });
        let dx = i32::from(pose.dx) - px + ox - rx;
        let dy = i32::from(pose.dy) - py + oy - ry;
        Rect::new(
            image.x0 + low.0 + dx,
            image.y0 + low.1 + dy,
            image.x1 + high.0 + dx,
            image.y1 + high.1 + dy,
        )
    }

    fn state(&self, layer: usize, own: u16) -> u16 {
        match self.asset.layers[layer].mask {
            Some(Mask::Layer { slot, .. }) => {
                let mask = self
                    .resolved(usize::from(slot))
                    .and_then(|l| self.pose.layers.get(l))
                    .map_or(0, |p| p.state);
                mask_state(
                    self.asset,
                    u8::try_from(layer).unwrap_or(u8::MAX),
                    own,
                    mask,
                )
            }
            _ => own,
        }
    }

    fn lanes(&self, layer: usize, body: Rect, exprs: &dyn Fn(u32) -> f64) -> [f64; LANE_COUNT] {
        let mut lanes = [0.0; LANE_COUNT];
        let LayerContent::Material(content) = &self.asset.layers[layer].content else {
            return lanes;
        };
        let Some(ir) = self.asset.materials.get(usize::from(content.material)) else {
            return lanes;
        };
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
}

pub(super) fn layer_walk(
    asset: &NameplateAsset,
    placement: &Placement,
    layout: &LayoutResult,
    layer: usize,
) -> Option<Walk> {
    let spec = &asset.layers[layer];
    let LayerContent::Sprite(sprite) = &spec.content else {
        return None;
    };
    let path = sprite.path.as_ref()?;
    let (width, height) = layer_sprite_size(asset, placement, layer, sprite);
    let size = [width, height].map(|v| u16::try_from(v.max(0)).unwrap_or(u16::MAX));
    let route = route_for(
        path,
        layer_target(layout, layer),
        &placement.block,
        body_of(asset, spec.of),
        size,
    );
    Some(Walk::new(
        path,
        &route,
        path_cycle(asset.loop_frames, spec.step),
    ))
}

pub(super) fn material_time(
    asset: &NameplateAsset,
    layer: usize,
    time: &TagTime,
    delay: i64,
) -> (i32, [i32; 4]) {
    let step = i64::from(asset.layers[layer].step.max(1));
    let cycle = (i64::from(asset.loop_frames) / step).max(1);
    let drift = matches!(&asset.layers[layer].content, LayerContent::Material(m) if m.drift);
    let back = delay * step;
    let (frame, state) = if drift {
        let frame = i64::from(time.engine) + i64::from(time.offset) - back;
        (frame, frame.div_euclid(step))
    } else {
        let frame = i64::from(time.loop_frame) - back;
        (
            frame.rem_euclid(i64::from(asset.loop_frames.max(1))),
            frame.div_euclid(step).rem_euclid(cycle),
        )
    };
    (
        i32::try_from(frame).unwrap_or(0),
        [
            i32::try_from(state).unwrap_or(0),
            i32::try_from(cycle).unwrap_or(1),
            i32::from(asset.fps),
            i32::try_from(step).unwrap_or(1),
        ],
    )
}

pub fn draws(
    scene: &Scene<'_>,
    painter: &dyn Painter,
    exprs: &dyn Fn(u32, i32) -> f64,
) -> Vec<Draw> {
    draws_with(scene, painter, exprs, &NoTextures)
}

pub fn draws_with(
    scene: &Scene<'_>,
    painter: &dyn Painter,
    exprs: &dyn Fn(u32, i32) -> f64,
    textures: &dyn TextureSource,
) -> Vec<Draw> {
    let asset = scene.asset;
    let mut out: Vec<Draw> = Vec::new();
    for slot in 0..asset.layers.len() {
        if slot < 32 && scene.hidden & (1 << slot) != 0 {
            continue;
        }
        let Some(layer) = scene.resolved(slot) else {
            continue;
        };
        if !scene.shown(slot, layer) {
            continue;
        }
        let pose = scene.pose.layers.get(layer).copied().unwrap_or_default();
        if !pose.visible {
            continue;
        }
        let spec = &asset.layers[layer];
        let index = u8::try_from(slot).unwrap_or(u8::MAX);
        let base = |source: DrawSource| Draw {
            layer: index,
            depth: spec.depth,
            source,
            dx: i32::from(pose.dx),
            dy: i32::from(pose.dy),
            mirror: None,
            opacity: pose.opacity,
            blend: Blend::Alpha,
            clip: Clip::None,
        };
        let paint = |state: u16| {
            Arc::new(painter.paint(
                asset,
                u8::try_from(layer).unwrap_or(u8::MAX),
                state,
                scene.placement,
                scene.layout,
            ))
        };
        match &spec.content {
            LayerContent::Body(_) => out.push(base(DrawSource::Image(paint(
                scene.state(layer, pose.state),
            )))),
            LayerContent::Sprite(sprite) => {
                let (width, height) = scene.sprite_size(layer, sprite);
                let target = scene.target(layer);
                let size = [width, height].map(|v| u16::try_from(v.max(0)).unwrap_or(u16::MAX));
                let [ox, oy] = sprite.offset.map(i32::from);
                let (rx, ry) = scene.spot(layer, sprite, false);
                let place = |image: Arc<LayerImage>, x: i32, y: i32, flip: bool| {
                    let mut draw = base(DrawSource::Image(image));
                    draw.dx += x - rx;
                    draw.dy += y - ry;
                    draw.mirror = flip.then_some(2 * rx + width - 1);
                    draw
                };
                if let Some(path) = &sprite.path {
                    let block = &scene.placement.block;
                    let route = route_for(path, target, block, scene.body_of(spec.of), size);
                    let walk = Walk::new(path, &route, path_cycle(asset.loop_frames, spec.step));
                    let shift = |t: i64| -> i64 {
                        scene
                            .speeds
                            .iter()
                            .filter(|s| scene.paths && usize::from(s.layer) == layer)
                            .map(|s| speed_offset(&walk, &s.profile, s.episode, spec.step, t))
                            .sum()
                    };
                    let frame = i64::from(scene.time.engine) + i64::from(scene.time.offset);
                    let (t, walkers) = walk_gate(path, scene.paths, frame, spec.step);
                    let (px, py) = match path.follow {
                        Some(_) => (0, 0),
                        None => pivot_offset(sprite.pivot, width, height),
                    };
                    for index in 0..walkers {
                        let at = walker_pose(&walk, &route, path, index, t, &shift);
                        let own = at.facing.unwrap_or(pose.state);
                        let image = paint(scene.state(layer, own));
                        out.push(place(image, at.x - px + ox, at.y - py + oy, at.flip));
                    }
                } else if let Some(repeat) = &sprite.repeat {
                    let key = repeat_route_key(
                        u8::try_from(layer).unwrap_or(u8::MAX),
                        repeat,
                        target,
                        sprite.follow,
                        scene.body_of(spec.of),
                        size,
                    );
                    let route = key.map(|key| build(&key));
                    let mut spots = Vec::new();
                    let pivot = sprite.pivot;
                    positions(
                        repeat,
                        target,
                        width,
                        height,
                        pivot,
                        route.as_ref(),
                        &mut spots,
                    );
                    let image = paint(scene.state(layer, pose.state));
                    for (x, y) in spots {
                        out.push(place(image.clone(), x + ox, y + oy, false));
                    }
                } else {
                    let image = paint(scene.state(layer, pose.state));
                    let main = place(image, rx, ry, false);
                    let mirrored = spec.mirror.map(|_| {
                        let (x, y) = scene.spot(layer, sprite, true);
                        Draw {
                            dx: x - rx - i32::from(pose.dx),
                            dy: y - ry + i32::from(pose.dy),
                            mirror: Some(2 * rx + width - 1),
                            ..main.clone()
                        }
                    });
                    out.push(main);
                    out.extend(mirrored);
                }
            }
            LayerContent::Emitter(_) => {
                for particle in scene
                    .particles
                    .iter()
                    .filter(|p| usize::from(p.layer) == layer)
                {
                    let mut draw = base(DrawSource::Image(paint(particle.state)));
                    draw.dx += i32::from(particle.x);
                    draw.dy += i32::from(particle.y);
                    draw.opacity = multiply(pose.opacity, particle.opacity);
                    out.push(draw);
                }
            }
            LayerContent::Material(content) => {
                let Some(ir) = asset.materials.get(usize::from(content.material)) else {
                    continue;
                };
                let body = scene.material_body(layer);
                let area = ir.area(body);
                let delays = [Some(0), spec.mirror.map(|m| i64::from(m.delay))];
                for (copy, delay) in delays.into_iter().flatten().enumerate() {
                    let (at, clock) = material_time(asset, layer, &scene.time, delay);
                    let frame = MaterialFrame {
                        area,
                        body,
                        clock,
                        lanes: scene.lanes(layer, body, &|program| exprs(program, at)),
                        tones: &scene.pose.palette.tones,
                        atlas: [[0.0; 4]; MAX_SAMPLES],
                    };
                    let pixels = evaluate_with(ir, &frame, &asset.textures, textures);
                    let mut draw = base(DrawSource::Pixels {
                        area,
                        pixels: pixels.into(),
                    });
                    draw.blend = ir.blend;
                    if copy == 1 {
                        draw.mirror = Some(body.x0 + body.x1 - 1);
                        draw.dx = -i32::from(pose.dx);
                    }
                    out.push(draw);
                }
            }
        }
    }
    apply_masks(scene, &mut out);
    out
}

fn apply_masks(scene: &Scene<'_>, draws: &mut [Draw]) {
    let asset = scene.asset;
    let mut covers: Vec<(u8, bool, Arc<Cover>)> = Vec::new();
    for i in 0..draws.len() {
        let slot = usize::from(draws[i].layer);
        let Some(mask) = scene.resolved(slot).and_then(|l| asset.layers[l].mask) else {
            continue;
        };
        draws[i].clip = match mask {
            Mask::Name => Clip::Rect(scene.layout.name),
            Mask::Layer { .. } if matches!(draws[i].source, DrawSource::Image(_)) => continue,
            Mask::Layer { slot: target, fill } => {
                let cover = match covers.iter().find(|(t, f, _)| *t == target && *f == fill) {
                    Some((_, _, cover)) => cover.clone(),
                    None => {
                        let cover = Arc::new(cover_of(scene, draws, target, fill));
                        covers.push((target, fill, cover.clone()));
                        cover
                    }
                };
                Clip::Cover(cover)
            }
        };
    }
}

fn cover_of(scene: &Scene<'_>, draws: &[Draw], target: u8, fill: bool) -> Cover {
    let limit = if fill {
        scene
            .resolved(usize::from(target))
            .and_then(|l| scene.layout.bodies.get(l).copied().flatten())
    } else {
        None
    };
    if fill && limit.is_none() {
        return Cover::new(&[]);
    }
    let mut rects = Vec::new();
    for draw in draws.iter().filter(|d| d.layer == target) {
        let DrawSource::Image(image) = &draw.source else {
            continue;
        };
        let sources = image
            .rects
            .iter()
            .map(|r| (r.x0, r.y0, r.x1, r.y1))
            .chain(image.strips.iter().map(|s| (s.x0, s.y0, s.x1, s.y1)));
        for (x0, y0, x1, y1) in sources {
            let mut rect = Rect::new(i32::from(x0), i32::from(y0), i32::from(x1), i32::from(y1));
            if let Some(limit) = limit {
                rect = rect.intersect(limit);
            }
            if !rect.is_empty() {
                rects.push(draw.place_rect(rect));
            }
        }
    }
    Cover::new(&rects)
}

pub fn render(scene: &Scene<'_>, painter: &dyn Painter, exprs: &dyn Fn(u32, i32) -> f64) -> Canvas {
    render_with(scene, painter, exprs, &NoTextures)
}

pub fn render_with(
    scene: &Scene<'_>,
    painter: &dyn Painter,
    exprs: &dyn Fn(u32, i32) -> f64,
    textures: &dyn TextureSource,
) -> Canvas {
    let list = draws_with(scene, painter, exprs, textures);
    let canvas = composited(scene, &list);
    let area = scene.placement.block.background.union(canvas.painted());
    canvas.crop(area)
}

pub fn render_band_with(
    scene: &Scene<'_>,
    painter: &dyn Painter,
    exprs: &dyn Fn(u32, i32) -> f64,
    textures: &dyn TextureSource,
    omit: u32,
    band: RasterBand,
) -> Canvas {
    let list = draws_with(scene, painter, exprs, textures);
    let full = composited(scene, &list);
    let area = scene.placement.block.background.union(full.painted());
    if omit == 0 && band == RasterBand::All {
        return full.crop(area);
    }
    let kept: Vec<Draw> = list
        .into_iter()
        .filter(|d| !(d.layer < 32 && omit & (1 << d.layer) != 0) && band.keeps(d.depth))
        .collect();
    composited(scene, &kept).crop(area)
}

pub fn render_layers_with(
    scene: &Scene<'_>,
    painter: &dyn Painter,
    exprs: &dyn Fn(u32, i32) -> f64,
    textures: &dyn TextureSource,
    omit: u32,
    band: RasterBand,
) -> (Rect, Vec<LayerCanvas>) {
    let list = draws_with(scene, painter, exprs, textures);
    let colors = Colors::new(scene.asset, scene.variant, &scene.pose.palette);
    let plate = plate_offset(scene);
    let mut order: Vec<usize> = (0..list.len()).collect();
    order.sort_by_key(|i| list[*i].depth.band());
    let mut scratch = LayerPasses::new(CANVAS);
    let mut painted = Rect::EMPTY;
    let mut out = Vec::new();
    let mut start = 0;
    while start < order.len() {
        let first = &list[order[start]];
        let end = order[start..]
            .iter()
            .position(|i| list[*i].layer != first.layer)
            .map_or(order.len(), |n| start + n);
        for i in &order[start..end] {
            blit_draws(
                slice::from_ref(&list[*i]),
                &colors,
                plate,
                &mut scratch,
                CANVAS,
            );
        }
        let bounds = scratch.combined.painted();
        painted = painted.union(bounds);
        let slot = first.layer;
        let omitted = slot < 32 && omit & (1 << slot) != 0;
        if !omitted && band.keeps(first.depth) {
            out.push(LayerCanvas {
                slot,
                depth: first.depth,
                blend: first.blend,
                canvas: scratch.combined.crop(bounds),
                passes: scratch.exact_passes(bounds),
            });
        }
        scratch.clear_within(bounds);
        start = end;
    }
    let area = scene.placement.block.background.union(painted);
    for layer in &mut out {
        layer.canvas = layer.canvas.crop(area);
        for pass in &mut layer.passes {
            *pass = pass.crop(area);
        }
    }
    (area, out)
}

fn plate_offset(scene: &Scene<'_>) -> (i32, i32) {
    if scene.display {
        (0, 0)
    } else {
        (i32::from(scene.pose.plate.0), i32::from(scene.pose.plate.1))
    }
}

fn composited(scene: &Scene<'_>, list: &[Draw]) -> Canvas {
    let colors = Colors::new(scene.asset, scene.variant, &scene.pose.palette);
    let mut canvas = Canvas::new(CANVAS);
    composite(list, &colors, plate_offset(scene), &mut canvas);
    canvas
}

struct Molang<'a> {
    asset: &'a NameplateAsset,
    frame: i64,
    variant: &'a Variant,
    slots: QuerySlots,
    runtime: RefCell<Runtime>,
}

impl<'a> Molang<'a> {
    fn new(
        asset: &'a NameplateAsset,
        variant: &'a Variant,
        layout: &LayoutResult,
        frame: i64,
        signals: &[(bool, f32); SIGNAL_COUNT],
        (seed, clock): (u64, f64),
    ) -> Molang<'a> {
        let interner = &asset.molang.interner;
        let slots = QuerySlots::with_params(interner, &asset.params);
        let mut runtime = Runtime::new(interner);
        runtime.set_seed(seed);
        runtime.set_clock(clock);
        let block = &layout.block;
        let name = block.name_line_rect();
        let bg = block.background;
        let values = [
            (NameplateQuery::Width, layout.bounds.width()),
            (NameplateQuery::Height, layout.bounds.height()),
            (NameplateQuery::LineCount, i32::from(block.line_count)),
            (NameplateQuery::Frame, i32::try_from(frame).unwrap_or(0)),
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
        for (id, (on, time)) in signals.iter().enumerate() {
            if let Some(slot) = slots.signal(id) {
                runtime.set_query(slot, if *on { 1.0 } else { 0.0 });
            }
            if let Some(slot) = slots.time(id) {
                runtime.set_query(slot, *time);
            }
        }
        Molang {
            asset,
            frame,
            variant,
            slots,
            runtime: RefCell::new(runtime),
        }
    }

    fn at(&self, program: u32, frame: i32) -> f64 {
        self.raw(program, Some(frame))
    }

    fn finite(&self, program: u32, frame: i32) -> f64 {
        let raw = self.raw(program, Some(frame));
        if raw.is_finite() {
            raw
        } else {
            0.0
        }
    }

    fn argb(&self, program: u32, frame: i32) -> u32 {
        let raw = self.raw(program, Some(frame));
        if !raw.is_finite() {
            return 0xff00_0000;
        }
        0xff00_0000 | libm::floor(raw.clamp(0.0, f64::from(0x00ff_ffff))) as u32
    }

    fn raw(&self, program: u32, frame: Option<i32>) -> f64 {
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
            values: &self.variant.params,
        };
        let value = entry.program.run(&mut runtime, &mut queries);
        if let (Some(_), Some(slot)) = (frame, slot) {
            runtime.set_query(slot, self.frame as f32);
        }
        value
    }

    fn letters(
        &self,
        program: u32,
        frame: i32,
        mode: TextMode,
        glyphs: &[f32],
        out: &mut [LetterPose],
    ) -> bool {
        let slot = self.slots.slot(NameplateQuery::Frame);
        let mut runtime = self.runtime.borrow_mut();
        if let Some(slot) = slot {
            runtime.set_query(slot, frame as f32);
        }
        let mut queries = ParamQueries {
            slots: &self.slots,
            values: &self.variant.params,
        };
        let ok = run_letters(
            self.asset,
            program,
            mode,
            glyphs,
            &mut runtime.frame(),
            &mut queries,
            out,
        );
        if let Some(slot) = slot {
            runtime.set_query(slot, self.frame as f32);
        }
        ok
    }
}

fn shown_frame(frame: u32, fps: u8, rate: AnimationRate) -> u32 {
    if rate == AnimationRate::EveryFrame {
        return frame;
    }
    let clock = (f64::from(frame) + 0.5) / f64::from(fps.max(1));
    u32::try_from(absolute_frame(rate.time(clock), fps.max(1))).unwrap_or(0)
}

fn playing(
    asset: &NameplateAsset,
    time: &TagTime,
    live: u32,
    reactions: u32,
    held_for: Option<u32>,
    shots: &[RasterShot],
) -> (Vec<ClipTime>, Vec<Tail>) {
    let engine = i64::from(time.engine);
    let mut out = vec![ClipTime {
        clip: ClipRef::Loop,
        frame: i32::from(time.loop_frame),
        start: 0,
    }];
    for (index, schedule) in asset.animations.schedules.iter().enumerate() {
        if let Some(event) = event_at(schedule, time.seed, engine) {
            out.push(ClipTime {
                clip: ClipRef::Schedule {
                    schedule: u16::try_from(index).unwrap_or(u16::MAX),
                    event: event.event,
                },
                frame: i32::try_from(engine - event.start).unwrap_or(0),
                start: event.start,
            });
        }
    }
    for (index, reaction) in asset.animations.reactions.iter().enumerate() {
        if held(reactions, reaction.signal) {
            out.push(ClipTime {
                clip: ClipRef::Reaction(u16::try_from(index).unwrap_or(u16::MAX)),
                frame: i32::from(time.loop_frame),
                start: held_for.map_or(FOREVER, |h| i64::from(live) - i64::from(h)),
            });
        }
    }
    let first = out.len();
    let mut tails = Vec::new();
    for shot in shots {
        let Some(one) = asset.animations.one_shots.get(usize::from(shot.index)) else {
            continue;
        };
        let age = i64::from(shot.age);
        let start = i64::from(live) - age;
        let at = ClipTime {
            clip: ClipRef::OneShot(shot.index),
            frame: (engine - start).clamp(0, i64::from(i32::MAX)) as i32,
            start,
        };
        if age < i64::from(one.clip.length) {
            out.push(at);
        } else if age < burst_tail(asset, &one.clip) {
            tails.push(Tail {
                time: at,
                until: i64::MAX,
            });
        }
    }
    schedule_tails(asset, time.seed, engine, engine, &mut tails);
    out[first..].sort_unstable_by_key(|t| match t.clip {
        ClipRef::OneShot(index) => (index, t.start),
        _ => (u16::MAX, t.start),
    });
    (out, tails)
}

struct History<'a> {
    live: i64,
    reactions: u32,
    held_for: Option<u32>,
    observed_for: Option<u32>,
    stopped: &'a [RasterStop],
    shots: &'a [RasterShot],
}

impl History<'_> {
    fn held_since(&self, signal: u8) -> Option<i64> {
        held(self.reactions, signal).then(|| self.held_for.map_or(0, |h| self.live - i64::from(h)))
    }

    fn observed_since(&self) -> i64 {
        self.observed_for.map_or(0, |o| self.live - i64::from(o))
    }

    fn changed(&self, signal: u8) -> i64 {
        if held(self.reactions, signal) {
            return self
                .held_for
                .map_or_else(|| self.observed_since(), |h| self.live - i64::from(h));
        }
        self.stopped
            .iter()
            .filter(|s| s.signal == signal)
            .map(|s| self.live - i64::from(s.ago))
            .max()
            .unwrap_or_else(|| self.observed_since())
    }

    fn signal_runs(&self, signal: u8) -> Vec<(i64, Option<i64>)> {
        let mut runs: Vec<(i64, Option<i64>)> = self
            .stopped
            .iter()
            .filter(|s| s.signal == signal)
            .map(|s| {
                let stop = self.live - i64::from(s.ago);
                (stop - i64::from(s.held), Some(stop))
            })
            .collect();
        runs.extend(self.held_since(signal).map(|start| (start, None)));
        runs.sort_unstable();
        runs
    }

    fn shot_runs(&self, index: u16, length: i64) -> Vec<(i64, Option<i64>)> {
        let mut runs: Vec<(i64, Option<i64>)> = self
            .shots
            .iter()
            .filter(|s| s.index == index)
            .map(|s| {
                let start = self.live - i64::from(s.age);
                (start, Some(start + length))
            })
            .collect();
        runs.sort_unstable();
        runs
    }

    fn runs(&self, asset: &NameplateAsset, clip: ClipRef) -> Vec<(i64, Option<i64>)> {
        match clip {
            ClipRef::Reaction(index) => asset
                .animations
                .reactions
                .get(usize::from(index))
                .map_or_else(Vec::new, |r| self.signal_runs(r.signal)),
            ClipRef::OneShot(index) => asset
                .animations
                .one_shots
                .get(usize::from(index))
                .map_or_else(Vec::new, |o| {
                    self.shot_runs(index, i64::from(o.clip.length))
                }),
            _ => Vec::new(),
        }
    }
}

fn path_speeds(
    asset: &NameplateAsset,
    placement: &Placement,
    layout: &LayoutResult,
    history: &History<'_>,
    (now, offset): (i64, i64),
) -> Vec<PathSpeed> {
    let walks: Vec<Option<Walk>> = (0..asset.layers.len())
        .map(|layer| layer_walk(asset, placement, layout, layer))
        .collect();
    let profile = |clip: ClipRef, layer: usize, origin: Option<i64>| {
        let track = speed_track(asset, clip, layer)?;
        let clock = origin.map_or(ClipClock::Loop, ClipClock::Local);
        let step = asset.layers[layer].step;
        Some(Profile::reaction(track, clock, step, asset.loop_frames))
    };
    let mut out = Vec::new();
    for clip in speed_clips(&asset.animations) {
        let Some(primary) = primary_layer(asset, clip) else {
            continue;
        };
        let Some(walk) = walks.get(primary).and_then(Option::as_ref) else {
            continue;
        };
        let one_shot = matches!(clip, ClipRef::OneShot(_));
        let step = asset.layers[primary].step;
        let runs: Vec<(i64, Option<i64>)> = history
            .runs(asset, clip)
            .into_iter()
            .map(|(start, stop)| (start + offset, stop.map(|s| s + offset)))
            .collect();
        let episode = replay(&runs, now, |previous, ended| {
            let origin = one_shot.then_some(previous.start);
            rejoin_frame(walk, &profile(clip, primary, origin)?, ended, step)
        });
        let Some(episode) = episode else {
            continue;
        };
        let origin = one_shot.then_some(episode.start);
        for (layer, walk) in walks.iter().enumerate() {
            if walk.is_none() {
                continue;
            }
            if let Some(profile) = profile(clip, layer, origin) {
                out.push(PathSpeed {
                    layer: u8::try_from(layer).unwrap_or(u8::MAX),
                    profile: Arc::new(profile),
                    episode,
                });
            }
        }
    }
    out
}

fn held(reactions: u32, signal: u8) -> bool {
    1u32.checked_shl(u32::from(signal))
        .is_some_and(|bit| reactions & bit != 0)
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

fn particles(
    asset: &NameplateAsset,
    layout: &LayoutResult,
    time: &TagTime,
    kept: &[ClipTime],
    tails: &[Tail],
    gates: &Gates,
) -> Vec<ParticlePose> {
    let mut out = Vec::new();
    if !gates.particles {
        return out;
    }
    let engine = i64::from(time.engine);
    let now = engine + i64::from(time.offset);
    let loop_frames = asset.loop_frames;
    for (index, layer) in asset.layers.iter().enumerate() {
        let LayerContent::Emitter(emitter) = &layer.content else {
            continue;
        };
        let id = u8::try_from(index).unwrap_or(u8::MAX);
        let rect = layout.targets.get(index).copied().unwrap_or(layout.bounds);
        let body = body_of(asset, layer.of);
        let route = particle_route_key(id, emitter, &asset.sprites, rect, body).map(|k| build(&k));
        let area = SpawnArea::new(&emitter.spawn, rect, &layout.block, body, route.as_ref());
        let longest = emitter.lifetime[1];
        let mut bursts = Vec::new();
        let lists = |time: &ClipTime| {
            anim_clip(asset, time.clip)
                .map(|clip| clip.targets.as_slice())
                .unwrap_or(&[])
                .iter()
                .filter(|t| t.target == Target::Layer(id))
                .map(|t| t.channels.bursts.as_slice())
                .collect::<Vec<&[Burst]>>()
        };
        for time in kept {
            for list in lists(time) {
                match time.clip {
                    ClipRef::Loop => {
                        let window = (i64::MIN, None);
                        looped_starts(list, loop_frames, now, longest, window, &mut bursts)
                    }
                    ClipRef::Reaction(_) => {
                        let from = match time.start {
                            FOREVER => i64::MIN,
                            start => now - (engine - start),
                        };
                        let window = (from, None);
                        looped_starts(list, loop_frames, now, longest, window, &mut bursts)
                    }
                    ClipRef::Schedule { .. } | ClipRef::OneShot(_) => {
                        let origin = now - i64::from(time.frame);
                        run_starts(list, origin, now, longest, &mut bursts)
                    }
                }
            }
        }
        if !gates.first_frame {
            for tail in tails {
                for list in lists(&tail.time) {
                    let origin = now - i64::from(tail.time.frame);
                    run_starts_until(list, origin, now, longest, tail.until, &mut bursts);
                }
            }
        }
        bursts.sort_unstable_by_key(|b| (b.start, b.count));
        let sprites = &asset.sprites;
        emit_in(
            emitter, sprites, id, &area, time.seed, now, &bursts, &mut out,
        );
    }
    out
}

pub fn canvas(asset: &NameplateAsset, options: &RasterOptions) -> Option<Canvas> {
    canvas_with(asset, options, &NoTextures).map(|(canvas, _)| canvas)
}

pub fn canvas_with(
    asset: &NameplateAsset,
    options: &RasterOptions,
    textures: &dyn TextureSource,
) -> Option<(Canvas, RasterPose)> {
    with_scene(asset, options, |scene, exprs| {
        render_band_with(
            scene,
            &CanvasPainter,
            exprs,
            textures,
            options.omit,
            options.band,
        )
    })
}

pub fn layer_canvases_with(
    asset: &NameplateAsset,
    options: &RasterOptions,
    textures: &dyn TextureSource,
) -> Option<((Rect, Vec<LayerCanvas>), RasterPose)> {
    with_scene(asset, options, |scene, exprs| {
        render_layers_with(
            scene,
            &CanvasPainter,
            exprs,
            textures,
            options.omit,
            options.band,
        )
    })
}

fn with_scene<R>(
    asset: &NameplateAsset,
    options: &RasterOptions,
    run: impl FnOnce(&Scene<'_>, &dyn Fn(u32, i32) -> f64) -> R,
) -> Option<(R, RasterPose)> {
    let designed = asset.variant(options.variant)?;
    let custom =
        (!options.custom.is_empty()).then(|| custom_variant(asset, designed, &options.custom));
    let variant = custom.as_ref().unwrap_or(designed);
    let block = build_block(&options.block)?;
    let placement = Placement {
        block,
        variant: options.variant,
        breakpoint: asset.breakpoint(block.name_width),
    };
    let layout = resolve(asset, &placement);
    let mut gates = Gates::new(options.motion, options.detail);
    if options.effects_off {
        gates.effects = false;
    }
    if options.animated_names_off {
        gates.letters = false;
    }
    let engine = shown_frame(options.frame, asset.fps, gates.rate);
    let time = TagTime::at(engine, asset.loop_frames, asset.phase, options.seed);
    let mut reactions = options.reactions;
    let mut shots = options.one_shots.clone();
    match options.state {
        RasterState::Idle => {}
        RasterState::While(signal) => {
            if let Some(bit) = 1u32.checked_shl(u32::from(signal)) {
                reactions |= bit;
            }
        }
        RasterState::OneShot(index) => shots.push(RasterShot {
            index,
            age: options.frame,
        }),
    }
    let (clips, tails) = playing(
        asset,
        &time,
        options.frame,
        reactions,
        options.held_for,
        &shots,
    );
    let history = History {
        live: i64::from(options.frame),
        reactions,
        held_for: options.held_for,
        observed_for: options.observed_for,
        stopped: &options.stopped,
        shots: &shots,
    };
    let fps = f64::from(asset.fps.max(1));
    let mut signals = [(false, 0.0f32); SIGNAL_COUNT];
    for (id, (on, time)) in signals.iter_mut().enumerate() {
        let Ok(id) = u8::try_from(id) else {
            continue;
        };
        *on = held(reactions, id);
        *time = ((history.live - history.changed(id)) as f64 / fps) as f32;
    }
    let clock = (f64::from(options.frame) + 0.5) / fps;
    let molang = Molang::new(
        asset,
        variant,
        &layout,
        i64::from(time.loop_frame),
        &signals,
        (time.seed, clock),
    );
    let mut kept = Vec::new();
    shown(asset, &clips, &gates, &mut kept);
    let sampler =
        |program: u32, frame: i32, channel: Channel| channel.convert(molang.at(program, frame));
    let colors = |program: u32, frame: i32| molang.argb(program, frame);
    let mut pose = TagPose::default();
    pose_layers(asset, variant, &kept, &sampler, &gates, &[], &mut pose);
    animate_palette(asset, variant, &kept, &colors, &gates, &mut pose);
    let particles = particles(asset, &layout, &time, &kept, &tails, &gates);

    let text_mode = if options.animated_names_off {
        TextMode::Vanilla
    } else {
        variant.text.mode
    };
    let count = options.glyphs.len() / GLYPH_FLOATS;
    let mut letters = Vec::new();
    if text_mode != TextMode::Vanilla && count <= MAX_LETTERS && gates.letters {
        if let Some((program, frame)) = letter_program(asset, variant, &kept, &gates) {
            let glyphs = &options.glyphs[..count * GLYPH_FLOATS];
            let mut posed = vec![LetterPose::STILL; count];
            if molang.letters(program, frame, text_mode, glyphs, &mut posed) {
                letters = posed;
            }
        }
    }
    let plate = if options.block.display {
        (0, 0)
    } else {
        (i32::from(pose.plate.0), i32::from(pose.plate.1))
    };
    let badge_at = layout.badge.filter(|_| pose.badge.visible).map(|rect| {
        (
            rect.x0 + i32::from(pose.badge.dx) + plate.0,
            rect.y0 + i32::from(pose.badge.dy) + plate.1,
        )
    });

    let speeds = if gates.paths {
        let offset = i64::from(time.offset);
        let now = i64::from(time.engine) + offset;
        path_speeds(asset, &placement, &layout, &history, (now, offset))
    } else {
        Vec::new()
    };
    let mut scene = Scene {
        asset,
        variant,
        placement: &placement,
        layout: &layout,
        pose: &pose,
        particles: &particles,
        time,
        display: options.block.display,
        hidden: options.hidden,
        paths: gates.paths,
        speeds: &speeds,
    };
    if !gates.effects {
        for slot in 0..asset.layers.len().min(32) {
            let material = scene
                .resolved(slot)
                .is_some_and(|l| matches!(asset.layers[l].content, LayerContent::Material(_)));
            if material {
                scene.hidden |= 1 << slot;
            }
        }
    }
    let exprs = |program: u32, frame: i32| molang.finite(program, frame);
    let rendered = run(&scene, &exprs);
    let raster_pose = RasterPose {
        plate: pose.plate,
        badge: pose.badge,
        badge_at,
        text_mode,
        letters,
    };
    Some((rendered, raster_pose))
}

pub fn raster(asset: &NameplateAsset, options: &RasterOptions) -> Option<Raster> {
    raster_with(asset, options, &NoTextures)
}

impl Raster {
    pub fn info(&self) -> [i32; INFO_WORDS] {
        info_words(
            (self.image.width, self.image.height),
            self.origin,
            &self.pose,
        )
    }
}

impl RasterLayers {
    pub fn info(&self) -> [i32; INFO_WORDS] {
        info_words(self.size, self.origin, &self.pose)
    }
}

fn info_words(size: (u32, u32), origin: (i32, i32), pose: &RasterPose) -> [i32; INFO_WORDS] {
    let mut out = [0; INFO_WORDS];
    out[INFO_WIDTH] = i32::try_from(size.0).unwrap_or(i32::MAX);
    out[INFO_HEIGHT] = i32::try_from(size.1).unwrap_or(i32::MAX);
    out[INFO_X] = origin.0;
    out[INFO_Y] = origin.1;
    out[INFO_PLATE_DX] = i32::from(pose.plate.0);
    out[INFO_PLATE_DY] = i32::from(pose.plate.1);
    if let Some((x, y)) = pose.badge_at {
        out[INFO_BADGE_SHOWN] = 1;
        out[INFO_BADGE_X] = x;
        out[INFO_BADGE_Y] = y;
        out[INFO_BADGE_OPACITY] = i32::from(pose.badge.opacity);
    }
    out[INFO_TEXT_MODE] = pose.text_mode.value();
    let letters = &pose.letters[..pose.letters.len().min(MAX_LETTERS)];
    out[INFO_LETTER_COUNT] = letters.len() as i32;
    for (letter, words) in letters
        .iter()
        .zip(out[INFO_LETTERS..].chunks_exact_mut(LETTER_WORDS))
    {
        words[LETTER_DX] = i32::from(letter.dx);
        words[LETTER_DY] = i32::from(letter.dy);
        words[LETTER_ARGB] = letter.color as i32;
        words[LETTER_SHADOW] = letter.shadow as i32;
        words[LETTER_OPACITY] = i32::from(letter.opacity);
        words[LETTER_SCALE_ROT] = pack_scale_rotation(letter.scale_q, letter.rotation);
        words[LETTER_OUTLINE] = letter.outline as i32;
    }
    out
}

pub fn raster_with(
    asset: &NameplateAsset,
    options: &RasterOptions,
    textures: &dyn TextureSource,
) -> Option<Raster> {
    let (canvas, pose) = canvas_with(asset, options, textures)?;
    let area = canvas.area();
    Some(Raster {
        image: canvas.image(options.scale),
        origin: (area.x0, area.y0),
        pose,
    })
}

pub fn raster_layers(asset: &NameplateAsset, options: &RasterOptions) -> Option<RasterLayers> {
    raster_layers_with(asset, options, &NoTextures)
}

pub fn raster_layers_with(
    asset: &NameplateAsset,
    options: &RasterOptions,
    textures: &dyn TextureSource,
) -> Option<RasterLayers> {
    let ((area, canvases), pose) = layer_canvases_with(asset, options, textures)?;
    let scale = u32::from(options.scale.clamp(1, MAX_SCALE));
    let size = [area.width(), area.height()].map(|v| u32::try_from(v.max(0)).unwrap_or(0) * scale);
    Some(RasterLayers {
        size: (size[0], size[1]),
        origin: (area.x0, area.y0),
        pose,
        layers: canvases
            .into_iter()
            .map(|layer| RasterLayer {
                slot: layer.slot,
                depth: layer.depth,
                blend: layer.blend,
                image: layer.canvas.image(options.scale),
                passes: layer
                    .passes
                    .iter()
                    .map(|pass| pass.image(options.scale))
                    .collect(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use serde_json::{json, Value};

    use super::*;
    use crate::nameplate::abi::*;
    use crate::nameplate::anim::SEED_PURPOSE;
    use crate::nameplate::frame::tests::{clock_of, compile_files, draw_with, kit, paint, patch, same, Files};
    use crate::nameplate::frame::NameplateRenderer;
    use crate::nameplate::types::Trigger;
    use crate::signals::signal_id;
    use crate::wearer::seed;

    const WEARER: u64 = 0x5eed;
    const FIRED_AT: u32 = 603;
    const RED: u32 = 0xffff_0000;
    const GREEN: u32 = 0xff00_ff00;
    const HALF_BLUE: u32 = 0x8000_00ff;

    #[test]
    fn source_over_and_additive_blends_match_the_integer_reference() {
        assert_eq!(over(0xff12_3456, 0xff00_00ff), 0xff12_3456);
        assert_eq!(over(0x8012_3456, 0), 0x8012_3456);
        assert_eq!(over(0, 0xff00_00ff), 0xff00_00ff);
        assert_eq!(over(0x80ff_0000, 0xff00_00ff), 0xff80_007f);
        assert_eq!(over(0x80ff_0000, 0x8000_00ff), 0xc0aa_0055);
        assert_eq!(add(0x80ff_0000, 0xff00_00ff), 0xff80_00ff);
        assert_eq!(add(0xffff_8000, 0xff80_8080), 0xffff_ff80);
        assert_eq!(fade(0xff12_3456, 128), 0x8012_3456);
        assert_eq!(fade(0x8012_3456, 0), 0x0012_3456);
        assert_eq!(fade(0x0112_3456, 127), 0x0012_3456);
    }

    #[test]
    fn canvases_crop_to_what_was_painted_and_scale_into_rgba() {
        let mut canvas = Canvas::new(Rect::new(-2, 0, 3, 2));
        canvas.put(1, 1, 0xff10_2030, Blend::Alpha);
        canvas.put(0, 0, 0x0010_2030, Blend::Alpha);
        canvas.put(9, 9, 0xff10_2030, Blend::Alpha);
        canvas.put(1, 1, 0x80ff_0000, Blend::Additive);
        assert_eq!(canvas.get(1, 1), add(0x80ff_0000, 0xff10_2030));
        assert_eq!((canvas.get(0, 0), canvas.get(9, 9)), (0, 0));
        assert_eq!(canvas.painted(), Rect::new(1, 1, 2, 2));
        let cropped = canvas.crop(Rect::new(0, 1, 3, 3));
        assert_eq!(cropped.pixels(), &[0, canvas.get(1, 1), 0, 0, 0, 0]);
        let image = cropped.image(2);
        assert_eq!((image.width, image.height), (6, 4));
        let px = |x: usize, y: usize| &image.rgba[(y * 6 + x) * 4..][..4];
        let [a, r, g, b] = canvas.get(1, 1).to_be_bytes();
        assert!([(2, 0), (3, 0), (2, 1), (3, 1)].iter().all(|&(x, y)| px(x, y) == [r, g, b, a]));
        assert_eq!((px(1, 0), px(2, 2)), (&[0u8; 4][..], &[0u8; 4][..]));
        assert_eq!(cropped.image(200).width, 3 * u32::from(MAX_SCALE));
    }

    fn input() -> BlockInput {
        let background = Rect::vanilla_background(40, 8);
        BlockInput { line_widths: [40, 0, 0], line_count: 1, name_width: 40, background, badge: true, ..BlockInput::default() }
    }

    fn glyphs() -> Vec<f32> {
        (0..5).flat_map(|i| [i as f32 * 8.0, 6.0]).collect()
    }

    fn shot_index(asset: &NameplateAsset, signal: &str) -> u16 {
        let trigger = Trigger::Start(signal_id(signal).unwrap() as u8);
        asset.animations.one_shots.iter().position(|s| s.trigger == trigger).unwrap() as u16
    }

    fn moving_files() -> Files {
        let mut files = kit("reaction_plate");
        patch(&mut files, "manifest.json", |m| {
            m["requires"].as_array_mut().unwrap().push(json!("nameplate_letters"));
            m["text"]["mode"] = json!("motion");
            m["text"]["motion"] = json!("math.sin(q.frame * 30 + q.glyph_index * 40) * 2");
            m["animations"]["hurt_shake"]["tracks"]["$plate"] = json!({ "position": [[0, [2, -1]], [4, [0, 0], "linear"]] });
            m["animations"]["fire_spark"]["tracks"]["$text"] = json!({ "motion": "q.glyph_index - 2" });
            m["animations"]["sneak_drop"]["tracks"]["gem"]["position"] = json!([[0, ["q.is_sneaking_time * 4", 3]]]);
        });
        files
    }

    fn letter_words(pose: &LetterPose) -> [i32; LETTER_WORDS] {
        let (color, shadow, outline) = (pose.color as i32, pose.shadow as i32, pose.outline as i32);
        let scale = pack_scale_rotation(pose.scale_q, pose.rotation);
        [i32::from(pose.dx), i32::from(pose.dy), color, shadow, i32::from(pose.opacity), scale, outline]
    }

    fn check(files: &Files, motion: Motion, detail: Detail, flags: i32, runs: &[(&str, Range<u32>)], frames: Range<u32>) {
        let asset = Arc::new(compile_files(files, None));
        let mut r = NameplateRenderer::new();
        r.set_motion(motion);
        let id = r.add_asset(asset.clone());
        let mut row = [0; TAG_INTS];
        assert!(write_block(&input(), &mut row));
        (row[TAG_ASSET], row[TAG_SNEAK_ALPHA], row[TAG_DETAIL], row[TAG_GLYPH_COUNT]) = (id as i32, 255, i32::from(detail.value()), 5);
        row[TAG_FLAGS] |= flags;
        let shot = shot_index(&asset, "is_hurt");
        for frame in 560..frames.end {
            let mut signals = vec![0.0f32; SIGNAL_FLOATS];
            let (seed, observed_for, animated_names_off) = (seed(WEARER, asset.identity, SEED_PURPOSE), Some(frame - 560), flags & FLAG_ANIMATED_NAMES_OFF != 0);
            let mut options = RasterOptions { block: input(), frame, scale: 1, seed, held_for: Some(0), observed_for, glyphs: glyphs(), detail, motion, animated_names_off, ..RasterOptions::default() };
            if frame > FIRED_AT {
                options.one_shots.push(RasterShot { index: shot, age: frame - FIRED_AT });
            }
            for (name, run) in runs {
                let signal = signal_id(name).unwrap();
                if run.contains(&frame) {
                    signals[signal] = 1.0;
                    options.reactions |= 1 << signal;
                    options.held_for = options.held_for.max(Some(frame - run.start));
                } else if run.end <= frame {
                    let stop = RasterStop { signal: signal as u8, held: run.end - run.start, ago: frame - run.end };
                    options.stopped.push(stop);
                }
                for (index, one) in asset.animations.one_shots.iter().enumerate() {
                    let edge = match one.trigger {
                        Trigger::Start(s) if usize::from(s) == signal => run.start,
                        Trigger::Stop(s) if usize::from(s) == signal => run.end,
                        _ => continue,
                    };
                    if edge <= frame {
                        options.one_shots.push(RasterShot { index: index as u16, age: frame - edge });
                    }
                }
            }
            let drawn = draw_with(&mut r, clock_of(frame, asset.fps), &[WEARER], &[row], &signals, &glyphs());
            if frame == FIRED_AT {
                assert!(r.fire(WEARER, id, shot));
            }
            if !frames.contains(&frame) {
                continue;
            }
            let (canvas, pose) = canvas_with(&asset, &options, &NoTextures).expect("raster");
            let at = format!("{motion:?}/{detail:?} frame {frame}");
            assert!(same(&paint(&r, &drawn, 0), &canvas), "{at}");
            let section = drawn.section(0);
            let plate = (i32::from(pose.plate.0), i32::from(pose.plate.1));
            assert_eq!((section[HEAD_PLATE_DX], section[HEAD_PLATE_DY]), plate, "{at}");
            assert_eq!(section[HEAD_TEXT_MODE], pose.text_mode.value(), "{at}");
            let text = TAG_HEAD + section[HEAD_OPS] as usize * OP_WORDS + section[HEAD_MATERIAL_WORDS] as usize;
            let letters: Vec<i32> = pose.letters.iter().flat_map(letter_words).collect();
            assert_eq!(section[text + TEXT_LETTER_COUNT] as usize, pose.letters.len(), "{at}");
            assert_eq!(section[text + TEXT_FIXED..][..letters.len()], letters[..], "{at}");
            let badge = &section[text + TEXT_FIXED + letters.len()..];
            let expected = pose.badge_at.map_or([0; 3], |(x, y)| [x, y, i32::from(pose.badge.opacity)]);
            assert_eq!([badge[BADGE_X], badge[BADGE_Y], badge[BADGE_OPACITY]], expected, "{at}");
        }
    }

    #[test]
    fn rasters_match_the_frame_for_signals_one_shots_motion_and_detail() {
        let moving = moving_files();
        let fire = [("is_on_fire", 600..u32::MAX)];
        for (motion, detail, flags) in [
            (Motion::Full, Detail::Full, 0),
            (Motion::Reduced, Detail::Full, 0),
            (Motion::Off, Detail::Full, 0),
            (Motion::Full, Detail::Reduced, 0),
            (Motion::Full, Detail::Minimal, 0),
            (Motion::Full, Detail::Full, FLAG_ANIMATED_NAMES_OFF),
        ] {
            check(&moving, motion, detail, flags, &fire, 598..616);
        }
        let sprint = [("is_sprinting", 600..606), ("is_sprinting", 612..u32::MAX)];
        for motion in [Motion::Full, Motion::Reduced] {
            check(&kit("reaction_plate"), motion, Detail::Full, 0, &sprint, 598..640);
        }
        let mixed = [("is_sneaking", 600..609), ("is_riding", 615..630)];
        check(&moving, Motion::Full, Detail::Full, 0, &mixed, 580..640);
    }

    #[test]
    fn raster_info_carries_the_pose_and_legacy_states_match_their_sets() {
        let asset = Arc::new(compile_files(&moving_files(), None));
        let hurt = shot_index(&asset, "is_hurt");
        let fire = signal_id("is_on_fire").unwrap() as u8;
        let options = RasterOptions {
            block: input(),
            frame: 605,
            reactions: 1 << fire,
            one_shots: vec![RasterShot { index: hurt, age: 1 }],
            glyphs: glyphs(),
            ..RasterOptions::default()
        };
        let (_, pose) = canvas_with(&asset, &options, &NoTextures).unwrap();
        let info = raster(&asset, &options).unwrap().info();
        assert_ne!(pose.plate, (0, 0));
        assert_eq!((info[INFO_PLATE_DX], info[INFO_PLATE_DY]), (i32::from(pose.plate.0), i32::from(pose.plate.1)));
        assert_eq!((info[INFO_BADGE_SHOWN], info[INFO_BADGE_X], info[INFO_BADGE_Y]), (1, pose.badge_at.unwrap().0, pose.badge_at.unwrap().1));
        assert_eq!((info[INFO_TEXT_MODE], info[INFO_LETTER_COUNT]), (TextMode::Motion.value(), 5));
        let letters: Vec<i32> = pose.letters.iter().flat_map(letter_words).collect();
        assert_eq!(info[INFO_LETTERS..INFO_LETTERS + letters.len()], letters[..]);
        assert!(info[INFO_LETTERS + letters.len()..].iter().all(|w| *w == 0));

        let base = RasterOptions { block: input(), frame: 4, ..RasterOptions::default() };
        let with = |state, reactions, one_shots| RasterOptions { state, reactions, one_shots, ..base.clone() };
        let set = with(RasterState::Idle, 1 << fire, vec![]);
        assert_eq!(raster(&asset, &with(RasterState::While(fire), 0, vec![])), raster(&asset, &set));
        let set = with(RasterState::Idle, 0, vec![RasterShot { index: hurt, age: 4 }]);
        assert_eq!(raster(&asset, &with(RasterState::OneShot(hurt), 0, vec![])), raster(&asset, &set));
        assert_ne!(raster(&asset, &base), raster(&asset, &set));
        assert_eq!(raster(&asset, &RasterOptions { variant: 99, ..base.clone() }), None);
        assert_eq!(raster(&asset, &RasterOptions { block: BlockInput::default(), ..base }), None);
    }

    fn bundle(layers: Value, files: Vec<(&str, Value)>, variants: Value) -> Arc<NameplateAsset> {
        let mut manifest = json!({ "format": 1, "kind": "nameplate", "id": "bands", "fps": 6, "loop": 12,
            "requires": ["nameplate", "nameplate_materials", "nameplate_masks"], "text": { "color": "ink" }, "layers": layers });
        if !variants.is_null() {
            manifest["variants"] = variants;
        }
        let palettes = json!({ "slots": [{ "id": "ink", "ramp": ["#FFFFFF"] }, { "id": "green", "ramp": ["#00FF00"] }, { "id": "blue", "ramp": ["#800000FF"] }] });
        let mut all = vec![("manifest.json".to_string(), manifest.to_string().into_bytes()), ("palettes.json".into(), palettes.to_string().into_bytes())];
        all.extend(files.into_iter().map(|(name, value)| (name.to_string(), value.to_string().into_bytes())));
        Arc::new(compile_files(&all, None))
    }

    fn constant(color: [f64; 4], blend: &str) -> Value {
        json!({ "resolution": "pixel", "margins": [4, 4, 4, 4],
            "nodes": [{ "id": "out", "type": "Output", "in": { "color": color }, "set": { "blend": blend } }] })
    }

    fn body(fill: &str) -> Value {
        json!({ "kind": "body", "padding": [2, 1, 2, 1], "fill": { "color": fill } })
    }

    fn options(variant: u16, band: RasterBand, omit: u32) -> RasterOptions {
        let block = BlockInput { badge: false, ..input() };
        RasterOptions { variant, block, band, omit, ..RasterOptions::default() }
    }

    fn band(asset: &NameplateAsset, variant: u16, band: RasterBand, omit: u32) -> Canvas {
        canvas(asset, &options(variant, band, omit)).expect("a canvas")
    }

    fn front_body(asset: &NameplateAsset, variant: u16, body: usize) -> (i32, i32) {
        let block = build_block(&options(0, RasterBand::All, 0).block).unwrap();
        let placement = Placement { block, variant, breakpoint: asset.breakpoint(block.name_width) };
        let rect = resolve(asset, &placement).bodies[body].unwrap();
        (rect.x0, rect.y0)
    }

    #[test]
    fn masked_materials_keep_their_clip_in_each_band_and_swapped_slots_band_by_layer() {
        let layers = json!([{ "id": "back", "depth": "background", "mask": "front" }, { "id": "front", "depth": "over_text" },
            { "id": "alt", "depth": "over_text", "mask": "front" }]);
        let files = vec![
            ("materials/red.json", constant([1.0, 0.0, 0.0, 1.0], "alpha")),
            ("materials/green.json", constant([0.0, 1.0, 0.0, 1.0], "alpha")),
            ("layers/back.json", json!({ "kind": "material", "material": "red" })),
            ("layers/front.json", body("blue")),
            ("layers/alt.json", json!({ "kind": "material", "material": "green" })),
        ];
        let variants = json!({ "plain": { "hide": ["alt"] }, "swapped": { "hide": ["alt"], "swap": { "back": "alt" } } });
        let asset = bundle(layers, files, variants);
        let (x, y) = front_body(&asset, 0, 1);
        let all = band(&asset, 0, RasterBand::All, 0);
        let under = band(&asset, 0, RasterBand::UnderText, 0);
        let over_text = band(&asset, 0, RasterBand::OverText, 0);
        assert_eq!((all.get(x, y), under.get(x, y), over_text.get(x, y)), (over(HALF_BLUE, RED), RED, HALF_BLUE));
        assert_eq!((under.get(x - 2, y), over_text.get(x - 2, y)), (0, 0), "clipped to the front body");
        assert_eq!((under.area(), over_text.area()), (all.area(), all.area()));
        assert_eq!(band(&asset, 0, RasterBand::All, 1 << 1).get(x, y), RED);
        let hidden = canvas(&asset, &RasterOptions { hidden: 1 << 1, ..options(0, RasterBand::All, 0) }).unwrap();
        assert_eq!(hidden.get(x, y), 0, "hiding the mask target drops the clip");
        assert_eq!(band(&asset, 1, RasterBand::UnderText, 0).get(x, y), 0);
        assert_eq!(band(&asset, 1, RasterBand::OverText, 0).get(x, y), over(HALF_BLUE, GREEN));
        assert_eq!(band(&asset, 1, RasterBand::OverText, 0).get(x - 2, y), 0);
    }

    fn glow(blend: &str, alpha: f64, masked: bool) -> Arc<NameplateAsset> {
        let mut front = json!({ "id": "glow", "depth": "over_text", "mask": "back" });
        if !masked {
            front.as_object_mut().unwrap().remove("mask");
        }
        let files = vec![
            ("materials/glow.json", constant([1.0, 0.0, 0.0, alpha], blend)),
            ("layers/back.json", body("green")),
            ("layers/glow.json", json!({ "kind": "material", "material": "glow", "mirror": true })),
        ];
        bundle(json!([{ "id": "back", "depth": "background" }, front]), files, Value::Null)
    }

    fn flatten(area: Rect, layers: &[LayerCanvas], into: &mut Canvas) {
        for layer in layers {
            let passes = if layer.passes.is_empty() { slice::from_ref(&layer.canvas) } else { &layer.passes[..] };
            for (pass, y) in passes.iter().flat_map(|p| (area.y0..area.y1).map(move |y| (p, y))) {
                (area.x0..area.x1).for_each(|x| into.put(x, y, pass.get(x, y), layer.blend));
            }
        }
    }

    #[test]
    fn layer_canvases_flatten_exactly_into_every_band() {
        for (blend, alpha, masked) in [("additive", 0.5, true), ("alpha", 0.25, true), ("additive", 1.0, false), ("alpha", 0.5, false)] {
            let asset = glow(blend, alpha, masked);
            let all = band(&asset, 0, RasterBand::All, 0);
            let ((area, layers), _) = layer_canvases_with(&asset, &options(0, RasterBand::All, 0), &NoTextures).unwrap();
            assert_eq!(area, all.area());
            assert_eq!((layers[1].depth, layers[1].blend), (Depth::OverText, if blend == "alpha" { Blend::Alpha } else { Blend::Additive }));
            assert_eq!(layers[1].passes.len(), 2, "a mirrored material draws twice");
            let mut flat = Canvas::new(area);
            flatten(area, &layers, &mut flat);
            assert_eq!(flat, all, "{blend} {alpha} {masked}");
            let ((_, over_text), _) = layer_canvases_with(&asset, &options(0, RasterBand::OverText, 0), &NoTextures).unwrap();
            let mut stacked = band(&asset, 0, RasterBand::UnderText, 0).crop(area);
            flatten(area, &over_text, &mut stacked);
            assert_eq!(stacked, all, "{blend} {alpha} {masked} over text");
        }
        let options = RasterOptions { scale: 2, ..options(0, RasterBand::All, 0) };
        let (whole, layers) = (raster(&glow("additive", 0.5, true), &options).unwrap(), raster_layers(&glow("additive", 0.5, true), &options).unwrap());
        assert_eq!((layers.info(), layers.size), (whole.info(), (whole.image.width, whole.image.height)));
        assert!(layers.layers.iter().flat_map(|l| [&l.image].into_iter().chain(&l.passes)).all(|i| (i.width, i.height) == layers.size));
    }
}
