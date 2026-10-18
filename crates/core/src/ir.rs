//! Backend-agnostic model IR. Box UV and `mirror` are resolved at parse time. 64x32 skins unsupported.
//! Right-handed, Y up, feet at origin, front +Z, right on -X. Bedrock files negate Z and `bedrock::to_ir` converts.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    North = 0,
    South = 1,
    East = 2,
    West = 3,
    Up = 4,
    Down = 5,
}

impl Face {
    pub const ALL: [Face; 6] = [
        Face::North,
        Face::South,
        Face::East,
        Face::West,
        Face::Up,
        Face::Down,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn normal(self) -> [f32; 3] {
        match self {
            Face::North => [0.0, 0.0, -1.0],
            Face::South => [0.0, 0.0, 1.0],
            Face::East => [1.0, 0.0, 0.0],
            Face::West => [-1.0, 0.0, 0.0],
            Face::Up => [0.0, 1.0, 0.0],
            Face::Down => [0.0, -1.0, 0.0],
        }
    }

    pub fn from_name(s: &str) -> Option<Face> {
        Some(match s {
            "north" => Face::North,
            "south" => Face::South,
            "east" => Face::East,
            "west" => Face::West,
            "up" => Face::Up,
            "down" => Face::Down,
            _ => return None,
        })
    }
}

/// In texture pixels. A negative `size` flips that axis (`mirror`, vanilla's down face).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceUv {
    pub uv: [f32; 2],
    pub size: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Cube {
    /// Minimum corner, before inflate.
    pub origin: [f32; 3],
    pub size: [f32; 3],
    pub inflate: f32,
    /// Per-cube rotation (degrees, XYZ) about `pivot`, applied before the bone's.
    pub rotation: [f32; 3],
    pub pivot: [f32; 3],
    /// Indexed by `Face::index`. `None` = not drawn.
    pub faces: [Option<FaceUv>; 6],
    pub box_uv: bool,
    pub mirror: bool,
}

impl Cube {
    pub fn toggle_mirror(&mut self) {
        self.faces.swap(Face::East.index(), Face::West.index());
        for f in self.faces.iter_mut().flatten() {
            f.uv[0] += f.size[0];
            f.size[0] = -f.size[0];
        }
        self.mirror = !self.mirror;
    }

    /// Inflated min and max. Zero-depth billboards get a sliver of thickness so the depth sort can order them.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let i = self.inflate;
        let mut min = [self.origin[0] - i, self.origin[1] - i, self.origin[2] - i];
        let mut max = [
            self.origin[0] + self.size[0] + i,
            self.origin[1] + self.size[1] + i,
            self.origin[2] + self.size[2] + i,
        ];

        if self.size[2].abs() < BILLBOARD_EPSILON {
            min[2] = self.origin[2] - BILLBOARD_HALF_THICKNESS;
            max[2] = self.origin[2] + BILLBOARD_HALF_THICKNESS;
        }

        (min, max)
    }

    /// Corners in texture order (TL, TR, BR, BL), matching `corner_uv`, clockwise seen from outside.
    pub fn face_quad(&self, f: Face) -> [[f32; 3]; 4] {
        let ([x0, y0, z0], [x1, y1, z1]) = self.bounds();
        match f {
            Face::North => [[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]],
            Face::South => [[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]],
            Face::East => [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]],
            Face::West => [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]],
            Face::Up => [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
            Face::Down => [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]],
        }
    }
}

const BILLBOARD_EPSILON: f32 = 1e-4;
/// The in-game client's value.
const BILLBOARD_HALF_THICKNESS: f32 = 0.01;

/// In texture pixels. `corner` indexes `face_quad`.
pub fn corner_uv(f: FaceUv, corner: usize) -> [f32; 2] {
    let [u, v] = f.uv;
    let [w, h] = f.size;
    match corner {
        0 => [u, v],
        1 => [u + w, v],
        2 => [u + w, v + h],
        _ => [u, v + h],
    }
}

#[derive(Clone, Debug)]
pub struct Bone {
    pub name: String,
    /// Always less than this bone's index. `None` with `parent_name` set: the parent is in another model.
    pub parent: Option<usize>,
    pub parent_name: Option<String>,
    pub pivot: [f32; 3],
    /// Rest rotation (degrees, XYZ) about `pivot`.
    pub rotation: [f32; 3],
    pub cubes: Vec<Cube>,
    /// Minimum light level 0..=15. Only cosmetics glow (`cosmetic::prepare`).
    pub glow: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub identifier: String,
    pub texture_size: [f32; 2],
    /// Topologically sorted: every bone appears after its parent.
    pub bones: Vec<Bone>,
}

impl Model {
    pub fn bone(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }
}
