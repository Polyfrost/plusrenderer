use std::collections::HashMap;
use std::slice::ChunksExactMut;

use crate::animation::flag_words;
use crate::cost::Detail;
use crate::limits::{BONES_PER_PET, EFFECT_RING_POINTS, WEARER_RECORD_BYTES};
use crate::pose::POSE_CHANNELS;
use crate::signals::{SignalState, SIGNAL_COUNT};

pub const WEARER_RECORD_BUDGET_BYTES: usize = WEARER_RECORD_BYTES as usize;
pub const WEARER_IDLE_SECONDS: f64 = 30.0;
pub const MAX_VARIABLE_SLOTS: usize = 64;
pub const MAX_ONE_SHOTS: usize = 8;
pub const SPRING_BONE_FLOATS: usize = 6;
pub const ONE_SHOT_EMPTY: u32 = u32::MAX;
pub const TIME_NEVER: f32 = f32::NEG_INFINITY;
pub const REBASE_SECONDS: f32 = 4096.0;
pub const RECORD_HEADER_BYTES: usize = std::mem::size_of::<Header>();
pub const SPRING_HEADER_FLOATS: usize = 13;
pub const SPRING_ROOT: usize = 0;
pub const SPRING_CLOCK: usize = 12;
pub const RING_POINTS: usize = EFFECT_RING_POINTS as usize;
pub const RING_HEAD: usize = 0;
pub const RING_TIME: usize = 1;
pub const RING_MARK_FLOATS: usize = 5;
pub const RING_MARK_AT: usize = 1;
pub const RING_MARK_TURN: usize = 4;
pub const RING_SEEN: usize = 2;
pub const RING_BREAKS: usize = RING_SEEN + RING_MARK_FLOATS;
pub const RING_BREAK_COUNT: usize = 2;
pub const RING_SEGMENTS: usize = RING_BREAKS + RING_BREAK_COUNT * RING_MARK_FLOATS;
pub const RING_SEGMENT_BITS: usize = 24;
pub const RING_CLOCK_FLOATS: usize = RING_SEGMENTS + 3;
pub const RING_CLOCK_TIMES: [usize; 2 + RING_BREAK_COUNT] = [
    RING_TIME,
    RING_SEEN,
    RING_BREAKS,
    RING_BREAKS + RING_MARK_FLOATS,
];
pub const RING_POINT_FLOATS: usize = 3;

const _: () = assert!(RING_CLOCK_FLOATS - RING_SEGMENTS >= RING_POINTS.div_ceil(RING_SEGMENT_BITS));

pub fn reset_ring_clock(clock: &mut [f32]) {
    if clock.len() < RING_CLOCK_FLOATS {
        return;
    }
    clock[..RING_CLOCK_FLOATS].fill(0.0);
    for at in RING_CLOCK_TIMES {
        clock[at] = TIME_NEVER;
    }
}
pub const MAX_HELD_CHANNELS: usize = BONES_PER_PET as usize * POSE_CHANNELS;
pub const HELD_CHANNEL_BYTES: usize = 12;
pub const MAX_SEED_WEARERS: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WearerDeclarations {
    pub variables: usize,
    pub controllers: usize,
    pub one_shots: usize,
    pub spring_bones: usize,
    pub effect_times: usize,
    pub effect_floats: usize,
    pub channels: usize,
    pub rings: usize,
    pub ring_points: usize,
    pub ring_turns: usize,
    pub ring_gates: usize,
    pub held_channels: usize,
}

impl WearerDeclarations {
    pub fn spring_header(&self) -> usize {
        if self.spring_bones > 0 {
            SPRING_HEADER_FLOATS
        } else {
            0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationError {
    TooManyVariables { declared: usize, limit: usize },
    TooManyOneShots { declared: usize, limit: usize },
    OverBudget { bytes: usize, budget: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordLayout {
    declarations: WearerDeclarations,
    floats: usize,
    ints: usize,
    bytes: usize,
    held: usize,
}

impl RecordLayout {
    pub fn new(declarations: WearerDeclarations) -> Result<Self, DeclarationError> {
        let d = declarations;
        if d.variables > MAX_VARIABLE_SLOTS {
            return Err(DeclarationError::TooManyVariables {
                declared: d.variables,
                limit: MAX_VARIABLE_SLOTS,
            });
        }
        if d.one_shots > MAX_ONE_SHOTS {
            return Err(DeclarationError::TooManyOneShots {
                declared: d.one_shots,
                limit: MAX_ONE_SHOTS,
            });
        }
        let over = |bytes| DeclarationError::OverBudget {
            bytes,
            budget: WEARER_RECORD_BUDGET_BYTES,
        };
        let floats = d
            .spring_bones
            .checked_mul(SPRING_BONE_FLOATS)
            .and_then(|n| n.checked_add(d.variables))
            .and_then(|n| n.checked_add(d.controllers))
            .and_then(|n| n.checked_add(d.one_shots))
            .and_then(|n| n.checked_add(d.effect_times))
            .and_then(|n| n.checked_add(d.effect_floats))
            .and_then(|n| n.checked_add(d.spring_header()))
            .and_then(|n| n.checked_add(d.rings.checked_mul(RING_CLOCK_FLOATS)?))
            .and_then(|n| n.checked_add(d.ring_points.checked_mul(RING_POINT_FLOATS)?))
            .and_then(|n| n.checked_add(d.ring_turns))
            .and_then(|n| n.checked_add(d.ring_gates))
            .ok_or(over(usize::MAX))?;
        let ints = d
            .controllers
            .checked_add(d.one_shots)
            .and_then(|n| n.checked_add(flag_words(d.channels)))
            .ok_or(over(usize::MAX))?;
        let held = d
            .held_channels
            .checked_mul(HELD_CHANNEL_BYTES)
            .ok_or(over(usize::MAX))?;
        let bytes = floats
            .checked_add(ints)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(RECORD_HEADER_BYTES))
            .and_then(|n| n.checked_add(held))
            .ok_or(over(usize::MAX))?;
        if bytes > WEARER_RECORD_BUDGET_BYTES {
            return Err(over(bytes));
        }
        Ok(Self {
            declarations,
            floats,
            ints,
            bytes,
            held,
        })
    }

    fn unheld_bytes(&self) -> usize {
        self.bytes - self.held
    }

    pub fn declarations(&self) -> WearerDeclarations {
        self.declarations
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CosmeticSlot {
    index: u32,
    generation: u32,
}

pub struct WearerRecord<'a> {
    pub wearer: u64,
    pub appear: bool,
    pub times_reset: bool,
    pub now: f32,
    pub epoch: f64,
    pub born: f64,
    pub shifted: f32,
    pub signals: &'a mut SignalState,
    pub variables: &'a mut [f32],
    pub controller_states: &'a mut [u32],
    pub controller_entered: &'a mut [f32],
    pub one_shot_ids: &'a mut [u32],
    pub one_shot_starts: &'a mut [f32],
    pub spring_bones: &'a mut [f32],
    pub spring_header: &'a mut [f32],
    pub effect_times: &'a mut [f32],
    pub effect_floats: &'a mut [f32],
    pub rings: &'a mut [f32],
    pub detail: &'a mut Detail,
    pub held: &'a mut Option<i64>,
    pub sampled: &'a mut f32,
    pub disabled: &'a mut [u32],
    pub held_pose: &'a mut [[f32; 3]],
    ring_count: usize,
}

impl WearerRecord<'_> {
    fn reset_times(&mut self) {
        *self.held = None;
        *self.sampled = TIME_NEVER;
        self.signals.reset_times(0.0);
        self.controller_entered.fill(0.0);
        self.one_shot_ids.fill(ONE_SHOT_EMPTY);
        self.one_shot_starts.fill(0.0);
        self.effect_times.fill(TIME_NEVER);
        if let Some(clock) = self.spring_header.get_mut(SPRING_CLOCK) {
            *clock = TIME_NEVER;
        }
        for clock in self.ring_clocks() {
            reset_ring_clock(clock);
        }
    }

    fn ring_clocks(&mut self) -> ChunksExactMut<'_, f32> {
        let clocks = (self.ring_count * RING_CLOCK_FLOATS).min(self.rings.len());
        self.rings[..clocks].chunks_exact_mut(RING_CLOCK_FLOATS)
    }

    fn shift(&mut self, delta: f32) {
        self.signals.shift(delta);
        let clocks = (self.ring_count * RING_CLOCK_FLOATS).min(self.rings.len());
        for time in self
            .controller_entered
            .iter_mut()
            .chain(self.one_shot_starts.iter_mut())
            .chain(self.effect_times.iter_mut())
            .chain(self.spring_header.get_mut(SPRING_CLOCK))
        {
            *time -= delta;
        }
        for clock in self.rings[..clocks].chunks_exact_mut(RING_CLOCK_FLOATS) {
            for at in RING_CLOCK_TIMES {
                clock[at] -= delta;
            }
        }
        *self.sampled -= delta;
    }
}

struct Header {
    wearer: u64,
    live: bool,
    rewound: bool,
    epoch: f64,
    born: f64,
    last_drawn: f64,
    sampled: f32,
    signals: SignalState,
    detail: Detail,
    held: Option<i64>,
}

struct Pool {
    layout: RecordLayout,
    headers: Vec<Header>,
    floats: Vec<f32>,
    ints: Vec<u32>,
    channels: usize,
    poses: Vec<[f32; 3]>,
    free: Vec<u32>,
    index: HashMap<u64, u32>,
}

impl Pool {
    fn new(layout: RecordLayout) -> Self {
        Self {
            layout,
            headers: Vec::new(),
            floats: Vec::new(),
            ints: Vec::new(),
            channels: 0,
            poses: Vec::new(),
            free: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn set_channels(&mut self, channels: usize) {
        let held = channels.saturating_mul(HELD_CHANNEL_BYTES);
        let fits = self.layout.unheld_bytes().saturating_add(held) <= WEARER_RECORD_BUDGET_BYTES;
        self.channels = if channels <= MAX_HELD_CHANNELS && fits {
            channels
        } else {
            0
        };
        self.poses
            .resize(self.headers.len() * self.channels, [0.0; 3]);
        for header in &mut self.headers {
            header.held = None;
        }
    }

    fn live(&self) -> usize {
        self.index.len()
    }

    fn record_bytes(&self) -> usize {
        self.layout.unheld_bytes() + self.channels * HELD_CHANNEL_BYTES
    }

    fn view(&mut self, slot: usize, clock: f64) -> WearerRecord<'_> {
        let d = self.layout.declarations;
        let (f, n) = (self.layout.floats, self.layout.ints);
        let header = &mut self.headers[slot];
        let floats = &mut self.floats[slot * f..(slot + 1) * f];
        let ints = &mut self.ints[slot * n..(slot + 1) * n];
        let held_pose = &mut self.poses[slot * self.channels..(slot + 1) * self.channels];
        let (variables, rest) = floats.split_at_mut(d.variables);
        let (controller_entered, rest) = rest.split_at_mut(d.controllers);
        let (one_shot_starts, rest) = rest.split_at_mut(d.one_shots);
        let (spring_bones, rest) = rest.split_at_mut(d.spring_bones * SPRING_BONE_FLOATS);
        let (spring_header, rest) = rest.split_at_mut(d.spring_header());
        let (effect_times, rest) = rest.split_at_mut(d.effect_times);
        let (effect_floats, rings) = rest.split_at_mut(d.effect_floats);
        let (controller_states, rest) = ints.split_at_mut(d.controllers);
        let (one_shot_ids, disabled) = rest.split_at_mut(d.one_shots);
        WearerRecord {
            wearer: header.wearer,
            appear: false,
            times_reset: false,
            now: (clock - header.epoch) as f32,
            epoch: header.epoch,
            born: header.born,
            shifted: 0.0,
            signals: &mut header.signals,
            variables,
            controller_states,
            controller_entered,
            one_shot_ids,
            one_shot_starts,
            spring_bones,
            spring_header,
            effect_times,
            effect_floats,
            rings,
            detail: &mut header.detail,
            held: &mut header.held,
            sampled: &mut header.sampled,
            disabled,
            held_pose,
            ring_count: d.rings,
        }
    }

    fn create(&mut self, wearer: u64, clock: f64) -> usize {
        let header = Header {
            wearer,
            live: true,
            rewound: false,
            epoch: clock,
            born: clock,
            last_drawn: clock,
            sampled: TIME_NEVER,
            signals: SignalState::new(0.0),
            detail: Detail::Full,
            held: None,
        };
        let slot = match self.free.pop() {
            Some(slot) => {
                self.headers[slot as usize] = header;
                slot as usize
            }
            None => {
                self.headers.push(header);
                let count = self.headers.len();
                self.floats.resize(count * self.layout.floats, 0.0);
                self.ints.resize(count * self.layout.ints, 0);
                self.poses.resize(count * self.channels, [0.0; 3]);
                count - 1
            }
        };
        self.index.insert(wearer, slot as u32);
        let mut rec = self.view(slot, clock);
        rec.variables.fill(0.0);
        rec.controller_states.fill(0);
        rec.spring_bones.fill(0.0);
        rec.spring_header.fill(f32::NAN);
        rec.effect_floats.fill(0.0);
        rec.rings.fill(0.0);
        rec.disabled.fill(0);
        rec.reset_times();
        slot
    }

    fn drop_slot(&mut self, slot: usize) {
        let header = &mut self.headers[slot];
        if header.live {
            header.live = false;
            self.index.remove(&header.wearer);
            self.free.push(slot as u32);
        }
    }

    fn forget(&mut self, wearer: u64) -> bool {
        match self.index.get(&wearer) {
            Some(&slot) => {
                self.drop_slot(slot as usize);
                true
            }
            None => false,
        }
    }
}

pub struct WearerTable {
    pools: Vec<Option<Pool>>,
    generations: Vec<u32>,
    overrides: Vec<(u64, [f32; SIGNAL_COUNT])>,
    seeds: Vec<(u64, u64)>,
    clock: f64,
    started: bool,
}

impl Default for WearerTable {
    fn default() -> Self {
        Self::new()
    }
}

impl WearerTable {
    pub fn new() -> Self {
        Self {
            pools: Vec::new(),
            generations: Vec::new(),
            overrides: Vec::new(),
            seeds: Vec::new(),
            clock: 0.0,
            started: false,
        }
    }

    fn pool(&self, slot: CosmeticSlot) -> Option<&Pool> {
        let index = slot.index as usize;
        if *self.generations.get(index)? != slot.generation {
            return None;
        }
        self.pools[index].as_ref()
    }

    fn pool_mut(&mut self, slot: CosmeticSlot) -> Option<&mut Pool> {
        let index = slot.index as usize;
        if *self.generations.get(index)? != slot.generation {
            return None;
        }
        self.pools[index].as_mut()
    }

    pub fn add_cosmetic(
        &mut self,
        declarations: WearerDeclarations,
    ) -> Result<CosmeticSlot, DeclarationError> {
        let pool = Pool::new(RecordLayout::new(declarations)?);
        let index = match self.pools.iter().position(Option::is_none) {
            Some(index) => {
                self.pools[index] = Some(pool);
                index
            }
            None => {
                self.pools.push(Some(pool));
                self.generations.push(0);
                self.pools.len() - 1
            }
        };
        Ok(CosmeticSlot {
            index: index as u32,
            generation: self.generations[index],
        })
    }

    pub fn remove_cosmetic(&mut self, slot: CosmeticSlot) -> bool {
        if self.pool(slot).is_none() {
            return false;
        }
        let index = slot.index as usize;
        self.pools[index] = None;
        self.generations[index] = self.generations[index].wrapping_add(1);
        true
    }

    pub fn layout(&self, slot: CosmeticSlot) -> Option<&RecordLayout> {
        self.pool(slot).map(|pool| &pool.layout)
    }

    pub fn set_held_channels(&mut self, slot: CosmeticSlot, channels: usize) -> bool {
        match self.pool_mut(slot) {
            Some(pool) => {
                pool.set_channels(channels);
                true
            }
            None => false,
        }
    }

    pub fn detail(&self, wearer: u64) -> Option<Detail> {
        self.pools.iter().flatten().find_map(|pool| {
            let &record = pool.index.get(&wearer)?;
            Some(pool.headers[record as usize].detail)
        })
    }

    pub fn clock(&self) -> f64 {
        self.clock
    }

    pub fn begin_frame(&mut self, now: f64) -> bool {
        if !now.is_finite() {
            return false;
        }
        let rewound = self.started && now < self.clock;
        if rewound {
            for pool in self.pools.iter_mut().flatten() {
                for slot in 0..pool.headers.len() {
                    let header = &mut pool.headers[slot];
                    if !header.live {
                        continue;
                    }
                    header.epoch = now;
                    header.born = now;
                    header.last_drawn = now;
                    header.rewound = true;
                    pool.view(slot, now).reset_times();
                }
            }
        }
        self.clock = now;
        self.started = true;
        self.sweep(now);
        rewound
    }

    pub fn sweep(&mut self, now: f64) -> usize {
        let mut dropped = 0;
        let mut unseeded = Vec::new();
        for pool in self.pools.iter_mut().flatten() {
            for slot in 0..pool.headers.len() {
                let header = &pool.headers[slot];
                if header.live && now - header.last_drawn >= WEARER_IDLE_SECONDS {
                    let wearer = header.wearer;
                    if self.seeds.iter().any(|(id, _)| *id == wearer) {
                        unseeded.push(wearer);
                    }
                    pool.drop_slot(slot);
                    dropped += 1;
                }
            }
        }
        for wearer in unseeded {
            if !self
                .pools
                .iter()
                .flatten()
                .any(|p| p.index.contains_key(&wearer))
            {
                self.seeds.retain(|(id, _)| *id != wearer);
            }
        }
        dropped
    }

    pub fn draw(
        &mut self,
        slot: CosmeticSlot,
        wearer: u64,
        raw: &[f32; SIGNAL_COUNT],
        events: u32,
    ) -> Option<WearerRecord<'_>> {
        let clock = self.clock;
        let index = slot.index as usize;
        if *self.generations.get(index)? != slot.generation {
            return None;
        }
        let pool = self.pools[index].as_mut()?;
        let overrides = self
            .overrides
            .iter()
            .find(|(id, _)| *id == wearer)
            .map(|(_, values)| values);
        let (record, appear) = match pool.index.get(&wearer) {
            Some(&record) => (record as usize, false),
            None => (pool.create(wearer, clock), true),
        };
        let header = &mut pool.headers[record];
        let times_reset = appear || header.rewound;
        header.rewound = false;
        header.last_drawn = clock;
        let mut shifts = 0.0f32;
        while ((clock - header.epoch) as f32) >= REBASE_SECONDS {
            header.epoch += REBASE_SECONDS as f64;
            shifts += 1.0;
        }
        let mut rec = pool.view(record, clock);
        if shifts > 0.0 {
            rec.shifted = shifts * REBASE_SECONDS;
            rec.shift(rec.shifted);
        }
        rec.signals.step(raw, overrides, events, rec.now);
        rec.appear = appear;
        rec.times_reset = times_reset;
        Some(rec)
    }

    pub fn record(&mut self, slot: CosmeticSlot, wearer: u64) -> Option<WearerRecord<'_>> {
        let clock = self.clock;
        let pool = self.pool_mut(slot)?;
        let record = *pool.index.get(&wearer)? as usize;
        Some(pool.view(record, clock))
    }

    pub fn each_record(&mut self, slot: CosmeticSlot, mut f: impl FnMut(WearerRecord<'_>)) {
        let clock = self.clock;
        let Some(pool) = self.pool_mut(slot) else {
            return;
        };
        for record in 0..pool.headers.len() {
            if pool.headers[record].live {
                f(pool.view(record, clock));
            }
        }
    }

    pub fn forget(&mut self, slot: CosmeticSlot, wearer: u64) -> bool {
        self.pool_mut(slot).is_some_and(|pool| pool.forget(wearer))
    }

    pub fn forget_wearer(&mut self, wearer: u64) -> bool {
        let mut forgot = false;
        for pool in self.pools.iter_mut().flatten() {
            forgot |= pool.forget(wearer);
        }
        self.overrides.retain(|(id, _)| *id != wearer);
        self.seeds.retain(|(id, _)| *id != wearer);
        forgot
    }

    pub fn set_seed_wearer(&mut self, wearer: u64, seed: u64) {
        self.seeds.retain(|(id, _)| *id != wearer);
        if seed == wearer {
            return;
        }
        if self.seeds.len() >= MAX_SEED_WEARERS {
            self.seeds.remove(0);
        }
        self.seeds.push((wearer, seed));
    }

    pub fn seed_wearer(&self, wearer: u64) -> u64 {
        self.seeds
            .iter()
            .find(|(id, _)| *id == wearer)
            .map_or(wearer, |&(_, seed)| seed)
    }

    pub fn set_signal_override(&mut self, wearer: u64, signal: usize, value: f32) -> bool {
        if signal >= SIGNAL_COUNT {
            return false;
        }
        let entry = self.overrides.iter().position(|(id, _)| *id == wearer);
        match (entry, value.is_nan()) {
            (Some(entry), true) => {
                let values = &mut self.overrides[entry].1;
                values[signal] = f32::NAN;
                if values.iter().all(|v| v.is_nan()) {
                    self.overrides.swap_remove(entry);
                }
            }
            (Some(entry), false) => self.overrides[entry].1[signal] = value,
            (None, true) => {}
            (None, false) => {
                let mut values = [f32::NAN; SIGNAL_COUNT];
                values[signal] = value;
                self.overrides.push((wearer, values));
            }
        }
        true
    }

    pub fn wearer_count(&self) -> usize {
        self.pools.iter().flatten().map(Pool::live).sum()
    }

    pub fn wearer_bytes(&self) -> usize {
        self.pools
            .iter()
            .flatten()
            .map(|pool| pool.live() * pool.record_bytes())
            .sum()
    }
}

const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn seed(wearer_id: u64, cosmetic_id: u64, purpose: u64) -> u64 {
    let h = mix64(wearer_id.wrapping_add(GOLDEN_GAMMA));
    let h = mix64(h ^ cosmetic_id.wrapping_add(GOLDEN_GAMMA.wrapping_mul(2)));
    mix64(h ^ purpose.wrapping_add(GOLDEN_GAMMA.wrapping_mul(3)))
}

pub fn seed_key(name: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    mix64(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::Detail;
    use crate::signals::{signal_id, EVENT_HURT, IS_IN_COMBAT};

    const NONE: [f32; SIGNAL_COUNT] = [0.0; SIGNAL_COUNT];

    fn sneaking() -> [f32; SIGNAL_COUNT] {
        let mut raw = NONE;
        raw[signal_id("is_sneaking").unwrap()] = 1.0;
        raw
    }

    fn decl() -> WearerDeclarations {
        WearerDeclarations {
            variables: 4,
            controllers: 2,
            one_shots: 3,
            spring_bones: 2,
            effect_times: 2,
            effect_floats: 5,
            channels: 40,
            ..WearerDeclarations::default()
        }
    }

    fn ringed() -> WearerDeclarations {
        WearerDeclarations { spring_bones: 1, effect_times: 1, rings: 2, ring_points: 74, ..WearerDeclarations::default() }
    }

    fn both() -> WearerDeclarations {
        WearerDeclarations { rings: 2, ring_points: 74, ..decl() }
    }

    fn table_with(declarations: WearerDeclarations) -> (WearerTable, CosmeticSlot) {
        let mut table = WearerTable::new();
        let slot = table.add_cosmetic(declarations).unwrap();
        (table, slot)
    }

    fn draw(table: &mut WearerTable, slot: CosmeticSlot, wearer: u64) -> bool {
        table.draw(slot, wearer, &NONE, 0).unwrap().appear
    }

    #[test]
    fn layouts_count_every_part_and_hold_the_budget() {
        let bytes = |d: WearerDeclarations| RecordLayout::new(d).map(|l| l.bytes());
        assert_eq!(bytes(WearerDeclarations::default()), Ok(RECORD_HEADER_BYTES));
        assert_eq!(RECORD_HEADER_BYTES, std::mem::size_of::<Header>());
        let parts = 4 * 4 + 2 * 8 + 3 * 8 + 2 * 6 * 4 + SPRING_HEADER_FLOATS * 4 + 2 * 4 + 5 * 4 + 2 * 4;
        assert_eq!(bytes(decl()), Ok(RECORD_HEADER_BYTES + parts));
        assert_eq!(bytes(ringed()), Ok(RECORD_HEADER_BYTES + 4 * (6 + SPRING_HEADER_FLOATS + 1 + 2 * RING_CLOCK_FLOATS + 74 * 3)));
        let channels = |channels| bytes(WearerDeclarations { channels, ..WearerDeclarations::default() });
        assert_eq!((channels(32), channels(33)), (Ok(RECORD_HEADER_BYTES + 4), Ok(RECORD_HEADER_BYTES + 8)));
        let held = |held_channels| bytes(WearerDeclarations { held_channels, ..decl() });
        assert_eq!(held(3), Ok(RECORD_HEADER_BYTES + parts + 3 * HELD_CHANNEL_BYTES));

        assert_eq!(
            RecordLayout::new(WearerDeclarations { variables: 65, ..WearerDeclarations::default() }),
            Err(DeclarationError::TooManyVariables { declared: 65, limit: 64 })
        );
        assert_eq!(
            RecordLayout::new(WearerDeclarations { one_shots: 9, ..WearerDeclarations::default() }),
            Err(DeclarationError::TooManyOneShots { declared: 9, limit: 8 })
        );
        let springs = WearerDeclarations { spring_bones: 100, ..WearerDeclarations::default() };
        assert_eq!(
            RecordLayout::new(springs),
            Err(DeclarationError::OverBudget {
                bytes: RECORD_HEADER_BYTES + 100 * 24 + SPRING_HEADER_FLOATS * 4,
                budget: WEARER_RECORD_BUDGET_BYTES
            })
        );
        let room = (WEARER_RECORD_BUDGET_BYTES - RECORD_HEADER_BYTES) / 4;
        assert_eq!(bytes(WearerDeclarations { effect_floats: room, ..WearerDeclarations::default() }), Ok(WEARER_RECORD_BUDGET_BYTES));
        for over in [
            WearerDeclarations { effect_floats: room + 1, ..WearerDeclarations::default() },
            WearerDeclarations { effect_floats: usize::MAX, ..WearerDeclarations::default() },
            WearerDeclarations { channels: usize::MAX, ..WearerDeclarations::default() },
            WearerDeclarations { rings: usize::MAX / 2, ..WearerDeclarations::default() },
        ] {
            assert!(RecordLayout::new(over).is_err(), "{over:?}");
        }
        let mut table = WearerTable::new();
        assert!(table.add_cosmetic(springs).is_err());
    }

    #[test]
    fn records_start_fresh_persist_and_stay_per_wearer() {
        let (mut table, slot) = table_with(decl());
        table.begin_frame(5.0);
        let rec = table.draw(slot, 42, &NONE, 0).unwrap();
        assert!(rec.appear && rec.times_reset);
        assert_eq!((rec.wearer, rec.now), (42, 0.0));
        assert_eq!(rec.controller_states, &[0; 2]);
        assert_eq!(rec.one_shot_ids, &[ONE_SHOT_EMPTY; 3]);
        assert_eq!((rec.spring_bones.len(), rec.effect_times[0], rec.disabled.len()), (12, TIME_NEVER, 2));
        assert_eq!((*rec.detail, *rec.held), (Detail::Full, None));
        rec.variables[2] = 9.0;
        table.draw(slot, 43, &NONE, EVENT_HURT);
        table.begin_frame(5.5);
        let rec = table.draw(slot, 42, &sneaking(), 0).unwrap();
        assert!(!rec.appear && !rec.times_reset);
        assert_eq!((rec.now, rec.variables[2], rec.signals.value(IS_IN_COMBAT)), (0.5, 9.0, 0.0));
        assert_eq!(table.draw(slot, 43, &NONE, 0).unwrap().variables[2], 0.0);

        let (mut big, slot) = table_with(decl());
        big.begin_frame(1_700_000_000.0);
        big.draw(slot, 1, &sneaking(), 0);
        big.begin_frame(1_700_000_000.25);
        let rec = big.draw(slot, 1, &sneaking(), 0).unwrap();
        assert_eq!(rec.signals.time(signal_id("is_sneaking").unwrap(), rec.now), 0.25);

        let (mut ringed, slot) = table_with(ringed());
        ringed.begin_frame(3.0);
        let rec = ringed.draw(slot, 1, &NONE, 0).unwrap();
        assert!(rec.spring_header[SPRING_ROOT..SPRING_CLOCK].iter().all(|v| v.is_nan()));
        assert_eq!((rec.spring_header[SPRING_CLOCK], rec.rings.len()), (TIME_NEVER, 2 * RING_CLOCK_FLOATS + 74 * 3));
        for clock in rec.rings[..2 * RING_CLOCK_FLOATS].chunks_exact(RING_CLOCK_FLOATS) {
            assert_eq!([clock[RING_HEAD], clock[RING_TIME], clock[RING_SEEN], clock[RING_BREAKS]], [0.0, TIME_NEVER, TIME_NEVER, TIME_NEVER]);
        }
    }

    #[test]
    fn records_drop_when_idle_forgotten_or_removed() {
        let mut table = WearerTable::new();
        let a = table.add_cosmetic(decl()).unwrap();
        let b = table.add_cosmetic(WearerDeclarations::default()).unwrap();
        let a_bytes = table.layout(a).unwrap().bytes();
        table.begin_frame(0.0);
        table.draw(a, 9, &NONE, EVENT_HURT).unwrap().one_shot_ids[0] = 2;
        draw(&mut table, a, 1);
        draw(&mut table, b, 1);
        assert_eq!((table.wearer_count(), table.wearer_bytes()), (3, 2 * a_bytes + RECORD_HEADER_BYTES));
        table.begin_frame(20.0);
        draw(&mut table, a, 1);
        table.begin_frame(29.5);
        assert_eq!(table.wearer_count(), 3);
        table.begin_frame(30.5);
        assert_eq!(table.wearer_count(), 1);
        let rec = table.draw(a, 9, &NONE, 0).unwrap();
        assert!(rec.appear);
        assert_eq!((rec.one_shot_ids[0], rec.signals.value(IS_IN_COMBAT)), (ONE_SHOT_EMPTY, 0.0));
        draw(&mut table, b, 1);
        assert!(table.forget_wearer(1) && !table.forget_wearer(1));
        assert!(table.record(b, 1).is_none() && table.record(a, 9).is_some());
        assert!(table.forget(a, 9) && table.record(a, 9).is_none());

        draw(&mut table, a, 2);
        assert!(table.remove_cosmetic(a) && !table.remove_cosmetic(a));
        assert_eq!((table.wearer_count(), table.wearer_bytes()), (0, 0));
        assert!(table.draw(a, 2, &NONE, 0).is_none());
        let c = table.add_cosmetic(decl()).unwrap();
        assert_ne!(a, c);
        assert!(table.draw(a, 2, &NONE, 0).is_none() && draw(&mut table, c, 2));
    }

    #[test]
    fn long_sessions_rebase_every_stored_time_and_report_the_shift() {
        let (mut table, slot) = table_with(both());
        let sneak = signal_id("is_sneaking").unwrap();
        table.begin_frame(0.0);
        table.draw(slot, 1, &NONE, 0);
        table.begin_frame(1.0);
        {
            let rec = table.draw(slot, 1, &sneaking(), 0).unwrap();
            rec.controller_entered[0] = rec.now;
            rec.one_shot_starts[0] = rec.now;
            rec.effect_times[0] = rec.now;
            rec.spring_header[SPRING_CLOCK] = rec.now;
            rec.rings[RING_TIME] = rec.now;
            rec.rings[RING_BREAKS] = rec.now - 1.0;
        }
        let mut shifts = Vec::new();
        let mut t = 1.0;
        while t + 20.0 < 9000.0 {
            t += 20.0;
            table.begin_frame(t);
            let rec = table.draw(slot, 1, &sneaking(), 0).unwrap();
            assert_eq!(rec.epoch + f64::from(rec.now), t);
            if rec.shifted != 0.0 {
                shifts.push((t, rec.shifted));
            }
        }
        assert_eq!(shifts.iter().map(|s| s.1).collect::<Vec<_>>(), [REBASE_SECONDS; 2]);
        let rec = table.draw(slot, 1, &sneaking(), 0).unwrap();
        let age = (t - 1.0) as f32;
        assert!(rec.now < REBASE_SECONDS);
        assert_eq!(rec.signals.time(sneak, rec.now), age);
        for stored in [rec.controller_entered[0], rec.one_shot_starts[0], rec.effect_times[0], rec.spring_header[SPRING_CLOCK], rec.rings[RING_TIME]] {
            assert_eq!(rec.now - stored, age);
        }
        assert_eq!(rec.now - rec.rings[RING_BREAKS], age + 1.0);
        assert_eq!(rec.rings[RING_CLOCK_FLOATS + RING_TIME], TIME_NEVER);
    }

    #[test]
    fn a_clock_rewind_resets_times_without_a_second_appear() {
        let (mut table, slot) = table_with(both());
        let sneak = signal_id("is_sneaking").unwrap();
        table.begin_frame(100.0);
        assert!(draw(&mut table, slot, 1));
        table.begin_frame(110.0);
        {
            let rec = table.draw(slot, 1, &sneaking(), EVENT_HURT).unwrap();
            rec.variables[1] = 4.0;
            rec.controller_states[0] = 3;
            rec.controller_entered[0] = rec.now;
            rec.one_shot_ids[0] = 1;
            rec.effect_times[0] = rec.now;
            rec.spring_bones[0] = 0.5;
            rec.spring_header[SPRING_CLOCK] = rec.now;
            rec.rings[RING_HEAD] = 5.0;
            rec.rings[2 * RING_CLOCK_FLOATS] = 2.0;
            *rec.detail = Detail::Minimal;
            *rec.held = Some(17);
        }
        assert!(!table.begin_frame(116.0));
        assert!(table.begin_frame(50.0));
        let rec = table.draw(slot, 1, &sneaking(), 0).unwrap();
        assert!(!rec.appear && rec.times_reset);
        assert_eq!((rec.now, rec.signals.time(sneak, 0.0), rec.signals.value(sneak)), (0.0, 0.0, 1.0));
        assert_eq!((rec.signals.value(IS_IN_COMBAT), rec.variables[1], rec.controller_states[0]), (0.0, 4.0, 3));
        assert_eq!((rec.controller_entered[0], rec.one_shot_ids[0], rec.effect_times[0]), (0.0, ONE_SHOT_EMPTY, TIME_NEVER));
        assert_eq!((rec.spring_bones[0], rec.spring_header[SPRING_CLOCK]), (0.5, TIME_NEVER));
        assert_eq!((rec.rings[RING_HEAD], rec.rings[2 * RING_CLOCK_FLOATS]), (0.0, 2.0));
        assert_eq!((*rec.detail, *rec.held), (Detail::Minimal, None));
        table.begin_frame(52.0);
        let rec = table.draw(slot, 1, &sneaking(), 0).unwrap();
        assert_eq!((rec.times_reset, rec.signals.time(sneak, rec.now)), (false, 2.0));
    }

    #[test]
    fn wearer_seed_is_pinned_and_order_sensitive() {
        assert_eq!(seed(0, 0, 0), 0x1e35_cdb4_bf4f_6f16);
        assert_eq!(seed(1, 2, 3), 0xcead_1f2f_a8f8_a720);
        assert_eq!(seed(u64::MAX, 0x0123_4567_89ab_cdef, 7), 0x7e88_231b_a7cd_f7d2);
        assert_eq!(seed_key("polyplus:halo"), 0xca8a_5148_1151_f8ea);
        assert_ne!(seed(1, 2, 3), seed(2, 1, 3));
        assert_ne!(seed(1, 2, 3), seed(1, 3, 2));
    }
}
