use crate::nameplate::geom::{Anchor, Edge, Rect};
use crate::nameplate::layout::pivot_offset;
use crate::nameplate::route::{Route, RouteKey};
use crate::nameplate::types::{Body, Breakpoint, Follow, Preset, Repeat};

fn edge_length(edge: Edge, rect: Rect) -> i32 {
    match edge {
        Edge::Top | Edge::Bottom => rect.width(),
        Edge::Left | Edge::Right => rect.height(),
    }
    .max(0)
}

fn edge_point(edge: Edge, rect: Rect, along: i32) -> (i32, i32) {
    match edge {
        Edge::Top => (rect.x0 + along, rect.y0),
        Edge::Bottom => (rect.x0 + along, rect.y1),
        Edge::Left => (rect.x0, rect.y0 + along),
        Edge::Right => (rect.x1, rect.y0 + along),
    }
}

pub fn copies(repeat: &Repeat, rect: Rect, art_width: i32, art_height: i32) -> u8 {
    copies_along(
        repeat,
        rect,
        edge_length(repeat.edge, rect),
        art_width,
        art_height,
    )
}

pub fn copies_along(repeat: &Repeat, rect: Rect, run: i32, art_width: i32, art_height: i32) -> u8 {
    let spacing = i32::from(repeat.spacing.max(1));
    let art = match repeat.edge {
        Edge::Top | Edge::Bottom => art_width,
        Edge::Left | Edge::Right => art_height,
    };
    let spaced = edge_length(repeat.edge, rect) / spacing;
    let count = if art < spacing || art <= 0 {
        spaced
    } else {
        spaced.max((run.max(0) + art - 1) / art)
    };
    let min = repeat.min.max(1);
    let max = repeat.max.max(min);
    u8::try_from(count.clamp(i32::from(min), i32::from(max))).unwrap_or(max)
}

pub fn positions(
    repeat: &Repeat,
    rect: Rect,
    art_width: i32,
    art_height: i32,
    pivot: Anchor,
    route: Option<&Route>,
    out: &mut Vec<(i32, i32)>,
) {
    out.clear();
    match route {
        Some(route) => {
            let length = i64::from(route.length());
            let count = i64::from(copies_along(
                repeat,
                rect,
                route.length() as i32,
                art_width,
                art_height,
            ));
            out.extend((0..count).map(|c| route.at((2 * c + 1) * length / (2 * count))));
        }
        None => {
            let length = edge_length(repeat.edge, rect);
            let count = i32::from(copies(repeat, rect, art_width, art_height));
            let (px, py) = pivot_offset(pivot, art_width, art_height);
            out.extend((0..count).map(|c| {
                let along = (2 * c + 1) * length / (2 * count);
                let (x, y) = edge_point(repeat.edge, rect, along);
                (x - px, y - py)
            }));
        }
    }
}

pub fn route_key(
    layer: u8,
    repeat: &Repeat,
    rect: Rect,
    follow: Option<Follow>,
    body: Option<&Body>,
    size: [u16; 2],
) -> Option<RouteKey> {
    let side = if repeat.edge == Edge::Bottom {
        Preset::Bottom
    } else {
        Preset::Top
    };
    follow.map(|follow| RouteKey::new(layer, body, rect, Some(follow), Some(side), size))
}

pub fn active_breakpoint(breakpoints: &[Breakpoint], name_width: i32) -> Option<usize> {
    breakpoints.iter().position(|b| b.contains(name_width))
}

pub fn shown(breakpoints: &[Breakpoint], name_width: i32) -> bool {
    breakpoints.is_empty() || active_breakpoint(breakpoints, name_width).is_some()
}

pub fn sprite_for(sprite: u16, breakpoints: &[Breakpoint], name_width: i32) -> Option<u16> {
    if breakpoints.is_empty() {
        return Some(sprite);
    }
    active_breakpoint(breakpoints, name_width).map(|i| breakpoints[i].sprite.unwrap_or(sprite))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_follow_the_spacing_sit_centred_and_breakpoints_pick_their_set() {
        let bar = Rect::new(0, 0, 40, 8);
        let repeat = |edge, spacing, min, max| Repeat { edge, spacing, min, max };
        let top = repeat(Edge::Top, 10, 1, 64);
        assert_eq!([copies(&top, bar, 4, 4), copies(&top, Rect::new(0, 0, 45, 8), 10, 4), copies(&repeat(Edge::Top, 100, 2, 64), bar, 4, 4)], [4, 5, 2]);
        assert_eq!((copies(&repeat(Edge::Top, 10, 1, 3), bar, 4, 4), copies(&repeat(Edge::Left, 2, 1, 64), bar, 30, 1)), (3, 4));
        let placed = |r: &Repeat, pivot| {
            let mut out = vec![(99, 99)];
            positions(r, bar, 4, 3, pivot, None, &mut out);
            out
        };
        assert_eq!(placed(&top, Anchor::Bottom), [(3, -3), (13, -3), (23, -3), (33, -3)]);
        assert_eq!(placed(&repeat(Edge::Left, 4, 1, 64), Anchor::Right), [(-4, 1), (-4, 5)]);
        let widths = [(0, Some(64), Some(4)), (64, Some(200), None), (300, None, Some(6))];
        let b: Vec<Breakpoint> = widths.iter().map(|&(min, max, sprite)| Breakpoint { min, max, sprite }).collect();
        assert_eq!([0, 63, 64, 250, 416].map(|w| active_breakpoint(&b, w)), [Some(0), Some(0), Some(1), None, Some(2)]);
        assert_eq!([10, 100, 250, 400].map(|w| sprite_for(1, &b, w)), [Some(4), Some(1), None, Some(6)]);
        assert!(shown(&[], 10) && !shown(&b, 250));
    }
}
