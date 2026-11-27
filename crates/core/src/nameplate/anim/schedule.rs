use crate::nameplate::types::{Schedule, CHANCE_ONE};
use crate::wearer::seed as hash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScheduledEvent {
    pub event: u16,
    pub start: i64,
}

const PICK_BITS: u32 = 20;
const PICK_MASK: u64 = (1 << PICK_BITS) - 1;

fn scaled(bits: u64, range: u64) -> u64 {
    ((bits & PICK_MASK) * range) >> PICK_BITS
}

pub fn roll(schedule: &Schedule, seed: u64, window: i64) -> Option<ScheduledEvent> {
    if schedule.window == 0 {
        return None;
    }
    let total: u64 = schedule.events.iter().map(|e| u64::from(e.weight)).sum();
    if total == 0 {
        return None;
    }
    let h = hash(seed, schedule.salt, window as u64);
    let chance = u64::from(schedule.chance.min(CHANCE_ONE));
    if h & u64::from(CHANCE_ONE - 1) >= chance {
        return None;
    }
    let mut pick = scaled(h >> 24, total);
    let mut event = 0;
    for (index, e) in schedule.events.iter().enumerate() {
        let weight = u64::from(e.weight);
        if pick < weight {
            event = index;
            break;
        }
        pick -= weight;
    }
    let offset = scaled(h >> 44, u64::from(schedule.jitter) + 1) as i64;
    Some(ScheduledEvent {
        event: event as u16,
        start: window * i64::from(schedule.window) + offset,
    })
}

pub fn event_at(schedule: &Schedule, seed: u64, frame: i64) -> Option<ScheduledEvent> {
    if schedule.window == 0 {
        return None;
    }
    let window = frame.div_euclid(i64::from(schedule.window));
    let event = roll(schedule, seed, window)?;
    let length = i64::from(schedule.events[usize::from(event.event)].clip.length);
    (frame >= event.start && frame - event.start < length).then_some(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::types::{Clip, ScheduledClip};
    use crate::wearer::{seed, seed_key};

    #[test]
    fn schedules_roll_seeded_windows_and_play_whole_events_inside_them() {
        let events = [(5, 2), (2, 1)].map(|(length, weight)| ScheduledClip { clip: Clip { length, ..Clip::default() }, weight });
        let s = Schedule { id: "blink".into(), salt: seed_key("blink"), window: 8, jitter: 3, chance: CHANCE_ONE * 3 / 5, events: events.to_vec() };
        let played: Vec<_> = (0..250).map(|k| roll(&s, 1, k).is_some()).collect();
        let expected: Vec<_> = (0..250).map(|k| (seed(1, s.salt, k as u64) & 0xff_ffff) < u64::from(s.chance)).collect();
        assert_eq!(played, expected, "the same bits on every client");
        let shown = |seed| (0..2_000).map(|f| event_at(&s, seed, f)).collect::<Vec<_>>();
        assert_ne!(shown(1), shown(42));
        for seed in 0..40u64 {
            for k in -4..60i64 {
                let shown: Vec<_> = (0..8).map(|i| event_at(&s, seed, k * 8 + i)).collect();
                let rolled = roll(&s, seed, k);
                for (i, at) in shown.iter().enumerate() {
                    let f = k * 8 + i as i64;
                    let playing = rolled.filter(|e| f >= e.start && f < e.start + i64::from(s.events[usize::from(e.event)].clip.length));
                    assert_eq!(*at, playing, "{seed} {k} {i}");
                }
                assert!(rolled.is_none_or(|e| (0..=3).contains(&(e.start - k * 8))));
            }
        }
        let always = Schedule { chance: CHANCE_ONE, ..s.clone() };
        assert!((0..500).all(|k| roll(&always, 5, k).is_some()));
        assert_eq!(event_at(&Schedule { events: Vec::new(), ..always }, 1, 3), None);
    }
}
