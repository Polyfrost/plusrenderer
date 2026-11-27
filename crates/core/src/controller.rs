use std::cell::RefCell;
use std::mem;
use std::ops::Range;

use serde_json::{Map, Value};

use crate::animation::{
    has_timeline, varies, Animation, Disabled, KeyEventKind, Loop, KEPT_DELAY_FLOATS,
};
use crate::issues::{join, path, Issue, IssueCode, Report};
use crate::molang::{
    compile_with, random_frame, Interner, NoQueries, Options, Program, QueryScope, Rules, Scope,
    Warnings, FRAME_FUEL,
};
use crate::pose::{BonePose, Pose};
use crate::reactions::Speed;
use crate::wearer::WearerRecord;

pub const MAX_STATES: usize = 32;
pub const MAX_CONTROLLERS: usize = 8;
pub const MAX_TRANSITIONS_PER_FRAME: usize = 4;
pub const SLOTS_PER_CONTROLLER: usize = 2;
pub const CATCH_UP_SECONDS: f32 = 0.05;
pub const CATCH_UP_STEPS: u32 = 1_200;
pub const CATCH_UP_SNAPSHOTS: usize = 64;
pub const CATCH_UP_FRESH_SECONDS: f32 = 1.0;
pub const CATCH_UP_FUEL: u32 = 64 * FRAME_FUEL;
pub const CATCH_UP_GRACE_SECONDS: f32 = 0.25;
const CATCH_UP_PRUNE: usize = 256;

pub fn fresh_play(since_start: f32) -> bool {
    since_start <= CATCH_UP_GRACE_SECONDS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Particle,
    Sound,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub kind: EffectKind,
    pub name: String,
    pub locator: Option<String>,
    pub bind_to_actor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatchUpKeep {
    pub seconds: f32,
    pub window: f32,
    pub spacing: f32,
}

impl CatchUpKeep {
    pub const NONE: CatchUpKeep = CatchUpKeep {
        seconds: 0.0,
        window: 0.0,
        spacing: 0.0,
    };

    fn unknown(kind: EffectKind) -> Self {
        match kind {
            EffectKind::Particle => CatchUpKeep {
                seconds: f32::INFINITY,
                ..CatchUpKeep::NONE
            },
            EffectKind::Sound => CatchUpKeep::NONE,
        }
    }

    fn shows(&self, ago: f32, next: Option<f32>) -> bool {
        ago <= self.seconds
            && (self.seconds.is_finite() || next.is_none_or(|next| next <= self.window))
    }
}

type Kept = (CatchUpKeep, [usize; 2]);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectEvent {
    pub controller: usize,
    pub state: usize,
    pub kind: EffectKind,
    pub effect: usize,
    pub ago: f32,
    pub emitter: Option<u16>,
    pub fresh: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Blend {
    Instant,
    Seconds(f32),
    Curve(Vec<[f32; 2]>),
}

#[derive(Clone, Debug)]
pub struct StateAnimation {
    pub name: String,
    weight: Program,
    index: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub target: usize,
    condition: Program,
}

#[derive(Clone, Debug)]
struct StateVariable {
    slot: u16,
    input: Program,
    curve: Vec<[f32; 2]>,
}

#[derive(Clone, Debug)]
pub struct State {
    pub name: String,
    pub animations: Vec<StateAnimation>,
    pub transitions: Vec<Transition>,
    pub blend: Blend,
    pub shortest_path: bool,
    pub effects: Vec<Effect>,
    on_entry: Vec<Program>,
    on_exit: Vec<Program>,
    variables: Vec<StateVariable>,
    anim_time_update: Option<Program>,
}

impl State {
    fn programs(&self) -> impl Iterator<Item = &Program> {
        self.transitions
            .iter()
            .map(|t| &t.condition)
            .chain(&self.on_entry)
            .chain(&self.on_exit)
            .chain(self.variables.iter().map(|v| &v.input))
            .chain(&self.anim_time_update)
            .chain(self.animations.iter().map(|entry| &entry.weight))
    }
}

#[derive(Clone, Debug)]
pub struct Controller {
    pub file: String,
    pub name: String,
    pub initial: usize,
    pub states: Vec<State>,
}

const ROOT_KEYS: &[&str] = &["format_version", "animation_controllers"];
const CONTROLLER_KEYS: &[&str] = &["initial_state", "states"];
const STATE_KEYS: &[&str] = &[
    "animations",
    "transitions",
    "blend_transition",
    "blend_via_shortest_path",
    "on_entry",
    "on_exit",
    "particle_effects",
    "sound_effects",
    "variables",
    "anim_time_update",
];
const EFFECT_KEYS: &[&str] = &["effect", "locator", "bind_to_actor"];
const VARIABLE_KEYS: &[&str] = &["input", "remap_curve"];
const CONTROLLERS: &str = "animation_controllers";
const STATE_MASK: u32 = 0xff;
const PREVIOUS_SHIFT: u32 = 8;

fn server_side(query: &str) -> bool {
    query.contains("component") || query == "property" || query == "has_property"
}

impl Blend {
    pub fn outgoing_weight(&self, elapsed: f32) -> f32 {
        let weight = match self {
            Blend::Instant => 0.0,
            Blend::Seconds(seconds) if *seconds > 0.0 => 1.0 - elapsed / seconds,
            Blend::Seconds(_) => 0.0,
            Blend::Curve(keys) => match keys.last() {
                Some(last) if elapsed <= last[0] => sample_curve(keys, elapsed),
                _ => 0.0,
            },
        };
        weight.clamp(0.0, 1.0)
    }
}

fn sample_curve(keys: &[[f32; 2]], x: f32) -> f32 {
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
        return x;
    };
    if x <= first[0] {
        return first[1];
    }
    if x >= last[0] {
        return last[1];
    }
    let next = keys.partition_point(|k| k[0] <= x);
    let (a, b) = (keys[next - 1], keys[next]);
    let span = b[0] - a[0];
    if span <= 0.0 {
        return b[1];
    }
    a[1] + (b[1] - a[1]) * (x - a[0]) / span
}

struct Reader<'a> {
    file: &'a str,
    interner: &'a mut Interner,
    rules: Rules,
    report: &'a mut Report,
    broken: bool,
}

impl Reader<'_> {
    fn issue(&self, code: IssueCode, at: &str) -> Issue {
        Issue::error(code, path(self.file, at))
    }

    fn push(&mut self, issue: Issue) {
        self.broken = true;
        self.report.push(issue);
    }

    fn fail(&mut self, code: IssueCode, at: &str) {
        let issue = self.issue(code, at);
        self.push(issue);
    }

    fn keys(&mut self, at: &str, object: &Map<String, Value>, known: &[&str]) {
        for key in object.keys() {
            if !known.contains(&key.as_str()) {
                let issue = self
                    .issue(IssueCode::ControllerUnsupported, &join(at, key))
                    .name("key", key);
                self.push(issue);
            }
        }
    }

    fn compile(&mut self, at: &str, source: &str) -> Option<Program> {
        let start = source.trim_start();
        if start.starts_with('/') || start.starts_with('@') {
            let issue = self
                .issue(IssueCode::ControllerServerSide, at)
                .name("source", source);
            self.push(issue);
            return None;
        }
        let denied: RefCell<Option<String>> = RefCell::new(None);
        let filter = |name: &str| {
            let server = server_side(name);
            if server && denied.borrow().is_none() {
                *denied.borrow_mut() = Some(name.to_string());
            }
            !server
        };
        let options = Options {
            rules: self.rules,
            queries: QueryScope::Filter(&filter),
        };
        let compiled = compile_with(source, self.interner, &options, &mut Warnings::default());
        if let Some(query) = denied.take() {
            let issue = self
                .issue(IssueCode::ControllerServerSide, at)
                .name("query", &query);
            self.push(issue);
            return None;
        }
        match compiled {
            Ok(program) => Some(program),
            Err(error) => {
                let issue = self
                    .issue(IssueCode::ControllerMolangInvalid, at)
                    .name("error", &error.0);
                self.push(issue);
                None
            }
        }
    }

    fn expression(&mut self, at: &str, value: &Value) -> Option<Program> {
        match value {
            Value::Number(n) => Some(Program::number(n.as_f64().unwrap_or(0.0) as f32)),
            Value::Bool(b) => Some(Program::number(if *b { 1.0 } else { 0.0 })),
            Value::String(source) => self.compile(at, source),
            _ => {
                self.fail(IssueCode::ControllerInvalid, at);
                None
            }
        }
    }

    fn scripts(&mut self, at: &str, value: Option<&Value>) -> Vec<Program> {
        let sources: Vec<&Value> = match value {
            None => return Vec::new(),
            Some(Value::Array(items)) => items.iter().collect(),
            Some(other) => vec![other],
        };
        let mut out = Vec::with_capacity(sources.len());
        for source in sources {
            match source.as_str() {
                Some(source) => out.extend(self.compile(at, source)),
                None => self.fail(IssueCode::ControllerInvalid, at),
            }
        }
        out
    }

    fn curve(&mut self, at: &str, map: &Map<String, Value>) -> Option<Vec<[f32; 2]>> {
        let mut keys = Vec::with_capacity(map.len());
        for (key, value) in map {
            let x = key.trim().parse::<f32>().ok().filter(|x| x.is_finite());
            let y = value.as_f64().map(|y| y as f32).filter(|y| y.is_finite());
            match (x, y) {
                (Some(x), Some(y)) => keys.push([x, y]),
                _ => {
                    self.fail(IssueCode::ControllerInvalid, &join(at, key));
                    return None;
                }
            }
        }
        keys.sort_by(|a, b| a[0].total_cmp(&b[0]));
        Some(keys)
    }

    fn blend(&mut self, at: &str, value: Option<&Value>) -> Blend {
        let blend = match value {
            None => return Blend::Instant,
            Some(Value::Number(n)) => n
                .as_f64()
                .filter(|s| s.is_finite() && *s >= 0.0)
                .map(|s| Blend::Seconds(s as f32)),
            Some(Value::Object(map)) => match self.curve(at, map) {
                Some(keys) if keys.iter().all(|k| k[0] >= 0.0) => Some(Blend::Curve(keys)),
                Some(_) => None,
                None => return Blend::Instant,
            },
            Some(_) => None,
        };
        blend.unwrap_or_else(|| {
            self.fail(IssueCode::ControllerInvalid, at);
            Blend::Instant
        })
    }

    fn animations(&mut self, at: &str, value: Option<&Value>) -> Vec<StateAnimation> {
        let Some(value) = value else {
            return Vec::new();
        };
        let Some(items) = value.as_array() else {
            self.fail(IssueCode::ControllerInvalid, at);
            return Vec::new();
        };
        let mut out = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let at = join(at, &i.to_string());
            let entry = match item {
                Value::String(name) => Some((name, None)),
                Value::Object(map) if map.len() == 1 => {
                    map.iter().next().map(|(name, weight)| (name, Some(weight)))
                }
                _ => None,
            };
            let Some((name, weight)) = entry else {
                self.fail(IssueCode::ControllerInvalid, &at);
                continue;
            };
            let weight = match weight {
                None => Program::number(1.0),
                Some(weight) => match self.expression(&join(&at, name), weight) {
                    Some(program) => program,
                    None => continue,
                },
            };
            out.push(StateAnimation {
                name: name.clone(),
                weight,
                index: None,
            });
        }
        out
    }

    fn transitions(&mut self, names: &[&str], at: &str, value: Option<&Value>) -> Vec<Transition> {
        let Some(value) = value else {
            return Vec::new();
        };
        let Some(items) = value.as_array() else {
            self.fail(IssueCode::ControllerInvalid, at);
            return Vec::new();
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let Some((target, condition)) = item
                .as_object()
                .filter(|map| map.len() == 1)
                .and_then(|map| map.iter().next())
            else {
                self.fail(IssueCode::ControllerInvalid, at);
                continue;
            };
            let Some(target_index) = names.iter().position(|n| n == target) else {
                let issue = self
                    .issue(IssueCode::ControllerUnknownState, at)
                    .name("state", target);
                self.push(issue);
                continue;
            };
            if let Some(condition) = self.expression(at, condition) {
                out.push(Transition {
                    target: target_index,
                    condition,
                });
            }
        }
        out
    }

    fn effects(
        &mut self,
        at: &str,
        value: Option<&Value>,
        kind: EffectKind,
        out: &mut Vec<Effect>,
    ) {
        let Some(value) = value else {
            return;
        };
        let Some(items) = value.as_array() else {
            self.fail(IssueCode::ControllerInvalid, at);
            return;
        };
        for (i, item) in items.iter().enumerate() {
            let at = join(at, &i.to_string());
            let Some(object) = item.as_object() else {
                self.fail(IssueCode::ControllerInvalid, &at);
                continue;
            };
            self.keys(&at, object, EFFECT_KEYS);
            let name = object
                .get("effect")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty());
            let locator = match object.get("locator") {
                None => Some(None),
                Some(Value::String(locator)) => Some(Some(locator.clone())),
                Some(_) => None,
            };
            let bind_to_actor = match object.get("bind_to_actor") {
                None => Some(true),
                Some(Value::Bool(bind)) => Some(*bind),
                Some(_) => None,
            };
            match (name, locator, bind_to_actor) {
                (Some(name), Some(locator), Some(bind_to_actor)) => out.push(Effect {
                    kind,
                    name: name.to_string(),
                    locator,
                    bind_to_actor,
                }),
                _ => self.fail(IssueCode::ControllerInvalid, &at),
            }
        }
    }

    fn variables(&mut self, at: &str, value: Option<&Value>) -> Vec<StateVariable> {
        let Some(value) = value else {
            return Vec::new();
        };
        let Some(map) = value.as_object() else {
            self.fail(IssueCode::ControllerInvalid, at);
            return Vec::new();
        };
        let mut out = Vec::with_capacity(map.len());
        for (name, body) in map {
            let at = join(at, name);
            let lower = name.to_ascii_lowercase();
            let named = lower
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                && lower
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_');
            let Some(object) = body.as_object().filter(|_| named) else {
                self.fail(IssueCode::ControllerInvalid, &at);
                continue;
            };
            self.keys(&at, object, VARIABLE_KEYS);
            let input_at = join(&at, "input");
            let Some(input) = object.get("input") else {
                self.fail(IssueCode::ControllerInvalid, &input_at);
                continue;
            };
            let Some(input) = self.expression(&input_at, input) else {
                continue;
            };
            let curve_at = join(&at, "remap_curve");
            let curve = match object.get("remap_curve") {
                None => Vec::new(),
                Some(Value::Object(keys)) => match self.curve(&curve_at, keys) {
                    Some(keys) => keys,
                    None => continue,
                },
                Some(_) => {
                    self.fail(IssueCode::ControllerInvalid, &curve_at);
                    continue;
                }
            };
            if self.compile(&at, &format!("variable.{lower}")).is_none() {
                continue;
            }
            let Some(slot) = self.interner.variable_slot(&lower) else {
                self.fail(IssueCode::ControllerInvalid, &at);
                continue;
            };
            out.push(StateVariable { slot, input, curve });
        }
        out
    }

    fn state(&mut self, names: &[&str], name: &str, at: &str, body: &Value) -> State {
        let empty = Map::new();
        let object = match body.as_object() {
            Some(object) => object,
            None => {
                self.fail(IssueCode::ControllerInvalid, at);
                &empty
            }
        };
        self.keys(at, object, STATE_KEYS);
        let field = |key: &str| (join(at, key), object.get(key));

        let (key_at, value) = field("animations");
        let animations = self.animations(&key_at, value);
        let (key_at, value) = field("transitions");
        let transitions = self.transitions(names, &key_at, value);
        let (key_at, value) = field("blend_transition");
        let blend = self.blend(&key_at, value);
        let (key_at, value) = field("blend_via_shortest_path");
        let shortest_path = match value {
            None => false,
            Some(Value::Bool(shortest)) => *shortest,
            Some(_) => {
                self.fail(IssueCode::ControllerInvalid, &key_at);
                false
            }
        };
        let (key_at, value) = field("on_entry");
        let on_entry = self.scripts(&key_at, value);
        let (key_at, value) = field("on_exit");
        let on_exit = self.scripts(&key_at, value);
        let mut effects = Vec::new();
        let (key_at, value) = field("particle_effects");
        self.effects(&key_at, value, EffectKind::Particle, &mut effects);
        let (key_at, value) = field("sound_effects");
        self.effects(&key_at, value, EffectKind::Sound, &mut effects);
        let (key_at, value) = field("variables");
        let variables = self.variables(&key_at, value);
        let (key_at, value) = field("anim_time_update");
        let anim_time_update = value.and_then(|value| self.expression(&key_at, value));

        State {
            name: name.to_string(),
            animations,
            transitions,
            blend,
            shortest_path,
            effects,
            on_entry,
            on_exit,
            variables,
            anim_time_update,
        }
    }

    fn controller(&mut self, name: &str, at: &str, body: &Value) -> Option<Controller> {
        let Some(object) = body.as_object() else {
            self.fail(IssueCode::ControllerInvalid, at);
            return None;
        };
        self.keys(at, object, CONTROLLER_KEYS);
        let states_at = join(at, "states");
        let Some(states) = object
            .get("states")
            .and_then(Value::as_object)
            .filter(|states| !states.is_empty())
        else {
            self.fail(IssueCode::ControllerInvalid, &states_at);
            return None;
        };
        if states.len() > MAX_STATES {
            let issue = self
                .issue(IssueCode::ControllerTooManyStates, &states_at)
                .value("states", states.len() as f64)
                .value("limit", MAX_STATES as f64);
            self.push(issue);
            return None;
        }
        let names: Vec<&str> = states.keys().map(String::as_str).collect();
        let initial_at = join(at, "initial_state");
        let initial_name = match object.get("initial_state") {
            None => "default",
            Some(Value::String(initial)) => initial.as_str(),
            Some(_) => {
                self.fail(IssueCode::ControllerInvalid, &initial_at);
                return None;
            }
        };
        let Some(initial) = names.iter().position(|n| *n == initial_name) else {
            let issue = self
                .issue(IssueCode::ControllerUnknownState, &initial_at)
                .name("state", initial_name);
            self.push(issue);
            return None;
        };
        let states = states
            .iter()
            .map(|(state, body)| self.state(&names, state, &join(&states_at, state), body))
            .collect();
        Some(Controller {
            file: self.file.to_string(),
            name: name.to_string(),
            initial,
            states,
        })
    }
}

pub fn parse(
    file: &str,
    root: &Value,
    interner: &mut Interner,
    rules: Rules,
    report: &mut Report,
) -> Vec<Controller> {
    let mut reader = Reader {
        file,
        interner,
        rules,
        report,
        broken: false,
    };
    let Some(object) = root.as_object() else {
        reader.fail(IssueCode::ControllerInvalid, "");
        return Vec::new();
    };
    reader.keys("", object, ROOT_KEYS);
    let Some(list) = object.get(CONTROLLERS).and_then(Value::as_object) else {
        reader.fail(IssueCode::ControllerInvalid, CONTROLLERS);
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.len());
    for (name, body) in list {
        reader.broken = false;
        let controller = reader.controller(name, &join(CONTROLLERS, name), body);
        if let Some(controller) = controller.filter(|_| !reader.broken) {
            out.push(controller);
        }
    }
    out
}

pub fn blend_bone(from: &BonePose, to: &BonePose, t: f32, shortest: bool) -> BonePose {
    if t <= 0.0 {
        return *from;
    }
    if t >= 1.0 {
        return *to;
    }
    BonePose {
        rotation: blend_rotation(from.rotation, to.rotation, t, shortest),
        position: lerp3(from.position, to.position, t),
        scale: lerp3(from.scale, to.scale, t),
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn blend_rotation(from: [f32; 3], to: [f32; 3], t: f32, shortest: bool) -> [f32; 3] {
    if from == to {
        return from;
    }
    let by_value = lerp3(from, to, t);
    let a = quaternion(from);
    let mut b = quaternion(to);
    let mut cos = dot4(a, b);
    if shortest && cos < 0.0 {
        b = b.map(|v| -v);
        cos = -cos;
    }
    let q = if cos > 0.9995 {
        let q: [f32; 4] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
        let len = libm::sqrtf(dot4(q, q));
        q.map(|v| v / len)
    } else if cos < -0.9995 {
        return by_value;
    } else {
        let theta = libm::acosf(cos);
        let sin = libm::sinf(theta);
        let wa = libm::sinf((1.0 - t) * theta) / sin;
        let wb = libm::sinf(t * theta) / sin;
        std::array::from_fn(|i| a[i] * wa + b[i] * wb)
    };
    euler_near(q, by_value)
}

fn dot4(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn quaternion(degrees: [f32; 3]) -> [f32; 4] {
    let half = |d: f32| d.to_radians() * 0.5;
    let (a, b, c) = (half(degrees[0]), half(-degrees[1]), half(-degrees[2]));
    let (sa, ca) = (libm::sinf(a), libm::cosf(a));
    let (sb, cb) = (libm::sinf(b), libm::cosf(b));
    let (sc, cc) = (libm::sinf(c), libm::cosf(c));
    [
        ca * cb * cc + sa * sb * sc,
        sa * cb * cc - ca * sb * sc,
        ca * sb * cc + sa * cb * sc,
        ca * cb * sc - sa * sb * cc,
    ]
}

fn euler_near(q: [f32; 4], near: [f32; 3]) -> [f32; 3] {
    let [w, x, y, z] = q;
    let a = libm::atan2f(2.0 * (w * x + y * z), 1.0 - 2.0 * (x * x + y * y));
    let b = libm::asinf((2.0 * (w * y - z * x)).clamp(-1.0, 1.0));
    let c = libm::atan2f(2.0 * (w * z + x * y), 1.0 - 2.0 * (y * y + z * z));
    let target = [
        near[0].to_radians(),
        -near[1].to_radians(),
        -near[2].to_radians(),
    ];
    let pi = std::f32::consts::PI;
    let turn = 2.0 * pi;
    let mut best = [a, b, c];
    let mut best_distance = f32::INFINITY;
    for candidate in [[a, b, c], [a + pi, pi - b, c + pi]] {
        let unwrapped: [f32; 3] = std::array::from_fn(|i| {
            candidate[i] + turn * libm::roundf((target[i] - candidate[i]) / turn)
        });
        let distance: f32 = (0..3)
            .map(|i| (unwrapped[i] - target[i]) * (unwrapped[i] - target[i]))
            .sum();
        if distance < best_distance {
            best = unwrapped;
            best_distance = distance;
        }
    }
    [
        best[0].to_degrees(),
        -best[1].to_degrees(),
        -best[2].to_degrees(),
    ]
}

fn accumulate(target: &mut BonePose, add: &BonePose) {
    for i in 0..3 {
        target.rotation[i] += add.rotation[i];
        target.position[i] += add.position[i];
        target.scale[i] *= add.scale[i];
    }
}

fn flag(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

pub struct Slots<'a> {
    pub states: &'a mut [u32],
    pub entered: &'a mut [f32],
    pub delays: &'a mut [f32],
    pub anim_times: &'a mut [f32],
    pub anim_clock: Option<&'a mut f32>,
    pub now: f32,
    pub epoch: f64,
    pub appear: bool,
    pub times_reset: bool,
    pub speed: Speed,
    pub timelines: bool,
}

pub struct Clips<'a> {
    pub animations: &'a [Animation],
    pub binds: &'a [Vec<Option<usize>>],
    pub offsets: &'a [usize],
    pub disabled: &'a mut [u32],
    pub first_frame: bool,
    pub props: Option<ClipProps<'a>>,
}

pub struct ClipProps<'a> {
    pub binds: &'a [Vec<Option<usize>>],
    pub pose: &'a mut Pose,
}

pub const ARRIVAL: f32 = -1.0;
const TIMELINE_CROSSINGS: usize = 64;
const INLINE_TIMELINE_KEYS: usize = 16;

const CAUGHT_RUNS: usize = 16;
pub const SAME_ENTRY: f32 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caught {
    pub clip: usize,
    pub from: f32,
    pub to: f32,
    pub delays: [f32; 2],
    pub ago: [f32; 2],
    pub fresh: bool,
    pub age: f32,
    entered: f32,
}

fn sound_keys(animation: &Animation) -> bool {
    animation
        .events
        .iter()
        .any(|e| matches!(e.kind, KeyEventKind::Sound { .. }))
}

pub fn particle_keys(animation: &Animation) -> bool {
    animation
        .events
        .iter()
        .any(|e| matches!(e.kind, KeyEventKind::Particle { .. }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub base: f32,
    pub at: f32,
    pub before: f32,
}

impl Anchor {
    pub fn at(at: f32) -> Self {
        Anchor::based(0.0, at)
    }

    pub fn based(base: f32, at: f32) -> Self {
        Anchor {
            base,
            at,
            before: at,
        }
    }

    pub fn span(&self, [last, now]: [f32; 2]) -> [f32; 2] {
        [
            self.base + (last - self.before),
            self.base + (now - self.at),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    pub clip: usize,
    pub elapsed: f32,
    pub age: f32,
    pub from: Option<f32>,
    pub delays: [f32; 2],
    pub timeline_from: Option<f32>,
    pub anchor: Option<Anchor>,
    pub late: bool,
    pub fresh: bool,
    pub first: bool,
    pub reaction: Option<u32>,
}

impl Played {
    pub fn new(clip: usize, elapsed: f32, age: f32, from: Option<f32>, delays: [f32; 2]) -> Self {
        Played {
            clip,
            elapsed,
            age,
            from,
            delays,
            timeline_from: None,
            anchor: None,
            late: true,
            fresh: fresh_play(age),
            first: false,
            reaction: None,
        }
    }

    pub fn anchored(self, anchor: Option<Anchor>) -> Self {
        Played { anchor, ..self }
    }

    pub fn reacting(self, reaction: usize) -> Self {
        Played {
            reaction: u32::try_from(reaction).ok(),
            ..self
        }
    }

    fn arrival(clip: usize, delays: [f32; 2]) -> Self {
        Played::new(clip, 0.0, 0.0, Some(ARRIVAL), delays)
    }
}

pub fn crossing_span(
    elapsed: f32,
    from: Option<f32>,
    anchor: Option<Anchor>,
    [last, now]: [f32; 2],
) -> [f32; 2] {
    match (from, anchor) {
        (Some(from), _) => [from, elapsed],
        (None, Some(anchor)) if last.is_finite() && now.is_finite() => anchor.span([last, now]),
        (None, _) => [elapsed - (now - last), elapsed],
    }
}

fn timeline_keys<'a>(
    animations: &'a [Animation],
    clip: &Played,
    clock: [f32; 2],
    speed: Option<Speed>,
) -> impl Iterator<Item = (f32, &'a Program)> + 'a {
    let &Played {
        clip,
        elapsed,
        from,
        delays,
        timeline_from,
        anchor,
        ..
    } = clip;
    let dt = clock[1] - clock[0];
    let [from, elapsed] = crossing_span(elapsed, timeline_from.or(from), anchor, clock);
    let span = elapsed - from;
    animations
        .get(clip)
        .into_iter()
        .flat_map(move |animation| {
            animation.crossings_every_lap(Some(from), elapsed, delays, TIMELINE_CROSSINGS, |e| {
                matches!(e.kind, KeyEventKind::Timeline(_))
            })
        })
        .filter_map(move |(event, at)| match &event.kind {
            KeyEventKind::Timeline(program) => {
                let within = if span > 0.0 {
                    ((at - from) / span).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let within = speed.map_or(within, |speed| speed.wall_share(dt, within));
                Some((within, program))
            }
            _ => None,
        })
}

pub fn run_timelines(
    animations: &[Animation],
    playing: &[Played],
    led: Option<(Speed, usize)>,
    clock: [f32; 2],
    scope: &mut Scope<'_>,
) {
    if let [clip] = playing {
        for (_, program) in timeline_keys(animations, clip, clock, None) {
            program.evaluate(scope, &mut NoQueries);
        }
        return;
    }
    let mut inline: [Option<(f32, &Program)>; INLINE_TIMELINE_KEYS] = [None; INLINE_TIMELINE_KEYS];
    let mut held = 0;
    let mut spill: Vec<(f32, &Program)> = Vec::new();
    let keys = playing.iter().enumerate().flat_map(|(at, clip)| {
        let speed = led.filter(|&(_, count)| at < count).map(|(speed, _)| speed);
        timeline_keys(animations, clip, clock, speed)
    });
    for key in keys {
        if spill.is_empty() && held < INLINE_TIMELINE_KEYS {
            inline[held] = Some(key);
            held += 1;
            continue;
        }
        if spill.is_empty() {
            spill.extend(inline[..held].iter().flatten().copied());
        }
        spill.push(key);
    }
    if spill.is_empty() {
        let keys = &mut inline[..held];
        keys.sort_by(|a, b| {
            let (a, b) = (a.map_or(0.0, |k| k.0), b.map_or(0.0, |k| k.0));
            a.total_cmp(&b)
        });
        for (_, program) in keys.iter().flatten() {
            program.evaluate(scope, &mut NoQueries);
        }
        return;
    }
    spill.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, program) in spill {
        program.evaluate(scope, &mut NoQueries);
    }
}

#[derive(Debug)]
pub struct Scratch {
    clip: Pose,
    from: Pose,
    to: Pose,
    prop_clip: Pose,
    prop_from: Pose,
    prop_to: Pose,
    played: Vec<Played>,
    posed: Vec<Played>,
    arrivals: Vec<Played>,
    caught: Vec<Caught>,
    steps: Vec<Option<[f32; 2]>>,
    remaining: Option<f32>,
}

impl Default for Scratch {
    fn default() -> Self {
        Scratch {
            clip: Pose(Vec::new()),
            from: Pose(Vec::new()),
            to: Pose(Vec::new()),
            prop_clip: Pose(Vec::new()),
            prop_from: Pose(Vec::new()),
            prop_to: Pose(Vec::new()),
            played: Vec::new(),
            posed: Vec::new(),
            arrivals: Vec::new(),
            caught: Vec::new(),
            steps: Vec::new(),
            remaining: None,
        }
    }
}

impl Scratch {
    pub fn played(&self) -> &[Played] {
        &self.played
    }

    pub fn posed(&self) -> &[Played] {
        &self.posed
    }

    pub fn remaining(&self) -> Option<f32> {
        self.remaining
    }

    pub fn caught(&self) -> &[Caught] {
        &self.caught
    }

    fn uncatch(&mut self, now: f32) {
        for play in &self.played {
            let entered = now - play.age;
            let continued = self.caught.iter().rposition(|c| {
                c.clip == play.clip && (c.entered - entered).abs() <= SAME_ENTRY && !c.fresh
            });
            if let Some(at) = continued {
                self.caught.remove(at);
            }
        }
    }

    fn fit(&mut self, bones: usize, props: usize) {
        for pose in [&mut self.clip, &mut self.from, &mut self.to] {
            pose.0.resize(bones, BonePose::default());
        }
        for pose in [&mut self.prop_clip, &mut self.prop_from, &mut self.prop_to] {
            pose.0.resize(props, BonePose::default());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Current,
    Outgoing,
}

#[derive(Clone, Copy, Default)]
struct Track {
    current: usize,
    previous: Option<usize>,
    holds: u32,
    entered: f32,
    previous_entered: f32,
    entry: bool,
    previous_entry: bool,
    moved: bool,
}

impl Track {
    fn load(words: &[u32], times: &[f32]) -> Track {
        let previous = (words[0] >> PREVIOUS_SHIFT) & STATE_MASK;
        Track {
            current: (words[0] & STATE_MASK) as usize,
            previous: previous.checked_sub(1).map(|p| p as usize),
            holds: words[1],
            entered: times[0],
            previous_entered: times[1],
            entry: false,
            previous_entry: false,
            moved: false,
        }
    }

    fn entered_at(&self, now: f32) -> bool {
        self.entry && self.entered == now
    }

    fn store(&self, words: &mut [u32], times: &mut [f32]) {
        let previous = self.previous.map_or(0, |p| p as u32 + 1);
        words[0] = self.current as u32 | (previous << PREVIOUS_SHIFT);
        words[1] = self.holds;
        times[0] = self.entered;
        times[1] = self.previous_entered;
    }

    fn held(&self, state: usize) -> bool {
        self.holds & (1 << state) != 0
    }
}

struct Seen {
    states: Vec<(usize, Option<usize>, u32, i64)>,
    times: Vec<u32>,
    gates: Vec<u32>,
    tick: u32,
    variables: Vec<f64>,
}

struct Frame<'f, 'a, 'b> {
    set: &'f ControllerSet,
    scope: &'f mut Scope<'a>,
    clips: &'f mut Clips<'b>,
    effects: &'f mut Vec<EffectEvent>,
    kept: &'f mut [f32],
    pool: Range<usize>,
    times: &'f mut [f32],
    lanes: Range<usize>,
    per: usize,
    width: usize,
    steps: &'f mut [Option<[f32; 2]>],
    arrivals: &'f mut Vec<Played>,
    stepped: [bool; 2],
    delta: f32,
    speed: f32,
    now: f32,
    epoch: f64,
    origin: Option<f32>,
    rewound: bool,
    caught: Option<f32>,
    late: bool,
    fresh: bool,
    since: f32,
    catch_up: u32,
    failures: usize,
    entry_timelines: bool,
}

const CURRENT: usize = 0;
const OUTGOING: usize = 1;

fn update_for<'s>(state: &'s State, animation: &'s Animation) -> Option<&'s Program> {
    animation
        .anim_time_update()
        .or(state.anim_time_update.as_ref())
}

impl Frame<'_, '_, '_> {
    fn run(&mut self, scripts: &[Program]) {
        for script in scripts {
            if script.evaluate(self.scope, &mut NoQueries).failed {
                self.failures += 1;
            }
        }
    }

    fn delays(&mut self, animation: &Animation, clip: usize, started: f64) -> [f32; 2] {
        let kept = self.kept.get_mut(self.pool.clone()).unwrap_or_default();
        animation.delays_kept(clip, self.scope, started, kept)
    }

    fn lane(&self, lane: usize) -> Option<Range<usize>> {
        let width = self.lanes.len() / 2;
        let start = self.lanes.start + lane * width;
        (width > 0 && start + width <= self.times.len()).then_some(start..start + width)
    }

    fn time_slot(&self, lane: usize, entry: usize) -> Option<usize> {
        self.lane(lane)
            .map(|range| range.start + entry)
            .filter(|&at| at < self.lanes.end && at < self.times.len())
    }

    fn clear_lane(&mut self, lane: usize) {
        if let Some(range) = self.lane(lane) {
            self.times[range].fill(0.0);
        }
        self.stepped[lane] = false;
    }

    fn carry_lane(&mut self) {
        if let (Some(current), Some(outgoing)) = (self.lane(CURRENT), self.lane(OUTGOING)) {
            let base = self.lanes.start;
            self.steps.copy_within(
                current.start - base..current.end - base,
                outgoing.start - base,
            );
            self.times.copy_within(current, outgoing.start);
        }
        self.stepped[OUTGOING] = self.stepped[CURRENT];
    }

    fn step_slot(&self, lane: usize, entry: usize) -> Option<usize> {
        self.time_slot(lane, entry).map(|at| at - self.lanes.start)
    }

    fn step_current(&mut self, state: &State, fresh: bool) {
        let animations = self.clips.animations;
        for (slot, entry) in state.animations.iter().enumerate() {
            let Some(animation) = entry.index.and_then(|index| animations.get(index)) else {
                continue;
            };
            let stepped = self.advance(state, animation, CURRENT, slot, fresh);
            if let Some(at) = self.step_slot(CURRENT, slot) {
                self.steps[at] = stepped;
            }
        }
        self.stepped[CURRENT] = true;
    }

    fn stepped(
        &mut self,
        state: &State,
        animation: &Animation,
        lane: usize,
        entry: usize,
        fresh: bool,
    ) -> Option<[f32; 2]> {
        if self.stepped[lane] {
            self.step_slot(lane, entry).and_then(|at| self.steps[at])
        } else {
            self.advance(state, animation, lane, entry, fresh)
        }
    }

    fn laned(&self, state: &State, animation: &Animation) -> bool {
        self.set.paced || update_for(state, animation).is_some()
    }

    fn clock(
        &self,
        state: &State,
        animation: &Animation,
        lane: usize,
        entry: usize,
        elapsed: f32,
    ) -> f32 {
        match (self.laned(state, animation), self.time_slot(lane, entry)) {
            (true, Some(at)) => self.times[at],
            _ => elapsed,
        }
    }

    fn advance(
        &mut self,
        state: &State,
        animation: &Animation,
        lane: usize,
        entry: usize,
        fresh: bool,
    ) -> Option<[f32; 2]> {
        if !self.laned(state, animation) {
            return None;
        }
        let at = self.time_slot(lane, entry)?;
        let before = self.times[at];
        if fresh {
            return Some([before, before]);
        }
        let delta = self.delta * self.speed;
        let after = match update_for(state, animation) {
            None => before + delta,
            Some(program) => {
                self.query(self.set.anim_time, before);
                self.query(self.set.delta_time, delta);
                let outcome = program.evaluate(self.scope, &mut NoQueries);
                self.query(self.set.delta_time, self.delta);
                let value = outcome.value as f32;
                if outcome.failed || !value.is_finite() {
                    self.failures += usize::from(outcome.failed);
                    before + delta
                } else {
                    value
                }
            }
        };
        self.times[at] = after;
        Some([before, after])
    }

    fn query(&mut self, slot: Option<u16>, value: f32) {
        if let Some(slot) = slot {
            self.scope.set_query(slot, value);
        }
    }

    fn finished(&mut self, state: &State, elapsed: f32, held: bool, entered: f32) -> [bool; 3] {
        let started = self.epoch + f64::from(entered);
        let animations = self.clips.animations;
        let (mut all, mut any, mut holds) = (true, false, true);
        for (slot, entry) in state.animations.iter().enumerate() {
            let Some(index) = entry.index else {
                continue;
            };
            let Some(animation) = animations.get(index) else {
                continue;
            };
            let hold = animation.loop_mode == Loop::Hold;
            let done = (held && hold) || {
                let [delay, _] = self.delays(animation, index, started);
                let clock = self.clock(state, animation, CURRENT, slot, elapsed);
                clock >= delay.max(0.0) + animation.length
            };
            all &= done;
            any |= done;
            if hold {
                holds &= done;
            }
        }
        [all, any, holds]
    }

    fn remaining(&mut self, controller: &Controller, track: &Track) -> Option<f32> {
        let state = &controller.states[track.current];
        if !state.transitions.is_empty() {
            return None;
        }
        let elapsed = self.now - track.entered;
        let started = self.epoch + f64::from(track.entered);
        let animations = self.clips.animations;
        let mut left = -elapsed;
        for (slot, entry) in state.animations.iter().enumerate() {
            let Some(index) = entry.index else {
                continue;
            };
            let Some(animation) = animations.get(index) else {
                continue;
            };
            if animation.loop_mode != Loop::Once {
                return None;
            }
            let [delay, _] = self.delays(animation, index, started);
            let clock = self.clock(state, animation, CURRENT, slot, elapsed);
            left = left.max(delay.max(0.0) + animation.length - clock);
        }
        Some(left)
    }

    fn set_queries(&mut self, state: &State, elapsed: f32, held: bool, entered: f32) {
        let set = self.set;
        self.query(set.anim_time, elapsed);
        if set.all_finished.is_some() || set.any_finished.is_some() {
            let [all, any, _] = self.finished(state, elapsed, held, entered);
            self.query(set.all_finished, flag(all));
            self.query(set.any_finished, flag(any));
        }
    }

    fn variables(&mut self, state: &State) {
        for variable in &state.variables {
            let outcome = variable.input.evaluate(self.scope, &mut NoQueries);
            if outcome.failed {
                self.failures += 1;
                continue;
            }
            let value = sample_curve(&variable.curve, outcome.value as f32);
            self.scope.set_variable(variable.slot, value);
        }
    }

    fn pick(&mut self, state: &State) -> Option<usize> {
        for transition in &state.transitions {
            let outcome = transition.condition.evaluate(self.scope, &mut NoQueries);
            if outcome.failed {
                self.failures += 1;
                continue;
            }
            if outcome.value != 0.0 {
                return Some(transition.target);
            }
        }
        None
    }

    fn enter(&mut self, index: usize, controller: &Controller, track: &mut Track, target: usize) {
        track.current = target;
        track.entered = self.now;
        track.entry = true;
        self.clear_lane(CURRENT);
        let state = &controller.states[target];
        self.set_queries(state, 0.0, track.held(target), self.now);
        self.run(&state.on_entry);
        for (effect, entry) in state.effects.iter().enumerate() {
            self.effects.push(EffectEvent {
                controller: index,
                state: target,
                kind: entry.kind,
                effect,
                ago: 0.0,
                emitter: None,
                fresh: false,
            });
        }
        self.variables(state);
        self.arrive(state, track.held(target));
    }

    fn weight(&mut self, entry: &StateAnimation, animation: &Animation) -> Option<f32> {
        let weight = entry.weight.evaluate(self.scope, &mut NoQueries);
        let own = animation.weight_in(self.scope);
        if weight.failed || own.failed {
            self.failures += 1;
            return None;
        }
        let weight = (weight.value as f32).clamp(0.0, 1.0) * own.value as f32;
        (!weight.is_nan() && weight > 0.0).then_some(weight)
    }

    fn arrive(&mut self, state: &State, held: bool) {
        let started = self.epoch + f64::from(self.now);
        let animations = self.clips.animations;
        for entry in &state.animations {
            let Some(index) = entry.index else {
                continue;
            };
            let Some(animation) = animations.get(index) else {
                continue;
            };
            if animation.events.is_empty() {
                continue;
            }
            self.query(self.set.anim_time, 0.0);
            if self.weight(entry, animation).is_none() {
                continue;
            }
            let delays = self.delays(animation, index, started);
            let hold = held && animation.loop_mode == Loop::Hold;
            let clock = if hold { f32::INFINITY } else { 0.0 };
            if animation.local_time(clock, delays).is_some() {
                let mut arrival = Played::arrival(index, delays);
                if self.entry_timelines {
                    let clock = [self.now, self.now];
                    for (_, program) in timeline_keys(animations, &arrival, clock, None) {
                        program.evaluate(self.scope, &mut NoQueries);
                    }
                    arrival.timeline_from = Some(0.0);
                }
                self.arrivals.push(arrival);
            }
        }
    }

    fn focus(&mut self, index: usize) {
        self.pool = index * self.per..(index + 1) * self.per;
        self.lanes = index * self.width..(index + 1) * self.width;
        self.stepped = [false; 2];
    }

    fn settle(
        &mut self,
        index: usize,
        controller: &Controller,
        track: &mut Track,
        appear: bool,
        times_reset: bool,
    ) {
        let fresh = appear || track.current >= controller.states.len();
        if fresh {
            track.holds = 0;
            track.previous = None;
            track.previous_entered = self.now;
            self.enter(index, controller, track, controller.initial);
            self.step_current(&controller.states[track.current], true);
        } else {
            if times_reset {
                track.previous = None;
                self.clear_lane(CURRENT);
            }
            self.step_current(&controller.states[track.current], track.entered == self.now);
            self.remap(controller, track);
        }
        self.transitions(index, controller, track);
    }

    fn resume(&mut self, index: usize, controller: &Controller, track: &mut Track) {
        self.step_current(&controller.states[track.current], false);
        self.remap(controller, track);
        self.transitions(index, controller, track);
    }

    fn remap(&mut self, controller: &Controller, track: &Track) {
        let state = &controller.states[track.current];
        if state.variables.is_empty() {
            return;
        }
        let elapsed = self.now - track.entered;
        self.set_queries(state, elapsed, track.held(track.current), track.entered);
        self.variables(state);
    }

    fn remember(
        &mut self,
        seen: &mut Vec<Seen>,
        joining: &[usize],
        tracks: &[Track],
        gates: &[Option<u32>],
        tick: u32,
    ) -> Option<u32> {
        let mut variables = Vec::new();
        self.scope.copy_variables(&mut variables);
        let mut times = Vec::new();
        let mut states = Vec::with_capacity(joining.len());
        for &index in joining {
            let track = &tracks[index];
            let ticks = libm::roundf((self.now - track.entered) / CATCH_UP_SECONDS) as i64;
            states.push((track.current, track.previous, track.holds, ticks));
            let lanes = index * self.width..(index + 1) * self.width;
            if let Some(lanes) = self.times.get(lanes) {
                times.extend(lanes.iter().map(|t| t.to_bits()));
            }
        }
        let gates = self.set.gate_phase(gates, tick);
        let snapshot = Seen {
            states,
            times,
            gates,
            tick,
            variables,
        };
        let same = |seen: &Seen| {
            seen.states == snapshot.states
                && seen.times == snapshot.times
                && seen.gates == snapshot.gates
                && seen.variables.len() == snapshot.variables.len()
                && seen
                    .variables
                    .iter()
                    .zip(&snapshot.variables)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
        };
        let lap = seen.iter().rev().find(|s| same(s)).map(|s| tick - s.tick);
        if seen.len() >= CATCH_UP_SNAPSHOTS {
            seen.remove(0);
        }
        seen.push(snapshot);
        lap
    }

    fn replay_at(&mut self, at: f32, delta: f32) {
        self.scope.clear_temps();
        self.since = if delta > 0.0 { self.now } else { at };
        self.now = at;
        self.delta = delta;
        self.query(self.set.delta_time, delta);
        let clock = self.epoch + f64::from(at);
        self.scope.set_random_frame(random_frame(clock));
    }

    fn settled(&mut self, controller: &Controller, track: &Track) -> bool {
        let state = &controller.states[track.current];
        let animations = self.clips.animations;
        let delta = self.set.delta_time;
        let still = state.transitions.is_empty()
            && state.variables.iter().all(|variable| {
                let reads = variable.input.reads();
                reads.variables.is_empty()
                    && delta.is_none_or(|slot| !reads.queries.contains(&slot))
            })
            && state.animations.iter().all(|entry| {
                entry
                    .index
                    .and_then(|index| animations.get(index))
                    .is_none_or(|animation| update_for(state, animation).is_none())
            });
        if !still {
            return false;
        }
        let weighed = state.animations.iter().any(|entry| {
            varies(entry.weight.reads())
                || entry
                    .index
                    .and_then(|index| animations.get(index))
                    .is_some_and(Animation::weight_varies)
        });
        !weighed || !self.keys_ahead(state, track)
    }

    fn keys_ahead(&mut self, state: &State, track: &Track) -> bool {
        let elapsed = self.now - track.entered;
        let started = self.epoch + f64::from(track.entered);
        let animations = self.clips.animations;
        for (slot, entry) in state.animations.iter().enumerate() {
            let Some(index) = entry.index else {
                continue;
            };
            let Some(animation) = animations.get(index) else {
                continue;
            };
            let Some(last) = animation.events.iter().map(|e| e.time).reduce(f32::max) else {
                continue;
            };
            if animation.loop_mode == Loop::Repeat {
                return true;
            }
            let [delay, _] = self.delays(animation, index, started);
            let clock = self.clock(state, animation, CURRENT, slot, elapsed);
            if clock <= delay.max(0.0) + last {
                return true;
            }
        }
        false
    }

    fn sweep(&mut self, controller: &Controller, track: &Track, scratch: &mut Scratch) {
        let arrived = track.moved && track.entered == self.now;
        if let Some(previous) = track.previous {
            let state = &controller.states[previous];
            let elapsed = self.now - track.entered;
            if arrived || state.blend.outgoing_weight(elapsed) > 0.0 {
                self.state_pose(
                    state,
                    OUTGOING,
                    self.now - track.previous_entered,
                    track.previous_entered,
                    track.held(previous),
                    arrived && track.previous_entry,
                    scratch,
                    None,
                    false,
                );
            }
        }
        self.state_pose(
            &controller.states[track.current],
            CURRENT,
            self.now - track.entered,
            track.entered,
            track.held(track.current),
            track.entered_at(self.now),
            scratch,
            None,
            false,
        );
        scratch.played.append(self.arrivals);
    }

    fn catch_keys(&mut self, scratch: &mut Scratch, now: f32) {
        let animations = self.clips.animations;
        let end = self.now;
        let start = self.since;
        for play in &scratch.played {
            if !self.set.keyed(animations, play.clip, self.fresh) {
                continue;
            }
            let entered = end - play.age;
            let arrival = play.from == Some(ARRIVAL);
            let [from, to] = match play.from {
                Some(ARRIVAL) => [ARRIVAL, play.elapsed],
                from => crossing_span(
                    play.elapsed,
                    play.timeline_from.or(from),
                    play.anchor,
                    [start, end],
                ),
            };
            let continued = scratch.caught.iter_mut().rev().find(|c| {
                c.clip == play.clip
                    && (c.entered - entered).abs() <= SAME_ENTRY
                    && c.ago[1] == start
                    && !arrival
            });
            match continued {
                Some(run) => {
                    run.to = to;
                    run.ago[1] = end;
                }
                None => {
                    if scratch.caught.len() >= CAUGHT_RUNS {
                        self.set.prune_caught(&mut scratch.caught, now);
                    }
                    if scratch.caught.len() >= CAUGHT_RUNS {
                        scratch.caught.remove(0);
                    }
                    scratch.caught.push(Caught {
                        clip: play.clip,
                        from,
                        to,
                        delays: play.delays,
                        ago: [if arrival { end } else { start }, end],
                        fresh: self.fresh,
                        age: 0.0,
                        entered,
                    });
                }
            }
        }
    }

    fn replay_timelines(&mut self, scratch: &mut Scratch, run: bool) {
        if run && !scratch.played.is_empty() {
            let animations = self.clips.animations;
            let clock = [self.since, self.now];
            run_timelines(animations, &scratch.played, None, clock, self.scope);
        }
        scratch.played.clear();
        scratch.posed.clear();
        self.arrivals.clear();
    }

    fn catch_up(
        &mut self,
        joining: &[usize],
        tracks: &mut [Track],
        origin: f32,
        scratch: &mut Scratch,
    ) -> (f32, f32) {
        let set = self.set;
        let timelines = set.timelines && !self.clips.first_frame;
        let keys = (set.particle_keys || self.fresh) && !self.clips.first_frame;
        let sweeping = timelines || keys;
        let rewound = mem::replace(&mut self.rewound, false);
        let entry_timelines = self.entry_timelines;
        let now = self.now;
        let mut fuel = self.scope.fuel();
        let frame = self.scope.random_frame();
        let first = self.effects.len();
        let funded = self.catch_up >= FRAME_FUEL;
        let periodic = joining
            .iter()
            .all(|&index| set.periodic.get(index).copied().unwrap_or(false));
        let mut seen = Vec::new();
        let mut gates = vec![None; set.gate_ticks.len()];
        let mut temps = Vec::new();
        self.scope.copy_temps(&mut temps);
        if funded {
            self.scope.set_fuel(FRAME_FUEL);
        }
        self.entry_timelines = timelines && funded;
        self.replay_at(origin, 0.0);
        for &index in joining {
            let controller = &set.controllers[index];
            let track = &mut tracks[index];
            self.focus(index);
            track.holds = 0;
            track.previous = None;
            track.previous_entered = origin;
            self.enter(index, controller, track, controller.initial);
            self.step_current(&controller.states[track.current], true);
            if funded {
                self.transitions(index, controller, track);
            }
            if sweeping && funded {
                self.sweep(controller, track, scratch);
            }
        }
        if sweeping && funded {
            self.catch_keys(scratch, now);
            self.replay_timelines(scratch, timelines);
        }
        self.arrivals.clear();
        if funded {
            self.catch_up -= FRAME_FUEL - self.scope.fuel();
        } else {
            fuel = self.scope.fuel();
        }
        self.space(first, 0, &mut gates);
        if periodic {
            self.remember(&mut seen, joining, tracks, &gates, 0);
        }
        self.age(first, now);
        self.entry_timelines = timelines;
        let history_ticks = libm::ceilf(set.history(joining) / CATCH_UP_SECONDS) as u32;
        let mut tick = 0u32;
        let mut steps = 0u32;
        loop {
            let at = origin + CATCH_UP_SECONDS * (tick + 1) as f32;
            let settled = joining.iter().all(|&index| {
                self.focus(index);
                self.settled(&set.controllers[index], &tracks[index])
            });
            if at >= now || settled || steps >= CATCH_UP_STEPS || self.catch_up < FRAME_FUEL {
                break;
            }
            tick += 1;
            steps += 1;
            let fired = self.effects.len();
            self.scope.set_fuel(FRAME_FUEL);
            self.replay_at(at, CATCH_UP_SECONDS);
            let mut moved = false;
            for &index in joining {
                let controller = &set.controllers[index];
                let track = &mut tracks[index];
                self.focus(index);
                self.resume(index, controller, track);
                moved |= track.entered == self.now;
                if sweeping {
                    self.sweep(controller, track, scratch);
                }
            }
            if sweeping {
                self.catch_keys(scratch, now);
                self.replay_timelines(scratch, timelines);
            }
            self.arrivals.clear();
            self.catch_up -= FRAME_FUEL - self.scope.fuel();
            self.space(fired, tick, &mut gates);
            let mut skipped = false;
            if periodic && moved {
                if let Some(lap) = self.remember(&mut seen, joining, tracks, &gates, tick) {
                    let left = libm::floorf((now - self.now) / CATCH_UP_SECONDS) as u32;
                    let laps = left.saturating_sub(history_ticks) / lap;
                    if laps > 2 {
                        let skip = (laps - 2) * lap;
                        tick += skip;
                        let at = origin + CATCH_UP_SECONDS * tick as f32;
                        let shift = at - self.now;
                        for &index in joining {
                            tracks[index].previous_entered += shift;
                            tracks[index].entered += shift;
                        }
                        for last in gates.iter_mut().flatten() {
                            *last += skip;
                        }
                        self.now = at;
                        seen.clear();
                        set.prune_caught(&mut scratch.caught, now);
                        skipped = true;
                    }
                }
            }
            self.age(fired, now);
            if skipped || self.effects.len() - first > CATCH_UP_PRUNE {
                self.prune(first);
            }
        }
        let left = (now - self.now).max(0.0);
        let reached = self.now;
        set.prune_caught(&mut scratch.caught, now);
        for run in &mut scratch.caught {
            run.ago = run.ago.map(|at| (now - at).max(0.0));
            run.age = now - run.entered;
        }
        self.now = now;
        self.rewound = rewound;
        self.entry_timelines = entry_timelines;
        self.scope.set_random_frame(frame);
        self.scope.set_temps(&temps);
        self.prune(first);
        self.scope.set_fuel(fuel);
        (left, reached)
    }

    fn age(&mut self, from: usize, now: f32) {
        let ago = (now - self.now).max(0.0);
        for event in &mut self.effects[from..] {
            event.ago = ago;
            event.fresh = self.fresh;
        }
    }

    fn space(&mut self, from: usize, tick: u32, gates: &mut [Option<u32>]) {
        let set = self.set;
        let mut at = from;
        while at < self.effects.len() {
            let event = self.effects[at];
            let linked = set.gates(&event);
            let passed = linked
                .iter()
                .filter(|&&(_, gate)| set.open(gates, gate, tick))
                .count();
            if passed == linked.len() {
                for &(_, gate) in linked {
                    if let Some(last) = gates.get_mut(gate) {
                        *last = Some(tick);
                    }
                }
                at += 1;
                continue;
            }
            self.effects.remove(at);
            for &(emitter, gate) in linked {
                if !set.open(gates, gate, tick) {
                    continue;
                }
                if let Some(last) = gates.get_mut(gate) {
                    *last = Some(tick);
                }
                let only = EffectEvent {
                    emitter: Some(emitter),
                    ..event
                };
                self.effects.insert(at, only);
                at += 1;
            }
        }
    }

    fn prune(&mut self, first: usize) {
        let mut later: Vec<((usize, usize, usize, Option<u16>), f32)> = Vec::new();
        let mut keep = vec![true; self.effects.len()];
        for (at, event) in self.effects.iter().enumerate().skip(first).rev() {
            let key = (event.controller, event.state, event.effect, event.emitter);
            let next = match later.iter_mut().find(|(k, _)| *k == key) {
                Some((_, ago)) => Some(mem::replace(ago, event.ago)),
                None => {
                    later.push((key, event.ago));
                    None
                }
            };
            let kept = self.set.keep(event);
            let closing = event.ago < kept.spacing && !self.set.gates(event).is_empty();
            keep[at] = event.fresh || closing || kept.shows(event.ago, next);
        }
        let mut at = 0;
        self.effects.retain(|_| {
            at += 1;
            keep[at - 1]
        });
    }

    fn transitions(&mut self, index: usize, controller: &Controller, track: &mut Track) {
        let mut moved = false;
        for _ in 0..MAX_TRANSITIONS_PER_FRAME {
            let from = track.current;
            let state = &controller.states[from];
            let elapsed = self.now - track.entered;
            let held = track.held(from);
            self.set_queries(state, elapsed, held, track.entered);
            let Some(target) = self.pick(state) else {
                break;
            };
            self.run(&state.on_exit);
            let [_, _, holds_done] = self.finished(state, elapsed, held, track.entered);
            if holds_done {
                track.holds |= 1 << from;
            } else {
                track.holds &= !(1 << from);
            }
            if !moved {
                track.previous = Some(from);
                track.previous_entered = track.entered;
                track.previous_entry = track.entered_at(self.now);
                track.moved = true;
                self.carry_lane();
                moved = true;
            }
            self.enter(index, controller, track, target);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn state_pose(
        &mut self,
        state: &State,
        lane: usize,
        elapsed: f32,
        entered: f32,
        held: bool,
        arrived: bool,
        scratch: &mut Scratch,
        into: Option<Lane>,
        still: bool,
    ) {
        let Scratch {
            clip,
            from,
            to,
            prop_clip,
            prop_from,
            prop_to,
            played,
            posed,
            ..
        } = scratch;
        let (out, prop_out) = match into {
            Some(Lane::Current) | None => (to, prop_to),
            Some(Lane::Outgoing) => (from, prop_from),
        };
        out.reset();
        prop_out.reset();
        let started = self.epoch + f64::from(entered);
        let animations = self.clips.animations;
        let binds = self.clips.binds;
        let first_frame = self.clips.first_frame;
        let fresh = still || entered == self.now;
        let age = elapsed;
        for (slot, entry) in state.animations.iter().enumerate() {
            let Some(index) = entry.index else {
                continue;
            };
            let Some(animation) = animations.get(index) else {
                continue;
            };
            let stepped = self.stepped(state, animation, lane, slot, fresh);
            let elapsed = stepped.map_or(elapsed, |[_, after]| after);
            self.query(self.set.anim_time, elapsed);
            let Some(weight) = self.weight(entry, animation) else {
                continue;
            };
            let hold = held && animation.loop_mode == Loop::Hold;
            let delays = self.delays(animation, index, started);
            let clock = if hold { f32::INFINITY } else { elapsed };
            let Some(time) = animation.local_time(clock, delays) else {
                continue;
            };
            if !still && !arrived && !animation.events.is_empty() {
                let from = match stepped {
                    Some([before, after]) => Some(if after <= 0.0 && after < before {
                        ARRIVAL
                    } else {
                        before
                    }),
                    None => self.rewound.then_some(elapsed),
                };
                let anchor = Anchor::at(entered);
                let mut play =
                    Played::new(index, elapsed, age, from, delays).anchored(Some(anchor));
                play.late = self.late;
                play.fresh &= !self.late || self.fresh;
                if stepped.is_none() {
                    play.timeline_from = self.caught.map(|at| (at - entered).max(0.0));
                }
                played.push(play);
            }
            if into.is_none() {
                continue;
            }
            posed.push(Played::new(index, elapsed, age, None, delays));
            let time = if first_frame { 0.0 } else { time };
            self.query(self.set.anim_time, time);
            if animation.pre_animation(self.scope) {
                self.failures += 1;
            }
            let bind = binds.get(index).map_or(&[][..], Vec::as_slice);
            let first = self.clips.offsets.get(index).copied();
            let disabled = match first {
                Some(first) => Disabled::new(self.clips.disabled, first),
                None => Disabled::none(),
            };
            self.failures += animation.pose(bind, time, weight, self.scope, disabled, clip);
            for (target, add) in out.0.iter_mut().zip(&clip.0) {
                accumulate(target, add);
            }
            let Some(props) = self.clips.props.as_ref() else {
                continue;
            };
            let bind = props.binds.get(index).map_or(&[][..], Vec::as_slice);
            let disabled = match first {
                Some(first) => Disabled::new(self.clips.disabled, first),
                None => Disabled::none(),
            };
            self.failures += animation.pose(bind, time, weight, self.scope, disabled, prop_clip);
            for (target, add) in prop_out.0.iter_mut().zip(&prop_clip.0) {
                accumulate(target, add);
            }
        }
    }

    fn pose(
        &mut self,
        controller: &Controller,
        track: &mut Track,
        scratch: &mut Scratch,
        pose: &mut Pose,
    ) {
        let elapsed = self.now - track.entered;
        let outgoing = track.previous.and_then(|previous| {
            let state = controller.states.get(previous)?;
            let weight = state.blend.outgoing_weight(elapsed);
            (weight > 0.0).then_some((previous, weight, state.shortest_path))
        });
        let current = &controller.states[track.current];
        let animations = self.clips.animations;
        let empty_end = self.set.keep_ends
            && current.transitions.is_empty()
            && !current
                .animations
                .iter()
                .any(|entry| entry.index.is_some_and(|index| index < animations.len()));
        let kept = track
            .previous
            .and_then(|previous| controller.states.get(previous))
            .filter(|state| empty_end && state.blend.outgoing_weight(0.0) <= 0.0);
        if outgoing.is_none() {
            let arrived = track.moved && track.entered == self.now;
            let left = track.previous.filter(|_| arrived);
            if let Some(state) = left.and_then(|previous| controller.states.get(previous)) {
                self.state_pose(
                    state,
                    OUTGOING,
                    self.now - track.previous_entered,
                    track.previous_entered,
                    track.held(track.previous.unwrap_or(0)),
                    track.previous_entry,
                    scratch,
                    kept.is_some().then_some(Lane::Current),
                    false,
                );
            } else if let Some(state) = kept {
                self.state_pose(
                    state,
                    OUTGOING,
                    track.entered - track.previous_entered,
                    track.previous_entered,
                    track.held(track.previous.unwrap_or(0)),
                    false,
                    scratch,
                    Some(Lane::Current),
                    true,
                );
            }
            if kept.is_none() {
                track.previous = None;
            }
        }
        if let Some((previous, _, _)) = outgoing {
            self.state_pose(
                &controller.states[previous],
                OUTGOING,
                self.now - track.previous_entered,
                track.previous_entered,
                track.held(previous),
                track.previous_entry && track.moved,
                scratch,
                Some(Lane::Outgoing),
                false,
            );
        }
        if kept.is_none() {
            let held = track.held(track.current);
            self.state_pose(
                current,
                CURRENT,
                elapsed,
                track.entered,
                held,
                track.entered_at(self.now),
                scratch,
                Some(Lane::Current),
                false,
            );
        }
        scratch.played.append(self.arrivals);
        let Some((_, weight, shortest)) = outgoing else {
            for (target, add) in pose.0.iter_mut().zip(&scratch.to.0) {
                accumulate(target, add);
            }
            if let Some(props) = self.clips.props.as_mut() {
                for (target, add) in props.pose.0.iter_mut().zip(&scratch.prop_to.0) {
                    accumulate(target, add);
                }
            }
            return;
        };
        let blended = pose.0.iter_mut().zip(&scratch.from.0).zip(&scratch.to.0);
        for ((target, from), to) in blended {
            accumulate(target, &blend_bone(from, to, 1.0 - weight, shortest));
        }
        if let Some(props) = self.clips.props.as_mut() {
            let blended = props
                .pose
                .0
                .iter_mut()
                .zip(&scratch.prop_from.0)
                .zip(&scratch.prop_to.0);
            for ((target, from), to) in blended {
                accumulate(target, &blend_bone(from, to, 1.0 - weight, shortest));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Clock {
    Plain,
    Rated(f64),
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatePlays {
    pub clips: Vec<(usize, Clock)>,
    pub steady: Vec<usize>,
    pub reentered: bool,
    pub lasting: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerPlays {
    pub states: Vec<StatePlays>,
}

#[derive(Clone, Debug, Default)]
pub struct ControllerSet {
    controllers: Vec<Controller>,
    anim_time: Option<u16>,
    delta_time: Option<u16>,
    all_finished: Option<u16>,
    any_finished: Option<u16>,
    rated: bool,
    paced: bool,
    clocked: bool,
    keep_ends: bool,
    timelines: bool,
    particle_keys: bool,
    periodic: Vec<bool>,
    reads: Vec<u16>,
    keeps: Vec<Vec<Vec<Kept>>>,
    gate_ticks: Vec<u32>,
    gated: Vec<(u16, usize)>,
    clip_keeps: Vec<Option<CatchUpKeep>>,
}

fn repeats(state: &State, animations: &[Animation]) -> bool {
    let own = state.programs().all(|program| !program.reads().random);
    own && state
        .animations
        .iter()
        .filter_map(|entry| entry.index.and_then(|index| animations.get(index)))
        .all(|animation| {
            animation.programs().iter().all(|(name, program)| {
                let timed = matches!(
                    name.as_str(),
                    "start_delay" | "loop_delay" | "anim_time_update"
                ) || name.starts_with("timeline");
                !timed || !program.reads().random
            })
        })
}

fn resolve(animations: &[Animation], name: &str) -> Option<usize> {
    if let Some(index) = animations.iter().position(|a| a.name == name) {
        return Some(index);
    }
    let mut found = animations.iter().enumerate().filter(|(_, a)| {
        a.name
            .strip_suffix(name)
            .is_some_and(|rest| rest.ends_with('.'))
    });
    match (found.next(), found.next()) {
        (Some((index, _)), None) => Some(index),
        _ => None,
    }
}

impl ControllerSet {
    pub fn select(loaded: Vec<Controller>, declared: &[&str], report: &mut Report) -> Self {
        let mut controllers = if declared.is_empty() {
            loaded
        } else {
            let mut chosen = Vec::with_capacity(declared.len());
            for name in declared {
                match loaded.iter().find(|c| c.name == *name) {
                    Some(controller) => chosen.push(controller.clone()),
                    None => report.push(
                        Issue::error(IssueCode::ControllerMissing, "controllers")
                            .name("controller", name),
                    ),
                }
            }
            chosen
        };
        if controllers.len() > MAX_CONTROLLERS {
            report.push(
                Issue::error(IssueCode::ControllerTooManyControllers, "controllers")
                    .value("controllers", controllers.len() as f64)
                    .value("limit", MAX_CONTROLLERS as f64),
            );
            controllers.truncate(MAX_CONTROLLERS);
        }
        ControllerSet {
            controllers,
            ..ControllerSet::default()
        }
    }

    pub fn plays(&self, clip: usize) -> bool {
        self.controllers.iter().any(|controller| {
            controller.states.iter().any(|state| {
                state
                    .animations
                    .iter()
                    .any(|entry| entry.index == Some(clip))
            })
        })
    }

    pub fn concurrent_plays(&self, keyed: impl Fn(usize) -> bool) -> usize {
        self.concurrent_weight(&[], |clip, _| usize::from(keyed(clip)))
    }

    pub fn concurrent_weight(
        &self,
        animations: &[Animation],
        weight: impl Fn(usize, bool) -> usize,
    ) -> usize {
        let plays = |state: &State| {
            state
                .animations
                .iter()
                .filter_map(|entry| {
                    let clip = entry.index?;
                    let rated = state.anim_time_update.is_some()
                        || animations
                            .get(clip)
                            .is_some_and(|animation| animation.anim_time_update().is_some());
                    Some(weight(clip, rated))
                })
                .fold(0, usize::saturating_add)
        };
        self.controllers
            .iter()
            .map(|controller| {
                let states = &controller.states;
                let counts: Vec<usize> = states.iter().map(plays).collect();
                let mut reachable = vec![false; states.len()];
                let mut stack = vec![controller.initial];
                while let Some(at) = stack.pop() {
                    if at >= states.len() || reachable[at] {
                        continue;
                    }
                    reachable[at] = true;
                    stack.extend(states[at].transitions.iter().map(|t| t.target));
                }
                let mut chain: Vec<Option<usize>> = counts
                    .iter()
                    .zip(&reachable)
                    .map(|(&count, &on)| on.then_some(count))
                    .collect();
                let mut best = chain.iter().flatten().copied().max().unwrap_or(0);
                for _ in 0..MAX_TRANSITIONS_PER_FRAME {
                    let mut next = vec![None; states.len()];
                    for (from, sum) in chain.iter().enumerate() {
                        let Some(sum) = *sum else {
                            continue;
                        };
                        for transition in &states[from].transitions {
                            let Some(slot) = next.get_mut(transition.target) else {
                                continue;
                            };
                            let total = sum.saturating_add(counts[transition.target]);
                            *slot = Some(slot.map_or(total, |s: usize| s.max(total)));
                        }
                    }
                    best = best.max(next.iter().flatten().copied().max().unwrap_or(0));
                    chain = next;
                }
                best
            })
            .fold(0, usize::saturating_add)
    }

    pub fn state_plays(&self, animations: &[Animation]) -> Vec<ControllerPlays> {
        let clock = |state: &State, animation: &Animation| match update_for(state, animation) {
            None => Clock::Plain,
            Some(program) => self
                .anim_time
                .zip(self.delta_time)
                .and_then(|(time, delta)| program.clock_scale(time, delta))
                .map_or(Clock::Unknown, Clock::Rated),
        };
        self.controllers
            .iter()
            .map(|controller| {
                let states = &controller.states;
                let reach = |from: &mut Vec<usize>| {
                    let mut seen = vec![false; states.len()];
                    while let Some(at) = from.pop() {
                        if at >= states.len() || seen[at] {
                            continue;
                        }
                        seen[at] = true;
                        from.extend(states[at].transitions.iter().map(|t| t.target));
                    }
                    seen
                };
                let reachable = reach(&mut vec![controller.initial]);
                let plays = states
                    .iter()
                    .enumerate()
                    .filter(|&(at, _)| reachable[at])
                    .map(|(at, state)| {
                        let mut next: Vec<usize> =
                            state.transitions.iter().map(|t| t.target).collect();
                        StatePlays {
                            clips: state
                                .animations
                                .iter()
                                .filter_map(|entry| {
                                    let clip = entry.index?;
                                    let animation = animations.get(clip)?;
                                    Some((clip, clock(state, animation)))
                                })
                                .collect(),
                            steady: state
                                .animations
                                .iter()
                                .filter(|entry| {
                                    matches!(
                                        entry.weight.bounds(&|_| None),
                                        Some((low, high)) if low == high && low > 0.0
                                    )
                                })
                                .filter_map(|entry| entry.index)
                                .filter(|&clip| clip < animations.len())
                                .collect(),
                            reentered: reach(&mut next)[at],
                            lasting: state.transitions.is_empty(),
                        }
                    })
                    .collect();
                ControllerPlays { states: plays }
            })
            .collect()
    }

    pub fn link(&mut self, animations: &[Animation], interner: &Interner, report: &mut Report) {
        self.anim_time = interner.query_slot("anim_time");
        self.delta_time = interner.query_slot("delta_time");
        self.all_finished = interner.query_slot("all_animations_finished");
        self.any_finished = interner.query_slot("any_animation_finished");
        for controller in &mut self.controllers {
            let at = join(&join(CONTROLLERS, &controller.name), "states");
            for state in &mut controller.states {
                for entry in &mut state.animations {
                    entry.index = resolve(animations, &entry.name);
                    if entry.index.is_none() {
                        let at = join(&join(&at, &state.name), "animations");
                        report.push(
                            Issue::warning(
                                IssueCode::ControllerUnknownAnimation,
                                path(&controller.file, &at),
                            )
                            .name("animation", &entry.name),
                        );
                    }
                }
            }
        }
        self.timelines = has_timeline(animations);
        self.particle_keys = animations.iter().any(particle_keys);
        self.periodic = self
            .controllers
            .iter()
            .map(|controller| {
                controller
                    .states
                    .iter()
                    .all(|state| repeats(state, animations))
            })
            .collect();
        self.rated = self
            .controllers
            .iter()
            .flat_map(|c| &c.states)
            .any(|state| {
                state.anim_time_update.is_some()
                    || state.animations.iter().any(|entry| {
                        entry
                            .index
                            .and_then(|index| animations.get(index))
                            .is_some_and(|animation| animation.anim_time_update().is_some())
                    })
            });
        let mut reads: Vec<u16> = self
            .controllers
            .iter()
            .flat_map(|c| &c.states)
            .flat_map(|state| {
                let clips = state
                    .animations
                    .iter()
                    .filter_map(|entry| entry.index.and_then(|index| animations.get(index)))
                    .flat_map(|animation| animation.programs().into_iter().map(|(_, p)| p));
                state.programs().chain(clips)
            })
            .flat_map(|program| program.reads().variables.iter().copied())
            .collect();
        reads.sort_unstable();
        reads.dedup();
        self.reads = reads;
        self.clocked = self.delta_time.is_some_and(|slot| {
            let reads = |program: &Program| program.reads().queries.contains(&slot);
            self.controllers
                .iter()
                .flat_map(|c| &c.states)
                .any(|state| {
                    state.programs().any(reads)
                        || state
                            .animations
                            .iter()
                            .filter_map(|entry| entry.index.and_then(|index| animations.get(index)))
                            .any(|animation| {
                                animation.programs().into_iter().any(|(_, p)| reads(p))
                            })
                })
        });
    }

    pub fn controllers(&self) -> &[Controller] {
        &self.controllers
    }

    pub fn record_slots(&self) -> usize {
        self.controllers.len() * SLOTS_PER_CONTROLLER
    }

    fn widest(&self) -> usize {
        self.controllers
            .iter()
            .flat_map(|c| &c.states)
            .map(|s| s.animations.len())
            .max()
            .unwrap_or(0)
    }

    fn kept_per_controller(&self) -> usize {
        2 * self.widest() * KEPT_DELAY_FLOATS
    }

    fn laned(&self) -> bool {
        self.rated || self.paced
    }

    pub fn reads_variable(&self, slot: u16) -> bool {
        self.reads.binary_search(&slot).is_ok()
    }

    pub fn needs_clock(&self) -> bool {
        self.laned() || self.clocked
    }

    pub fn pace(&mut self) {
        self.paced = !self.controllers.is_empty();
    }

    pub fn keep_ends(&mut self) {
        self.keep_ends = true;
    }

    pub fn anim_time_floats(&self) -> usize {
        if self.laned() {
            self.controllers.len() * 2 * self.widest()
        } else {
            0
        }
    }

    pub fn kept_delay_floats(&self) -> usize {
        self.controllers.len() * self.kept_per_controller()
    }

    pub fn size_catch_up(
        &mut self,
        animations: &[Animation],
        keep: impl Fn(&Effect) -> CatchUpKeep,
        gates: impl Fn(&Effect) -> Vec<(u16, u32)>,
        keyed: impl Fn(&Animation) -> Option<CatchUpKeep>,
    ) {
        self.clip_keeps = animations.iter().map(keyed).collect();
        self.particle_keys = self.clip_keeps.iter().any(Option::is_some);
        let mut groups: Vec<(usize, u16, u32)> = Vec::new();
        let mut ticks = Vec::new();
        let mut gated = Vec::new();
        let keeps = self
            .controllers
            .iter()
            .enumerate()
            .map(|(index, controller)| {
                controller
                    .states
                    .iter()
                    .map(|state| {
                        state
                            .effects
                            .iter()
                            .map(|effect| {
                                let kept = keep(effect);
                                let start = gated.len();
                                if kept.spacing <= 0.0 {
                                    return (kept, [start, start]);
                                }
                                for (emitter, entry) in gates(effect) {
                                    let group = (index, emitter, entry);
                                    let gate = match groups.iter().position(|g| *g == group) {
                                        Some(gate) => gate,
                                        None => {
                                            groups.push(group);
                                            let spacing = kept.spacing / CATCH_UP_SECONDS - 1e-3;
                                            ticks.push(libm::ceilf(spacing).max(1.0) as u32);
                                            groups.len() - 1
                                        }
                                    };
                                    gated.push((emitter, gate));
                                }
                                (kept, [start, gated.len()])
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        self.keeps = keeps;
        self.gate_ticks = ticks;
        self.gated = gated;
    }

    fn keyed(&self, animations: &[Animation], clip: usize, fresh: bool) -> bool {
        let kept = match self.clip_keeps.get(clip) {
            Some(keep) => keep.is_some(),
            None => animations.get(clip).is_some_and(particle_keys),
        };
        kept || fresh && animations.get(clip).is_some_and(sound_keys)
    }

    fn clip_keep(&self, clip: usize) -> CatchUpKeep {
        match self.clip_keeps.get(clip) {
            Some(keep) => keep.unwrap_or(CatchUpKeep::NONE),
            None => CatchUpKeep::unknown(EffectKind::Particle),
        }
    }

    fn kept(&self, event: &EffectEvent) -> Option<Kept> {
        self.keeps
            .get(event.controller)
            .and_then(|states| states.get(event.state))
            .and_then(|effects| effects.get(event.effect))
            .copied()
    }

    fn keep(&self, event: &EffectEvent) -> CatchUpKeep {
        self.kept(event)
            .map_or_else(|| CatchUpKeep::unknown(event.kind), |(keep, _)| keep)
    }

    fn gates(&self, event: &EffectEvent) -> &[(u16, usize)] {
        let Some((_, [start, end])) = self.kept(event) else {
            return &[];
        };
        let gates = self.gated.get(start..end).unwrap_or(&[]);
        match event.emitter {
            Some(emitter) => match gates.iter().position(|&(at, _)| at == emitter) {
                Some(at) => &gates[at..at + 1],
                None => &[],
            },
            None => gates,
        }
    }

    fn open(&self, gates: &[Option<u32>], gate: usize, tick: u32) -> bool {
        let spacing = self.gate_ticks.get(gate).copied().unwrap_or(0);
        gates
            .get(gate)
            .copied()
            .flatten()
            .is_none_or(|last| tick.saturating_sub(last) >= spacing)
    }

    fn gate_phase(&self, gates: &[Option<u32>], tick: u32) -> Vec<u32> {
        gates
            .iter()
            .zip(&self.gate_ticks)
            .map(|(last, &spacing)| {
                last.map_or(spacing, |last| tick.saturating_sub(last).min(spacing))
            })
            .collect()
    }

    fn prune_caught(&self, caught: &mut Vec<Caught>, now: f32) {
        caught.retain(|run| run.fresh || self.clip_keep(run.clip).shows(now - run.ago[1], None));
    }

    fn history(&self, joining: &[usize]) -> f32 {
        let mut window = CATCH_UP_FRESH_SECONDS;
        for &index in joining {
            let Some(controller) = self.controllers.get(index) else {
                continue;
            };
            let effects = self.keeps.get(index).into_iter().flatten().flatten();
            let clips = controller
                .states
                .iter()
                .flat_map(|state| &state.animations)
                .filter_map(|entry| self.clip_keeps.get(entry.index?).copied().flatten());
            for keep in effects.map(|(keep, _)| *keep).chain(clips) {
                if keep.window.is_finite() {
                    window = window.max(keep.window);
                }
            }
        }
        window
    }

    pub fn effect(&self, event: &EffectEvent) -> Option<&Effect> {
        self.controllers
            .get(event.controller)?
            .states
            .get(event.state)?
            .effects
            .get(event.effect)
    }

    pub fn controller_state(
        &self,
        record: &WearerRecord<'_>,
        controller: usize,
    ) -> Option<(&str, f32)> {
        let base = controller * SLOTS_PER_CONTROLLER;
        let word = *record.controller_states.get(base)?;
        let entered = *record.controller_entered.get(base)?;
        let state = self
            .controllers
            .get(controller)?
            .states
            .get((word & STATE_MASK) as usize)?;
        Some((state.name.as_str(), record.now - entered))
    }

    pub fn restart(&self, states: &mut [u32]) {
        for words in states
            .chunks_mut(SLOTS_PER_CONTROLLER)
            .take(self.controllers.len())
        {
            words[0] = STATE_MASK;
            if let Some(holds) = words.get_mut(1) {
                *holds = 0;
            }
        }
    }

    pub fn state_clips(&self, states: &[u32], out: &mut Vec<usize>) {
        for (controller, words) in self
            .controllers
            .iter()
            .zip(states.as_chunks::<SLOTS_PER_CONTROLLER>().0)
        {
            let current = (words[0] & STATE_MASK) as usize;
            let previous = ((words[0] >> PREVIOUS_SHIFT) & STATE_MASK).checked_sub(1);
            let indices = [Some(current), previous.map(|p| p as usize)];
            for state in indices.into_iter().flatten() {
                let Some(state) = controller.states.get(state) else {
                    continue;
                };
                for index in state.animations.iter().filter_map(|entry| entry.index) {
                    if !out.contains(&index) {
                        out.push(index);
                    }
                }
            }
        }
    }

    pub fn step(
        &self,
        slots: Slots<'_>,
        scope: &mut Scope<'_>,
        clips: &mut Clips<'_>,
        scratch: &mut Scratch,
        pose: &mut Pose,
        effects: &mut Vec<EffectEvent>,
    ) -> usize {
        self.step_from(slots, None, scope, clips, scratch, pose, effects)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step_from(
        &self,
        slots: Slots<'_>,
        origin: Option<f32>,
        scope: &mut Scope<'_>,
        clips: &mut Clips<'_>,
        scratch: &mut Scratch,
        pose: &mut Pose,
        effects: &mut Vec<EffectEvent>,
    ) -> usize {
        let Slots {
            states,
            entered,
            delays,
            anim_times,
            anim_clock,
            now,
            epoch,
            appear,
            times_reset,
            speed,
            timelines,
        } = slots;
        let props = clips.props.as_ref().map_or(0, |p| p.pose.0.len());
        scratch.fit(pose.0.len(), props);
        scratch.played.clear();
        scratch.posed.clear();
        scratch.caught.clear();
        scratch.remaining = Some(f32::NEG_INFINITY);
        let delta = match anim_clock {
            Some(clock) => {
                let last = *clock;
                *clock = now;
                if last.is_finite() && !appear && !times_reset {
                    (now - last).max(0.0)
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        let steady = speed.current();
        let steady = if steady.is_finite() { steady } else { 1.0 };
        let paced = if delta > 0.0 {
            speed.over(delta) / delta
        } else {
            steady
        };
        let paced = if paced.is_finite() { paced } else { steady };
        let per = self.kept_per_controller();
        let lanes = if self.laned() { 2 * self.widest() } else { 0 };
        let mut steps = mem::take(&mut scratch.steps);
        steps.clear();
        steps.resize(lanes, None);
        let mut arrivals = mem::take(&mut scratch.arrivals);
        arrivals.clear();
        let entry_timelines = timelines && self.timelines && !clips.first_frame;
        let mut frame = Frame {
            set: self,
            scope,
            clips,
            effects,
            kept: delays,
            pool: 0..0,
            times: anim_times,
            lanes: 0..0,
            per,
            width: lanes,
            steps: &mut steps,
            arrivals: &mut arrivals,
            stepped: [false; 2],
            delta,
            speed: steady,
            now,
            epoch,
            origin: origin.filter(|o| o.is_finite()),
            rewound: times_reset && !appear,
            caught: None,
            late: false,
            fresh: false,
            since: now,
            catch_up: CATCH_UP_FUEL,
            failures: 0,
            entry_timelines,
        };
        frame.query(self.delta_time, delta);
        let count = (0..self.controllers.len())
            .take_while(|index| {
                let end = (index + 1) * SLOTS_PER_CONTROLLER;
                end <= states.len() && end <= entered.len()
            })
            .count();
        let mut tracks = [Track::default(); MAX_CONTROLLERS];
        let mut joining = [0usize; MAX_CONTROLLERS];
        let mut joined = 0;
        for (index, track) in tracks.iter_mut().enumerate().take(count) {
            let at = index * SLOTS_PER_CONTROLLER;
            *track = Track::load(&states[at..], &entered[at..]);
            let fresh = appear || track.current >= self.controllers[index].states.len();
            if fresh || times_reset {
                joining[joined] = index;
                joined += 1;
            }
        }
        let origin = frame.origin.filter(|&origin| origin < now && joined > 0);
        frame.fresh = origin.is_some_and(|origin| fresh_play(now - origin));
        let caught =
            origin.map(|origin| frame.catch_up(&joining[..joined], &mut tracks, origin, scratch));
        frame.delta = delta;
        frame.speed = paced;
        frame.query(self.delta_time, delta);
        for (index, controller) in self.controllers.iter().enumerate().take(count) {
            let track = &mut tracks[index];
            frame.focus(index);
            frame.late = joining[..joined].contains(&index);
            match caught {
                Some((left, reached)) if joining[..joined].contains(&index) => {
                    frame.delta = left;
                    frame.speed = steady;
                    frame.caught = Some(reached);
                    frame.query(self.delta_time, left);
                    frame.resume(index, controller, track);
                }
                _ => frame.settle(index, controller, track, appear, times_reset),
            }
            frame.pose(controller, track, scratch, pose);
            frame.late = false;
            frame.delta = delta;
            frame.speed = paced;
            frame.caught = None;
            frame.query(self.delta_time, delta);
            let left = frame.remaining(controller, track);
            scratch.remaining = scratch.remaining.zip(left).map(|(a, b)| a.max(b));
            let at = index * SLOTS_PER_CONTROLLER;
            track.store(&mut states[at..], &mut entered[at..]);
        }
        if caught.is_some() {
            scratch.uncatch(now);
        }
        let failures = frame.failures;
        scratch.steps = steps;
        scratch.arrivals = arrivals;
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{self, channel_offsets, flag_words};
    use crate::issues::{Issue, IssueCode, Level, Mode};
    use crate::math::bedrock_rotation;
    use crate::molang::{NoQueries, Runtime};
    use crate::player::{self, Arms};
    use crate::signals::SIGNAL_COUNT;
    use crate::wearer::{seed, CosmeticSlot, WearerDeclarations, WearerTable};
    use crate::Model;

    const FILE: &str = "test.animation_controllers.json";

    const ANIMATIONS: &str = r#"{"format_version":"1.8.0","animations":{
        "animation.test.lift":{"loop":true,"animation_length":1.0,"bones":{"rightArm":{
            "rotation":[90,0,0],"position":[0,2,0],"scale":[2,2,2]}}},
        "animation.test.wave":{"animation_length":1.0,"bones":{"rightArm":{"rotation":[0,0,30]}}},
        "animation.test.spin":{"loop":true,"bones":{"rightArm":{"rotation":[350,0,0]}}},
        "animation.test.rest":{"loop":true,"bones":{"rightArm":{"rotation":[0,0,0]}}}
    }}"#;

    fn wrap(controllers: &str) -> String {
        format!(r#"{{"format_version":"1.10.0","animation_controllers":{{{controllers}}}}}"#)
    }

    fn load(text: &str, rules: Rules, mode: Mode) -> (Vec<Controller>, Interner, Vec<Issue>) {
        let mut interner = Interner::default();
        let mut report = Report::new(mode);
        let json: Value = serde_json::from_str(text).unwrap();
        let controllers = parse(FILE, &json, &mut interner, rules, &mut report);
        (controllers, interner, report.into_issues())
    }

    fn codes(text: &str) -> Vec<IssueCode> {
        let (controllers, _, issues) = load(text, Rules::Bedrock, Mode::Client);
        assert!(controllers.is_empty(), "a broken controller must not load");
        assert!(issues.iter().all(|i| i.level == Level::Error));
        issues.iter().map(|i| i.code).collect()
    }

    const FULL: &str = r#"
        "controller.animation.test.full":{"states":{
            "default":{
                "animations":["lift",{"animation.test.wave":"variable.mood * 0.5"}],
                "transitions":[{"busy":"query.is_hurt"},{"default":1}],
                "blend_transition":0.25,
                "blend_via_shortest_path":true,
                "on_entry":["variable.entries = variable.entries + 1;"],
                "on_exit":"variable.exits = variable.exits + 1;",
                "particle_effects":[{"effect":"sparkle","locator":"tip","bind_to_actor":false}],
                "sound_effects":[{"effect":"chime"}],
                "variables":{"speed_curve":{"input":"query.ground_speed",
                    "remap_curve":{"1.0":0.7,"0.0":0.2}}},
                "anim_time_update":"query.anim_time + query.delta_time * 2"
            },
            "busy":{"blend_transition":{"0.0":1.0,"1.0":0.0,"0.5":0.25}}
        }},
        "controller.animation.test.named":{"initial_state":"b","states":{"a":{},"b":{}}}"#;

    #[test]
    fn controller_parse_reads_every_supported_field() {
        let (controllers, interner, issues) = load(&wrap(FULL), Rules::Strict, Mode::Authoring);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(controllers.len(), 2);
        let full = &controllers[0];
        let (default, busy) = (&full.states[0], &full.states[1]);
        assert_eq!((full.initial, controllers[1].initial, busy.name.as_str()), (0, 1, "busy"));
        let names: Vec<_> = default.animations.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["lift", "animation.test.wave"]);
        let mut rt = Runtime::new(&interner);
        let mut slots = vec![0.0f32; interner.variable_count()];
        slots[interner.variable_slot("mood").unwrap() as usize] = 4.0;
        let mut scope = rt.wearer_frame(&mut slots, 0, 0.0);
        let weight = default.animations[1].weight.evaluate(&mut scope, &mut NoQueries);
        assert_eq!(weight.value, 2.0);
        let targets: Vec<_> = default.transitions.iter().map(|t| t.target).collect();
        assert_eq!(targets, [1, 0]);
        assert_eq!(default.blend, Blend::Seconds(0.25));
        assert_eq!(busy.blend, Blend::Curve(vec![[0.0, 1.0], [0.5, 0.25], [1.0, 0.0]]));
        assert!(default.shortest_path && !busy.shortest_path);
        assert_eq!((default.on_entry.len(), default.on_exit.len()), (1, 1));
        let effect = |kind, name: &str, locator: Option<&str>, bind_to_actor| {
            Effect { kind, name: name.into(), locator: locator.map(Into::into), bind_to_actor }
        };
        let sparkle = effect(EffectKind::Particle, "sparkle", Some("tip"), false);
        assert_eq!(default.effects, [sparkle, effect(EffectKind::Sound, "chime", None, true)]);
        assert_eq!(Some(default.variables[0].slot), interner.variable_slot("speed_curve"));
        assert_eq!(default.variables[0].curve, [[0.0, 0.2], [1.0, 0.7]]);
        assert!(default.anim_time_update.is_some() && busy.anim_time_update.is_none());
    }

    #[test]
    fn controller_parse_rejects_what_it_cannot_run() {
        let groups: [(IssueCode, &[&str]); 5] = [
            (IssueCode::ControllerUnsupported, &[
                r#""c":{"loop":true,"states":{"default":{}}}"#,
                r#""c":{"states":{"default":{"variables":{"v":{"input":"1","clamp":true}}}}}"#,
            ]),
            (IssueCode::ControllerServerSide, &[
                r#""c":{"states":{"default":{"transitions":[{"default":"query.has_component('minecraft:tameable')"}]}}}"#,
                r#""c":{"states":{"default":{"on_entry":["/say hi"]}}}"#,
                r#""c":{"states":{"default":{"animations":[{"lift":"q.property('pet:mood')"}]}}}"#,
            ]),
            (IssueCode::ControllerUnknownState, &[
                r#""c":{"states":{"default":{"transitions":[{"nowhere":"1"}]}}}"#,
                r#""c":{"states":{"idle":{}}}"#,
                r#""c":{"initial_state":"x","states":{"default":{}}}"#,
            ]),
            (IssueCode::ControllerMolangInvalid, &[r#""c":{"states":{"default":{"transitions":[{"default":"1 +"}]}}}"#]),
            (IssueCode::ControllerInvalid, &[
                r#""c":{"states":{}}"#,
                r#""c":{"initial_state":3,"states":{"default":{}}}"#,
                r#""c":{"states":{"default":{"blend_transition":-1}}}"#,
                r#""c":{"states":{"default":{"variables":{"v.x":{"input":"1"}}}}}"#,
            ]),
        ];
        for (code, bodies) in groups {
            for body in bodies {
                assert_eq!(codes(&wrap(body)), [code], "{body}");
            }
        }
        let states = |count: usize| {
            let rest: String = (1..count).map(|i| format!(r#","s{i}":{{}}"#)).collect();
            wrap(&format!(r#""c":{{"states":{{"default":{{}}{rest}}}}}"#))
        };
        assert!(load(&states(MAX_STATES), Rules::Bedrock, Mode::Client).2.is_empty());
        assert_eq!(codes(&states(MAX_STATES + 1)), [IssueCode::ControllerTooManyStates]);
        let text = wrap(r#""bad":{"states":{"default":{"transitions":[{"gone":"1"}]}}},"good":{"states":{"default":{}}}"#);
        let (controllers, _, issues) = load(&text, Rules::Bedrock, Mode::Client);
        assert_eq!((controllers.len(), controllers[0].name.as_str()), (1, "good"));
        let path = format!("{FILE}#animation_controllers.bad.states.default.transitions");
        assert_eq!(issues[0].path, path);
    }

    fn many(count: usize) -> Vec<Controller> {
        let body: Vec<String> = (0..count).map(|i| format!(r#""c{i}":{{"states":{{"default":{{}}}}}}"#)).collect();
        load(&wrap(&body.join(",")), Rules::Bedrock, Mode::Client).0
    }

    #[test]
    fn controller_selection_checks_names_the_limit_and_linked_animations() {
        let mut report = Report::new(Mode::Client);
        let set = ControllerSet::select(many(MAX_CONTROLLERS + 1), &[], &mut report);
        assert_eq!(report.issues()[0].code, IssueCode::ControllerTooManyControllers);
        assert_eq!(set.controllers().len(), MAX_CONTROLLERS);
        let mut report = Report::new(Mode::Client);
        let set = ControllerSet::select(many(3), &["c2", "nope", "c0"], &mut report);
        let names: Vec<_> = set.controllers().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["c2", "c0"]);
        assert_eq!(report.issues()[0].code, IssueCode::ControllerMissing);

        let text = wrap(r#""c":{"states":{"default":{"animations":["lift","animation.test.wave","ghost"]}}}"#);
        let (controllers, mut interner, _) = load(&text, Rules::Bedrock, Mode::Authoring);
        let animations = animation::parse(ANIMATIONS, &mut interner).unwrap();
        let mut report = Report::new(Mode::Authoring);
        let mut set = ControllerSet::select(controllers, &[], &mut report);
        set.link(&animations, &interner, &mut report);
        let state = &set.controllers()[0].states[0];
        let indices: Vec<_> = state.animations.iter().map(|a| a.index).collect();
        assert_eq!(indices, [Some(0), Some(1), None]);
        let issue = &report.issues()[0];
        assert_eq!((issue.code, issue.level), (IssueCode::ControllerUnknownAnimation, Level::Warning));
    }

    struct Rig {
        set: ControllerSet,
        interner: Interner,
        model: Model,
        animations: Vec<Animation>,
        binds: Vec<Vec<Option<usize>>>,
        offsets: Vec<usize>,
        disabled: Vec<u32>,
        table: WearerTable,
        slot: CosmeticSlot,
        rt: Runtime,
        scratch: Scratch,
        pose: Pose,
        effects: Vec<EffectEvent>,
        failures: usize,
        clock: f64,
        origin: Option<f64>,
    }

    impl Rig {
        fn new(controllers: &str) -> Rig {
            let mut interner = Interner::default();
            let animations = animation::parse(ANIMATIONS, &mut interner).unwrap();
            let mut report = Report::new(Mode::Authoring);
            let text: Value = serde_json::from_str(&wrap(controllers)).unwrap();
            let loaded = parse(FILE, &text, &mut interner, Rules::Bedrock, &mut report);
            let mut set = ControllerSet::select(loaded, &[], &mut report);
            set.link(&animations, &interner, &mut report);
            assert!(report.issues().is_empty(), "{:?}", report.issues());
            let model = player::model(Arms::Wide);
            let mut table = WearerTable::new();
            let declarations = WearerDeclarations {
                variables: interner.variable_count(),
                controllers: set.record_slots(),
                effect_times: 1,
                effect_floats: set.anim_time_floats(),
                ..WearerDeclarations::default()
            };
            let slot = table.add_cosmetic(declarations).unwrap();
            let (offsets, channels) = channel_offsets(&animations);
            let binds = animations.iter().map(|a| a.bind(&model)).collect();
            let (disabled, rt, pose) = (vec![0; flag_words(channels)], Runtime::new(&interner), Pose::rest(&model));
            let (scratch, effects, failures, clock, origin) = (Scratch::default(), Vec::new(), 0, f64::NAN, None);
            Rig { set, interner, model, animations, binds, offsets, disabled, table, slot, rt, scratch, pose,
                effects, failures, clock, origin }
        }

        fn frame(&mut self, clock: f64, wearer: u64, variables: &[(&str, f32)]) {
            if self.clock != clock {
                self.table.begin_frame(clock);
                self.clock = clock;
            }
            let signals = [0.0; SIGNAL_COUNT];
            let record = self.table.draw(self.slot, wearer, &signals, 0).unwrap();
            for (name, value) in variables {
                record.variables[self.interner.variable_slot(name).unwrap() as usize] = *value;
            }
            let epoch = record.epoch;
            let mut scope = self.rt.wearer_frame(record.variables, seed(wearer, 0, 0), clock);
            self.pose.reset();
            self.effects.clear();
            let mut clips = Clips {
                animations: &self.animations,
                binds: &self.binds,
                offsets: &self.offsets,
                disabled: &mut self.disabled,
                first_frame: false,
                props: None,
            };
            let slots = Slots {
                states: record.controller_states,
                entered: record.controller_entered,
                delays: &mut [],
                anim_times: record.effect_floats,
                anim_clock: record.effect_times.first_mut(),
                now: record.now,
                epoch,
                appear: record.appear,
                times_reset: record.times_reset,
                speed: Speed::default(),
                timelines: false,
            };
            let origin = self.origin.map(|o| (o - epoch) as f32);
            self.failures += self.set.step_from(
                slots,
                origin,
                &mut scope,
                &mut clips,
                &mut self.scratch,
                &mut self.pose,
                &mut self.effects,
            );
        }

        fn state(&mut self, wearer: u64, controller: usize) -> (String, f32) {
            let record = self.table.record(self.slot, wearer).unwrap();
            let (name, time) = self.set.controller_state(&record, controller).unwrap();
            (name.to_string(), time)
        }

        fn var(&mut self, wearer: u64, name: &str) -> f32 {
            let slot = self.interner.variable_slot(name).unwrap() as usize;
            self.table.record(self.slot, wearer).unwrap().variables[slot]
        }

        fn bone(&self, name: &str) -> BonePose {
            self.pose.0[self.model.bone(name).unwrap()]
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    const ENTRY_EXIT: &str = r#""c":{"states":{
        "default":{
            "on_entry":["v.starts = v.starts + 1;"],
            "on_exit":["v.left = v.left + 1;"],
            "transitions":[{"other":"v.go"}]},
        "other":{
            "on_entry":["v.arrived = v.left * 10 + v.starts;"],
            "transitions":[{"default":"!v.go"}]}}}"#;

    #[test]
    fn controller_transitions_and_scripts_run_per_wearer() {
        let chain = r#""c":{"states":{"default":{"transitions":[{"s1":"1"}]},"s1":{"transitions":[{"s2":"1"}]},
            "s2":{"transitions":[{"s3":"1"}]},"s3":{"transitions":[{"s4":"1"}]},"s4":{"transitions":[{"default":"1"}]}}}"#;
        let fuel = r#""c":{"states":{"default":{"transitions":[{"a":"math.die_roll(1000000000, 0, 1) >= 0"},{"b":"1"}]},
            "a":{},"b":{}}}"#;
        let first = r#""c":{"states":{"default":{"transitions":[{"a":"0"},{"b":"1"},{"c":"1"}]},"a":{},"b":{},"c":{}}}"#;
        for (text, state, failures) in [(first, "b", 0), (chain, "s4", 0), (fuel, "b", 1)] {
            let mut rig = Rig::new(text);
            rig.frame(0.0, 1, &[]);
            assert_eq!((rig.state(1, 0), rig.failures), ((state.to_string(), 0.0), failures), "{text}");
        }

        let mut rig = Rig::new(ENTRY_EXIT);
        rig.frame(0.0, 1, &[]);
        rig.frame(0.0, 2, &[]);
        rig.frame(1.0, 1, &[("go", 1.0)]);
        rig.frame(1.0, 2, &[]);
        assert_eq!(rig.state(1, 0), ("other".to_string(), 0.0));
        assert_eq!(rig.state(2, 0), ("default".to_string(), 1.0));
        let vars = ["starts", "left", "arrived"];
        assert_eq!(vars.map(|v| rig.var(1, v)), [1.0, 1.0, 11.0]);
        assert_eq!(vars.map(|v| rig.var(2, v)), [1.0, 0.0, 0.0]);
        rig.frame(1.5, 1, &[("go", 0.0)]);
        assert_eq!((rig.state(1, 0).0, rig.var(1, "starts")), ("default".to_string(), 2.0));
        rig.table.forget(rig.slot, 1);
        rig.frame(2.0, 1, &[]);
        assert_eq!((rig.state(1, 0), rig.var(1, "starts")), (("default".to_string(), 0.0), 1.0));
        assert_eq!(rig.var(1, "left"), 0.0, "a forgotten wearer starts over");
        assert_eq!(rig.failures, 0);
    }

    #[test]
    fn controller_catch_up_keeps_the_effect_runs_a_watcher_still_shows() {
        let text = r#""c":{"states":{
            "default":{"particle_effects":[{"effect":"ring"}],"transitions":[{"b":"q.anim_time > 0.12"}]},
            "b":{"transitions":[{"default":"q.anim_time > 0.02"}]}}}"#;
        let keep = CatchUpKeep { seconds: 3.1, window: 3.1, spacing: 0.25 };
        for join in [13.01, 14.06, 70.01, 70.16] {
            let mut rig = Rig::new(text);
            rig.set.size_catch_up(&rig.animations, |_| keep, |_| vec![(0, 0)], |_| None);
            rig.origin = Some(10.0);
            let (mut expected, mut last, mut frame) = (Vec::new(), f64::NEG_INFINITY, 0u32);
            while 10.0 + f64::from(frame) / 20.0 < join {
                let clock = 10.0 + f64::from(frame) / 20.0;
                rig.frame(clock, 1, &[]);
                if !rig.effects.is_empty() && clock - last >= 0.25 - 1e-6 {
                    last = clock;
                    expected.push((join - clock) as f32);
                }
                frame += 1;
            }
            expected.retain(|&ago| ago <= keep.seconds);
            rig.frame(join, 2, &[]);
            let ages: Vec<f32> = rig.effects.iter().map(|e| e.ago).collect();
            assert!(expected.len() >= 7, "{expected:?}");
            assert_eq!(ages.len(), expected.len(), "{join}: {ages:?} vs {expected:?}");
            let same = ages.iter().zip(&expected).all(|(a, b)| (a - b).abs() < 0.01);
            assert!(same, "{join}: {ages:?} vs {expected:?}");
        }
    }

    fn same_rotation(a: [f32; 3], b: [f32; 3]) -> bool {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].into_iter().all(|p| {
            let (x, y) = (bedrock_rotation(a).point(p), bedrock_rotation(b).point(p));
            (0..3).all(|i| (x[i] - y[i]).abs() < 1e-3)
        })
    }

    #[test]
    fn controller_blends_out_over_seconds_or_a_curve_and_the_short_way() {
        let rig = |blend: &str, shortest: bool, from: &str, to: &str| {
            let mut rig = Rig::new(&format!(
                r#""c":{{"states":{{
                    "default":{{"animations":["{from}"],"transitions":[{{"other":"v.go"}}],
                        "blend_transition":{blend},"blend_via_shortest_path":{shortest}}},
                    "other":{{"animations":["{to}"]}}}}}}"#
            ));
            rig.frame(0.0, 1, &[]);
            rig.frame(1.0, 1, &[("go", 1.0)]);
            rig
        };
        for (blend, steps) in [
            ("1.0", [(1.25, 67.5), (1.5, 45.0), (2.0, 0.0)]),
            (r#"{"0.0":1.0,"0.2":0.5,"1.0":0.0}"#, [(1.2, 45.0), (1.6, 22.5), (2.2, 0.0)]),
        ] {
            let mut rig = rig(blend, false, "lift", "rest");
            assert!(close(rig.bone("rightArm").rotation[0], 90.0));
            for (clock, x) in steps {
                rig.frame(clock, 1, &[]);
                let (arm, w) = (rig.bone("rightArm"), x / 90.0);
                assert!(close(arm.rotation[0], x), "{blend} at {clock}: {arm:?}");
                assert!(close(arm.position[1], 2.0 * w) && close(arm.scale[1], 1.0 + w), "{arm:?}");
            }
        }
        for (shortest, halfway) in [(false, [175.0, 0.0, 0.0]), (true, [-5.0, 0.0, 0.0])] {
            let mut rig = rig("1.0", shortest, "rest", "spin");
            rig.frame(1.5, 1, &[]);
            let turned = rig.bone("rightArm").rotation;
            assert!(same_rotation(turned, halfway), "{shortest}: {turned:?}");
            rig.frame(2.0, 1, &[]);
            assert!(close(rig.bone("rightArm").rotation[0], 350.0));
        }
    }

    #[test]
    fn anchored_clips_cross_a_key_on_the_frame_boundary_once() {
        let mut interner = Interner::default();
        let clip = r#"{"animations":{"long":{"animation_length":3000,"timeline":{"2176.505859375":"v.hits = v.hits + 1;"}}}}"#;
        let animations = animation::parse(clip, &mut interner).unwrap();
        let hits = interner.variable_slot("hits").unwrap() as usize;
        let mut runtime = Runtime::new(&interner);
        let mut variables = vec![0.0; interner.variable_count()];
        let entered = 46.846_558_f32;
        let times = [2_223.302_f32, 2_223.352_3, 2_223.402_3];
        assert_eq!(times[1] - entered, 2_176.505_9);
        for pair in times.windows(2) {
            let [last, now] = [pair[0], pair[1]];
            let elapsed = now - entered;
            let played = Played::new(0, elapsed, elapsed, None, [0.0, 0.0]).anchored(Some(Anchor::at(entered)));
            run_timelines(&animations, &[played], None, [last, now], &mut runtime.wearer_frame(&mut variables, 1, 0.0));
        }
        assert_eq!(variables[hits], 1.0);
    }
}
