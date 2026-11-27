use std::borrow::Cow;
use std::collections::BTreeSet;

use crate::limits::{
    NAMEPLATE_LETTER_OFFSET, NAMEPLATE_LETTER_SCALE_MAX_QUARTERS,
    NAMEPLATE_LETTER_SCALE_MIN_QUARTERS,
};
use crate::molang::{snap, Interner, Queries, Scope};

use super::abi::{
    DECORATION_NONE, DECORATION_OUTLINE, DECORATION_SHADOW, GLYPH_FLOATS, GLYPH_WIDTH, GLYPH_X,
};
use super::color::{flatten, from_oklch, relative_luminance, to_oklch, BACKDROP_GRAY};
use super::geom::Rect;
use super::image::ColorRef;
use super::palette::resolve_ref;
use super::queries::NameplateQuery;
use super::types::{
    BadgeStyle, Decoration, LetterPose, NameplateAsset, Paint, PaletteFrame, TextMode, TextStyle,
    LETTER_KEEP, MAX_LETTERS, OPAQUE, SCALE_ONE_QUARTERS,
};

pub const CONTRAST_TARGET: f64 = 4.5;
pub const CONTRAST_LARGE: f64 = 3.0;
pub const CONTRAST_MAX: f64 = 21.0;
pub const ADJUST_SAMPLES: u32 = 256;
pub const ADJUST_BISECTIONS: u32 = 10;
pub const READABLE_SHIFT: i32 = 2;
pub const READABLE_OPACITY: u8 = 128;
pub const READABLE_ROTATION: i16 = 15;
pub const BADGE_ART_RGB: u32 = 0x00d6_d6d6;
pub const WHITE: u32 = 0xffff_ffff;
pub const NEAR_BLACK: u32 = 0xff1a_1a1a;
pub const DEFAULT_CANDIDATES: [u32; 2] = [WHITE, NEAR_BLACK];
pub const TEXT_KEEP: u32 = 0;
pub const LETTER_OFF: u32 = 1;

const ALPHA: u32 = 0xff00_0000;
const RGB: u32 = 0x00ff_ffff;
const KEEP_VALUE: f32 = -1.0;
const FULL_TURN: i32 = 360;
const OUTPUT_X: usize = 0;
const OUTPUT_OPACITY: usize = 1;
const OUTPUT_COLOR: usize = 2;
const OUTPUT_SHADOW: usize = 3;
const OUTPUT_OUTLINE: usize = 4;
const OUTPUT_SCALE: usize = 5;
const OUTPUT_ROTATION: usize = 6;
const OUTPUTS: [&str; 7] = [
    "x", "opacity", "color", "shadow", "outline", "scale", "rotation",
];
const OUTPUT_DEFAULTS: [f32; 7] = [0.0, 1.0, KEEP_VALUE, KEEP_VALUE, KEEP_VALUE, 1.0, 0.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShadowUse {
    #[default]
    Leave,
    Remove,
    Always,
    WithDropShadow,
}

impl ShadowUse {
    pub const fn value(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ResolvedText {
    pub color: u32,
    pub shadow: u32,
    pub shadow_use: ShadowUse,
    pub outline: u32,
    pub decoration: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ResolvedBadge {
    pub tint: u32,
    pub decoration: i32,
    pub decoration_rgb: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backdrop {
    colors: BTreeSet<u32>,
}

impl Backdrop {
    pub fn new() -> Backdrop {
        Backdrop::default()
    }

    pub fn add(&mut self, argb: u32) {
        match argb >> 24 {
            0 => {}
            0xff => {
                self.colors.insert(argb);
            }
            _ => {
                self.colors.insert(flatten(argb, BACKDROP_GRAY));
            }
        }
    }

    pub fn add_opaque(&mut self, rgb: u32) {
        self.colors.insert(ALPHA | rgb);
    }

    pub fn sample(&mut self, pixels: &[u32], width: usize, origin: (i32, i32), area: Rect) {
        if width == 0 {
            return;
        }
        let rows = pixels.len().div_ceil(width);
        let span = |low: i32, high: i32, base: i32, size: usize| {
            let start = (i64::from(low) - i64::from(base)).max(0);
            let end = (i64::from(high) - i64::from(base)).min(size as i64);
            start as usize..end.max(start) as usize
        };
        let xs = span(area.x0, area.x1, origin.0, width);
        for y in span(area.y0, area.y1, origin.1, rows) {
            let row = y * width;
            for x in xs.clone() {
                if let Some(argb) = pixels.get(row + x) {
                    self.add(*argb);
                }
            }
        }
    }

    pub fn merge(&mut self, other: &Backdrop) {
        self.colors.extend(other.colors.iter().copied());
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn colors(&self) -> Vec<u32> {
        self.colors.iter().copied().collect()
    }
}

pub fn decoration_for(ratio: f64) -> i32 {
    if ratio >= CONTRAST_TARGET {
        DECORATION_NONE
    } else if ratio >= CONTRAST_LARGE {
        DECORATION_SHADOW
    } else {
        DECORATION_OUTLINE
    }
}

fn ratio(a: f64, b: f64) -> f64 {
    let (light, dark) = if a >= b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

fn luminances(backdrop: &[u32]) -> Vec<f64> {
    backdrop.iter().map(|c| relative_luminance(*c)).collect()
}

fn worst_ratio(luminance: f64, backdrop: &[f64]) -> f64 {
    backdrop
        .iter()
        .map(|l| ratio(luminance, *l))
        .fold(CONTRAST_MAX, f64::min)
}

pub fn min_contrast(color: u32, backdrop: &[u32]) -> f64 {
    let luminance = relative_luminance(color);
    backdrop
        .iter()
        .map(|c| ratio(luminance, relative_luminance(*c)))
        .fold(CONTRAST_MAX, f64::min)
}

fn or_default(candidates: &[u32]) -> &[u32] {
    if candidates.is_empty() {
        &DEFAULT_CANDIDATES
    } else {
        candidates
    }
}

pub fn choose(candidates: &[u32], backdrop: &[u32]) -> (u32, f64) {
    let candidates = or_default(candidates);
    let lums = luminances(backdrop);
    let mut best = (
        candidates[0],
        worst_ratio(relative_luminance(candidates[0]), &lums),
    );
    for candidate in &candidates[1..] {
        let score = worst_ratio(relative_luminance(*candidate), &lums);
        if score > best.1 {
            best = (*candidate, score);
        }
    }
    best
}

pub fn opposite(color: u32, candidates: &[u32]) -> u32 {
    let candidates = or_default(candidates);
    let luminance = relative_luminance(color);
    let mut best = (
        candidates[0],
        ratio(luminance, relative_luminance(candidates[0])),
    );
    for candidate in &candidates[1..] {
        let score = ratio(luminance, relative_luminance(*candidate));
        if score > best.1 {
            best = (*candidate, score);
        }
    }
    best.0
}

pub fn adjust(color: u32, backdrop: &[u32]) -> u32 {
    if backdrop.is_empty() {
        return color;
    }
    let lums = luminances(backdrop);
    let score = |c: u32| worst_ratio(relative_luminance(c), &lums);
    let original = score(color);
    if original >= CONTRAST_TARGET {
        return color;
    }
    let [origin, chroma, hue] = to_oklch(color | ALPHA);
    let at = |lightness: f64| from_oklch([lightness, chroma, hue], OPAQUE);
    let step = 1.0 / f64::from(ADJUST_SAMPLES);

    let mut feasible: Option<(f64, u32, f64)> = None;
    let mut best = (original, color | ALPHA, 0.0);
    for i in 0..=ADJUST_SAMPLES {
        let lightness = f64::from(i) * step;
        let sample = at(lightness);
        let ratio = score(sample);
        let distance = (lightness - origin).abs();
        if ratio >= CONTRAST_TARGET {
            if feasible.is_none_or(|(_, _, d)| distance < d) {
                feasible = Some((lightness, sample, distance));
            }
        } else if ratio > best.0 || (ratio == best.0 && distance < best.2) {
            best = (ratio, sample, distance);
        }
    }

    let found = match feasible {
        None => best.1,
        Some((mut high, mut found, _)) => {
            let mut low = if high > origin {
                origin.max(high - step)
            } else {
                origin.min(high + step)
            };
            for _ in 0..ADJUST_BISECTIONS {
                let mid = (low + high) * 0.5;
                let sample = at(mid);
                if score(sample) >= CONTRAST_TARGET {
                    high = mid;
                    found = sample;
                } else {
                    low = mid;
                }
            }
            found
        }
    };
    (color & ALPHA) | (found & RGB)
}

fn resolve(color: ColorRef, palette: &PaletteFrame, fixed: &[u32]) -> u32 {
    ALPHA | (resolve_ref(color, palette, fixed, &[]) & RGB)
}

fn resolve_all(colors: &[ColorRef], palette: &PaletteFrame, fixed: &[u32]) -> Vec<u32> {
    colors.iter().map(|c| resolve(*c, palette, fixed)).collect()
}

fn with_hint<'a>(
    backdrop: &'a [u32],
    hint: &[ColorRef],
    palette: &PaletteFrame,
    fixed: &[u32],
) -> Cow<'a, [u32]> {
    if hint.is_empty() {
        return Cow::Borrowed(backdrop);
    }
    let mut all = backdrop.to_vec();
    all.extend(resolve_all(hint, palette, fixed));
    Cow::Owned(all)
}

pub fn resolve_text(
    style: &TextStyle,
    palette: &PaletteFrame,
    fixed: &[u32],
    backdrop: &[u32],
    external: Option<u32>,
) -> ResolvedText {
    let backdrop = with_hint(backdrop, &style.backdrop_hint, palette, fixed);
    let candidates = resolve_all(&style.candidates, palette, fixed);
    let candidates = or_default(&candidates);
    let (judged, glyph, ratio) = match (external, style.color) {
        (Some(external), _) => {
            let rgb = ALPHA | (external & RGB);
            (rgb, TEXT_KEEP, min_contrast(rgb, &backdrop))
        }
        (None, Paint::Auto) => {
            let (picked, ratio) = choose(candidates, &backdrop);
            let picked = ALPHA | (picked & RGB);
            (picked, picked, ratio)
        }
        (None, Paint::Color(color)) => {
            let rgb = resolve(color, palette, fixed);
            (rgb, rgb, min_contrast(rgb, &backdrop))
        }
        (None, Paint::None) => (WHITE, TEXT_KEEP, min_contrast(WHITE, &backdrop)),
    };
    let decoration = decoration_for(ratio);
    let other = ALPHA | (opposite(judged, candidates) & RGB);

    let outlined = match style.outline {
        Paint::Color(_) => true,
        Paint::Auto => {
            decoration == DECORATION_OUTLINE
                || (decoration == DECORATION_SHADOW && style.shadow == Paint::None)
        }
        Paint::None => false,
    };
    let outline = match style.outline {
        Paint::Color(color) => resolve(color, palette, fixed),
        _ if outlined => other,
        _ => TEXT_KEEP,
    };
    let (shadow_use, shadow) = match style.shadow {
        _ if outlined => (ShadowUse::Remove, TEXT_KEEP),
        Paint::Color(color) => (ShadowUse::Always, resolve(color, palette, fixed)),
        Paint::Auto if decoration == DECORATION_NONE => (ShadowUse::WithDropShadow, other),
        Paint::Auto => (ShadowUse::Always, other),
        Paint::None => (ShadowUse::Leave, TEXT_KEEP),
    };
    ResolvedText {
        color: glyph,
        shadow,
        shadow_use,
        outline,
        decoration,
    }
}

pub fn resolve_badge(
    style: &BadgeStyle,
    palette: &PaletteFrame,
    fixed: &[u32],
    backdrop: &[u32],
) -> ResolvedBadge {
    let tint = style.tint.map_or(TEXT_KEEP, |c| resolve(c, palette, fixed));
    let judged = if tint == TEXT_KEEP {
        ALPHA | BADGE_ART_RGB
    } else {
        tint
    };
    let candidates = resolve_all(&style.candidates, palette, fixed);
    let other = ALPHA | (opposite(judged, or_default(&candidates)) & RGB);
    let ratio = min_contrast(judged, backdrop);
    let own = |color: Option<ColorRef>| color.map_or(other, |c| resolve(c, palette, fixed));
    let (decoration, decoration_rgb) = match style.decoration {
        Decoration::Outline(color) => (DECORATION_OUTLINE, own(color)),
        _ if ratio < CONTRAST_LARGE => (DECORATION_OUTLINE, other),
        Decoration::None => (DECORATION_NONE, TEXT_KEEP),
        Decoration::Shadow(color) => (DECORATION_SHADOW, own(color)),
        Decoration::Auto => match decoration_for(ratio) {
            DECORATION_NONE => (DECORATION_NONE, TEXT_KEEP),
            kind => (kind, other),
        },
    };
    ResolvedBadge {
        tint,
        decoration,
        decoration_rgb,
    }
}

fn glyph_slot(interner: &Interner, reads: &[u16], query: NameplateQuery) -> Option<u16> {
    interner
        .query_slot(query.name())
        .filter(|slot| reads.binary_search(slot).is_ok())
}

fn offset(value: f64) -> i8 {
    let limit = NAMEPLATE_LETTER_OFFSET as i32;
    snap(value).clamp(-limit, limit) as i8
}

fn letter_color(value: f32) -> u32 {
    if !value.is_finite() || value < 0.0 {
        return LETTER_KEEP;
    }
    ALPHA | libm::floor(f64::from(value).min(f64::from(RGB))) as u32
}

fn letter_decoration(value: f32) -> u32 {
    if !value.is_finite() || value == KEEP_VALUE {
        LETTER_KEEP
    } else if value < 0.0 {
        LETTER_OFF
    } else {
        letter_color(value)
    }
}

fn letter_pose(dy: f64, outputs: &[f32; 7], owned: bool) -> LetterPose {
    let opacity = snap(f64::from(outputs[OUTPUT_OPACITY]) * f64::from(OPAQUE));
    let mut pose = LetterPose {
        dx: offset(f64::from(outputs[OUTPUT_X])),
        dy: offset(dy),
        opacity: opacity.clamp(0, i32::from(OPAQUE)) as u8,
        ..LetterPose::STILL
    };
    if owned {
        let quarters = snap(f64::from(outputs[OUTPUT_SCALE]) * f64::from(SCALE_ONE_QUARTERS));
        pose.color = letter_color(outputs[OUTPUT_COLOR]);
        pose.shadow = letter_decoration(outputs[OUTPUT_SHADOW]);
        pose.outline = letter_decoration(outputs[OUTPUT_OUTLINE]);
        pose.scale_q = quarters.clamp(
            NAMEPLATE_LETTER_SCALE_MIN_QUARTERS as i32,
            NAMEPLATE_LETTER_SCALE_MAX_QUARTERS as i32,
        ) as u8;
        pose.rotation = snap(f64::from(outputs[OUTPUT_ROTATION])).rem_euclid(FULL_TURN) as i16;
    }
    pose
}

pub fn run_letters(
    asset: &NameplateAsset,
    program: u32,
    mode: TextMode,
    glyphs: &[f32],
    scope: &mut Scope<'_>,
    queries: &mut dyn Queries,
    out: &mut [LetterPose],
) -> bool {
    out.fill(LetterPose::STILL);
    let count = glyphs.len() / GLYPH_FLOATS;
    if mode == TextMode::Vanilla || count > MAX_LETTERS || count > out.len() {
        return true;
    }
    let Some(entry) = asset.molang.programs.get(program as usize) else {
        return false;
    };
    let interner = &asset.molang.interner;
    let reads = &entry.program.reads().queries;
    let index = glyph_slot(interner, reads, NameplateQuery::GlyphIndex);
    let x = glyph_slot(interner, reads, NameplateQuery::GlyphX);
    let width = glyph_slot(interner, reads, NameplateQuery::GlyphWidth);
    if let Some(slot) = glyph_slot(interner, reads, NameplateQuery::GlyphCount) {
        scope.set_query(slot, count as f32);
    }
    let slots = OUTPUTS.map(|name| interner.variable_slot(name));
    let saved = slots.map(|slot| slot.map_or(0.0, |s| scope.variable(s)));
    let owned = mode == TextMode::Owned;

    let mut ok = true;
    for (i, glyph) in glyphs.chunks_exact(GLYPH_FLOATS).enumerate() {
        let values = [
            (index, i as f32),
            (x, glyph[GLYPH_X]),
            (width, glyph[GLYPH_WIDTH]),
        ];
        for (slot, value) in values {
            if let Some(slot) = slot {
                scope.set_query(slot, value);
            }
        }
        for (slot, value) in slots.iter().zip(OUTPUT_DEFAULTS) {
            if let Some(slot) = slot {
                scope.set_variable(*slot, value);
            }
        }
        scope.clear_temps();
        let outcome = entry.program.evaluate(scope, queries);
        if outcome.failed {
            ok = false;
            break;
        }
        let mut outputs = OUTPUT_DEFAULTS;
        for (value, slot) in outputs.iter_mut().zip(slots) {
            if let Some(slot) = slot {
                *value = scope.variable(slot);
            }
        }
        out[i] = letter_pose(outcome.value, &outputs, owned);
    }

    for (slot, value) in slots.iter().zip(saved) {
        if let Some(slot) = slot {
            scope.set_variable(*slot, value);
        }
    }
    if !ok {
        out.fill(LetterPose::STILL);
    }
    ok
}

fn readable(letter: &LetterPose) -> bool {
    let turn = i32::from(letter.rotation).rem_euclid(FULL_TURN);
    i32::from(letter.dx).abs() <= READABLE_SHIFT
        && i32::from(letter.dy).abs() <= READABLE_SHIFT
        && letter.opacity >= READABLE_OPACITY
        && letter.scale_q.abs_diff(SCALE_ONE_QUARTERS) <= 1
        && turn.min(FULL_TURN - turn) <= i32::from(READABLE_ROTATION)
}

pub fn readable_in_place(letters: &[LetterPose]) -> bool {
    letters.iter().all(readable)
}

pub fn readable_long_enough(in_place: u32, frames: u32) -> bool {
    frames > 0 && u64::from(in_place) * 2 >= u64::from(frames)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::container::CAPABILITIES;
    use crate::molang::{NoQueries, Runtime};
    use crate::nameplate::abi::{DECORATION_NONE, DECORATION_OUTLINE, DECORATION_SHADOW};
    use crate::nameplate::color::{contrast_ratio, from_oklch, to_oklch};
    use crate::nameplate::frame::tests::compile_files;
    use crate::nameplate::types::{Paint, LETTER_KEEP, MAX_LETTERS, OPAQUE};

    const GOLD: [u32; 2] = [0xfffff6d8, 0xffffe3a0];

    fn auto() -> TextStyle {
        TextStyle { color: Paint::Auto, shadow: Paint::Auto, outline: Paint::Auto, ..TextStyle::default() }
    }

    #[test]
    fn auto_text_picks_the_best_minimum_and_decorates_at_four_and_a_half_and_three() {
        assert_eq!(choose(&DEFAULT_CANDIDATES, &[0xff20_2020; 4]).0, WHITE);
        assert_eq!(choose(&DEFAULT_CANDIDATES, &GOLD).0, NEAR_BLACK);
        let mut mostly_black = vec![0xff00_0000; 99];
        mostly_black.insert(42, 0xfff0_f0f0);
        assert_eq!(choose(&[WHITE, 0xff77_7777], &mostly_black), (0xff77_7777, min_contrast(0xff77_7777, &mostly_black)));
        assert_eq!(choose(&[0xff12_3456, WHITE], &[]), (0xff12_3456, 21.0));
        let decorations = [21.0, CONTRAST_TARGET, 4.499, CONTRAST_LARGE, 2.999].map(decoration_for);
        assert_eq!(decorations, [DECORATION_NONE, DECORATION_NONE, DECORATION_SHADOW, DECORATION_SHADOW, DECORATION_OUTLINE]);
        let frame = PaletteFrame::default();
        let clean = resolve_text(&auto(), &frame, &[], &GOLD, None);
        assert_eq!((clean.color, clean.decoration, clean.shadow_use, clean.shadow), (NEAR_BLACK, DECORATION_NONE, ShadowUse::WithDropShadow, WHITE));
        let shadowed = resolve_text(&auto(), &frame, &[], &[0xff77_7777], None);
        assert_eq!((shadowed.color, shadowed.decoration, shadowed.shadow_use), (WHITE, DECORATION_SHADOW, ShadowUse::Always));
        let outlined = resolve_text(&auto(), &frame, &[], &[0xff00_0000, WHITE], None);
        assert_eq!((outlined.decoration, outlined.shadow_use), (DECORATION_OUTLINE, ShadowUse::Remove));
        assert_eq!(outlined.outline, opposite(outlined.color, &DEFAULT_CANDIDATES));
        let vanilla = resolve_text(&TextStyle::default(), &frame, &[], &GOLD, None);
        assert_eq!((vanilla.color, vanilla.shadow_use, vanilla.outline), (TEXT_KEEP, ShadowUse::Leave, TEXT_KEEP));
        let external = resolve_text(&auto(), &frame, &[], &GOLD, Some(0xffff_ff55));
        assert_eq!((external.color, external.decoration), (TEXT_KEEP, DECORATION_OUTLINE));
    }

    #[test]
    fn adjust_keeps_hue_and_moves_lightness_only_as_far_as_four_and_a_half_needs() {
        let mut seed = 99u64;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            0xff00_0000 | ((seed >> 33) as u32 & 0x00ff_ffff)
        };
        let mut cases = vec![(0xff3a_7bd5, vec![WHITE; 4]), (0xff20_3060, vec![0xff10_1010; 3]), (0xffff_4040, vec![0xff00_0000, 0xff80_8080, WHITE])];
        for _ in 0..120 {
            let count = 1 + next() as usize % 24;
            cases.push((next(), (0..count).map(|_| next()).collect()));
        }
        for (color, backdrop) in cases {
            let adjusted = adjust(color, &backdrop);
            let (before, after) = (min_contrast(color, &backdrop), min_contrast(adjusted, &backdrop));
            if before >= CONTRAST_TARGET {
                assert_eq!(adjusted, color);
                continue;
            }
            let [origin, chroma, hue] = to_oklch(color);
            assert!(after >= before, "{color:08x}: never worse than the input");
            let samples = (0..=ADJUST_SAMPLES).map(|i| f64::from(i) / f64::from(ADJUST_SAMPLES));
            let samples: Vec<(f64, u32)> = samples.map(|l| (l, from_oklch([l, chroma, hue], OPAQUE))).collect();
            let feasible = samples.iter().filter(|(_, c)| min_contrast(*c, &backdrop) >= CONTRAST_TARGET);
            let nearest = feasible.map(|(l, _)| (l - origin).abs()).fold(f64::INFINITY, f64::min);
            if nearest.is_finite() {
                let moved = (to_oklch(adjusted)[0] - origin).abs();
                assert!(after >= CONTRAST_TARGET, "{color:08x}: reaches 4.5");
                assert!(moved <= nearest + 0.006 && moved >= nearest - 1.0 / f64::from(ADJUST_SAMPLES) - 0.006, "{color:08x}");
            } else {
                assert!(samples.iter().all(|(_, s)| min_contrast(*s, &backdrop) <= after), "{color:08x}: a sample beats the best");
            }
        }
        let (blue, darker) = (0xff3a_7bd5, adjust(0xff3a_7bd5, &[WHITE]));
        assert!(contrast_ratio(darker, WHITE) >= CONTRAST_TARGET && to_oklch(darker)[0] < to_oklch(blue)[0]);
        assert!((to_oklch(blue)[2] - to_oklch(darker)[2]).abs() < 2.5);
    }

    fn letters(text: Value, mode: TextMode, count: usize) -> (bool, [LetterPose; MAX_LETTERS]) {
        let manifest = json!({ "format": 1, "kind": "nameplate", "id": "plate", "requires": CAPABILITIES, "fps": 6, "loop": 12,
            "text": text, "layers": [{ "id": "body" }] });
        let files = vec![
            ("manifest.json".to_string(), manifest.to_string().into_bytes()),
            ("palettes.json".into(), br##"{"slots":[{"id":"ink","ramp":["#FFFFFF"]}]}"##.to_vec()),
            ("layers/body.json".into(), br#"{"kind":"body","fill":{"color":"ink"}}"#.to_vec()),
        ];
        let asset = compile_files(&files, None);
        let glyphs: Vec<f32> = (0..count).flat_map(|i| [i as f32 * 6.0, 5.0 + (i % 2) as f32]).collect();
        let mut runtime = Runtime::new(&asset.molang.interner);
        let mut out = [LetterPose::default(); MAX_LETTERS];
        let program = asset.variants[0].text.motion.expect("a letter program");
        (run_letters(&asset, program, mode, &glyphs, &mut runtime.frame(), &mut NoQueries, &mut out), out)
    }

    #[test]
    fn letter_programs_read_glyph_queries_clamp_and_write_owned_decorations() {
        let motion = "v.x = q.glyph_index - q.glyph_count; v.opacity = 0.5; return q.glyph_x / 2 + q.glyph_width - 5;";
        let (ok, out) = letters(json!({ "mode": "motion", "motion": motion }), TextMode::Motion, 3);
        assert!(ok);
        let moved: Vec<_> = out[..3].iter().map(|p| (p.dx, p.dy, p.opacity, p.color)).collect();
        assert_eq!(moved, [(-3, 0, 128, LETTER_KEEP), (-2, 4, 128, LETTER_KEEP), (-1, 6, 128, LETTER_KEEP)]);
        assert!(out[3..].iter().all(|p| *p == LetterPose::STILL));
        let clamped = json!({ "mode": "motion", "motion": "v.x = -100; return q.glyph_index * 20;" });
        let (_, out) = letters(clamped.clone(), TextMode::Motion, 4);
        assert_eq!(out[..3].iter().map(|p| (p.dx, p.dy)).collect::<Vec<_>>(), [(-32, 0), (-32, 20), (-32, 32)]);
        assert!(letters(clamped.clone(), TextMode::Motion, MAX_LETTERS + 1).1.iter().all(|p| *p == LetterPose::STILL));
        assert!(letters(clamped, TextMode::Vanilla, 3).1.iter().all(|p| *p == LetterPose::STILL));
        let owned = "v.color = 16711680; v.shadow = -2; v.scale = q.glyph_index == 0 ? 1.25 : (q.glyph_index == 1 ? 0.1 : 5); v.rotation = -10; return 0;";
        let (_, out) = letters(json!({ "mode": "owned", "motion": owned }), TextMode::Owned, 3);
        assert_eq!((out[0].color, out[0].shadow, out[0].outline, out[0].rotation), (0xffff_0000, LETTER_OFF, LETTER_KEEP, 350));
        assert_eq!([out[0].scale_q, out[1].scale_q, out[2].scale_q], [5, 1, 12]);
        let failing = json!({ "mode": "motion", "motion": "return math.die_roll(400, 0, 1) * 0 + 3;" });
        let (ok, out) = letters(failing, TextMode::Motion, 20);
        assert!(!ok && out.iter().all(|p| *p == LetterPose::STILL));
    }
}
