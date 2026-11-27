use std::iter;

use crate::issues::{path, Issue, IssueCode, Report};
use crate::limits::{
    FLASHES_PER_SECOND, FLASH_AREA_PIXELS, FLASH_LUMINANCE_CHANGE, NAMEPLATE_APPEAR_SECONDS_MIN,
    NAMEPLATE_FLASH_FRAMES, NAMEPLATE_FLASH_SCHEDULE_REPEATS, NAMEPLATE_FLASH_WORK,
};

use super::super::anim::ClipRef;
use super::super::geom::{BlockInput, Rect};
use super::super::layout::reference_inputs;
use super::super::raster::{Canvas, Draw, DrawSource};
use super::super::types::{LayerContent, NameplateAsset, PaletteFrame, Phase, Trigger, CHANCE_ONE};
use super::budget::Measured;
use super::cover::{lanes, sample, Cover, Lane};
use super::sim::{
    breakpoint_inputs, clip_length, distinct_variants, read_signals, resolved, Event, Scenario,
    Source, Stage,
};
use super::{animation_path, layer_path, MANIFEST};

pub const SCALE: i32 = 100_000;
pub const DARK_LIMIT: i32 = SCALE * 8 / 10;
pub const SCENARIO_FRAMES: u64 = 32 * NAMEPLATE_FLASH_FRAMES as u64;
pub const SCENARIO_ROWS: usize = 256;
const FRAME_WORK: u64 = 800;
const CPU_WORK: u64 = 2;
const DRAW_WORK: u64 = 130;
const PIXEL_WORK: u64 = 3;
const RUN_WORK: u64 = 8;
const MATERIAL_WORK: u64 = 2;

const WEIGHT_RED: i64 = 2126;
const WEIGHT_GREEN: i64 = 7152;
const WEIGHT_BLUE: i64 = 722;
const WEIGHT_UNIT: i64 = 10_000;

fn linear_table() -> [i64; 256] {
    let mut table = [0; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let c = i as f64 / 255.0;
        let linear = if c <= 0.04045 {
            c / 12.92
        } else {
            libm::pow((c + 0.055) / 1.055, 2.4)
        };
        *slot = libm::floor(linear * f64::from(SCALE) + 0.5) as i64;
    }
    table
}

pub struct Luminance {
    table: [i64; 256],
}

impl Luminance {
    pub fn new() -> Luminance {
        Luminance {
            table: linear_table(),
        }
    }

    pub fn of(&self, rgb: u32) -> i32 {
        let r = self.table[((rgb >> 16) & 0xff) as usize];
        let g = self.table[((rgb >> 8) & 0xff) as usize];
        let b = self.table[(rgb & 0xff) as usize];
        ((WEIGHT_RED * r + WEIGHT_GREEN * g + WEIGHT_BLUE * b + WEIGHT_UNIT / 2) / WEIGHT_UNIT)
            as i32
    }

    pub fn over(&self, argb: u32, backdrop: u32) -> i32 {
        let a = argb >> 24;
        if a == 255 {
            return self.of(argb);
        }
        let mut rgb = 0;
        for shift in [16, 8, 0] {
            let c = (argb >> shift) & 0xff;
            let b = (backdrop >> shift) & 0xff;
            rgb |= ((c * a + b * (255 - a) + 127) / 255) << shift;
        }
        self.of(rgb)
    }
}

impl Default for Luminance {
    fn default() -> Luminance {
        Luminance::new()
    }
}

struct Swing {
    pixel: usize,
    direction: i8,
    changed_at: u32,
    seen: bool,
    armed: [i8; 2],
    recent: [Vec<u32>; 2],
    covered: i64,
}

pub struct Tracker {
    fps: u32,
    last: Vec<i32>,
    anchor: Vec<i32>,
    active: Vec<u32>,
    swings: Vec<Swing>,
    diff: Vec<i64>,
    most_times: u32,
}

impl Tracker {
    pub fn new(pixels: usize, fps: u32, frames: u32) -> Tracker {
        Tracker {
            fps: fps.max(1),
            last: vec![0; pixels],
            anchor: vec![0; pixels],
            active: vec![u32::MAX; pixels],
            swings: Vec::new(),
            diff: vec![0; frames as usize + 2],
            most_times: 0,
        }
    }

    pub fn start(&mut self, pixel: usize, value: i32) {
        self.last[pixel] = value;
        self.anchor[pixel] = value;
    }

    fn swing(&mut self, pixel: usize) -> usize {
        let slot = self.active[pixel];
        if slot != u32::MAX {
            return slot as usize;
        }
        self.active[pixel] = u32::try_from(self.swings.len()).unwrap_or(u32::MAX);
        self.swings.push(Swing {
            pixel,
            direction: 0,
            changed_at: 0,
            seen: false,
            armed: [0; 2],
            recent: [Vec::new(), Vec::new()],
            covered: -1,
        });
        self.swings.len() - 1
    }

    pub fn step(&mut self, pixel: usize, value: i32, frame: u32) {
        let before = self.last[pixel];
        if value == before {
            return;
        }
        let slot = self.swing(pixel);
        let toward: i8 = if value > before { 1 } else { -1 };
        let run = self.swings[slot].direction;
        if run != 0 && run != toward {
            self.close(slot);
        }
        let swing = &mut self.swings[slot];
        swing.direction = toward;
        swing.changed_at = frame;
        self.last[pixel] = value;
    }

    fn close(&mut self, slot: usize) {
        let pixel = self.swings[slot].pixel;
        let run = self.swings[slot].direction;
        let before = self.last[pixel];
        let from = self.anchor[pixel];
        let swing = (before - from).abs();
        let threshold = (FLASH_LUMINANCE_CHANGE * f64::from(SCALE)) as i32;
        if swing >= threshold && before.min(from) < DARK_LIMIT {
            let at = self.swings[slot].changed_at;
            self.change(slot, run, at);
        }
        self.anchor[pixel] = before;
    }

    pub fn finish(&mut self) {
        for slot in 0..self.swings.len() {
            if self.swings[slot].direction != 0 {
                self.close(slot);
                self.swings[slot].direction = 0;
            }
        }
    }

    fn change(&mut self, slot: usize, toward: i8, time: u32) {
        let swing = &mut self.swings[slot];
        let pairings = if swing.seen { 2 } else { 1 };
        swing.seen = true;
        for pairing in 0..pairings {
            let pending = self.swings[slot].armed[pairing];
            if pending != 0 && pending == -toward {
                self.swings[slot].armed[pairing] = 0;
                self.flash(slot, pairing, time);
            } else {
                self.swings[slot].armed[pairing] = toward;
            }
        }
    }

    fn flash(&mut self, slot: usize, pairing: usize, time: u32) {
        let fps = self.fps;
        let swing = &mut self.swings[slot];
        let recent = &mut swing.recent[pairing];
        recent.retain(|t| *t + fps > time);
        recent.push(time);
        let count = recent.len() as u32;
        self.most_times = self.most_times.max(count);
        let needed = (FLASHES_PER_SECOND + 1) as usize;
        if recent.len() < needed {
            return;
        }
        let first = i64::from(recent[recent.len() - needed]);
        let from = (i64::from(time) - i64::from(fps) + 1)
            .max(0)
            .max(swing.covered + 1);
        if from <= first {
            let last = self.diff.len() as i64 - 1;
            self.diff[from.min(last) as usize] += 1;
            self.diff[(first + 1).min(last) as usize] -= 1;
        }
        swing.covered = swing.covered.max(first);
    }

    pub fn most_pixels(&self) -> u32 {
        let mut running = 0;
        let mut most = 0;
        for d in &self.diff {
            running += d;
            most = most.max(running);
        }
        u32::try_from(most).unwrap_or(u32::MAX)
    }

    pub fn most_times(&self) -> u32 {
        self.most_times
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub pixels: u32,
    pub times: u32,
}

impl Outcome {
    pub fn fails(&self) -> bool {
        self.pixels > FLASH_AREA_PIXELS
    }

    fn worse(self, other: Outcome) -> Outcome {
        if (other.pixels, other.times) > (self.pixels, self.times) {
            other
        } else {
            self
        }
    }
}

fn draw_area(draw: &Draw, plate: (i32, i32)) -> Rect {
    let bounds = match &draw.source {
        DrawSource::Image(image) => image.bounds,
        DrawSource::Pixels { area, pixels } => {
            let width = area.width().max(1) as usize;
            let rows = i32::try_from(pixels.len().div_ceil(width)).unwrap_or(i32::MAX);
            Rect::new(area.x0, area.y0, area.x1, area.y0.saturating_add(rows))
        }
    };
    draw.place_rect(bounds).offset(plate.0, plate.1)
}

fn changed(before: &[Draw], after: &[Draw], plate: (i32, i32)) -> Rect {
    let head = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    let tail = before[head..]
        .iter()
        .rev()
        .zip(after[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    before[head..before.len() - tail]
        .iter()
        .chain(&after[head..after.len() - tail])
        .fold(Rect::EMPTY, |all, draw| all.union(draw_area(draw, plate)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Meter {
    spent: u64,
    limit: u64,
    frame: u64,
}

impl Meter {
    fn new(limit: u64, frame: u64) -> Meter {
        Meter {
            spent: 0,
            limit,
            frame,
        }
    }

    fn spare(&self) -> bool {
        self.spent < self.limit
    }

    fn spend(&mut self, work: u64) {
        self.spent = self.spent.saturating_add(work);
    }
}

fn pixels(area: Rect) -> u64 {
    if area.is_empty() {
        0
    } else {
        u64::from(area.width().unsigned_abs()) * u64::from(area.height().unsigned_abs())
    }
}

fn material_costs(stage: &Stage<'_>) -> Vec<u64> {
    let asset = stage.asset;
    (0..asset.layers.len())
        .map(|slot| {
            let layer = resolved(asset, stage.variant, slot);
            match layer.map(|l| &asset.layers[l].content) {
                Some(LayerContent::Material(m)) => asset
                    .materials
                    .get(usize::from(m.material))
                    .map_or(1, |ir| u64::from(ir.cost) + ir.ops.len() as u64),
                _ => 1,
            }
        })
        .collect()
}

fn run(
    stage: &Stage<'_>,
    region: Rect,
    scenario: &Scenario,
    hidden: u32,
    light: &Luminance,
    meter: &mut Meter,
) -> Outcome {
    if region.is_empty() {
        return Outcome::default();
    }
    let costs = material_costs(stage);
    meter.spend(RUN_WORK * pixels(region));
    let mut canvas = Canvas::new(region);
    let count = canvas.pixels().len();
    let width = region.width() as usize;
    let fps = u32::from(stage.asset.fps.max(1));
    let mut dark = Tracker::new(count, fps, scenario.frames);
    let mut bright = Tracker::new(count, fps, scenario.frames);
    let mut previous: Vec<u32> = vec![0; count];
    let mut before: Option<(Vec<Draw>, PaletteFrame, (i32, i32))> = None;
    for frame in 0..scenario.frames {
        if !meter.spare() {
            break;
        }
        let shot = stage.shot(scenario, frame);
        let list = stage.draws(&shot, hidden);
        let plate = stage.plate(&shot);
        let mut work = meter.frame + DRAW_WORK * list.len() as u64;
        for draw in &list {
            if let DrawSource::Pixels { pixels, .. } = &draw.source {
                let cost = costs.get(usize::from(draw.layer)).copied().unwrap_or(1);
                work += MATERIAL_WORK * cost * pixels.len() as u64;
            }
        }
        let dirty = match &before {
            Some((draws, palette, at)) if *palette == shot.pose.palette && *at == plate => {
                changed(draws, &list, plate).intersect(region)
            }
            _ => region,
        };
        if !dirty.is_empty() {
            let composited: u64 = list
                .iter()
                .map(|draw| pixels(draw_area(draw, plate).intersect(dirty)))
                .sum();
            work += PIXEL_WORK * (composited + 2 * pixels(dirty));
            stage.render_within(&shot, &list, &mut canvas, dirty);
            let pixels = canvas.pixels();
            for y in dirty.y0..dirty.y1 {
                let row = (y - region.y0) as usize * width;
                for x in dirty.x0..dirty.x1 {
                    let i = row + (x - region.x0) as usize;
                    let argb = pixels[i];
                    if frame > 0 && argb == previous[i] {
                        continue;
                    }
                    let over_black = light.over(argb, 0);
                    let over_white = light.over(argb, 0x00ff_ffff);
                    if frame == 0 {
                        dark.start(i, over_black);
                        bright.start(i, over_white);
                    } else {
                        dark.step(i, over_black, frame);
                        bright.step(i, over_white, frame);
                    }
                    previous[i] = argb;
                }
            }
        }
        before = Some((list, shot.pose.palette, plate));
        meter.spend(work);
    }
    dark.finish();
    bright.finish();
    let first = Outcome {
        pixels: dark.most_pixels(),
        times: dark.most_times(),
    };
    first.worse(Outcome {
        pixels: bright.most_pixels(),
        times: bright.most_times(),
    })
}

fn capped(frames: i64) -> u32 {
    frames.clamp(1, i64::from(NAMEPLATE_FLASH_FRAMES)) as u32
}

pub fn base_frames(asset: &NameplateAsset) -> u32 {
    let looped = i64::from(asset.loop_frames.max(1));
    let fps = i64::from(asset.fps.max(1));
    let laps = 2.max((fps + looped - 1) / looped + 1);
    capped(looped * laps)
}

fn shot_period(trigger: Trigger, length: i64, fps: i64) -> i64 {
    match trigger {
        Trigger::Appear(_) => length + fps * i64::from(NAMEPLATE_APPEAR_SECONDS_MIN),
        Trigger::Start(_) | Trigger::Stop(_) | Trigger::Equip => length,
    }
}

fn scheduled_start(round: i64, window: i64, jitter: i64) -> i64 {
    round * window + if round == 0 { jitter } else { 0 }
}

const SWEEP_LEVELS: i64 = 9;

fn jitter_levels(jitter: i64) -> Vec<i64> {
    if jitter < SWEEP_LEVELS {
        return (0..=jitter).collect();
    }
    let mut levels = vec![0, 1, 2, jitter - 2, jitter - 1, jitter];
    levels.extend((1..4).map(|q| jitter * q / 4));
    levels.sort_unstable();
    levels.dedup();
    levels
}

fn sweep_offsets(jitter: i64) -> Vec<i64> {
    let levels = jitter_levels(jitter);
    let mut out = Vec::new();
    for (i, low) in levels.iter().enumerate() {
        out.push(*low);
        for high in &levels[i + 1..] {
            out.extend([*low, *high]);
        }
    }
    out
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.abs().max(1)
    } else {
        gcd(b, a.rem_euclid(b))
    }
}

fn sweep_rounds(window: i64, loop_frames: i64, cycle: i64, rounds: i64) -> i64 {
    let loop_frames = loop_frames.max(1);
    let phases = loop_frames / gcd(cycle * window, loop_frames);
    let most = i64::from(NAMEPLATE_FLASH_FRAMES) / window + 1;
    (cycle * phases + 1).max(rounds).min(most)
}

fn phase_picks(span: i64) -> Vec<i64> {
    let mut picks: Vec<i64> = if span <= SWEEP_LEVELS {
        (1..span).collect()
    } else {
        let mut picks = vec![1, 2, span - 2, span - 1];
        picks.extend((1..4).map(|q| span * q / 4));
        picks
    };
    picks.sort_unstable();
    picks.dedup();
    picks
}

fn loop_offsets(asset: &NameplateAsset, step: i64, covered: bool) -> Vec<u64> {
    let looped = i64::from(asset.loop_frames.max(1));
    if asset.phase != Phase::PerPlayer {
        return Vec::new();
    }
    let span = if covered { gcd(step, looped) } else { looped };
    phase_picks(span)
        .into_iter()
        .filter_map(|offset| u64::try_from(offset).ok())
        .collect()
}

fn shot_offsets(looped: i64, period: i64, runs: i64) -> (i64, Vec<i64>) {
    let span = gcd(period, looped);
    let span = if looped / span <= runs { span } else { looped };
    (span, phase_picks(span))
}

fn de_bruijn(symbols: usize, order: usize) -> Vec<usize> {
    fn visit(t: usize, p: usize, symbols: usize, word: &mut [usize], out: &mut Vec<usize>) {
        let order = word.len() - 1;
        if t > order {
            if order % p == 0 {
                out.extend_from_slice(&word[1..=p]);
            }
            return;
        }
        word[t] = word[t - p];
        visit(t + 1, p, symbols, word, out);
        for next in word[t - p] + 1..symbols {
            word[t] = next;
            visit(t + 1, t, symbols, word, out);
        }
    }
    if symbols == 0 || order == 0 {
        return Vec::new();
    }
    let mut word = vec![0; order + 1];
    let mut out = Vec::new();
    visit(1, 1, symbols, &mut word, &mut out);
    let head: Vec<usize> = out.iter().take(order - 1).copied().collect();
    out.extend(head);
    out
}

fn mixed_events(asset: &NameplateAsset, id: u16) -> Option<(Vec<Event>, i64)> {
    let schedule = &asset.animations.schedules[usize::from(id)];
    let window = i64::from(schedule.window.max(1));
    let jitter = i64::from(schedule.jitter);
    if schedule.chance == 0 {
        return None;
    }
    let mut offsets = vec![0];
    if jitter > 0 {
        offsets.push(jitter);
    }
    let mut symbols: Vec<Option<(u16, i64)>> = Vec::new();
    if schedule.chance < CHANCE_ONE {
        symbols.push(None);
    }
    for (index, event) in schedule.events.iter().enumerate() {
        if event.weight == 0 {
            continue;
        }
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        symbols.extend(offsets.iter().map(|offset| Some((index, *offset))));
    }
    let playable = schedule.events.iter().filter(|e| e.weight > 0).count();
    let outcomes = playable + usize::from(schedule.chance < CHANCE_ONE);
    let count = symbols.len();
    if outcomes < 2 {
        return None;
    }
    let most = (i64::from(NAMEPLATE_FLASH_FRAMES) / window).max(1);
    let fps = i64::from(asset.fps.max(1));
    let reach = (fps + window - 1) / window + 1;
    let words = |order: i64| -> i64 {
        u32::try_from(order)
            .ok()
            .and_then(|order| i64::try_from(count).ok()?.checked_pow(order))
            .map_or(i64::MAX, |words| words.saturating_add(order - 1))
    };
    let mut order = 2;
    while order < reach && words(order + 1) <= most {
        order += 1;
    }
    let mut sequence = de_bruijn(count, usize::try_from(order).unwrap_or(2));
    sequence.truncate(usize::try_from(most).unwrap_or(usize::MAX));
    let windows = sequence.len() as i64;
    let events = sequence
        .into_iter()
        .enumerate()
        .filter_map(|(k, symbol)| {
            let (event, offset) = symbols[symbol]?;
            let clip = ClipRef::Schedule {
                schedule: id,
                event,
            };
            Some(Event {
                clip,
                start: k as i64 * window + offset,
                length: clip_length(asset, clip),
            })
        })
        .collect();
    Some((events, windows * window))
}

fn playable(asset: &NameplateAsset, id: u16) -> Vec<u16> {
    let schedule = &asset.animations.schedules[usize::from(id)];
    if schedule.chance == 0 || schedule.window == 0 {
        return Vec::new();
    }
    schedule
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.weight > 0)
        .map(|(index, _)| u16::try_from(index).unwrap_or(u16::MAX))
        .collect()
}

fn schedule_events(
    id: u16,
    playable: &[u16],
    asset: &NameplateAsset,
    rounds: i64,
    start: impl Fn(i64) -> i64,
) -> Vec<Event> {
    let count = playable.len() as i64;
    (0..rounds)
        .map(|k| {
            let clip = ClipRef::Schedule {
                schedule: id,
                event: playable[k.rem_euclid(count) as usize],
            };
            Event {
                clip,
                start: start(k),
                length: clip_length(asset, clip),
            }
        })
        .collect()
}

struct Template {
    scenario: Scenario,
    pins: Vec<(u8, f32)>,
    optional: bool,
}

impl Template {
    fn new(scenario: Scenario, pins: Vec<(u8, f32)>) -> Template {
        Template {
            scenario,
            pins,
            optional: false,
        }
    }
}

fn trigger_state(trigger: Trigger) -> (Vec<u8>, Vec<(u8, f32)>, Option<u8>) {
    match trigger {
        Trigger::Start(signal) => (vec![signal], vec![(signal, 1.0)], Some(signal)),
        Trigger::Stop(signal) => (Vec::new(), vec![(signal, 0.0)], Some(signal)),
        Trigger::Appear(_) | Trigger::Equip => (Vec::new(), Vec::new(), None),
    }
}

fn edge_runs(asset: &NameplateAsset, trigger: Trigger, edges: &[i64], after: i64) -> Vec<Event> {
    let opposite = match trigger {
        Trigger::Start(signal) => Trigger::Stop(signal),
        Trigger::Stop(signal) => Trigger::Start(signal),
        Trigger::Appear(_) | Trigger::Equip => return Vec::new(),
    };
    let mut out = Vec::new();
    for (index, shot) in asset.animations.one_shots.iter().enumerate() {
        let shift = if shot.trigger == trigger {
            0
        } else if shot.trigger == opposite {
            after
        } else {
            continue;
        };
        let clip = ClipRef::OneShot(u16::try_from(index).unwrap_or(u16::MAX));
        let length = clip_length(asset, clip);
        let mut runs: Vec<Event> = Vec::new();
        for start in edges.iter().map(|edge| edge + shift) {
            if let Some(last) = runs.last_mut() {
                if start - last.start < last.length {
                    if !shot.restart {
                        continue;
                    }
                    last.length = start - last.start;
                }
            }
            runs.push(Event {
                clip,
                start,
                length,
            });
        }
        out.extend(runs);
    }
    out.sort_by_key(|event| event.start);
    out
}

fn templates(asset: &NameplateAsset) -> Vec<Template> {
    let base = base_frames(asset);
    let repeats = i64::from(NAMEPLATE_FLASH_SCHEDULE_REPEATS);
    let fps = i64::from(asset.fps.max(1));
    let animations = &asset.animations;
    let mut out = vec![Template::new(Scenario::idle(base, 0), Vec::new())];
    let mut signals: Vec<u8> = Vec::new();
    for reaction in &animations.reactions {
        if !signals.contains(&reaction.signal) {
            signals.push(reaction.signal);
        }
    }
    for signal in &signals {
        out.push(Template::new(
            Scenario::held(Source::Reaction(*signal), vec![*signal], base, 0),
            vec![(*signal, 1.0)],
        ));
    }
    let mut everything: Vec<Event> = Vec::new();
    let mut longest = i64::from(base);
    let mut flips = Vec::new();
    let mut shifted = Vec::new();
    for (index, shot) in animations.one_shots.iter().enumerate() {
        let clip = ClipRef::OneShot(u16::try_from(index).unwrap_or(u16::MAX));
        let length = clip_length(asset, clip);
        let period = shot_period(shot.trigger, length, fps);
        let events: Vec<Event> = (0..repeats)
            .map(|k| Event {
                clip,
                start: k * period,
                length,
            })
            .collect();
        let span = period * repeats;
        longest = longest.max(span);
        everything.extend(events.iter().copied());
        let (held, pins, restart) = trigger_state(shot.trigger);
        let template = Scenario {
            source: Source::OneShot(u16::try_from(index).unwrap_or(u16::MAX)),
            signals: held,
            events,
            restart,
            ..Scenario::idle(capped(span.max(i64::from(base))), 0)
        };
        if restart.is_some() {
            let mut gaps = vec![0, 1, length];
            gaps.dedup();
            for (rank, gap) in gaps.into_iter().enumerate() {
                let period = length + gap;
                let runs = repeats.max(fps / period + 2);
                let edges: Vec<i64> = (0..runs).map(|k| k * period).collect();
                let mut points = vec![1, length / 2];
                if gap > 0 {
                    points.extend([length, period - 1]);
                }
                points.sort_unstable();
                points.dedup();
                for after in points
                    .into_iter()
                    .filter(|after| (1..period).contains(after))
                {
                    flips.push((
                        rank,
                        Scenario {
                            flip: Some(after),
                            events: edge_runs(asset, shot.trigger, &edges, after),
                            frames: capped((period * runs).max(i64::from(base))),
                            ..template.clone()
                        },
                        pins.clone(),
                    ));
                }
            }
        }
        let looped = i64::from(asset.loop_frames.max(1));
        let mut events = Vec::new();
        let mut next = 0;
        let (span, offsets) = shot_offsets(looped, period, repeats);
        for offset in offsets {
            let start = next + (offset - next).rem_euclid(span);
            events.push(Event {
                clip,
                start,
                length,
            });
            next = start + period;
        }
        if !events.is_empty() {
            shifted.push(Template {
                scenario: Scenario {
                    events,
                    frames: capped(next.max(i64::from(base))),
                    ..template.clone()
                },
                pins: pins.clone(),
                optional: true,
            });
        }
        out.push(Template::new(template, pins));
    }
    let mut mixed = Vec::new();
    let mut sweeps = Vec::new();
    let mut phased = Vec::new();
    let mut sources = signals.len() + animations.one_shots.len();
    for (s, schedule) in animations.schedules.iter().enumerate() {
        let window = i64::from(schedule.window);
        let jitter = i64::from(schedule.jitter);
        let id = u16::try_from(s).unwrap_or(u16::MAX);
        let playable = playable(asset, id);
        let count = playable.len() as i64;
        if count == 0 {
            continue;
        }
        sources += 1;
        let rounds = if count > 1 { count * repeats } else { repeats };
        let events = schedule_events(id, &playable, asset, rounds, |k| {
            scheduled_start(k, window, jitter)
        });
        let span = rounds * window;
        longest = longest.max(span);
        everything.extend(events.iter().copied());
        out.push(Template::new(
            Scenario {
                source: Source::Schedule(id),
                events,
                ..Scenario::idle(capped(span.max(i64::from(base))), 0)
            },
            Vec::new(),
        ));
        if let Some((events, span)) = mixed_events(asset, id) {
            mixed.push(Template {
                scenario: Scenario {
                    source: Source::Schedule(id),
                    events,
                    ..Scenario::idle(capped(span.max(i64::from(base))), 0)
                },
                pins: Vec::new(),
                optional: true,
            });
        }
        let offsets = sweep_offsets(jitter);
        let cycle = offsets.len() as i64;
        let looped = i64::from(asset.loop_frames.max(1));
        let swept = sweep_rounds(window, looped, cycle, rounds);
        let sweep = Scenario {
            source: Source::Schedule(id),
            events: schedule_events(id, &playable, asset, swept, |k| {
                k * window + offsets[k.rem_euclid(cycle) as usize]
            }),

            ..Scenario::idle(capped((swept * window).max(i64::from(base))), 0)
        };
        let covered = swept > cycle * (looped / gcd(cycle * window, looped));
        phased.extend(
            loop_offsets(asset, cycle * window, covered)
                .into_iter()
                .map(|seed| Template {
                    scenario: Scenario {
                        seed,
                        ..sweep.clone()
                    },
                    pins: Vec::new(),
                    optional: true,
                }),
        );
        if jitter > 0 || swept > rounds {
            sweeps.push(Template {
                scenario: sweep,
                pins: Vec::new(),
                optional: true,
            });
        }
        if count > 1 {
            for event in playable {
                let clip = ClipRef::Schedule {
                    schedule: id,
                    event,
                };
                let events: Vec<Event> = (0..repeats)
                    .map(|k| Event {
                        clip,
                        start: scheduled_start(k, window, jitter),
                        length: clip_length(asset, clip),
                    })
                    .collect();
                out.push(Template::new(
                    Scenario {
                        source: Source::Schedule(id),
                        events,
                        ..Scenario::idle(capped((repeats * window).max(i64::from(base))), 0)
                    },
                    Vec::new(),
                ));
            }
        }
    }
    if sources > 1 {
        let pins = signals.iter().map(|s| (*s, 1.0)).collect();
        out.push(Template::new(
            Scenario {
                source: Source::Everything,
                signals,
                events: everything,
                ..Scenario::idle(capped(longest), 0)
            },
            pins,
        ));
    }
    out.extend(route_shifts(asset, base));
    out.extend(mixed);
    out.extend(sweeps);
    out.extend(phased);
    out.extend(shifted);
    flips.sort_by_key(|(rank, _, _)| *rank);
    out.extend(flips.into_iter().map(|(_, scenario, pins)| Template {
        scenario,
        pins,
        optional: true,
    }));
    out
}

const ROUTE_WINDOWS: i64 = 16;

fn route_loops(asset: &NameplateAsset) -> i64 {
    let paths = asset.layers.iter().any(|layer| match &layer.content {
        LayerContent::Sprite(sprite) => sprite.path.is_some(),
        _ => false,
    });
    if !paths {
        return 1;
    }
    let inputs = placements(asset);
    let mut most = 1;
    for variant in distinct_variants(asset) {
        for input in &inputs {
            if let Some(stage) = Stage::new(asset, variant, input) {
                most = most.max(stage.route_loops());
            }
        }
    }
    most
}

fn route_shifts(asset: &NameplateAsset, base: u32) -> Vec<Template> {
    let looped = i64::from(asset.loop_frames.max(1));
    let span = (i64::from(base) / looped).max(1);
    let windows = (route_loops(asset) + span - 1) / span;
    let picks = (windows - 1).clamp(0, ROUTE_WINDOWS - 1);
    let mut starts: Vec<i64> = (1..=picks).map(|j| j * windows / (picks + 1)).collect();
    starts.dedup();
    starts
        .into_iter()
        .filter_map(|window| u32::try_from(window * span * looped).ok())
        .map(|ahead| Template {
            scenario: Scenario {
                ahead,
                ..Scenario::idle(base, 0)
            },
            pins: Vec::new(),
            optional: true,
        })
        .collect()
}

struct Context {
    template: Scenario,
    optional: bool,
    pins: Vec<Option<f32>>,
    cover: Cover,
    rows: Vec<Vec<usize>>,
}

struct Plan {
    contexts: Vec<Context>,
    global: Cover,
    work: u64,
    rows: usize,
    order: Vec<usize>,
}

impl Plan {
    fn fits(&self, context: usize) -> bool {
        let frames = u64::from(self.contexts[context].template.frames);
        self.work + frames <= SCENARIO_FRAMES && self.rows < SCENARIO_ROWS
    }

    fn add(&mut self, context: usize, seeds: &[(usize, usize)]) {
        let c = &mut self.contexts[context];
        let row = c.cover.row(seeds, Some(&self.global));
        c.cover.mark(&row);
        self.global.mark(&row);
        self.work += u64::from(c.template.frames);
        self.rows += 1;
        c.rows.push(row);
        self.order.push(context);
    }

    fn round(
        &mut self,
        extras: bool,
        pick: impl Fn(&mut Cover) -> Option<Vec<(usize, usize)>>,
    ) -> bool {
        let mut progressed = false;
        for context in 0..self.contexts.len() {
            if !self.fits(context) || (!extras && self.contexts[context].optional) {
                continue;
            }
            if let Some(seeds) = pick(&mut self.contexts[context].cover) {
                self.add(context, &seeds);
                progressed = true;
            }
        }
        progressed
    }

    fn cheapest(&self, pair: [(usize, usize); 2]) -> Option<usize> {
        (0..self.contexts.len())
            .filter(|c| {
                let cover = &self.contexts[*c].cover;
                pair.iter().all(|(l, v)| cover.allows(*l, *v))
            })
            .min_by_key(|c| (self.contexts[*c].template.frames, *c))
    }
}

fn context(template: Template, lanes: &[Lane], sizes: &[usize]) -> Context {
    let Template {
        scenario: template,
        pins: held,
        optional,
    } = template;
    let pins: Vec<Option<f32>> = lanes
        .iter()
        .map(|lane| {
            if lane.time {
                template
                    .restart
                    .filter(|s| usize::from(*s) == lane.id)
                    .map(|_| 0.0)
            } else {
                held.iter()
                    .find(|(s, _)| usize::from(*s) == lane.id)
                    .map(|(_, v)| *v)
            }
        })
        .collect();
    let pinned: Vec<Option<usize>> = pins
        .iter()
        .zip(lanes)
        .map(|(pin, lane)| pin.map(|v| lane.index_of(v)))
        .collect();
    Context {
        cover: Cover::new(sizes, &pinned),
        template,
        optional,
        pins,
        rows: Vec::new(),
    }
}

pub fn planned(asset: &NameplateAsset) -> (Vec<(Scenario, bool)>, Vec<Source>) {
    let (values, times) = read_signals(asset);
    let lanes = lanes(&values, &times);
    let sizes: Vec<usize> = lanes.iter().map(|l| l.samples.len()).collect();
    let contexts = templates(asset)
        .into_iter()
        .map(|template| context(template, &lanes, &sizes))
        .collect();
    let mut plan = Plan {
        contexts,
        global: Cover::new(&sizes, &vec![None; sizes.len()]),
        work: 0,
        rows: 0,
        order: Vec::new(),
    };
    let first = |plan: &mut Plan, optional: bool| {
        for context in 0..plan.contexts.len() {
            if plan.contexts[context].optional != optional || !plan.fits(context) {
                continue;
            }
            let seeds: Vec<(usize, usize)> = plan.contexts[context]
                .cover
                .first_single()
                .into_iter()
                .collect();
            plan.add(context, &seeds);
        }
    };
    first(&mut plan, false);
    let single = |cover: &mut Cover| cover.first_single().map(|s| vec![s]);
    while plan.round(false, single) {}
    let settled = plan.order.len();
    let mut unplanned = Vec::new();
    for c in &mut plan.contexts {
        let missing = c.rows.is_empty() || c.cover.first_single().is_some();
        if !c.optional && missing && !unplanned.contains(&c.template.source) {
            unplanned.push(c.template.source);
        }
    }
    first(&mut plan, true);
    while let Some(pair) = plan.global.first_pair() {
        let Some(context) = plan.cheapest(pair).filter(|c| plan.fits(*c)) else {
            break;
        };
        plan.add(context, &pair);
    }
    while plan.round(true, single) {}
    while plan.round(true, |cover| cover.first_pair().map(|p| p.to_vec())) {}
    let mut next = vec![0; plan.contexts.len()];
    let mut out: Vec<(Scenario, bool)> = plan
        .order
        .iter()
        .enumerate()
        .map(|(rank, context)| {
            let c = &plan.contexts[*context];
            let row = &c.rows[next[*context]];
            next[*context] += 1;
            let scenario = Scenario {
                sample: sample(&lanes, row, &c.pins),
                ..c.template.clone()
            };
            (scenario, !c.optional && rank < settled)
        })
        .collect();
    out.sort_by_key(|(_, required)| !*required);
    (out, unplanned)
}

pub fn animated(asset: &NameplateAsset) -> bool {
    let a = &asset.animations;
    let clips = iter::once(&a.looping)
        .chain(
            a.schedules
                .iter()
                .flat_map(|s| s.events.iter().map(|e| &e.clip)),
        )
        .chain(a.reactions.iter().map(|r| &r.clip))
        .chain(a.one_shots.iter().map(|o| &o.clip));
    let tracks = clips
        .clone()
        .any(|c| !c.targets.is_empty() || !c.palette.is_empty());
    let layers = asset.layers.iter().any(|l| match &l.content {
        LayerContent::Sprite(s) => {
            s.path.is_some()
                || asset
                    .sprites
                    .get(usize::from(s.sprite))
                    .is_some_and(|sprite| sprite.frames.len() > 1)
                || l.breakpoints.iter().filter_map(|b| b.sprite).any(|id| {
                    asset
                        .sprites
                        .get(usize::from(id))
                        .is_some_and(|s| s.frames.len() > 1)
                })
        }
        LayerContent::Emitter(_) | LayerContent::Material(_) => true,
        LayerContent::Body(_) => false,
    });
    tracks || layers
}

pub fn placements(asset: &NameplateAsset) -> Vec<BlockInput> {
    let mut inputs = reference_inputs();
    inputs.extend(breakpoint_inputs(asset));
    inputs
}

fn source_name(asset: &NameplateAsset, source: Source) -> String {
    let a = &asset.animations;
    match source {
        Source::Loop => a.looping.name.clone(),
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
        Source::Everything => String::from("*"),
    }
}

fn culprit(
    stage: &Stage<'_>,
    region: Rect,
    scenario: &Scenario,
    light: &Luminance,
    full: Outcome,
    meter: &mut Meter,
) -> Option<usize> {
    let asset = stage.asset;
    let mut best: Option<(u32, usize)> = None;
    for slot in 0..asset.layers.len().min(32) {
        let Some(layer) = resolved(asset, stage.variant, slot) else {
            continue;
        };
        if !meter.spare() {
            break;
        }
        let without = run(stage, region, scenario, 1 << slot, light, meter);
        if !meter.spare() {
            break;
        }
        let drop = full.pixels.saturating_sub(without.pixels);
        if drop == 0 {
            continue;
        }
        if !without.fails() {
            return Some(layer);
        }
        if best.is_none_or(|(most, _)| drop > most) {
            best = Some((drop, layer));
        }
    }
    best.map(|(_, layer)| layer)
}

fn drawn(measured: &Measured, input: &BlockInput, variant: u16) -> Rect {
    measured
        .extents
        .iter()
        .flatten()
        .filter(|(i, v, _)| i == input && *v == variant)
        .fold(Rect::EMPTY, |all, (_, _, r)| all.union(*r))
}

fn frame_work(measured: &Measured) -> u64 {
    FRAME_WORK + CPU_WORK * u64::from(measured.cpu.0.total())
}

pub fn check(asset: &NameplateAsset, measured: &Measured, report: &mut Report) {
    spend(asset, measured, report, NAMEPLATE_FLASH_WORK);
}

pub fn spend(asset: &NameplateAsset, measured: &Measured, report: &mut Report, limit: u64) -> u64 {
    if !animated(asset) {
        return 0;
    }
    let light = Luminance::new();
    let (ranked, unplanned) = planned(asset);
    let inputs = placements(asset);
    let mut stages = Vec::new();
    for variant in distinct_variants(asset) {
        for input in &inputs {
            let Some(stage) = Stage::new(asset, variant, input) else {
                continue;
            };
            let region = drawn(measured, input, variant).intersect(stage.region());
            if !region.is_empty() {
                stages.push((stage, region));
            }
        }
    }
    let required = ranked.iter().filter(|(_, r)| *r).count();
    let scenarios = required + unplanned.len();
    let mut meter = Meter::new(limit, frame_work(measured));
    let unchecked = |checked: usize, source: Source, spent: u64| {
        Issue::error(IssueCode::FlashUnchecked, path(MANIFEST, "animations"))
            .value("checked", checked as f64)
            .value("scenarios", scenarios as f64)
            .value("work", spent as f64)
            .value("limit", limit as f64)
            .name("animation", &source_name(asset, source))
    };
    for (rank, (scenario, needed)) in ranked.iter().enumerate() {
        for (stage, region) in &stages {
            if meter.spare() {
                let outcome = run(stage, *region, scenario, 0, &light, &mut meter);
                if outcome.fails() {
                    let mut spare = Meter::new(limit / 2, meter.frame);
                    let layer = culprit(stage, *region, scenario, &light, outcome, &mut spare);
                    report.push(flash_issue(stage, scenario, outcome, layer));
                    return meter.spent.saturating_add(spare.spent);
                }
            }
            if !meter.spare() {
                if *needed {
                    report.push(unchecked(rank, scenario.source, meter.spent));
                } else if let Some(source) = unplanned.first() {
                    report.push(unchecked(required, *source, meter.spent));
                }
                return meter.spent;
            }
        }
    }
    if let Some(source) = unplanned.first() {
        report.push(unchecked(required, *source, meter.spent));
    }
    meter.spent
}

fn flash_issue(
    stage: &Stage<'_>,
    scenario: &Scenario,
    outcome: Outcome,
    layer: Option<usize>,
) -> Issue {
    let asset = stage.asset;
    let source = source_name(asset, scenario.source);
    let at = match (scenario.source, layer) {
        (Source::Loop, Some(layer)) => layer_path(&asset.layers[layer].id),
        (Source::Everything, _) => path(MANIFEST, "animations"),
        _ => animation_path(&source),
    };
    let mut issue = Issue::error(IssueCode::Flash, at)
        .value("flashes", f64::from(outcome.times))
        .value("px", f64::from(outcome.pixels))
        .value("limit", f64::from(FLASHES_PER_SECOND))
        .value("area", f64::from(FLASH_AREA_PIXELS))
        .value("name_width", f64::from(stage.placement.block.name_width))
        .value("lines", f64::from(stage.placement.block.line_count))
        .name("animation", &source)
        .name("variant", &stage.variant.key);
    if let Some(layer) = layer {
        issue = issue.name("layer", &asset.layers[layer].id);
    }
    issue
}

#[cfg(test)]
pub fn simulate(
    stage: &Stage<'_>,
    region: Rect,
    scenario: &Scenario,
    hidden: u32,
    light: &Luminance,
) -> Outcome {
    let mut meter = Meter::new(u64::MAX, FRAME_WORK);
    run(stage, region, scenario, hidden, light, &mut meter)
}

#[cfg(test)]
pub fn scenarios(asset: &NameplateAsset) -> Vec<Scenario> {
    ranked(asset).into_iter().map(|(s, _)| s).collect()
}

#[cfg(test)]
pub fn ranked(asset: &NameplateAsset) -> Vec<(Scenario, bool)> {
    planned(asset).0
}
