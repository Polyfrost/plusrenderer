use crate::limits::NAMEPLATE_MATERIAL_OCTAVES;

use super::ir::{Op, OpKind, Ty};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Accepts {
    Number,
    Float,
    Vec2,
    Color,
    Surface,
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SocketDefault {
    Required,
    Absent,
    Value([f64; 4], u8),
    Input(OpKind),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Socket {
    pub name: &'static str,
    pub accepts: Accepts,
    pub default: SocketDefault,
}

pub const TONE_COST: u32 = 6;

const fn socket(name: &'static str, accepts: Accepts, default: SocketDefault) -> Socket {
    Socket {
        name,
        accepts,
        default,
    }
}

const fn number(name: &'static str) -> Socket {
    socket(name, Accepts::Number, SocketDefault::Required)
}

const fn float(name: &'static str) -> Socket {
    socket(name, Accepts::Float, SocketDefault::Required)
}

const fn value(values: [f64; 4], width: u8) -> SocketDefault {
    SocketDefault::Value(values, width)
}

static NONE: [Socket; 0] = [];
static UNARY: [Socket; 1] = [number("x")];
static PAIR: [Socket; 2] = [number("a"), number("b")];
static STEP: [Socket; 2] = [number("edge"), number("x")];
static POW: [Socket; 2] = [number("x"), number("y")];
static ATAN2: [Socket; 2] = [number("y"), number("x")];
static CLAMP: [Socket; 3] = [number("x"), number("min"), number("max")];
static MIX: [Socket; 3] = [number("a"), number("b"), number("t")];
static SMOOTHSTEP: [Socket; 3] = [number("edge0"), number("edge1"), number("x")];
static LENGTH: [Socket; 1] = [number("v")];
static POLAR: [Socket; 1] = [socket("v", Accepts::Vec2, SocketDefault::Required)];
static BODY_EDGE: [Socket; 1] = [socket(
    "p",
    Accepts::Vec2,
    SocketDefault::Input(OpKind::PixelPos),
)];
static COMBINE: [Socket; 4] = [
    float("x"),
    float("y"),
    socket("z", Accepts::Float, SocketDefault::Absent),
    socket("w", Accepts::Float, SocketDefault::Absent),
];
static COMPONENT: [Socket; 1] = [number("v")];
static SELECT: [Socket; 3] = [
    float("condition"),
    socket("a", Accepts::Any, SocketDefault::Required),
    socket("b", Accepts::Any, SocketDefault::Required),
];
static POINT: [Socket; 1] = [socket("p", Accepts::Vec2, SocketDefault::Required)];
static CELL: [Socket; 1] = [socket(
    "p",
    Accepts::Vec2,
    SocketDefault::Input(OpKind::PixelCell),
)];
static GRADIENT: [Socket; 3] = [
    socket("p", Accepts::Vec2, SocketDefault::Input(OpKind::TagUv)),
    socket("from", Accepts::Vec2, value([0.0; 4], 2)),
    socket("to", Accepts::Vec2, value([1.0, 0.0, 0.0, 0.0], 2)),
];
static HSV: [Socket; 3] = [
    float("h"),
    socket("s", Accepts::Float, value([1.0, 0.0, 0.0, 0.0], 1)),
    socket("v", Accepts::Float, value([1.0, 0.0, 0.0, 0.0], 1)),
];
static QUANTIZE: [Socket; 1] = [float("level")];
static DITHER: [Socket; 2] = [
    float("level"),
    socket("p", Accepts::Vec2, SocketDefault::Input(OpKind::PixelCell)),
];
static CYCLE: [Socket; 2] = [
    float("index"),
    socket("shift", Accepts::Float, SocketDefault::Input(OpKind::Frame)),
];
static SHADE: [Socket; 2] = [
    socket("color", Accepts::Color, SocketDefault::Required),
    float("amount"),
];
static ALPHA: [Socket; 2] = [
    socket("color", Accepts::Color, SocketDefault::Required),
    float("alpha"),
];
static SAMPLE_AT: [Socket; 2] = [
    socket("offset", Accepts::Vec2, value([0.0; 4], 2)),
    socket("frameOffset", Accepts::Float, value([0.0; 4], 1)),
];
static SPRITE: [Socket; 2] = [
    socket("at", Accepts::Vec2, SocketDefault::Input(OpKind::PixelCell)),
    socket("frame", Accepts::Float, value([0.0; 4], 1)),
];
static TEXTURE: [Socket; 1] = [socket(
    "uv",
    Accepts::Vec2,
    SocketDefault::Input(OpKind::TagUv),
)];

pub fn sockets(kind: OpKind) -> &'static [Socket] {
    match kind {
        OpKind::PixelPos
        | OpKind::PixelCell
        | OpKind::TagUv
        | OpKind::AreaSize
        | OpKind::BodyRect
        | OpKind::Frame
        | OpKind::SteppedTime
        | OpKind::ScreenPos
        | OpKind::PixelScreenPos
        | OpKind::Slot
        | OpKind::Constant
        | OpKind::Param
        | OpKind::Expr
        | OpKind::Repeat
        | OpKind::RepeatIndex => &NONE,
        OpKind::BodyEdge => &BODY_EDGE,
        OpKind::Abs | OpKind::Floor | OpKind::Frac | OpKind::Sin | OpKind::Cos | OpKind::Exp => {
            &UNARY
        }
        OpKind::Add
        | OpKind::Sub
        | OpKind::Mul
        | OpKind::Div
        | OpKind::Mod
        | OpKind::Min
        | OpKind::Max
        | OpKind::Dot => &PAIR,
        OpKind::Step => &STEP,
        OpKind::Pow => &POW,
        OpKind::Atan2 => &ATAN2,
        OpKind::Clamp => &CLAMP,
        OpKind::Mix => &MIX,
        OpKind::Smoothstep => &SMOOTHSTEP,
        OpKind::Length => &LENGTH,
        OpKind::Polar => &POLAR,
        OpKind::Combine => &COMBINE,
        OpKind::Component => &COMPONENT,
        OpKind::Select => &SELECT,
        OpKind::Noise | OpKind::Fbm | OpKind::Hash => &POINT,
        OpKind::Bayer | OpKind::Checker | OpKind::Stripes => &CELL,
        OpKind::Gradient => &GRADIENT,
        OpKind::Hsv => &HSV,
        OpKind::Quantize => &QUANTIZE,
        OpKind::DitherQuantize => &DITHER,
        OpKind::Cycle => &CYCLE,
        OpKind::Lighten | OpKind::Darken => &SHADE,
        OpKind::Alpha => &ALPHA,
        OpKind::SampleAt => &SAMPLE_AT,
        OpKind::Sprite => &SPRITE,
        OpKind::BundleTexture | OpKind::VanillaTexture | OpKind::AtlasSprite => &TEXTURE,
    }
}

pub fn is_tone(op: &Op) -> bool {
    op.kind == OpKind::Slot && op.ty == Ty::Vec4
}

fn imm_count(value: f64, min: u32, max: u32) -> u32 {
    if value.is_finite() {
        (value as u32).clamp(min, max)
    } else {
        min
    }
}

pub fn cost(op: &Op) -> u32 {
    let width = u32::from(op.ty.width());
    match op.kind {
        OpKind::PixelPos
        | OpKind::PixelCell
        | OpKind::AreaSize
        | OpKind::BodyRect
        | OpKind::Frame
        | OpKind::SteppedTime
        | OpKind::Param
        | OpKind::Expr
        | OpKind::RepeatIndex
        | OpKind::Combine
        | OpKind::Component => 1,
        OpKind::TagUv | OpKind::ScreenPos | OpKind::PixelScreenPos => 3,
        OpKind::Slot if op.ty == Ty::Vec4 => TONE_COST,
        OpKind::Slot | OpKind::Constant => 0,
        OpKind::BodyEdge | OpKind::Sprite => 6,
        OpKind::Abs
        | OpKind::Floor
        | OpKind::Frac
        | OpKind::Add
        | OpKind::Sub
        | OpKind::Mul
        | OpKind::Min
        | OpKind::Max
        | OpKind::Step => width,
        OpKind::Sin | OpKind::Cos => 9 * width,
        OpKind::Exp => 14 * width,
        OpKind::Div | OpKind::Clamp | OpKind::Mix => 2 * width,
        OpKind::Mod => 3 * width,
        OpKind::Pow => 70 * width,
        OpKind::Atan2 => 40 * width,
        OpKind::Smoothstep => 5 * width,
        OpKind::Length => 5 + 2 * imm_count(op.imm[0], 1, 4),
        OpKind::Dot => 1 + 2 * imm_count(op.imm[0], 1, 4),
        OpKind::Polar => 44,
        OpKind::Select => 1 + width,
        OpKind::Noise => 52,
        OpKind::Fbm => 52 * imm_count(op.imm[3], 1, NAMEPLATE_MATERIAL_OCTAVES),
        OpKind::Hash => 10,
        OpKind::Bayer => 6,
        OpKind::Checker | OpKind::Stripes => 5,
        OpKind::Gradient | OpKind::DitherQuantize => 8,
        OpKind::Hsv => 12,
        OpKind::Quantize => 3,
        OpKind::Cycle => 4,
        OpKind::Lighten | OpKind::Darken | OpKind::Alpha => 6,
        OpKind::SampleAt | OpKind::Repeat => 2,
        OpKind::BundleTexture | OpKind::VanillaTexture | OpKind::AtlasSprite => 16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::NAMEPLATE_MATERIAL_COST;
    use crate::nameplate::material::ir::Reg;

    fn op(kind: OpKind, ty: Ty, imm: [f64; 4]) -> Op {
        Op {
            kind,
            ty,
            dst: Reg(0),
            args: [Reg::NONE; 4],
            imm,
        }
    }

    #[test]
    fn node_costs_match_the_table_and_the_exported_limits() {
        let table = "vec2 1, vec2 1, vec2 3, vec2 1, vec4 1, float 1, float 1, vec2 3, vec2 3, slot 0, vec3 0, float 1, float 1, float 6, vec2 2, float 1, \
            vec3 3, vec2 18, float 9, float 14, vec4 4, float 1, vec2 2, vec2 4, float 3, float 1, vec3 3, float 1, vec2 140, float 40, vec4 8, float 2, \
            float 5, float@3 11, float@2 5, vec2 44, vec3 1, float 1, vec4 5, float 52, float@3 156, float 10, float 6, float 5, float 5, float 8, vec4 12, \
            slot 3, slot 8, slot 4, vec4 6, vec4 6, vec4 6, vec4 2, float@4 2, float 1, slot 6, vec4 16, vec4 16, vec4 16";
        let rows: Vec<&str> = table.split(", ").collect();
        assert_eq!(rows.len(), OpKind::ALL.len());
        for (kind, row) in OpKind::ALL.into_iter().zip(rows) {
            let (ty, expected) = row.split_once(' ').unwrap();
            let (ty, n) = ty
                .split_once('@')
                .map_or((ty, 0.0), |(t, n)| (t, n.parse().unwrap()));
            let ty = [
                ("float", Ty::Float),
                ("vec2", Ty::Vec2),
                ("vec3", Ty::Vec3),
                ("vec4", Ty::Vec4),
            ]
            .iter()
            .find(|(name, _)| *name == ty)
            .map_or(Ty::Slot, |(_, t)| *t);
            let imm = if kind == OpKind::Fbm {
                [0.0, 0.0, 0.0, n]
            } else {
                [n, 0.0, 0.0, 0.0]
            };
            assert_eq!(cost(&op(kind, ty, imm)).to_string(), expected, "{kind:?}");
        }
        assert_eq!(cost(&op(OpKind::Slot, Ty::Vec4, [0.0; 4])), TONE_COST);
        let deepest = op(
            OpKind::Fbm,
            Ty::Float,
            [0.0, 0.0, 0.0, f64::from(NAMEPLATE_MATERIAL_OCTAVES)],
        );
        assert_eq!(cost(&deepest), 52 * NAMEPLATE_MATERIAL_OCTAVES);
        assert!(cost(&deepest) < NAMEPLATE_MATERIAL_COST);
    }
}
