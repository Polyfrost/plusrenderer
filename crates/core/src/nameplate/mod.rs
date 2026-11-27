pub mod abi;
pub mod anim;
pub mod cache;
pub mod color;
pub mod constants;
pub mod flag;
pub mod format;
pub mod frame;
pub mod geom;
pub mod image;
pub mod layout;
pub mod material;
pub mod paint;
pub mod palette;
pub mod queries;
pub mod raster;
pub mod route;
pub mod schema;
pub mod text;
pub mod types;
pub mod validate;

use std::fmt;
use std::sync::Arc;

use serde::Serialize;

use crate::container::{
    open_source, Kind, Opened, Options as ContainerOptions, CAPABILITIES, MANIFEST,
};
pub use crate::container::{Skip, Source};
pub use crate::cost::Motion;
use crate::issues::{path, Issue, IssueCode, Level, Mode};
use crate::limits::{NAMEPLATE_CAPS, NAMEPLATE_FLASH_FRAMES};
pub use format::{check_molang, MolangCheck};
pub use frame::{FallbackImage, FallbackJob, NameplateRenderer};
use material::ir::{Dialect, Lane, ShaderSource};
use types::LayerContent;
pub use types::NameplateAsset;

#[derive(Clone, Copy, Debug)]
pub struct LoadOptions<'a> {
    pub mode: Mode,
    pub capabilities: &'a [&'a str],
    pub identity: Option<u64>,
    pub dialect: Option<Dialect>,
}

impl LoadOptions<'static> {
    pub fn new(mode: Mode) -> LoadOptions<'static> {
        LoadOptions {
            mode,
            capabilities: CAPABILITIES,
            identity: None,
            dialect: None,
        }
    }
}

impl Default for LoadOptions<'static> {
    fn default() -> LoadOptions<'static> {
        LoadOptions::new(Mode::Client)
    }
}

#[derive(Debug)]
pub enum Loaded {
    Asset {
        asset: Arc<NameplateAsset>,
        issues: Vec<Issue>,
    },
    Draft {
        asset: Arc<NameplateAsset>,
        issues: Vec<Issue>,
    },
    Skipped(Skip),
    Invalid(Vec<Issue>),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LayerAnalysis {
    pub id: String,
    pub states: u32,
    pub image_bytes: u32,
    pub quads: u32,
    pub cpu_units: u32,
    pub margin: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EffectAnalysis {
    pub layer: String,
    pub states: u32,
    pub shot_states: u32,
    pub tone_slots: u32,
    pub reads: Vec<String>,
    pub build_ops: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Analysis {
    pub requires: Vec<String>,
    pub quads: u32,
    pub cpu_units: u32,
    pub image_bytes: u32,
    pub fallback_bytes: u32,
    pub record_bytes: u32,
    pub max_out_words: u32,
    pub material_build_ops: u64,
    pub layers: Vec<LayerAnalysis>,
    pub effects: Vec<EffectAnalysis>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveError {
    NoLayer,
    NotAnEffect,
    NotRead,
    NoVariant,
    TooLong,
}

impl fmt::Display for CurveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurveError::NoLayer => write!(f, "nameplate: there is no layer with that id"),
            CurveError::NotAnEffect => write!(f, "nameplate: that layer is not an effect"),
            CurveError::NotRead => write!(f, "nameplate: that effect does not read the setting"),
            CurveError::NoVariant => write!(f, "nameplate: there is no variant with that index"),
            CurveError::TooLong => write!(
                f,
                "nameplate: a curve holds at most {NAMEPLATE_FLASH_FRAMES} frames"
            ),
        }
    }
}

impl std::error::Error for CurveError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    TagRows { ints: usize },
    Wearers { tags: usize, wearers: usize },
    Signals { expected: usize, got: usize },
    OutTooSmall { needed: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TagRows { ints } => {
                write!(
                    f,
                    "nameplate: {ints} tag ints is not a whole number of rows"
                )
            }
            FrameError::Wearers { tags, wearers } => {
                write!(f, "nameplate: {tags} tags but {wearers} wearers")
            }
            FrameError::Signals { expected, got } => {
                write!(f, "nameplate: expected {expected} signal floats, got {got}")
            }
            FrameError::OutTooSmall { needed } => {
                write!(f, "nameplate: the output needs at least {needed} words")
            }
        }
    }
}

impl std::error::Error for FrameError {}

pub fn load(src: Source<'_>, o: &LoadOptions) -> Loaded {
    let options = ContainerOptions {
        mode: o.mode,
        legacy: NAMEPLATE_CAPS,
        capabilities: o.capabilities,
    };
    let mut bundle = match open_source(src, &options) {
        Opened::Bundle(bundle) => bundle,
        Opened::Skipped(skip) => return Loaded::Skipped(skip),
        Opened::Invalid(issues) => return Loaded::Invalid(issues),
        Opened::Legacy(_) => {
            let issue = Issue::error(IssueCode::MissingFile, MANIFEST).name("file", MANIFEST);
            return Loaded::Invalid(vec![issue]);
        }
    };
    if bundle.kind() != Kind::Nameplate {
        let issue = Issue::error(IssueCode::KindUnknown, path(MANIFEST, "kind"))
            .name("kind", bundle.kind().as_str());
        bundle.report().push(issue);
        return Loaded::Invalid(bundle.finish());
    }
    let Some(asset) = format::compile(&mut bundle, o) else {
        return Loaded::Invalid(bundle.finish());
    };
    validate::validate(&asset, bundle.report());
    if let Some(dialect) = o.dialect {
        for issue in validate::dialect_issues(&asset, dialect) {
            bundle.report().push(issue);
        }
    }
    let issues = bundle.finish();
    let asset = Arc::new(asset);
    let errors = issues.iter().any(|i| i.level == Level::Error);
    match (errors, o.mode) {
        (false, _) => Loaded::Asset { asset, issues },
        (true, Mode::Authoring) => Loaded::Draft { asset, issues },
        (true, Mode::Client) => Loaded::Invalid(issues),
    }
}

pub fn shaders(a: &NameplateAsset, d: Dialect) -> Vec<ShaderSource> {
    a.materials
        .iter()
        .enumerate()
        .map(|(i, ir)| ShaderSource {
            material: i as u16,
            ..material::glsl::generate(ir, d)
        })
        .collect()
}

pub fn analyze(a: &NameplateAsset) -> Analysis {
    validate::analysis(a)
}

pub fn param_curve(
    a: &NameplateAsset,
    layer: &str,
    param: &str,
    variant: u16,
    count: u32,
) -> Result<Vec<f64>, CurveError> {
    let spec = a
        .layers
        .iter()
        .find(|l| l.id == layer)
        .ok_or(CurveError::NoLayer)?;
    let LayerContent::Material(content) = &spec.content else {
        return Err(CurveError::NotAnEffect);
    };
    let ir = a
        .materials
        .get(usize::from(content.material))
        .ok_or(CurveError::NotAnEffect)?;
    let index = a
        .params
        .iter()
        .position(|p| p.id == param)
        .filter(|i| {
            ir.lanes
                .iter()
                .any(|lane| matches!(lane, Lane::Param(p) if usize::from(*p) == *i))
        })
        .ok_or(CurveError::NotRead)?;
    let variant = a
        .variants
        .get(usize::from(variant))
        .ok_or(CurveError::NoVariant)?;
    if count > NAMEPLATE_FLASH_FRAMES {
        return Err(CurveError::TooLong);
    }
    let value = variant
        .params
        .get(index)
        .copied()
        .unwrap_or(a.params[index].value);
    Ok(vec![value; count as usize])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::format::tests::Kit;
    use super::*;

    fn plate(script: &str) -> Vec<(String, Vec<u8>)> {
        let manifest = json!({ "format": 1, "kind": "nameplate", "id": "plate", "fps": 6, "loop": 12, "text": { "color": "ink" }, "layers": [] });
        let mut kit = Kit::new(
            manifest,
            json!({ "slots": [{ "id": "ink", "ramp": ["#FFFFFF"] }] }),
        );
        kit.edit(script);
        kit.bytes()
    }

    #[test]
    fn loads_report_the_chosen_dialect_s_unsupported_nodes() {
        let files = plate("materials/fuzz.json {'resolution': 'pixel', 'nodes': [{'id': 'p', 'type': 'PixelPos'}, {'id': 'n', 'type': 'Noise', 'in': {'p': 'p'}}, \
            {'id': 'h', 'type': 'Hsv', 'in': {'h': 'n'}}, {'id': 'out', 'type': 'Output', 'in': {'color': 'h'}}]} & +fuzz:behind {'kind': 'material', 'material': 'fuzz'}");
        let unsupported = |dialect: Option<Dialect>| {
            let options = LoadOptions {
                dialect,
                ..LoadOptions::new(Mode::Authoring)
            };
            let (Loaded::Asset { issues, .. } | Loaded::Draft { issues, .. }) =
                load(Source::Files(&files), &options)
            else {
                panic!("loaded")
            };
            issues
                .iter()
                .filter(|i| i.code == IssueCode::DialectUnsupported)
                .count()
        };
        assert_eq!(unsupported(Some(Dialect::Sksl)), 1);
        assert_eq!(unsupported(Some(Dialect::Core330)), 0);
    }

    #[test]
    fn setting_curves_hold_each_variant_s_value_on_every_frame() {
        let files = plate("manifest.json/params {'speed': {'value': 4, 'min': 1, 'max': 8}} & manifest.json/variants {'slow': {}, 'fast': {'params': {'speed': 7}}} \
            & materials/wave.json {'resolution': 'pixel', 'nodes': [{'id': 'f', 'type': 'Frame'}, {'id': 'p', 'type': 'Param', 'set': {'name': 'speed'}}, \
            {'id': 'm', 'type': 'mod', 'in': {'a': 'f', 'b': 'p'}}, {'id': 'h', 'type': 'Hsv', 'in': {'h': 'm'}}, {'id': 'out', 'type': 'Output', 'in': {'color': 'h'}}]} \
            & +wave:behind {'kind': 'material', 'material': 'wave'} & +body:background {'kind': 'body', 'fill': {'color': 'ink'}}");
        let Loaded::Asset { asset, .. } =
            load(Source::Files(&files), &LoadOptions::new(Mode::Authoring))
        else {
            panic!("an asset")
        };
        let variant = |key: &str| asset.variants.iter().position(|v| v.key == key).unwrap() as u16;
        let (slow, fast) = (variant("slow"), variant("fast"));
        assert_eq!(
            param_curve(&asset, "wave", "speed", slow, 4),
            Ok(vec![4.0; 4])
        );
        assert_eq!(
            param_curve(&asset, "wave", "speed", fast, 2),
            Ok(vec![7.0; 2])
        );
        assert_eq!(param_curve(&asset, "wave", "speed", slow, 0), Ok(vec![]));
        let curve =
            |layer, param, variant, count| param_curve(&asset, layer, param, variant, count).err();
        assert_eq!(curve("nope", "speed", slow, 1), Some(CurveError::NoLayer));
        assert_eq!(
            curve("body", "speed", slow, 1),
            Some(CurveError::NotAnEffect)
        );
        assert_eq!(curve("wave", "size", slow, 1), Some(CurveError::NotRead));
        assert_eq!(curve("wave", "speed", 9, 1), Some(CurveError::NoVariant));
        assert_eq!(
            curve("wave", "speed", slow, 4801),
            Some(CurveError::TooLong)
        );
    }
}
