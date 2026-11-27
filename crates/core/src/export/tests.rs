use serde_json::{json, Value};

use super::*;
use crate::asset::{load, LoadOptions, Loaded, Source};
use crate::attach::Attach;
use crate::container::{decode_png, CAPABILITIES, ENGINE_CAPABILITIES};
use crate::ir::{Bone, Cube, Face, FaceUv, Model};
use crate::issues::{Mode, Report};

fn uv(u: f32, v: f32, w: f32, h: f32) -> Option<FaceUv> {
    Some(FaceUv {
        uv: [u, v],
        size: [w, h],
    })
}

fn every_face() -> [Option<FaceUv>; 6] {
    let mut faces = [None; 6];
    faces[Face::North.index()] = uv(0.0, 0.0, 4.0, 6.0);
    faces[Face::South.index()] = uv(4.0, 0.0, 4.0, 6.0);
    faces[Face::East.index()] = uv(8.0, 0.0, 2.0, 6.0);
    faces[Face::West.index()] = uv(10.0, 0.0, 2.0, 6.0);
    faces[Face::Up.index()] = uv(0.0, 8.0, 4.0, 2.0);
    faces[Face::Down.index()] = uv(4.0, 8.0, 4.0, 2.0);
    faces
}

fn cube(origin: [f32; 3], size: [f32; 3], rotation: [f32; 3]) -> Cube {
    Cube {
        origin,
        size,
        inflate: 0.0,
        rotation,
        pivot: std::array::from_fn(|k| origin[k] + size[k] * 0.5),
        faces: every_face(),
        box_uv: false,
        mirror: false,
    }
}

fn bone(name: &str, cubes: Vec<Cube>) -> Bone {
    Bone {
        name: name.to_string(),
        parent: None,
        parent_name: Some("body".to_string()),
        pivot: [0.0, 24.0, 0.0],
        rotation: [0.0; 3],
        cubes,
        glow: None,
        locators: Vec::new(),
        spring: None,
    }
}

fn item(bones: Vec<Bone>, attach_to: &str) -> CosmeticAsset {
    let mut a = CosmeticAsset::new("cube", 1, Kind::Item);
    a.model = Some(Model {
        identifier: "geometry.cube".to_string(),
        texture_size: [32.0, 16.0],
        bones,
    });
    a.textures = vec![IndexedTexture {
        name: "cube".to_string(),
        width: 2,
        height: 1,
        rgba: vec![10, 20, 30, 255, 40, 50, 60, 255],
        index: None,
    }];
    a.attach = Attach {
        rig: RigKind::Player,
        bone: Some(attach_to.to_string()),
        replaces: None,
        anchor: None,
    };
    a
}

fn one_cube(c: Cube) -> CosmeticAsset {
    item(vec![bone("part", vec![c])], "body")
}

fn run(a: &CosmeticAsset, target: Target) -> Export {
    export(a, &ExportOptions::new("test", target)).unwrap()
}

fn decoded(png: &[u8]) -> crate::container::Image {
    let mut report = Report::new(Mode::Authoring);
    decode_png(png, "texture.png", 4096, &mut report).expect("valid png")
}

#[test]
fn a_cuboid_exports_exactly() {
    let a = one_cube(cube([-2.0, 20.0, -1.0], [4.0, 6.0, 2.0], [0.0; 3]));
    let e = run(&a, Target::ITEMS_FREE);

    let face =
        |u0: f64, v0: f64, u1: f64, v1: f64| json!({ "uv": [u0, v0, u1, v1], "texture": "#0" });
    assert_eq!(
        e.model,
        json!({
            "textures": { "0": "test:item/cube", "particle": "test:item/cube" },
            "elements": [{
                "from": [6.0, 4.0, 7.0],
                "to": [10.0, 10.0, 9.0],
                "faces": {
                    "north": face(2.0, 0.0, 4.0, 6.0),
                    "east": face(5.0, 0.0, 6.0, 6.0),
                    "south": face(0.0, 0.0, 2.0, 6.0),
                    "west": face(4.0, 0.0, 5.0, 6.0),
                    "up": face(2.0, 10.0, 0.0, 8.0),
                    "down": face(4.0, 10.0, 2.0, 8.0)
                }
            }]
        })
    );
    assert_eq!(
        e.definition,
        Some(json!({ "model": { "type": "minecraft:model", "model": "test:item/cube" } }))
    );
    let paths: Vec<&str> = e.files.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            "assets/test/models/item/cube.json",
            "assets/test/textures/item/cube.png",
            "assets/test/items/cube.json"
        ]
    );
    let written: Value = serde_json::from_slice(&e.files[0].1).unwrap();
    assert_eq!(written, e.model);
    let png = decoded(&e.files[1].1);
    assert_eq!((png.width, png.height), (2, 1));
    assert_eq!(png.rgba, a.textures[0].rgba);
    assert_eq!((e.anchor, e.scale), ([8.0; 3], 1.0));
    assert_eq!(e.report, ConversionReport::default());
    assert!(e.report.offerable());
}

#[test]
fn the_sample_wings_load_and_export_with_a_report() {
    let names = [
        "manifest.json",
        "models/wings.geo.json",
        "textures/wings.png",
        "textures/wings.idx.png",
        "palettes.json",
        "animations/wings.animation.json",
        "animation_controllers/wings.animation_controllers.json",
    ];
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/sample_wings/");
    let files: Vec<(String, Vec<u8>)> =
        names.iter().map(|n| (n.to_string(), std::fs::read(format!("{dir}{n}")).unwrap())).collect();
    let caps: Vec<&str> = CAPABILITIES.iter().chain(ENGINE_CAPABILITIES).copied().collect();
    let options = LoadOptions { mode: Mode::Authoring, identity: 7, capabilities: &caps };
    let Loaded::Asset { asset: wings, .. } = load(Source::Files(&files), &options) else { panic!("sample did not load") };

    let e = run(&wings, Target::ITEMS_FREE);
    let paths: Vec<&str> = e.files.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths[0], "assets/test/models/item/sample_wings.json");
    let element = &e.model["elements"][0];
    assert_eq!((&element["from"], &element["to"]), (&json!([0.0, -2.0, 10.0]), &json!([8.0, 6.0, 11.0])));
    let kinds: Vec<LossKind> = e.report.losses.iter().map(|l| l.kind).collect();
    let dropped = [LossKind::Animations, LossKind::Controllers, LossKind::Reactions, LossKind::Colorways, LossKind::CustomColors];
    assert_eq!((kinds.as_slice(), e.report.offerable()), (dropped.as_slice(), false));
    let report = serde_json::to_value(&e.report).unwrap();
    assert_eq!((&report["losses"][0]["kind"], &report["losses"][0]["severity"]), (&json!("animations"), &json!("dropped")));
    assert_eq!(&decoded(&e.texture).rgba[..3], &[0x30, 0x40, 0x50]);
    let storm = ExportOptions { colorway: wings.palette.colorway("storm"), ..ExportOptions::new("test", Target::ITEMS_FREE) };
    assert_eq!(&decoded(&export(&wings, &storm).unwrap().texture).rgba[..3], &[0x20, 0x20, 0x20]);
}
