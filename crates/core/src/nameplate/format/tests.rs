use std::path::Path;

use serde_json::{json, Value};

use crate::container::tests::zip_of;
use crate::container::{open_files, Opened, Options as ContainerOptions, CAPABILITIES};
use crate::issues::{Issue, IssueCode, Level, Mode};
use crate::limits::NAMEPLATE_CAPS;
use crate::signals::signal_id;

use super::super::geom::Anchor;
use super::super::image::{Axis, ColorRef};
use super::super::queries::MolangUse;
use super::super::types::{
    Ease, Fill, Key, LayerContent, LetterOverride, Mask, NameplateAsset, Of, Paint, PaletteTrack,
    PaletteValue, PathSpeed, Reduced, Target, TextMode, Trigger, Value as KeyValue, CHANCE_ONE,
    SPRITE_CLEAR,
};
use super::super::{check_molang, load, LoadOptions, Loaded, Skip, Source};
use super::compile;

pub(crate) struct Kit {
    files: Vec<(String, Value)>,
    raw: Vec<(String, Vec<u8>)>,
    keep_requires: bool,
}

impl Kit {
    pub(crate) fn new(manifest: Value, palettes: Value) -> Kit {
        let files = vec![
            ("manifest.json".into(), manifest),
            ("palettes.json".into(), palettes),
        ];
        Kit {
            files,
            raw: Vec::new(),
            keep_requires: false,
        }
    }

    fn minimal() -> Kit {
        let manifest = json!({ "format": 1, "kind": "nameplate", "id": "plate", "requires": ["nameplate"], "fps": 6, "loop": 12,
            "text": { "color": "ink" }, "layers": [{ "id": "body" }, { "id": "gem", "depth": "over_text" }] });
        let palettes = json!({ "slots": [{ "id": "fill", "name": "Fill", "ramp": ["#202020"] }, { "id": "ink", "ramp": ["#FFFFFF"] },
            { "id": "glow", "ramp": ["#101010", "#808080", "#F0F0F0"] }] });
        let mut kit = Kit::new(manifest, palettes);
        kit.edit("layers/body.json {'kind': 'body', 'padding': [2, 1, 2, 1], 'fill': {'color': 'fill'}, 'rings': [{'color': 'glow[2]'}]} \
            & layers/gem.json {'kind': 'sprite', 'sprite': 'gem', 'of': 'body', 'anchor': 'top', 'pivot': 'bottom'} \
            & sprites/gem.json {'colors': {'o': 'glow', 'h': 'glow[2]'}, 'frames': [['.o.', 'oho', '.o.'], ['...', '.h.', '...']]}");
        kit
    }

    pub(crate) fn file(&mut self, name: &str) -> &mut Value {
        if !self.files.iter().any(|(n, _)| n == name) {
            self.files.push((name.to_string(), Value::Null));
        }
        &mut self.files.iter_mut().find(|(n, _)| n == name).unwrap().1
    }

    pub(crate) fn edit(&mut self, script: &str) -> &mut Kit {
        for step in script.split(" & ").map(str::trim).filter(|s| !s.is_empty()) {
            let (target, text) = step.split_once(' ').unwrap_or((step, "-"));
            let json = text.replace('\'', "\"").replace('`', "'");
            let value = (text != "-")
                .then(|| serde_json::from_str(&json).unwrap_or_else(|e| panic!("{step}: {e}")));
            if let Some((id, depth)) = target.strip_prefix('+').and_then(|t| t.split_once(':')) {
                let layers = self.file("manifest.json")["layers"].as_array_mut().unwrap();
                layers.push(json!({ "id": id, "depth": depth }));
                *self.file(&format!("layers/{id}.json")) = value.unwrap();
                continue;
            }
            let (name, pointer) =
                target.split_at(target.find(".json").map_or(target.len(), |i| i + 5));
            match (pointer, value) {
                ("", None) => self.files.retain(|(n, _)| n != name),
                ("", Some(value)) => *self.file(name) = value,
                (pointer, value) => put(self.file(name), pointer, value),
            }
        }
        self
    }

    pub(crate) fn bytes(&self) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = self.raw.clone();
        for (name, value) in &self.files {
            let mut value = value.clone();
            if name == "manifest.json" && !self.keep_requires {
                value["requires"] = json!(CAPABILITIES);
            }
            out.push((name.clone(), serde_json::to_vec(&value).unwrap()));
        }
        out
    }

    pub(crate) fn compile(&self, mode: Mode) -> (Option<NameplateAsset>, Vec<Issue>) {
        let files = self.bytes();
        let options = ContainerOptions {
            mode,
            legacy: NAMEPLATE_CAPS,
            capabilities: CAPABILITIES,
        };
        match open_files(&files, &options) {
            Opened::Bundle(mut bundle) => {
                let asset = compile(&mut bundle, &LoadOptions::new(mode));
                (asset, bundle.finish())
            }
            Opened::Invalid(issues) => (None, issues),
            _ => panic!("not a bundle"),
        }
    }

    pub(crate) fn asset(&self) -> NameplateAsset {
        let (asset, issues) = self.compile(Mode::Authoring);
        let errors: Vec<&Issue> = issues.iter().filter(|i| i.level == Level::Error).collect();
        assert!(errors.is_empty(), "unexpected issues: {errors:#?}");
        asset.expect("an asset")
    }
}

fn put(root: &mut Value, pointer: &str, value: Option<Value>) {
    let (parent, last) = pointer.rsplit_once('/').unwrap();
    let mut at = root;
    for key in parent.split('/').skip(1) {
        at = child(at, key);
    }
    match value {
        None => drop(at.as_object_mut().unwrap().remove(last)),
        Some(value) if last == "-" => at.as_array_mut().unwrap().push(value),
        Some(value) => *child(at, last) = value,
    }
}

fn child<'v>(at: &'v mut Value, key: &str) -> &'v mut Value {
    if let Value::Array(items) = at {
        return &mut items[key.parse::<usize>().unwrap()];
    }
    if !at.is_object() {
        *at = json!({});
    }
    at.as_object_mut()
        .unwrap()
        .entry(key)
        .or_insert(Value::Null)
}

pub(crate) fn testdata(name: &str) -> Vec<(String, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/nameplate/format/testdata");
    let root = root.join(name);
    let mut files = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for path in std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()) {
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(&root).unwrap().to_string_lossy();
                files.push((rel.replace('\\', "/"), std::fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

fn shown(issues: &[Issue]) -> String {
    let lines: Vec<String> = issues
        .iter()
        .map(|i| format!("{} {}", i.code.as_str(), i.path))
        .collect();
    lines.join("\n")
}

fn has(issues: &[Issue], expected: &str) -> bool {
    let (code, path) = expected.split_once(' ').unwrap();
    issues
        .iter()
        .any(|i| i.code.as_str() == code && i.path == path && i.level == Level::Error)
}

fn rows<'r, S: AsRef<str>>(setup: &str, rows: &'r [S]) -> Vec<(Kit, &'r str, &'r str)> {
    let parse = |row: &'r S| {
        let (script, expected) = row.as_ref().split_once(" => ").unwrap();
        let mut kit = Kit::minimal();
        kit.edit(setup).edit(script);
        (kit, expected, row.as_ref())
    };
    rows.iter().map(parse).collect()
}

fn rejects<S: AsRef<str>>(setup: &str, cases: &[S]) {
    for (kit, expected, row) in rows(setup, cases) {
        for mode in [Mode::Client, Mode::Authoring] {
            let (asset, issues) = kit.compile(mode);
            let ok = has(&issues, expected) && asset.is_none();
            assert!(ok, "{mode:?} {row}\n{}", shown(&issues));
        }
    }
}

fn forgives<S: AsRef<str>>(setup: &str, cases: &[S]) -> Vec<NameplateAsset> {
    let check = |(kit, expected, row): (Kit, &str, &str)| {
        let (client, forgiven) = kit.compile(Mode::Client);
        let (asset, issues) = kit.compile(Mode::Authoring);
        let ok = forgiven.is_empty() && client.is_some() && has(&issues, expected);
        assert!(ok, "{row}\n{}\n{}", shown(&forgiven), shown(&issues));
        asset.expect("authoring errors that clients forgive still build the asset")
    };
    rows(setup, cases).into_iter().map(check).collect()
}

fn accepts(script: &str) -> NameplateAsset {
    Kit::minimal().edit(script).asset()
}

fn sprite(asset: &NameplateAsset) -> &super::super::types::SpriteLayer {
    match &asset.layers[1].content {
        LayerContent::Sprite(sprite) => sprite,
        _ => panic!("layer 1 is not a sprite"),
    }
}

fn body(asset: &NameplateAsset) -> &super::super::types::Body {
    match &asset.layers[0].content {
        LayerContent::Body(body) => body,
        _ => panic!("layer 0 is not a body"),
    }
}

fn needs(asset: &NameplateAsset, capability: &str) -> bool {
    asset.requires.iter().any(|r| r == capability)
}

#[test]
fn format_minimal_bundle_compiles() {
    let asset = accepts("");
    let variant = &asset.variants[0];
    assert_eq!(variant.slots, vec![Some(0), Some(1)]);
    assert_eq!(variant.tones[2 * 16 + 1], 0xff80_8080);
    assert_eq!(variant.tones[2 * 16 + 9], 0xfff0_f0f0);
    assert_eq!(asset.identity, crate::wearer::seed_key("plate"));
    assert_eq!(asset.layers[1].of, Of::Layer(0));
    let pixels = &asset.sprites[usize::from(sprite(&asset).sprite)];
    assert_eq!(pixels.colors, [ColorRef::slot(2, 0), ColorRef::slot(2, 2)]);
    assert_eq!(pixels.frames[0][..3], [SPRITE_CLEAR, 0, SPRITE_CLEAR]);
    assert_eq!(body(&asset).rings, [Fill::Color(ColorRef::slot(2, 2))]);
    assert_eq!(asset.text.color, Paint::Color(ColorRef::slot(1, 0)));
}

#[test]
fn format_rejects_broken_headers_palettes_and_layers() {
    let many_layers: Vec<Value> = (0..33).map(|i| json!({ "id": format!("l{i}") })).collect();
    let many_frames = json!((0..121).map(|_| json!(["o"])).collect::<Vec<_>>());
    let wide = "o".repeat(65);
    rejects(
        "",
        &[
            format!(
                "manifest.json/layers {} => layers manifest.json#layers",
                json!(many_layers)
            ),
            format!("sprites/gem.json/frames {many_frames} => sprite sprites/gem.json#frames"),
            format!("sprites/gem.json/frames [['{wide}']] => sprite sprites/gem.json#frames"),
        ],
    );
    rejects("", &[
        "manifest.json/fps 0 => manifest manifest.json#fps",
        "manifest.json/loop 601 => manifest manifest.json#loop",
        "manifest.json/phase 'sometimes' => manifest manifest.json#phase",
        "manifest.json/params {'r': {'value': 5, 'min': 0, 'max': 4}} => manifest manifest.json#params.r",
        "manifest.json/params {'r': {'value': 2, 'min': 0, 'max': 4}} & manifest.json/variants {'a': {'params': {'r': 9}}} => variant manifest.json#variants.a.params.r",
        "palettes.json - => missing_file manifest.json",
        "palettes.json/slots/0/ramp ['red'] => palette palettes.json#slots.fill.ramp[0]",
        "manifest.json/variants {'a': {'palette': 'mint'}} => variant manifest.json#variants.a.palette",
        "sprites/gem.json/colors/o 'glow[3]' => sprite sprites/gem.json#colors.o",
        "manifest.json/layers/1/id 'body' => layers manifest.json#layers.body.id",
        "manifest.json/layers/0/id '../body' => id_invalid manifest.json#layers[0].id",
        "layers/gem.json - => missing_file manifest.json#layers.gem",
        "layers/body.json/padding [33, 0, 0, 0] => layer layers/body.json#padding",
        "layers/body.json/radius 17 => corners layers/body.json#radius",
        "layers/body.json/corners [4, 4, 0, 0] & layers/body.json/radius 2 => corners_both layers/body.json#radius",
        "layers/body.json/rings [{'color': 'fill'}, {'color': 'fill'}, {'color': 'fill'}, {'color': 'fill'}, {'color': 'fill'}] => layer layers/body.json#rings",
        "layers/body.json/of 'gem' => name_target layers/body.json#of",
        "sprites/gem.json - => missing_file layers/gem.json#sprite",
        "layers/gem.json/sprite '../gem' => id_invalid layers/gem.json#sprite",
        "sprites/gem.json/frames/1 ['...', '.x.', '...'] => sprite sprites/gem.json#frames[1]",
        "sprites/gem.json/frames [['o'], ['h'], ['o'], ['h'], ['o']] => sprite_frames sprites/gem.json#frames",
        "layers/gem.json/anchor 'name_left' => anchor layers/gem.json#of",
        "layers/gem.json/offset [65, 0] => layer layers/gem.json#offset",
    ]);
}

#[test]
fn format_rejects_broken_art_paths_repeats_and_emitters() {
    rejects("sprites/frame.json {'colors': {'a': 'fill'}, 'rows': ['aaaaa', 'a...a', 'aaaaa']}", &[
        "layers/body.json/art {'sprite': 'gem'} => body_art layers/body.json#art.sprite",
        "layers/body.json/art {'sprite': 'frame', 'left': 2, 'right': 1, 'mirror': true} => body_art layers/body.json#art.mirror",
    ]);
    rejects("layers/gem.json/pivot -", &[
        "layers/gem.json/path {'preset': 'top', 'points': []} => path layers/gem.json#path",
        "layers/gem.json/path {'preset': 'top', 'laps': 2, 'speed': 1} => path_timing layers/gem.json#path.laps",
        "layers/gem.json/path {'preset': 'top', 'speed': 17} => speed_range layers/gem.json#path.speed",
        "layers/gem.json/path {'preset': 'top', 'stops': [{'at': 0.5, 'pause': 2}, {'at': 0.4, 'pause': 2}]} => path_timing layers/gem.json#path.stops[1].at",
        "layers/gem.json/path {'preset': 'top'} & layers/gem.json/of '$name' => name_target layers/gem.json#of",
    ]);
    rejects("", &[
        "layers/gem.json/repeat {'edge': 'top', 'spacing': 0} => repeat layers/gem.json#repeat.spacing",
        "layers/gem.json/repeat {'edge': 'top', 'spacing': 4} & layers/gem.json/path {'preset': 'top'} => repeat layers/gem.json#repeat",
        "layers/gem.json/breakpoints [{'min': 0}, {'min': 1}, {'min': 2}, {'min': 3}, {'min': 4}, {'min': 5}, {'min': 6}, {'min': 7}, {'min': 8}] => widths layers/gem.json#breakpoints",
    ]);
    rejects("+sparks:over_text {'kind': 'emitter', 'sprites': ['gem']}", &[
        "layers/sparks.json/sprites [] => emitter layers/sparks.json#sprites",
        "layers/sparks.json/sprites ['nope'] => missing_file layers/sparks.json#sprites[0]",
        "layers/sparks.json/velocity {'x': [1, -1]} => emitter layers/sparks.json#velocity.x",
        "layers/gem.json/path {'preset': 'top'} & layers/sparks.json/of 'gem' => emitter layers/sparks.json#of",
    ]);
}

#[test]
fn format_rejects_broken_groups_masks_text_badges_variants_and_flags() {
    rejects("", &[
        "manifest.json/groups [{'id': 'body'}] => group manifest.json#groups[0].id",
        "manifest.json/groups [{'id': 'a', 'parent': 'b'}, {'id': 'b', 'parent': 'a'}] => group manifest.json#groups[0].parent",
        "manifest.json/layers/1/mask 'gem' => mask manifest.json#layers.gem.mask",
        "manifest.json/text/mode 'fancy' => text manifest.json#text.mode",
        "manifest.json/text/shadow '#12' => text manifest.json#text.shadow",
        "manifest.json/badge {'placement': 'placed'} => badge manifest.json#badge.anchor",
        "manifest.json/badge {'placement': 'carried', 'follow': 'outside'} => follow manifest.json#badge.follow",
        "manifest.json/variants {'a': {'text': {'color': 'nope'}}} => text manifest.json#variants.a.text.color",
        "manifest.json/variants {'a': {'hide': ['nope']}} => variant manifest.json#variants.a.hide[0]",
        "layers/body.json/fill {'flag': true} => missing_file manifest.json",
    ]);
    let bands = json!((0..13)
        .map(|_| json!({ "color": "fill" }))
        .collect::<Vec<_>>());
    rejects("layers/body.json/fill {'flag': true}", &[
        format!("flags.json {{'flags': [{{'id': 'a', 'bands': {bands}}}]}} => flags flags.json#flags.a.bands"),
        "flags.json {'flags': [{'id': 'a', 'bands': [{'color': 'fill'}, {'color': 'ink'}], 'rowsOverride': {'5': [2, 2]}}]} => flags flags.json#flags.a.rowsOverride.5".into(),
        "flags.json {'flags': [{'id': 'pride', 'bands': [{'color': 'fill'}]}]} & manifest.json/variants {'a': {'flag': 'trans'}} => flags manifest.json#variants.a.flag".into(),
    ]);
    let mut kit = Kit::minimal();
    kit.edit("manifest.json/text/shadow 'nope' & manifest.json/variants {'a': {'text': {'color': 'glow'}}}");
    let (_, issues) = kit.compile(Mode::Client);
    assert!(has(&issues, "text manifest.json#text.shadow"));
    assert!(!issues.iter().any(|i| i.path.contains("#variants")));
}

#[test]
fn format_rejects_broken_animations() {
    rejects("", &[
        "manifest.json/animations {'Loop': {}} => id_invalid manifest.json#animations.Loop",
        "manifest.json/animations/loop/tracks/gem/frame [[1, 0]] => key_start manifest.json#animations.loop.tracks.gem.frame[0]",
        "manifest.json/animations/loop/tracks/gem/frame [[0, 0], [4, 1], [4, 0]] => key_order manifest.json#animations.loop.tracks.gem.frame[2]",
        "manifest.json/animations/loop/tracks/gem/position [[0, [65, 0]]] => animation manifest.json#animations.loop.tracks.gem.position[0]",
        "manifest.json/animations/hop {'trigger': {'reaction': 'sneaking', 'on': 'while'}} => reaction manifest.json#animations.hop.trigger.reaction",
        "manifest.json/animations/hop {'trigger': {'reaction': 'is_hurt', 'on': 'while'}, 'length': 4} => animation manifest.json#animations.hop.length",
        "manifest.json/animations/hop {'trigger': 'equip', 'length': 4, 'reduced': 'drop'} => reduced_while manifest.json#animations.hop.reduced",
        "manifest.json/animations/hop {'trigger': 'equip'} => animation manifest.json#animations.hop.length",
        "layers/gem.json/path {'preset': 'top', 'speed': 1} & layers/gem.json/pivot - & manifest.json/animations/loop/tracks/gem/speed [[0, 2]] => speed_trigger manifest.json#animations.loop.tracks.gem.speed",
        "manifest.json/animations/blink {'schedule': {'window': 10, 'jitter': 7}, 'length': 4, 'tracks': {'gem': {'opacity': [[0, 0.5]]}}} => animation manifest.json#animations.blink.length",
        "manifest.json/animations/blink {'schedule': {'window': 20}, 'length': 4, 'tracks': {'$badge': {'opacity': [[0, 0.5]]}}} => badge_schedule manifest.json#animations.blink.tracks.$badge",
        "manifest.json/animations/a {'schedule': {'window': 10, 'id': 's'}, 'length': 4, 'tracks': {'gem': {'opacity': [[0, 0.5]]}}} & manifest.json/animations/b {'schedule': {'window': 12, 'id': 's'}, 'length': 4, 'tracks': {'gem': {'opacity': [[0, 0.5]]}}} => animation manifest.json#animations.b.schedule.window",
        "manifest.json/animations/a {'schedule': {'window': 10}, 'length': 4, 'tracks': {'gem': {'opacity': [[0, 0.5]]}}} & manifest.json/animations/b {'schedule': {'window': 10}, 'length': 4, 'tracks': {'gem': {'opacity': [[0, 0.5]]}}} => animation manifest.json#animations.b.tracks.gem",
        "manifest.json/animations/loop/tracks/palette.glow {'keys': [[0, 'base']], 'cycle': {'every': 1, 'mode': 'forward'}} => palette_animation manifest.json#animations.loop.tracks.palette.glow",
        "manifest.json/text {'mode': 'motion', 'motion': 'q.x'} => molang_query_place manifest.json#text.motion",
        "manifest.json/animations/loop/tracks/$text/motion 'q.glyph_index' => letter_program manifest.json#animations.loop.tracks.$text",
        "manifest.json/text/mode 'motion' & manifest.json/animations/hurt {'trigger': {'reaction': 'is_hurt', 'on': 'while'}, 'tracks': {'$text': {'wave': '1'}}} => letter_program manifest.json#animations.hurt.tracks.$text.wave",
    ]);
    rejects("manifest.json/animations/duck/trigger {'reaction': 'is_sneaking', 'on': 'while'}", &[
        "manifest.json/animations/duck/tracks/gem/speed [[0, 2]] => speed_path manifest.json#animations.duck.tracks.gem.speed",
        "layers/gem.json/path {'preset': 'top', 'speed': 1} & layers/gem.json/pivot - & manifest.json/animations/duck/tracks/gem/speed [[0, -17]] => speed_range manifest.json#animations.duck.tracks.gem.speed[0]",
        "+sparks:over_text {'kind': 'emitter', 'sprites': ['gem']} & manifest.json/animations/duck/tracks/sparks/burst [[0, 65]] => burst_count manifest.json#animations.duck.tracks.sparks.burst[0]",
        "manifest.json/animations/duck/tracks/$badge/position [[0, ['q.frame', 0]]] => badge_molang manifest.json#animations.duck.tracks.$badge",
    ]);
    for extreme in [i64::MIN, i64::MAX] {
        rejects("", &[
            format!("layers/gem.json/offset [{extreme}, 0] => layer layers/gem.json#offset"),
            format!("manifest.json/badge {{'placement': 'placed', 'anchor': 'top', 'offset': [{extreme}, 0]}} => badge manifest.json#badge.offset"),
        ]);
    }
}

const POSITION: &str = "manifest.json/animations/loop/tracks/gem/position";

#[test]
fn format_molang_rules_are_strict_and_scoped() {
    let rows: Vec<String> = [
        "q.mystery => molang_unknown_query",
        "q.x + 1 => molang_query_place",
        "math.nope(1) => molang_unknown_math",
        "c.thing => molang_unknown_root",
        "(1 + => molang_unexpected",
        "  => molang_empty",
        "q.variant_param(`nope`) => molang_unknown_param",
        "1 + 2 => molang_constant",
    ]
    .iter()
    .map(|row| row.split_once(" => ").unwrap())
    .map(|(src, code)| format!("{POSITION} [[0, ['{src}', 0]]] => {code} manifest.json#animations.loop.tracks.gem.position[0]"))
    .collect();
    rejects("", &rows[..7]);
    forgives("", &rows[7..]);
    let soft = |source: &str, place: MolangUse| check_molang(source, place).unwrap().soft;
    let actual = &soft("1 + 2", MolangUse::Channel)[0].code;
    assert_eq!(*actual, IssueCode::MolangConstant);
    assert!(soft("1 + 2", MolangUse::Text).is_empty());
    assert!(check_molang("q.x", MolangUse::Field).is_ok());
    let broken = check_molang("q.nope", MolangUse::Channel).unwrap_err();
    assert!(broken.names.iter().any(|(k, _)| k == "message"));
}

#[test]
fn format_authoring_errors_clients_forgive() {
    let pride = "layers/body.json/fill {'flag': true} & flags.json {'flags': [{'id': 'pride', 'bands': [{'color': 'fill'}, {'color': 'ink'}]}]}";
    let assets = forgives("", &[
        "manifest.json/colourways {} => unknown_key manifest.json#colourways".to_string(),
        "manifest.json/text/colour 'ink' => unknown_key manifest.json#text.colour".into(),
        "manifest.json/variants {'a': {'text': {'colour': 'ink'}}} => unknown_key manifest.json#variants.a.text.colour".into(),
        "layers/body.json/fill {'gradient': 'glow'} & palettes.json/colorways {'mint': {'glow': ['#000000', '#FFFFFF']}} => colorway_tones palettes.json#colorways.mint.glow".into(),
        "manifest.json/ownsBackground true & manifest.json/layers [{'id': 'gem'}] & layers/body.json - & layers/gem.json/of - => background_without_body manifest.json#variants.default.ownsBackground".into(),
        "manifest.json/text {'mode': 'motion', 'motion': 'v.color = 255; return 1;'} => letter_owned manifest.json#text.motion".into(),
        "manifest.json/animations/hurt {'trigger': {'reaction': 'is_hurt', 'on': 'while'}, 'tracks': {'$text': {'motion': 'q.glyph_x'}}} => letter_program manifest.json#animations.hurt.tracks.$text.motion".into(),
        "manifest.json/ownsBackground true & manifest.json/variants {'bare': {'hide': ['body']}, 'full': {}} => background_without_body manifest.json#variants.bare.ownsBackground".into(),
        "manifest.json/text {'motion': 'q.glyph_index'} => letter_program manifest.json#text.motion".into(),
        "manifest.json/text {'mode': 'motion'} & manifest.json/variants {'a': {'text': {'mode': 'vanilla', 'motion': 'q.glyph_index'}}, 'b': {}} => letter_program manifest.json#variants.a.text.motion".into(),
        format!("manifest.json/variants {{'a': {{}}}} & {pride} => variant_flag_missing manifest.json#variants.a"),
        format!("{pride} => variant_flag_missing manifest.json#variants"),
    ]);
    assert!(assets[7].variants[0].owns_background);
    assert_eq!(assets[8].text.motion, None);
    assert_eq!(assets[9].variants[0].text.motion, None);
    assert_eq!(assets[10].variants[0].flag, Some(0));
    let mut kit = Kit::minimal();
    kit.edit("sprites/unused.json {'colors': {}, 'rows': ['.']}");
    let (_, issues) = kit.compile(Mode::Authoring);
    let unused = issues
        .iter()
        .find(|i| i.code == IssueCode::UnreferencedFile)
        .unwrap();
    let actual = &(unused.level, unused.path.as_str());
    assert_eq!(*actual, (Level::Warning, "sprites/unused.json"));
    assert!(kit.compile(Mode::Client).1.is_empty());
}

#[test]
fn format_compiles_conversions_tables_and_triggers() {
    let asset = accepts("manifest.json/params {'radius': 3, 'speed': {'value': 2, 'min': 1, 'max': 4}} & manifest.json/variants {'a': {'params': {'speed': 4}}, 'b': {}}");
    assert_eq!(asset.params[0].max, 3.0);
    assert_eq!(asset.variants[0].params, [3.0, 4.0]);
    let asset = accepts("palettes.json/colorways {'mint': {'glow': ['#000000', '#FFFFFF']}} & palettes.json/slots/2/interpolate 5");
    assert_eq!(asset.variants[0].colorway, Some(0));
    assert_eq!(asset.variants[0].tones[2 * 16 + 4], 0xffff_ffff);
    assert_eq!(asset.palette.slots[2].ramp.len(), 5);
    let asset = accepts("+alt:background {'kind': 'body', 'fill': {'color': 'glow'}} \
        & manifest.json/variants {'a': {'hide': ['gem'], 'swap': {'body': 'alt'}}, 'b': {'text': {'color': 'glow[2]'}}}");
    assert_eq!(asset.variants[0].slots, [Some(2), None, Some(2)]);
    let actual = &asset.variants[1].text.color;
    assert_eq!(*actual, Paint::Color(ColorRef::slot(2, 2)));
    assert_eq!(body(&accepts("layers/body.json/radius 5")).corners, [5; 4]);
    assert_eq!(
        sprite(&accepts(
            "layers/gem.json/anchor 'name_right' & layers/gem.json/of - & layers/gem.json/pivot -"
        ))
        .pivot,
        Anchor::Left
    );
    accepts("sprites/gem.json/frames [['o'], ['h'], ['o'], ['h'], ['o']] & layers/gem.json/timeline [{'frame': 4, 'hold': 6}, {'frame': 0, 'hold': 6}]");

    let asset = accepts("layers/gem.json/pivot - & layers/gem.json/path {'preset': 'bottom', 'speed': [[0, 2], [6, 0.5, 'linear']]} \
        & layers/gem.json/breakpoints [{'min': 0, 'max': 40}, {'min': 40}, {'min': 96, 'max': 400}]");
    let PathSpeed::Track(speed) = &sprite(&asset).path.as_ref().unwrap().speed else {
        panic!("a speed track")
    };
    assert_eq!(speed.keys[0].value, KeyValue::Fixed(512));
    assert_eq!(
        speed.keys[1],
        Key {
            frame: 6,
            value: KeyValue::Fixed(128),
            ease: Ease::Linear
        }
    );
    assert_eq!(asset.breakpoint_edges, [40, 96, 400]);
    assert_eq!(asset.breakpoint(96), 2);
    let asset = accepts("+sparks:over_text {'kind': 'emitter', 'sprites': ['gem'], 'velocity': {'x': [-0.5, 0.5], 'y': [-1.5, -0.75]}, \
        'gravity': [0, 0.1], 'drift': [0.25, 0], 'fade': {'after': 0.5, 'to': 0.2}}");
    let LayerContent::Emitter(e) = &asset.layers[2].content else {
        panic!("an emitter")
    };
    assert_eq!((e.velocity_x, e.velocity_y), ([-128, 128], [-384, -192]));
    assert_eq!((e.gravity, e.drift), ([0, 26], [64, 0]));
    assert_eq!(e.fade.map(|f| f.to), Some(51));
    let asset = accepts("manifest.json/text {'mode': 'owned', 'color': '#102030'} & manifest.json/layers/1/mask 'body' & manifest.json/layers/1/clip 'fill'");
    assert_eq!(asset.fixed_colors[0], 0xff10_2030);
    let actual = &asset.layers[1].mask;
    assert_eq!(
        *actual,
        Some(Mask::Layer {
            slot: 0,
            fill: true
        })
    );
    let asset = accepts("layers/body.json/fill {'flag': true} & manifest.json/variants {'a': {'flag': 'bi'}, 'b': {'flag': 'pride'}} \
        & flags.json {'flags': [{'id': 'pride', 'bands': [{'color': 'fill'}, {'color': 'ink'}], 'rowsOverride': {'5': [2, 3]}}, {'id': 'bi', 'axis': 'columns', 'bands': [{'color': 'fill'}]}]}");
    assert_eq!(asset.flags[0].rows[0].bands, [2, 3]);
    assert_eq!(asset.flags[1].axis, Axis::X);
    let actual = &(asset.variants[0].flag, asset.variants[1].flag);
    assert_eq!(*actual, (Some(1), Some(0)));

    let asset = accepts("manifest.json/animations/loop/tracks/gem {'frame': [[0, 0], [6, 1]], 'position': [[0, [0, -2]], [6, ['math.sin(q.frame)', 1], 'ease']], \
        'opacity': [[0, 0], [11, 1, 'linear']]}");
    let channels = &asset.animations.looping.targets[0].channels;
    assert_eq!(channels.frame.as_ref().unwrap().keys[1].ease, Ease::Step);
    let actual = &channels.x.as_ref().unwrap().keys[1].value;
    assert_eq!(*actual, KeyValue::Molang(0));
    let actual = &channels.opacity.as_ref().unwrap().keys[1].value;
    assert_eq!(*actual, KeyValue::Fixed(255));
    assert_eq!(
        asset.molang.programs[0].path,
        "manifest.json#animations.loop.tracks.gem.position[1]"
    );
    let asset = accepts("manifest.json/animations {'duck': {'trigger': {'reaction': 'is_sneaking', 'on': 'while'}, 'reduced': 'drop', 'tracks': {'gem': {'frame': [[0, 1]]}}}, \
        'ouch': {'trigger': {'reaction': 'is_hurt', 'on': 'start'}, 'length': 6, 'restart': true}, 'rise': {'trigger': {'reaction': 'is_sneaking', 'on': 'stop'}, 'length': 3}, \
        'hello': {'trigger': {'appear': 3}, 'length': 12}, 'again': {'trigger': 'appear', 'length': 2}, 'worn': {'trigger': 'equip', 'length': 8}}");
    let (hurt, sneak) = (
        signal_id("is_hurt").unwrap() as u8,
        signal_id("is_sneaking").unwrap() as u8,
    );
    assert_eq!(asset.animations.reactions[0].reduced, Reduced::Drop);
    let triggers: Vec<Trigger> = asset
        .animations
        .one_shots
        .iter()
        .map(|s| s.trigger)
        .collect();
    assert_eq!(
        triggers,
        [
            Trigger::Start(hurt),
            Trigger::Stop(sneak),
            Trigger::Appear(3),
            Trigger::Appear(5),
            Trigger::Equip
        ]
    );
    assert!(asset.animations.one_shots[0].restart && needs(&asset, "nameplate_one_shots"));
    let asset = accepts("layers/gem.json/path {'preset': 'top', 'speed': 1} & layers/gem.json/pivot - \
        & manifest.json/animations/duck {'trigger': {'reaction': 'is_sneaking', 'on': 'while'}, 'tracks': {'gem': {'speed': [[0, -2]]}, '$plate': {'position': [[0, [1, -1]]]}}}");
    let clip = &asset.animations.reactions[0].clip;
    let actual = &clip.targets[0].channels.speed.as_ref().unwrap().keys[0].value;
    assert_eq!(*actual, KeyValue::Fixed(-512));
    assert_eq!(clip.targets[1].target, Target::Plate);
    let asset = accepts("manifest.json/animations {'a': {'schedule': {'window': 12, 'chance': 0.75, 'id': 'shared', 'weight': 3}, 'length': 6, \
        'tracks': {'gem': {'opacity': [[0, 0.5]]}}}, 'b': {'schedule': {'window': 12, 'chance': 0.75, 'id': 'shared'}, 'length': 2, 'tracks': {'gem': {'frame': [[0, 1]]}}}}");
    let shared = &asset.animations.schedules[0];
    assert_eq!(
        (shared.chance, shared.salt),
        (CHANCE_ONE / 4 * 3, crate::wearer::seed_key("shared"))
    );
    assert_eq!((shared.events.len(), shared.events[0].weight), (2, 3));
    let asset = accepts("manifest.json/animations/loop/tracks/palette.ink {'keys': [[0, '#FF0000'], [4, 'glow[1]'], [8, 'math.hsv(q.frame, 1, 1)'], [10, 'base']]}");
    let PaletteTrack::Keys(keys) = &asset.animations.looping.palette[0].track else {
        panic!("keys")
    };
    assert_eq!(keys[1].tones, [PaletteValue::Ref(ColorRef::slot(2, 1))]);
    assert!(
        matches!(keys[2].tones[0], PaletteValue::Molang(_))
            && keys[3].tones == [PaletteValue::Base]
    );
    let asset = accepts("manifest.json/text/mode 'motion' & manifest.json/animations/calm {'trigger': 'equip', 'length': 4, 'tracks': {'$text': {'motion': null}}}");
    let actual = &asset.animations.one_shots[0].clip.letters;
    assert_eq!(*actual, Some(LetterOverride::Off));
    assert_eq!(asset.variants[0].text.mode, TextMode::Motion);
}

#[test]
fn format_requires_and_wearer_records_follow_the_content() {
    let mut kit = Kit::minimal();
    kit.keep_requires = true;
    kit.edit("+sparks:over_text {'kind': 'emitter', 'sprites': ['gem']}");
    let (asset, issues) = kit.compile(Mode::Client);
    assert!(issues.is_empty());
    let actual = &asset.unwrap().requires;
    assert_eq!(*actual, ["nameplate", "nameplate_particles"]);
    let (asset, issues) = kit.compile(Mode::Authoring);
    assert!(asset.is_some() && has(&issues, "manifest manifest.json#requires"));
    kit.edit("manifest.json/requires ['nameplate', 'nameplate_particles']");
    assert!(kit.compile(Mode::Authoring).1.is_empty());
    kit.edit("manifest.json/fps 0 & layers/body.json/radius 40 & sprites/gem.json/colors/h 'nope' & manifest.json/animations {'hop': {'trigger': 'equip'}}");
    let (_, issues) = kit.compile(Mode::Client);
    let expected = "manifest manifest.json#fps,corners layers/body.json#radius,sprite sprites/gem.json#colors.h";
    assert!(
        expected.split(',').all(|e| has(&issues, e)),
        "{}",
        shown(&issues)
    );

    let small = accepts("");
    let asset = accepts("manifest.json/animations {'loop': {'tracks': {'gem': {'position': [[0, ['v.lift = (v.lift ?? 0) + 1; return v.lift;', 0]]]}}}, \
        'a': {'trigger': 'equip', 'length': 2}, 'b': {'trigger': 'appear', 'length': 2, 'restart': true}} & +halo:behind {'kind': 'body', 'of': '$name', 'fill': {'color': 'glow'}}");
    let record = asset.record;
    assert_eq!((record.variables, record.one_shots), (1, 2));
    assert_eq!((record.effect_floats, record.channels), (1, 3));
    assert!(
        asset.max_out_words > small.max_out_words
            && crate::wearer::RecordLayout::new(record).is_ok()
    );
    let asset = accepts("+sheen:over_background {'kind': 'material', 'material': 'sheen', 'drift': true} \
        & materials/sheen.json {'nodes': [{'id': 'c', 'type': 'Slot', 'set': {'slot': 'glow'}}, {'id': 'out', 'type': 'Output', 'in': {'color': 'c'}}]}");
    assert!(matches!(&asset.layers[2].content, LayerContent::Material(m) if m.drift));
}

fn kit_asset(name: &str) -> NameplateAsset {
    let kit = Kit {
        files: Vec::new(),
        raw: testdata(name),
        keep_requires: true,
    };
    for mode in [Mode::Client, Mode::Authoring] {
        let (asset, issues) = kit.compile(mode);
        let holds = issues.is_empty() && asset.is_some();
        assert!(holds, "{name} {mode:?}:\n{}", shown(&issues));
    }
    let asset = kit.compile(Mode::Client).0.unwrap();
    let manifest = &kit
        .raw
        .iter()
        .find(|(n, _)| n == "manifest.json")
        .unwrap()
        .1;
    let manifest: Value = serde_json::from_slice(manifest).unwrap();
    assert_eq!(json!(asset.requires), manifest["requires"], "{name}");
    asset
}

#[test]
fn format_every_test_kit_bundle_loads() {
    let names = "test_plate wardrobe_plate reaction_plate dome_plate sample_plate material_plate atlas_plate";
    let kits: Vec<NameplateAsset> = names.split(' ').map(kit_asset).collect();
    assert_eq!(kits[2].animations.reactions.len(), 4);
    assert_eq!(kits[1].variants[2].key, "moss");
    assert_eq!(kits[3].variants[0].slots, [Some(0), None, Some(2)]);
    assert_eq!(kits[4].animations.schedules[0].id, "tail_moves");
    assert!(kits[5..].iter().all(|kit| kit.materials.len() == 3));
    assert_eq!(kits[5].textures[0].width, 8);

    let files = testdata("reaction_plate");
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let zip = zip_of(&entries);
    let options = ContainerOptions::new(Mode::Authoring);
    let Opened::Bundle(mut bundle) = crate::container::open(&zip, &options) else {
        panic!("a bundle")
    };
    let asset = compile(&mut bundle, &LoadOptions::new(Mode::Authoring));
    assert!(bundle.finish().is_empty() && asset.is_some());
}

#[test]
fn format_load_skips_rejects_and_returns_drafts() {
    let bytes = |manifest: &str| {
        vec![(
            "manifest.json".to_string(),
            manifest.replace('\'', "\"").into_bytes(),
        )]
    };
    let client = |files: &[(String, Vec<u8>)]| load(Source::Files(files), &LoadOptions::default());
    let future = bytes("{'format': 2, 'kind': 'nameplate', 'id': 'a'}");
    assert!(matches!(client(&future), Loaded::Skipped(Skip::Format(2))));
    let needs =
        bytes("{'format': 1, 'kind': 'nameplate', 'id': 'a', 'requires': ['nameplate_holograms']}");
    assert!(
        matches!(client(&needs), Loaded::Skipped(Skip::Requires(r)) if r == ["nameplate_holograms"])
    );
    let older = LoadOptions {
        capabilities: &["nameplate"],
        ..LoadOptions::new(Mode::Client)
    };
    let flagged = bytes("{'format': 1, 'kind': 'nameplate', 'id': 'a', 'requires': ['nameplate', 'nameplate_flags']}");
    assert!(matches!(
        load(Source::Files(&flagged), &older),
        Loaded::Skipped(_)
    ));
    for row in [
        "{'format': 1, 'kind': 'item', 'id': 'a'} => kind_unknown manifest.json#kind",
        "{'format': 1, 'kind': 'nameplate', 'id': 'a', 'fps': 6, 'loop': 12} => missing_file manifest.json",
    ] {
        let (manifest, expected) = row.split_once(" => ").unwrap();
        assert!(matches!(client(&bytes(manifest)), Loaded::Invalid(issues) if has(&issues, expected)), "{row}");
    }
    assert!(
        matches!(client(&testdata("test_plate")), Loaded::Asset { issues, .. } if issues.is_empty())
    );
    let files = Kit::minimal().edit("manifest.json/colourways {}").bytes();
    assert!(matches!(client(&files), Loaded::Asset { .. }));
    let draft = load(Source::Files(&files), &LoadOptions::new(Mode::Authoring));
    assert!(
        matches!(draft, Loaded::Draft { issues, .. } if issues[0].code == IssueCode::UnknownKey)
    );
}
