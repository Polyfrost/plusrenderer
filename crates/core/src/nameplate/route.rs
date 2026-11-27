use super::geom::Rect;
use super::layout::{row_insets, scaled_corners};
use super::types::{Body, Follow, Preset};

pub const ROUTE_CACHE_ENTRIES: usize = 64;

const PARTS: [Facing; 8] = [
    Facing::Up,
    Facing::Right,
    Facing::Right,
    Facing::Down,
    Facing::Down,
    Facing::Left,
    Facing::Left,
    Facing::Up,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Facing {
    Right,
    Left,
    Up,
    Down,
}

impl Facing {
    pub fn opposite(self) -> Facing {
        match self {
            Facing::Right => Facing::Left,
            Facing::Left => Facing::Right,
            Facing::Up => Facing::Down,
            Facing::Down => Facing::Up,
        }
    }

    pub fn of_step(dx: i32, dy: i32) -> Option<Facing> {
        if dx == 0 && dy == 0 {
            return None;
        }
        Some(if dx.abs() >= dy.abs() {
            if dx > 0 {
                Facing::Right
            } else {
                Facing::Left
            }
        } else if dy > 0 {
            Facing::Down
        } else {
            Facing::Up
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    pub steps: Vec<(i16, i16)>,
    pub facings: Vec<Facing>,
    pub vertices: Vec<u32>,
    pub size: [u16; 2],
    pub closed: bool,
}

impl Route {
    pub fn point(x: i32, y: i32) -> Route {
        Route {
            steps: vec![(narrow(x), narrow(y))],
            facings: Vec::new(),
            vertices: vec![0],
            size: [0, 0],
            closed: false,
        }
    }

    pub fn length(&self) -> u32 {
        let steps = self.steps.len();
        let length = if self.closed {
            steps
        } else {
            steps.saturating_sub(1)
        };
        u32::try_from(length).unwrap_or(u32::MAX)
    }

    pub fn at(&self, along: i64) -> (i32, i32) {
        wrap(self.steps.len(), self.closed, along)
            .and_then(|i| self.steps.get(i))
            .map_or((0, 0), |(x, y)| (i32::from(*x), i32::from(*y)))
    }

    pub fn facing(&self, along: i64) -> Option<Facing> {
        wrap(self.facings.len(), self.closed, along).and_then(|i| self.facings.get(i).copied())
    }
}

fn wrap(len: usize, closed: bool, along: i64) -> Option<usize> {
    let len = i64::try_from(len).ok().filter(|len| *len > 0)?;
    let index = if closed {
        along.rem_euclid(len)
    } else {
        along.clamp(0, len - 1)
    };
    usize::try_from(index).ok()
}

fn narrow(value: i32) -> i16 {
    i16::try_from(value.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RouteKey {
    pub layer: u8,
    pub rect: Rect,
    pub corners: [u8; 4],
    pub rings: u8,
    pub follow: Option<Follow>,
    pub side: Preset,
    pub size: [u16; 2],
}

impl RouteKey {
    pub fn new(
        layer: u8,
        body: Option<&Body>,
        rect: Rect,
        follow: Option<Follow>,
        side: Option<Preset>,
        size: [u16; 2],
    ) -> RouteKey {
        let (corners, rings, size) = match (follow, body) {
            (None, _) => ([0; 4], 0, [0, 0]),
            (Some(_), None) => ([0; 4], 0, size),
            (Some(_), Some(body)) => (
                if body.art.is_some() {
                    [0; 4]
                } else {
                    body.corners
                },
                u8::try_from(body.rings.len()).unwrap_or(u8::MAX),
                size,
            ),
        };
        RouteKey {
            layer,
            rect,
            corners,
            rings,
            follow,
            side: side.unwrap_or(Preset::Perimeter),
            size,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RouteCache {
    routes: Vec<(RouteKey, Route, u64)>,
    tick: u64,
}

impl RouteCache {
    pub fn new() -> RouteCache {
        RouteCache::default()
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn contains(&self, key: &RouteKey) -> bool {
        self.routes.iter().any(|(k, _, _)| k == key)
    }

    pub fn route(&mut self, key: RouteKey) -> &Route {
        self.tick += 1;
        let tick = self.tick;
        let index = match self.routes.iter().position(|(k, _, _)| *k == key) {
            Some(index) => {
                self.routes[index].2 = tick;
                index
            }
            None => {
                if self.routes.len() >= ROUTE_CACHE_ENTRIES {
                    let oldest = self
                        .routes
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, (_, _, used))| *used)
                        .map(|(i, _)| i);
                    if let Some(oldest) = oldest {
                        self.routes.swap_remove(oldest);
                    }
                }
                self.routes.push((key, build(&key), tick));
                self.routes.len() - 1
            }
        };
        &self.routes[index].1
    }

    pub fn clear(&mut self) {
        self.routes.clear();
    }
}

pub fn build(key: &RouteKey) -> Route {
    match key.follow {
        None => rect_route(key.side, key.rect),
        Some(follow) => follow_route(key, follow),
    }
}

pub fn outline(
    body: Option<&Body>,
    rect: Rect,
    follow: Follow,
    side: Preset,
    size: [u16; 2],
) -> Route {
    build(&RouteKey::new(
        0,
        body,
        rect,
        Some(follow),
        Some(side),
        size,
    ))
}

pub fn preset(
    preset: Preset,
    rect: Rect,
    follow: Option<Follow>,
    body: Option<&Body>,
    size: [u16; 2],
) -> Route {
    build(&RouteKey::new(0, body, rect, follow, Some(preset), size))
}

pub fn polyline(points: &[(i32, i32)], closed: bool) -> Route {
    let Some(&(x, y)) = points.first() else {
        return Route::default();
    };
    let mut route = Route::point(x, y);
    for pair in points.windows(2) {
        segment(&mut route, pair[0], pair[1]);
        route.vertices.push(route.length());
    }
    if closed && points.len() > 1 {
        let before = route.steps.len();
        segment(&mut route, points[points.len() - 1], (x, y));
        if route.steps.len() > before {
            route.steps.pop();
            route.closed = true;
        }
    }
    route
}

pub fn nearest_step(route: &Route, x: i32, y: i32) -> usize {
    let width = i64::from(route.size[0]);
    let height = i64::from(route.size[1]);
    let mut best = (i64::MAX, i16::MAX);
    let mut nearest = 0;
    for (i, &(sx, sy)) in route.steps.iter().enumerate() {
        let dx = 2 * (i64::from(sx) - i64::from(x)) + width;
        let dy = 2 * (i64::from(sy) - i64::from(y)) + height;
        let distance = dx * dx + dy * dy;
        if distance < best.0 || (distance == best.0 && sy < best.1) {
            best = (distance, sy);
            nearest = i;
        }
    }
    nearest
}

fn segment(route: &mut Route, from: (i32, i32), to: (i32, i32)) {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let Some(facing) = Facing::of_step(dx, dy) else {
        return;
    };
    let length = dx.abs().max(dy.abs());
    for k in 1..=length {
        let x = from.0 + dx * k / length;
        let y = from.1 + dy * k / length;
        route.steps.push((narrow(x), narrow(y)));
        route.facings.push(facing);
    }
}

fn rect_route(side: Preset, rect: Rect) -> Route {
    let top_left = (rect.x0, rect.y0);
    let top_right = (rect.x1, rect.y0);
    let bottom_right = (rect.x1, rect.y1);
    let bottom_left = (rect.x0, rect.y1);
    match side {
        Preset::Top => polyline(&[top_left, top_right], false),
        Preset::Bottom => polyline(&[bottom_left, bottom_right], false),
        Preset::Perimeter => polyline(&[top_left, top_right, bottom_right, bottom_left], true),
    }
}

fn follow_route(key: &RouteKey, follow: Follow) -> Route {
    let width = i32::from(key.size[0]).max(1);
    let height = i32::from(key.size[1]).max(1);
    let mut route = match Rows::outline(key.corners, key.rings, key.rect) {
        None => Route::point(key.rect.x0, key.rect.y0),
        Some(shape) => match follow {
            Follow::Outside => Loop::new(&shape.grown(width, height)).route(key.side, 0, 0),
            Follow::Line => Loop::new(&shape).route(key.side, -width / 2, -height / 2),
            Follow::Inside => {
                let (fit, fit_width, fit_height) = shape.fit(width, height);
                Loop::new(&fit).route(key.side, (fit_width - width) / 2, (fit_height - height) / 2)
            }
        },
    };
    route.size = key.size;
    route
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rows {
    top: i32,
    lefts: Vec<i32>,
    rights: Vec<i32>,
}

impl Rows {
    fn outline(corners: [u8; 4], rings: u8, rect: Rect) -> Option<Rows> {
        let ring = i32::from(rings);
        let area = rect.grow(ring, ring, ring, ring);
        let radii = scaled_corners(
            scaled_corners(corners, rect).map(|r| r.saturating_add(rings)),
            area,
        );
        let (lefts, rights): (Vec<i32>, Vec<i32>) = (area.y0..area.y1)
            .map(|y| {
                let (left, right) = row_insets(radii, area, y);
                (area.x0 + left, area.x1 - 1 - right)
            })
            .unzip();
        Rows::valid(area.y0, &lefts, &rights)
    }

    fn valid(top: i32, lefts: &[i32], rights: &[i32]) -> Option<Rows> {
        let mut best = (0, 0);
        let mut start = 0;
        for row in 0..=lefts.len() {
            if row < lefts.len() && lefts[row] <= rights[row] {
                continue;
            }
            if row - start > best.1 {
                best = (start, row - start);
            }
            start = row + 1;
        }
        if best.1 == 0 {
            return None;
        }
        let rows = best.0..best.0 + best.1;
        Some(Rows {
            top: top + i32::try_from(best.0).ok()?,
            lefts: lefts[rows.clone()].to_vec(),
            rights: rights[rows].to_vec(),
        })
    }

    fn count(&self) -> i32 {
        i32::try_from(self.lefts.len()).unwrap_or(i32::MAX)
    }

    fn width(&self) -> i32 {
        let right = self.rights.iter().copied().max().unwrap_or(0);
        let left = self.lefts.iter().copied().min().unwrap_or(0);
        right - left + 1
    }

    fn grown(&self, width: i32, height: i32) -> Rows {
        let rows = self.count() + height - 1;
        let size = usize::try_from(rows + 2).unwrap_or(0);
        let mut lefts = vec![i32::MAX; size];
        let mut rights = vec![i32::MIN; size];
        for (row, slot) in (0..rows).zip(1usize..) {
            let left = least(&self.lefts, row - height + 1, row + 1).saturating_sub(width - 1);
            let right = most(&self.rights, row - height + 1, row + 1);
            lefts[slot] = lefts[slot].min(left - 1);
            rights[slot] = rights[slot].max(right + 1);
            for near in [slot - 1, slot + 1] {
                lefts[near] = lefts[near].min(left);
                rights[near] = rights[near].max(right);
            }
        }
        Rows {
            top: self.top - height,
            lefts,
            rights,
        }
    }

    fn eroded(&self, width: i32, height: i32) -> Option<Rows> {
        let rows = self.count() - height + 1;
        if rows <= 0 {
            return None;
        }
        let lefts: Vec<i32> = (0..rows)
            .map(|row| most(&self.lefts, row, row + height))
            .collect();
        let rights: Vec<i32> = (0..rows)
            .map(|row| least(&self.rights, row, row + height).saturating_sub(width - 1))
            .collect();
        Rows::valid(self.top, &lefts, &rights)
    }

    fn fit(&self, width: i32, height: i32) -> (Rows, i32, i32) {
        let mut fit_width = width.min(self.width());
        let mut fit_height = height.min(self.count());
        loop {
            if let Some(rows) = self.eroded(fit_width, fit_height) {
                return (rows, fit_width, fit_height);
            }
            if fit_width <= 1 && fit_height <= 1 {
                return (self.clone(), 1, 1);
            }
            fit_width = (fit_width - 1).max(1);
            fit_height = (fit_height - 1).max(1);
        }
    }
}

fn rows_in(values: &[i32], from: i32, until: i32) -> &[i32] {
    let len = values.len();
    let from = usize::try_from(from.max(0)).unwrap_or(0).min(len);
    let until = usize::try_from(until.max(0)).unwrap_or(0).min(len);
    &values[from..until.max(from)]
}

fn least(values: &[i32], from: i32, until: i32) -> i32 {
    rows_in(values, from, until)
        .iter()
        .copied()
        .min()
        .unwrap_or(i32::MAX)
}

fn most(values: &[i32], from: i32, until: i32) -> i32 {
    rows_in(values, from, until)
        .iter()
        .copied()
        .max()
        .unwrap_or(i32::MIN)
}

struct Loop {
    points: Vec<(i32, i32)>,
    top_end: usize,
    bottom_start: usize,
    bottom_end: usize,
    leftmost: i32,
    rightmost: i32,
}

impl Loop {
    fn new(rows: &Rows) -> Loop {
        let mut walk = Loop {
            points: Vec::new(),
            top_end: 0,
            bottom_start: 0,
            bottom_end: 0,
            leftmost: rows.lefts.iter().copied().min().unwrap_or(0),
            rightmost: rows.rights.iter().copied().max().unwrap_or(0),
        };
        let count = rows.lefts.len();
        if count == 0 {
            walk.add(0, rows.top);
            return walk;
        }
        walk.add(rows.lefts[0], rows.top);
        walk.walk_to(rows.rights[0], rows.top);
        walk.top_end = walk.last();
        for (row, y) in (1..count).zip(rows.top + 1..) {
            walk.down(rows.rights[row], y);
        }
        walk.bottom_start = walk.last();
        walk.walk_to(rows.lefts[count - 1], rows.top + rows.count() - 1);
        walk.bottom_end = walk.last();
        for (row, y) in (0..count - 1)
            .rev()
            .zip((rows.top..rows.top + rows.count() - 1).rev())
        {
            walk.up(rows.lefts[row], y);
        }
        walk
    }

    fn last(&self) -> usize {
        self.points.len() - 1
    }

    fn last_x(&self) -> i32 {
        self.points.last().map_or(0, |p| p.0)
    }

    fn add(&mut self, x: i32, y: i32) {
        if self.points.last() != Some(&(x, y)) {
            self.points.push((x, y));
        }
    }

    fn walk_to(&mut self, x: i32, y: i32) {
        let mut at = self.last_x();
        while at != x {
            at += (x - at).signum();
            self.add(at, y);
        }
    }

    fn down(&mut self, to: i32, y: i32) {
        if to < self.last_x() {
            self.walk_to(to + 1, y - 1);
        }
        let x = self.last_x();
        self.add(if to > x { x + 1 } else { to }, y);
        self.walk_to(to, y);
    }

    fn up(&mut self, to: i32, y: i32) {
        if to > self.last_x() {
            self.walk_to(to - 1, y + 1);
        }
        let x = self.last_x();
        self.add(if to < x { x - 1 } else { to }, y);
        self.walk_to(to, y);
    }

    fn first_at(&self, x: i32, from: usize, to: usize) -> usize {
        (from..=to).find(|i| self.points[*i].0 == x).unwrap_or(to)
    }

    fn last_at(&self, x: i32, from: usize, to: usize) -> usize {
        (from..=to)
            .rev()
            .find(|i| self.points[*i].0 == x)
            .unwrap_or(from)
    }

    fn route(&self, side: Preset, dx: i32, dy: i32) -> Route {
        let closing = self.last();
        if closing == 0 {
            let (x, y) = self.points[0];
            return Route::point(x + dx, y + dy);
        }
        let start = self.last_at(self.leftmost, self.bottom_end, closing);
        let shift = closing - start;
        let right_top = self.first_at(self.rightmost, self.top_end, self.bottom_start) + shift;
        let right_bottom = self.last_at(self.rightmost, self.top_end, self.bottom_start) + shift;
        let left_bottom = self.first_at(self.leftmost, self.bottom_end, closing) + shift;
        let marks = [
            0,
            shift,
            self.top_end + shift,
            right_top,
            right_bottom,
            self.bottom_start + shift,
            self.bottom_end + shift,
            left_bottom,
        ];
        let facings: Vec<Facing> = (0..closing).map(|step| facing_at(step, &marks)).collect();
        let at = |along: usize| {
            let (x, y) = self.points[(along + start) % closing];
            (narrow(x + dx), narrow(y + dy))
        };
        let mut route = match side {
            Preset::Perimeter => Route {
                steps: (0..closing).map(at).collect(),
                facings,
                vertices: vec![0],
                size: [0, 0],
                closed: true,
            },
            Preset::Top => Route {
                steps: (0..=right_top).map(at).collect(),
                facings: facings[..right_top].to_vec(),
                vertices: vec![0],
                size: [0, 0],
                closed: false,
            },
            Preset::Bottom => {
                let to = right_bottom.min(left_bottom);
                Route {
                    steps: (to..=left_bottom).rev().map(at).collect(),
                    facings: (to + 1..=left_bottom)
                        .rev()
                        .map(|along| facings[along - 1].opposite())
                        .collect(),
                    vertices: vec![0],
                    size: [0, 0],
                    closed: false,
                }
            }
        };
        if !route.closed {
            route.vertices.push(route.length());
        }
        route
    }
}

fn facing_at(step: usize, marks: &[usize; 8]) -> Facing {
    let part = (0..marks.len())
        .rev()
        .find(|p| step >= marks[*p])
        .unwrap_or(0);
    let first = PARTS[part];
    if part % 2 == 1 || part + 1 >= marks.len() {
        return first;
    }
    let span = marks[part + 1].saturating_sub(marks[part]);
    if (step - marks[part]) * 2 < span {
        first
    } else {
        PARTS[part + 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Facing::{Down, Left, Right, Up};

    const BOX: Rect = Rect::new(0, 0, 4, 2);

    fn points(route: &Route) -> Vec<(i32, i32)> {
        (0..i64::from(route.length()) + i64::from(!route.closed)).map(|along| route.at(along)).collect()
    }

    #[test]
    fn outlines_walk_clockwise_on_inside_and_outside_the_body() {
        let square = Body::default();
        let line = outline(Some(&square), BOX, Follow::Line, Preset::Perimeter, [1, 1]);
        assert_eq!(points(&line), [(0, 0), (1, 0), (2, 0), (3, 0), (3, 1), (2, 1), (1, 1), (0, 1)]);
        assert_eq!((line.closed, line.at(8), line.at(-1)), (true, (0, 0), (0, 1)));
        let outside = outline(Some(&square), BOX, Follow::Outside, Preset::Perimeter, [1, 1]);
        assert_eq!(points(&outside), [(-1, 0), (0, -1), (1, -1), (2, -1), (3, -1), (4, 0), (4, 1), (3, 2), (2, 2), (1, 2), (0, 2), (-1, 1)]);
        assert_eq!(outside.facings, [Up, Right, Right, Right, Right, Down, Down, Left, Left, Left, Left, Up]);
        let bottom = outline(Some(&square), BOX, Follow::Outside, Preset::Bottom, [1, 1]);
        assert_eq!((points(&bottom), bottom.facings), (vec![(-1, 1), (0, 2), (1, 2), (2, 2), (3, 2), (4, 1)], vec![Right, Right, Right, Right, Up]));
        let inside = outline(Some(&square), BOX, Follow::Inside, Preset::Perimeter, [2, 2]);
        assert_eq!((points(&inside), inside.facings), (vec![(0, 0), (1, 0), (2, 0), (1, 0)], vec![Right, Right, Left, Left]));
        let too_big = outline(Some(&square), BOX, Follow::Inside, Preset::Perimeter, [10, 10]);
        assert_eq!((too_big.length(), too_big.at(7), too_big.facing(0)), (0, (-3, -4), None));
        let rect = Rect::new(0, 0, 40, 8);
        let hat = outline(Some(&square), rect, Follow::Outside, Preset::Perimeter, [4, 3]);
        let sprites = points(&hat).into_iter().map(|(x, y)| Rect::new(x, y, x + 4, y + 3));
        sprites.for_each(|s| assert!(s.intersect(rect).is_empty() && !s.grow(1, 1, 1, 1).intersect(rect).is_empty(), "{s:?}"));
        assert_eq!(hat.at(nearest_step(&hat, 20, 0) as i64), (18, -3));
        let rounded = Body { corners: [4; 4], ..Body::default() };
        let big = outline(Some(&rounded), Rect::new(0, 0, 30, 12), Follow::Inside, Preset::Perimeter, [3, 3]);
        assert!(points(&big).iter().all(|&(x, y)| Rect::new(0, 0, 30, 12).contains(x, y) && Rect::new(0, 0, 30, 12).contains(x + 2, y + 2)));
    }

    #[test]
    fn route_keys_hold_every_input_that_changes_the_route_and_the_cache_drops_the_oldest() {
        let rounded = |radius| Body { corners: [radius; 4], ..Body::default() };
        let rect = Rect::new(0, 0, 12, 8);
        let key = |body: &Body, rect: Rect, follow, preset, size| RouteKey::new(0, Some(body), rect, follow, Some(preset), size);
        let base = key(&rounded(3), rect, Some(Follow::Outside), Preset::Top, [2, 2]);
        let variants = [
            key(&rounded(3), rect, Some(Follow::Line), Preset::Top, [2, 2]),
            key(&rounded(3), rect, Some(Follow::Inside), Preset::Top, [2, 2]),
            key(&rounded(3), rect, Some(Follow::Outside), Preset::Bottom, [2, 2]),
            key(&rounded(3), rect, Some(Follow::Outside), Preset::Top, [3, 2]),
            key(&Body::default(), rect, Some(Follow::Outside), Preset::Top, [2, 2]),
            key(&rounded(3), rect.offset(1, 0), Some(Follow::Outside), Preset::Top, [2, 2]),
            key(&rounded(3), rect, None, Preset::Top, [2, 2]),
        ];
        let mut cache = RouteCache::new();
        let first = cache.route(base).clone();
        for key in variants {
            let route = cache.route(key).clone();
            assert_eq!(route, build(&key));
            assert_ne!(route, first, "{key:?}");
        }
        assert_eq!(key(&rounded(3), rect, None, Preset::Top, [3, 4]), key(&rounded(5), rect, None, Preset::Top, [9, 9]));
        let mut cache = RouteCache::new();
        let at = |x: i32| RouteKey::new(0, None, BOX.offset(x, 0), None, None, [0, 0]);
        (0..ROUTE_CACHE_ENTRIES as i32).chain([0, 1000]).for_each(|x| _ = cache.route(at(x)));
        assert_eq!(cache.len(), ROUTE_CACHE_ENTRIES);
        assert!(cache.contains(&at(0)) && !cache.contains(&at(1)) && cache.contains(&at(1000)));
    }
}
