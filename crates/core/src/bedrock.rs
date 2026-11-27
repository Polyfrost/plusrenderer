//! Bedrock `.geo.json` (1.12+ and legacy 1.8.0) -> [`Model`], with box UV and `mirror` resolved.
//! File space becomes IR space here: Z is negated (see `to_ir`), so the front lands on +Z.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::ir::{Bone, Cube, Face, FaceUv, Locator, Model, SpringParams};
use crate::issues::{join, path, Issue, IssueCode, Mode, Report};

#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    NoGeometry,
    CyclicBones(Vec<String>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Json(e) => write!(f, "malformed geometry json: {e}"),
            Error::NoGeometry => write!(f, "no geometry found in file"),
            Error::CyclicBones(n) => write!(f, "cyclic or self-parented bones: {}", n.join(", ")),
        }
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fault {
    pub at: String,
    pub kind: FaultKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaultKind {
    SpringNotObject,
    SpringUnknownKey,
    SpringWrongType,
    SpringOutOfRange { value: f64 },
    LocatorInvalid,
    LocatorDuplicate,
}

#[derive(Debug)]
pub struct Parsed {
    pub models: Vec<Model>,
    pub faults: Vec<Fault>,
}

pub fn report_faults(file: &str, faults: &[Fault], report: &mut Report) {
    for fault in faults {
        let at = path(file, &fault.at);
        let key = fault.at.rsplit('.').next().unwrap_or_default();
        let issue = match fault.kind {
            FaultKind::SpringNotObject | FaultKind::SpringWrongType => {
                Issue::error(IssueCode::SpringInvalid, at)
            }
            FaultKind::SpringOutOfRange { value } => {
                Issue::warning(IssueCode::SpringSettingRange, at).value("value", value)
            }
            FaultKind::SpringUnknownKey if report.mode() == Mode::Authoring => {
                Issue::error(IssueCode::UnknownKey, at).name("key", key)
            }
            FaultKind::SpringUnknownKey => continue,
            FaultKind::LocatorInvalid => {
                Issue::error(IssueCode::GeometryInvalid, at).name("locator", key)
            }
            FaultKind::LocatorDuplicate => {
                Issue::warning(IssueCode::GeometryInvalid, at).name("locator", key)
            }
        };
        report.push(issue);
    }
}

pub fn parse(json: &str) -> Result<Vec<Model>, Error> {
    parse_checked(json).map(|p| p.models)
}

pub fn parse_checked(json: &str) -> Result<Parsed, Error> {
    let mut root: serde_json::Value = serde_json::from_str(json)?;
    let mut faults = Vec::new();

    if let Some(list) = root.get_mut("minecraft:geometry") {
        let raw: Vec<RawGeometry> = serde_json::from_value(list.take())?;
        let models = raw
            .into_iter()
            .map(|g| build(g, &mut faults))
            .collect::<Result<_, _>>()?;
        return Ok(Parsed { models, faults });
    }

    // 1.8.0: each `geometry.<name>` key is one model.
    let obj = root.as_object_mut().ok_or(Error::NoGeometry)?;
    let mut out = Vec::new();

    for (key, value) in obj {
        if !key.starts_with("geometry.") {
            continue;
        }
        let mut raw: RawGeometry = serde_json::from_value(value.take())?;
        raw.description.identifier = key.clone();
        out.push(build(raw, &mut faults)?);
    }

    if out.is_empty() {
        return Err(Error::NoGeometry);
    }
    Ok(Parsed {
        models: out,
        faults,
    })
}

pub fn parse_one(json: &str) -> Result<Model, Error> {
    parse(json)?.into_iter().next().ok_or(Error::NoGeometry)
}

fn build(raw: RawGeometry, faults: &mut Vec<Fault>) -> Result<Model, Error> {
    let texture_size = [
        raw.texturewidth.unwrap_or(raw.description.texture_width),
        raw.textureheight.unwrap_or(raw.description.texture_height),
    ];

    let model_at = raw.description.identifier.clone();
    let mut bones = topo_sort(raw.bones, |b| {
        let at = join(&join(&model_at, "bones"), &b.name);
        (
            locators(b.locators.as_ref(), &at, faults),
            b.spring.as_ref().and_then(|v| spring(v, &at, faults)),
        )
    })?;
    flag_duplicate_locators(&mut bones, &model_at, faults);

    Ok(Model {
        identifier: raw.description.identifier,
        texture_size,
        bones,
    })
}

/// Parents first. A bone whose parent is outside the file becomes a root but keeps `parent_name`.
fn topo_sort(
    raw: Vec<RawBone>,
    mut extras: impl FnMut(&RawBone) -> (Vec<Locator>, Option<SpringParams>),
) -> Result<Vec<Bone>, Error> {
    let index: HashMap<&str, usize> = raw
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.as_str(), i))
        .collect();

    let mut placed: Vec<Option<usize>> = vec![None; raw.len()];
    let mut order: Vec<usize> = Vec::with_capacity(raw.len());

    // ponytail: O(n^2). Switch to Kahn's algorithm if a model ever has thousands of bones.
    loop {
        let before = order.len();

        for (i, bone) in raw.iter().enumerate() {
            if placed[i].is_some() {
                continue;
            }
            let ready = match bone.parent.as_deref().and_then(|p| index.get(p)) {
                Some(&p) => placed[p].is_some(),
                None => true, // root, or parented into another model
            };
            if ready {
                placed[i] = Some(order.len());
                order.push(i);
            }
        }

        if order.len() == raw.len() {
            break;
        }
        if order.len() == before {
            let stuck = raw
                .iter()
                .enumerate()
                .filter(|(i, _)| placed[*i].is_none())
                .map(|(_, b)| b.name.clone())
                .collect();
            return Err(Error::CyclicBones(stuck));
        }
    }

    Ok(order
        .iter()
        .map(|&i| {
            let b = &raw[i];
            let parent = b
                .parent
                .as_deref()
                .and_then(|p| index.get(p))
                .and_then(|&p| placed[p]);
            let (locators, spring) = extras(b);

            Bone {
                name: b.name.clone(),
                parent,
                parent_name: if parent.is_none() {
                    b.parent.clone()
                } else {
                    None
                },
                pivot: to_ir(b.pivot),
                rotation: b.rotation,
                cubes: b.cubes.iter().map(|c| cube(c, b.mirror)).collect(),
                glow: None,
                locators,
                spring,
            }
        })
        .collect())
}

fn locators(value: Option<&Value>, at: &str, faults: &mut Vec<Fault>) -> Vec<Locator> {
    let Some(Value::Object(map)) = value else {
        return Vec::new();
    };
    let at = join(at, "locators");
    map.iter()
        .filter_map(|(name, v)| {
            let found = match v {
                Value::Object(o) => o.get("offset").and_then(triple).map(|offset| {
                    let rotation = o.get("rotation").and_then(triple).unwrap_or([0.0; 3]);
                    (offset, rotation)
                }),
                other => triple(other).map(|offset| (offset, [0.0; 3])),
            };
            if found.is_none() {
                faults.push(Fault {
                    at: join(&at, name),
                    kind: FaultKind::LocatorInvalid,
                });
            }
            let (offset, rotation) = found?;
            Some(Locator {
                name: name.clone(),
                offset: to_ir(offset),
                rotation,
            })
        })
        .collect()
}

fn triple(value: &Value) -> Option<[f32; 3]> {
    let items = value.as_array().filter(|a| a.len() == 3)?;
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64()? as f32;
    }
    Some(out)
}

fn flag_duplicate_locators(bones: &mut [Bone], model_at: &str, faults: &mut Vec<Fault>) {
    let mut seen: Vec<String> = Vec::new();
    for bone in bones.iter() {
        for locator in &bone.locators {
            if seen.contains(&locator.name) {
                faults.push(Fault {
                    at: join(
                        &join(&join(&join(model_at, "bones"), &bone.name), "locators"),
                        &locator.name,
                    ),
                    kind: FaultKind::LocatorDuplicate,
                });
            } else {
                seen.push(locator.name.clone());
            }
        }
    }
}

const SPRING_RANGES: [(&str, f64, f64); 5] = [
    ("stiffness", 0.0, 1.0),
    ("damping", 0.0, 1.0),
    ("gravity", -1.0, 1.0),
    ("drag", 0.0, 1.0),
    ("limit", 0.0, 180.0),
];

fn spring(value: &Value, at: &str, faults: &mut Vec<Fault>) -> Option<SpringParams> {
    let at = join(at, "polyplus:spring");
    let map = match value {
        Value::Null | Value::Bool(false) => return None,
        Value::Object(map) => map,
        _ => {
            faults.push(Fault {
                at,
                kind: FaultKind::SpringNotObject,
            });
            return None;
        }
    };

    let mut params = SpringParams::default();
    for (key, v) in map {
        let mut fault = |kind| {
            faults.push(Fault {
                at: join(&at, key),
                kind,
            })
        };
        if key == "collide" {
            match v.as_bool() {
                Some(b) => params.collide = b,
                None => fault(FaultKind::SpringWrongType),
            }
            continue;
        }
        let Some(&(_, lo, hi)) = SPRING_RANGES.iter().find(|(k, _, _)| k == key) else {
            fault(FaultKind::SpringUnknownKey);
            continue;
        };
        let Some(raw) = v.as_f64() else {
            fault(FaultKind::SpringWrongType);
            continue;
        };
        if !(lo..=hi).contains(&raw) {
            fault(FaultKind::SpringOutOfRange { value: raw });
        }
        let clamped = raw.clamp(lo, hi) as f32;
        match key.as_str() {
            "stiffness" => params.stiffness = clamped,
            "damping" => params.damping = clamped,
            "gravity" => params.gravity = clamped,
            "drag" => params.drag = clamped,
            _ => params.limit_deg = clamped,
        }
    }
    Some(params)
}

fn cube(raw: &RawCube, bone_mirror: bool) -> Cube {
    let mut faces = match &raw.uv {
        Some(RawUv::Box(uv)) => box_uv(*uv, raw.size),
        Some(RawUv::PerFace(map)) => per_face_uv(map),
        None => [None; 6],
    };

    // Zero-width UV means no face (Blockbench writes `uv_size: [0, 0]` for absent faces).
    for face in &mut faces {
        if face.is_some_and(|f| f.size[0].abs() < EPSILON || f.size[1].abs() < EPSILON) {
            *face = None;
        }
    }

    let pivot = raw
        .pivot
        .unwrap_or(std::array::from_fn(|i| raw.origin[i] + raw.size[i] * 0.5));

    // Negating Z turns the file's span z..z+depth into -(z+depth)..-z.
    let [x, y, z] = raw.origin;
    let mut cube = Cube {
        origin: [x, y, -(z + raw.size[2])],
        size: raw.size,
        inflate: raw.inflate,
        rotation: raw.rotation,
        pivot: to_ir(pivot),
        faces,
        box_uv: matches!(raw.uv, Some(RawUv::Box(_))),
        mirror: false,
    };

    if cube.box_uv && raw.mirror.unwrap_or(bone_mirror) {
        cube.toggle_mirror();
    }

    cube
}

/// File space to IR space: Z negates, as the client's Y-down model space plus the entity
/// renderer's `scale(-1, -1, 1)` and 180-degree turn do. See also `math::bedrock_rotation`.
pub(crate) fn to_ir([x, y, z]: [f32; 3]) -> [f32; 3] {
    [x, y, -z]
}

const EPSILON: f32 = 1e-4;

/// The vanilla box-UV cross layout.
fn box_uv(uv: [f32; 2], size: [f32; 3]) -> [Option<FaceUv>; 6] {
    let [u, v] = uv;
    // Box UV uses whole texels, as Blockbench does.
    let [w, h, d] = size.map(|s| s.abs().floor());

    let mut faces = [None; 6];
    let mut set = |f: Face, uv: [f32; 2], size: [f32; 2]| {
        faces[f.index()] = Some(FaceUv { uv, size });
    };

    set(Face::Up, [u + d, v], [w, d]);
    // Vanilla V-flips the underside, and negative height encodes that.
    set(Face::Down, [u + d + w, v + d], [w, -d]);
    // The band runs -X, +Z, +X, -Z, so the front (the skin's face at (8,8)) lands on south.
    set(Face::West, [u, v + d], [d, h]);
    set(Face::South, [u + d, v + d], [w, h]);
    set(Face::East, [u + d + w, v + d], [d, h]);
    set(Face::North, [u + 2.0 * d + w, v + d], [w, h]);

    faces
}

/// Face names are file-space, so a file's `north` is the IR's south and vice versa.
fn per_face_uv(map: &HashMap<String, RawFaceUv>) -> [Option<FaceUv>; 6] {
    let mut faces = [None; 6];

    for (name, raw) in map {
        let face = Face::from_name(name).map(|f| match f {
            Face::North => Face::South,
            Face::South => Face::North,
            other => other,
        });

        if let Some(f) = face {
            faces[f.index()] = Some(FaceUv {
                uv: raw.uv,
                size: raw.uv_size.unwrap_or([0.0, 0.0]),
            });
        }
    }

    faces
}

#[derive(Deserialize)]
struct RawGeometry {
    #[serde(default)]
    description: RawDescription,
    // 1.8.0 put these at the geometry level, without the underscore.
    texturewidth: Option<f32>,
    textureheight: Option<f32>,
    #[serde(default)]
    bones: Vec<RawBone>,
}

#[derive(Deserialize)]
#[serde(default)]
struct RawDescription {
    identifier: String,
    texture_width: f32,
    texture_height: f32,
}

impl Default for RawDescription {
    fn default() -> Self {
        RawDescription {
            identifier: String::new(),
            texture_width: 64.0,
            texture_height: 64.0,
        }
    }
}

#[derive(Deserialize)]
struct RawBone {
    name: String,
    parent: Option<String>,
    #[serde(default)]
    pivot: [f32; 3],
    #[serde(default)]
    rotation: [f32; 3],
    #[serde(default)]
    mirror: bool,
    #[serde(default)]
    cubes: Vec<RawCube>,
    locators: Option<Value>,
    #[serde(rename = "polyplus:spring")]
    spring: Option<Value>,
}

#[derive(Deserialize)]
struct RawCube {
    #[serde(default)]
    origin: [f32; 3],
    #[serde(default)]
    size: [f32; 3],
    #[serde(default)]
    inflate: f32,
    mirror: Option<bool>,
    pivot: Option<[f32; 3]>,
    #[serde(default)]
    rotation: [f32; 3],
    uv: Option<RawUv>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawUv {
    Box([f32; 2]),
    PerFace(HashMap<String, RawFaceUv>),
}

#[derive(Deserialize)]
struct RawFaceUv {
    uv: [f32; 2],
    uv_size: Option<[f32; 2]>,
}

#[cfg(test)]
mod tests {
    use crate::bedrock;
    use crate::ir::{Face, SpringParams};

    #[test]
    fn parses_legacy_1_8_layout() {
        let m = bedrock::parse_one(
            r#"{
              "format_version": "1.8.0",
              "geometry.cape": {
                "texturewidth": 64, "textureheight": 32,
                "bones": [{ "name": "cape", "pivot": [0, 24, 2],
                  "cubes": [{ "origin": [-5, 8, 2], "size": [10, 16, 1], "uv": [0, 0] }] }]
              }
            }"#,
        )
        .unwrap();
        assert_eq!(m.identifier, "geometry.cape");
        assert_eq!(m.texture_size, [64.0, 32.0]);
        assert_eq!(m.bones.len(), 1);
    }

    fn one_cube(cube: &str) -> crate::ir::Cube {
        let m = bedrock::parse_one(&format!(
            r#"{{"format_version":"1.12.0","minecraft:geometry":[{{
              "description": {{ "identifier": "g" }},
              "bones": [{{ "name": "b", "pivot": [0,0,0], "cubes": [{cube}] }}]
            }}]}}"#
        ))
        .unwrap();
        m.bones[0].cubes[0].clone()
    }

    #[test]
    fn faces_mapped_to_nothing_are_dropped() {
        let c = one_cube(
            r#"{"origin":[0,0,0],"size":[4,4,4],"uv":{
                "north":{"uv":[0,0],"uv_size":[4,4]},
                "south":{"uv":[0,0],"uv_size":[0,0]},
                "east":{"uv":[0,0],"uv_size":[4,0]}}}"#,
        );
        // The file's north is the IR's south.
        assert!(c.faces[Face::South.index()].is_some());
        assert!(c.faces[Face::North.index()].is_none());
        assert!(c.faces[Face::East.index()].is_none());
    }

    #[test]
    fn zero_depth_cubes_are_two_sided_billboards() {
        let c = one_cube(r#"{"origin":[0,0,-2],"size":[8,8,0],"uv":[0,0]}"#);
        let present: Vec<_> = Face::ALL
            .iter()
            .filter(|f| c.faces[f.index()].is_some())
            .collect();
        assert_eq!(present, [&Face::North, &Face::South]);
        let (min, max) = c.bounds();
        assert!(
            min[2] < 2.0 && max[2] > 2.0,
            "front and back must not be coplanar"
        );
    }

    #[test]
    fn cyclic_bones_are_rejected() {
        let err = bedrock::parse_one(
            r#"{"format_version":"1.12.0","minecraft:geometry":[{
              "description": { "identifier": "g" },
              "bones": [{ "name": "a", "parent": "b" }, { "name": "b", "parent": "a" }]
            }]}"#,
        );
        assert!(matches!(err, Err(bedrock::Error::CyclicBones(_))));
    }

    fn checked(bones: &str) -> bedrock::Parsed {
        bedrock::parse_checked(&format!(
            r#"{{"format_version":"1.12.0","minecraft:geometry":[{{
              "description": {{ "identifier": "g" }},
              "bones": [{bones}]
            }}]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn spring_blocks_parse_with_defaults_for_missing_keys() {
        let p = checked(
            r#"{ "name": "tail_1", "pivot": [0,12,2] },
               { "name": "tail_2", "parent": "tail_1", "pivot": [0,12,4],
                 "polyplus:spring": { "stiffness": 0.7, "gravity": -0.5, "limit": 90, "collide": false } },
               { "name": "tail_3", "parent": "tail_2", "polyplus:spring": {} }"#,
        );
        assert!(p.faults.is_empty(), "{:?}", p.faults);
        let m = &p.models[0];
        assert_eq!(m.bones[0].spring, None);
        assert_eq!(
            m.bones[1].spring,
            Some(SpringParams {
                stiffness: 0.7,
                gravity: -0.5,
                limit_deg: 90.0,
                collide: false,
                ..SpringParams::default()
            })
        );
        assert_eq!(m.bones[2].spring, Some(SpringParams::default()));
        assert_eq!(
            SpringParams::default(),
            SpringParams {
                stiffness: 0.4,
                damping: 0.2,
                gravity: 0.6,
                drag: 0.1,
                limit_deg: 50.0,
                collide: true,
            }
        );
    }
}
