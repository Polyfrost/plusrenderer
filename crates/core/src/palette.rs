use crate::nameplate::color::{from_oklch, to_oklch};

pub const MAX_SLOTS: usize = 32;
pub const MAX_TONES: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub id: String,
    pub name: String,
    pub ramp: Vec<u32>,
    pub stops: Vec<u32>,
    pub interpolate: Option<u8>,
    pub customizable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotOverride {
    pub slot: u8,
    pub ramp: Vec<u32>,
    pub stops: Vec<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Colorway {
    pub id: String,
    pub overrides: Vec<SlotOverride>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Palette {
    pub slots: Vec<Slot>,
    pub colorways: Vec<Colorway>,
}

impl Palette {
    pub fn slot(&self, id: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.id == id)
    }

    pub fn colorway(&self, id: &str) -> Option<usize> {
        self.colorways.iter().position(|c| c.id == id)
    }

    pub fn ramp(&self, slot: usize, colorway: Option<usize>) -> Option<&[u32]> {
        self.slot_override(slot, colorway)
            .map(|o| o.ramp.as_slice())
            .or_else(|| self.slots.get(slot).map(|s| s.ramp.as_slice()))
    }

    pub fn stops(&self, slot: usize, colorway: Option<usize>) -> Option<&[u32]> {
        self.slot_override(slot, colorway)
            .map(|o| o.stops.as_slice())
            .or_else(|| self.slots.get(slot).map(|s| s.stops.as_slice()))
    }

    pub fn tones(&self, slot: usize, colorway: Option<usize>) -> Vec<u32> {
        match self.shape(slot, colorway) {
            Shape::Interpolated(stops, count) => interpolate(stops, count),
            Shape::Listed(tones) => tones.to_vec(),
        }
    }

    pub fn tone_count(&self, slot: usize, colorway: Option<usize>) -> usize {
        match self.shape(slot, colorway) {
            Shape::Interpolated(_, count) => count,
            Shape::Listed(tones) => tones.len(),
        }
    }

    fn shape(&self, slot: usize, colorway: Option<usize>) -> Shape<'_> {
        let Some(authored) = self.slots.get(slot) else {
            return Shape::Listed(&[]);
        };
        let ramp = self.ramp(slot, colorway).unwrap_or(&[]);
        let stops = self
            .stops(slot, colorway)
            .filter(|s| !s.is_empty())
            .unwrap_or(ramp);
        if let Some(count) = authored.interpolate.map(usize::from) {
            if stops.len() >= 2 && (1..=MAX_TONES).contains(&count) {
                return Shape::Interpolated(stops, count);
            }
        }
        Shape::Listed(if ramp.is_empty() { stops } else { ramp })
    }

    pub fn blend_stops(&self, slot: usize, colorway: Option<usize>) -> Vec<u32> {
        match (self.stops(slot, colorway), self.ramp(slot, colorway)) {
            (Some(stops), _) if !stops.is_empty() => stops.to_vec(),
            (_, Some(ramp)) if !ramp.is_empty() => ramp.to_vec(),
            _ => self.tones(slot, colorway),
        }
    }

    pub fn resolve(&self, colorway: Option<usize>) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.slots.len() * MAX_TONES);
        for slot in 0..self.slots.len() {
            let tones = self.tones(slot, colorway);
            for tone in 0..MAX_TONES {
                let last = tones.len().saturating_sub(1);
                out.push(tones.get(tone.min(last)).copied().unwrap_or(0));
            }
        }
        out
    }

    pub fn resolve_stops(&self, colorway: Option<usize>) -> Vec<Vec<u32>> {
        (0..self.slots.len())
            .map(|slot| self.blend_stops(slot, colorway))
            .collect()
    }

    fn slot_override(&self, slot: usize, colorway: Option<usize>) -> Option<&SlotOverride> {
        self.colorways
            .get(colorway?)?
            .overrides
            .iter()
            .find(|o| usize::from(o.slot) == slot)
    }
}

enum Shape<'a> {
    Interpolated(&'a [u32], usize),
    Listed(&'a [u32]),
}

pub fn tone_index(slot: usize, tone: usize) -> usize {
    slot * MAX_TONES + tone.min(MAX_TONES - 1)
}

pub fn lerp(from: u32, to: u32, num: i64, den: i64) -> u32 {
    if den == 0 {
        return from;
    }
    let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
    let mut out = 0;
    for shift in [24, 16, 8, 0] {
        let a = i64::from((from >> shift) & 0xff);
        let b = i64::from((to >> shift) & 0xff);
        let mixed = a + ((b - a) * num).div_euclid(den);
        out |= (mixed.clamp(0, 255) as u32) << shift;
    }
    out
}

pub fn along(stops: &[u32], position: u32, positions: u32) -> u32 {
    let Some(first) = stops.first() else {
        return 0;
    };
    if stops.len() == 1 || positions <= 1 {
        return *first;
    }
    let segments = (stops.len() - 1) as i64;
    let span = i64::from(positions) - 1;
    let scaled = i64::from(position.min(positions - 1)) * segments;
    let segment = (scaled / span).min(segments - 1);
    let index = segment as usize;
    lerp(
        stops[index],
        stops[index + 1],
        scaled - segment * span,
        span,
    )
}

pub fn srgb_lerp(from: u32, to: u32, t: f64) -> u32 {
    let t = if t.is_finite() { t } else { 0.0 };
    let mut out = 0;
    for shift in [24, 16, 8, 0] {
        let a = f64::from((from >> shift) & 0xff);
        let b = f64::from((to >> shift) & 0xff);
        let mixed = a + libm::floor(t * (b - a));
        out |= (mixed.clamp(0.0, 255.0) as u32) << shift;
    }
    out
}

pub fn interpolate(stops: &[u32], count: usize) -> Vec<u32> {
    if stops.is_empty() {
        return Vec::new();
    }
    let positions = u32::try_from(count).unwrap_or(u32::MAX);
    (0..positions).map(|i| along(stops, i, positions)).collect()
}

pub fn base_tone(tones: usize) -> usize {
    tones.saturating_sub(1) / 2
}

const ACHROMATIC: f64 = 1.0e-3;

pub fn derive_ramp(authored: &[u32], base: u32) -> Vec<u32> {
    let anchor_index = base_tone(authored.len());
    let Some(anchor) = authored.get(anchor_index) else {
        return Vec::new();
    };
    let from = to_oklch(*anchor);
    let to = to_oklch(base);
    authored
        .iter()
        .enumerate()
        .map(|(index, tone)| {
            if index == anchor_index {
                return (tone & 0xff00_0000) | (base & 0x00ff_ffff);
            }
            shift(*tone, from, to)
        })
        .collect()
}

fn shift(tone: u32, from: [f64; 3], to: [f64; 3]) -> u32 {
    let [base_l, base_c, base_h] = from;
    let [new_l, new_c, new_h] = to;
    let alpha = (tone >> 24) as u8;
    let [l, c, h] = to_oklch(tone);
    let lightness = (new_l + l - base_l).clamp(0.0, 1.0);
    if new_c < ACHROMATIC {
        return from_oklch([lightness, 0.0, 0.0], alpha);
    }
    let chroma = (new_c + c - base_c).max(0.0);
    let turn = if c < ACHROMATIC || base_c < ACHROMATIC {
        0.0
    } else {
        h - base_h
    };
    from_oklch([lightness, chroma, new_h + turn], alpha)
}

pub fn derive_stops(stops: &[u32], tones: &[u32], base: u32) -> Vec<u32> {
    let Some(anchor) = tones.get(base_tone(tones.len())).copied() else {
        return stops.to_vec();
    };
    let from = to_oklch(anchor);
    let to = to_oklch(base);
    stops
        .iter()
        .map(|stop| {
            if (stop ^ anchor) & 0x00ff_ffff == 0 {
                return (stop & 0xff00_0000) | (base & 0x00ff_ffff);
            }
            shift(*stop, from, to)
        })
        .collect()
}

pub fn custom_stops(p: &Palette, colorway: Option<usize>, custom: &[(u8, u32)]) -> Vec<Vec<u32>> {
    let mut out = p.resolve_stops(colorway);
    for (slot, authored) in p.slots.iter().enumerate() {
        if !authored.customizable {
            continue;
        }
        let Some((_, rgb)) = custom.iter().rev().find(|(s, _)| usize::from(*s) == slot) else {
            continue;
        };
        out[slot] = derive_stops(&out[slot], &p.tones(slot, colorway), rgb & 0x00ff_ffff);
    }
    out
}

pub fn custom_tones(p: &Palette, colorway: Option<usize>, custom: &[(u8, u32)]) -> Vec<u32> {
    let mut out = p.resolve(colorway);
    for (slot, authored) in p.slots.iter().enumerate() {
        if !authored.customizable {
            continue;
        }
        let Some((_, rgb)) = custom.iter().rev().find(|(s, _)| usize::from(*s) == slot) else {
            continue;
        };
        let derived = derive_ramp(&p.tones(slot, colorway), rgb & 0x00ff_ffff);
        let Some(last) = derived.last().copied() else {
            continue;
        };
        for tone in 0..MAX_TONES {
            out[tone_index(slot, tone)] = derived.get(tone).copied().unwrap_or(last);
        }
    }
    out
}

pub fn recolor(rgba: &mut [u8], index: &[u8], authored: &[u32], tones: &[u32]) {
    for (pixel, entry) in rgba.chunks_exact_mut(4).zip(index) {
        let Some(reference) = usize::from(*entry).checked_sub(1) else {
            continue;
        };
        let Some(tone) = tones.get(reference) else {
            continue;
        };
        let base = authored
            .get(reference)
            .map(|tone| tone >> 24)
            .filter(|&alpha| alpha > 0)
            .unwrap_or(255);
        pixel[0] = (tone >> 16) as u8;
        pixel[1] = (tone >> 8) as u8;
        pixel[2] = *tone as u8;
        let alpha = u32::from(pixel[3]) * (tone >> 24);
        pixel[3] = ((alpha + base / 2) / base).min(255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_find_slots_and_colorway_overrides() {
        let palette = Palette {
            slots: vec![
                Slot {
                    id: "fill".into(),
                    name: "Fill".into(),
                    ramp: vec![1, 2],
                    stops: vec![1, 2],
                    interpolate: None,
                    customizable: false,
                },
                Slot {
                    id: "glow".into(),
                    name: "Glow".into(),
                    ramp: vec![3],
                    stops: vec![3],
                    interpolate: None,
                    customizable: true,
                },
            ],
            colorways: vec![Colorway {
                id: "mint".into(),
                overrides: vec![SlotOverride {
                    slot: 1,
                    ramp: vec![9, 8],
                    stops: vec![9],
                }],
            }],
        };
        assert_eq!(palette.slot("glow"), Some(1));
        assert_eq!(palette.colorway("mint"), Some(0));
        assert_eq!(palette.ramp(1, None), Some(&[3][..]));
        assert_eq!(palette.ramp(1, Some(0)), Some(&[9, 8][..]));
        assert_eq!(palette.stops(1, Some(0)), Some(&[9][..]));
        assert_eq!(palette.ramp(0, Some(0)), Some(&[1, 2][..]));
        assert_eq!(palette.ramp(2, None), None);
        assert_eq!(tone_index(2, 3), 35);
        assert_eq!(tone_index(0, 40), 15);
    }
}
