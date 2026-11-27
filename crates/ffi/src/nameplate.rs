use std::cell::RefCell;
use std::fmt::Write;
use std::sync::Arc;

use plusrenderer::container::{Image, Skip, Source, CAPABILITIES};
use plusrenderer::cost::{Detail, Motion};
use plusrenderer::issues::{Issue, Level, Mode};
use plusrenderer::molang::WarningKind;
use plusrenderer::nameplate::abi::{
    read_block, FLAG_ANIMATED_NAMES_OFF, FLAG_EFFECTS_OFF, GLYPH_FLOATS, TAG_DETAIL, TAG_FLAGS,
    TAG_INTS, TAG_VARIANT,
};
use plusrenderer::nameplate::constants::{
    accepts_name, anchors, capabilities, constants, limits, nodes, signals,
};
use plusrenderer::nameplate::material::glsl::{loose_uniforms, unsupported};
use plusrenderer::nameplate::material::ir::{Blend, Dialect, ShaderSource, TexRef};
use plusrenderer::nameplate::material::nodes::SocketDefault;
use plusrenderer::nameplate::queries::MolangUse;
use plusrenderer::nameplate::raster::{
    raster, raster_layers_with, raster_with, RasterBand, RasterOptions, RasterShot, RasterState,
    RasterStop, RasterTextures, INFO_WORDS,
};
use plusrenderer::nameplate::types::{LayerContent, Phase, Trigger};
use plusrenderer::nameplate::validate::dialect_issues;
use plusrenderer::nameplate::{
    analyze, check_molang, load, param_curve, shaders, FallbackImage, FallbackJob, LoadOptions,
    Loaded, NameplateAsset, NameplateRenderer,
};
use plusrenderer::signals::KNOWN_QUERIES;
use plusrenderer::signals::{SIGNALS, SIGNAL_COUNT};

use super::{checked, checked_mut, guarded};

pub const STATUS_ASSET: i32 = 0;
pub const STATUS_DRAFT: i32 = 1;
pub const STATUS_SKIPPED: i32 = 2;
pub const STATUS_INVALID: i32 = 3;

pub const RASTER_INFO_INTS: usize = 4;

pub struct ParsedAsset {
    pub(crate) status: i32,
    pub(crate) asset: Option<Arc<NameplateAsset>>,
    issues: Vec<Issue>,
    skip: Option<Skip>,
}

pub struct Nameplates {
    pub(crate) renderer: NameplateRenderer,
    pub(crate) error: String,
    pub(crate) fault: bool,
}

impl Nameplates {
    pub(crate) fn new() -> Nameplates {
        Nameplates {
            renderer: NameplateRenderer::new(),
            error: String::new(),
            fault: false,
        }
    }

    pub(crate) fn fail(&mut self, fail: &Fail) -> i32 {
        self.error.clear();
        self.error.push_str(fail.message());
        fail.code()
    }
}

pub enum Fallback {
    Job(Box<FallbackJob>),
    Image(FallbackImage),
    Spent,
}

impl Fallback {
    pub(crate) fn run(&mut self) -> bool {
        match std::mem::replace(self, Fallback::Spent) {
            Fallback::Job(job) => {
                *self = Fallback::Image(job.run());
                true
            }
            other => {
                *self = other;
                false
            }
        }
    }
}

pub(crate) fn run_fallbacks(n: &mut Nameplates, limit: i64) -> Result<usize, Fail> {
    let Ok(limit) = usize::try_from(limit) else {
        return Err(Fail::Args(format!(
            "nameplate: fallback limit {limit} is negative"
        )));
    };
    let renderer = &mut n.renderer;
    caught("the core failed making effect fallbacks", || {
        Ok(renderer.run_fallbacks(limit))
    })
}

pub(crate) fn finish_fallback(n: &mut Nameplates, mut fallback: Fallback) -> Result<bool, Fail> {
    fallback.run();
    let Fallback::Image(image) = fallback else {
        return Ok(false);
    };
    let renderer = &mut n.renderer;
    caught("the core failed storing an effect fallback", || {
        Ok(renderer.finish_fallback(image))
    })
}

pub(crate) enum Fail {
    Args(String),
    Core(&'static str),
}

impl Fail {
    pub(crate) fn code(&self) -> i32 {
        match self {
            Fail::Args(_) => -1,
            Fail::Core(_) => -2,
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Fail::Args(message) => message,
            Fail::Core(message) => message,
        }
    }
}

thread_local! {
    static ERROR: RefCell<String> = const { RefCell::new(String::new()) };
    static TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

fn caught<T>(what: &'static str, f: impl FnOnce() -> Result<T, Fail>) -> Result<T, Fail> {
    guarded(Err(Fail::Core(what)), f)
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

unsafe fn view<'a, T>(ptr: *const T, len: u32, name: &str) -> Result<&'a [T], Fail> {
    checked(ptr, len as usize, name).map_err(Fail::Args)
}

unsafe fn view_mut<'a, T>(ptr: *mut T, len: u32, name: &str) -> Result<&'a mut [T], Fail> {
    checked_mut(ptr, len as usize, name).map_err(Fail::Args)
}

unsafe fn text_of<'a>(ptr: *const u8, len: u32, name: &str, what: &str) -> Result<&'a str, Fail> {
    std::str::from_utf8(view(ptr, len, name)?).map_err(|_| Fail::Args(format!("{what} not utf-8")))
}

unsafe fn info_out<'a>(ptr: *mut i32, len: usize) -> Option<&'a mut [i32]> {
    (!ptr.is_null()).then(|| std::slice::from_raw_parts_mut(ptr, len))
}

pub(crate) fn mode_of(mode: i64) -> Result<Mode, Fail> {
    match mode {
        0 => Ok(Mode::Client),
        1 => Ok(Mode::Authoring),
        _ => Err(Fail::Args(format!("no mode {mode}"))),
    }
}

pub(crate) fn parse_asset(
    bytes: &[u8],
    mode: Mode,
    capabilities: Option<&str>,
    identity: u64,
) -> Result<ParsedAsset, Fail> {
    let names: Vec<&str> = match capabilities {
        Some(joined) => joined.split('\n').filter(|c| !c.is_empty()).collect(),
        None => CAPABILITIES.to_vec(),
    };
    let options = LoadOptions {
        mode,
        capabilities: &names,
        identity: (identity != 0).then_some(identity),
        dialect: None,
    };
    caught("the core failed on this bundle", || {
        Ok(match load(Source::Zip(bytes), &options) {
            Loaded::Asset { asset, issues } => parsed(STATUS_ASSET, Some(asset), issues, None),
            Loaded::Draft { asset, issues } => parsed(STATUS_DRAFT, Some(asset), issues, None),
            Loaded::Skipped(skip) => parsed(STATUS_SKIPPED, None, Vec::new(), Some(skip)),
            Loaded::Invalid(issues) => parsed(STATUS_INVALID, None, issues, None),
        })
    })
}

fn parsed(
    status: i32,
    asset: Option<Arc<NameplateAsset>>,
    issues: Vec<Issue>,
    skip: Option<Skip>,
) -> ParsedAsset {
    ParsedAsset {
        status,
        asset,
        issues,
        skip,
    }
}

impl ParsedAsset {
    pub(crate) fn asset(&self) -> Result<&Arc<NameplateAsset>, Fail> {
        self.asset.as_ref().ok_or_else(|| {
            Fail::Args(format!(
                "the bundle did not load: it is {}",
                status_name(self.status)
            ))
        })
    }
}

fn status_name(status: i32) -> &'static str {
    match status {
        STATUS_ASSET => "asset",
        STATUS_DRAFT => "draft",
        STATUS_SKIPPED => "skipped",
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

fn warning_name(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::UnknownFunction => "unknown_function",
        WarningKind::UnknownQuery => "unknown_query",
        WarningKind::UnknownName => "unknown_name",
        WarningKind::UnknownRoot => "unknown_root",
        WarningKind::Arity => "arity",
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

    fn int(&mut self, key: &str, value: i64) {
        let _ = write!(self.key(key), "{value}");
    }

    fn num(&mut self, key: &str, value: f64) {
        push_num(self.key(key), value);
    }

    fn bool(&mut self, key: &str, value: bool) {
        self.key(key).push_str(if value { "true" } else { "false" });
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

fn push_issues(out: &mut String, issues: &[Issue]) {
    push_list(out, issues, push_issue);
}

pub(crate) fn issues_json(p: &ParsedAsset) -> String {
    let mut out = String::new();
    let mut o = Obj::new(&mut out);
    o.str("status", status_name(p.status));
    match &p.skip {
        None => o.key("skip").push_str("null"),
        Some(Skip::Format(format)) => {
            let mut skip = Obj::new(o.key("skip"));
            skip.int("format", *format as i64);
            skip.end();
        }
        Some(Skip::Requires(names)) => {
            let mut skip = Obj::new(o.key("skip"));
            push_list(skip.key("requires"), names, |out, name| push_str(out, name));
            skip.end();
        }
    }
    push_issues(o.key("issues"), &p.issues);
    o.end();
    out
}

fn push_texture(out: &mut String, texture: &TexRef) {
    let mut o = Obj::new(out);
    match texture {
        TexRef::Bundle(index) => {
            o.str("kind", "bundle");
            o.int("index", i64::from(*index));
        }
        TexRef::Vanilla(vanilla) => {
            o.str("kind", "vanilla");
            o.str("name", vanilla.as_str());
        }
        TexRef::Atlas(name) => {
            o.str("kind", "atlas");
            o.str("name", name);
        }
    }
    o.end();
}

pub(crate) fn dialect_of(value: i64) -> Result<Dialect, Fail> {
    i32::try_from(value)
        .ok()
        .and_then(Dialect::from_value)
        .ok_or_else(|| Fail::Args(format!("no dialect {value}")))
}

pub(crate) fn shaders_json(p: &ParsedAsset, dialect: Dialect) -> Result<String, Fail> {
    let asset = p.asset()?;
    caught("the core failed generating shaders", || {
        let sources: Vec<ShaderSource> = shaders(asset, dialect);
        let issues = dialect_issues(asset, dialect);
        let mut out = String::new();
        let mut o = Obj::new(&mut out);
        o.str("dialect", dialect.as_str());
        push_list(o.key("shaders"), &sources, |out, s| {
            let ir = asset.materials.get(usize::from(s.material));
            let mut sh = Obj::new(out);
            sh.str("key", &s.key);
            sh.int("material", i64::from(s.material));
            sh.str("vertex", &s.vertex);
            sh.str("fragment", &s.fragment);
            push_list(sh.key("samplers"), &s.samplers, |out, sampler| {
                let mut so = Obj::new(out);
                so.str("name", &sampler.name);
                so.int("unit", i64::from(sampler.unit));
                push_texture(so.key("texture"), &sampler.texture);
                so.end();
            });
            sh.int("uniform_words", i64::from(s.uniform_words));
            push_list(sh.key("tones"), &s.tones, |out, tone| {
                let _ = write!(out, "{tone}");
            });
            let blend = match s.blend {
                Blend::Alpha => "alpha",
                Blend::Additive => "additive",
            };
            sh.str("blend", blend);
            sh.bool("depth_prepass", s.depth_prepass);
            let loose = ir.map(|ir| loose_uniforms(ir, dialect)).unwrap_or_default();
            push_list(sh.key("loose_uniforms"), &loose, |out, u| {
                let mut uo = Obj::new(out);
                uo.str("name", &u.name);
                uo.bool("ints", u.ints);
                uo.int("word", i64::from(u.word));
                uo.int("count", i64::from(u.count));
                uo.end();
            });
            let missing = ir.map(|ir| unsupported(ir, dialect)).unwrap_or_default();
            push_list(sh.key("unsupported"), &missing, |out, kind| {
                push_str(out, kind.name());
            });
            sh.end();
        });
        push_issues(o.key("issues"), &issues);
        o.end();
        Ok(out)
    })
}

pub(crate) fn dialect_issues_json(p: &ParsedAsset, dialect: Dialect) -> Result<String, Fail> {
    let asset = p.asset()?;
    caught("the core failed checking this dialect", || {
        let issues = dialect_issues(asset, dialect);
        let mut out = String::new();
        let mut o = Obj::new(&mut out);
        o.str("dialect", dialect.as_str());
        push_issues(o.key("issues"), &issues);
        o.end();
        Ok(out)
    })
}

pub(crate) fn meta_json(p: &ParsedAsset) -> Result<String, Fail> {
    let a = p.asset()?;
    let mut out = String::new();
    let mut o = Obj::new(&mut out);
    o.str("id", &a.id);
    o.str("identity", &a.identity.to_string());
    o.int("fps", i64::from(a.fps));
    o.int("loop_frames", i64::from(a.loop_frames));
    let phase = match a.phase {
        Phase::Global => "global",
        Phase::PerPlayer => "per_player",
    };
    o.str("phase", phase);
    push_list(o.key("requires"), &a.requires, |out, name| {
        push_str(out, name)
    });
    push_list(o.key("variants"), &a.variants, |out, v| {
        let mut vo = Obj::new(out);
        vo.str("key", &v.key);
        vo.str("identity", &v.identity.to_string());
        match v.colorway {
            Some(colorway) => vo.int("colorway", i64::from(colorway)),
            None => vo.key("colorway").push_str("null"),
        }
        vo.bool("owns_background", v.owns_background);
        vo.end();
    });
    push_list(
        o.key("layers"),
        a.layers.iter().enumerate(),
        |out, (slot, layer)| {
            let mut lo = Obj::new(out);
            lo.int("slot", slot as i64);
            lo.str("id", &layer.id);
            let content = match layer.content {
                LayerContent::Body(_) => "body",
                LayerContent::Sprite(_) => "sprite",
                LayerContent::Emitter(_) => "emitter",
                LayerContent::Material(_) => "material",
            };
            lo.str("content", content);
            lo.str("depth", layer.depth.as_str());
            lo.end();
        },
    );
    let mut anim = Obj::new(o.key("animations"));
    push_list(anim.key("reactions"), &a.animations.reactions, |out, r| {
        let mut ro = Obj::new(out);
        ro.str("name", &r.clip.name);
        push_signal(&mut ro, r.signal);
        ro.end();
    });
    push_list(
        anim.key("one_shots"),
        &a.animations.one_shots,
        |out, one| {
            let mut so = Obj::new(out);
            so.str("name", &one.clip.name);
            let (trigger, signal) = match one.trigger {
                Trigger::Start(s) => ("start", Some(s)),
                Trigger::Stop(s) => ("stop", Some(s)),
                Trigger::Appear(s) => ("appear", Some(s)),
                Trigger::Equip => ("equip", None),
            };
            so.str("trigger", trigger);
            match signal {
                Some(signal) => push_signal(&mut so, signal),
                None => so.key("signal").push_str("null"),
            }
            so.bool("restart", one.restart);
            so.end();
        },
    );
    push_list(
        anim.key("schedules"),
        &a.animations.schedules,
        |out, schedule| {
            let mut so = Obj::new(out);
            so.str("id", &schedule.id);
            push_list(so.key("events"), &schedule.events, |out, e| {
                push_str(out, &e.clip.name)
            });
            so.end();
        },
    );
    anim.end();
    push_list(o.key("params"), &a.params, |out, param| {
        let mut po = Obj::new(out);
        po.str("id", &param.id);
        po.num("value", param.value);
        po.num("min", param.min);
        po.num("max", param.max);
        po.end();
    });
    push_list(o.key("textures"), &a.textures, |out, t| {
        let mut to = Obj::new(out);
        to.int("width", i64::from(t.width));
        to.int("height", i64::from(t.height));
        to.end();
    });
    o.end();
    Ok(out)
}

fn push_signal(o: &mut Obj<'_>, signal: u8) {
    match SIGNALS.get(usize::from(signal)) {
        Some((name, _)) => o.str("signal", name),
        None => o.key("signal").push_str("null"),
    }
    o.int("signal_index", i64::from(signal));
}

pub(crate) fn texture_count(p: &ParsedAsset) -> Result<usize, Fail> {
    Ok(p.asset()?.textures.len())
}

pub(crate) fn texture(p: &ParsedAsset, index: i64) -> Result<&Image, Fail> {
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

pub(crate) fn analyze_json(p: &ParsedAsset) -> Result<String, Fail> {
    let asset = p.asset()?;
    caught("the core failed analysing this bundle", || {
        let a = analyze(asset);
        let mut out = String::new();
        let mut o = Obj::new(&mut out);
        push_list(o.key("requires"), &a.requires, |out, name| {
            push_str(out, name)
        });
        o.int("quads", i64::from(a.quads));
        o.int("cpu_units", i64::from(a.cpu_units));
        o.int("image_bytes", i64::from(a.image_bytes));
        o.int("fallback_bytes", i64::from(a.fallback_bytes));
        o.int("record_bytes", i64::from(a.record_bytes));
        o.int("max_out_words", i64::from(a.max_out_words));
        o.num("material_build_ops", a.material_build_ops as f64);
        push_list(o.key("layers"), &a.layers, |out, layer| {
            let mut lo = Obj::new(out);
            lo.str("id", &layer.id);
            lo.int("states", i64::from(layer.states));
            lo.int("image_bytes", i64::from(layer.image_bytes));
            lo.int("quads", i64::from(layer.quads));
            lo.int("cpu_units", i64::from(layer.cpu_units));
            lo.int("margin", i64::from(layer.margin));
            lo.end();
        });
        push_list(o.key("effects"), &a.effects, |out, effect| {
            let mut eo = Obj::new(out);
            eo.str("layer", &effect.layer);
            eo.int("states", i64::from(effect.states));
            eo.int("shot_states", i64::from(effect.shot_states));
            eo.int("tone_slots", i64::from(effect.tone_slots));
            push_list(eo.key("reads"), &effect.reads, |out, name| {
                push_str(out, name)
            });
            eo.num("build_ops", effect.build_ops as f64);
            eo.end();
        });
        o.end();
        Ok(out)
    })
}

pub(crate) fn param_curve_of(
    p: &ParsedAsset,
    layer: &str,
    param: &str,
    variant: i64,
    count: i64,
) -> Result<Vec<f64>, Fail> {
    let asset = p.asset()?;
    let variant =
        u16::try_from(variant).map_err(|_| Fail::Args(format!("no variant {variant}")))?;
    let count =
        u32::try_from(count).map_err(|_| Fail::Args(format!("no curve of {count} frames")))?;
    caught("the core failed drawing this setting curve", || {
        param_curve(asset, layer, param, variant, count).map_err(|e| Fail::Args(e.to_string()))
    })
}

pub(crate) fn constants_json() -> String {
    let mut out = String::new();
    let mut o = Obj::new(&mut out);
    for (name, value) in constants() {
        o.int(name, *value);
    }
    let mut lo = Obj::new(o.key("limits"));
    for (name, value) in limits() {
        lo.num(name, *value);
    }
    lo.end();
    push_list(o.key("nodes"), nodes(), |out, node| {
        let mut no = Obj::new(out);
        no.str("type", node.name);
        push_list(no.key("inputs"), node.sockets, |out, socket| {
            let mut so = Obj::new(out);
            so.str("name", socket.name);
            so.str("accepts", accepts_name(socket.accepts));
            match socket.default {
                SocketDefault::Required => so.bool("required", true),
                SocketDefault::Absent => so.bool("required", false),
                SocketDefault::Value(values, width) => {
                    so.bool("required", false);
                    let shown = &values[..usize::from(width).min(values.len())];
                    push_list(so.key("default"), shown, |out, v| push_num(out, *v));
                }
                SocketDefault::Input(kind) => {
                    so.bool("required", false);
                    so.str("default_node", kind.name());
                }
            }
            so.end();
        });
        push_list(no.key("settings"), node.settings, |out, setting| {
            let mut so = Obj::new(out);
            so.str("name", setting.name);
            so.str("kind", setting.kind.as_str());
            so.bool("required", setting.required);
            so.int("min", setting.min);
            so.int("max", setting.max);
            push_list(so.key("default"), setting.default, |out, v| {
                let _ = write!(out, "{v}");
            });
            push_list(so.key("choices"), setting.choices, |out, c| {
                push_str(out, c)
            });
            so.end();
        });
        let mut co = Obj::new(no.key("cost"));
        co.str("by", node.cost_by.as_str());
        push_list(co.key("values"), &node.costs, |out, c| {
            let _ = write!(out, "{c}");
        });
        co.end();
        no.end();
    });
    push_list(o.key("anchors"), anchors(), push_str);
    push_list(o.key("signals"), signals(), push_str);
    push_list(o.key("host_queries"), &KNOWN_QUERIES, |out, q| {
        let mut qo = Obj::new(out);
        qo.str("name", q.name);
        qo.num("min", f64::from(q.min));
        qo.num("max", f64::from(q.max));
        qo.end();
    });
    push_list(o.key("capabilities"), capabilities(), |out, name| {
        push_str(out, name)
    });
    o.end();
    out
}

pub(crate) fn check_molang_json(source: &str, context: i64) -> Result<String, Fail> {
    let scope = usize::try_from(context)
        .ok()
        .and_then(|i| MolangUse::ALL.get(i).copied())
        .ok_or_else(|| Fail::Args(format!("no MoLang context {context}")))?;
    caught("the core failed checking this MoLang", || {
        let mut out = String::new();
        let mut o = Obj::new(&mut out);
        match check_molang(source, scope) {
            Ok(check) => {
                o.bool("ok", true);
                o.int("cost", i64::from(check.program.nodes()));
                push_list(o.key("warnings"), check.warnings.as_slice(), |out, w| {
                    let mut wo = Obj::new(out);
                    wo.str("kind", warning_name(w.kind));
                    wo.str("name", &w.name);
                    wo.end();
                });
                push_issues(o.key("issues"), &check.soft);
            }
            Err(issue) => {
                let message = issue
                    .names
                    .iter()
                    .find(|(key, _)| key == "message")
                    .map_or("", |(_, message)| message.as_str());
                o.bool("ok", false);
                o.str("code", issue.code.as_str());
                o.str("message", message);
            }
        }
        o.end();
        Ok(out)
    })
}

pub(crate) struct Rastered {
    pub(crate) info: [i32; RASTER_INFO_INTS],
    pub(crate) rgba: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn raster_asset(
    p: &ParsedAsset,
    row: &[i32],
    frame: u32,
    scale: u32,
    seed: u64,
    state: u32,
    state_index: u32,
    hidden: u32,
) -> Result<Rastered, Fail> {
    let asset = p.asset()?;
    if row.len() != TAG_INTS {
        return Err(Fail::Args(format!(
            "the tag row holds {} ints, not {TAG_INTS}",
            row.len()
        )));
    }
    let block =
        read_block(row).ok_or_else(|| Fail::Args("the tag row is not a valid block".into()))?;
    let variant = u16::try_from(row[TAG_VARIANT])
        .map_err(|_| Fail::Args(format!("no variant {}", row[TAG_VARIANT])))?;
    let state = match state {
        0 => RasterState::Idle,
        1 => RasterState::While(
            u8::try_from(state_index)
                .map_err(|_| Fail::Args(format!("no signal {state_index}")))?,
        ),
        2 => RasterState::OneShot(
            u16::try_from(state_index)
                .map_err(|_| Fail::Args(format!("no one-shot {state_index}")))?,
        ),
        _ => return Err(Fail::Args(format!("no raster state {state}"))),
    };
    let options = RasterOptions {
        variant,
        block,
        frame,
        scale: scale.min(u32::from(u8::MAX)) as u8,
        seed,
        state,
        hidden,
        ..RasterOptions::default()
    };
    let image = caught("the core failed rastering this bundle", || {
        Ok(raster(asset, &options))
    })?
    .ok_or_else(|| {
        Fail::Args(format!(
            "variant {variant} or the tag row does not fit this bundle"
        ))
    })?;
    Ok(Rastered {
        info: [
            image.image.width as i32,
            image.image.height as i32,
            image.origin.0,
            image.origin.1,
        ],
        rgba: image.image.rgba,
    })
}

pub(crate) struct RasterRequest<'a> {
    pub(crate) row: &'a [i32],
    pub(crate) frame: u32,
    pub(crate) scale: u32,
    pub(crate) seed: u64,
    pub(crate) reactions: u32,
    pub(crate) held_for: u32,
    pub(crate) observed_for: u32,
    pub(crate) one_shots: &'a [u32],
    pub(crate) stopped: &'a [u32],
    pub(crate) glyphs: &'a [f32],
    pub(crate) motion: u32,
    pub(crate) hidden: u32,
    pub(crate) omit: u32,
    pub(crate) band: u32,
}

pub(crate) fn raster_ex(
    p: &ParsedAsset,
    textures: Option<&RasterTextures>,
    request: &RasterRequest<'_>,
) -> Result<([i32; INFO_WORDS], Vec<u8>), Fail> {
    let asset = p.asset()?;
    let options = raster_options(asset, request)?;
    let empty = RasterTextures::new();
    let source = textures.unwrap_or(&empty);
    let image = caught("the core failed rastering this bundle", || {
        Ok(raster_with(asset, &options, source))
    })?
    .ok_or_else(|| unfit(options.variant))?;
    Ok((image.info(), image.image.rgba))
}

pub const RASTER_IMAGE_META: usize = 4;

pub struct RasterImages {
    info: [i32; INFO_WORDS],
    images: Vec<([i32; RASTER_IMAGE_META], Vec<u8>)>,
    passes: Vec<i32>,
}

pub(crate) fn raster_images(
    p: &ParsedAsset,
    textures: Option<&RasterTextures>,
    request: &RasterRequest<'_>,
    per_layer: bool,
) -> Result<RasterImages, Fail> {
    if !per_layer {
        let (info, rgba) = raster_ex(p, textures, request)?;
        let meta = [-1, -1, -1, rgba.len() as i32];
        return Ok(RasterImages {
            info,
            images: vec![(meta, rgba)],
            passes: vec![-1],
        });
    }
    let asset = p.asset()?;
    let options = raster_options(asset, request)?;
    let empty = RasterTextures::new();
    let source = textures.unwrap_or(&empty);
    let layers = caught("the core failed rastering this bundle", || {
        Ok(raster_layers_with(asset, &options, source))
    })?
    .ok_or_else(|| unfit(options.variant))?;
    let info = layers.info();
    let mut images = Vec::new();
    let mut passes = Vec::new();
    for layer in layers.layers {
        let blend = match layer.blend {
            Blend::Alpha => 0,
            Blend::Additive => 1,
        };
        let meta = |rgba: &[u8]| {
            [
                i32::from(layer.slot),
                i32::from(layer.depth.band()),
                blend,
                rgba.len() as i32,
            ]
        };
        images.push((meta(&layer.image.rgba), layer.image.rgba));
        passes.push(-1);
        for (index, pass) in layer.passes.into_iter().enumerate() {
            images.push((meta(&pass.rgba), pass.rgba));
            passes.push(index as i32);
        }
    }
    Ok(RasterImages {
        info,
        images,
        passes,
    })
}

fn unfit(variant: u16) -> Fail {
    Fail::Args(format!(
        "variant {variant} or the tag row does not fit this bundle"
    ))
}

fn raster_options(
    asset: &NameplateAsset,
    request: &RasterRequest<'_>,
) -> Result<RasterOptions, Fail> {
    let row = request.row;
    if row.len() != TAG_INTS {
        return Err(Fail::Args(format!(
            "the tag row holds {} ints, not {TAG_INTS}",
            row.len()
        )));
    }
    let block =
        read_block(row).ok_or_else(|| Fail::Args("the tag row is not a valid block".into()))?;
    let variant = u16::try_from(row[TAG_VARIANT])
        .map_err(|_| Fail::Args(format!("no variant {}", row[TAG_VARIANT])))?;
    let detail = u8::try_from(row[TAG_DETAIL])
        .ok()
        .and_then(Detail::from_value)
        .ok_or_else(|| Fail::Args(format!("no detail {}", row[TAG_DETAIL])))?;
    let motion = u8::try_from(request.motion)
        .ok()
        .and_then(Motion::from_value)
        .ok_or_else(|| Fail::Args(format!("no motion {}", request.motion)))?;
    let band = RasterBand::from_value(request.band)
        .ok_or_else(|| Fail::Args(format!("no raster band {}", request.band)))?;
    if !request.one_shots.len().is_multiple_of(2) {
        return Err(Fail::Args(format!(
            "one-shots hold {} ints, not index and age pairs",
            request.one_shots.len()
        )));
    }
    if !request.glyphs.len().is_multiple_of(GLYPH_FLOATS) {
        return Err(Fail::Args(format!(
            "glyphs hold {} floats, not {GLYPH_FLOATS} per glyph",
            request.glyphs.len()
        )));
    }
    if !request.stopped.len().is_multiple_of(3) {
        return Err(Fail::Args(format!(
            "stopped signals hold {} ints, not signal, held and ago triples",
            request.stopped.len()
        )));
    }
    let mut stopped = Vec::with_capacity(request.stopped.len() / 3);
    for triple in request.stopped.chunks_exact(3) {
        let signal = u8::try_from(triple[0])
            .ok()
            .filter(|s| *s < 32 && usize::from(*s) < SIGNAL_COUNT)
            .ok_or_else(|| Fail::Args(format!("no signal {}", triple[0])))?;
        stopped.push(RasterStop {
            signal,
            held: triple[1],
            ago: triple[2],
        });
    }
    let mut one_shots = Vec::with_capacity(request.one_shots.len() / 2);
    for pair in request.one_shots.chunks_exact(2) {
        let index = u16::try_from(pair[0])
            .ok()
            .filter(|i| usize::from(*i) < asset.animations.one_shots.len())
            .ok_or_else(|| Fail::Args(format!("no one-shot {}", pair[0])))?;
        one_shots.push(RasterShot {
            index,
            age: pair[1],
        });
    }
    let flags = row[TAG_FLAGS];
    let options = RasterOptions {
        variant,
        block,
        frame: request.frame,
        scale: request.scale.min(u32::from(u8::MAX)) as u8,
        seed: request.seed,
        state: RasterState::Idle,
        hidden: request.hidden,
        omit: request.omit,
        band,
        reactions: request.reactions,
        held_for: (request.held_for != u32::MAX).then_some(request.held_for),
        observed_for: (request.observed_for != u32::MAX).then_some(request.observed_for),
        stopped,
        one_shots,
        glyphs: request.glyphs.to_vec(),
        detail,
        motion,
        effects_off: flags & FLAG_EFFECTS_OFF != 0,
        animated_names_off: flags & FLAG_ANIMATED_NAMES_OFF != 0,
        custom: Vec::new(),
    };
    Ok(options)
}

pub(crate) fn set_raster_texture(
    t: &mut RasterTextures,
    id: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Fail> {
    if t.set(id, width, height, rgba.to_vec()) {
        Ok(())
    } else {
        Err(Fail::Args(format!(
            "rgba holds {} bytes, a {width}x{height} texture needs {}",
            rgba.len(),
            u64::from(width) * u64::from(height) * 4
        )))
    }
}

pub(crate) fn add_asset(n: &mut Nameplates, p: &ParsedAsset) -> Result<u32, Fail> {
    let asset = p.asset()?.clone();
    let id = caught("the core failed adding this asset", || {
        Ok(n.renderer.add_asset(asset))
    })?;
    if id == 0 {
        return Err(Fail::Args("no room for another nameplate asset".into()));
    }
    Ok(id)
}

pub(crate) fn needed_words(n: &Nameplates, tags: &[i32]) -> Result<usize, Fail> {
    if !tags.len().is_multiple_of(TAG_INTS) {
        return Err(Fail::Args(format!(
            "nameplate: {} tag ints is not a whole number of rows",
            tags.len()
        )));
    }
    let renderer = &n.renderer;
    caught("the core failed sizing this frame", || {
        Ok(renderer.needed_words(tags))
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn frame(
    n: &mut Nameplates,
    clock: f64,
    wearers: &[u64],
    tags: &[i32],
    signals: &[f32],
    glyphs: &[f32],
    colors: &[i32],
    out: &mut [i32],
) -> Result<usize, Fail> {
    let fault = std::mem::take(&mut n.fault);
    let renderer = &mut n.renderer;
    caught("the core failed on this frame", || {
        if fault {
            panic!("nameplate fault");
        }
        renderer
            .frame(clock, wearers, tags, signals, glyphs, colors, out)
            .map_err(|e| Fail::Args(e.to_string()))
    })
}

pub(crate) fn set_motion(n: &mut Nameplates, motion: i64) -> Result<(), Fail> {
    let motion = u8::try_from(motion)
        .ok()
        .and_then(Motion::from_value)
        .ok_or_else(|| Fail::Args(format!("no motion {motion}")))?;
    n.renderer.set_motion(motion);
    Ok(())
}

pub(crate) fn set_vanilla_texture(
    n: &mut Nameplates,
    id: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Fail> {
    if n.renderer
        .set_vanilla_texture(id, width, height, rgba.to_vec())
    {
        Ok(())
    } else {
        Err(Fail::Args(format!(
            "rgba holds {} bytes, a {width}x{height} texture needs {}",
            rgba.len(),
            u64::from(width) * u64::from(height) * 4
        )))
    }
}

pub(crate) fn set_signal_override(
    n: &mut Nameplates,
    wearer: u64,
    signal: i64,
    value: f32,
) -> Result<(), Fail> {
    let known =
        usize::try_from(signal).is_ok_and(|s| n.renderer.set_signal_override(wearer, s, value));
    if known {
        Ok(())
    } else {
        Err(Fail::Args(format!("no signal {signal}")))
    }
}

pub(crate) fn set_custom_colors(
    n: &mut Nameplates,
    wearer: u64,
    asset: i64,
    slots: &[i32],
    rgb: &[i32],
) -> Result<(), Fail> {
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
    let known =
        u32::try_from(asset).is_ok_and(|a| n.renderer.set_custom_colors(wearer, a, &custom));
    if known {
        Ok(())
    } else {
        Err(Fail::Args(format!("no nameplate asset {asset}")))
    }
}

pub(crate) fn fire(
    n: &mut Nameplates,
    wearer: u64,
    asset: i64,
    one_shot: i64,
) -> Result<bool, Fail> {
    let (Ok(asset), Ok(one_shot)) = (u32::try_from(asset), u16::try_from(one_shot)) else {
        return Ok(false);
    };
    let renderer = &mut n.renderer;
    caught("the core failed firing this one-shot", || {
        Ok(renderer.fire(wearer, asset, one_shot))
    })
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_parse(
    ptr: *const u8,
    len: u32,
    mode: u32,
    caps_ptr: *const u8,
    caps_len: u32,
    identity: u64,
) -> *const ParsedAsset {
    let parsed = (|| {
        let capabilities = if caps_ptr.is_null() && caps_len == 0 {
            None
        } else {
            Some(text_of(caps_ptr, caps_len, "caps_ptr", "capabilities are")?)
        };
        let mode = mode_of(i64::from(mode))?;
        parse_asset(view(ptr, len, "ptr")?, mode, capabilities, identity)
    })();
    match parsed {
        Ok(parsed) => Arc::into_raw(Arc::new(parsed)),
        Err(fail) => {
            free_fail(&fail);
            std::ptr::null()
        }
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_asset_free(h: *const ParsedAsset) {
    if !h.is_null() {
        drop(Arc::from_raw(h));
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_asset_status(h: *const ParsedAsset) -> i32 {
    (&*h).status
}

#[no_mangle]
pub extern "C" fn nameplate_error() -> *const u8 {
    ERROR.with(|e| e.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn nameplate_error_len() -> u32 {
    ERROR.with(|e| e.borrow().len() as u32)
}

#[no_mangle]
pub extern "C" fn nameplate_text() -> *const u8 {
    TEXT.with(|t| t.borrow().as_ptr())
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_asset_issues_json(h: *const ParsedAsset) -> i32 {
    text_out(Ok(issues_json(&*h)))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_shaders_json(h: *const ParsedAsset, dialect: u32) -> i32 {
    text_out(dialect_of(i64::from(dialect)).and_then(|d| shaders_json(&*h, d)))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_dialect_issues_json(h: *const ParsedAsset, dialect: u32) -> i32 {
    text_out(dialect_of(i64::from(dialect)).and_then(|d| dialect_issues_json(&*h, d)))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_analyze_json(h: *const ParsedAsset) -> i32 {
    text_out(analyze_json(&*h))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_meta_json(h: *const ParsedAsset) -> i32 {
    text_out(meta_json(&*h))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_texture_count(h: *const ParsedAsset) -> i32 {
    match texture_count(&*h) {
        Ok(count) => count as i32,
        Err(fail) => free_fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_texture(
    h: *const ParsedAsset,
    index: u32,
    info: *mut i32,
    rgba_out: *mut u8,
    rgba_len: u32,
) -> i32 {
    let image = match texture(&*h, i64::from(index)) {
        Ok(image) => image,
        Err(fail) => return free_fail(&fail),
    };
    if let Some(info) = info_out(info, 2) {
        info.copy_from_slice(&[image.width as i32, image.height as i32]);
    }
    let bytes = image.rgba.len();
    if rgba_out.is_null() {
        return bytes as i32;
    }
    if (rgba_len as usize) < bytes {
        return free_fail(&Fail::Args(format!(
            "rgba_out holds {rgba_len} bytes, the texture needs {bytes}"
        )));
    }
    rgba_out.copy_from_nonoverlapping(image.rgba.as_ptr(), bytes);
    bytes as i32
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_param_curve(
    h: *const ParsedAsset,
    layer: *const u8,
    layer_len: u32,
    param: *const u8,
    param_len: u32,
    variant: u32,
    count: u32,
    out: *mut f64,
    out_len: u32,
) -> i32 {
    let curve = (|| {
        let layer = text_of(layer, layer_len, "layer", "the layer id is")?;
        let param = text_of(param, param_len, "param", "the setting name is")?;
        let values = param_curve_of(&*h, layer, param, i64::from(variant), i64::from(count))?;
        if (out_len as usize) < values.len() {
            return Err(Fail::Args(format!(
                "out holds {out_len} values, the curve needs {}",
                values.len()
            )));
        }
        let out = view_mut(out, values.len() as u32, "out")?;
        out.copy_from_slice(&values);
        Ok(values.len())
    })();
    match curve {
        Ok(len) => len as i32,
        Err(fail) => free_fail(&fail),
    }
}

#[no_mangle]
pub extern "C" fn nameplate_constants_json() -> i32 {
    text_out(Ok(constants_json()))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_check_molang_json(
    ptr: *const u8,
    len: u32,
    context: u32,
) -> i32 {
    match text_of(ptr, len, "ptr", "MoLang is") {
        Ok(source) => text_out(check_molang_json(source, i64::from(context))),
        Err(fail) => free_fail(&fail),
    }
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster(
    h: *const ParsedAsset,
    tag_row: *const i32,
    tag_len: u32,
    frame: u32,
    scale: u32,
    seed: u64,
    state: u32,
    state_index: u32,
    hidden: u32,
    info: *mut i32,
    rgba_out: *mut u8,
    rgba_len: u32,
) -> i32 {
    let rastered = view(tag_row, tag_len, "tag_row")
        .and_then(|row| raster_asset(&*h, row, frame, scale, seed, state, state_index, hidden));
    let rastered = match rastered {
        Ok(rastered) => rastered,
        Err(fail) => return free_fail(&fail),
    };
    if let Some(info) = info_out(info, RASTER_INFO_INTS) {
        info.copy_from_slice(&rastered.info);
    }
    let bytes = rastered.rgba.len();
    if rgba_out.is_null() {
        return bytes as i32;
    }
    if (rgba_len as usize) < bytes {
        return free_fail(&Fail::Args(format!(
            "rgba_out holds {rgba_len} bytes, the image needs {bytes}"
        )));
    }
    rgba_out.copy_from_nonoverlapping(rastered.rgba.as_ptr(), bytes);
    bytes as i32
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_ex(
    h: *const ParsedAsset,
    textures: *const RasterTextures,
    tag_row: *const i32,
    tag_len: u32,
    frame: u32,
    scale: u32,
    seed: u64,
    reactions: u32,
    held_for: u32,
    observed_for: u32,
    one_shots: *const u32,
    one_shot_len: u32,
    stopped: *const u32,
    stopped_len: u32,
    glyphs: *const f32,
    glyph_len: u32,
    motion: u32,
    hidden: u32,
    omit: u32,
    band: u32,
    info: *mut i32,
    info_len: u32,
    rgba_out: *mut u8,
    rgba_len: u32,
) -> i32 {
    let textures = if textures.is_null() {
        None
    } else {
        Some(&*textures)
    };
    let rastered = (|| {
        let request = RasterRequest {
            row: view(tag_row, tag_len, "tag_row")?,
            frame,
            scale,
            seed,
            reactions,
            held_for,
            observed_for,
            one_shots: view(one_shots, one_shot_len, "one_shots")?,
            stopped: view(stopped, stopped_len, "stopped")?,
            glyphs: view(glyphs, glyph_len, "glyphs")?,
            motion,
            hidden,
            omit,
            band,
        };
        let info = view_mut(info, info_len, "info")?;
        Ok((info, raster_ex(&*h, textures, &request)?))
    })();
    let (info, (words, rgba)) = match rastered {
        Ok(rastered) => rastered,
        Err(fail) => return free_fail(&fail),
    };
    let shared = info.len().min(INFO_WORDS);
    info[..shared].copy_from_slice(&words[..shared]);
    let bytes = rgba.len();
    if rgba_out.is_null() {
        return bytes as i32;
    }
    if (rgba_len as usize) < bytes {
        return free_fail(&Fail::Args(format!(
            "rgba_out holds {rgba_len} bytes, the image needs {bytes}"
        )));
    }
    rgba_out.copy_from_nonoverlapping(rgba.as_ptr(), bytes);
    bytes as i32
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_images(
    h: *const ParsedAsset,
    textures: *const RasterTextures,
    tag_row: *const i32,
    tag_len: u32,
    frame: u32,
    scale: u32,
    seed: u64,
    reactions: u32,
    held_for: u32,
    observed_for: u32,
    one_shots: *const u32,
    one_shot_len: u32,
    stopped: *const u32,
    stopped_len: u32,
    glyphs: *const f32,
    glyph_len: u32,
    motion: u32,
    hidden: u32,
    omit: u32,
    band: u32,
    per_layer: u32,
) -> *mut RasterImages {
    let textures = if textures.is_null() {
        None
    } else {
        Some(&*textures)
    };
    let rastered = (|| {
        let request = RasterRequest {
            row: view(tag_row, tag_len, "tag_row")?,
            frame,
            scale,
            seed,
            reactions,
            held_for,
            observed_for,
            one_shots: view(one_shots, one_shot_len, "one_shots")?,
            stopped: view(stopped, stopped_len, "stopped")?,
            glyphs: view(glyphs, glyph_len, "glyphs")?,
            motion,
            hidden,
            omit,
            band,
        };
        raster_images(&*h, textures, &request, per_layer != 0)
    })();
    match rastered {
        Ok(images) => Box::into_raw(Box::new(images)),
        Err(fail) => {
            free_fail(&fail);
            std::ptr::null_mut()
        }
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_images_info(r: *const RasterImages) -> *const i32 {
    if r.is_null() {
        return std::ptr::null();
    }
    (&*r).info.as_ptr()
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_images_count(r: *const RasterImages) -> u32 {
    if r.is_null() {
        return 0;
    }
    (&*r).images.len() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_image_meta(
    r: *const RasterImages,
    index: u32,
) -> *const i32 {
    if r.is_null() {
        return std::ptr::null();
    }
    (&*r)
        .images
        .get(index as usize)
        .map_or(std::ptr::null(), |(meta, _)| meta.as_ptr())
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_image_pass(r: *const RasterImages, index: u32) -> i32 {
    if r.is_null() {
        return -1;
    }
    (&*r).passes.get(index as usize).copied().unwrap_or(-1)
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_image_rgba(
    r: *const RasterImages,
    index: u32,
) -> *const u8 {
    if r.is_null() {
        return std::ptr::null();
    }
    (&*r)
        .images
        .get(index as usize)
        .map_or(std::ptr::null(), |(_, rgba)| rgba.as_ptr())
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_raster_images_free(r: *mut RasterImages) {
    if !r.is_null() {
        drop(Box::from_raw(r));
    }
}

#[no_mangle]
pub extern "C" fn nameplate_textures_new() -> *mut RasterTextures {
    Box::into_raw(Box::new(RasterTextures::new()))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_textures_free(t: *mut RasterTextures) {
    if !t.is_null() {
        drop(Box::from_raw(t));
    }
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_textures_set(
    t: *mut RasterTextures,
    id_ptr: *const u8,
    id_len: u32,
    width: u32,
    height: u32,
    rgba: *const u8,
    rgba_len: u32,
) -> i32 {
    let set = (|| {
        let id = text_of(id_ptr, id_len, "id_ptr", "texture id is")?;
        set_raster_texture(&mut *t, id, width, height, view(rgba, rgba_len, "rgba")?)
    })();
    match set {
        Ok(()) => 0,
        Err(fail) => free_fail(&fail),
    }
}

#[no_mangle]
pub extern "C" fn nameplates_new() -> *mut Nameplates {
    Box::into_raw(Box::new(Nameplates::new()))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_free(n: *mut Nameplates) {
    if !n.is_null() {
        drop(Box::from_raw(n));
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_error(n: *const Nameplates) -> *const u8 {
    (&*n).error.as_ptr()
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_error_len(n: *const Nameplates) -> u32 {
    (&*n).error.len() as u32
}

#[no_mangle]
pub extern "C" fn nameplates_tag_ints() -> u32 {
    TAG_INTS as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_add_asset(n: *mut Nameplates, h: *const ParsedAsset) -> i32 {
    let n = &mut *n;
    if h.is_null() {
        return n.fail(&Fail::Args("the asset handle is null".into()));
    }
    match add_asset(n, &*h) {
        Ok(id) => id as i32,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_remove_asset(n: *mut Nameplates, id: u32) {
    (&mut *n).renderer.remove_asset(id);
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_set_motion(n: *mut Nameplates, motion: u32) -> i32 {
    let n = &mut *n;
    match set_motion(n, i64::from(motion)) {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_set_vanilla_texture(
    n: *mut Nameplates,
    id_ptr: *const u8,
    id_len: u32,
    width: u32,
    height: u32,
    rgba: *const u8,
    rgba_len: u32,
) -> i32 {
    let n = &mut *n;
    let set = (|| {
        let id = text_of(id_ptr, id_len, "id_ptr", "texture id is")?;
        set_vanilla_texture(n, id, width, height, view(rgba, rgba_len, "rgba")?)
    })();
    match set {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_needed_words(
    n: *mut Nameplates,
    tags: *const i32,
    tags_len: u32,
) -> i32 {
    let n = &mut *n;
    match view(tags, tags_len, "tags").and_then(|tags| needed_words(n, tags)) {
        Ok(words) => words as i32,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_frame(
    n: *mut Nameplates,
    clock: f64,
    wearers: *const u64,
    wearer_count: u32,
    tags: *const i32,
    tags_len: u32,
    signals: *const f32,
    signals_len: u32,
    glyphs: *const f32,
    glyphs_len: u32,
    colors: *const i32,
    colors_len: u32,
    out: *mut i32,
    out_len: u32,
) -> i32 {
    let n = &mut *n;
    let framed = (|| {
        frame(
            n,
            clock,
            view(wearers, wearer_count, "wearers")?,
            view(tags, tags_len, "tags")?,
            view(signals, signals_len, "signals")?,
            view(glyphs, glyphs_len, "glyphs")?,
            view(colors, colors_len, "colors")?,
            view_mut(out, out_len, "out")?,
        )
    })();
    match framed {
        Ok(words) => words as i32,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_forget_wearer(n: *mut Nameplates, wearer: u64) -> u32 {
    (&mut *n).renderer.forget_wearer(wearer) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_set_signal_override(
    n: *mut Nameplates,
    wearer: u64,
    signal: u32,
    value: f32,
) -> i32 {
    let n = &mut *n;
    match set_signal_override(n, wearer, i64::from(signal), value) {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_set_custom_colors(
    n: *mut Nameplates,
    wearer: u64,
    asset: u32,
    slots: *const u32,
    rgb: *const u32,
    count: u32,
) -> i32 {
    let n = &mut *n;
    let set = (|| {
        let slots: &[i32] = view(slots.cast(), count, "slots")?;
        let rgb: &[i32] = view(rgb.cast(), count, "rgb")?;
        set_custom_colors(n, wearer, i64::from(asset), slots, rgb)
    })();
    match set {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_clear_custom_colors(
    n: *mut Nameplates,
    wearer: u64,
    asset: u32,
) -> u32 {
    (&mut *n).renderer.clear_custom_colors(wearer, asset) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_fire(
    n: *mut Nameplates,
    wearer: u64,
    asset: u32,
    one_shot: u32,
) -> i32 {
    let n = &mut *n;
    match fire(n, wearer, i64::from(asset), i64::from(one_shot)) {
        Ok(fired) => fired as i32,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_strip_layout(n: *const Nameplates) -> u32 {
    (&*n).renderer.strip_layout()
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_strip_texels(
    n: *const Nameplates,
    id: u32,
    len: *mut u32,
) -> *const u32 {
    let texels = (&*n).renderer.strip_texels(id);
    if !len.is_null() {
        *len = texels.len() as u32;
    }
    texels.as_ptr()
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_wearer_count(n: *const Nameplates) -> u32 {
    (&*n).renderer.wearer_count() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_wearer_bytes(n: *const Nameplates) -> u32 {
    (&*n).renderer.wearer_bytes() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_pending_fallbacks(n: *const Nameplates) -> u32 {
    (&*n).renderer.pending_fallbacks() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_run_fallbacks(n: *mut Nameplates, limit: u32) -> i32 {
    let n = &mut *n;
    match run_fallbacks(n, i64::from(limit)) {
        Ok(left) => left as i32,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_take_fallback(n: *mut Nameplates) -> *mut Fallback {
    match (&mut *n).renderer.take_fallback() {
        Some(job) => Box::into_raw(Box::new(Fallback::Job(Box::new(job)))),
        None => std::ptr::null_mut(),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_fallback_run(f: *mut Fallback) -> u32 {
    if f.is_null() {
        return 0;
    }
    (&mut *f).run() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplates_finish_fallback(n: *mut Nameplates, f: *mut Fallback) -> i32 {
    if f.is_null() {
        return 0;
    }
    let fallback = *Box::from_raw(f);
    let n = &mut *n;
    match finish_fallback(n, fallback) {
        Ok(stored) => stored as i32,
        Err(fail) => n.fail(&fail),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn nameplate_fallback_free(f: *mut Fallback) {
    if !f.is_null() {
        drop(Box::from_raw(f));
    }
}

#[cfg(test)]
mod tests {
    use plusrenderer::nameplate::abi::{
        write_block, DIR_OFFSET, DIR_STATUS, DIR_WORDS, FLAG_EFFECTS_CPU, HEAD_OPS, OP_AUX0,
        OP_BAND_LAYER_KIND, OP_KIND_IMAGE, OP_KIND_RECT, OP_WORDS, OUT_HEADER, OUT_TAGS, OUT_WORDS,
        SIGNAL_FLOATS, STATUS_OK, STATUS_UNKNOWN_ASSET, TAG_ASSET, TAG_HEAD, TAG_SNEAK_ALPHA,
    };
    use plusrenderer::nameplate::geom::{BlockInput, Rect};
    use std::ptr::{null, null_mut};

    use super::*;
    use crate::tests::{bundle, tree, zip};

    fn kit(name: &str) -> Vec<u8> {
        bundle(&tree(&format!("src/nameplate/format/testdata/{name}")))
    }

    const INK: &[u8] = br##"{"slots":[{"id":"ink","ramp":["#FFFFFF"]}]}"##;

    fn one_material(id: &str, extra: &str, material: &str) -> Vec<u8> {
        let manifest = format!(
            r#"{{"format":1,"kind":"nameplate","id":"{id}","fps":6,"loop":12,"text":{{"color":"ink"}},"layers":[{{"id":"{id}","depth":"behind"}}]{extra}}}"#
        );
        let layer = format!(r#"{{"kind":"material","material":"{id}"}}"#);
        let (material_at, layer_at) = (format!("materials/{id}.json"), format!("layers/{id}.json"));
        zip(&[
            ("manifest.json", manifest.as_bytes()),
            ("palettes.json", INK),
            (material_at.as_str(), material.as_bytes()),
            (layer_at.as_str(), layer.as_bytes()),
        ])
    }

    fn parse(bytes: &[u8], mode: u32) -> *const ParsedAsset {
        unsafe { nameplate_parse(bytes.as_ptr(), bytes.len() as u32, mode, null(), 0, 0) }
    }

    fn row(asset: i32, width: i32) -> [i32; TAG_INTS] {
        let mut row = [0; TAG_INTS];
        row[TAG_ASSET] = asset;
        row[TAG_SNEAK_ALPHA] = 255;
        let input = BlockInput {
            line_widths: [width, 0, 0],
            line_count: 1,
            name_width: width,
            background: Rect::vanilla_background(width, 8),
            ..BlockInput::default()
        };
        assert!(write_block(&input, &mut row));
        row
    }

    fn text() -> String {
        TEXT.with(|t| {
            let t = t.borrow();
            assert_eq!(nameplate_text(), t.as_ptr());
            t.clone()
        })
    }

    fn error() -> String {
        let len = nameplate_error_len() as usize;
        let bytes = unsafe { std::slice::from_raw_parts(nameplate_error(), len) };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn handle_error(n: *const Nameplates) -> String {
        unsafe {
            let len = nameplates_error_len(n) as usize;
            let bytes = std::slice::from_raw_parts(nameplates_error(n), len);
            String::from_utf8(bytes.to_vec()).unwrap()
        }
    }

    unsafe fn plates(name: &str) -> (*mut Nameplates, i32) {
        let h = parse(&kit(name), 0);
        let n = nameplates_new();
        let id = nameplates_add_asset(n, h);
        assert_eq!(id, 1, "{}", handle_error(n));
        nameplate_asset_free(h);
        (n, id)
    }

    #[rustfmt::skip]
    unsafe fn draw(n: *mut Nameplates, clock: f64, wearers: &[u64], tags: &[i32], out: &mut [i32]) -> i32 {
        let signals = vec![0.0f32; tags.len() / TAG_INTS * SIGNAL_FLOATS];
        let (count, tag_len, signal_len) = (wearers.len() as u32, tags.len() as u32, signals.len() as u32);
        let (wearers, tags, signals) = (wearers.as_ptr(), tags.as_ptr(), signals.as_ptr());
        let (out_ptr, out_len) = (out.as_mut_ptr(), out.len() as u32);
        nameplates_frame(
            n, clock, wearers, count, tags, tag_len, signals, signal_len, null(), 0, null(), 0, out_ptr, out_len,
        )
    }

    fn ops(out: &[i32], tag: usize, kind: i32) -> Vec<&[i32]> {
        let at = out[OUT_HEADER + tag * DIR_WORDS + DIR_OFFSET] as usize;
        (0..out[at + HEAD_OPS] as usize)
            .map(|k| &out[at + TAG_HEAD + k * OP_WORDS..][..OP_WORDS])
            .filter(|op| (op[OP_BAND_LAYER_KIND] >> 16) & 0xff == kind)
            .collect()
    }

    #[test]
    fn a_kit_bundle_parses_and_frames_three_tags() {
        unsafe {
            let (n, id) = plates("test_plate");
            let tags: Vec<i32> = [row(id, 40), row(id, 60), row(id, 24)].concat();
            let needed = nameplates_needed_words(n, tags.as_ptr(), tags.len() as u32);
            assert!(needed > (OUT_HEADER + 3 * DIR_WORDS) as i32);
            let mut out = vec![0; needed as usize];
            let words = draw(n, 10.0, &[1, 2, 3], &tags, &mut out);
            assert!(words > 0 && words <= needed, "{words}: {}", handle_error(n));
            assert_eq!((out[OUT_TAGS], out[OUT_WORDS]), (3, words));
            for tag in 0..3 {
                assert_eq!(out[OUT_HEADER + tag * DIR_WORDS + DIR_STATUS], STATUS_OK);
            }
            assert!(!ops(&out, 0, OP_KIND_RECT).is_empty());
            assert!(nameplates_wearer_bytes(n) > 0 && nameplates_tag_ints() == 24);
            assert_eq!(
                (nameplates_forget_wearer(n, 1), nameplates_wearer_count(n)),
                (1, 2)
            );
            nameplates_remove_asset(n, id as u32);
            assert!(draw(n, 10.1, &[1, 2, 3], &tags, &mut out) > 0);
            assert_eq!(out[OUT_HEADER + DIR_STATUS], STATUS_UNKNOWN_ASSET);
            nameplates_free(n);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn bad_arguments_fail_with_a_message_and_keep_the_renderer() {
        unsafe {
            let bundle = kit("test_plate");
            let h = parse(&bundle, 0);
            let (n, id) = plates("test_plate");
            let tags = row(id, 40);
            let needed = nameplates_needed_words(n, tags.as_ptr(), TAG_INTS as u32);
            let mut out = vec![0; needed as usize];
            let signals = [0.0f32; SIGNAL_FLOATS];
            let framed = move |wearers: *const u64, signals: *const f32, out: *mut i32, len: u32| {
                let (tags, ints, floats) = (tags.as_ptr(), TAG_INTS as u32, SIGNAL_FLOATS as u32);
                nameplates_frame(n, 1.0, wearers, 1, tags, ints, signals, floats, null(), 0, null(), 0, out, len)
            };
            let e = |code: i32| (code, error());
            let he = |code: i32| (code, handle_error(n));
            let nul = |h: *const ParsedAsset| e(-i32::from(h.is_null()));
            let mut info = [0i32; RASTER_INFO_INTS];
            let info_ptr = info.as_mut_ptr();
            let textures = nameplate_textures_new();
            let (rgba, glint, ok) = ([0u8; 16], "glint".as_ptr(), signals.as_ptr());
            let out_ptr = out.as_mut_ptr();
            let rows: Vec<((i32, String), &str)> = vec![
                (nul(nameplate_parse(null(), 4, 0, null(), 0, 0)), "ptr is null with a length of 4"),
                (nul(nameplate_parse(bundle.as_ptr(), 9, 2, null(), 0, 0)), "no mode 2"),
                (e(nameplate_textures_set(textures, glint, 5, 1, 1, null(), 4)), "rgba is null"),
                (e(nameplate_raster(h, null(), 24, 0, 1, 0, 0, 0, 0, info_ptr, null_mut(), 0)), "tag_row is null"),
                (he(nameplates_add_asset(n, null())), "null"),
                (he(framed(null(), ok, out_ptr, 192)), "wearers is null"),
                (he(framed(&7, ok, out_ptr, 4)), "needs at least"),
                (he(draw(n, 1.0, &[1, 2], &tags, &mut out)), "1 tags but 2 wearers"),
                (he(nameplates_set_custom_colors(n, 7, id as u32, &256, &1, 1)), "no palette slot 256"),
                (he(nameplates_set_motion(n, 3)), "no motion 3"),
            ];
            for ((code, message), expected) in rows {
                assert_eq!(code, -1, "{expected}");
                assert!(message.contains(expected), "{message} for {expected}");
            }
            assert_eq!(nameplates_set_vanilla_texture(n, glint, 5, 2, 2, rgba.as_ptr(), 16), 0);
            assert_eq!(nameplate_textures_set(textures, glint, 5, 2, 2, rgba.as_ptr(), 16), 0);
            (*n).fault = true;
            assert_eq!(framed(&7, ok, out_ptr, needed as u32), -2);
            assert!(handle_error(n).contains("core failed"));
            assert!(framed(&7, ok, out_ptr, needed as u32) > 0, "a panic leaves the renderer usable");
            assert_eq!(out[OUT_HEADER + DIR_STATUS], STATUS_OK);

            let invalid = parse(b"not a zip", 1);
            assert_eq!(nameplate_asset_status(invalid), STATUS_INVALID);
            assert!(nameplate_asset_issues_json(invalid) > 0 && text().starts_with("{\"status\":\"invalid\",\"skip\":null,\"issues\":[{\"code\":"));
            assert_eq!(nameplates_add_asset(n, invalid), -1);
            assert_eq!((nameplate_analyze_json(invalid), nameplate_texture_count(invalid)), (-1, -1));
            nameplate_textures_free(textures);
            nameplate_asset_free(invalid);
            nameplate_asset_free(h);
            nameplates_free(n);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn custom_colours_and_one_shots_cross_the_abi_per_wearer() {
        unsafe {
            let (n, id) = plates("reaction_plate");
            let (id, slots, rgb) = (id as u32, [1u32, 2], [0xff3f_a34du32, 0x00e0_a020]);
            assert_eq!(nameplates_set_custom_colors(n, 5, id, slots.as_ptr(), rgb.as_ptr(), 2), 0, "{}", handle_error(n));
            assert_eq!((*n).renderer.custom_colors(5, id), Some(&[(1, 0x3f_a3_4d), (2, 0xe0_a0_20)][..]));
            assert_eq!((*n).renderer.custom_colors(6, id), None, "colours belong to one wearer");
            assert_eq!((nameplates_clear_custom_colors(n, 5, id), nameplates_clear_custom_colors(n, 5, id)), (1, 0));
            assert_eq!((*n).renderer.custom_colors(5, id), None);
            let h = parse(&kit("reaction_plate"), 0);
            let shots = (*h).asset.as_ref().unwrap().animations.one_shots.len() as u32;
            nameplate_asset_free(h);
            assert!(shots > 0);
            let tags = row(id as i32, 40);
            let mut out = vec![0; nameplates_needed_words(n, tags.as_ptr(), TAG_INTS as u32) as usize];
            assert!(draw(n, 10.0, &[5], &tags, &mut out) > 0);
            let still = out.clone();
            assert_eq!((nameplates_fire(n, 5, id, 0), nameplates_fire(n, 5, id, shots)), (1, 0));
            assert!(draw(n, 10.25, &[5], &tags, &mut out) > 0);
            assert_ne!(out, still, "the fired one-shot plays");
            nameplates_free(n);
        }
    }

    #[test]
    fn cpu_effect_fallbacks_cross_the_abi_and_run_on_any_thread() {
        unsafe {
            let (n, id) = plates("material_plate");
            let mut tag = row(id, 40);
            tag[TAG_FLAGS] |= FLAG_EFFECTS_CPU;
            let mut out =
                vec![0; nameplates_needed_words(n, tag.as_ptr(), TAG_INTS as u32) as usize];
            assert!(draw(n, 60.0, &[1], &tag, &mut out) > 0);
            assert_eq!(ops(&out, 0, OP_KIND_IMAGE).len(), 0);
            assert_eq!(nameplates_pending_fallbacks(n), 3);
            let job = nameplates_take_fallback(n) as usize;
            let ran = std::thread::spawn(move || nameplate_fallback_run(job as *mut Fallback));
            assert_eq!(ran.join().unwrap(), 1);
            assert_eq!(nameplate_fallback_run(job as *mut Fallback), 0);
            assert_eq!(nameplates_finish_fallback(n, job as *mut Fallback), 1);
            let unrun = nameplates_take_fallback(n);
            assert_eq!(nameplates_finish_fallback(n, unrun), 1);
            nameplate_fallback_free(nameplates_take_fallback(n));
            assert!(nameplates_take_fallback(n).is_null());
            assert_eq!(nameplates_run_fallbacks(n, 8), 0);
            assert!(draw(n, 60.0, &[1], &tag, &mut out) > 0);
            let images = ops(&out, 0, OP_KIND_IMAGE);
            assert_eq!(images.len(), 2);
            for op in images {
                let mut len = u32::MAX;
                let texels = nameplates_strip_texels(n, op[OP_AUX0] as u32, &mut len);
                assert!(len > 0 && !texels.is_null());
            }
            nameplates_free(n);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn json_queries_cross_the_abi() {
        unsafe {
            let fuzz = r#"{"resolution":"pixel","nodes":[{"id":"p","type":"PixelPos"},{"id":"n","type":"Noise","in":{"p":"p"}},{"id":"h","type":"Hsv","in":{"h":"n"}},{"id":"out","type":"Output","in":{"color":"h"}}]}"#;
            let h = parse(&one_material("fuzz", "", fuzz), 1);
            let len = nameplate_dialect_issues_json(h, Dialect::Sksl.value() as u32);
            let json = text();
            assert_eq!(json.len(), len as usize);
            assert!(json.starts_with(r#"{"dialect":"sksl","issues":[{"code":"dialect_unsupported""#), "{json}");
            assert!(json.contains(r#""path":"layers/fuzz.json#material""#), "{json}");
            nameplate_asset_free(h);

            let h = parse(&kit("material_plate"), 1);
            for dialect in Dialect::ALL {
                let len = nameplate_shaders_json(h, dialect.value() as u32);
                let json = text();
                assert_eq!(json.len(), len as usize);
                assert!(json.starts_with(&format!(r#"{{"dialect":"{}","shaders":[{{"key":"#, dialect.as_str())), "{json}");
            }
            assert!(nameplate_analyze_json(h) > 0);
            assert!(text().contains(r#""effects":[{"layer":"halo","states":"#), "{}", text());
            assert!(nameplate_meta_json(h) > 0);
            let meta = text();
            assert!(meta.starts_with(r#"{"id":"material_plate","identity":""#), "{meta}");
            for key in [r#""variants":[{"key":"#, r#""layers":[{"slot":0,"#, r#""animations":{"reactions":["#, r#""textures":[{"width":"#] {
                assert!(meta.contains(key), "{key} in {meta}");
            }
            let check = |src: &str| (nameplate_check_molang_json(src.as_ptr(), src.len() as u32, 0) > 0).then(text).unwrap();
            assert!(check("q.name_width * 2").starts_with(r#"{"ok":true,"cost":"#));
            assert!(check("q.x").starts_with(r#"{"ok":false,"code":"molang_query_place""#));

            let (tags, mut info) = (row(1, 40), [0i32; RASTER_INFO_INTS]);
            let raster = |out: &mut [u8], info: &mut [i32]| {
                let ptr = if out.is_empty() { null_mut() } else { out.as_mut_ptr() };
                nameplate_raster(h, tags.as_ptr(), TAG_INTS as u32, 0, 1, 0, 0, 0, 0, info.as_mut_ptr(), ptr, out.len() as u32)
            };
            let bytes = raster(&mut [], &mut info);
            assert_eq!(bytes, info[0] * info[1] * 4);
            let mut rgba = vec![0u8; bytes as usize];
            assert_eq!(raster(&mut rgba[1..], &mut info), -1);
            assert!(error().starts_with("rgba_out holds"));
            assert_eq!(raster(&mut rgba, &mut info), bytes);

            let (count, mut size) = (nameplate_texture_count(h), [0i32; 2]);
            let bytes = nameplate_texture(h, 0, size.as_mut_ptr(), null_mut(), 0);
            assert_eq!(bytes, size[0] * size[1] * 4);
            let mut rgba = vec![0u8; bytes as usize];
            assert_eq!(nameplate_texture(h, 0, size.as_mut_ptr(), rgba.as_mut_ptr(), bytes as u32 - 1), -1);
            assert!(error().contains("the texture needs"));
            assert_eq!(nameplate_texture(h, 0, null_mut(), rgba.as_mut_ptr(), bytes as u32), bytes);
            assert_eq!(rgba, (*h).asset.as_ref().unwrap().textures[0].rgba);
            assert_eq!(nameplate_texture(h, count as u32, size.as_mut_ptr(), null_mut(), 0), -1);
            nameplate_asset_free(h);
        }
        let mut out = String::new();
        push_str(&mut out, "a\"b\\c\nd\u{1}");
        push_num(&mut out, f64::NAN);
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001\"null");
    }

    #[rustfmt::skip]
    #[test]
    fn setting_curves_cross_the_abi() {
        let wave = r#"{"resolution":"pixel","nodes":[{"id":"f","type":"Frame"},{"id":"p","type":"Param","set":{"name":"speed"}},{"id":"m","type":"mod","in":{"a":"f","b":"p"}},{"id":"h","type":"Hsv","in":{"h":"m"}},{"id":"out","type":"Output","in":{"color":"h"}}]}"#;
        let extra = r#","requires":["nameplate","nameplate_materials"],"params":{"speed":{"value":4,"min":1,"max":8}},"variants":{"slow":{},"fast":{"params":{"speed":7}}}"#;
        unsafe {
            let h = parse(&one_material("wave", extra, wave), 1);
            assert_eq!(nameplate_asset_status(h), STATUS_ASSET, "{}", error());
            let mut out = [0.0f64; 3];
            let mut curve = |param: &str, variant: u32, count: u32| {
                let (param_ptr, param_len, values) = (param.as_ptr(), param.len() as u32, out.as_mut_ptr());
                (nameplate_param_curve(h, "wave".as_ptr(), 4, param_ptr, param_len, variant, count, values, 3), out)
            };
            assert_eq!(curve("speed", 1, 3), (3, [7.0; 3]));
            assert_eq!(curve("speed", 0, 3), (3, [4.0; 3]));
            assert_eq!(curve("size", 0, 3).0, -1);
            assert_eq!(error(), "nameplate: that effect does not read the setting");
            assert_eq!(curve("speed", 0, 4).0, -1);
            assert_eq!(error(), "out holds 3 values, the curve needs 4");
            nameplate_asset_free(h);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn constants_list_limits_nodes_anchors_signals_and_capabilities() {
        assert!(nameplate_constants_json() > 0);
        let json = text();
        for key in [
            r#"{"tag_ints":24,"#, r#""limits":{"download_bytes":2097152,"#, r#""anchors":["top","#, r#""signals":["is_sneaking","#,
            r#""capabilities":["nameplate","#, r#""cost":{"by":"octaves","values":[52,104,156,208,260,312,364,416]}"#,
            r#"{"type":"Fbm","inputs":[{"name":"p","accepts":"vec2","required":true}],"settings":[{"name":"seed","kind":"int","required":false,"min":0,"max":2147483647,"default":[0,0],"choices":[]},"#,
            r#"{"name":"uv","accepts":"vec2","required":false,"default_node":"TagUV"}"#,
        ] {
            assert!(json.contains(key), "{key} in {json}");
        }
    }

    #[derive(Clone)]
    struct Ex {
        h: *const ParsedAsset,
        textures: *const RasterTextures,
        tags: [i32; TAG_INTS],
        frame: u32,
        reactions: u32,
        shots: Vec<u32>,
        motion: u32,
        omit: u32,
        band: u32,
    }

    fn ex(h: *const ParsedAsset, frame: u32) -> Ex {
        let (textures, tags) = (null(), row(1, 40));
        let (reactions, shots, motion, omit, band) = (0, Vec::new(), 0, 0, 0);
        Ex {
            h,
            textures,
            tags,
            frame,
            reactions,
            shots,
            motion,
            omit,
            band,
        }
    }

    #[rustfmt::skip]
    impl Ex {
        fn with(&self, change: impl FnOnce(&mut Ex)) -> Ex {
            let mut ex = self.clone();
            change(&mut ex);
            ex
        }

        unsafe fn call(&self, info: &mut [i32], out: &mut [u8]) -> i32 {
            let ptr = if out.is_empty() { null_mut() } else { out.as_mut_ptr() };
            let (tags, shots, shot_len) = (self.tags.as_ptr(), self.shots.as_ptr(), self.shots.len() as u32);
            nameplate_raster_ex(
                self.h, self.textures, tags, TAG_INTS as u32, self.frame, 1, 0, self.reactions, u32::MAX, u32::MAX, shots,
                shot_len, null(), 0, null(), 0, self.motion, 0, self.omit, self.band, info.as_mut_ptr(), info.len() as u32,
                ptr, out.len() as u32,
            )
        }

        unsafe fn pixels(&self) -> (Vec<i32>, Vec<u8>) {
            let mut info = vec![0; INFO_WORDS];
            let bytes = self.call(&mut info, &mut []);
            assert!(bytes > 0, "{}", error());
            let mut rgba = vec![0u8; bytes as usize];
            assert_eq!(self.call(&mut info, &mut rgba), bytes, "{}", error());
            (info, rgba)
        }

        unsafe fn layers(&self) -> (Vec<i32>, Vec<Vec<u8>>) {
            let (tags, shots, shot_len) = (self.tags.as_ptr(), self.shots.as_ptr(), self.shots.len() as u32);
            let r = nameplate_raster_images(
                self.h, self.textures, tags, TAG_INTS as u32, self.frame, 1, 0, self.reactions, u32::MAX, u32::MAX, shots,
                shot_len, null(), 0, null(), 0, self.motion, 0, 0, 0, 1,
            );
            let info = std::slice::from_raw_parts(nameplate_raster_images_info(r), INFO_WORDS).to_vec();
            let len = |i| *nameplate_raster_image_meta(r, i).add(3) as usize;
            let rgba = |i| std::slice::from_raw_parts(nameplate_raster_image_rgba(r, i), len(i)).to_vec();
            let layers = (0..nameplate_raster_images_count(r)).filter(|&i| nameplate_raster_image_pass(r, i) < 0).map(rgba).collect();
            nameplate_raster_images_free(r);
            (info, layers)
        }
    }

    fn signal(name: &str) -> u32 {
        SIGNALS.iter().position(|(n, _)| *n == name).unwrap() as u32
    }

    #[test]
    fn raster_ex_takes_reactions_one_shots_motion_and_host_textures() {
        unsafe {
            let h = parse(&kit("reaction_plate"), 0);
            let (fire, hurt) = (signal("is_on_fire"), signal("is_hurt") as u8);
            let one_shots = &(*h).asset.as_ref().unwrap().animations.one_shots;
            let shot = one_shots
                .iter()
                .position(|s| s.trigger == Trigger::Start(hurt));
            let shot = shot.unwrap() as u32;
            let idle = ex(h, 4);
            let (info, _) = idle.pixels();
            let held = idle.with(|e| e.reactions = 1 << fire);
            let (burning_info, burning) = held.pixels();
            let (layer_info, layers) = held.layers();
            assert_eq!(layer_info, burning_info);
            assert!(layers.len() > 1 && layers.iter().all(|rgba| rgba.len() == burning.len()));
            let mut legacy = vec![0u8; burning.len()];
            let mut legacy_info = [0i32; RASTER_INFO_INTS];
            let (tags, len, out) = (idle.tags.as_ptr(), legacy.len() as u32, legacy.as_mut_ptr());
            let ints = (TAG_INTS as u32, legacy_info.as_mut_ptr());
            let drawn = nameplate_raster(h, tags, ints.0, 4, 1, 0, 1, fire, 0, ints.1, out, len);
            assert_eq!((drawn, legacy), (burning.len() as i32, burning.clone()));
            let shaking = held.with(|e| e.shots = vec![shot, 1]);
            let (shaken_info, shaken) = shaking.pixels();
            assert_ne!(shaken, burning);
            assert_ne!(shaken_info[7..9], info[7..9]);
            assert_eq!(shaking.with(|e| e.motion = 2).pixels().0[7..9], info[7..9]);
            let mut info = [0i32; 4];
            for (call, message) in [
                (idle.with(|e| e.shots = vec![99, 0]), "no one-shot 99"),
                (idle.with(|e| e.motion = 3), "no motion 3"),
            ] {
                assert_eq!(call.call(&mut info, &mut []), -1, "{message}");
                assert_eq!(error(), message);
            }
            nameplate_asset_free(h);

            let h = parse(&kit("atlas_plate"), 0);
            let textures = nameplate_textures_new();
            for (name, rgba) in [
                ("glint", [200, 40, 255, 255]),
                ("minecraft:block/oak_planks", [120, 90, 40, 255]),
            ] {
                let (rgba, len) = (rgba.repeat(4), name.len() as u32);
                assert_eq!(
                    nameplate_textures_set(textures, name.as_ptr(), len, 2, 2, rgba.as_ptr(), 16),
                    0
                );
            }
            let bare = ex(h, 700);
            assert_ne!(
                bare.pixels().1,
                bare.with(|e| e.textures = textures).pixels().1
            );
            nameplate_textures_free(textures);
            nameplate_asset_free(h);
        }
    }
}
