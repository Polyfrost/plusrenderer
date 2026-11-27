use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::issues::{join, path, Issue, IssueCode, Level, Mode, Report};
use crate::limits::{
    NAMEPLATE_MATERIAL_COST, NAMEPLATE_MATERIAL_INPUTS, NAMEPLATE_MATERIAL_MARGIN,
    NAMEPLATE_MATERIAL_NODES, NAMEPLATE_MATERIAL_OCTAVES, NAMEPLATE_MATERIAL_OUTLINE_FACTOR,
    NAMEPLATE_MATERIAL_REPEAT, NAMEPLATE_MATERIAL_SAMPLES,
};
use crate::molang::{compile_checked, Interner, Options, Program, Rules, Warnings};
use crate::nameplate::image::{ColorRef, RefKind};
use crate::nameplate::queries::{scope_for, MolangUse};
use crate::nameplate::types::{MolangProgram, MolangSet, Param, Sprite};
use crate::palette::{Palette, Slot as PaletteSlot, MAX_TONES};
use crate::signals::{SIGNALS, TIME_SUFFIX};

use super::ir::{
    Blend, EdgeLane, Lane, MaterialIr, Op, OpKind, Period, Reduce, Reg, Resolution, TexRef, Ty,
    VanillaTexture,
};
use super::nodes::{cost as op_cost, sockets, Accepts, Socket, SocketDefault, TONE_COST};

pub struct GraphContext<'a> {
    pub palette: &'a Palette,
    pub params: &'a [Param],
    pub textures: &'a [String],
    pub sprites: &'a [Option<Sprite>],
    pub molang: &'a mut MolangSet,
}

pub const MAX_PERIOD: i64 = 65_536;
pub const MAX_CELL_SIZE: i64 = 256;
pub const MAX_DIRECTION: i64 = 16;
pub const MAX_LITERAL: f64 = 1_000_000.0;
pub const MAX_SPRITE_CELLS: usize = 4096;
pub const DEFAULT_OCTAVES: i64 = 3;

const OUTPUT: &str = "Output";
const FILE_KEYS: [&str; 6] = [
    "resolution",
    "margins",
    "outline",
    "emissive",
    "toneHint",
    "nodes",
];
const NODE_KEYS: [&str; 4] = ["id", "type", "in", "set"];
const COMPONENTS: [&str; 4] = ["x", "y", "z", "w"];
const BLENDS: [&str; 2] = ["alpha", "additive"];
const EDGES: [&str; 2] = ["fill", "outer"];
const POS: u8 = 1;
const FRAME: u8 = 2;
const INDEX: u8 = 4;
const NO_ENV: u32 = u32::MAX;
const COST_CAP: u64 = 1_000_000_000;
const PERIOD_CAP: u64 = 1 << 20;

static OUTPUT_SOCKETS: [Socket; 2] = [
    Socket {
        name: "color",
        accepts: Accepts::Surface,
        default: SocketDefault::Required,
    },
    Socket {
        name: "coverage",
        accepts: Accepts::Float,
        default: SocketDefault::Value([1.0, 0.0, 0.0, 0.0], 1),
    },
];

pub fn build(
    file: &str,
    json: &Value,
    ctx: &mut GraphContext<'_>,
    report: &mut Report,
) -> Option<MaterialIr> {
    build_named(file, json, ctx, report).map(|(ir, _)| ir)
}

pub(crate) fn build_named(
    file: &str,
    json: &Value,
    ctx: &mut GraphContext<'_>,
    report: &mut Report,
) -> Option<(MaterialIr, Vec<(String, Reg)>)> {
    let start = errors(report);
    let mut builder = Builder {
        file,
        ctx,
        report,
        start,
        nodes: Vec::new(),
        ids: BTreeMap::new(),
    };
    builder.run(json)
}

fn errors(report: &Report) -> usize {
    report
        .issues()
        .iter()
        .filter(|i| i.level == Level::Error)
        .count()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Op(OpKind),
    Output,
}

impl Kind {
    fn sockets(self) -> &'static [Socket] {
        match self {
            Kind::Op(kind) => sockets(kind),
            Kind::Output => &OUTPUT_SOCKETS,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Op(kind) => kind.name(),
            Kind::Output => OUTPUT,
        }
    }

    fn is(self, kind: OpKind) -> bool {
        self == Kind::Op(kind)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Literal {
    values: [f64; 4],
    width: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct Input {
    node: Option<usize>,
    literal: Option<Literal>,
    target: Option<Ty>,
    given: bool,
}

impl Input {
    fn source_type(&self, nodes: &[Node]) -> Option<Ty> {
        match (self.node, self.literal) {
            (Some(node), _) => Some(nodes[node].ty),
            (None, Some(literal)) => Ty::vector(literal.width),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Settings {
    seed: i64,
    period: [i64; 2],
    octaves: i64,
    count: i64,
    size: i64,
    width: i64,
    direction: [i64; 2],
    slot: usize,
    tone: usize,
    tones: usize,
    source: Option<String>,
    reduce: Reduce,
    component: usize,
    value: Option<Literal>,
    param: usize,
    program: Option<usize>,
    varies: bool,
    sprite: Option<usize>,
    texture: Option<TexRef>,
    outer: bool,
    blend: Blend,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            seed: 0,
            period: [0, 0],
            octaves: 1,
            count: 1,
            size: 1,
            width: 1,
            direction: [1, 0],
            slot: 0,
            tone: 0,
            tones: 1,
            source: None,
            reduce: Reduce::Max,
            component: 0,
            value: None,
            param: 0,
            program: None,
            varies: false,
            sprite: None,
            texture: None,
            outer: false,
            blend: Blend::Alpha,
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    id: String,
    kind: Kind,
    inputs: Vec<Input>,
    source: Option<usize>,
    set: Settings,
    ty: Ty,
    typed: bool,
    order: usize,
}

impl Node {
    fn new(id: String, kind: Kind, set: Settings) -> Node {
        Node {
            id,
            kind,
            inputs: Vec::new(),
            source: None,
            set,
            ty: Ty::Float,
            typed: false,
            order: 0,
        }
    }

    fn dependencies(&self) -> impl Iterator<Item = usize> + '_ {
        self.inputs.iter().filter_map(|i| i.node).chain(self.source)
    }
}

struct Reader<'v> {
    values: &'v Map<String, Value>,
    at: String,
    seen: Vec<String>,
}

impl<'v> Reader<'v> {
    fn get(&mut self, name: &str) -> Option<&'v Value> {
        self.seen.push(name.to_string());
        self.values.get(name)
    }

    fn present(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    fn at(&self, name: &str) -> String {
        join(&self.at, name)
    }
}

struct Builder<'b, 'c> {
    file: &'b str,
    ctx: &'b mut GraphContext<'c>,
    report: &'b mut Report,
    start: usize,
    nodes: Vec<Node>,
    ids: BTreeMap<String, usize>,
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn atlas_id(id: &str) -> bool {
    let Some((space, name)) = id.split_once(':') else {
        return false;
    };
    let allowed = |b: u8, slash: bool| {
        b.is_ascii_lowercase()
            || b.is_ascii_digit()
            || matches!(b, b'_' | b'.' | b'-')
            || (slash && b == b'/')
    };
    (1..=64).contains(&space.len())
        && (1..=128).contains(&name.len())
        && space.bytes().all(|b| allowed(b, false))
        && name.bytes().all(|b| allowed(b, true))
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn literal(value: &Value) -> Option<Literal> {
    let bounded =
        |v: &Value| number(v).filter(|n| n.is_finite() && (-MAX_LITERAL..=MAX_LITERAL).contains(n));
    match value {
        Value::Array(items) if (1..=4).contains(&items.len()) => {
            let mut values = [0.0; 4];
            for (slot, item) in values.iter_mut().zip(items) {
                *slot = bounded(item)?;
            }
            Some(Literal {
                values,
                width: items.len() as u8,
            })
        }
        Value::Array(_) => None,
        other => bounded(other).map(|v| Literal {
            values: [v, 0.0, 0.0, 0.0],
            width: 1,
        }),
    }
}

fn parse_slot(text: &str) -> Option<(&str, usize)> {
    let (name, tone) = match text.strip_suffix(']') {
        Some(rest) => {
            let (name, digits) = rest.split_once('[')?;
            if !(1..=2).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (name, digits.parse().ok()?)
        }
        None => (text, 0),
    };
    valid_id(name).then_some((name, tone))
}

fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(if digits.len() == 6 {
        value | 0xff00_0000
    } else {
        value
    })
}

fn tone_count(slot: &PaletteSlot) -> usize {
    if !slot.ramp.is_empty() {
        slot.ramp.len()
    } else if let Some(count) = slot.interpolate {
        usize::from(count)
    } else {
        slot.stops.len()
    }
}

fn label(ty: Ty) -> &'static str {
    match ty {
        Ty::Float => "float",
        Ty::Vec2 => "vec2",
        Ty::Vec3 => "vec3",
        Ty::Vec4 => "vec4",
        Ty::Slot => "slot",
    }
}

fn target(accepts: Accepts, source: Ty) -> Option<Ty> {
    match accepts {
        Accepts::Float => (source == Ty::Float).then_some(Ty::Float),
        Accepts::Vec2 => matches!(source, Ty::Float | Ty::Vec2).then_some(Ty::Vec2),
        Accepts::Color => matches!(source, Ty::Vec4 | Ty::Slot).then_some(Ty::Vec4),
        Accepts::Surface => matches!(source, Ty::Vec4 | Ty::Slot).then_some(source),
        Accepts::Number => Some(if source == Ty::Slot { Ty::Vec4 } else { source }),
        Accepts::Any => Some(source),
    }
}

fn is_elementwise(kind: OpKind) -> bool {
    matches!(
        kind,
        OpKind::Abs
            | OpKind::Floor
            | OpKind::Frac
            | OpKind::Sin
            | OpKind::Cos
            | OpKind::Exp
            | OpKind::Add
            | OpKind::Sub
            | OpKind::Mul
            | OpKind::Div
            | OpKind::Mod
            | OpKind::Min
            | OpKind::Max
            | OpKind::Step
            | OpKind::Pow
            | OpKind::Atan2
            | OpKind::Clamp
            | OpKind::Mix
            | OpKind::Smoothstep
    )
}

fn env_bits(kind: Kind) -> u8 {
    match kind {
        Kind::Op(
            OpKind::PixelPos
            | OpKind::PixelCell
            | OpKind::TagUv
            | OpKind::ScreenPos
            | OpKind::PixelScreenPos,
        ) => POS,
        Kind::Op(OpKind::Frame | OpKind::SteppedTime) => FRAME,
        Kind::Op(OpKind::RepeatIndex) => INDEX,
        _ => 0,
    }
}

type Typed = Result<(Ty, Vec<Option<Ty>>), Issue>;

impl<'b, 'c> Builder<'b, 'c> {
    fn clean(&self) -> bool {
        errors(self.report) == self.start
    }

    fn issue(&self, code: IssueCode, at: &str) -> Issue {
        Issue::error(code, path(self.file, at))
    }

    fn push(&mut self, issue: Issue) {
        self.report.push(issue);
    }

    fn fail(&mut self, code: IssueCode, at: &str) {
        let issue = self.issue(code, at);
        self.push(issue);
    }

    fn authoring(&self) -> bool {
        self.report.mode() == Mode::Authoring
    }

    fn run(&mut self, json: &Value) -> Option<(MaterialIr, Vec<(String, Reg)>)> {
        let Some(object) = json.as_object() else {
            self.fail(IssueCode::Material, "");
            return None;
        };
        self.report.check_keys(self.file, "", object, &FILE_KEYS);
        let resolution = match object.get("resolution") {
            None => Resolution::Pixel,
            Some(Value::String(s)) if s == "pixel" => Resolution::Pixel,
            Some(Value::String(s)) if s == "smooth" => Resolution::Smooth,
            Some(_) => {
                self.fail(IssueCode::Material, "resolution");
                Resolution::Pixel
            }
        };
        let margins = self.margins(object.get("margins"));
        let emissive = match object.get("emissive") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                self.fail(IssueCode::Material, "emissive");
                false
            }
        };
        let outline = match object.get("outline") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => {
                let found = self.slot_ref(text);
                if found.is_none() {
                    self.fail(IssueCode::Material, "outline");
                }
                found.map(|(slot, tone, _)| ColorRef::slot(slot as u8, tone as u8))
            }
            Some(_) => {
                self.fail(IssueCode::Material, "outline");
                None
            }
        };
        let (tone_hint, hint_colors) = self.hints(object.get("toneHint"));
        let empty = Vec::new();
        let raw = match object.get("nodes") {
            None => &empty,
            Some(Value::Array(items)) => items,
            Some(_) => {
                self.fail(IssueCode::Material, "nodes");
                &empty
            }
        };
        if raw.len() > NAMEPLATE_MATERIAL_NODES as usize {
            let issue = self
                .issue(IssueCode::MaterialNodes, "nodes")
                .value("nodes", raw.len() as f64)
                .value("limit", f64::from(NAMEPLATE_MATERIAL_NODES));
            self.push(issue);
            return None;
        }
        let declared = self.declare(raw);
        if self.clean() {
            self.link(&declared);
        }
        if self.clean() {
            self.sort();
        }
        if self.clean() {
            let mut order: Vec<usize> = (0..self.nodes.len()).collect();
            order.sort_by_key(|&i| self.nodes[i].order);
            for i in order {
                self.type_node(i);
            }
        }
        let outputs: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].kind == Kind::Output)
            .collect();
        if self.clean() && outputs.len() != 1 {
            let issue = self
                .issue(IssueCode::MaterialOutputs, "nodes")
                .value("count", outputs.len() as f64);
            self.push(issue);
        }
        if !self.clean() {
            return None;
        }
        let head = Head {
            resolution,
            margins,
            emissive,
            outline,
            tone_hint,
            hint_colors,
        };
        self.finish(outputs[0], head)
    }

    fn margins(&mut self, value: Option<&Value>) -> [u8; 4] {
        let Some(value) = value else {
            return [0; 4];
        };
        let parsed = value.as_array().filter(|a| a.len() == 4).and_then(|items| {
            let mut out = [0u8; 4];
            for (slot, item) in out.iter_mut().zip(items) {
                let n = number(item)?;
                if n.fract() != 0.0 || !(0.0..=f64::from(NAMEPLATE_MATERIAL_MARGIN)).contains(&n) {
                    return None;
                }
                *slot = n as u8;
            }
            Some(out)
        });
        parsed.unwrap_or_else(|| {
            let issue = self
                .issue(IssueCode::MaterialMargins, "margins")
                .value("limit", f64::from(NAMEPLATE_MATERIAL_MARGIN));
            self.push(issue);
            [0; 4]
        })
    }

    fn slot_ref(&self, text: &str) -> Option<(usize, usize, usize)> {
        let (name, tone) = parse_slot(text)?;
        let slot = self.ctx.palette.slot(name)?;
        let tones = tone_count(&self.ctx.palette.slots[slot]);
        (tone < tones && tone < MAX_TONES).then_some((slot, tone, tones))
    }

    fn hints(&mut self, value: Option<&Value>) -> (Vec<ColorRef>, Vec<u32>) {
        let mut refs = Vec::new();
        let mut colors = Vec::new();
        let Some(value) = value else {
            return (refs, colors);
        };
        let Some(items) = value.as_array() else {
            self.fail(IssueCode::Material, "toneHint");
            return (refs, colors);
        };
        for (i, item) in items.iter().enumerate() {
            let text = item.as_str().unwrap_or("");
            if let Some(argb) = parse_hex(text) {
                colors.push(argb);
            } else if let Some((slot, tone, _)) = self.slot_ref(text) {
                refs.push(ColorRef::slot(slot as u8, tone as u8));
            } else {
                self.fail(IssueCode::Material, &format!("toneHint[{i}]"));
            }
        }
        (refs, colors)
    }

    fn declare<'j>(&mut self, raw: &'j [Value]) -> Vec<(usize, &'j Map<String, Value>)> {
        let mut declared = Vec::new();
        let empty_text = String::new();
        for (i, item) in raw.iter().enumerate() {
            let Some(object) = item.as_object() else {
                let issue = self.issue(IssueCode::NodeId, &format!("nodes[{i}]"));
                self.push(issue);
                continue;
            };
            let id = object
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(&empty_text);
            if !valid_id(id) {
                let issue = self.issue(IssueCode::NodeId, "nodes").name("node", id);
                self.push(issue);
                continue;
            }
            let at = join("nodes", id);
            self.report.check_keys(self.file, &at, object, &NODE_KEYS);
            if self.ids.contains_key(id) {
                self.fail(IssueCode::NodeDuplicate, &at);
                continue;
            }
            let type_name = object.get("type").and_then(Value::as_str);
            let kind = match type_name {
                Some(OUTPUT) => Some(Kind::Output),
                Some(name) => OpKind::parse(name).map(Kind::Op),
                None => None,
            };
            let Some(kind) = kind else {
                let issue = self
                    .issue(IssueCode::NodeType, &join(&at, "type"))
                    .name("type", type_name.unwrap_or(""));
                self.push(issue);
                continue;
            };
            if self.authoring() {
                if let Some(Value::Object(given)) = object.get("in") {
                    for key in given.keys() {
                        if !kind.sockets().iter().any(|s| s.name == key) {
                            let issue = self
                                .issue(IssueCode::NodeInput, &join(&join(&at, "in"), key))
                                .name("type", kind.name());
                            self.push(issue);
                        }
                    }
                }
            }
            let set = self.settings(&at, kind, object.get("set"));
            self.ids.insert(id.to_string(), self.nodes.len());
            declared.push((self.nodes.len(), object));
            self.nodes.push(Node::new(id.to_string(), kind, set));
        }
        declared
    }

    fn settings(&mut self, at: &str, kind: Kind, raw: Option<&Value>) -> Settings {
        let empty = Map::new();
        let at = join(at, "set");
        let values = match raw {
            None => &empty,
            Some(Value::Object(map)) => map,
            Some(_) => {
                self.fail(IssueCode::NodeSetting, &at);
                &empty
            }
        };
        let mut r = Reader {
            values,
            at,
            seen: Vec::new(),
        };
        let mut s = Settings::default();
        match kind {
            Kind::Output => {
                s.blend = if self.choice(&mut r, "blend", &BLENDS, 0) == 1 {
                    Blend::Additive
                } else {
                    Blend::Alpha
                };
            }
            Kind::Op(op) => self.read(op, &mut r, &mut s),
        }
        if self.authoring() {
            for key in r.values.keys() {
                if !r.seen.iter().any(|seen| seen == key) {
                    let issue = self.issue(IssueCode::NodeSetting, &r.at(key));
                    self.push(issue);
                }
            }
        }
        s
    }

    fn read(&mut self, kind: OpKind, r: &mut Reader<'_>, s: &mut Settings) {
        match kind {
            OpKind::Slot | OpKind::Quantize | OpKind::DitherQuantize | OpKind::Cycle => {
                self.read_slot(r, s);
            }
            OpKind::Constant => {
                let value = r.get("value");
                match value.map(literal) {
                    None => self.fail(IssueCode::NodeRequired, &r.at("value")),
                    Some(None) => self.fail(IssueCode::NodeSetting, &r.at("value")),
                    Some(found) => s.value = found,
                }
            }
            OpKind::Param => {
                if let Some(name) = self.required(r, "name") {
                    match self.ctx.params.iter().position(|p| p.id == name) {
                        Some(index) => s.param = index,
                        None => {
                            let issue = self
                                .issue(IssueCode::NodeParam, &r.at("name"))
                                .name("param", &name);
                            self.push(issue);
                        }
                    }
                }
            }
            OpKind::Expr => {
                if let Some(source) = self.required(r, "expr") {
                    self.compile(&r.at("expr"), &source, s);
                }
            }
            OpKind::BodyEdge => {
                s.outer = self.choice(r, "edge", &EDGES, 0) == 1;
            }
            OpKind::Noise | OpKind::Fbm => {
                s.seed = self.int(r, "seed", 0, i64::from(i32::MAX), 0);
                s.period = self.pair(r, "period", 0, MAX_PERIOD, [0, 0]);
                if kind == OpKind::Fbm {
                    s.octaves = self.int(
                        r,
                        "octaves",
                        1,
                        i64::from(NAMEPLATE_MATERIAL_OCTAVES),
                        DEFAULT_OCTAVES,
                    );
                }
            }
            OpKind::Hash => {
                s.seed = self.int(r, "seed", 0, i64::from(i32::MAX), 0);
            }
            OpKind::Checker => {
                s.size = self.int(r, "size", 1, MAX_CELL_SIZE, 1);
            }
            OpKind::Stripes => {
                s.size = self.int(r, "period", 1, MAX_CELL_SIZE, 2);
                s.width = self.int(r, "width", 1, MAX_CELL_SIZE, 1);
                s.direction = self.pair(r, "direction", -MAX_DIRECTION, MAX_DIRECTION, [1, 0]);
            }
            OpKind::Component => {
                s.component = self.choice(r, "component", &COMPONENTS, 0);
            }
            OpKind::SampleAt => {
                s.source = self.required(r, "source");
            }
            OpKind::Repeat => {
                s.source = self.required(r, "source");
                if !r.present("count") {
                    self.fail(IssueCode::NodeRequired, &r.at("count"));
                }
                s.count = self.int(r, "count", 1, i64::from(NAMEPLATE_MATERIAL_REPEAT), 1);
                let names: Vec<&str> = Reduce::ALL.iter().map(|r| r.as_str()).collect();
                s.reduce = Reduce::ALL[self.choice(r, "reduce", &names, Reduce::Max as usize)];
            }
            OpKind::Sprite => {
                if let Some(id) = self.required(r, "sprite") {
                    self.read_sprite(r, &id, s);
                }
            }
            OpKind::BundleTexture => {
                if let Some(id) = self.required(r, "texture") {
                    match self.ctx.textures.iter().position(|t| *t == id) {
                        Some(index) => s.texture = Some(TexRef::Bundle(index as u16)),
                        None => {
                            let issue = self
                                .issue(IssueCode::NodeTexture, &r.at("texture"))
                                .name("texture", &id)
                                .name("path", &format!("textures/{id}.png"));
                            self.push(issue);
                        }
                    }
                }
            }
            OpKind::VanillaTexture => {
                if let Some(name) = self.required(r, "texture") {
                    match VanillaTexture::parse(&name) {
                        Some(texture) => s.texture = Some(TexRef::Vanilla(texture)),
                        None => {
                            let issue = self
                                .issue(IssueCode::NodeTexture, &r.at("texture"))
                                .name("texture", &name);
                            self.push(issue);
                        }
                    }
                }
            }
            OpKind::AtlasSprite => {
                if let Some(id) = self.required(r, "sprite") {
                    if atlas_id(&id) {
                        s.texture = Some(TexRef::Atlas(id));
                    } else {
                        let issue = self
                            .issue(IssueCode::NodeTexture, &r.at("sprite"))
                            .name("texture", &id);
                        self.push(issue);
                    }
                }
            }
            _ => {}
        }
    }

    fn read_slot(&mut self, r: &mut Reader<'_>, s: &mut Settings) {
        let Some(text) = self.required(r, "slot") else {
            return;
        };
        match self.slot_ref(&text) {
            Some((slot, tone, tones)) => {
                s.slot = slot;
                s.tone = tone;
                s.tones = tones.clamp(1, MAX_TONES);
            }
            None => {
                let issue = self
                    .issue(IssueCode::NodeSetting, &r.at("slot"))
                    .name("slot", &text);
                self.push(issue);
            }
        }
    }

    fn read_sprite(&mut self, r: &Reader<'_>, id: &str, s: &mut Settings) {
        let found = self
            .ctx
            .sprites
            .iter()
            .position(|sprite| sprite.as_ref().is_some_and(|sprite| sprite.id == id));
        let usable = found.filter(|&i| {
            self.ctx.sprites[i].as_ref().is_some_and(|sprite| {
                let cells =
                    usize::from(sprite.width) * usize::from(sprite.height) * sprite.frames.len();
                cells > 0 && cells <= MAX_SPRITE_CELLS
            })
        });
        match usable {
            Some(index) => s.sprite = Some(index),
            None => {
                let issue = self
                    .issue(IssueCode::NodeSprite, &r.at("sprite"))
                    .name("sprite", id)
                    .value("limit", MAX_SPRITE_CELLS as f64);
                self.push(issue);
            }
        }
    }

    fn compile(&mut self, at: &str, source: &str, s: &mut Settings) {
        let options = Options {
            rules: Rules::Strict,
            queries: scope_for(MolangUse::Material),
        };
        let program_path = path(self.file, at);
        let molang = &mut *self.ctx.molang;
        let compiled = compile_checked(
            source,
            &mut molang.interner,
            &options,
            &mut Warnings::default(),
        );
        match compiled {
            Ok(program) => {
                s.varies = expr_varies(&program, &molang.interner, source);
                s.program = Some(molang.programs.len());
                let cost = program.nodes();
                molang.programs.push(MolangProgram {
                    program,
                    scope: MolangUse::Material,
                    path: program_path,
                    cost,
                });
            }
            Err(error) => {
                let issue = self
                    .issue(IssueCode::from(error.code), at)
                    .name("error", error.message());
                self.push(issue);
            }
        }
    }

    fn int(&mut self, r: &mut Reader<'_>, name: &str, min: i64, max: i64, default: i64) -> i64 {
        let Some(value) = r.get(name) else {
            return default;
        };
        match number(value) {
            Some(n) if n.fract() == 0.0 && n >= min as f64 && n <= max as f64 => n as i64,
            _ => {
                let issue = self
                    .issue(IssueCode::NodeSetting, &r.at(name))
                    .value("min", min as f64)
                    .value("max", max as f64);
                self.push(issue);
                default
            }
        }
    }

    fn pair(
        &mut self,
        r: &mut Reader<'_>,
        name: &str,
        min: i64,
        max: i64,
        default: [i64; 2],
    ) -> [i64; 2] {
        let Some(value) = r.get(name) else {
            return default;
        };
        let whole = |v: &Value| {
            number(v).filter(|n| n.fract() == 0.0 && *n >= min as f64 && *n <= max as f64)
        };
        match value.as_array().map(Vec::as_slice) {
            Some([a, b]) => match (whole(a), whole(b)) {
                (Some(a), Some(b)) => [a as i64, b as i64],
                _ => self.bad_pair(r, name, min, max, default),
            },
            _ => self.bad_pair(r, name, min, max, default),
        }
    }

    fn bad_pair(
        &mut self,
        r: &Reader<'_>,
        name: &str,
        min: i64,
        max: i64,
        default: [i64; 2],
    ) -> [i64; 2] {
        let issue = self
            .issue(IssueCode::NodeSetting, &r.at(name))
            .value("min", min as f64)
            .value("max", max as f64);
        self.push(issue);
        default
    }

    fn text(&mut self, r: &mut Reader<'_>, name: &str) -> Option<String> {
        let value = r.get(name)?;
        match value.as_str() {
            Some(text) => Some(text.to_string()),
            None => {
                self.fail(IssueCode::NodeSetting, &r.at(name));
                None
            }
        }
    }

    fn required(&mut self, r: &mut Reader<'_>, name: &str) -> Option<String> {
        if !r.present(name) {
            r.seen.push(name.to_string());
            self.fail(IssueCode::NodeRequired, &r.at(name));
            return None;
        }
        self.text(r, name)
    }

    fn choice(
        &mut self,
        r: &mut Reader<'_>,
        name: &str,
        options: &[&str],
        default: usize,
    ) -> usize {
        let Some(text) = self.text(r, name) else {
            return default;
        };
        match options.iter().position(|o| *o == text) {
            Some(index) => index,
            None => {
                let issue = self
                    .issue(IssueCode::NodeSetting, &r.at(name))
                    .name("value", &text);
                self.push(issue);
                default
            }
        }
    }

    fn link(&mut self, declared: &[(usize, &Map<String, Value>)]) {
        let empty = Map::new();
        for &(index, object) in declared {
            let kind = self.nodes[index].kind;
            let at = join("nodes", &self.nodes[index].id);
            let given = match object.get("in") {
                None => &empty,
                Some(Value::Object(map)) => map,
                Some(_) => {
                    self.fail(IssueCode::NodeInput, &join(&at, "in"));
                    &empty
                }
            };
            let mut inputs = Vec::new();
            for socket in kind.sockets() {
                let socket_at = join(&join(&at, "in"), socket.name);
                let input = self.input(given.get(socket.name), socket, &socket_at);
                inputs.push(input);
            }
            self.nodes[index].inputs = inputs;
            if let Some(source) = self.nodes[index].set.source.clone() {
                let source_at = join(&at, "set.source");
                match self.ids.get(&source).copied() {
                    None => {
                        let issue = self
                            .issue(IssueCode::NodeLink, &source_at)
                            .name("node", &source);
                        self.push(issue);
                    }
                    Some(found) if self.nodes[found].kind == Kind::Output => {
                        self.fail(IssueCode::NodeOutputLink, &source_at);
                    }
                    Some(found) => self.nodes[index].source = Some(found),
                }
            }
        }
    }

    fn input(&mut self, value: Option<&Value>, socket: &Socket, at: &str) -> Input {
        let given = value.is_some();
        match value {
            Some(Value::String(id)) => match self.ids.get(id).copied() {
                None => {
                    let issue = self.issue(IssueCode::NodeLink, at).name("node", id);
                    self.push(issue);
                    Input::default()
                }
                Some(found) if self.nodes[found].kind == Kind::Output => {
                    self.fail(IssueCode::NodeOutputLink, at);
                    Input::default()
                }
                Some(found) => Input {
                    node: Some(found),
                    given,
                    ..Input::default()
                },
            },
            Some(other) => match literal(other) {
                Some(found) => Input {
                    literal: Some(found),
                    given,
                    ..Input::default()
                },
                None => {
                    self.fail(IssueCode::NodeLiteral, at);
                    Input::default()
                }
            },
            None => match socket.default {
                SocketDefault::Required => {
                    self.fail(IssueCode::NodeRequired, at);
                    Input::default()
                }
                SocketDefault::Absent => Input::default(),
                SocketDefault::Value(values, width) => Input {
                    literal: Some(Literal { values, width }),
                    ..Input::default()
                },
                SocketDefault::Input(kind) => {
                    let node = self.nodes.len();
                    self.nodes.push(Node::new(
                        format!("~{}", kind.name()),
                        Kind::Op(kind),
                        Settings::default(),
                    ));
                    Input {
                        node: Some(node),
                        ..Input::default()
                    }
                }
            },
        }
    }

    fn sort(&mut self) {
        let count = self.nodes.len();
        let mut state = vec![0u8; count];
        let mut order = Vec::with_capacity(count);
        let mut cycle = None;
        for i in 0..count {
            if cycle.is_none() {
                visit(&self.nodes, i, &mut state, &mut order, &mut cycle);
            }
        }
        if let Some(node) = cycle {
            let at = join("nodes", &self.nodes[node].id);
            self.fail(IssueCode::MaterialCycle, &at);
            return;
        }
        for (k, &i) in order.iter().enumerate() {
            self.nodes[i].order = k;
        }
    }

    fn type_node(&mut self, i: usize) {
        let ready = self.nodes[i].dependencies().all(|d| self.nodes[d].typed);
        if !ready {
            return;
        }
        let sources: Vec<Option<Ty>> = self.nodes[i]
            .inputs
            .iter()
            .map(|input| input.source_type(&self.nodes))
            .collect();
        match self.typing(i, &sources) {
            Ok((ty, targets)) => {
                let node = &mut self.nodes[i];
                node.ty = ty;
                node.typed = true;
                for (input, target) in node.inputs.iter_mut().zip(targets) {
                    input.target = target;
                }
            }
            Err(issue) => self.push(issue),
        }
    }

    fn mismatch(&self, i: usize, socket: &Socket, source: Option<Ty>) -> Issue {
        self.issue(IssueCode::NodeTyping, &join("nodes", &self.nodes[i].id))
            .name("socket", socket.name)
            .name("type", source.map_or("none", label))
    }

    fn fixed(&self, i: usize, sources: &[Option<Ty>], output: Ty) -> Typed {
        let sockets = self.nodes[i].kind.sockets();
        let mut targets = Vec::with_capacity(sources.len());
        for (socket, source) in sockets.iter().zip(sources) {
            match source {
                None => targets.push(None),
                Some(ty) => match target(socket.accepts, *ty) {
                    Some(found) => targets.push(Some(found)),
                    None => return Err(self.mismatch(i, socket, *source)),
                },
            }
        }
        Ok((output, targets))
    }

    fn elementwise(&self, i: usize, sources: &[Option<Ty>]) -> Typed {
        let sockets = self.nodes[i].kind.sockets();
        let widths: Vec<Option<u8>> = sources
            .iter()
            .map(|s| s.and_then(|ty| target(Accepts::Number, ty)).map(Ty::width))
            .collect();
        let width = widths.iter().flatten().copied().max().unwrap_or(1);
        let output = Ty::vector(width).unwrap_or(Ty::Float);
        if let Some(bad) = widths
            .iter()
            .position(|w| w.is_some_and(|w| w != 1 && w != width))
        {
            return Err(self
                .mismatch(i, &sockets[bad], sources[bad])
                .name("expected", label(output)));
        }
        Ok((output, sources.iter().map(|s| s.map(|_| output)).collect()))
    }

    fn typing(&self, i: usize, sources: &[Option<Ty>]) -> Typed {
        let node = &self.nodes[i];
        let at = join("nodes", &node.id);
        let reason = |text: &str| Err(self.issue(IssueCode::NodeTyping, &at).name("reason", text));
        let kind = match node.kind {
            Kind::Output => return self.fixed(i, sources, Ty::Float),
            Kind::Op(kind) => kind,
        };
        let bare = |ty: Ty| Ok((ty, Vec::new()));
        match kind {
            OpKind::PixelPos
            | OpKind::PixelCell
            | OpKind::TagUv
            | OpKind::AreaSize
            | OpKind::ScreenPos
            | OpKind::PixelScreenPos => bare(Ty::Vec2),
            OpKind::BodyRect => bare(Ty::Vec4),
            OpKind::Frame
            | OpKind::SteppedTime
            | OpKind::Param
            | OpKind::Expr
            | OpKind::RepeatIndex => bare(Ty::Float),
            OpKind::Slot => bare(Ty::Slot),
            OpKind::Constant => {
                let width = node.set.value.map_or(1, |v| v.width);
                bare(Ty::vector(width).unwrap_or(Ty::Float))
            }
            OpKind::BodyEdge
            | OpKind::Noise
            | OpKind::Fbm
            | OpKind::Hash
            | OpKind::Bayer
            | OpKind::Checker
            | OpKind::Stripes
            | OpKind::Gradient => self.fixed(i, sources, Ty::Float),
            OpKind::Polar => self.fixed(i, sources, Ty::Vec2),
            OpKind::Hsv
            | OpKind::Lighten
            | OpKind::Darken
            | OpKind::Alpha
            | OpKind::BundleTexture
            | OpKind::VanillaTexture
            | OpKind::AtlasSprite => self.fixed(i, sources, Ty::Vec4),
            OpKind::Quantize | OpKind::DitherQuantize | OpKind::Cycle | OpKind::Sprite => {
                self.fixed(i, sources, Ty::Slot)
            }
            OpKind::Length | OpKind::Dot => {
                self.elementwise(i, sources).map(|(_, t)| (Ty::Float, t))
            }
            OpKind::Combine => {
                let width = sources
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or(sources.len());
                if sources[width..].iter().any(Option::is_some) {
                    return reason("w needs z");
                }
                self.fixed(i, sources, Ty::vector(width as u8).unwrap_or(Ty::Float))
            }
            OpKind::Component => {
                let (ty, targets) = self.elementwise(i, sources)?;
                if node.set.component >= usize::from(ty.width()) {
                    return Err(self
                        .issue(IssueCode::NodeTyping, &at)
                        .name("type", label(ty))
                        .name("component", COMPONENTS[node.set.component]));
                }
                Ok((Ty::Float, targets))
            }
            OpKind::Select => {
                let (a, b) = (sources[1], sources[2]);
                let unified = match (a, b) {
                    (Some(a), Some(b)) if a == b => Some(a),
                    (Some(Ty::Float), Some(b)) if b != Ty::Slot => Some(b),
                    (Some(a), Some(Ty::Float)) if a != Ty::Slot => Some(a),
                    (Some(Ty::Slot), Some(Ty::Vec4)) | (Some(Ty::Vec4), Some(Ty::Slot)) => {
                        Some(Ty::Vec4)
                    }
                    _ => None,
                };
                if sources[0] != Some(Ty::Float) {
                    return Err(self.mismatch(i, &sockets(kind)[0], sources[0]));
                }
                match unified {
                    Some(ty) => Ok((ty, vec![Some(Ty::Float), Some(ty), Some(ty)])),
                    None => Err(self
                        .issue(IssueCode::NodeTyping, &at)
                        .name("a", a.map_or("none", label))
                        .name("b", b.map_or("none", label))),
                }
            }
            OpKind::SampleAt => match node.source {
                Some(source) => self.fixed(i, sources, self.nodes[source].ty),
                None => reason("needs a source"),
            },
            OpKind::Repeat => match node.source.map(|s| self.nodes[s].ty) {
                None => reason("needs a source"),
                Some(Ty::Slot) if node.set.reduce != Reduce::First => {
                    reason("slots only reduce with first")
                }
                Some(ty) => Ok((ty, Vec::new())),
            },
            kind if is_elementwise(kind) => self.elementwise(i, sources),
            _ => reason("unsupported"),
        }
    }

    fn own_cost(&self, i: usize) -> u64 {
        let node = &self.nodes[i];
        let tones = node
            .inputs
            .iter()
            .filter(|input| {
                input.node.is_some_and(|n| self.nodes[n].ty == Ty::Slot)
                    && input.target == Some(Ty::Vec4)
            })
            .count() as u64;
        let own = match node.kind {
            Kind::Output => 1,
            Kind::Op(kind) => u64::from(op_cost(&Op {
                kind,
                ty: node.ty,
                dst: Reg::NONE,
                args: [Reg::NONE; 4],
                imm: self.imm(i, None, None, None),
            })),
        };
        own + u64::from(TONE_COST) * tones
    }

    fn load(
        &mut self,
        root: usize,
        inside: bool,
        memo: &mut BTreeMap<(usize, bool), (u64, u64)>,
        reported: &mut BTreeSet<usize>,
    ) -> (u64, u64) {
        if let Some(found) = memo.get(&(root, inside)) {
            return *found;
        }
        let mut cost = 0u64;
        let mut samples = 0u64;
        for node in cone(&self.nodes, root) {
            let kind = self.nodes[node].kind;
            let repeat = kind.is(OpKind::Repeat);
            let misplaced = (repeat && inside) || (kind.is(OpKind::RepeatIndex) && !inside);
            if misplaced && reported.insert(node) {
                let at = join("nodes", &self.nodes[node].id);
                let reason = if repeat { "nested" } else { "outside" };
                let issue = self
                    .issue(IssueCode::NodeRepeat, &at)
                    .name("reason", reason);
                self.push(issue);
            }
            let sub = match self.nodes[node].source {
                Some(source) => self.load(source, inside || repeat, memo, reported),
                None => (0, 0),
            };
            let times = if repeat {
                self.nodes[node].set.count.max(1) as u64
            } else {
                1
            };
            let own_samples = match kind {
                Kind::Op(op) => u64::from(op.samples()),
                Kind::Output => 0,
            };
            cost = cost
                .saturating_add(self.own_cost(node))
                .saturating_add(times.saturating_mul(sub.0));
            samples = samples
                .saturating_add(own_samples)
                .saturating_add(times.saturating_mul(sub.1));
        }
        let result = (cost.min(COST_CAP), samples.min(COST_CAP));
        memo.insert((root, inside), result);
        result
    }

    fn imm(
        &self,
        i: usize,
        lane: Option<usize>,
        texture: Option<usize>,
        sprite: Option<usize>,
    ) -> [f64; 4] {
        let node = &self.nodes[i];
        let s = &node.set;
        let input_width = node
            .inputs
            .first()
            .and_then(|input| input.target)
            .map_or(1.0, |ty| f64::from(ty.width()));
        let index = |v: Option<usize>| v.map_or(0.0, |v| v as f64);
        let Kind::Op(kind) = node.kind else {
            return [0.0; 4];
        };
        match kind {
            OpKind::Constant => s.value.map_or([0.0; 4], |v| v.values),
            OpKind::Slot => [(s.slot * MAX_TONES + s.tone) as f64, 0.0, 0.0, 0.0],
            OpKind::Param | OpKind::Expr => [index(lane), 0.0, 0.0, 0.0],
            OpKind::BodyEdge => [index(lane), f64::from(u8::from(s.outer)), 0.0, 0.0],
            OpKind::Length | OpKind::Dot => [input_width, 0.0, 0.0, 0.0],
            OpKind::Component => [s.component as f64, 0.0, 0.0, 0.0],
            OpKind::Noise | OpKind::Fbm => [
                s.seed as f64,
                s.period[0] as f64,
                s.period[1] as f64,
                s.octaves as f64,
            ],
            OpKind::Hash => [s.seed as f64, 0.0, 0.0, 0.0],
            OpKind::Checker => [s.size as f64, 0.0, 0.0, 0.0],
            OpKind::Stripes => [
                s.size as f64,
                s.width as f64,
                s.direction[0] as f64,
                s.direction[1] as f64,
            ],
            OpKind::Quantize | OpKind::DitherQuantize | OpKind::Cycle => {
                [s.slot as f64, s.tones as f64, 0.0, 0.0]
            }
            OpKind::Repeat => [0.0, 0.0, s.count as f64, s.reduce.value()],
            OpKind::Sprite => [index(sprite), 0.0, 0.0, 0.0],
            OpKind::BundleTexture | OpKind::VanillaTexture | OpKind::AtlasSprite => {
                [index(texture), 0.0, 0.0, 0.0]
            }
            _ => [0.0; 4],
        }
    }

    fn finish(&mut self, output: usize, head: Head) -> Option<(MaterialIr, Vec<(String, Reg)>)> {
        let mut memo = BTreeMap::new();
        let mut reported = BTreeSet::new();
        let (graph_cost, samples) = self.load(output, false, &mut memo, &mut reported);
        let factor = if head.outline.is_some() {
            u64::from(NAMEPLATE_MATERIAL_OUTLINE_FACTOR)
        } else {
            1
        };
        let total = graph_cost.saturating_mul(factor);
        if total > u64::from(NAMEPLATE_MATERIAL_COST) {
            let issue = self
                .issue(IssueCode::MaterialCost, "nodes")
                .value("ops", total as f64)
                .value("limit", f64::from(NAMEPLATE_MATERIAL_COST))
                .value("graph", graph_cost as f64)
                .value("factor", factor as f64);
            self.push(issue);
        }
        if samples > u64::from(NAMEPLATE_MATERIAL_SAMPLES) {
            let issue = self
                .issue(IssueCode::MaterialSamples, "nodes")
                .value("samples", samples as f64)
                .value("limit", f64::from(NAMEPLATE_MATERIAL_SAMPLES));
            self.push(issue);
        }
        let reachable = self.reachable(output);
        let mut lanes = Vec::new();
        let mut lane_varies = Vec::new();
        let mut lane_of = vec![None; self.nodes.len()];
        for &i in &reachable {
            let set = &self.nodes[i].set;
            if self.nodes[i].kind.is(OpKind::Param) {
                let lane = Lane::Param(set.param as u16);
                let at = lanes.iter().position(|l| *l == lane).unwrap_or_else(|| {
                    lanes.push(lane);
                    lane_varies.push(false);
                    lanes.len() - 1
                });
                lane_of[i] = Some(at);
            } else if self.nodes[i].kind.is(OpKind::Expr) {
                lanes.push(Lane::Expr(set.program.unwrap_or(0) as u32));
                lane_varies.push(set.varies);
                lane_of[i] = Some(lanes.len() - 1);
            }
        }
        if reachable
            .iter()
            .any(|&i| self.nodes[i].kind.is(OpKind::BodyEdge))
        {
            let base = lanes.len();
            for edge in [
                EdgeLane::TopLeft,
                EdgeLane::TopRight,
                EdgeLane::BottomRight,
                EdgeLane::BottomLeft,
                EdgeLane::Rings,
            ] {
                lanes.push(Lane::BodyEdge(edge));
                lane_varies.push(false);
            }
            for &i in &reachable {
                if self.nodes[i].kind.is(OpKind::BodyEdge) {
                    lane_of[i] = Some(base);
                }
            }
        }
        if lanes.len() > NAMEPLATE_MATERIAL_INPUTS as usize {
            let issue = self
                .issue(IssueCode::MaterialInputs, "nodes")
                .value("inputs", lanes.len() as f64)
                .value("limit", f64::from(NAMEPLATE_MATERIAL_INPUTS));
            self.push(issue);
        }
        if !self.clean() {
            return None;
        }
        let mut textures: Vec<TexRef> = Vec::new();
        let mut texture_of = vec![None; self.nodes.len()];
        let mut sprites: Vec<Sprite> = Vec::new();
        let mut sprite_ids: Vec<usize> = Vec::new();
        let mut sprite_of = vec![None; self.nodes.len()];
        let mut slots = BTreeSet::new();
        if let Some(outline) = head.outline {
            slots.insert(outline.slot_index());
        }
        for &i in &reachable {
            let node = &self.nodes[i];
            if let Some(texture) = &node.set.texture {
                let at = textures
                    .iter()
                    .position(|t| t == texture)
                    .unwrap_or_else(|| {
                        textures.push(texture.clone());
                        textures.len() - 1
                    });
                texture_of[i] = Some(at);
            }
            match node.kind {
                Kind::Op(
                    OpKind::Slot | OpKind::Quantize | OpKind::DitherQuantize | OpKind::Cycle,
                ) => {
                    slots.insert(node.set.slot as u8);
                }
                Kind::Op(OpKind::Sprite) => {
                    let Some(source) = node.set.sprite else {
                        continue;
                    };
                    let at = sprite_ids
                        .iter()
                        .position(|&s| s == source)
                        .unwrap_or_else(|| {
                            sprite_ids.push(source);
                            sprites.push(self.ctx.sprites[source].clone().unwrap_or_default());
                            sprites.len() - 1
                        });
                    sprite_of[i] = Some(at);
                    for color in &sprites[at].colors {
                        if color.kind() == RefKind::Slot {
                            slots.insert(color.slot_index());
                        }
                    }
                }
                _ => {}
            }
        }
        let mut lower = Lower {
            nodes: &self.nodes,
            builder: self,
            masks: masks(&self.nodes),
            ops: Vec::new(),
            memo: BTreeMap::new(),
            converted: BTreeMap::new(),
            next_env: 1,
            lane_of: &lane_of,
            texture_of: &texture_of,
            sprite_of: &sprite_of,
        };
        let top = Env {
            pos: 0,
            frame: 0,
            index: 0,
            scope: 0,
        };
        let color_input = self.nodes[output].inputs[0];
        let coverage_input = self.nodes[output].inputs[1];
        let color = lower.arg(&color_input, top);
        let color = match lower.type_of(color) {
            Ty::Slot => lower.convert(color, Ty::Slot, Ty::Vec4, top.scope),
            _ => color,
        };
        let coverage = coverage_input
            .given
            .then(|| lower.arg(&coverage_input, top));
        let mut named = Vec::new();
        for &i in &reachable {
            if self.nodes[i].id.starts_with('~') {
                continue;
            }
            if let Some(reg) = lower.lookup(i, top) {
                named.push((self.nodes[i].id.clone(), reg));
            }
        }
        let ops = lower.ops;
        let period = period(&ops, color, coverage, &lane_varies, &sprites);
        let ir = MaterialIr {
            resolution: head.resolution,
            blend: self.nodes[output].set.blend,
            emissive: head.emissive,
            outline: head.outline,
            margins: head.margins,
            ops,
            color,
            coverage,
            lanes,
            textures,
            tone_slots: slots.into_iter().collect(),
            tone_hint: head.tone_hint,
            period,
            cost: u32::try_from(total).unwrap_or(u32::MAX),
            sprites,
            hint_colors: head.hint_colors,
        };
        Some((ir, named))
    }

    fn reachable(&self, root: usize) -> Vec<usize> {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            stack.extend(self.nodes[i].dependencies());
        }
        let mut found: Vec<usize> = (0..self.nodes.len()).filter(|&i| seen[i]).collect();
        found.sort_by_key(|&i| self.nodes[i].order);
        found
    }
}

struct Head {
    resolution: Resolution,
    margins: [u8; 4],
    emissive: bool,
    outline: Option<ColorRef>,
    tone_hint: Vec<ColorRef>,
    hint_colors: Vec<u32>,
}

fn visit(
    nodes: &[Node],
    i: usize,
    state: &mut [u8],
    order: &mut Vec<usize>,
    cycle: &mut Option<usize>,
) {
    match state[i] {
        1 => {
            cycle.get_or_insert(i);
        }
        2 => {}
        _ => {
            state[i] = 1;
            for dependency in nodes[i].dependencies() {
                if cycle.is_some() {
                    break;
                }
                visit(nodes, dependency, state, order, cycle);
            }
            state[i] = 2;
            order.push(i);
        }
    }
}

fn cone(nodes: &[Node], root: usize) -> Vec<usize> {
    let mut seen = vec![false; nodes.len()];
    let mut order = Vec::new();
    let mut stack = vec![(root, false)];
    while let Some((i, expanded)) = stack.pop() {
        if expanded {
            order.push(i);
            continue;
        }
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        stack.push((i, true));
        for input in nodes[i].inputs.iter().rev() {
            if let Some(n) = input.node {
                if !seen[n] {
                    stack.push((n, false));
                }
            }
        }
    }
    order
}

fn masks(nodes: &[Node]) -> Vec<u8> {
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&i| nodes[i].order);
    let mut masks = vec![0u8; nodes.len()];
    for i in order {
        let node = &nodes[i];
        let mut mask = env_bits(node.kind);
        for input in &node.inputs {
            if let Some(n) = input.node {
                mask |= masks[n];
            }
        }
        if let Some(source) = node.source {
            mask |= if node.kind.is(OpKind::Repeat) {
                masks[source] & !INDEX
            } else {
                masks[source]
            };
        }
        masks[i] = mask;
    }
    masks
}

fn expr_varies(program: &Program, interner: &Interner, source: &str) -> bool {
    let mut varying: Vec<u16> = interner.query_slot("frame").into_iter().collect();
    for (name, _) in SIGNALS.iter() {
        if let Some(slot) = interner.query_slot(&format!("{name}{TIME_SUFFIX}")) {
            varying.push(slot);
        }
    }
    program.reads().queries.iter().any(|q| varying.contains(q))
        || source.contains("random")
        || source.contains("die_roll")
}

#[derive(Clone, Copy, Debug)]
struct Env {
    pos: u32,
    frame: u32,
    index: u32,
    scope: u32,
}

struct Lower<'l, 'b, 'c> {
    nodes: &'l [Node],
    builder: &'l Builder<'b, 'c>,
    masks: Vec<u8>,
    ops: Vec<Op>,
    memo: BTreeMap<(usize, [u32; 3]), Reg>,
    converted: BTreeMap<(u16, u8, u32), Reg>,
    next_env: u32,
    lane_of: &'l [Option<usize>],
    texture_of: &'l [Option<usize>],
    sprite_of: &'l [Option<usize>],
}

fn ty_key(ty: Ty) -> u8 {
    match ty {
        Ty::Float => 0,
        Ty::Vec2 => 1,
        Ty::Vec3 => 2,
        Ty::Vec4 => 3,
        Ty::Slot => 4,
    }
}

impl Lower<'_, '_, '_> {
    fn key(&self, i: usize, env: Env) -> (usize, [u32; 3]) {
        let mask = self.masks[i];
        let pick = |bit: u8, id: u32| if mask & bit != 0 { id } else { NO_ENV };
        (
            i,
            [
                pick(POS, env.pos),
                pick(FRAME, env.frame),
                pick(INDEX, env.index),
            ],
        )
    }

    fn lookup(&self, i: usize, env: Env) -> Option<Reg> {
        self.memo.get(&self.key(i, env)).copied()
    }

    fn type_of(&self, reg: Reg) -> Ty {
        self.ops
            .get(usize::from(reg.0))
            .map_or(Ty::Float, |op| op.ty)
    }

    fn emit(&mut self, kind: OpKind, ty: Ty, args: [Reg; 4], imm: [f64; 4]) -> Reg {
        let dst = Reg(self.ops.len() as u16);
        self.ops.push(Op {
            kind,
            ty,
            dst,
            args,
            imm,
        });
        dst
    }

    fn fresh(&mut self) -> u32 {
        self.next_env += 1;
        self.next_env
    }

    fn value(&mut self, i: usize, env: Env) -> Reg {
        let key = self.key(i, env);
        if let Some(found) = self.memo.get(&key) {
            return *found;
        }
        let nodes = self.nodes;
        let node = &nodes[i];
        let Kind::Op(kind) = node.kind else {
            return Reg::NONE;
        };
        let mut args = [Reg::NONE; 4];
        for (slot, input) in args.iter_mut().zip(&node.inputs) {
            *slot = self.arg(input, env);
        }
        let reg = match kind {
            OpKind::SampleAt | OpKind::Repeat => self.body(i, kind, args, env),
            _ => {
                let imm =
                    self.builder
                        .imm(i, self.lane_of[i], self.texture_of[i], self.sprite_of[i]);
                self.emit(kind, node.ty, args, imm)
            }
        };
        self.memo.insert(key, reg);
        reg
    }

    fn body(&mut self, i: usize, kind: OpKind, mut args: [Reg; 4], env: Env) -> Reg {
        let nodes = self.nodes;
        let node = &nodes[i];
        let Some(source) = node.source else {
            return Reg::NONE;
        };
        let zero = |input: Option<&Input>| {
            input.is_some_and(|input| {
                input.node.is_none() && input.literal.is_some_and(|l| l.values == [0.0; 4])
            })
        };
        let changed = if kind == OpKind::Repeat {
            INDEX
        } else {
            let mut bits = 0;
            if !zero(node.inputs.first()) {
                bits |= POS;
            }
            if !zero(node.inputs.get(1)) {
                bits |= FRAME;
            }
            bits
        };
        let repeat_free = env.index == 0;
        for j in cone_with_sources(self.nodes, source) {
            let mask = self.masks[j];
            if mask & changed == 0 && !(repeat_free && mask & INDEX != 0) {
                self.value(j, env);
            }
        }
        let inner = Env {
            pos: if changed & POS != 0 {
                self.fresh()
            } else {
                env.pos
            },
            frame: if changed & FRAME != 0 {
                self.fresh()
            } else {
                env.frame
            },
            index: if changed & INDEX != 0 {
                self.fresh()
            } else {
                env.index
            },
            scope: self.fresh(),
        };
        let start = self.ops.len();
        let result = self.value(source, inner);
        let end = self.ops.len();
        let s = &node.set;
        let imm = if kind == OpKind::Repeat {
            [start as f64, end as f64, s.count as f64, s.reduce.value()]
        } else {
            [start as f64, end as f64, 0.0, 0.0]
        };
        if kind == OpKind::Repeat {
            args = [result, Reg::NONE, Reg::NONE, Reg::NONE];
        } else {
            args[2] = result;
        }
        self.emit(kind, node.ty, args, imm)
    }

    fn arg(&mut self, input: &Input, env: Env) -> Reg {
        let Some(target) = input.target else {
            return Reg::NONE;
        };
        if let Some(literal) = input.literal {
            let width = usize::from(target.width());
            let mut values = [0.0; 4];
            for (c, value) in values.iter_mut().enumerate().take(width) {
                *value = if literal.width == 1 {
                    literal.values[0]
                } else {
                    literal.values[c]
                };
            }
            return self.emit(OpKind::Constant, target, [Reg::NONE; 4], values);
        }
        let Some(node) = input.node else {
            return Reg::NONE;
        };
        let reg = self.value(node, env);
        let from = self.nodes[node].ty;
        self.convert(reg, from, target, env.scope)
    }

    fn convert(&mut self, reg: Reg, from: Ty, to: Ty, scope: u32) -> Reg {
        if from == to || reg.is_none() {
            return reg;
        }
        let key = (reg.0, ty_key(to), scope);
        if let Some(found) = self.converted.get(&key) {
            return *found;
        }
        let made = if from == Ty::Slot && to == Ty::Vec4 {
            self.emit(
                OpKind::Slot,
                Ty::Vec4,
                [reg, Reg::NONE, Reg::NONE, Reg::NONE],
                [0.0; 4],
            )
        } else if from == Ty::Float {
            let mut args = [Reg::NONE; 4];
            for slot in args.iter_mut().take(usize::from(to.width())) {
                *slot = reg;
            }
            self.emit(OpKind::Combine, to, args, [0.0; 4])
        } else {
            reg
        };
        self.converted.insert(key, made);
        made
    }
}

fn cone_with_sources(nodes: &[Node], root: usize) -> Vec<usize> {
    let mut seen = vec![false; nodes.len()];
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        stack.extend(nodes[i].dependencies());
    }
    let mut found: Vec<usize> = (0..nodes.len()).filter(|&i| seen[i]).collect();
    found.sort_by_key(|&i| nodes[i].order);
    found
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wave {
    Still,
    Ramp,
    Every(u64),
    Unknown,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn join_wave(a: Wave, b: Wave) -> Wave {
    match (a, b) {
        (Wave::Unknown, _) | (_, Wave::Unknown) | (Wave::Ramp, _) | (_, Wave::Ramp) => {
            Wave::Unknown
        }
        (Wave::Still, other) | (other, Wave::Still) => other,
        (Wave::Every(p), Wave::Every(q)) => {
            let lcm = p / gcd(p, q) * q;
            if lcm > PERIOD_CAP {
                Wave::Unknown
            } else {
                Wave::Every(lcm)
            }
        }
    }
}

fn every(period: f64) -> Wave {
    if period >= 1.0 && period.fract() == 0.0 && period <= PERIOD_CAP as f64 {
        Wave::Every(period as u64)
    } else {
        Wave::Unknown
    }
}

fn constant(ops: &[Op], reg: Reg, c: usize) -> Option<f64> {
    let op = ops.get(usize::from(reg.0))?;
    match op.kind {
        OpKind::Constant => Some(op.imm[c.min(3)]),
        OpKind::Combine => constant(ops, op.args[c.min(3)], 0),
        _ => None,
    }
}

fn period(
    ops: &[Op],
    color: Reg,
    coverage: Option<Reg>,
    lane_varies: &[bool],
    sprites: &[Sprite],
) -> Period {
    let mut waves: Vec<[Wave; 4]> = Vec::with_capacity(ops.len());
    for op in ops {
        let w = wave(op, &waves, ops, lane_varies, sprites);
        waves.push(w);
    }
    let get = |reg: Reg| -> [Wave; 4] {
        waves
            .get(usize::from(reg.0))
            .copied()
            .unwrap_or([Wave::Still; 4])
    };
    let mut total = get(color).into_iter().fold(Wave::Still, join_wave);
    if let Some(coverage) = coverage {
        total = join_wave(total, get(coverage)[0]);
    }
    match total {
        Wave::Still => Period::Still,
        Wave::Every(p) => Period::Frames(p as u32),
        _ => Period::Unknown,
    }
}

fn wave(
    op: &Op,
    waves: &[[Wave; 4]],
    ops: &[Op],
    lane_varies: &[bool],
    sprites: &[Sprite],
) -> [Wave; 4] {
    let get = |reg: Reg| -> [Wave; 4] {
        if reg.is_none() {
            [Wave::Still; 4]
        } else {
            waves
                .get(usize::from(reg.0))
                .copied()
                .unwrap_or([Wave::Unknown; 4])
        }
    };
    let width = usize::from(op.ty.width());
    let a = get(op.args[0]);
    let b = get(op.args[1]);
    let all = || {
        op.args
            .iter()
            .flat_map(|r| get(*r))
            .fold(Wave::Still, join_wave)
    };
    let fill = |w: Wave| {
        let mut out = [Wave::Still; 4];
        for slot in out.iter_mut().take(width) {
            *slot = w;
        }
        out
    };
    let per = |f: &dyn Fn(usize) -> Wave| {
        let mut out = [Wave::Still; 4];
        for (c, slot) in out.iter_mut().enumerate().take(width) {
            *slot = f(c);
        }
        out
    };
    let ramp_or = |c: usize, ramp: Wave| match (a[c], b[c]) {
        (Wave::Ramp, Wave::Ramp) | (Wave::Ramp, Wave::Still) | (Wave::Still, Wave::Ramp) => ramp,
        (x, y) => join_wave(x, y),
    };
    match op.kind {
        OpKind::Frame => fill(Wave::Ramp),
        OpKind::SteppedTime => fill(Wave::Unknown),
        OpKind::Expr => {
            let lane = op.imm[0] as usize;
            fill(if lane_varies.get(lane).copied().unwrap_or(true) {
                Wave::Unknown
            } else {
                Wave::Still
            })
        }
        OpKind::Add | OpKind::Sub => per(&|c| ramp_or(c, Wave::Ramp)),
        OpKind::Mul => per(&|c| {
            let integer = |reg: Reg| constant(ops, reg, c).is_some_and(|v| v.fract() == 0.0);
            match (a[c], b[c]) {
                (Wave::Ramp, Wave::Still) if integer(op.args[1]) => Wave::Ramp,
                (Wave::Still, Wave::Ramp) if integer(op.args[0]) => Wave::Ramp,
                (x, y) => join_wave(x, y),
            }
        }),
        OpKind::Floor => per(&|c| a[c]),
        OpKind::Frac => per(&|c| {
            if a[c] == Wave::Ramp {
                Wave::Still
            } else {
                a[c]
            }
        }),
        OpKind::Mod => per(&|c| match (a[c], constant(ops, op.args[1], c)) {
            (Wave::Ramp, Some(m)) if m > 0.0 => every(m),
            _ => join_wave(a[c], b[c]),
        }),
        OpKind::Cycle => {
            let tones = op.imm[1];
            let contribution = |w: Wave| {
                if w == Wave::Ramp {
                    every(tones)
                } else {
                    w
                }
            };
            fill(join_wave(contribution(a[0]), contribution(b[0])))
        }
        OpKind::Noise | OpKind::Fbm => {
            let axis = |c: usize| match a[c] {
                Wave::Ramp if op.imm[1 + c] > 0.0 => every(op.imm[1 + c]),
                other => other,
            };
            fill(join_wave(axis(0), axis(1)))
        }
        OpKind::Stripes => {
            let axis = |c: usize| match a[c] {
                Wave::Ramp if op.imm[2 + c] == 0.0 => Wave::Still,
                Wave::Ramp => every(op.imm[0]),
                other => other,
            };
            fill(join_wave(axis(0), axis(1)))
        }
        OpKind::Checker => {
            let axis = |c: usize| match a[c] {
                Wave::Ramp => every(2.0 * op.imm[0]),
                other => other,
            };
            fill(join_wave(axis(0), axis(1)))
        }
        OpKind::Bayer => {
            let axis = |c: usize| {
                if a[c] == Wave::Ramp {
                    Wave::Every(4)
                } else {
                    a[c]
                }
            };
            fill(join_wave(axis(0), axis(1)))
        }
        OpKind::DitherQuantize => {
            let axis = |c: usize| {
                if b[c] == Wave::Ramp {
                    Wave::Every(4)
                } else {
                    b[c]
                }
            };
            fill(join_wave(a[0], join_wave(axis(0), axis(1))))
        }
        OpKind::Sprite => {
            let frames = sprites
                .get(op.imm[0] as usize)
                .map_or(1, |s| s.frames.len().max(1));
            let frame = if b[0] == Wave::Ramp {
                every(frames as f64)
            } else {
                b[0]
            };
            fill(join_wave(join_wave(a[0], a[1]), frame))
        }
        OpKind::Combine => {
            let mut out = [Wave::Still; 4];
            for (c, slot) in out.iter_mut().enumerate().take(width) {
                *slot = get(op.args[c])[0];
            }
            out
        }
        OpKind::Component => fill(a[(op.imm[0] as usize).min(3)]),
        OpKind::SampleAt => {
            let result = get(op.args[2]);
            let shift = b[0];
            let offset = join_wave(a[0], a[1]);
            per(&|c| {
                let moved = match (result[c], shift) {
                    (w, Wave::Ramp) => w,
                    (Wave::Ramp, Wave::Still) => Wave::Ramp,
                    (w, s) => join_wave(w, s),
                };
                if moved == Wave::Ramp && offset == Wave::Still {
                    Wave::Ramp
                } else {
                    join_wave(moved, offset)
                }
            })
        }
        OpKind::Repeat => {
            let result = get(op.args[0]);
            per(&|c| join_wave(result[c], Wave::Still))
        }
        OpKind::PixelPos
        | OpKind::PixelCell
        | OpKind::TagUv
        | OpKind::AreaSize
        | OpKind::BodyRect
        | OpKind::ScreenPos
        | OpKind::PixelScreenPos
        | OpKind::Constant
        | OpKind::Param
        | OpKind::RepeatIndex => fill(Wave::Still),
        OpKind::Slot if op.args[0].is_none() => fill(Wave::Still),
        _ => fill(all()),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use serde_json::{json, Value};

    use super::GraphContext;
    use crate::nameplate::image::ColorRef;
    use crate::nameplate::types::{MolangSet, Param, Sprite, SPRITE_CLEAR};
    use crate::palette::{Palette, Slot};

    pub(crate) const FILL: [u32; 4] = [0xff11_2233, 0xff44_5566, 0xff77_8899, 0xffaa_bbcc];
    pub(crate) const GLOW: [u32; 2] = [0x80ff_0000, 0xff00_ff00];

    pub(crate) struct Fixture {
        pub(crate) palette: Palette,
        pub(crate) params: Vec<Param>,
        pub(crate) textures: Vec<String>,
        pub(crate) sprites: Vec<Option<Sprite>>,
        pub(crate) molang: MolangSet,
    }

    fn slot(id: &str, ramp: &[u32]) -> Slot {
        Slot {
            id: id.to_string(),
            name: id.to_string(),
            ramp: ramp.to_vec(),
            stops: ramp.to_vec(),
            interpolate: None,
            customizable: false,
        }
    }

    fn sprite(id: &str, side: u8, colors: Vec<ColorRef>, frames: Vec<Vec<u8>>) -> Option<Sprite> {
        Some(Sprite {
            id: id.to_string(),
            width: side,
            height: side,
            colors,
            frames,
        })
    }

    impl Fixture {
        pub(crate) fn new() -> Fixture {
            let star = vec![vec![0, 1, SPRITE_CLEAR, 0], vec![1, 1, 1, 1]];
            Fixture {
                palette: Palette {
                    slots: vec![slot("fill", &FILL), slot("glow", &GLOW)],
                    colorways: Vec::new(),
                },
                params: vec![Param {
                    id: "speed".to_string(),
                    value: 2.0,
                    min: 0.0,
                    max: 4.0,
                }],
                textures: vec!["dots".to_string()],
                sprites: vec![
                    sprite(
                        "star",
                        2,
                        vec![ColorRef::slot(0, 1), ColorRef::slot(1, 0)],
                        star,
                    ),
                    None,
                    sprite(
                        "huge",
                        64,
                        vec![ColorRef::slot(0, 0)],
                        vec![vec![0; 64 * 64]; 2],
                    ),
                ],
                molang: MolangSet::default(),
            }
        }
    }

    pub(crate) fn context(fixture: &mut Fixture) -> GraphContext<'_> {
        GraphContext {
            palette: &fixture.palette,
            params: &fixture.params,
            textures: &fixture.textures,
            sprites: &fixture.sprites,
            molang: &mut fixture.molang,
        }
    }

    pub(crate) fn nodes(source: &str) -> Value {
        let node = |text: &str| {
            let mut words = text.split(' ');
            let (id, kind) = words.next().unwrap().split_once(':').unwrap();
            let mut node = json!({ "id": id, "type": kind });
            for word in words {
                let (key, raw) = word.split_once('=').unwrap();
                let value =
                    serde_json::from_str(&raw.replace('\'', "\"")).unwrap_or_else(|_| json!(raw));
                match key.strip_prefix('.') {
                    Some(key) => node["set"][key] = value,
                    None => node["in"][key] = value,
                }
            }
            node
        };
        Value::Array(source.split("; ").map(node).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use test_support::{context, nodes, Fixture};

    fn built(mode: Mode, file: Value) -> (Option<MaterialIr>, Vec<(IssueCode, String)>, Fixture) {
        let mut fixture = Fixture::new();
        let mut report = Report::new(mode);
        let ir = build(
            "materials/m.json",
            &file,
            &mut context(&mut fixture),
            &mut report,
        );
        let issues = report
            .issues()
            .iter()
            .map(|i| (i.code, i.path.clone()))
            .collect();
        (ir, issues, fixture)
    }

    fn ok(source: &str) -> MaterialIr {
        let (ir, issues, _) = built(Mode::Client, json!({ "nodes": nodes(source) }));
        assert!(issues.is_empty(), "{source}: {issues:?}");
        ir.expect("built")
    }

    fn fails(rows: &[&str]) {
        for row in rows {
            let (source, expected) = row.split_once(" => ").unwrap();
            let (code, path) = expected.split_once(' ').unwrap();
            let (ir, issues, _) = built(Mode::Client, json!({ "nodes": nodes(source) }));
            let path = format!("materials/m.json#{path}");
            assert!(ir.is_none(), "{row}");
            assert!(
                issues.iter().any(|(c, p)| c.as_str() == code && *p == path),
                "{row}: {issues:?}"
            );
        }
    }

    #[test]
    fn a_plain_slot_material_lowers_to_a_tone_lookup() {
        let ir = ok("c:Slot .slot=glow[1]; out:Output color=c");
        let ops: Vec<(OpKind, Ty, Reg)> =
            ir.ops.iter().map(|op| (op.kind, op.ty, op.dst)).collect();
        assert_eq!(
            ops,
            [
                (OpKind::Slot, Ty::Slot, Reg(0)),
                (OpKind::Slot, Ty::Vec4, Reg(1))
            ]
        );
        let actual = &(ir.ops[0].imm[0], ir.ops[1].args[0], ir.color);
        assert_eq!(*actual, (17.0, Reg(0), Reg(1)));
        let actual = &(ir.coverage, ir.tone_slots.as_slice(), ir.period, ir.cost);
        assert_eq!(*actual, (None, &[1][..], Period::Still, 1));
    }

    #[test]
    fn the_material_file_sets_the_template_fields() {
        let file = json!({ "resolution": "smooth", "margins": [1, 2, 3, 4], "outline": "fill[3]", "emissive": true,
            "toneHint": ["glow", "#102030", "#80ffffff"], "nodes": nodes("out:Output color=[1,0,0,1] coverage=0.5 .blend=additive") });
        let (ir, issues, _) = built(Mode::Authoring, file);
        let ir = ir.expect("built");
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(
            (ir.resolution, ir.margins, ir.outline),
            (Resolution::Smooth, [1, 2, 3, 4], Some(ColorRef::slot(0, 3)))
        );
        assert!(ir.emissive && ir.coverage.is_some() && ir.blend == Blend::Additive);
        assert_eq!(
            (ir.tone_hint.as_slice(), ir.hint_colors.as_slice()),
            (&[ColorRef::slot(1, 0)][..], &[0xff10_2030, 0x80ff_ffff][..])
        );
        assert_eq!((ir.tone_slots.as_slice(), ir.cost), (&[0][..], 5));

        let file = json!({ "resolution": "blurry", "margins": [0, 33, 0, 0], "outline": "nope", "toneHint": ["#12"], "shader": "x", "nodes": nodes("out:Output color=c") });
        let (ir, issues, _) = built(Mode::Authoring, file);
        assert!(ir.is_none());
        let paths: Vec<String> = issues
            .iter()
            .map(|(c, p)| format!("{} {p}", c.as_str()))
            .collect();
        for expected in "unknown_key shader,material resolution,material_margins margins,material outline,material toneHint[0]".split(',') {
            let (code, at) = expected.split_once(' ').unwrap();
            assert!(paths.contains(&format!("{code} materials/m.json#{at}")), "{expected}: {paths:?}");
        }
    }

    #[test]
    fn broken_graphs_have_codes_and_paths() {
        fails(&[
            "r:Lighten color=1.0 amount=0.5; out:Output color=r => node_typing nodes.r",
            "a:Constant .value=[1,2]; b:Constant .value=[1,2,3]; r:add a=a b=b; c:Combine x=r y=0; out:Output color=c => node_typing nodes.r",
            "s:Slot .slot=fill; r:Select condition=1 a=s b=[1,2]; out:Output color=r => node_typing nodes.r",
            "s:Slot .slot=fill; r:Repeat .source=s .count=2 .reduce=sum; out:Output color=r => node_typing nodes.r",
            "r:Constant .value=1; out:Output color=r => node_typing nodes.out",
            "r:Wobble; out:Output color=r => node_type nodes.r.type",
            "r:abs x=ghost; out:Output color=r => node_link nodes.r.in.x",
            "r:abs x='true'; out:Output color=r => node_link nodes.r.in.x",
            "BadId:Frame; out:Output color=r => node_id nodes",
            "a:abs x=b; b:abs x=a; out:Output color=a => material_cycle nodes.a",
            "c:Slot .slot=fill; out:Output color=c; out2:Output color=c => material_outputs nodes",
            "r:abs x=out; out:Output color=r => node_output_link nodes.r.in.x",
            "r:BundleTexture .texture=nope; out:Output color=r => node_texture nodes.r.set.texture",
            "r:Slot .slot=fill[9]; out:Output color=r => node_setting nodes.r.set.slot",
            "r:Fbm p=[0,0] .octaves=9; out:Output color=r => node_setting nodes.r.set.octaves",
            "r:SampleAt .source=ghost; out:Output color=r => node_link nodes.r.set.source",
            "i:RepeatIndex; r:Repeat .source=i .count=17; c:Hsv h=r; out:Output color=c => node_setting nodes.r.set.count",
            "i:RepeatIndex; r:Repeat .source=i .count=2.5; c:Hsv h=r; out:Output color=c => node_setting nodes.r.set.count",
            "i:RepeatIndex; inner:Repeat .source=i .count=2; outer:Repeat .source=inner .count=2; c:Hsv h=outer; out:Output color=c => node_repeat nodes.inner",
            "i:RepeatIndex; inner:Repeat .source=i .count=2; moved:SampleAt .source=inner; outer:Repeat .source=moved .count=2; c:Hsv h=outer; out:Output color=c => node_repeat nodes.inner",
            "i:RepeatIndex; c:Hsv h=i; out:Output color=c => node_repeat nodes.i",
            "t:BundleTexture .texture=dots; r:Repeat .source=t .count=5; out:Output color=r => material_samples nodes",
        ]);
        let source = "r:Frame x=1 .speed=2; c:Hsv h=r; out:Output color=c";
        let (ir, issues, _) = built(Mode::Client, json!({ "nodes": nodes(source) }));
        assert!(ir.is_some() && issues.is_empty());
        let (ir, issues, _) = built(Mode::Authoring, json!({ "nodes": nodes(source) }));
        assert!(ir.is_none());
        assert!(issues.contains(&(
            IssueCode::NodeInput,
            "materials/m.json#nodes.r.in.x".to_string()
        )));
        assert!(issues.contains(&(
            IssueCode::NodeSetting,
            "materials/m.json#nodes.r.set.speed".to_string()
        )));
    }

    #[test]
    fn repeat_and_sample_bodies_sit_right_before_their_op() {
        let ir = ok("i:RepeatIndex; p:PixelPos; x:Component v=p; sum:add a=i b=x; r:Repeat .source=sum .count=3 .reduce=sum; \
            s:SampleAt offset=[1,0] .source=r; c:Hsv h=s; out:Output color=c");
        for (k, op) in ir.ops.iter().enumerate() {
            let Some(body) = op.body() else { continue };
            assert!(body.end == k && op.body_result().0 < k as u16, "{op:?}");
            assert!(ir.ops[body]
                .iter()
                .flat_map(|o| o.args)
                .all(|a| a.is_none() || usize::from(a.0) < k));
        }
        let body = |kind| {
            ir.ops
                .iter()
                .find(|op| op.kind == kind)
                .unwrap()
                .body()
                .unwrap()
        };
        let (repeat, sample) = (body(OpKind::Repeat), body(OpKind::SampleAt));
        assert!(sample.start <= repeat.start && repeat.end < sample.end);
        let inner: Vec<OpKind> = ir.ops[repeat].iter().map(|op| op.kind).collect();
        assert!(inner.contains(&OpKind::RepeatIndex) && !inner.contains(&OpKind::PixelPos));
    }

    #[test]
    fn costs_samples_inputs_and_nodes_are_capped() {
        let fbm: Vec<String> = (0..5)
            .map(|i| format!("f{i}:Fbm p=[{i},0] .octaves=8"))
            .collect();
        let sums = "a:add a=f0 b=f1; b:add a=f2 b=f3; c:add a=a b=b; d:add a=c b=f4; h:Hsv h=d; out:Output color=h";
        fails(&[format!("{}; {sums} => material_cost nodes", fbm.join("; ")).as_str()]);
        assert_eq!(
            ok("t:BundleTexture .texture=dots; r:Repeat .source=t .count=4; out:Output color=r")
                .textures,
            [TexRef::Bundle(0)]
        );
        let widest =
            ok("i:RepeatIndex; r:Repeat .source=i .count=16; c:Hsv h=r; out:Output color=c");
        let op = widest
            .ops
            .iter()
            .find(|op| op.kind == OpKind::Repeat)
            .unwrap();
        let actual = &(op.imm[2], Reduce::from_value(op.imm[3]));
        assert_eq!(*actual, (16.0, Reduce::Max));

        let chain = |count: usize| {
            let terms: Vec<String> = (0..count)
                .map(|i| {
                    format!(
                        "e{i}:Expr .expr='{i}+q.width'; s{i}:add a={} b=e{i}",
                        if i == 0 {
                            "edge".to_string()
                        } else {
                            format!("s{}", i - 1)
                        }
                    )
                })
                .collect();
            format!(
                "edge:BodyEdge; {}; h:Hsv h=s{}; out:Output color=h",
                terms.join("; "),
                count - 1
            )
        };
        let ir = ok(&chain(11));
        assert_eq!(
            (ir.lanes.len(), ir.lanes[11], ir.lanes[15]),
            (
                16,
                Lane::BodyEdge(EdgeLane::TopLeft),
                Lane::BodyEdge(EdgeLane::Rings)
            )
        );
        fails(&[format!("{} => material_inputs nodes", chain(12)).as_str()]);
        let many: Vec<String> = (0..64).map(|i| format!("n{i}:Frame")).collect();
        fails(&[format!(
            "{}; out:Output color=n0 => material_nodes nodes",
            many.join("; ")
        )
        .as_str()]);

        let squares = nodes("p:pow x=[1,1,1,1] y=2; q:pow x=p y=2; out:Output color=q");
        let (plain, _, _) = built(Mode::Client, json!({ "nodes": squares }));
        assert_eq!(plain.unwrap().cost, 1 + 280 * 2);
        let mut fixture = Fixture::new();
        let mut report = Report::new(Mode::Client);
        assert!(build(
            "m",
            &json!({ "outline": "fill", "nodes": squares }),
            &mut context(&mut fixture),
            &mut report
        )
        .is_none());
        assert_eq!(report.issues()[0].code, IssueCode::MaterialCost);
        let actual = &report.issues()[0].values[0];
        assert_eq!(*actual, ("ops".to_string(), 561.0 * 5.0));
    }

    #[test]
    fn params_and_expressions_take_lanes() {
        let source = "a:Param .name=speed; b:Param .name=speed; e:Expr .expr=q.width*2; s:add a=a b=b; t:add a=s b=e; h:Hsv h=t; out:Output color=h";
        let (ir, _, fixture) = built(Mode::Client, json!({ "nodes": nodes(source) }));
        assert_eq!(ir.unwrap().lanes, [Lane::Param(0), Lane::Expr(0)]);
        let programs = &fixture.molang.programs;
        let actual = &(programs.len(), programs[0].scope);
        assert_eq!(*actual, (1, MolangUse::Material));
        assert_eq!(programs[0].path, "materials/m.json#nodes.e.set.expr");
    }

    #[test]
    fn periods_are_detected_from_the_graph() {
        for row in [
            "c:Slot .slot=fill; out:Output color=c => still",
            "f:Frame; m:mod a=f b=4; h:Hsv h=m; out:Output color=h => 4",
            "f:Frame; x:PixelPos; x0:Component v=x; p:Combine x=x0 y=f; n:Noise p=p .period=[0,6]; m:mod a=f b=4; s:add a=n b=m; h:Hsv h=s; out:Output color=h => 12",
            "f:Frame; s:mul a=f b=0.5; m:mod a=s b=3; h:Hsv h=m; out:Output color=h => unknown",
            "s:Sprite frame=f .sprite=star; f:Frame; out:Output color=s => 2",
        ] {
            let (source, expected) = row.split_once(" => ").unwrap();
            let period = match expected {
                "still" => Period::Still,
                "unknown" => Period::Unknown,
                frames => Period::Frames(frames.parse().unwrap()),
            };
            assert_eq!(ok(source).period, period, "{row}");
        }
    }
}
