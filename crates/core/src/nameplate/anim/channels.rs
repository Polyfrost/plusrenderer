use crate::cost::{Detail, Motion};
use crate::nameplate::types::{
    BadgePose, Channels, LayerContent, LayerPose, LetterOverride, NameplateAsset, Of, Reduced,
    TagPose, Target, Track, Variant, MAX_LAYERS, OPAQUE,
};

use super::tracks::{sample, Channel};
use super::{clip, ClipRef, ClipTime, Gates, TagTime};

type Sampler<'a> = &'a dyn Fn(u32, i32, Channel) -> i32;

pub fn shown(asset: &NameplateAsset, playing: &[ClipTime], gates: &Gates, out: &mut Vec<ClipTime>) {
    out.clear();
    for time in playing {
        let keep = match time.clip {
            ClipRef::Loop => true,
            ClipRef::Reaction(index) => {
                !gates.first_frame
                    && !(gates.hold_reactions
                        && asset
                            .animations
                            .reactions
                            .get(usize::from(index))
                            .is_some_and(|r| r.reduced == Reduced::Drop))
            }
            ClipRef::Schedule { .. } | ClipRef::OneShot(_) => !gates.first_frame,
        };
        if keep {
            let frame = if gates.first_frame { 0 } else { time.frame };
            out.push(ClipTime { frame, ..*time });
        }
    }
}

fn rank(clip: ClipRef) -> u8 {
    match clip {
        ClipRef::Loop => 0,
        ClipRef::Schedule { .. } => 1,
        ClipRef::Reaction(_) => 2,
        ClipRef::OneShot(_) => 3,
    }
}

pub fn beats(challenger: &ClipTime, holder: &ClipTime) -> bool {
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

fn winner<'a>(
    asset: &'a NameplateAsset,
    shown: &[ClipTime],
    target: Target,
    has: fn(&Channels) -> bool,
) -> Option<(ClipTime, &'a Channels)> {
    let mut best: Option<(ClipTime, &Channels)> = None;
    for time in shown {
        let Some(found) = clip(asset, time.clip).and_then(|c| {
            c.targets
                .iter()
                .find(|t| t.target == target && has(&t.channels))
        }) else {
            continue;
        };
        if best.is_none_or(|(held, _)| beats(time, &held)) {
            best = Some((*time, &found.channels));
        }
    }
    best
}

fn read(
    track: &Track,
    time: &ClipTime,
    channel: Channel,
    gates: &Gates,
    sampler: Sampler<'_>,
) -> i32 {
    let held = gates.hold_reactions
        && channel != Channel::Frame
        && matches!(time.clip, ClipRef::Reaction(_));
    let frame = if held { 0 } else { time.frame };
    channel.clamp(sample(track, frame, &|program| {
        sampler(program, frame, channel)
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Own {
    dx: i32,
    dy: i32,
    opacity: i32,
    visible: bool,
}

fn position(
    asset: &NameplateAsset,
    shown: &[ClipTime],
    target: Target,
    gates: &Gates,
    sampler: Sampler<'_>,
) -> (i32, i32) {
    match winner(asset, shown, target, |c| c.x.is_some() || c.y.is_some()) {
        Some((time, channels)) => {
            let axis = |track: &Option<Track>| {
                track
                    .as_ref()
                    .map_or(0, |t| read(t, &time, Channel::Position, gates, sampler))
            };
            (axis(&channels.x), axis(&channels.y))
        }
        None => (0, 0),
    }
}

fn own(
    asset: &NameplateAsset,
    shown: &[ClipTime],
    target: Target,
    gates: &Gates,
    sampler: Sampler<'_>,
) -> Own {
    let (dx, dy) = position(asset, shown, target, gates, sampler);
    let opacity = winner(asset, shown, target, |c| c.opacity.is_some())
        .and_then(|(time, c)| {
            let track = c.opacity.as_ref()?;
            Some(read(track, &time, Channel::Opacity, gates, sampler))
        })
        .unwrap_or(i32::from(OPAQUE));
    let visible = winner(asset, shown, target, |c| c.visible.is_some())
        .and_then(|(time, c)| {
            let track = c.visible.as_ref()?;
            Some(read(track, &time, Channel::Visible, gates, sampler) != 0)
        })
        .unwrap_or(true);
    Own {
        dx,
        dy,
        opacity,
        visible,
    }
}

fn multiply(a: i32, b: i32) -> i32 {
    (a * b + 127) / i32::from(OPAQUE)
}

fn group_of(asset: &NameplateAsset, layer: u8) -> Option<usize> {
    asset.groups.iter().position(|g| g.layers.contains(&layer))
}

fn chain(asset: &NameplateAsset, layer: u8) -> impl Iterator<Item = usize> + '_ {
    let mut next = group_of(asset, layer);
    let mut steps = 0;
    std::iter::from_fn(move || {
        let current = next?;
        steps += 1;
        if steps > asset.groups.len() {
            return None;
        }
        next = asset
            .groups
            .get(current)
            .and_then(|g| g.parent)
            .map(usize::from)
            .filter(|p| *p < asset.groups.len());
        Some(current)
    })
}

fn placed(
    asset: &NameplateAsset,
    shown: &[ClipTime],
    layer: u8,
    gates: &Gates,
    sampler: Sampler<'_>,
) -> Own {
    let mut total = own(asset, shown, Target::Layer(layer), gates, sampler);
    for group in chain(asset, layer) {
        let g = own(asset, shown, Target::Group(group as u8), gates, sampler);
        total.dx += g.dx;
        total.dy += g.dy;
        total.opacity = multiply(total.opacity, g.opacity);
        total.visible &= g.visible;
    }
    if let Some(Of::Layer(body)) = asset.layers.get(usize::from(layer)).map(|l| l.of) {
        if body != layer && usize::from(body) < asset.layers.len() {
            let (mut dx, mut dy) = position(asset, shown, Target::Layer(body), gates, sampler);
            for group in chain(asset, body) {
                if chain(asset, layer).any(|g| g == group) {
                    break;
                }
                let (gx, gy) = position(asset, shown, Target::Group(group as u8), gates, sampler);
                dx += gx;
                dy += gy;
            }
            total.dx += dx;
            total.dy += dy;
        }
    }
    total
}

fn disabled_bit(disabled: &[u32], layer: usize) -> bool {
    disabled
        .get(layer / 32)
        .is_some_and(|word| word & (1 << (layer % 32)) != 0)
}

fn loop_frame_of(shown: &[ClipTime]) -> u16 {
    shown
        .iter()
        .find(|t| t.clip == ClipRef::Loop)
        .map_or(0, |t| t.frame.clamp(0, i32::from(u16::MAX)) as u16)
}

fn sprite_frames(asset: &NameplateAsset, layer: usize) -> Option<usize> {
    let LayerContent::Sprite(sprite) = &asset.layers.get(layer)?.content else {
        return None;
    };
    Some(
        asset
            .sprites
            .get(usize::from(sprite.sprite))
            .map_or(1, |s| s.frames.len().max(1)),
    )
}

pub fn sprite_frame(asset: &NameplateAsset, layer: usize, loop_frame: u16) -> u16 {
    let Some(frames) = sprite_frames(asset, layer) else {
        return 0;
    };
    let entry = &asset.layers[layer];
    let LayerContent::Sprite(sprite) = &entry.content else {
        return 0;
    };
    let stepped = u32::from(loop_frame) / u32::from(entry.step.max(1));
    let last = (frames - 1) as u16;
    if sprite.timeline.is_empty() {
        return (stepped % frames as u32) as u16;
    }
    let total: u32 = sprite.timeline.iter().map(|h| u32::from(h.hold)).sum();
    if total == 0 {
        return sprite.timeline[0].frame.min(last);
    }
    let mut at = stepped % total;
    for hold in &sprite.timeline {
        let span = u32::from(hold.hold);
        if at < span {
            return hold.frame.min(last);
        }
        at -= span;
    }
    0
}

fn clamp_i16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn clamp_i8(v: i32) -> i8 {
    v.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8
}

fn alpha(v: i32) -> u8 {
    v.clamp(0, i32::from(OPAQUE)) as u8
}

pub fn pose_layers(
    asset: &NameplateAsset,
    _variant: &Variant,
    shown: &[ClipTime],
    sampler: Sampler<'_>,
    gates: &Gates,
    disabled: &[u32],
    out: &mut TagPose,
) {
    let loop_frame = loop_frame_of(shown);
    out.layers = [LayerPose::IDENTITY; MAX_LAYERS];
    for (index, layer) in asset.layers.iter().enumerate().take(MAX_LAYERS) {
        let id = index as u8;
        let total = placed(asset, shown, id, gates, sampler);
        let state = match sprite_frames(asset, index) {
            Some(frames) => winner(asset, shown, Target::Layer(id), |c| c.frame.is_some())
                .and_then(|(time, c)| {
                    let track = c.frame.as_ref()?;
                    Some(read(track, &time, Channel::Frame, gates, sampler))
                })
                .map_or_else(
                    || sprite_frame(asset, index, loop_frame),
                    |f| f.clamp(0, frames as i32 - 1) as u16,
                ),
            None => 0,
        };
        let particles = !matches!(layer.content, LayerContent::Emitter(_)) || gates.particles;
        out.layers[index] = LayerPose {
            dx: clamp_i16(total.dx),
            dy: clamp_i16(total.dy),
            opacity: alpha(total.opacity),
            visible: total.visible && particles && !disabled_bit(disabled, index),
            state,
        };
    }
    out.plate = if gates.plate {
        let (dx, dy) = position(asset, shown, Target::Plate, gates, sampler);
        (clamp_i8(dx), clamp_i8(dy))
    } else {
        (0, 0)
    };
    let badge = own(asset, shown, Target::Badge, gates, sampler);
    out.badge = BadgePose {
        dx: clamp_i16(badge.dx),
        dy: clamp_i16(badge.dy),
        opacity: alpha(badge.opacity),
        visible: badge.visible,
    };
}

pub fn pose_tag(
    asset: &NameplateAsset,
    variant: &Variant,
    time: &TagTime,
    playing: &[ClipTime],
    values: &dyn Fn(u32) -> i32,
    motion: Motion,
    out: &mut TagPose,
) {
    let gates = Gates::new(motion, Detail::Full);
    let mut kept = Vec::with_capacity(playing.len() + 1);
    shown(asset, playing, &gates, &mut kept);
    if !kept.iter().any(|t| t.clip == ClipRef::Loop) {
        let frame = if gates.first_frame {
            0
        } else {
            i32::from(time.loop_frame)
        };
        kept.insert(
            0,
            ClipTime {
                clip: ClipRef::Loop,
                frame,
                start: 0,
            },
        );
    }
    let sampler = |program: u32, _frame: i32, channel: Channel| channel.clamp(values(program));
    pose_layers(asset, variant, &kept, &sampler, &gates, &[], out);
}

pub fn letters(
    asset: &NameplateAsset,
    variant: &Variant,
    shown: &[ClipTime],
    gates: &Gates,
) -> Option<(u32, i32)> {
    if !gates.letters {
        return None;
    }
    let mut best: Option<(ClipTime, LetterOverride)> = None;
    for time in shown {
        let Some(letters) = clip(asset, time.clip).and_then(|c| c.letters) else {
            continue;
        };
        if best.is_none_or(|(held, _)| beats(time, &held)) {
            best = Some((*time, letters));
        }
    }
    match best {
        Some((_, LetterOverride::Off)) => None,
        Some((time, LetterOverride::Program(program))) => Some((program, time.frame)),
        None => variant
            .text
            .motion
            .map(|program| (program, i32::from(loop_frame_of(shown)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::anim::fixture::{asset, body, emitter, frames, sprite};
    use crate::nameplate::types::{Animations, Clip, Ease, Key, OneShot, Reaction, Schedule, ScheduledClip, TargetTracks, Trigger, Value};

    const GEM: Target = Target::Layer(1);

    fn clip(length: u16, tracks: &[(Target, i32, &[(u16, i32)], &[(u16, i32)])]) -> Clip {
        let fixed = |keys: &[(u16, i32)]| {
            let keys = keys.iter().map(|&(frame, v)| Key { frame, value: Value::Fixed(v), ease: Ease::Step });
            (keys.len() != 0).then(|| Track { keys: keys.collect() })
        };
        let targets = tracks.iter().map(|&(target, x, opacity, frame)| TargetTracks {
            target,
            channels: Channels { x: Some(fixed(&[(0, x)]).unwrap()), y: fixed(&[(0, -x)]), opacity: fixed(opacity), frame: fixed(frame), ..Channels::default() },
        });
        Clip { name: "c".into(), length, targets: targets.collect(), ..Clip::default() }
    }

    fn rich() -> NameplateAsset {
        let reaction = |signal, clip, reduced| Reaction { signal, clip, reduced };
        let shot = |clip| OneShot { trigger: Trigger::Equip, clip, restart: false };
        let animations = Animations {
            looping: clip(24, &[(GEM, 1, &[], &[])]),
            schedules: vec![Schedule { window: 8, chance: 1 << 24, events: vec![ScheduledClip { clip: clip(4, &[(GEM, 2, &[(0, 100)], &[])]), weight: 1 }], ..Schedule::default() }],
            reactions: vec![
                reaction(0, clip(24, &[(GEM, 5, &[(0, 10), (2, 20)], &[(0, 1), (2, 2)])]), Reduced::Hold),
                reaction(1, clip(24, &[(GEM, 7, &[], &[(0, 3)])]), Reduced::Drop),
            ],
            one_shots: vec![shot(clip(6, &[(GEM, 11, &[], &[])])), shot(clip(6, &[(GEM, 13, &[], &[]), (Target::Plate, 3, &[], &[])]))],
        };
        asset(12, 24, vec![body("plate"), sprite("gem", 0, 2), emitter("sparks", 4)], vec![frames("gem", 4)], animations)
    }

    fn pose_with(asset: &NameplateAsset, playing: &[ClipTime], gates: &Gates) -> TagPose {
        let mut kept = Vec::new();
        shown(asset, playing, gates, &mut kept);
        let mut out = TagPose::default();
        pose_layers(asset, &asset.variants[0], &kept, &|_, _, c: Channel| c.convert(0.0), gates, &[0], &mut out);
        out
    }

    #[test]
    fn channels_run_one_shot_reaction_schedule_loop_and_follow_the_motion_gates() {
        let a = rich();
        let at = |clip, frame, start| ClipTime { clip, frame, start };
        let gem = |playing: &[ClipTime], gates: &Gates| {
            let p = pose_with(&a, playing, gates);
            (p.layers[1].dx, p.layers[1].opacity, p.layers[1].state, p.plate)
        };
        let full = Gates::default();
        let looped = at(ClipRef::Loop, 4, 0);
        let scheduled = at(ClipRef::Schedule { schedule: 0, event: 0 }, 1, 50);
        let (r0, r1) = (at(ClipRef::Reaction(0), 4, 30), at(ClipRef::Reaction(1), 4, 40));
        let newer = at(ClipRef::OneShot(0), 0, 12);
        assert_eq!(gem(&[looped], &full), (1, 255, 2, (0, 0)));
        assert_eq!(gem(&[looped, scheduled], &full).0, 2);
        assert_eq!(gem(&[looped, scheduled, r1, r0], &full), (5, 20, 2, (0, 0)), "declaration order");
        let (dx, opacity, state, _) = gem(&[looped, scheduled, r1], &full);
        assert_eq!((dx, opacity, state), (7, 100, 3));
        assert_eq!(gem(&[looped, r0, at(ClipRef::OneShot(1), 2, 10), newer], &full).0, 11, "newest start wins");
        assert_eq!(gem(&[looped, r0, at(ClipRef::OneShot(1), 2, 12), newer], &full).0, 11, "ties go to the lower index");
        let shot = at(ClipRef::OneShot(1), 0, 14);
        assert_eq!(gem(&[looped, r0, shot, newer], &full), (13, 20, 2, (3, -3)));
        let reduced = Gates::new(Motion::Reduced, Detail::Full);
        let mut kept = Vec::new();
        shown(&a, &[looped, r1, r0, shot], &reduced, &mut kept);
        assert_eq!(kept, [looped, r0, shot], "drop reactions go, hold reactions stay");
        assert_eq!(gem(&[looped, r0, shot], &reduced), (13, 10, 2, (0, 0)), "held at the first key, no plate motion");
        let off = Gates::new(Motion::Off, Detail::Full);
        shown(&a, &[looped, scheduled, r1, r0, shot], &off, &mut kept);
        assert_eq!(kept, [at(ClipRef::Loop, 0, 0)]);
        assert!(pose_with(&a, &[looped], &full).layers[2].visible);
        assert!(!pose_with(&a, &[looped], &Gates::new(Motion::Full, Detail::Reduced)).layers[2].visible, "particles drop with detail");
        assert_eq!((0..8).map(|f| sprite_frame(&a, 1, f).to_string()).collect::<String>(), "00112233");
    }
}
