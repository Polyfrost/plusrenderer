use std::mem;

use crate::palette::MAX_TONES;

use super::flag::{band_rows, color_at, span};
use super::geom::{Block, LayoutResult, Placement, Rect};
use super::image::{Axis, ColorRef, LayerImage, PaintRect, Painter, StripRun};
use super::layout::{anchor_point, body_rect, pivot_offset, row_insets, scaled_corners};
use super::palette::blend_ref;
use super::route::{build, nearest_step, RouteKey};
use super::types::{
    Body, BodyArt, Emitter, Fill, Flag, Layer, LayerContent, Mask, NameplateAsset, Of, Sprite,
    SpriteLayer, Variant, MAX_LAYERS,
};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Copy, Debug, Default)]
pub struct CanvasPainter;

impl Painter for CanvasPainter {
    fn paint(
        &self,
        a: &NameplateAsset,
        layer: u8,
        state: u16,
        p: &Placement,
        g: &LayoutResult,
    ) -> LayerImage {
        paint_layer(a, layer, state, p, g)
    }
}

pub fn paint_layer(
    asset: &NameplateAsset,
    layer: u8,
    state: u16,
    placement: &Placement,
    _layout: &LayoutResult,
) -> LayerImage {
    let index = usize::from(layer);
    let Some(spec) = asset.layers.get(index) else {
        return LayerImage::new(Vec::new(), Vec::new());
    };
    let masks = mask_states(asset, spec);
    let own = u32::from(state) / masks;
    let mask = u32::from(state) % masks;
    let mut canvas = own_canvas(asset, index, narrow_state(own), placement);
    if let Some(Mask::Layer { slot, fill }) = spec.mask {
        let clip = Clip {
            slot,
            fill,
            state: narrow_state(mask),
        };
        apply_mask(&mut canvas, asset, clip, placement);
    }
    canvas.image()
}

pub fn build_rects(
    width: usize,
    height: usize,
    origin: (i32, i32),
    pixels: &[Option<ColorRef>],
) -> Vec<PaintRect> {
    let mut rects = Vec::new();
    merge_runs(
        height,
        |y, out| {
            let row = |x: usize| pixels.get(y * width + x).copied().flatten();
            let mut x = 0;
            while x < width {
                let Some(color) = row(x) else {
                    x += 1;
                    continue;
                };
                let start = x;
                while x < width && row(x) == Some(color) {
                    x += 1;
                }
                out.push(Run::new(start, x, y, color));
            }
        },
        |open, current, _| open.x1 == current.x1 && open.value == current.value,
        |run, bottom| {
            rects.push(PaintRect {
                x0: narrow(origin.0 + run.x0),
                y0: narrow(origin.1 + run.y0),
                x1: narrow(origin.0 + run.x1),
                y1: narrow(origin.1 + bottom),
                color: run.value,
            })
        },
    );
    rects
}

pub fn layer_states(asset: &NameplateAsset, layer: u8) -> u16 {
    let Some(spec) = asset.layers.get(usize::from(layer)) else {
        return 0;
    };
    narrow_state(own_states(asset, usize::from(layer)).saturating_mul(mask_states(asset, spec)))
}

pub fn mask_state(asset: &NameplateAsset, layer: u8, own: u16, mask: u16) -> u16 {
    let Some(spec) = asset.layers.get(usize::from(layer)) else {
        return own;
    };
    let masks = mask_states(asset, spec);
    if masks <= 1 {
        return own;
    }
    narrow_state(
        u32::from(own)
            .saturating_mul(masks)
            .saturating_add(u32::from(mask) % masks),
    )
}

pub fn emitter_state(emitter: &Emitter, sprites: &[Sprite], pick: usize, frame: u16) -> u16 {
    let before: u32 = emitter
        .sprites
        .iter()
        .take(pick)
        .map(|id| frame_count(sprites, *id))
        .sum();
    narrow_state(before.saturating_add(u32::from(frame)))
}

pub fn gradient_tone(row: i32, rows: i32, tones: i32) -> u8 {
    if rows <= 1 || tones <= 1 {
        return 0;
    }
    let row = i64::from(row.clamp(0, rows - 1));
    let (rows, tones) = (i64::from(rows), i64::from(tones));
    let tone = (row * (tones - 1) * 2 + (rows - 1)) / (2 * (rows - 1));
    u8::try_from(tone).unwrap_or(u8::MAX)
}

pub fn placement_inputs(asset: &NameplateAsset, layer: u8, placement: &Placement) -> u64 {
    let mut inputs = Inputs::new();
    let index = usize::from(layer);
    let Some(spec) = asset.layers.get(index) else {
        return inputs.0;
    };
    own_inputs(&mut inputs, asset, index, placement, true);
    if let Some(Mask::Layer { slot, fill }) = spec.mask {
        match mask_target(asset, slot, placement) {
            None => inputs.push(-1),
            Some(target) => {
                inputs.push(i64::try_from(target).unwrap_or(i64::MAX));
                own_inputs(&mut inputs, asset, target, placement, !fill);
            }
        }
    }
    inputs.0
}

struct Inputs(u64);

impl Inputs {
    fn new() -> Inputs {
        Inputs(FNV_OFFSET)
    }

    fn push(&mut self, value: i64) {
        for byte in value.to_le_bytes() {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }
}

fn own_inputs(
    inputs: &mut Inputs,
    asset: &NameplateAsset,
    index: usize,
    placement: &Placement,
    colors: bool,
) {
    let Some(layer) = asset.layers.get(index) else {
        inputs.push(0);
        return;
    };
    match &layer.content {
        LayerContent::Body(body) => {
            let rect = body_rect(body, layer.of, &placement.block);
            inputs.push(1);
            push_rect(inputs, rect);
            if colors && reads_flag(body) {
                let flag = active_flag(asset, placement)
                    .and_then(|(i, _)| i64::try_from(i).ok())
                    .unwrap_or(-1);
                inputs.push(flag);
            }
        }
        LayerContent::Sprite(sprite) => {
            let block = &placement.block;
            inputs.push(2);
            inputs.push(i64::from(chosen_sprite(layer, sprite, block.name_width)));
            push_rect(inputs, sprite_rest(asset, index, sprite, block, 0));
        }
        LayerContent::Emitter(_) => inputs.push(3),
        LayerContent::Material(_) => inputs.push(4),
    }
}

fn push_rect(inputs: &mut Inputs, rect: Rect) {
    for value in [rect.x0, rect.y0, rect.x1, rect.y1] {
        inputs.push(i64::from(value));
    }
}

fn narrow(value: i32) -> i16 {
    i16::try_from(value.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or_default()
}

fn narrow_state(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Empty,
    Ref(ColorRef),
    Sweep {
        slot: u8,
        axis: Axis,
        tone: u8,
        tones: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Band {
    slot: u8,
    axis: Axis,
    phase: u8,
    tones: u8,
}

#[derive(Clone, Copy, Debug)]
struct Run<T> {
    x0: i32,
    x1: i32,
    y0: i32,
    value: T,
}

impl<T> Run<T> {
    fn new(x0: usize, x1: usize, y: usize, value: T) -> Run<T> {
        Run {
            x0: i32::try_from(x0).unwrap_or(i32::MAX),
            x1: i32::try_from(x1).unwrap_or(i32::MAX),
            y0: i32::try_from(y).unwrap_or(i32::MAX),
            value,
        }
    }
}

fn merge_runs<T: Copy>(
    height: usize,
    mut scan: impl FnMut(usize, &mut Vec<Run<T>>),
    continues: impl Fn(&Run<T>, &Run<T>, i32) -> bool,
    mut close: impl FnMut(&Run<T>, i32),
) {
    let mut open: Vec<Run<T>> = Vec::new();
    let mut current: Vec<Run<T>> = Vec::new();
    for y in 0..=height {
        current.clear();
        if y < height {
            scan(y, &mut current);
        }
        let row = i32::try_from(y).unwrap_or(i32::MAX);
        let mut o = 0;
        for run in current.iter_mut() {
            while o < open.len() && open[o].x0 < run.x0 {
                close(&open[o], row);
                o += 1;
            }
            if o < open.len() && open[o].x0 == run.x0 && continues(&open[o], run, row) {
                run.y0 = open[o].y0;
                run.value = open[o].value;
                o += 1;
            }
        }
        while o < open.len() {
            close(&open[o], row);
            o += 1;
        }
        mem::swap(&mut open, &mut current);
    }
}

fn build_strips(width: usize, height: usize, origin: (i32, i32), cells: &[Cell]) -> Vec<StripRun> {
    let mut strips = Vec::new();
    merge_runs(
        height,
        |y, out| {
            let row = |x: usize| cells.get(y * width + x).copied().unwrap_or(Cell::Empty);
            let mut x = 0;
            while x < width {
                let Cell::Sweep {
                    slot,
                    axis,
                    tone,
                    tones,
                } = row(x)
                else {
                    x += 1;
                    continue;
                };
                let start = x;
                let mut last = tone;
                x += 1;
                while x < width {
                    let next = match axis {
                        Axis::X => (last + 1) % tones.max(1),
                        Axis::Y => last,
                    };
                    match row(x) {
                        Cell::Sweep {
                            slot: s,
                            axis: a,
                            tone: t,
                            tones: n,
                        } if s == slot && a == axis && n == tones && t == next => {
                            last = t;
                            x += 1;
                        }
                        _ => break,
                    }
                }
                let band = Band {
                    slot,
                    axis,
                    phase: tone,
                    tones,
                };
                out.push(Run::new(start, x, y, band));
            }
        },
        |open, current, y| {
            let (a, b) = (open.value, current.value);
            if open.x1 != current.x1 || a.slot != b.slot || a.axis != b.axis || a.tones != b.tones {
                return false;
            }
            match a.axis {
                Axis::X => a.phase == b.phase,
                Axis::Y => {
                    let tones = i32::from(a.tones.max(1));
                    (i32::from(a.phase) + y - open.y0).rem_euclid(tones) == i32::from(b.phase)
                }
            }
        },
        |run, bottom| {
            strips.push(StripRun {
                y0: narrow(origin.1 + run.y0),
                y1: narrow(origin.1 + bottom),
                x0: narrow(origin.0 + run.x0),
                x1: narrow(origin.0 + run.x1),
                slot: run.value.slot,
                axis: run.value.axis,
                phase: i16::from(run.value.phase),
            })
        },
    );
    strips
}

struct Canvas {
    area: Rect,
    cells: Vec<Cell>,
}

impl Canvas {
    fn new(area: Rect) -> Canvas {
        let area = if area.is_empty() { Rect::EMPTY } else { area };
        let width = usize::try_from(area.width()).unwrap_or(0);
        let height = usize::try_from(area.height()).unwrap_or(0);
        Canvas {
            area,
            cells: vec![Cell::Empty; width * height],
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if !self.area.contains(x, y) {
            return None;
        }
        usize::try_from((y - self.area.y0) * self.area.width() + (x - self.area.x0)).ok()
    }

    fn get(&self, x: i32, y: i32) -> Cell {
        self.index(x, y)
            .and_then(|i| self.cells.get(i).copied())
            .unwrap_or(Cell::Empty)
    }

    fn set(&mut self, x: i32, y: i32, cell: Cell) {
        if let Some(slot) = self.index(x, y).and_then(|i| self.cells.get_mut(i)) {
            *slot = cell;
        }
    }

    fn clear(&mut self) {
        self.cells.fill(Cell::Empty);
    }

    fn image(&self) -> LayerImage {
        let width = usize::try_from(self.area.width()).unwrap_or(0);
        let height = usize::try_from(self.area.height()).unwrap_or(0);
        let origin = (self.area.x0, self.area.y0);
        let refs: Vec<Option<ColorRef>> = self
            .cells
            .iter()
            .map(|c| match c {
                Cell::Ref(color) => Some(*color),
                _ => None,
            })
            .collect();
        let rects = build_rects(width, height, origin, &refs);
        let strips = build_strips(width, height, origin, &self.cells);
        LayerImage::new(rects, strips)
    }
}

enum Brush<'a> {
    Keep,
    Clear,
    Ref(ColorRef),
    Gradient { slot: u8, tones: i32 },
    Blend { slot: u8 },
    Sweep { slot: u8, axis: Axis, tones: u8 },
    Flag { flag: &'a Flag, fitted: Vec<u16> },
}

impl Brush<'_> {
    fn of<'a>(asset: &NameplateAsset, fill: Fill, area: Rect, flag: Option<&'a Flag>) -> Brush<'a> {
        match fill {
            Fill::Color(color) => Brush::Ref(color),
            Fill::Gradient { slot } => Brush::Gradient {
                slot,
                tones: i32::from(slot_tones(asset, slot)),
            },
            Fill::Blend { slot } => Brush::Blend { slot },
            Fill::Sweep { slot, axis } => Brush::Sweep {
                slot,
                axis,
                tones: slot_tones(asset, slot),
            },
            Fill::Flag => match flag {
                Some(flag) => {
                    let along = span(flag, clamp_u16(area.width()), clamp_u16(area.height()));
                    let fitted = band_rows(flag, along).unwrap_or_else(|| {
                        let mut fallback = vec![0; flag.bands.len().max(1)];
                        fallback[0] = along;
                        fallback
                    });
                    Brush::Flag { flag, fitted }
                }
                None => Brush::Keep,
            },
        }
    }

    fn cell(&self, area: Rect, x: i32, y: i32) -> Option<Cell> {
        let (column, row) = (x - area.x0, y - area.y0);
        match self {
            Brush::Keep => None,
            Brush::Clear => Some(Cell::Empty),
            Brush::Ref(color) => Some(Cell::Ref(*color)),
            Brush::Gradient { slot, tones } => Some(Cell::Ref(ColorRef::slot(
                *slot,
                gradient_tone(row, area.height(), *tones),
            ))),
            Brush::Blend { slot } => {
                let packed = u16::try_from(row)
                    .ok()
                    .zip(u16::try_from(area.height()).ok())
                    .and_then(|(row, rows)| blend_ref(*slot, row, rows));
                Some(Cell::Ref(packed.unwrap_or(ColorRef::slot(*slot, 0))))
            }
            Brush::Sweep { slot, axis, tones } => {
                let offset = match axis {
                    Axis::X => column,
                    Axis::Y => row,
                };
                let tone = offset.rem_euclid(i32::from((*tones).max(1)));
                Some(Cell::Sweep {
                    slot: *slot,
                    axis: *axis,
                    tone: u8::try_from(tone).unwrap_or(0),
                    tones: *tones,
                })
            }
            Brush::Flag { flag, fitted } => Some(Cell::Ref(color_at(
                flag,
                fitted,
                column,
                row,
                area.width(),
                area.height(),
            ))),
        }
    }
}

fn clamp_u16(value: i32) -> u16 {
    u16::try_from(value.max(0)).unwrap_or(u16::MAX)
}

fn slot_tones(asset: &NameplateAsset, slot: u8) -> u8 {
    let tones = asset
        .palette
        .tone_count(usize::from(slot), None)
        .clamp(1, MAX_TONES);
    u8::try_from(tones).unwrap_or(1)
}

fn fill_rounded(canvas: &mut Canvas, area: Rect, corners: [u8; 4], brush: &Brush<'_>) {
    if area.is_empty() {
        return;
    }
    let fitted = scaled_corners(corners, area);
    for y in area.y0..area.y1 {
        let (left, right) = row_insets(fitted, area, y);
        for x in area.x0 + left..area.x1 - right {
            if let Some(cell) = brush.cell(area, x, y) {
                canvas.set(x, y, cell);
            }
        }
    }
}

fn body_canvas(asset: &NameplateAsset, body: &Body, local: Rect, flag: Option<&Flag>) -> Canvas {
    if local.is_empty() {
        return Canvas::new(Rect::EMPTY);
    }
    let rings = i32::try_from(body.rings.len()).unwrap_or(0);
    let mut canvas = Canvas::new(local.grow(rings, rings, rings, rings));
    let fitted = scaled_corners(body.corners, local);
    for (i, ring) in body.rings.iter().enumerate().rev() {
        let grow = i32::try_from(i + 1).unwrap_or(i32::MAX);
        let area = local.grow(grow, grow, grow, grow);
        let step = u8::try_from(i + 1).unwrap_or(u8::MAX);
        let corners = fitted.map(|c| c.saturating_add(step));
        let brush = Brush::of(asset, *ring, area, flag);
        fill_rounded(&mut canvas, area, corners, &brush);
    }
    let fill = body
        .fill
        .map_or(Brush::Clear, |f| Brush::of(asset, f, local, flag));
    fill_rounded(&mut canvas, local, body.corners, &fill);
    if let Some(art) = &body.art {
        paint_art(&mut canvas, asset, art, local);
    }
    canvas
}

fn fill_coverage(asset: &NameplateAsset, body: &Body, local: Rect) -> Canvas {
    if local.is_empty() {
        return Canvas::new(Rect::EMPTY);
    }
    let mut canvas = Canvas::new(local);
    if body.fill.is_some() {
        let solid = Brush::Ref(ColorRef::default());
        fill_rounded(&mut canvas, local, body.corners, &solid);
    }
    if let Some(art) = &body.art {
        paint_art(&mut canvas, asset, art, local);
    }
    canvas
}

fn sprite_cell(sprite: &Sprite, frame: usize, column: i32, row: i32) -> Option<ColorRef> {
    let column = usize::try_from(column).ok()?;
    let row = usize::try_from(row).ok()?;
    if column >= usize::from(sprite.width) || row >= usize::from(sprite.height) {
        return None;
    }
    sprite.pixel(frame, column, row)
}

fn art_span(pair: Option<[u8; 2]>) -> (i32, i32) {
    pair.map_or((0, 0), |[from, to]| {
        (i32::from(from), i32::from(to) - i32::from(from))
    })
}

fn paint_art(canvas: &mut Canvas, asset: &NameplateAsset, art: &BodyArt, local: Rect) {
    let (width, height) = (local.width(), local.height());
    let Some(sprite) = asset.sprites.get(usize::from(art.sprite)) else {
        return;
    };
    let sprite_width = i32::from(sprite.width);
    let sprite_height = i32::from(sprite.height);
    let left = i32::from(art.left);
    let right = if art.mirror {
        left
    } else {
        i32::from(art.right)
    };
    let middle_end = width - right;
    let (segment_start, segment) = art_span(art.segment);
    let (center_start, center) = art_span(art.center);
    let center_x = left + (middle_end - left - center).div_euclid(2);
    let top = i32::from(art.top);
    let bottom = i32::from(art.bottom);
    let middle = (sprite_height - top - bottom).max(1);
    let mut put = |column: i32, row: i32, x: i32, y: i32| {
        if let Some(color) = sprite_cell(sprite, 0, column, row) {
            canvas.set(local.x0 + x, local.y0 + y, Cell::Ref(color));
        }
    };
    for y in 0..height {
        let row = if y < top {
            y
        } else if y >= height - bottom {
            sprite_height - (height - y)
        } else {
            top + (y - top) % middle
        };
        if segment > 0 {
            for x in left..middle_end {
                put(segment_start + (x - left) % segment, row, x, y);
            }
        }
        for k in 0..center {
            put(center_start + k, row, center_x + k, y);
        }
        for x in 0..left.min(width) {
            put(x, row, x, y);
        }
        for k in 0..right {
            let column = if art.mirror {
                right - 1 - k
            } else {
                sprite_width - right + k
            };
            put(column, row, middle_end + k, y);
        }
    }
}

fn sprite_canvas(sprite: &Sprite, state: u16, origin: (i32, i32)) -> Canvas {
    let frames = sprite.frames.len().max(1);
    let frame = usize::from(state) % frames;
    let (width, height) = (i32::from(sprite.width), i32::from(sprite.height));
    let area = Rect::new(0, 0, width, height).offset(origin.0, origin.1);
    let mut canvas = Canvas::new(area);
    for y in 0..height {
        for x in 0..width {
            if let Some(color) = sprite_cell(sprite, frame, x, y) {
                canvas.set(origin.0 + x, origin.1 + y, Cell::Ref(color));
            }
        }
    }
    canvas
}

fn frame_count(sprites: &[Sprite], id: u16) -> u32 {
    sprites
        .get(usize::from(id))
        .map_or(0, |s| u32::try_from(s.frames.len()).unwrap_or(u32::MAX))
}

fn emitter_pick(asset: &NameplateAsset, emitter: &Emitter, state: u16) -> Option<(u16, u16)> {
    let total: u32 = emitter
        .sprites
        .iter()
        .map(|id| frame_count(&asset.sprites, *id))
        .sum();
    if total == 0 {
        return None;
    }
    let mut rest = u32::from(state) % total;
    for id in &emitter.sprites {
        let frames = frame_count(&asset.sprites, *id);
        if rest < frames {
            return Some((*id, narrow_state(rest)));
        }
        rest -= frames;
    }
    None
}

fn variant_of(asset: &NameplateAsset, index: u16) -> Option<&Variant> {
    asset.variant(index).or_else(|| asset.variants.first())
}

fn active_flag<'a>(asset: &'a NameplateAsset, placement: &Placement) -> Option<(usize, &'a Flag)> {
    let index = variant_of(asset, placement.variant)
        .and_then(|v| v.flag)
        .map(usize::from)
        .filter(|i| *i < asset.flags.len())
        .unwrap_or(0);
    asset.flags.get(index).map(|flag| (index, flag))
}

fn reads_flag(body: &Body) -> bool {
    body.fill == Some(Fill::Flag) || body.rings.contains(&Fill::Flag)
}

fn chosen_sprite(layer: &Layer, sprite: &SpriteLayer, name_width: i32) -> u16 {
    layer
        .breakpoints
        .iter()
        .find(|b| b.contains(name_width))
        .and_then(|b| b.sprite)
        .unwrap_or(sprite.sprite)
}

fn own_states(asset: &NameplateAsset, index: usize) -> u32 {
    let Some(layer) = asset.layers.get(index) else {
        return 0;
    };
    match &layer.content {
        LayerContent::Body(_) => 1,
        LayerContent::Sprite(sprite) => layer
            .breakpoints
            .iter()
            .filter_map(|b| b.sprite)
            .chain([sprite.sprite])
            .map(|id| frame_count(&asset.sprites, id))
            .max()
            .unwrap_or(0)
            .max(1),
        LayerContent::Emitter(emitter) => emitter
            .sprites
            .iter()
            .map(|id| frame_count(&asset.sprites, *id))
            .sum::<u32>()
            .max(1),
        LayerContent::Material(_) => 0,
    }
}

fn mask_states(asset: &NameplateAsset, layer: &Layer) -> u32 {
    let Some(Mask::Layer { slot, fill: false }) = layer.mask else {
        return 1;
    };
    let own = usize::from(slot);
    asset
        .variants
        .iter()
        .filter_map(|v| v.slots.get(own).copied().flatten())
        .map(usize::from)
        .chain([own])
        .map(|index| own_states(asset, index))
        .max()
        .unwrap_or(1)
        .max(1)
}

fn mask_target(asset: &NameplateAsset, slot: u8, placement: &Placement) -> Option<usize> {
    let resolved = match variant_of(asset, placement.variant) {
        Some(variant) => variant
            .slots
            .get(usize::from(slot))
            .copied()
            .unwrap_or(Some(slot)),
        None => Some(slot),
    };
    resolved
        .map(usize::from)
        .filter(|i| *i < asset.layers.len())
}

fn own_canvas(asset: &NameplateAsset, index: usize, state: u16, placement: &Placement) -> Canvas {
    let Some(layer) = asset.layers.get(index) else {
        return Canvas::new(Rect::EMPTY);
    };
    match &layer.content {
        LayerContent::Body(body) => {
            let rect = body_rect(body, layer.of, &placement.block);
            let flag = if reads_flag(body) {
                active_flag(asset, placement).map(|(_, f)| f)
            } else {
                None
            };
            body_canvas(asset, body, rect, flag)
        }
        LayerContent::Sprite(sprite) => {
            let block = &placement.block;
            let id = chosen_sprite(layer, sprite, block.name_width);
            let rest = sprite_rest(asset, index, sprite, block, 0);
            asset.sprites.get(usize::from(id)).map_or_else(
                || Canvas::new(Rect::EMPTY),
                |s| sprite_canvas(s, state, (rest.x0, rest.y0)),
            )
        }
        LayerContent::Emitter(emitter) => emitter_pick(asset, emitter, state)
            .and_then(|(id, frame)| {
                asset
                    .sprites
                    .get(usize::from(id))
                    .map(|s| sprite_canvas(s, frame, (0, 0)))
            })
            .unwrap_or_else(|| Canvas::new(Rect::EMPTY)),
        LayerContent::Material(_) => Canvas::new(Rect::EMPTY),
    }
}

#[derive(Clone, Copy)]
struct Clip {
    slot: u8,
    fill: bool,
    state: u16,
}

fn apply_mask(canvas: &mut Canvas, asset: &NameplateAsset, clip: Clip, placement: &Placement) {
    let block = &placement.block;
    let Some(target) = mask_target(asset, clip.slot, placement) else {
        canvas.clear();
        return;
    };
    let mask = match &asset.layers[target] {
        Layer {
            content: LayerContent::Body(body),
            of,
            ..
        } if clip.fill => fill_coverage(asset, body, body_rect(body, *of, block)),
        _ => own_canvas(asset, target, clip.state, placement),
    };
    let area = canvas.area;
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            if canvas.get(x, y) != Cell::Empty && mask.get(x, y) == Cell::Empty {
                canvas.set(x, y, Cell::Empty);
            }
        }
    }
}

fn of_rect(asset: &NameplateAsset, of: Of, block: &Block, depth: usize) -> Rect {
    match of {
        Of::Block => block.bounds(),
        Of::Name => block.name_rect(),
        Of::Layer(target) => {
            let index = usize::from(target);
            match asset.layers.get(index) {
                Some(Layer {
                    content: LayerContent::Body(body),
                    of,
                    ..
                }) => body_rect(body, *of, block),
                Some(Layer {
                    content: LayerContent::Sprite(sprite),
                    ..
                }) if depth < MAX_LAYERS => sprite_rest(asset, index, sprite, block, depth + 1),
                _ => block.bounds(),
            }
        }
    }
}

fn body_of(asset: &NameplateAsset, of: Of) -> Option<&Body> {
    match of {
        Of::Layer(target) => match asset.layers.get(usize::from(target)).map(|l| &l.content) {
            Some(LayerContent::Body(body)) => Some(body),
            _ => None,
        },
        Of::Block | Of::Name => None,
    }
}

fn sprite_rest(
    asset: &NameplateAsset,
    index: usize,
    sprite: &SpriteLayer,
    block: &Block,
    depth: usize,
) -> Rect {
    let layer = &asset.layers[index];
    let id = chosen_sprite(layer, sprite, block.name_width);
    let (width, height) = asset
        .sprites
        .get(usize::from(id))
        .map_or((0, 0), |s| (i32::from(s.width), i32::from(s.height)));
    let target = of_rect(asset, layer.of, block, depth);
    let (ax, ay) = anchor_point(sprite.anchor, sprite.at, false, target, block);
    let (x, y) = match sprite.follow {
        Some(follow) => {
            let key = RouteKey::new(
                u8::try_from(index).unwrap_or(u8::MAX),
                body_of(asset, layer.of),
                target,
                Some(follow),
                None,
                [clamp_u16(width), clamp_u16(height)],
            );
            let route = build(&key);
            route.at(i64::try_from(nearest_step(&route, ax, ay)).unwrap_or(0))
        }
        None => {
            let (px, py) = pivot_offset(sprite.pivot, width, height);
            (ax - px, ay - py)
        }
    };
    let [dx, dy] = sprite.offset.map(i32::from);
    Rect::new(x + dx, y + dy, x + dx + width, y + dy + height)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use super::super::flag::{band_rows, color_at};
    use super::super::frame::tests::{compile_files, kit};
    use super::super::geom::{Block, BlockInput, Rect};
    use super::super::layout::{build_block, reference_blocks, resolve};
    use super::super::palette::{base_frame, resolve_ref};
    use super::super::types::PaletteFrame;
    use super::*;

    const A: ColorRef = ColorRef::slot(0, 0);
    const B: ColorRef = ColorRef::slot(0, 1);

    fn rects_of(rows: &[&str]) -> Vec<PaintRect> {
        let pixels: Vec<_> = rows.concat().chars().map(|c| [None, Some(A), Some(B)][".ab".find(c).unwrap()]).collect();
        build_rects(rows[0].len(), rows.len(), (0, 0), &pixels)
    }

    fn rect(x0: i16, y0: i16, x1: i16, y1: i16, color: ColorRef) -> PaintRect {
        PaintRect { x0, y0, x1, y1, color }
    }

    fn pixels(image: &LayerImage) -> BTreeMap<(i32, i32), ColorRef> {
        let mut out = BTreeMap::new();
        for r in &image.rects {
            for (x, y) in (r.x0..r.x1).flat_map(|x| (r.y0..r.y1).map(move |y| (i32::from(x), i32::from(y)))) {
                assert!(out.insert((x, y), r.color).is_none(), "rects overlap at {x},{y}");
            }
        }
        out
    }

    fn at(asset: &NameplateAsset, block: Block, variant: &str) -> Placement {
        Placement { block, variant: asset.variant_index(variant).unwrap(), breakpoint: 0 }
    }

    fn paint(asset: &NameplateAsset, layer: &str, placement: &Placement) -> LayerImage {
        let layer = asset.layers.iter().position(|l| l.id == layer).unwrap() as u8;
        CanvasPainter.paint(asset, layer, 0, placement, &resolve(asset, placement))
    }

    fn line(width: i32) -> Block {
        let input = BlockInput { line_widths: [width, 0, 0], line_count: 1, name_width: width, ..BlockInput::default() };
        build_block(&input).unwrap()
    }

    #[test]
    fn rects_merge_runs_downward_only_on_an_exact_match_and_split_by_colour() {
        let rects = rects_of(&["aaa", "aaa", "aa.", "aab"]);
        assert_eq!(rects, [rect(0, 0, 3, 2, A), rect(0, 2, 2, 4, A), rect(2, 3, 3, 4, B)]);
        assert_eq!(rects_of(&[".aa", "aa.", "aa."]), [rect(1, 0, 3, 1, A), rect(0, 1, 2, 3, A)]);
        let split = [rect(0, 0, 2, 2, A), rect(2, 0, 4, 2, B), rect(0, 2, 1, 3, A), rect(1, 2, 4, 3, B)];
        assert_eq!(rects_of(&["aabb", "aabb", "abbb"]), split);
        assert!(rects_of(&["...", "..."]).is_empty());
    }

    #[test]
    fn test_kit_layers_paint_like_the_old_engine() {
        let dome = compile_files(&kit("dome_plate"), None);
        let plate = compile_files(&kit("test_plate"), None);
        let cases = [
            (&dome, "dome", "outer_dome", 32, 1858, Rect::new(-7, -7, 103, 11)),
            (&dome, "plate", "outer_plate", 11, 1968, Rect::new(-7, -7, 103, 11)),
            (&dome, "dome", "inner_dome", 13, 1258, Rect::new(-3, -3, 99, 10)),
            (&plate, "default", "body", 20, 1452, Rect::new(-4, -3, 100, 11)),
            (&plate, "default", "gem", 11, 13, Rect::new(46, -6, 51, -1)),
        ];
        for (asset, variant, id, rects, count, bounds) in cases {
            let image = paint(asset, id, &at(asset, reference_blocks()[1], variant));
            assert_eq!((image.rects.len(), pixels(&image).len(), image.bounds), (rects, count, bounds), "{id}");
        }
        let placement = at(&dome, reference_blocks()[1], "dome");
        let variant = &dome.variants[usize::from(placement.variant)];
        let mut frame = PaletteFrame::default();
        base_frame(variant, &mut frame);
        let argb = |color: ColorRef| resolve_ref(color, &frame, &dome.fixed_colors, &variant.stops);
        let outer = pixels(&paint(&dome, "outer_dome", &placement));
        let inner = pixels(&paint(&dome, "inner_dome", &placement));
        assert_eq!((argb(outer[&(-7, 9)]), argb(outer[&(0, 9)])), (0xffe0_b04a, 0xff1e_2340));
        assert_eq!((argb(inner[&(40, -3)]), argb(inner[&(40, 9)])), (0xff6a_7fd0, 0xff34_407a));
    }

    fn plate(fill: Value) -> NameplateAsset {
        let manifest = json!({ "format": 1, "kind": "nameplate", "id": "fills", "requires": ["nameplate"], "fps": 6, "loop": 12,
            "layers": [{ "id": "plate" }], "variants": { "a": { "flag": "pride" }, "b": { "flag": "bi" } } });
        let band = |slot: &str, weight: u8| json!({ "color": slot, "weight": weight });
        let flags = json!({ "flags": [{ "id": "pride", "bands": [band("ink", 1), band("glow", 2), band("ink", 1)] },
            { "id": "bi", "bands": [band("glow", 1), band("ink", 1)] }] });
        let palettes = json!({ "slots": [{ "id": "glow", "name": "Glow", "ramp": ["#101010", "#404040", "#808080", "#C0C0C0"] },
            { "id": "ink", "name": "Ink", "ramp": ["#FFFFFF"] }] });
        let files = [("manifest.json", manifest), ("palettes.json", palettes), ("flags.json", flags),
            ("layers/plate.json", json!({ "kind": "body", "fill": fill }))];
        compile_files(&files.map(|(n, v)| (n.to_string(), v.to_string().into_bytes())).to_vec(), None)
    }

    #[test]
    fn body_fills_paint_gradients_blends_sweeps_and_flags() {
        assert_eq!([0, 5, 11, 17].map(|row| gradient_tone(row, if row > 11 { 18 } else { 12 }, 12)), [0, 5, 11, 11]);
        assert_eq!((gradient_tone(0, 1, 12), gradient_tone(3, 8, 1)), (0, 0));
        let gradient = plate(json!({ "gradient": "glow" }));
        let image = paint(&gradient, "plate", &at(&gradient, line(20), "a"));
        let (y0, rows) = (i32::from(image.bounds.y0), image.bounds.height());
        for ((_, y), color) in pixels(&image) {
            assert_eq!(color, ColorRef::slot(0, gradient_tone(y - y0, rows, 4)));
        }
        let blend = plate(json!({ "blend": "glow" }));
        let image = paint(&blend, "plate", &at(&blend, line(20), "a"));
        for (row, r) in image.rects.iter().enumerate() {
            assert_eq!(r.color, ColorRef::blend(0, row as u16, image.rects.len() as u16));
        }
        let sweep = plate(json!({ "sweep": "glow", "axis": "x" }));
        let image = paint(&sweep, "plate", &at(&sweep, line(20), "a"));
        assert!(image.rects.is_empty() && !image.strips.is_empty());
        assert_eq!(image.quads() as usize, image.strips.len());
        let flagged = plate(json!({ "flag": true }));
        for (variant, flag) in [("a", 0), ("b", 1)] {
            let image = paint(&flagged, "plate", &at(&flagged, line(20), variant));
            let (b, f) = (image.bounds, &flagged.flags[flag]);
            let fitted = band_rows(f, b.height() as u16).unwrap();
            for ((x, y), color) in pixels(&image) {
                assert_eq!(color, color_at(f, &fitted, x - b.x0, y - b.y0, b.width(), b.height()), "{variant} {x},{y}");
            }
        }
    }

    #[test]
    fn body_keys_follow_its_rect_and_nothing_else() {
        let asset = compile_files(&kit("test_plate"), None);
        let key = |placement: &Placement| placement_inputs(&asset, 0, placement);
        let base = at(&asset, line(30), "default");
        let renamed = Placement { block: Block { name_x: base.block.name_x + 5, name_width: base.block.name_width - 5, prefix_width: 5, ..base.block }, ..base };
        let moved = Placement { block: Block { background: base.block.background.offset(5, -3), ..base.block }, ..base };
        let other = Placement { breakpoint: 3, ..base };
        assert_eq!((key(&renamed), key(&other)), (key(&base), key(&base)));
        assert_ne!(key(&moved), key(&base));
        assert_ne!(key(&at(&asset, line(31), "default")), key(&base));
        assert_eq!(paint(&asset, "body", &moved).bounds, paint(&asset, "body", &base).bounds.offset(5, -3));
    }
}
