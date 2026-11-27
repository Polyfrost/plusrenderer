use std::cell::RefCell;

use serde_json::{Map, Value};

use crate::animation::{Animation, Disabled, KEPT_DELAY_FLOATS};
use crate::container::MANIFEST;
use crate::controller::{blend_bone, Anchor, Clips, Played};
use crate::effects::EffectSet;
use crate::issues::{join, path, Issue, IssueCode, Mode, Report};
use crate::limits::EVENT_EFFECT_SECONDS;
use crate::molang::Scope;
use crate::palette::Palette;
use crate::pose::{BonePose, Pose};
use crate::signals::{is_bool_signal, signal_id, SignalState};
use crate::wearer::{MAX_ONE_SHOTS, ONE_SHOT_EMPTY};

pub const MAX_REACTIONS: usize = 32;
pub const PALETTE_SHOT_SECONDS: f32 = EVENT_EFFECT_SECONDS;
pub const APPEAR: u8 = u8::MAX;
pub const EQUIP: u8 = u8::MAX - 1;
pub const FLOATS_PER_REACTION: usize = 6;
pub const CLOCK_FLOATS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum On {
    Start,
    Stop,
    While,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reaction {
    pub signal: u8,
    pub on: On,
    pub play: Option<u16>,
    pub restart: bool,
    pub blend: f32,
    pub speed: Option<f32>,
    pub palette: Option<u16>,
    pub effect: Option<u16>,
}

pub struct ReactionRecord<'a> {
    pub wearer: u64,
    pub now: f32,
    pub epoch: f64,
    pub shifted: f32,
    pub appear: bool,
    pub equip: bool,
    pub times_reset: bool,
    pub signals: &'a SignalState,
    pub one_shot_ids: &'a mut [u32],
    pub one_shot_starts: &'a mut [f32],
    pub floats: &'a mut [f32],
    pub delays: &'a mut [f32],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReactionOut {
    pub colorway: Option<u16>,
    pub held: u32,
    pub fired: u32,
}

const KEYS: [&str; 8] = [
    "signal", "on", "play", "restart", "blend", "speed", "palette", "effect",
];
const ACTIONS: [&str; 4] = ["play", "speed", "palette", "effect"];
const REACTIONS: &str = "reactions";
const PREVIOUS: usize = 0;
const WEIGHT_EDGE: usize = 1;
const TIME_EDGE: usize = 2;
const WEIGHT: usize = 3;
const TIME: usize = 4;
const SAMPLED: usize = 5;
const CLOCK_LEAD: usize = 0;
const CLOCK_ELAPSED: usize = 1;
const RESTART_SLACK: f32 = 1e-3;
const WALL_SHARE_STEPS: usize = 24;

thread_local! {
    static SCRATCH: RefCell<[Pose; 2]> = const { RefCell::new([Pose(Vec::new()), Pose(Vec::new())]) };
}

struct Reader<'a> {
    report: &'a mut Report,
    authoring: bool,
}

impl Reader<'_> {
    fn hard(&mut self, issue: Issue) {
        self.report.push(issue);
    }

    fn soft(&mut self, issue: Issue) {
        if self.authoring {
            self.report.push(issue);
        }
    }
}

fn at(index: usize, key: &str) -> String {
    let base = join(REACTIONS, &index.to_string());
    path(MANIFEST, &join(&base, key))
}

pub fn resolve(anims: &[Animation], name: &str) -> Option<usize> {
    if let Some(index) = anims.iter().position(|a| a.name == name) {
        return Some(index);
    }
    let mut found = anims.iter().enumerate().filter(|(_, a)| {
        a.name
            .strip_suffix(name)
            .is_some_and(|rest| rest.ends_with('.'))
    });
    match (found.next(), found.next()) {
        (Some((index, _)), None) => Some(index),
        _ => None,
    }
}

pub fn parse(
    manifest: &Value,
    anims: &[Animation],
    p: &Palette,
    e: &EffectSet,
    r: &mut Report,
) -> Vec<Reaction> {
    let Some(value) = manifest.get(REACTIONS) else {
        return Vec::new();
    };
    let authoring = r.mode() == Mode::Authoring;
    let mut reader = Reader {
        report: r,
        authoring,
    };
    let Some(items) = value.as_array() else {
        reader.hard(Issue::error(
            IssueCode::ReactionInvalid,
            path(MANIFEST, REACTIONS),
        ));
        return Vec::new();
    };
    if items.len() > MAX_REACTIONS {
        reader.soft(
            Issue::error(IssueCode::ReactionInvalid, path(MANIFEST, REACTIONS))
                .value("count", items.len() as f64)
                .value("limit", MAX_REACTIONS as f64),
        );
    }
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate().take(MAX_REACTIONS) {
        let context = Context {
            index,
            anims,
            palette: p,
            effects: e,
        };
        if let Some(reaction) = one(&mut reader, &context, item) {
            out.push(reaction);
        }
    }
    out
}

struct Context<'a> {
    index: usize,
    anims: &'a [Animation],
    palette: &'a Palette,
    effects: &'a EffectSet,
}

fn text<'v>(
    reader: &mut Reader<'_>,
    object: &'v Map<String, Value>,
    index: usize,
    key: &str,
) -> Option<Option<&'v str>> {
    match object.get(key) {
        None => Some(None),
        Some(Value::String(s)) => Some(Some(s.as_str())),
        Some(_) => {
            reader.hard(Issue::error(IssueCode::ReactionInvalid, at(index, key)));
            None
        }
    }
}

fn number(
    reader: &mut Reader<'_>,
    object: &Map<String, Value>,
    index: usize,
    key: &str,
    positive: bool,
) -> Option<Option<f32>> {
    let Some(value) = object.get(key) else {
        return Some(None);
    };
    let found = value
        .as_f64()
        .map(|n| n as f32)
        .filter(|n| n.is_finite() && if positive { *n > 0.0 } else { *n >= 0.0 });
    match found {
        Some(n) => Some(Some(n)),
        None => {
            let mut issue = Issue::error(IssueCode::ReactionInvalid, at(index, key));
            if let Some(n) = value.as_f64() {
                issue = issue.value("value", n);
            }
            reader.hard(issue);
            None
        }
    }
}

fn trigger(
    reader: &mut Reader<'_>,
    index: usize,
    on: Option<&str>,
    signal: Option<&str>,
) -> Option<(u8, On)> {
    let Some(on) = on else {
        reader.hard(Issue::error(IssueCode::ReactionInvalid, at(index, "on")));
        return None;
    };
    let on = match on {
        "appear" | "equip" => {
            if signal.is_some() {
                reader.soft(Issue::error(
                    IssueCode::ReactionInvalid,
                    at(index, "signal"),
                ));
            }
            let sentinel = if on == "equip" { EQUIP } else { APPEAR };
            return Some((sentinel, On::Start));
        }
        "start" => On::Start,
        "stop" => On::Stop,
        "while" => On::While,
        other => {
            reader
                .soft(Issue::error(IssueCode::ReactionInvalid, at(index, "on")).name("on", other));
            return None;
        }
    };
    let Some(name) = signal else {
        reader.hard(Issue::error(
            IssueCode::ReactionInvalid,
            at(index, "signal"),
        ));
        return None;
    };
    let Some(id) = signal_id(name) else {
        reader.soft(
            Issue::error(IssueCode::ReactionSignalUnknown, at(index, "signal"))
                .name("signal", name),
        );
        return None;
    };
    if !is_bool_signal(id) {
        reader.soft(
            Issue::error(IssueCode::ReactionInvalid, at(index, "signal")).name("signal", name),
        );
        return None;
    }
    Some((id as u8, on))
}

fn found<T>(
    reader: &mut Reader<'_>,
    index: usize,
    key: &str,
    name: &str,
    value: Option<T>,
) -> Option<T> {
    if value.is_none() {
        reader.soft(Issue::error(IssueCode::ReactionMissing, at(index, key)).name(key, name));
    }
    value
}

fn one(reader: &mut Reader<'_>, cx: &Context<'_>, item: &Value) -> Option<Reaction> {
    let index = cx.index;
    let Some(object) = item.as_object() else {
        reader.hard(Issue::error(IssueCode::ReactionInvalid, at(index, "")));
        return None;
    };
    let base = join(REACTIONS, &index.to_string());
    reader.report.check_keys(MANIFEST, &base, object, &KEYS);

    let on = text(reader, object, index, "on")?;
    let signal = text(reader, object, index, "signal")?;
    let play = text(reader, object, index, "play")?;
    let palette = text(reader, object, index, "palette")?;
    let effect = text(reader, object, index, "effect")?;
    let restart = match object.get("restart") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            reader.hard(Issue::error(
                IssueCode::ReactionInvalid,
                at(index, "restart"),
            ));
            return None;
        }
    };
    let blend = number(reader, object, index, "blend", false)?.unwrap_or(0.0);
    let mut speed = number(reader, object, index, "speed", true)?;
    let (signal, on) = trigger(reader, index, on, signal)?;

    if speed.is_some() && on != On::While {
        reader.soft(Issue::error(IssueCode::ReactionInvalid, at(index, "speed")));
        speed = None;
    }
    let play = play.and_then(|name| {
        let index_of = resolve(cx.anims, name).and_then(|i| u16::try_from(i).ok());
        found(reader, index, "play", name, index_of)
    });
    let palette = palette.and_then(|name| {
        let colorway = cx
            .palette
            .colorway(name)
            .and_then(|i| u16::try_from(i).ok());
        found(reader, index, "palette", name, colorway)
    });
    let effect = effect.and_then(|name| {
        let named = cx.effects.named_id(name);
        found(reader, index, "effect", name, named)
    });

    if !ACTIONS.iter().any(|key| object.contains_key(*key)) {
        reader.soft(Issue::error(IssueCode::ReactionInvalid, at(index, "")));
        return None;
    }
    if play.is_none() && speed.is_none() && palette.is_none() && effect.is_none() {
        return None;
    }
    Some(Reaction {
        signal,
        on,
        play,
        restart,
        blend,
        speed,
        palette,
        effect,
    })
}

fn counted(rs: &[Reaction]) -> &[Reaction] {
    &rs[..rs.len().min(MAX_REACTIONS)]
}

fn speeds(rs: &[Reaction]) -> bool {
    counted(rs).iter().any(|r| r.speed.is_some())
}

pub fn paces(rs: &[Reaction]) -> bool {
    speeds(rs)
}

fn shoots(r: &Reaction) -> bool {
    r.play.is_some() || r.palette.is_some()
}

pub fn one_shots(rs: &[Reaction]) -> usize {
    counted(rs)
        .iter()
        .filter(|r| r.on != On::While && shoots(r))
        .count()
}

pub fn concurrent_plays(rs: &[Reaction], keyed: impl Fn(usize) -> bool) -> usize {
    concurrent_weight(rs, |clip| usize::from(keyed(clip)))
}

pub fn concurrent_weight(rs: &[Reaction], weight: impl Fn(usize) -> usize) -> usize {
    let rs = counted(rs);
    let plays = |on: fn(On) -> bool| -> Vec<usize> {
        rs.iter()
            .filter(|r| on(r.on))
            .filter_map(|r| r.play.map(|play| weight(usize::from(play))))
            .filter(|&w| w > 0)
            .collect()
    };
    let held = plays(|on| on == On::While)
        .into_iter()
        .fold(0, usize::saturating_add);
    let mut shots = plays(|on| on != On::While);
    shots.sort_unstable_by(|a, b| b.cmp(a));
    let shots = shots
        .into_iter()
        .take(MAX_ONE_SHOTS)
        .fold(0, usize::saturating_add);
    held.saturating_add(shots.saturating_mul(2))
}

pub fn fastest(rs: &[Reaction]) -> f32 {
    1.0 + counted(rs)
        .iter()
        .filter(|r| r.on == On::While)
        .filter_map(|r| r.speed)
        .map(|speed| (speed - 1.0).max(0.0))
        .sum::<f32>()
}

pub fn kept_delay_floats(rs: &[Reaction]) -> usize {
    let rs = counted(rs);
    if rs.is_empty() {
        0
    } else {
        (rs.len() + 1) * KEPT_DELAY_FLOATS
    }
}

fn kept(delays: &mut [f32], entry: usize) -> &mut [f32] {
    let range = entry * KEPT_DELAY_FLOATS..(entry + 1) * KEPT_DELAY_FLOATS;
    delays.get_mut(range).unwrap_or_default()
}

pub fn floats(rs: &[Reaction]) -> usize {
    counted(rs).len() * FLOATS_PER_REACTION + if speeds(rs) { CLOCK_FLOATS } else { 0 }
}

fn flag(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

fn while_weight(blend: f32, edge: f32, on: bool, since: f32) -> f32 {
    if blend <= 0.0 {
        return flag(on);
    }
    let step = since / blend;
    let weight = if on { edge + step } else { edge - step };
    weight.clamp(0.0, 1.0)
}

fn shot_weight(blend: f32, elapsed: f32, end: f32) -> f32 {
    if blend <= 0.0 {
        return 1.0;
    }
    (elapsed / blend)
        .min((end - elapsed) / blend)
        .clamp(0.0, 1.0)
}

fn start_shot(ids: &mut [u32], starts: &mut [f32], reaction: usize, start: f32, restart: bool) {
    let id = reaction as u32;
    if let Some(slot) = ids.iter().position(|&i| i == id) {
        if restart {
            starts[slot] = start;
        }
        return;
    }
    let free = ids.iter().position(|&i| i == ONE_SHOT_EMPTY);
    let oldest = || (0..ids.len()).min_by(|&a, &b| starts[a].total_cmp(&starts[b]));
    if let Some(slot) = free.or_else(oldest) {
        ids[slot] = id;
        starts[slot] = start;
    }
}

fn shot_end(
    animation: &Animation,
    clip: usize,
    scope: &mut Scope<'_>,
    started: f64,
    kept: &mut [f32],
) -> f32 {
    let [delay, _] = animation.delays_kept(clip, scope, started, kept);
    delay.max(0.0) + animation.length
}

fn disabled(words: &mut [u32], first: Option<usize>) -> Disabled<'_> {
    match first {
        Some(first) => Disabled::new(words, first),
        None => Disabled::none(),
    }
}

fn layer(
    index: usize,
    time: f32,
    weight: f32,
    clips: &mut Clips<'_>,
    scope: &mut Scope<'_>,
    scratch: &mut [Pose; 2],
    pose: &mut Pose,
) {
    let Some(animation) = clips.animations.get(index) else {
        return;
    };
    if weight.is_nan() || weight <= 0.0 {
        return;
    }
    let bind = clips.binds.get(index).map_or(&[][..], Vec::as_slice);
    let first = clips.offsets.get(index).copied();
    animation.pre_animation(scope);
    let [values, mask] = scratch;
    animation.pose(
        bind,
        time,
        f32::NAN,
        scope,
        disabled(clips.disabled, first),
        mask,
    );
    animation.pose(
        bind,
        time,
        1.0,
        scope,
        disabled(clips.disabled, first),
        values,
    );
    for ((target, value), marks) in pose.0.iter_mut().zip(&values.0).zip(&mask.0) {
        let mut to = *target;
        if marks.rotation[0].is_nan() {
            to.rotation = value.rotation;
        }
        if marks.position[0].is_nan() {
            to.position = value.position;
        }
        if marks.scale[0].is_nan() {
            to.scale = value.scale;
        }
        if to != *target {
            *target = blend_bone(target, &to, weight, true);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Ramp {
    scale: f32,
    blend: f32,
    edge: f32,
    before: f32,
    since: f32,
    on: bool,
}

impl Ramp {
    fn weight(&self, at: f32, changed: f32, after: bool) -> f32 {
        if after {
            while_weight(self.blend, self.edge, self.on, at - changed)
        } else {
            while_weight(self.blend, self.before, !self.on, at)
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Speed {
    ramps: [Ramp; MAX_REACTIONS],
    count: usize,
}

impl Speed {
    fn push(&mut self, ramp: Ramp) {
        if let Some(slot) = self.ramps.get_mut(self.count) {
            *slot = ramp;
            self.count += 1;
        }
    }

    fn ramps(&self) -> &[Ramp] {
        &self.ramps[..self.count]
    }

    pub fn current(&self) -> f32 {
        let speed: f32 = self
            .ramps()
            .iter()
            .map(|r| r.scale * while_weight(r.blend, r.edge, r.on, r.since))
            .sum();
        (1.0 + speed).max(0.0)
    }

    fn at(&self, at: f32, step: f32, after: impl Fn(&Ramp) -> bool) -> f32 {
        let speed: f32 = self
            .ramps()
            .iter()
            .map(|r| r.scale * r.weight(at, step - r.since, after(r)))
            .sum();
        1.0 + speed
    }

    pub fn over(&self, step: f32) -> f32 {
        self.until(step, step)
    }

    pub fn wall_share(&self, step: f32, share: f32) -> f32 {
        let total = self.over(step);
        if total.is_nan() || total <= 0.0 || share.is_nan() {
            return share;
        }
        let target = share.clamp(0.0, 1.0) * total;
        let (mut low, mut high) = (0.0f32, 1.0f32);
        for _ in 0..WALL_SHARE_STEPS {
            let mid = (low + high) / 2.0;
            if self.until(step, mid * step) < target {
                low = mid;
            } else {
                high = mid;
            }
        }
        (low + high) / 2.0
    }

    fn until(&self, step: f32, until: f32) -> f32 {
        if step.is_nan() || step <= 0.0 || step.is_infinite() || until.is_nan() {
            return 0.0;
        }
        let until = until.clamp(0.0, step);
        let mut marks = [0.0f32; 3 * MAX_REACTIONS + 2];
        let mut count = 0;
        let mut mark = |at: f32| {
            if at > 0.0 && at < until && count < marks.len() {
                marks[count] = at;
                count += 1;
            }
        };
        for r in self.ramps() {
            let changed = step - r.since;
            mark(changed);
            if r.blend > 0.0 {
                mark(changed + (flag(r.on) - r.edge).abs() * r.blend);
                mark((flag(!r.on) - r.before).abs() * r.blend);
            }
        }
        marks[count] = until;
        count += 1;
        let marks = &mut marks[..count];
        marks.sort_unstable_by(f32::total_cmp);
        let mut total = 0.0;
        let mut from = 0.0;
        for &to in marks.iter() {
            if to <= from {
                continue;
            }
            let mid = (from + to) / 2.0;
            let after = |r: &Ramp| mid >= step - r.since;
            let [a, b] = [self.at(from, step, after), self.at(to, step, after)];
            total += positive_area(a, b, to - from);
            from = to;
        }
        total
    }
}

fn positive_area(a: f32, b: f32, span: f32) -> f32 {
    if a >= 0.0 && b >= 0.0 {
        (a + b) / 2.0 * span
    } else if a <= 0.0 && b <= 0.0 {
        0.0
    } else {
        let high = a.max(b);
        high * high / (high - a.min(b)) / 2.0 * span
    }
}

struct Edges {
    out: ReactionOut,
    speed: Speed,
}

pub struct Lead {
    edges: Edges,
    ended: [Shot; MAX_ONE_SHOTS],
    ended_count: usize,
}

impl Lead {
    pub fn speed(&self) -> Speed {
        self.edges.speed
    }
}

pub fn lead(
    rs: &[Reaction],
    rec: &mut ReactionRecord<'_>,
    scope: &mut Scope<'_>,
    clips: &Clips<'_>,
) -> Option<Lead> {
    let rs = counted(rs);
    if !speeds(rs) || rec.floats.len() < floats(rs) {
        return None;
    }
    let mut ended = [(0.0, 0); MAX_ONE_SHOTS];
    let ended_count = prune(rs, rec, clips, scope, &mut ended);
    let edges = edges(rs, rec);
    Some(Lead {
        edges,
        ended,
        ended_count,
    })
}

fn edges(rs: &[Reaction], rec: &mut ReactionRecord<'_>) -> Edges {
    let now = rec.now;
    let reset = rec.times_reset;
    let mut out = ReactionOut::default();
    let mut speed = Speed::default();
    for (i, reaction) in rs.iter().enumerate() {
        let bit = 1u32 << i;
        if reaction.signal == APPEAR {
            if rec.appear {
                out.fired |= bit;
                if shoots(reaction) {
                    start_shot(rec.one_shot_ids, rec.one_shot_starts, i, now, true);
                }
            }
            continue;
        }
        if reaction.signal == EQUIP {
            if rec.equip {
                out.fired |= bit;
                if shoots(reaction) {
                    let ids = &mut *rec.one_shot_ids;
                    let starts = &mut *rec.one_shot_starts;
                    start_shot(ids, starts, i, now, reaction.restart);
                }
            }
            continue;
        }
        let floats = &mut rec.floats[i * FLOATS_PER_REACTION..(i + 1) * FLOATS_PER_REACTION];
        let id = usize::from(reaction.signal);
        let on = rec.signals.value(id) >= 0.5;
        let since = rec.signals.time(id, now).max(0.0);
        if reset {
            floats.copy_from_slice(&[flag(on), flag(on), 0.0, flag(on), 0.0, now]);
        } else {
            floats[SAMPLED] -= rec.shifted;
        }
        let edge = !reset && (floats[PREVIOUS] >= 0.5) != on;
        floats[PREVIOUS] = flag(on);
        match reaction.on {
            On::Start | On::Stop => {
                if edge && on == (reaction.on == On::Start) {
                    out.fired |= bit;
                    if shoots(reaction) {
                        let ids = &mut *rec.one_shot_ids;
                        let starts = &mut *rec.one_shot_starts;
                        start_shot(ids, starts, i, now - since, reaction.restart);
                    }
                }
            }
            On::While => {
                let before = floats[WEIGHT];
                if edge {
                    let gap = now - since - floats[SAMPLED];
                    let gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
                    let weight = while_weight(reaction.blend, before, !on, gap);
                    let fresh = on && (reaction.restart || weight <= 0.0);
                    floats[WEIGHT_EDGE] = weight;
                    floats[TIME_EDGE] = if fresh { 0.0 } else { floats[TIME] + gap };
                }
                let weight = while_weight(reaction.blend, floats[WEIGHT_EDGE], on, since);
                floats[WEIGHT] = weight;
                floats[TIME] = floats[TIME_EDGE] + since;
                floats[SAMPLED] = now;
                if on {
                    out.held |= bit;
                }
                if let Some(scale) = reaction.speed {
                    speed.push(Ramp {
                        scale: scale - 1.0,
                        blend: reaction.blend,
                        edge: floats[WEIGHT_EDGE],
                        before,
                        since,
                        on,
                    });
                }
            }
        }
    }
    Edges { out, speed }
}

fn prune(
    rs: &[Reaction],
    rec: &mut ReactionRecord<'_>,
    clips: &Clips<'_>,
    scope: &mut Scope<'_>,
    ended: &mut [Shot; MAX_ONE_SHOTS],
) -> usize {
    let mut count = 0;
    for slot in 0..rec.one_shot_ids.len() {
        let id = rec.one_shot_ids[slot];
        if id == ONE_SHOT_EMPTY {
            continue;
        }
        let reaction = rs.get(id as usize);
        let play = reaction.and_then(|r| r.play).map(usize::from);
        let animation = play.and_then(|play| clips.animations.get(play));
        let start = rec.one_shot_starts[slot];
        let elapsed = rec.now - start;
        let started = rec.epoch + f64::from(start);
        let kept = kept(rec.delays, 1 + id as usize);
        let alive = match (play, animation) {
            (Some(play), Some(a)) => elapsed <= shot_end(a, play, scope, started, kept),
            (Some(_), None) => false,
            (None, _) => {
                reaction.is_some_and(|r| r.palette.is_some()) && elapsed <= PALETTE_SHOT_SECONDS
            }
        };
        if !alive {
            rec.one_shot_ids[slot] = ONE_SHOT_EMPTY;
            if animation.is_some() && count < ended.len() {
                ended[count] = (start, id as usize);
                count += 1;
            }
        }
    }
    count
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paced {
    pub time: f32,
    pub from: Option<f32>,
}

pub fn pace(
    rs: &[Reaction],
    rec: &mut ReactionRecord<'_>,
    speed: &Speed,
    elapsed: f32,
) -> Option<Paced> {
    let rs = counted(rs);
    if !speeds(rs) || !elapsed.is_finite() {
        return None;
    }
    let clock = rec
        .floats
        .get_mut(rs.len() * FLOATS_PER_REACTION..)?
        .get_mut(..CLOCK_FLOATS)?;
    let [lead, last] = [clock[CLOCK_LEAD], clock[CLOCK_ELAPSED]];
    let fresh =
        rec.times_reset || !lead.is_finite() || !last.is_finite() || elapsed + RESTART_SLACK < last;
    let (lead, from) = if fresh {
        (elapsed * (speed.current() - 1.0), None)
    } else {
        let step = (elapsed - last).max(0.0);
        let lead = (lead + speed.over(step) - step).max(-elapsed);
        (lead, Some(last + clock[CLOCK_LEAD]))
    };
    clock[CLOCK_LEAD] = lead;
    clock[CLOCK_ELAPSED] = elapsed;
    Some(Paced {
        time: elapsed + lead,
        from,
    })
}

type Shot = (f32, usize);

fn running(rec: &ReactionRecord<'_>, shots: &mut [Shot; MAX_ONE_SHOTS]) -> usize {
    let mut count = 0;
    for (slot, &id) in rec.one_shot_ids.iter().enumerate() {
        if id == ONE_SHOT_EMPTY || count == shots.len() {
            continue;
        }
        let entry = (rec.one_shot_starts[slot], id as usize);
        let at = shots[..count]
            .iter()
            .position(|s| s.0 > entry.0 || (s.0 == entry.0 && s.1 < entry.1))
            .unwrap_or(count);
        shots.copy_within(at..count, at + 1);
        shots[at] = entry;
        count += 1;
    }
    count
}

fn colorway(rs: &[Reaction], shots: &[Shot], held: u32) -> Option<u16> {
    shots
        .iter()
        .rev()
        .find_map(|&(_, id)| rs[id].palette)
        .or_else(|| {
            rs.iter()
                .enumerate()
                .filter(|(i, r)| r.on == On::While && held & (1 << i) != 0)
                .find_map(|(_, r)| r.palette)
        })
}

pub fn held(rs: &[Reaction], rec: &mut ReactionRecord<'_>) {
    let rs = counted(rs);
    if rs.is_empty() || rec.floats.len() < floats(rs) {
        return;
    }
    edges(rs, rec);
}

pub fn idle(rs: &[Reaction], rec: &mut ReactionRecord<'_>) {
    held(rs, rec);
    rec.one_shot_ids.fill(ONE_SHOT_EMPTY);
}

pub fn overlay(
    rs: &[Reaction],
    rec: &mut ReactionRecord<'_>,
    scope: &mut Scope<'_>,
    clips: &mut Clips<'_>,
    pose: &mut Pose,
    played: &mut Vec<Played>,
) -> ReactionOut {
    overlay_with(rs, rec, scope, clips, pose, played, None)
}

pub fn overlay_led(
    rs: &[Reaction],
    rec: &mut ReactionRecord<'_>,
    scope: &mut Scope<'_>,
    clips: &mut Clips<'_>,
    pose: &mut Pose,
    played: &mut Vec<Played>,
    lead: Lead,
) -> ReactionOut {
    overlay_with(rs, rec, scope, clips, pose, played, Some(lead))
}

fn overlay_with(
    rs: &[Reaction],
    rec: &mut ReactionRecord<'_>,
    scope: &mut Scope<'_>,
    clips: &mut Clips<'_>,
    pose: &mut Pose,
    played: &mut Vec<Played>,
    lead: Option<Lead>,
) -> ReactionOut {
    let rs = counted(rs);
    if rs.is_empty() || rec.floats.len() < floats(rs) {
        return ReactionOut::default();
    }
    let (ended, ended_count, mut out) = match lead {
        Some(lead) => (lead.ended, lead.ended_count, lead.edges.out),
        None => {
            let mut ended = [(0.0, 0); MAX_ONE_SHOTS];
            let ended_count = prune(rs, rec, clips, scope, &mut ended);
            let Edges { out, .. } = edges(rs, rec);
            (ended, ended_count, out)
        }
    };
    let ended = &ended[..ended_count];
    let mut shots = [(0.0, 0); MAX_ONE_SHOTS];
    let count = running(rec, &mut shots);
    let shots = &shots[..count];
    out.colorway = colorway(rs, shots, out.held);

    let animations = clips.animations;
    let first_frame = clips.first_frame;
    let now = rec.now;
    let epoch = rec.epoch;
    let signals = rec.signals;
    let floats = &*rec.floats;
    let delays = &mut *rec.delays;
    let mut record =
        |id: usize, clip: usize, elapsed: f32, delays: [f32; 2], anchor: Option<Anchor>| {
            if animations.get(clip).is_some_and(|a| !a.events.is_empty()) {
                let play = Played::new(clip, elapsed, elapsed, None, delays);
                played.push(play.anchored(anchor).reacting(id));
            }
        };
    SCRATCH.with(|scratch| {
        let Ok(mut scratch) = scratch.try_borrow_mut() else {
            return;
        };
        for buffer in scratch.iter_mut() {
            buffer.0.resize(pose.0.len(), BonePose::default());
        }
        for (i, reaction) in rs.iter().enumerate().rev() {
            let (On::While, Some(play)) = (reaction.on, reaction.play) else {
                continue;
            };
            let Some(animation) = animations.get(usize::from(play)) else {
                continue;
            };
            let own = &floats[i * FLOATS_PER_REACTION..(i + 1) * FLOATS_PER_REACTION];
            let (weight, clock) = if first_frame {
                (flag(own[PREVIOUS] >= 0.5), 0.0)
            } else {
                (own[WEIGHT], own[TIME])
            };
            let changed = signals.changed_at(usize::from(reaction.signal));
            let started = changed.map_or(epoch, |at| {
                epoch + f64::from(at) - f64::from(own[TIME_EDGE])
            });
            let delays =
                animation.delays_kept(usize::from(play), scope, started, kept(delays, 1 + i));
            if weight > 0.0 {
                let anchor = changed
                    .filter(|&at| at <= now)
                    .map(|at| Anchor::based(own[TIME_EDGE], at));
                record(i, usize::from(play), own[TIME], delays, anchor);
            }
            if let Some(time) = animation.local_time(clock, delays) {
                layer(
                    usize::from(play),
                    time,
                    weight,
                    clips,
                    scope,
                    &mut scratch,
                    pose,
                );
            }
        }
        if first_frame {
            return;
        }
        for &(start, id) in ended {
            let Some(play) = rs[id].play else {
                continue;
            };
            let Some(animation) = animations.get(usize::from(play)) else {
                continue;
            };
            let started = epoch + f64::from(start);
            let kept = kept(delays, 1 + id);
            let delays = animation.delays_kept(usize::from(play), scope, started, kept);
            record(
                id,
                usize::from(play),
                now - start,
                delays,
                Some(Anchor::at(start)),
            );
        }
        for &(start, id) in shots {
            let Some(play) = rs[id].play else {
                continue;
            };
            let Some(animation) = animations.get(usize::from(play)) else {
                continue;
            };
            let elapsed = now - start;
            let started = epoch + f64::from(start);
            let play = usize::from(play);
            let end = shot_end(animation, play, scope, started, kept(delays, 1 + id));
            let delays = animation.delays_kept(play, scope, started, kept(delays, 1 + id));
            record(id, play, elapsed, delays, Some(Anchor::at(start)));
            let Some(time) = animation.local_time(elapsed, delays) else {
                continue;
            };
            let weight = shot_weight(rs[id].blend, elapsed, end);
            layer(play, time, weight, clips, scope, &mut scratch, pose);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::animation::{self, channel_offsets, flag_words};
    use crate::ir::{Bone, Model};
    use crate::issues::Mode;
    use crate::molang::{Interner, Runtime};
    use crate::palette::Colorway;
    use crate::pose::BonePose;
    use crate::signals::{signal_id, SIGNAL_COUNT};
    use crate::wearer::ONE_SHOT_EMPTY;

    const ANIMATIONS: &str = r#"{"format_version":"1.8.0","animations":{
        "animation.wings.idle":{"loop":true,"animation_length":2.0,"bones":{"wing":{
            "rotation":{"0.0":[0,0,0],"2.0":[0,0,40]}}}},
        "animation.wings.droop":{"loop":true,"animation_length":1.0,"bones":{"wing":{"rotation":[30,0,0]}}},
        "animation.wings.fold":{"loop":true,"animation_length":1.0,"bones":{"wing":{"rotation":[50,0,0]},
            "tip":{"scale":[2,2,2]}}},
        "animation.wings.flinch":{"animation_length":1.0,"bones":{"wing":{"rotation":[90,0,0]}}}
    }}"#;

    fn model() -> Model {
        let bone = |name: &str| Bone {
            name: name.to_string(),
            parent: None,
            parent_name: None,
            pivot: [0.0; 3],
            rotation: [0.0; 3],
            cubes: Vec::new(),
            glow: None,
            locators: Vec::new(),
            spring: None,
        };
        Model {
            identifier: "geometry.wings".to_string(),
            texture_size: [16.0, 16.0],
            bones: vec![bone("wing"), bone("tip")],
        }
    }

    fn palette() -> Palette {
        Palette {
            slots: Vec::new(),
            colorways: vec![
                Colorway {
                    id: "mint".to_string(),
                    overrides: Vec::new(),
                },
                Colorway {
                    id: "storm".to_string(),
                    overrides: Vec::new(),
                },
            ],
        }
    }

    fn effects() -> EffectSet {
        EffectSet {
            named: vec![("puff".to_string(), 0..0), ("burst".to_string(), 0..0)],
            ..EffectSet::default()
        }
    }

    fn signal(name: &str) -> usize {
        signal_id(name).unwrap()
    }

    fn manifest(reactions: Value) -> Value {
        json!({"format": 1, "kind": "item", "id": "wings", "reactions": reactions})
    }

    struct Bench {
        interner: Interner,
        animations: Vec<animation::Animation>,
        binds: Vec<Vec<Option<usize>>>,
        offsets: Vec<usize>,
        disabled: Vec<u32>,
        variables: Vec<f32>,
        signals: SignalState,
        raw: [f32; SIGNAL_COUNT],
        ids: Vec<u32>,
        starts: Vec<f32>,
        floats: Vec<f32>,
        delays: Vec<f32>,
        reactions: Vec<Reaction>,
        first: bool,
        equip: bool,
        first_frame: bool,
        shifted: f32,
        played: Vec<Played>,
    }

    impl Bench {
        fn new(reactions: Value) -> Bench {
            let mut interner = Interner::default();
            let animations = animation::parse(ANIMATIONS, &mut interner).unwrap();
            let mut report = Report::new(Mode::Authoring);
            let reactions = parse(
                &manifest(reactions),
                &animations,
                &palette(),
                &effects(),
                &mut report,
            );
            assert_eq!(report.issues(), &[]);
            let model = model();
            let binds = animations.iter().map(|a| a.bind(&model)).collect();
            let (offsets, channels) = channel_offsets(&animations);
            let floats = vec![0.0; super::floats(&reactions)];
            let delays = vec![0.0; kept_delay_floats(&reactions)];
            Bench {
                delays,
                variables: vec![0.0; interner.variable_count()],
                interner,
                animations,
                binds,
                offsets,
                disabled: vec![0; flag_words(channels)],
                signals: SignalState::new(0.0),
                raw: [0.0; SIGNAL_COUNT],
                ids: vec![ONE_SHOT_EMPTY; 4],
                starts: vec![0.0; 4],
                floats,
                reactions,
                first: true,
                equip: false,
                first_frame: false,
                shifted: 0.0,
                played: Vec::new(),
            }
        }

        fn set(&mut self, name: &str, on: bool) {
            self.raw[signal(name)] = if on { 1.0 } else { 0.0 };
        }

        fn frame_on(&mut self, now: f32, base: BonePose) -> (Pose, ReactionOut) {
            self.signals.step(&self.raw, None, 0, now);
            let mut pose = Pose(vec![base, BonePose::default()]);
            let mut runtime = Runtime::new(&self.interner);
            let mut scope = runtime.wearer_frame(&mut self.variables, 1, now as f64);
            let mut clips = Clips {
                animations: &self.animations,
                binds: &self.binds,
                offsets: &self.offsets,
                disabled: &mut self.disabled,
                first_frame: self.first_frame,
                props: None,
            };
            let mut rec = ReactionRecord {
                wearer: 1,
                now,
                epoch: 0.0,
                shifted: std::mem::take(&mut self.shifted),
                appear: self.first,
                equip: std::mem::take(&mut self.equip),
                times_reset: self.first,
                signals: &self.signals,
                one_shot_ids: &mut self.ids,
                one_shot_starts: &mut self.starts,
                floats: &mut self.floats,
                delays: &mut self.delays,
            };
            self.first = false;
            self.played.clear();
            let out = overlay(
                &self.reactions,
                &mut rec,
                &mut scope,
                &mut clips,
                &mut pose,
                &mut self.played,
            );
            (pose, out)
        }
    }

    #[test]
    fn reactions_overlay_per_channel_with_one_shots_over_while_over_base() {
        let mut bench = Bench::new(json!([
            { "signal": "is_in_rain", "on": "while", "play": "droop" },
            { "signal": "is_sneaking", "on": "while", "play": "fold" },
            { "signal": "is_hurt", "on": "start", "play": "flinch" }
        ]));
        let base = BonePose {
            rotation: [5.0, 0.0, 0.0],
            position: [0.0, 3.0, 0.0],
            scale: [1.0; 3],
        };
        let (pose, out) = bench.frame_on(0.0, base);
        assert_eq!(pose.0[0], base);
        assert_eq!(out, ReactionOut::default());

        bench.set("is_sneaking", true);
        let (pose, out) = bench.frame_on(0.1, base);
        assert_eq!(pose.0[0].rotation, [50.0, 0.0, 0.0]);
        assert_eq!(
            pose.0[0].position,
            [0.0, 3.0, 0.0],
            "fold leaves position alone"
        );
        assert_eq!(pose.0[1].scale, [2.0; 3]);
        assert_eq!(out.held, 0b010);

        bench.set("is_in_rain", true);
        let (pose, out) = bench.frame_on(0.2, base);
        assert_eq!(pose.0[0].rotation, [30.0, 0.0, 0.0], "first declared wins");
        assert_eq!(pose.0[1].scale, [2.0; 3], "fold still owns the tip");
        assert_eq!(out.held, 0b011);

        bench.set("is_hurt", true);
        let (pose, out) = bench.frame_on(0.3, base);
        assert_eq!(pose.0[0].rotation, [90.0, 0.0, 0.0], "one-shots beat while");
        assert_eq!(out.fired, 0b100);
        let (_, out) = bench.frame_on(0.4, base);
        assert_eq!(out.fired, 0, "a start fires once");
        let (pose, _) = bench.frame_on(1.5, base);
        assert_eq!(pose.0[0].rotation, [30.0, 0.0, 0.0], "the flinch ended");
        assert!(bench.ids.iter().all(|&id| id == ONE_SHOT_EMPTY));
    }
}
