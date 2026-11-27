use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::animation::{self, channel_offsets, has_timeline, Animation, KEPT_DELAY_FLOATS};
use crate::attach::{self, Attach, RigKind};
use crate::bedrock;
use crate::bundle;
use crate::container::{
    decode_png, guarded_json, is_id, open_files, open_source, Bundle, Legacy, Opened,
    Options as ContainerOptions, CAPABILITIES, FORMAT, MANIFEST,
};
use crate::controller::{self, Controller, ControllerSet};
use crate::cost::{self, Content, Geometry, Record, Springs, Texture};
use crate::effects::geometry::{texture_count, texture_path};
use crate::effects::{self, EffectSet, Solo, MOMENTS};
use crate::ir::{Bone, Model};
use crate::issues::{join, path, Issue, IssueCode, Level, Mode, Report};
use crate::limits::{ID_MAX_LEN, ITEM_CAPS};
use crate::molang::{Interner, Rules};
use crate::nameplate::format::palette as read_palette;
use crate::palette::{Palette, MAX_TONES};
use crate::pet::{self, PetSpec, LEGACY_PET};
use crate::player::{self, Arms};
use crate::pose;
use crate::reactions::{self, Reaction};
use crate::roles::{self, Role};
use crate::scene::{prop_model, EMOTE_FALLBACK_VARIABLE, EMOTE_ROLE_VARIABLE};
use crate::springs::{self, SpringSet};
use crate::texture::{load_texture, IndexedTexture};
use crate::wearer::{WearerDeclarations, MAX_ONE_SHOTS, RING_POINTS};

pub use crate::container::{Kind, Skip, Source};

pub const MANIFEST_KEYS: [&str; 8] = [
    "geometry",
    "textures",
    "palettes",
    "animations",
    "animationControllers",
    "controllers",
    "reactions",
    "attach",
];
pub const EMOTE_KEYS: [&str; 5] = ["roles", "rules", "requirement", "fallback", "main"];
pub const MODELS_DIR: &str = "models";
pub const GEOMETRY_EXT: &str = "geo.json";
pub const ANIMATIONS_DIR: &str = "animations";
pub const ANIMATION_EXT: &str = "animation.json";
pub const CONTROLLERS_DIR: &str = "animation_controllers";
pub const CONTROLLER_EXT: &str = "animation_controllers.json";
pub const ITEM_PALETTE_TONES: usize = 255;
pub const ITEM_PALETTE_SLOTS: usize = ITEM_PALETTE_TONES / MAX_TONES + 1;
pub const ENGINE_ORDER: [&str; 13] = [
    "controllers",
    "springs",
    "effects",
    "particles",
    "trails",
    "sounds",
    "reactions",
    "palettes",
    "custom_colors",
    "rigs",
    "pets",
    "roles",
    "events",
];

const TEXTURE_KEYS: [&str; 2] = ["id", "palette"];
const RULES: [(&str, &str); 7] = [
    ("allow_walking", "allowWalking"),
    ("allow_sprinting", "allowSprinting"),
    ("allow_crouching", "allowCrouching"),
    ("allow_falling", "allowFalling"),
    ("allow_elytra_flying", "allowElytraFlying"),
    ("allow_swimming", "allowSwimming"),
    ("allow_vanilla_pose", "allowVanillaPose"),
];
const HEAD_RULE: (&str, &str) = ("absolute_head", "absoluteHead");
pub const KEPT_REACTION_FLOATS: usize = 3;
pub const EMOTE_RECORD_FLOATS: usize = 12;
pub const PET_PLACE_FLOATS: usize = 17;
const EMPTY_BASE: &str = r#"{"animations":{"":{"loop":true}}}"#;
const LEGACY_EMOTE_DIR: &str = "emotes/";
const LEGACY_RULES_EXT: &str = ".emote.json";
const LEGACY_EMOTE_ID: &str = "emote";
const LEGACY_TEXTURE_DIRS: [&str; 4] = ["textures/", "", "textures/emotes/", "textures/cosmetics/"];

#[derive(Clone, Copy, Debug)]
pub struct LoadOptions<'a> {
    pub mode: Mode,
    pub identity: u64,
    pub capabilities: &'a [&'a str],
}

impl LoadOptions<'static> {
    pub fn new(mode: Mode, identity: u64) -> LoadOptions<'static> {
        LoadOptions {
            mode,
            identity,
            capabilities: CAPABILITIES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EmoteRules {
    pub allow_walking: bool,
    pub allow_sprinting: bool,
    pub allow_crouching: bool,
    pub allow_falling: bool,
    pub allow_elytra_flying: bool,
    pub allow_swimming: bool,
    pub allow_vanilla_pose: bool,
    pub absolute_head: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmoteRequirement {
    #[default]
    Floor,
    Ledge,
    Wall,
}

impl EmoteRequirement {
    pub const ALL: [EmoteRequirement; 3] = [
        EmoteRequirement::Floor,
        EmoteRequirement::Ledge,
        EmoteRequirement::Wall,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EmoteRequirement::Floor => "floor",
            EmoteRequirement::Ledge => "ledge",
            EmoteRequirement::Wall => "wall",
        }
    }

    pub fn parse(name: &str) -> Option<EmoteRequirement> {
        EmoteRequirement::ALL
            .into_iter()
            .find(|r| r.as_str() == name)
    }
}

impl Default for EmoteRules {
    fn default() -> Self {
        EmoteRules {
            allow_walking: false,
            allow_sprinting: false,
            allow_crouching: false,
            allow_falling: true,
            allow_elytra_flying: false,
            allow_swimming: false,
            allow_vanilla_pose: false,
            absolute_head: true,
        }
    }
}

impl EmoteRules {
    fn flags(&mut self) -> [&mut bool; 8] {
        [
            &mut self.allow_walking,
            &mut self.allow_sprinting,
            &mut self.allow_crouching,
            &mut self.allow_falling,
            &mut self.allow_elytra_flying,
            &mut self.allow_swimming,
            &mut self.allow_vanilla_pose,
            &mut self.absolute_head,
        ]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectMedia {
    pub textures: Vec<IndexedTexture>,
    pub sounds: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub struct CosmeticAsset {
    pub id: String,
    pub identity: u64,
    pub kind: Kind,
    pub model: Option<Model>,
    pub textures: Vec<IndexedTexture>,
    pub palette: Palette,
    pub animations: Vec<Animation>,
    pub interner: Interner,
    pub controllers: ControllerSet,
    pub reactions: Vec<Reaction>,
    pub springs: SpringSet,
    pub effects: EffectSet,
    pub media: EffectMedia,
    pub attach: Attach,
    pub pet: Option<PetSpec>,
    pub roles: Vec<Role>,
    pub rules: Option<EmoteRules>,
    pub requirement: EmoteRequirement,
    pub fallback: Option<u16>,
    pub main: Option<u16>,
    pub record: WearerDeclarations,
    pub requires: Vec<String>,
}

impl CosmeticAsset {
    pub fn new(id: &str, identity: u64, kind: Kind) -> CosmeticAsset {
        CosmeticAsset {
            id: id.to_string(),
            identity,
            kind,
            model: None,
            textures: Vec::new(),
            palette: Palette::default(),
            animations: Vec::new(),
            interner: Interner::default(),
            controllers: ControllerSet::default(),
            reactions: Vec::new(),
            springs: SpringSet::default(),
            effects: EffectSet::default(),
            media: EffectMedia::default(),
            attach: Attach::default(),
            pet: None,
            roles: Vec::new(),
            rules: None,
            requirement: EmoteRequirement::Floor,
            fallback: None,
            main: None,
            record: WearerDeclarations::default(),
            requires: Vec::new(),
        }
    }

    pub fn is_moment_set(&self) -> bool {
        self.kind == Kind::Effect && !self.effects.moments.is_empty()
    }
}

#[derive(Debug)]
pub enum Loaded {
    Asset {
        asset: Arc<CosmeticAsset>,
        issues: Vec<Issue>,
    },
    Draft {
        asset: Arc<CosmeticAsset>,
        issues: Vec<Issue>,
    },
    Skipped(Skip),
    Invalid(Vec<Issue>),
    Legacy(Legacy),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Analysis {
    pub requires: Vec<String>,
    pub quads: u32,
    pub bones: u32,
    pub spring_bones: u32,
    pub live_particles: f64,
    pub trail_points: u32,
    pub record_bytes: u32,
    pub texture_bytes: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Picks {
    geometry: Option<String>,
    texture: Option<String>,
    animations: Vec<String>,
    controllers: Vec<String>,
}

fn file_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn is_controller_file(lower: &str) -> bool {
    lower.ends_with(".animation_controllers.json")
        || (lower.ends_with(".json") && lower.rsplit('/').nth(1) == Some(CONTROLLERS_DIR))
}

fn picks(files: &[(String, Vec<u8>)]) -> Picks {
    let mut names: Vec<&str> = files
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| *n != MANIFEST && !file_name(n).starts_with("._"))
        .collect();
    names.sort_unstable();
    let parts = bundle::parts(&names);
    let pick = |at: Option<usize>| at.map(|i| names[i].to_string());
    let animations = names
        .iter()
        .filter(|n| {
            let lower = n.to_ascii_lowercase();
            lower.ends_with(".json")
                && !lower.ends_with(".geo.json")
                && !lower.ends_with(".emote.json")
                && !is_controller_file(&lower)
                && file_name(&lower) != LEGACY_PET
        })
        .map(|n| n.to_string())
        .collect();
    Picks {
        geometry: pick(parts.geometry),
        texture: pick(parts.texture),
        animations,
        controllers: parts
            .controllers
            .iter()
            .map(|&i| names[i].to_string())
            .collect(),
    }
}

fn container_options<'a>(o: &LoadOptions<'a>) -> ContainerOptions<'a> {
    ContainerOptions {
        mode: o.mode,
        legacy: ITEM_CAPS,
        capabilities: o.capabilities,
    }
}

pub fn load(src: Source<'_>, o: &LoadOptions<'_>) -> Loaded {
    let options = container_options(o);
    match open_source(src, &options) {
        Opened::Bundle(bundle) => load_bundle(bundle, None, o),
        Opened::Skipped(skip) => Loaded::Skipped(skip),
        Opened::Invalid(issues) => Loaded::Invalid(issues),
        Opened::Legacy(legacy) => {
            if !legacy.files.iter().any(|(n, _)| n == LEGACY_PET) {
                return match convert_legacy_emote(&legacy.files) {
                    Some(files) => {
                        load_converted(&files, None, &options, o, || Loaded::Legacy(legacy))
                    }
                    None => Loaded::Legacy(legacy),
                };
            }
            let Some((files, skin)) = pet::convert_legacy_with_skin(&legacy.files) else {
                let issue =
                    Issue::error(IssueCode::PetLegacyInvalid, LEGACY_PET).name("file", LEGACY_PET);
                return Loaded::Invalid(vec![issue]);
            };
            load_converted(&files, Some(skin), &options, o, || {
                let issue =
                    Issue::error(IssueCode::PetLegacyInvalid, LEGACY_PET).name("file", LEGACY_PET);
                Loaded::Invalid(vec![issue])
            })
        }
    }
}

fn load_converted(
    files: &[(String, Vec<u8>)],
    texture: Option<String>,
    options: &ContainerOptions<'_>,
    o: &LoadOptions<'_>,
    fallback: impl FnOnce() -> Loaded,
) -> Loaded {
    let mut picked = picks(files);
    if texture.is_some() {
        picked.texture = texture;
    }
    match open_files(files, options) {
        Opened::Bundle(bundle) => load_bundle(bundle, Some(&picked), o),
        Opened::Skipped(skip) => Loaded::Skipped(skip),
        Opened::Invalid(issues) => Loaded::Invalid(issues),
        Opened::Legacy(_) => fallback(),
    }
}

fn clip_stem(file: &str) -> &str {
    let name = file_name(file);
    let name = name.strip_suffix(".json").unwrap_or(name);
    name.strip_suffix(".animation").unwrap_or(name)
}

fn legacy_pack(clip: &str) -> Option<&str> {
    let rel = clip.strip_prefix(LEGACY_EMOTE_DIR).unwrap_or(clip);
    let (dir, _) = rel.split_once('/')?;
    (clip_stem(rel) == dir).then_some(dir)
}

fn legacy_id(stem: &str) -> String {
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
        LEGACY_EMOTE_ID.to_string()
    }
}

fn is_legacy_emote(names: &[&str]) -> bool {
    let geometry = |n: &&str| n.ends_with(".geo.json") && !n.ends_with("player.geo.json");
    names.iter().any(|n| n.ends_with(LEGACY_RULES_EXT))
        || names.iter().any(|n| n.starts_with(LEGACY_EMOTE_DIR))
        || !names.iter().any(geometry)
}

fn legacy_rule(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(on) => Some(*on),
        Value::String(text) => Some(text.eq_ignore_ascii_case("true")),
        Value::Number(_) => Some(false),
        _ => None,
    }
}

fn legacy_rules(root: &Value, clip: &str) -> Option<Map<String, Value>> {
    let animations = root.get("animations")?.as_object()?;
    let mut picked = None;
    for (name, entry) in animations {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let mut rules = Map::new();
        match entry.get("rules") {
            None => {}
            Some(Value::Object(map)) => {
                for (snake, camel) in RULES {
                    let Some(value) = map.get(snake).or_else(|| map.get(camel)) else {
                        continue;
                    };
                    rules.insert(snake.to_string(), Value::Bool(legacy_rule(value)?));
                }
            }
            Some(_) => return None,
        }
        if name == clip && picked.is_none() {
            picked = Some(rules);
        }
    }
    picked
}

fn has_cubes(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    bedrock::parse_checked(text).is_ok_and(|parsed| {
        parsed
            .models
            .first()
            .is_some_and(|m| m.bones.iter().any(|b| !b.cubes.is_empty()))
    })
}

pub fn convert_legacy_emote(files: &[(String, Vec<u8>)]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut sorted: Vec<&(String, Vec<u8>)> = files
        .iter()
        .filter(|(name, _)| !file_name(name).starts_with("._"))
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let names: Vec<&str> = sorted.iter().map(|(n, _)| n.as_str()).collect();
    if names
        .iter()
        .any(|&n| n == MANIFEST || file_name(n) == LEGACY_PET)
        || !is_legacy_emote(&names)
    {
        return None;
    }
    let mut scratch = Report::new(Mode::Client);
    let (clip_file, clip_bytes, clip) = sorted.iter().find_map(|(name, bytes)| {
        let lower = name.to_ascii_lowercase();
        let candidate = lower.ends_with(".json")
            && !lower.ends_with(".geo.json")
            && !lower.ends_with(LEGACY_RULES_EXT)
            && !is_controller_file(&lower);
        if !candidate {
            return None;
        }
        let root = guarded_json(bytes, name, &mut scratch)?;
        let first = root.get("animations")?.as_object()?.keys().next()?.clone();
        Some((name.as_str(), bytes, first))
    })?;

    let rules_file = format!(
        "{}{LEGACY_RULES_EXT}",
        clip_file
            .strip_suffix(".animation.json")
            .or_else(|| clip_file.strip_suffix(".json"))
            .unwrap_or(clip_file)
    );
    let rules = sorted
        .iter()
        .find(|(name, _)| *name == rules_file || name.ends_with(&format!("/{rules_file}")))
        .and_then(|(name, bytes)| guarded_json(bytes, name, &mut scratch))
        .and_then(|root| legacy_rules(&root, &clip))
        .unwrap_or_default();

    let pack = legacy_pack(clip_file);
    let props = pack.and_then(|pack| {
        let wanted = format!("{LEGACY_EMOTE_DIR}{pack}/{pack}.geo.json");
        let nested = format!("/{wanted}");
        let flat = format!("{pack}.geo.json");
        let geometry = sorted.iter().find(|(name, _)| {
            *name == wanted || name.ends_with(&nested) || name.ends_with(&flat)
        })?;
        if !has_cubes(&geometry.1) {
            return None;
        }
        let texture = LEGACY_TEXTURE_DIRS.iter().find_map(|dir| {
            let path = format!("{dir}{pack}.png");
            sorted.iter().find(|(name, _)| *name == path)
        })?;
        Some([*geometry, *texture])
    });

    let mut manifest = Map::new();
    manifest.insert("format".to_string(), Value::from(FORMAT));
    manifest.insert("kind".to_string(), Value::from(Kind::Emote.as_str()));
    let stem = pack.unwrap_or_else(|| clip_stem(clip_file));
    manifest.insert("id".to_string(), Value::from(legacy_id(stem)));
    if !rules.is_empty() {
        manifest.insert("rules".to_string(), Value::Object(rules));
    }
    manifest.insert("main".to_string(), Value::from(clip.as_str()));
    let manifest = serde_json::to_vec_pretty(&Value::Object(manifest)).ok()?;

    let mut out = vec![
        (MANIFEST.to_string(), manifest),
        (clip_file.to_string(), (*clip_bytes).clone()),
    ];
    for (name, bytes) in props.into_iter().flatten() {
        out.push((name.clone(), bytes.clone()));
    }
    Some(out)
}

fn kind_keys(kind: Kind) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = MANIFEST_KEYS.to_vec();
    keys.extend_from_slice(effects::MANIFEST_KEYS);
    match kind {
        Kind::Pet => keys.extend_from_slice(&pet::PET_KEYS),
        Kind::Emote => keys.extend_from_slice(&EMOTE_KEYS),
        _ => {}
    }
    keys
}

fn manifest_at(key: &str) -> String {
    path(MANIFEST, key)
}

fn authoring(r: &mut Report, issue: Issue) {
    if r.mode() == Mode::Authoring {
        r.push(issue);
    }
}

fn id_list<'v>(manifest: &'v Value, key: &str, code: IssueCode, r: &mut Report) -> Vec<&'v str> {
    let Some(value) = manifest.get(key) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        r.push(Issue::error(code, manifest_at(key)));
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        match item.as_str() {
            Some(text) => out.push(text),
            None => r.push(Issue::error(code, manifest_at(&join(key, &i.to_string())))),
        }
    }
    out
}

fn json_text(b: &mut Bundle<'_>, file: &str, from: &str) -> Option<String> {
    b.json(file, from).map(|value| value.to_string())
}

fn geometry(
    b: &mut Bundle<'_>,
    manifest: &Value,
    legacy: Option<&Picks>,
    r: &mut Report,
) -> (Option<Model>, Option<String>) {
    let from = manifest_at("geometry");
    let file = match legacy {
        Some(picks) => picks.geometry.clone(),
        None => match manifest.get("geometry") {
            None => None,
            Some(Value::String(id)) => b.id_path(MODELS_DIR, id, GEOMETRY_EXT, &from),
            Some(_) => {
                r.push(Issue::error(IssueCode::GeometryInvalid, from.as_str()));
                None
            }
        },
    };
    let Some(file) = file else {
        return (None, None);
    };
    let Some(text) = json_text(b, &file, &from) else {
        return (None, Some(file));
    };
    match bedrock::parse_checked(&text) {
        Ok(parsed) => {
            bedrock::report_faults(&file, &parsed.faults, r);
            let model = parsed.models.into_iter().next();
            if model.is_none() {
                r.push(Issue::error(IssueCode::GeometryInvalid, file.as_str()));
            }
            (model, Some(file))
        }
        Err(e) => {
            r.push(
                Issue::error(IssueCode::GeometryInvalid, file.as_str())
                    .name("error", &e.to_string()),
            );
            (None, Some(file))
        }
    }
}

fn textures(
    b: &mut Bundle<'_>,
    manifest: &Value,
    legacy: Option<&Picks>,
    r: &mut Report,
) -> Vec<IndexedTexture> {
    if let Some(picks) = legacy {
        let Some(name) = &picks.texture else {
            return Vec::new();
        };
        let Some(image) = b.png(name, MANIFEST) else {
            return Vec::new();
        };
        let stem = file_name(name).trim_end_matches(".png").to_string();
        return vec![IndexedTexture {
            name: stem,
            width: image.width,
            height: image.height,
            rgba: image.rgba,
            index: None,
        }];
    }
    let Some(value) = manifest.get("textures") else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        r.push(Issue::error(IssueCode::Texture, manifest_at("textures")));
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at = join("textures", &i.to_string());
        let (id, indexed) = match item {
            Value::String(id) => (id.as_str(), false),
            Value::Object(map) => {
                r.check_keys(MANIFEST, &at, map, &TEXTURE_KEYS);
                let id = map.get("id").and_then(Value::as_str);
                let indexed = match map.get("palette") {
                    None => Some(false),
                    Some(Value::Bool(b)) => Some(*b),
                    Some(_) => None,
                };
                match (id, indexed) {
                    (Some(id), Some(indexed)) => (id, indexed),
                    _ => {
                        r.push(Issue::error(IssueCode::Texture, manifest_at(&at)));
                        continue;
                    }
                }
            }
            _ => {
                r.push(Issue::error(IssueCode::Texture, manifest_at(&at)));
                continue;
            }
        };
        if let Some(texture) = load_texture(b, id, indexed, &manifest_at(&at)) {
            out.push(texture);
        }
    }
    out
}

fn palette(
    b: &mut Bundle<'_>,
    manifest: &Value,
    textures: &[IndexedTexture],
    r: &mut Report,
) -> Palette {
    let wanted = match manifest.get("palettes") {
        None => false,
        Some(Value::Bool(on)) => *on,
        Some(_) => {
            r.push(Issue::error(IssueCode::Palette, manifest_at("palettes")));
            false
        }
    };
    let palette = if wanted {
        read_palette(b).unwrap_or_default()
    } else {
        Palette::default()
    };
    if palette.slots.len() > ITEM_PALETTE_SLOTS {
        r.push(
            Issue::error(IssueCode::Palette, manifest_at("palettes"))
                .value("count", palette.slots.len() as f64)
                .value("limit", ITEM_PALETTE_SLOTS as f64),
        );
    }
    for slot in 0..palette.slots.len().min(ITEM_PALETTE_SLOTS) {
        let limit = (ITEM_PALETTE_TONES - slot * MAX_TONES).min(MAX_TONES);
        let count = (0..palette.colorways.len())
            .map(Some)
            .chain([None])
            .map(|colorway| palette.tone_count(slot, colorway))
            .max()
            .unwrap_or(0);
        if count > limit {
            r.push(
                Issue::error(IssueCode::Palette, manifest_at("palettes"))
                    .value("slot", slot as f64)
                    .value("count", count as f64)
                    .value("limit", limit as f64),
            );
        }
    }
    for texture in textures {
        if texture.index.is_some() && !wanted {
            r.push(
                Issue::error(IssueCode::TextureIndexInvalid, manifest_at("palettes"))
                    .name("texture", &texture.name),
            );
        }
    }
    if wanted {
        for texture in textures {
            crate::texture::check_index(texture, &palette, r);
        }
    }
    palette
}

fn animations(
    b: &mut Bundle<'_>,
    manifest: &Value,
    legacy: Option<&Picks>,
    interner: &mut Interner,
    rules: Rules,
    r: &mut Report,
) -> (Vec<Animation>, Vec<String>) {
    let files: Vec<(String, String)> = match legacy {
        Some(picks) => picks
            .animations
            .iter()
            .map(|f| (f.clone(), MANIFEST.to_string()))
            .collect(),
        None => id_list(manifest, "animations", IssueCode::AnimationsInvalid, r)
            .into_iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let from = manifest_at(&join("animations", &i.to_string()));
                let file = b.id_path(ANIMATIONS_DIR, id, ANIMATION_EXT, &from)?;
                Some((file, from))
            })
            .collect(),
    };
    let mut anims = Vec::new();
    let mut sources = Vec::new();
    for (file, from) in files {
        let Some(text) = json_text(b, &file, &from) else {
            continue;
        };
        match animation::parse_with(&text, interner, rules) {
            Ok(parsed) => {
                sources.extend(std::iter::repeat_n(file.clone(), parsed.len()));
                anims.extend(parsed);
            }
            Err(e) => r.push(
                Issue::error(IssueCode::AnimationsInvalid, file.as_str())
                    .name("error", &e.to_string()),
            ),
        }
    }
    (anims, sources)
}

struct ControllerLoad {
    loaded: Vec<Controller>,
    declared: Vec<String>,
}

struct Inputs<'a> {
    manifest: &'a Value,
    legacy: Option<&'a Picks>,
    rules: Rules,
}

#[allow(clippy::too_many_arguments)]
fn controllers(
    b: &mut Bundle<'_>,
    inputs: &Inputs<'_>,
    spec: Option<&PetSpec>,
    look: Option<&str>,
    anims: &[Animation],
    interner: &mut Interner,
    r: &mut Report,
) -> ControllerLoad {
    let Inputs {
        manifest,
        legacy,
        rules,
    } = *inputs;
    let files: Vec<(String, String)> = match legacy {
        Some(picks) => picks
            .controllers
            .iter()
            .map(|f| (f.clone(), MANIFEST.to_string()))
            .collect(),
        None => id_list(manifest, "animationControllers", IssueCode::Manifest, r)
            .into_iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let from = manifest_at(&join("animationControllers", &i.to_string()));
                let file = b.id_path(CONTROLLERS_DIR, id, CONTROLLER_EXT, &from)?;
                Some((file, from))
            })
            .collect(),
    };
    let mut read: Vec<(String, Vec<u8>)> = Vec::with_capacity(files.len());
    for (file, from) in files {
        if let Some(bytes) = b.bytes(&file, &from) {
            read.push((file, bytes));
        }
    }
    let slices: Vec<(&str, &[u8])> = read
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let mut loaded = bundle::controllers(&slices, interner, rules, r);

    let mut declared: Vec<String> = match spec {
        Some(spec) if spec.controllers.is_empty() => {
            let names: Vec<&str> = anims.iter().map(|a| a.name.as_str()).collect();
            let json = pet::default_controller_for(spec.archetype, &names);
            if let Ok(root) = serde_json::from_str::<Value>(&json) {
                loaded.extend(controller::parse(
                    pet::GENERATED_CONTROLLER,
                    &root,
                    interner,
                    rules,
                    r,
                ));
            }
            vec![pet::default_controller_name(spec.archetype).to_string()]
        }
        Some(spec) => spec.controllers.clone(),
        None => id_list(manifest, "controllers", IssueCode::Manifest, r)
            .into_iter()
            .map(str::to_string)
            .collect(),
    };
    if let Some(clip) = look {
        let json = pet::LOOK_CONTROLLER.replace(
            &format!("[\"{}\"]", pet::LOOK_ANIMATION_NAME),
            &format!("[\"{clip}\"]"),
        );
        if let Ok(root) = serde_json::from_str::<Value>(&json) {
            loaded.extend(controller::parse(
                pet::GENERATED_CONTROLLER,
                &root,
                interner,
                rules,
                r,
            ));
            declared.push(pet::LOOK_CONTROLLER_NAME.to_string());
        }
    }
    ControllerLoad { loaded, declared }
}

fn base_first(
    manifest: &Value,
    anims: &mut Vec<Animation>,
    sources: &mut Vec<String>,
    interner: &mut Interner,
    rules: Rules,
) {
    let played: Vec<usize> = manifest
        .get("reactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("play")?.as_str())
        .filter_map(|name| reactions::resolve(anims, name))
        .collect();
    if played.is_empty() {
        return;
    }
    match (0..anims.len()).find(|i| !played.contains(i)) {
        Some(0) => {}
        Some(base) => {
            anims[..=base].rotate_right(1);
            sources[..=base].rotate_right(1);
        }
        None => {
            if let Ok(mut empty) = animation::parse_with(EMPTY_BASE, interner, rules) {
                anims.splice(0..0, empty.drain(..));
                sources.insert(0, MANIFEST.to_string());
            }
        }
    }
}

fn shift_bone(bone: &mut Bone, by: [f32; 3]) {
    let add = |at: &mut [f32; 3]| {
        for (axis, step) in at.iter_mut().zip(by) {
            *axis += step;
        }
    };
    add(&mut bone.pivot);
    for cube in &mut bone.cubes {
        add(&mut cube.origin);
        add(&mut cube.pivot);
    }
    for locator in &mut bone.locators {
        add(&mut locator.offset);
    }
}

fn emote_requirement(manifest: &Value, r: &mut Report) -> EmoteRequirement {
    match manifest.get("requirement") {
        None => EmoteRequirement::Floor,
        Some(value) => match value.as_str().and_then(EmoteRequirement::parse) {
            Some(requirement) => requirement,
            None => {
                r.push(Issue::error(
                    IssueCode::EmoteRulesInvalid,
                    manifest_at("requirement"),
                ));
                EmoteRequirement::Floor
            }
        },
    }
}

fn solo_plays(kind: Kind, asset: &CosmeticAsset, anims: &[Animation], set: &ControllerSet) -> Solo {
    if asset.is_moment_set() {
        return Solo::moments(&asset.effects);
    }
    let roles = asset.roles.iter().map(|role| usize::from(role.animation));
    if set.controllers().is_empty() {
        let base = Solo::worn(anims.len().min(1), set);
        return match kind {
            Kind::Pet => base.bypassing(roles),
            Kind::Emote => {
                let fallback = asset.fallback.map(usize::from);
                let main = asset.main.map(usize::from);
                base.bypassing(roles.chain(main).chain(fallback))
            }
            _ => base,
        };
    }
    let solo = Solo::worn(anims.len(), set);
    let unread = |name: &str| {
        asset
            .interner
            .variable_slot(name)
            .is_none_or(|slot| !set.reads_variable(slot))
    };
    match kind {
        Kind::Pet => solo.bypassing(roles),
        Kind::Emote => {
            let roles = roles.filter(|_| unread(EMOTE_ROLE_VARIABLE));
            let fallback = asset
                .fallback
                .map(usize::from)
                .filter(|_| anims.len() > 1 && unread(EMOTE_FALLBACK_VARIABLE));
            solo.bypassing(roles.chain(fallback))
        }
        _ => solo,
    }
}

fn emote_fallback(manifest: &Value, anims: &[Animation], r: &mut Report) -> Option<u16> {
    let value = manifest.get("fallback")?;
    let found = value
        .as_str()
        .and_then(|name| reactions::resolve(anims, name))
        .and_then(|i| u16::try_from(i).ok());
    if found.is_none() {
        let mut issue = Issue::error(IssueCode::EmoteRulesInvalid, manifest_at("fallback"));
        if let Some(name) = value.as_str() {
            issue = issue.name("animation", name);
        }
        r.push(issue);
    }
    found
}

fn emote_main(
    manifest: &Value,
    anims: &[Animation],
    fallback: Option<u16>,
    r: &mut Report,
) -> Option<u16> {
    let fallback = fallback.map(usize::from);
    let others = || (0..anims.len()).filter(|&clip| Some(clip) != fallback);
    let guess = if anims.len() == 1 {
        Some(0)
    } else {
        others().next()
    };
    let at = manifest_at("main");
    let main = match manifest.get("main") {
        None => {
            if let (Some(clip), true) = (guess, others().count() > 1) {
                authoring(
                    r,
                    Issue::error(IssueCode::EmoteMainUnnamed, at)
                        .value("clips", anims.len() as f64)
                        .name("animation", &anims[clip].name),
                );
            }
            guess
        }
        Some(value) => match value
            .as_str()
            .and_then(|name| reactions::resolve(anims, name))
        {
            None => {
                let mut issue = Issue::error(IssueCode::EmoteRulesInvalid, at);
                if let Some(name) = value.as_str() {
                    issue = issue.name("animation", name);
                }
                r.push(issue);
                guess
            }
            Some(clip) => {
                if Some(clip) == fallback && anims.len() > 1 {
                    authoring(
                        r,
                        Issue::error(IssueCode::EmoteRulesInvalid, at)
                            .name("animation", &anims[clip].name),
                    );
                }
                Some(clip)
            }
        },
    };
    main.and_then(|clip| u16::try_from(clip).ok())
}

fn emote_rules(manifest: &Value, r: &mut Report) -> EmoteRules {
    let mut rules = EmoteRules::default();
    let Some(value) = manifest.get("rules") else {
        return rules;
    };
    let Some(map) = value.as_object() else {
        r.push(Issue::error(
            IssueCode::EmoteRulesInvalid,
            manifest_at("rules"),
        ));
        return rules;
    };
    let keys = || RULES.iter().chain([&HEAD_RULE]);
    let known: Vec<&str> = keys().flat_map(|(a, b)| [*a, *b]).collect();
    r.check_keys(MANIFEST, "rules", map, &known);
    for ((snake, camel), flag) in keys().zip(rules.flags()) {
        let key = if map.contains_key(*snake) {
            snake
        } else {
            camel
        };
        match map.get(*key) {
            None => {}
            Some(Value::Bool(on)) => *flag = *on,
            Some(_) => r.push(Issue::error(
                IssueCode::EmoteRulesInvalid,
                manifest_at(&join("rules", key)),
            )),
        }
    }
    rules
}

fn finish(b: Bundle<'_>, r: Report, asset: CosmeticAsset, mode: Mode) -> Loaded {
    let mut issues = b.finish();
    issues.extend(r.into_issues());
    let asset = Arc::new(asset);
    let errors = issues.iter().any(|i| i.level == Level::Error);
    match (errors, mode) {
        (false, _) => Loaded::Asset { asset, issues },
        (true, Mode::Authoring) => Loaded::Draft { asset, issues },
        (true, Mode::Client) => Loaded::Invalid(issues),
    }
}

fn effect_media(b: &mut Bundle<'_>, e: &EffectSet, r: &mut Report) -> EffectMedia {
    let max = b.kind().max_texture_size();
    let mut read = |name: &str| {
        Some(name)
            .filter(|n| b.has_file(n))
            .and_then(|n| b.bytes(n, n))
    };
    let mut textures: Vec<IndexedTexture> = Vec::new();
    for local in 0..texture_count(e) {
        let name = texture_path(e, local).unwrap_or_default().to_string();
        if let Some(seen) = textures.iter().find(|t| t.name == name) {
            textures.push(seen.clone());
            continue;
        }
        let image = read(&name).and_then(|bytes| decode_png(&bytes, &name, max, r));
        textures.push(match image {
            Some(image) => IndexedTexture {
                name,
                width: image.width,
                height: image.height,
                rgba: image.rgba,
                index: None,
            },
            None => IndexedTexture {
                name,
                ..IndexedTexture::default()
            },
        });
    }
    let sounds = e
        .sounds
        .iter()
        .map(|s| s.file.as_deref().and_then(&mut read).unwrap_or_default())
        .collect();
    EffectMedia { textures, sounds }
}

fn unused_clip_name(anims: &[Animation], wanted: &str) -> String {
    let taken = |name: &str| anims.iter().any(|a| a.name == name);
    if !taken(wanted) {
        return wanted.to_string();
    }
    (2..)
        .map(|n| format!("{wanted}_{n}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

fn load_bundle(mut b: Bundle<'_>, legacy: Option<&Picks>, o: &LoadOptions<'_>) -> Loaded {
    let kind = b.kind();
    if kind == Kind::Nameplate {
        let issue = Issue::error(IssueCode::AssetKindUnsupported, manifest_at("kind"))
            .name("kind", kind.as_str());
        b.report().push(issue);
        return Loaded::Invalid(b.finish());
    }
    let mode = b.mode();
    let manifest = Value::Object(b.manifest().clone());
    b.check_manifest_keys(&kind_keys(kind));
    let mut r = Report::new(mode);
    let rules = if legacy.is_some() {
        Rules::Bedrock
    } else {
        Rules::Strict
    };

    let mut asset = CosmeticAsset::new(b.id(), o.identity, kind);
    asset.requires = b.requires().to_vec();
    if let (Kind::Pet, Value::Object(map)) = (kind, &manifest) {
        asset.pet = Some(pet::parse(map, &mut r));
    }

    let (model, geometry_file) = geometry(&mut b, &manifest, legacy, &mut r);
    if model.is_none() && geometry_file.is_none() && matches!(kind, Kind::Item | Kind::Pet) {
        r.push(Issue::error(
            IssueCode::GeometryMissing,
            manifest_at("geometry"),
        ));
    }
    asset.textures = textures(&mut b, &manifest, legacy, &mut r);
    asset.palette = palette(&mut b, &manifest, &asset.textures, &mut r);

    let (mut anims, mut sources) = animations(
        &mut b,
        &manifest,
        legacy,
        &mut asset.interner,
        rules,
        &mut r,
    );
    if kind == Kind::Emote && anims.is_empty() {
        r.push(Issue::error(
            IssueCode::AnimationsInvalid,
            manifest_at("animations"),
        ));
    }
    let look = asset.pet.as_ref().is_some_and(pet::uses_default_controller)
        && model
            .as_ref()
            .is_some_and(|m| m.bone(pet::LOOK_BONE).is_some());
    let look = look.then(|| unused_clip_name(&anims, pet::LOOK_ANIMATION_NAME));
    if let Some(name) = &look {
        if let Ok(mut extra) =
            animation::parse_with(pet::LOOK_ANIMATION, &mut asset.interner, rules)
        {
            for clip in &mut extra {
                clip.name = name.clone();
            }
            sources.extend(extra.iter().map(|_| MANIFEST.to_string()));
            anims.extend(extra);
        }
    }
    let inputs = Inputs {
        manifest: &manifest,
        legacy,
        rules,
    };
    let ControllerLoad { loaded, declared } = controllers(
        &mut b,
        &inputs,
        asset.pet.as_ref(),
        look.as_deref(),
        &anims,
        &mut asset.interner,
        &mut r,
    );
    let names: Vec<&str> = declared.iter().map(String::as_str).collect();
    let mut set = ControllerSet::select(loaded, &names, &mut r);
    if set.controllers().is_empty() && kind != Kind::Emote {
        base_first(
            &manifest,
            &mut anims,
            &mut sources,
            &mut asset.interner,
            rules,
        );
    }
    set.link(&anims, &asset.interner, &mut r);
    for (index, animation) in anims.iter().enumerate() {
        if animation.anim_time_update().is_some() && !set.plays(index) {
            let at = join(&join("animations", &animation.name), "anim_time_update");
            r.push(
                Issue::warning(IssueCode::Animation, path(&sources[index], &at))
                    .name("key", "anim_time_update"),
            );
        }
    }

    asset.springs = model
        .as_ref()
        .map_or_else(SpringSet::default, |m| springs::build(m, &mut r));
    asset.effects = effects::parse(&mut b, model.as_ref(), &mut r);
    effects::bind_locators(&mut asset.effects, &anims, &set);
    asset.media = effect_media(&mut b, &asset.effects, &mut r);
    effects::plan_moments(
        &mut asset.effects,
        &anims,
        &asset.media.sounds,
        kind == Kind::Effect,
        &mut r,
    );
    effects::check_events(&asset.effects, &anims, &set, &mut r);
    asset.reactions = reactions::parse(&manifest, &anims, &asset.palette, &asset.effects, &mut r);

    asset.attach = match manifest.get("attach") {
        Some(value) => Attach::parse(value, &mut r),
        None if kind == Kind::Pet => Attach {
            rig: RigKind::Actor,
            ..Attach::default()
        },
        None => Attach::default(),
    };
    let mut model = model;
    if let (Some(target), Some(model)) = (asset.attach.bone.as_deref(), model.as_mut()) {
        let offset = asset.attach.offset();
        let mut hung = vec![false; model.bones.len()];
        for i in 0..model.bones.len() {
            let bone = &mut model.bones[i];
            hung[i] = match bone.parent {
                Some(parent) => hung.get(parent).copied().unwrap_or(false),
                None => bone.parent_name.is_none(),
            };
            if bone.parent.is_none() && hung[i] {
                bone.parent_name = Some(target.to_string());
            }
            if hung[i] && offset != [0.0; 3] {
                shift_bone(bone, offset);
            }
        }
    }
    if kind == Kind::Pet {
        attach::warn_plain_signals(&asset.interner, rules, MANIFEST, &mut r);
        let declared = asset
            .pet
            .as_ref()
            .map_or(&[][..], |pet| pet.interactions.as_slice());
        asset.roles = roles::parse_pet(&manifest, &anims, declared, &mut r);
    }
    if kind == Kind::Emote {
        asset.roles = roles::parse(&manifest, &anims, &mut r);
        asset.rules = Some(emote_rules(&manifest, &mut r));
        asset.requirement = emote_requirement(&manifest, &mut r);
        asset.fallback = emote_fallback(&manifest, &anims, &mut r);
        asset.main = emote_main(&manifest, &anims, asset.fallback, &mut r);
        set.keep_ends();
    }
    let solo = solo_plays(kind, &asset, &anims, &set);
    effects::size_playbacks(&mut asset.effects, &anims, &set, &asset.reactions, &solo);
    effects::size_catch_up(&asset.effects, &anims, &mut set);
    effects::check_runs(
        &asset.effects,
        &anims,
        &set,
        &asset.reactions,
        &solo,
        &mut r,
    );
    effects::check_budgets(&asset.effects, MANIFEST, &mut r);
    effects::check_plays(&anims, &set, &asset.reactions, &solo, MANIFEST, &mut r);

    asset.model = model;
    asset.animations = anims;
    if reactions::paces(&asset.reactions) {
        set.pace();
    }
    asset.controllers = set;
    asset.record = record_plan(&asset).declarations;

    let mut content = Content::new(kind, MANIFEST);
    content.geometry = asset.model.as_ref().map(|model| Geometry {
        file: geometry_file
            .clone()
            .unwrap_or_else(|| MANIFEST.to_string()),
        at: String::new(),
        model,
    });
    content.textures = asset
        .textures
        .iter()
        .map(|t| Texture {
            path: t.color_file(),
            width: t.width,
            height: t.height,
        })
        .collect();
    for (i, texture) in asset.media.textures.iter().enumerate() {
        let first = asset.media.textures[..i]
            .iter()
            .all(|t| t.name != texture.name);
        if first && texture.width > 0 {
            content.textures.push(Texture {
                path: texture.name.clone(),
                width: texture.width,
                height: texture.height,
            });
        }
    }
    content.record = Some(Record {
        path: MANIFEST.to_string(),
        declarations: asset.record,
    });
    if mode == Mode::Authoring {
        for (animation, file) in asset.animations.iter().zip(&sources) {
            content
                .expressions
                .extend(cost::expressions(animation, file));
        }
    }
    if !asset.springs.is_empty() {
        content.springs = Some(Springs {
            path: geometry_file.unwrap_or_else(|| MANIFEST.to_string()),
            bones: asset.springs.bones.len() as u32,
            chain_depth: u32::from(asset.springs.depth),
        });
    }
    cost::check(&content, &mut r);

    for capability in required_capabilities(&asset) {
        if !asset.requires.iter().any(|d| d == capability) {
            authoring(
                &mut r,
                Issue::error(IssueCode::Manifest, manifest_at("requires"))
                    .name("capability", capability),
            );
        }
    }
    finish(b, r, asset, mode)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeptDelays {
    pub at: usize,
    pub base: usize,
    pub reactions: usize,
    pub controllers: usize,
    pub anim_times: usize,
}

impl KeptDelays {
    pub fn len(&self) -> usize {
        self.base + self.reactions + self.controllers + self.anim_times
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordPlan {
    pub declarations: WearerDeclarations,
    pub reaction_floats: usize,
    pub timeline: Option<usize>,
    pub emote_floats: Option<usize>,
    pub emote_time: Option<usize>,
    pub place_floats: Option<usize>,
    pub place_time: Option<usize>,
    pub anim_clock: Option<usize>,
    pub anchor: Option<usize>,
    pub delays: KeptDelays,
}

pub fn reaction_span(reaction_floats: usize) -> usize {
    if reaction_floats > 0 {
        reaction_floats + KEPT_REACTION_FLOATS
    } else {
        0
    }
}

fn claim(count: &mut usize, size: usize) -> usize {
    let at = *count;
    *count += size;
    at
}

pub fn record_plan(a: &CosmeticAsset) -> RecordPlan {
    let reaction_floats = reactions::floats(&a.reactions);
    let fx = effects::declarations(&a.effects);
    let mut times = fx.effect_times;
    let timeline = has_timeline(&a.animations).then(|| claim(&mut times, 1));
    let emote = a.kind == Kind::Emote;
    let emote_time = emote.then(|| claim(&mut times, 1));
    let placed = a.kind == Kind::Pet && !a.roles.is_empty();
    let place_time = placed.then(|| claim(&mut times, 1));
    let anim_times = a.controllers.anim_time_floats();
    let anim_clock = a.controllers.needs_clock().then(|| claim(&mut times, 1));
    let anchor = keyed(&a.animations).then(|| claim(&mut times, 1));
    let mut floats = reaction_span(reaction_floats) + fx.effect_floats;
    let emote_floats = emote.then(|| claim(&mut floats, EMOTE_RECORD_FLOATS));
    let place_floats = placed.then(|| claim(&mut floats, PET_PLACE_FLOATS));
    let mut delays = KeptDelays {
        at: 0,
        base: if a.animations.is_empty() {
            0
        } else {
            KEPT_DELAY_FLOATS
        },
        reactions: reactions::kept_delay_floats(&a.reactions),
        controllers: a.controllers.kept_delay_floats(),
        anim_times,
    };
    delays.at = claim(&mut floats, delays.len());
    RecordPlan {
        declarations: WearerDeclarations {
            variables: a.interner.variable_count(),
            controllers: a.controllers.record_slots(),
            one_shots: reactions::one_shots(&a.reactions).min(MAX_ONE_SHOTS),
            spring_bones: a.springs.bones.len(),
            effect_times: times,
            effect_floats: floats,
            channels: channel_offsets(&a.animations).1,
            rings: fx.rings,
            ring_points: fx.ring_points,
            ring_turns: fx.ring_turns,
            ring_gates: fx.ring_gates,
            held_channels: held_channels(a),
        },
        reaction_floats,
        timeline,
        emote_floats,
        emote_time,
        place_floats,
        place_time,
        anim_clock,
        anchor,
        delays,
    }
}

pub fn keyed(animations: &[Animation]) -> bool {
    animations.iter().any(|a| !a.events.is_empty())
}

fn held_channels(a: &CosmeticAsset) -> usize {
    let binds = |model: &Model| -> Vec<Vec<Option<usize>>> {
        a.animations.iter().map(|anim| anim.bind(model)).collect()
    };
    let own = a.model.as_ref().map_or(0, |model| {
        pose::held_channels(&a.animations, &[(&binds(model), 0)]).len()
    });
    if a.kind != Kind::Emote || a.animations.is_empty() {
        return own;
    }
    let props = a.model.as_ref().and_then(prop_model);
    let prop_binds = props.as_ref().map_or_else(Vec::new, binds);
    [Arms::Wide, Arms::Slim]
        .into_iter()
        .map(|arms| {
            let rig = player::model(arms);
            let groups = [(&binds(&rig)[..], 0), (&prop_binds[..], rig.bones.len())];
            pose::held_channels(&a.animations, &groups).len()
        })
        .fold(own, usize::max)
}

pub fn required_capabilities(a: &CosmeticAsset) -> Vec<&'static str> {
    let e = &a.effects;
    let particles = e
        .emitters
        .iter()
        .any(|d| d.particle.is_some() || d.vanilla.is_some());
    let moments = e
        .named
        .iter()
        .any(|(name, _)| MOMENTS.contains(&name.as_str()));
    let rig =
        a.kind != Kind::Pet && (a.attach.rig != RigKind::Player || a.attach.replaces.is_some());
    let used = [
        !a.controllers.controllers().is_empty(),
        !a.springs.is_empty(),
        !e.is_empty(),
        particles,
        !e.trails.is_empty(),
        !e.sounds.is_empty(),
        !a.reactions.is_empty(),
        !a.palette.slots.is_empty() || a.textures.iter().any(|t| t.index.is_some()),
        a.palette.slots.iter().any(|s| s.customizable),
        rig,
        a.kind == Kind::Pet,
        !a.roles.is_empty(),
        moments,
    ];
    ENGINE_ORDER
        .iter()
        .zip(used)
        .filter_map(|(name, on)| on.then_some(*name))
        .collect()
}

pub fn analyze(a: &CosmeticAsset) -> Analysis {
    let live: f64 = effects::worst_case(&a.effects)
        .iter()
        .map(cost::live_particles)
        .sum();
    let texture_bytes: usize = a
        .textures
        .iter()
        .map(|t| t.rgba.len() + t.index.as_ref().map_or(0, Vec::len))
        .sum();
    Analysis {
        requires: required_capabilities(a)
            .into_iter()
            .map(str::to_string)
            .collect(),
        quads: a.model.as_ref().map_or(0, cost::quads),
        bones: a.model.as_ref().map_or(0, cost::bones),
        spring_bones: a.springs.bones.len() as u32,
        live_particles: live,
        trail_points: (a.effects.trails.len() * RING_POINTS) as u32,
        record_bytes: u32::try_from(cost::record_bytes(a.record)).unwrap_or(u32::MAX),
        texture_bytes: u32::try_from(texture_bytes).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use serde_json::json;

    use super::*;
    use crate::container::ENGINE_CAPABILITIES;

    macro_rules! sample_file {
        ($name:literal) => {
            (
                $name.to_string(),
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/data/sample_wings/",
                    $name
                ))
                .to_vec(),
            )
        };
    }

    fn sample() -> Vec<(String, Vec<u8>)> {
        vec![
            sample_file!("manifest.json"),
            sample_file!("models/wings.geo.json"),
            sample_file!("textures/wings.png"),
            sample_file!("textures/wings.idx.png"),
            sample_file!("palettes.json"),
            sample_file!("animations/wings.animation.json"),
            sample_file!("animation_controllers/wings.animation_controllers.json"),
        ]
    }

    fn with_manifest(
        files: &[(String, Vec<u8>)],
        edit: impl FnOnce(&mut Value),
    ) -> Vec<(String, Vec<u8>)> {
        let mut out = files.to_vec();
        let mut manifest: Value = serde_json::from_slice(&out[0].1).unwrap();
        edit(&mut manifest);
        out[0].1 = serde_json::to_vec(&manifest).unwrap();
        out
    }

    fn every_capability() -> Vec<&'static str> {
        CAPABILITIES
            .iter()
            .chain(ENGINE_CAPABILITIES)
            .copied()
            .collect()
    }

    fn load_files(files: &[(String, Vec<u8>)], mode: Mode) -> Loaded {
        let caps = every_capability();
        let options = LoadOptions {
            mode,
            identity: 42,
            capabilities: &caps,
        };
        load(Source::Files(files), &options)
    }

    fn asset(loaded: Loaded) -> (Arc<CosmeticAsset>, Vec<Issue>) {
        match loaded {
            Loaded::Asset { asset, issues } => (asset, issues),
            Loaded::Draft { issues, .. } => panic!("expected an asset, got a draft: {issues:?}"),
            Loaded::Invalid(issues) => panic!("expected an asset: {issues:?}"),
            Loaded::Skipped(skip) => panic!("expected an asset, skipped: {skip:?}"),
            Loaded::Legacy(_) => panic!("expected an asset, got legacy"),
        }
    }

    fn codes(issues: &[Issue]) -> Vec<(IssueCode, String)> {
        issues.iter().map(|i| (i.code, i.path.clone())).collect()
    }

    #[test]
    fn asset_loads_the_sample_item_with_every_part() {
        let (a, issues) = asset(load_files(&sample(), Mode::Authoring));
        assert_eq!(codes(&issues), []);
        assert_eq!(
            (a.id.as_str(), a.identity, a.kind),
            ("sample_wings", 42, Kind::Item)
        );
        let model = a.model.as_ref().unwrap();
        assert_eq!(model.identifier, "geometry.sample_wings");
        assert_eq!(model.bones.len(), 2);
        assert_eq!(a.textures.len(), 1);
        let index = a.textures[0].index.as_ref().unwrap();
        assert_eq!((index[0], index[16 * 4], index[15]), (1, 2, 0));
        assert_eq!(a.palette.slots[0].id, "feather");
        assert_eq!(a.palette.colorway("storm"), Some(0));
        let names: Vec<&str> = a.animations.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "animation.wings.idle",
                "animation.wings.droop",
                "animation.wings.flinch"
            ]
        );
        assert_eq!(
            a.controllers.controllers()[0].name,
            "controller.animation.wings.mood"
        );
        assert_eq!(a.reactions.len(), 2);
        assert_eq!(
            (a.reactions[0].play, a.reactions[0].palette),
            (Some(1), Some(0))
        );
        assert_eq!(a.reactions[1].play, Some(2));
        assert_eq!(a.attach.rig, RigKind::Player);
        assert_eq!(a.attach.bone.as_deref(), Some("body"));
        assert_eq!(
            a.record,
            WearerDeclarations {
                controllers: 2,
                one_shots: 1,
                effect_floats: reactions::floats(&a.reactions)
                    + KEPT_REACTION_FLOATS
                    + KEPT_DELAY_FLOATS
                    + reactions::kept_delay_floats(&a.reactions)
                    + a.controllers.kept_delay_floats(),
                channels: channel_offsets(&a.animations).1,
                held_channels: 1,
                ..WearerDeclarations::default()
            }
        );
        assert_eq!(
            required_capabilities(&a),
            ["controllers", "reactions", "palettes", "custom_colors"]
        );
        let (client, issues) = asset(load_files(&sample(), Mode::Client));
        assert!(issues.is_empty());
        assert_eq!(client.reactions, a.reactions);
    }

    #[test]
    fn asset_authoring_rejects_unknown_keys_and_silent_fallbacks() {
        let files = with_manifest(&sample(), |m| {
            m["colourways"] = json!({});
            m["reactions"][0]["play"] = json!("animation.wings.sulk");
            m["textures"][0]["palete"] = json!(true);
        });
        let Loaded::Draft {
            asset: draft,
            issues,
        } = load_files(&files, Mode::Authoring)
        else {
            panic!("expected a draft");
        };
        assert_eq!(
            codes(&issues),
            [
                (
                    IssueCode::UnknownKey,
                    "manifest.json#colourways".to_string()
                ),
                (
                    IssueCode::UnknownKey,
                    "manifest.json#textures.0.palete".to_string()
                ),
                (
                    IssueCode::ReactionMissing,
                    "manifest.json#reactions.0.play".to_string()
                ),
            ]
        );
        assert_eq!(draft.reactions.len(), 2);
        let (client, issues) = asset(load_files(&files, Mode::Client));
        assert!(issues.is_empty());
        assert_eq!(client.reactions[0].play, None);
        assert_eq!(client.reactions[0].palette, Some(0), "the colourway stays");
    }

    #[test]
    fn asset_converts_old_pets() {
        let files = sample();
        let old = vec![
            (
                "pet.json".to_string(),
                br#"{"archetype":"flying","states":{"idle":"animation.wings.idle"}}"#.to_vec(),
            ),
            ("wings.geo.json".to_string(), files[1].1.clone()),
            ("wings.png".to_string(), files[2].1.clone()),
            ("wings.animation.json".to_string(), files[5].1.clone()),
        ];
        let (a, issues) = asset(load_files(&old, Mode::Client));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(a.kind, Kind::Pet);
        assert_eq!(a.attach.rig, RigKind::Actor);
        assert_eq!(a.pet.as_ref().unwrap().archetype, pet::Archetype::Flying);
        assert!(a.model.is_some());
        assert_eq!(a.textures.len(), 1);
        assert_eq!(a.animations.len(), 3);
        assert_eq!(a.controllers.controllers().len(), 1);
        assert!(required_capabilities(&a).contains(&"pets"));
    }

    #[test]
    fn a_legacy_emote_with_a_huge_expression_is_invalid_instead_of_crashing() {
        let mut chain = String::from("1");
        for _ in 1..120_000 {
            chain.push_str("+1");
        }
        let json = format!(
            r#"{{"animations":{{"wave":{{"animation_length":1,"bones":{{"rightArm":{{"rotation":["{chain}",0,0]}}}}}}}}}}"#
        );
        let files = vec![("wave.animation.json".to_string(), json.into_bytes())];
        let loaded = thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || load(Source::Files(&files), &LoadOptions::new(Mode::Client, 1)))
            .unwrap()
            .join()
            .unwrap();
        let Loaded::Invalid(issues) = loaded else {
            panic!("expected the emote to be rejected");
        };
        assert!(issues
            .iter()
            .any(|i| i.code == IssueCode::AnimationsInvalid));
    }
}
