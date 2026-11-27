use std::cell::{Cell, RefCell};
use std::fmt::Write;
use std::sync::Arc;

use plusrenderer::animation::Loop;
use plusrenderer::asset::{analyze, load, CosmeticAsset, LoadOptions, Loaded, Skip, Source};
use plusrenderer::attach::{
    actor_wearer, event_key, event_wearer, pair_seed, PlayerLocator, RigKind,
};
use plusrenderer::container::CAPABILITIES;
use plusrenderer::effects::{View, QUAD_FLOATS};
use plusrenderer::issues::{Issue, Level};
use plusrenderer::limits::constants;
use plusrenderer::palette::custom_tones;
use plusrenderer::reactions::{On, Reaction, APPEAR, EQUIP};
use plusrenderer::scene::{FrameError, FrameSections, OutputLayout, Physics, EFFECT_EVENT_FLOATS};
use plusrenderer::signals::SIGNALS;
use plusrenderer::texture::{resolve_texture, texture_key, IndexedTexture};
use plusrenderer::wearer::DeclarationError;

use super::nameplate::{mode_of, Fail};
use super::{
    checked, checked_mut, core_failed, fail, guarded, slot_index, space_transform, Renderer,
};

pub const STATUS_ASSET: i32 = 0;
pub const STATUS_DRAFT: i32 = 1;
pub const STATUS_SKIPPED: i32 = 2;
pub const STATUS_INVALID: i32 = 3;
pub const STATUS_LEGACY: i32 = 4;

pub const SECTION_INTS: usize = 9;
pub const UNHELD_SECTION_INTS: usize = 8;
const SECTION_NAMES: [&str; SECTION_INTS] = [
    "parts",
    "queries",
    "root",
    "variables",
    "play",
    "role",
    "colorway",
    "part_deltas",
    "hold",
];

pub const LAYOUT_INTS: usize = 12;
pub const LAYOUT_STRIDE: usize = 9;

pub const INPUT_LAYOUT_INTS: usize = 9;
pub const INPUT_STRIDE: usize = 7;

pub const TEXTURE_INFO_INTS: usize = 3;
pub const PHYSICS_ON: u32 = 0;
pub const PHYSICS_OWN: u32 = 1;
pub const PHYSICS_OFF: u32 = 2;
pub const VIEW_FLOATS: usize = 12;
pub const NO_COLORWAY: i64 = -1;

pub struct ParsedCosmetic {
    pub(crate) status: i32,
    pub(crate) asset: Option<Arc<CosmeticAsset>>,
    issues: Vec<Issue>,
    skip: Option<Skip>,
    legacy: Vec<String>,
}

impl ParsedCosmetic {
    pub(crate) fn asset(&self) -> Result<&Arc<CosmeticAsset>, Fail> {
        self.asset.as_ref().ok_or_else(|| {
            Fail::Args(format!(
                "the bundle did not load: it is {}",
                status_name(self.status)
            ))
        })
    }
}

thread_local! {
    static ERROR: RefCell<String> = const { RefCell::new(String::new()) };
    static TEXT: RefCell<String> = const { RefCell::new(String::new()) };
    static FAULT: Cell<usize> = const { Cell::new(0) };
}

fn set_error(message: &str) {
    ERROR.with(|e| {
        let mut e = e.borrow_mut();
        e.clear();
        e.push_str(message);
    });
}

fn free_fail(fail: &Fail) -> i32 {
    set_error(fail.message());
    fail.code()
}

fn text_out(result: Result<String, Fail>) -> i32 {
    match result {
        Ok(text) => {
            let len = text.len() as i32;
            TEXT.with(|t| *t.borrow_mut() = text);
            len
        }
        Err(fail) => free_fail(&fail),
    }
}

pub(crate) fn renderer_fail(r: &mut Renderer, f: &Fail) -> i32 {
    match f {
        Fail::Args(message) => fail(r, message),
        Fail::Core(message) => core_failed(r, message),
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn fault_next_frame(r: *const Renderer) {
    FAULT.with(|f| f.set(r as usize));
}

fn take_fault(r: *const Renderer) -> bool {
    FAULT.with(|f| {
        let hit = f.get() == r as usize;
        if hit {
            f.set(0);
        }
        hit
    })
}

unsafe fn view<'a, T>(ptr: *const T, len: u32, name: &str) -> Result<&'a [T], Fail> {
    checked(ptr, len as usize, name).map_err(Fail::Args)
}

unsafe fn view_mut<'a, T>(ptr: *mut T, len: u32, name: &str) -> Result<&'a mut [T], Fail> {
    checked_mut(ptr, len as usize, name).map_err(Fail::Args)
}

unsafe fn text_of<'a>(ptr: *const u8, len: u32, name: &str, what: &str) -> Result<&'a str, Fail> {
    std::str::from_utf8(view(ptr, len, name)?).map_err(|_| Fail::Args(format!("{what} not utf-8")))
}

unsafe fn custom<'a>(
    slots: *const u32,
    rgb: *const u32,
    count: u32,
) -> Result<(&'a [i32], &'a [i32]), Fail> {
    Ok((
        view(slots.cast(), count, "custom_slots")?,
        view(rgb.cast(), count, "custom_rgb")?,
    ))
}

pub(crate) fn parse_cosmetic(
    bytes: &[u8],
    mode: i64,
    capabilities: Option<&str>,
    identity: u64,
) -> Result<ParsedCosmetic, Fail> {
    let mode = mode_of(mode)?;
    let names: Vec<&str> = match capabilities {
        Some(joined) => joined.split('\n').filter(|c| !c.is_empty()).collect(),
        None => CAPABILITIES.to_vec(),
    };
    let options = LoadOptions {
        mode,
        identity,
        capabilities: &names,
    };
    guarded(Err(Fail::Core("the core failed on this bundle")), || {
        Ok(match load(Source::Zip(bytes), &options) {
            Loaded::Asset { asset, issues } => parsed(STATUS_ASSET, Some(asset), issues),
            Loaded::Draft { asset, issues } => parsed(STATUS_DRAFT, Some(asset), issues),
            Loaded::Skipped(skip) => ParsedCosmetic {
                skip: Some(skip),
                ..parsed(STATUS_SKIPPED, None, Vec::new())
            },
            Loaded::Invalid(issues) => parsed(STATUS_INVALID, None, issues),
            Loaded::Legacy(legacy) => ParsedCosmetic {
                legacy: legacy.files.into_iter().map(|(name, _)| name).collect(),
                ..parsed(STATUS_LEGACY, None, Vec::new())
            },
        })
    })
}

fn parsed(status: i32, asset: Option<Arc<CosmeticAsset>>, issues: Vec<Issue>) -> ParsedCosmetic {
    ParsedCosmetic {
        status,
        asset,
        issues,
        skip: None,
        legacy: Vec::new(),
    }
}

fn status_name(status: i32) -> &'static str {
    match status {
        STATUS_ASSET => "asset",
        STATUS_DRAFT => "draft",
        STATUS_SKIPPED => "skipped",
        STATUS_LEGACY => "legacy",
        _ => "invalid",
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Info => "info",
    }
}

fn on_name(reaction: &Reaction) -> &'static str {
    match (reaction.signal, reaction.on) {
        (APPEAR, _) => "appear",
        (EQUIP, _) => "equip",
        (_, On::Start) => "start",
        (_, On::Stop) => "stop",
        (_, On::While) => "while",
    }
}

fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_num(out: &mut String, value: f64) {
    if value.is_finite() {
        let _ = write!(out, "{value}");
    } else {
        out.push_str("null");
    }
}

fn push_list<T>(
    out: &mut String,
    items: impl IntoIterator<Item = T>,
    mut each: impl FnMut(&mut String, T),
) {
    out.push('[');
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        each(out, item);
    }
    out.push(']');
}

fn push_names<'a>(out: &mut String, names: impl IntoIterator<Item = &'a str>) {
    push_list(out, names, push_str);
}

struct Obj<'a> {
    out: &'a mut String,
    first: bool,
}

impl<'a> Obj<'a> {
    fn new(out: &'a mut String) -> Obj<'a> {
        out.push('{');
        Obj { out, first: true }
    }

    fn key(&mut self, key: &str) -> &mut String {
        if !self.first {
            self.out.push(',');
        }
        self.first = false;
        push_str(self.out, key);
        self.out.push(':');
        self.out
    }

    fn str(&mut self, key: &str, value: &str) {
        push_str(self.key(key), value);
    }

    fn opt_str(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(value) => self.str(key, value),
            None => self.null(key),
        }
    }

    fn int(&mut self, key: &str, value: i64) {
        let _ = write!(self.key(key), "{value}");
    }

    fn num(&mut self, key: &str, value: f64) {
        push_num(self.key(key), value);
    }

    fn bool(&mut self, key: &str, value: bool) {
        self.key(key).push_str(if value { "true" } else { "false" });
    }

    fn null(&mut self, key: &str) {
        self.key(key).push_str("null");
    }

    fn end(self) {
        self.out.push('}');
    }
}

fn push_issue(out: &mut String, issue: &Issue) {
    let mut o = Obj::new(out);
    o.str("code", issue.code.as_str());
    o.str("level", level_name(issue.level));
    o.str("path", &issue.path);
    let mut values = Obj::new(o.key("values"));
    for (key, value) in &issue.values {
        values.num(key, *value);
    }
    values.end();
    let mut names = Obj::new(o.key("names"));
    for (key, name) in &issue.names {
        names.str(key, name);
    }
    names.end();
    o.end();
}

pub(crate) fn issues_json(p: &ParsedCosmetic) -> String {
    let mut out = String::new();
    let mut o = Obj::new(&mut out);
    o.str("status", status_name(p.status));
    match &p.skip {
        None => o.null("skip"),
        Some(Skip::Format(format)) => {
            let mut skip = Obj::new(o.key("skip"));
            skip.int("format", i64::try_from(*format).unwrap_or(i64::MAX));
            skip.end();
        }
        Some(Skip::Requires(names)) => {
            let mut skip = Obj::new(o.key("skip"));
            push_names(skip.key("requires"), names.iter().map(String::as_str));
            skip.end();
        }
    }
    if p.status == STATUS_LEGACY {
        let mut legacy = Obj::new(o.key("legacy"));
        push_names(legacy.key("files"), p.legacy.iter().map(String::as_str));
        legacy.end();
    } else {
        o.null("legacy");
    }
    push_list(o.key("issues"), &p.issues, push_issue);
    o.end();
    out
}

pub(crate) fn meta_json(p: &ParsedCosmetic) -> Result<String, Fail> {
    let a = p.asset()?;
    let mut out = String::new();
    let mut o = Obj::new(&mut out);
    o.str("id", &a.id);
    o.str("identity", &a.identity.to_string());
    o.str("kind", a.kind.as_str());
    push_names(o.key("requires"), a.requires.iter().map(String::as_str));
    let mut attach = Obj::new(o.key("attach"));
    attach.str("rig", a.attach.rig.as_str());
    attach.opt_str("bone", a.attach.bone.as_deref());
    attach.opt_str("replaces", a.attach.replaces.as_deref());
    attach.opt_str("anchor", a.attach.anchor.map(PlayerLocator::as_str));
    attach.end();
    match &a.model {
        None => o.null("model"),
        Some(model) => {
            let mut mo = Obj::new(o.key("model"));
            mo.str("identifier", &model.identifier);
            mo.int("bones", model.bones.len() as i64);
            let locators = model
                .bones
                .iter()
                .flat_map(|b| b.locators.iter().map(|l| l.name.as_str()));
            push_names(mo.key("locators"), locators);
            mo.end();
        }
    }
    push_list(o.key("textures"), &a.textures, |out, t| {
        let mut to = Obj::new(out);
        to.str("name", &t.name);
        to.int("width", i64::from(t.width));
        to.int("height", i64::from(t.height));
        to.bool("indexed", t.index.is_some());
        to.end();
    });
    let mut palette = Obj::new(o.key("palette"));
    push_list(palette.key("slots"), &a.palette.slots, |out, s| {
        let mut so = Obj::new(out);
        so.str("id", &s.id);
        so.str("name", &s.name);
        so.bool("customizable", s.customizable);
        so.end();
    });
    push_names(
        palette.key("colorways"),
        a.palette.colorways.iter().map(|c| c.id.as_str()),
    );
    palette.end();
    let animation = |index: u16| {
        a.animations
            .get(usize::from(index))
            .map(|x| x.name.as_str())
    };
    push_names(
        o.key("animations"),
        a.animations.iter().map(|x| x.name.as_str()),
    );
    push_list(o.key("clips"), &a.animations, |out, x| {
        let mut co = Obj::new(out);
        co.str("name", &x.name);
        co.num("length", f64::from(x.length));
        co.str("loop", loop_name(x.loop_mode));
        if let Some(delay) = x.constant_start_delay().filter(|d| *d > 0.0) {
            co.num("start_delay", f64::from(delay));
        }
        co.end();
    });
    push_names(
        o.key("controllers"),
        a.controllers.controllers().iter().map(|c| c.name.as_str()),
    );
    push_list(o.key("reactions"), &a.reactions, |out, r| {
        let mut ro = Obj::new(out);
        match SIGNALS.get(usize::from(r.signal)) {
            Some((name, _)) => ro.str("signal", name),
            None => ro.null("signal"),
        }
        ro.int("signal_index", i64::from(r.signal));
        ro.str("on", on_name(r));
        ro.opt_str("play", r.play.and_then(animation));
        ro.bool("restart", r.restart);
        ro.num("blend", f64::from(r.blend));
        match r.speed {
            Some(speed) => ro.num("speed", f64::from(speed)),
            None => ro.null("speed"),
        }
        let colorway = r
            .palette
            .and_then(|c| a.palette.colorways.get(usize::from(c)));
        ro.opt_str("colorway", colorway.map(|c| c.id.as_str()));
        let effect = r.effect.and_then(|e| a.effects.named.get(usize::from(e)));
        ro.opt_str("effect", effect.map(|(name, _)| name.as_str()));
        ro.end();
    });
    o.int("spring_bones", a.springs.bones.len() as i64);
    let e = &a.effects;
    let mut effects = Obj::new(o.key("effects"));
    push_list(effects.key("particles"), &e.particles, |out, p| {
        let mut po = Obj::new(out);
        po.str("id", &p.id);
        po.str("texture", &p.texture);
        po.end();
    });
    push_list(effects.key("sounds"), &e.sounds, |out, s| {
        let mut so = Obj::new(out);
        so.str("id", &s.id);
        so.opt_str("file", s.file.as_deref());
        so.opt_str("vanilla", s.vanilla.as_deref());
        so.num("volume", f64::from(s.volume));
        so.num("pitch", f64::from(s.pitch));
        so.end();
    });
    effects.int("emitters", e.emitters.len() as i64);
    push_list(
        effects.key("emitter_vanilla"),
        &e.emitters,
        |out, em| match em.vanilla.as_deref() {
            Some(name) => push_str(out, name),
            None => out.push_str("null"),
        },
    );
    push_list(effects.key("trails"), &e.trails, |out, t| {
        let mut to = Obj::new(out);
        to.str("texture", &t.texture);
        to.end();
    });
    push_names(
        effects.key("named"),
        e.named.iter().map(|(name, _)| name.as_str()),
    );
    push_list(effects.key("moments"), &e.moments, |out, m| {
        let mut mo = Obj::new(out);
        mo.str("name", &m.name);
        match m.clip {
            Some(clip) => mo.int("clip", i64::from(clip)),
            None => mo.null("clip"),
        }
        mo.num("seconds", f64::from(m.seconds));
        mo.end();
    });
    push_names(
        effects.key("locators"),
        e.locators.iter().map(String::as_str),
    );
    effects.end();
    match &a.pet {
        None => o.null("pet"),
        Some(pet) => {
            let mut po = Obj::new(o.key("pet"));
            po.str("archetype", pet.archetype.as_str());
            po.num("leash_radius", f64::from(pet.leash_radius));
            po.num("scale", f64::from(pet.scale));
            po.opt_str("anchor", pet.anchor.as_deref());
            push_names(
                po.key("controllers"),
                pet.controllers.iter().map(String::as_str),
            );
            let mut io = Obj::new(po.key("interactions"));
            for d in &pet.interactions {
                let roles = [d.as_a.then_some("a"), d.as_b.then_some("b")];
                push_names(io.key(&d.id), roles.into_iter().flatten());
            }
            io.end();
            po.end();
        }
    }
    push_list(o.key("roles"), &a.roles, |out, role| {
        let mut ro = Obj::new(out);
        ro.str("id", &role.id);
        if let Some(interaction) = role.interaction.as_deref() {
            ro.str("interaction", interaction);
        }
        ro.opt_str("animation", animation(role.animation));
        push_list(ro.key("offset"), role.offset, |out, v| {
            push_num(out, f64::from(v))
        });
        ro.num("yaw", f64::from(role.yaw));
        ro.end();
    });
    match &a.rules {
        None => o.null("rules"),
        Some(rules) => {
            let mut ru = Obj::new(o.key("rules"));
            ru.bool("allow_walking", rules.allow_walking);
            ru.bool("allow_sprinting", rules.allow_sprinting);
            ru.bool("allow_crouching", rules.allow_crouching);
            ru.bool("allow_falling", rules.allow_falling);
            ru.bool("allow_elytra_flying", rules.allow_elytra_flying);
            ru.bool("allow_swimming", rules.allow_swimming);
            ru.bool("allow_vanilla_pose", rules.allow_vanilla_pose);
            ru.end();
        }
    }
    o.str("requirement", a.requirement.as_str());
    match a.fallback.and_then(|i| a.animations.get(usize::from(i))) {
        Some(clip) => o.str("fallback", &clip.name),
        None => o.null("fallback"),
    }
    o.opt_str("main", a.main.and_then(animation));
    o.end();
    Ok(out)
}

pub(crate) fn analyze_json(p: &ParsedCosmetic) -> Result<String, Fail> {
    let asset = p.asset()?;
    guarded(
        Err(Fail::Core("the core failed analysing this bundle")),
        || {
            let a = analyze(asset);
            let mut out = String::new();
            let mut o = Obj::new(&mut out);
            push_names(o.key("requires"), a.requires.iter().map(String::as_str));
            o.int("quads", i64::from(a.quads));
            o.int("bones", i64::from(a.bones));
            o.int("spring_bones", i64::from(a.spring_bones));
            o.num("live_particles", a.live_particles);
            o.int("trail_points", i64::from(a.trail_points));
            o.int("record_bytes", i64::from(a.record_bytes));
            o.int("texture_bytes", i64::from(a.texture_bytes));
            o.end();
            Ok(out)
        },
    )
}

pub(crate) fn texture_count(p: &ParsedCosmetic) -> Result<usize, Fail> {
    Ok(p.asset()?.textures.len())
}

pub(crate) fn texture(p: &ParsedCosmetic, index: i64) -> Result<&IndexedTexture, Fail> {
    let textures = &p.asset()?.textures;
    usize::try_from(index)
        .ok()
        .and_then(|i| textures.get(i))
        .ok_or_else(|| {
            Fail::Args(format!(
                "no texture {index}, the bundle has {}",
                textures.len()
            ))
        })
}

pub(crate) fn effect_texture_count(p: &ParsedCosmetic) -> Result<usize, Fail> {
    Ok(p.asset()?.media.textures.len())
}

pub(crate) fn effect_texture(p: &ParsedCosmetic, index: i64) -> Result<&IndexedTexture, Fail> {
    let textures = &p.asset()?.media.textures;
    usize::try_from(index)
        .ok()
        .and_then(|i| textures.get(i))
        .ok_or_else(|| {
            Fail::Args(format!(
                "no effect texture {index}, the bundle has {}",
                textures.len()
            ))
        })
}

pub(crate) fn sound_count(p: &ParsedCosmetic) -> Result<usize, Fail> {
    Ok(p.asset()?.media.sounds.len())
}

pub(crate) fn sound(p: &ParsedCosmetic, index: i64) -> Result<&[u8], Fail> {
    let sounds = &p.asset()?.media.sounds;
    usize::try_from(index)
        .ok()
        .and_then(|i| sounds.get(i))
        .map(Vec::as_slice)
        .ok_or_else(|| Fail::Args(format!("no sound {index}, the bundle has {}", sounds.len())))
}

pub(crate) fn texture_info(t: &IndexedTexture) -> [i32; TEXTURE_INFO_INTS] {
    [
        t.width as i32,
        t.height as i32,
        i32::from(t.index.is_some()),
    ]
}

pub(crate) struct Paint {
    colorway: Option<usize>,
    custom: Vec<(u8, u32)>,
}

pub(crate) fn paint(
    asset: &CosmeticAsset,
    colorway: i64,
    slots: &[i32],
    rgb: &[i32],
) -> Result<Paint, Fail> {
    let colorways = asset.palette.colorways.len();
    let colorway = match colorway {
        NO_COLORWAY => None,
        c => match usize::try_from(c).ok().filter(|&c| c < colorways) {
            Some(c) => Some(c),
            None => {
                return Err(Fail::Args(format!(
                    "no colorway {c}, the bundle has {colorways}"
                )))
            }
        },
    };
    if slots.len() != rgb.len() {
        return Err(Fail::Args(format!(
            "{} custom slots for {} colors",
            slots.len(),
            rgb.len()
        )));
    }
    let mut custom = Vec::with_capacity(slots.len());
    for (&slot, &color) in slots.iter().zip(rgb) {
        let slot = u8::try_from(slot).map_err(|_| Fail::Args(format!("no palette slot {slot}")))?;
        custom.push((slot, color as u32 & 0x00ff_ffff));
    }
    Ok(Paint { colorway, custom })
}

pub(crate) fn set_effect_colors(
    r: &mut Renderer,
    wearer: u64,
    colorway: i64,
    slots: &[i32],
    rgb: &[i32],
) -> Result<(), Fail> {
    let colorway = match colorway {
        NO_COLORWAY => None,
        c => Some(usize::try_from(c).map_err(|_| Fail::Args(format!("no colorway {c}")))?),
    };
    if slots.len() != rgb.len() {
        return Err(Fail::Args(format!(
            "{} custom slots for {} colors",
            slots.len(),
            rgb.len()
        )));
    }
    let mut custom = Vec::with_capacity(slots.len());
    for (&slot, &color) in slots.iter().zip(rgb) {
        let slot = u8::try_from(slot).map_err(|_| Fail::Args(format!("no palette slot {slot}")))?;
        custom.push((slot, color as u32 & 0x00ff_ffff));
    }
    r.scene.set_effect_colors(wearer, colorway, &custom);
    Ok(())
}

pub(crate) fn resolved<'a>(
    p: &'a ParsedCosmetic,
    index: i64,
    colorway: i64,
    slots: &[i32],
    rgb: &[i32],
) -> Result<(&'a IndexedTexture, Vec<u8>), Fail> {
    let asset = p.asset()?;
    let t = texture(p, index)?;
    let paint = paint(asset, colorway, slots, rgb)?;
    let rgba = guarded(
        Err(Fail::Core("the core failed resolving this texture")),
        || {
            let tones = custom_tones(&asset.palette, paint.colorway, &paint.custom);
            Ok(resolve_texture(t, &asset.palette.resolve(None), &tones))
        },
    )?;
    Ok((t, rgba))
}

pub(crate) fn key(
    p: &ParsedCosmetic,
    index: i64,
    colorway: i64,
    slots: &[i32],
    rgb: &[i32],
) -> Result<u64, Fail> {
    let asset = p.asset()?;
    texture(p, index)?;
    let paint = paint(asset, colorway, slots, rgb)?;
    let colorway = paint.colorway.map(|c| c as u16);
    Ok(texture_key(
        asset.identity,
        index as u16,
        colorway,
        &paint.custom,
    ))
}

pub(crate) fn sections_of(ints: &[i32]) -> Result<FrameSections, Fail> {
    if ints.len() != SECTION_INTS && ints.len() != UNHELD_SECTION_INTS {
        return Err(Fail::Args(format!(
            "sections hold {} ints, not {SECTION_INTS}",
            ints.len()
        )));
    }
    let count = |at: usize| {
        usize::try_from(ints[at]).map_err(|_| {
            Fail::Args(format!(
                "section {} is {}, not a count",
                SECTION_NAMES[at], ints[at]
            ))
        })
    };
    let flag = |at: usize| match ints[at] {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(Fail::Args(format!(
            "section {} is {value}, not 0 or 1",
            SECTION_NAMES[at]
        ))),
    };
    Ok(FrameSections {
        parts: count(0)?,
        queries: count(1)?,
        root: flag(2)?,
        variables: count(3)?,
        play: flag(4)?,
        role: flag(5)?,
        colorway: flag(6)?,
        part_deltas: flag(7)?,
        hold: ints.len() > 8 && flag(8)?,
    })
}

fn int(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub(crate) fn input_layout(s: &FrameSections) -> [i32; INPUT_LAYOUT_INTS] {
    let shape = s.shape();
    [
        int(shape.signals()),
        int(shape.events()),
        int(shape.detail()),
        int(s.root_at()),
        int(s.variables_at()),
        int(s.play_at()),
        int(s.role_at()),
        int(s.stride()),
        int(s.hold_at()),
    ]
}

pub(crate) fn layout_ints(l: &OutputLayout) -> [i32; LAYOUT_INTS] {
    [
        int(l.bones),
        int(l.runs),
        int(l.locators),
        i32::from(l.colorway),
        int(l.part_deltas),
        int(l.runs_at()),
        int(l.locators_at()),
        int(l.colorway_at()),
        int(l.part_deltas_at()),
        int(l.stride()),
        i32::from(l.emote),
        int(l.emote_at()),
    ]
}

fn declaration_message(e: DeclarationError) -> String {
    match e {
        DeclarationError::TooManyVariables { declared, limit } => {
            format!("the asset declares {declared} variables, the limit is {limit}")
        }
        DeclarationError::TooManyOneShots { declared, limit } => {
            format!("the asset declares {declared} one-shots, the limit is {limit}")
        }
        DeclarationError::OverBudget { bytes, budget } => {
            format!("a wearer record of {bytes} bytes is over the {budget} byte budget")
        }
    }
}

pub(crate) fn load_asset(r: &mut Renderer, slot: i64, p: &ParsedCosmetic) -> Result<(), Fail> {
    let slot = slot_index(slot).map_err(Fail::Args)?;
    let asset = p.asset()?.clone();
    let scene = &mut r.scene;
    guarded(
        Err(Fail::Core("the core failed loading this asset")),
        || {
            scene
                .load_asset(slot, asset)
                .map_err(|e| Fail::Args(declaration_message(e)))
        },
    )
}

fn loop_name(mode: Loop) -> &'static str {
    match mode {
        Loop::Once => "once",
        Loop::Repeat => "repeat",
        Loop::Hold => "hold",
    }
}

pub(crate) fn load_emote(
    r: &mut Renderer,
    slot: i64,
    props: i64,
    p: &ParsedCosmetic,
) -> Result<(), Fail> {
    let slot = slot_index(slot).map_err(Fail::Args)?;
    let props = if props < 0 {
        None
    } else {
        Some(slot_index(props).map_err(Fail::Args)?)
    };
    let asset = p.asset()?.clone();
    let scene = &mut r.scene;
    guarded(
        Err(Fail::Core("the core failed loading this emote")),
        || {
            scene
                .load_emote(slot, props, asset)
                .map_err(|e| Fail::Args(declaration_message(e)))
        },
    )
}

pub(crate) fn output_layout(r: &mut Renderer, s: &FrameSections) -> Result<OutputLayout, Fail> {
    let scene = &mut r.scene;
    guarded(Err(Fail::Core("the core failed building the skin")), || {
        Ok(scene.output_layout(s))
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_shaped(
    r: &mut Renderer,
    clock: f64,
    wearers: &[u64],
    sections: &FrameSections,
    input: &[f32],
    space: u32,
    out: &mut [f32],
) -> Result<usize, Fail> {
    let fault = take_fault(r);
    let to_host = space_transform(space);
    let scene = &mut r.scene;
    guarded(Err(Fail::Core("the core failed on this frame")), || {
        if fault {
            panic!("cosmetic fault");
        }
        scene
            .frame_wearers_shaped(clock, wearers, sections, input, &to_host, out)
            .map_err(|e| Fail::Args(e.to_string()))
    })
}

pub(crate) fn set_rig(r: &mut Renderer, kind: i64) -> Result<(), Fail> {
    let rig = u32::try_from(kind)
        .ok()
        .and_then(RigKind::from_index)
        .ok_or_else(|| Fail::Args(format!("no rig {kind}")))?;
    r.scene.set_rig(rig);
    Ok(())
}

pub(crate) fn set_physics(r: &mut Renderer, mode: i64, own: u64) -> Result<(), Fail> {
    let physics = match u32::try_from(mode) {
        Ok(PHYSICS_ON) => Physics::On,
        Ok(PHYSICS_OWN) => Physics::Own(own),
        Ok(PHYSICS_OFF) => Physics::Off,
        _ => return Err(Fail::Args(format!("no physics mode {mode}"))),
    };
    r.scene.set_physics(physics);
    Ok(())
}

pub(crate) fn locator_id(r: &mut Renderer, name: &str) -> Result<usize, Fail> {
    let scene = &mut r.scene;
    guarded(
        Err(Fail::Core("the core failed finding this locator")),
        || {
            scene
                .locator_id(name)
                .ok_or_else(|| Fail::Args(format!("no locator `{name}`")))
        },
    )
}

pub(crate) fn view_of(floats: &[f32]) -> Result<View, Fail> {
    if floats.len() != VIEW_FLOATS {
        return Err(Fail::Args(format!(
            "view holds {} floats, not {VIEW_FLOATS}",
            floats.len()
        )));
    }
    let three = |at: usize| [floats[at], floats[at + 1], floats[at + 2]];
    Ok(View {
        camera: three(0),
        right: three(3),
        up: three(6),
        origin_shift: three(9),
    })
}

pub(crate) fn effect_geometry(
    r: &mut Renderer,
    view: &View,
    out: &mut [f32],
    sizing: bool,
) -> Result<usize, Fail> {
    let scene = &mut r.scene;
    guarded(
        Err(Fail::Core("the core failed drawing effects")),
        || match scene.effect_geometry(view, out) {
            Ok(quads) => Ok(quads),
            Err(FrameError::Output { needed, .. }) if sizing => Ok(needed / QUAD_FLOATS),
            Err(e) => Err(Fail::Args(e.to_string())),
        },
    )
}

pub(crate) fn whole_events(floats: &[f32], room: usize) -> &[f32] {
    let whole = room / EFFECT_EVENT_FLOATS * EFFECT_EVENT_FLOATS;
    &floats[..whole.min(floats.len())]
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_parse(
    ptr: *const u8,
    len: u32,
    mode: u32,
    caps_ptr: *const u8,
    caps_len: u32,
    identity: u64,
) -> *const ParsedCosmetic {
    let capabilities = if caps_ptr.is_null() && caps_len == 0 {
        Ok(None)
    } else {
        text_of(caps_ptr, caps_len, "capabilities", "capabilities are").map(Some)
    };
    let parsed = capabilities.and_then(|capabilities| {
        let bytes = view(ptr, len, "bundle")?;
        parse_cosmetic(bytes, i64::from(mode), capabilities, identity)
    });
    match parsed {
        Ok(parsed) => Arc::into_raw(Arc::new(parsed)),
        Err(fail) => {
            free_fail(&fail);
            std::ptr::null()
        }
    }
}

fn bundle_identity(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= bytes.len() as u64;
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_bundle_identity(ptr: *const u8, len: u32) -> u64 {
    match view(ptr, len, "bundle") {
        Ok(bytes) => bundle_identity(bytes),
        Err(fail) => {
            free_fail(&fail);
            0
        }
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_asset_free(h: *const ParsedCosmetic) {
    if !h.is_null() {
        drop(Arc::from_raw(h));
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_asset_status(h: *const ParsedCosmetic) -> i32 {
    (&*h).status
}

#[no_mangle]
pub extern "C" fn cosmetic_error() -> *const u8 {
    ERROR.with(|e| e.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn cosmetic_error_len() -> u32 {
    ERROR.with(|e| e.borrow().len() as u32)
}

#[no_mangle]
pub extern "C" fn cosmetic_text() -> *const u8 {
    TEXT.with(|t| t.borrow().as_ptr())
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_asset_issues_json(h: *const ParsedCosmetic) -> i32 {
    text_out(Ok(issues_json(&*h)))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_asset_analyze_json(h: *const ParsedCosmetic) -> i32 {
    text_out(analyze_json(&*h))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_asset_meta_json(h: *const ParsedCosmetic) -> i32 {
    text_out(meta_json(&*h))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_texture_count(h: *const ParsedCosmetic) -> i32 {
    match texture_count(&*h) {
        Ok(count) => int(count),
        Err(fail) => free_fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_texture_info(
    h: *const ParsedCosmetic,
    index: u32,
    info: *mut i32,
) -> i32 {
    let t = match texture(&*h, i64::from(index)) {
        Ok(t) => t,
        Err(fail) => return free_fail(&fail),
    };
    if !info.is_null() {
        info.copy_from_nonoverlapping(texture_info(t).as_ptr(), TEXTURE_INFO_INTS);
    }
    int(t.pixels().saturating_mul(4))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_effect_texture_count(h: *const ParsedCosmetic) -> i32 {
    match effect_texture_count(&*h) {
        Ok(count) => int(count),
        Err(fail) => free_fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_effect_texture_info(
    h: *const ParsedCosmetic,
    index: u32,
    info: *mut i32,
) -> i32 {
    let t = match effect_texture(&*h, i64::from(index)) {
        Ok(t) => t,
        Err(fail) => return free_fail(&fail),
    };
    if !info.is_null() {
        info.copy_from_nonoverlapping(texture_info(t).as_ptr(), TEXTURE_INFO_INTS);
    }
    int(t.rgba.len())
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_effect_texture(
    h: *const ParsedCosmetic,
    index: u32,
    rgba_out: *mut u8,
    rgba_len: u32,
) -> i32 {
    let t = match effect_texture(&*h, i64::from(index)) {
        Ok(t) => t,
        Err(fail) => return free_fail(&fail),
    };
    bytes_out(&t.rgba, rgba_out, rgba_len, "rgba_out")
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_sound_count(h: *const ParsedCosmetic) -> i32 {
    match sound_count(&*h) {
        Ok(count) => int(count),
        Err(fail) => free_fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_sound(
    h: *const ParsedCosmetic,
    index: u32,
    out: *mut u8,
    out_len: u32,
) -> i32 {
    match sound(&*h, i64::from(index)) {
        Ok(bytes) => bytes_out(bytes, out, out_len, "out"),
        Err(fail) => free_fail(&fail),
    }
}

unsafe fn bytes_out(bytes: &[u8], out: *mut u8, out_len: u32, name: &str) -> i32 {
    let size = bytes.len();
    if out.is_null() {
        return int(size);
    }
    if (out_len as usize) < size {
        return free_fail(&Fail::Args(format!(
            "{name} holds {out_len} bytes, the file needs {size}"
        )));
    }
    out.copy_from_nonoverlapping(bytes.as_ptr(), size);
    int(size)
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_resolve_texture(
    h: *const ParsedCosmetic,
    index: u32,
    colorway: i32,
    custom_slots: *const u32,
    custom_rgb: *const u32,
    custom_count: u32,
    rgba_out: *mut u8,
    rgba_len: u32,
) -> i32 {
    let painted = custom(custom_slots, custom_rgb, custom_count)
        .and_then(|(slots, rgb)| resolved(&*h, i64::from(index), i64::from(colorway), slots, rgb));
    let rgba = match painted {
        Ok((_, rgba)) => rgba,
        Err(fail) => return free_fail(&fail),
    };
    let bytes = rgba.len();
    if rgba_out.is_null() {
        return int(bytes);
    }
    if (rgba_len as usize) < bytes {
        return free_fail(&Fail::Args(format!(
            "rgba_out holds {rgba_len} bytes, the texture needs {bytes}"
        )));
    }
    rgba_out.copy_from_nonoverlapping(rgba.as_ptr(), bytes);
    int(bytes)
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_texture_key(
    h: *const ParsedCosmetic,
    index: u32,
    colorway: i32,
    custom_slots: *const u32,
    custom_rgb: *const u32,
    custom_count: u32,
    key_out: *mut u64,
) -> i32 {
    if key_out.is_null() {
        return free_fail(&Fail::Args("key_out is null".into()));
    }
    let keyed = custom(custom_slots, custom_rgb, custom_count)
        .and_then(|(slots, rgb)| key(&*h, i64::from(index), i64::from(colorway), slots, rgb));
    match keyed {
        Ok(key) => {
            key_out.write_unaligned(key);
            0
        }
        Err(fail) => free_fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn cosmetic_input_layout(
    sections: *const i32,
    sections_len: u32,
    out: *mut i32,
    out_len: u32,
) -> i32 {
    let shaped = view(sections, sections_len, "sections")
        .and_then(sections_of)
        .and_then(|s| Ok((s, view_mut(out, out_len, "out")?)));
    let (s, out) = match shaped {
        Ok(shaped) => shaped,
        Err(fail) => return free_fail(&fail),
    };
    let ints = input_layout(&s);
    let n = out.len().min(INPUT_LAYOUT_INTS);
    out[..n].copy_from_slice(&ints[..n]);
    ints[INPUT_STRIDE]
}

pub(crate) fn constants_json() -> String {
    let mut out = String::from("{");
    push_str(&mut out, "limits");
    out.push_str(":{");
    for (i, (name, value)) in constants().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_str(&mut out, name);
        out.push(':');
        push_num(&mut out, *value);
    }
    out.push_str("}}");
    out
}

#[no_mangle]
pub extern "C" fn cosmetic_constants_json() -> i32 {
    text_out(Ok(constants_json()))
}

#[no_mangle]
pub extern "C" fn cosmetic_pair_seed(a: u64, b: u64, minute: i64) -> u64 {
    pair_seed(a, b, minute)
}

#[no_mangle]
pub extern "C" fn cosmetic_actor_wearer(owner: u64, slot: u8) -> u64 {
    actor_wearer(owner, slot)
}

#[no_mangle]
pub extern "C" fn cosmetic_event_wearer(owner: u64, event: u8, seq: u32) -> u64 {
    event_wearer(owner, event, seq)
}

#[no_mangle]
pub extern "C" fn cosmetic_event_key(owner: u64, event: u8, seq: u32, nonce: u32) -> u64 {
    event_key(owner, event, seq, nonce)
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_load_asset(
    r: *mut Renderer,
    slot: u32,
    h: *const ParsedCosmetic,
) -> i32 {
    let r = &mut *r;
    if h.is_null() {
        return fail(r, "the asset handle is null");
    }
    match load_asset(r, i64::from(slot), &*h) {
        Ok(()) => 0,
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_load_emote(
    r: *mut Renderer,
    slot: u32,
    props: i32,
    h: *const ParsedCosmetic,
) -> i32 {
    let r = &mut *r;
    if h.is_null() {
        return fail(r, "the asset handle is null");
    }
    match load_emote(r, i64::from(slot), i64::from(props), &*h) {
        Ok(()) => 0,
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_frame_wearers_shaped(
    r: *mut Renderer,
    clock: f64,
    wearers: *const u64,
    wearer_count: u32,
    sections: *const i32,
    sections_len: u32,
    input: *const f32,
    input_len: u32,
    space: u32,
    out: *mut f32,
    out_len: u32,
) -> i32 {
    let r = &mut *r;
    let views = view(sections, sections_len, "sections")
        .and_then(sections_of)
        .and_then(|sections| {
            let wearers = view(wearers, wearer_count, "wearers")?;
            let input = view(input, input_len, "input")?;
            Ok((sections, wearers, input, view_mut(out, out_len, "out")?))
        });
    let framed = views.and_then(|(sections, wearers, input, out)| {
        frame_shaped(r, clock, wearers, &sections, input, space, out)
    });
    match framed {
        Ok(bones) => int(bones),
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_output_layout(
    r: *mut Renderer,
    sections: *const i32,
    sections_len: u32,
    out: *mut i32,
    out_len: u32,
) -> i32 {
    let r = &mut *r;
    let shaped = view(sections, sections_len, "sections")
        .and_then(sections_of)
        .and_then(|s| Ok((s, view_mut(out, out_len, "out")?)));
    let layout = shaped.and_then(|(s, out)| Ok((output_layout(r, &s)?, out)));
    let (ints, out) = match layout {
        Ok((layout, out)) => (layout_ints(&layout), out),
        Err(f) => return renderer_fail(r, &f),
    };
    let n = out.len().min(LAYOUT_INTS);
    out[..n].copy_from_slice(&ints[..n]);
    ints[LAYOUT_STRIDE]
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_host_variable_id(
    r: *mut Renderer,
    name_ptr: *const u8,
    name_len: u32,
) -> i32 {
    let r = &mut *r;
    match text_of(name_ptr, name_len, "name", "the variable name is") {
        Ok(name) => int(r.scene.host_variable_id(name)),
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_locator_id(
    r: *mut Renderer,
    name_ptr: *const u8,
    name_len: u32,
) -> i32 {
    let r = &mut *r;
    match text_of(name_ptr, name_len, "name", "the locator name is")
        .and_then(|name| locator_id(r, name))
    {
        Ok(id) => int(id),
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_set_rig(r: *mut Renderer, kind: u32) -> i32 {
    let r = &mut *r;
    match set_rig(r, i64::from(kind)) {
        Ok(()) => 0,
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_set_physics(r: *mut Renderer, mode: u32, own: u64) -> i32 {
    let r = &mut *r;
    match set_physics(r, i64::from(mode), own) {
        Ok(()) => 0,
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_fire_effect(
    r: *mut Renderer,
    wearer: u64,
    name_ptr: *const u8,
    name_len: u32,
) -> i32 {
    let r = &mut *r;
    match text_of(name_ptr, name_len, "name", "the effect name is") {
        Ok(name) => i32::from(r.scene.fire_named(wearer, name)),
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_set_effect_colors(
    r: *mut Renderer,
    wearer: u64,
    colorway: i32,
    custom_slots: *const u32,
    custom_rgb: *const u32,
    custom_count: u32,
) -> i32 {
    let r = &mut *r;
    let set = custom(custom_slots, custom_rgb, custom_count)
        .and_then(|(slots, rgb)| set_effect_colors(r, wearer, i64::from(colorway), slots, rgb));
    match set {
        Ok(()) => 0,
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_has_effects(r: *const Renderer) -> u32 {
    u32::from((&*r).scene.has_effects())
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_effect_geometry(
    r: *mut Renderer,
    view_ptr: *const f32,
    view_len: u32,
    out: *mut f32,
    out_len: u32,
) -> i32 {
    let r = &mut *r;
    let sizing = out.is_null();
    let views = view(view_ptr, view_len, "view")
        .and_then(view_of)
        .and_then(|v| Ok((v, view_mut(out, out_len, "out")?)));
    match views.and_then(|(v, out)| effect_geometry(r, &v, out, sizing)) {
        Ok(quads) => int(quads),
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_sample_effect_rings(
    r: *mut Renderer,
    view_ptr: *const f32,
    view_len: u32,
) -> i32 {
    let r = &mut *r;
    let scene = &mut r.scene;
    let sampled = view(view_ptr, view_len, "view")
        .and_then(view_of)
        .and_then(|v| {
            guarded(
                Err(Fail::Core("the core failed sampling effect rings")),
                || {
                    scene.sample_effect_rings(&v);
                    Ok(())
                },
            )
        });
    match sampled {
        Ok(()) => 0,
        Err(f) => renderer_fail(r, &f),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_effect_event_floats(
    r: *const Renderer,
    out: *mut f32,
    out_len: u32,
) -> u32 {
    let floats = (&*r).scene.effect_event_floats();
    if !out.is_null() {
        let whole = whole_events(floats, out_len as usize);
        out.copy_from_nonoverlapping(whole.as_ptr(), whole.len());
    }
    (floats.len() / EFFECT_EVENT_FLOATS) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_effect_texture_count(r: *const Renderer) -> u32 {
    (&*r).scene.effect_texture_count() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_effect_texture(
    r: *const Renderer,
    id: u32,
    slot_out: *mut u32,
    path_out: *mut u8,
    path_len: u32,
) -> i32 {
    let Some((slot, path)) = (&*r).scene.effect_texture(id as usize) else {
        return -1;
    };
    if !slot_out.is_null() {
        *slot_out = slot as u32;
    }
    if !path_out.is_null() {
        let n = path.len().min(path_len as usize);
        path_out.copy_from_nonoverlapping(path.as_ptr(), n);
    }
    int(path.len())
}

#[cfg(test)]
mod tests {
    use plusrenderer::attach::{actor_wearer, event_wearer, RigKind};
    use plusrenderer::effects::{
        CORNER_FLOATS, QUAD_CORNERS, QUAD_FLAGS, QUAD_FLAG_EMISSIVE, QUAD_FLOATS,
    };
    use plusrenderer::limits::{EFFECT_PARTICLE_LIFE_SECONDS, EVENT_EFFECT_SECONDS};
    use plusrenderer::scene::{
        Physics, COLORWAY_FLOATS, EFFECT_EVENT_FLOATS, EMOTE_CANCEL, EMOTE_CANCEL_CROUCHING,
        EMOTE_CANCEL_ELYTRA_FLYING, EMOTE_CANCEL_FALLING, EMOTE_CANCEL_SLEEPING,
        EMOTE_CANCEL_SPRINTING, EMOTE_CANCEL_SWIMMING, EMOTE_CANCEL_SWINGING, EMOTE_CANCEL_WALKING,
        EMOTE_ENDED, EMOTE_HELD, EMOTE_POSING, EMOTE_STATUS_FLOATS, EMOTE_STOPPING, HOLD_FLOATS,
        LOCATOR_FLOATS, NEUTRAL_SIGNALS, NO_COLORWAY, PART_DELTA_FLOATS, PART_FLOATS, PLAY_FLOATS,
        ROLE_FLOATS, ROOT_FLOATS,
    };
    use plusrenderer::signals::{
        signal_id, EVENT_ATTACK_SWING, EVENT_DEATH, EVENT_DIMENSION_CHANGE, EVENT_EQUIP,
        EVENT_HURT, EVENT_JUMP, EVENT_RESPAWN, EVENT_SLEEP, SIGNAL_COUNT,
    };
    use std::ptr::{null, null_mut};

    use super::*;
    use crate::tests::{bundle, file, n, tree, zip};
    use crate::{renderer_free, renderer_load_player, renderer_new, renderer_play};

    const LAYOUT_LOCATORS: usize = 2;
    const LAYOUT_COLORWAY: usize = 3;
    const LAYOUT_RUNS_AT: usize = 5;
    const LAYOUT_LOCATORS_AT: usize = 6;
    const LAYOUT_COLORWAY_AT: usize = 7;
    const LAYOUT_EMOTE: usize = 10;
    const LAYOUT_EMOTE_AT: usize = 11;
    const INPUT_HOLD_AT: usize = 8;
    const CAPS: &str = "controllers\nreactions\npalettes\ncustom_colors";
    const ROLES: &str = "controllers\nroles\nevents";
    const EFFECT_CAPS: &str = "effects\nparticles\nsounds\ntrails\nevents";
    const COLOR_SECTIONS: [i32; SECTION_INTS] = [0, 0, 0, 0, 0, 0, 1, 0, 0];
    const PLAYED: [i32; SECTION_INTS] = [0, 0, 0, 0, 1, 0, 0, 0, 0];

    const WINGS: &str = "tests/data/sample_wings";
    const SPARK: &str = "tests/data/sample_spark";

    const HEART_CLIPS: &str = r#"{"format_version":"1.8.0","animations":{
        "animation.heart.emote":{"animation_length":1.5,
          "bones":{"rightArm":{"rotation":[-90,0,0]},"heart":{"scale":[2,2,2]}}},
        "animation.heart.extra":{"loop":true,"animation_length":0.5}}}"#;

    const HEART_GEOMETRY: &[u8] = br#"{"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.heart","texture_width":16,"texture_height":16},
        "bones":[{"name":"main","pivot":[0,0,0]},
          {"name":"heart","parent":"main","pivot":[5,14,-3],
            "cubes":[{"origin":[4,13,-4],"size":[2,2,2],"uv":[0,0]}]}]}]}"#;

    fn wings() -> Vec<u8> {
        bundle(&tree(WINGS))
    }

    fn png() -> Vec<u8> {
        file("tests/data/sample_wings/textures/wings.png")
    }

    fn heart(clips: &str) -> Vec<u8> {
        let rules = br#"{"animations":{"animation.heart.emote":{"rules":{"allow_walking":true}}}}"#;
        zip(&[
            (
                "emotes/heart_emote/heart_emote.animation.json",
                clips.as_bytes(),
            ),
            ("emotes/heart_emote/heart_emote.emote.json", rules),
            ("emotes/heart_emote/heart_emote.geo.json", HEART_GEOMETRY),
            ("heart_emote.png", &png()),
        ])
    }

    unsafe fn parse(bytes: &[u8], caps: Option<&str>) -> *const ParsedCosmetic {
        let (ptr, len) = caps.map_or((null(), 0), |c| (c.as_ptr(), n(c)));
        let h = cosmetic_parse(bytes.as_ptr(), bytes.len() as u32, 0, ptr, len, 42);
        assert!(!h.is_null(), "{}", error());
        h
    }

    fn error() -> String {
        unsafe {
            let bytes = std::slice::from_raw_parts(cosmetic_error(), cosmetic_error_len() as usize);
            String::from_utf8_lossy(bytes).into_owned()
        }
    }

    fn text(len: i32) -> String {
        assert!(len >= 0, "{}", error());
        unsafe {
            let bytes = std::slice::from_raw_parts(cosmetic_text(), len as usize);
            String::from_utf8(bytes.to_vec()).unwrap()
        }
    }

    unsafe fn loaded(bundle: &[u8], caps: &str) -> (*mut Renderer, *const ParsedCosmetic) {
        let h = parse(bundle, Some(caps));
        let r = renderer_new();
        assert_eq!(renderer_load_asset(r, 0, h), 0, "{}", (*r).error);
        (r, h)
    }

    #[rustfmt::skip]
    unsafe fn frame(r: *mut Renderer, clock: f64, sections: &[i32], out: &mut [f32]) -> i32 {
        let (wearers, input) = ([1u64, 2], rows(true));
        let (count, input_len, out_len) = (sections.len() as u32, input.len() as u32, out.len() as u32);
        let (sections, input) = (sections.as_ptr(), input.as_ptr());
        let out_ptr = out.as_mut_ptr();
        renderer_frame_wearers_shaped(
            r, clock, wearers.as_ptr(), 2, sections, count, input, input_len, 0, out_ptr, out_len,
        )
    }

    fn rows(wet: bool) -> Vec<f32> {
        let rain = signal_id("is_in_rain").unwrap();
        let mut input = Vec::new();
        for raining in [false, wet] {
            let mut row = NEUTRAL_SIGNALS.to_vec();
            row[rain] = f32::from(u8::from(raining));
            row.extend_from_slice(&[0.0, 0.0]);
            input.extend(row);
        }
        input
    }

    unsafe fn layout_of(r: *mut Renderer, sections: &[i32; SECTION_INTS]) -> [i32; LAYOUT_INTS] {
        let mut layout = [0i32; LAYOUT_INTS];
        let (count, out) = (SECTION_INTS as u32, layout.as_mut_ptr());
        let stride = renderer_output_layout(r, sections.as_ptr(), count, out, LAYOUT_INTS as u32);
        assert!(stride > 0, "{}", (*r).error);
        assert_eq!(layout[LAYOUT_STRIDE], stride);
        layout
    }

    #[rustfmt::skip]
    #[test]
    fn bundles_read_back_as_json() {
        let mut reacting = tree(WINGS);
        let manifest = &mut reacting.iter_mut().find(|f| f.0 == "manifest.json").unwrap().1;
        *manifest = String::from_utf8_lossy(manifest).replace(
            r#""reactions": ["#,
            r#""reactions": [{ "on": "equip", "play": "flinch" }, { "on": "appear", "play": "flinch" },"#,
        ).into_bytes();
        let delayed = HEART_CLIPS
            .replace(r#"1.5,"#, r#"1.5,"start_delay":"q.is_sneaking","#)
            .replace(r#""loop":true,"#, r#""loop":true,"start_delay":"2.5","#);
        let cases: [(Vec<u8>, Option<&str>, &[&str]); 3] = [
            (
                wings(),
                Some(CAPS),
                &[
                    r#"{"id":"sample_wings","identity":"42","kind":"item""#,
                    r#""attach":{"rig":"player","bone":"body","replaces":null,"anchor":null}"#,
                    r#"{"signal":"is_in_rain","signal_index":4,"on":"while","play":"animation.wings.droop""#,
                    r#""colorways":["storm"]"#,
                    r#""indexed":true"#,
                ],
            ),
            (
                bundle(&reacting),
                Some(CAPS),
                &[
                    r#"{"signal":null,"signal_index":254,"on":"equip","play":"animation.wings.flinch""#,
                    r#"{"signal":null,"signal_index":255,"on":"appear","play":"animation.wings.flinch""#,
                ],
            ),
            (
                heart(&delayed),
                Some(ROLES),
                &[
                    r#""clips":[{"name":"animation.heart.emote","length":1.5,"loop":"once"},{"name":"animation.heart.extra","length":0.5,"loop":"repeat","start_delay":2.5}]"#,
                ],
            ),
        ];
        unsafe {
            for (bytes, caps, expected) in cases {
                let h = parse(&bytes, caps);
                let issues = text(cosmetic_asset_issues_json(h));
                assert_eq!(cosmetic_asset_status(h), STATUS_ASSET, "{issues}");
                let meta = text(cosmetic_asset_meta_json(h));
                for key in expected {
                    assert!(meta.contains(key), "{key} in {meta}");
                }
                cosmetic_asset_free(h);
            }
            let h = parse(&wings(), Some(CAPS));
            let issues = text(cosmetic_asset_issues_json(h));
            assert_eq!(issues, r#"{"status":"asset","skip":null,"legacy":null,"issues":[]}"#);
            let analysis = text(cosmetic_asset_analyze_json(h));
            assert!(analysis.contains(r#""quads":6"#), "{analysis}");
            cosmetic_asset_free(h);
            cosmetic_asset_free(null());
        }
    }

    #[rustfmt::skip]
    #[test]
    fn skipped_legacy_and_invalid_bundles_say_why() {
        unsafe {
            let invalid = zip(&[("manifest.json", b"{")]);
            for (bytes, caps, status, issues) in [
                (wings(), None, STATUS_SKIPPED, r#"{"status":"skipped","skip":{"requires":["controllers""#),
                (b"not a bundle".to_vec(), Some(CAPS), STATUS_LEGACY, r#"{"status":"legacy","skip":null,"legacy":{"files":["asset.bin"]},"issues":[]}"#),
                (invalid, Some(CAPS), STATUS_INVALID, r#"{"status":"invalid","skip":null,"legacy":null,"issues":[{"#),
            ] {
                let h = parse(&bytes, caps);
                assert_eq!(cosmetic_asset_status(h), status);
                let json = text(cosmetic_asset_issues_json(h));
                assert!(json.starts_with(issues), "{json}");
                cosmetic_asset_free(h);
            }
        }
    }

    #[rustfmt::skip]
    #[test]
    fn bad_arguments_fail_with_a_message() {
        unsafe {
            let bytes = wings();
            let len = bytes.len() as u32;
            let h = parse(&bytes, Some(CAPS));
            let skipped = parse(&bytes, None);
            let heart = parse(&heart(HEART_CLIPS), Some(ROLES));
            let r = renderer_new();
            assert_eq!(renderer_load_asset(r, 0, h), 0);
            let stride = layout_of(r, &COLOR_SECTIONS)[LAYOUT_STRIDE] as u32;
            let mut out = vec![0.0f32; stride as usize * 2];
            let mut buf = [0u8; 16];
            let (rgb, bad) = ([0xff0000u32], [0xffu8]);
            let e = |code: i32| (code, error());
            let re = |code: i32| (code, (*r).error.clone());
            let nul = |h: *const ParsedCosmetic| e(if h.is_null() { -1 } else { 0 });
            let shaped = |wearers: *const u64, input: *const f32, out: *mut f32| {
                let s = COLOR_SECTIONS.as_ptr();
                re(renderer_frame_wearers_shaped(r, 1.0, wearers, 2, s, 9, input, 50, 0, out, stride * 2))
            };
            let input = rows(true);
            let (input, out_ptr) = (input.as_ptr(), out.as_mut_ptr());
            let resolve = |colorway: i32, slots: *const u32, rgb: *const u32, count: u32| {
                e(cosmetic_resolve_texture(h, 0, colorway, slots, rgb, count, null_mut(), 0))
            };
            let rows: Vec<((i32, String), &str)> = vec![
                (nul(cosmetic_parse(null(), 9, 0, null(), 0, 1)), "bundle is null with a length of 9"),
                (nul(cosmetic_parse(bytes.as_ptr(), len, 9, null(), 0, 1)), "no mode 9"),
                (resolve(-1, null(), rgb.as_ptr(), 1), "custom_slots is null with a length of 1"),
                (e(cosmetic_resolve_texture(h, 0, -1, null(), null(), 0, buf.as_mut_ptr(), 16)), "rgba_out holds 16 bytes"),
                (e(cosmetic_asset_meta_json(skipped)), "it is skipped"),
                (shaped(null(), input, out_ptr), "wearers is null with a length of 2"),
                (re(frame(r, 4.0, &COLOR_SECTIONS, &mut out[1..])), "out holds"),
                (re(frame(r, 4.0, &[0, 0, 2, 0, 0, 0, 0, 0], &mut out)), "section root is 2, not 0 or 1"),
                (re(renderer_fire_effect(r, 7, bad.as_ptr(), 1)), "the effect name is not utf-8"),
                (re(renderer_set_rig(r, 6)), "no rig 6"),
                (re(renderer_load_asset(r, 256, h)), "no slot 256, the last is 255"),
                (re(renderer_load_emote(r, 0, 1, null())), "the asset handle is null"),
                (re(renderer_load_asset(r, 1, skipped)), "the bundle did not load: it is skipped"),
            ];
            for ((code, message), expected) in rows {
                assert_eq!(code, -1, "{expected}");
                assert!(message.contains(expected), "{message} for {expected}");
            }
            assert_eq!(cosmetic_bundle_identity(null(), 4), 0);
            assert_eq!(error(), "bundle is null with a length of 4");
            assert!(frame(r, 4.0, &COLOR_SECTIONS[..8], &mut out) >= 0);
            fault_next_frame(r);
            assert_eq!(frame(r, 5.0, &COLOR_SECTIONS, &mut out), -2);
            assert_eq!((*r).error, "the core failed on this frame");
            assert!(frame(r, 5.1, &COLOR_SECTIONS, &mut out) >= 0);
            renderer_free(r);
            for h in [h, skipped, heart] {
                cosmetic_asset_free(h);
            }
        }
    }

    #[rustfmt::skip]
    #[test]
    fn textures_resolve_through_colorways_and_custom_colours() {
        unsafe {
            let h = parse(&wings(), Some(CAPS));
            assert_eq!(cosmetic_texture_count(h), 1);
            let mut info = [0i32; TEXTURE_INFO_INTS];
            assert_eq!(cosmetic_texture_info(h, 0, info.as_mut_ptr()), 16 * 16 * 4);
            assert_eq!(info, [16, 16, 1]);
            let resolve = |colorway: i32, slots: &[u32], rgb: &[u32]| {
                let (s, c, count, mut out, mut key) = (slots.as_ptr(), rgb.as_ptr(), slots.len() as u32, vec![0u8; 1024], 0u64);
                assert_eq!(cosmetic_resolve_texture(h, 0, colorway, s, c, count, null_mut(), 0), 1024);
                assert_eq!(cosmetic_resolve_texture(h, 0, colorway, s, c, count, out.as_mut_ptr(), 1024), 1024);
                assert_eq!(cosmetic_texture_key(h, 0, colorway, s, c, count, &mut key), 0);
                (out, key)
            };
            let (plain, storm, custom) = (resolve(-1, &[], &[]), resolve(0, &[], &[]), resolve(-1, &[0], &[0xff0000]));
            assert_eq!(plain, resolve(-1, &[], &[]));
            assert!(plain.0 != storm.0 && plain.0 != custom.0 && storm.0 != custom.0);
            assert!(plain.1 != storm.1 && plain.1 != custom.1);
            cosmetic_asset_free(h);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn shaped_frames_carry_poses_colorways_locators_and_emote_statuses() {
        unsafe {
            let (r, h) = loaded(&wings(), CAPS);
            cosmetic_asset_free(h);
            renderer_play(r, 0, 0.0);
            assert_eq!(renderer_locator_id(r, "head".as_ptr(), 4), 0);
            let layout = layout_of(r, &COLOR_SECTIONS);
            assert_eq!((layout[LAYOUT_LOCATORS], layout[LAYOUT_COLORWAY]), (1, 1));
            let mut input_layout = [0i32; INPUT_LAYOUT_INTS];
            let in_stride = cosmetic_input_layout(COLOR_SECTIONS.as_ptr(), 9, input_layout.as_mut_ptr(), 9);
            assert_eq!((in_stride as usize, input_layout[INPUT_STRIDE]), (SIGNAL_COUNT + 2, in_stride));
            let stride = layout[LAYOUT_STRIDE] as usize;
            let mut out = vec![0.0f32; stride * 2];
            assert!(frame(r, 1.0, &COLOR_SECTIONS, &mut out) >= 0 && frame(r, 3.0, &COLOR_SECTIONS, &mut out) >= 0);
            let bones = layout[LAYOUT_RUNS_AT] as usize;
            assert_ne!(out[..bones], out[stride..stride + bones], "the droop moves the wing");
            let (colorway, locator) = (layout[LAYOUT_COLORWAY_AT] as usize, layout[LAYOUT_LOCATORS_AT] as usize);
            assert_eq!((out[colorway], out[stride + colorway]), (NO_COLORWAY as f32, 0.0), "storm while it rains");
            assert!(out[locator..locator + LOCATOR_FLOATS].iter().any(|v| *v != 0.0));
            let mood = renderer_host_variable_id(r, "variable.mood".as_ptr(), 13);
            assert_eq!(renderer_host_variable_id(r, "v.mood".as_ptr(), 6), mood);
            assert_eq!((renderer_set_rig(r, 2), (*r).scene.rig()), (0, RigKind::Actor));
            assert_eq!((renderer_set_physics(r, PHYSICS_OWN, 5), (*r).scene.physics()), (0, Physics::Own(5)));
            renderer_free(r);

            let h = parse(&heart(HEART_CLIPS), Some(ROLES));
            let r = renderer_new();
            renderer_load_player(r, 0, 0);
            assert_eq!(renderer_load_emote(r, 0, 1, h), 0, "{}", (*r).error);
            assert!((*r).scene.bone_pose(1, "heart").is_some() && (*r).scene.bone_pose(0, "rightArm").is_some());
            assert_eq!((renderer_load_emote(r, 0, -1, h), renderer_load_emote(r, 0, 255, h)), (0, 0));
            let plain = layout_of(r, &[0; SECTION_INTS]);
            assert_eq!((plain[LAYOUT_EMOTE], plain[LAYOUT_EMOTE_AT]), (0, plain[LAYOUT_STRIDE]));
            let layout = layout_of(r, &PLAYED);
            assert_eq!((layout[LAYOUT_EMOTE], layout[LAYOUT_EMOTE_AT] + EMOTE_STATUS_FLOATS as i32), (1, layout[LAYOUT_STRIDE]));
            let mut row = NEUTRAL_SIGNALS.to_vec();
            row.extend_from_slice(&[0.0, 0.0, 0.0, 0.5]);
            let mut out = vec![f32::NAN; layout[LAYOUT_STRIDE] as usize];
            let (input, len, out_len) = (row.as_ptr(), row.len() as u32, out.len() as u32);
            assert!(renderer_frame_wearers_shaped(r, 10.5, &5, 1, PLAYED.as_ptr(), 9, input, len, 0, out.as_mut_ptr(), out_len) >= 0);
            let status = out[layout[LAYOUT_EMOTE_AT] as usize] as u32;
            assert_eq!(status & EMOTE_POSING, EMOTE_POSING, "{status}");
            renderer_free(r);
            cosmetic_asset_free(h);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn effect_pixels_and_sounds_read_back_and_fire_by_name() {
        unsafe {
            let (r, h) = loaded(&bundle(&tree(SPARK)), EFFECT_CAPS);
            let mut info = [0i32; TEXTURE_INFO_INTS];
            assert_eq!((cosmetic_effect_texture_count(h), cosmetic_effect_texture_info(h, 1, info.as_mut_ptr())), (2, 32));
            assert_eq!(info, [4, 2, 0]);
            let mut rgba = [0u8; 16];
            assert_eq!(cosmetic_effect_texture(h, 0, null_mut(), 0), 16);
            assert_eq!(cosmetic_effect_texture(h, 0, rgba.as_mut_ptr(), 16), 16);
            assert_eq!((rgba[..4].to_vec(), rgba[12..].to_vec()), (vec![255, 0, 0, 255], vec![255, 255, 255, 128]));
            assert_eq!(cosmetic_effect_texture(h, 0, rgba.as_mut_ptr(), 8), -1);
            assert!(error().contains("holds 8 bytes"), "{}", error());
            let pop = file("tests/data/sample_spark/sounds/pop.ogg");
            let mut bytes = vec![0u8; pop.len()];
            assert_eq!((cosmetic_sound_count(h), cosmetic_sound(h, 0, null_mut(), 0)), (2, pop.len() as i32));
            assert_eq!(cosmetic_sound(h, 0, bytes.as_mut_ptr(), pop.len() as u32), pop.len() as i32);
            assert_eq!((bytes, cosmetic_sound(h, 1, null_mut(), 0)), (pop, 0));
            assert_eq!(cosmetic_sound(h, 2, null_mut(), 0), -1);
            let meta = text(cosmetic_asset_meta_json(h));
            assert!(meta.contains(r#""emitter_vanilla":[null,null,"minecraft:flame"]"#), "{meta}");
            assert!(meta.contains(r#""moments":[{"name":"join","clip":null,"seconds":"#), "{meta}");
            assert_eq!((renderer_fire_effect(r, 7, "join".as_ptr(), 4), renderer_fire_effect(r, 7, "kill".as_ptr(), 4)), (1, 0));
            assert_eq!(renderer_set_effect_colors(r, 7, 0, [0].as_ptr(), [0xff0000].as_ptr(), 1), 0);
            assert_eq!(renderer_set_effect_colors(r, 7, -1, null(), null(), 0), 0);
            assert_eq!(renderer_fire_effect(r, 1, "join".as_ptr(), 4), 1);
            let (mut out, mut quads) = (vec![0.0f32; 4096], vec![0.0f32; QUAD_FLOATS * 64]);
            let view = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
            let drawn = (1..9).map(|step| {
                assert!(frame(r, 1.0 + f64::from(step) * 0.05, &[0; SECTION_INTS], &mut out) >= 0);
                renderer_effect_geometry(r, view.as_ptr(), 12, quads.as_mut_ptr(), quads.len() as u32)
            });
            assert!(drawn.max() > Some(0), "the join sparks come back as quads");
            assert_eq!(renderer_sample_effect_rings(r, view.as_ptr(), 12), 0);
            renderer_free(r);
            cosmetic_asset_free(h);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn identities_and_seeds_match_the_core() {
        let wings = wings();
        let of = |bytes: &[u8]| unsafe { cosmetic_bundle_identity(bytes.as_ptr(), bytes.len() as u32) };
        assert_eq!((of(&wings), unsafe { cosmetic_bundle_identity(null(), 0) }), (of(&wings.clone()), of(b"")));
        assert_ne!(of(&wings), of(&heart(HEART_CLIPS)));
        assert_eq!((cosmetic_pair_seed(3, 9, 100), cosmetic_actor_wearer(7, 1)), (cosmetic_pair_seed(9, 3, 100), actor_wearer(7, 1)));
        assert_ne!(cosmetic_pair_seed(3, 9, 100), cosmetic_pair_seed(3, 9, 101));
        assert_eq!((cosmetic_event_wearer(7, 2, 5), cosmetic_event_key(7, 2, 5, 3)), (event_wearer(7, 2, 5), event_key(7, 2, 5, 3)));
    }

    #[rustfmt::skip]
    #[test]
    fn layouts_and_constants_match_what_hosts_hard_code() {
        let held: [i32; SECTION_INTS] = [1, 0, 0, 0, 1, 1, 0, 0, 1];
        let (mut out, mut legacy) = ([0i32; INPUT_LAYOUT_INTS], [0i32; 8]);
        unsafe {
            let stride = cosmetic_input_layout(held.as_ptr(), 9, out.as_mut_ptr(), 9);
            let play_at = PART_FLOATS + SIGNAL_COUNT + 2;
            assert_eq!(out[INPUT_HOLD_AT] as usize, play_at + PLAY_FLOATS + ROLE_FLOATS);
            assert_eq!((stride, out[INPUT_STRIDE]), (out[INPUT_HOLD_AT] + HOLD_FLOATS as i32, stride));
            let unheld = cosmetic_input_layout(held.as_ptr(), 8, legacy.as_mut_ptr(), 8);
            assert_eq!((unheld, legacy[INPUT_STRIDE]), (out[INPUT_HOLD_AT], out[INPUT_HOLD_AT]));
        }
        let json = text(cosmetic_constants_json());
        assert!(json.starts_with("{\"limits\":{\"cape_download_bytes\":16777216,"), "{json}");
        assert!(json.contains("\"quads_per_wearer\":2000,") && json.ends_with("\"shared_break_blocks\":1}}"), "{json}");
        let sizes = [
            ROOT_FLOATS, PLAY_FLOATS, ROLE_FLOATS, LOCATOR_FLOATS, COLORWAY_FLOATS, PART_DELTA_FLOATS, EFFECT_EVENT_FLOATS,
            QUAD_FLOATS, QUAD_CORNERS, CORNER_FLOATS, QUAD_FLAGS, SIGNAL_COUNT, LAYOUT_INTS, LAYOUT_STRIDE, EMOTE_STATUS_FLOATS,
            SECTION_INTS, UNHELD_SECTION_INTS, INPUT_LAYOUT_INTS, INPUT_STRIDE, HOLD_FLOATS,
        ];
        assert_eq!(sizes, [12, 2, 5, 12, 1, 10, 5, 27, 6, 5, 26, 23, 12, 9, 1, 9, 8, 9, 7, 1]);
        let statuses = [
            EMOTE_POSING, EMOTE_ENDED, EMOTE_CANCEL, EMOTE_STOPPING, EMOTE_CANCEL_WALKING, EMOTE_CANCEL_SPRINTING,
            EMOTE_CANCEL_CROUCHING, EMOTE_CANCEL_FALLING, EMOTE_CANCEL_ELYTRA_FLYING, EMOTE_CANCEL_SWIMMING,
            EMOTE_CANCEL_SLEEPING, EMOTE_CANCEL_SWINGING, EMOTE_HELD,
        ];
        assert_eq!(statuses, [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]);
        let events = [EVENT_HURT, EVENT_ATTACK_SWING, EVENT_DEATH, EVENT_RESPAWN, EVENT_DIMENSION_CHANGE, EVENT_SLEEP, EVENT_JUMP, EVENT_EQUIP];
        assert_eq!(events, [1, 2, 4, 8, 16, 32, 64, 128]);
        assert_eq!((QUAD_FLAG_EMISSIVE, [EVENT_EFFECT_SECONDS, EFFECT_PARTICLE_LIFE_SECONDS]), (1, [3.0, 3.1]));
        let rigs: Vec<(u32, &str)> = RigKind::ALL.iter().map(|k| (k.index(), k.as_str())).collect();
        assert_eq!(rigs, [(0, "player"), (2, "actor"), (3, "boat"), (4, "minecart"), (5, "horse")]);
    }
}
