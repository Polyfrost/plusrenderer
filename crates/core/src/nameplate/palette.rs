use std::iter;

use crate::issues::{join, path, Issue, IssueCode};
use crate::limits::NAMEPLATE_BLEND_ROWS;
use crate::palette::{along, custom_stops, custom_tones, lerp, Palette, MAX_SLOTS, MAX_TONES};

use super::anim::{ClipRef, ClipTime};
use super::image::{ColorRef, RefKind};
use super::types::{
    Clip, CycleMode, Ease, Fill, LayerContent, NameplateAsset, PaletteFrame, PaletteKey,
    PaletteTrack, PaletteValue, Variant, PALETTE_TONES,
};

pub const EASE_UNITS: i64 = 1024;

pub fn blend_ref(slot: u8, row: u16, rows: u16) -> Option<ColorRef> {
    let fits = usize::from(slot) < MAX_SLOTS
        && rows >= 1
        && u32::from(rows) <= NAMEPLATE_BLEND_ROWS
        && row < rows;
    fits.then(|| ColorRef::blend(slot, row, rows))
}

pub fn clip(asset: &NameplateAsset, clip: ClipRef) -> Option<&Clip> {
    let animations = &asset.animations;
    match clip {
        ClipRef::Loop => Some(&animations.looping),
        ClipRef::Schedule { schedule, event } => animations
            .schedules
            .get(usize::from(schedule))?
            .events
            .get(usize::from(event))
            .map(|e| &e.clip),
        ClipRef::Reaction(index) => animations
            .reactions
            .get(usize::from(index))
            .map(|r| &r.clip),
        ClipRef::OneShot(index) => animations
            .one_shots
            .get(usize::from(index))
            .map(|o| &o.clip),
    }
}

pub fn custom_variant(asset: &NameplateAsset, variant: &Variant, custom: &[(u8, u32)]) -> Variant {
    let colorway = variant.colorway.map(usize::from);
    let mut out = variant.clone();
    out.tones = custom_tones(&asset.palette, colorway, custom);
    out.stops = custom_stops(&asset.palette, colorway, custom);
    out
}

pub fn base_frame(variant: &Variant, out: &mut PaletteFrame) {
    let count = variant.tones.len().min(PALETTE_TONES);
    out.tones[..count].copy_from_slice(&variant.tones[..count]);
    out.tones[count..].fill(0);
    out.animated = 0;
    out.stop_counts = [0; MAX_SLOTS];
}

fn slot_tones(frame: &PaletteFrame, slot: usize) -> &[u32] {
    let start = (slot * MAX_TONES).min(PALETTE_TONES);
    &frame.tones[start..(start + MAX_TONES).min(PALETTE_TONES)]
}

pub fn resolve_ref(
    color: ColorRef,
    frame: &PaletteFrame,
    fixed: &[u32],
    stops: &[Vec<u32>],
) -> u32 {
    match color.kind() {
        RefKind::Slot => frame
            .tones
            .get(color.tone_ref() as usize)
            .copied()
            .unwrap_or(0),
        RefKind::Fixed => fixed
            .get(color.fixed_index() as usize)
            .copied()
            .unwrap_or(0),
        RefKind::Blend => {
            let slot = usize::from(color.slot_index());
            let rows = u32::from(color.blend_rows().max(1));
            let row = u32::from(color.blend_row());
            let animated = usize::from(frame.stop_counts.get(slot).copied().unwrap_or(0));
            let own = slot_tones(frame, slot);
            if animated > 0 {
                return along(&own[..animated.min(own.len())], row, rows);
            }
            match stops.get(slot) {
                Some(stops) if !stops.is_empty() => along(stops, row, rows),
                _ => own.first().copied().unwrap_or(0),
            }
        }
    }
}

pub fn cycle_tone(tone: u8, tones: u8, frame: i64, every: u16, mode: CycleMode) -> u8 {
    if tones == 0 {
        return 0;
    }
    let tone = tone.min(tones - 1);
    if tones == 1 || every == 0 {
        return tone;
    }
    let n = i64::from(tones);
    let steps = frame.div_euclid(i64::from(every));
    let shift = match mode {
        CycleMode::Forward => steps.rem_euclid(n),
        CycleMode::Backward => -steps.rem_euclid(n),
        CycleMode::PingPong => {
            let span = n - 1;
            let position = steps.rem_euclid(2 * span);
            if position <= span {
                position
            } else {
                2 * span - position
            }
        }
    };
    (i64::from(tone) + shift).rem_euclid(n) as u8
}

fn rank(clip: ClipRef) -> u8 {
    match clip {
        ClipRef::Loop => 0,
        ClipRef::Schedule { .. } => 1,
        ClipRef::Reaction(_) => 2,
        ClipRef::OneShot(_) => 3,
    }
}

fn beats(challenger: &ClipTime, holder: &ClipTime) -> bool {
    let (a, b) = (rank(challenger.clip), rank(holder.clip));
    if a != b {
        return a > b;
    }
    match (challenger.clip, holder.clip) {
        (ClipRef::OneShot(x), ClipRef::OneShot(y)) => {
            challenger.start > holder.start || (challenger.start == holder.start && x < y)
        }
        (ClipRef::Reaction(x), ClipRef::Reaction(y)) => x < y,
        (
            ClipRef::Schedule {
                schedule: s,
                event: e,
            },
            ClipRef::Schedule {
                schedule: t,
                event: f,
            },
        ) => (s, e) < (t, f),
        _ => false,
    }
}

struct Base<'a> {
    asset: &'a NameplateAsset,
    variant: &'a Variant,
}

impl Base<'_> {
    fn tone(&self, slot: usize, tone: usize) -> u32 {
        self.variant
            .tones
            .get(slot * MAX_TONES + tone.min(MAX_TONES - 1))
            .copied()
            .unwrap_or(0)
    }

    fn color(&self, color: ColorRef) -> u32 {
        match color.kind() {
            RefKind::Slot => self
                .variant
                .tones
                .get(color.tone_ref() as usize)
                .copied()
                .unwrap_or(0),
            RefKind::Fixed => self
                .asset
                .fixed_colors
                .get(color.fixed_index() as usize)
                .copied()
                .unwrap_or(0),
            RefKind::Blend => {
                let slot = usize::from(color.slot_index());
                match self.variant.stops.get(slot) {
                    Some(stops) if !stops.is_empty() => along(
                        stops,
                        u32::from(color.blend_row()),
                        u32::from(color.blend_rows().max(1)),
                    ),
                    _ => self.tone(slot, 0),
                }
            }
        }
    }

    fn tone_count(&self, slot: usize) -> usize {
        self.asset
            .palette
            .tone_count(slot, self.variant.colorway.map(usize::from))
            .min(MAX_TONES)
    }
}

fn key_values(
    base: &Base<'_>,
    slot: usize,
    key: &PaletteKey,
    colors: &dyn Fn(u32) -> u32,
) -> [u32; MAX_TONES] {
    let mut values = [0; MAX_TONES];
    for (tone, value) in key.tones.iter().take(MAX_TONES).enumerate() {
        values[tone] = match *value {
            PaletteValue::Color(argb) => argb,
            PaletteValue::Ref(color) => base.color(color),
            PaletteValue::Molang(program) => colors(program),
            PaletteValue::Base => base.tone(slot, tone),
        };
    }
    values
}

fn pick(values: &[u32; MAX_TONES], len: usize, tone: usize) -> u32 {
    values[tone.min(len.clamp(1, MAX_TONES) - 1)]
}

fn smoothstep(elapsed: i64, span: i64) -> i64 {
    elapsed * elapsed * (3 * span - 2 * elapsed) * EASE_UNITS / (span * span * span)
}

fn blend(ease: Ease, from: u32, to: u32, elapsed: i64, span: i64) -> u32 {
    if span <= 0 {
        return to;
    }
    let elapsed = elapsed.clamp(0, span);
    match ease {
        Ease::Step => from,
        Ease::Linear => lerp(from, to, elapsed, span),
        Ease::Ease => lerp(from, to, smoothstep(elapsed, span), EASE_UNITS),
    }
}

fn apply_keys(
    base: &Base<'_>,
    slot: usize,
    keys: &[PaletteKey],
    frame: i32,
    colors: &dyn Fn(u32) -> u32,
    out: &mut PaletteFrame,
) {
    if keys.is_empty() {
        return;
    }
    let mut index = 0;
    while index + 1 < keys.len() && i32::from(keys[index + 1].frame) <= frame {
        index += 1;
    }
    let key = &keys[index];
    let next = keys.get(index + 1).filter(|k| k.ease != Ease::Step);
    let width = key
        .tones
        .len()
        .max(next.map_or(0, |k| k.tones.len()))
        .clamp(1, MAX_TONES);
    let tones = match base.tone_count(slot) {
        0 => width,
        n => n,
    };
    let from = key_values(base, slot, key, colors);
    let to = next.map(|k| (k, key_values(base, slot, k, colors)));
    let mut blended = [0; MAX_TONES];
    for (tone, color) in blended.iter_mut().enumerate().take(tones) {
        let start = pick(&from, key.tones.len(), tone);
        *color = match &to {
            Some((next, to)) => blend(
                next.ease,
                start,
                pick(to, next.tones.len(), tone),
                i64::from(frame) - i64::from(key.frame),
                i64::from(next.frame) - i64::from(key.frame),
            ),
            None => start,
        };
    }
    let offset = slot * MAX_TONES;
    for tone in 0..MAX_TONES {
        out.tones[offset + tone] = blended[tone.min(tones - 1)];
    }
    out.stop_counts[slot] = tones as u8;
}

fn apply_cycle(
    base: &Base<'_>,
    slot: usize,
    every: u16,
    mode: CycleMode,
    frame: i32,
    out: &mut PaletteFrame,
) {
    let tones = base.tone_count(slot) as u8;
    let offset = slot * MAX_TONES;
    for tone in 0..MAX_TONES {
        let source = cycle_tone(tone as u8, tones, i64::from(frame), every, mode);
        out.tones[offset + tone] = base.tone(slot, usize::from(source));
    }
}

pub fn animate(
    asset: &NameplateAsset,
    variant: &Variant,
    playing: &[ClipTime],
    colors: &dyn Fn(u32) -> u32,
    palette_animation: bool,
    out: &mut PaletteFrame,
) {
    base_frame(variant, out);
    if !palette_animation {
        return;
    }
    let mut winners: [Option<usize>; MAX_SLOTS] = [None; MAX_SLOTS];
    for (index, time) in playing.iter().enumerate() {
        let Some(clip) = clip(asset, time.clip) else {
            continue;
        };
        for track in &clip.palette {
            let Some(winner) = winners.get_mut(usize::from(track.slot)) else {
                continue;
            };
            if winner.is_none_or(|held| beats(time, &playing[held])) {
                *winner = Some(index);
            }
        }
    }
    let base = Base { asset, variant };
    for (slot, winner) in winners.iter().enumerate() {
        let Some(index) = *winner else {
            continue;
        };
        let time = &playing[index];
        let Some(track) = clip(asset, time.clip)
            .and_then(|c| c.palette.iter().find(|t| usize::from(t.slot) == slot))
        else {
            continue;
        };
        match &track.track {
            PaletteTrack::Cycle { every, mode } => {
                apply_cycle(&base, slot, *every, *mode, time.frame, out)
            }
            PaletteTrack::Keys(keys) => apply_keys(&base, slot, keys, time.frame, colors, out),
        }
        out.animated |= 1 << slot;
    }
}

pub fn banding_mismatches(palette: &Palette, slot: usize) -> Vec<usize> {
    let expected = palette.tone_count(slot, None);
    (0..palette.colorways.len())
        .filter(|cw| {
            palette.colorways[*cw]
                .overrides
                .iter()
                .any(|o| usize::from(o.slot) == slot)
                && palette.tone_count(slot, Some(*cw)) != expected
        })
        .collect()
}

fn banding_use(asset: &NameplateAsset, slot: usize) -> Option<&'static str> {
    for layer in &asset.layers {
        let LayerContent::Body(body) = &layer.content else {
            continue;
        };
        for fill in body.fill.iter().chain(&body.rings) {
            match *fill {
                Fill::Gradient { slot: s } if usize::from(s) == slot => return Some("gradient"),
                Fill::Sweep { slot: s, .. } if usize::from(s) == slot => return Some("sweep"),
                _ => {}
            }
        }
    }
    let animations = &asset.animations;
    let mut clips = iter::once(&animations.looping)
        .chain(
            animations
                .schedules
                .iter()
                .flat_map(|s| s.events.iter().map(|e| &e.clip)),
        )
        .chain(animations.reactions.iter().map(|r| &r.clip))
        .chain(animations.one_shots.iter().map(|o| &o.clip));
    let cycles = clips.any(|clip| {
        clip.palette
            .iter()
            .any(|t| usize::from(t.slot) == slot && matches!(t.track, PaletteTrack::Cycle { .. }))
    });
    cycles.then_some("cycle")
}

pub fn banding_issues(asset: &NameplateAsset) -> Vec<Issue> {
    let palette = &asset.palette;
    let mut issues = Vec::new();
    for slot in 0..palette.slots.len() {
        let Some(used) = banding_use(asset, slot) else {
            continue;
        };
        let id = &palette.slots[slot].id;
        let expected = palette.tone_count(slot, None);
        for cw in banding_mismatches(palette, slot) {
            let colorway = &palette.colorways[cw].id;
            issues.push(
                Issue::error(
                    IssueCode::ColorwayTones,
                    path("palettes.json", &join(&join("colorways", colorway), id)),
                )
                .name("slot", id)
                .name("colorway", colorway)
                .name("use", used)
                .value("tones", expected as f64)
                .value("colorway_tones", palette.tone_count(slot, Some(cw)) as f64),
            );
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::anim::fixture;
    use crate::nameplate::types::{
        Animations, Ease, OneShot, PaletteKey, PaletteTrack, PaletteValue, Reaction, Reduced, Schedule, ScheduledClip, SlotTrack, Trigger,
    };
    use crate::palette::{along, derive_ramp, Colorway, Slot, SlotOverride};

    const GLINT: u8 = 0;
    const PAIR: u8 = 1;
    const PLAIN: [u32; 4] = [0xff10_2030, 0xff30_4050, 0xff50_6070, 0xff70_8090];
    const ALT: [u32; 4] = [0xffaa_0000, 0xffbb_0000, 0xffcc_0000, 0xffdd_0000];

    fn asset(animations: Animations) -> NameplateAsset {
        let slot = |id: &str, ramp: &[u32]| Slot { id: id.into(), name: id.into(), ramp: ramp.to_vec(), ..Slot::default() };
        let alt = SlotOverride { slot: GLINT, ramp: ALT.to_vec(), stops: Vec::new() };
        let palette = Palette {
            slots: vec![slot("glint", &PLAIN), slot("pair", &[0xff00_0000, 0xffff_ffff])],
            colorways: vec![Colorway { id: "alt".into(), overrides: vec![alt] }],
        };
        let mut asset = fixture::asset(12, 24, Vec::new(), Vec::new(), animations);
        asset.variants = [None, Some(0u16)].map(|colorway| Variant {
            colorway,
            tones: palette.resolve(colorway.map(usize::from)),
            stops: palette.resolve_stops(colorway.map(usize::from)),
            slots: vec![None; 2],
            ..Variant::default()
        }).to_vec();
        asset.palette = palette;
        asset.fixed_colors = vec![0xff01_0203, 0xff04_0506];
        asset
    }

    fn clip(slot: u8, track: PaletteTrack) -> Clip {
        Clip { name: "clip".into(), length: 24, palette: vec![SlotTrack { slot, track }], ..Clip::default() }
    }

    fn keys(slot: u8, keys: &[(u16, Ease, u32)]) -> Clip {
        let keys = keys.iter().map(|&(frame, ease, argb)| PaletteKey { frame, ease, tones: vec![PaletteValue::Color(argb)] });
        clip(slot, PaletteTrack::Keys(keys.collect()))
    }

    fn tones(asset: &NameplateAsset, variant: usize, playing: &[ClipTime], slot: u8, count: u8) -> Vec<u32> {
        let mut out = PaletteFrame::default();
        animate(asset, &asset.variants[variant], playing, &|id| 0xff00_0000 | id, true, &mut out);
        (0..count).map(|t| resolve_ref(ColorRef::slot(slot, t), &out, &[], &[])).collect()
    }

    fn looped(frame: i32) -> ClipTime {
        ClipTime { clip: ClipRef::Loop, frame, start: 0 }
    }

    #[test]
    fn refs_resolve_slots_fixed_colors_and_blends_of_one_to_1024_rows() {
        let asset = asset(Animations::default());
        let mut frame = PaletteFrame::default();
        base_frame(&asset.variants[0], &mut frame);
        let (fixed, stops) = (&asset.fixed_colors, &asset.variants[0].stops);
        let resolve = |color| resolve_ref(color, &frame, fixed, stops);
        assert_eq!((resolve(ColorRef::slot(GLINT, 1)), resolve(ColorRef::slot(GLINT, 15))), (PLAIN[1], PLAIN[3]));
        assert_eq!((resolve(ColorRef::fixed(1)), resolve(ColorRef::fixed(9))), (0xff04_0506, 0));
        let pair: Vec<u32> = [(0, 3), (1, 3), (2, 3), (511, 1024)].map(|(row, rows)| resolve(ColorRef::blend(PAIR, row, rows))).to_vec();
        assert_eq!(pair, [0xff00_0000, 0xff7f_7f7f, 0xffff_ffff, 0xff7f_7f7f]);
        assert!((0..1024).all(|row| resolve(ColorRef::blend(PAIR, row, 1024)) & 0xff == 255 * u32::from(row) / 1023));
        assert_eq!((resolve(ColorRef::blend(GLINT, 2, 7)), resolve(ColorRef::blend(GLINT, 1, 7))), (PLAIN[1], along(&PLAIN, 1, 7)));
        assert_eq!(blend_ref(31, 1023, 1024), Some(ColorRef::blend(31, 1023, 1024)));
        assert!([(0, 0, 0), (0, 0, 1025), (0, 4, 4), (32, 0, 4)].iter().all(|&(s, r, n)| blend_ref(s, r, n).is_none()));
    }

    #[test]
    fn cycles_rotate_tones_per_colorway_and_keys_ease_between_colours() {
        let forward = [0, 1, 2, 4, 7, -2].map(|f| cycle_tone(0, 4, f, 2, CycleMode::Forward));
        assert_eq!(forward, [0, 0, 1, 2, 3, 3]);
        let ping: Vec<u8> = (0..8).map(|i| cycle_tone(0, 4, i * 2, 2, CycleMode::PingPong)).collect();
        assert_eq!(ping, [0, 1, 2, 3, 2, 1, 0, 1]);
        assert_eq!([cycle_tone(0, 4, 3, 3, CycleMode::Backward), cycle_tone(5, 1, 99, 2, CycleMode::Forward), cycle_tone(2, 4, 99, 0, CycleMode::Forward)], [3, 0, 2]);
        let cycling = asset(Animations { looping: clip(GLINT, PaletteTrack::Cycle { every: 2, mode: CycleMode::Forward }), ..Animations::default() });
        assert_eq!(tones(&cycling, 0, &[looped(2)], GLINT, 4), [PLAIN[1], PLAIN[2], PLAIN[3], PLAIN[0]]);
        assert_eq!(tones(&cycling, 1, &[looped(4)], GLINT, 4), [ALT[2], ALT[3], ALT[0], ALT[1]]);
        let keyed = keys(GLINT, &[(0, Ease::Step, 0xff00_0000), (8, Ease::Linear, 0xffff_8040), (16, Ease::Step, 0xff10_2030), (20, Ease::Ease, 0xffff_ffff)]);
        let keyed = asset(Animations { looping: keyed, ..Animations::default() });
        let at: Vec<u32> = [0, 4, 15, 16, 18, 23, -1].iter().map(|&f| tones(&keyed, 0, &[looped(f)], GLINT, 1)[0]).collect();
        assert_eq!(at, [0xff00_0000, 0xff7f_4020, 0xffff_8040, 0xff10_2030, 0xff87_8f97, 0xffff_ffff, 0xff00_0000]);
        let mut custom = asset(Animations::default());
        custom.palette.slots[0].customizable = true;
        let variant = custom_variant(&custom, &custom.variants[0], &[(GLINT, 0x0020_40c0), (PAIR, 0x00c0_4020)]);
        assert_eq!(variant.tones[..4], derive_ramp(&PLAIN, 0x0020_40c0)[..]);
        assert_eq!(variant.tones[MAX_TONES..], custom.variants[0].tones[MAX_TONES..], "locked slots keep their tones");
    }

    #[test]
    fn one_shots_beat_reactions_beat_schedules_beat_the_loop() {
        let solid = |n: u32| keys(GLINT, &[(0, Ease::Step, 0xff00_0000 | n)]);
        let reaction = |signal, argb| Reaction { signal, clip: solid(argb), reduced: Reduced::Hold };
        let shot = |trigger, argb| OneShot { trigger, clip: solid(argb), restart: false };
        let asset = asset(Animations {
            looping: clip(GLINT, PaletteTrack::Cycle { every: 2, mode: CycleMode::Forward }),
            schedules: vec![Schedule { id: "blink".into(), events: vec![ScheduledClip { clip: solid(1), weight: 1 }], ..Schedule::default() }],
            reactions: vec![reaction(0, 2), reaction(1, 3)],
            one_shots: vec![shot(Trigger::Equip, 4), shot(Trigger::Start(0), 5)],
        });
        let at = |clip, start| ClipTime { clip, frame: 0, start };
        let scheduled = at(ClipRef::Schedule { schedule: 0, event: 0 }, 0);
        let glint = |playing: &[ClipTime]| tones(&asset, 0, playing, GLINT, 1)[0] & 0xff;
        assert_eq!(glint(&[looped(2)]), PLAIN[1] & 0xff);
        assert_eq!(glint(&[looped(2), scheduled]), 1);
        assert_eq!(glint(&[scheduled, at(ClipRef::Reaction(1), 0), looped(2)]), 3);
        assert_eq!(glint(&[at(ClipRef::Reaction(1), 0), at(ClipRef::Reaction(0), 0)]), 2, "lower reactions win");
        assert_eq!(glint(&[at(ClipRef::OneShot(1), 10), at(ClipRef::OneShot(0), 12), at(ClipRef::Reaction(0), 0)]), 4);
        assert_eq!(glint(&[at(ClipRef::OneShot(1), 12), at(ClipRef::OneShot(0), 10)]), 5, "the newest one-shot wins");
        assert_eq!(glint(&[at(ClipRef::OneShot(1), 12), at(ClipRef::OneShot(0), 12)]), 4, "ties go to the lower index");
        assert_eq!(glint(&[at(ClipRef::OneShot(7), 99), looped(2)]), PLAIN[1] & 0xff);
        let mut still = PaletteFrame::default();
        animate(&asset, &asset.variants[0], &[looped(2)], &|_| 0, false, &mut still);
        assert_eq!((resolve_ref(ColorRef::slot(GLINT, 0), &still, &[], &[]), still.animated), (PLAIN[0], 0));
    }
}
