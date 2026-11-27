use serde_json::{Map, Value};

use crate::issues::{Issue, IssueCode};
use crate::limits::NAMEPLATE_BADGE_OFFSET;
use crate::molang::{compile_checked, Interner, Options, Rules, Warnings};

use super::super::geom::Anchor;
use super::super::image::ColorRef;
use super::super::queries::{scope_for, MolangUse};
use super::super::types::{
    BadgePlacement, BadgeStyle, Decoration, Follow, Formatting, Of, Paint, PlacedBadge, TextMode,
    TextStyle,
};
use super::read::{self, Loc};
use super::{Compiler, Kind};

const TEXT_KEYS: [&str; 8] = [
    "mode",
    "color",
    "shadow",
    "outline",
    "formatting",
    "candidates",
    "backdropHint",
    "motion",
];
const BADGE_KEYS: [&str; 10] = [
    "placement",
    "anchor",
    "offset",
    "tint",
    "decoration",
    "decorationColor",
    "candidates",
    "of",
    "follow",
    "at",
];
const PLACEMENT_KEYS: [&str; 6] = ["placement", "anchor", "offset", "of", "follow", "at"];
const OWNED_OUTPUTS: [&str; 5] = ["color", "shadow", "outline", "scale", "rotation"];

pub(super) struct Fields<'v> {
    entries: Vec<(&'v str, &'v Value, Option<Loc>)>,
    owner: Loc,
}

impl<'v> Fields<'v> {
    fn base(map: Option<&'v Map<String, Value>>, loc: &Loc) -> Fields<'v> {
        Fields {
            entries: map
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.as_str(), v, Some(loc.key(k))))
                .collect(),
            owner: loc.clone(),
        }
    }

    fn merged(
        base: Option<&'v Map<String, Value>>,
        over: &'v Map<String, Value>,
        loc: &Loc,
    ) -> Fields<'v> {
        let mut entries: Vec<(&'v str, &'v Value, Option<Loc>)> = base
            .into_iter()
            .flatten()
            .filter(|(k, _)| !over.contains_key(k.as_str()))
            .map(|(k, v)| (k.as_str(), v, None))
            .collect();
        entries.extend(over.iter().map(|(k, v)| (k.as_str(), v, Some(loc.key(k)))));
        Fields {
            entries,
            owner: loc.clone(),
        }
    }

    fn get(&self, key: &str) -> Option<(&'v Value, Option<&Loc>)> {
        self.entries
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, v, loc)| (*v, loc.as_ref()))
    }

    fn touched(&self, keys: &[&str]) -> bool {
        self.entries
            .iter()
            .any(|(k, _, loc)| loc.is_some() && keys.contains(k))
    }

    fn text(&self, key: &str) -> Option<(&'v str, Option<&Loc>)> {
        let (value, loc) = self.get(key)?;
        Some((value.as_str()?, loc))
    }
}

impl Compiler<'_, '_> {
    fn at(&mut self, loc: Option<&Loc>, issue: impl FnOnce(&Loc) -> Issue) {
        if let Some(loc) = loc {
            let issue = issue(loc);
            self.push(issue);
        }
    }

    fn muted(
        &mut self,
        loc: Option<&Loc>,
        text: &str,
        code: IssueCode,
        any: bool,
    ) -> Option<ColorRef> {
        let result = self.color_ref(text, any);
        match loc {
            Some(loc) => self.report_color(result, loc, code),
            None => result.ok(),
        }
    }

    fn paint(&mut self, fields: &Fields<'_>, key: &str, base: Paint) -> Paint {
        let Some((value, loc)) = fields.get(key) else {
            return base;
        };
        match value.as_str() {
            Some("none") => Paint::None,
            Some("auto") => Paint::Auto,
            Some(text) => self
                .muted(loc, text, IssueCode::Text, true)
                .map_or(base, Paint::Color),
            None => {
                self.at(loc, |l| l.error(IssueCode::Text));
                base
            }
        }
    }

    fn colors(
        &mut self,
        fields: &Fields<'_>,
        key: &str,
        code: IssueCode,
        empty_ok: bool,
        base: Vec<ColorRef>,
    ) -> Vec<ColorRef> {
        let Some((value, loc)) = fields.get(key) else {
            return base;
        };
        let Some(items) = value.as_array() else {
            self.at(loc, |l| l.error(code));
            return base;
        };
        if items.is_empty() && !empty_ok {
            self.at(loc, |l| l.error(code));
            return base;
        }
        let mut colors = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let here = loc.map(|l| l.index(i));
            match item.as_str() {
                Some(text) => {
                    if let Some(color) = self.muted(here.as_ref(), text, code, true) {
                        colors.push(color);
                    }
                }
                None => self.at(here.as_ref(), |l| l.error(code)),
            }
        }
        colors
    }

    pub(super) fn text(
        &mut self,
        manifest: &Map<String, Value>,
        root: &Loc,
    ) -> (TextStyle, Option<Map<String, Value>>) {
        let loc = root.key("text");
        let raw = match manifest.get("text") {
            None => None,
            Some(value) => self.object(value, &loc, IssueCode::Text).cloned(),
        };
        if let Some(map) = &raw {
            self.keys(&loc, map, &TEXT_KEYS);
        }
        let fields = Fields::base(raw.as_ref(), &loc);
        let mut style = self.text_fields(&fields, TextStyle::default());
        style.motion = None;
        (style, raw)
    }

    fn text_fields(&mut self, fields: &Fields<'_>, mut style: TextStyle) -> TextStyle {
        if let Some((value, loc)) = fields.get("mode") {
            match value.as_str() {
                Some("vanilla") => style.mode = TextMode::Vanilla,
                Some("motion") => style.mode = TextMode::Motion,
                Some("owned") => style.mode = TextMode::Owned,
                _ => self.at(loc, |l| l.error(IssueCode::Text)),
            }
        }
        if let Some((value, loc)) = fields.get("formatting") {
            match value.as_str() {
                Some("adjust") => style.formatting = Formatting::Adjust,
                Some("keep") => style.formatting = Formatting::Keep,
                Some("override") => style.formatting = Formatting::Override,
                _ => self.at(loc, |l| l.error(IssueCode::Text)),
            }
        }
        style.color = self.paint(fields, "color", style.color);
        style.shadow = self.paint(fields, "shadow", style.shadow);
        style.outline = self.paint(fields, "outline", style.outline);
        style.candidates = self.colors(
            fields,
            "candidates",
            IssueCode::Text,
            false,
            style.candidates,
        );
        style.backdrop_hint = self.colors(
            fields,
            "backdropHint",
            IssueCode::Text,
            true,
            style.backdrop_hint,
        );
        if let Some((value, loc)) = fields.get("motion") {
            if !value.is_string() && !value.is_null() {
                self.at(loc, |l| l.error(IssueCode::LetterProgram));
            }
        }
        style
    }

    pub(super) fn letter_program(
        &mut self,
        source: Option<&str>,
        mode: TextMode,
        loc: &Loc,
    ) -> Option<u32> {
        let source = source.filter(|s| !s.trim().is_empty())?;
        if mode == TextMode::Vanilla {
            return None;
        }
        let program = self.molang(source, MolangUse::Text, loc)?;
        if mode == TextMode::Motion {
            let owned = owned_outputs(source);
            if !owned.is_empty() {
                self.soft(
                    loc.error(IssueCode::LetterOwned)
                        .name("outputs", &owned.join(", ")),
                );
            }
        }
        Some(program)
    }

    pub(super) fn variant_text(
        &mut self,
        spec: &Map<String, Value>,
        at: &Loc,
        base: &TextStyle,
        base_raw: Option<&Map<String, Value>>,
    ) -> TextStyle {
        let base_motion = base_raw
            .and_then(|m| m.get("motion"))
            .and_then(Value::as_str);
        let Some(value) = spec.get("text") else {
            return base.clone();
        };
        let loc = at.key("text");
        let Some(over) = self.object(value, &loc, IssueCode::Text) else {
            return base.clone();
        };
        self.keys(&loc, over, &TEXT_KEYS);
        let fields = Fields::merged(base_raw, over, &loc);
        let mut style = self.text_fields(&fields, base.clone());
        if over.contains_key("motion") || over.contains_key("mode") {
            let source = match over.get("motion") {
                Some(value) => value.as_str(),
                None => base_motion,
            };
            let same = style.mode == base.mode && source == base_motion;
            style.motion = if same {
                base.motion
            } else {
                let here = if over.contains_key("motion") {
                    loc.key("motion")
                } else {
                    Loc::new(super::MANIFEST, "text.motion")
                };
                self.letter_program(source, style.mode, &here)
            };
            let written = over
                .get("motion")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty());
            if written && style.mode == TextMode::Vanilla {
                self.soft(loc.key("motion").error(IssueCode::LetterProgram));
            }
        }
        style
    }

    pub(super) fn base_letters(
        &mut self,
        base: &mut TextStyle,
        base_raw: Option<&Map<String, Value>>,
        any_letters: bool,
    ) {
        let source = base_raw
            .and_then(|m| m.get("motion"))
            .and_then(Value::as_str);
        let loc = Loc::new(super::MANIFEST, "text.motion");
        base.motion = self.letter_program(source, base.mode, &loc);
        if base.mode == TextMode::Vanilla
            && source.is_some_and(|s| !s.trim().is_empty())
            && !any_letters
        {
            self.soft(loc.error(IssueCode::LetterProgram));
        }
    }

    pub(super) fn badge(
        &mut self,
        manifest: &Map<String, Value>,
        root: &Loc,
    ) -> (BadgeStyle, Option<Map<String, Value>>) {
        let loc = root.key("badge");
        let raw = match manifest.get("badge") {
            None => None,
            Some(value) => self.object(value, &loc, IssueCode::Badge).cloned(),
        };
        if let Some(map) = &raw {
            self.keys(&loc, map, &BADGE_KEYS);
        }
        let fields = Fields::base(raw.as_ref(), &loc);
        let style = self.badge_fields(&fields, BadgeStyle::default(), true);
        (style, raw)
    }

    pub(super) fn variant_badge(
        &mut self,
        spec: &Map<String, Value>,
        at: &Loc,
        base: &BadgeStyle,
        base_raw: Option<&Map<String, Value>>,
    ) -> BadgeStyle {
        let Some(value) = spec.get("badge") else {
            return base.clone();
        };
        let loc = at.key("badge");
        let Some(over) = self.object(value, &loc, IssueCode::Badge) else {
            return base.clone();
        };
        self.keys(&loc, over, &BADGE_KEYS);
        let fields = Fields::merged(base_raw, over, &loc);
        let placement = fields.touched(&PLACEMENT_KEYS);
        self.badge_fields(&fields, base.clone(), placement)
    }

    fn badge_fields(
        &mut self,
        fields: &Fields<'_>,
        mut style: BadgeStyle,
        placement: bool,
    ) -> BadgeStyle {
        let code = IssueCode::Badge;
        if placement {
            style.placement = self.placement(fields);
        }
        if let Some((value, loc)) = fields.get("tint") {
            style.tint = match value.as_str() {
                Some("none") => None,
                Some(text) => self.muted(loc, text, code, true),
                None => {
                    self.at(loc, |l| l.error(code));
                    None
                }
            };
        }
        if fields.get("decoration").is_some() || fields.get("decorationColor").is_some() {
            let color = match fields.text("decorationColor") {
                Some((text, loc)) => self.muted(loc, text, code, true),
                None => {
                    if let Some((_, loc)) = fields.get("decorationColor") {
                        self.at(loc, |l| l.error(code));
                    }
                    None
                }
            };
            let kind = fields.get("decoration");
            style.decoration = match kind.map(|(v, l)| (v.as_str(), l)) {
                None | Some((Some("none"), _)) => Decoration::None,
                Some((Some("auto"), _)) => Decoration::Auto,
                Some((Some("shadow"), _)) => Decoration::Shadow(color),
                Some((Some("outline"), _)) => Decoration::Outline(color),
                Some((_, loc)) => {
                    self.at(loc, |l| l.error(code));
                    Decoration::None
                }
            };
            if !matches!(
                style.decoration,
                Decoration::Shadow(_) | Decoration::Outline(_)
            ) {
                if let Some((_, loc)) = fields.get("decorationColor") {
                    self.at(loc, |l| l.error(code));
                }
            }
        }
        style.candidates = self.colors(fields, "candidates", code, false, style.candidates);
        style
    }

    fn placement(&mut self, fields: &Fields<'_>) -> BadgePlacement {
        let code = IssueCode::Badge;
        let owner = fields.owner.clone();
        let kind = match fields.get("placement") {
            None => "reserve",
            Some((value, loc)) => match value.as_str() {
                Some(name @ ("reserve" | "carried" | "placed")) => name,
                _ => {
                    self.at(loc, |l| l.error(code));
                    "reserve"
                }
            },
        };
        let anchor = match fields.get("anchor") {
            None => None,
            Some((value, loc)) => {
                let parsed = value.as_str().and_then(Anchor::parse);
                if parsed.is_none() {
                    self.at(loc, |l| l.error(code));
                }
                parsed
            }
        };
        let offset = match fields.get("offset") {
            None => [0, 0],
            Some((value, loc)) => {
                let limit = i64::from(NAMEPLATE_BADGE_OFFSET);
                match read::pair_i64(value) {
                    Some([x, y])
                        if (-limit..=limit).contains(&x) && (-limit..=limit).contains(&y) =>
                    {
                        [x as i8, y as i8]
                    }
                    _ => {
                        self.at(loc, |l| l.error(code).value("limit", limit as f64));
                        [0, 0]
                    }
                }
            }
        };
        let follow = match fields.get("follow") {
            None => None,
            Some((value, loc)) => {
                let parsed = match value.as_str() {
                    Some("outside") => Some(Follow::Outside),
                    Some("line") => Some(Follow::Line),
                    _ => None,
                };
                if parsed.is_none() || kind != "placed" {
                    self.at(loc, |l| l.error(IssueCode::Follow));
                }
                parsed
            }
        };
        let of = match fields.get("of") {
            None => {
                if follow.is_some() {
                    self.push(owner.key("of").error(code));
                }
                Of::Block
            }
            Some((value, loc)) => {
                let index = value
                    .as_str()
                    .and_then(|id| self.layer_index(id))
                    .filter(|i| self.decls[usize::from(*i)].kind == Some(Kind::Body));
                match index {
                    Some(i) if follow.is_some() => Of::Layer(i),
                    _ => {
                        self.at(loc, |l| l.error(code));
                        Of::Block
                    }
                }
            }
        };
        let at = match fields.get("at") {
            None => None,
            Some((value, loc)) => match read::number(value) {
                Some(v)
                    if follow.is_some()
                        && anchor.is_some_and(Anchor::takes_at)
                        && (0.0..=1.0).contains(&v) =>
                {
                    Some(v)
                }
                _ => {
                    self.at(loc, |l| l.error(IssueCode::Anchor));
                    None
                }
            },
        };
        match kind {
            "carried" => BadgePlacement::Carried,
            "placed" => match anchor {
                Some(anchor) => BadgePlacement::Placed(PlacedBadge {
                    anchor,
                    at,
                    offset,
                    of,
                    follow,
                }),
                None => {
                    if fields.get("anchor").is_none() {
                        self.push(owner.key("anchor").error(code));
                    }
                    BadgePlacement::Reserve
                }
            },
            _ => BadgePlacement::Reserve,
        }
    }
}

fn owned_outputs(source: &str) -> Vec<&'static str> {
    let options = Options {
        rules: Rules::Strict,
        queries: scope_for(MolangUse::Text),
    };
    let mut interner = Interner::default();
    if compile_checked(source, &mut interner, &options, &mut Warnings::default()).is_err() {
        return Vec::new();
    }
    OWNED_OUTPUTS
        .into_iter()
        .filter(|name| interner.variable_slot(name).is_some())
        .collect()
}
