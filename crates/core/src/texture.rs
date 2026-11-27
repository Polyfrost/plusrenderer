use crate::container::{Bundle, Image};
use crate::issues::{Issue, IssueCode, Report};
use crate::palette::{recolor, Palette, MAX_TONES};

pub const TEXTURE_DIR: &str = "textures";
pub const COLOR_EXT: &str = "png";
pub const INDEX_EXT: &str = "idx.png";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexedTexture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub index: Option<Vec<u8>>,
}

impl IndexedTexture {
    pub fn pixels(&self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn color_file(&self) -> String {
        color_file(&self.name)
    }

    pub fn index_file(&self) -> String {
        index_file(&self.name)
    }
}

pub fn color_file(name: &str) -> String {
    format!("{TEXTURE_DIR}/{name}.{COLOR_EXT}")
}

pub fn index_file(name: &str) -> String {
    format!("{TEXTURE_DIR}/{name}.{INDEX_EXT}")
}

pub fn load_texture(
    bundle: &mut Bundle<'_>,
    name: &str,
    indexed: bool,
    from: &str,
) -> Option<IndexedTexture> {
    let color = bundle.id_path(TEXTURE_DIR, name, COLOR_EXT, from)?;
    let image = bundle.png(&color, from)?;
    let index = if indexed {
        let file = index_file(name);
        bundle.png(&file, &color).and_then(|map| {
            let report = bundle.report();
            if (map.width, map.height) != (image.width, image.height) {
                report.push(
                    Issue::error(IssueCode::Texture, file.as_str())
                        .name("file", &file)
                        .value("width", f64::from(image.width))
                        .value("height", f64::from(image.height))
                        .value("index_width", f64::from(map.width))
                        .value("index_height", f64::from(map.height)),
                );
                return None;
            }
            grey_index(&map, &file, report)
        })
    } else {
        None
    };
    Some(IndexedTexture {
        name: name.to_string(),
        width: image.width,
        height: image.height,
        rgba: image.rgba,
        index,
    })
}

pub fn grey_index(image: &Image, file: &str, r: &mut Report) -> Option<Vec<u8>> {
    let mut index = Vec::with_capacity(image.rgba.len() / 4);
    for (at, pixel) in image.rgba.chunks_exact(4).enumerate() {
        if pixel[0] != pixel[1] || pixel[1] != pixel[2] {
            let width = image.width.max(1) as usize;
            r.push(
                Issue::error(IssueCode::Texture, file)
                    .name("file", file)
                    .value("x", (at % width) as f64)
                    .value("y", (at / width) as f64),
            );
            return None;
        }
        index.push(pixel[0]);
    }
    Some(index)
}

pub fn resolve_texture(t: &IndexedTexture, authored: &[u32], tones: &[u32]) -> Vec<u8> {
    let mut out = t.rgba.clone();
    if let Some(index) = &t.index {
        if index.len() == t.pixels() && out.len() == t.pixels() * 4 {
            recolor(&mut out, index, authored, tones);
        }
    }
    out
}

const KEY_SEED: u64 = 0x7465_7874_7572_6573;
const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn absorb(h: u64, value: u64) -> u64 {
    mix64(h.wrapping_add(GOLDEN_GAMMA) ^ value)
}

pub fn texture_key(
    identity: u64,
    texture: u16,
    colorway: Option<u16>,
    custom: &[(u8, u32)],
) -> u64 {
    let mut h = absorb(KEY_SEED, identity);
    h = absorb(h, u64::from(texture));
    h = absorb(h, colorway.map_or(0, |c| u64::from(c) + 1));
    let mut sum = 0u64;
    let mut count = 0u64;
    for (at, (slot, rgb)) in custom.iter().enumerate() {
        if custom[at + 1..].iter().any(|(later, _)| later == slot) {
            continue;
        }
        let entry = (u64::from(*slot) << 32) | u64::from(rgb & 0x00ff_ffff);
        sum = sum.wrapping_add(mix64(entry ^ KEY_SEED));
        count += 1;
    }
    h = absorb(h, count);
    absorb(h, sum)
}

pub fn check_index(t: &IndexedTexture, p: &Palette, r: &mut Report) {
    let Some(index) = &t.index else {
        return;
    };
    let file = t.index_file();
    if index.len() != t.pixels() {
        r.push(
            Issue::error(IssueCode::Texture, file.as_str())
                .name("file", &file)
                .value("pixels", t.pixels() as f64)
                .value("index", index.len() as f64),
        );
        return;
    }
    let mut first = [None::<usize>; 256];
    for (at, value) in index.iter().enumerate() {
        first[usize::from(*value)].get_or_insert(at);
    }
    let width = t.width.max(1) as usize;
    for (value, at) in first.iter().enumerate().skip(1) {
        let Some(at) = at else {
            continue;
        };
        let slot = (value - 1) / MAX_TONES;
        let tone = (value - 1) % MAX_TONES;
        if slot < p.slots.len() && tone < p.tone_count(slot, None) {
            continue;
        }
        r.push(
            Issue::error(IssueCode::Palette, file.as_str())
                .name("file", &file)
                .value("index", value as f64)
                .value("slot", slot as f64)
                .value("tone", tone as f64)
                .value("x", (at % width) as f64)
                .value("y", (at / width) as f64),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::palette::{custom_tones, Colorway, Slot, SlotOverride};

    fn slot(id: &str, ramp: &[u32], customizable: bool) -> Slot {
        Slot {
            id: id.into(),
            name: id.into(),
            ramp: ramp.to_vec(),
            stops: Vec::new(),
            interpolate: None,
            customizable,
        }
    }

    fn felt() -> Palette {
        Palette {
            slots: vec![
                slot("fur", &[0xff40_2010, 0xff80_4020, 0xffc0_8040], true),
                slot("band", &[0x80ff_0000], false),
            ],
            colorways: vec![Colorway {
                id: "snow".into(),
                overrides: vec![SlotOverride {
                    slot: 0,
                    ramp: vec![0xff60_6060, 0xffa0_a0a0, 0xffe0_e0e0],
                    stops: Vec::new(),
                }],
            }],
        }
    }

    fn hat() -> IndexedTexture {
        IndexedTexture {
            name: "hat".into(),
            width: 2,
            height: 2,
            rgba: vec![10, 20, 30, 255, 1, 1, 1, 255, 2, 2, 2, 200, 3, 3, 3, 128],
            index: Some(vec![0, 1, 3, 17]),
        }
    }

    fn pixel(rgba: &[u8], i: usize) -> u32 {
        let p = &rgba[i * 4..i * 4 + 4];
        u32::from_be_bytes([p[3], p[0], p[1], p[2]])
    }

    #[test]
    fn switching_colourway_changes_the_key_and_the_pixels() {
        let palette = felt();
        let texture = hat();
        let base = resolve_texture(
            &texture,
            &palette.resolve(None),
            &custom_tones(&palette, None, &[]),
        );
        let snow = resolve_texture(
            &texture,
            &palette.resolve(None),
            &custom_tones(&palette, Some(0), &[]),
        );
        assert_ne!(base, snow);
        assert_eq!(pixel(&snow, 1), 0xff60_6060);
        assert_eq!(pixel(&snow, 3), pixel(&base, 3));

        let id = 0x0123_4567_89ab_cdef;
        assert_ne!(
            texture_key(id, 0, None, &[]),
            texture_key(id, 0, Some(0), &[])
        );
        assert_ne!(
            texture_key(id, 0, Some(0), &[]),
            texture_key(id, 0, Some(1), &[])
        );
        assert_ne!(texture_key(id, 0, None, &[]), texture_key(id, 1, None, &[]));
        assert_ne!(
            texture_key(id, 0, None, &[]),
            texture_key(id + 1, 0, None, &[])
        );
        assert_ne!(
            texture_key(id, 0, None, &[]),
            texture_key(id, 0, None, &[(0, 0x00ff_0000)])
        );
        assert_ne!(
            texture_key(id, 0, None, &[(0, 0x00ff_0000)]),
            texture_key(id, 0, None, &[(1, 0x00ff_0000)])
        );
    }
}
