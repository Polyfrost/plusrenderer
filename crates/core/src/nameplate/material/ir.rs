use std::ops::Range;

use crate::nameplate::geom::Rect;
use crate::nameplate::image::ColorRef;
use crate::nameplate::types::Sprite;

pub const UNIFORM_AREA: u32 = 0;
pub const UNIFORM_BODY: u32 = 4;
pub const UNIFORM_CLOCK: u32 = 8;
pub const UNIFORM_LANES: u32 = 12;
pub const UNIFORM_ATLAS: u32 = 28;
pub const UNIFORM_TONES: u32 = 44;
pub const LANE_COUNT: usize = 16;
pub const MAX_SAMPLES: usize = 4;
pub const WORDS_PER_TONE: u32 = 4;
pub const TONES_PER_SLOT: u32 = 16;
pub const CLOCK_FRAME: usize = 0;
pub const CLOCK_CYCLE: usize = 1;
pub const CLOCK_FPS: usize = 2;
pub const CLOCK_STEP: usize = 3;
pub const COVERAGE_THRESHOLD: f64 = 0.5 / 255.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Resolution {
    #[default]
    Pixel,
    Smooth,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    #[default]
    Alpha,
    Additive,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Reg(pub u16);

impl Reg {
    pub const NONE: Reg = Reg(u16::MAX);

    pub const fn is_none(self) -> bool {
        self.0 == u16::MAX
    }
}

impl Default for Reg {
    fn default() -> Reg {
        Reg::NONE
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Ty {
    #[default]
    Float,
    Vec2,
    Vec3,
    Vec4,
    Slot,
}

impl Ty {
    pub const fn width(self) -> u8 {
        match self {
            Ty::Float | Ty::Slot => 1,
            Ty::Vec2 => 2,
            Ty::Vec3 => 3,
            Ty::Vec4 => 4,
        }
    }

    pub fn vector(width: u8) -> Option<Ty> {
        match width {
            1 => Some(Ty::Float),
            2 => Some(Ty::Vec2),
            3 => Some(Ty::Vec3),
            4 => Some(Ty::Vec4),
            _ => None,
        }
    }

    pub fn glsl(self) -> &'static str {
        match self {
            Ty::Float => "float",
            Ty::Vec2 => "vec2",
            Ty::Vec3 => "vec3",
            Ty::Vec4 => "vec4",
            Ty::Slot => "int",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpKind {
    PixelPos,
    PixelCell,
    TagUv,
    AreaSize,
    BodyRect,
    Frame,
    SteppedTime,
    ScreenPos,
    PixelScreenPos,
    Slot,
    Constant,
    Param,
    Expr,
    BodyEdge,
    Abs,
    Floor,
    Frac,
    Sin,
    Cos,
    Exp,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Min,
    Max,
    Step,
    Pow,
    Atan2,
    Clamp,
    Mix,
    Smoothstep,
    Length,
    Dot,
    Polar,
    Combine,
    Component,
    Select,
    Noise,
    Fbm,
    Hash,
    Bayer,
    Checker,
    Stripes,
    Gradient,
    Hsv,
    Quantize,
    DitherQuantize,
    Cycle,
    Lighten,
    Darken,
    Alpha,
    SampleAt,
    Repeat,
    RepeatIndex,
    Sprite,
    BundleTexture,
    VanillaTexture,
    AtlasSprite,
}

impl OpKind {
    pub const ALL: [OpKind; 60] = [
        OpKind::PixelPos,
        OpKind::PixelCell,
        OpKind::TagUv,
        OpKind::AreaSize,
        OpKind::BodyRect,
        OpKind::Frame,
        OpKind::SteppedTime,
        OpKind::ScreenPos,
        OpKind::PixelScreenPos,
        OpKind::Slot,
        OpKind::Constant,
        OpKind::Param,
        OpKind::Expr,
        OpKind::BodyEdge,
        OpKind::Abs,
        OpKind::Floor,
        OpKind::Frac,
        OpKind::Sin,
        OpKind::Cos,
        OpKind::Exp,
        OpKind::Add,
        OpKind::Sub,
        OpKind::Mul,
        OpKind::Div,
        OpKind::Mod,
        OpKind::Min,
        OpKind::Max,
        OpKind::Step,
        OpKind::Pow,
        OpKind::Atan2,
        OpKind::Clamp,
        OpKind::Mix,
        OpKind::Smoothstep,
        OpKind::Length,
        OpKind::Dot,
        OpKind::Polar,
        OpKind::Combine,
        OpKind::Component,
        OpKind::Select,
        OpKind::Noise,
        OpKind::Fbm,
        OpKind::Hash,
        OpKind::Bayer,
        OpKind::Checker,
        OpKind::Stripes,
        OpKind::Gradient,
        OpKind::Hsv,
        OpKind::Quantize,
        OpKind::DitherQuantize,
        OpKind::Cycle,
        OpKind::Lighten,
        OpKind::Darken,
        OpKind::Alpha,
        OpKind::SampleAt,
        OpKind::Repeat,
        OpKind::RepeatIndex,
        OpKind::Sprite,
        OpKind::BundleTexture,
        OpKind::VanillaTexture,
        OpKind::AtlasSprite,
    ];

    pub fn name(self) -> &'static str {
        match self {
            OpKind::PixelPos => "PixelPos",
            OpKind::PixelCell => "PixelCell",
            OpKind::TagUv => "TagUV",
            OpKind::AreaSize => "AreaSize",
            OpKind::BodyRect => "BodyRect",
            OpKind::Frame => "Frame",
            OpKind::SteppedTime => "SteppedTime",
            OpKind::ScreenPos => "ScreenPos",
            OpKind::PixelScreenPos => "PixelScreenPos",
            OpKind::Slot => "Slot",
            OpKind::Constant => "Constant",
            OpKind::Param => "Param",
            OpKind::Expr => "Expr",
            OpKind::BodyEdge => "BodyEdge",
            OpKind::Abs => "abs",
            OpKind::Floor => "floor",
            OpKind::Frac => "frac",
            OpKind::Sin => "sin",
            OpKind::Cos => "cos",
            OpKind::Exp => "exp",
            OpKind::Add => "add",
            OpKind::Sub => "sub",
            OpKind::Mul => "mul",
            OpKind::Div => "div",
            OpKind::Mod => "mod",
            OpKind::Min => "min",
            OpKind::Max => "max",
            OpKind::Step => "step",
            OpKind::Pow => "pow",
            OpKind::Atan2 => "atan2",
            OpKind::Clamp => "clamp",
            OpKind::Mix => "mix",
            OpKind::Smoothstep => "smoothstep",
            OpKind::Length => "length",
            OpKind::Dot => "dot",
            OpKind::Polar => "polar",
            OpKind::Combine => "Combine",
            OpKind::Component => "Component",
            OpKind::Select => "Select",
            OpKind::Noise => "Noise",
            OpKind::Fbm => "Fbm",
            OpKind::Hash => "Hash",
            OpKind::Bayer => "Bayer",
            OpKind::Checker => "Checker",
            OpKind::Stripes => "Stripes",
            OpKind::Gradient => "Gradient",
            OpKind::Hsv => "Hsv",
            OpKind::Quantize => "Quantize",
            OpKind::DitherQuantize => "DitherQuantize",
            OpKind::Cycle => "Cycle",
            OpKind::Lighten => "Lighten",
            OpKind::Darken => "Darken",
            OpKind::Alpha => "Alpha",
            OpKind::SampleAt => "SampleAt",
            OpKind::Repeat => "Repeat",
            OpKind::RepeatIndex => "RepeatIndex",
            OpKind::Sprite => "Sprite",
            OpKind::BundleTexture => "BundleTexture",
            OpKind::VanillaTexture => "VanillaTexture",
            OpKind::AtlasSprite => "AtlasSprite",
        }
    }

    pub fn parse(name: &str) -> Option<OpKind> {
        OpKind::ALL.into_iter().find(|k| k.name() == name)
    }

    pub fn samples(self) -> u32 {
        match self {
            OpKind::BundleTexture | OpKind::VanillaTexture | OpKind::AtlasSprite => 1,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Op {
    pub kind: OpKind,
    pub ty: Ty,
    pub dst: Reg,
    pub args: [Reg; 4],
    pub imm: [f64; 4],
}

impl Op {
    pub fn body(&self) -> Option<Range<usize>> {
        match self.kind {
            OpKind::SampleAt | OpKind::Repeat => {
                let start = self.imm[0].max(0.0) as usize;
                let end = self.imm[1].max(0.0) as usize;
                Some(start..end.max(start))
            }
            _ => None,
        }
    }

    pub fn body_result(&self) -> Reg {
        match self.kind {
            OpKind::SampleAt => self.args[2],
            OpKind::Repeat => self.args[0],
            _ => Reg::NONE,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Reduce {
    Sum,
    Min,
    #[default]
    Max,
    First,
}

impl Reduce {
    pub const ALL: [Reduce; 4] = [Reduce::Sum, Reduce::Min, Reduce::Max, Reduce::First];

    pub fn as_str(self) -> &'static str {
        match self {
            Reduce::Sum => "sum",
            Reduce::Min => "min",
            Reduce::Max => "max",
            Reduce::First => "first",
        }
    }

    pub fn parse(name: &str) -> Option<Reduce> {
        Reduce::ALL.into_iter().find(|r| r.as_str() == name)
    }

    pub const fn value(self) -> f64 {
        self as u8 as f64
    }

    pub fn from_value(value: f64) -> Reduce {
        Reduce::ALL
            .get(value.max(0.0) as usize)
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeLane {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    Rings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    Param(u16),
    Expr(u32),
    BodyEdge(EdgeLane),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VanillaTexture {
    EndSky,
    EndPortal,
    Glint,
    BeaconBeam,
    CreeperArmor,
    BreezeWind,
}

impl VanillaTexture {
    pub const ALL: [VanillaTexture; 6] = [
        VanillaTexture::EndSky,
        VanillaTexture::EndPortal,
        VanillaTexture::Glint,
        VanillaTexture::BeaconBeam,
        VanillaTexture::CreeperArmor,
        VanillaTexture::BreezeWind,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            VanillaTexture::EndSky => "end_sky",
            VanillaTexture::EndPortal => "end_portal",
            VanillaTexture::Glint => "glint",
            VanillaTexture::BeaconBeam => "beacon_beam",
            VanillaTexture::CreeperArmor => "creeper_armor",
            VanillaTexture::BreezeWind => "breeze_wind",
        }
    }

    pub fn parse(name: &str) -> Option<VanillaTexture> {
        VanillaTexture::ALL.into_iter().find(|t| t.as_str() == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TexRef {
    Bundle(u16),
    Vanilla(VanillaTexture),
    Atlas(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Period {
    #[default]
    Still,
    Frames(u32),
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialIr {
    pub resolution: Resolution,
    pub blend: Blend,
    pub emissive: bool,
    pub outline: Option<ColorRef>,
    pub margins: [u8; 4],
    pub ops: Vec<Op>,
    pub color: Reg,
    pub coverage: Option<Reg>,
    pub lanes: Vec<Lane>,
    pub textures: Vec<TexRef>,
    pub tone_slots: Vec<u8>,
    pub tone_hint: Vec<ColorRef>,
    pub period: Period,
    pub cost: u32,
    pub sprites: Vec<Sprite>,
    pub hint_colors: Vec<u32>,
}

impl MaterialIr {
    pub fn area(&self, body: Rect) -> Rect {
        let [left, top, right, bottom] = self.margins.map(i32::from);
        body.grow(left, top, right, bottom)
    }

    pub fn samples(&self) -> u32 {
        self.ops.iter().map(|op| op.kind.samples()).sum()
    }

    pub fn uniform_words(&self) -> u32 {
        UNIFORM_TONES + self.tone_slots.len() as u32 * TONES_PER_SLOT * WORDS_PER_TONE
    }

    pub fn reads_frame(&self) -> bool {
        self.ops.iter().any(|op| {
            matches!(
                op.kind,
                OpKind::Frame | OpKind::SteppedTime | OpKind::Cycle | OpKind::Expr
            )
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dialect {
    Core330,
    Core330Ubo,
    Legacy150,
    Compat130,
    Es300,
    Sksl,
}

impl Dialect {
    pub const ALL: [Dialect; 6] = [
        Dialect::Core330,
        Dialect::Core330Ubo,
        Dialect::Legacy150,
        Dialect::Compat130,
        Dialect::Es300,
        Dialect::Sksl,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Dialect::Core330 => "core_330",
            Dialect::Core330Ubo => "core_330_ubo",
            Dialect::Legacy150 => "legacy_150",
            Dialect::Compat130 => "compat_130",
            Dialect::Es300 => "es_300",
            Dialect::Sksl => "sksl",
        }
    }

    pub fn parse(name: &str) -> Option<Dialect> {
        Dialect::ALL.into_iter().find(|d| d.as_str() == name)
    }

    pub const fn value(self) -> i32 {
        self as i32
    }

    pub fn from_value(value: i32) -> Option<Dialect> {
        usize::try_from(value)
            .ok()
            .and_then(|i| Dialect::ALL.get(i).copied())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sampler {
    pub name: String,
    pub unit: u8,
    pub texture: TexRef,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderSource {
    pub key: String,
    pub material: u16,
    pub vertex: String,
    pub fragment: String,
    pub samplers: Vec<Sampler>,
    pub uniform_words: u32,
    pub tones: Vec<u8>,
    pub blend: Blend,
    pub depth_prepass: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialFrame<'a> {
    pub area: Rect,
    pub body: Rect,
    pub clock: [i32; 4],
    pub lanes: [f64; LANE_COUNT],
    pub tones: &'a [u32],
    pub atlas: [[f32; 4]; MAX_SAMPLES],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_kinds_and_dialects_round_trip_by_name_and_value() {
        assert_eq!(OpKind::ALL.len(), 60);
        for (i, kind) in OpKind::ALL.iter().enumerate() {
            assert_eq!(OpKind::parse(kind.name()), Some(*kind));
            assert!(!OpKind::ALL[i + 1..].iter().any(|k| k.name() == kind.name()));
        }
        assert_eq!(OpKind::parse("TagUV"), Some(OpKind::TagUv));
        assert_eq!(OpKind::parse("ADD"), None);
        for dialect in Dialect::ALL {
            assert_eq!(Dialect::from_value(dialect.value()), Some(dialect));
        }
        assert_eq!(Dialect::from_value(5), Some(Dialect::Sksl));
        assert_eq!(Dialect::from_value(6), None);
    }
}
