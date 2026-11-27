mod anim;
mod layers;
mod palette;
mod read;
mod style;

use serde_json::{Map, Value};

use crate::container::{is_id, Bundle, Image, MANIFEST};
use crate::issues::{Issue, IssueCode, Level, Mode};
use crate::limits::{
    NAMEPLATE_FPS_MAX, NAMEPLATE_FPS_MIN, NAMEPLATE_LETTERS, NAMEPLATE_LOOP_FRAMES,
    NAMEPLATE_PARAMS, QUADS_PER_NAMEPLATE,
};
use crate::molang::{
    compile_checked, ErrorCode, Interner, Options, Program, Rules, Runtime, Warnings,
};
use crate::palette::Palette;
use crate::wearer::{seed_key, DeclarationError, RecordLayout, WearerDeclarations, MAX_ONE_SHOTS};

use super::abi::{
    BADGE_WORDS, CLIP_WORDS, EVENT_WORDS, LETTER_WORDS, MAT_FIXED, OP_WORDS, TAG_HEAD, TEXT_FIXED,
};
use super::anim::triggers::tail_slots;
use super::image::ColorRef;
use super::material::ir::MaterialIr;
use super::queries::{known_scope, scope_for, MolangUse, NameplateQuery};
use super::types::{
    Animations, BadgeStyle, Depth, Flag, Group, Layer, LayerContent, MolangProgram, MolangSet,
    NameplateAsset, Param, Phase, Sprite, Target, TextMode, TextStyle, Trigger, Variant,
};
use super::LoadOptions;
use read::Loc;

pub const PALETTES: &str = "palettes.json";
pub const FLAGS: &str = "flags.json";
pub const NAME_TARGET: &str = "$name";
pub const BADGE_TARGET: &str = "$badge";
pub const PLATE_TARGET: &str = "$plate";
pub const TEXT_TARGET: &str = "$text";
pub const PALETTE_PREFIX: &str = "palette.";
pub const LOOP: &str = "loop";
pub const DEFAULT_VARIANT: &str = "default";
pub const MANIFEST_FIELDS: [&str; 11] = [
    "fps",
    "loop",
    "phase",
    "ownsBackground",
    "params",
    "text",
    "badge",
    "layers",
    "groups",
    "animations",
    "variants",
];
pub const MAX_CLIPS: u32 = 4;

const VARIANT_KEYS: [&str; 8] = [
    "palette",
    "hide",
    "swap",
    "flag",
    "ownsBackground",
    "text",
    "badge",
    "params",
];
const PARAM_KEYS: [&str; 3] = ["value", "min", "max"];
const LAYER_ENTRY_KEYS: [&str; 5] = ["id", "depth", "step", "mask", "clip"];
const GROUP_KEYS: [&str; 4] = ["id", "parent", "layers", "breakpoints"];
const RESERVED: [&str; 3] = ["auto", "none", "base"];

pub fn compile(bundle: &mut Bundle<'_>, options: &LoadOptions<'_>) -> Option<NameplateAsset> {
    let mut compiler = Compiler::new(bundle);
    let asset = compiler.run();
    let hard = compiler.hard_errors();
    if hard {
        None
    } else {
        asset.map(|mut asset| {
            asset.identity = options.identity.unwrap_or(asset.identity);
            asset
        })
    }
}

pub fn palette(bundle: &mut Bundle<'_>) -> Option<Palette> {
    let mut compiler = Compiler::new(bundle);
    compiler
        .palettes()
        .then(|| std::mem::take(&mut compiler.palette))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Body,
    Sprite,
    Emitter,
    Material,
}

impl Kind {
    fn parse(name: &str) -> Option<Kind> {
        match name {
            "body" => Some(Kind::Body),
            "sprite" => Some(Kind::Sprite),
            "emitter" => Some(Kind::Emitter),
            "material" => Some(Kind::Material),
            _ => None,
        }
    }
}

struct Decl {
    pub id: String,
    pub loc: Loc,
    pub file: String,
    pub depth: Depth,
    pub step: u16,
    pub kind: Option<Kind>,
    pub json: Option<Map<String, Value>>,
    pub path: bool,
    pub laps: bool,
    pub repeat: bool,
    pub timeline: bool,
    pub facings: bool,
    pub sprite: Option<u16>,
}

struct Compiler<'b, 'a> {
    bundle: &'b mut Bundle<'a>,
    mode: Mode,
    soft: usize,
    id: String,
    fps: u8,
    loop_frames: u16,
    palette: Palette,
    tones: Vec<u8>,
    fixed: Vec<u32>,
    params: Vec<Param>,
    decls: Vec<Decl>,
    layers: Vec<Option<Layer>>,
    groups: Vec<Group>,
    group_ids: Vec<String>,
    sprite_ids: Vec<String>,
    sprites: Vec<Option<Sprite>>,
    flags: Vec<Flag>,
    flags_loaded: bool,
    uses_flags: bool,
    material_ids: Vec<String>,
    materials: Vec<Option<MaterialIr>>,
    texture_names: Vec<String>,
    textures: Vec<Image>,
    molang: MolangSet,
    caps: Vec<&'static str>,
    breakpoint_edges: Vec<i32>,
    keyed_frames: Vec<bool>,
    banded: Vec<u8>,
    letters_allowed: bool,
}

impl<'b, 'a> Compiler<'b, 'a> {
    fn new(bundle: &'b mut Bundle<'a>) -> Compiler<'b, 'a> {
        let mode = bundle.mode();
        let id = bundle.id().to_string();
        Compiler {
            bundle,
            mode,
            soft: 0,
            id,
            fps: 1,
            loop_frames: 1,
            palette: Palette::default(),
            tones: Vec::new(),
            fixed: Vec::new(),
            params: Vec::new(),
            decls: Vec::new(),
            layers: Vec::new(),
            groups: Vec::new(),
            group_ids: Vec::new(),
            sprite_ids: Vec::new(),
            sprites: Vec::new(),
            flags: Vec::new(),
            flags_loaded: false,
            uses_flags: false,
            material_ids: Vec::new(),
            materials: Vec::new(),
            texture_names: Vec::new(),
            textures: Vec::new(),
            molang: MolangSet::default(),
            caps: vec!["nameplate"],
            breakpoint_edges: Vec::new(),
            keyed_frames: Vec::new(),
            banded: Vec::new(),
            letters_allowed: false,
        }
    }

    fn hard_errors(&self) -> bool {
        let errors = self
            .bundle
            .issues()
            .iter()
            .filter(|i| i.level == Level::Error && i.code != IssueCode::UnknownKey)
            .count();
        errors > self.soft
    }

    fn run(&mut self) -> Option<NameplateAsset> {
        let manifest = self.bundle.manifest().clone();
        let root = Loc::new(MANIFEST, "");
        self.bundle.check_manifest_keys(&MANIFEST_FIELDS);

        self.header(&manifest, &root);
        let phase = self.phase(&manifest, &root);
        let owns_background = self.boolean(
            &manifest,
            "ownsBackground",
            &root,
            IssueCode::Manifest,
            false,
        );
        self.params = self.params(&manifest, &root);

        if !self.palettes() {
            return None;
        }
        self.declare_layers(&manifest, &root);
        self.read_layer_files();
        self.groups(&manifest, &root);
        self.layer_contents();
        self.masks(&manifest, &root);
        if self.uses_flags || self.variants_name_flags(&manifest) {
            self.load_flags();
        }

        self.letters_allowed = letter_modes(&manifest);
        let (mut text, text_raw) = self.text(&manifest, &root);
        let (badge, badge_raw) = self.badge(&manifest, &root);
        let animations = self.animations(&manifest, &root);
        self.sprite_cycles();
        self.base_letters(&mut text, text_raw.as_ref(), self.letters_allowed);
        let variants = self.variants(
            &manifest,
            &root,
            owns_background,
            &text,
            text_raw.as_ref(),
            &badge,
            badge_raw.as_ref(),
            &animations,
        );

        let banded = std::mem::take(&mut self.banded);
        self.colorway_tones(&banded);
        self.capabilities(&animations, &variants);
        let record = self.record(&animations);
        let max_out_words = self.max_out_words();

        if self.hard_errors() {
            return None;
        }

        let layers: Vec<Layer> = self.layers.iter_mut().filter_map(Option::take).collect();
        let sprites: Vec<Sprite> = self.sprites.iter_mut().filter_map(Option::take).collect();
        let materials: Vec<MaterialIr> =
            self.materials.iter_mut().filter_map(Option::take).collect();
        if layers.len() != self.decls.len()
            || sprites.len() != self.sprite_ids.len()
            || materials.len() != self.material_ids.len()
        {
            return None;
        }

        let mut requires: Vec<String> = self.caps.iter().map(|c| c.to_string()).collect();
        requires.dedup();
        self.breakpoint_edges.sort_unstable();
        self.breakpoint_edges.dedup();

        Some(NameplateAsset {
            id: self.id.clone(),
            identity: seed_key(&self.id),
            fps: self.fps,
            loop_frames: self.loop_frames,
            phase,
            params: std::mem::take(&mut self.params),
            palette: std::mem::take(&mut self.palette),
            variants,
            layers,
            groups: std::mem::take(&mut self.groups),
            sprites,
            flags: std::mem::take(&mut self.flags),
            materials,
            textures: std::mem::take(&mut self.textures),
            animations,
            text,
            badge,
            molang: std::mem::take(&mut self.molang),
            record,
            max_out_words,
            requires,
            fixed_colors: std::mem::take(&mut self.fixed),
            breakpoint_edges: std::mem::take(&mut self.breakpoint_edges),
        })
    }

    fn push(&mut self, issue: Issue) {
        self.bundle.report().push(issue);
    }

    fn soft(&mut self, issue: Issue) {
        if self.mode == Mode::Authoring {
            self.soft += 1;
            self.bundle.report().push(issue);
        }
    }

    fn keys(&mut self, loc: &Loc, map: &Map<String, Value>, known: &[&str]) {
        self.bundle
            .report()
            .check_keys(&loc.file, &loc.at, map, known);
    }

    fn cap(&mut self, name: &'static str) {
        if !self.caps.contains(&name) {
            self.caps.push(name);
        }
    }

    fn int(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        loc: &Loc,
        code: IssueCode,
        range: (i64, i64),
        default: i64,
    ) -> i64 {
        let Some(value) = map.get(key) else {
            return default;
        };
        match read::whole(value) {
            Some(n) if n >= range.0 && n <= range.1 => n,
            other => {
                let mut issue = loc
                    .key(key)
                    .error(code)
                    .value("min", range.0 as f64)
                    .value("max", range.1 as f64);
                if let Some(n) = other {
                    issue = issue.value("value", n as f64);
                }
                self.push(issue);
                default
            }
        }
    }

    fn num(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        loc: &Loc,
        code: IssueCode,
        range: (f64, f64),
        default: f64,
    ) -> f64 {
        let Some(value) = map.get(key) else {
            return default;
        };
        match read::number(value) {
            Some(n) if n >= range.0 && n <= range.1 => n,
            other => {
                let mut issue = loc
                    .key(key)
                    .error(code)
                    .value("min", range.0)
                    .value("max", range.1);
                if let Some(n) = other {
                    issue = issue.value("value", n);
                }
                self.push(issue);
                default
            }
        }
    }

    fn boolean(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        loc: &Loc,
        code: IssueCode,
        default: bool,
    ) -> bool {
        match map.get(key) {
            None => default,
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                self.push(loc.key(key).error(code));
                default
            }
        }
    }

    fn string<'v>(
        &mut self,
        map: &'v Map<String, Value>,
        key: &str,
        loc: &Loc,
        code: IssueCode,
    ) -> Option<&'v str> {
        let value = map.get(key)?;
        match value.as_str() {
            Some(s) => Some(s),
            None => {
                self.push(loc.key(key).error(code));
                None
            }
        }
    }

    fn choice<T>(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        loc: &Loc,
        code: IssueCode,
        parse: impl Fn(&str) -> Option<T>,
        default: T,
    ) -> T {
        let Some(value) = map.get(key) else {
            return default;
        };
        match value.as_str().and_then(&parse) {
            Some(v) => v,
            None => {
                let mut issue = loc.key(key).error(code);
                if let Some(name) = value.as_str() {
                    issue = issue.name("value", name);
                }
                self.push(issue);
                default
            }
        }
    }

    fn offset(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        loc: &Loc,
        code: IssueCode,
        limit: i64,
    ) -> [i8; 2] {
        let Some(value) = map.get(key) else {
            return [0, 0];
        };
        match read::pair_i64(value) {
            Some([x, y]) if (-limit..=limit).contains(&x) && (-limit..=limit).contains(&y) => {
                [x as i8, y as i8]
            }
            _ => {
                self.push(loc.key(key).error(code).value("limit", limit as f64));
                [0, 0]
            }
        }
    }

    fn object<'v>(
        &mut self,
        value: &'v Value,
        loc: &Loc,
        code: IssueCode,
    ) -> Option<&'v Map<String, Value>> {
        match value.as_object() {
            Some(map) => Some(map),
            None => {
                self.push(loc.error(code));
                None
            }
        }
    }

    fn list<'v>(&mut self, value: &'v Value, loc: &Loc, code: IssueCode) -> Option<&'v Vec<Value>> {
        match value.as_array() {
            Some(items) => Some(items),
            None => {
                self.push(loc.error(code));
                None
            }
        }
    }

    fn fixed_color(&mut self, argb: u32) -> ColorRef {
        let index = match self.fixed.iter().position(|c| *c == argb) {
            Some(i) => i,
            None => {
                self.fixed.push(argb);
                self.fixed.len() - 1
            }
        };
        ColorRef::fixed(index as u32)
    }

    fn slot_index(&self, id: &str) -> Option<u8> {
        self.palette.slot(id).and_then(|i| u8::try_from(i).ok())
    }

    fn color_ref(&mut self, text: &str, any: bool) -> Result<ColorRef, Issue> {
        let issue = |i: Issue| i.name("color", text);
        if any && text.starts_with('#') {
            return match read::hex(text) {
                Some(argb) => Ok(self.fixed_color(argb)),
                None => Err(issue(Issue::error(IssueCode::Palette, ""))),
            };
        }
        let Some((id, tone)) = read::slot_ref(text) else {
            return Err(issue(Issue::error(IssueCode::Palette, "")));
        };
        let Some(slot) = self.slot_index(id) else {
            return Err(Issue::error(IssueCode::Palette, "").name("slot", id));
        };
        let tones = self.tones[usize::from(slot)];
        if tone >= tones {
            return Err(Issue::error(IssueCode::Palette, "")
                .name("slot", id)
                .value("tone", f64::from(tone))
                .value("tones", f64::from(tones)));
        }
        Ok(ColorRef::slot(slot, tone))
    }

    fn report_color(
        &mut self,
        result: Result<ColorRef, Issue>,
        loc: &Loc,
        code: IssueCode,
    ) -> Option<ColorRef> {
        match result {
            Ok(color) => Some(color),
            Err(detail) => {
                self.push(Issue {
                    code,
                    path: loc.path(),
                    ..detail
                });
                None
            }
        }
    }

    fn slot_color(&mut self, text: &str, loc: &Loc, code: IssueCode) -> Option<ColorRef> {
        let result = self.color_ref(text, false);
        self.report_color(result, loc, code)
    }

    fn bare_slot(&mut self, value: &Value, loc: &Loc, code: IssueCode) -> Option<u8> {
        let slot = value.as_str().and_then(|id| self.slot_index(id));
        if slot.is_none() {
            let mut issue = loc.error(code);
            if let Some(id) = value.as_str() {
                issue = issue.name("slot", id);
            }
            self.push(issue);
        }
        slot
    }

    fn molang(&mut self, source: &str, scope: MolangUse, loc: &Loc) -> Option<u32> {
        let checked = match check_in(source, scope, &mut self.molang.interner) {
            Ok(checked) => checked,
            Err(issue) => {
                self.push(Issue {
                    path: loc.path(),
                    ..issue
                });
                return None;
            }
        };
        if !self.check_params(source, &checked.program, loc) {
            return None;
        }
        for issue in checked.soft {
            self.soft(Issue {
                path: loc.path(),
                ..issue
            });
        }
        let index = u32::try_from(self.molang.programs.len()).ok()?;
        self.molang.programs.push(MolangProgram {
            cost: checked.program.nodes(),
            program: checked.program,
            scope,
            path: loc.path(),
        });
        Some(index)
    }

    fn check_params(&mut self, source: &str, program: &Program, loc: &Loc) -> bool {
        let Some(slot) = self
            .molang
            .interner
            .query_slot(NameplateQuery::VariantParam.name())
        else {
            return true;
        };
        if !program.reads().queries.contains(&slot) {
            return true;
        }
        let mut ok = true;
        for call in param_calls(source) {
            match call {
                Some(name) if self.params.iter().any(|p| p.id == name) => {}
                Some(name) => {
                    self.push(
                        loc.error(IssueCode::MolangUnknownParam)
                            .name("param", &name),
                    );
                    ok = false;
                }
                None => {
                    self.push(loc.error(IssueCode::MolangParamCall));
                    ok = false;
                }
            }
        }
        ok
    }

    fn header(&mut self, manifest: &Map<String, Value>, root: &Loc) {
        match manifest.get("fps") {
            None => self.push(root.key("fps").error(IssueCode::Manifest)),
            Some(_) => {
                let range = (i64::from(NAMEPLATE_FPS_MIN), i64::from(NAMEPLATE_FPS_MAX));
                self.fps = self.int(manifest, "fps", root, IssueCode::Manifest, range, 1) as u8;
            }
        }
        match manifest.get("loop") {
            None => self.push(root.key("loop").error(IssueCode::Manifest)),
            Some(_) => {
                let range = (1, i64::from(NAMEPLATE_LOOP_FRAMES));
                self.loop_frames =
                    self.int(manifest, "loop", root, IssueCode::Manifest, range, 1) as u16;
            }
        }
    }

    fn phase(&mut self, manifest: &Map<String, Value>, root: &Loc) -> Phase {
        let parse = |name: &str| match name {
            "global" => Some(Phase::Global),
            "per_player" => Some(Phase::PerPlayer),
            _ => None,
        };
        self.choice(
            manifest,
            "phase",
            root,
            IssueCode::Manifest,
            parse,
            Phase::Global,
        )
    }

    fn params(&mut self, manifest: &Map<String, Value>, root: &Loc) -> Vec<Param> {
        let mut params = Vec::new();
        let Some(value) = manifest.get("params") else {
            return params;
        };
        let loc = root.key("params");
        let Some(map) = self.object(value, &loc, IssueCode::Manifest) else {
            return params;
        };
        if map.len() > NAMEPLATE_PARAMS as usize {
            self.push(
                loc.error(IssueCode::Manifest)
                    .value("count", map.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_PARAMS)),
            );
        }
        for (name, value) in map {
            let at = loc.key(name);
            if !is_id(name) {
                self.push(at.error(IssueCode::IdInvalid).name("id", name));
                continue;
            }
            let param = match value {
                Value::Object(spec) => {
                    self.keys(&at, spec, &PARAM_KEYS);
                    let Some(v) = spec.get("value").and_then(read::number) else {
                        self.push(at.key("value").error(IssueCode::Manifest));
                        continue;
                    };
                    let min = self.num(
                        spec,
                        "min",
                        &at,
                        IssueCode::Manifest,
                        (f64::MIN, f64::MAX),
                        v,
                    );
                    let max = self.num(
                        spec,
                        "max",
                        &at,
                        IssueCode::Manifest,
                        (f64::MIN, f64::MAX),
                        v,
                    );
                    if !(min <= v && v <= max) {
                        self.push(
                            at.error(IssueCode::Manifest)
                                .value("value", v)
                                .value("min", min)
                                .value("max", max),
                        );
                    }
                    Param {
                        id: name.clone(),
                        value: v,
                        min,
                        max,
                    }
                }
                other => match read::number(other) {
                    Some(v) => Param {
                        id: name.clone(),
                        value: v,
                        min: v,
                        max: v,
                    },
                    None => {
                        self.push(at.error(IssueCode::Manifest));
                        continue;
                    }
                },
            };
            params.push(param);
        }
        params
    }

    fn variants_name_flags(&self, manifest: &Map<String, Value>) -> bool {
        manifest
            .get("variants")
            .and_then(Value::as_object)
            .is_some_and(|v| v.values().any(|v| v.get("flag").is_some()))
    }

    #[allow(clippy::too_many_arguments)]
    fn variants(
        &mut self,
        manifest: &Map<String, Value>,
        root: &Loc,
        owns_background: bool,
        text: &TextStyle,
        text_raw: Option<&Map<String, Value>>,
        badge: &BadgeStyle,
        badge_raw: Option<&Map<String, Value>>,
        animations: &Animations,
    ) -> Vec<Variant> {
        let plate = moves_plate(animations);
        let mut variants = Vec::new();
        let loc = root.key("variants");
        let declared = match manifest.get("variants") {
            None => None,
            Some(value) => self.object(value, &loc, IssueCode::Variant),
        };
        let entries: Vec<(String, Option<&Map<String, Value>>)> = match declared {
            Some(map) if !map.is_empty() => map
                .iter()
                .map(|(k, v)| (k.clone(), v.as_object()))
                .collect(),
            _ => vec![(DEFAULT_VARIANT.to_string(), None)],
        };
        let implicit = declared.is_none_or(Map::is_empty);
        for (key, spec) in entries {
            let at = loc.key(&key);
            if !implicit && !is_id(&key) {
                self.push(at.error(IssueCode::IdInvalid).name("id", &key));
                continue;
            }
            let empty = Map::new();
            let spec = match spec {
                Some(spec) => spec,
                None if implicit => &empty,
                None => {
                    self.push(at.error(IssueCode::Variant));
                    continue;
                }
            };
            self.keys(&at, spec, &VARIANT_KEYS);
            let colorway = self.variant_colorway(spec, &at, implicit);
            let slots = self.variant_slots(spec, &at);
            let flag = self.variant_flag(spec, &at, implicit, &loc);
            let owns = self.boolean(
                spec,
                "ownsBackground",
                &at,
                IssueCode::Variant,
                owns_background,
            );
            if owns && !self.shows_body(&slots) {
                self.soft(
                    at.key("ownsBackground")
                        .error(IssueCode::BackgroundWithoutBody),
                );
            }
            let mut variant_text = self.variant_text(spec, &at, text, text_raw);
            if variant_text.mode == TextMode::Vanilla && plate {
                variant_text.mode = TextMode::Motion;
            }
            let variant_badge = self.variant_badge(spec, &at, badge, badge_raw);
            let params = self.variant_params(spec, &at);
            let tones = self.palette.resolve(colorway.map(usize::from));
            let stops = self.palette.resolve_stops(colorway.map(usize::from));
            variants.push(Variant {
                identity: seed_key(&format!("{}#{key}", self.id)),
                key,
                colorway,
                tones,
                stops,
                slots,
                flag,
                owns_background: owns,
                text: variant_text,
                badge: variant_badge,
                params,
            });
        }
        variants
    }

    fn variant_colorway(
        &mut self,
        spec: &Map<String, Value>,
        at: &Loc,
        implicit: bool,
    ) -> Option<u16> {
        match spec.get("palette") {
            None if implicit && !self.palette.colorways.is_empty() => Some(0),
            None => None,
            Some(value) => {
                let index = value.as_str().and_then(|name| self.palette.colorway(name));
                if index.is_none() {
                    let mut issue = at.key("palette").error(IssueCode::Variant);
                    if let Some(name) = value.as_str() {
                        issue = issue.name("colorway", name);
                    }
                    self.push(issue);
                }
                index.and_then(|i| u16::try_from(i).ok())
            }
        }
    }

    fn variant_slots(&mut self, spec: &Map<String, Value>, at: &Loc) -> Vec<Option<u8>> {
        let mut slots: Vec<Option<u8>> = (0..self.decls.len()).map(|i| Some(i as u8)).collect();
        if let Some(value) = spec.get("swap") {
            let loc = at.key("swap");
            if let Some(map) = self.object(value, &loc, IssueCode::Variant) {
                for (from, to) in map {
                    let source = self.layer_index(from);
                    let target = to.as_str().and_then(|t| self.layer_index(t));
                    match (source, target) {
                        (Some(s), Some(t)) => slots[usize::from(s)] = Some(t),
                        _ => self.push(loc.key(from).error(IssueCode::Variant).name("layer", from)),
                    }
                }
            }
        }
        if let Some(value) = spec.get("hide") {
            let loc = at.key("hide");
            if let Some(items) = self.list(value, &loc, IssueCode::Variant) {
                for (i, item) in items.iter().enumerate() {
                    match item.as_str().and_then(|id| self.layer_index(id)) {
                        Some(index) => slots[usize::from(index)] = None,
                        None => {
                            let mut issue = loc.index(i).error(IssueCode::Variant);
                            if let Some(id) = item.as_str() {
                                issue = issue.name("layer", id);
                            }
                            self.push(issue);
                        }
                    }
                }
            }
        }
        slots
    }

    fn shows_body(&self, slots: &[Option<u8>]) -> bool {
        slots.iter().flatten().any(|i| {
            self.decls
                .get(usize::from(*i))
                .is_some_and(|d| d.kind == Some(Kind::Body))
        })
    }

    fn variant_flag(
        &mut self,
        spec: &Map<String, Value>,
        at: &Loc,
        implicit: bool,
        variants: &Loc,
    ) -> Option<u16> {
        match spec.get("flag") {
            Some(value) => {
                let index = value
                    .as_str()
                    .and_then(|id| self.flags.iter().position(|f| f.id == id));
                if index.is_none() && self.flags_loaded {
                    let mut issue = at.key("flag").error(IssueCode::Flags);
                    if let Some(id) = value.as_str() {
                        issue = issue.name("flag", id);
                    }
                    self.push(issue);
                }
                index.and_then(|i| u16::try_from(i).ok())
            }
            None if self.uses_flags && !self.flags.is_empty() => {
                let loc = if implicit {
                    variants.clone()
                } else {
                    at.clone()
                };
                self.soft(loc.error(IssueCode::VariantFlagMissing));
                Some(0)
            }
            None => None,
        }
    }

    fn variant_params(&mut self, spec: &Map<String, Value>, at: &Loc) -> Vec<f64> {
        let mut values: Vec<f64> = self.params.iter().map(|p| p.value).collect();
        let Some(value) = spec.get("params") else {
            return values;
        };
        let loc = at.key("params");
        let Some(map) = self.object(value, &loc, IssueCode::Variant) else {
            return values;
        };
        for (name, value) in map {
            let here = loc.key(name);
            let Some(index) = self.params.iter().position(|p| &p.id == name) else {
                self.push(here.error(IssueCode::Variant).name("param", name));
                continue;
            };
            let param = &self.params[index];
            let (min, max) = (param.min, param.max);
            match read::number(value) {
                Some(v) if v >= min && v <= max => values[index] = v,
                other => {
                    let mut issue = here
                        .error(IssueCode::Variant)
                        .value("min", min)
                        .value("max", max);
                    if let Some(v) = other {
                        issue = issue.value("value", v);
                    }
                    self.push(issue);
                }
            }
        }
        values
    }

    fn layer_index(&self, id: &str) -> Option<u8> {
        self.decls
            .iter()
            .position(|d| d.id == id)
            .and_then(|i| u8::try_from(i).ok())
    }

    fn group_index(&self, id: &str) -> Option<u8> {
        self.group_ids
            .iter()
            .position(|g| g == id)
            .and_then(|i| u8::try_from(i).ok())
    }

    fn capabilities(&mut self, animations: &Animations, variants: &[Variant]) {
        let mut found: Vec<&'static str> = Vec::new();
        for layer in self.layers.iter().flatten() {
            if layer.mask.is_some() {
                found.push("nameplate_masks");
            }
            match &layer.content {
                LayerContent::Body(body) => {
                    if body.art.is_some() {
                        found.push("nameplate_body_art");
                    }
                    if body
                        .fill
                        .iter()
                        .chain(&body.rings)
                        .any(|f| matches!(f, super::types::Fill::Flag))
                    {
                        found.push("nameplate_flags");
                    }
                }
                LayerContent::Sprite(sprite) => {
                    if sprite.path.is_some() {
                        found.push("nameplate_paths");
                    }
                }
                LayerContent::Emitter(_) => found.push("nameplate_particles"),
                LayerContent::Material(_) => found.push("nameplate_materials"),
            }
        }
        if !animations.reactions.is_empty() {
            found.push("nameplate_reactions");
        }
        if !animations.one_shots.is_empty() {
            found.push("nameplate_one_shots");
        }
        if !animations.schedules.is_empty() {
            found.push("nameplate_schedules");
        }
        let clips = std::iter::once(&animations.looping)
            .chain(
                animations
                    .schedules
                    .iter()
                    .flat_map(|s| s.events.iter().map(|e| &e.clip)),
            )
            .chain(animations.reactions.iter().map(|r| &r.clip))
            .chain(animations.one_shots.iter().map(|o| &o.clip));
        let mut letters = false;
        let mut palette = false;
        for clip in clips {
            palette |= !clip.palette.is_empty();
            letters |= matches!(clip.letters, Some(super::types::LetterOverride::Program(_)));
        }
        for variant in variants {
            letters |= variant.text.motion.is_some();
            if variant.text.mode == TextMode::Owned {
                found.push("nameplate_owned_text");
            }
            if variant.flag.is_some() {
                found.push("nameplate_flags");
            }
        }
        if letters {
            found.push("nameplate_letters");
        }
        if palette {
            found.push("nameplate_palette_animation");
        }
        for cap in found {
            self.cap(cap);
        }
        let mut ordered: Vec<&'static str> = crate::container::CAPABILITIES
            .iter()
            .copied()
            .filter(|c| self.caps.contains(c))
            .collect();
        ordered.extend(
            self.caps
                .iter()
                .copied()
                .filter(|c| !crate::container::CAPABILITIES.contains(c)),
        );
        self.caps = ordered;

        let declared: Vec<String> = self.bundle.requires().to_vec();
        let missing: Vec<&'static str> = self
            .caps
            .iter()
            .copied()
            .filter(|c| !declared.iter().any(|d| d == c))
            .collect();
        let loc = Loc::new(MANIFEST, "requires");
        for cap in missing {
            self.soft(loc.error(IssueCode::Manifest).name("capability", cap));
        }
    }

    fn record(&mut self, animations: &Animations) -> WearerDeclarations {
        let one_shots = animations.one_shots.len().min(MAX_ONE_SHOTS);
        let speeds = animations
            .reactions
            .iter()
            .map(|r| &r.clip)
            .chain(animations.one_shots.iter().map(|o| &o.clip))
            .filter(|clip| clip.targets.iter().any(|t| t.channels.speed.is_some()))
            .count();
        let equip = animations
            .one_shots
            .iter()
            .any(|o| matches!(o.trigger, Trigger::Equip));
        let record = WearerDeclarations {
            variables: self.molang.interner.variable_count(),
            one_shots,
            effect_times: tail_slots(animations),
            effect_floats: 2 * speeds + usize::from(equip),
            channels: self.decls.len(),
            ..WearerDeclarations::default()
        };
        if let Err(e) = RecordLayout::new(record) {
            let loc = Loc::new(MANIFEST, "");
            let issue = match e {
                DeclarationError::TooManyVariables { declared, limit } => loc
                    .error(IssueCode::WearerVariablesOverBudget)
                    .value("count", declared as f64)
                    .value("limit", limit as f64),
                DeclarationError::TooManyOneShots { declared, limit } => loc
                    .error(IssueCode::WearerOneShotsOverBudget)
                    .value("count", declared as f64)
                    .value("limit", limit as f64),
                DeclarationError::OverBudget { bytes, budget } => loc
                    .error(IssueCode::WearerRecordOverBudget)
                    .value("bytes", bytes as f64)
                    .value("limit", budget as f64),
            };
            self.push(issue);
        }
        record
    }

    fn max_out_words(&self) -> u32 {
        let mut words = TAG_HEAD as u32 + QUADS_PER_NAMEPLATE * OP_WORDS as u32;
        for layer in self.layers.iter().flatten() {
            if let LayerContent::Material(m) = &layer.content {
                let uniforms = self
                    .materials
                    .get(usize::from(m.material))
                    .and_then(Option::as_ref)
                    .map_or(0, MaterialIr::uniform_words);
                let copies = if layer.mirror.is_some() { 2 } else { 1 };
                words += copies * (MAT_FIXED as u32 + CLIP_WORDS as u32 * MAX_CLIPS + uniforms);
            }
        }
        words += TEXT_FIXED as u32 + NAMEPLATE_LETTERS * (LETTER_WORDS as u32 + 1);
        words += BADGE_WORDS as u32;
        words += EVENT_WORDS as u32 * (self.decls.len() as u32 + 5);
        words
    }
}

fn letter_modes(manifest: &Map<String, Value>) -> bool {
    let letters = |text: Option<&Value>| {
        matches!(
            text.and_then(|t| t.get("mode")).and_then(Value::as_str),
            Some("motion" | "owned")
        )
    };
    letters(manifest.get("text"))
        || manifest
            .get("variants")
            .and_then(Value::as_object)
            .is_some_and(|v| v.values().any(|v| letters(v.get("text"))))
}

fn moves_plate(animations: &Animations) -> bool {
    std::iter::once(&animations.looping)
        .chain(
            animations
                .schedules
                .iter()
                .flat_map(|s| s.events.iter().map(|e| &e.clip)),
        )
        .chain(animations.reactions.iter().map(|r| &r.clip))
        .chain(animations.one_shots.iter().map(|o| &o.clip))
        .any(|clip| clip.targets.iter().any(|t| t.target == Target::Plate))
}

#[derive(Debug)]
pub struct MolangCheck {
    pub program: Program,
    pub warnings: Warnings,
    pub soft: Vec<Issue>,
}

pub fn check_molang(source: &str, scope: MolangUse) -> Result<MolangCheck, Issue> {
    check_in(source, scope, &mut Interner::default())
}

fn check_in(source: &str, scope: MolangUse, interner: &mut Interner) -> Result<MolangCheck, Issue> {
    if source.trim().is_empty() {
        return Err(Issue::error(IssueCode::MolangEmpty, ""));
    }
    let options = Options {
        rules: Rules::Strict,
        queries: scope_for(scope),
    };
    let mut warnings = Warnings::default();
    let program = compile_checked(source, interner, &options, &mut warnings).map_err(|e| {
        let code = if e.code == ErrorCode::UnknownQuery && known_elsewhere(source) {
            IssueCode::MolangQueryPlace
        } else {
            IssueCode::from(e.code)
        };
        Issue::error(code, "").name("message", e.message())
    })?;
    let mut soft = Vec::new();
    if flags_constants(scope) && is_constant(&program, interner) {
        soft.push(Issue::error(IssueCode::MolangConstant, ""));
    }
    Ok(MolangCheck {
        program,
        warnings,
        soft,
    })
}

fn flags_constants(scope: MolangUse) -> bool {
    matches!(scope, MolangUse::Channel | MolangUse::SlotColor)
}

fn known_elsewhere(source: &str) -> bool {
    let options = Options {
        rules: Rules::Strict,
        queries: known_scope(),
    };
    let mut interner = Interner::default();
    match compile_checked(source, &mut interner, &options, &mut Warnings::default()) {
        Ok(_) => true,
        Err(e) => e.code != ErrorCode::UnknownQuery,
    }
}

fn is_constant(program: &Program, interner: &Interner) -> bool {
    let reads = program.reads();
    if !reads.queries.is_empty() || !reads.variables.is_empty() {
        return false;
    }
    let mut first = Runtime::new(interner);
    first.set_seed(1);
    first.set_clock(0.0);
    let mut second = Runtime::new(interner);
    second.set_seed(0x9e37_79b9_7f4a_7c15);
    second.set_clock(1234.5);
    let a = program.eval(&mut first);
    let b = program.eval(&mut second);
    a.to_bits() == b.to_bits()
}

fn param_calls(source: &str) -> Vec<Option<String>> {
    let lower = source.to_ascii_lowercase();
    let name = NameplateQuery::VariantParam.name();
    let bytes = source.as_bytes();
    let mut calls = Vec::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let start = from + found;
        from = start + name.len();
        let before = start.checked_sub(1).map(|i| bytes[i]);
        if before != Some(b'.') {
            continue;
        }
        let mut i = from;
        let skip = |i: &mut usize| {
            while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
                *i += 1;
            }
        };
        skip(&mut i);
        if bytes.get(i) != Some(&b'(') {
            calls.push(None);
            continue;
        }
        i += 1;
        skip(&mut i);
        if bytes.get(i) != Some(&b'\'') {
            calls.push(None);
            continue;
        }
        let open = i + 1;
        let Some(len) = source[open..].find('\'') else {
            calls.push(None);
            continue;
        };
        let mut j = open + len + 1;
        skip(&mut j);
        if bytes.get(j) != Some(&b')') {
            calls.push(None);
            continue;
        }
        calls.push(Some(source[open..open + len].to_string()));
        from = j;
    }
    calls
}

fn reserved(id: &str) -> bool {
    RESERVED.contains(&id)
}

fn group_keys() -> &'static [&'static str] {
    &GROUP_KEYS
}

fn layer_entry_keys() -> &'static [&'static str] {
    &LAYER_ENTRY_KEYS
}

#[cfg(test)]
pub(crate) mod tests;
