use std::collections::BTreeSet;
use std::str::from_utf8;

use crc32fast::hash;
use miniz_oxide::inflate::decompress_to_vec_with_limit;

use super::ReadFail;

const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const END_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;

const LOCAL_LEN: usize = 30;
const CENTRAL_LEN: usize = 46;
const END_LEN: usize = 22;
const ZIP64_LOCATOR_LEN: usize = 20;

const STORED: u16 = 0;
const DEFLATED: u16 = 8;

const FLAG_ENCRYPTED: u16 = 1;
const FLAG_DESCRIPTOR: u16 = 1 << 3;
const FLAG_STRONG_ENCRYPTION: u16 = 1 << 6;
const FLAG_MASKED_HEADERS: u16 = 1 << 13;
const FLAGS_UNSUPPORTED: u16 = FLAG_ENCRYPTED | FLAG_STRONG_ENCRYPTION | FLAG_MASKED_HEADERS;

const CP437_HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";

struct Record<'a> {
    raw_name: &'a [u8],
    name: String,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: usize,
}

pub(super) struct Archive<'a> {
    bytes: &'a [u8],
    records: Vec<Record<'a>>,
}

impl<'a> Archive<'a> {
    pub(super) fn parse(bytes: &'a [u8]) -> Option<Archive<'a>> {
        let end = find_end(bytes)?;
        if end >= ZIP64_LOCATOR_LEN
            && u32_at(bytes, end - ZIP64_LOCATOR_LEN) == Some(ZIP64_LOCATOR_SIGNATURE)
        {
            return None;
        }
        let disk = u16_at(bytes, end + 4)?;
        let directory_disk = u16_at(bytes, end + 6)?;
        let on_disk = u16_at(bytes, end + 8)?;
        let total = u16_at(bytes, end + 10)?;
        let size = u32_at(bytes, end + 12)?;
        let offset = u32_at(bytes, end + 16)?;
        if disk != 0
            || directory_disk != 0
            || on_disk != total
            || total == u16::MAX
            || size == u32::MAX
            || offset == u32::MAX
        {
            return None;
        }
        let start = offset as usize;
        let stop = start.checked_add(size as usize)?;
        if stop > end {
            return None;
        }
        let directory = &bytes[start..stop];

        let mut records: Vec<Record<'a>> = Vec::new();
        let mut names: BTreeSet<String> = BTreeSet::new();
        let mut at = 0;
        for _ in 0..total {
            let (record, next) = central_record(directory, at)?;
            at = next;
            if !names.insert(record.name.clone()) {
                return None;
            }
            records.push(record);
        }
        Some(Archive { bytes, records })
    }

    pub(super) fn len(&self) -> usize {
        self.records.len()
    }

    pub(super) fn name(&self, index: usize) -> &str {
        &self.records[index].name
    }

    pub(super) fn read(&self, index: usize, limit: u64) -> Result<Vec<u8>, ReadFail> {
        let record = self.records.get(index).ok_or(ReadFail::Broken)?;
        if record.flags & FLAGS_UNSUPPORTED != 0
            || (record.method != STORED && record.method != DEFLATED)
        {
            return Err(ReadFail::Broken);
        }
        let data = self.data(record).ok_or(ReadFail::Broken)?;
        let size = u64::from(record.size);
        if size > limit {
            return Err(ReadFail::TooLarge);
        }
        let out = if record.method == STORED {
            if data.len() as u64 != size {
                return Err(ReadFail::Broken);
            }
            data.to_vec()
        } else {
            decompress_to_vec_with_limit(data, record.size as usize)
                .map_err(|_| ReadFail::Broken)?
        };
        if out.len() as u64 != size || hash(&out) != record.crc {
            return Err(ReadFail::Broken);
        }
        Ok(out)
    }

    fn data(&self, record: &Record) -> Option<&'a [u8]> {
        let bytes = self.bytes;
        let at = record.offset;
        if u32_at(bytes, at)? != LOCAL_SIGNATURE {
            return None;
        }
        let flags = u16_at(bytes, at + 6)?;
        if u16_at(bytes, at + 8)? != record.method
            || flags & FLAGS_UNSUPPORTED != record.flags & FLAGS_UNSUPPORTED
        {
            return None;
        }
        if flags & FLAG_DESCRIPTOR == 0
            && (u32_at(bytes, at + 14)? != record.crc
                || u32_at(bytes, at + 18)? != record.compressed
                || u32_at(bytes, at + 22)? != record.size)
        {
            return None;
        }
        let name_len = usize::from(u16_at(bytes, at + 26)?);
        let extra_len = usize::from(u16_at(bytes, at + 28)?);
        let name_start = at.checked_add(LOCAL_LEN)?;
        let name_stop = name_start.checked_add(name_len)?;
        if bytes.get(name_start..name_stop)? != record.raw_name {
            return None;
        }
        let start = name_stop.checked_add(extra_len)?;
        let stop = start.checked_add(record.compressed as usize)?;
        bytes.get(start..stop)
    }
}

fn find_end(bytes: &[u8]) -> Option<usize> {
    let last = bytes.len().checked_sub(END_LEN)?;
    let first = last.saturating_sub(usize::from(u16::MAX));
    (first..=last).rev().find(|&at| {
        u32_at(bytes, at) == Some(END_SIGNATURE)
            && u16_at(bytes, at + 20).map(|len| at + END_LEN + usize::from(len))
                == Some(bytes.len())
    })
}

fn central_record(directory: &[u8], at: usize) -> Option<(Record<'_>, usize)> {
    if u32_at(directory, at)? != CENTRAL_SIGNATURE {
        return None;
    }
    let compressed = u32_at(directory, at + 20)?;
    let size = u32_at(directory, at + 24)?;
    let name_len = usize::from(u16_at(directory, at + 28)?);
    let extra_len = usize::from(u16_at(directory, at + 30)?);
    let comment_len = usize::from(u16_at(directory, at + 32)?);
    let disk = u16_at(directory, at + 34)?;
    let offset = u32_at(directory, at + 42)?;
    if compressed == u32::MAX || size == u32::MAX || offset == u32::MAX || disk != 0 {
        return None;
    }
    let name_start = at.checked_add(CENTRAL_LEN)?;
    let name_stop = name_start.checked_add(name_len)?;
    let raw_name = directory.get(name_start..name_stop)?;
    let next = name_stop.checked_add(extra_len)?.checked_add(comment_len)?;
    if next > directory.len() {
        return None;
    }
    let record = Record {
        raw_name,
        name: decode_name(raw_name),
        flags: u16_at(directory, at + 8)?,
        method: u16_at(directory, at + 10)?,
        crc: u32_at(directory, at + 16)?,
        compressed,
        size,
        offset: offset as usize,
    };
    Some((record, next))
}

fn decode_name(raw: &[u8]) -> String {
    match from_utf8(raw) {
        Ok(name) => name.to_string(),
        Err(_) => raw
            .iter()
            .map(|&b| match b {
                0..=0x7f => char::from(b),
                _ => CP437_HIGH
                    .chars()
                    .nth(usize::from(b - 0x80))
                    .unwrap_or(char::REPLACEMENT_CHARACTER),
            })
            .collect(),
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
