use serde_json::{Map, Value};

use crate::container::is_id;
use crate::issues::IssueCode;
use crate::limits::{
    NAMEPLATE_BREAKPOINTS, NAMEPLATE_CACHED_STATES, NAMEPLATE_CORNER_RADIUS,
    NAMEPLATE_EMITTER_POOL, NAMEPLATE_FADE_STEPS, NAMEPLATE_LAPS, NAMEPLATE_LAYERS,
    NAMEPLATE_LOOP_FRAMES, NAMEPLATE_MIN_WIDTH, NAMEPLATE_OFFSET, NAMEPLATE_PADDING,
    NAMEPLATE_PARTICLE_GRAVITY, NAMEPLATE_PARTICLE_LIFETIME, NAMEPLATE_PARTICLE_SPEED,
    NAMEPLATE_PATH_POINTS, NAMEPLATE_PATH_POINTS_MIN, NAMEPLATE_PATH_SPEED, NAMEPLATE_PATH_STOPS,
    NAMEPLATE_PAUSE_FRAMES, NAMEPLATE_REPEAT_COPIES, NAMEPLATE_RINGS, NAMEPLATE_SPAWN_COUNT,
    NAMEPLATE_SPRITE_COLORS, NAMEPLATE_SPRITE_FRAMES, NAMEPLATE_SPRITE_SIZE, NAMEPLATE_WALKERS,
    NAMEPLATE_WOBBLE,
};

use super::super::geom::{Anchor, Edge};
use super::super::image::{Axis, ColorRef};
use super::super::material::graph::{build, GraphContext};
use super::super::types::{
    Body, BodyArt, Breakpoint, Depth, Ease, Emitter, Facings, Fade, Fill, Follow, Group, Hold, Key,
    Layer, LayerContent, Mask, MaterialLayer, Mirror, Of, ParticleFrames, Path, PathEnd, PathPoint,
    PathShape, PathSpeed, PathStop, Preset, Rate, Repeat, Spawn, SpawnShape, Sprite, SpriteLayer,
    Track, Value as KeyValue, Walkers, Wobble, SPRITE_CLEAR,
};
use super::read::{self, Loc};
use super::{group_keys, layer_entry_keys, Compiler, Decl, Kind, NAME_TARGET};

pub(super) const UNIT: f64 = 256.0;
const MAX_WIDTH: i64 = 4096;
const MAX_SPACING: i64 = 256;

const BODY_KEYS: [&str; 10] = [
    "kind",
    "of",
    "padding",
    "minWidth",
    "radius",
    "corners",
    "rings",
    "fill",
    "art",
    "breakpoints",
];
const SPRITE_LAYER_KEYS: [&str; 15] = [
    "kind",
    "sprite",
    "of",
    "anchor",
    "at",
    "pivot",
    "offset",
    "mirror",
    "mirrorDelay",
    "mirrorSeed",
    "timeline",
    "follow",
    "path",
    "repeat",
    "breakpoints",
];
const EMITTER_KEYS: [&str; 13] = [
    "kind",
    "sprites",
    "of",
    "spawn",
    "rate",
    "lifetime",
    "velocity",
    "gravity",
    "drift",
    "wobble",
    "frames",
    "fade",
    "breakpoints",
];
const MATERIAL_LAYER_KEYS: [&str; 7] = [
    "kind",
    "material",
    "of",
    "drift",
    "mirror",
    "mirrorDelay",
    "mirrorSeed",
];
const FILL_KEYS: [&str; 6] = ["color", "gradient", "blend", "sweep", "flag", "axis"];
const ART_KEYS: [&str; 8] = [
    "sprite", "left", "right", "mirror", "segment", "center", "top", "bottom",
];
const SPRITE_FILE_KEYS: [&str; 3] = ["colors", "rows", "frames"];
const HOLD_KEYS: [&str; 2] = ["frame", "hold"];
const PATH_KEYS: [&str; 10] = [
    "preset", "points", "follow", "speed", "laps", "end", "reverse", "stops", "frames", "walkers",
];
const POINT_KEYS: [&str; 3] = ["anchor", "offset", "pause"];
const STOP_KEYS: [&str; 2] = ["at", "pause"];
const WALKER_KEYS: [&str; 3] = ["count", "spacing", "even"];
const FACING_KEYS: [&str; 4] = ["right", "left", "up", "down"];
const REPEAT_KEYS: [&str; 4] = ["edge", "spacing", "min", "max"];
const BREAKPOINT_KEYS: [&str; 3] = ["min", "max", "sprite"];
const SPAWN_KEYS: [&str; 5] = ["shape", "anchor", "edge", "offset", "follow"];
const RATE_KEYS: [&str; 3] = ["every", "count", "jitter"];
const VELOCITY_KEYS: [&str; 2] = ["x", "y"];
const WOBBLE_KEYS: [&str; 3] = ["x", "y", "period"];
const FADE_KEYS: [&str; 3] = ["after", "to", "steps"];

fn follow(name: &str) -> Option<Follow> {
    match name {
        "outside" => Some(Follow::Outside),
        "line" => Some(Follow::Line),
        "inside" => Some(Follow::Inside),
        _ => None,
    }
}

fn block_anchor(name: &str) -> Option<Anchor> {
    Anchor::parse(name).filter(|a| !a.is_name())
}

impl Compiler<'_, '_> {
    pub(super) fn declare_layers(&mut self, manifest: &Map<String, Value>, root: &Loc) {
        let loc = root.key("layers");
        let Some(value) = manifest.get("layers") else {
            return;
        };
        let Some(items) = self.list(value, &loc, IssueCode::Layers) else {
            return;
        };
        if items.len() > NAMEPLATE_LAYERS as usize {
            self.push(
                loc.error(IssueCode::Layers)
                    .value("count", items.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_LAYERS)),
            );
            return;
        }
        for (i, item) in items.iter().enumerate() {
            let Some(entry) = self.object(item, &loc.index(i), IssueCode::Layers) else {
                continue;
            };
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let at = if is_id(&id) {
                loc.key(&id)
            } else {
                loc.index(i)
            };
            self.keys(&at, entry, layer_entry_keys());
            if !is_id(&id) {
                self.push(at.key("id").error(IssueCode::IdInvalid).name("id", &id));
                continue;
            }
            if self.layer_index(&id).is_some() {
                self.push(at.key("id").error(IssueCode::Layers).name("id", &id));
                continue;
            }
            let depth = self.choice(
                entry,
                "depth",
                &at,
                IssueCode::Layers,
                Depth::parse,
                Depth::Background,
            );
            let step = self.int(
                entry,
                "step",
                &at,
                IssueCode::Layers,
                (1, i64::from(NAMEPLATE_LOOP_FRAMES)),
                1,
            );
            if i64::from(self.loop_frames) % step != 0 {
                self.push(
                    at.key("step")
                        .error(IssueCode::Layers)
                        .value("step", step as f64)
                        .value("loop", f64::from(self.loop_frames)),
                );
            }
            self.decls.push(Decl {
                id,
                loc: at,
                file: String::new(),
                depth,
                step: step as u16,
                kind: None,
                json: None,
                path: false,
                laps: false,
                repeat: false,
                timeline: false,
                facings: false,
                sprite: None,
            });
            self.layers.push(None);
            self.keyed_frames.push(false);
        }
    }

    pub(super) fn read_layer_files(&mut self) {
        for i in 0..self.decls.len() {
            let from = self.decls[i].loc.path();
            let id = self.decls[i].id.clone();
            let Some(name) = self.bundle.id_path("layers", &id, "json", &from) else {
                continue;
            };
            self.decls[i].file = name.clone();
            let Some(json) = self.bundle.json(&name, &from) else {
                continue;
            };
            let root = Loc::new(&name, "");
            let Some(map) = self.object(&json, &root, IssueCode::Layer).cloned() else {
                continue;
            };
            let kind = map
                .get("kind")
                .and_then(Value::as_str)
                .and_then(Kind::parse);
            if kind.is_none() {
                let mut issue = root.key("kind").error(IssueCode::Layer);
                if let Some(name) = map.get("kind").and_then(Value::as_str) {
                    issue = issue.name("kind", name);
                }
                self.push(issue);
                continue;
            }
            let path = map.get("path").and_then(Value::as_object);
            let decl = &mut self.decls[i];
            decl.kind = kind;
            decl.path = path.is_some();
            decl.laps = path.is_some_and(|p| p.contains_key("laps"));
            decl.facings = path.is_some_and(|p| p.contains_key("frames"));
            decl.repeat = map.contains_key("repeat");
            decl.timeline = map.contains_key("timeline");
            decl.json = Some(map);
        }
    }

    pub(super) fn groups(&mut self, manifest: &Map<String, Value>, root: &Loc) {
        let loc = root.key("groups");
        let Some(value) = manifest.get("groups") else {
            return;
        };
        let Some(items) = self.list(value, &loc, IssueCode::Group).cloned() else {
            return;
        };
        let mut parents: Vec<Option<(String, Loc)>> = Vec::new();
        let mut claimed: Vec<(u8, String)> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let at = loc.index(i);
            let Some(entry) = self.object(item, &at, IssueCode::Group) else {
                continue;
            };
            self.keys(&at, entry, group_keys());
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !is_id(&id) {
                self.push(at.key("id").error(IssueCode::IdInvalid).name("id", &id));
                continue;
            }
            if self.layer_index(&id).is_some() || self.group_index(&id).is_some() {
                self.push(at.key("id").error(IssueCode::Group).name("id", &id));
                continue;
            }
            let mut layers = Vec::new();
            if let Some(value) = entry.get("layers") {
                let list_loc = at.key("layers");
                if let Some(members) = self.list(value, &list_loc, IssueCode::Group) {
                    for (j, member) in members.iter().enumerate() {
                        let here = list_loc.index(j);
                        let Some(index) = member.as_str().and_then(|m| self.layer_index(m)) else {
                            self.push(here.error(IssueCode::Group));
                            continue;
                        };
                        if let Some((_, owner)) = claimed.iter().find(|(l, _)| *l == index) {
                            let owner = owner.clone();
                            self.push(here.error(IssueCode::Group).name("group", &owner));
                            continue;
                        }
                        claimed.push((index, id.clone()));
                        layers.push(index);
                    }
                }
            }
            let breakpoints = match entry.get("breakpoints") {
                Some(value) => self.breakpoints(value, &at.key("breakpoints"), None),
                None => Vec::new(),
            };
            let parent = match entry.get("parent") {
                None => None,
                Some(Value::String(p)) => Some((p.clone(), at.key("parent"))),
                Some(_) => {
                    self.push(at.key("parent").error(IssueCode::Group));
                    None
                }
            };
            parents.push(parent);
            self.group_ids.push(id.clone());
            self.groups.push(Group {
                id,
                parent: None,
                layers,
                breakpoints,
            });
        }
        for (g, parent) in parents.into_iter().enumerate() {
            let Some((name, at)) = parent else {
                continue;
            };
            match self.group_index(&name) {
                Some(p) => self.groups[g].parent = Some(p),
                None => self.push(at.error(IssueCode::Group).name("group", &name)),
            }
        }
        for g in 0..self.groups.len() {
            let mut current = self.groups[g].parent;
            let mut hops = 0;
            while let Some(p) = current {
                if usize::from(p) == g {
                    let at = loc.index(g).key("parent");
                    self.push(at.error(IssueCode::Group));
                    self.groups[g].parent = None;
                    break;
                }
                hops += 1;
                if hops > self.groups.len() {
                    break;
                }
                current = self.groups[usize::from(p)].parent;
            }
        }
    }

    pub(super) fn layer_contents(&mut self) {
        for i in 0..self.decls.len() {
            let (Some(kind), Some(map)) = (self.decls[i].kind, self.decls[i].json.clone()) else {
                continue;
            };
            let root = Loc::new(&self.decls[i].file, "");
            let layer = match kind {
                Kind::Body => self.body_layer(i, &map, &root),
                Kind::Sprite => self.sprite_layer(i, &map, &root),
                Kind::Emitter => self.emitter_layer(i, &map, &root),
                Kind::Material => self.material_layer(i, &map, &root),
            };
            self.layers[i] = layer;
        }
    }

    fn layer(
        &self,
        i: usize,
        of: Of,
        mirror: Option<Mirror>,
        breakpoints: Vec<Breakpoint>,
        content: LayerContent,
    ) -> Layer {
        Layer {
            id: self.decls[i].id.clone(),
            depth: self.decls[i].depth,
            step: self.decls[i].step,
            mask: None,
            of,
            mirror,
            breakpoints,
            content,
        }
    }

    fn is_kind(&self, id: &str, kind: Kind) -> bool {
        self.layer_index(id)
            .is_some_and(|i| self.decls[usize::from(i)].kind == Some(kind))
    }

    fn body_of(&mut self, map: &Map<String, Value>, root: &Loc) -> Of {
        match map.get("of") {
            None => Of::Block,
            Some(Value::String(s)) if s == NAME_TARGET => Of::Name,
            Some(_) => {
                self.push(root.key("of").error(IssueCode::NameTarget));
                Of::Block
            }
        }
    }

    fn mirror(&mut self, map: &Map<String, Value>, root: &Loc) -> Option<Mirror> {
        let on = self.boolean(map, "mirror", root, IssueCode::Layer, false);
        let delay = self.int(
            map,
            "mirrorDelay",
            root,
            IssueCode::Animation,
            (0, i64::from(self.loop_frames)),
            0,
        );
        let seeded = self.boolean(map, "mirrorSeed", root, IssueCode::Animation, false);
        if !on && (map.contains_key("mirrorDelay") || map.contains_key("mirrorSeed")) {
            self.push(root.key("mirror").error(IssueCode::Animation));
        }
        on.then_some(Mirror {
            delay: delay as u16,
            seeded,
        })
    }

    fn body_layer(&mut self, i: usize, map: &Map<String, Value>, root: &Loc) -> Option<Layer> {
        self.keys(root, map, &BODY_KEYS);
        let of = self.body_of(map, root);
        if map.contains_key("breakpoints") {
            self.push(root.key("breakpoints").error(IssueCode::Widths));
        }
        let mut padding = [0u8; 4];
        if let Some(value) = map.get("padding") {
            let values: Option<Vec<i64>> = value
                .as_array()
                .filter(|a| a.len() == 4)
                .and_then(|a| a.iter().map(read::whole).collect());
            match values {
                Some(v)
                    if v.iter()
                        .all(|p| (0..=i64::from(NAMEPLATE_PADDING)).contains(p)) =>
                {
                    for (slot, p) in padding.iter_mut().zip(v) {
                        *slot = p as u8;
                    }
                }
                _ => self.push(
                    root.key("padding")
                        .error(IssueCode::Layer)
                        .value("limit", f64::from(NAMEPLATE_PADDING)),
                ),
            }
        }
        let min_width = self.int(
            map,
            "minWidth",
            root,
            IssueCode::Layer,
            (0, i64::from(NAMEPLATE_MIN_WIDTH)),
            0,
        );
        let radius_limit = i64::from(NAMEPLATE_CORNER_RADIUS);
        let radius = self.int(
            map,
            "radius",
            root,
            IssueCode::Corners,
            (0, radius_limit),
            0,
        ) as u8;
        let mut corners = [radius; 4];
        if let Some(value) = map.get("corners") {
            if map.contains_key("radius") {
                self.push(root.key("radius").error(IssueCode::CornersBoth));
            }
            let values: Option<Vec<i64>> = value
                .as_array()
                .filter(|a| a.len() == 4)
                .and_then(|a| a.iter().map(read::whole).collect());
            match values {
                Some(v) if v.iter().all(|c| (0..=radius_limit).contains(c)) => {
                    for (slot, c) in corners.iter_mut().zip(v) {
                        *slot = c as u8;
                    }
                }
                _ => self.push(
                    root.key("corners")
                        .error(IssueCode::Corners)
                        .value("limit", radius_limit as f64),
                ),
            }
        }
        let mut rings = Vec::new();
        if let Some(value) = map.get("rings") {
            let loc = root.key("rings");
            if let Some(items) = self.list(value, &loc, IssueCode::Layer).cloned() {
                if items.len() > NAMEPLATE_RINGS as usize {
                    self.push(
                        loc.error(IssueCode::Layer)
                            .value("count", items.len() as f64)
                            .value("limit", f64::from(NAMEPLATE_RINGS)),
                    );
                }
                for (r, item) in items.iter().enumerate() {
                    if let Some(fill) = self.fill(item, &loc.index(r)) {
                        rings.push(fill);
                    }
                }
            }
        }
        let fill = match map.get("fill") {
            Some(value) => self.fill(value, &root.key("fill")),
            None => None,
        };
        let art = match map.get("art") {
            Some(value) => self.body_art(value, &root.key("art")),
            None => None,
        };
        let body = Body {
            padding,
            min_width: min_width as u16,
            corners,
            rings,
            fill,
            art,
        };
        Some(self.layer(i, of, None, Vec::new(), LayerContent::Body(body)))
    }

    fn fill(&mut self, value: &Value, loc: &Loc) -> Option<Fill> {
        let map = self.object(value, loc, IssueCode::Layer)?;
        self.keys(loc, map, &FILL_KEYS);
        let flag = map.get("flag").is_some_and(|f| f == &Value::Bool(true));
        if map.get("flag").is_some_and(|f| !f.is_boolean()) {
            self.push(loc.key("flag").error(IssueCode::Layer));
        }
        let picked: Vec<&str> = ["color", "gradient", "blend", "sweep"]
            .into_iter()
            .filter(|k| map.contains_key(*k))
            .chain(flag.then_some("flag"))
            .collect();
        if picked.len() != 1 {
            self.push(
                loc.error(IssueCode::Layer)
                    .value("kinds", picked.len() as f64),
            );
            return None;
        }
        if map.contains_key("axis") && picked[0] != "sweep" {
            self.push(loc.key("axis").error(IssueCode::Layer));
        }
        match picked[0] {
            "flag" => {
                self.uses_flags = true;
                Some(Fill::Flag)
            }
            "color" => {
                let at = loc.key("color");
                match map["color"].as_str() {
                    Some(text) => self
                        .slot_color(text, &at, IssueCode::Layer)
                        .map(Fill::Color),
                    None => {
                        self.push(at.error(IssueCode::Layer));
                        None
                    }
                }
            }
            "gradient" => {
                let slot =
                    self.bare_slot(&map["gradient"], &loc.key("gradient"), IssueCode::Layer)?;
                self.band(slot);
                Some(Fill::Gradient { slot })
            }
            "blend" => {
                let slot = self.bare_slot(&map["blend"], &loc.key("blend"), IssueCode::Layer)?;
                Some(Fill::Blend { slot })
            }
            _ => {
                let slot = self.bare_slot(&map["sweep"], &loc.key("sweep"), IssueCode::Layer);
                let axis = match map.get("axis").and_then(Value::as_str) {
                    Some("x") => Some(Axis::X),
                    Some("y") => Some(Axis::Y),
                    _ => {
                        self.push(loc.key("axis").error(IssueCode::Layer));
                        None
                    }
                };
                let slot = slot?;
                self.band(slot);
                Some(Fill::Sweep { slot, axis: axis? })
            }
        }
    }

    fn band(&mut self, slot: u8) {
        if !self.banded.contains(&slot) {
            self.banded.push(slot);
        }
    }

    fn body_art(&mut self, value: &Value, loc: &Loc) -> Option<BodyArt> {
        let map = self.object(value, loc, IssueCode::BodyArt)?;
        self.keys(loc, map, &ART_KEYS);
        let Some(id) = self.string(map, "sprite", loc, IssueCode::BodyArt) else {
            self.push(loc.key("sprite").error(IssueCode::BodyArt));
            return None;
        };
        let sprite = self.sprite(id, &loc.key("sprite"))?;
        let (width, height, frames) = {
            let s = self.sprites[usize::from(sprite)].as_ref()?;
            (i64::from(s.width), i64::from(s.height), s.frames.len())
        };
        if frames != 1 {
            self.push(
                loc.key("sprite")
                    .error(IssueCode::BodyArt)
                    .value("frames", frames as f64),
            );
            return None;
        }
        let left = self.int(map, "left", loc, IssueCode::BodyArt, (0, width), 0);
        let right = self.int(map, "right", loc, IssueCode::BodyArt, (0, width), 0);
        let mirror = self.boolean(map, "mirror", loc, IssueCode::BodyArt, false);
        if left + right > width {
            self.push(loc.error(IssueCode::BodyArt).value("width", width as f64));
        }
        if mirror && (right != 0 || left < 1) {
            self.push(loc.key("mirror").error(IssueCode::BodyArt));
        }
        let columns = |c: &mut Self, key: &str| -> Option<[u8; 2]> {
            let value = map.get(key)?;
            match read::pair_i64(value) {
                Some([a, b]) if 0 <= a && a <= b && b <= width => Some([a as u8, b as u8]),
                _ => {
                    c.push(
                        loc.key(key)
                            .error(IssueCode::BodyArt)
                            .value("width", width as f64),
                    );
                    None
                }
            }
        };
        let segment = columns(self, "segment");
        let center = columns(self, "center");
        let right_width = if mirror { 0 } else { right };
        let segment = segment.or_else(|| {
            (!map.contains_key("center")).then_some([left as u8, (width - right_width) as u8])
        });
        let top = self.int(
            map,
            "top",
            loc,
            IssueCode::BodyArt,
            (0, height),
            (height - 1) / 2,
        );
        let bottom = self.int(
            map,
            "bottom",
            loc,
            IssueCode::BodyArt,
            (0, height),
            height - 1 - top,
        );
        if top + bottom > height - 1 {
            self.push(loc.error(IssueCode::BodyArt).value("height", height as f64));
        }
        Some(BodyArt {
            sprite,
            left: left as u8,
            right: right_width as u8,
            mirror,
            segment,
            center,
            top: top as u8,
            bottom: bottom as u8,
        })
    }

    pub(super) fn sprite(&mut self, id: &str, from: &Loc) -> Option<u16> {
        if let Some(i) = self.sprite_ids.iter().position(|s| s == id) {
            return self.sprites[i].as_ref().map(|_| i as u16);
        }
        let name = self.bundle.id_path("sprites", id, "json", &from.path())?;
        let index = self.sprite_ids.len();
        self.sprite_ids.push(id.to_string());
        self.sprites.push(None);
        let json = self.bundle.json(&name, &from.path())?;
        let sprite = self.sprite_file(id, &json, &Loc::new(&name, ""));
        let ok = sprite.is_some();
        self.sprites[index] = sprite;
        ok.then_some(index as u16)
    }

    fn sprite_file(&mut self, id: &str, json: &Value, root: &Loc) -> Option<Sprite> {
        let map = self.object(json, root, IssueCode::Sprite)?;
        self.keys(root, map, &SPRITE_FILE_KEYS);
        let colors_loc = root.key("colors");
        let mut keys: Vec<char> = Vec::new();
        let mut colors: Vec<ColorRef> = Vec::new();
        let mut ok = true;
        match map.get("colors").and_then(Value::as_object) {
            Some(entries) => {
                if entries.len() > NAMEPLATE_SPRITE_COLORS as usize {
                    self.push(
                        colors_loc
                            .error(IssueCode::Sprite)
                            .value("count", entries.len() as f64)
                            .value("limit", f64::from(NAMEPLATE_SPRITE_COLORS)),
                    );
                    ok = false;
                }
                for (key, value) in entries {
                    let at = colors_loc.key(key);
                    let mut chars = key.chars();
                    let (Some(c), None) = (chars.next(), chars.next()) else {
                        self.push(at.error(IssueCode::Sprite).name("key", key));
                        ok = false;
                        continue;
                    };
                    if c == '.' || c.is_whitespace() {
                        self.push(at.error(IssueCode::Sprite).name("key", key));
                        ok = false;
                        continue;
                    }
                    match value
                        .as_str()
                        .and_then(|t| self.slot_color(t, &at, IssueCode::Sprite))
                    {
                        Some(color) => {
                            keys.push(c);
                            colors.push(color);
                        }
                        None => {
                            if value.as_str().is_none() {
                                self.push(at.error(IssueCode::Sprite));
                            }
                            ok = false;
                        }
                    }
                }
            }
            None => {
                self.push(colors_loc.error(IssueCode::Sprite));
                ok = false;
            }
        }
        let frames: Option<Vec<Vec<String>>> = match (map.get("rows"), map.get("frames")) {
            (Some(rows), None) => strings(rows).map(|r| vec![r]),
            (None, Some(frames)) => frames
                .as_array()
                .and_then(|f| f.iter().map(strings).collect()),
            _ => {
                self.push(root.error(IssueCode::Sprite));
                return None;
            }
        };
        let frames_loc = root.key(if map.contains_key("rows") {
            "rows"
        } else {
            "frames"
        });
        let Some(frames) = frames else {
            self.push(frames_loc.error(IssueCode::Sprite));
            return None;
        };
        if frames.is_empty() || frames.len() > NAMEPLATE_SPRITE_FRAMES as usize {
            self.push(
                frames_loc
                    .error(IssueCode::Sprite)
                    .value("count", frames.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_SPRITE_FRAMES)),
            );
            return None;
        }
        let height = frames[0].len();
        let width = frames[0].first().map_or(0, |r| r.chars().count());
        let size = NAMEPLATE_SPRITE_SIZE as usize;
        if !(1..=size).contains(&width) || !(1..=size).contains(&height) {
            self.push(
                frames_loc
                    .error(IssueCode::Sprite)
                    .value("width", width as f64)
                    .value("height", height as f64)
                    .value("limit", size as f64),
            );
            return None;
        }
        let mut pixels = Vec::with_capacity(frames.len());
        for (f, rows) in frames.iter().enumerate() {
            let at = if map.contains_key("rows") {
                frames_loc.clone()
            } else {
                frames_loc.index(f)
            };
            if rows.len() != height || rows.iter().any(|r| r.chars().count() != width) {
                self.push(
                    at.error(IssueCode::Sprite)
                        .value("width", width as f64)
                        .value("height", height as f64),
                );
                ok = false;
                continue;
            }
            let mut frame = Vec::with_capacity(width * height);
            for row in rows {
                for c in row.chars() {
                    if c == '.' {
                        frame.push(SPRITE_CLEAR);
                    } else if let Some(k) = keys.iter().position(|k| *k == c) {
                        frame.push(k as u8);
                    } else {
                        self.push(at.error(IssueCode::Sprite).name("key", &c.to_string()));
                        ok = false;
                        break;
                    }
                }
            }
            pixels.push(frame);
        }
        ok.then(|| Sprite {
            id: id.to_string(),
            width: width as u8,
            height: height as u8,
            colors,
            frames: pixels,
        })
    }

    fn frame_count(&self, sprite: u16) -> usize {
        self.sprites
            .get(usize::from(sprite))
            .and_then(Option::as_ref)
            .map_or(0, |s| s.frames.len())
    }

    fn sprite_layer(&mut self, i: usize, map: &Map<String, Value>, root: &Loc) -> Option<Layer> {
        self.keys(root, map, &SPRITE_LAYER_KEYS);
        let sprite = match self.string(map, "sprite", root, IssueCode::Sprite) {
            Some(id) => self.sprite(id, &root.key("sprite")),
            None => {
                self.push(root.key("sprite").error(IssueCode::Sprite));
                None
            }
        };
        self.decls[i].sprite = sprite;
        let frames = sprite.map_or(0, |s| self.frame_count(s));
        let has_path = map.contains_key("path");
        let of = match map.get("of") {
            None => Of::Block,
            Some(Value::String(s)) if s == NAME_TARGET => {
                if has_path {
                    self.push(root.key("of").error(IssueCode::NameTarget));
                }
                Of::Name
            }
            Some(Value::String(s)) if self.is_kind(s, Kind::Body) => {
                Of::Layer(self.layer_index(s)?)
            }
            Some(_) => {
                self.push(root.key("of").error(IssueCode::Layer));
                Of::Block
            }
        };
        let anchor = self.choice(
            map,
            "anchor",
            root,
            IssueCode::Anchor,
            Anchor::parse,
            Anchor::TopLeft,
        );
        if anchor.is_name() && map.contains_key("of") {
            self.push(root.key("of").error(IssueCode::Anchor));
        }
        let at = match map.get("at") {
            None => None,
            Some(value) => match read::number(value) {
                Some(v) if anchor.takes_at() && (0.0..=1.0).contains(&v) => Some(v),
                _ => {
                    self.push(root.key("at").error(IssueCode::Anchor));
                    None
                }
            },
        };
        let pivot = self.choice(
            map,
            "pivot",
            root,
            IssueCode::Anchor,
            block_anchor,
            anchor.default_pivot(),
        );
        let offset = self.offset(
            map,
            "offset",
            root,
            IssueCode::Layer,
            i64::from(NAMEPLATE_OFFSET),
        );
        let mirror = self.mirror(map, root);
        let has_repeat = map.contains_key("repeat");
        if mirror.is_some() && (has_path || has_repeat) {
            self.push(root.key("mirror").error(IssueCode::Layer));
        }
        let timeline = match map.get("timeline") {
            Some(value) => self.timeline(value, &root.key("timeline"), i, frames),
            None => Vec::new(),
        };
        let follow_at = root.key("follow");
        let sprite_follow = match map.get("follow") {
            None => None,
            Some(value) => {
                let parsed = value
                    .as_str()
                    .and_then(follow)
                    .filter(|f| *f != Follow::Inside);
                if parsed.is_none() || anchor.is_name() || has_path {
                    self.push(follow_at.error(IssueCode::Follow));
                }
                parsed
            }
        };
        let path = match map.get("path") {
            Some(value) => self.path(value, &root.key("path"), i, frames),
            None => None,
        };
        let repeat = match map.get("repeat") {
            Some(value) => self.repeat(value, &root.key("repeat")),
            None => None,
        };
        if has_path && has_repeat {
            self.push(root.key("repeat").error(IssueCode::Repeat));
        }
        if sprite_follow.is_some()
            && repeat.is_some_and(|r| matches!(r.edge, Edge::Left | Edge::Right))
        {
            self.push(root.key("repeat").key("edge").error(IssueCode::Follow));
        }
        let breakpoints = match map.get("breakpoints") {
            Some(value) => self.breakpoints(value, &root.key("breakpoints"), Some(frames)),
            None => Vec::new(),
        };
        let content = SpriteLayer {
            sprite: sprite?,
            anchor,
            at,
            pivot,
            offset,
            timeline,
            follow: sprite_follow,
            path,
            repeat,
        };
        Some(self.layer(i, of, mirror, breakpoints, LayerContent::Sprite(content)))
    }

    fn timeline(&mut self, value: &Value, loc: &Loc, layer: usize, frames: usize) -> Vec<Hold> {
        let Some(items) = self.list(value, loc, IssueCode::Animation).cloned() else {
            return Vec::new();
        };
        let cycle = i64::from(self.loop_frames) / i64::from(self.decls[layer].step.max(1));
        let mut holds = Vec::new();
        let mut total = 0;
        for (h, item) in items.iter().enumerate() {
            let at = loc.index(h);
            let Some(entry) = self.object(item, &at, IssueCode::Animation) else {
                continue;
            };
            self.keys(&at, entry, &HOLD_KEYS);
            let last = (frames.max(1) - 1) as i64;
            let frame = self.int(entry, "frame", &at, IssueCode::Animation, (0, last), 0);
            if !entry.contains_key("frame") || !entry.contains_key("hold") {
                self.push(at.error(IssueCode::Animation));
                continue;
            }
            let hold = self.int(
                entry,
                "hold",
                &at,
                IssueCode::Animation,
                (1, cycle.max(1)),
                1,
            );
            total += hold;
            holds.push(Hold {
                frame: frame as u16,
                hold: hold as u16,
            });
        }
        if total != cycle {
            self.push(
                loc.error(IssueCode::Animation)
                    .value("holds", total as f64)
                    .value("cycle", cycle as f64),
            );
        }
        holds
    }

    fn path(&mut self, value: &Value, loc: &Loc, layer: usize, frames: usize) -> Option<Path> {
        let map = self.object(value, loc, IssueCode::Path)?;
        self.keys(loc, map, &PATH_KEYS);
        let cycle = i64::from(self.loop_frames) / i64::from(self.decls[layer].step.max(1));
        let shape = match (map.get("preset"), map.get("points")) {
            (Some(preset), None) => {
                let parsed = match preset.as_str() {
                    Some("top") => Some(Preset::Top),
                    Some("bottom") => Some(Preset::Bottom),
                    Some("perimeter") => Some(Preset::Perimeter),
                    _ => None,
                };
                if parsed.is_none() {
                    self.push(loc.key("preset").error(IssueCode::Path));
                }
                parsed.map(PathShape::Preset)
            }
            (None, Some(points)) => self
                .points(points, &loc.key("points"))
                .map(PathShape::Points),
            _ => {
                self.push(loc.error(IssueCode::Path));
                None
            }
        };
        let preset = matches!(shape, Some(PathShape::Preset(_)));
        let path_follow = match map.get("follow") {
            None => None,
            Some(value) => {
                let parsed = value.as_str().and_then(follow);
                if parsed.is_none() || !preset {
                    self.push(loc.key("follow").error(IssueCode::Follow));
                }
                parsed
            }
        };
        let end = self.choice(
            map,
            "end",
            loc,
            IssueCode::Path,
            |name| match name {
                "wrap" => Some(PathEnd::Wrap),
                "bounce" => Some(PathEnd::Bounce),
                "stop" => Some(PathEnd::Stop),
                _ => None,
            },
            PathEnd::Wrap,
        );
        let reverse = self.boolean(map, "reverse", loc, IssueCode::Path, false);
        let timing = IssueCode::PathTiming;
        let pauses = match &shape {
            Some(PathShape::Points(points)) => points.iter().map(|p| i64::from(p.pause)).sum(),
            _ => 0,
        };
        let (speed, keyed) = match (map.get("speed"), map.get("laps")) {
            (Some(_), Some(_)) => {
                self.push(loc.key("laps").error(timing));
                (PathSpeed::Laps(1), false)
            }
            (None, Some(_)) => {
                let laps = self.int(map, "laps", loc, timing, (1, i64::from(NAMEPLATE_LAPS)), 1);
                (PathSpeed::Laps(laps as u8), false)
            }
            (Some(value), None) => self.path_speed(value, &loc.key("speed"), cycle),
            (None, None) => (
                PathSpeed::Track(Track {
                    keys: vec![Key {
                        frame: 0,
                        value: KeyValue::Fixed(UNIT as i32),
                        ease: Ease::Step,
                    }],
                }),
                false,
            ),
        };
        if keyed && pauses > 0 {
            self.push(loc.key("speed").error(timing));
        }
        let mut stops = Vec::new();
        if let Some(value) = map.get("stops") {
            let at = loc.key("stops");
            if !preset || keyed {
                self.push(at.error(timing));
            }
            if let Some(items) = self.list(value, &at, timing).cloned() {
                if items.is_empty() || items.len() > NAMEPLATE_PATH_STOPS as usize {
                    self.push(
                        at.error(timing)
                            .value("count", items.len() as f64)
                            .value("limit", f64::from(NAMEPLATE_PATH_STOPS)),
                    );
                }
                let mut last = -1.0;
                for (s, item) in items.iter().enumerate() {
                    let here = at.index(s);
                    let Some(entry) = self.object(item, &here, timing) else {
                        continue;
                    };
                    self.keys(&here, entry, &STOP_KEYS);
                    let stop_at = self.num(entry, "at", &here, timing, (0.0, 1.0), 0.0);
                    if !entry.contains_key("at") || stop_at <= last {
                        self.push(here.key("at").error(timing));
                    }
                    last = stop_at;
                    let pause = self.int(
                        entry,
                        "pause",
                        &here,
                        timing,
                        (1, i64::from(NAMEPLATE_PAUSE_FRAMES)),
                        1,
                    );
                    stops.push(PathStop {
                        at: stop_at,
                        pause: pause as u16,
                    });
                }
            }
        }
        let waits = pauses + stops.iter().map(|s| i64::from(s.pause)).sum::<i64>();
        if let PathSpeed::Laps(laps) = speed {
            let laps = i64::from(laps);
            if end == PathEnd::Stop {
                self.push(loc.key("laps").error(timing));
            } else if waits > 0 {
                let total = if end == PathEnd::Bounce {
                    2 * waits
                } else {
                    waits
                };
                if cycle % laps != 0 || cycle / laps - total < 1 {
                    self.push(
                        loc.key("laps")
                            .error(timing)
                            .value("laps", laps as f64)
                            .value("cycle", cycle as f64)
                            .value("pauses", total as f64),
                    );
                }
            }
        }
        let facings = match map.get("frames") {
            Some(value) => self.facings(value, &loc.key("frames"), frames),
            None => Facings::default(),
        };
        let walkers = match map.get("walkers") {
            Some(value) => self.walkers(value, &loc.key("walkers"), end),
            None => Walkers::default(),
        };
        Some(Path {
            shape: shape?,
            follow: path_follow,
            speed,
            end,
            reverse,
            stops,
            facings,
            walkers,
        })
    }

    fn points(&mut self, value: &Value, loc: &Loc) -> Option<Vec<PathPoint>> {
        let items = self.list(value, loc, IssueCode::Path)?.clone();
        let (min, max) = (
            NAMEPLATE_PATH_POINTS_MIN as usize,
            NAMEPLATE_PATH_POINTS as usize,
        );
        if items.len() < min || items.len() > max {
            self.push(
                loc.error(IssueCode::Path)
                    .value("count", items.len() as f64)
                    .value("min", min as f64)
                    .value("max", max as f64),
            );
            return None;
        }
        let mut points = Vec::new();
        for (p, item) in items.iter().enumerate() {
            let at = loc.index(p);
            let Some(entry) = self.object(item, &at, IssueCode::Path) else {
                continue;
            };
            self.keys(&at, entry, &POINT_KEYS);
            if !entry.contains_key("anchor") {
                self.push(at.key("anchor").error(IssueCode::Path));
            }
            let anchor = self.choice(
                entry,
                "anchor",
                &at,
                IssueCode::Path,
                block_anchor,
                Anchor::TopLeft,
            );
            let offset = self.offset(
                entry,
                "offset",
                &at,
                IssueCode::Path,
                i64::from(NAMEPLATE_OFFSET),
            );
            let pause = self.int(
                entry,
                "pause",
                &at,
                IssueCode::Path,
                (0, i64::from(NAMEPLATE_PAUSE_FRAMES)),
                0,
            );
            points.push(PathPoint {
                anchor,
                offset,
                pause: pause as u16,
            });
        }
        Some(points)
    }

    fn path_speed(&mut self, value: &Value, loc: &Loc, cycle: i64) -> (PathSpeed, bool) {
        let limit = f64::from(NAMEPLATE_PATH_SPEED);
        if let Some(v) = read::number(value) {
            if read::fixed_point(v, UNIT) < 1 || v > limit {
                self.push(
                    loc.error(IssueCode::SpeedRange)
                        .value("value", v)
                        .value("limit", limit),
                );
            }
            let fixed = read::fixed_point(v.clamp(0.0, limit), UNIT);
            let track = Track {
                keys: vec![Key {
                    frame: 0,
                    value: KeyValue::Fixed(fixed),
                    ease: Ease::Step,
                }],
            };
            return (PathSpeed::Track(track), false);
        }
        let track = self.speed_keys(value, loc, cycle, (0.0, limit));
        (PathSpeed::Track(track.unwrap_or_default()), true)
    }

    pub(super) fn speed_keys(
        &mut self,
        value: &Value,
        loc: &Loc,
        length: i64,
        range: (f64, f64),
    ) -> Option<Track> {
        let Some(items) = value.as_array().filter(|a| !a.is_empty()) else {
            self.push(loc.error(IssueCode::Path));
            return None;
        };
        let mut keys = Vec::new();
        let mut last = -1;
        let mut ok = true;
        for (k, item) in items.iter().enumerate() {
            let at = loc.index(k);
            let key = item.as_array().filter(|a| (2..=3).contains(&a.len()));
            let frame = key.and_then(|a| read::whole(&a[0]));
            let speed = key.and_then(|a| read::number(&a[1]));
            let ease = key.map(|a| a.get(2).map(|e| e.as_str().and_then(super::anim::ease)));
            let (Some(frame), Some(speed), Some(ease)) = (frame, speed, ease) else {
                self.push(at.error(IssueCode::Animation));
                ok = false;
                continue;
            };
            let Some(ease) = ease.unwrap_or(Some(Ease::Step)) else {
                self.push(at.error(IssueCode::Animation));
                ok = false;
                continue;
            };
            if !self.key_frame(&at, k, frame, last, length) {
                ok = false;
            }
            last = frame;
            if speed < range.0 || speed > range.1 {
                self.push(
                    at.error(IssueCode::SpeedRange)
                        .value("value", speed)
                        .value("min", range.0)
                        .value("max", range.1),
                );
                ok = false;
            }
            keys.push(Key {
                frame: frame.clamp(0, i64::from(u16::MAX)) as u16,
                value: KeyValue::Fixed(read::fixed_point(speed, UNIT)),
                ease,
            });
        }
        ok.then_some(Track { keys })
    }

    pub(super) fn key_frame(
        &mut self,
        at: &Loc,
        index: usize,
        frame: i64,
        last: i64,
        length: i64,
    ) -> bool {
        let code = if index == 0 && frame != 0 {
            IssueCode::KeyStart
        } else if index > 0 && frame <= last {
            IssueCode::KeyOrder
        } else if frame < 0 || frame >= length {
            IssueCode::KeyRange
        } else {
            return true;
        };
        self.push(
            at.error(code)
                .value("frame", frame as f64)
                .value("length", length as f64),
        );
        false
    }

    fn facings(&mut self, value: &Value, loc: &Loc, frames: usize) -> Facings {
        let mut facings = Facings::default();
        let Some(map) = self.object(value, loc, IssueCode::Path) else {
            return facings;
        };
        self.keys(loc, map, &FACING_KEYS);
        if !map.contains_key("left") && !map.contains_key("right") {
            self.push(loc.error(IssueCode::Path));
        }
        for (name, list) in map {
            let at = loc.key(name);
            let parsed: Option<Vec<u16>> = list.as_array().and_then(|items| {
                items
                    .iter()
                    .map(|v| {
                        read::whole(v)
                            .filter(|f| *f >= 0 && (*f as usize) < frames)
                            .map(|f| f as u16)
                    })
                    .collect()
            });
            let Some(parsed) =
                parsed.filter(|p| !p.is_empty() && p.len() <= NAMEPLATE_SPRITE_FRAMES as usize)
            else {
                self.push(at.error(IssueCode::Path).value("frames", frames as f64));
                continue;
            };
            match name.as_str() {
                "right" => facings.right = parsed,
                "left" => facings.left = parsed,
                "up" => facings.up = parsed,
                "down" => facings.down = parsed,
                _ => {}
            }
        }
        facings
    }

    fn walkers(&mut self, value: &Value, loc: &Loc, end: PathEnd) -> Walkers {
        let Some(map) = self.object(value, loc, IssueCode::Path) else {
            return Walkers::default();
        };
        self.keys(loc, map, &WALKER_KEYS);
        let count = self.int(
            map,
            "count",
            loc,
            IssueCode::Path,
            (1, i64::from(NAMEPLATE_WALKERS)),
            1,
        );
        let spacing = self.int(map, "spacing", loc, IssueCode::Path, (0, MAX_SPACING), 0);
        let even = self.boolean(map, "even", loc, IssueCode::Path, false);
        if even && (spacing != 0 || end == PathEnd::Stop) {
            self.push(loc.key("even").error(IssueCode::PathTiming));
        }
        Walkers {
            count: count as u8,
            spacing: spacing as u16,
            even,
        }
    }

    fn repeat(&mut self, value: &Value, loc: &Loc) -> Option<Repeat> {
        let map = self.object(value, loc, IssueCode::Repeat)?;
        self.keys(loc, map, &REPEAT_KEYS);
        if !map.contains_key("edge") || !map.contains_key("spacing") {
            self.push(loc.error(IssueCode::Repeat));
        }
        let edge = self.choice(map, "edge", loc, IssueCode::Repeat, Edge::parse, Edge::Top);
        let spacing = self.int(map, "spacing", loc, IssueCode::Repeat, (1, MAX_SPACING), 1);
        let copies = i64::from(NAMEPLATE_REPEAT_COPIES);
        let min = self.int(map, "min", loc, IssueCode::Repeat, (1, copies), 1);
        let max = self.int(
            map,
            "max",
            loc,
            IssueCode::Repeat,
            (min, copies),
            min.max(1),
        );
        Some(Repeat {
            edge,
            spacing: spacing as u16,
            min: min as u8,
            max: max as u8,
        })
    }

    pub(super) fn breakpoints(
        &mut self,
        value: &Value,
        loc: &Loc,
        frames: Option<usize>,
    ) -> Vec<Breakpoint> {
        let Some(items) = self.list(value, loc, IssueCode::Widths).cloned() else {
            return Vec::new();
        };
        if items.is_empty() || items.len() > NAMEPLATE_BREAKPOINTS as usize {
            self.push(
                loc.error(IssueCode::Widths)
                    .value("count", items.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_BREAKPOINTS)),
            );
        }
        let mut breakpoints = Vec::new();
        for (b, item) in items.iter().enumerate() {
            let at = loc.index(b);
            let Some(entry) = self.object(item, &at, IssueCode::Widths) else {
                continue;
            };
            self.keys(&at, entry, &BREAKPOINT_KEYS);
            let min = self.int(entry, "min", &at, IssueCode::Widths, (0, MAX_WIDTH), 0);
            let max = entry.get("max").map(|_| {
                self.int(
                    entry,
                    "max",
                    &at,
                    IssueCode::Widths,
                    (min + 1, MAX_WIDTH),
                    MAX_WIDTH,
                )
            });
            let sprite = match entry.get("sprite") {
                None => None,
                Some(value) => match (frames, value.as_str()) {
                    (Some(frames), Some(id)) => {
                        let sprite = self.sprite(id, &at.key("sprite"));
                        if let Some(s) = sprite {
                            let count = self.frame_count(s);
                            if count != frames {
                                self.push(
                                    at.key("sprite")
                                        .error(IssueCode::Widths)
                                        .value("frames", count as f64)
                                        .value("expected", frames as f64),
                                );
                            }
                        }
                        sprite
                    }
                    _ => {
                        self.push(at.key("sprite").error(IssueCode::Widths));
                        None
                    }
                },
            };
            if min > 0 {
                self.breakpoint_edges.push(min as i32);
            }
            if let Some(max) = max {
                self.breakpoint_edges.push(max as i32);
            }
            breakpoints.push(Breakpoint {
                min: min as i32,
                max: max.map(|m| m as i32),
                sprite,
            });
        }
        breakpoints
    }

    fn emitter_layer(&mut self, i: usize, map: &Map<String, Value>, root: &Loc) -> Option<Layer> {
        self.keys(root, map, &EMITTER_KEYS);
        let code = IssueCode::Emitter;
        let mut pool = Vec::new();
        let sprites_loc = root.key("sprites");
        let ids = match map.get("sprites") {
            Some(value) => self
                .list(value, &sprites_loc, code)
                .cloned()
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if ids.is_empty() || ids.len() > NAMEPLATE_EMITTER_POOL as usize {
            self.push(
                sprites_loc
                    .error(code)
                    .value("count", ids.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_EMITTER_POOL)),
            );
        }
        let mut states = 0;
        for (s, id) in ids.iter().enumerate() {
            let at = sprites_loc.index(s);
            match id.as_str() {
                Some(id) => {
                    if let Some(sprite) = self.sprite(id, &at) {
                        states += self.frame_count(sprite);
                        pool.push(sprite);
                    }
                }
                None => self.push(at.error(code)),
            }
        }
        if states > NAMEPLATE_CACHED_STATES as usize {
            self.push(
                sprites_loc
                    .error(IssueCode::OverStates)
                    .value("frames", states as f64)
                    .value("limit", f64::from(NAMEPLATE_CACHED_STATES)),
            );
        }
        let of = match map.get("of") {
            None => Of::Block,
            Some(Value::String(s)) if self.is_kind(s, Kind::Body) => {
                Of::Layer(self.layer_index(s)?)
            }
            Some(Value::String(s)) if self.is_kind(s, Kind::Sprite) => {
                let index = self.layer_index(s)?;
                let decl = &self.decls[usize::from(index)];
                if decl.path || decl.repeat {
                    self.push(root.key("of").error(code));
                }
                Of::Layer(index)
            }
            Some(_) => {
                self.push(root.key("of").error(code));
                Of::Block
            }
        };
        let spawn = match map.get("spawn") {
            Some(value) => self.spawn(value, &root.key("spawn"))?,
            None => Spawn {
                shape: SpawnShape::Point,
                anchor: Anchor::TopLeft,
                edge: Edge::Top,
                offset: [0, 0],
                follow: None,
            },
        };
        let rate = match map.get("rate") {
            Some(value) => {
                let at = root.key("rate");
                let rate = self.object(value, &at, code)?;
                self.keys(&at, rate, &RATE_KEYS);
                if !rate.contains_key("every") {
                    self.push(at.key("every").error(code));
                }
                let every = self.int(
                    rate,
                    "every",
                    &at,
                    code,
                    (1, i64::from(NAMEPLATE_LOOP_FRAMES)),
                    1,
                );
                let count = self.int(
                    rate,
                    "count",
                    &at,
                    code,
                    (1, i64::from(NAMEPLATE_SPAWN_COUNT)),
                    1,
                );
                let jitter = self.int(rate, "jitter", &at, code, (0, every - 1), 0);
                Some(Rate {
                    every: every as u16,
                    count: count as u8,
                    jitter: jitter as u16,
                })
            }
            None => None,
        };
        let lifetime = match map.get("lifetime") {
            None => [1, 1],
            Some(value) => match read::pair_i64(value) {
                Some([a, b]) if 1 <= a && a <= b && b <= i64::from(NAMEPLATE_PARTICLE_LIFETIME) => {
                    [a as u16, b as u16]
                }
                _ => {
                    self.push(
                        root.key("lifetime")
                            .error(code)
                            .value("limit", f64::from(NAMEPLATE_PARTICLE_LIFETIME)),
                    );
                    [1, 1]
                }
            },
        };
        let speed = f64::from(NAMEPLATE_PARTICLE_SPEED);
        let (velocity_x, velocity_y) = match map.get("velocity") {
            None => ([0, 0], [0, 0]),
            Some(value) => {
                let at = root.key("velocity");
                let velocity = self.object(value, &at, code)?;
                self.keys(&at, velocity, &VELOCITY_KEYS);
                (
                    self.range(velocity.get("x"), &at.key("x"), speed, true),
                    self.range(velocity.get("y"), &at.key("y"), speed, true),
                )
            }
        };
        let gravity = self.range(
            map.get("gravity"),
            &root.key("gravity"),
            f64::from(NAMEPLATE_PARTICLE_GRAVITY),
            false,
        );
        let drift = self.range(map.get("drift"), &root.key("drift"), speed, false);
        let wobble = match map.get("wobble") {
            None => None,
            Some(value) => {
                let at = root.key("wobble");
                let wobble = self.object(value, &at, code)?;
                self.keys(&at, wobble, &WOBBLE_KEYS);
                let limit = i64::from(NAMEPLATE_WOBBLE);
                let x = self.int(wobble, "x", &at, code, (0, limit), 0);
                let y = self.int(wobble, "y", &at, code, (0, limit), 0);
                let period = self.int(
                    wobble,
                    "period",
                    &at,
                    code,
                    (1, i64::from(NAMEPLATE_PARTICLE_LIFETIME)),
                    1,
                );
                Some(Wobble {
                    x: x as u8,
                    y: y as u8,
                    period: period as u16,
                })
            }
        };
        let frames = self.choice(
            map,
            "frames",
            root,
            code,
            |name| match name {
                "loop" => Some(ParticleFrames::Loop),
                "life" => Some(ParticleFrames::Life),
                _ => None,
            },
            ParticleFrames::Loop,
        );
        let fade = match map.get("fade") {
            None => None,
            Some(value) => {
                let at = root.key("fade");
                let fade = self.object(value, &at, code)?;
                self.keys(&at, fade, &FADE_KEYS);
                let after = self.num(fade, "after", &at, code, (0.0, 1.0), 0.0);
                if after >= 1.0 {
                    self.push(at.key("after").error(code));
                }
                let to = self.num(fade, "to", &at, code, (0.0, 1.0), 0.0);
                let steps = self.int(
                    fade,
                    "steps",
                    &at,
                    code,
                    (1, i64::from(NAMEPLATE_FADE_STEPS)),
                    4,
                );
                Some(Fade {
                    after,
                    to: libm::floor(to * 255.0 + 0.5) as u8,
                    steps: steps as u8,
                })
            }
        };
        let breakpoints = match map.get("breakpoints") {
            Some(value) => self.breakpoints(value, &root.key("breakpoints"), None),
            None => Vec::new(),
        };
        let emitter = Emitter {
            sprites: pool,
            spawn,
            rate,
            lifetime,
            velocity_x,
            velocity_y,
            gravity,
            drift,
            wobble,
            frames,
            fade,
        };
        Some(self.layer(i, of, None, breakpoints, LayerContent::Emitter(emitter)))
    }

    fn range(&mut self, value: Option<&Value>, loc: &Loc, limit: f64, ordered: bool) -> [i32; 2] {
        let Some(value) = value else {
            return [0, 0];
        };
        match read::pair_f64(value) {
            Some([a, b]) if a.abs() <= limit && b.abs() <= limit && (!ordered || a <= b) => {
                [read::fixed_point(a, UNIT), read::fixed_point(b, UNIT)]
            }
            _ => {
                self.push(loc.error(IssueCode::Emitter).value("limit", limit));
                [0, 0]
            }
        }
    }

    fn spawn(&mut self, value: &Value, loc: &Loc) -> Option<Spawn> {
        let code = IssueCode::Emitter;
        let map = self.object(value, loc, code)?;
        self.keys(loc, map, &SPAWN_KEYS);
        let shape = self.choice(
            map,
            "shape",
            loc,
            code,
            |name| match name {
                "point" => Some(SpawnShape::Point),
                "line" => Some(SpawnShape::Line),
                "area" => Some(SpawnShape::Area),
                "border" => Some(SpawnShape::Border),
                _ => None,
            },
            SpawnShape::Point,
        );
        let anchor = self.choice(map, "anchor", loc, code, block_anchor, Anchor::TopLeft);
        let edge = self.choice(map, "edge", loc, code, Edge::parse, Edge::Top);
        let offset = self.offset(map, "offset", loc, code, i64::from(NAMEPLATE_OFFSET));
        let spawn_follow = match map.get("follow") {
            None => None,
            Some(value) => {
                let parsed = value.as_str().and_then(follow);
                let ok = match (parsed, shape) {
                    (Some(Follow::Inside), SpawnShape::Area) => true,
                    (Some(_), SpawnShape::Area) => false,
                    (Some(_), SpawnShape::Border) => true,
                    _ => false,
                };
                if !ok {
                    self.push(loc.key("follow").error(IssueCode::Follow));
                }
                parsed
            }
        };
        Some(Spawn {
            shape,
            anchor,
            edge,
            offset,
            follow: spawn_follow,
        })
    }

    fn material_layer(&mut self, i: usize, map: &Map<String, Value>, root: &Loc) -> Option<Layer> {
        self.keys(root, map, &MATERIAL_LAYER_KEYS);
        let of = match map.get("of") {
            None => Of::Block,
            Some(Value::String(s)) if s == NAME_TARGET => Of::Name,
            Some(Value::String(s)) if self.is_kind(s, Kind::Body) => {
                Of::Layer(self.layer_index(s)?)
            }
            Some(_) => {
                self.push(root.key("of").error(IssueCode::Material));
                Of::Block
            }
        };
        let drift = self.boolean(map, "drift", root, IssueCode::Material, false);
        let mirror = self.mirror(map, root);
        let material = match self.string(map, "material", root, IssueCode::Material) {
            Some(id) => self.material(id, &root.key("material")),
            None => {
                self.push(root.key("material").error(IssueCode::Material));
                None
            }
        };
        let content = MaterialLayer {
            material: material?,
            drift,
        };
        Some(self.layer(i, of, mirror, Vec::new(), LayerContent::Material(content)))
    }

    fn material(&mut self, id: &str, from: &Loc) -> Option<u16> {
        if let Some(i) = self.material_ids.iter().position(|m| m == id) {
            return self.materials[i].as_ref().map(|_| i as u16);
        }
        let name = self.bundle.id_path("materials", id, "json", &from.path())?;
        let index = self.material_ids.len();
        self.material_ids.push(id.to_string());
        self.materials.push(None);
        let json = self.bundle.json(&name, &from.path())?;
        let mut textures = Vec::new();
        let mut sprites = Vec::new();
        referenced(&json, &mut textures, &mut sprites);
        let at = Loc::new(&name, "");
        for texture in textures {
            if self.texture_names.contains(&texture) {
                continue;
            }
            let Some(file) = self.bundle.id_path("textures", &texture, "png", &at.path()) else {
                continue;
            };
            if let Some(image) = self.bundle.png(&file, &at.path()) {
                self.texture_names.push(texture);
                self.textures.push(image);
            }
        }
        for sprite in sprites {
            self.sprite(&sprite, &at);
        }
        let mut ctx = GraphContext {
            palette: &self.palette,
            params: &self.params,
            textures: &self.texture_names,
            sprites: &self.sprites,
            molang: &mut self.molang,
        };
        let ir = build(&name, &json, &mut ctx, self.bundle.report());
        let ok = ir.is_some();
        self.materials[index] = ir;
        ok.then_some(index as u16)
    }

    pub(super) fn masks(&mut self, manifest: &Map<String, Value>, root: &Loc) {
        let Some(items) = manifest.get("layers").and_then(Value::as_array) else {
            return;
        };
        let entries: Vec<(String, Option<String>, Option<Value>)> = items
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|e| {
                let id = e.get("id")?.as_str()?.to_string();
                let mask = e.get("mask").map(|m| m.as_str().map(String::from));
                let clip = e.get("clip").cloned();
                match mask {
                    Some(None) => Some((id, Some(String::new()), clip)),
                    other => Some((id, other.flatten(), clip)),
                }
            })
            .collect();
        for (id, mask, clip) in entries {
            let Some(index) = self.layer_index(&id) else {
                continue;
            };
            let at = root.key("layers").key(&id);
            let clip_fill = match &clip {
                None => false,
                Some(Value::String(s)) if s == "fill" => true,
                Some(_) => {
                    self.push(at.key("clip").error(IssueCode::Mask));
                    false
                }
            };
            let Some(mask) = mask else {
                if clip.is_some() {
                    self.push(at.key("clip").error(IssueCode::Mask));
                }
                continue;
            };
            let mask_at = at.key("mask");
            let own = &self.decls[usize::from(index)];
            if own.kind == Some(Kind::Emitter) || own.path {
                self.push(mask_at.error(IssueCode::Mask));
                continue;
            }
            if mask == NAME_TARGET {
                if clip_fill {
                    self.push(at.key("clip").error(IssueCode::Mask));
                }
                if let Some(layer) = self.layers[usize::from(index)].as_mut() {
                    layer.mask = Some(Mask::Name);
                }
                continue;
            }
            let Some(target) = self.layer_index(&mask).filter(|t| *t != index) else {
                self.push(mask_at.error(IssueCode::Mask).name("layer", &mask));
                continue;
            };
            let target_masked = items.iter().filter_map(Value::as_object).any(|e| {
                e.get("id").and_then(Value::as_str) == Some(mask.as_str()) && e.contains_key("mask")
            });
            if target_masked {
                self.push(mask_at.error(IssueCode::Mask).name("layer", &mask));
                continue;
            }
            let decl = &self.decls[usize::from(target)];
            let ok = match decl.kind {
                Some(Kind::Body) => true,
                Some(Kind::Sprite) => !decl.path && !clip_fill,
                _ => false,
            };
            if !ok {
                self.push(mask_at.error(IssueCode::Mask).name("layer", &mask));
                continue;
            }
            if let Some(layer) = self.layers[usize::from(index)].as_mut() {
                layer.mask = Some(Mask::Layer {
                    slot: target,
                    fill: clip_fill,
                });
            }
        }
    }

    pub(super) fn sprite_cycles(&mut self) {
        let mut checked: Vec<(u16, i64)> = Vec::new();
        for i in 0..self.decls.len() {
            let decl = &self.decls[i];
            if decl.kind != Some(Kind::Sprite)
                || decl.timeline
                || decl.facings
                || self.keyed_frames[i]
            {
                continue;
            }
            let Some(sprite) = decl.sprite else {
                continue;
            };
            let cycle = i64::from(self.loop_frames) / i64::from(decl.step.max(1));
            if checked.contains(&(sprite, cycle)) {
                continue;
            }
            checked.push((sprite, cycle));
            let frames = self.frame_count(sprite) as i64;
            if frames > 0 && cycle % frames != 0 {
                let file = format!("sprites/{}.json", self.sprite_ids[usize::from(sprite)]);
                self.push(
                    Loc::new(&file, "frames")
                        .error(IssueCode::SpriteFrames)
                        .value("frames", frames as f64)
                        .value("cycle", cycle as f64),
                );
            }
        }
    }
}

fn strings(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(String::from))
        .collect()
}

fn referenced(value: &Value, textures: &mut Vec<String>, sprites: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            let kind = map.get("type").and_then(Value::as_str);
            let set = map.get("set").and_then(Value::as_object);
            if let (Some(kind), Some(set)) = (kind, set) {
                let push = |list: &mut Vec<String>, key: &str| {
                    if let Some(name) = set.get(key).and_then(Value::as_str) {
                        if !list.iter().any(|n| n == name) {
                            list.push(name.to_string());
                        }
                    }
                };
                match kind {
                    "BundleTexture" => push(textures, "texture"),
                    "Sprite" => push(sprites, "sprite"),
                    _ => {}
                }
            }
            map.values().for_each(|v| referenced(v, textures, sprites));
        }
        Value::Array(items) => items.iter().for_each(|v| referenced(v, textures, sprites)),
        _ => {}
    }
}
