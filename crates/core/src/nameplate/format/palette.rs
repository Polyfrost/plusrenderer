use serde_json::{Map, Value};

use crate::container::is_id;
use crate::issues::IssueCode;
use crate::limits::{
    NAMEPLATE_BAND_WEIGHT, NAMEPLATE_FLAGS, NAMEPLATE_FLAG_BANDS, NAMEPLATE_OVERLAY_COLORS,
    NAMEPLATE_OVERLAY_WIDTH, NAMEPLATE_SLOTS, NAMEPLATE_TONES,
};
use crate::palette::{interpolate, Colorway, Slot, SlotOverride};

use super::super::image::Axis;
use super::super::types::{Band, Flag, Overlay, OverlayKind, RowsOverride};
use super::read::{self, Loc};
use super::{reserved, Compiler, FLAGS, PALETTES};

const PALETTE_KEYS: [&str; 2] = ["slots", "colorways"];
const SLOT_KEYS: [&str; 5] = ["id", "name", "ramp", "interpolate", "customizable"];
const FLAGS_KEYS: [&str; 1] = ["flags"];
const FLAG_KEYS: [&str; 6] = [
    "id",
    "bands",
    "overlay",
    "rowsOverride",
    "symmetric",
    "axis",
];
const BAND_KEYS: [&str; 2] = ["color", "weight"];
const OVERLAY_KEYS: [&str; 3] = ["kind", "colors", "width"];

impl Compiler<'_, '_> {
    pub(super) fn palettes(&mut self) -> bool {
        let from = Loc::new(super::MANIFEST, "");
        let Some(json) = self.bundle.json(PALETTES, &from.path()) else {
            return false;
        };
        let root = Loc::new(PALETTES, "");
        let Some(map) = self.object(&json, &root, IssueCode::Palette) else {
            return false;
        };
        self.keys(&root, map, &PALETTE_KEYS);

        let slots_loc = root.key("slots");
        let slots = match map.get("slots") {
            Some(value) => self
                .list(value, &slots_loc, IssueCode::Palette)
                .cloned()
                .unwrap_or_default(),
            None => {
                self.push(slots_loc.error(IssueCode::Palette));
                return false;
            }
        };
        if slots.is_empty() || slots.len() > NAMEPLATE_SLOTS as usize {
            self.push(
                slots_loc
                    .error(IssueCode::Palette)
                    .value("count", slots.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_SLOTS)),
            );
        }
        for (i, value) in slots.iter().enumerate() {
            self.slot(value, &slots_loc, i);
        }

        if let Some(value) = map.get("colorways") {
            let loc = root.key("colorways");
            if let Some(colorways) = self.object(value, &loc, IssueCode::Colorway) {
                for (name, overrides) in colorways {
                    self.colorway(name, overrides, &loc);
                }
            }
        }
        self.palette.slots.len() == slots.len() && !self.palette.slots.is_empty()
    }

    fn slot(&mut self, value: &Value, slots: &Loc, i: usize) {
        let Some(map) = self.object(value, &slots.index(i), IssueCode::Palette) else {
            return;
        };
        let id = map.get("id").and_then(Value::as_str).unwrap_or("");
        let loc = if is_id(id) {
            slots.key(id)
        } else {
            slots.index(i)
        };
        self.keys(&loc, map, &SLOT_KEYS);
        if !is_id(id) {
            self.push(loc.key("id").error(IssueCode::IdInvalid).name("id", id));
            return;
        }
        if reserved(id) || self.palette.slot(id).is_some() {
            self.push(loc.key("id").error(IssueCode::Palette).name("id", id));
            return;
        }
        let name = match self.string(map, "name", &loc, IssueCode::Palette) {
            Some(name) => name.to_string(),
            None => id.to_string(),
        };
        let customizable = self.boolean(map, "customizable", &loc, IssueCode::Palette, false);
        let Some(stops) = self.ramp(map.get("ramp"), &loc.key("ramp"), IssueCode::Palette) else {
            return;
        };
        let interpolate_to = match map.get("interpolate") {
            None => None,
            Some(value) => {
                let at = loc.key("interpolate");
                match read::whole(value) {
                    Some(n)
                        if stops.len() >= 2
                            && n >= stops.len() as i64
                            && n <= i64::from(NAMEPLATE_TONES) =>
                    {
                        Some(n as u8)
                    }
                    other => {
                        let mut issue = at
                            .error(IssueCode::Palette)
                            .value("stops", stops.len() as f64)
                            .value("limit", f64::from(NAMEPLATE_TONES));
                        if let Some(n) = other {
                            issue = issue.value("value", n as f64);
                        }
                        self.push(issue);
                        None
                    }
                }
            }
        };
        let ramp = expand(&stops, interpolate_to);
        self.tones.push(ramp.len() as u8);
        self.palette.slots.push(Slot {
            id: id.to_string(),
            name,
            ramp,
            stops,
            interpolate: interpolate_to,
            customizable,
        });
    }

    fn ramp(&mut self, value: Option<&Value>, loc: &Loc, code: IssueCode) -> Option<Vec<u32>> {
        let Some(value) = value else {
            self.push(loc.error(code));
            return None;
        };
        let items = self.list(value, loc, code)?;
        if items.is_empty() || items.len() > NAMEPLATE_TONES as usize {
            self.push(
                loc.error(code)
                    .value("count", items.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_TONES)),
            );
            return None;
        }
        let mut colors = Vec::with_capacity(items.len());
        let mut ok = true;
        for (i, item) in items.iter().enumerate() {
            match item.as_str().and_then(read::hex) {
                Some(argb) => colors.push(argb),
                None => {
                    let mut issue = loc.index(i).error(code);
                    if let Some(text) = item.as_str() {
                        issue = issue.name("color", text);
                    }
                    self.push(issue);
                    ok = false;
                }
            }
        }
        ok.then_some(colors)
    }

    fn colorway(&mut self, name: &str, overrides: &Value, colorways: &Loc) {
        let loc = colorways.key(name);
        if !is_id(name) {
            self.push(loc.error(IssueCode::IdInvalid).name("id", name));
            return;
        }
        let Some(map) = self.object(overrides, &loc, IssueCode::Colorway) else {
            return;
        };
        let mut colorway = Colorway {
            id: name.to_string(),
            overrides: Vec::new(),
        };
        for (slot_id, value) in map {
            let at = loc.key(slot_id);
            let Some(slot) = self.palette.slot(slot_id) else {
                self.push(at.error(IssueCode::Colorway).name("slot", slot_id));
                continue;
            };
            let Some(stops) = self.ramp(Some(value), &at, IssueCode::Colorway) else {
                continue;
            };
            let ramp = expand(&stops, self.palette.slots[slot].interpolate);
            colorway.overrides.push(SlotOverride {
                slot: slot as u8,
                ramp,
                stops,
            });
        }
        self.palette.colorways.push(colorway);
    }

    pub(super) fn colorway_tones(&mut self, banded: &[u8]) {
        let mut found = Vec::new();
        for colorway in &self.palette.colorways {
            for o in &colorway.overrides {
                let base = self.tones.get(usize::from(o.slot)).copied().unwrap_or(0);
                if banded.contains(&o.slot) && o.ramp.len() != usize::from(base) {
                    let slot = &self.palette.slots[usize::from(o.slot)].id;
                    found.push((colorway.id.clone(), slot.clone(), o.ramp.len(), base));
                }
            }
        }
        let root = Loc::new(PALETTES, "colorways");
        for (colorway, slot, tones, base) in found {
            self.soft(
                root.key(&colorway)
                    .key(&slot)
                    .error(IssueCode::ColorwayTones)
                    .name("slot", &slot)
                    .value("tones", tones as f64)
                    .value("base", f64::from(base)),
            );
        }
    }

    pub(super) fn load_flags(&mut self) {
        self.flags_loaded = true;
        let from = Loc::new(super::MANIFEST, "");
        let Some(json) = self.bundle.json(FLAGS, &from.path()) else {
            return;
        };
        let root = Loc::new(FLAGS, "");
        let Some(map) = self.object(&json, &root, IssueCode::Flags) else {
            return;
        };
        self.keys(&root, map, &FLAGS_KEYS);
        let loc = root.key("flags");
        let Some(items) = map.get("flags") else {
            self.push(loc.error(IssueCode::Flags));
            return;
        };
        let Some(items) = self.list(items, &loc, IssueCode::Flags) else {
            return;
        };
        if items.is_empty() || items.len() > NAMEPLATE_FLAGS as usize {
            self.push(
                loc.error(IssueCode::Flags)
                    .value("count", items.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_FLAGS)),
            );
        }
        for (i, item) in items.iter().enumerate() {
            if let Some(flag) = self.flag(item, &loc, i) {
                self.flags.push(flag);
            }
        }
    }

    fn flag(&mut self, value: &Value, flags: &Loc, i: usize) -> Option<Flag> {
        let map = self.object(value, &flags.index(i), IssueCode::Flags)?;
        let id = map
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let loc = if is_id(&id) {
            flags.key(&id)
        } else {
            flags.index(i)
        };
        self.keys(&loc, map, &FLAG_KEYS);
        if !is_id(&id) {
            self.push(loc.key("id").error(IssueCode::IdInvalid).name("id", &id));
            return None;
        }
        if self.flags.iter().any(|f| f.id == id) {
            self.push(loc.key("id").error(IssueCode::Flags).name("id", &id));
            return None;
        }
        let bands_loc = loc.key("bands");
        let bands = match map.get("bands") {
            Some(value) => self
                .list(value, &bands_loc, IssueCode::Flags)
                .cloned()
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if bands.is_empty() || bands.len() > NAMEPLATE_FLAG_BANDS as usize {
            self.push(
                bands_loc
                    .error(IssueCode::Flags)
                    .value("count", bands.len() as f64)
                    .value("limit", f64::from(NAMEPLATE_FLAG_BANDS)),
            );
        }
        let mut parsed = Vec::new();
        for (b, band) in bands.iter().enumerate() {
            let at = bands_loc.index(b);
            let Some(band) = self.object(band, &at, IssueCode::Flags) else {
                continue;
            };
            self.keys(&at, band, &BAND_KEYS);
            let color = match self.string(band, "color", &at, IssueCode::Flags) {
                Some(text) => self.slot_color(text, &at.key("color"), IssueCode::Flags),
                None => {
                    self.push(at.key("color").error(IssueCode::Flags));
                    None
                }
            };
            let weight = self.int(
                band,
                "weight",
                &at,
                IssueCode::Flags,
                (1, i64::from(NAMEPLATE_BAND_WEIGHT)),
                1,
            );
            if let Some(color) = color {
                parsed.push(Band {
                    color,
                    weight: weight as u16,
                });
            }
        }
        let axis = self.choice(
            map,
            "axis",
            &loc,
            IssueCode::Flags,
            |name| match name {
                "rows" => Some(Axis::Y),
                "columns" => Some(Axis::X),
                _ => None,
            },
            Axis::Y,
        );
        let symmetric = self.boolean(map, "symmetric", &loc, IssueCode::Flags, false);
        let weights: Vec<u16> = parsed.iter().map(|b| b.weight).collect();
        if symmetric && !weights.iter().eq(weights.iter().rev()) {
            self.push(loc.key("symmetric").error(IssueCode::Flags));
        }
        let overlay = match map.get("overlay") {
            Some(value) => self.overlay(value, &loc.key("overlay")),
            None => None,
        };
        let mut rows = Vec::new();
        if let Some(value) = map.get("rowsOverride") {
            let at = loc.key("rowsOverride");
            if axis == Axis::X {
                self.push(at.error(IssueCode::Flags));
            } else if let Some(overrides) = self.object(value, &at, IssueCode::Flags) {
                for (count, fitted) in overrides {
                    let here = at.key(count);
                    let total = count.parse::<u16>().ok();
                    let fitted: Option<Vec<u16>> = fitted.as_array().and_then(|items| {
                        items
                            .iter()
                            .map(|v| {
                                read::whole(v)
                                    .filter(|n| *n >= 1)
                                    .and_then(|n| u16::try_from(n).ok())
                            })
                            .collect()
                    });
                    match (total, fitted) {
                        (Some(total), Some(fitted))
                            if fitted.len() == parsed.len()
                                && fitted.iter().map(|n| u32::from(*n)).sum::<u32>()
                                    == u32::from(total) =>
                        {
                            rows.push(RowsOverride {
                                rows: total,
                                bands: fitted,
                            });
                        }
                        _ => self.push(
                            here.error(IssueCode::Flags)
                                .value("bands", parsed.len() as f64),
                        ),
                    }
                }
            }
        }
        Some(Flag {
            id,
            bands: parsed,
            overlay,
            rows,
            symmetric,
            axis,
        })
    }

    fn overlay(&mut self, value: &Value, loc: &Loc) -> Option<Overlay> {
        let map: &Map<String, Value> = self.object(value, loc, IssueCode::Flags)?;
        self.keys(loc, map, &OVERLAY_KEYS);
        let kind = match map.get("kind").and_then(Value::as_str) {
            Some("chevron") => OverlayKind::Chevron,
            Some("triangle") => OverlayKind::Triangle,
            Some("circle") => OverlayKind::Circle,
            _ => {
                self.push(loc.key("kind").error(IssueCode::Flags));
                return None;
            }
        };
        let (min, max) = match kind {
            OverlayKind::Chevron => (1, NAMEPLATE_OVERLAY_COLORS as usize),
            OverlayKind::Triangle => (1, 1),
            OverlayKind::Circle => (1, 2),
        };
        let colors_loc = loc.key("colors");
        let items = match map.get("colors") {
            Some(value) => self
                .list(value, &colors_loc, IssueCode::Flags)
                .cloned()
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if items.len() < min || items.len() > max {
            self.push(
                colors_loc
                    .error(IssueCode::Flags)
                    .value("count", items.len() as f64)
                    .value("min", min as f64)
                    .value("max", max as f64),
            );
        }
        let mut colors = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let at = colors_loc.index(i);
            match item.as_str() {
                Some(text) => {
                    if let Some(color) = self.slot_color(text, &at, IssueCode::Flags) {
                        colors.push(color);
                    }
                }
                None => self.push(at.error(IssueCode::Flags)),
            }
        }
        let width = self.int(
            map,
            "width",
            loc,
            IssueCode::Flags,
            (1, i64::from(NAMEPLATE_OVERLAY_WIDTH)),
            1,
        );
        Some(Overlay {
            kind,
            colors,
            width: width as u8,
        })
    }
}

fn expand(stops: &[u32], to: Option<u8>) -> Vec<u32> {
    match to {
        Some(n) if stops.len() >= 2 && usize::from(n) > stops.len() => {
            interpolate(stops, usize::from(n))
        }
        _ => stops.to_vec(),
    }
}
