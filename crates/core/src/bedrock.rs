//! Bedrock `.geo.json` (1.12+ and legacy 1.8.0) -> [`Model`], with box UV and `mirror` resolved.
//! File space becomes IR space here: Z is negated (see `to_ir`), so the front lands on +Z.

use std::collections::HashMap;

use serde::Deserialize;

use crate::ir::{Bone, Cube, Face, FaceUv, Model};

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

pub fn parse(json: &str) -> Result<Vec<Model>, Error> {
    let mut root: serde_json::Value = serde_json::from_str(json)?;

    if let Some(list) = root.get_mut("minecraft:geometry") {
        let raw: Vec<RawGeometry> = serde_json::from_value(list.take())?;
        return raw.into_iter().map(build).collect();
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
        out.push(build(raw)?);
    }

    if out.is_empty() {
        return Err(Error::NoGeometry);
    }
    Ok(out)
}

pub fn parse_one(json: &str) -> Result<Model, Error> {
    parse(json)?.into_iter().next().ok_or(Error::NoGeometry)
}

fn build(raw: RawGeometry) -> Result<Model, Error> {
    let texture_size = [
        raw.texturewidth.unwrap_or(raw.description.texture_width),
        raw.textureheight.unwrap_or(raw.description.texture_height),
    ];

    let bones = topo_sort(raw.bones)?;

    Ok(Model {
        identifier: raw.description.identifier,
        texture_size,
        bones,
    })
}

/// Parents first. A bone whose parent is outside the file becomes a root but keeps `parent_name`.
fn topo_sort(raw: Vec<RawBone>) -> Result<Vec<Bone>, Error> {
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
            }
        })
        .collect())
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
    use crate::ir::Face;

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

    #[test]
    fn mirror_flips_u_and_swaps_sides() {
        let json = |mirror: bool| {
            format!(
                r#"{{"format_version":"1.12.0","minecraft:geometry":[{{
                  "description": {{ "identifier": "g" }},
                  "bones": [{{ "name": "b", "pivot": [0,0,0], "cubes": [
                    {{ "origin": [0,0,0], "size": [4,6,2], "uv": [0,0], "mirror": {mirror} }}
                  ] }}]
                }}]}}"#
            )
        };
        let plain = bedrock::parse_one(&json(false)).unwrap();
        let flipped = bedrock::parse_one(&json(true)).unwrap();

        let p = plain.bones[0].cubes[0].faces[Face::South.index()].unwrap();
        let f = flipped.bones[0].cubes[0].faces[Face::South.index()].unwrap();
        assert!(f.size[0] < 0.0, "mirrored faces run backwards in u");
        assert_eq!(f.uv[0], p.uv[0] + p.size[0], "and start at the far edge");

        // East and West swap rects.
        let pe = plain.bones[0].cubes[0].faces[Face::East.index()].unwrap();
        let fw = flipped.bones[0].cubes[0].faces[Face::West.index()].unwrap();
        assert_eq!(pe.uv[1], fw.uv[1]);
    }

    #[test]
    fn unresolved_parents_are_kept_for_attachables() {
        let m = bedrock::parse_one(
            r#"{"format_version":"1.12.0","minecraft:geometry":[{
              "description": { "identifier": "geometry.hat" },
              "bones": [{ "name": "brim", "parent": "head", "pivot": [0,32,0] }]
            }]}"#,
        )
        .unwrap();
        assert_eq!(m.bones[0].parent, None);
        assert_eq!(m.bones[0].parent_name.as_deref(), Some("head"));
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
    fn box_uv_lays_out_on_whole_texels() {
        let c = one_cube(r#"{"origin":[0,0,0],"size":[4.25,3.25,5.7],"uv":[0,0]}"#);
        let south = c.faces[Face::South.index()].unwrap();
        assert_eq!(south.uv, [5.0, 5.0], "offset by the floored depth");
        assert_eq!(south.size, [4.0, 3.0]);
    }

    #[test]
    fn a_rotated_cube_without_a_pivot_turns_about_its_centre() {
        let c = one_cube(r#"{"origin":[2,4,6],"size":[2,4,6],"rotation":[0,0,45],"uv":[0,0]}"#);
        assert_eq!(c.pivot, [3.0, 6.0, -9.0], "file centre, Z negated");
        let explicit = one_cube(r#"{"origin":[2,4,6],"size":[2,4,6],"pivot":[0,0,0],"uv":[0,0]}"#);
        assert_eq!(explicit.pivot, [0.0; 3]);
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
    fn file_space_back_is_ir_space_back() {
        let c = one_cube(r#"{"origin":[-8,20,2],"size":[16,16,0],"uv":[0,0]}"#);
        let (min, max) = c.bounds();
        assert!(
            max[2] < 0.0,
            "a wing at file z=2 sits behind the body: {min:?}..{max:?}"
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
}
