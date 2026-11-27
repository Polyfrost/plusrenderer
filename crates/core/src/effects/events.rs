use std::cell::Cell;
use std::mem;
use std::ops::Range;

use std::ptr;

use super::particles::{
    activation_seed, effective_rate, emitter_randoms, fold_loop, loop_start, period, Budget, Vars,
    KEY_TICKS_PER_SECOND,
};
use super::{
    Caught, ControllerFire, EffectInput, EffectSet, Emission, EmitKind, Lifetime, LocatorRef,
    Playing, Rate, Sources, EMITTER_ACTIVATIONS, EMITTER_ACTIVATION_TIMES, EMITTER_LOCATOR,
    EMITTER_START, EMITTER_STOP, MOMENTS, NO_CONTROLLER, SOURCE_EVENT, SOURCE_KEYFRAME,
    SOURCE_REACTION,
};
use crate::animation::{Animation, KeyEvent, KeyEventKind, Loop};
use crate::attach::PlayerLocator;
use crate::controller::{
    crossing_span, CatchUpKeep, Clock, ControllerSet, Effect, EffectKind, CATCH_UP_FUEL,
    CATCH_UP_SECONDS, CATCH_UP_STEPS, SAME_ENTRY,
};
use crate::issues::{Issue, IssueCode, Report};
use crate::limits::{EFFECT_CLIPS, EFFECT_EMITTER_RUNS, EVENT_COOLDOWN_SECONDS};
use crate::math::Affine;
use crate::reactions::{self, On, Reaction, ReactionOut};
use crate::wearer::{WearerRecord, TIME_NEVER};

pub const FIRST_FRAME_SECONDS: f32 = 0.05;
const HELD_BY_REACTION: f32 = f32::INFINITY;
const REACTION_BITS: usize = 32;
const REBUILD_STEPS_PER_LAP: usize = 2;
const RUN_SLACK: f64 = 1e-3;
const RUN_COUNT_CAP: f64 = 65_536.0;
const UNBOUNDED: usize = usize::MAX;
const RETRIGGER_SECONDS: f64 = 0.25;
const AMBIENT_EDGE: u32 = u32::MAX;
const STEADY_EVENTS: usize = 8192;
const STEADY_LAPS: f64 = 64.0;
pub const PLAYBACK_SLOTS: usize = EFFECT_CLIPS as usize;
const PLAYBACK_FLOATS: usize = 2;
const PLAYBACK_MATCH: f32 = 2e-3;
const CAUGHT_KEYS: usize = 64;

fn playback_floats(e: &EffectSet) -> usize {
    if e.emitters.iter().any(|def| def.particle.is_some()) {
        usize::from(e.playback_slots).min(PLAYBACK_SLOTS) * PLAYBACK_FLOATS
    } else {
        0
    }
}

pub fn playback_slots(
    e: &EffectSet,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
) -> u8 {
    let keyed = |clip: usize| anims.get(clip).is_some_and(|a| particle_keys(e, a));
    if !anims.iter().any(|a| particle_keys(e, a)) {
        return 0;
    }
    let direct = 1;
    let plays = c.concurrent_plays(keyed).max(direct) + reactions::concurrent_plays(rs, keyed);
    plays.min(PLAYBACK_SLOTS) as u8
}

pub fn check_plays(
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    at: &str,
    r: &mut Report,
) {
    let keyed = |clip: usize| anims.get(clip).is_some_and(effect_keys);
    if !anims.iter().any(effect_keys) {
        return;
    }
    let plays = c.concurrent_plays(keyed).max(1) + reactions::concurrent_plays(rs, keyed);
    if plays > PLAYBACK_SLOTS {
        r.push(
            Issue::error(IssueCode::EffectClipsTooMany, at)
                .value("clips", plays as f64)
                .value("limit", PLAYBACK_SLOTS as f64),
        );
    }
}

fn key_range(e: &EffectSet, event: &KeyEvent) -> Option<Range<u16>> {
    match &event.kind {
        KeyEventKind::Particle { effect, .. } => e.named(effect),
        KeyEventKind::Sound { effect } if e.sound(effect).is_none() => e.named(effect),
        KeyEventKind::Sound { .. } | KeyEventKind::Timeline(_) => None,
    }
}

fn fires(e: &EffectSet, emitter: usize, event: &KeyEvent) -> bool {
    key_range(e, event).is_some_and(|range| range.contains(&(emitter as u16)))
}

fn entry_range(e: &EffectSet, effect: &Effect) -> Option<Range<u16>> {
    match effect.kind {
        EffectKind::Sound if e.sound(&effect.name).is_some() => None,
        EffectKind::Particle | EffectKind::Sound => e.named(&effect.name),
    }
}

fn locator_entry(e: &EffectSet, emitter: u16, locator: Option<LocatorRef>) -> u32 {
    let at = locator.or_else(|| e.emitters.get(usize::from(emitter)).map(|def| def.locator));
    match at {
        Some(LocatorRef::Model(index)) => 1 + 2 * u32::from(index),
        Some(LocatorRef::Player(player)) => 2 + 2 * player.index() as u32,
        None => 0,
    }
}

fn entry_locator(e: &EffectSet, effect: &Effect) -> Option<LocatorRef> {
    effect.locator.as_deref().and_then(|l| e.locator(l))
}

fn key_spacing(clip: &Animation) -> f64 {
    match clip.loop_mode {
        Loop::Repeat => {
            let period = f64::from(clip.length) + f64::from(clip.shortest_loop_delay());
            RETRIGGER_SECONDS.min(period - f64::from(PLAYBACK_MATCH))
        }
        Loop::Once | Loop::Hold => RETRIGGER_SECONDS,
    }
}

fn spaced(window: f64, spacing: f64) -> usize {
    if spacing <= 0.0 {
        return RUN_COUNT_CAP as usize;
    }
    ((window - RUN_SLACK) / spacing)
        .ceil()
        .clamp(1.0, RUN_COUNT_CAP) as usize
}

fn clip_keys(e: &EffectSet, emitter: usize, clip: &Animation) -> usize {
    clip.events
        .iter()
        .filter(|event| event.time <= clip.length && fires(e, emitter, event))
        .count()
}

fn gated_runs(e: &EffectSet, emitter: usize, clip: &Animation, window: f64) -> usize {
    clip_keys(e, emitter, clip).saturating_mul(spaced(window, key_spacing(clip)))
}

fn once(r: &Reaction) -> bool {
    r.signal == reactions::APPEAR || r.signal == reactions::EQUIP
}

fn run_window(e: &EffectSet, emitter: usize) -> (f64, usize) {
    let def = &e.emitters[emitter];
    let longest = def
        .particle
        .and_then(|p| e.particles.get(p as usize))
        .map_or(0.0, |p| f64::from(p.longest_life.max(0.0)));
    match (def.rate, &def.lifetime) {
        (Rate::Steady { .. }, Lifetime::Once { seconds }) => {
            (f64::from(seconds.max(0.0)) + longest, 0)
        }
        (Rate::Steady { .. }, Lifetime::Expression { .. }) | (_, Lifetime::Looping { .. }) => {
            (longest, 1)
        }
        (Rate::Instant { .. } | Rate::Manual { .. }, _) => (longest, 0),
    }
}

fn clip_runs(
    e: &EffectSet,
    emitter: usize,
    clip: &Animation,
    window: f64,
    delay: Option<f32>,
) -> usize {
    let period = delay.map(|delay| f64::from(clip.length) + f64::from(delay));
    let times: Vec<f64> = clip
        .events
        .iter()
        .filter(|event| event.time <= clip.length && fires(e, emitter, event))
        .map(|event| f64::from(event.time))
        .collect();
    let span = window - RUN_SLACK;
    let period = period.filter(|&p| clip.loop_mode == Loop::Repeat && p > 0.0 && p.is_finite());
    let within = |from: f64| -> f64 {
        times
            .iter()
            .map(|&at| match period {
                Some(period) => {
                    let first = ((from - at) / period).ceil().max(0.0);
                    let last = ((from + span - at) / period).ceil();
                    (last - first).clamp(0.0, RUN_COUNT_CAP)
                }
                None if at >= from && at < from + span => 1.0,
                None => 0.0,
            })
            .sum()
    };
    times
        .iter()
        .map(|&from| within(from).max(1.0).min(RUN_COUNT_CAP) as usize)
        .max()
        .unwrap_or(0)
}

fn steady_runs(
    e: &EffectSet,
    emitter: usize,
    window: f64,
    clips: &[(&Animation, [f32; 2])],
) -> usize {
    let keys: Vec<(f64, f64, Vec<(f64, u32)>)> = clips
        .iter()
        .map(|&(clip, [start, delay])| {
            let period = f64::from(clip.length) + f64::from(delay);
            let keys: Vec<(f64, u32)> = clip
                .events
                .iter()
                .filter(|event| event.time <= clip.length && fires(e, emitter, event))
                .map(|event| {
                    let locator = match &event.kind {
                        KeyEventKind::Particle { locator, .. } => locator.as_deref(),
                        KeyEventKind::Sound { .. } | KeyEventKind::Timeline(_) => None,
                    };
                    let locator = locator.and_then(|l| e.locator(l));
                    (
                        f64::from(event.time),
                        locator_entry(e, emitter as u16, locator),
                    )
                })
                .collect();
            (f64::from(start), period, keys)
        })
        .filter(|(_, period, keys)| *period > 0.0 && period.is_finite() && !keys.is_empty())
        .collect();
    if keys.len() < 2 {
        return 0;
    }
    let longest = keys
        .iter()
        .map(|(_, period, _)| *period)
        .fold(0.0, f64::max);
    let per_second: f64 = keys
        .iter()
        .map(|(_, period, keys)| keys.len() as f64 / period)
        .sum();
    let horizon = (window * 2.0)
        .max(longest * STEADY_LAPS)
        .min(STEADY_EVENTS as f64 / per_second);
    let mut times: Vec<(f64, u32)> = Vec::new();
    for (start, period, keys) in &keys {
        let laps = ((horizon - start) / period).ceil().max(1.0) as usize;
        for lap in 0..laps {
            let base = start + lap as f64 * period;
            times.extend(keys.iter().map(|&(at, locator)| (base + at, locator)));
        }
    }
    times.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut kept: Vec<(f64, u32)> = Vec::with_capacity(times.len());
    for (at, locator) in times {
        let repeat = kept
            .iter()
            .rev()
            .take_while(|(seen, _)| at - seen <= f64::from(PLAYBACK_MATCH))
            .any(|&(_, seen)| seen == locator);
        if !repeat {
            kept.push((at, locator));
        }
    }
    let span = window - RUN_SLACK;
    let mut most = 0;
    let mut end = 0;
    for from in 0..kept.len() {
        while end < kept.len() && kept[end].0 < kept[from].0 + span {
            end += 1;
        }
        most = most.max(end - from);
    }
    most.min(RUN_COUNT_CAP as usize)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Solo {
    pub controllers: bool,
    pub clips: Vec<usize>,
}

impl Solo {
    pub fn worn(clips: usize, c: &ControllerSet) -> Solo {
        if c.controllers().is_empty() {
            Solo {
                controllers: false,
                clips: (0..clips).collect(),
            }
        } else {
            Solo {
                controllers: true,
                clips: Vec::new(),
            }
        }
    }

    pub fn moments(e: &EffectSet) -> Solo {
        Solo {
            controllers: false,
            clips: e
                .moments
                .iter()
                .filter_map(|moment| moment.clip.map(usize::from))
                .collect(),
        }
    }

    pub fn bypassing(mut self, clips: impl IntoIterator<Item = usize>) -> Solo {
        self.clips.extend(clips);
        self.clips.sort_unstable();
        self.clips.dedup();
        self
    }
}

fn solo_runs(
    e: &EffectSet,
    emitter: usize,
    window: f64,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
) -> usize {
    let own = |clip: usize, speed: f64| {
        anims.get(clip).map_or(0, |a| {
            clip_runs(e, emitter, a, window * speed, a.longest_loop_delay())
        })
    };
    let direct = solo.clips.iter().map(|&clip| own(clip, 1.0));
    let reacted = rs.iter().take(REACTION_BITS).filter_map(|r| {
        let clip = anims.get(usize::from(r.play?))?;
        Some(match r.on {
            On::While => clip_runs(e, emitter, clip, window, clip.longest_loop_delay()),
            On::Start | On::Stop => clip_runs(e, emitter, clip, window, None),
        })
    });
    let states: Vec<_> = c
        .state_plays(anims)
        .into_iter()
        .flat_map(|controller| controller.states)
        .collect();
    let controlled =
        states
            .iter()
            .flat_map(|state| &state.clips)
            .map(|&(clip, clock)| match clock {
                Clock::Plain => own(clip, 1.0),
                Clock::Rated(scale) => own(clip, scale),
                Clock::Unknown => 0,
            });
    let together = states.iter().filter(|state| state.lasting).map(|state| {
        let mut steady: Vec<usize> = state
            .clips
            .iter()
            .filter(|&&(clip, clock)| clock == Clock::Plain && state.steady.contains(&clip))
            .map(|&(clip, _)| clip)
            .collect();
        steady.sort_unstable();
        steady.dedup();
        let clips: Vec<(&Animation, [f32; 2])> = steady
            .into_iter()
            .filter_map(|clip| anims.get(clip))
            .filter(|clip| clip.loop_mode == Loop::Repeat && clip.steady_weight())
            .filter_map(|clip| Some((clip, clip.steady_delays()?)))
            .collect();
        steady_runs(e, emitter, window, &clips)
    });
    direct
        .chain(reacted)
        .chain(controlled)
        .chain(together)
        .max()
        .unwrap_or(0)
}

fn keyed_runs(
    e: &EffectSet,
    emitter: usize,
    window: f64,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
) -> usize {
    let fastest = f64::from(reactions::fastest(rs));
    let most = |clip: usize, speed: f64| {
        anims.get(clip).map_or(0, |a| {
            clip_runs(e, emitter, a, window * speed, Some(a.shortest_loop_delay()))
        })
    };
    let keyed = |clip: usize| most(clip, 1.0) > 0;
    if !(0..anims.len()).any(keyed) {
        return 0;
    }

    let direct = solo
        .clips
        .iter()
        .map(|&clip| most(clip, fastest))
        .max()
        .unwrap_or(0);

    let mut played: usize = 0;
    for controller in c.state_plays(anims) {
        for state in &controller.states {
            for &(clip, clock) in state.clips.iter().filter(|&&(clip, _)| keyed(clip)) {
                let bound = match clock {
                    Clock::Plain if !state.reentered => most(clip, fastest),
                    _ => UNBOUNDED,
                };
                played = played.saturating_add(bound);
            }
        }
    }

    let mut overlays: usize = 0;
    for r in rs.iter().take(REACTION_BITS) {
        let Some(clip) = r.play.map(usize::from).filter(|&clip| keyed(clip)) else {
            continue;
        };
        let bound = if once(r) {
            most(clip, fastest)
        } else if edged(e, emitter as u16) {
            gated_runs(e, emitter, &anims[clip], window)
        } else {
            UNBOUNDED
        };
        overlays = overlays.saturating_add(bound);
    }

    played.max(direct).saturating_add(overlays)
}

fn event_runs(
    e: &EffectSet,
    emitter: usize,
    window: f64,
    c: &ControllerSet,
    rs: &[Reaction],
) -> [usize; 2] {
    let index = emitter as u16;
    let mut runs: usize = 0;
    if e.ambient.contains(&index) {
        runs = runs.saturating_add(match e.emitters[emitter].lifetime {
            Lifetime::Expression { .. } => UNBOUNDED,
            _ => 1,
        });
    }
    if e.named
        .iter()
        .any(|(name, range)| range.contains(&index) && MOMENTS.contains(&name.as_str()))
    {
        let repeats = ((window - RUN_SLACK) / f64::from(EVENT_COOLDOWN_SECONDS))
            .ceil()
            .clamp(1.0, RUN_COUNT_CAP);
        runs = runs.saturating_add(repeats as usize);
    }
    for r in rs.iter().take(REACTION_BITS) {
        let fires = r
            .effect
            .and_then(|n| e.named.get(n as usize))
            .is_some_and(|(_, range)| range.contains(&index));
        if fires {
            runs = runs.saturating_add(if once(r) { 1 } else { UNBOUNDED });
        }
    }
    for controller in c.controllers() {
        let fires = controller
            .states
            .iter()
            .flat_map(|state| &state.effects)
            .any(|effect| entry_range(e, effect).is_some_and(|range| range.contains(&index)));
        if fires {
            runs = UNBOUNDED;
        }
    }
    [usize::from(runs > 0), runs]
}

fn retriggers(window: f64) -> usize {
    spaced(window, RETRIGGER_SECONDS)
}

fn budget_runs(
    e: &EffectSet,
    emitter: usize,
    window: f64,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
) -> usize {
    let index = emitter as u16;
    let again = retriggers(window);
    let fastest = f64::from(reactions::fastest(rs));
    let paced: Vec<usize> = anims
        .iter()
        .map(|a| {
            clip_runs(
                e,
                emitter,
                a,
                window * fastest,
                Some(a.shortest_loop_delay()),
            )
        })
        .collect();
    let keys = if paced.iter().all(|&runs| runs == 0) {
        0
    } else {
        let most = solo
            .clips
            .iter()
            .filter_map(|&clip| paced.get(clip).copied())
            .max()
            .unwrap_or(0);
        let reacted = rs
            .iter()
            .take(REACTION_BITS)
            .filter_map(|r| {
                let clip = anims.get(usize::from(r.play?))?;
                Some(if once(r) {
                    clip_runs(e, emitter, clip, window, None)
                } else {
                    gated_runs(e, emitter, clip, window)
                })
            })
            .fold(0, usize::saturating_add);
        c.concurrent_weight(anims, |clip, rated| {
            let runs = paced.get(clip).copied().unwrap_or(0);
            if rated && runs > 0 {
                runs.max(again)
            } else {
                runs
            }
        })
        .max(most)
        .saturating_add(reacted)
    };

    let mut events: usize = 0;
    if e.ambient.contains(&index) {
        events = match e.emitters[emitter].lifetime {
            Lifetime::Expression { .. } => again,
            _ => 1,
        };
    }
    if e.named
        .iter()
        .any(|(name, range)| range.contains(&index) && MOMENTS.contains(&name.as_str()))
    {
        events = events.saturating_add(1);
    }
    for r in rs.iter().take(REACTION_BITS) {
        let fires = r
            .effect
            .and_then(|n| e.named.get(n as usize))
            .is_some_and(|(_, range)| range.contains(&index));
        if fires {
            events = events.saturating_add(if once(r) { 1 } else { again });
        }
    }
    for controller in c.controllers() {
        let mut entries: Vec<u32> = controller
            .states
            .iter()
            .flat_map(|state| &state.effects)
            .filter(|effect| entry_range(e, effect).is_some_and(|range| range.contains(&index)))
            .map(|effect| locator_entry(e, index, entry_locator(e, effect)))
            .collect();
        entries.sort_unstable();
        entries.dedup();
        events = events.saturating_add(again.saturating_mul(entries.len()));
    }
    keys.saturating_add(events)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunNeed {
    pub certain: usize,
    pub likely: usize,
    pub budget: usize,
}

pub fn emitter_runs(
    e: &EffectSet,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
) -> Vec<RunNeed> {
    let still = ControllerSet::default();
    let c = if solo.controllers { c } else { &still };
    (0..e.emitters.len())
        .map(|emitter| {
            let (window, running) = run_window(e, emitter);
            let keys = solo_runs(e, emitter, window, anims, c, rs, solo);
            let keys_likely = keyed_runs(e, emitter, window, anims, c, rs, solo);
            let [events, events_likely] = event_runs(e, emitter, window, c, rs);
            let budget = budget_runs(e, emitter, window, anims, c, rs, solo)
                .saturating_add(running)
                .max(EMITTER_ACTIVATIONS);
            let certain = keys
                .max(events)
                .saturating_add(running)
                .max(EMITTER_ACTIVATIONS);
            let likely = keys_likely
                .saturating_add(events_likely)
                .saturating_add(running)
                .max(EMITTER_ACTIVATIONS);
            let likely = likely.max(certain);
            RunNeed {
                certain,
                likely,
                budget: budget.max(certain).min(likely),
            }
        })
        .collect()
}

pub fn check_runs(
    e: &EffectSet,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
    solo: &Solo,
    r: &mut Report,
) {
    let limit = EFFECT_EMITTER_RUNS as usize;
    for (def, need) in e.emitters.iter().zip(emitter_runs(e, anims, c, rs, solo)) {
        if need.certain > limit {
            r.push(
                Issue::error(IssueCode::EffectRunsTooMany, def.path.clone())
                    .value("runs", need.certain as f64)
                    .value("limit", limit as f64),
            );
        }
    }
}

pub fn effect_keys(animation: &Animation) -> bool {
    animation.events.iter().any(|event| {
        matches!(
            event.kind,
            KeyEventKind::Particle { .. } | KeyEventKind::Sound { .. }
        )
    })
}

pub fn last_step(e: &EffectSet) -> usize {
    e.run_floats() + playback_floats(e) + e.edges.len()
}

fn edged(e: &EffectSet, emitter: u16) -> bool {
    e.emitters
        .get(usize::from(emitter))
        .is_some_and(|def| def.particle.is_some() && def.sound.is_none() && def.vanilla.is_none())
}

fn clip_edge(reaction: u32) -> u32 {
    (REACTION_BITS as u32).saturating_add(reaction)
}

fn controller_edge(controller: u32) -> u32 {
    (2 * REACTION_BITS as u32).saturating_add(controller)
}

pub fn edges(
    e: &EffectSet,
    anims: &[Animation],
    c: &ControllerSet,
    rs: &[Reaction],
) -> Vec<(u16, u32, u32)> {
    let mut edges = Vec::new();
    for (index, r) in rs.iter().enumerate().take(REACTION_BITS) {
        let index = index as u32;
        if let Some((_, range)) = r.effect.and_then(|n| e.named.get(usize::from(n))) {
            edges.extend(
                range
                    .clone()
                    .filter(|&emitter| edged(e, emitter))
                    .map(|emitter| (emitter, index, 0)),
            );
        }
        let Some(clip) = r.play.and_then(|play| anims.get(usize::from(play))) else {
            continue;
        };
        for (key, event) in clip.events.iter().enumerate() {
            let Some(range) = key_range(e, event).filter(|_| event.time <= clip.length) else {
                continue;
            };
            edges.extend(
                range
                    .filter(|&emitter| edged(e, emitter))
                    .map(|emitter| (emitter, clip_edge(index), key as u32)),
            );
        }
    }
    for (index, controller) in c.controllers().iter().enumerate() {
        let source = controller_edge(index as u32);
        for effect in controller.states.iter().flat_map(|state| &state.effects) {
            if let Some(range) = entry_range(e, effect) {
                let locator = entry_locator(e, effect);
                edges.extend(
                    range
                        .filter(|&emitter| edged(e, emitter))
                        .map(|emitter| (emitter, source, locator_entry(e, emitter, locator))),
                );
            }
        }
    }
    for emitter in e.ambient.clone().filter(|&emitter| edged(e, emitter)) {
        let def = &e.emitters[usize::from(emitter)];
        let restarts = matches!(def.lifetime, Lifetime::Expression { .. })
            && !matches!(def.rate, Rate::Manual { .. });
        if restarts {
            edges.push((emitter, AMBIENT_EDGE, 0));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

pub fn particle_keys(e: &EffectSet, animation: &Animation) -> bool {
    animation.events.iter().any(|event| match &event.kind {
        KeyEventKind::Particle { .. } => true,
        KeyEventKind::Sound { .. } => key_range(e, event).is_some(),
        KeyEventKind::Timeline(_) => false,
    })
}

#[derive(Clone, Copy)]
struct Gate {
    source: u32,
    key: Option<u32>,
    spacing: f64,
}

impl Gate {
    fn once(source: u32) -> Gate {
        Gate {
            source,
            key: Some(0),
            spacing: RETRIGGER_SECONDS,
        }
    }

    fn located(source: u32) -> Gate {
        Gate {
            source,
            key: None,
            spacing: RETRIGGER_SECONDS,
        }
    }
}

#[derive(Clone, Copy)]
struct Pending {
    at: f32,
    emitter: u16,
    locator: Option<LocatorRef>,
    controller: u32,
    source: u32,
    silent: bool,
}

#[derive(Clone, Copy)]
struct Backfilled {
    emitter: usize,
    slot: usize,
    start: f32,
    step: f32,
    checked: u32,
}

struct Run<'a, 'i> {
    e: &'a EffectSet,
    times: &'a mut [f32],
    input: &'a EffectInput<'i>,
    now: f32,
    epoch: f64,
    out: &'a mut Vec<Emission>,
    budget: Budget,
    pending: &'a mut Vec<Pending>,
    backfilled: &'a mut Vec<Backfilled>,
    held: &'a mut Vec<(u32, Range<u16>)>,
    idle: &'a mut Vec<Range<u16>>,
    spare: Budget,
}

#[derive(Default)]
struct StepScratch {
    pending: Vec<Pending>,
    backfilled: Vec<Backfilled>,
    held: Vec<(u32, Range<u16>)>,
    idle: Vec<Range<u16>>,
    live: Vec<(usize, f32, f32)>,
}

thread_local! {
    static SCRATCH: Cell<StepScratch> = Cell::new(StepScratch {
        pending: Vec::new(),
        backfilled: Vec::new(),
        held: Vec::new(),
        idle: Vec::new(),
        live: Vec::new(),
    });
}

impl Run<'_, '_> {
    fn at(&self, locator: LocatorRef) -> [f32; 3] {
        let tf = match locator {
            LocatorRef::Model(i) => self.input.model_locators.get(i as usize),
            LocatorRef::Player(p) => self.input.player_locators.get(p.index()),
        }
        .copied()
        .unwrap_or(Affine::IDENTITY);
        self.input.root.vector([tf.0[3], tf.0[7], tf.0[11]])
    }

    fn slots(&self, emitter: usize) -> impl Iterator<Item = usize> + Clone {
        let (base, runs) = self.e.runs(emitter);
        (0..runs).map(move |a| base + a * EMITTER_ACTIVATION_TIMES)
    }

    fn current(&self, emitter: usize) -> usize {
        let base = self.e.runs(emitter).0;
        self.slots(emitter)
            .filter(|&at| self.times[at + EMITTER_START].is_finite())
            .max_by(|&a, &b| {
                self.times[a + EMITTER_START].total_cmp(&self.times[b + EMITTER_START])
            })
            .unwrap_or(base)
    }

    fn started(&self, emitter: usize) -> bool {
        self.times[self.current(emitter) + EMITTER_START].is_finite()
    }

    fn running(&self, emitter: usize) -> bool {
        let at = self.current(emitter);
        self.times[at + EMITTER_START].is_finite() && !self.times[at + EMITTER_STOP].is_finite()
    }

    fn ended(&self, at: usize, emitter: usize, when: f32) -> bool {
        let start = self.times[at + EMITTER_START];
        if !start.is_finite() {
            return true;
        }
        let stop = self.times[at + EMITTER_STOP];
        let def = &self.e.emitters[emitter];
        let until = match (def.rate, &def.lifetime) {
            (Rate::Steady { .. }, Lifetime::Once { seconds }) => start + seconds.max(0.0),
            (Rate::Steady { .. }, Lifetime::Expression { .. }) | (_, Lifetime::Looping { .. }) => {
                f32::INFINITY
            }
            (Rate::Instant { .. } | Rate::Manual { .. }, _) => start,
        };
        let until = if stop.is_finite() {
            until.min(stop)
        } else {
            until
        };
        until + self.longest(emitter) <= when
    }

    fn free(&self, emitter: usize, when: f32) -> usize {
        let mut slots = self.slots(emitter);
        let oldest = slots.clone().min_by(|&a, &b| {
            self.times[a + EMITTER_START].total_cmp(&self.times[b + EMITTER_START])
        });
        let empty = slots
            .clone()
            .find(|&at| !self.times[at + EMITTER_START].is_finite());
        empty
            .or_else(|| slots.find(|&at| self.ended(at, emitter, when)))
            .or(oldest)
            .unwrap_or(self.e.runs(emitter).0)
    }

    fn longest(&self, emitter: usize) -> f32 {
        self.e.emitters[emitter]
            .particle
            .and_then(|p| self.e.particles.get(p as usize))
            .map_or(0.0, |p| p.longest_life)
    }

    fn tick(&self) -> f32 {
        let at = libm::floor((self.epoch + f64::from(self.now)) * KEY_TICKS_PER_SECOND);
        ((at / KEY_TICKS_PER_SECOND - self.epoch) as f32).min(self.now)
    }

    fn ambient_start(&self, emitter: usize) -> f32 {
        let def = &self.e.emitters[emitter];
        match (def.rate, period(&def.lifetime)) {
            (Rate::Manual { .. }, _) => self.now,
            (_, Some((_, period))) => {
                loop_start(self.epoch, self.now, self.longest(emitter), period).min(self.now)
            }
            (Rate::Steady { per_second, max }, None) => {
                let rate = f64::from(effective_rate(per_second, max, self.longest(emitter)));
                if rate > 0.0 {
                    let at = libm::floor((self.epoch + f64::from(self.now)) * rate) / rate;
                    ((at - self.epoch) as f32).min(self.now)
                } else {
                    self.tick()
                }
            }
            (Rate::Instant { .. }, None) => self.tick(),
        }
    }

    fn repeats(&self, emitter: usize, locator: Option<LocatorRef>, at: f32) -> bool {
        let current = self.current(emitter);
        let start = self.times[current + EMITTER_START];
        if !start.is_finite() || (start - at).abs() > PLAYBACK_MATCH {
            return false;
        }
        let kept = self.times[current + EMITTER_LOCATOR];
        let kept = kept.is_finite().then(|| libm::roundf(kept - start));
        kept == locator.map(LocatorRef::encode)
    }

    fn activate(&mut self, emitter: usize, locator: Option<LocatorRef>, at: f32) -> Option<usize> {
        if self.repeats(emitter, locator, at) {
            return None;
        }
        let current = self.current(emitter);
        if self.running(emitter) {
            self.times[current + EMITTER_STOP] = at.max(self.times[current + EMITTER_START]);
        }
        let next = self.free(emitter, at);
        self.times[next + EMITTER_START] = at;
        self.times[next + EMITTER_STOP] = TIME_NEVER;
        self.times[next + EMITTER_LOCATOR] = locator.map_or(TIME_NEVER, |l| at + l.encode());
        Some(next)
    }

    fn backfill(&mut self, p: Pending) {
        let emitter = usize::from(p.emitter);
        let Some(slot) = self.activate(emitter, p.locator, p.at) else {
            return;
        };
        if !matches!(
            self.e.emitters[emitter].lifetime,
            Lifetime::Expression { .. }
        ) {
            return;
        }
        self.backfilled.retain(|b| b.slot != slot);
        let span = (self.now - p.at).max(0.0);
        self.backfilled.push(Backfilled {
            emitter,
            slot,
            start: p.at,
            step: CATCH_UP_SECONDS.max(span / CATCH_UP_STEPS as f32),
            checked: 0,
        });
    }

    fn expire_backfilled(&mut self, emitter: Option<usize>, until: f32) {
        let mut backfilled = mem::take(&mut *self.backfilled);
        for b in &mut backfilled {
            if emitter.is_none_or(|emitter| emitter == b.emitter) {
                self.expire(b, until);
            }
        }
        backfilled.retain(|b| {
            self.times[b.slot + EMITTER_START] == b.start
                && !self.times[b.slot + EMITTER_STOP].is_finite()
        });
        *self.backfilled = backfilled;
    }

    fn expire(&mut self, b: &mut Backfilled, until: f32) {
        let Lifetime::Expression { expiration, .. } = &self.e.emitters[b.emitter].lifetime else {
            return;
        };
        let at = b.slot;
        if self.times[at + EMITTER_START] != b.start || self.times[at + EMITTER_STOP].is_finite() {
            return;
        }
        let seed = activation_seed(self.input.seed, self.input.seed_epoch, b.start);
        let randoms = emitter_randoms(seed, b.emitter as u16);
        loop {
            let age = (b.checked + 1) as f32 * b.step;
            if b.start + age > until || self.spare.left() == 0 {
                return;
            }
            b.checked += 1;
            let vars = Vars::emitter(randoms, age, 0.0);
            if vars.eval_in(expiration, seed, &mut self.spare) != 0.0 {
                self.times[at + EMITTER_STOP] = b.start + age;
                return;
            }
        }
    }

    fn fold(&mut self) {
        for (index, def) in self.e.emitters.iter().enumerate() {
            let Some((_, period)) = period(&def.lifetime) else {
                continue;
            };
            if matches!(def.rate, Rate::Manual { .. }) {
                continue;
            }
            let back = self.longest(index);
            let (base, runs) = self.e.runs(index);
            for activation in 0..runs {
                let at = base + activation * EMITTER_ACTIVATION_TIMES;
                let start = self.times[at + EMITTER_START];
                if !start.is_finite() || self.times[at + EMITTER_STOP].is_finite() {
                    continue;
                }
                let folded = fold_loop(self.epoch, start, self.now, back, period);
                if folded == start {
                    continue;
                }
                self.times[at + EMITTER_START] = folded;
                let locator = &mut self.times[at + EMITTER_LOCATOR];
                if locator.is_finite() {
                    *locator = folded + libm::roundf(*locator - start);
                }
            }
        }
    }

    fn sound(&mut self, sound: u16, at: [f32; 3], controller: u32, source: u32) {
        let Some(def) = self.e.sounds.get(sound as usize) else {
            return;
        };
        self.out.push(Emission {
            kind: EmitKind::Sound,
            controller,
            source,
            effect: sound,
            at,
            volume: def.volume,
            pitch: def.pitch,
        });
    }

    fn start(
        &mut self,
        emitter: u16,
        locator: Option<LocatorRef>,
        controller: u32,
        source: u32,
        at: f32,
    ) {
        let Some(def) = self.e.emitters.get(emitter as usize) else {
            return;
        };
        let _ = self.activate(emitter as usize, locator, at);
        let at = self.at(locator.unwrap_or(def.locator));
        let particles = self.input.features.particles;
        if let Some(sound) = def.sound {
            self.sound(sound, at, controller, source);
            return;
        }
        let (kind, count) = match (&def.vanilla, def.rate) {
            (Some(_), Rate::Instant { count }) => (EmitKind::VanillaParticle, count),
            (Some(_), _) => (EmitKind::VanillaParticle, 1),
            (None, _) => (EmitKind::Particle, 1),
        };
        if !particles {
            return;
        }
        for _ in 0..count {
            self.out.push(Emission {
                kind,
                controller,
                source,
                effect: emitter,
                at,
                volume: 1.0,
                pitch: 1.0,
            });
        }
    }

    fn fire(
        &mut self,
        range: Range<u16>,
        locator: Option<LocatorRef>,
        controller: u32,
        source: u32,
    ) {
        self.fire_at(range, locator, controller, source, self.now);
    }

    fn fire_at(
        &mut self,
        range: Range<u16>,
        locator: Option<LocatorRef>,
        controller: u32,
        source: u32,
        at: f32,
    ) {
        for emitter in range {
            self.pending.push(Pending {
                at,
                emitter,
                locator,
                controller,
                source,
                silent: false,
            });
        }
    }

    fn flush(&mut self) {
        let mut pending = mem::take(&mut *self.pending);
        pending.sort_by(|a, b| a.at.total_cmp(&b.at));
        for p in pending.drain(..) {
            if !self.backfilled.is_empty() {
                self.expire_backfilled(Some(usize::from(p.emitter)), p.at);
            }
            if p.silent {
                self.backfill(p);
            } else {
                self.start(p.emitter, p.locator, p.controller, p.source, p.at);
            }
        }
        *self.pending = pending;
        if !self.backfilled.is_empty() {
            self.expire_backfilled(None, self.now);
        }
    }

    fn release(&mut self, emitter: usize) {
        if self.running(emitter) {
            let at = self.current(emitter);
            if self.times[at + EMITTER_LOCATOR] == HELD_BY_REACTION {
                self.times[at + EMITTER_STOP] = self.now;
            }
        }
    }

    fn edge(&mut self, emitter: u16, gate: Gate, locator: Option<LocatorRef>, at: f32) -> bool {
        let entry = gate
            .key
            .unwrap_or_else(|| locator_entry(self.e, emitter, locator));
        let Ok(index) = self.e.edges.binary_search(&(emitter, gate.source, entry)) else {
            return true;
        };
        let slot = self.playback().end + index;
        let Some(last) = self.times.get(slot).copied() else {
            return true;
        };
        if last.is_finite() && (f64::from(at) - f64::from(last)).abs() < gate.spacing {
            return false;
        }
        self.times[slot] = at;
        true
    }

    fn fire_edge(&mut self, range: Range<u16>, source: u32, edge: u32) {
        let now = self.now;
        for emitter in range {
            if self.edge(emitter, Gate::once(edge), None, now) {
                self.fire_at(emitter..emitter + 1, None, NO_CONTROLLER, source, now);
            }
        }
    }

    fn hold(&mut self, range: Range<u16>, source: u32, edge: u32) {
        let now = self.now;
        for emitter in range {
            if !self.running(emitter as usize) && self.edge(emitter, Gate::once(edge), None, now) {
                self.start(emitter, None, NO_CONTROLLER, source, self.now);
                let at = self.current(emitter as usize);
                self.times[at + EMITTER_LOCATOR] = HELD_BY_REACTION;
            }
        }
    }

    fn reactions(&mut self, reactions: &[Reaction], triggers: &ReactionOut) {
        let named = |reaction: &Reaction| {
            reaction
                .effect
                .and_then(|n| self.e.named.get(n as usize))
                .map(|(_, range)| range.clone())
        };
        let mut held = mem::take(&mut *self.held);
        let mut idle = mem::take(&mut *self.idle);
        held.clear();
        idle.clear();
        for (i, reaction) in reactions.iter().enumerate().take(REACTION_BITS) {
            let Some(range) = named(reaction) else {
                continue;
            };
            let bit = 1u32 << i;
            match reaction.on {
                On::While if triggers.held & bit != 0 => held.push((i as u32, range)),
                On::While => idle.push(range),
                On::Start | On::Stop if triggers.fired & bit != 0 => {
                    self.fire_edge(range, SOURCE_REACTION, i as u32)
                }
                On::Start | On::Stop => {}
            }
        }
        for (edge, range) in &held {
            self.hold(range.clone(), SOURCE_REACTION, *edge);
        }
        for emitter in idle.iter().cloned().flatten() {
            if !held.iter().any(|(_, range)| range.contains(&emitter)) {
                self.release(emitter as usize);
            }
        }
        *self.held = held;
        *self.idle = idle;
    }

    fn sound_or_named(
        &mut self,
        name: &str,
        locator: Option<LocatorRef>,
        controller: u32,
        source: u32,
        gate: Option<Gate>,
        at: f32,
    ) {
        if let Some(sound) = self.e.sound(name) {
            let spot = self.at(locator.unwrap_or(LocatorRef::Player(PlayerLocator::Feet)));
            self.sound(sound, spot, controller, source);
        } else if let Some(range) = self.e.named(name) {
            for emitter in range {
                if gate.is_none_or(|gate| self.edge(emitter, gate, locator, at)) {
                    self.fire_at(emitter..emitter + 1, locator, controller, source, at);
                }
            }
        }
    }

    fn named(
        &mut self,
        name: &str,
        locator: Option<LocatorRef>,
        controller: u32,
        source: u32,
        at: f32,
    ) {
        let Some(range) = self.e.named(name) else {
            return;
        };
        let gate = Gate::located(controller_edge(controller));
        for emitter in range {
            if self.edge(emitter, gate, locator, at) {
                self.fire_at(emitter..emitter + 1, locator, controller, source, at);
            }
        }
    }

    fn shows(&self, emitter: u16, ago: f32) -> bool {
        let emitter = usize::from(emitter);
        backdated(self.e, emitter) && ago <= live_seconds(self.e, emitter)
    }

    fn mark(&mut self, emitter: u16, gate: Gate, locator: Option<LocatorRef>, at: f32) {
        let entry = gate
            .key
            .unwrap_or_else(|| locator_entry(self.e, emitter, locator));
        let Ok(index) = self.e.edges.binary_search(&(emitter, gate.source, entry)) else {
            return;
        };
        let slot = self.playback().end + index;
        if let Some(last) = self.times.get_mut(slot) {
            if !last.is_finite() || *last < at {
                *last = at;
            }
        }
    }

    fn backdate(&mut self, fire: &ControllerFire<'_>, locator: Option<LocatorRef>, ago: f32) {
        let &ControllerFire {
            controller,
            state: source,
            effect,
            emitter: only,
            ..
        } = fire;
        let Some(mut range) = entry_range(self.e, effect) else {
            return;
        };
        if let Some(only) = only {
            if !range.contains(&only) {
                return;
            }
            range = only..only + 1;
        }
        let at = self.now - ago;
        let gate = Gate::located(controller_edge(controller));
        for emitter in range {
            self.mark(emitter, gate, locator, at);
            if self.shows(emitter, ago) {
                self.pending.push(Pending {
                    at,
                    emitter,
                    locator,
                    controller,
                    source,
                    silent: true,
                });
            }
        }
    }

    fn keyframe(
        &mut self,
        name: &str,
        locator: Option<LocatorRef>,
        ago: f32,
        silent: bool,
        gate: Option<Gate>,
    ) {
        let Some(range) = self.e.named(name) else {
            return;
        };
        let at = self.now - ago;
        for emitter in range {
            if gate.is_some_and(|gate| !self.edge(emitter, gate, locator, at)) {
                continue;
            }
            if !silent || self.shows(emitter, ago) {
                self.pending.push(Pending {
                    at,
                    emitter,
                    locator,
                    controller: NO_CONTROLLER,
                    source: SOURCE_KEYFRAME,
                    silent,
                });
            }
        }
    }

    fn keyframes(
        &mut self,
        playing: &Playing<'_>,
        [from, to]: [f32; 2],
        frame: f32,
        real: impl Fn(f32) -> f32,
    ) {
        let clip = playing.animation;
        let spacing = key_spacing(clip);
        let gate = |event: &KeyEvent| {
            playing.reaction.map(|reaction| Gate {
                source: clip_edge(reaction),
                key: Some(
                    clip.events
                        .iter()
                        .position(|key| ptr::eq(key, event))
                        .map_or(u32::MAX, |key| key as u32),
                ),
                spacing,
            })
        };
        for (event, crossed) in clip.crossings(Some(from), to, playing.delays) {
            let ago = real(crossed);
            let ago = if ago.is_finite() { ago.max(0.0) } else { 0.0 };
            match &event.kind {
                KeyEventKind::Particle {
                    effect, locator, ..
                } => {
                    let locator = locator.as_deref().and_then(|l| self.e.locator(l));
                    self.keyframe(effect, locator, ago, ago > frame, gate(event));
                }
                KeyEventKind::Sound { effect } if ago <= frame => self.sound_or_named(
                    effect,
                    None,
                    NO_CONTROLLER,
                    SOURCE_KEYFRAME,
                    gate(event),
                    self.now - ago,
                ),
                KeyEventKind::Sound { effect } if key_range(self.e, event).is_some() => {
                    self.keyframe(effect, None, ago, true, gate(event))
                }
                KeyEventKind::Sound { .. } | KeyEventKind::Timeline(_) => {}
            }
        }
    }

    fn caught(&mut self, caught: &Caught<'_>, live: f32) {
        let &Caught {
            animation,
            from,
            to,
            delays,
            ago: [first, last],
            fresh,
            ..
        } = caught;
        let lo = from.max(0.0);
        let span = to - lo;
        let e = self.e;
        let particle = |key: &KeyEvent| match &key.kind {
            KeyEventKind::Particle { .. } => true,
            KeyEventKind::Sound { .. } => fresh || key_range(e, key).is_some(),
            KeyEventKind::Timeline(_) => false,
        };
        for (event, crossed) in
            animation.crossings_every_lap(Some(from), to, delays, CAUGHT_KEYS, particle)
        {
            if crossed > live {
                continue;
            }
            let (effect, locator) = match &event.kind {
                KeyEventKind::Particle {
                    effect, locator, ..
                } => (effect, locator.as_deref()),
                KeyEventKind::Sound { effect } => (effect, None),
                KeyEventKind::Timeline(_) => continue,
            };
            let share = if span > 0.0 {
                ((to - crossed) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let ago = (last + share * (first - last)).max(0.0);
            let locator = locator.and_then(|l| self.e.locator(l));
            match &event.kind {
                KeyEventKind::Sound { effect } if fresh => self.sound_or_named(
                    effect,
                    None,
                    NO_CONTROLLER,
                    SOURCE_KEYFRAME,
                    None,
                    self.now - ago,
                ),
                _ => self.keyframe(effect, locator, ago, !fresh, None),
            }
        }
    }

    fn playback(&self) -> Range<usize> {
        let base = self.e.run_floats();
        base..base + playback_floats(self.e)
    }

    fn seen(&self, id: f32, start: f32) -> bool {
        let Some(table) = self.times.get(self.playback()) else {
            return false;
        };
        table.chunks_exact(PLAYBACK_FLOATS).any(|entry| {
            let [at, code] = [entry[0], entry[1]];
            at.is_finite()
                && code.is_finite()
                && libm::roundf(code - at) - 1.0 == id
                && (at - start).abs() <= PLAYBACK_MATCH
        })
    }

    fn rebuild(&mut self, playing: &Playing<'_>) {
        let elapsed = playing.elapsed;
        let rate = if elapsed > 0.0 && playing.age.is_finite() && playing.age >= 0.0 {
            playing.age / elapsed
        } else {
            1.0
        };
        let since = |at: f32| (elapsed - at) * rate;
        let live = if playing.fresh { f32::INFINITY } else { 0.0 };
        match playing.animation.period(playing.delays) {
            Some(period) => {
                let laps = (0..self.e.emitters.len())
                    .map(|emitter| self.e.runs(emitter).1)
                    .fold(EMITTER_ACTIVATIONS, usize::max);
                let steps = laps * REBUILD_STEPS_PER_LAP;
                let step = period / REBUILD_STEPS_PER_LAP as f32;
                let at = |k: usize| elapsed - step * (steps - k) as f32;
                for k in 0..steps {
                    self.keyframes(playing, [at(k), at(k + 1)], live, since);
                }
            }
            None => self.keyframes(playing, [f32::NEG_INFINITY, elapsed], live, since),
        }
    }

    fn expressions(&mut self, seed: u64) {
        let ambient = self.e.ambient.clone();
        for (index, def) in self.e.emitters.iter().enumerate() {
            let Lifetime::Expression {
                activation,
                expiration,
            } = &def.lifetime
            else {
                continue;
            };
            let running = self.running(index);
            let start = if running {
                self.times[self.current(index) + EMITTER_START]
            } else {
                self.ambient_start(index)
            };
            let seed = activation_seed(seed, self.input.seed_epoch, start);
            let randoms = emitter_randoms(seed, index as u16);
            if running {
                let at = self.current(index);
                let age = self.now - start;
                let vars = Vars::emitter(randoms, age, 0.0);
                if vars.eval_in(expiration, seed, &mut self.budget) != 0.0 {
                    self.times[at + EMITTER_STOP] = self.now;
                }
            } else if ambient.contains(&(index as u16))
                && !matches!(def.rate, Rate::Manual { .. })
                && Vars::emitter(randoms, 0.0, 0.0).eval_in(activation, seed, &mut self.budget)
                    != 0.0
                && self.edge(index as u16, Gate::once(AMBIENT_EDGE), None, start)
            {
                self.start(index as u16, None, NO_CONTROLLER, SOURCE_EVENT, start);
            }
        }
    }
}

fn backdated(e: &EffectSet, emitter: usize) -> bool {
    e.emitters
        .get(emitter)
        .is_some_and(|def| def.particle.is_some() && def.sound.is_none() && def.vanilla.is_none())
}

fn live_seconds(e: &EffectSet, emitter: usize) -> f32 {
    if !backdated(e, emitter) {
        return 0.0;
    }
    let def = &e.emitters[emitter];
    let life = def
        .particle
        .and_then(|p| e.particles.get(p as usize))
        .map_or(0.0, |p| p.longest_life);
    match def.lifetime {
        Lifetime::Once { seconds } => seconds + life,
        Lifetime::Looping { .. } | Lifetime::Expression { .. } => f32::INFINITY,
    }
}

fn window_seconds(e: &EffectSet, emitter: usize) -> f32 {
    let def = &e.emitters[emitter];
    let life = def
        .particle
        .and_then(|p| e.particles.get(p as usize))
        .map_or(0.0, |p| p.longest_life);
    match def.lifetime {
        Lifetime::Once { seconds } => seconds.max(0.0) + life,
        Lifetime::Looping { active, .. } => active.max(0.0) + life,
        Lifetime::Expression { .. } => life,
    }
}

fn controller_spaced(e: &EffectSet, emitter: u16) -> bool {
    let first = controller_edge(0);
    e.edges
        .iter()
        .any(|&(at, source, _)| at == emitter && source >= first && source != AMBIENT_EDGE)
}

fn keep_of(e: &EffectSet, emitters: impl Iterator<Item = u16>, spaced: bool) -> CatchUpKeep {
    let mut keep = CatchUpKeep::NONE;
    for emitter in emitters.filter(|&emitter| backdated(e, usize::from(emitter))) {
        keep.seconds = keep.seconds.max(live_seconds(e, usize::from(emitter)));
        keep.window = keep.window.max(window_seconds(e, usize::from(emitter)));
        if spaced && controller_spaced(e, emitter) {
            keep.spacing = RETRIGGER_SECONDS as f32;
        }
    }
    keep
}

pub(super) fn catch_up_keep(e: &EffectSet, effect: &Effect) -> CatchUpKeep {
    keep_of(e, entry_range(e, effect).into_iter().flatten(), true)
}

pub(super) fn catch_up_gates(e: &EffectSet, effect: &Effect) -> Vec<(u16, u32)> {
    let locator = entry_locator(e, effect);
    entry_range(e, effect)
        .into_iter()
        .flatten()
        .filter(|&emitter| edged(e, emitter))
        .map(|emitter| (emitter, locator_entry(e, emitter, locator)))
        .collect()
}

pub(super) fn catch_up_fires(e: &EffectSet) -> usize {
    let runs: usize = (0..e.emitters.len()).map(|emitter| e.runs(emitter).1).sum();
    runs + e.edges.len()
}

pub(super) fn prune_fires(e: &EffectSet, fires: &mut Vec<ControllerFire<'_>>) {
    let mut newest: Vec<usize> = (0..fires.len()).collect();
    newest.sort_by(|&a, &b| fires[a].ago.max(0.0).total_cmp(&fires[b].ago.max(0.0)));
    let mut keep = vec![false; fires.len()];
    let mut runs = vec![0usize; e.emitters.len()];
    let mut gates = vec![false; e.edges.len()];
    for at in newest {
        let fire = &fires[at];
        let ago = fire.ago.max(0.0);
        if ago <= 0.0 || fire.fresh {
            keep[at] = true;
            continue;
        }
        let Some(range) = entry_range(e, fire.effect) else {
            continue;
        };
        let locator = entry_locator(e, fire.effect);
        let source = controller_edge(fire.controller);
        for emitter in range.filter(|&emitter| fire.emitter.is_none_or(|only| only == emitter)) {
            let entry = locator_entry(e, emitter, locator);
            if let Ok(gate) = e.edges.binary_search(&(emitter, source, entry)) {
                keep[at] |= !mem::replace(&mut gates[gate], true);
            }
            let index = usize::from(emitter);
            if backdated(e, index) && ago <= live_seconds(e, index) && runs[index] < e.runs(index).1
            {
                runs[index] += 1;
                keep[at] = true;
            }
        }
    }
    let mut at = 0;
    fires.retain(|_| {
        at += 1;
        keep[at - 1]
    });
}

pub(super) fn clip_catch_up(e: &EffectSet, animation: &Animation) -> Option<CatchUpKeep> {
    let emitters = animation
        .events
        .iter()
        .filter_map(|event| key_range(e, event))
        .flatten();
    particle_keys(e, animation).then(|| keep_of(e, emitters, false))
}

pub(super) fn step(
    e: &EffectSet,
    rec: &mut WearerRecord<'_>,
    input: &EffectInput<'_>,
    sources: &Sources<'_>,
    out: &mut Vec<Emission>,
) -> u32 {
    let mut scratch = SCRATCH.with(Cell::take);
    scratch.pending.clear();
    scratch.backfilled.clear();
    scratch.live.clear();
    let left = step_in(e, rec, input, sources, out, &mut scratch);
    SCRATCH.with(|cell| cell.set(scratch));
    left
}

fn step_in(
    e: &EffectSet,
    rec: &mut WearerRecord<'_>,
    input: &EffectInput<'_>,
    sources: &Sources<'_>,
    out: &mut Vec<Emission>,
    scratch: &mut StepScratch,
) -> u32 {
    let now = rec.now;
    let epoch = rec.epoch;
    let last_index = last_step(e);
    let Some(&last) = rec.effect_times.get(last_index) else {
        return input.fuel;
    };
    let times = &mut rec.effect_times[..last_index + 1];
    times[last_index] = now;
    let dt = if last.is_finite() {
        (now - last).max(0.0)
    } else {
        FIRST_FRAME_SECONDS
    };
    let mut run = Run {
        e,
        times,
        input,
        now,
        epoch,
        out,
        budget: Budget::new(input.fuel),
        pending: &mut scratch.pending,
        backfilled: &mut scratch.backfilled,
        held: &mut scratch.held,
        idle: &mut scratch.idle,
        spare: Budget::new(CATCH_UP_FUEL),
    };
    run.fold();
    if !input.features.effects {
        return input.fuel;
    }

    for emitter in e.ambient.clone() {
        let def = &e.emitters[emitter as usize];
        let expression = matches!(def.lifetime, Lifetime::Expression { .. });
        let manual = matches!(def.rate, Rate::Manual { .. });
        if !expression && !manual && !run.started(emitter as usize) {
            let at = match def.lifetime {
                Lifetime::Looping { .. } => run.ambient_start(emitter as usize),
                _ => now,
            };
            run.start(emitter, None, NO_CONTROLLER, SOURCE_EVENT, at);
        }
    }
    run.expressions(input.seed);

    let slots = run.playback().len() / PLAYBACK_FLOATS;
    let mut table = [TIME_NEVER; PLAYBACK_SLOTS * PLAYBACK_FLOATS];
    let mut tracked = 0;
    let fresh = sources.caught.iter().any(|caught| caught.fresh);
    let live = &mut scratch.live;
    let clock = [
        if last.is_finite() {
            last.min(now)
        } else {
            now - dt
        },
        now,
    ];
    for playing in sources.playing {
        let elapsed = playing.elapsed;
        let [previous, to] = crossing_span(elapsed, playing.from, playing.anchor, clock);
        let mut first_seen = playing.from.is_none() && !last.is_finite();
        first_seen |= playing.first && previous >= 0.0;
        if tracked < slots && particle_keys(e, playing.animation) {
            let id = playing.clip as f32;
            first_seen |= playing.late && previous >= 0.0 && !run.seen(id, now - dt - previous);
            let start = now - elapsed;
            table[tracked * PLAYBACK_FLOATS] = start;
            table[tracked * PLAYBACK_FLOATS + 1] = start + 1.0 + id;
            tracked += 1;
        }
        if fresh {
            let start = match playing.from {
                _ if first_seen => f32::NEG_INFINITY,
                Some(from) => from,
                None => previous,
            };
            live.push((playing.clip, playing.age, start));
        }
        if first_seen {
            run.rebuild(playing);
            continue;
        }
        match playing.from {
            Some(from) => {
                let span = elapsed - from.max(0.0);
                run.keyframes(playing, [from, elapsed], dt, |at| {
                    if span > 0.0 {
                        ((elapsed - at) / span * dt).min(dt)
                    } else {
                        0.0
                    }
                });
            }
            None => run.keyframes(playing, [previous, to], dt, |at| (to - at).min(dt)),
        }
    }
    let playback = run.playback();
    let floats = playback.len();
    run.times[playback].copy_from_slice(&table[..floats]);
    for caught in sources.caught {
        let live = live
            .iter()
            .filter(|&&(clip, age, _)| {
                caught.fresh && clip == caught.clip && (age - caught.age).abs() <= SAME_ENTRY
            })
            .fold(f32::INFINITY, |at, &(_, _, start)| at.min(start));
        run.caught(caught, live);
    }

    for fire in sources.controller {
        let locator = fire.effect.locator.as_deref().and_then(|l| e.locator(l));
        let ago = fire.ago.max(0.0);
        let at = now - ago;
        if ago > 0.0 && !fire.fresh {
            run.backdate(fire, locator, ago);
            continue;
        }
        match fire.effect.kind {
            EffectKind::Particle => {
                run.named(&fire.effect.name, locator, fire.controller, fire.state, at)
            }
            EffectKind::Sound => {
                let gate = Gate::located(controller_edge(fire.controller));
                run.sound_or_named(
                    &fire.effect.name,
                    locator,
                    fire.controller,
                    fire.state,
                    Some(gate),
                    at,
                )
            }
        }
    }
    run.flush();

    run.reactions(sources.reactions, input.triggers);

    for &named in sources.named {
        if let Some((_, range)) = e.named.get(named as usize) {
            run.fire(range.clone(), None, NO_CONTROLLER, SOURCE_EVENT);
        }
    }
    run.flush();
    run.budget.left()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{parse, Animation};
    use crate::controller::{ControllerSet, Effect};
    use crate::cost::Features;
    use crate::effects::format::tests::sample;
    use crate::effects::geometry::tests::{centers, locators, view};
    use crate::effects::geometry::{draw, Locators, QuadSink, WearerEffects};
    use crate::effects::{declarations, step_with, ControllerFire, EMITTER_TIMES, QUAD_FLOATS};
    use crate::molang::{Interner, Program, FRAME_FUEL};
    use crate::reactions::{Reaction, ReactionOut};
    use crate::signals::{SignalState, SIGNAL_COUNT};
    use crate::wearer::{CosmeticSlot, WearerDeclarations, WearerTable};

    const TIP: [f32; 3] = [0.0, 4.0, -6.0];

    struct Bench {
        table: WearerTable,
        slot: CosmeticSlot,
        e: EffectSet,
        root: Affine,
    }

    impl Bench {
        fn new() -> Bench {
            Bench::with(sample())
        }

        fn with(e: EffectSet) -> Bench {
            let mut table = WearerTable::new();
            let slot = table.add_cosmetic(declarations(&e)).unwrap();
            Bench {
                table,
                slot,
                e,
                root: Affine::translation([100.0, 64.0, -3.0]).mul(&Affine::scale([1.0 / 16.0; 3])),
            }
        }

        fn frame(
            &mut self,
            clock: f64,
            f: Features,
            sources: &Sources<'_>,
            triggers: ReactionOut,
        ) -> Vec<Emission> {
            self.table.begin_frame(clock);
            let mut rec = self
                .table
                .draw(self.slot, 1, &[0.0; SIGNAL_COUNT], 0)
                .unwrap();
            let players: Vec<Affine> = PlayerLocator::ALL
                .iter()
                .map(|l| Affine::translation(l.offset()))
                .collect();
            let models = [Affine::translation(TIP)];
            let input = EffectInput {
                model_locators: &models,
                player_locators: &players,
                root: &self.root,
                features: f,
                triggers: &triggers,
                seed: 5,
                seed_epoch: rec.epoch,
                fuel: FRAME_FUEL,
            };
            let mut out = Vec::new();
            step_with(&self.e, &mut rec, &input, sources, &mut out);
            out
        }

        fn quiet(&mut self, clock: f64, sources: &Sources<'_>) -> Vec<Emission> {
            self.frame(clock, Features::ALL, sources, ReactionOut::default())
        }
    }

    fn wave() -> Vec<Animation> {
        parse(
            r#"{"animations":{"wave":{"loop":true,"animation_length":1,
                "particle_effects":{"0.5":{"effect":"sparkle"}},
                "sound_effects":{"0.25":{"effect":"pop"}}}}}"#,
            &mut Interner::default(),
        )
        .unwrap()
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    #[test]
    fn effects_declare_two_times_per_emitter_and_shared_rings() {
        let e = sample();
        assert_eq!(
            declarations(&e),
            WearerDeclarations {
                effect_times: e.emitters.len() * EMITTER_TIMES + 1,
                rings: 2,
                ring_points: 42 + 21,
                ring_turns: 42,
                ring_gates: 21,
                ..WearerDeclarations::default()
            }
        );
        let mut watching = sample();
        let clips = [fire_join_at("feet"), looping_join(), wave()].concat();
        let shot = |on, play| Reaction {
            signal: 0,
            on,
            play: Some(play),
            restart: false,
            blend: 0.0,
            speed: None,
            palette: None,
            effect: None,
        };
        let reactions = [shot(On::While, 0), shot(On::Start, 1), shot(On::While, 9)];
        watching.playback_slots = playback_slots(&e, &clips, &ControllerSet::default(), &reactions);
        assert_eq!(
            watching.playback_slots, 4,
            "the direct play, a held play, and a one-shot with the one it may replace"
        );
        assert_eq!(
            declarations(&watching).effect_times - declarations(&e).effect_times,
            4 * 2,
            "one start and one clip code per concurrent particle playback"
        );
        let many = vec![shot(On::While, 0); 40];
        assert_eq!(
            usize::from(playback_slots(&e, &clips, &ControllerSet::default(), &many)),
            PLAYBACK_SLOTS
        );
        assert_eq!(
            playback_slots(&e, &[], &ControllerSet::default(), &reactions),
            0,
            "no clip with particle keys needs no table"
        );
        assert_eq!(
            declarations(&EffectSet::default()),
            WearerDeclarations::default()
        );
    }

    #[test]
    fn effects_events_carry_position_volume_and_pitch() {
        let mut bench = Bench::new();
        let ids = [
            bench.e.named_id("join").unwrap(),
            bench.e.named_id("sparkle").unwrap(),
        ];
        let sources = Sources {
            named: &ids,
            ..Sources::default()
        };
        let out = bench.quiet(10.0, &sources);
        let chime = out.iter().find(|e| e.kind == EmitKind::Sound).unwrap();
        assert_eq!((chime.effect, chime.volume, chime.pitch), (0, 1.0, 1.5));
        assert!(close(chime.at, [0.0; 3]));
        let flames: Vec<&Emission> = out
            .iter()
            .filter(|e| e.kind == EmitKind::VanillaParticle)
            .collect();
        assert_eq!(flames.len(), 2);
        assert!(flames
            .iter()
            .all(|f| f.effect == 3 && f.source == SOURCE_EVENT));
        assert!(
            close(flames[0].at, [0.0, 0.25, -0.375]),
            "{:?}",
            flames[0].at
        );

        let tip = Effect {
            kind: EffectKind::Sound,
            name: "pop".to_string(),
            locator: Some("tip".to_string()),
            bind_to_actor: false,
        };
        let fires = [ControllerFire {
            controller: 2,
            state: 5,
            effect: &tip,
            ago: 0.0,
            emitter: None,
            fresh: false,
        }];
        let out = bench.quiet(
            10.1,
            &Sources {
                controller: &fires,
                ..Sources::default()
            },
        );
        assert_eq!(out.len(), 1);
        let pop = out[0];
        assert_eq!(
            (
                pop.kind,
                pop.controller,
                pop.source,
                pop.effect,
                pop.volume,
                pop.pitch
            ),
            (EmitKind::Sound, 2, 5, 1, 0.5, 1.0)
        );
        assert!(close(pop.at, [0.0, 0.25, -0.375]));
    }

    fn fire_join_at(locator: &str) -> Vec<Animation> {
        let json = format!(
            r#"{{"animations":{{"wave":{{"animation_length":1,
                "particle_effects":{{"0.1":{{"effect":"join","locator":"{locator}"}}}}}}}}}}"#
        );
        parse(&json, &mut Interner::default()).unwrap()
    }

    fn local(mut e: EffectSet) -> EffectSet {
        for emitter in &mut e.emitters {
            emitter.world = false;
            emitter.ring = None;
        }
        e
    }

    fn drawn(bench: &mut Bench, particle: usize) -> Vec<[f32; 3]> {
        let (model, player) = locators(bench.root);
        let record = bench.table.record(bench.slot, 1).unwrap();
        let signals = SignalState::new(0.0);
        let w = WearerEffects {
            locators: Locators {
                model: &model,
                player: &player,
                root: bench.root,
            },
            times: record.effect_times,
            rings: &[],
            signals: &signals,
            now: record.now,
            epoch: record.epoch,
            seen: (record.born - record.epoch) as f32,
            seed_epoch: record.epoch,
            seed: 9,
            tones: &[],
            features: Features::ALL,
        };
        let mut out = vec![0.0; 256 * QUAD_FLOATS];
        let mut sink = QuadSink::new(&mut out);
        draw(
            &bench.e,
            particle,
            &w,
            &view(),
            0,
            &mut sink,
            &mut Budget::new(u32::MAX),
        );
        let count = sink.count();
        let mut centers = centers(&out, count);
        centers.sort_by(|a, b| a.partial_cmp(b).unwrap());
        centers
    }

    fn same_particles(a: &[[f32; 3]], b: &[[f32; 3]]) {
        assert!(!a.is_empty());
        assert_eq!(
            a.len(),
            b.len(),
            "{a:?}
{b:?}"
        );
        for (x, y) in a.iter().zip(b) {
            assert!((0..3).all(|i| (x[i] - y[i]).abs() < 5e-3), "{x:?} vs {y:?}");
        }
    }

    #[test]
    fn effects_late_joiners_see_the_same_ambient_particles() {
        let mut steady = local(sample());
        steady.emitters[0].lifetime = Lifetime::Looping {
            active: 0.6,
            sleep: 0.35,
        };
        let mut bursts = steady.clone();
        bursts.emitters[0].rate = Rate::Instant { count: 3 };
        let mut endless = steady.clone();
        endless.emitters[0].lifetime = Lifetime::Expression {
            activation: Program::number(1.0),
            expiration: Program::number(0.0),
        };
        let joined = 1007.37;
        for (e, settled) in [(steady, joined), (bursts, joined), (endless, joined + 2.0)] {
            let mut early = Bench::with(e.clone());
            let mut late = Bench::with(e);
            for clock in [1000.0, 1003.3, 1006.05] {
                early.quiet(clock, &Sources::default());
            }
            late.quiet(joined, &Sources::default());
            for clock in [1008.0, 1009.25, 1009.4, 1012.6] {
                early.quiet(clock, &Sources::default());
                late.quiet(clock, &Sources::default());
                if clock > settled {
                    same_particles(&drawn(&mut early, 0), &drawn(&mut late, 0));
                }
            }
        }
    }

    fn looping_join() -> Vec<Animation> {
        parse(
            r#"{"animations":{"wave":{"loop":true,"animation_length":1,
                "particle_effects":{"0.25":{"effect":"join"}}}}}"#,
            &mut Interner::default(),
        )
        .unwrap()
    }
}
