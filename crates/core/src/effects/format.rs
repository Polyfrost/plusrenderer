use std::mem;

use serde_json::{Map, Value};

use super::moments::{ogg_codec, ogg_vorbis};
use super::particles::{
    self, emitter_lifetime, Vars, EMITTER_AGE, EMITTER_LIFETIME, EMITTER_RANDOM, PARTICLE_AGE,
    PARTICLE_RANDOM,
};
use super::ring::{particle_points, trail_points, RING_STEP_SECONDS};
use super::{
    Blend, Direction, EffectSet, EmitterDef, Facing, Lifetime, LocatorRef, Motion, ParticleDef,
    Rate, Shape, SoundDef, Tint, TrailDef, Uv,
};
use crate::animation::{Animation, KeyEventKind};
use crate::attach::PlayerLocator;
use crate::container::{is_id, Bundle, MANIFEST};
use crate::controller::{ControllerSet, EffectKind};
use crate::ir::Model;
use crate::issues::{join, path, Issue, IssueCode, Mode, Report};
use crate::limits::{
    EFFECT_PARTICLE_LIFE_SECONDS, SOUND_CHANNELS, SOUND_DECODED_BYTES, SOUND_RATE, SOUND_SECONDS,
};
use crate::molang::{Interner, Program};
use crate::signals::{signal_id, JUMP_QUERY, JUMP_SIGNAL};

pub const MANIFEST_KEYS: &[&str] = &["particles", "sounds", "emitters", "effects", "trails"];
pub const MOMENTS: [&str; 4] = ["join", "respawn", "death", "kill"];
pub const VANILLA_SOUNDS: &[&str] = &[
    "minecraft:block.amethyst_block.chime",
    "minecraft:block.beacon.activate",
    "minecraft:block.bubble_column.bubble_pop",
    "minecraft:block.enchantment_table.use",
    "minecraft:block.end_portal_frame.fill",
    "minecraft:block.fire.extinguish",
    "minecraft:block.note_block.bell",
    "minecraft:block.note_block.chime",
    "minecraft:block.note_block.flute",
    "minecraft:block.note_block.harp",
    "minecraft:block.note_block.pling",
    "minecraft:block.note_block.xylophone",
    "minecraft:block.respawn_anchor.charge",
    "minecraft:entity.allay.ambient_without_item",
    "minecraft:entity.evoker.cast_spell",
    "minecraft:entity.experience_orb.pickup",
    "minecraft:entity.firework_rocket.blast",
    "minecraft:entity.firework_rocket.launch",
    "minecraft:entity.firework_rocket.twinkle",
    "minecraft:entity.illusioner.cast_spell",
    "minecraft:entity.player.levelup",
];
pub const VANILLA_PARTICLES: &[&str] = &[
    "minecraft:bubble_pop",
    "minecraft:cherry_leaves",
    "minecraft:cloud",
    "minecraft:composter",
    "minecraft:crit",
    "minecraft:electric_spark",
    "minecraft:enchant",
    "minecraft:enchanted_hit",
    "minecraft:end_rod",
    "minecraft:firework",
    "minecraft:flame",
    "minecraft:glow",
    "minecraft:happy_villager",
    "minecraft:heart",
    "minecraft:note",
    "minecraft:poof",
    "minecraft:portal",
    "minecraft:reverse_portal",
    "minecraft:scrape",
    "minecraft:small_flame",
    "minecraft:snowflake",
    "minecraft:soul",
    "minecraft:soul_fire_flame",
    "minecraft:totem_of_undying",
    "minecraft:wax_off",
    "minecraft:wax_on",
    "minecraft:white_ash",
    "minecraft:witch",
];

const ROOT_KEYS: &[&str] = &["format_version", "particle_effect"];
const EFFECT_KEYS: &[&str] = &["description", "curves", "events", "components"];
const DESCRIPTION_KEYS: &[&str] = &["identifier", "basic_render_parameters"];
const RENDER_KEYS: &[&str] = &["material", "texture"];
const PARTICLE_OPTION_KEYS: &[&str] = &["tint"];
const SLOT_KEYS: &[&str] = &["slot", "tone"];
const SOUND_KEYS: &[&str] = &["vanilla", "volume", "pitch"];
const EMITTER_KEYS: &[&str] = &["particle", "sound", "vanilla", "locator", "count"];
const TRAIL_KEYS: &[&str] = &[
    "locator",
    "texture",
    "blend",
    "width",
    "tile",
    "tint",
    "fade",
    "min_speed",
    "while",
];
const GRADIENT_KEYS: &[&str] = &["interpolant", "gradient"];
const VANILLA_COUNT_MAX: f32 = 16.0;
const DEFAULT_SIZE: f32 = 0.1;
const DEFAULT_ACTIVE_SECONDS: f32 = 10.0;
const DEFAULT_FADE_SECONDS: f32 = 1.0;
const PREFIX: &str = "minecraft:";

#[derive(Clone)]
struct Template {
    shape: Shape,
    offset: [f32; 3],
    rate: Rate,
    lifetime: Lifetime,
    world: bool,
    path: String,
}

struct Cx<'r> {
    interner: Interner,
    report: &'r mut Report,
}

impl Cx<'_> {
    fn error(&mut self, code: IssueCode, at: String) {
        self.report.push(Issue::error(code, at));
    }

    fn unsupported(&mut self, at: String, feature: &str) {
        self.report
            .push(Issue::error(IssueCode::EffectUnsupported, at).name("feature", feature));
    }

    fn keys(&mut self, file: &str, at: &str, object: &Map<String, Value>, known: &[&str]) {
        self.report.check_keys(file, at, object, known);
    }

    fn keys_at(&mut self, at: &str, object: &Map<String, Value>, known: &[&str]) {
        if self.report.mode() != Mode::Authoring {
            return;
        }
        for key in object.keys().filter(|k| !known.contains(&k.as_str())) {
            self.report
                .push(Issue::error(IssueCode::UnknownKey, join(at, key)).name("key", key));
        }
    }

    fn compile(&mut self, source: &str, at: &str) -> Option<Program> {
        if assigns_variable(source) {
            self.unsupported(at.to_string(), "particle_script");
            return None;
        }
        match particles::compile(source, &mut self.interner) {
            Ok(program) => Some(program),
            Err(error) => {
                self.report.push(
                    Issue::error(IssueCode::from(error.code), at).name("error", error.message()),
                );
                None
            }
        }
    }

    fn expr(&mut self, value: Option<&Value>, default: f32, at: &str) -> Program {
        match value {
            None | Some(Value::Null) => Program::number(default),
            Some(Value::Number(n)) => Program::number(n.as_f64().unwrap_or(0.0) as f32),
            Some(Value::Bool(b)) => Program::number(if *b { 1.0 } else { 0.0 }),
            Some(Value::String(s)) if s.trim().is_empty() => Program::number(default),
            Some(Value::String(s)) => self
                .compile(s, at)
                .unwrap_or_else(|| Program::number(default)),
            Some(_) => {
                self.error(IssueCode::EffectInvalid, at.to_string());
                Program::number(default)
            }
        }
    }

    fn timeless(&mut self, value: Option<&Value>, default: f32, at: &str) -> Program {
        let program = self.expr(value, default, at);
        if program.reads().variables.contains(&(PARTICLE_AGE as u16)) {
            self.unsupported(at.to_string(), "changes_over_age");
            return Program::number(default);
        }
        program
    }

    fn constant(&mut self, value: Option<&Value>, default: f32, at: &str) -> f32 {
        let program = self.expr(value, default, at);
        if !program.reads().variables.is_empty() {
            self.unsupported(at.to_string(), "expression");
            return default;
        }
        Vars::default().eval(&program, 0)
    }

    fn vec3(&mut self, value: Option<&Value>, default: f32, at: &str) -> [Program; 3] {
        match value {
            None | Some(Value::Null) => [0, 1, 2].map(|_| Program::number(default)),
            Some(Value::Array(items)) if items.len() == 3 => {
                [0, 1, 2].map(|i| self.expr(items.get(i), default, &join(at, &i.to_string())))
            }
            Some(_) => {
                self.error(IssueCode::EffectInvalid, at.to_string());
                [0, 1, 2].map(|_| Program::number(default))
            }
        }
    }

    fn const3(&mut self, value: Option<&Value>, default: f32, at: &str) -> [f32; 3] {
        match value {
            None | Some(Value::Null) => [default; 3],
            Some(Value::Array(items)) if items.len() == 3 => {
                [0, 1, 2].map(|i| self.constant(items.get(i), default, &join(at, &i.to_string())))
            }
            Some(_) => {
                self.error(IssueCode::EffectInvalid, at.to_string());
                [default; 3]
            }
        }
    }

    fn const2(&mut self, value: Option<&Value>, default: f32, at: &str) -> [f32; 2] {
        match value {
            None | Some(Value::Null) => [default; 2],
            Some(Value::Array(items)) if items.len() == 2 => {
                [0, 1].map(|i| self.constant(items.get(i), default, &join(at, &i.to_string())))
            }
            Some(_) => {
                self.error(IssueCode::EffectInvalid, at.to_string());
                [default; 2]
            }
        }
    }
}

pub(super) fn parse(b: &mut Bundle<'_>, m: Option<&Model>, r: &mut Report) -> EffectSet {
    let manifest = b.manifest().clone();
    let mut cx = Cx {
        interner: particles::interner(),
        report: r,
    };
    let mut set = EffectSet {
        locators: m.map_or_else(Vec::new, |m| {
            m.locators().map(|(_, l)| l.name.clone()).collect()
        }),
        ..EffectSet::default()
    };

    let mut templates = Vec::new();
    let mut broken = Vec::new();
    for (id, options, at) in entries(&manifest, "particles", &mut cx) {
        let Some(file) = b.id_path("particles", &id, "json", &at) else {
            continue;
        };
        let Some(json) = b.json(&file, &at) else {
            cx.error(IssueCode::ParticleMissing, at.clone());
            continue;
        };
        if let Some((mut particle, template)) = particle(&mut cx, b, &file, &id, &json) {
            if let Some(options) = options.as_object() {
                cx.keys(MANIFEST, &at, options, PARTICLE_OPTION_KEYS);
                if let Some(tint) = options.get("tint") {
                    particle.tint = slot_tint(&mut cx, tint, &join(&at, "tint"));
                }
            }
            set.particles.push(particle);
            templates.push(template);
        } else {
            broken.push(id);
        }
    }

    for (id, options, at) in entries(&manifest, "sounds", &mut cx) {
        if let Some(sound) = sound(&mut cx, b, &id, &options, &at) {
            set.sounds.push(sound);
        }
    }

    let ambient_at = path(MANIFEST, "emitters");
    let start = set.emitters.len();
    match manifest.get("emitters") {
        None => {}
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                let at = join("emitters", &i.to_string());
                if let Some(def) = emitter(&mut cx, &set, (&templates, &broken), item, &at) {
                    set.emitters.push(def);
                }
            }
        }
        Some(_) => cx.error(IssueCode::EffectInvalid, ambient_at),
    }
    set.ambient = to_u16(start)..to_u16(set.emitters.len());

    match manifest.get("effects") {
        None => {}
        Some(Value::Object(named)) => {
            for (name, value) in named {
                let at = join("effects", name);
                if !is_id(name) {
                    cx.error(IssueCode::EffectInvalid, path(MANIFEST, &at));
                    continue;
                }
                let items: Vec<&Value> = match value {
                    Value::Array(items) => items.iter().collect(),
                    other => vec![other],
                };
                let start = set.emitters.len();
                for (i, item) in items.into_iter().enumerate() {
                    let at = join(&at, &i.to_string());
                    if let Some(def) = emitter(&mut cx, &set, (&templates, &broken), item, &at) {
                        set.emitters.push(def);
                    }
                }
                set.named
                    .push((name.clone(), to_u16(start)..to_u16(set.emitters.len())));
            }
        }
        Some(_) => cx.error(IssueCode::EffectInvalid, path(MANIFEST, "effects")),
    }

    match manifest.get("trails") {
        None => {}
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                let at = join("trails", &i.to_string());
                if let Some(def) = trail(&mut cx, b, &set, item, &at) {
                    set.trails.push(def);
                }
            }
        }
        Some(_) => cx.error(IssueCode::TrailInvalid, path(MANIFEST, "trails")),
    }

    let mut rings = RingPlan::default();
    for def in &mut set.emitters {
        if let Some(life) = world_life(&set.particles, def) {
            def.ring = Some(rings.add(def.locator, particle_points(life), true));
        }
    }
    for def in &mut set.trails {
        let need = trail_points(def.fade_seconds);
        def.ring = rings.add(def.locator, need, false);
        if !def.gates.is_empty() {
            rings.gate(def.ring);
        }
    }
    set.rings = rings.locators;
    set.ring_points = rings.points;
    set.ring_turns = rings.turns;
    set.ring_gates = rings.gates;
    set
}

fn world_life(particles: &[ParticleDef], def: &EmitterDef) -> Option<f32> {
    let particle = particles.get(usize::from(def.particle?))?;
    def.world.then_some(particle.longest_life)
}

fn to_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

#[derive(Default)]
struct RingPlan {
    locators: Vec<LocatorRef>,
    points: Vec<u16>,
    turns: Vec<bool>,
    gates: Vec<bool>,
}

impl RingPlan {
    fn of(e: &mut EffectSet) -> RingPlan {
        RingPlan {
            locators: mem::take(&mut e.rings),
            points: mem::take(&mut e.ring_points),
            turns: mem::take(&mut e.ring_turns),
            gates: mem::take(&mut e.ring_gates),
        }
    }

    fn into(self, e: &mut EffectSet) {
        e.rings = self.locators;
        e.ring_points = self.points;
        e.ring_turns = self.turns;
        e.ring_gates = self.gates;
    }

    fn gate(&mut self, ring: u16) {
        let index = usize::from(ring);
        if self.gates.len() <= index {
            self.gates.resize(index + 1, false);
        }
        self.gates[index] = true;
    }

    fn add(&mut self, locator: LocatorRef, need: usize, turned: bool) -> u16 {
        let index = match self.locators.iter().position(|r| *r == locator) {
            Some(i) => i,
            None => {
                self.locators.push(locator);
                self.locators.len() - 1
            }
        };
        let count = self.locators.len();
        if self.points.len() < count {
            self.points.resize(count, 0);
        }
        if self.turns.len() < count {
            self.turns.resize(count, false);
        }
        let need = u16::try_from(need).unwrap_or(u16::MAX);
        self.points[index] = self.points[index].max(need);
        self.turns[index] |= turned;
        to_u16(index)
    }
}

fn entries(
    manifest: &Map<String, Value>,
    key: &str,
    cx: &mut Cx<'_>,
) -> Vec<(String, Value, String)> {
    match manifest.get(key) {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let at = join(key, &i.to_string());
                match item.as_str() {
                    Some(id) => Some((id.to_string(), Value::Null, at)),
                    None => {
                        cx.error(IssueCode::EffectInvalid, path(MANIFEST, &at));
                        None
                    }
                }
            })
            .collect(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(id, options)| (id.clone(), options.clone(), join(key, id)))
            .collect(),
        Some(_) => {
            cx.error(IssueCode::EffectInvalid, path(MANIFEST, key));
            Vec::new()
        }
    }
}

fn locator(
    cx: &mut Cx<'_>,
    set: &EffectSet,
    value: Option<&Value>,
    default: PlayerLocator,
    at: &str,
) -> Option<LocatorRef> {
    let Some(value) = value else {
        return Some(LocatorRef::Player(default));
    };
    let Some(name) = value.as_str() else {
        cx.error(IssueCode::EffectInvalid, path(MANIFEST, at));
        return None;
    };
    let found = set.locator(name);
    if found.is_none() {
        cx.report.push(
            Issue::error(IssueCode::LocatorMissing, path(MANIFEST, at)).name("locator", name),
        );
    }
    found
}

fn emitter(
    cx: &mut Cx<'_>,
    set: &EffectSet,
    (templates, broken): (&[Template], &[String]),
    value: &Value,
    at: &str,
) -> Option<EmitterDef> {
    let Some(object) = value.as_object() else {
        cx.error(IssueCode::EffectInvalid, path(MANIFEST, at));
        return None;
    };
    cx.keys(MANIFEST, at, object, EMITTER_KEYS);
    let locator = locator(
        cx,
        set,
        object.get("locator"),
        PlayerLocator::Feet,
        &join(at, "locator"),
    )?;
    let named = ["particle", "sound", "vanilla"]
        .iter()
        .filter(|k| object.contains_key(**k))
        .count();
    if named != 1 {
        cx.error(IssueCode::EffectInvalid, path(MANIFEST, at));
        return None;
    }
    let mut def = EmitterDef {
        path: path(MANIFEST, at),
        particle: None,
        sound: None,
        vanilla: None,
        locator,
        offset: [0.0; 3],
        shape: Shape::Point,
        rate: Rate::Instant { count: 1 },
        lifetime: Lifetime::Once { seconds: 0.0 },
        world: false,
        ring: None,
    };
    if let Some(id) = object.get("particle") {
        let id = id.as_str().unwrap_or_default();
        if broken.iter().any(|b| b == id) {
            return None;
        }
        let Some(index) = set.particles.iter().position(|p| p.id == id) else {
            cx.report.push(
                Issue::error(
                    IssueCode::ParticleMissing,
                    path(MANIFEST, &join(at, "particle")),
                )
                .name("particle", id),
            );
            return None;
        };
        let template = templates[index].clone();
        def.particle = Some(to_u16(index));
        def.offset = template.offset;
        def.shape = template.shape;
        def.rate = template.rate;
        def.lifetime = template.lifetime;
        def.world = template.world;
        def.path = template.path;
    } else if let Some(id) = object.get("sound") {
        let id = id.as_str().unwrap_or_default();
        let Some(index) = set.sound(id) else {
            cx.report.push(
                Issue::error(IssueCode::SoundMissing, path(MANIFEST, &join(at, "sound")))
                    .name("sound", id),
            );
            return None;
        };
        def.sound = Some(index);
    } else if let Some(id) = object.get("vanilla") {
        let at_vanilla = path(MANIFEST, &join(at, "vanilla"));
        let id = namespaced(id.as_str().unwrap_or_default());
        if !VANILLA_PARTICLES.contains(&id.as_str()) {
            cx.report.push(
                Issue::error(IssueCode::VanillaParticleNotAllowed, at_vanilla)
                    .name("particle", &id),
            );
            return None;
        }
        let count = cx.constant(
            object.get("count"),
            1.0,
            &path(MANIFEST, &join(at, "count")),
        );
        if !(1.0..=VANILLA_COUNT_MAX).contains(&count) {
            cx.report.push(
                Issue::error(IssueCode::EffectInvalid, path(MANIFEST, &join(at, "count")))
                    .value("limit", VANILLA_COUNT_MAX as f64),
            );
            return None;
        }
        def.rate = Rate::Instant {
            count: count as u32,
        };
        def.vanilla = Some(id);
    }
    Some(def)
}

fn namespaced(id: &str) -> String {
    if id.contains(':') {
        id.to_string()
    } else {
        format!("{PREFIX}{id}")
    }
}

pub(super) fn last_spawn(def: &EmitterDef) -> f32 {
    match (def.rate, &def.lifetime) {
        (Rate::Manual { .. }, _) | (Rate::Instant { .. }, Lifetime::Once { .. }) => 0.0,
        (Rate::Steady { .. }, Lifetime::Once { seconds }) => *seconds,
        (_, Lifetime::Looping { .. } | Lifetime::Expression { .. }) => f32::INFINITY,
    }
}

fn sound(
    cx: &mut Cx<'_>,
    b: &mut Bundle<'_>,
    id: &str,
    options: &Value,
    at: &str,
) -> Option<SoundDef> {
    let from = path(MANIFEST, at);
    let empty = Map::new();
    let object = match options {
        Value::Null => &empty,
        Value::Object(object) => object,
        _ => {
            cx.error(IssueCode::EffectInvalid, from);
            return None;
        }
    };
    cx.keys(MANIFEST, at, object, SOUND_KEYS);
    if !is_id(id) {
        cx.error(IssueCode::EffectInvalid, from);
        return None;
    }
    let volume = cx.constant(
        object.get("volume"),
        1.0,
        &path(MANIFEST, &join(at, "volume")),
    );
    let pitch = cx.constant(
        object.get("pitch"),
        1.0,
        &path(MANIFEST, &join(at, "pitch")),
    );
    if !(volume > 0.0 && volume <= 1.0) || !(0.5..=2.0).contains(&pitch) {
        cx.error(IssueCode::EffectInvalid, from);
        return None;
    }
    let mut def = SoundDef {
        id: id.to_string(),
        file: None,
        vanilla: None,
        volume,
        pitch,
    };
    match object.get("vanilla") {
        Some(vanilla) => {
            let name = namespaced(vanilla.as_str().unwrap_or_default());
            if !VANILLA_SOUNDS.contains(&name.as_str()) {
                cx.report.push(
                    Issue::error(
                        IssueCode::SoundNotAllowed,
                        path(MANIFEST, &join(at, "vanilla")),
                    )
                    .name("sound", &name),
                );
                return None;
            }
            def.vanilla = Some(name);
        }
        None => {
            let file = b.id_path("sounds", id, "ogg", &from)?;
            let Some(bytes) = b.bytes(&file, &from) else {
                cx.report
                    .push(Issue::error(IssueCode::SoundMissing, from).name("file", &file));
                return None;
            };
            let codec = ogg_codec(&bytes);
            let vorbis = ogg_vorbis(&bytes).filter(|_| codec == "vorbis");
            let Some(vorbis) = vorbis else {
                cx.report.push(
                    Issue::error(IssueCode::SoundUnsupported, from)
                        .name("file", &file)
                        .name("codec", codec),
                );
                return None;
            };
            if vorbis.seconds() > SOUND_SECONDS {
                cx.report.push(
                    Issue::error(IssueCode::SoundTooLong, from)
                        .name("file", &file)
                        .value("seconds", f64::from(vorbis.seconds()))
                        .value("limit", f64::from(SOUND_SECONDS)),
                );
                return None;
            }
            if vorbis.decoded_bytes() > SOUND_DECODED_BYTES {
                cx.report.push(
                    Issue::error(IssueCode::SoundTooBig, from)
                        .name("file", &file)
                        .value("bytes", vorbis.decoded_bytes() as f64)
                        .value("limit", SOUND_DECODED_BYTES as f64),
                );
                return None;
            }
            if vorbis.rate > SOUND_RATE {
                cx.report.push(
                    Issue::error(IssueCode::SoundRateUnsupported, from)
                        .name("file", &file)
                        .value("rate", f64::from(vorbis.rate))
                        .value("limit", f64::from(SOUND_RATE)),
                );
                return None;
            }
            if u32::from(vorbis.channels) > SOUND_CHANNELS {
                cx.report.push(
                    Issue::error(IssueCode::SoundChannelsUnsupported, from)
                        .name("file", &file)
                        .value("channels", f64::from(vorbis.channels))
                        .value("limit", f64::from(SOUND_CHANNELS)),
                );
                return None;
            }
            def.file = Some(file);
        }
    }
    Some(def)
}

fn particle(
    cx: &mut Cx<'_>,
    b: &mut Bundle<'_>,
    file: &str,
    id: &str,
    json: &Value,
) -> Option<(ParticleDef, Template)> {
    let invalid = |cx: &mut Cx<'_>, at: &str| cx.error(IssueCode::EffectInvalid, path(file, at));
    let Some(root) = json.as_object() else {
        invalid(cx, "");
        return None;
    };
    cx.keys(file, "", root, ROOT_KEYS);
    let Some(effect) = root.get("particle_effect").and_then(Value::as_object) else {
        invalid(cx, "particle_effect");
        return None;
    };
    let base = "particle_effect";
    cx.keys(file, base, effect, EFFECT_KEYS);
    let errors = cx.report.issues().len();

    for key in ["curves", "events"] {
        match effect.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) if map.is_empty() => {}
            Some(_) => cx.unsupported(path(file, &join(base, key)), key),
        }
    }

    let description = effect.get("description").and_then(Value::as_object);
    let at_description = join(base, "description");
    if let Some(description) = description {
        cx.keys(file, &at_description, description, DESCRIPTION_KEYS);
    }
    let render = description
        .and_then(|d| d.get("basic_render_parameters"))
        .and_then(Value::as_object);
    let at_render = join(&at_description, "basic_render_parameters");
    if let Some(render) = render {
        cx.keys(file, &at_render, render, RENDER_KEYS);
    }
    let blend = match render
        .and_then(|r| r.get("material"))
        .and_then(Value::as_str)
    {
        None | Some("particles_alpha" | "particles_blend" | "particles_opaque") => Blend::Alpha,
        Some("particles_add") => Blend::Additive,
        Some(other) => {
            cx.report.push(
                Issue::error(
                    IssueCode::EffectInvalid,
                    path(file, &join(&at_render, "material")),
                )
                .name("material", other),
            );
            Blend::Alpha
        }
    };
    let texture = match render
        .and_then(|r| r.get("texture"))
        .and_then(Value::as_str)
    {
        Some(texture) if !texture.is_empty() => {
            let name = if texture.ends_with(".png") {
                texture.to_string()
            } else {
                format!("{texture}.png")
            };
            let from = path(file, &join(&at_render, "texture"));
            if b.bytes(&name, &from).is_none() {
                cx.report.push(
                    Issue::error(IssueCode::ParticleTextureMissing, from).name("file", &name),
                );
            }
            name
        }
        _ => {
            cx.error(
                IssueCode::ParticleTextureMissing,
                path(file, &join(&at_render, "texture")),
            );
            String::new()
        }
    };

    let at_components = join(base, "components");
    let empty = Map::new();
    let components = match effect.get("components") {
        Some(Value::Object(components)) => components,
        None => &empty,
        Some(_) => {
            invalid(cx, &at_components);
            &empty
        }
    };
    let mut parts = Parts::default();
    for (key, value) in components {
        let at = path(file, &join(&at_components, key));
        component(cx, &mut parts, key, value, &at);
    }

    let at_components_path = path(file, &at_components);
    let Some(rate) = parts.rate else {
        cx.report.push(
            Issue::error(IssueCode::EffectInvalid, at_components_path.clone())
                .name("component", "emitter_rate"),
        );
        return None;
    };
    let lifetime = parts.lifetime.unwrap_or(Lifetime::Looping {
        active: DEFAULT_ACTIVE_SECONDS,
        sleep: 0.0,
    });
    let Some(max_life) = parts.max_life else {
        cx.report.push(
            Issue::error(IssueCode::EffectInvalid, at_components_path.clone())
                .name("component", "particle_lifetime_expression"),
        );
        return None;
    };
    let longest_life = match longest(&max_life, rate, &lifetime) {
        Some(longest) if longest > 0.0 => longest,
        Some(_) => {
            cx.error(IssueCode::EffectInvalid, parts.max_life_at.clone());
            0.0
        }
        None => {
            cx.unsupported(parts.max_life_at.clone(), "unbounded_lifetime");
            0.0
        }
    };
    if parts.world && longest_life > EFFECT_PARTICLE_LIFE_SECONDS {
        cx.report.push(
            Issue::error(IssueCode::EffectTooLong, at_components_path.clone())
                .value("limit", EFFECT_PARTICLE_LIFE_SECONDS as f64),
        );
    }
    let motion = match (parts.parametric, parts.dynamic) {
        (Some(_), Some(_)) => {
            cx.report.push(
                Issue::error(IssueCode::EffectInvalid, at_components_path.clone())
                    .name("component", "particle_motion"),
            );
            return None;
        }
        (Some(offset), None) => Motion::Parametric { offset },
        (None, dynamic) => {
            let (acceleration, drag) = dynamic.unwrap_or_else(|| {
                (
                    [0, 1, 2].map(|_| Program::number(0.0)),
                    Program::number(0.0),
                )
            });
            let (speed, direction) = match parts.speed {
                Some(Speed::Scalar(speed)) => (speed, parts.direction),
                Some(Speed::Vector(v)) => (Program::number(1.0), Direction::Velocity(Box::new(v))),
                None => (Program::number(0.0), parts.direction),
            };
            Motion::Dynamic {
                speed,
                direction,
                acceleration: Box::new(acceleration),
                drag,
            }
        }
    };
    if cx.report.issues()[errors..]
        .iter()
        .any(|i| i.level == crate::issues::Level::Error)
    {
        return None;
    }
    let particle = ParticleDef {
        id: id.to_string(),
        texture,
        blend,
        facing: parts.facing,
        size: parts
            .size
            .unwrap_or_else(|| [0, 1].map(|_| Program::number(DEFAULT_SIZE))),
        uv: parts.uv.unwrap_or(Uv::Fixed {
            uv: [0.0, 0.0],
            size: [1.0, 1.0],
        }),
        tint: parts.tint.unwrap_or(Tint::Fixed(u32::MAX)),
        lifetime: max_life,
        longest_life,
        motion,
        emissive: !parts.lit,
    };
    let template = Template {
        shape: parts.shape,
        offset: parts.offset,
        rate,
        lifetime,
        world: parts.world,
        path: at_components_path,
    };
    Some((particle, template))
}

fn longest(max_life: &Program, rate: Rate, lifetime: &Lifetime) -> Option<f32> {
    let age = match (rate, lifetime) {
        (Rate::Instant { .. } | Rate::Manual { .. }, _) => Some((0.0, 0.0)),
        (Rate::Steady { .. }, Lifetime::Once { seconds }) => Some((0.0, f64::from(*seconds))),
        (Rate::Steady { .. }, Lifetime::Looping { active, .. }) => Some((0.0, f64::from(*active))),
        (Rate::Steady { .. }, Lifetime::Expression { .. }) => None,
    };
    let fixed = f64::from(emitter_lifetime(lifetime));
    let variable = |slot: u16| match slot as usize {
        s if (PARTICLE_RANDOM..PARTICLE_RANDOM + 4).contains(&s)
            || (EMITTER_RANDOM..EMITTER_RANDOM + 4).contains(&s) =>
        {
            Some((0.0, 1.0))
        }
        EMITTER_AGE => age,
        EMITTER_LIFETIME => Some((fixed, fixed)),
        _ => Some((0.0, 0.0)),
    };
    let (_, high) = max_life.bounds(&variable)?;
    let high = high as f32;
    high.is_finite().then_some(high.max(0.0))
}

enum Speed {
    Scalar(Program),
    Vector([Program; 3]),
}

struct Parts {
    rate: Option<Rate>,
    lifetime: Option<Lifetime>,
    shape: Shape,
    shapes: usize,
    offset: [f32; 3],
    direction: Direction,
    world: bool,
    speed: Option<Speed>,
    dynamic: Option<([Program; 3], Program)>,
    parametric: Option<[Program; 3]>,
    max_life: Option<Program>,
    max_life_at: String,
    size: Option<[Program; 2]>,
    facing: Facing,
    uv: Option<Uv>,
    tint: Option<Tint>,
    lit: bool,
}

impl Default for Parts {
    fn default() -> Self {
        Parts {
            rate: None,
            lifetime: None,
            shape: Shape::Point,
            shapes: 0,
            offset: [0.0; 3],
            direction: Direction::Outwards,
            world: true,
            speed: None,
            dynamic: None,
            parametric: None,
            max_life: None,
            max_life_at: String::new(),
            size: None,
            facing: Facing::RotateXyz,
            uv: None,
            tint: None,
            lit: false,
        }
    }
}

fn component(cx: &mut Cx<'_>, parts: &mut Parts, key: &str, value: &Value, at: &str) {
    let Some(name) = key.strip_prefix(PREFIX) else {
        cx.unsupported(at.to_string(), "component");
        return;
    };
    let empty = Map::new();
    let object = value.as_object().unwrap_or(&empty);
    let field = |k: &str| join(at, k);
    match name {
        "emitter_rate_instant" | "emitter_rate_steady" | "emitter_rate_manual" => {
            if parts.rate.is_some() {
                cx.error(IssueCode::EffectInvalid, at.to_string());
                return;
            }
            parts.rate = Some(match name {
                "emitter_rate_instant" => Rate::Instant {
                    count: count(
                        cx,
                        object.get("num_particles"),
                        10.0,
                        &field("num_particles"),
                    ),
                },
                "emitter_rate_steady" => Rate::Steady {
                    per_second: cx
                        .constant(object.get("spawn_rate"), 1.0, &field("spawn_rate"))
                        .max(0.0),
                    max: count(
                        cx,
                        object.get("max_particles"),
                        50.0,
                        &field("max_particles"),
                    ),
                },
                _ => Rate::Manual {
                    max: count(
                        cx,
                        object.get("max_particles"),
                        50.0,
                        &field("max_particles"),
                    ),
                },
            });
        }
        "emitter_lifetime_once" | "emitter_lifetime_looping" | "emitter_lifetime_expression" => {
            if parts.lifetime.is_some() {
                cx.error(IssueCode::EffectInvalid, at.to_string());
                return;
            }
            let active = |cx: &mut Cx<'_>| {
                cx.constant(
                    object.get("active_time"),
                    DEFAULT_ACTIVE_SECONDS,
                    &field("active_time"),
                )
                .max(0.0)
            };
            parts.lifetime = Some(match name {
                "emitter_lifetime_once" => Lifetime::Once {
                    seconds: active(cx),
                },
                "emitter_lifetime_looping" => {
                    let active = active(cx);
                    let sleep = cx
                        .constant(object.get("sleep_time"), 0.0, &field("sleep_time"))
                        .max(0.0);
                    if active + sleep < RING_STEP_SECONDS {
                        cx.error(IssueCode::EffectInvalid, field("active_time"));
                    }
                    Lifetime::Looping { active, sleep }
                }
                _ => Lifetime::Expression {
                    activation: cx.expr(
                        object.get("activation_expression"),
                        1.0,
                        &field("activation_expression"),
                    ),
                    expiration: cx.expr(
                        object.get("expiration_expression"),
                        0.0,
                        &field("expiration_expression"),
                    ),
                },
            });
        }
        "emitter_shape_point"
        | "emitter_shape_sphere"
        | "emitter_shape_box"
        | "emitter_shape_disc"
        | "emitter_shape_entity_aabb" => {
            parts.shapes += 1;
            if parts.shapes > 1 {
                cx.error(IssueCode::EffectInvalid, at.to_string());
                return;
            }
            shape(cx, parts, name, object, at);
        }
        "emitter_local_space" => {
            parts.world = !object
                .get("position")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        }
        "particle_initial_speed" => {
            parts.speed = Some(match value {
                Value::Array(_) => Speed::Vector(cx.vec3(Some(value), 0.0, at)),
                _ => Speed::Scalar(cx.timeless(Some(value), 0.0, at)),
            });
        }
        "particle_initial_spin" => {
            for k in ["rotation", "rotation_rate"] {
                if cx.constant(object.get(k), 0.0, &field(k)) != 0.0 {
                    cx.unsupported(field(k), "spin");
                }
            }
        }
        "particle_motion_dynamic" => {
            let acceleration = match object.get("linear_acceleration") {
                Some(Value::Array(items)) if items.len() == 3 => [0, 1, 2].map(|i| {
                    cx.timeless(
                        items.get(i),
                        0.0,
                        &join(&field("linear_acceleration"), &i.to_string()),
                    )
                }),
                None => [0, 1, 2].map(|_| Program::number(0.0)),
                Some(_) => {
                    cx.error(IssueCode::EffectInvalid, field("linear_acceleration"));
                    [0, 1, 2].map(|_| Program::number(0.0))
                }
            };
            let drag = cx.timeless(
                object.get("linear_drag_coefficient"),
                0.0,
                &field("linear_drag_coefficient"),
            );
            for k in ["rotation_acceleration", "rotation_drag_coefficient"] {
                if cx.constant(object.get(k), 0.0, &field(k)) != 0.0 {
                    cx.unsupported(field(k), "spin");
                }
            }
            parts.dynamic = Some((acceleration, drag));
        }
        "particle_motion_parametric" => {
            parts.parametric = Some(cx.vec3(
                object.get("relative_position"),
                0.0,
                &field("relative_position"),
            ));
            if cx.constant(object.get("rotation"), 0.0, &field("rotation")) != 0.0 {
                cx.unsupported(field("rotation"), "spin");
            }
        }
        "particle_lifetime_expression" => {
            parts.max_life =
                Some(cx.timeless(object.get("max_lifetime"), 1.0, &field("max_lifetime")));
            parts.max_life_at = field("max_lifetime");
            let expiration = cx.expr(
                object.get("expiration_expression"),
                0.0,
                &field("expiration_expression"),
            );
            if !expiration.reads().variables.is_empty()
                || Vars::default().eval(&expiration, 0) != 0.0
            {
                cx.unsupported(field("expiration_expression"), "expiration");
            }
        }
        "particle_appearance_billboard" => billboard(cx, parts, object, at),
        "particle_appearance_tinting" => {
            parts.tint = Some(color_tint(cx, object.get("color"), &field("color")));
        }
        "particle_appearance_lighting" => parts.lit = true,
        "emitter_initialization" => cx.unsupported(at.to_string(), "emitter_script"),
        "particle_initialization" => cx.unsupported(at.to_string(), "particle_script"),
        "emitter_lifetime_events" | "particle_lifetime_events" => {
            cx.unsupported(at.to_string(), "events")
        }
        "particle_motion_collision"
        | "particle_expire_if_in_blocks"
        | "particle_expire_if_not_in_blocks"
        | "particle_kill_plane" => cx.unsupported(at.to_string(), "collision"),
        "emitter_shape_custom" => cx.unsupported(at.to_string(), "custom_shape"),
        _ => cx.unsupported(at.to_string(), "component"),
    }
}

fn count(cx: &mut Cx<'_>, value: Option<&Value>, default: f32, at: &str) -> u32 {
    let n = cx.constant(value, default, at);
    if n.is_finite() && n > 0.0 {
        (n.round() as u32).min(u16::MAX as u32)
    } else {
        0
    }
}

fn shape(cx: &mut Cx<'_>, parts: &mut Parts, name: &str, object: &Map<String, Value>, at: &str) {
    let field = |k: &str| join(at, k);
    parts.offset = cx.const3(object.get("offset"), 0.0, &field("offset"));
    let surface = object
        .get("surface_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    parts.direction = match object.get("direction") {
        None | Some(Value::Null) => Direction::Outwards,
        Some(Value::String(s)) if s == "outwards" => Direction::Outwards,
        Some(Value::String(s)) if s == "inwards" => Direction::Inwards,
        Some(Value::Array(items)) if items.len() == 3 => Direction::Vector(Box::new(cx.vec3(
            object.get("direction"),
            0.0,
            &field("direction"),
        ))),
        Some(_) => {
            cx.error(IssueCode::EffectInvalid, field("direction"));
            Direction::Outwards
        }
    };
    parts.shape = match name {
        "emitter_shape_point" => Shape::Point,
        "emitter_shape_sphere" => Shape::Sphere {
            radius: cx
                .constant(object.get("radius"), 1.0, &field("radius"))
                .max(0.0),
            surface,
        },
        "emitter_shape_box" => Shape::Box {
            half: cx
                .const3(
                    object.get("half_dimensions"),
                    0.0,
                    &field("half_dimensions"),
                )
                .map(f32::abs),
            surface,
        },
        "emitter_shape_disc" => Shape::Disc {
            radius: cx
                .constant(object.get("radius"), 1.0, &field("radius"))
                .max(0.0),
            normal: match object.get("plane_normal") {
                None | Some(Value::Null) => [0.0, 1.0, 0.0],
                Some(Value::String(s)) if s == "x" => [1.0, 0.0, 0.0],
                Some(Value::String(s)) if s == "y" => [0.0, 1.0, 0.0],
                Some(Value::String(s)) if s == "z" => [0.0, 0.0, 1.0],
                Some(value @ Value::Array(_)) => {
                    let normal = cx.const3(Some(value), 0.0, &field("plane_normal"));
                    if normal == [0.0; 3] {
                        cx.error(IssueCode::EffectInvalid, field("plane_normal"));
                    }
                    normal
                }
                Some(_) => {
                    cx.error(IssueCode::EffectInvalid, field("plane_normal"));
                    [0.0, 1.0, 0.0]
                }
            },
            surface,
        },
        _ => Shape::EntityAabb { surface },
    };
}

fn billboard(cx: &mut Cx<'_>, parts: &mut Parts, object: &Map<String, Value>, at: &str) {
    let field = |k: &str| join(at, k);
    parts.size = Some(match object.get("size") {
        Some(Value::Array(items)) if items.len() == 2 => [0, 1].map(|i| {
            cx.expr(
                items.get(i),
                DEFAULT_SIZE,
                &join(&field("size"), &i.to_string()),
            )
        }),
        None => [0, 1].map(|_| Program::number(DEFAULT_SIZE)),
        Some(_) => {
            cx.error(IssueCode::EffectInvalid, field("size"));
            [0, 1].map(|_| Program::number(DEFAULT_SIZE))
        }
    });
    parts.facing = match object.get("facing_camera_mode").and_then(Value::as_str) {
        None | Some("rotate_xyz") => Facing::RotateXyz,
        Some("rotate_y") => Facing::RotateY,
        Some("lookat_xyz") => Facing::LookatXyz,
        Some("lookat_y") => Facing::LookatY,
        Some("lookat_direction") => Facing::Direction,
        Some("emitter_transform_xy") => Facing::EmitterXy,
        Some("emitter_transform_xz") => Facing::EmitterXz,
        Some("emitter_transform_yz") => Facing::EmitterYz,
        Some(_) => {
            cx.unsupported(field("facing_camera_mode"), "facing");
            Facing::RotateXyz
        }
    };
    let Some(uv) = object.get("uv").and_then(Value::as_object) else {
        return;
    };
    let at_uv = field("uv");
    let uv_field = |k: &str| join(&at_uv, k);
    let width = cx.constant(uv.get("texture_width"), 1.0, &uv_field("texture_width"));
    let height = cx.constant(uv.get("texture_height"), 1.0, &uv_field("texture_height"));
    if !(width > 0.0 && height > 0.0) {
        cx.error(IssueCode::EffectInvalid, at_uv);
        return;
    }
    let norm = |p: [f32; 2]| [p[0] / width, p[1] / height];
    parts.uv = Some(match uv.get("flipbook").and_then(Value::as_object) {
        Some(flip) => {
            let at_flip = uv_field("flipbook");
            let flip_field = |k: &str| join(&at_flip, k);
            Uv::Flipbook {
                base: norm(cx.const2(flip.get("base_UV"), 0.0, &flip_field("base_UV"))),
                size: norm(cx.const2(flip.get("size_UV"), 1.0, &flip_field("size_UV"))),
                step: norm(cx.const2(flip.get("step_UV"), 0.0, &flip_field("step_UV"))),
                frames: count(cx, flip.get("max_frame"), 1.0, &flip_field("max_frame")).max(1),
                fps: cx
                    .constant(
                        flip.get("frames_per_second"),
                        8.0,
                        &flip_field("frames_per_second"),
                    )
                    .max(0.0),
                stretch: flip
                    .get("stretch_to_lifetime")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                looping: flip.get("loop").and_then(Value::as_bool).unwrap_or(false),
            }
        }
        None => Uv::Fixed {
            uv: norm(cx.const2(uv.get("uv"), 0.0, &uv_field("uv"))),
            size: norm(cx.const2(uv.get("uv_size"), width.max(height), &uv_field("uv_size"))),
        },
    });
}

fn hex(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(0xff00_0000 | value),
        8 => Some(value),
        _ => None,
    }
}

fn constant_color(cx: &mut Cx<'_>, value: &Value, at: &str) -> Option<u32> {
    match value {
        Value::String(text) => {
            let color = hex(text);
            if color.is_none() {
                cx.error(IssueCode::EffectInvalid, at.to_string());
            }
            color
        }
        Value::Array(items) if (3..=4).contains(&items.len()) => {
            let channel = |cx: &mut Cx<'_>, i: usize| {
                cx.constant(items.get(i), 1.0, &join(at, &i.to_string()))
            };
            let rgba = [0, 1, 2, 3].map(|i| channel(cx, i));
            Some(particles::pack(rgba))
        }
        _ => {
            cx.error(IssueCode::EffectInvalid, at.to_string());
            None
        }
    }
}

fn color_tint(cx: &mut Cx<'_>, value: Option<&Value>, at: &str) -> Tint {
    match value {
        None | Some(Value::Null) => Tint::Fixed(u32::MAX),
        Some(text @ Value::String(_)) => {
            Tint::Fixed(constant_color(cx, text, at).unwrap_or(u32::MAX))
        }
        Some(Value::Array(items)) if (3..=4).contains(&items.len()) => {
            let programs: [Program; 4] =
                [0, 1, 2, 3].map(|i| cx.expr(items.get(i), 1.0, &join(at, &i.to_string())));
            if programs.iter().all(|p| p.reads().variables.is_empty()) {
                Tint::Fixed(particles::pack(
                    programs.each_ref().map(|p| Vars::default().eval(p, 0)),
                ))
            } else {
                Tint::Expression(Box::new(programs))
            }
        }
        Some(Value::Object(object)) => {
            cx.keys_at(at, object, GRADIENT_KEYS);
            let interpolant = cx.expr(object.get("interpolant"), 0.0, &join(at, "interpolant"));
            let keys = gradient_keys(cx, object.get("gradient"), &join(at, "gradient"));
            Tint::Gradient { interpolant, keys }
        }
        Some(_) => {
            cx.error(IssueCode::EffectInvalid, at.to_string());
            Tint::Fixed(u32::MAX)
        }
    }
}

fn gradient_keys(cx: &mut Cx<'_>, value: Option<&Value>, at: &str) -> Vec<(f32, u32)> {
    let mut keys = Vec::new();
    match value {
        Some(Value::Object(map)) => {
            for (key, color) in map {
                let Some(position) = key
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|p| (0.0..=1.0).contains(p))
                else {
                    cx.error(IssueCode::EffectInvalid, join(at, key));
                    continue;
                };
                if let Some(color) = constant_color(cx, color, &join(at, key)) {
                    keys.push((position, color));
                }
            }
        }
        Some(Value::Array(items)) => {
            let last = items.len().saturating_sub(1).max(1) as f32;
            for (i, color) in items.iter().enumerate() {
                if let Some(color) = constant_color(cx, color, &join(at, &i.to_string())) {
                    keys.push((i as f32 / last, color));
                }
            }
        }
        _ => cx.error(IssueCode::EffectInvalid, at.to_string()),
    }
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    if keys.is_empty() {
        keys.push((0.0, u32::MAX));
    }
    keys
}

fn slot_tint(cx: &mut Cx<'_>, value: &Value, at: &str) -> Tint {
    let Some(object) = value.as_object() else {
        cx.error(IssueCode::EffectInvalid, path(MANIFEST, at));
        return Tint::Fixed(u32::MAX);
    };
    cx.keys(MANIFEST, at, object, SLOT_KEYS);
    let index = |v: Option<&Value>| v.and_then(Value::as_u64).and_then(|n| u8::try_from(n).ok());
    match (index(object.get("slot")), object.get("tone")) {
        (Some(slot), None) => Tint::Slot { slot, tone: None },
        (Some(slot), Some(tone)) if index(Some(tone)).is_some() => Tint::Slot {
            slot,
            tone: index(Some(tone)),
        },
        _ => {
            cx.error(IssueCode::EffectInvalid, path(MANIFEST, at));
            Tint::Fixed(u32::MAX)
        }
    }
}

fn trail(
    cx: &mut Cx<'_>,
    b: &mut Bundle<'_>,
    set: &EffectSet,
    value: &Value,
    at: &str,
) -> Option<TrailDef> {
    let from = path(MANIFEST, at);
    let Some(object) = value.as_object() else {
        cx.error(IssueCode::TrailInvalid, from);
        return None;
    };
    cx.keys(MANIFEST, at, object, TRAIL_KEYS);
    let field = |k: &str| join(at, k);
    let invalid = |cx: &mut Cx<'_>, k: &str| {
        cx.report.push(
            Issue::error(IssueCode::TrailInvalid, path(MANIFEST, &join(at, k))).name("key", k),
        );
    };
    let locator = locator(
        cx,
        set,
        object.get("locator"),
        PlayerLocator::Back,
        &field("locator"),
    )?;
    let Some(texture) = object.get("texture").and_then(Value::as_str) else {
        invalid(cx, "texture");
        return None;
    };
    let file = b.id_path(
        "textures",
        texture,
        "png",
        &path(MANIFEST, &field("texture")),
    )?;
    if b.bytes(&file, &from).is_none() {
        cx.report.push(
            Issue::error(
                IssueCode::ParticleTextureMissing,
                path(MANIFEST, &field("texture")),
            )
            .name("file", &file),
        );
        return None;
    }
    let blend = match object.get("blend").and_then(Value::as_str) {
        None | Some("alpha") => Blend::Alpha,
        Some("additive") => Blend::Additive,
        Some(_) => {
            invalid(cx, "blend");
            return None;
        }
    };
    let width = match object.get("width") {
        None => [0.25, 0.25],
        Some(Value::Number(n)) => [n.as_f64().unwrap_or(0.0) as f32; 2],
        Some(Value::Array(items)) if items.len() == 2 => {
            let w = |i: usize| items[i].as_f64().unwrap_or(-1.0) as f32;
            [w(0), w(1)]
        }
        Some(_) => [-1.0; 2],
    };
    if width.iter().any(|w| !(*w >= 0.0 && w.is_finite())) {
        invalid(cx, "width");
        return None;
    }
    let tint = match object.get("tint") {
        None => Tint::Fixed(u32::MAX),
        Some(Value::Object(o)) if o.contains_key("slot") => {
            slot_tint(cx, &Value::Object(o.clone()), &field("tint"))
        }
        Some(Value::Object(o)) => {
            cx.keys_at(&path(MANIFEST, &field("tint")), o, &["gradient"]);
            Tint::Gradient {
                interpolant: Program::number(0.0),
                keys: gradient_keys(
                    cx,
                    o.get("gradient"),
                    &path(MANIFEST, &join(&field("tint"), "gradient")),
                ),
            }
        }
        Some(value) => Tint::Fixed(constant_color(cx, value, &path(MANIFEST, &field("tint")))?),
    };
    let number = |k: &str, default: f32| match object.get(k) {
        None => Some(default),
        Some(v) => v
            .as_f64()
            .map(|n| n as f32)
            .filter(|n| *n >= 0.0 && n.is_finite()),
    };
    let Some(fade_seconds) =
        number("fade", DEFAULT_FADE_SECONDS).filter(|f| *f <= EFFECT_PARTICLE_LIFE_SECONDS)
    else {
        invalid(cx, "fade");
        return None;
    };
    let Some(min_speed) = number("min_speed", 0.0) else {
        invalid(cx, "min_speed");
        return None;
    };
    let mut gates = Vec::new();
    match object.get("while") {
        None => {}
        Some(Value::Array(items)) => {
            for item in items {
                let name = item.as_str().unwrap_or_default();
                match signal_id(name)
                    .or_else(|| (name == JUMP_QUERY).then_some(JUMP_SIGNAL))
                    .and_then(|id| u8::try_from(id).ok())
                {
                    Some(id) => gates.push(id),
                    None => {
                        cx.report.push(
                            Issue::error(IssueCode::TrailInvalid, path(MANIFEST, &field("while")))
                                .name("signal", name),
                        );
                        return None;
                    }
                }
            }
        }
        Some(_) => {
            invalid(cx, "while");
            return None;
        }
    }
    Some(TrailDef {
        locator,
        ring: 0,
        texture: file,
        blend,
        width,
        tile: object.get("tile").and_then(Value::as_bool).unwrap_or(false),
        tint,
        fade_seconds,
        min_speed,
        gates,
    })
}

fn assigns_variable(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut quoted = false;
    for (i, &c) in bytes.iter().enumerate() {
        if c == b'\'' {
            quoted = !quoted;
        }
        if quoted || c != b'=' {
            continue;
        }
        let before = i.checked_sub(1).map(|j| bytes[j]);
        let after = bytes.get(i + 1).copied();
        if matches!(before, Some(b'=' | b'!' | b'<' | b'>')) || after == Some(b'=') {
            continue;
        }
        let target = source[..i].trim_end();
        let start = target
            .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'))
            .map_or(0, |p| p + 1);
        let name = target[start..].to_ascii_lowercase();
        if name.starts_with("v.") || name.starts_with("variable.") {
            return true;
        }
    }
    false
}

pub(super) fn check_events(e: &EffectSet, anims: &[Animation], c: &ControllerSet, r: &mut Report) {
    if r.mode() != Mode::Authoring {
        return;
    }
    for animation in anims {
        let base = join("animations", &animation.name);
        for (i, event) in animation.events.iter().enumerate() {
            let at = join(&base, &i.to_string());
            match &event.kind {
                KeyEventKind::Particle {
                    effect, locator, ..
                } => {
                    if e.named(effect).is_none() {
                        unknown(r, &at, effect);
                    }
                    if let Some(locator) = locator {
                        if e.locator(locator).is_none() {
                            r.push(
                                Issue::error(IssueCode::LocatorMissing, at.clone())
                                    .name("locator", locator),
                            );
                        }
                    }
                }
                KeyEventKind::Sound { effect } => {
                    if e.sound(effect).is_none() && e.named(effect).is_none() {
                        unknown(r, &at, effect);
                    }
                }
                KeyEventKind::Timeline(_) => {}
            }
        }
    }
    for controller in c.controllers() {
        let base = join(&controller.file, &controller.name);
        for state in &controller.states {
            let at = join(&base, &state.name);
            for effect in &state.effects {
                let found = match effect.kind {
                    EffectKind::Particle => e.named(&effect.name).is_some(),
                    EffectKind::Sound => {
                        e.sound(&effect.name).is_some() || e.named(&effect.name).is_some()
                    }
                };
                if !found {
                    unknown(r, &at, &effect.name);
                }
                if let Some(locator) = &effect.locator {
                    if e.locator(locator).is_none() {
                        r.push(
                            Issue::error(IssueCode::LocatorMissing, at.clone())
                                .name("locator", locator),
                        );
                    }
                }
            }
        }
    }
}

pub(super) fn bind_locators(e: &mut EffectSet, anims: &[Animation], c: &ControllerSet) {
    let keyframes = anims
        .iter()
        .flat_map(|a| &a.events)
        .filter_map(|event| match &event.kind {
            KeyEventKind::Particle {
                effect,
                locator: Some(locator),
                ..
            } => Some((effect.as_str(), locator.as_str())),
            _ => None,
        });
    let states = c
        .controllers()
        .iter()
        .flat_map(|controller| &controller.states)
        .flat_map(|state| &state.effects)
        .filter_map(|effect| Some((effect.name.as_str(), effect.locator.as_deref()?)));
    let mut rings = RingPlan::of(e);
    for (effect, locator) in keyframes.chain(states) {
        let (Some(range), Some(locator)) = (e.named(effect), e.locator(locator)) else {
            continue;
        };
        let life = e.emitters[usize::from(range.start)..usize::from(range.end)]
            .iter()
            .filter_map(|def| world_life(&e.particles, def))
            .reduce(f32::max);
        if let Some(life) = life {
            rings.add(locator, particle_points(life), true);
        }
    }
    rings.into(e);
}

fn unknown(r: &mut Report, at: &str, effect: &str) {
    r.push(Issue::error(IssueCode::EffectEventUnknown, at).name("effect", effect));
}

#[cfg(test)]
pub(super) mod tests {
    use serde_json::json;

    use super::*;
    use crate::container::{open_files, Opened, Options};
    use crate::effects::moments::tests::ogg;
    use crate::effects::plan_moments;
    use crate::ir::{Bone, Locator};

    pub(in crate::effects) fn png() -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&16u32.to_be_bytes());
        bytes.extend_from_slice(&16u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    pub(in crate::effects) fn spark() -> Value {
        json!({
            "format_version": "1.10.0",
            "particle_effect": {
                "description": {
                    "identifier": "sample:spark",
                    "basic_render_parameters": {
                        "material": "particles_add",
                        "texture": "textures/particle/spark"
                    }
                },
                "components": {
                    "minecraft:emitter_rate_steady": { "spawn_rate": 10, "max_particles": 20 },
                    "minecraft:emitter_lifetime_looping": { "active_time": 1 },
                    "minecraft:emitter_shape_sphere": { "radius": 0.5, "direction": "outwards" },
                    "minecraft:particle_lifetime_expression": {
                        "max_lifetime": "1 + variable.particle_random_1"
                    },
                    "minecraft:particle_initial_speed": 2,
                    "minecraft:particle_motion_dynamic": {
                        "linear_acceleration": [0, -9.8, 0],
                        "linear_drag_coefficient": 0.5
                    },
                    "minecraft:particle_appearance_billboard": {
                        "size": ["0.1 * (1 - v.particle_age / v.particle_lifetime)", 0.1],
                        "facing_camera_mode": "lookat_xyz",
                        "uv": {
                            "texture_width": 16,
                            "texture_height": 16,
                            "flipbook": {
                                "base_UV": [0, 0],
                                "size_UV": [8, 8],
                                "step_UV": [8, 0],
                                "frames_per_second": 4,
                                "max_frame": 2,
                                "loop": true
                            }
                        }
                    },
                    "minecraft:particle_appearance_tinting": {
                        "color": {
                            "interpolant": "v.particle_age / v.particle_lifetime",
                            "gradient": { "0.0": "#ffff0000", "1.0": "#00ff0000" }
                        }
                    }
                }
            }
        })
    }

    pub(in crate::effects) fn puff() -> Value {
        json!({
            "format_version": "1.10.0",
            "particle_effect": {
                "description": {
                    "identifier": "sample:puff",
                    "basic_render_parameters": {
                        "material": "particles_alpha",
                        "texture": "textures/particle/puff"
                    }
                },
                "components": {
                    "minecraft:emitter_local_space": { "position": true, "rotation": true },
                    "minecraft:emitter_rate_instant": { "num_particles": 8 },
                    "minecraft:emitter_lifetime_once": { "active_time": 0.5 },
                    "minecraft:emitter_shape_disc": { "radius": 0.4, "plane_normal": "y" },
                    "minecraft:particle_lifetime_expression": { "max_lifetime": 1 },
                    "minecraft:particle_motion_parametric": {
                        "relative_position": [0, "v.particle_age", 0]
                    },
                    "minecraft:particle_appearance_billboard": { "size": [0.2, 0.2] },
                    "minecraft:particle_appearance_lighting": {}
                }
            }
        })
    }

    pub(in crate::effects) fn manifest() -> Value {
        json!({
            "format": 1,
            "kind": "effect",
            "id": "sample",
            "particles": { "spark": {}, "puff": { "tint": { "slot": 1, "tone": 2 } } },
            "sounds": {
                "chime": { "vanilla": "minecraft:block.note_block.chime", "pitch": 1.5 },
                "pop": { "volume": 0.5 }
            },
            "emitters": [{ "particle": "spark", "locator": "above_head" }],
            "effects": {
                "join": [{ "particle": "puff", "locator": "feet" }, { "sound": "chime" }],
                "sparkle": { "vanilla": "flame", "locator": "tip", "count": 2 }
            },
            "trails": [{
                "locator": "back",
                "texture": "ribbon",
                "width": [0.3, 0],
                "tint": { "gradient": ["#ffffffff", "#00ffffff"] },
                "while": ["is_gliding"]
            }]
        })
    }

    pub(in crate::effects) fn files(
        manifest: &Value,
        particles: &[(&str, Value)],
    ) -> Vec<(String, Vec<u8>)> {
        let mut files = vec![(
            "manifest.json".to_string(),
            serde_json::to_vec(manifest).unwrap(),
        )];
        for (id, json) in particles {
            files.push((
                format!("particles/{id}.json"),
                serde_json::to_vec(json).unwrap(),
            ));
        }
        for texture in [
            "textures/particle/spark.png",
            "textures/particle/puff.png",
            "textures/ribbon.png",
        ] {
            files.push((texture.to_string(), png()));
        }
        files.push(("sounds/pop.ogg".to_string(), ogg(44_100, 4_410)));
        files
    }

    pub(in crate::effects) fn model() -> Model {
        Model {
            identifier: "geometry.sample".to_string(),
            texture_size: [16.0, 16.0],
            bones: vec![Bone {
                name: "tail".to_string(),
                parent: None,
                parent_name: None,
                pivot: [0.0; 3],
                rotation: [0.0; 3],
                cubes: Vec::new(),
                glow: None,
                locators: vec![Locator {
                    name: "tip".to_string(),
                    offset: [0.0, 4.0, -6.0],
                    rotation: [0.0; 3],
                }],
                spring: None,
            }],
        }
    }

    pub(in crate::effects) fn load(
        files: &[(String, Vec<u8>)],
        mode: Mode,
    ) -> (EffectSet, Vec<Issue>) {
        load_planned(files, mode, &[], &[])
    }

    pub(in crate::effects) fn load_planned(
        files: &[(String, Vec<u8>)],
        mode: Mode,
        anims: &[Animation],
        sounds: &[Vec<u8>],
    ) -> (EffectSet, Vec<Issue>) {
        load_as(files, mode, anims, sounds, true)
    }

    pub(in crate::effects) fn load_as(
        files: &[(String, Vec<u8>)],
        mode: Mode,
        anims: &[Animation],
        sounds: &[Vec<u8>],
        moment_set: bool,
    ) -> (EffectSet, Vec<Issue>) {
        let Opened::Bundle(mut bundle) = open_files(files, &Options::new(mode)) else {
            panic!("the bundle opens");
        };
        let mut report = Report::new(mode);
        let model = model();
        let mut set = parse(&mut bundle, Some(&model), &mut report);
        plan_moments(&mut set, anims, sounds, moment_set, &mut report);
        let mut issues = bundle.finish();
        issues.extend(report.into_issues());
        (set, issues)
    }

    pub(in crate::effects) fn sample() -> EffectSet {
        let (set, issues) = load(
            &files(&manifest(), &[("spark", spark()), ("puff", puff())]),
            Mode::Authoring,
        );
        assert_eq!(issues, Vec::new());
        set
    }

    #[test]
    fn effects_parse_a_whole_bundle() {
        let set = sample();
        assert_eq!(set.particles.len(), 2);
        let spark = &set.particles[0];
        assert_eq!(spark.texture, "textures/particle/spark.png");
        assert_eq!(spark.blend, Blend::Additive);
        assert_eq!(spark.facing, Facing::LookatXyz);
        assert!(spark.emissive);
        assert!((spark.longest_life - 2.0).abs() < 1e-5);
        assert!(matches!(
            spark.uv,
            Uv::Flipbook {
                frames: 2,
                looping: true,
                ..
            }
        ));
        assert!(matches!(&spark.tint, Tint::Gradient { keys, .. } if keys.len() == 2));
        let puff = &set.particles[1];
        assert!(!puff.emissive);
        assert!(matches!(
            puff.tint,
            Tint::Slot {
                slot: 1,
                tone: Some(2)
            }
        ));
        assert!(matches!(puff.motion, Motion::Parametric { .. }));

        assert_eq!(set.sounds.len(), 2);
        assert_eq!(
            set.sounds[0].vanilla.as_deref(),
            Some("minecraft:block.note_block.chime")
        );
        assert_eq!(set.sounds[0].pitch, 1.5);
        assert_eq!(set.sounds[1].file.as_deref(), Some("sounds/pop.ogg"));

        assert_eq!(set.ambient, 0..1);
        assert_eq!(set.named("join"), Some(1..3));
        assert_eq!(set.named("sparkle"), Some(3..4));
        assert_eq!(set.named_id("sparkle"), Some(1));
        let ambient = &set.emitters[0];
        assert_eq!(
            ambient.locator,
            LocatorRef::Player(PlayerLocator::AboveHead)
        );
        assert!(matches!(ambient.rate, Rate::Steady { max: 20, .. }));
        assert!(ambient.world);
        assert_eq!(ambient.ring, Some(0));
        let join = &set.emitters[1];
        assert!(!join.world);
        assert_eq!(join.ring, None);
        assert!(matches!(join.rate, Rate::Instant { count: 8 }));
        assert_eq!(set.emitters[2].sound, Some(0));
        let sparkle = &set.emitters[3];
        assert_eq!(sparkle.vanilla.as_deref(), Some("minecraft:flame"));
        assert_eq!(sparkle.locator, LocatorRef::Model(0));
        assert!(matches!(sparkle.rate, Rate::Instant { count: 2 }));

        assert_eq!(set.trails.len(), 1);
        let trail = &set.trails[0];
        assert_eq!(trail.ring, 1);
        assert_eq!(trail.width, [0.3, 0.0]);
        assert_eq!(trail.gates, vec![signal_id("is_gliding").unwrap() as u8]);
        assert_eq!(
            set.rings,
            vec![
                LocatorRef::Player(PlayerLocator::AboveHead),
                LocatorRef::Player(PlayerLocator::Back)
            ]
        );
        assert_eq!(set.locators, vec!["tip".to_string()]);
    }

    #[test]
    fn effects_sounds_accept_only_measurable_ogg_vorbis() {
        let manifest = json!({
            "format": 1, "kind": "effect", "id": "sample",
            "sounds": ["pop"],
            "effects": { "join": { "sound": "pop" } }
        });
        let mut flac = b"OggS\0\x02".to_vec();
        flac.extend_from_slice(&[0; 20]);
        flac.extend_from_slice(&[1, 9]);
        flac.extend_from_slice(b"\x7fFLAC\x01\0\0\x01");
        let mut cut = ogg(44_100, 44_100);
        cut.truncate(60);
        for (bytes, codec) in [
            (flac, "flac"),
            (b"OggS".to_vec(), "unknown"),
            (cut, "unknown"),
            (b"RIFF\0\0\0\0WAVE".to_vec(), "unknown"),
        ] {
            let mut bundle = files(&manifest, &[]);
            bundle
                .iter_mut()
                .find(|(name, _)| name == "sounds/pop.ogg")
                .unwrap()
                .1 = bytes;
            let (set, issues) = load(&bundle, Mode::Authoring);
            let unsupported: Vec<&Issue> = issues
                .iter()
                .filter(|i| i.code == IssueCode::SoundUnsupported)
                .collect();
            assert_eq!(unsupported.len(), 1, "{codec}: {issues:?}");
            assert_eq!(
                unsupported[0].names,
                [
                    ("file".to_string(), "sounds/pop.ogg".to_string()),
                    ("codec".to_string(), codec.to_string())
                ]
            );
            assert!(set.sounds.is_empty(), "{codec}");
        }
    }
}
