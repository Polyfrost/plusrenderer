use std::collections::BTreeMap;
use std::slice;

use crate::issues::{path, Issue, IssueCode, Report};
use crate::limits::NAMEPLATE_OFFSET;

use super::super::abi::{GLYPH_FLOATS, GLYPH_WIDTH, GLYPH_X};
use super::super::anim::ClipRef;
use super::super::geom::{BlockInput, Rect};
use super::super::layout::reference_inputs;
use super::super::palette::{base_frame, resolve_ref};
use super::super::raster::{composite, Canvas, Colors, Draw, DrawSource};
use super::super::text::{
    choose, min_contrast, readable_in_place, Backdrop, CONTRAST_LARGE, DEFAULT_CANDIDATES, WHITE,
};
use super::super::types::{
    BadgePlacement, Clip, Depth, LayerContent, LetterPose, NameplateAsset, Paint, PaletteFrame,
    Target, TextMode, Variant, MAX_LETTERS, OPAQUE,
};
use super::budget::{clips, Measured};
use super::sim::{
    clip_length, distinct_variants, one_line, resolved, sampled_frames, signal_cases, Event,
    Scenario, SignalCase, SignalSample, Source, Stage,
};
use super::{animation_path, layer_path, MANIFEST};

const SAMPLED_FRAMES: u32 = 60;
const FADED_BELOW: u8 = 128;
const COVERED_PERCENT: u32 = 25;
const COVERAGE_PERCENT: u32 = 50;
const SOLID_ALPHA: u32 = 128;
const VISIBLE_ALPHA: u32 = 32;
const AREA_TIMES: u32 = 4;
const TYPICAL_NAME: i32 = 27;
const MEDIUM: i32 = 96;
const SEEDS: [u64; 3] = [0, 0x9e37_79b9_7f4a_7c15, 0x2545_f491_4f6c_dd1d];
const MIXED_WIDTHS: [i32; 4] = [2, 5, 3, 4];
const GLYPH_WIDTHS: [i32; 2] = [2, 8];
const LETTER_BUDGET: u64 = 200_000_000;
const LETTER_OVERHEAD: u64 = 20;

fn seeds(asset: &NameplateAsset) -> &'static [u64] {
    let random = asset.layers.iter().any(|l| {
        matches!(l.content, LayerContent::Emitter(_)) || l.mirror.is_some_and(|m| m.seeded)
    });
    if random {
        &SEEDS
    } else {
        &SEEDS[..1]
    }
}

fn signals(asset: &NameplateAsset) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for reaction in &asset.animations.reactions {
        if !out.contains(&reaction.signal) {
            out.push(reaction.signal);
        }
    }
    out
}

fn single(asset: &NameplateAsset, clip: ClipRef, source: Source, seed: u64) -> Scenario {
    let length = clip_length(asset, clip);
    Scenario {
        source,
        events: vec![Event {
            clip,
            start: 0,
            length,
        }],
        ..Scenario::idle(u32::try_from(length).unwrap_or(1), seed)
    }
}

pub fn contrast_scenarios(asset: &NameplateAsset) -> Vec<Scenario> {
    let looped = u32::from(asset.loop_frames.max(1));
    let mut out = Vec::new();
    for seed in seeds(asset) {
        out.push(Scenario::idle(looped, *seed));
        for signal in signals(asset) {
            out.push(Scenario::held(
                Source::Reaction(signal),
                vec![signal],
                looped,
                *seed,
            ));
        }
        for index in 0..asset.animations.one_shots.len() {
            let id = u16::try_from(index).unwrap_or(u16::MAX);
            out.push(single(
                asset,
                ClipRef::OneShot(id),
                Source::OneShot(id),
                *seed,
            ));
        }
        for (s, schedule) in asset.animations.schedules.iter().enumerate() {
            let id = u16::try_from(s).unwrap_or(u16::MAX);
            for event in 0..schedule.events.len() {
                let clip = ClipRef::Schedule {
                    schedule: id,
                    event: u16::try_from(event).unwrap_or(u16::MAX),
                };
                out.push(single(asset, clip, Source::Schedule(id), *seed));
            }
        }
    }
    out
}

fn name_backdrop(stage: &Stage<'_>, scenario: &Scenario, out: &mut Backdrop) {
    let mut canvas: Option<Canvas> = None;
    for frame in 0..scenario.frames {
        let shot = stage.shot(scenario, frame);
        let mut list = stage.draws(&shot, 0);
        list.retain(|d| d.depth != Depth::OverText);
        let (px, py) = stage.plate(&shot);
        let area = stage.layout.name.offset(px, py);
        let canvas = match &mut canvas {
            Some(c) if c.area() == area => c,
            _ => canvas.insert(Canvas::new(area)),
        };
        stage.render(&shot, &list, canvas);
        let width = canvas.area().width().max(0) as usize;
        out.sample(canvas.pixels(), width, (area.x0, area.y0), area);
    }
}

pub fn sample_backdrop(asset: &NameplateAsset, variant: u16, block: &BlockInput) -> Backdrop {
    let mut all = Backdrop::new();
    let Some(stage) = Stage::new(asset, variant, block) else {
        return all;
    };
    for scenario in contrast_scenarios(asset) {
        name_backdrop(&stage, &scenario, &mut all);
    }
    all
}

fn source_name(asset: &NameplateAsset, source: Source) -> String {
    let a = &asset.animations;
    match source {
        Source::Loop | Source::Everything => a.looping.name.clone(),
        Source::Reaction(signal) => a
            .reactions
            .iter()
            .find(|r| r.signal == signal)
            .map_or_else(String::new, |r| r.clip.name.clone()),
        Source::OneShot(i) => a
            .one_shots
            .get(usize::from(i))
            .map_or_else(String::new, |o| o.clip.name.clone()),
        Source::Schedule(i) => a
            .schedules
            .get(usize::from(i))
            .map_or_else(String::new, |s| s.id.clone()),
    }
}

fn judged(asset: &NameplateAsset, variant: &Variant, backdrop: &[u32]) -> u32 {
    let mut palette = PaletteFrame::default();
    base_frame(variant, &mut palette);
    let resolve = |c| resolve_ref(c, &palette, &asset.fixed_colors, &variant.stops);
    let style = &variant.text;
    match style.color {
        Paint::Auto => {
            let candidates: Vec<u32> = style.candidates.iter().map(|c| resolve(*c)).collect();
            let candidates = if candidates.is_empty() {
                DEFAULT_CANDIDATES.to_vec()
            } else {
                candidates
            };
            0xff00_0000 | (choose(&candidates, backdrop).0 & 0x00ff_ffff)
        }
        Paint::Color(c) => 0xff00_0000 | (resolve(c) & 0x00ff_ffff),
        Paint::None => WHITE,
    }
}

fn hinted(asset: &NameplateAsset, variant: &Variant, backdrop: &Backdrop) -> Backdrop {
    let mut palette = PaletteFrame::default();
    base_frame(variant, &mut palette);
    let mut out = backdrop.clone();
    for hint in &variant.text.backdrop_hint {
        out.add_opaque(resolve_ref(
            *hint,
            &palette,
            &asset.fixed_colors,
            &variant.stops,
        ));
    }
    out
}

fn text_path(asset: &NameplateAsset, variant: &Variant) -> String {
    if variant.text == asset.text {
        path(MANIFEST, "text")
    } else {
        path(MANIFEST, &format!("variants.{}.text", variant.key))
    }
}

pub fn readability(asset: &NameplateAsset, report: &mut Report) {
    let scenarios = contrast_scenarios(asset);
    for variant in distinct_text_variants(asset) {
        let chosen = &asset.variants[usize::from(variant)];
        let mut worst: Option<(f64, String, i32, u8)> = None;
        for input in reference_inputs() {
            let Some(stage) = Stage::new(asset, variant, &input) else {
                continue;
            };
            let mut all = Backdrop::new();
            let mut parts = Vec::new();
            for scenario in &scenarios {
                let mut part = Backdrop::new();
                name_backdrop(&stage, scenario, &mut part);
                all.merge(&part);
                parts.push((scenario.source, part));
            }
            if all.is_empty() {
                continue;
            }
            let all = hinted(asset, chosen, &all);
            let colors = all.colors();
            let color = judged(asset, chosen, &colors);
            let ratio = min_contrast(color, &colors);
            if ratio >= CONTRAST_LARGE {
                continue;
            }
            let mut culprit = (f64::MAX, Source::Loop);
            for (source, part) in &parts {
                if part.is_empty() {
                    continue;
                }
                let r = min_contrast(color, &part.colors());
                if r < culprit.0 {
                    culprit = (r, *source);
                }
            }
            if worst.as_ref().is_none_or(|w| ratio < w.0) {
                worst = Some((
                    ratio,
                    source_name(asset, culprit.1),
                    stage.placement.block.name_width,
                    stage.placement.block.line_count,
                ));
            }
        }
        if let Some((ratio, source, width, lines)) = worst {
            report.push(
                Issue::warning(IssueCode::Readability, text_path(asset, chosen))
                    .value("ratio", libm::floor(ratio * 100.0) / 100.0)
                    .value("limit", CONTRAST_LARGE)
                    .value("name_width", f64::from(width))
                    .value("lines", f64::from(lines))
                    .name("variant", &chosen.key)
                    .name("animation", &source),
            );
        }
    }
}

fn width_patterns() -> Vec<Vec<i32>> {
    let [narrowest, widest] = GLYPH_WIDTHS;
    let mut patterns: Vec<Vec<i32>> = (narrowest..=widest).map(|width| vec![width]).collect();
    for turn in 0..MIXED_WIDTHS.len() {
        let mut turned = MIXED_WIDTHS.to_vec();
        turned.rotate_left(turn);
        patterns.push(turned);
    }
    patterns
}

struct ReferenceName {
    glyphs: Vec<f32>,
    width: i32,
    pattern: usize,
    count: usize,
}

fn reference_names() -> (Vec<ReferenceName>, usize) {
    let patterns = width_patterns();
    let mixed = patterns.len() - MIXED_WIDTHS.len();
    let names = patterns
        .iter()
        .enumerate()
        .flat_map(|(pattern, widths)| {
            (1..=MAX_LETTERS).map(move |count| {
                let (glyphs, width) = reference_glyphs(count, widths);
                ReferenceName {
                    glyphs,
                    width,
                    pattern,
                    count,
                }
            })
        })
        .collect();
    (names, mixed)
}

fn reference_glyphs(count: usize, widths: &[i32]) -> (Vec<f32>, i32) {
    let mut glyphs = vec![0.0; count * GLYPH_FLOATS];
    let mut x = 0;
    for (glyph, width) in glyphs
        .chunks_exact_mut(GLYPH_FLOATS)
        .zip(widths.iter().cycle())
    {
        glyph[GLYPH_X] = x as f32;
        glyph[GLYPH_WIDTH] = *width as f32;
        x += width;
    }
    (glyphs, x)
}

fn letter_jobs(
    names: &[ReferenceName],
    mixed: usize,
    cases: &[SignalCase],
    half: u32,
    program_cost: u32,
) -> Vec<(usize, usize)> {
    let default = names
        .iter()
        .position(|n| n.pattern == mixed && n.count == MAX_LETTERS)
        .unwrap_or(0);
    let weight = u64::from(half) * (u64::from(program_cost) + LETTER_OVERHEAD);
    let cost = |name: usize| names[name].count as u64 * weight;
    let mut jobs: Vec<(usize, usize)> = (0..names.len()).map(|name| (0, name)).collect();
    let mut spent: u64 = (0..names.len()).map(cost).sum();
    let crossed: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(n, name)| *n != default && (name.pattern == mixed || name.count == MAX_LETTERS))
        .map(|(n, _)| n)
        .collect();
    let more = (1..cases.len()).map(|case| (case, default)).chain(
        cases
            .iter()
            .enumerate()
            .filter(|(_, case)| case.changed == 1)
            .flat_map(|(case, _)| crossed.iter().map(move |name| (case, *name))),
    );
    for job in more {
        let next = spent + cost(job.1);
        if next > LETTER_BUDGET {
            break;
        }
        spent = next;
        jobs.push(job);
    }
    jobs
}

fn frames_in_place(
    stage: &Stage<'_>,
    program: u32,
    glyphs: &[f32],
    signals: &SignalSample,
    frames: u32,
    limit: u32,
) -> u32 {
    let fps = f32::from(stage.asset.fps.max(1));
    let mut posed = vec![LetterPose::STILL; glyphs.len() / GLYPH_FLOATS];
    let mut in_place = 0;
    for frame in 0..frames {
        if in_place >= limit {
            break;
        }
        let ran = stage.eval.letters(
            program,
            i64::from(frame),
            TextMode::Owned,
            glyphs,
            &signals.after(frame as f32 / fps),
            &mut posed,
        );
        if !ran || readable_in_place(&posed) {
            in_place += 1;
        }
    }
    in_place
}

pub fn letters_readable(asset: &NameplateAsset, report: &mut Report) {
    let frames = u32::from(asset.loop_frames.max(1));
    let half = frames.div_ceil(2);
    let (names, mixed) = reference_names();
    for variant in distinct_text_variants(asset) {
        let chosen = &asset.variants[usize::from(variant)];
        let style = &chosen.text;
        let Some(program) = style.motion.filter(|_| style.mode == TextMode::Owned) else {
            continue;
        };
        let mut stages: BTreeMap<i32, Option<Stage<'_>>> = BTreeMap::new();
        let Some(cases) = names.iter().find_map(|name| {
            let stage = stages
                .entry(name.width)
                .or_insert_with(|| Stage::new(asset, variant, &one_line(name.width)))
                .as_ref()?;
            let (values, times) = stage.eval.signals_read(program);
            Some(signal_cases(&values, &times))
        }) else {
            continue;
        };
        let cost = asset
            .molang
            .programs
            .get(program as usize)
            .map_or(0, |p| p.cost);
        let mut worst: Option<(u32, i32)> = None;
        for (case, name) in letter_jobs(&names, mixed, &cases, half, cost) {
            let reference = &names[name];
            let Some(stage) = stages
                .entry(reference.width)
                .or_insert_with(|| Stage::new(asset, variant, &one_line(reference.width)))
                .as_ref()
            else {
                continue;
            };
            let limit = worst.map_or(half, |w| w.0.min(half));
            let in_place = frames_in_place(
                stage,
                program,
                &reference.glyphs,
                &cases[case].sample,
                frames,
                limit,
            );
            if in_place < limit {
                worst = Some((in_place, reference.width));
            }
            if worst.is_some_and(|w| w.0 == 0) {
                break;
            }
        }
        if let Some((in_place, width)) = worst {
            report.push(
                Issue::error(
                    IssueCode::LetterReadable,
                    format!("{}.motion", text_path(asset, chosen)),
                )
                .value("frames", f64::from(in_place))
                .value("of", f64::from(frames))
                .value("name_width", f64::from(width))
                .name("variant", &chosen.key),
            );
        }
    }
}

fn distinct_text_variants(asset: &NameplateAsset) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for (index, variant) in asset.variants.iter().enumerate() {
        let same = out.iter().any(|seen| {
            let seen = &asset.variants[usize::from(*seen)];
            seen.tones == variant.tones
                && seen.stops == variant.stops
                && seen.slots == variant.slots
                && seen.flag == variant.flag
                && seen.params == variant.params
                && seen.text == variant.text
        });
        if !same {
            out.push(u16::try_from(index).unwrap_or(u16::MAX));
        }
    }
    out
}

fn target_name(asset: &NameplateAsset, target: Target) -> String {
    match target {
        Target::Layer(l) => asset
            .layers
            .get(usize::from(l))
            .map_or_else(String::new, |l| l.id.clone()),
        Target::Group(g) => asset
            .groups
            .get(usize::from(g))
            .map_or_else(String::new, |g| g.id.clone()),
        Target::Badge => String::from("$badge"),
        Target::Plate => String::from("$plate"),
    }
}

fn tracks_path(clip: &str, target: &str) -> String {
    format!("{}.tracks.{target}", animation_path(clip))
}

pub fn badge_tracks(asset: &NameplateAsset, report: &mut Report) {
    let Some(input) = reference_inputs().into_iter().next() else {
        return;
    };
    let Some(stage) = Stage::new(asset, 0, &input) else {
        return;
    };
    let a = &asset.animations;
    let looped = u32::from(asset.loop_frames.max(1));
    let fps = u32::from(asset.fps.max(1));
    let badged = |clip: &Clip| {
        clip.targets.iter().any(|t| {
            t.target == Target::Badge
                && (t.channels.opacity.is_some() || t.channels.visible.is_some())
        })
    };
    let mut spans: Vec<(String, Scenario, u32)> = Vec::new();
    if badged(&a.looping) {
        spans.push((a.looping.name.clone(), Scenario::idle(looped, 0), 0));
    }
    for reaction in &a.reactions {
        if badged(&reaction.clip) {
            spans.push((
                reaction.clip.name.clone(),
                Scenario::held(
                    Source::Reaction(reaction.signal),
                    vec![reaction.signal],
                    looped,
                    0,
                ),
                0,
            ));
        }
    }
    for (index, shot) in a.one_shots.iter().enumerate() {
        if badged(&shot.clip) {
            let id = u16::try_from(index).unwrap_or(u16::MAX);
            spans.push((
                shot.clip.name.clone(),
                single(asset, ClipRef::OneShot(id), Source::OneShot(id), 0),
                fps,
            ));
        }
    }
    for (s, schedule) in a.schedules.iter().enumerate() {
        for (e, event) in schedule.events.iter().enumerate() {
            if badged(&event.clip) {
                let clip = ClipRef::Schedule {
                    schedule: u16::try_from(s).unwrap_or(u16::MAX),
                    event: u16::try_from(e).unwrap_or(u16::MAX),
                };
                spans.push((
                    event.clip.name.clone(),
                    single(asset, clip, Source::Schedule(clip_schedule(clip)), 0),
                    fps,
                ));
            }
        }
    }
    for (name, scenario, brief) in spans {
        let mut hidden = 0u32;
        let mut faded = 0u32;
        let mut lowest = OPAQUE;
        for frame in 0..scenario.frames {
            let shot = stage.shot(&scenario, frame);
            let badge = shot.pose.badge;
            if !badge.visible || badge.opacity == 0 {
                hidden += 1;
            } else if badge.opacity < FADED_BELOW {
                faded += 1;
                lowest = lowest.min(badge.opacity);
            }
        }
        let at = tracks_path(&name, "$badge");
        if hidden > brief {
            report.push(
                Issue::warning(IssueCode::BadgeHidden, at.clone())
                    .value("frames", f64::from(hidden))
                    .value("of", f64::from(scenario.frames)),
            );
        }
        if faded > brief {
            report.push(
                Issue::warning(IssueCode::BadgeFaded, at)
                    .value("opacity", f64::from(lowest))
                    .value("frames", f64::from(faded))
                    .value("of", f64::from(scenario.frames))
                    .value("limit", f64::from(FADED_BELOW)),
            );
        }
    }
}

fn clip_schedule(clip: ClipRef) -> u16 {
    match clip {
        ClipRef::Schedule { schedule, .. } => schedule,
        _ => 0,
    }
}

fn single_draw(
    stage: &Stage<'_>,
    draw: &Draw,
    palette: &PaletteFrame,
    plate: (i32, i32),
    canvas: &mut Canvas,
) {
    let colors = Colors::new(stage.asset, stage.variant, palette);
    canvas.clear();
    composite(slice::from_ref(draw), &colors, plate, canvas);
}

fn held_scenarios(asset: &NameplateAsset) -> Vec<Scenario> {
    let looped = u32::from(asset.loop_frames.max(1));
    let mut out = vec![Scenario::idle(looped, 0)];
    for signal in signals(asset) {
        out.push(Scenario::held(
            Source::Reaction(signal),
            vec![signal],
            looped,
            0,
        ));
    }
    out
}

pub fn badge_covered(asset: &NameplateAsset, report: &mut Report) {
    let placed = asset
        .variants
        .iter()
        .any(|v| matches!(v.badge.placement, BadgePlacement::Placed(_)));
    let inputs: Vec<BlockInput> = reference_inputs()
        .into_iter()
        .filter(|i| i.badge || placed)
        .collect();
    let frames = sampled_frames(asset.loop_frames, SAMPLED_FRAMES);
    let area = 9 * 8;
    let mut most: Vec<(u32, Source)> = vec![(0, Source::Loop); asset.layers.len()];
    for variant in distinct_variants(asset) {
        for input in &inputs {
            let Some(stage) = Stage::new(asset, variant, input) else {
                continue;
            };
            let Some(badge) = stage.layout.badge else {
                continue;
            };
            for scenario in held_scenarios(asset) {
                for frame in &frames {
                    let shot = stage.shot(&scenario, *frame);
                    let pose = shot.pose.badge;
                    if !pose.visible || pose.opacity == 0 {
                        continue;
                    }
                    let plate = stage.plate(&shot);
                    let rect = badge
                        .offset(i32::from(pose.dx), i32::from(pose.dy))
                        .offset(plate.0, plate.1);
                    let mut canvas = Canvas::new(rect);
                    for draw in stage.draws(&shot, 0) {
                        if draw.depth != Depth::OverText {
                            continue;
                        }
                        let Some(layer) = resolved(asset, stage.variant, usize::from(draw.layer))
                        else {
                            continue;
                        };
                        single_draw(&stage, &draw, &shot.pose.palette, plate, &mut canvas);
                        let count = canvas
                            .pixels()
                            .iter()
                            .filter(|p| *p >> 24 >= SOLID_ALPHA)
                            .count() as u32;
                        if count > most[layer].0 {
                            most[layer] = (count, scenario.source);
                        }
                    }
                }
            }
        }
    }
    for (layer, (count, source)) in most.iter().enumerate() {
        if *count * 100 >= area * COVERED_PERCENT && *count > 0 {
            let mut issue =
                Issue::warning(IssueCode::BadgeCovered, layer_path(&asset.layers[layer].id))
                    .value("px", f64::from(*count))
                    .value("percent", f64::from(*count * 100 / area))
                    .value("limit", f64::from(COVERED_PERCENT));
            if *source != Source::Loop {
                issue = issue.name("animation", &source_name(asset, *source));
            }
            report.push(issue);
        }
    }
}

fn idle_canvases(stage: &Stage<'_>, frames: &[u32], mut visit: impl FnMut(&Canvas)) {
    let scenario = Scenario::idle(u32::from(stage.asset.loop_frames.max(1)), 0);
    let mut canvas = Canvas::new(stage.region());
    for frame in frames {
        let shot = stage.shot(&scenario, *frame);
        let list = stage.draws(&shot, 0);
        stage.render(&shot, &list, &mut canvas);
        visit(&canvas);
    }
}

pub fn art_coverage(asset: &NameplateAsset, report: &mut Report) {
    let frames = sampled_frames(asset.loop_frames, SAMPLED_FRAMES);
    let variants = distinct_variants(asset);
    let typical = one_line(TYPICAL_NAME);
    let mut most_area = 0u32;
    let mut plain = 0u32;
    for variant in &variants {
        let Some(stage) = Stage::new(asset, *variant, &typical) else {
            continue;
        };
        let bg = stage.placement.block.background;
        plain = (bg.width() * bg.height()).max(1) as u32;
        idle_canvases(&stage, &frames, |canvas| {
            let count = canvas
                .pixels()
                .iter()
                .filter(|p| *p >> 24 >= VISIBLE_ALPHA)
                .count() as u32;
            most_area = most_area.max(count);
        });
    }
    if plain > 0 && most_area >= plain * AREA_TIMES {
        report.push(
            Issue::warning(IssueCode::ArtCoverage, MANIFEST)
                .value("times", f64::from(most_area * 10 / plain) / 10.0)
                .value("px", f64::from(most_area))
                .value("limit", f64::from(AREA_TIMES)),
        );
        return;
    }
    let mut worst = (0u32, 0u32);
    for variant in &variants {
        for input in reference_inputs() {
            let Some(stage) = Stage::new(asset, *variant, &input) else {
                continue;
            };
            let bg = stage.placement.block.background;
            let region = stage.region();
            let ring = (region.width() * region.height()
                - bg.intersect(region).width() * bg.intersect(region).height())
            .max(1) as u32;
            idle_canvases(&stage, &frames, |canvas| {
                let area = canvas.area();
                let width = area.width().max(1) as usize;
                let count = canvas
                    .pixels()
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| {
                        let x = area.x0 + (i % width) as i32;
                        let y = area.y0 + (i / width) as i32;
                        **p >> 24 != 0 && !bg.contains(x, y)
                    })
                    .count() as u32;
                let percent = count * 100 / ring;
                if percent > worst.0 {
                    worst = (percent, count);
                }
            });
        }
    }
    if worst.0 >= COVERAGE_PERCENT {
        report.push(
            Issue::warning(IssueCode::ArtCoverage, MANIFEST)
                .value("percent", f64::from(worst.0))
                .value("px", f64::from(worst.1))
                .value("limit", f64::from(COVERAGE_PERCENT)),
        );
    }
}

type Shapes = Vec<Vec<(usize, Rect)>>;

fn shapes(stage: &Stage<'_>, frames: &[u32]) -> (Shapes, Option<(i32, i32)>) {
    let asset = stage.asset;
    let scenario = Scenario::idle(u32::from(asset.loop_frames.max(1)), 0);
    let region = stage.region();
    let mut all = Vec::new();
    let mut extent: Option<(i32, i32)> = None;
    for frame in frames {
        let shot = stage.shot(&scenario, *frame);
        let mut boxes: Vec<(usize, Rect)> = Vec::new();
        for draw in stage.draws(&shot, 0) {
            let rect = match &draw.source {
                DrawSource::Image(image) => draw.place_rect(image.bounds),
                DrawSource::Pixels { area, .. } => draw.place_rect(*area),
            };
            let rect = rect.intersect(region);
            if rect.is_empty() {
                continue;
            }
            let slot = usize::from(draw.layer);
            match boxes.iter_mut().find(|(s, _)| *s == slot) {
                Some((_, b)) => *b = b.union(rect),
                None => boxes.push((slot, rect)),
            }
            extent = Some(match extent {
                Some((top, bottom)) => (top.min(rect.y0), bottom.max(rect.y1)),
                None => (rect.y0, rect.y1),
            });
        }
        all.push(boxes);
    }
    (all, extent)
}

pub fn fixed_height(asset: &NameplateAsset, report: &mut Report) {
    let frames = sampled_frames(asset.loop_frames, SAMPLED_FRAMES);
    let inputs: Vec<BlockInput> = reference_inputs()
        .into_iter()
        .filter(|i| i.name_width == MEDIUM && !i.badge)
        .collect();
    let Some(one) = inputs.iter().find(|i| i.line_count == 1) else {
        return;
    };
    for variant in distinct_variants(asset) {
        let Some(stage) = Stage::new(asset, variant, one) else {
            continue;
        };
        let (one_shapes, extent) = shapes(&stage, &frames);
        let bg = stage.placement.block.background;
        let spans_one = extent.is_some_and(|(top, bottom)| top <= bg.y0 && bottom >= bg.y1);
        let mut taller = Vec::new();
        for input in inputs.iter().filter(|i| i.line_count > 1) {
            let Some(stage) = Stage::new(asset, variant, input) else {
                continue;
            };
            let bg = stage.placement.block.background;
            let (found, extent) = shapes(&stage, &frames);
            let spans = extent.is_some_and(|(top, bottom)| top <= bg.y0 && bottom >= bg.y1);
            taller.push((input.line_count, spans, found));
        }
        if spans_one {
            if let Some((lines, _, _)) = taller.iter().find(|(_, spans, _)| !spans) {
                report.push(
                    Issue::warning(IssueCode::FixedHeight, MANIFEST)
                        .value("lines", f64::from(*lines))
                        .name("variant", &stage.variant.key),
                );
                return;
            }
        }
        let any = one_shapes.iter().any(|f| !f.is_empty());
        if any && !taller.is_empty() && taller.iter().all(|(_, _, s)| *s == one_shapes) {
            let lines = taller.iter().map(|(l, _, _)| *l).max().unwrap_or(1);
            report.push(
                Issue::warning(IssueCode::FixedHeight, MANIFEST)
                    .value("lines", f64::from(lines))
                    .name("variant", &stage.variant.key),
            );
            return;
        }
    }
}

pub fn translucent_overlap(asset: &NameplateAsset, report: &mut Report) {
    let frames = sampled_frames(asset.loop_frames, SAMPLED_FRAMES);
    let inputs: Vec<BlockInput> = reference_inputs()
        .into_iter()
        .filter(|i| i.name_width == MEDIUM && !i.badge && i.line_count <= 2)
        .collect();
    let scenario = Scenario::idle(u32::from(asset.loop_frames.max(1)), 0);
    let mut most = vec![0u32; asset.layers.len()];
    for variant in distinct_variants(asset) {
        for input in &inputs {
            let Some(stage) = Stage::new(asset, variant, input) else {
                continue;
            };
            let region = stage.region();
            let mut scratch = Canvas::new(region);
            let mut covered = vec![false; scratch.pixels().len()];
            for frame in &frames {
                let shot = stage.shot(&scenario, *frame);
                let mut list = stage.draws(&shot, 0);
                list.sort_by_key(|d| d.depth.band());
                let plate = stage.plate(&shot);
                covered.fill(false);
                for draw in &list {
                    let Some(layer) = resolved(asset, stage.variant, usize::from(draw.layer))
                    else {
                        continue;
                    };
                    single_draw(&stage, draw, &shot.pose.palette, plate, &mut scratch);
                    let mut count = 0u32;
                    for (i, p) in scratch.pixels().iter().enumerate() {
                        let alpha = *p >> 24;
                        if alpha == 0 {
                            continue;
                        }
                        if alpha < 255 && covered[i] {
                            count += 1;
                        }
                        covered[i] = true;
                    }
                    most[layer] = most[layer].max(count);
                }
            }
        }
    }
    for (layer, count) in most.iter().enumerate() {
        if *count > 0 {
            report.push(
                Issue::warning(
                    IssueCode::TranslucentOverlap,
                    layer_path(&asset.layers[layer].id),
                )
                .value("px", f64::from(*count)),
            );
        }
    }
}

pub fn position_clamped(asset: &NameplateAsset, measured: &Measured, report: &mut Report) {
    let limit = NAMEPLATE_OFFSET as i32;
    for (reach, place) in &measured.reaches {
        let target = target_name(asset, reach.target);
        report.push(
            Issue::warning(
                IssueCode::PositionClamped,
                format!("{}.position", tracks_path(&reach.clip, &target)),
            )
            .value("px", f64::from(reach.raw.abs()))
            .value("limit", f64::from(limit))
            .value("name_width", f64::from(place.name_width))
            .value("lines", f64::from(place.lines))
            .name("animation", &reach.clip)
            .name("target", &target)
            .name("axis", if reach.x { "x" } else { "y" }),
        );
    }
}

pub fn plate_background(asset: &NameplateAsset, report: &mut Report) {
    let Some((clip, _)) = clips(asset)
        .into_iter()
        .find(|(c, _)| c.targets.iter().any(|t| t.target == Target::Plate))
    else {
        return;
    };
    let left: Vec<&str> = asset
        .variants
        .iter()
        .filter(|v| !v.owns_background)
        .map(|v| v.key.as_str())
        .collect();
    if left.is_empty() {
        return;
    }
    report.push(
        Issue::warning(
            IssueCode::PlateBackground,
            tracks_path(&clip.name, "$plate"),
        )
        .value("variants", left.len() as f64)
        .name("variants", &left.join(",")),
    );
}
