pub const GLYPH_HEIGHT: i32 = 8;
pub const LINE_PITCH: i32 = 10;
pub const MAX_LINES: usize = 3;
pub const BADGE_WIDTH: i32 = 9;
pub const BADGE_HEIGHT: i32 = 8;
pub const BADGE_ADVANCE: i32 = 11;
pub const CANVAS: Rect = Rect::new(-192, -64, 448, 64);
pub const CANVAS_MARGIN: i32 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub const EMPTY: Rect = Rect::new(0, 0, 0, 0);

    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    pub const fn vanilla_background(width: i32, height: i32) -> Rect {
        Rect::new(-1, -1, width, height + 1)
    }

    pub const fn width(&self) -> i32 {
        self.x1 - self.x0
    }

    pub const fn height(&self) -> i32 {
        self.y1 - self.y0
    }

    pub const fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    pub fn union(&self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::new(
            self.x0.min(other.x0),
            self.y0.min(other.y0),
            self.x1.max(other.x1),
            self.y1.max(other.y1),
        )
    }

    pub fn intersect(&self, other: Rect) -> Rect {
        let r = Rect::new(
            self.x0.max(other.x0),
            self.y0.max(other.y0),
            self.x1.min(other.x1),
            self.y1.min(other.y1),
        );
        if r.is_empty() {
            Rect::EMPTY
        } else {
            r
        }
    }

    pub const fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x0 + dx, self.y0 + dy, self.x1 + dx, self.y1 + dy)
    }

    pub const fn grow(&self, left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect::new(
            self.x0 - left,
            self.y0 - top,
            self.x1 + right,
            self.y1 + bottom,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right];

    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    pub fn parse(name: &str) -> Option<Edge> {
        Edge::ALL.into_iter().find(|e| e.as_str() == name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Anchor {
    Top,
    Bottom,
    Left,
    Right,
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    NameLeft,
    NameRight,
}

impl Anchor {
    pub const ALL: [Anchor; 10] = [
        Anchor::Top,
        Anchor::Bottom,
        Anchor::Left,
        Anchor::Right,
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomLeft,
        Anchor::BottomRight,
        Anchor::NameLeft,
        Anchor::NameRight,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Anchor::Top => "top",
            Anchor::Bottom => "bottom",
            Anchor::Left => "left",
            Anchor::Right => "right",
            Anchor::TopLeft => "top_left",
            Anchor::TopRight => "top_right",
            Anchor::BottomLeft => "bottom_left",
            Anchor::BottomRight => "bottom_right",
            Anchor::NameLeft => "name_left",
            Anchor::NameRight => "name_right",
        }
    }

    pub fn parse(name: &str) -> Option<Anchor> {
        Anchor::ALL.into_iter().find(|a| a.as_str() == name)
    }

    pub fn is_name(self) -> bool {
        matches!(self, Anchor::NameLeft | Anchor::NameRight)
    }

    pub fn takes_at(self) -> bool {
        matches!(
            self,
            Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right
        )
    }

    pub fn mirrored(self) -> Anchor {
        match self {
            Anchor::Left => Anchor::Right,
            Anchor::Right => Anchor::Left,
            Anchor::TopLeft => Anchor::TopRight,
            Anchor::TopRight => Anchor::TopLeft,
            Anchor::BottomLeft => Anchor::BottomRight,
            Anchor::BottomRight => Anchor::BottomLeft,
            Anchor::NameLeft => Anchor::NameRight,
            Anchor::NameRight => Anchor::NameLeft,
            Anchor::Top => Anchor::Top,
            Anchor::Bottom => Anchor::Bottom,
        }
    }

    pub fn default_pivot(self) -> Anchor {
        match self {
            Anchor::NameLeft => Anchor::Right,
            Anchor::NameRight => Anchor::Left,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    #[default]
    Left,
    Right,
    Center,
}

impl Align {
    pub const ALL: [Align; 3] = [Align::Left, Align::Right, Align::Center];

    pub const fn value(self) -> i32 {
        self as i32
    }

    pub fn from_value(value: i32) -> Option<Align> {
        usize::try_from(value)
            .ok()
            .and_then(|i| Align::ALL.get(i).copied())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BadgeSlot {
    #[default]
    None,
    Beside {
        x: i32,
        y: i32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Block {
    pub lines: [Rect; 3],
    pub line_count: u8,
    pub name_line: u8,
    pub name_x: i32,
    pub name_width: i32,
    pub prefix_width: i32,
    pub background: Rect,
    pub badge: BadgeSlot,
}

impl Block {
    pub fn bounds(&self) -> Rect {
        let lines = self.lines[..usize::from(self.line_count).min(MAX_LINES)]
            .iter()
            .fold(Rect::EMPTY, |all, line| all.union(*line));
        match self.badge {
            BadgeSlot::None => lines,
            BadgeSlot::Beside { x, y } => {
                lines.union(Rect::new(x, y, x + BADGE_WIDTH, y + BADGE_HEIGHT))
            }
        }
    }

    pub fn name_line_rect(&self) -> Rect {
        self.lines[usize::from(self.name_line).min(MAX_LINES - 1)]
    }

    pub fn name_rect(&self) -> Rect {
        let line = self.name_line_rect();
        Rect::new(
            self.name_x,
            line.y0,
            self.name_x + self.name_width,
            line.y0 + GLYPH_HEIGHT,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockInput {
    pub display: bool,
    pub line_widths: [i32; 3],
    pub line_count: u8,
    pub name_line: u8,
    pub align: Align,
    pub name_x: i32,
    pub name_width: i32,
    pub prefix_width: i32,
    pub background: Rect,
    pub badge: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Placement {
    pub block: Block,
    pub variant: u16,
    pub breakpoint: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutResult {
    pub block: Block,
    pub bounds: Rect,
    pub name: Rect,
    pub margin: Rect,
    pub bodies: Vec<Option<Rect>>,
    pub targets: Vec<Rect>,
    pub badge: Option<Rect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_combine_and_anchors_edges_and_aligns_parse_their_names() {
        let a = Rect::new(0, 0, 10, 8);
        assert_eq!((a.union(Rect::new(-4, 2, 3, 12)), a.intersect(Rect::new(20, 20, 30, 30)), a.union(Rect::EMPTY)), (Rect::new(-4, 0, 10, 12), Rect::EMPTY, a));
        assert!(a.contains(9, 7) && !a.contains(10, 7));
        assert!(Anchor::ALL.iter().all(|&anchor| Anchor::parse(anchor.as_str()) == Some(anchor) && anchor.mirrored().mirrored() == anchor));
        assert!(Edge::ALL.iter().all(|&edge| Edge::parse(edge.as_str()) == Some(edge)));
        assert_eq!((Anchor::NameLeft.default_pivot(), Align::from_value(2), Align::from_value(3)), (Anchor::Right, Some(Align::Center), None));
    }
}
