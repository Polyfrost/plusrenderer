use std::ops::Range;

use crate::container::Image;
use crate::molang::{Interner, Program};
use crate::palette::{Palette, MAX_SLOTS};
use crate::wearer::WearerDeclarations;

use super::geom::{Anchor, Edge};
use super::image::{Axis, ColorRef};
use super::material::ir::MaterialIr;
use super::queries::MolangUse;

pub const MAX_LAYERS: usize = 32;
pub const MAX_LETTERS: usize = 24;
pub const PALETTE_TONES: usize = 32 * 16;
pub const SPRITE_CLEAR: u8 = u8::MAX;
pub const CHANCE_ONE: u32 = 1 << 24;
pub const OPAQUE: u8 = u8::MAX;
pub const LETTER_KEEP: u32 = 0;
pub const SCALE_ONE_QUARTERS: u8 = 4;

#[derive(Debug)]
pub struct NameplateAsset {
    pub id: String,
    pub identity: u64,
    pub fps: u8,
    pub loop_frames: u16,
    pub phase: Phase,
    pub params: Vec<Param>,
    pub palette: Palette,
    pub variants: Vec<Variant>,
    pub layers: Vec<Layer>,
    pub groups: Vec<Group>,
    pub sprites: Vec<Sprite>,
    pub flags: Vec<Flag>,
    pub materials: Vec<MaterialIr>,
    pub textures: Vec<Image>,
    pub animations: Animations,
    pub text: TextStyle,
    pub badge: BadgeStyle,
    pub molang: MolangSet,
    pub record: WearerDeclarations,
    pub max_out_words: u32,
    pub requires: Vec<String>,
    pub fixed_colors: Vec<u32>,
    pub breakpoint_edges: Vec<i32>,
}

impl NameplateAsset {
    pub fn slot_count(&self) -> usize {
        self.variants.first().map_or(0, |v| v.slots.len())
    }

    pub fn variant(&self, index: u16) -> Option<&Variant> {
        self.variants.get(usize::from(index))
    }

    pub fn variant_index(&self, key: &str) -> Option<u16> {
        self.variants
            .iter()
            .position(|v| v.key == key)
            .and_then(|i| u16::try_from(i).ok())
    }

    pub fn breakpoint(&self, name_width: i32) -> u8 {
        let passed = self
            .breakpoint_edges
            .iter()
            .take_while(|edge| **edge <= name_width)
            .count();
        u8::try_from(passed).unwrap_or(u8::MAX)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Phase {
    #[default]
    Global,
    PerPlayer,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Param {
    pub id: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Variant {
    pub key: String,
    pub identity: u64,
    pub colorway: Option<u16>,
    pub tones: Vec<u32>,
    pub stops: Vec<Vec<u32>>,
    pub slots: Vec<Option<u8>>,
    pub flag: Option<u16>,
    pub owns_background: bool,
    pub text: TextStyle,
    pub badge: BadgeStyle,
    pub params: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Depth {
    Behind,
    #[default]
    Background,
    OverBackground,
    OverText,
}

impl Depth {
    pub const ALL: [Depth; 4] = [
        Depth::Behind,
        Depth::Background,
        Depth::OverBackground,
        Depth::OverText,
    ];

    pub const fn band(self) -> u8 {
        self as u8
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Depth::Behind => "behind",
            Depth::Background => "background",
            Depth::OverBackground => "over_background",
            Depth::OverText => "over_text",
        }
    }

    pub fn parse(name: &str) -> Option<Depth> {
        Depth::ALL.into_iter().find(|d| d.as_str() == name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Of {
    #[default]
    Block,
    Name,
    Layer(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mask {
    Name,
    Layer { slot: u8, fill: bool },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mirror {
    pub delay: u16,
    pub seeded: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Breakpoint {
    pub min: i32,
    pub max: Option<i32>,
    pub sprite: Option<u16>,
}

impl Breakpoint {
    pub fn contains(&self, name_width: i32) -> bool {
        name_width >= self.min && self.max.is_none_or(|max| name_width < max)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: String,
    pub depth: Depth,
    pub step: u16,
    pub mask: Option<Mask>,
    pub of: Of,
    pub mirror: Option<Mirror>,
    pub breakpoints: Vec<Breakpoint>,
    pub content: LayerContent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerContent {
    Body(Body),
    Sprite(SpriteLayer),
    Emitter(Emitter),
    Material(MaterialLayer),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    pub padding: [u8; 4],
    pub min_width: u16,
    pub corners: [u8; 4],
    pub rings: Vec<Fill>,
    pub fill: Option<Fill>,
    pub art: Option<BodyArt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fill {
    Color(ColorRef),
    Gradient { slot: u8 },
    Blend { slot: u8 },
    Sweep { slot: u8, axis: Axis },
    Flag,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BodyArt {
    pub sprite: u16,
    pub left: u8,
    pub right: u8,
    pub mirror: bool,
    pub segment: Option<[u8; 2]>,
    pub center: Option<[u8; 2]>,
    pub top: u8,
    pub bottom: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Follow {
    Outside,
    Line,
    Inside,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Hold {
    pub frame: u16,
    pub hold: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteLayer {
    pub sprite: u16,
    pub anchor: Anchor,
    pub at: Option<f64>,
    pub pivot: Anchor,
    pub offset: [i8; 2],
    pub timeline: Vec<Hold>,
    pub follow: Option<Follow>,
    pub path: Option<Path>,
    pub repeat: Option<Repeat>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    Top,
    Bottom,
    Perimeter,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PathPoint {
    pub anchor: Anchor,
    pub offset: [i8; 2],
    pub pause: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PathShape {
    Points(Vec<PathPoint>),
    Preset(Preset),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PathSpeed {
    Track(Track),
    Laps(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PathEnd {
    #[default]
    Wrap,
    Bounce,
    Stop,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathStop {
    pub at: f64,
    pub pause: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facings {
    pub right: Vec<u16>,
    pub left: Vec<u16>,
    pub up: Vec<u16>,
    pub down: Vec<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Walkers {
    pub count: u8,
    pub spacing: u16,
    pub even: bool,
}

impl Default for Walkers {
    fn default() -> Walkers {
        Walkers {
            count: 1,
            spacing: 0,
            even: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub shape: PathShape,
    pub follow: Option<Follow>,
    pub speed: PathSpeed,
    pub end: PathEnd,
    pub reverse: bool,
    pub stops: Vec<PathStop>,
    pub facings: Facings,
    pub walkers: Walkers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Repeat {
    pub edge: Edge,
    pub spacing: u16,
    pub min: u8,
    pub max: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SpawnShape {
    #[default]
    Point,
    Line,
    Area,
    Border,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Spawn {
    pub shape: SpawnShape,
    pub anchor: Anchor,
    pub edge: Edge,
    pub offset: [i8; 2],
    pub follow: Option<Follow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rate {
    pub every: u16,
    pub count: u8,
    pub jitter: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Wobble {
    pub x: u8,
    pub y: u8,
    pub period: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ParticleFrames {
    #[default]
    Loop,
    Life,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fade {
    pub after: f64,
    pub to: u8,
    pub steps: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    pub sprites: Vec<u16>,
    pub spawn: Spawn,
    pub rate: Option<Rate>,
    pub lifetime: [u16; 2],
    pub velocity_x: [i32; 2],
    pub velocity_y: [i32; 2],
    pub gravity: [i32; 2],
    pub drift: [i32; 2],
    pub wobble: Option<Wobble>,
    pub frames: ParticleFrames,
    pub fade: Option<Fade>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialLayer {
    pub material: u16,
    pub drift: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub parent: Option<u8>,
    pub layers: Vec<u8>,
    pub breakpoints: Vec<Breakpoint>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sprite {
    pub id: String,
    pub width: u8,
    pub height: u8,
    pub colors: Vec<ColorRef>,
    pub frames: Vec<Vec<u8>>,
}

impl Sprite {
    pub fn pixel(&self, frame: usize, x: usize, y: usize) -> Option<ColorRef> {
        let index = *self
            .frames
            .get(frame)?
            .get(y * usize::from(self.width) + x)?;
        if index == SPRITE_CLEAR {
            return None;
        }
        self.colors.get(usize::from(index)).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Band {
    pub color: ColorRef,
    pub weight: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverlayKind {
    Chevron,
    Triangle,
    Circle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub kind: OverlayKind,
    pub colors: Vec<ColorRef>,
    pub width: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowsOverride {
    pub rows: u16,
    pub bands: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub id: String,
    pub bands: Vec<Band>,
    pub overlay: Option<Overlay>,
    pub rows: Vec<RowsOverride>,
    pub symmetric: bool,
    pub axis: Axis,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextMode {
    #[default]
    Vanilla,
    Motion,
    Owned,
}

impl TextMode {
    pub const fn value(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Paint {
    #[default]
    None,
    Auto,
    Color(ColorRef),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Formatting {
    #[default]
    Adjust,
    Keep,
    Override,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    pub mode: TextMode,
    pub color: Paint,
    pub shadow: Paint,
    pub outline: Paint,
    pub formatting: Formatting,
    pub candidates: Vec<ColorRef>,
    pub backdrop_hint: Vec<ColorRef>,
    pub motion: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedBadge {
    pub anchor: Anchor,
    pub at: Option<f64>,
    pub offset: [i8; 2],
    pub of: Of,
    pub follow: Option<Follow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BadgePlacement {
    #[default]
    Reserve,
    Carried,
    Placed(PlacedBadge),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Decoration {
    #[default]
    None,
    Auto,
    Shadow(Option<ColorRef>),
    Outline(Option<ColorRef>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BadgeStyle {
    pub placement: BadgePlacement,
    pub tint: Option<ColorRef>,
    pub decoration: Decoration,
    pub candidates: Vec<ColorRef>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Ease {
    #[default]
    Step,
    Linear,
    Ease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Fixed(i32),
    Molang(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub frame: u16,
    pub value: Value,
    pub ease: Ease,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Track {
    pub keys: Vec<Key>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Burst {
    pub frame: u16,
    pub count: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Channels {
    pub frame: Option<Track>,
    pub x: Option<Track>,
    pub y: Option<Track>,
    pub opacity: Option<Track>,
    pub visible: Option<Track>,
    pub speed: Option<Track>,
    pub bursts: Vec<Burst>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Layer(u8),
    Group(u8),
    Badge,
    Plate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetTracks {
    pub target: Target,
    pub channels: Channels,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CycleMode {
    #[default]
    Forward,
    Backward,
    PingPong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaletteValue {
    Color(u32),
    Ref(ColorRef),
    Molang(u32),
    Base,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteKey {
    pub frame: u16,
    pub ease: Ease,
    pub tones: Vec<PaletteValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteTrack {
    Cycle { every: u16, mode: CycleMode },
    Keys(Vec<PaletteKey>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotTrack {
    pub slot: u8,
    pub track: PaletteTrack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LetterOverride {
    Off,
    Program(u32),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clip {
    pub name: String,
    pub length: u16,
    pub targets: Vec<TargetTracks>,
    pub palette: Vec<SlotTrack>,
    pub letters: Option<LetterOverride>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScheduledClip {
    pub clip: Clip,
    pub weight: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    pub id: String,
    pub salt: u64,
    pub window: u16,
    pub jitter: u16,
    pub chance: u32,
    pub events: Vec<ScheduledClip>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Reduced {
    #[default]
    Hold,
    Drop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    pub signal: u8,
    pub clip: Clip,
    pub reduced: Reduced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    Start(u8),
    Stop(u8),
    Appear(u8),
    Equip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneShot {
    pub trigger: Trigger,
    pub clip: Clip,
    pub restart: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Animations {
    pub looping: Clip,
    pub schedules: Vec<Schedule>,
    pub reactions: Vec<Reaction>,
    pub one_shots: Vec<OneShot>,
}

#[derive(Clone, Debug)]
pub struct MolangProgram {
    pub program: Program,
    pub scope: MolangUse,
    pub path: String,
    pub cost: u32,
}

#[derive(Debug, Default)]
pub struct MolangSet {
    pub interner: Interner,
    pub programs: Vec<MolangProgram>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerPose {
    pub dx: i16,
    pub dy: i16,
    pub opacity: u8,
    pub visible: bool,
    pub state: u16,
}

impl LayerPose {
    pub const IDENTITY: LayerPose = LayerPose {
        dx: 0,
        dy: 0,
        opacity: OPAQUE,
        visible: true,
        state: 0,
    };
}

impl Default for LayerPose {
    fn default() -> LayerPose {
        LayerPose::IDENTITY
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteFrame {
    pub tones: [u32; PALETTE_TONES],
    pub animated: u32,
    pub stop_counts: [u8; MAX_SLOTS],
}

impl Default for PaletteFrame {
    fn default() -> PaletteFrame {
        PaletteFrame {
            tones: [0; PALETTE_TONES],
            animated: 0,
            stop_counts: [0; MAX_SLOTS],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BadgePose {
    pub dx: i16,
    pub dy: i16,
    pub opacity: u8,
    pub visible: bool,
}

impl Default for BadgePose {
    fn default() -> BadgePose {
        BadgePose {
            dx: 0,
            dy: 0,
            opacity: OPAQUE,
            visible: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LetterPose {
    pub dx: i8,
    pub dy: i8,
    pub opacity: u8,
    pub color: u32,
    pub shadow: u32,
    pub outline: u32,
    pub scale_q: u8,
    pub rotation: i16,
}

impl LetterPose {
    pub const STILL: LetterPose = LetterPose {
        dx: 0,
        dy: 0,
        opacity: OPAQUE,
        color: LETTER_KEEP,
        shadow: LETTER_KEEP,
        outline: LETTER_KEEP,
        scale_q: SCALE_ONE_QUARTERS,
        rotation: 0,
    };
}

impl Default for LetterPose {
    fn default() -> LetterPose {
        LetterPose::STILL
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParticlePose {
    pub x: i16,
    pub y: i16,
    pub layer: u8,
    pub state: u16,
    pub opacity: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagPose {
    pub layers: [LayerPose; MAX_LAYERS],
    pub plate: (i8, i8),
    pub palette: PaletteFrame,
    pub badge: BadgePose,
    pub letters: [LetterPose; MAX_LETTERS],
    pub particles: Range<u32>,
}

impl Default for TagPose {
    fn default() -> TagPose {
        TagPose {
            layers: [LayerPose::IDENTITY; MAX_LAYERS],
            plate: (0, 0),
            palette: PaletteFrame::default(),
            badge: BadgePose::default(),
            letters: [LetterPose::STILL; MAX_LETTERS],
            particles: 0..0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depths_and_breakpoints_order_their_ranges() {
        assert!(Depth::Behind.band() < Depth::Background.band());
        assert!(Depth::OverBackground.band() < Depth::OverText.band());
        let closed = Breakpoint {
            min: 0,
            max: Some(16),
            sprite: None,
        };
        assert!(closed.contains(15) && !closed.contains(16));
        let open = Breakpoint {
            max: None,
            ..closed
        };
        assert!(open.contains(10_000));
    }
}
