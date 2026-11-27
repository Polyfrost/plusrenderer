mod elements;
mod report;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::asset::{CosmeticAsset, Kind};
use crate::attach::{rig, rig_bone, RigKind};
use crate::palette::custom_tones;
use crate::texture::{resolve_texture, IndexedTexture};

pub use elements::{euler_angles, euler_matrix, fit_rotation, turn_matrix, Fit, Mat3, Turn};
pub use report::{ConversionReport, Loss, LossKind, Severity};

pub const MODEL_MIN: f32 = -16.0;
pub const MODEL_MAX: f32 = 32.0;
pub const MODEL_SPAN: f32 = MODEL_MAX - MODEL_MIN;
pub const MODEL_CENTER: [f32; 3] = [8.0, 8.0, 8.0];
pub const ANGLE_STEPS: [f32; 5] = [-45.0, -22.5, 0.0, 22.5, 45.0];
pub const ANGLE_LIMIT: f32 = 45.0;
pub const HEAD_CENTER: [f32; 3] = [0.0, 28.0, 0.0];
pub const HEAD_LAYER_SCALE: f32 = 0.625;
pub const DISPLAY_SCALE_LIMIT: f32 = 4.0;
pub const DISPLAY_TRANSLATION_LIMIT: f32 = 80.0;
pub const LEGACY_BASE_ITEM: &str = "paper";
pub const TEXTURE_KEY: &str = "0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotations {
    Steps,
    Clamped,
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Definition {
    Overrides,
    Items,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub rotations: Rotations,
    pub definition: Definition,
    pub light_emission: bool,
}

impl Target {
    pub const OVERRIDES_UNLIT: Target = Target {
        rotations: Rotations::Steps,
        definition: Definition::Overrides,
        light_emission: false,
    };
    pub const OVERRIDES: Target = Target {
        rotations: Rotations::Steps,
        definition: Definition::Overrides,
        light_emission: true,
    };
    pub const ITEMS_STEPPED: Target = Target {
        rotations: Rotations::Steps,
        definition: Definition::Items,
        light_emission: true,
    };
    pub const ITEMS_CLAMPED: Target = Target {
        rotations: Rotations::Clamped,
        definition: Definition::Items,
        light_emission: true,
    };
    pub const ITEMS_FREE: Target = Target {
        rotations: Rotations::Free,
        definition: Definition::Items,
        light_emission: true,
    };

    pub fn for_version(version: &str) -> Option<Target> {
        VERSIONS
            .iter()
            .find(|(v, _)| *v == version)
            .map(|(_, t)| *t)
    }
}

pub const VERSIONS: [(&str, Target); 11] = [
    ("1.21.1", Target::OVERRIDES_UNLIT),
    ("1.21.3", Target::OVERRIDES),
    ("1.21.4", Target::ITEMS_STEPPED),
    ("1.21.5", Target::ITEMS_STEPPED),
    ("1.21.6", Target::ITEMS_CLAMPED),
    ("1.21.8", Target::ITEMS_CLAMPED),
    ("1.21.10", Target::ITEMS_CLAMPED),
    ("1.21.11", Target::ITEMS_FREE),
    ("26.1", Target::ITEMS_FREE),
    ("26.2", Target::ITEMS_FREE),
    ("26.3", Target::ITEMS_FREE),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mount {
    #[default]
    Display,
    Head,
}

#[derive(Clone, Copy, Debug)]
pub struct ExportOptions<'a> {
    pub namespace: &'a str,
    pub target: Target,
    pub mount: Mount,
    pub colorway: Option<usize>,
    pub custom: &'a [(u8, u32)],
}

impl<'a> ExportOptions<'a> {
    pub fn new(namespace: &'a str, target: Target) -> ExportOptions<'a> {
        ExportOptions {
            namespace,
            target,
            mount: Mount::Display,
            colorway: None,
            custom: &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportError {
    Namespace,
    NoGeometry,
    NoTexture,
    Colorway,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    pub name: String,
    pub model_id: String,
    pub model: Value,
    pub definition: Option<Value>,
    pub texture: Vec<u8>,
    pub files: Vec<(String, Vec<u8>)>,
    pub anchor: [f32; 3],
    pub scale: f32,
    pub report: ConversionReport,
}

pub fn is_namespace(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        })
}

pub fn model_path(namespace: &str, name: &str) -> String {
    format!("assets/{namespace}/models/item/{name}.json")
}

pub fn texture_path(namespace: &str, name: &str) -> String {
    format!("assets/{namespace}/textures/item/{name}.png")
}

pub fn definition_path(namespace: &str, name: &str) -> String {
    format!("assets/{namespace}/items/{name}.json")
}

pub fn legacy_base_path() -> String {
    format!("assets/minecraft/models/item/{LEGACY_BASE_ITEM}.json")
}

pub fn anchor_point(asset: &CosmeticAsset, mount: Mount) -> [f32; 3] {
    match mount {
        Mount::Head => HEAD_CENTER,
        Mount::Display => {
            let at = asset
                .attach
                .bone
                .as_deref()
                .and_then(|b| rig_bone(asset.attach.rig, b))
                .unwrap_or(0);
            rig(asset.attach.rig).get(at).map_or([0.0; 3], |b| b.pivot)
        }
    }
}

pub fn export(asset: &CosmeticAsset, o: &ExportOptions<'_>) -> Result<Export, ExportError> {
    if !is_namespace(o.namespace) {
        return Err(ExportError::Namespace);
    }
    let Some(model) = asset.model.as_ref() else {
        return Err(ExportError::NoGeometry);
    };
    let Some(texture) = asset.textures.first() else {
        return Err(ExportError::NoTexture);
    };
    if o.colorway
        .is_some_and(|c| c >= asset.palette.colorways.len())
    {
        return Err(ExportError::Colorway);
    }

    let mut report = ConversionReport::default();
    asset_losses(asset, o, &mut report);

    let anchor = anchor_point(asset, o.mount);
    let built = elements::build(model, anchor, o.target, &mut report);
    if built.elements.is_empty() {
        return Err(ExportError::NoGeometry);
    }

    let name = asset.id.clone();
    let model_id = format!("{}:item/{name}", o.namespace);
    let mut root = Map::new();
    root.insert(
        "textures".into(),
        json!({ TEXTURE_KEY: model_id, "particle": model_id }),
    );
    root.insert("elements".into(), Value::Array(built.elements));
    if o.mount == Mount::Head {
        root.insert("display".into(), head_display(&built.fit, &mut report));
    }
    let model_json = Value::Object(root);

    let rgba = texture_rgba(asset, texture, o);
    let png = encode_png(texture.width, texture.height, &rgba);

    let definition = match o.target.definition {
        Definition::Items => {
            Some(json!({ "model": { "type": "minecraft:model", "model": model_id } }))
        }
        Definition::Overrides => None,
    };

    let mut files = vec![
        (model_path(o.namespace, &name), pretty(&model_json)),
        (texture_path(o.namespace, &name), png.clone()),
    ];
    if let Some(d) = &definition {
        files.push((definition_path(o.namespace, &name), pretty(d)));
    }

    Ok(Export {
        name,
        model_id,
        model: model_json,
        definition,
        texture: png,
        files,
        anchor: built.fit.anchor(),
        scale: 1.0 / built.fit.scale,
        report,
    })
}

pub fn legacy_overrides(entries: &[(i32, &str)]) -> (String, Vec<u8>) {
    let mut sorted: Vec<(i32, &str)> = entries.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    sorted.dedup_by_key(|e| e.0);
    let overrides: Vec<Value> = sorted
        .iter()
        .map(|(cmd, model)| json!({ "predicate": { "custom_model_data": cmd }, "model": model }))
        .collect();
    let value = json!({
        "parent": "minecraft:item/generated",
        "textures": { "layer0": format!("minecraft:item/{LEGACY_BASE_ITEM}") },
        "overrides": overrides,
    });
    (legacy_base_path(), pretty(&value))
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut out = serde_json::to_vec_pretty(value).unwrap_or_default();
    out.push(b'\n');
    out
}

fn texture_rgba(asset: &CosmeticAsset, texture: &IndexedTexture, o: &ExportOptions<'_>) -> Vec<u8> {
    if texture.index.is_none() {
        return texture.rgba.clone();
    }
    let tones = custom_tones(&asset.palette, o.colorway, o.custom);
    resolve_texture(texture, &asset.palette.resolve(None), &tones)
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let written = encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgba));
    if written.is_err() {
        return Vec::new();
    }
    out
}

fn head_display(fit: &elements::Fit, report: &mut ConversionReport) -> Value {
    let wanted = 1.0 / (HEAD_LAYER_SCALE * fit.scale);
    let scale = wanted.min(DISPLAY_SCALE_LIMIT);
    if wanted > DISPLAY_SCALE_LIMIT {
        report.push(
            LossKind::Bounds,
            Severity::Approximated,
            "display.head",
            format!(
                "needs scale {} but the limit is {DISPLAY_SCALE_LIMIT}",
                elements::tidy(wanted)
            ),
        );
    }
    let mut translation = [0.0f32; 3];
    let mut clamped = false;
    for (axis, t) in translation.iter_mut().enumerate() {
        let wanted = (fit.center[axis] - MODEL_CENTER[axis]) / HEAD_LAYER_SCALE;
        *t = wanted.clamp(-DISPLAY_TRANSLATION_LIMIT, DISPLAY_TRANSLATION_LIMIT);
        clamped |= *t != wanted;
    }
    if clamped {
        report.push(
            LossKind::Bounds,
            Severity::Approximated,
            "display.head",
            format!("translation clamped to {DISPLAY_TRANSLATION_LIMIT}"),
        );
    }
    let mut head = Map::new();
    if translation != [0.0; 3] {
        head.insert("translation".into(), elements::vec3(translation));
    }
    head.insert("scale".into(), elements::vec3([scale; 3]));
    json!({ "head": head })
}

fn asset_losses(asset: &CosmeticAsset, o: &ExportOptions<'_>, r: &mut ConversionReport) {
    let manifest = "manifest.json";
    if asset.kind != Kind::Item {
        r.push(
            LossKind::Kind,
            Severity::Dropped,
            manifest,
            format!("{} behaviour is not exported", asset.kind.as_str()),
        );
    }
    if !asset.animations.is_empty() {
        r.push(
            LossKind::Animations,
            Severity::Dropped,
            "animations",
            format!(
                "{} animations; the rest pose is exported",
                asset.animations.len()
            ),
        );
    }
    let controllers = asset.controllers.controllers().len();
    if controllers > 0 {
        r.push(
            LossKind::Controllers,
            Severity::Dropped,
            "animation_controllers",
            format!("{controllers} controllers"),
        );
    }
    if !asset.reactions.is_empty() {
        r.push(
            LossKind::Reactions,
            Severity::Dropped,
            manifest,
            format!("{} reactions", asset.reactions.len()),
        );
    }
    let e = &asset.effects;
    let effects = e.particles.len() + e.sounds.len() + e.emitters.len() + e.trails.len();
    if effects > 0 {
        r.push(
            LossKind::Effects,
            Severity::Dropped,
            manifest,
            format!(
                "{} particles, {} sounds, {} emitters, {} trails",
                e.particles.len(),
                e.sounds.len(),
                e.emitters.len(),
                e.trails.len()
            ),
        );
    }
    if !asset.springs.bones.is_empty() {
        r.push(
            LossKind::Springs,
            Severity::Dropped,
            manifest,
            format!("{} spring bones stay at rest", asset.springs.bones.len()),
        );
    }
    let colorways = asset.palette.colorways.len();
    if colorways > 0 {
        let chosen = o
            .colorway
            .and_then(|c| asset.palette.colorways.get(c))
            .map_or("default", |c| c.id.as_str());
        r.push(
            LossKind::Colorways,
            Severity::Dropped,
            "palettes.json",
            format!("{colorways} colorways besides the default; exported {chosen}"),
        );
    }
    let customizable: Vec<&str> = asset
        .palette
        .slots
        .iter()
        .filter(|s| s.customizable)
        .map(|s| s.id.as_str())
        .collect();
    if !customizable.is_empty() {
        r.push(
            LossKind::CustomColors,
            Severity::Dropped,
            "palettes.json",
            format!(
                "{} customizable; one fixed set of colors per model",
                customizable.join(", ")
            ),
        );
    }
    if asset.textures.len() > 1 {
        r.push(
            LossKind::Textures,
            Severity::Dropped,
            "textures",
            format!("{} textures; only the first is used", asset.textures.len()),
        );
    }
    if o.mount == Mount::Head {
        let on_head = asset.attach.rig == RigKind::Player
            && matches!(asset.attach.bone.as_deref(), Some("head" | "hat"));
        if !on_head {
            let bone = asset.attach.bone.as_deref().unwrap_or("root");
            r.push(
                LossKind::Attach,
                Severity::Approximated,
                manifest,
                format!("attached to {bone} but shown on the head"),
            );
        }
    }
}

#[cfg(test)]
mod tests;
