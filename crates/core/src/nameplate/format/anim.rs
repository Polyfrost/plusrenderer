use serde_json::{Map, Value};

use crate::container::is_id;
use crate::issues::IssueCode;
use crate::limits::{
    NAMEPLATE_APPEAR_SECONDS, NAMEPLATE_APPEAR_SECONDS_MIN, NAMEPLATE_BURST, NAMEPLATE_LOOP_FRAMES,
    NAMEPLATE_OFFSET, NAMEPLATE_PATH_SPEED, NAMEPLATE_SCHEDULE_WEIGHT,
};
use crate::signals::{is_bool_signal, signal_id};
use crate::wearer::seed_key;

use super::super::queries::MolangUse;
use super::super::types::{
    Animations, Burst, Channels, Clip, CycleMode, Ease, Key, LetterOverride, OneShot, PaletteKey,
    PaletteTrack, PaletteValue, Reaction, Reduced, Schedule, ScheduledClip, SlotTrack, Target,
    TargetTracks, Track, Trigger, Value as KeyValue, CHANCE_ONE,
};
use super::read::{self, Loc};
use super::{Compiler, Kind, BADGE_TARGET, LOOP, PALETTE_PREFIX, PLATE_TARGET, TEXT_TARGET};

const ANIMATION_KEYS: [&str; 6] = [
    "schedule", "trigger", "length", "tracks", "restart", "reduced",
];
const SCHEDULE_KEYS: [&str; 5] = ["window", "jitter", "chance", "id", "weight"];
const TRIGGER_KEYS: [&str; 3] = ["reaction", "on", "appear"];
const CYCLE_KEYS: [&str; 2] = ["every", "mode"];
const DEFAULT_APPEAR_SECONDS: u8 = 5;

pub(super) fn ease(name: &str) -> Option<Ease> {
    match name {
        "step" => Some(Ease::Step),
        "linear" => Some(Ease::Linear),
        "ease" => Some(Ease::Ease),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ClipKind {
    Loop,
    While,
    OneShot,
    Schedule(String),
}

impl ClipKind {
    fn triggered(&self) -> bool {
        matches!(self, ClipKind::While | ClipKind::OneShot)
    }
}

enum Parsed {
    While(u8),
    Start(u8),
    Stop(u8),
    Appear(u8),
    Equip,
}

#[derive(Clone, Copy)]
enum Scalar {
    Frame(usize),
    Opacity,
    Visible,
}

#[derive(Default)]
pub(super) struct Drivers {
    targets: Vec<(Target, String)>,
    steps: Vec<(String, u16)>,
}

impl Compiler<'_, '_> {
    pub(super) fn animations(&mut self, manifest: &Map<String, Value>, root: &Loc) -> Animations {
        let mut animations = Animations {
            looping: Clip {
                name: LOOP.to_string(),
                length: self.loop_frames,
                ..Clip::default()
            },
            ..Animations::default()
        };
        let loc = root.key("animations");
        let Some(value) = manifest.get("animations") else {
            return animations;
        };
        let Some(map) = self.object(value, &loc, IssueCode::Animation).cloned() else {
            return animations;
        };
        let mut drivers = Drivers::default();
        for (name, value) in &map {
            let at = loc.key(name);
            if !is_id(name) {
                self.push(at.error(IssueCode::IdInvalid).name("id", name));
                continue;
            }
            let Some(spec) = self.object(value, &at, IssueCode::Animation) else {
                continue;
            };
            self.keys(&at, spec, &ANIMATION_KEYS);
            if name == LOOP {
                for key in ["trigger", "schedule", "length", "restart", "reduced"] {
                    if spec.contains_key(key) {
                        self.push(at.key(key).error(IssueCode::Animation));
                    }
                }
                animations.looping = self.clip(
                    name,
                    spec,
                    &at,
                    &ClipKind::Loop,
                    self.loop_frames,
                    &mut drivers,
                );
                continue;
            }
            match (spec.get("trigger"), spec.get("schedule")) {
                (Some(_), Some(_)) => self.push(at.key("schedule").error(IssueCode::Animation)),
                (None, None) => self.push(at.error(IssueCode::Animation)),
                (Some(trigger), None) => {
                    self.triggered(name, spec, trigger, &at, &mut animations, &mut drivers)
                }
                (None, Some(schedule)) => {
                    self.scheduled(name, spec, schedule, &at, &mut animations, &mut drivers)
                }
            }
        }
        animations
    }

    fn triggered(
        &mut self,
        name: &str,
        spec: &Map<String, Value>,
        trigger: &Value,
        at: &Loc,
        animations: &mut Animations,
        drivers: &mut Drivers,
    ) {
        let parsed = self.trigger(trigger, &at.key("trigger"));
        let while_ = matches!(parsed, Some(Parsed::While(_)));
        if let Some(value) = spec.get("reduced") {
            if while_ {
                if !matches!(value.as_str(), Some("hold" | "drop")) {
                    self.push(at.key("reduced").error(IssueCode::ReducedChoice));
                }
            } else if parsed.is_some() {
                self.push(at.key("reduced").error(IssueCode::ReducedWhile));
            }
        }
        let Some(parsed) = parsed else {
            return;
        };
        if let Parsed::While(signal) = parsed {
            if spec.contains_key("length") {
                self.push(at.key("length").error(IssueCode::Animation));
            }
            if spec.contains_key("restart") {
                self.push(at.key("restart").error(IssueCode::Reaction));
            }
            let reduced = match spec.get("reduced").and_then(Value::as_str) {
                Some("drop") => Reduced::Drop,
                _ => Reduced::Hold,
            };
            let clip = self.clip(name, spec, at, &ClipKind::While, self.loop_frames, drivers);
            animations.reactions.push(Reaction {
                signal,
                clip,
                reduced,
            });
            return;
        }
        if !spec.contains_key("length") {
            self.push(at.key("length").error(IssueCode::Animation));
            return;
        }
        let length = self.int(
            spec,
            "length",
            at,
            IssueCode::Animation,
            (1, i64::from(NAMEPLATE_LOOP_FRAMES)),
            1,
        ) as u16;
        let restart = self.boolean(spec, "restart", at, IssueCode::Reaction, false);
        let trigger = match parsed {
            Parsed::Start(s) => Trigger::Start(s),
            Parsed::Stop(s) => Trigger::Stop(s),
            Parsed::Appear(seconds) => Trigger::Appear(seconds),
            Parsed::Equip | Parsed::While(_) => Trigger::Equip,
        };
        let clip = self.clip(name, spec, at, &ClipKind::OneShot, length, drivers);
        animations.one_shots.push(OneShot {
            trigger,
            clip,
            restart,
        });
    }

    fn trigger(&mut self, value: &Value, loc: &Loc) -> Option<Parsed> {
        match value {
            Value::String(s) if s == "equip" => Some(Parsed::Equip),
            Value::String(s) if s == "appear" => Some(Parsed::Appear(DEFAULT_APPEAR_SECONDS)),
            Value::Object(map) => {
                self.keys(loc, map, &TRIGGER_KEYS);
                match (map.get("reaction"), map.get("appear")) {
                    (Some(_), Some(_)) => {
                        self.push(loc.error(IssueCode::Reaction));
                        None
                    }
                    (None, Some(_)) => {
                        let range = (
                            i64::from(NAMEPLATE_APPEAR_SECONDS_MIN),
                            i64::from(NAMEPLATE_APPEAR_SECONDS),
                        );
                        let before = self.bundle.issues().len();
                        let seconds = self.int(map, "appear", loc, IssueCode::Reaction, range, 0);
                        (self.bundle.issues().len() == before)
                            .then_some(Parsed::Appear(seconds as u8))
                    }
                    (Some(reaction), None) => {
                        let signal = reaction
                            .as_str()
                            .and_then(signal_id)
                            .filter(|id| is_bool_signal(*id))
                            .map(|id| id as u8);
                        if signal.is_none() {
                            let mut issue = loc.key("reaction").error(IssueCode::Reaction);
                            if let Some(name) = reaction.as_str() {
                                issue = issue.name("signal", name);
                            }
                            self.push(issue);
                        }
                        let on = match map.get("on").and_then(Value::as_str) {
                            Some(on @ ("while" | "start" | "stop")) => Some(on),
                            _ => {
                                self.push(loc.key("on").error(IssueCode::Reaction));
                                None
                            }
                        };
                        match (signal?, on?) {
                            (s, "while") => Some(Parsed::While(s)),
                            (s, "start") => Some(Parsed::Start(s)),
                            (s, _) => Some(Parsed::Stop(s)),
                        }
                    }
                    (None, None) => {
                        self.push(loc.error(IssueCode::Reaction));
                        None
                    }
                }
            }
            _ => {
                self.push(loc.error(IssueCode::Reaction));
                None
            }
        }
    }

    fn scheduled(
        &mut self,
        name: &str,
        spec: &Map<String, Value>,
        schedule: &Value,
        at: &Loc,
        animations: &mut Animations,
        drivers: &mut Drivers,
    ) {
        let sloc = at.key("schedule");
        let Some(map) = self.object(schedule, &sloc, IssueCode::Animation) else {
            return;
        };
        self.keys(&sloc, map, &SCHEDULE_KEYS);
        for key in ["restart", "reduced"] {
            if spec.contains_key(key) {
                self.push(at.key(key).error(IssueCode::Animation));
            }
        }
        if !map.contains_key("window") {
            self.push(sloc.key("window").error(IssueCode::Animation));
            return;
        }
        let window = self.int(
            map,
            "window",
            &sloc,
            IssueCode::Animation,
            (1, i64::from(u16::MAX)),
            1,
        );
        let jitter = self.int(map, "jitter", &sloc, IssueCode::Animation, (0, window), 0);
        let chance = self.num(map, "chance", &sloc, IssueCode::Animation, (0.0, 1.0), 1.0);
        let weight = self.int(
            map,
            "weight",
            &sloc,
            IssueCode::Animation,
            (1, i64::from(NAMEPLATE_SCHEDULE_WEIGHT)),
            1,
        );
        let id = match map.get("id") {
            None => name.to_string(),
            Some(value) => match value.as_str().filter(|s| is_id(s)) {
                Some(id) => id.to_string(),
                None => {
                    self.push(sloc.key("id").error(IssueCode::IdInvalid));
                    return;
                }
            },
        };
        if !spec.contains_key("length") {
            self.push(at.key("length").error(IssueCode::Animation));
            return;
        }
        let length = self.int(
            spec,
            "length",
            at,
            IssueCode::Animation,
            (1, i64::from(NAMEPLATE_LOOP_FRAMES)),
            1,
        );
        if length + jitter > window {
            self.push(
                at.key("length")
                    .error(IssueCode::Animation)
                    .value("length", length as f64)
                    .value("jitter", jitter as f64)
                    .value("window", window as f64),
            );
        }
        let chance = libm::floor(chance * f64::from(CHANCE_ONE) + 0.5) as u32;
        let index = match animations.schedules.iter().position(|s| s.id == id) {
            Some(i) => {
                let shared = &animations.schedules[i];
                let mismatched: Vec<&str> = [
                    ("window", i64::from(shared.window) != window),
                    ("jitter", i64::from(shared.jitter) != jitter),
                    ("chance", shared.chance != chance),
                ]
                .into_iter()
                .filter(|(_, bad)| *bad)
                .map(|(key, _)| key)
                .collect();
                for key in mismatched {
                    self.push(
                        sloc.key(key)
                            .error(IssueCode::Animation)
                            .name("schedule", &id),
                    );
                }
                i
            }
            None => {
                animations.schedules.push(Schedule {
                    id: id.clone(),
                    salt: seed_key(&id),
                    window: window as u16,
                    jitter: jitter as u16,
                    chance,
                    events: Vec::new(),
                });
                animations.schedules.len() - 1
            }
        };
        let clip = self.clip(
            name,
            spec,
            at,
            &ClipKind::Schedule(id),
            length as u16,
            drivers,
        );
        animations.schedules[index].events.push(ScheduledClip {
            clip,
            weight: weight as u16,
        });
    }

    fn clip(
        &mut self,
        name: &str,
        spec: &Map<String, Value>,
        at: &Loc,
        kind: &ClipKind,
        length: u16,
        drivers: &mut Drivers,
    ) -> Clip {
        let mut clip = Clip {
            name: name.to_string(),
            length,
            ..Clip::default()
        };
        let Some(value) = spec.get("tracks") else {
            return clip;
        };
        let tloc = at.key("tracks");
        let Some(tracks) = self.object(value, &tloc, IssueCode::Animation).cloned() else {
            return clip;
        };
        for (target_name, channels) in &tracks {
            let here = tloc.key(target_name);
            let Some(channels) = self.object(channels, &here, IssueCode::Animation) else {
                continue;
            };
            if let Some(slot) = target_name.strip_prefix(PALETTE_PREFIX) {
                if matches!(kind, ClipKind::Schedule(_)) {
                    self.push(here.error(IssueCode::PaletteAnimation));
                    continue;
                }
                if let Some(track) = self.palette_track(slot, channels, &here, kind, length) {
                    clip.palette.push(track);
                }
                continue;
            }
            if target_name == TEXT_TARGET {
                clip.letters = self.letters(channels, &here, kind);
                continue;
            }
            let target = match target_name.as_str() {
                BADGE_TARGET => Some(Target::Badge),
                PLATE_TARGET => Some(Target::Plate),
                other => self
                    .layer_index(other)
                    .map(Target::Layer)
                    .or_else(|| self.group_index(other).map(Target::Group)),
            };
            let Some(target) = target else {
                self.push(here.error(IssueCode::Animation).name("target", target_name));
                continue;
            };
            if let ClipKind::Schedule(id) = kind {
                if target == Target::Badge {
                    self.push(here.error(IssueCode::BadgeSchedule));
                    continue;
                }
                self.driver(target, id, &here, drivers);
            }
            let parsed = self.channels(target, channels, &here, kind, length);
            if *kind == ClipKind::Loop && parsed.frame.is_some() {
                if let Target::Layer(i) = target {
                    self.keyed_frames[usize::from(i)] = true;
                }
            }
            clip.targets.push(TargetTracks {
                target,
                channels: parsed,
            });
        }
        clip
    }

    fn driver(&mut self, target: Target, schedule: &str, loc: &Loc, drivers: &mut Drivers) {
        match drivers.targets.iter().find(|(t, _)| *t == target) {
            Some((_, other)) if other != schedule => {
                let other = other.clone();
                self.push(loc.error(IssueCode::Animation).name("schedule", &other));
            }
            Some(_) => {}
            None => drivers.targets.push((target, schedule.to_string())),
        }
        if let Target::Layer(i) = target {
            let step = self.decls[usize::from(i)].step;
            match drivers.steps.iter().find(|(s, _)| s == schedule) {
                Some((_, shared)) if *shared != step => {
                    let shared = *shared;
                    self.push(
                        loc.error(IssueCode::Animation)
                            .value("step", f64::from(step))
                            .value("shared", f64::from(shared)),
                    );
                }
                Some(_) => {}
                None => drivers.steps.push((schedule.to_string(), step)),
            }
        }
    }

    fn layer_kind(&self, target: Target) -> Option<Kind> {
        match target {
            Target::Layer(i) => self.decls.get(usize::from(i)).and_then(|d| d.kind),
            _ => None,
        }
    }

    fn channels(
        &mut self,
        target: Target,
        map: &Map<String, Value>,
        loc: &Loc,
        kind: &ClipKind,
        length: u16,
    ) -> Channels {
        let mut channels = Channels::default();
        let length = i64::from(length);
        let wrong = match target {
            Target::Badge => IssueCode::BadgeChannel,
            Target::Plate => IssueCode::PlateChannel,
            _ => IssueCode::Animation,
        };
        for (channel, value) in map {
            let here = loc.key(channel);
            let allowed = match (channel.as_str(), target) {
                ("position", _) => true,
                ("opacity" | "visible", Target::Plate) => false,
                ("opacity" | "visible", _) => true,
                ("frame", Target::Layer(_)) => self.layer_kind(target) == Some(Kind::Sprite),
                ("speed", Target::Layer(_)) => true,
                ("burst", Target::Layer(_)) => self.layer_kind(target) == Some(Kind::Emitter),
                _ => false,
            };
            if !allowed {
                self.push(here.error(wrong).name("channel", channel));
                continue;
            }
            match channel.as_str() {
                "position" => {
                    if let Some((x, y)) = self.position_track(value, &here, length) {
                        channels.x = Some(x);
                        channels.y = Some(y);
                    }
                }
                "opacity" => {
                    channels.opacity = self.scalar_track(value, &here, length, Scalar::Opacity)
                }
                "visible" => {
                    channels.visible = self.scalar_track(value, &here, length, Scalar::Visible)
                }
                "frame" => {
                    let frames = match target {
                        Target::Layer(i) => self.decls[usize::from(i)]
                            .sprite
                            .and_then(|s| self.sprites.get(usize::from(s)))
                            .and_then(Option::as_ref)
                            .map_or(0, |s| s.frames.len()),
                        _ => 0,
                    };
                    channels.frame = self.scalar_track(value, &here, length, Scalar::Frame(frames));
                }
                "speed" => channels.speed = self.speed_track(target, value, &here, kind, length),
                _ => channels.bursts = self.bursts(value, &here, length),
            }
        }
        if target == Target::Badge {
            let molang = [
                &channels.x,
                &channels.y,
                &channels.opacity,
                &channels.visible,
            ]
            .into_iter()
            .flatten()
            .any(|t| {
                t.keys
                    .iter()
                    .any(|k| matches!(k.value, KeyValue::Molang(_)))
            });
            if molang {
                self.push(loc.error(IssueCode::BadgeMolang));
            }
        }
        channels
    }

    fn speed_track(
        &mut self,
        target: Target,
        value: &Value,
        loc: &Loc,
        kind: &ClipKind,
        length: i64,
    ) -> Option<Track> {
        if !kind.triggered() {
            self.push(loc.error(IssueCode::SpeedTrigger));
            return None;
        }
        let Target::Layer(i) = target else {
            self.push(loc.error(IssueCode::SpeedPath));
            return None;
        };
        let decl = &self.decls[usize::from(i)];
        if decl.kind != Some(Kind::Sprite) || !decl.path {
            self.push(loc.error(IssueCode::SpeedPath));
            return None;
        }
        if decl.laps {
            self.push(loc.error(IssueCode::SpeedPath));
            return None;
        }
        let limit = f64::from(NAMEPLATE_PATH_SPEED);
        self.speed_keys(value, loc, length, (-limit, limit))
    }

    fn bursts(&mut self, value: &Value, loc: &Loc, length: i64) -> Vec<Burst> {
        let Some(items) = value.as_array().filter(|a| !a.is_empty()) else {
            self.push(loc.error(IssueCode::Animation));
            return Vec::new();
        };
        let mut bursts = Vec::new();
        let mut last = -1;
        for (k, item) in items.iter().enumerate() {
            let at = loc.index(k);
            let Some([frame, count]) = read::pair_i64(item) else {
                self.push(at.error(IssueCode::Animation));
                continue;
            };
            if frame <= last {
                self.push(at.error(IssueCode::KeyOrder).value("frame", frame as f64));
            } else if frame < 0 || frame >= length {
                self.push(
                    at.error(IssueCode::KeyRange)
                        .value("frame", frame as f64)
                        .value("length", length as f64),
                );
            } else if !(1..=i64::from(NAMEPLATE_BURST)).contains(&count) {
                self.push(
                    at.error(IssueCode::BurstCount)
                        .value("count", count as f64)
                        .value("limit", f64::from(NAMEPLATE_BURST)),
                );
            } else {
                bursts.push(Burst {
                    frame: frame as u16,
                    count: count as u8,
                });
            }
            last = frame;
        }
        bursts
    }

    fn key_entries<'v>(
        &mut self,
        value: &'v Value,
        loc: &Loc,
        length: i64,
        step_only: bool,
    ) -> Option<Vec<(u16, &'v Value, Ease, Loc)>> {
        let Some(items) = value.as_array() else {
            self.push(loc.error(IssueCode::Animation));
            return None;
        };
        if items.is_empty() {
            self.push(loc.error(IssueCode::Animation));
            return None;
        }
        let mut entries = Vec::new();
        let mut last = -1;
        let mut ok = true;
        for (k, item) in items.iter().enumerate() {
            let at = loc.index(k);
            let key = item.as_array().filter(|a| (2..=3).contains(&a.len()));
            let Some(key) = key else {
                self.push(at.error(IssueCode::Animation));
                ok = false;
                continue;
            };
            let Some(frame) = read::whole(&key[0]) else {
                self.push(at.error(IssueCode::Animation));
                ok = false;
                continue;
            };
            let ease = match key.get(2) {
                None => Some(Ease::Step),
                Some(e) => e
                    .as_str()
                    .and_then(ease)
                    .filter(|e| !step_only || *e == Ease::Step),
            };
            let Some(ease) = ease else {
                self.push(at.error(IssueCode::Animation));
                ok = false;
                continue;
            };
            if !self.key_frame(&at, k, frame, last, length) {
                ok = false;
            }
            last = frame;
            entries.push((
                frame.clamp(0, i64::from(u16::MAX)) as u16,
                &key[1],
                ease,
                at,
            ));
        }
        ok.then_some(entries)
    }

    fn scalar_track(
        &mut self,
        value: &Value,
        loc: &Loc,
        length: i64,
        kind: Scalar,
    ) -> Option<Track> {
        let step_only = !matches!(kind, Scalar::Opacity);
        let entries = self.key_entries(value, loc, length, step_only)?;
        let mut keys = Vec::new();
        let mut ok = true;
        for (frame, value, ease, at) in entries {
            let parsed = match (value, kind) {
                (Value::String(source), _) => self
                    .molang(source, MolangUse::Channel, &at)
                    .map(KeyValue::Molang),
                (v, Scalar::Frame(frames)) => read::whole(v)
                    .filter(|f| *f >= 0 && (*f as usize) < frames)
                    .map(|f| KeyValue::Fixed(f as i32)),
                (v, Scalar::Opacity) => read::number(v)
                    .filter(|o| (0.0..=1.0).contains(o))
                    .map(|o| KeyValue::Fixed(read::fixed_point(o, 255.0))),
                (Value::Bool(b), Scalar::Visible) => Some(KeyValue::Fixed(i32::from(*b))),
                _ => None,
            };
            match parsed {
                Some(value) => keys.push(Key { frame, value, ease }),
                None => {
                    if !value.is_string() {
                        self.push(at.error(IssueCode::Animation));
                    }
                    ok = false;
                }
            }
        }
        ok.then_some(Track { keys })
    }

    fn position_track(&mut self, value: &Value, loc: &Loc, length: i64) -> Option<(Track, Track)> {
        let entries = self.key_entries(value, loc, length, false)?;
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        let mut ok = true;
        let limit = i64::from(NAMEPLATE_OFFSET);
        for (frame, value, ease, at) in entries {
            let Some(pair) = value.as_array().filter(|a| a.len() == 2) else {
                self.push(at.error(IssueCode::Animation));
                ok = false;
                continue;
            };
            let mut parts = [None, None];
            for (axis, component) in pair.iter().enumerate() {
                parts[axis] = match component {
                    Value::String(source) => self
                        .molang(source, MolangUse::Channel, &at)
                        .map(KeyValue::Molang),
                    other => match read::whole(other) {
                        Some(n) if (-limit..=limit).contains(&n) => Some(KeyValue::Fixed(n as i32)),
                        _ => {
                            self.push(at.error(IssueCode::Animation).value("limit", limit as f64));
                            None
                        }
                    },
                };
            }
            match parts {
                [Some(x), Some(y)] => {
                    xs.push(Key {
                        frame,
                        value: x,
                        ease,
                    });
                    ys.push(Key {
                        frame,
                        value: y,
                        ease,
                    });
                }
                _ => ok = false,
            }
        }
        ok.then_some((Track { keys: xs }, Track { keys: ys }))
    }

    fn letters(
        &mut self,
        channels: &Map<String, Value>,
        loc: &Loc,
        kind: &ClipKind,
    ) -> Option<LetterOverride> {
        let mut letters = None;
        for (channel, value) in channels {
            let here = loc.key(channel);
            if channel != "motion" {
                self.push(
                    here.error(IssueCode::LetterProgram)
                        .name("channel", channel),
                );
                continue;
            }
            if !kind.triggered() {
                self.push(loc.error(IssueCode::LetterProgram));
                return None;
            }
            letters = match value {
                Value::Null => Some(LetterOverride::Off),
                Value::String(s) if s.trim().is_empty() => Some(LetterOverride::Off),
                Value::String(source) => {
                    if !self.letters_allowed {
                        self.soft(here.error(IssueCode::LetterProgram));
                        return None;
                    }
                    self.molang(source, MolangUse::Text, &here)
                        .map(LetterOverride::Program)
                }
                _ => {
                    self.push(here.error(IssueCode::LetterProgram));
                    None
                }
            };
        }
        letters
    }

    fn palette_track(
        &mut self,
        slot_name: &str,
        map: &Map<String, Value>,
        loc: &Loc,
        kind: &ClipKind,
        length: u16,
    ) -> Option<SlotTrack> {
        let code = IssueCode::PaletteAnimation;
        let Some(slot) = self.slot_index(slot_name) else {
            self.push(loc.error(code).name("slot", slot_name));
            return None;
        };
        for key in map.keys().filter(|k| *k != "cycle" && *k != "keys") {
            self.push(loc.key(key).error(code).name("channel", key));
        }
        let tones = self.tones[usize::from(slot)];
        let track = match (map.get("cycle"), map.get("keys")) {
            (Some(_), Some(_)) | (None, None) => {
                self.push(loc.error(code));
                return None;
            }
            (Some(cycle), None) => {
                let at = loc.key("cycle");
                let cycle = self.object(cycle, &at, code)?;
                self.keys(&at, cycle, &CYCLE_KEYS);
                if !cycle.contains_key("every") || !cycle.contains_key("mode") {
                    self.push(at.error(code));
                    return None;
                }
                let every = self.int(
                    cycle,
                    "every",
                    &at,
                    code,
                    (1, i64::from(NAMEPLATE_LOOP_FRAMES)),
                    1,
                ) as u16;
                let mode = self.choice(
                    cycle,
                    "mode",
                    &at,
                    code,
                    |name| match name {
                        "forward" => Some(CycleMode::Forward),
                        "backward" => Some(CycleMode::Backward),
                        "ping_pong" => Some(CycleMode::PingPong),
                        _ => None,
                    },
                    CycleMode::Forward,
                );
                let period = cycle_period(mode, tones, every);
                if matches!(kind, ClipKind::Loop | ClipKind::While)
                    && u64::from(length) % period != 0
                {
                    self.push(
                        at.error(IssueCode::PaletteCycle)
                            .value("cycle", period as f64)
                            .value("length", f64::from(length)),
                    );
                }
                if !self.banded.contains(&slot) {
                    self.banded.push(slot);
                }
                PaletteTrack::Cycle { every, mode }
            }
            (None, Some(keys)) => PaletteTrack::Keys(self.palette_keys(
                keys,
                &loc.key("keys"),
                tones,
                i64::from(length),
            )?),
        };
        Some(SlotTrack { slot, track })
    }

    fn palette_keys(
        &mut self,
        value: &Value,
        loc: &Loc,
        tones: u8,
        length: i64,
    ) -> Option<Vec<PaletteKey>> {
        let entries = self.key_entries(value, loc, length, false)?;
        let mut keys = Vec::new();
        let mut ok = true;
        for (frame, value, ease, at) in entries {
            let texts: Vec<&Value> = match value {
                Value::String(s) if s == "base" => vec![value; usize::from(tones)],
                Value::String(_) if tones == 1 => vec![value],
                Value::Array(items) if items.len() == usize::from(tones) => items.iter().collect(),
                _ => {
                    self.push(
                        at.error(IssueCode::PaletteAnimation)
                            .value("tones", f64::from(tones)),
                    );
                    ok = false;
                    continue;
                }
            };
            let mut values = Vec::new();
            for text in texts {
                let Some(text) = text.as_str() else {
                    self.push(at.error(IssueCode::PaletteAnimation));
                    ok = false;
                    continue;
                };
                let parsed = if text == "base" {
                    Some(PaletteValue::Base)
                } else if text.starts_with('#') {
                    match read::hex(text) {
                        Some(argb) => Some(PaletteValue::Color(argb)),
                        None => {
                            self.push(at.error(IssueCode::PaletteAnimation).name("color", text));
                            None
                        }
                    }
                } else if read::slot_ref(text).is_some() {
                    self.slot_color(text, &at, IssueCode::PaletteAnimation)
                        .map(PaletteValue::Ref)
                } else {
                    self.molang(text, MolangUse::SlotColor, &at)
                        .map(PaletteValue::Molang)
                };
                match parsed {
                    Some(v) => values.push(v),
                    None => ok = false,
                }
            }
            keys.push(PaletteKey {
                frame,
                ease,
                tones: values,
            });
        }
        ok.then_some(keys)
    }
}

pub(super) fn cycle_period(mode: CycleMode, tones: u8, every: u16) -> u64 {
    if tones <= 1 {
        return 1;
    }
    let rotations = match mode {
        CycleMode::PingPong => 2 * (u64::from(tones) - 1),
        _ => u64::from(tones),
    };
    rotations * u64::from(every)
}
