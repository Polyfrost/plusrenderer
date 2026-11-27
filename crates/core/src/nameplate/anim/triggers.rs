use crate::limits::{NAMEPLATE_APPEAR_SECONDS, NAMEPLATE_PARTICLE_LIFETIME};
use crate::nameplate::types::{
    Animations, Clip, LayerContent, NameplateAsset, OneShot, Target, Trigger,
};
use crate::signals::{SignalState, JUMP_SIGNAL};
use crate::wearer::{WearerRecord, ONE_SHOT_EMPTY, REBASE_SECONDS, TIME_NEVER};

use super::schedule::{event_at, roll};
use super::speed::{episode_floats, write_episode};
use super::{ClipRef, ClipTime, Moment, Tail};

const UNSEEN_SLACK: f32 = 1e-3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub previous: Option<f32>,
}

impl Edges {
    pub fn edge(&self, signals: &SignalState, signal: u8) -> Option<bool> {
        let id = usize::from(signal);
        let changed = signals.changed_at(id).filter(|_| id != JUMP_SIGNAL)?;
        let previous = self.previous?;
        (changed > previous).then(|| signals.value(usize::from(signal)) >= 0.5)
    }
}

fn has_speed(clip: &Clip) -> bool {
    clip.targets.iter().any(|t| t.channels.speed.is_some())
}

pub fn speed_count(asset: &NameplateAsset) -> usize {
    let animations = &asset.animations;
    let reactions = animations.reactions.iter().map(|r| &r.clip);
    let shots = animations.one_shots.iter().map(|o| &o.clip);
    reactions.chain(shots).filter(|c| has_speed(c)).count()
}

pub fn equip_slot(asset: &NameplateAsset) -> Option<usize> {
    asset
        .animations
        .one_shots
        .iter()
        .any(|o| o.trigger == Trigger::Equip)
        .then(|| 2 * speed_count(asset))
}

pub fn burst_tail(asset: &NameplateAsset, clip: &Clip) -> i64 {
    clip.targets
        .iter()
        .filter_map(|t| {
            let Target::Layer(layer) = t.target else {
                return None;
            };
            let LayerContent::Emitter(emitter) = &asset.layers.get(usize::from(layer))?.content
            else {
                return None;
            };
            let last = t.channels.bursts.iter().map(|b| b.frame).max()?;
            Some(i64::from(last) + i64::from(emitter.lifetime[1]) + 1)
        })
        .max()
        .unwrap_or(0)
}

pub fn variant_changed(
    asset: &NameplateAsset,
    record: &mut WearerRecord<'_>,
    variant: u16,
) -> bool {
    let appear = record.appear;
    let Some(last) = equip_slot(asset).and_then(|slot| record.effect_floats.get_mut(slot)) else {
        return false;
    };
    let current = f32::from(variant) + 1.0;
    let before = *last;
    *last = current;
    !appear && before != 0.0 && before != current
}

fn previous(record: &WearerRecord<'_>) -> Option<f32> {
    if record.times_reset {
        return None;
    }
    let mut prev = f32::from_bits((*record.held)? as u32);
    if !prev.is_finite() {
        return None;
    }
    let mut rebases = 0;
    while prev > record.now && rebases < 4 {
        prev -= REBASE_SECONDS;
        rebases += 1;
    }
    Some(prev)
}

fn start_of(record: &WearerRecord<'_>, moment: &Moment, slot: usize) -> i64 {
    moment.frame_of(record.now, record.one_shot_starts[slot])
}

fn has_bursts(clip: &Clip) -> bool {
    clip.targets.iter().any(|t| !t.channels.bursts.is_empty())
}

const TAIL_TIMES: usize = 2;
pub const KEPT_RUNS: usize = 4;
const REACTION_TIMES: usize = 1 + 2 * KEPT_RUNS;
const SCHEDULE_TAIL_WINDOWS: i64 = NAMEPLATE_PARTICLE_LIFETIME as i64 + 2;

fn shot_slots(animations: &Animations) -> usize {
    TAIL_TIMES
        * animations
            .one_shots
            .iter()
            .filter(|o| has_bursts(&o.clip))
            .count()
}

pub fn tail_slots(animations: &Animations) -> usize {
    shot_slots(animations)
        + REACTION_TIMES
            * animations
                .reactions
                .iter()
                .filter(|r| has_bursts(&r.clip))
                .count()
}

fn reaction_slot(asset: &NameplateAsset, index: usize) -> Option<usize> {
    let animations = &asset.animations;
    let reactions = &animations.reactions;
    has_bursts(&reactions.get(index)?.clip).then(|| {
        shot_slots(animations)
            + REACTION_TIMES
                * reactions[..index]
                    .iter()
                    .filter(|r| has_bursts(&r.clip))
                    .count()
    })
}

pub fn burst_life(asset: &NameplateAsset, clip: &Clip) -> i64 {
    clip.targets
        .iter()
        .filter(|t| !t.channels.bursts.is_empty())
        .filter_map(|t| {
            let Target::Layer(layer) = t.target else {
                return None;
            };
            match &asset.layers.get(usize::from(layer))?.content {
                LayerContent::Emitter(emitter) => Some(i64::from(emitter.lifetime[1]) + 1),
                _ => None,
            }
        })
        .max()
        .unwrap_or(0)
}

fn keep_reaction_tails(asset: &NameplateAsset, record: &mut WearerRecord<'_>, moment: &Moment) {
    let signals = *record.signals;
    let now = record.now;
    let reset = record.times_reset;
    for (index, reaction) in asset.animations.reactions.iter().enumerate() {
        let Some(k) = reaction_slot(asset, index) else {
            continue;
        };
        let Some(kept) = record.effect_times.get_mut(k..k + REACTION_TIMES) else {
            continue;
        };
        if reset {
            kept.fill(TIME_NEVER);
        }
        let id = usize::from(reaction.signal);
        let changed = signals.changed_at(id).unwrap_or(now);
        let running = kept[0];
        let mut ended = None;
        if signals.value(id) >= 0.5 {
            if running.is_finite() && running != changed {
                ended = Some((running, changed));
            }
            kept[0] = changed;
        } else if running.is_finite() {
            ended = Some((running, changed));
            kept[0] = TIME_NEVER;
        }
        let life = burst_life(asset, &reaction.clip);
        let mut runs = [(TIME_NEVER, TIME_NEVER); KEPT_RUNS];
        let mut count = 0;
        let earlier = kept[1..].chunks_exact(2).map(|pair| (pair[0], pair[1]));
        for (began, stopped) in ended.into_iter().chain(earlier) {
            let live = began.is_finite()
                && stopped.is_finite()
                && moment.live - moment.frame_of(now, stopped) < life;
            if live && count < KEPT_RUNS {
                runs[count] = (began, stopped);
                count += 1;
            }
        }
        for (pair, (began, stopped)) in kept[1..].chunks_exact_mut(2).zip(runs) {
            pair[0] = began;
            pair[1] = stopped;
        }
    }
}

pub fn schedule_tails(
    asset: &NameplateAsset,
    seed: u64,
    engine: i64,
    absolute: i64,
    out: &mut Vec<Tail>,
) {
    for (index, schedule) in asset.animations.schedules.iter().enumerate() {
        let window = i64::from(schedule.window);
        let longest = schedule
            .events
            .iter()
            .map(|e| burst_tail(asset, &e.clip))
            .max()
            .unwrap_or(0);
        if window == 0 || longest == 0 {
            continue;
        }
        let current = engine.div_euclid(window);
        let first = (engine - longest)
            .div_euclid(window)
            .max(current - SCHEDULE_TAIL_WINDOWS);
        for w in first..=current {
            let Some(event) = roll(schedule, seed, w) else {
                continue;
            };
            let Some(clip) = schedule
                .events
                .get(usize::from(event.event))
                .map(|e| &e.clip)
            else {
                continue;
            };
            let local = engine - event.start;
            let end = (event.start + i64::from(clip.length)).min((w + 1) * window);
            if local < 0 || engine < end || local >= burst_tail(asset, clip) {
                continue;
            }
            out.push(Tail {
                time: ClipTime {
                    clip: ClipRef::Schedule {
                        schedule: index as u16,
                        event: event.event,
                    },
                    frame: local.clamp(0, i64::from(i32::MAX)) as i32,
                    start: absolute - local,
                },
                until: end - event.start,
            });
        }
    }
}

fn tail_slot(asset: &NameplateAsset, index: usize) -> Option<usize> {
    let shots = &asset.animations.one_shots;
    has_bursts(&shots.get(index)?.clip).then(|| TAIL_TIMES * tail_slots_before(shots, index))
}

fn tail_slots_before(shots: &[OneShot], index: usize) -> usize {
    shots[..index]
        .iter()
        .filter(|o| has_bursts(&o.clip))
        .count()
}

fn tail_times<'a>(
    asset: &NameplateAsset,
    record: &'a WearerRecord<'_>,
    index: usize,
) -> Option<&'a [f32]> {
    let k = tail_slot(asset, index)?;
    record.effect_times.get(k..k + TAIL_TIMES)
}

fn retire(asset: &NameplateAsset, record: &mut WearerRecord<'_>, moment: &Moment, slot: usize) {
    let id = record.one_shot_ids[slot];
    let Some(shot) = asset.animations.one_shots.get(id as usize) else {
        return;
    };
    let began = start_of(record, moment, slot);
    let age = moment.live - began;
    if !(0..burst_tail(asset, &shot.clip)).contains(&age) {
        return;
    }
    let start = record.one_shot_starts[slot];
    let ended = moment.live.min(began + i64::from(shot.clip.length));
    let retired = moment.stamp(record.now, ended);
    let Some(k) = tail_slot(asset, id as usize) else {
        return;
    };
    if let Some(kept) = record.effect_times.get_mut(k..k + TAIL_TIMES) {
        kept.copy_from_slice(&[start, retired]);
    }
}

fn prune(asset: &NameplateAsset, record: &mut WearerRecord<'_>, moment: &Moment) {
    for slot in 0..record.one_shot_ids.len() {
        let id = record.one_shot_ids[slot];
        if id == ONE_SHOT_EMPTY {
            continue;
        }
        let age = moment.live - start_of(record, moment, slot);
        let running = asset
            .animations
            .one_shots
            .get(id as usize)
            .is_some_and(|shot| (0..i64::from(shot.clip.length)).contains(&age));
        if !running {
            retire(asset, record, moment, slot);
            record.one_shot_ids[slot] = ONE_SHOT_EMPTY;
        }
    }
    for (index, shot) in asset.animations.one_shots.iter().enumerate() {
        let Some(k) = tail_slot(asset, index) else {
            continue;
        };
        let Some(kept) = record.effect_times.get_mut(k..k + TAIL_TIMES) else {
            continue;
        };
        let start = kept[0];
        let age = moment.live - moment.frame_of(record.now, start);
        if !start.is_finite() || !(0..burst_tail(asset, &shot.clip)).contains(&age) {
            kept.fill(TIME_NEVER);
        }
    }
}

fn oldest(record: &WearerRecord<'_>, moment: &Moment) -> usize {
    (0..record.one_shot_ids.len())
        .min_by_key(|&slot| {
            (
                start_of(record, moment, slot),
                record.one_shot_ids[slot],
                slot,
            )
        })
        .unwrap_or(0)
}

fn fire_shot(
    asset: &NameplateAsset,
    record: &mut WearerRecord<'_>,
    moment: &Moment,
    index: u16,
) -> bool {
    let Some(shot) = asset.animations.one_shots.get(usize::from(index)) else {
        return false;
    };
    if record.one_shot_ids.is_empty() {
        return false;
    }
    let id = u32::from(index);
    let length = i64::from(shot.clip.length);
    let stamp = moment.stamp(record.now, moment.live);
    if let Some(slot) = record.one_shot_ids.iter().position(|i| *i == id) {
        let age = moment.live - start_of(record, moment, slot);
        if age < length && !shot.restart {
            return false;
        }
        retire(asset, record, moment, slot);
        record.one_shot_starts[slot] = stamp;
        return true;
    }
    let slot = match record
        .one_shot_ids
        .iter()
        .position(|i| *i == ONE_SHOT_EMPTY)
    {
        Some(slot) => slot,
        None => {
            let slot = oldest(record, moment);
            retire(asset, record, moment, slot);
            slot
        }
    };
    record.one_shot_ids[slot] = id;
    record.one_shot_starts[slot] = stamp;
    true
}

pub fn fire(
    asset: &NameplateAsset,
    record: &mut WearerRecord<'_>,
    moment: &Moment,
    index: u16,
) -> bool {
    prune(asset, record, moment);
    fire_shot(asset, record, moment, index)
}

pub fn update(
    asset: &NameplateAsset,
    record: &mut WearerRecord<'_>,
    moment: &Moment,
    equipped: bool,
    playing: &mut Vec<ClipTime>,
) -> Edges {
    playing.clear();
    let animations = &asset.animations;
    if record.times_reset {
        for index in 0..speed_count(asset) {
            if let Some(floats) = record.effect_floats.get_mut(episode_floats(index)) {
                write_episode(floats, None);
            }
        }
    }
    let edges = Edges {
        previous: previous(record),
    };
    prune(asset, record, moment);
    keep_reaction_tails(asset, record, moment);
    let signals = *record.signals;
    let on = |signal: u8| signals.value(usize::from(signal)) >= 0.5;
    let edge = |signal: u8| edges.edge(&signals, signal);
    let unseen = edges.previous.map(|p| record.now - p);
    let cap = NAMEPLATE_APPEAR_SECONDS as u8;
    for (index, shot) in animations.one_shots.iter().enumerate() {
        let go = match shot.trigger {
            Trigger::Appear(seconds) => {
                record.appear
                    || unseen.is_some_and(|u| u + UNSEEN_SLACK >= f32::from(seconds.min(cap)))
            }
            Trigger::Equip => equipped,
            Trigger::Start(signal) => edge(signal) == Some(true),
            Trigger::Stop(signal) => edge(signal) == Some(false),
        };
        if go {
            fire_shot(asset, record, moment, index as u16);
        }
    }
    *record.held = Some(i64::from(record.now.to_bits()));

    let time = moment.tag;
    playing.push(ClipTime {
        clip: ClipRef::Loop,
        frame: i32::from(time.loop_frame),
        start: 0,
    });
    for (index, schedule) in animations.schedules.iter().enumerate() {
        if let Some(event) = event_at(schedule, time.seed, i64::from(time.engine)) {
            let local = i64::from(time.engine) - event.start;
            playing.push(ClipTime {
                clip: ClipRef::Schedule {
                    schedule: index as u16,
                    event: event.event,
                },
                frame: local as i32,
                start: moment.absolute - local,
            });
        }
    }
    for (index, reaction) in animations.reactions.iter().enumerate() {
        if !on(reaction.signal) {
            continue;
        }
        let changed = signals
            .changed_at(usize::from(reaction.signal))
            .unwrap_or(record.now);
        playing.push(ClipTime {
            clip: ClipRef::Reaction(index as u16),
            frame: i32::from(time.loop_frame),
            start: moment.frame_of(record.now, changed),
        });
    }
    let first = playing.len();
    for slot in 0..record.one_shot_ids.len() {
        let id = record.one_shot_ids[slot];
        if id == ONE_SHOT_EMPTY {
            continue;
        }
        let Some(shot) = animations.one_shots.get(id as usize) else {
            continue;
        };
        let start = start_of(record, moment, slot);
        if moment.live - start >= i64::from(shot.clip.length) {
            continue;
        }
        playing.push(ClipTime {
            clip: ClipRef::OneShot(id as u16),
            frame: (moment.absolute - start).clamp(0, i64::from(i32::MAX)) as i32,
            start,
        });
    }
    playing[first..].sort_unstable_by_key(|t| match t.clip {
        ClipRef::OneShot(index) => (index, t.start),
        _ => (u16::MAX, t.start),
    });
    edges
}

pub fn tails(
    asset: &NameplateAsset,
    record: &WearerRecord<'_>,
    moment: &Moment,
    out: &mut Vec<Tail>,
) {
    out.clear();
    for index in 0..asset.animations.one_shots.len() {
        let Some(&[kept, retired]) = tail_times(asset, record, index) else {
            continue;
        };
        if !kept.is_finite() {
            continue;
        }
        let start = moment.frame_of(record.now, kept);
        let until = if retired.is_finite() {
            moment.frame_of(record.now, retired) - start
        } else {
            i64::MAX
        };
        out.push(Tail {
            time: ClipTime {
                clip: ClipRef::OneShot(index as u16),
                frame: (moment.absolute - start).clamp(0, i64::from(i32::MAX)) as i32,
                start,
            },
            until,
        });
    }
    for index in 0..asset.animations.reactions.len() {
        let Some(k) = reaction_slot(asset, index) else {
            continue;
        };
        let Some(kept) = record.effect_times.get(k + 1..k + REACTION_TIMES) else {
            continue;
        };
        for pair in kept.chunks_exact(2) {
            let (began, ended) = (pair[0], pair[1]);
            if !began.is_finite() || !ended.is_finite() {
                continue;
            }
            let start = moment.frame_of(record.now, began);
            out.push(Tail {
                time: ClipTime {
                    clip: ClipRef::Reaction(index as u16),
                    frame: (moment.absolute - start).clamp(0, i64::from(i32::MAX)) as i32,
                    start,
                },
                until: moment.frame_of(record.now, ended) - start,
            });
        }
    }
    let time = moment.tag;
    schedule_tails(
        asset,
        time.seed,
        i64::from(time.engine),
        moment.absolute,
        out,
    );
    out.sort_unstable_by_key(|t| (t.time.start, t.time.frame));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::AnimationRate;
    use crate::nameplate::anim::fixture::{asset, body, emitter};
    use crate::nameplate::types::{Animations, Burst, Channels, OneShot, Phase, Reaction, Reduced, Target, TargetTracks, Track};
    use crate::signals::{signal_id, SIGNAL_COUNT};
    use crate::wearer::{CosmeticSlot, WearerTable};

    fn sig(name: &str) -> u8 {
        signal_id(name).unwrap() as u8
    }

    fn shot(trigger: Trigger, length: u16, restart: bool, bursts: bool, speed: bool) -> OneShot {
        let channels = Channels { bursts: if bursts { vec![Burst { frame: 0, count: 3 }] } else { Vec::new() }, speed: speed.then(Track::default), ..Channels::default() };
        let targets = vec![TargetTracks { target: Target::Layer(1), channels }];
        OneShot { trigger, clip: Clip { name: "shot".into(), length, targets, ..Clip::default() }, restart }
    }

    fn reaction(signal: &str) -> Reaction {
        Reaction { signal: sig(signal), clip: Clip { name: "while".into(), length: 24, ..Clip::default() }, reduced: Reduced::Hold }
    }

    struct Rig { asset: NameplateAsset, table: WearerTable, slot: CosmeticSlot, raw: [f32; SIGNAL_COUNT], variant: u16, time: Moment }

    impl Rig {
        fn new(reactions: Vec<Reaction>, one_shots: Vec<OneShot>) -> Rig {
            let animations = Animations { reactions, one_shots, ..Animations::default() };
            let asset = asset(12, 24, vec![body("plate"), emitter("sparks", 5)], Vec::new(), animations);
            let (mut table, raw) = (WearerTable::new(), [0.0; SIGNAL_COUNT]);
            Rig { slot: table.add_cosmetic(asset.record).unwrap(), asset, table, raw, variant: 0, time: Moment::default() }
        }

        fn set(&mut self, signal: &str, on: bool) {
            self.raw[usize::from(sig(signal))] = f32::from(u8::from(on));
        }

        fn draw(&mut self, frame: i64) -> Vec<ClipTime> {
            let clock = (frame as f64 + 0.5) / 12.0;
            self.table.begin_frame(clock);
            self.time = Moment::new(clock, AnimationRate::EveryFrame, 12, 24, Phase::Global, 9);
            let mut rec = self.table.draw(self.slot, 77, &self.raw, 0).unwrap();
            let equipped = variant_changed(&self.asset, &mut rec, self.variant);
            let mut playing = Vec::new();
            update(&self.asset, &mut rec, &self.time, equipped, &mut playing);
            playing
        }

        fn shots(&mut self, frame: i64) -> Vec<(u16, i32)> {
            let mut shots: Vec<_> = self.draw(frame).iter().filter_map(|t| match t.clip { ClipRef::OneShot(i) => Some((i, t.frame)), _ => None }).collect();
            shots.sort_unstable();
            shots
        }

        fn tails(&mut self) -> Vec<(ClipRef, i32)> {
            let mut out = Vec::new();
            tails(&self.asset, &self.table.record(self.slot, 77).unwrap(), &self.time, &mut out);
            out.iter().map(|t| (t.time.clip, t.time.frame)).collect()
        }
    }

    #[test]
    fn appear_and_equip_fire_on_sight_and_change_and_a_rewound_clock_refires_neither() {
        let hurt = sig("is_hurt");
        let shots = vec![shot(Trigger::Appear(5), 6, false, false, false), shot(Trigger::Equip, 4, false, false, false), shot(Trigger::Start(hurt), 30, false, false, false)];
        let mut rig = Rig::new(vec![reaction("is_hurt")], shots);
        let base = 100_000;
        assert_eq!(rig.shots(base), [(0, 0)], "appear on first sight, no equip");
        assert_eq!(rig.shots(base + 6), []);
        assert_eq!(rig.shots(base + 6 + 12 * 4), [], "4 s unseen");
        assert_eq!(rig.shots(base + 6 + 12 * 9), [(0, 0)], "5 s unseen");
        rig.variant = 1;
        assert_eq!(rig.shots(base + 120), [(1, 0)], "a new variant equips");
        rig.set("is_hurt", true);
        assert_eq!(rig.shots(base + 122), [(1, 2), (2, 0)]);
        let rewound = base - 500;
        let playing = rig.draw(rewound);
        assert!(playing.iter().all(|t| !matches!(t.clip, ClipRef::OneShot(_))), "{playing:?}");
        assert_eq!(playing.iter().find(|t| t.clip == ClipRef::Reaction(0)).unwrap().start, rig.time.live);
        assert!(rig.table.record(rig.slot, 77).unwrap().one_shot_ids.iter().all(|id| *id == ONE_SHOT_EMPTY));
        assert_eq!(rig.shots(rewound + 1), [], "no appear, no edge");
        rig.set("is_hurt", false);
        rig.draw(rewound + 2);
        rig.set("is_hurt", true);
        assert_eq!(rig.shots(rewound + 3), [(2, 0)]);
    }

    #[test]
    fn one_shots_cap_at_eight_restart_into_tails_and_stop_edges_fire() {
        let (hurt, sneak) = (sig("is_hurt"), sig("is_sneaking"));
        let mut shots: Vec<OneShot> = (0..10).map(|_| shot(Trigger::Start(hurt), 50, false, false, false)).collect();
        shots.push(shot(Trigger::Stop(sneak), 50, false, false, false));
        let mut rig = Rig::new(vec![reaction("is_sneaking")], shots);
        assert_eq!(rig.asset.record.one_shots, 8);
        rig.set("is_sneaking", true);
        rig.draw(10);
        rig.set("is_hurt", true);
        let ids: Vec<u16> = rig.shots(11).iter().map(|(i, _)| *i).collect();
        assert_eq!(ids, [2, 3, 4, 5, 6, 7, 8, 9], "ties evict the earlier fired");
        rig.set("is_sneaking", false);
        assert_eq!(rig.shots(13), [(3, 2), (4, 2), (5, 2), (6, 2), (7, 2), (8, 2), (9, 2), (10, 0)]);

        let mut rig = Rig::new(Vec::new(), vec![shot(Trigger::Start(hurt), 10, false, false, false), shot(Trigger::Start(hurt), 10, true, true, false)]);
        assert_eq!((rig.asset.record.one_shots, rig.asset.record.effect_times), (2, 2));
        rig.draw(0);
        rig.set("is_hurt", true);
        assert_eq!(rig.shots(1), [(0, 0), (1, 0)]);
        rig.set("is_hurt", false);
        rig.draw(3);
        rig.set("is_hurt", true);
        assert_eq!(rig.shots(4), [(0, 3), (1, 0)], "only the restart shot starts over");
        assert_eq!(rig.tails(), [(ClipRef::OneShot(1), 3)], "the cut run's particles live on");
        assert_eq!(burst_tail(&rig.asset, &rig.asset.animations.one_shots[1].clip), 6);
        rig.draw(7);
        assert!(rig.tails().is_empty());
    }

    #[test]
    fn effect_slots_follow_the_declared_layout() {
        let hurt = sig("is_hurt");
        let dash = Reaction { clip: shot(Trigger::Equip, 24, false, false, true).clip, ..reaction("is_hurt") };
        let rig = Rig::new(vec![reaction("is_sneaking"), dash], vec![shot(Trigger::Equip, 2, false, false, false), shot(Trigger::Start(hurt), 30, false, false, true)]);
        assert_eq!((rig.asset.record.effect_floats, speed_count(&rig.asset), equip_slot(&rig.asset)), (5, 2, Some(4)));
    }
}
