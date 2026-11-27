use libm::{floor, round, sqrt};

use crate::limits::NAMEPLATE_DECORATION_MARGIN;

use super::geom::{
    Align, Anchor, BadgeSlot, Block, BlockInput, LayoutResult, Placement, Rect, BADGE_ADVANCE,
    BADGE_HEIGHT, BADGE_WIDTH, CANVAS, CANVAS_MARGIN, GLYPH_HEIGHT, LINE_PITCH, MAX_LINES,
};
use super::route::{nearest_step, RouteCache, RouteKey};
use super::types::{
    BadgePlacement, Body, BodyArt, LayerContent, NameplateAsset, Of, SpriteLayer, MAX_LAYERS,
};

pub const REFERENCE_WIDTHS: [i32; 3] = [16, 96, 416];
pub const MAX_BLOCK_WIDTH: i32 = CANVAS.x1 - CANVAS_MARGIN;
pub const BADGE_ROUTE_LAYER: u8 = u8::MAX;

#[derive(Clone, Copy)]
enum Part {
    Start,
    Middle,
    End,
}

impl Part {
    fn at(self, from: i32, to: i32) -> i32 {
        match self {
            Part::Start => from,
            Part::Middle => (from + to).div_euclid(2),
            Part::End => to,
        }
    }
}

fn parts(anchor: Anchor) -> (Part, Part) {
    match anchor.default_pivot() {
        Anchor::TopLeft => (Part::Start, Part::Start),
        Anchor::Top => (Part::Middle, Part::Start),
        Anchor::TopRight => (Part::End, Part::Start),
        Anchor::Left => (Part::Start, Part::Middle),
        Anchor::Right => (Part::End, Part::Middle),
        Anchor::BottomLeft => (Part::Start, Part::End),
        Anchor::Bottom => (Part::Middle, Part::End),
        Anchor::BottomRight | Anchor::NameLeft | Anchor::NameRight => (Part::End, Part::End),
    }
}

fn doubled_offset(align: Align, widest: i32, width: i32) -> i32 {
    match align {
        Align::Left => 0,
        Align::Right => 2 * (widest - width),
        Align::Center => widest - width,
    }
}

fn ceil_half(value: i32) -> i32 {
    -(-value).div_euclid(2)
}

fn background_or_vanilla(background: Rect, width: i32, height: i32) -> Rect {
    if background.is_empty() {
        Rect::vanilla_background(width, height)
    } else {
        background
    }
}

pub fn build_block(input: &BlockInput) -> Option<Block> {
    let count = usize::from(input.line_count);
    if count == 0 || input.name_width < 0 || input.line_widths.iter().any(|w| *w < 0) {
        return None;
    }
    let tall = count > MAX_LINES;
    if !tall && input.name_line >= input.line_count {
        return None;
    }
    if !input.display {
        return if count == 1 {
            vanilla_block(input)
        } else {
            None
        };
    }

    let widths = if tall {
        &input.line_widths[..1]
    } else {
        &input.line_widths[..count]
    };
    let name = if tall {
        0
    } else {
        usize::from(input.name_line)
    };
    let widest = widths.iter().copied().max().unwrap_or(0);
    let name_offset = doubled_offset(input.align, widest, widths[name]);

    let mut placed = [None; MAX_LINES];
    let mut extent = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (index, (&width, line)) in widths.iter().zip(placed.iter_mut()).enumerate() {
        if index != name && width <= 0 {
            continue;
        }
        let relative = doubled_offset(input.align, widest, width) - name_offset;
        let y = (i32::try_from(index).ok()? - i32::try_from(name).ok()?) * LINE_PITCH;
        let rect = Rect::new(
            relative.div_euclid(2),
            y,
            ceil_half(relative + 2 * width),
            y + GLYPH_HEIGHT,
        );
        extent = (
            extent.0.min(rect.x0),
            extent.1.min(rect.y0),
            extent.2.max(rect.x1),
            extent.3.max(rect.y1),
        );
        *line = Some(rect);
    }
    if input.badge {
        extent.0 = extent.0.min(-BADGE_ADVANCE);
    }
    let (x0, y0, x1, y1) = extent;
    let (width, height) = (x1 - x0, y1 - y0);
    if width > MAX_BLOCK_WIDTH {
        return None;
    }

    let mut lines = [Rect::EMPTY; MAX_LINES];
    for (slot, line) in lines.iter_mut().zip(placed) {
        if let Some(rect) = line {
            *slot = rect.offset(-x0, -y0);
        }
    }
    let badge = if input.badge {
        BadgeSlot::Beside {
            x: -BADGE_ADVANCE - x0,
            y: -y0,
        }
    } else {
        BadgeSlot::None
    };
    Some(Block {
        lines,
        line_count: if tall { 1 } else { input.line_count },
        name_line: if tall { 0 } else { input.name_line },
        name_x: input.name_x - x0,
        name_width: input.name_width,
        prefix_width: input.prefix_width,
        background: background_or_vanilla(input.background, width, height),
        badge,
    })
}

fn vanilla_block(input: &BlockInput) -> Option<Block> {
    let width = input.line_widths[0];
    if width > MAX_BLOCK_WIDTH {
        return None;
    }
    let mut lines = [Rect::EMPTY; MAX_LINES];
    lines[0] = Rect::new(0, 0, width, GLYPH_HEIGHT);
    let badge = if input.badge {
        BadgeSlot::Beside {
            x: input.name_x - BADGE_ADVANCE,
            y: 0,
        }
    } else {
        BadgeSlot::None
    };
    Some(Block {
        lines,
        line_count: 1,
        name_line: 0,
        name_x: input.name_x,
        name_width: input.name_width,
        prefix_width: input.prefix_width,
        background: background_or_vanilla(input.background, width, GLYPH_HEIGHT),
        badge,
    })
}

pub fn body_rect(body: &Body, of: Of, block: &Block) -> Rect {
    let around = match of {
        Of::Name => block.name_rect(),
        Of::Block | Of::Layer(_) => {
            let bg = block.background;
            Rect::new(bg.x0 + 1, bg.y0 + 1, bg.x1, bg.y1 - 1)
        }
    };
    let [left, top, right, bottom] = body.padding.map(i32::from);
    let rect = around.grow(left, top, right, bottom);
    let least = i32::from(body.min_width).max(body.art.as_ref().map_or(0, art_min_width));
    let missing = least - rect.width();
    if missing <= 0 {
        return rect;
    }
    Rect::new(
        rect.x0 - missing / 2,
        rect.y0,
        rect.x1 + missing - missing / 2,
        rect.y1,
    )
}

fn art_min_width(art: &BodyArt) -> i32 {
    let right = if art.mirror { art.left } else { art.right };
    let center = art
        .center
        .map_or(0, |[start, end]| i32::from(end.saturating_sub(start)));
    i32::from(art.left) + i32::from(right) + center
}

pub fn scaled_corners(corners: [u8; 4], rect: Rect) -> [u8; 4] {
    let width = i64::from(rect.width().max(0));
    let height = i64::from(rect.height().max(0));
    let [tl, tr, br, bl] = corners.map(i64::from);
    let sides = [
        (width, tl + tr),
        (width, bl + br),
        (height, tl + bl),
        (height, tr + br),
    ];
    let mut scale = None;
    for (length, sum) in sides {
        let (num, den) = scale.unwrap_or((1, 1));
        if sum > 0 && length * den < num * sum {
            scale = Some((length, sum));
        }
    }
    match scale {
        None => corners,
        Some((num, den)) => {
            corners.map(|r| u8::try_from(i64::from(r) * num / den).unwrap_or(u8::MAX))
        }
    }
}

pub fn corner_inset(radius: u8, row: i32) -> i32 {
    let r = i32::from(radius);
    if row < 0 || row >= r {
        return 0;
    }
    let dy = f64::from(r - row) - 0.5;
    let reach = round(sqrt(f64::from(r * r) - dy * dy));
    r - reach as i32
}

pub fn row_insets(corners: [u8; 4], rect: Rect, y: i32) -> (i32, i32) {
    let [tl, tr, br, bl] = scaled_corners(corners, rect);
    let from_top = y - rect.y0;
    let from_bottom = rect.y1 - 1 - y;
    (
        corner_inset(tl, from_top).max(corner_inset(bl, from_bottom)),
        corner_inset(tr, from_top).max(corner_inset(br, from_bottom)),
    )
}

pub fn anchor_point(
    anchor: Anchor,
    at: Option<f64>,
    mirrored: bool,
    rect: Rect,
    block: &Block,
) -> (i32, i32) {
    let placed = if mirrored { anchor.mirrored() } else { anchor };
    let name_y = block.name_line_rect().y0 + GLYPH_HEIGHT / 2;
    match placed {
        Anchor::NameLeft => return (block.name_x, name_y),
        Anchor::NameRight => return (block.name_x + block.name_width, name_y),
        _ => {}
    }
    let (column, row) = parts(placed);
    let mut x = column.at(rect.x0, rect.x1);
    let mut y = row.at(rect.y0, rect.y1);
    if let Some(at) = at.filter(|_| placed.takes_at()) {
        match placed {
            Anchor::Top | Anchor::Bottom => x = along(rect.x0, rect.x1, at, mirrored),
            _ => y = along(rect.y0, rect.y1, at, false),
        }
    }
    (x, y)
}

fn along(from: i32, to: i32, at: f64, from_end: bool) -> i32 {
    let offset = floor(at * f64::from(to - from)) as i32;
    if from_end {
        to - offset
    } else {
        from + offset
    }
}

pub fn pivot_offset(pivot: Anchor, width: i32, height: i32) -> (i32, i32) {
    let (column, row) = parts(pivot);
    (column.at(0, width), row.at(0, height))
}

pub fn resolve(asset: &NameplateAsset, placement: &Placement) -> LayoutResult {
    resolve_with(asset, placement, &mut RouteCache::new())
}

pub fn resolve_with(
    asset: &NameplateAsset,
    placement: &Placement,
    routes: &mut RouteCache,
) -> LayoutResult {
    let block = placement.block;
    let margin = i32::try_from(NAMEPLATE_DECORATION_MARGIN).unwrap_or(i32::MAX);
    let bodies: Vec<Option<Rect>> = asset
        .layers
        .iter()
        .map(|layer| match &layer.content {
            LayerContent::Body(body) => Some(body_rect(body, layer.of, &block)),
            _ => None,
        })
        .collect();
    let mut resolver = Resolver {
        asset,
        block,
        bounds: block.bounds(),
        bodies: &bodies,
        routes,
    };
    let targets = (0..asset.layers.len())
        .map(|index| resolver.target(index))
        .collect();
    let badge = resolver.badge(placement.variant);
    LayoutResult {
        block,
        bounds: block.bounds(),
        name: block.name_rect(),
        margin: block.background.grow(margin, margin, margin, margin),
        bodies,
        targets,
        badge,
    }
}

struct Resolver<'a> {
    asset: &'a NameplateAsset,
    block: Block,
    bounds: Rect,
    bodies: &'a [Option<Rect>],
    routes: &'a mut RouteCache,
}

impl<'a> Resolver<'a> {
    fn target(&mut self, index: usize) -> Rect {
        match self.bodies.get(index).copied().flatten() {
            Some(body) => body,
            None => {
                let of = self.asset.layers[index].of;
                self.of_rect(of, 0)
            }
        }
    }

    fn of_rect(&mut self, of: Of, depth: usize) -> Rect {
        match of {
            Of::Block => self.bounds,
            Of::Name => self.block.name_rect(),
            Of::Layer(layer) => {
                let index = usize::from(layer);
                if let Some(body) = self.bodies.get(index).copied().flatten() {
                    return body;
                }
                let asset = self.asset;
                let sprite = asset.layers.get(index).and_then(|l| match &l.content {
                    LayerContent::Sprite(sprite) => Some(sprite),
                    _ => None,
                });
                match sprite {
                    Some(sprite) if depth < MAX_LAYERS => self.rest(index, sprite, depth + 1),
                    _ => self.bounds,
                }
            }
        }
    }

    fn body_of(&self, of: Of) -> Option<&'a Body> {
        let asset: &'a NameplateAsset = self.asset;
        match of {
            Of::Layer(layer) => match asset.layers.get(usize::from(layer)).map(|l| &l.content) {
                Some(LayerContent::Body(body)) => Some(body),
                _ => None,
            },
            Of::Block | Of::Name => None,
        }
    }

    fn rest(&mut self, index: usize, sprite: &SpriteLayer, depth: usize) -> Rect {
        let asset = self.asset;
        let layer = &asset.layers[index];
        let (width, height) = self.sprite_size(index, sprite);
        let target = self.of_rect(layer.of, depth);
        let (ax, ay) = anchor_point(sprite.anchor, sprite.at, false, target, &self.block);
        let (x, y) = match sprite.follow {
            Some(follow) => {
                let key = RouteKey::new(
                    u8::try_from(index).unwrap_or(u8::MAX),
                    self.body_of(layer.of),
                    target,
                    Some(follow),
                    None,
                    [clamp_size(width), clamp_size(height)],
                );
                let route = self.routes.route(key);
                route.at(i64::try_from(nearest_step(route, ax, ay)).unwrap_or(0))
            }
            None => {
                let (px, py) = pivot_offset(sprite.pivot, width, height);
                (ax - px, ay - py)
            }
        };
        let [dx, dy] = sprite.offset.map(i32::from);
        Rect::new(x + dx, y + dy, x + dx + width, y + dy + height)
    }

    fn sprite_size(&self, index: usize, sprite: &SpriteLayer) -> (i32, i32) {
        let chosen = self.asset.layers[index]
            .breakpoints
            .iter()
            .find(|b| b.contains(self.block.name_width))
            .and_then(|b| b.sprite)
            .unwrap_or(sprite.sprite);
        self.asset
            .sprites
            .get(usize::from(chosen))
            .map_or((0, 0), |s| (i32::from(s.width), i32::from(s.height)))
    }

    fn badge(&mut self, variant: u16) -> Option<Rect> {
        let style = self
            .asset
            .variant(variant)
            .map_or(&self.asset.badge, |v| &v.badge);
        match style.placement {
            BadgePlacement::Reserve | BadgePlacement::Carried => match self.block.badge {
                BadgeSlot::Beside { x, y } => {
                    Some(Rect::new(x, y, x + BADGE_WIDTH, y + BADGE_HEIGHT))
                }
                BadgeSlot::None => None,
            },
            BadgePlacement::Placed(placed) => {
                let rect = self.of_rect(placed.of, 0);
                let (ax, ay) = anchor_point(placed.anchor, placed.at, false, rect, &self.block);
                let (x, y) = match placed.follow {
                    Some(follow) => {
                        let key = RouteKey::new(
                            BADGE_ROUTE_LAYER,
                            self.body_of(placed.of),
                            rect,
                            Some(follow),
                            None,
                            [clamp_size(BADGE_WIDTH), clamp_size(BADGE_HEIGHT)],
                        );
                        let route = self.routes.route(key);
                        route.at(i64::try_from(nearest_step(route, ax, ay)).unwrap_or(0))
                    }
                    None => (ax - BADGE_WIDTH / 2, ay - BADGE_HEIGHT / 2),
                };
                let [dx, dy] = placed.offset.map(i32::from);
                Some(Rect::new(
                    x + dx,
                    y + dy,
                    x + dx + BADGE_WIDTH,
                    y + dy + BADGE_HEIGHT,
                ))
            }
        }
    }
}

fn clamp_size(value: i32) -> u16 {
    u16::try_from(value.max(0)).unwrap_or(u16::MAX)
}

fn line_input(width: i32, name_x: i32, name_width: i32, badge: bool) -> BlockInput {
    BlockInput {
        display: false,
        line_widths: [width, 0, 0],
        line_count: 1,
        name_line: 0,
        align: Align::Left,
        name_x,
        name_width,
        prefix_width: name_x,
        background: Rect::EMPTY,
        badge,
    }
}

fn lines_input(width: i32, count: u8, badge: bool) -> BlockInput {
    let name_line = count / 2;
    let mut line_widths = [0; MAX_LINES];
    for (line, slot) in (0..count).zip(line_widths.iter_mut()) {
        *slot = if line == name_line { width } else { width / 2 };
    }
    BlockInput {
        display: true,
        line_widths,
        line_count: count,
        name_line,
        align: Align::Center,
        name_x: 0,
        name_width: width,
        prefix_width: 0,
        background: Rect::EMPTY,
        badge,
    }
}

pub fn reference_inputs() -> Vec<BlockInput> {
    let mut inputs: Vec<BlockInput> = REFERENCE_WIDTHS
        .iter()
        .map(|width| line_input(*width, 0, *width, false))
        .collect();
    for count in [2, 3] {
        for width in [REFERENCE_WIDTHS[1], REFERENCE_WIDTHS[2]] {
            inputs.push(lines_input(width, count, false));
        }
    }
    for width in [REFERENCE_WIDTHS[0], REFERENCE_WIDTHS[1]] {
        inputs.push(line_input(
            width + BADGE_ADVANCE,
            BADGE_ADVANCE,
            width,
            true,
        ));
        inputs.push(lines_input(width, 2, true));
    }
    inputs
}

pub fn reference_blocks() -> Vec<Block> {
    reference_inputs().iter().filter_map(build_block).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::geom::{Align, BadgeSlot};

    fn display(widths: &[i32], name_line: u8, align: Align, name_x: i32, badge: bool) -> BlockInput {
        let mut line_widths = [0; 3];
        line_widths[..widths.len()].copy_from_slice(widths);
        let (line_count, name_width) = (widths.len() as u8, widths[usize::from(name_line)]);
        BlockInput { display: true, line_widths, line_count, name_line, align, name_x, name_width, prefix_width: name_x, background: Rect::EMPTY, badge }
    }

    fn vanilla(width: i32, name_x: i32, name_width: i32) -> BlockInput {
        BlockInput { line_widths: [width, 0, 0], line_count: 1, name_x, name_width, prefix_width: name_x, ..BlockInput::default() }
    }

    #[test]
    fn blocks_stack_align_and_badge_their_lines_like_vanilla() {
        let r = Rect::new;
        let center = |widths: &[i32], badge| display(widths, widths.len() as u8 - 1, Align::Center, 0, badge);
        let four = BlockInput { line_widths: [40, 30, 30], line_count: 4, name_line: 2, name_width: 30, ..display(&[40], 0, Align::Center, 3, false) };
        let cases = [
            (vanilla(37, 4, 30), vec![r(0, 0, 37, 8)], r(0, 0, 37, 8), 4, BadgeSlot::None),
            (center(&[35, 48], false), vec![r(6, 0, 42, 8), r(0, 10, 48, 18)], r(0, 0, 48, 18), 0, BadgeSlot::None),
            (center(&[35, 20], false), vec![r(0, 0, 36, 8), r(8, 10, 28, 18)], r(0, 0, 36, 18), 8, BadgeSlot::None),
            (display(&[35, 48], 1, Align::Right, 0, false), vec![r(13, 0, 48, 8), r(0, 10, 48, 18)], r(0, 0, 48, 18), 0, BadgeSlot::None),
            (center(&[41, 40], false), vec![r(0, 0, 42, 8), r(1, 10, 41, 18)], r(0, 0, 42, 18), 1, BadgeSlot::None),
            (center(&[30, 0, 40], false), vec![r(5, 0, 35, 8), Rect::EMPTY, r(0, 20, 40, 28)], r(0, 0, 40, 28), 0, BadgeSlot::None),
            (four, vec![r(0, 0, 40, 8)], r(0, 0, 40, 8), 3, BadgeSlot::None),
            (center(&[35, 48], true), vec![r(17, 0, 53, 8), r(11, 10, 59, 18)], r(0, 0, 59, 18), 11, BadgeSlot::Beside { x: 0, y: 10 }),
            (BlockInput { badge: true, ..vanilla(41, 11, 30) }, vec![r(0, 0, 41, 8)], r(0, 0, 41, 8), 11, BadgeSlot::Beside { x: 0, y: 0 }),
        ];
        for (input, lines, bounds, name_x, badge) in cases {
            let block = build_block(&input).unwrap();
            assert_eq!(block.lines[..usize::from(block.line_count)], lines[..], "{input:?}");
            assert_eq!((block.bounds(), block.name_x, block.badge), (bounds, name_x, badge), "{input:?}");
            assert_eq!(block.background, r(-1, -1, bounds.x1, bounds.y1 + 1));
        }
        let host = BlockInput { background: r(-3, -2, 40, 10), ..vanilla(37, 0, 37) };
        assert_eq!(build_block(&host).unwrap().background, r(-3, -2, 40, 10));
        assert!(build_block(&vanilla(416, 0, 416)).is_some());
        assert_eq!(build_block(&vanilla(417, 0, 417)), None);
        assert_eq!(build_block(&display(&[300, 410], 1, Align::Center, 0, true)), None);
        let mut bad = display(&[30, 40], 1, Align::Center, 0, false);
        bad.name_line = 2;
        assert_eq!(build_block(&bad), None);
        assert_eq!(build_block(&vanilla(-4, 0, 0)), None);
        assert_eq!(build_block(&BlockInput { line_count: 2, ..vanilla(30, 0, 30) }), None);
    }

    #[test]
    fn bodies_pad_grow_and_round_their_corners() {
        let block = build_block(&vanilla(40, 4, 30)).unwrap();
        let body = |padding, min_width| Body { padding, min_width, ..Body::default() };
        assert_eq!(body_rect(&body([0; 4], 0), Of::Block, &block), Rect::new(0, 0, 40, 8));
        assert_eq!(body_rect(&body([2, 1, 3, 4], 0), Of::Block, &block), Rect::new(-2, -1, 43, 12));
        assert_eq!(body_rect(&body([2, 1, 3, 4], 0), Of::Name, &block), Rect::new(2, -1, 37, 12));
        assert_eq!(body_rect(&body([0; 4], 45), Of::Block, &block), Rect::new(-2, 0, 43, 8));
        assert_eq!(body_rect(&body([0; 4], 10), Of::Block, &block), Rect::new(0, 0, 40, 8));
        for (radius, rows) in [(2, &[1, 0][..]), (5, &[3, 1, 1, 0, 0]), (16, &[12, 9, 7, 6, 5, 4, 3, 2, 2, 1, 1, 1, 0, 0, 0, 0])] {
            let insets: Vec<i32> = (0..i32::from(radius)).map(|row| corner_inset(radius, row)).collect();
            assert_eq!(insets, rows);
            assert_eq!((corner_inset(radius, i32::from(radius)), corner_inset(radius, -1)), (0, 0));
        }
        assert_eq!(scaled_corners([4, 4, 4, 4], Rect::new(0, 0, 10, 4)), [2, 2, 2, 2]);
        assert_eq!(scaled_corners([3, 5, 0, 0], Rect::new(0, 0, 6, 20)), [2, 3, 0, 0]);
        let rect = Rect::new(0, 0, 20, 16);
        let rows: Vec<_> = [0, 1, 8, 14, 15].map(|y| row_insets([4, 0, 2, 8], rect, y)).to_vec();
        assert_eq!(rows, [(2, 0), (1, 0), (0, 0), (3, 0), (5, 1)]);
    }

    #[test]
    fn anchors_floor_their_middles_and_mirror_across_the_rect() {
        let block = build_block(&display(&[25, 41], 1, Align::Center, 3, false)).unwrap();
        let rect = Rect::new(0, 0, 41, 18);
        let cases = [
            (Anchor::Top, None, false, (20, 0)),
            (Anchor::Left, None, false, (0, 9)),
            (Anchor::TopLeft, None, true, (41, 0)),
            (Anchor::NameLeft, None, false, (3, 14)),
            (Anchor::NameRight, None, true, (3, 14)),
            (Anchor::Top, Some(0.25), true, (31, 0)),
            (Anchor::Left, Some(0.3), true, (41, 5)),
        ];
        for (anchor, along, mirrored, point) in cases {
            assert_eq!(anchor_point(anchor, along, mirrored, rect, &block), point, "{anchor:?} {along:?} {mirrored}");
        }
        assert_eq!(anchor_point(Anchor::Top, None, false, Rect::new(-5, -3, 0, 0), &block), (-3, -3));
        let pivots = [Anchor::TopLeft, Anchor::Top, Anchor::BottomRight, Anchor::NameLeft].map(|a| pivot_offset(a, 7, 5));
        assert_eq!(pivots, [(0, 0), (3, 0), (7, 5), (7, 2)]);
    }
}
