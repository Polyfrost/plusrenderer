use std::ops::Range;

use super::format::{last_spawn, MOMENTS};
use super::{EffectSet, MomentPlan};
use crate::animation::{Animation, KeyEventKind, Loop};
use crate::container::MANIFEST;
use crate::issues::{join, path, Issue, IssueCode, Report};
use crate::limits::EVENT_EFFECT_SECONDS;

const OGG_HEADER: usize = 27;
const OGG_CONTINUED: u8 = 0x01;
const OGG_FIRST: u8 = 0x02;
const OGG_LAST: u8 = 0x04;
const OGG_NO_GRANULE: i64 = -1;
const VORBIS_HEADERS: usize = 3;
const VORBIS_ID_BYTES: usize = 30;
const VORBIS_BLOCK_EXPONENTS: std::ops::RangeInclusive<u8> = 6..=13;
const PCM_BYTES: u64 = 2;
const VORBIS_SYNC: u32 = 0x56_4342;
const VORBIS_FLOOR_POSTS: u32 = 63;
const VANILLA_SOUND_SECONDS: &[(&str, f32)] = &[
    ("minecraft:block.amethyst_block.chime", 4.33),
    ("minecraft:block.beacon.activate", 2.96),
    ("minecraft:block.bubble_column.bubble_pop", 0.38),
    ("minecraft:block.enchantment_table.use", 1.71),
    ("minecraft:block.end_portal_frame.fill", 1.75),
    ("minecraft:block.fire.extinguish", 1.25),
    ("minecraft:block.note_block.bell", 0.44),
    ("minecraft:block.note_block.chime", 1.11),
    ("minecraft:block.note_block.flute", 0.62),
    ("minecraft:block.note_block.harp", 0.58),
    ("minecraft:block.note_block.pling", 0.65),
    ("minecraft:block.note_block.xylophone", 0.15),
    ("minecraft:block.respawn_anchor.charge", 4.21),
    ("minecraft:entity.allay.ambient_without_item", 3.0),
    ("minecraft:entity.evoker.cast_spell", 1.98),
    ("minecraft:entity.experience_orb.pickup", 0.75),
    ("minecraft:entity.firework_rocket.blast", 1.0),
    ("minecraft:entity.firework_rocket.launch", 1.63),
    ("minecraft:entity.firework_rocket.twinkle", 1.34),
    ("minecraft:entity.illusioner.cast_spell", 1.98),
    ("minecraft:entity.player.levelup", 1.75),
];

fn plays(clip: &str, moment: &str) -> bool {
    clip == moment
        || clip
            .strip_prefix("animation.")
            .and_then(|rest| rest.strip_suffix(moment))
            .is_some_and(|rest| rest.ends_with('.'))
}

pub fn moment_clip(anims: &[Animation], moment: &str) -> Option<usize> {
    if let Some(index) = anims.iter().position(|a| plays(&a.name, moment)) {
        return Some(index);
    }
    let authored = MOMENTS
        .iter()
        .any(|m| anims.iter().any(|a| plays(&a.name, m)));
    let once = anims.first().is_some_and(|a| a.loop_mode != Loop::Repeat);
    (!authored && once).then_some(0)
}

pub fn plan(e: &mut EffectSet, anims: &[Animation], sounds: &[Vec<u8>], set: bool, r: &mut Report) {
    if !set {
        return;
    }
    let mut moments = Vec::new();
    for (name, range) in &e.named {
        if !MOMENTS.contains(&name.as_str()) {
            continue;
        }
        let clip = moment_clip(anims, name);
        let mut seconds = emitters_length(e, sounds, range.clone());
        if let Some(animation) = clip.and_then(|c| anims.get(c)) {
            seconds = seconds.max(clip_length(e, sounds, animation));
        }
        if seconds > EVENT_EFFECT_SECONDS {
            r.push(
                Issue::error(
                    IssueCode::EffectTooLong,
                    path(MANIFEST, &join("effects", name)),
                )
                .value("limit", EVENT_EFFECT_SECONDS as f64),
            );
        }
        moments.push(MomentPlan {
            name: name.clone(),
            clip: clip.and_then(|c| u16::try_from(c).ok()),
            seconds: seconds.min(EVENT_EFFECT_SECONDS),
        });
    }
    e.moments = moments;
}

fn emitters_length(e: &EffectSet, sounds: &[Vec<u8>], range: Range<u16>) -> f32 {
    let mut longest = 0.0f32;
    for def in e
        .emitters
        .get(usize::from(range.start)..usize::from(range.end))
        .unwrap_or(&[])
    {
        let length = if let Some(sound) = def.sound {
            sound_length(e, sounds, sound)
        } else {
            let life = def
                .particle
                .and_then(|p| e.particles.get(usize::from(p)))
                .map_or(0.0, |p| p.longest_life);
            last_spawn(def) + life
        };
        longest = longest.max(length);
    }
    longest
}

fn clip_length(e: &EffectSet, sounds: &[Vec<u8>], animation: &Animation) -> f32 {
    let Some(delay) = animation.longest_start_delay() else {
        return f32::INFINITY;
    };
    if animation.loop_mode == Loop::Repeat {
        return f32::INFINITY;
    }
    let mut longest = animation.length.max(0.0);
    for event in &animation.events {
        let effect = match &event.kind {
            KeyEventKind::Particle { effect, .. } => e
                .named(effect)
                .map(|range| emitters_length(e, sounds, range)),
            KeyEventKind::Sound { effect } => e
                .sound(effect)
                .map(|sound| sound_length(e, sounds, sound))
                .or_else(|| {
                    e.named(effect)
                        .map(|range| emitters_length(e, sounds, range))
                }),
            KeyEventKind::Timeline(_) => None,
        };
        longest = longest.max(event.time.max(0.0) + effect.unwrap_or(0.0));
    }
    delay + longest
}

fn sound_length(e: &EffectSet, sounds: &[Vec<u8>], sound: u16) -> f32 {
    let Some(def) = e.sounds.get(usize::from(sound)) else {
        return 0.0;
    };
    let seconds = match &def.vanilla {
        Some(name) => vanilla_seconds(name),
        None => sounds
            .get(usize::from(sound))
            .and_then(|bytes| ogg_seconds(bytes))
            .unwrap_or(0.0),
    };
    if def.pitch > 0.0 {
        seconds / def.pitch
    } else {
        seconds
    }
}

fn vanilla_seconds(name: &str) -> f32 {
    VANILLA_SOUND_SECONDS
        .iter()
        .find(|(id, _)| *id == name)
        .map_or(f32::INFINITY, |&(_, seconds)| seconds)
}

fn page_at(bytes: &[u8], at: usize) -> bool {
    bytes.get(at..at + 5) == Some(b"OggS\0".as_slice()) && at + OGG_HEADER <= bytes.len()
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn first_packet(bytes: &[u8]) -> Option<usize> {
    if !page_at(bytes, 0) {
        return None;
    }
    Some(OGG_HEADER + usize::from(*bytes.get(26)?))
}

pub fn ogg_opus(bytes: &[u8]) -> bool {
    first_packet(bytes)
        .is_some_and(|packet| bytes.get(packet..packet + 8) == Some(b"OpusHead".as_slice()))
}

pub fn ogg_codec(bytes: &[u8]) -> &'static str {
    let Some(packet) = first_packet(bytes) else {
        return "unknown";
    };
    let head = |tag: &[u8]| bytes.get(packet..packet + tag.len()) == Some(tag);
    if head(b"OpusHead") {
        "opus"
    } else if head(b"\x7fFLAC") {
        "flac"
    } else if head(b"Speex   ") {
        "speex"
    } else if head(b"\x01vorbis") && ogg_seconds(bytes).is_some_and(|s| s > 0.0) {
        "vorbis"
    } else {
        "unknown"
    }
}

struct OggPage {
    flags: u8,
    granule: i64,
    serial: u32,
    sequence: u32,
    lacing: Range<usize>,
    body: Range<usize>,
}

fn ogg_page(bytes: &[u8], at: usize) -> Option<OggPage> {
    let head = bytes.get(at..at.checked_add(OGG_HEADER)?)?;
    if head[..5] != *b"OggS\0" || head[5] & !(OGG_CONTINUED | OGG_FIRST | OGG_LAST) != 0 {
        return None;
    }
    let lacing_at = at + OGG_HEADER;
    let lacing = bytes.get(lacing_at..lacing_at + usize::from(head[26]))?;
    let start = lacing_at + lacing.len();
    let end = start + lacing.iter().map(|&l| usize::from(l)).sum::<usize>();
    bytes.get(start..end)?;
    Some(OggPage {
        flags: head[5],
        granule: i64::from_le_bytes(head[6..14].try_into().ok()?),
        serial: le_u32(head, 14)?,
        sequence: le_u32(head, 18)?,
        lacing: lacing_at..start,
        body: start..end,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vorbis {
    pub rate: u32,
    pub channels: u8,
    pub granule: i64,
    pub frames: u64,
}

impl Vorbis {
    pub fn seconds(&self) -> f32 {
        (self.granule as f64 / f64::from(self.rate)) as f32
    }

    pub fn decoded_bytes(&self) -> u64 {
        self.frames
            .max(u64::try_from(self.granule).unwrap_or(0))
            .saturating_mul(u64::from(self.channels))
            .saturating_mul(PCM_BYTES)
    }
}

#[derive(Default)]
struct Packets {
    count: usize,
    header: Vec<u8>,
    first: Option<u8>,
    open: bool,
    rate: u32,
    channels: u8,
    blocks: [u64; 2],
    modes: Vec<bool>,
    previous: Option<u64>,
    samples: u64,
    audio: u64,
}

impl Packets {
    fn push(&mut self, segment: &[u8], done: bool) -> Option<()> {
        if self.count < VORBIS_HEADERS {
            self.header.extend_from_slice(segment);
        } else if self.first.is_none() {
            self.first = segment.first().copied();
        }
        self.open = !done;
        if done {
            self.finish()?;
        }
        Some(())
    }

    fn finish(&mut self) -> Option<()> {
        let header = std::mem::take(&mut self.header);
        match self.count {
            0 => self.identification(&header)?,
            1 => comment(&header)?,
            2 => self.modes = setup(&header, self.channels)?,
            _ => {
                if let Some(first) = self.first.take() {
                    self.audio_packet(first)?;
                }
            }
        }
        self.count += 1;
        Some(())
    }

    fn identification(&mut self, packet: &[u8]) -> Option<()> {
        if packet.len() != VORBIS_ID_BYTES || packet.get(..7) != Some(b"\x01vorbis".as_slice()) {
            return None;
        }
        let channels = packet[11];
        let rate = le_u32(packet, 12)?;
        let [small, long] = [packet[28] & 0x0f, packet[28] >> 4];
        let blocks = VORBIS_BLOCK_EXPONENTS;
        if le_u32(packet, 7)? != 0
            || channels == 0
            || rate == 0
            || !blocks.contains(&small)
            || !blocks.contains(&long)
            || small > long
            || packet[29] & 1 == 0
        {
            return None;
        }
        self.channels = channels;
        self.rate = rate;
        self.blocks = [1 << small, 1 << long];
        Some(())
    }

    fn audio_packet(&mut self, first: u8) -> Option<()> {
        if first & 1 != 0 {
            return None;
        }
        let bits = ilog(u32::try_from(self.modes.len().checked_sub(1)?).ok()?);
        let mode = usize::from(first >> 1) & ((1 << bits) - 1);
        let size = self.blocks[usize::from(*self.modes.get(mode)?)];
        if let Some(previous) = self.previous {
            self.samples += previous / 4 + size / 4;
        }
        self.previous = Some(size);
        self.audio += 1;
        Some(())
    }
}

fn ilog(value: u32) -> u32 {
    u32::BITS - value.leading_zeros()
}

struct Bits<'a> {
    bytes: &'a [u8],
    at: u64,
}

impl Bits<'_> {
    fn left(&self) -> u64 {
        (self.bytes.len() as u64 * 8).saturating_sub(self.at)
    }

    fn read(&mut self, count: u32) -> Option<u32> {
        if u64::from(count) > self.left() {
            return None;
        }
        let mut value = 0u64;
        for bit in 0..u64::from(count) {
            let at = self.at + bit;
            let byte = self.bytes[(at / 8) as usize];
            value |= u64::from((byte >> (at % 8)) & 1) << bit;
        }
        self.at += u64::from(count);
        Some(value as u32)
    }

    fn flag(&mut self) -> Option<bool> {
        self.read(1).map(|bit| bit == 1)
    }

    fn skip(&mut self, count: u64) -> Option<()> {
        if count > self.left() {
            return None;
        }
        self.at += count;
        Some(())
    }
}

#[derive(Clone, Copy)]
struct Codebook {
    dimensions: u32,
    entries: u32,
    mapped: bool,
}

fn lookup1_values(entries: u32, dimensions: u32) -> u64 {
    if entries == 0 {
        return 0;
    }
    let fits = |root: u64| {
        let mut product = 1u64;
        for _ in 0..dimensions {
            product = product.saturating_mul(root);
            if product > u64::from(entries) {
                return false;
            }
        }
        true
    };
    let (mut low, mut high) = (1u64, u64::from(entries));
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if fits(middle) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

fn codebook(bits: &mut Bits<'_>) -> Option<Codebook> {
    if bits.read(24)? != VORBIS_SYNC {
        return None;
    }
    let dimensions = bits.read(16)?;
    let entries = bits.read(24)?;
    if ilog(dimensions) + ilog(entries) > 24 {
        return None;
    }
    let mut used = 0u64;
    let mut kraft = 0u64;
    if bits.flag()? {
        let mut length = bits.read(5)? + 1;
        let mut entry = 0;
        while entry < entries {
            let count = bits.read(ilog(entries - entry))?;
            let crowded = count > 0 && (count - 1) >> (length - 1).min(31) > 1;
            if length > 32 || count > entries - entry || crowded {
                return None;
            }
            used += u64::from(count);
            kraft += u64::from(count) << (32 - length);
            entry += count;
            length += 1;
        }
    } else {
        let sparse = bits.flag()?;
        for _ in 0..entries {
            if sparse && !bits.flag()? {
                continue;
            }
            let length = bits.read(5)? + 1;
            used += 1;
            kraft += 1 << (32 - length);
        }
    }
    if used > 1 && kraft != 1 << 32 {
        return None;
    }
    let mapped = match bits.read(4)? {
        0 => false,
        kind @ (1 | 2) => {
            bits.skip(64)?;
            let width = bits.read(4)? + 1;
            bits.skip(1)?;
            let values = match kind {
                1 if dimensions == 0 => 0,
                1 => lookup1_values(entries, dimensions),
                _ => u64::from(entries) * u64::from(dimensions),
            };
            bits.skip(values * u64::from(width))?;
            true
        }
        _ => return None,
    };
    Some(Codebook {
        dimensions,
        entries,
        mapped,
    })
}

fn floor0(bits: &mut Bits<'_>, books: &[Codebook]) -> Option<()> {
    let order = bits.read(8)?;
    let rate = bits.read(16)?;
    let bark = bits.read(16)?;
    bits.skip(6 + 8)?;
    let count = bits.read(4)? + 1;
    if order < 1 || rate < 1 || bark < 1 {
        return None;
    }
    for _ in 0..count {
        let book = books.get(bits.read(8)? as usize)?;
        if !book.mapped || book.dimensions < 1 {
            return None;
        }
    }
    Some(())
}

fn floor1(bits: &mut Bits<'_>, books: &[Codebook]) -> Option<()> {
    let partitions = bits.read(5)?;
    let mut classes = Vec::with_capacity(partitions as usize);
    for _ in 0..partitions {
        classes.push(bits.read(4)? as usize);
    }
    let mut dimensions = [0u32; 16];
    let count = classes.iter().max().map_or(0, |&top| top + 1);
    for dimension in dimensions.iter_mut().take(count) {
        *dimension = bits.read(3)? + 1;
        let subclasses = bits.read(2)?;
        if subclasses != 0 && bits.read(8)? as usize >= books.len() {
            return None;
        }
        for _ in 0..1 << subclasses {
            if bits.read(8)? as usize > books.len() {
                return None;
            }
        }
    }
    bits.skip(2)?;
    let range = bits.read(4)?;
    let mut posts = vec![0, 1u32 << range];
    for class in classes {
        if posts.len() as u32 - 2 + dimensions[class] > VORBIS_FLOOR_POSTS {
            return None;
        }
        for _ in 0..dimensions[class] {
            posts.push(bits.read(range)?);
        }
    }
    posts.sort_unstable();
    posts
        .windows(2)
        .all(|pair| pair[0] != pair[1])
        .then_some(())
}

fn residue(bits: &mut Bits<'_>, books: &[Codebook]) -> Option<()> {
    bits.skip(24 * 3)?;
    let partitions = bits.read(6)? + 1;
    let group = bits.read(8)? as usize;
    let mut cascades = 0;
    for _ in 0..partitions {
        let mut cascade = bits.read(3)?;
        if bits.flag()? {
            cascade |= bits.read(5)? << 3;
        }
        cascades += cascade.count_ones();
    }
    for _ in 0..cascades {
        if !books.get(bits.read(8)? as usize)?.mapped {
            return None;
        }
    }
    let group = books.get(group)?;
    if group.dimensions < 1 {
        return None;
    }
    let mut values = 1u64;
    for _ in 0..group.dimensions {
        values *= u64::from(partitions);
        if values > u64::from(group.entries) {
            return None;
        }
    }
    Some(())
}

fn mapping(bits: &mut Bits<'_>, channels: u32, floors: u32, residues: u32) -> Option<()> {
    let submaps = if bits.flag()? { bits.read(4)? + 1 } else { 1 };
    if bits.flag()? {
        let steps = bits.read(8)? + 1;
        let width = ilog(channels.checked_sub(1)?);
        for _ in 0..steps {
            let magnitude = bits.read(width)?;
            let angle = bits.read(width)?;
            if magnitude == angle || magnitude >= channels || angle >= channels {
                return None;
            }
        }
    }
    if bits.read(2)? != 0 {
        return None;
    }
    if submaps > 1 {
        for _ in 0..channels {
            if bits.read(4)? >= submaps {
                return None;
            }
        }
    }
    for _ in 0..submaps {
        bits.skip(8)?;
        if bits.read(8)? >= floors || bits.read(8)? >= residues {
            return None;
        }
    }
    Some(())
}

fn signed(packet: &[u8], kind: u8) -> bool {
    packet.first() == Some(&kind) && packet.get(1..7) == Some(b"vorbis".as_slice())
}

fn comment(packet: &[u8]) -> Option<()> {
    if !signed(packet, 3) {
        return None;
    }
    let mut at = 7;
    let field = |at: &mut usize| -> Option<()> {
        let length = usize::try_from(le_u32(packet, *at)?).ok()?;
        *at = at.checked_add(4)?.checked_add(length)?;
        (*at <= packet.len()).then_some(())
    };
    field(&mut at)?;
    let count = le_u32(packet, at)?;
    at += 4;
    for _ in 0..count {
        field(&mut at)?;
    }
    (packet.get(at)? & 1 == 1).then_some(())
}

fn setup(packet: &[u8], channels: u8) -> Option<Vec<bool>> {
    if !signed(packet, 5) {
        return None;
    }
    let mut bits = Bits {
        bytes: &packet[7..],
        at: 0,
    };
    let count = bits.read(8)? + 1;
    let mut books = Vec::with_capacity(count as usize);
    for _ in 0..count {
        books.push(codebook(&mut bits)?);
    }
    for _ in 0..bits.read(6)? + 1 {
        if bits.read(16)? != 0 {
            return None;
        }
    }
    let floors = bits.read(6)? + 1;
    for _ in 0..floors {
        match bits.read(16)? {
            0 => floor0(&mut bits, &books)?,
            1 => floor1(&mut bits, &books)?,
            _ => return None,
        }
    }
    let residues = bits.read(6)? + 1;
    for _ in 0..residues {
        if bits.read(16)? > 2 {
            return None;
        }
        residue(&mut bits, &books)?;
    }
    let maps = bits.read(6)? + 1;
    for _ in 0..maps {
        if bits.read(16)? != 0 {
            return None;
        }
        mapping(&mut bits, u32::from(channels), floors, residues)?;
    }
    let count = bits.read(6)? + 1;
    let mut modes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let long = bits.flag()?;
        if bits.read(16)? != 0 || bits.read(16)? != 0 || bits.read(8)? >= maps {
            return None;
        }
        modes.push(long);
    }
    bits.flag()?.then_some(modes)
}

pub fn ogg_vorbis(bytes: &[u8]) -> Option<Vorbis> {
    let first = ogg_page(bytes, 0)?;
    if first.flags & (OGG_FIRST | OGG_CONTINUED) != OGG_FIRST || first.granule != 0 {
        return None;
    }
    let mut packets = Packets::default();
    let mut granule = 0;
    let mut sequence = first.sequence;
    let mut page = first;
    loop {
        if (page.flags & OGG_CONTINUED != 0) != packets.open {
            return None;
        }
        let mut body = page.body.start;
        for &lace in bytes.get(page.lacing.clone())? {
            let end = body + usize::from(lace);
            packets.push(bytes.get(body..end)?, lace < u8::MAX)?;
            body = end;
        }
        if page.flags & OGG_FIRST != 0 && (packets.count != 1 || packets.open) {
            return None;
        }
        if page.granule != OGG_NO_GRANULE {
            let bound = i64::try_from(packets.samples).ok()?;
            if page.granule < granule || page.granule > bound {
                return None;
            }
            granule = page.granule;
        }
        let at = page.body.end;
        if at >= bytes.len() {
            break;
        }
        let next = ogg_page(bytes, at)?;
        sequence = sequence.checked_add(1)?;
        if page.flags & OGG_LAST != 0
            || next.serial != page.serial
            || next.sequence != sequence
            || next.flags & OGG_FIRST != 0
        {
            return None;
        }
        page = next;
    }
    let trimmed = packets.samples.saturating_sub(packets.blocks[1]);
    if page.flags & OGG_LAST == 0
        || page.granule == OGG_NO_GRANULE
        || packets.open
        || packets.count <= VORBIS_HEADERS
        || packets.audio == 0
        || u64::try_from(granule).ok()? < trimmed
    {
        return None;
    }
    Some(Vorbis {
        rate: packets.rate,
        channels: packets.channels,
        granule,
        frames: packets.samples,
    })
}

pub fn ogg_seconds(bytes: &[u8]) -> Option<f32> {
    ogg_vorbis(bytes).map(|vorbis| vorbis.seconds())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::animation::parse;
    use crate::effects::format::tests::{files, load_planned, puff};
    use crate::issues::Mode;
    use crate::molang::Interner;
    use serde_json::json;

    fn ogg_pages(flags: u8, granule: i64, serial: u32, sequence: u32, packets: &[&[u8]]) -> Vec<u8> {
        let mut out = b"OggS\0".to_vec();
        out.push(flags);
        out.extend_from_slice(&granule.to_le_bytes());
        out.extend_from_slice(&serial.to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        let lacing: Vec<u8> =
            packets.iter().flat_map(|p| std::iter::repeat_n(255u8, p.len() / 255).chain([(p.len() % 255) as u8])).collect();
        out.push(lacing.len() as u8);
        out.extend(lacing);
        packets.iter().for_each(|p| out.extend_from_slice(p));
        out
    }

    fn setup_header() -> Vec<u8> {
        let mut bits: Vec<(u64, u32)> = vec![(0, 8), (u64::from(VORBIS_SYNC), 24), (1, 16), (2, 24), (0, 1), (1, 1)];
        bits.extend([(1, 1), (0, 5), (1, 1), (0, 5), (1, 4), (0, 32), (0, 32), (3, 4), (0, 1)]);
        bits.extend(std::iter::repeat_n((5, 4), lookup1_values(2, 1) as usize));
        bits.extend([(0, 6), (0, 16), (0, 6), (1, 16), (0, 5), (0, 2), (2, 4), (0, 6), (0, 16), (0, 24), (0, 24), (0, 24)]);
        bits.extend([(0, 6), (0, 8), (0, 3), (0, 1), (0, 6), (0, 16), (0, 1), (0, 1), (0, 2), (0, 8), (0, 8), (0, 8), (1, 6)]);
        bits.extend([(0, 1), (0, 16), (0, 16), (0, 8), (1, 1), (0, 16), (0, 16), (0, 8), (1, 1)]);
        let mut packet = b"\x05vorbis".to_vec();
        let mut at = 0usize;
        for (value, count) in bits {
            for bit in 0..count {
                if at.is_multiple_of(8) {
                    packet.push(0);
                }
                *packet.last_mut().unwrap() |= (((value >> bit) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        packet
    }

    fn headers(rate: u32) -> Vec<u8> {
        let mut id = b"\x01vorbis".to_vec();
        id.extend_from_slice(&0u32.to_le_bytes());
        id.push(2);
        id.extend_from_slice(&rate.to_le_bytes());
        id.extend_from_slice(&[0; 12]);
        id.extend_from_slice(&[0xb8, 1]);
        let mut comment = b"\x03vorbis".to_vec();
        comment.extend_from_slice(&4u32.to_le_bytes());
        comment.extend_from_slice(b"test");
        comment.extend_from_slice(&1u32.to_le_bytes());
        comment.extend_from_slice(&5u32.to_le_bytes());
        comment.extend_from_slice(b"a=pop\x01");
        let mut out = ogg_pages(OGG_FIRST, 0, 7, 0, &[&id]);
        out.extend(ogg_pages(0, 0, 7, 1, &[&comment, &setup_header()]));
        out
    }

    fn body_pages(samples: i64, serial: u32, last: u32) -> Vec<u8> {
        let longs = vec![&[6u8; 40][..]; usize::try_from(samples).unwrap().div_ceil(1024)];
        let mut out = ogg_pages(0, -1, 7, 2, &longs);
        out.extend(ogg_pages(OGG_LAST, samples, serial, last, &[&[6; 40]]));
        out
    }

    pub(in crate::effects) fn ogg(rate: u32, samples: i64) -> Vec<u8> {
        let mut out = headers(rate);
        out.extend(body_pages(samples, 7, 3));
        out
    }

    #[test]
    fn effects_ogg_lengths_walk_every_page_of_the_stream() {
        let five = ogg(44_100, 44_100 * 5);
        assert_eq!(ogg_seconds(&five), Some(5.0));
        let mut forged = five.clone();
        forged.extend(ogg_pages(OGG_LAST, 5_292, 7, 4, &[&[]]));
        assert_eq!((ogg_seconds(&forged), ogg_codec(&forged)), (None, "unknown"));
        for cut in [five.len() - 1, five.len() - 41, 60] {
            assert_eq!(ogg_seconds(&five[..cut]), None, "cut at {cut}");
        }
        let stream = |serial: u32, sequence: u32| [headers(44_100), body_pages(44_100, serial, sequence)].concat();
        assert_eq!(ogg_seconds(&stream(7, 3)), Some(1.0));
        assert_eq!(ogg_seconds(&stream(8, 3)), None, "another stream's page");
        assert_eq!(ogg_seconds(&stream(7, 4)), None, "a skipped page");
    }

    #[test]
    fn effects_moments_count_their_clip_and_sounds() {
        let manifest = json!({
            "format": 1, "kind": "effect", "id": "sample", "particles": ["puff"], "sounds": ["pop"],
            "effects": { "death": { "particle": "puff" }, "join": { "sound": "pop" }, "kill": { "particle": "puff" } }
        });
        let bundle = files(&manifest, &[("puff", puff())]);
        let clips = json!({ "format_version": "1.8.0", "animations": {
            "animation.fx.death": { "animation_length": 2.5, "bones": {} },
            "animation.fx.kill": { "animation_length": 3.5, "bones": {} }
        }});
        let clips = parse(&clips.to_string(), &mut Interner::default()).unwrap();
        let (set, issues) = load_planned(&bundle, Mode::Client, &clips, &[ogg(44_100, 44_100)]);
        assert_eq!(issues.iter().map(|i| (i.code, i.path.as_str())).collect::<Vec<_>>(), [(IssueCode::EffectTooLong, "manifest.json#effects.kill")]);
        let seconds = |name: &str| set.moments.iter().find(|m| m.name == name).map(|m| (m.clip, m.seconds)).unwrap();
        assert_eq!([seconds("death"), seconds("join"), seconds("kill")], [(Some(0), 2.5), (None, 1.0), (Some(1), 3.0)]);
        let (_, issues) = load_planned(&bundle, Mode::Client, &[], &[ogg(44_100, 44_100 * 4)]);
        assert_eq!(issues.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["manifest.json#effects.join"]);
    }
}
