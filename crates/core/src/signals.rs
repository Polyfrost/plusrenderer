#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalKind {
    Bool,
    Float,
}

pub const SIGNALS: [(&str, SignalKind); 23] = [
    ("is_sneaking", SignalKind::Bool),
    ("is_sprinting", SignalKind::Bool),
    ("is_swimming", SignalKind::Bool),
    ("is_in_water", SignalKind::Bool),
    ("is_in_rain", SignalKind::Bool),
    ("is_in_snow", SignalKind::Bool),
    ("is_on_fire", SignalKind::Bool),
    ("is_hurt", SignalKind::Bool),
    ("is_gliding", SignalKind::Bool),
    ("is_flying", SignalKind::Bool),
    ("is_falling", SignalKind::Bool),
    ("is_on_ground", SignalKind::Bool),
    ("is_riding", SignalKind::Bool),
    ("is_sleeping", SignalKind::Bool),
    ("is_emoting", SignalKind::Bool),
    ("is_in_combat", SignalKind::Bool),
    ("is_moving", SignalKind::Bool),
    ("ground_speed", SignalKind::Float),
    ("vertical_speed", SignalKind::Float),
    ("time_of_day", SignalKind::Float),
    ("biome_temperature", SignalKind::Float),
    ("is_in_ocean", SignalKind::Bool),
    ("dimension", SignalKind::Float),
];

pub const SIGNAL_COUNT: usize = SIGNALS.len();
pub const BOOL_SIGNAL_COUNT: usize = count_bools();

const BOOL_SLOTS: [u8; SIGNAL_COUNT] = bool_slots();

const fn count_bools() -> usize {
    let (mut n, mut i) = (0, 0);
    while i < SIGNAL_COUNT {
        if matches!(SIGNALS[i].1, SignalKind::Bool) {
            n += 1;
        }
        i += 1;
    }
    n
}

const fn bool_slots() -> [u8; SIGNAL_COUNT] {
    let mut out = [u8::MAX; SIGNAL_COUNT];
    let (mut n, mut i) = (0, 0);
    while i < SIGNAL_COUNT {
        if matches!(SIGNALS[i].1, SignalKind::Bool) {
            out[i] = n;
            n += 1;
        }
        i += 1;
    }
    out
}

fn bool_slot(id: usize) -> Option<usize> {
    BOOL_SLOTS
        .get(id)
        .filter(|&&slot| slot != u8::MAX)
        .map(|&slot| usize::from(slot))
}

pub const EVENT_HURT: u32 = 1 << 0;
pub const EVENT_ATTACK_SWING: u32 = 1 << 1;
pub const EVENT_DEATH: u32 = 1 << 2;
pub const EVENT_RESPAWN: u32 = 1 << 3;
pub const EVENT_DIMENSION_CHANGE: u32 = 1 << 4;
pub const EVENT_SLEEP: u32 = 1 << 5;
pub const EVENT_JUMP: u32 = 1 << 6;
pub const EVENT_EQUIP: u32 = 1 << 7;

pub fn signal_id(name: &str) -> Option<usize> {
    SIGNALS.iter().position(|(n, _)| *n == name)
}

pub const IS_HURT: usize = 7;
pub const IS_IN_COMBAT: usize = 15;

pub const EVENT_COMBAT_ENTER: u32 = EVENT_HURT | EVENT_ATTACK_SWING;
pub const EVENT_COMBAT_EXIT: u32 =
    EVENT_DEATH | EVENT_RESPAWN | EVENT_DIMENSION_CHANGE | EVENT_SLEEP;

pub const COMBAT_REACH_BLOCKS: f32 = 4.5;
pub const COMBAT_CONE_DEGREES: f32 = 60.0;
pub const COMBAT_SECONDS: f32 = 10.0;

pub const TIME_SUFFIX: &str = "_time";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnownQuery {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
}

pub const KNOWN_QUERIES: [KnownQuery; 0] = [];

pub const JUMP_SIGNAL: usize = SIGNAL_COUNT;
pub const JUMP_QUERY: &str = "is_jumping";
pub const JUMP_GRACE_SECONDS: f32 = 0.1;

const IS_IN_WATER: usize = 3;
const IS_GLIDING: usize = 8;
const IS_FLYING: usize = 9;
const IS_ON_GROUND: usize = 11;
const IS_RIDING: usize = 12;

pub fn jump_query(name: &str) -> Option<bool> {
    match name.strip_prefix(JUMP_QUERY)? {
        "" => Some(false),
        TIME_SUFFIX => Some(true),
        _ => None,
    }
}

pub fn signal_time_id(name: &str) -> Option<usize> {
    let id = signal_id(name.strip_suffix(TIME_SUFFIX)?)?;
    is_bool_signal(id).then_some(id)
}

pub fn is_bool_signal(id: usize) -> bool {
    matches!(SIGNALS.get(id), Some((_, SignalKind::Bool)))
}

fn normalise(id: usize, value: f32) -> f32 {
    if is_bool_signal(id) {
        if value >= 0.5 {
            1.0
        } else {
            0.0
        }
    } else if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalState {
    pub values: [f32; SIGNAL_COUNT],
    pub changed: [f32; BOOL_SIGNAL_COUNT],
    pub combat_entered: f32,
    pub jumped: f32,
    pub jump_changed: f32,
}

impl SignalState {
    pub fn new(now: f32) -> Self {
        Self {
            values: [0.0; SIGNAL_COUNT],
            changed: [now; BOOL_SIGNAL_COUNT],
            combat_entered: f32::NAN,
            jumped: f32::NAN,
            jump_changed: now,
        }
    }

    pub fn step(
        &mut self,
        raw: &[f32; SIGNAL_COUNT],
        overrides: Option<&[f32; SIGNAL_COUNT]>,
        events: u32,
        now: f32,
    ) {
        let override_of = |id: usize| overrides.map_or(f32::NAN, |o| o[id]);
        let mut next = [0.0; SIGNAL_COUNT];
        for (id, slot) in next.iter_mut().enumerate() {
            let forced = override_of(id);
            *slot = normalise(id, if forced.is_nan() { raw[id] } else { forced });
        }

        let hurt_on = next[IS_HURT] == 1.0 && self.values[IS_HURT] != 1.0;
        if events & EVENT_COMBAT_ENTER != 0 || hurt_on {
            self.combat_entered = now;
        }
        if events & EVENT_COMBAT_EXIT != 0 {
            self.combat_entered = f32::NAN;
        }
        let mut combat_changed = now;
        if !self.combat_entered.is_nan() && now - self.combat_entered >= COMBAT_SECONDS {
            combat_changed = (self.combat_entered + COMBAT_SECONDS).min(now);
            self.combat_entered = f32::NAN;
        }
        let forced_combat = override_of(IS_IN_COMBAT);
        next[IS_IN_COMBAT] = if forced_combat.is_nan() {
            if self.combat_entered.is_nan() {
                0.0
            } else {
                1.0
            }
        } else {
            combat_changed = now;
            normalise(IS_IN_COMBAT, forced_combat)
        };

        for (id, (value, old)) in next.iter().zip(self.values).enumerate() {
            let Some(slot) = bool_slot(id) else {
                continue;
            };
            if *value != old {
                self.changed[slot] = if id == IS_IN_COMBAT {
                    combat_changed
                } else {
                    now
                };
            }
        }
        self.values = next;
        self.step_jump(events, now);
    }

    fn step_jump(&mut self, events: u32, now: f32) {
        let on = |id: usize| self.values[id] == 1.0;
        let blocked = [IS_GLIDING, IS_FLYING, IS_RIDING, IS_IN_WATER]
            .into_iter()
            .any(on);
        let was = !self.jumped.is_nan();
        if events & EVENT_JUMP != 0 && !blocked {
            self.jumped = now;
        } else if was {
            let landed = on(IS_ON_GROUND) && now - self.jumped >= JUMP_GRACE_SECONDS;
            if landed || blocked || events & EVENT_COMBAT_EXIT != 0 {
                self.jumped = f32::NAN;
            }
        }
        if was != !self.jumped.is_nan() {
            self.jump_changed = now;
        }
    }

    pub fn value(&self, id: usize) -> f32 {
        if id == JUMP_SIGNAL {
            return if self.jumped.is_nan() { 0.0 } else { 1.0 };
        }
        self.values.get(id).copied().unwrap_or(0.0)
    }

    pub fn time(&self, id: usize, now: f32) -> f32 {
        self.changed_at(id).map_or(0.0, |changed| now - changed)
    }

    pub fn changed_at(&self, id: usize) -> Option<f32> {
        if id == JUMP_SIGNAL {
            Some(self.jump_changed)
        } else {
            bool_slot(id).map(|slot| self.changed[slot])
        }
    }

    pub fn gates_at(&self, at: f32) -> u32 {
        let mut bits = 0;
        for id in 0..=JUMP_SIGNAL {
            let Some(changed) = self.changed_at(id) else {
                continue;
            };
            let on = self.value(id) >= 0.5;
            if on != (changed > at) {
                bits |= 1 << id;
            }
        }
        bits
    }

    pub fn reset_times(&mut self, now: f32) {
        self.changed = [now; BOOL_SIGNAL_COUNT];
        self.combat_entered = f32::NAN;
        self.values[IS_IN_COMBAT] = 0.0;
        self.jumped = f32::NAN;
        self.jump_changed = now;
    }

    pub fn shift(&mut self, delta: f32) {
        for changed in &mut self.changed {
            *changed -= delta;
        }
        self.combat_entered -= delta;
        self.jumped -= delta;
        self.jump_changed -= delta;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_with(pairs: &[(&str, f32)]) -> [f32; SIGNAL_COUNT] {
        let mut raw = [0.0; SIGNAL_COUNT];
        for (name, value) in pairs {
            raw[signal_id(name).unwrap()] = *value;
        }
        raw
    }

    fn combat(state: &SignalState) -> bool {
        state.value(IS_IN_COMBAT) == 1.0
    }

    #[test]
    fn host_ids_and_event_bits_are_fixed() {
        assert_eq!((signal_id("is_hurt"), signal_id("is_in_combat"), SIGNAL_COUNT), (Some(IS_HURT), Some(IS_IN_COMBAT), 23));
        let bits = [EVENT_HURT, EVENT_ATTACK_SWING, EVENT_DEATH, EVENT_RESPAWN, EVENT_DIMENSION_CHANGE, EVENT_SLEEP, EVENT_JUMP, EVENT_EQUIP];
        assert_eq!(bits, [1, 2, 4, 8, 16, 32, 64, 128]);
        assert_eq!((signal_time_id("is_in_ocean_time"), signal_time_id("ground_speed_time")), (signal_id("is_in_ocean"), None));
        assert_eq!((jump_query("is_jumping"), jump_query("is_jumping_time"), signal_id(JUMP_QUERY)), (Some(false), Some(true), None));
    }

    #[test]
    fn values_normalise_and_edges_set_time_since_change() {
        let sneak = signal_id("is_sneaking").unwrap();
        let speed = signal_id("ground_speed").unwrap();
        let mut s = SignalState::new(0.0);
        s.step(&raw_with(&[("is_sprinting", f32::NAN), ("ground_speed", 3.25), ("vertical_speed", f32::INFINITY)]), None, 0, 0.0);
        assert_eq!([s.value(signal_id("is_sprinting").unwrap()), s.value(speed), s.value(signal_id("vertical_speed").unwrap())], [0.0, 3.25, 0.0]);
        s.step(&raw_with(&[("is_sneaking", 0.7)]), None, 0, 3.0);
        assert_eq!((s.value(sneak), s.time(sneak, 3.0)), (1.0, 0.0));
        s.step(&raw_with(&[("is_sneaking", 1.0)]), None, 0, 4.5);
        assert_eq!(s.time(sneak, 4.5), 1.5);
        s.step(&raw_with(&[]), None, 0, 5.0);
        assert_eq!(s.time(sneak, 6.25), 1.25);

        let mut over = [f32::NAN; SIGNAL_COUNT];
        over[sneak] = 1.0;
        over[speed] = 4.0;
        s.step(&raw_with(&[("ground_speed", 1.0)]), Some(&over), 0, 7.0);
        assert_eq!((s.value(sneak), s.value(speed), s.time(sneak, 7.0)), (1.0, 4.0, 0.0));
        s.step(&raw_with(&[("ground_speed", 1.0)]), None, 0, 8.0);
        assert_eq!((s.value(sneak), s.value(speed)), (0.0, 1.0));
    }

    #[test]
    fn combat_enters_on_events_and_leaves_ten_seconds_after_the_last() {
        let none = raw_with(&[]);
        let mut s = SignalState::new(0.0);
        s.step(&raw_with(&[("is_in_combat", 1.0)]), None, 0, 0.0);
        assert!(!combat(&s));
        s.step(&none, None, EVENT_HURT, 1.0);
        s.step(&none, None, EVENT_ATTACK_SWING, 8.0);
        s.step(&none, None, 0, 17.0);
        assert!(combat(&s));
        s.step(&none, None, 0, 18.5);
        assert!(!combat(&s));
        assert_eq!(s.time(IS_IN_COMBAT, 18.5), 0.5);

        let hurt = raw_with(&[("is_hurt", 1.0)]);
        s.step(&hurt, None, 0, 20.0);
        s.step(&hurt, None, 0, 31.0);
        assert!(!combat(&s));
        for exit in [EVENT_DEATH, EVENT_RESPAWN, EVENT_DIMENSION_CHANGE, EVENT_SLEEP] {
            s.step(&none, None, EVENT_ATTACK_SWING, 40.0);
            s.step(&none, None, exit, 41.0);
            assert!(!combat(&s), "event {exit}");
        }
        s.step(&none, None, EVENT_HURT | EVENT_DEATH, 42.0);
        assert!(!combat(&s));

        let mut forced = [f32::NAN; SIGNAL_COUNT];
        forced[IS_IN_COMBAT] = 0.0;
        s.step(&none, Some(&forced), EVENT_HURT, 43.0);
        assert!(!combat(&s));
        forced[IS_IN_COMBAT] = f32::NAN;
        forced[IS_HURT] = 1.0;
        s.step(&none, Some(&forced), 0, 44.0);
        assert!(combat(&s));
    }

    #[test]
    fn a_jump_holds_until_the_player_lands() {
        let ground = raw_with(&[("is_on_ground", 1.0)]);
        let air = raw_with(&[]);
        let mut s = SignalState::new(0.0);
        s.step(&ground, None, EVENT_JUMP, 2.0);
        s.step(&ground, None, 0, 2.05);
        assert_eq!(s.value(JUMP_SIGNAL), 1.0);
        s.step(&air, None, 0, 2.4);
        assert_eq!(s.value(JUMP_SIGNAL), 1.0);
        s.step(&ground, None, 0, 2.7);
        assert_eq!(s.value(JUMP_SIGNAL), 0.0);
        assert!((s.time(JUMP_SIGNAL, 3.0) - 0.3).abs() < 1e-6);
        for (stops, events) in [(raw_with(&[("is_gliding", 1.0)]), 0), (air, EVENT_DEATH)] {
            s.step(&ground, None, EVENT_JUMP, 4.0);
            s.step(&stops, None, events, 4.3);
            assert_eq!(s.value(JUMP_SIGNAL), 0.0);
        }
        s.step(&raw_with(&[("is_in_water", 1.0)]), None, EVENT_JUMP, 6.0);
        assert_eq!(s.value(JUMP_SIGNAL), 0.0);

        let mut plain = SignalState::new(0.0);
        let mut equipped = SignalState::new(0.0);
        for (now, events) in [(0.0, 0), (1.0, EVENT_EQUIP), (2.0, 0)] {
            plain.step(&ground, None, 0, now);
            equipped.step(&ground, None, events, now);
            assert_eq!(format!("{plain:?}"), format!("{equipped:?}"));
        }
    }

    #[test]
    fn shift_and_reset_move_every_stored_time() {
        let sneak = signal_id("is_sneaking").unwrap();
        let held = raw_with(&[("is_sneaking", 1.0)]);
        let mut s = SignalState::new(0.0);
        s.step(&held, None, EVENT_HURT | EVENT_JUMP, 4100.0);
        s.shift(4096.0);
        assert_eq!((s.time(sneak, 5.0), s.time(JUMP_SIGNAL, 5.0)), (1.0, 1.0));
        s.step(&held, None, 0, 13.0);
        assert!(combat(&s));
        s.step(&held, None, 0, 14.5);
        assert_eq!((combat(&s), s.time(IS_IN_COMBAT, 14.5)), (false, 0.5));

        s.step(&held, None, EVENT_HURT, 15.0);
        s.reset_times(15.0);
        assert_eq!((s.value(sneak), s.value(IS_IN_COMBAT), s.value(JUMP_SIGNAL)), (1.0, 0.0, 0.0));
        assert!(s.combat_entered.is_nan());
        assert_eq!(s.time(sneak, 15.5), 0.5);
    }
}
