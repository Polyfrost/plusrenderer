use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::attach::{rig, RigKind};
use crate::container::{guarded_json, is_id, Kind, FORMAT, MANIFEST};
use crate::issues::{join, path, Issue, IssueCode, Mode, Report};
use crate::limits::{
    ID_MAX_LEN, PET_CLIP_RATE_MAX, PET_LEASH_RADIUS, PET_MOVE_CLIP_SPEED, PET_MOVING_SPEED,
    PET_RUN_CLIP_SPEED,
};
pub use crate::limits::{PET_SCALE_MAX, PET_SCALE_MIN};

pub const PET_VARIABLES: [&str; 8] = [
    "pet_mode",
    "pet_speed",
    "pet_distance",
    "pet_on_ground",
    "pet_in_water",
    "pet_action",
    "pet_look_yaw",
    "pet_look_pitch",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Archetype {
    #[default]
    Walking,
    Flying,
    Shoulder,
}

impl Archetype {
    pub const ALL: [Archetype; 3] = [Archetype::Walking, Archetype::Flying, Archetype::Shoulder];

    pub fn as_str(self) -> &'static str {
        match self {
            Archetype::Walking => "walking",
            Archetype::Flying => "flying",
            Archetype::Shoulder => "shoulder",
        }
    }

    pub fn parse(name: &str) -> Option<Archetype> {
        Archetype::ALL.into_iter().find(|a| a.as_str() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PetMode {
    Idle = 0,
    Following = 1,
    CatchingUp = 2,
    Sitting = 3,
    Riding = 4,
    Sleeping = 5,
    Wandering = 6,
    Playing = 7,
    IdleAction = 8,
}

impl PetMode {
    pub const fn value(self) -> f32 {
        self as u8 as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PetAction {
    None = 0,
    Sit = 1,
    Lie = 2,
    Groom = 3,
    Yawn = 4,
    Sniff = 5,
    Perch = 6,
}

impl PetAction {
    pub const fn value(self) -> f32 {
        self as u8 as f32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractionDecl {
    pub id: String,
    pub as_a: bool,
    pub as_b: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PetSpec {
    pub archetype: Archetype,
    pub leash_radius: f32,
    pub scale: f32,
    pub anchor: Option<String>,
    pub controllers: Vec<String>,
    pub interactions: Vec<InteractionDecl>,
}

impl Default for PetSpec {
    fn default() -> Self {
        PetSpec {
            archetype: Archetype::Walking,
            leash_radius: PET_LEASH_RADIUS,
            scale: 1.0,
            anchor: None,
            controllers: Vec::new(),
            interactions: Vec::new(),
        }
    }
}

pub const PET_KEYS: [&str; 7] = [
    "archetype",
    "leashRadius",
    "scale",
    "anchor",
    "controllers",
    "interactions",
    "roles",
];
pub const INTERACTIONS: [&str; 5] = ["greet", "follow", "nap", "play", "chase"];
pub const LEGACY_PET: &str = "pet.json";
pub const LEGACY_REQUIRES: [&str; 2] = ["controllers", "pets"];
pub const GENERATED_CONTROLLER: &str = "animation_controllers/pet.animation_controllers.json";
pub const LOOK_BONE: &str = "head";
pub const LOOK_ANIMATION_NAME: &str = "animation.pet.look";
pub const LOOK_CONTROLLER_NAME: &str = "controller.animation.pet.look";
pub const LOOK_ANIMATION: &str = r#"{"format_version":"1.8.0","animations":{"animation.pet.look":{"loop":true,"bones":{"head":{"rotation":["math.clamp(variable.pet_look_pitch, -40, 40)","math.clamp(variable.pet_look_yaw, -70, 70)",0]}}}}}"#;
pub const LOOK_CONTROLLER: &str = r#"{"format_version":"1.10.0","animation_controllers":{"controller.animation.pet.look":{"initial_state":"look","states":{"look":{"animations":["animation.pet.look"]}}}}}"#;

const CONTROLLER_DIR: &str = "animation_controllers/";
const TEXTURE_DIR: &str = "textures/";
const GLOW_SUFFIX: &str = "_glow";
const BASE_COLOUR_SUFFIX: &str = "_red";
const CONTROLLER_SUFFIX: &str = ".animation_controllers.json";
const FORMAT_VERSION: &str = "1.10.0";
const POSE_BLEND: f64 = 0.3;
const IDLE_VARIANT_PREFIX: &str = "idle_";
const IDLE_VARIANT_PICK: &str = "variable.pet_action";
const ONCE_BLEND: f64 = 0.2;

fn manifest_at(at: &str) -> String {
    path(MANIFEST, at)
}

fn authoring(report: &mut Report, issue: Issue) {
    if report.mode() == Mode::Authoring {
        report.push(issue);
    }
}

pub fn parse(manifest: &Map<String, Value>, report: &mut Report) -> PetSpec {
    let defaults = PetSpec::default();
    let archetype = archetype(manifest.get("archetype"), report);
    PetSpec {
        archetype,
        leash_radius: positive(manifest, "leashRadius", report).unwrap_or(defaults.leash_radius),
        scale: scale(manifest, report).unwrap_or(defaults.scale),
        anchor: anchor(manifest.get("anchor"), archetype, report),
        controllers: controllers(manifest.get("controllers"), report),
        interactions: interactions(manifest.get("interactions"), report),
    }
}

fn archetype(value: Option<&Value>, report: &mut Report) -> Archetype {
    let at = manifest_at("archetype");
    match value {
        Some(Value::String(name)) => match Archetype::parse(name) {
            Some(archetype) => return archetype,
            None => report
                .push(Issue::error(IssueCode::PetArchetypeUnknown, at).name("archetype", name)),
        },
        Some(_) => report.push(Issue::error(IssueCode::PetInvalid, at)),
        None => report.push(Issue::error(IssueCode::PetArchetypeUnknown, at)),
    }
    Archetype::default()
}

fn positive(manifest: &Map<String, Value>, key: &str, report: &mut Report) -> Option<f32> {
    let value = manifest.get(key)?;
    let at = manifest_at(key);
    let Some(number) = value.as_f64() else {
        report.push(Issue::error(IssueCode::PetInvalid, at));
        return None;
    };
    let single = number as f32;
    if single.is_finite() && single > 0.0 {
        return Some(single);
    }
    report.push(Issue::error(IssueCode::PetFieldRange, at).value("value", number));
    None
}

fn scale(manifest: &Map<String, Value>, report: &mut Report) -> Option<f32> {
    let scale = positive(manifest, "scale", report)?;
    let value = manifest.get("scale").and_then(Value::as_f64)?;
    let kept = value.clamp(PET_SCALE_MIN, PET_SCALE_MAX);
    if kept != value {
        report.push(
            Issue::warning(IssueCode::PetScaleRange, manifest_at("scale"))
                .value("value", value)
                .value("kept", kept)
                .value("min", PET_SCALE_MIN)
                .value("max", PET_SCALE_MAX),
        );
    }
    Some(scale)
}

fn anchor(value: Option<&Value>, archetype: Archetype, report: &mut Report) -> Option<String> {
    let value = value?;
    let at = manifest_at("anchor");
    let known = value
        .as_str()
        .filter(|name| rig(RigKind::Player).iter().any(|bone| bone.name == *name));
    let Some(name) = known else {
        let issue = Issue::error(IssueCode::PetInvalid, at);
        report.push(match value.as_str() {
            Some(name) => issue.name("anchor", name),
            None => issue,
        });
        return None;
    };
    if archetype != Archetype::Shoulder {
        authoring(
            report,
            Issue::error(IssueCode::PetInvalid, at).name("anchor", name),
        );
        return None;
    }
    Some(name.to_string())
}

fn controllers(value: Option<&Value>, report: &mut Report) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        report.push(Issue::error(
            IssueCode::PetInvalid,
            manifest_at("controllers"),
        ));
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        match item.as_str().filter(|name| !name.is_empty()) {
            Some(name) => out.push(name.to_string()),
            None => report.push(Issue::error(
                IssueCode::PetInvalid,
                manifest_at(&join("controllers", &i.to_string())),
            )),
        }
    }
    out
}

fn interactions(value: Option<&Value>, report: &mut Report) -> Vec<InteractionDecl> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(map) = value.as_object() else {
        report.push(Issue::error(
            IssueCode::PetInvalid,
            manifest_at("interactions"),
        ));
        return Vec::new();
    };
    let mut out = Vec::with_capacity(map.len());
    for (id, roles) in map {
        let at = join("interactions", id);
        let Some(roles) = roles.as_array() else {
            report.push(Issue::error(IssueCode::PetInvalid, manifest_at(&at)));
            continue;
        };
        if !INTERACTIONS.contains(&id.as_str()) {
            authoring(
                report,
                Issue::error(IssueCode::PetInteractionUnknown, manifest_at(&at))
                    .name("interaction", id),
            );
            continue;
        }
        let mut decl = InteractionDecl {
            id: id.clone(),
            as_a: false,
            as_b: false,
        };
        for (i, role) in roles.iter().enumerate() {
            let role_at = manifest_at(&join(&at, &i.to_string()));
            match role.as_str() {
                Some("a") => decl.as_a = true,
                Some("b") => decl.as_b = true,
                Some(other) => authoring(
                    report,
                    Issue::error(IssueCode::PetInvalid, role_at).name("role", other),
                ),
                None => report.push(Issue::error(IssueCode::PetInvalid, role_at)),
            }
        }
        if decl.as_a || decl.as_b || roles.is_empty() {
            out.push(decl);
        } else {
            authoring(
                report,
                Issue::error(IssueCode::PetInvalid, manifest_at(&at)).name("interaction", id),
            );
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Base,
    Pose,
    Once,
}

#[derive(Clone, Copy)]
struct Def {
    state: &'static str,
    chain: &'static [&'static str],
    enter: &'static str,
    shape: Shape,
    required: bool,
    latch: &'static str,
    arm: Option<(&'static str, &'static str)>,
    next: Option<&'static str>,
    variants: bool,
    moving: bool,
    stride: Option<f32>,
}

const fn pose(state: &'static str, chain: &'static [&'static str], enter: &'static str) -> Def {
    Def {
        state,
        chain,
        enter,
        shape: Shape::Pose,
        required: false,
        latch: "",
        arm: None,
        next: None,
        variants: false,
        moving: false,
        stride: None,
    }
}

const fn once(
    state: &'static str,
    chain: &'static [&'static str],
    enter: &'static str,
    latch: &'static str,
) -> Def {
    Def {
        shape: Shape::Once,
        latch,
        ..pose(state, chain, enter)
    }
}

const IDLE: Def = Def {
    shape: Shape::Base,
    required: true,
    ..pose("idle", &["idle"], "")
};
const SLEEP: Def = pose("sleep", &["sleep", "lie", "sit"], "variable.pet_mode == 5");
const RIDE: Def = pose("ride", &["ride", "sit"], "variable.pet_mode == 4");
const FLINCH: Def = once("flinch", &["flinch"], "query.owner.is_hurt", "pet_flinched");
const SWIM: Def = pose("swim", &["swim"], "variable.pet_in_water");
const GLIDE: Def = pose("glide", &["glide"], "query.owner.is_gliding");
const RUN: Def = Def {
    moving: true,
    ..pose("run", &["run"], "variable.pet_mode == 2")
};
const WALKING_RUN: Def = Def {
    stride: Some(PET_RUN_CLIP_SPEED),
    ..RUN
};
const JUMP: Def = pose("jump", &["jump"], "!variable.pet_on_ground");
const MOVING: &str = "variable.pet_mode == 1 || variable.pet_mode == 2 || variable.pet_mode == 6 || variable.pet_mode == 7";
const MOVE: Def = Def {
    required: true,
    moving: true,
    ..pose("move", &["move", "idle"], MOVING)
};
const WALKING_MOVE: Def = Def {
    stride: Some(PET_MOVE_CLIP_SPEED),
    ..MOVE
};
const SIT: Def = pose(
    "sit",
    &["sit"],
    "variable.pet_mode == 3 || variable.pet_action == 1",
);
const LIE: Def = pose("lie", &["lie", "sit"], "variable.pet_action == 2");
const GROOM: Def = pose("groom", &["groom"], "variable.pet_action == 3");
const YAWN: Def = pose("yawn", &["yawn"], "variable.pet_action == 4");
const SNIFF: Def = pose("sniff", &["sniff"], "variable.pet_action == 5");
const PERCH: Def = pose("perch", &["perch", "sit"], "variable.pet_action == 6");
const IDLE_ACTION: Def = Def {
    variants: true,
    ..pose("idle_action", &[], "variable.pet_mode == 8")
};
const CELEBRATE: Def = once(
    "celebrate",
    &["celebrate"],
    "query.owner.is_emoting",
    "pet_celebrated",
);
const ALERT: Def = pose("alert", &["alert"], "query.owner.is_in_combat");
const RAIN: Def = Def {
    next: Some("shake"),
    ..pose("rain", &["rain"], "query.owner.is_in_rain")
};
const SHAKE: Def = Def {
    arm: Some(("pet_wet", "query.owner.is_in_rain")),
    ..once(
        "shake",
        &["shake"],
        "!query.owner.is_in_rain && variable.pet_wet",
        "pet_shook",
    )
};
const PERCHED_SLEEP: Def = pose(
    "sleep",
    &["sleep", "lie", "sit"],
    "variable.pet_mode == 5 || query.owner.is_sleeping",
);
const PERCHED_RIDE: Def = pose(
    "ride",
    &["ride", "sit"],
    "variable.pet_mode == 4 || query.owner.is_riding",
);
const PERCHED_MOVING: &str = "variable.pet_mode == 1 || variable.pet_mode == 2 || ((variable.pet_mode == 6 || variable.pet_mode == 7) && variable.pet_speed > 0.25) || query.owner.is_moving";
const PERCHED_MOVE: Def = Def {
    required: true,
    ..pose("move", &["move", "idle"], PERCHED_MOVING)
};

const WALKING: [Def; 17] = [
    IDLE,
    SLEEP,
    RIDE,
    FLINCH,
    SWIM,
    WALKING_RUN,
    SHAKE,
    WALKING_MOVE,
    SIT,
    CELEBRATE,
    ALERT,
    LIE,
    GROOM,
    YAWN,
    SNIFF,
    IDLE_ACTION,
    RAIN,
];
const FLYING: [Def; 18] = [
    IDLE,
    SLEEP,
    RIDE,
    FLINCH,
    GLIDE,
    RUN,
    SHAKE,
    ALERT,
    MOVE,
    SIT,
    CELEBRATE,
    PERCH,
    LIE,
    GROOM,
    YAWN,
    SNIFF,
    IDLE_ACTION,
    RAIN,
];
const SHOULDER: [Def; 16] = [
    IDLE,
    PERCHED_SLEEP,
    PERCHED_RIDE,
    FLINCH,
    GLIDE,
    SHAKE,
    PERCHED_MOVE,
    SIT,
    CELEBRATE,
    ALERT,
    LIE,
    GROOM,
    YAWN,
    SNIFF,
    IDLE_ACTION,
    RAIN,
];

fn defs(a: Archetype, legacy: bool) -> Vec<Def> {
    let mut defs = match a {
        Archetype::Walking => WALKING.to_vec(),
        Archetype::Flying => FLYING.to_vec(),
        Archetype::Shoulder => SHOULDER.to_vec(),
    };
    if legacy && a == Archetype::Walking {
        let at = defs
            .iter()
            .position(|d| d.state == SWIM.state)
            .map_or(0, |i| i + 1);
        defs.insert(at, JUMP);
    }
    defs
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Missing {
    Name,
    Empty,
}

fn entry(key: &str, value: Value) -> Value {
    let mut map = Map::new();
    map.insert(key.to_string(), value);
    Value::Object(map)
}

fn entering(d: &Def) -> String {
    if d.moving {
        format!("({}) && variable.pet_speed > {PET_MOVING_SPEED}", d.enter)
    } else {
        d.enter.to_string()
    }
}

fn condition(d: &Def) -> String {
    if d.latch.is_empty() {
        entering(d)
    } else {
        format!("{} && !variable.{}", entering(d), d.latch)
    }
}

fn stride(natural: f32) -> String {
    format!(
        "query.anim_time + query.delta_time * math.clamp(variable.pet_speed / {natural}, 0, {PET_CLIP_RATE_MAX})"
    )
}

fn idle_variants(animations: &[&str]) -> Vec<String> {
    animations
        .iter()
        .filter(|name| {
            name.rsplit('.')
                .next()
                .and_then(|short| short.strip_prefix(IDLE_VARIANT_PREFIX))
                .is_some_and(|rest| !rest.is_empty())
        })
        .map(|name| name.to_string())
        .collect()
}

fn state_animations(names: Vec<String>, variants: bool) -> Vec<Value> {
    let count = names.len();
    if !variants || count < 2 {
        return names.into_iter().map(Value::from).collect();
    }
    names
        .into_iter()
        .enumerate()
        .map(|(k, name)| {
            let weight = format!("math.mod({IDLE_VARIANT_PICK}, {count}) == {k}");
            entry(&name, Value::from(weight))
        })
        .collect()
}

fn build(
    a: Archetype,
    pick: &dyn Fn(&str) -> Option<String>,
    variants: &[String],
    missing: Missing,
    legacy: bool,
) -> Value {
    let defs = defs(a, legacy);
    let defs = defs.as_slice();
    let mut chosen: Vec<Option<Vec<String>>> = defs
        .iter()
        .map(|d| {
            if d.variants {
                (!variants.is_empty()).then(|| variants.to_vec())
            } else {
                d.chain.iter().find_map(|name| pick(name)).map(|n| vec![n])
            }
        })
        .collect();
    for (d, animations) in defs.iter().zip(chosen.iter_mut()) {
        if d.required && animations.is_none() {
            *animations = Some(match missing {
                Missing::Name => vec![d.chain[0].to_string()],
                Missing::Empty => Vec::new(),
            });
        }
    }
    let kept_state = |chosen: &[Option<Vec<String>>], name: &str| {
        defs.iter()
            .zip(chosen)
            .any(|(d, animations)| d.state == name && animations.is_some())
    };
    let kept: Vec<usize> = (0..defs.len()).filter(|&i| chosen[i].is_some()).collect();
    let mut remaps: Vec<(&str, String)> = Vec::new();
    for d in kept.iter().map(|&i| &defs[i]) {
        if !d.latch.is_empty() {
            let input = format!("{} ? variable.{} : 0", d.enter, d.latch);
            remaps.push((d.latch, input));
        }
        if let Some((name, when)) = d.arm {
            remaps.push((name, format!("{when} ? 1 : variable.{name}")));
        }
    }

    let mut states = Map::new();
    for &i in &kept {
        let d = &defs[i];
        let mut transitions: Vec<Value> = kept
            .iter()
            .copied()
            .filter(|&j| match d.shape {
                Shape::Base => j != i,
                _ => j < i,
            })
            .filter(|&j| matches!(defs[j].shape, Shape::Pose | Shape::Once))
            .map(|j| entry(defs[j].state, Value::from(condition(&defs[j]))))
            .collect();
        match d.shape {
            Shape::Base => {}
            Shape::Pose => {
                let target = d
                    .next
                    .filter(|next| kept_state(&chosen, next))
                    .unwrap_or(IDLE.state);
                transitions.push(entry(target, Value::from(format!("!({})", entering(d)))));
            }
            Shape::Once => transitions.push(entry(
                IDLE.state,
                Value::from("query.all_animations_finished"),
            )),
        }
        let mut state = Map::new();
        let animations = chosen[i].clone().unwrap_or_default();
        state.insert(
            "animations".to_string(),
            Value::Array(state_animations(animations, d.variants)),
        );
        if !transitions.is_empty() {
            state.insert("transitions".to_string(), Value::Array(transitions));
        }
        let blend = match d.shape {
            Shape::Base | Shape::Pose => POSE_BLEND,
            Shape::Once => ONCE_BLEND,
        };
        state.insert("blend_transition".to_string(), Value::from(blend));
        if let Some(natural) = d.stride {
            state.insert("anim_time_update".to_string(), Value::from(stride(natural)));
        }
        if !d.latch.is_empty() {
            let set = format!("variable.{} = 1;", d.latch);
            state.insert("on_entry".to_string(), Value::Array(vec![Value::from(set)]));
        }
        if matches!(d.shape, Shape::Base | Shape::Pose) && !remaps.is_empty() {
            let mut variables = Map::new();
            for (name, input) in &remaps {
                variables.insert(
                    name.to_string(),
                    entry("input", Value::from(input.as_str())),
                );
            }
            state.insert("variables".to_string(), Value::Object(variables));
        }
        states.insert(d.state.to_string(), Value::Object(state));
    }

    let mut controller = Map::new();
    controller.insert("initial_state".to_string(), Value::from(IDLE.state));
    controller.insert("states".to_string(), Value::Object(states));
    let mut root = Map::new();
    root.insert("format_version".to_string(), Value::from(FORMAT_VERSION));
    root.insert(
        "animation_controllers".to_string(),
        entry(default_controller_name(a), Value::Object(controller)),
    );
    Value::Object(root)
}

fn render(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn find(animations: &[&str], short: &str) -> Option<String> {
    if animations.contains(&short) {
        return Some(short.to_string());
    }
    let mut found = animations.iter().filter(|name| {
        name.strip_suffix(short)
            .is_some_and(|rest| rest.ends_with('.'))
    });
    match (found.next(), found.next()) {
        (Some(name), None) => Some(name.to_string()),
        _ => None,
    }
}

pub fn default_controller_name(a: Archetype) -> &'static str {
    match a {
        Archetype::Walking => "controller.animation.pet.walking",
        Archetype::Flying => "controller.animation.pet.flying",
        Archetype::Shoulder => "controller.animation.pet.shoulder",
    }
}

pub fn default_controller(a: Archetype) -> &'static str {
    static DEFAULTS: OnceLock<[String; 3]> = OnceLock::new();
    let all = DEFAULTS.get_or_init(|| {
        Archetype::ALL.map(|a| {
            render(&build(
                a,
                &|name| Some(name.to_string()),
                &[],
                Missing::Name,
                false,
            ))
        })
    });
    &all[a as usize]
}

pub fn uses_default_controller(spec: &PetSpec) -> bool {
    match spec.controllers.as_slice() {
        [] => true,
        [only] => only == default_controller_name(spec.archetype),
        _ => false,
    }
}

pub fn default_controller_for(a: Archetype, animations: &[&str]) -> String {
    let variants = idle_variants(animations);
    render(&build(
        a,
        &|name| find(animations, name),
        &variants,
        Missing::Name,
        false,
    ))
}

fn legacy_archetype(name: &str) -> Option<Archetype> {
    match name.to_lowercase().as_str() {
        "flying" | "fly" => Some(Archetype::Flying),
        "walking" | "walk" | "land" => Some(Archetype::Walking),
        "shoulder" => Some(Archetype::Shoulder),
        _ => None,
    }
}

fn legacy_number(value: Option<&Value>) -> Option<Option<f64>> {
    let number = match value {
        None => return Some(None),
        Some(Value::Number(n)) => n.as_f64()?,
        Some(Value::String(s)) => s.trim().parse::<f64>().ok()?,
        Some(_) => return None,
    };
    let single = number as f32;
    Some((single.is_finite() && single > 0.0).then_some(number))
}

fn legacy_anchor(name: &str) -> Option<&'static str> {
    let key = name.to_lowercase().replace('_', "");
    Some(match key.as_str() {
        "root" => "root",
        "head" => "head",
        "helmet" => "hat",
        "body" => "body",
        "rightarm" => "rightArm",
        "leftarm" => "leftArm",
        "rightleg" => "rightLeg",
        "leftleg" => "leftLeg",
        _ => return None,
    })
}

fn file_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn legacy_id(geometry: &str) -> String {
    let file = file_name(geometry);
    let stem = file.strip_suffix(".geo.json").unwrap_or(file);
    let id: String = stem
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(ID_MAX_LEN)
        .collect();
    if is_id(&id) {
        id
    } else {
        "pet".to_string()
    }
}

fn is_legacy_animation(name: &str) -> bool {
    name.ends_with(".json")
        && !name.ends_with(".geo.json")
        && name != LEGACY_PET
        && !name.ends_with(CONTROLLER_SUFFIX)
}

fn unused_name(files: &[(String, Vec<u8>)], wanted: &str) -> String {
    let taken = |name: &str| files.iter().any(|(n, _)| n == name);
    if !taken(wanted) {
        return wanted.to_string();
    }
    let stem = wanted.strip_suffix(CONTROLLER_SUFFIX).unwrap_or(wanted);
    (2..)
        .map(|n| format!("{stem}_{n}{CONTROLLER_SUFFIX}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

pub fn convert_legacy(files: &[(String, Vec<u8>)]) -> Option<Vec<(String, Vec<u8>)>> {
    convert_legacy_with_skin(files).map(|(out, _)| out)
}

type Files = Vec<(String, Vec<u8>)>;

pub(crate) fn convert_legacy_with_skin(files: &[(String, Vec<u8>)]) -> Option<(Files, String)> {
    if files.iter().any(|(name, _)| name == MANIFEST) {
        return None;
    }
    let mut scratch = Report::new(Mode::Client);
    let (_, bytes) = files.iter().find(|(name, _)| name == LEGACY_PET)?;
    let old = guarded_json(bytes, LEGACY_PET, &mut scratch)?;
    let old = old.as_object()?;
    let leash = legacy_number(old.get("leashRadius"))?;
    let scale = legacy_number(old.get("scale"))?;
    let states: Vec<(String, String)> = match old.get("states") {
        None => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .filter_map(|(state, name)| Some((state.clone(), name.as_str()?.to_string())))
            .collect(),
        Some(_) => return None,
    };

    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let (geometry, _) = sorted
        .iter()
        .find(|(name, _)| name.ends_with(".geo.json") && !name.ends_with("player.geo.json"))?;
    let stem = geometry_stem(geometry);
    let skin = legacy_skin(&sorted, &stem)?;

    let named = old
        .get("controller")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .and_then(|name| {
            let (file, bytes) = sorted
                .iter()
                .find(|(file, _)| file.ends_with(CONTROLLER_SUFFIX))?;
            let root = guarded_json(bytes, file, &mut scratch)?;
            root.get("animation_controllers")?.as_object()?.get(name)?;
            Some((file.as_str(), bytes, name.to_string()))
        });

    let mut animations: Vec<String> = Vec::new();
    for (file, bytes) in sorted.iter().filter(|(file, _)| is_legacy_animation(file)) {
        let listed = guarded_json(bytes, file, &mut scratch);
        if let Some(Value::Object(clips)) = listed.as_ref().and_then(|r| r.get("animations")) {
            animations.extend(clips.keys().cloned());
        }
    }
    let names: Vec<&str> = animations.iter().map(String::as_str).collect();
    let archetype = match old.get("archetype") {
        None if find_own(&names, "fly", &stem).is_some() => Archetype::Flying,
        None => Archetype::Walking,
        Some(name) => legacy_archetype(name.as_str()?)?,
    };

    let (controller_file, controller_bytes, controller_name, moved) = match named {
        Some((file, bytes, name)) => {
            let wanted = format!("{CONTROLLER_DIR}{}", file_name(file));
            let target =
                if file.starts_with(CONTROLLER_DIR) || files.iter().any(|(n, _)| *n == wanted) {
                    file.to_string()
                } else {
                    wanted
                };
            (target, bytes.clone(), name, Some(file))
        }
        None => {
            let pick = |short: &str| -> Option<String> {
                if let Some((_, name)) = states.iter().find(|(state, _)| state == short) {
                    return Some(name.clone());
                }
                let fallbacks: &[&str] = match (short, archetype) {
                    ("move", Archetype::Flying) => &["fly", "walk"],
                    ("move", _) => &["walk"],
                    _ => &[],
                };
                find_own(&names, short, &stem)
                    .or_else(|| {
                        fallbacks
                            .iter()
                            .find_map(|name| find_own(&names, name, &stem))
                    })
                    .or_else(|| {
                        if short == IDLE.state || short == MOVE.state {
                            states.first().map(|(_, first)| first.clone())
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        if short == IDLE.state && archetype == Archetype::Shoulder {
                            animations.first().cloned()
                        } else {
                            None
                        }
                    })
            };
            let controller = build(
                archetype,
                &pick,
                &idle_variants(&names),
                Missing::Empty,
                true,
            );
            let bytes = render(&controller).into_bytes();
            let file = unused_name(files, GENERATED_CONTROLLER);
            (
                file,
                bytes,
                default_controller_name(archetype).to_string(),
                None,
            )
        }
    };

    let mut manifest = Map::new();
    manifest.insert("format".to_string(), Value::from(FORMAT));
    manifest.insert("kind".to_string(), Value::from(Kind::Pet.as_str()));
    manifest.insert("id".to_string(), Value::from(legacy_id(geometry)));
    manifest.insert(
        "requires".to_string(),
        Value::Array(LEGACY_REQUIRES.iter().map(|r| Value::from(*r)).collect()),
    );
    manifest.insert("archetype".to_string(), Value::from(archetype.as_str()));
    if let Some(leash) = leash {
        manifest.insert("leashRadius".to_string(), Value::from(leash));
    }
    if let Some(scale) = scale {
        manifest.insert("scale".to_string(), Value::from(scale));
    }
    let anchor = old
        .get("anchor")
        .and_then(Value::as_str)
        .and_then(legacy_anchor)
        .filter(|_| archetype == Archetype::Shoulder);
    if let Some(anchor) = anchor {
        manifest.insert("anchor".to_string(), Value::from(anchor));
    }
    manifest.insert(
        "controllers".to_string(),
        Value::Array(vec![Value::from(controller_name)]),
    );
    let manifest = serde_json::to_vec_pretty(&Value::Object(manifest)).ok()?;

    let skin_path = if skin.starts_with(TEXTURE_DIR) {
        skin.clone()
    } else {
        format!("{TEXTURE_DIR}{}", file_name(&skin))
    };
    let mut out = Vec::with_capacity(files.len() + 1);
    out.push((MANIFEST.to_string(), manifest));
    out.push((controller_file, controller_bytes));
    for (name, bytes) in files {
        if name == LEGACY_PET || moved == Some(name.as_str()) {
            continue;
        }
        let name = if *name == skin {
            skin_path.clone()
        } else {
            name.clone()
        };
        out.push((name, bytes.clone()));
    }
    Some((out, skin_path))
}

fn geometry_stem(geometry: &str) -> String {
    let file = file_name(geometry);
    file.strip_suffix(".geo.json")
        .unwrap_or(file)
        .to_ascii_lowercase()
}

fn png_stem(name: &str) -> Option<String> {
    let lower = file_name(name).to_ascii_lowercase();
    lower.strip_suffix(".png").map(str::to_string)
}

fn legacy_skin(sorted: &[&(String, Vec<u8>)], stem: &str) -> Option<String> {
    let skins: Vec<(&str, String)> = sorted
        .iter()
        .filter_map(|(name, _)| Some((name.as_str(), png_stem(name)?)))
        .filter(|(_, png)| !png.ends_with(GLOW_SUFFIX))
        .collect();
    skins
        .iter()
        .find(|(_, png)| png == stem)
        .or_else(|| {
            skins
                .iter()
                .find(|(_, png)| png.ends_with(BASE_COLOUR_SUFFIX))
        })
        .or_else(|| skins.first())
        .map(|(name, _)| name.to_string())
}

fn find_own(animations: &[&str], short: &str, stem: &str) -> Option<String> {
    let own = format!(".{stem}.");
    if !animations.iter().any(|name| name.contains(&own)) {
        return find(animations, short);
    }
    let mine: Vec<&str> = animations
        .iter()
        .copied()
        .filter(|name| name.contains(&own))
        .collect();
    find(&mine, short)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::animation::{self, channel_offsets, flag_words, Animation};
    use crate::controller::{self, Clips, ControllerSet, Scratch, Slots};
    use crate::issues::Level;
    use crate::molang::{Interner, Rules, Runtime};
    use crate::player::{self, Arms};
    use crate::pose::Pose;
    use crate::reactions::Speed;
    use crate::signals::SIGNAL_COUNT;
    use crate::wearer::{seed, CosmeticSlot, WearerDeclarations, WearerRecord, WearerTable};

    fn read(value: Value, mode: Mode) -> (PetSpec, Vec<Issue>) {
        let mut report = Report::new(mode);
        let spec = parse(value.as_object().unwrap(), &mut report);
        (spec, report.into_issues())
    }

    #[test]
    fn pet_manifests_read_every_field_and_reject_bad_ones() {
        let (spec, issues) = read(
            json!({
                "archetype": "shoulder", "leashRadius": 5, "scale": 0.5, "anchor": "rightArm",
                "controllers": ["controller.animation.parrot.main"],
                "interactions": { "greet": ["a", "b"], "nap": ["b"] }
            }),
            Mode::Authoring,
        );
        assert!(issues.is_empty(), "{issues:?}");
        let decl = |id: &str, as_a, as_b| InteractionDecl { id: id.to_string(), as_a, as_b };
        let expected = PetSpec {
            archetype: Archetype::Shoulder,
            leash_radius: 5.0,
            scale: 0.5,
            anchor: Some("rightArm".to_string()),
            controllers: vec!["controller.animation.parrot.main".to_string()],
            interactions: vec![decl("greet", true, true), decl("nap", false, true)],
        };
        assert_eq!(spec, expected);

        let cases = [
            (json!({}), vec![IssueCode::PetArchetypeUnknown]),
            (json!({ "archetype": 3 }), vec![IssueCode::PetInvalid]),
            (json!({ "archetype": "walking", "leashRadius": 0, "scale": -1 }), vec![IssueCode::PetFieldRange; 2]),
            (json!({ "archetype": "shoulder", "anchor": "tail" }), vec![IssueCode::PetInvalid]),
            (json!({ "archetype": "walking", "interactions": ["greet"] }), vec![IssueCode::PetInvalid]),
        ];
        for (value, expected) in cases {
            let (_, issues) = read(value.clone(), Mode::Client);
            assert_eq!(issues.iter().map(|i| i.code).collect::<Vec<_>>(), expected, "{value}");
            assert!(issues.iter().all(|i| i.level == Level::Error));
        }
        let (spec, _) = read(json!({ "archetype": "flying", "leashRadius": -2, "scale": 0 }), Mode::Client);
        assert_eq!((spec.leash_radius, spec.scale), (3.0, 1.0));
    }

    fn clip_json(clips: &[&str]) -> String {
        let body: Vec<String> = clips
            .iter()
            .map(|name| {
                let looped = !matches!(*name, "flinch" | "shake" | "celebrate");
                format!(r#""animation.cat.{name}":{{"loop":{looped},"animation_length":0.5,"bones":{{}}}}"#)
            })
            .collect();
        format!(r#"{{"format_version":"1.8.0","animations":{{{}}}}}"#, body.join(","))
    }

    struct Pet {
        set: ControllerSet,
        interner: Interner,
        animations: Vec<Animation>,
        binds: Vec<Vec<Option<usize>>>,
        offsets: Vec<usize>,
        disabled: Vec<u32>,
        table: WearerTable,
        slot: CosmeticSlot,
        rt: Runtime,
        scratch: Scratch,
        pose: Pose,
    }

    impl Pet {
        fn load(source: &str, clips: &[&str], rules: Rules) -> (Pet, Vec<Issue>) {
            let mut interner = Interner::default();
            let animations = animation::parse(&clip_json(clips), &mut interner).unwrap();
            let mut report = Report::new(Mode::Authoring);
            let root: Value = serde_json::from_str(source).unwrap();
            let loaded = controller::parse("animation_controllers/pet.json", &root, &mut interner, rules, &mut report);
            assert_eq!(loaded.len(), 1, "{:?}", report.issues());
            let mut set = ControllerSet::select(loaded, &[], &mut report);
            set.link(&animations, &interner, &mut report);
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
            let pet = Pet {
                binds: animations.iter().map(|a| a.bind(&model)).collect(),
                offsets,
                disabled: vec![0; flag_words(channels)],
                rt: Runtime::new(&interner),
                pose: Pose::rest(&model),
                set,
                interner,
                animations,
                table,
                slot,
                scratch: Scratch::default(),
            };
            (pet, report.into_issues())
        }

        fn state(&mut self, clock: f64, mode: PetMode, owner_moving: bool) -> String {
            self.table.begin_frame(clock);
            let record = self.table.draw(self.slot, 1, &[0.0; SIGNAL_COUNT], 0).unwrap();
            let WearerRecord { variables, controller_states, controller_entered, now, epoch, appear, times_reset, effect_times, effect_floats, .. } =
                record;
            let moving = matches!(mode, PetMode::Following | PetMode::CatchingUp | PetMode::Wandering | PetMode::Playing);
            for (name, value) in [("pet_mode", mode.value()), ("pet_speed", if moving { PET_MOVE_CLIP_SPEED } else { 0.0 })] {
                if let Some(slot) = self.interner.variable_slot(name) {
                    variables[slot as usize] = value;
                }
            }
            let mut scope = self.rt.wearer_frame(variables, seed(1, 0, 0), clock);
            if let Some(slot) = self.interner.query_slot("owner.is_moving") {
                scope.set_query(slot, if owner_moving { 1.0 } else { 0.0 });
            }
            self.pose.reset();
            let mut clips = Clips {
                animations: &self.animations,
                binds: &self.binds,
                offsets: &self.offsets,
                disabled: &mut self.disabled,
                first_frame: false,
                props: None,
            };
            let slots = Slots {
                states: controller_states,
                entered: controller_entered,
                delays: &mut [],
                anim_times: effect_floats,
                anim_clock: effect_times.first_mut(),
                now,
                epoch,
                appear,
                times_reset,
                speed: Speed::default(),
                timelines: false,
            };
            let failures = self.set.step(slots, &mut scope, &mut clips, &mut self.scratch, &mut self.pose, &mut Vec::new());
            assert_eq!(failures, 0);
            let record = self.table.record(self.slot, 1).unwrap();
            self.set.controller_state(&record, 0).unwrap().0.to_string()
        }
    }

    #[test]
    fn default_controllers_parse_strictly_and_walk_between_idle_and_move() {
        let every = [
            "idle", "move", "run", "swim", "glide", "sit", "ride", "sleep", "alert", "rain", "flinch", "shake", "celebrate", "lie",
            "groom", "yawn", "sniff", "perch",
        ];
        for archetype in Archetype::ALL {
            let source = default_controller(archetype);
            let root: Value = serde_json::from_str(source).unwrap();
            assert!(root["animation_controllers"].get(default_controller_name(archetype)).is_some());
            for rules in [Rules::Strict, Rules::Bedrock] {
                let (_, issues) = Pet::load(source, &every, rules);
                assert!(issues.is_empty(), "{archetype:?}: {issues:?}");
            }
            let (mut pet, issues) = Pet::load(source, &["idle", "move"], Rules::Strict);
            assert!(issues.iter().all(|i| i.code == IssueCode::ControllerUnknownAnimation && i.level == Level::Warning));
            assert_eq!(pet.state(0.0, PetMode::Idle, false), "idle");
            pet.state(0.1, PetMode::Following, true);
            assert_eq!(pet.state(1.0, PetMode::Following, true), "move", "{archetype:?}");
            pet.state(1.1, PetMode::Idle, false);
            assert_eq!(pet.state(2.0, PetMode::Idle, false), "idle");
        }
    }

    #[test]
    fn legacy_pet_json_becomes_a_manifest_and_a_controller() {
        let pet = json!({
            "archetype": "Land", "leashRadius": 4, "scale": "0.5",
            "states": { "idle": "animation.cat.sit_loop", "move": "animation.cat.walk" }
        });
        let clips = ["sit_loop", "walk", "sleep"];
        let files = vec![
            ("pet.json".to_string(), pet.to_string().into_bytes()),
            ("models/Cat Model.geo.json".to_string(), br#"{"format_version":"1.12.0","minecraft:geometry":[]}"#.to_vec()),
            ("textures/cat.png".to_string(), vec![0x89, b'P', b'N', b'G']),
            ("animations/cat.animation.json".to_string(), clip_json(&clips).into_bytes()),
        ];
        let converted = convert_legacy(&files).unwrap();
        let names: Vec<&str> = converted.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, [MANIFEST, "animation_controllers/pet.animation_controllers.json", "models/Cat Model.geo.json", "textures/cat.png", "animations/cat.animation.json"]);
        let manifest: Value = serde_json::from_slice(&converted[0].1).unwrap();
        let walking = default_controller_name(Archetype::Walking);
        assert_eq!(
            manifest,
            json!({
                "format": 1, "kind": "pet", "id": "cat_model", "requires": ["controllers", "pets"],
                "archetype": "walking", "leashRadius": 4.0, "scale": 0.5, "controllers": [walking]
            })
        );
        let (spec, issues) = read(manifest, Mode::Authoring);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!((spec.archetype, spec.leash_radius, spec.scale), (Archetype::Walking, 4.0, 0.5));

        let (mut pet, issues) = Pet::load(std::str::from_utf8(&converted[1].1).unwrap(), &clips, Rules::Strict);
        assert!(issues.is_empty(), "{issues:?}");
        let states: Vec<&str> = pet.set.controllers()[0].states.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(states, ["idle", "sleep", "move"]);
        pet.state(0.0, PetMode::Idle, false);
        pet.state(1.0, PetMode::Following, false);
        assert_eq!(pet.state(1.5, PetMode::Following, false), "move");
        pet.state(2.0, PetMode::Sleeping, false);
        assert_eq!(pet.state(2.5, PetMode::Sleeping, false), "sleep");
    }
}
