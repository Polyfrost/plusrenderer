mod zipread;

use std::io::Cursor;

use png::{ColorType, Decoder, Transformations};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::bundle::{self, Parts};
use crate::issues::{path, sanitize, Issue, IssueCode, Mode, Report};
use crate::limits::{
    Caps, EFFECT_CAPS, EMOTE_CAPS, ID_MAX_LEN, ID_MIN_LEN, ITEM_CAPS, JSON_MAX_BYTES,
    JSON_MAX_DEPTH, NAMEPLATE_CAPS, NAMEPLATE_TEXTURE_MAX_SIZE, PET_CAPS, TEXTURE_MAX_SIZE,
};
use zipread::Archive;

pub const FORMAT: u64 = 1;
pub const CAPABILITIES: &[&str] = &[
    "nameplate",
    "nameplate_materials",
    "nameplate_flags",
    "nameplate_particles",
    "nameplate_paths",
    "nameplate_letters",
    "nameplate_owned_text",
    "nameplate_reactions",
    "nameplate_one_shots",
    "nameplate_schedules",
    "nameplate_palette_animation",
    "nameplate_masks",
    "nameplate_body_art",
    "reactions",
    "palettes",
];
pub const ENGINE_CAPABILITIES: &[&str] = &[
    "controllers",
    "springs",
    "effects",
    "particles",
    "trails",
    "sounds",
    "custom_colors",
    "rigs",
    "pets",
    "roles",
    "events",
];
pub const MANIFEST: &str = "manifest.json";
pub const MANIFEST_KEYS: [&str; 4] = ["format", "kind", "id", "requires"];
pub const LEGACY_FILE: &str = "asset.bin";

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Item,
    Pet,
    Emote,
    Effect,
    Nameplate,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Item,
        Kind::Pet,
        Kind::Emote,
        Kind::Effect,
        Kind::Nameplate,
    ];

    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Item => "item",
            Kind::Pet => "pet",
            Kind::Emote => "emote",
            Kind::Effect => "effect",
            Kind::Nameplate => "nameplate",
        }
    }

    pub fn caps(self) -> Caps {
        match self {
            Kind::Item => ITEM_CAPS,
            Kind::Pet => PET_CAPS,
            Kind::Emote => EMOTE_CAPS,
            Kind::Effect => EFFECT_CAPS,
            Kind::Nameplate => NAMEPLATE_CAPS,
        }
    }

    pub fn max_texture_size(self) -> u32 {
        match self {
            Kind::Nameplate => NAMEPLATE_TEXTURE_MAX_SIZE,
            _ => TEXTURE_MAX_SIZE,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    pub mode: Mode,
    pub legacy: Caps,
    pub capabilities: &'a [&'a str],
}

impl Options<'static> {
    pub fn new(mode: Mode) -> Self {
        Options {
            mode,
            legacy: ITEM_CAPS,
            capabilities: CAPABILITIES,
        }
    }
}

impl Default for Options<'static> {
    fn default() -> Self {
        Options::new(Mode::Client)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Source<'a> {
    Zip(&'a [u8]),
    Files(&'a [(String, Vec<u8>)]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    Format(u64),
    Requires(Vec<String>),
}

pub enum Opened<'a> {
    Bundle(Bundle<'a>),
    Legacy(Legacy),
    Skipped(Skip),
    Invalid(Vec<Issue>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Legacy {
    pub files: Vec<(String, Vec<u8>)>,
    pub parts: Parts,
}

impl Legacy {
    pub fn file(&self, index: Option<usize>) -> Option<(&str, &[u8])> {
        let (name, data) = self.files.get(index?)?;
        Some((name.as_str(), data.as_slice()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Entry {
    index: usize,
    name: String,
    dir: bool,
    safe: bool,
    read: bool,
    counted: bool,
}

enum ReadFail {
    TooLarge,
    Broken,
}

enum Store<'a> {
    Zip(Archive<'a>),
    Files(Vec<&'a (String, Vec<u8>)>),
}

impl<'a> Store<'a> {
    fn files(files: &'a [(String, Vec<u8>)]) -> Store<'a> {
        let mut kept: Vec<&'a (String, Vec<u8>)> = Vec::new();
        for file in files {
            match kept.iter().position(|k| k.0 == file.0) {
                Some(at) => kept[at] = file,
                None => kept.push(file),
            }
        }
        Store::Files(kept)
    }

    fn len(&self) -> usize {
        match self {
            Store::Zip(archive) => archive.len(),
            Store::Files(files) => files.len(),
        }
    }

    fn name(&self, index: usize) -> &str {
        match self {
            Store::Zip(archive) => archive.name(index),
            Store::Files(files) => &files[index].0,
        }
    }

    fn read(&self, index: usize, limit: u64) -> Result<Vec<u8>, ReadFail> {
        match self {
            Store::Zip(archive) => archive.read(index, limit),
            Store::Files(files) => {
                let data = &files.get(index).ok_or(ReadFail::Broken)?.1;
                if data.len() as u64 > limit {
                    Err(ReadFail::TooLarge)
                } else {
                    Ok(data.clone())
                }
            }
        }
    }
}

pub struct Bundle<'a> {
    store: Store<'a>,
    entries: Vec<Entry>,
    kind: Kind,
    id: String,
    format: u64,
    requires: Vec<String>,
    manifest: Map<String, Value>,
    caps: Caps,
    unpacked: u64,
    exhausted: bool,
    report: Report,
}

impl<'a> Bundle<'a> {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn format(&self) -> u64 {
        self.format
    }

    pub fn requires(&self) -> &[String] {
        &self.requires
    }

    pub fn manifest(&self) -> &Map<String, Value> {
        &self.manifest
    }

    pub fn mode(&self) -> Mode {
        self.report.mode()
    }

    pub fn report(&mut self) -> &mut Report {
        &mut self.report
    }

    pub fn issues(&self) -> &[Issue] {
        self.report.issues()
    }

    pub fn has_errors(&self) -> bool {
        self.report.has_errors()
    }

    pub fn check_manifest_keys(&mut self, kind_keys: &[&str]) {
        let known: Vec<&str> = MANIFEST_KEYS.iter().chain(kind_keys).copied().collect();
        self.report.check_keys(MANIFEST, "", &self.manifest, &known);
    }

    pub fn id_path(&mut self, dir: &str, id: &str, ext: &str, from: &str) -> Option<String> {
        if !is_id(id) {
            self.report
                .push(Issue::error(IssueCode::IdInvalid, from).name("id", id));
            return None;
        }
        Some(if dir.is_empty() {
            format!("{id}.{ext}")
        } else {
            format!("{dir}/{id}.{ext}")
        })
    }

    pub fn has_file(&self, name: &str) -> bool {
        self.entries.iter().any(|e| !e.dir && e.name == name)
    }

    pub fn bytes(&mut self, name: &str, from: &str) -> Option<Vec<u8>> {
        self.read(name, from, None)
    }

    pub fn json(&mut self, name: &str, from: &str) -> Option<Value> {
        let data = self.read(name, from, Some(JSON_MAX_BYTES))?;
        guarded_json(&data, name, &mut self.report)
    }

    pub fn png(&mut self, name: &str, from: &str) -> Option<Image> {
        let data = self.read(name, from, None)?;
        decode_png(&data, name, self.kind.max_texture_size(), &mut self.report)
    }

    pub fn finish(mut self) -> Vec<Issue> {
        for entry in &self.entries {
            if !entry.dir && entry.safe && !entry.read {
                let name = sanitize(&entry.name);
                self.report.push(
                    Issue::warning(IssueCode::UnreferencedFile, name.as_str()).name("file", &name),
                );
            }
        }
        self.report.into_issues()
    }

    fn read(&mut self, name: &str, from: &str, json: Option<u64>) -> Option<Vec<u8>> {
        if !is_safe_path(name) {
            self.report
                .push(Issue::error(IssueCode::BadPath, from).name("file", name));
            return None;
        }
        let Some(at) = self.entries.iter().position(|e| !e.dir && e.name == name) else {
            self.report
                .push(Issue::error(IssueCode::MissingFile, from).name("file", name));
            return None;
        };
        if self.exhausted {
            return None;
        }

        let first = !self.entries[at].counted;
        self.entries[at].read = true;
        let remaining = if first {
            self.caps.unpacked.saturating_sub(self.unpacked)
        } else {
            u64::MAX
        };
        let (limit, code) = tightest(json, self.caps.per_file, remaining);

        match self.store.read(self.entries[at].index, limit) {
            Ok(data) => {
                if first {
                    self.unpacked += data.len() as u64;
                    self.entries[at].counted = true;
                }
                Some(data)
            }
            Err(ReadFail::TooLarge) => {
                let cap = if code == IssueCode::UnpackedTooLarge {
                    self.exhausted = true;
                    self.caps.unpacked
                } else {
                    limit
                };
                self.report
                    .push(Issue::error(code, name).value("limit", cap as f64));
                None
            }
            Err(ReadFail::Broken) => {
                self.report
                    .push(Issue::error(IssueCode::EntryUnreadable, name).name("file", name));
                None
            }
        }
    }
}

pub fn open<'a>(bytes: &'a [u8], options: &Options) -> Opened<'a> {
    let mut report = Report::new(options.mode);
    let size = bytes.len() as u64;

    if !bytes.starts_with(b"PK") {
        return legacy_single(bytes, options.legacy, report);
    }

    let largest = Kind::ALL
        .iter()
        .map(|k| k.caps().download)
        .chain([options.legacy.download])
        .max()
        .unwrap_or(0);
    if size > largest {
        report.push(too_large(IssueCode::DownloadTooLarge, size, largest));
        return Opened::Invalid(report.into_issues());
    }

    let Some(archive) = Archive::parse(bytes) else {
        report.push(Issue::error(IssueCode::ZipInvalid, ""));
        return Opened::Invalid(report.into_issues());
    };
    let store = Store::Zip(archive);
    let entries = index(&store);

    match entries.iter().position(|e| !e.dir && e.name == MANIFEST) {
        Some(manifest) => open_bundle(Some(size), store, entries, manifest, options, report),
        None => open_legacy(Some(size), store, entries, options.legacy, report),
    }
}

pub fn open_files<'a>(files: &'a [(String, Vec<u8>)], options: &Options) -> Opened<'a> {
    let report = Report::new(options.mode);
    let store = Store::files(files);
    let entries = index(&store);

    match entries.iter().position(|e| !e.dir && e.name == MANIFEST) {
        Some(manifest) => open_bundle(None, store, entries, manifest, options, report),
        None => open_legacy(None, store, entries, options.legacy, report),
    }
}

pub fn open_source<'a>(source: Source<'a>, options: &Options) -> Opened<'a> {
    match source {
        Source::Zip(bytes) => open(bytes, options),
        Source::Files(files) => open_files(files, options),
    }
}

pub fn is_id(text: &str) -> bool {
    (ID_MIN_LEN..=ID_MAX_LEN).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn json_fits(bytes: &[u8], file: &str, report: &mut Report) -> bool {
    let size = bytes.len() as u64;
    if size > JSON_MAX_BYTES {
        report.push(
            Issue::error(IssueCode::JsonTooLarge, file)
                .value("bytes", size as f64)
                .value("limit", JSON_MAX_BYTES as f64),
        );
        return false;
    }
    if too_deep(bytes, JSON_MAX_DEPTH) {
        report
            .push(Issue::error(IssueCode::JsonTooDeep, file).value("limit", JSON_MAX_DEPTH as f64));
        return false;
    }
    true
}

pub fn guarded_json(bytes: &[u8], file: &str, report: &mut Report) -> Option<Value> {
    if !json_fits(bytes, file, report) {
        return None;
    }
    let text = bytes.strip_prefix(&UTF8_BOM[..]).unwrap_or(bytes);
    match serde_json::from_slice(text) {
        Ok(value) => Some(value),
        Err(e) => {
            report.push(
                Issue::error(IssueCode::JsonInvalid, file)
                    .value("line", e.line() as f64)
                    .value("column", e.column() as f64),
            );
            None
        }
    }
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

pub fn decode_png(bytes: &[u8], file: &str, max_size: u32, report: &mut Report) -> Option<Image> {
    let Some((width, height)) = png_size(bytes) else {
        report.push(Issue::error(IssueCode::PngInvalid, file));
        return None;
    };

    let mut fits = true;
    if width > max_size || height > max_size {
        fits = false;
        report.push(
            Issue::error(IssueCode::TextureTooLarge, file)
                .value("width", width as f64)
                .value("height", height as f64)
                .value("limit", max_size as f64),
        );
    }
    if !width.is_power_of_two() || !height.is_power_of_two() {
        fits = false;
        report.push(
            Issue::error(IssueCode::TextureNotPowerOfTwo, file)
                .value("width", width as f64)
                .value("height", height as f64),
        );
    }
    if !fits {
        return None;
    }

    let image = decode_rgba(bytes);
    if image.is_none() {
        report.push(Issue::error(IssueCode::PngInvalid, file));
    }
    image
}

fn decode_rgba(bytes: &[u8]) -> Option<Image> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut data = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut data).ok()?;
    data.truncate(info.buffer_size());

    let rgba = match info.color_type {
        ColorType::Rgba => data,
        ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        ColorType::Grayscale => data.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        ColorType::Indexed => return None,
    };
    let expected = info.width as usize * info.height as usize * 4;
    (rgba.len() == expected).then_some(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn too_deep(bytes: &[u8], max: u32) -> bool {
    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for &b in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

fn is_safe_path(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment
                    .chars()
                    .any(|c| c == '\\' || c == ':' || c.is_control())
        })
}

fn normalize(name: &str) -> Option<String> {
    if name.starts_with(['/', '\\']) || name.contains(['\0', ':']) {
        return None;
    }
    let mut segments: Vec<&str> = Vec::new();
    for segment in name.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            s => segments.push(s),
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

fn index(store: &Store) -> Vec<Entry> {
    (0..store.len())
        .map(|index| {
            let name = store.name(index).to_string();
            Entry {
                index,
                dir: name.ends_with('/'),
                safe: normalize(&name).is_some(),
                name,
                read: false,
                counted: false,
            }
        })
        .collect()
}

fn tightest(json: Option<u64>, per_file: Option<u64>, remaining: u64) -> (u64, IssueCode) {
    [
        json.map(|l| (l, IssueCode::JsonTooLarge)),
        per_file.map(|l| (l, IssueCode::FileTooLarge)),
        Some((remaining, IssueCode::UnpackedTooLarge)),
    ]
    .into_iter()
    .flatten()
    .fold((u64::MAX, IssueCode::UnpackedTooLarge), |best, next| {
        if next.0 < best.0 {
            next
        } else {
            best
        }
    })
}

fn too_large(code: IssueCode, size: u64, limit: u64) -> Issue {
    Issue::error(code, "")
        .value("bytes", size as f64)
        .value("limit", limit as f64)
}

fn too_many_entries(count: usize, limit: u32) -> Issue {
    Issue::error(IssueCode::TooManyEntries, "")
        .value("entries", count as f64)
        .value("limit", limit as f64)
}

fn legacy_single(bytes: &[u8], caps: Caps, mut report: Report) -> Opened<'_> {
    let size = bytes.len() as u64;
    if size > caps.download {
        report.push(too_large(IssueCode::DownloadTooLarge, size, caps.download));
        return Opened::Invalid(report.into_issues());
    }
    Opened::Legacy(Legacy {
        files: vec![(LEGACY_FILE.to_string(), bytes.to_vec())],
        parts: bundle::parts(&[LEGACY_FILE]),
    })
}

fn open_legacy<'a>(
    size: Option<u64>,
    store: Store<'a>,
    entries: Vec<Entry>,
    caps: Caps,
    mut report: Report,
) -> Opened<'a> {
    if let Some(size) = size.filter(|s| *s > caps.download) {
        report.push(too_large(IssueCode::DownloadTooLarge, size, caps.download));
        return Opened::Invalid(report.into_issues());
    }
    if entries.len() > caps.entries as usize {
        report.push(too_many_entries(entries.len(), caps.entries));
        return Opened::Invalid(report.into_issues());
    }

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut unpacked = 0u64;
    for entry in entries.iter().filter(|e| !e.dir) {
        let Some(name) = normalize(&entry.name) else {
            let shown = sanitize(&entry.name);
            report.push(
                Issue::error(IssueCode::UnsafeEntryName, shown.as_str()).name("file", &shown),
            );
            return Opened::Invalid(report.into_issues());
        };
        let (limit, code) = tightest(None, caps.per_file, caps.unpacked - unpacked);
        match store.read(entry.index, limit) {
            Ok(data) => {
                unpacked += data.len() as u64;
                match files.iter_mut().find(|(n, _)| *n == name) {
                    Some(file) => file.1 = data,
                    None => files.push((name, data)),
                }
            }
            Err(ReadFail::TooLarge) => {
                let cap = if code == IssueCode::UnpackedTooLarge {
                    caps.unpacked
                } else {
                    limit
                };
                report.push(Issue::error(code, name.as_str()).value("limit", cap as f64));
                return Opened::Invalid(report.into_issues());
            }
            Err(ReadFail::Broken) => {
                report.push(
                    Issue::error(IssueCode::EntryUnreadable, name.as_str()).name("file", &name),
                );
                return Opened::Invalid(report.into_issues());
            }
        }
    }

    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    let parts = bundle::parts(&names);
    Opened::Legacy(Legacy { files, parts })
}

struct Header {
    format: u64,
    requires: Vec<String>,
    kind: Kind,
    id: String,
}

enum HeaderRead {
    Ok(Header),
    Skip(Skip),
    Invalid,
}

fn read_header(
    manifest: &Map<String, Value>,
    options: &Options,
    report: &mut Report,
) -> HeaderRead {
    let at = |key: &str| path(MANIFEST, key);
    let mut broken = false;

    let format = match manifest.get("format") {
        None => {
            report.push(Issue::error(IssueCode::FormatMissing, at("format")));
            None
        }
        Some(value) => match value.as_u64().filter(|f| *f >= 1) {
            Some(format) if format > FORMAT => return HeaderRead::Skip(Skip::Format(format)),
            Some(format) => Some(format),
            None => {
                let mut issue = Issue::error(IssueCode::FormatInvalid, at("format"));
                if let Some(n) = value.as_f64() {
                    issue = issue.value("format", n);
                }
                report.push(issue);
                None
            }
        },
    };

    let requires: Vec<String> = match manifest.get("requires") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        Some(_) => {
            report.push(Issue::error(IssueCode::RequiresInvalid, at("requires")));
            broken = true;
            Vec::new()
        }
    };

    if format.is_some() {
        let missing: Vec<String> = requires
            .iter()
            .filter(|r| !options.capabilities.contains(&r.as_str()))
            .map(|r| sanitize(r))
            .collect();
        if !missing.is_empty() {
            return HeaderRead::Skip(Skip::Requires(missing));
        }
    }

    let kind = match manifest.get("kind") {
        None => {
            report.push(Issue::error(IssueCode::KindMissing, at("kind")));
            None
        }
        Some(value) => {
            let kind = value.as_str().and_then(Kind::parse);
            if kind.is_none() {
                let mut issue = Issue::error(IssueCode::KindUnknown, at("kind"));
                if let Some(name) = value.as_str() {
                    issue = issue.name("kind", name);
                }
                report.push(issue);
            }
            kind
        }
    };

    let id = match manifest.get("id") {
        None => {
            report.push(Issue::error(IssueCode::IdMissing, at("id")));
            String::new()
        }
        Some(value) => match value.as_str() {
            Some(id) if is_id(id) => id.to_string(),
            name => {
                let mut issue = Issue::error(IssueCode::IdInvalid, at("id"));
                if let Some(name) = name {
                    issue = issue.name("id", name);
                }
                report.push(issue);
                String::new()
            }
        },
    };

    match (format, kind, broken) {
        (Some(format), Some(kind), false) => HeaderRead::Ok(Header {
            format,
            requires,
            kind,
            id,
        }),
        _ => HeaderRead::Invalid,
    }
}

fn open_bundle<'a>(
    size: Option<u64>,
    store: Store<'a>,
    mut entries: Vec<Entry>,
    manifest_at: usize,
    options: &Options,
    mut report: Report,
) -> Opened<'a> {
    let data = match store.read(entries[manifest_at].index, JSON_MAX_BYTES) {
        Ok(data) => data,
        Err(ReadFail::TooLarge) => {
            report.push(
                Issue::error(IssueCode::JsonTooLarge, MANIFEST)
                    .value("limit", JSON_MAX_BYTES as f64),
            );
            return Opened::Invalid(report.into_issues());
        }
        Err(ReadFail::Broken) => {
            report.push(Issue::error(IssueCode::EntryUnreadable, MANIFEST).name("file", MANIFEST));
            return Opened::Invalid(report.into_issues());
        }
    };
    entries[manifest_at].read = true;
    entries[manifest_at].counted = true;

    let Some(value) = guarded_json(&data, MANIFEST, &mut report) else {
        return Opened::Invalid(report.into_issues());
    };
    let Value::Object(manifest) = value else {
        report.push(Issue::error(IssueCode::ManifestNotObject, MANIFEST));
        return Opened::Invalid(report.into_issues());
    };

    let header = match read_header(&manifest, options, &mut report) {
        HeaderRead::Ok(header) => header,
        HeaderRead::Skip(skip) => return Opened::Skipped(skip),
        HeaderRead::Invalid => return Opened::Invalid(report.into_issues()),
    };

    let caps = header.kind.caps();
    let mut capped = false;
    if let Some(size) = size.filter(|s| *s > caps.download) {
        report.push(too_large(IssueCode::DownloadTooLarge, size, caps.download));
        capped = true;
    }
    if entries.len() > caps.entries as usize {
        report.push(too_many_entries(entries.len(), caps.entries));
        capped = true;
    }
    if capped {
        return Opened::Invalid(report.into_issues());
    }

    for entry in entries.iter().filter(|e| !e.dir && !e.safe) {
        let shown = sanitize(&entry.name);
        report.push(Issue::error(IssueCode::UnsafeEntryName, shown.as_str()).name("file", &shown));
    }

    Opened::Bundle(Bundle {
        store,
        entries,
        kind: header.kind,
        id: header.id,
        format: header.format,
        requires: header.requires,
        manifest,
        caps,
        unpacked: data.len() as u64,
        exhausted: false,
        report,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::issues::IssueCode;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    pub(crate) fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        zip_with(entries, CompressionMethod::Stored)
    }

    fn zip_with(entries: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(method);
        for (name, data) in entries {
            if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(data).unwrap();
            }
        }
        writer.finish().unwrap().into_inner()
    }

    fn manifest_zip(manifest: &str) -> Vec<u8> {
        zip_of(&[(MANIFEST, manifest.as_bytes())])
    }

    fn variant(opened: &Opened) -> &'static str {
        match opened {
            Opened::Bundle(_) => "bundle",
            Opened::Legacy(_) => "legacy",
            Opened::Skipped(_) => "skipped",
            Opened::Invalid(_) => "invalid",
        }
    }

    fn bundle(opened: Opened) -> Bundle {
        match opened {
            Opened::Bundle(bundle) => bundle,
            Opened::Invalid(issues) => panic!("expected a bundle, got {issues:?}"),
            other => panic!("expected a bundle, got {}", variant(&other)),
        }
    }

    fn codes(issues: &[Issue]) -> Vec<IssueCode> {
        issues.iter().map(|i| i.code).collect()
    }

    const OK: &str = r#"{"format":1,"kind":"item","id":"sample_wings"}"#;

    #[test]
    fn container_reads_a_minimal_manifest() {
        let bytes = manifest_zip(OK);
        let b = bundle(open(&bytes, &Options::default()));
        assert_eq!(b.kind(), Kind::Item);
        assert_eq!(b.id(), "sample_wings");
        assert_eq!(b.format(), 1);
        assert!(b.requires().is_empty());
        assert!(b.issues().is_empty());
        assert!(b.finish().is_empty());
    }

    #[test]
    fn container_rejects_path_traversal() {
        let bytes = zip_of(&[
            (MANIFEST, OK.as_bytes()),
            ("a.json", b"{}"),
            ("../evil.json", b"{}"),
        ]);
        let mut b = bundle(open(&bytes, &Options::default()));
        assert_eq!(codes(b.issues()), vec![IssueCode::UnsafeEntryName]);
        assert_eq!(
            b.issues()[0].names,
            vec![("file".to_string(), "../evil.json".to_string())]
        );

        for bad in [
            "../evil.json",
            "a/../../evil.json",
            "/etc/passwd",
            "C:/evil.json",
            "a\\..\\evil.json",
            "a//b.json",
            "./a.json",
            "",
        ] {
            assert_eq!(b.bytes(bad, "manifest.json#geometry"), None, "{bad:?}");
            let last = b.issues().last().unwrap();
            assert_eq!(last.code, IssueCode::BadPath, "{bad:?}");
            assert_eq!(last.path, "manifest.json#geometry");
        }
        assert_eq!(
            b.bytes("a.json", "manifest.json#geometry"),
            Some(b"{}".to_vec())
        );
    }

    fn kind_manifest(kind: Kind) -> String {
        format!(r#"{{"format":1,"kind":"{}","id":"a"}}"#, kind.as_str())
    }

    #[test]
    fn container_stops_zip_bombs_at_the_kind_caps() {
        for kind in Kind::ALL {
            let caps = kind.caps();
            let limit = caps.per_file.unwrap_or(caps.unpacked);
            let manifest = kind_manifest(kind);
            let zeros = vec![0u8; limit as usize + 1];
            let bytes = zip_with(
                &[(MANIFEST, manifest.as_bytes()), ("bomb.bin", &zeros)],
                CompressionMethod::Deflated,
            );
            assert!(bytes.len() < 1024 * 1024);
            let mut b = bundle(open(&bytes, &Options::default()));
            assert_eq!(
                b.bytes("bomb.bin", "manifest.json#geometry"),
                None,
                "{kind:?}"
            );
            let expected = if caps.per_file.is_some() {
                IssueCode::FileTooLarge
            } else {
                IssueCode::UnpackedTooLarge
            };
            assert_eq!(codes(b.issues()), vec![expected], "{kind:?}");
        }

        let manifest = kind_manifest(Kind::Nameplate);
        let mib = vec![0u8; 1024 * 1024];
        let names: Vec<String> = (0..9).map(|i| format!("t{i}.bin")).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![(MANIFEST, manifest.as_bytes())];
        entries.extend(names.iter().map(|n| (n.as_str(), &mib[..])));
        let bytes = zip_with(&entries, CompressionMethod::Deflated);
        let mut b = bundle(open(&bytes, &Options::default()));
        let read = names
            .iter()
            .filter(|n| b.bytes(n, "manifest.json#layers").is_some())
            .count();
        assert_eq!(read, 7);
        assert_eq!(codes(b.issues()), vec![IssueCode::UnpackedTooLarge]);
    }

    fn header_at(
        bytes: &[u8],
        signature: &[u8],
        name_len_at: usize,
        fixed: usize,
        name: &str,
    ) -> usize {
        (0..bytes.len())
            .find(|&at| {
                let Some(len) = bytes.get(at + name_len_at..at + name_len_at + 2) else {
                    return false;
                };
                let len = u16::from_le_bytes([len[0], len[1]]) as usize;
                bytes[at..].starts_with(signature)
                    && bytes.get(at + fixed..at + fixed + len) == Some(name.as_bytes())
            })
            .unwrap()
    }

    fn edit_local(bytes: &mut [u8], name: &str, field: usize, value: &[u8]) {
        let local = header_at(bytes, b"PK\x03\x04", 26, 30, name);
        bytes[local + field..local + field + value.len()].copy_from_slice(value);
    }

    fn edit_both(bytes: &mut [u8], name: &str, field: usize, value: &[u8]) {
        edit_local(bytes, name, field, value);
        let central = header_at(bytes, b"PK\x01\x02", 28, 46, name);
        bytes[central + field + 2..central + field + 2 + value.len()].copy_from_slice(value);
    }

    fn unreadable(bytes: &[u8], name: &str) {
        let mut b = bundle(open(bytes, &Options::default()));
        assert_eq!(b.bytes(name, "manifest.json#a"), None);
        assert_eq!(codes(b.issues()), vec![IssueCode::EntryUnreadable]);
    }

    const SIZE: usize = 22;

    #[test]
    fn container_zip_does_not_trust_declared_sizes() {
        let caps = Kind::Item.caps();
        let zeros = vec![0u8; caps.unpacked as usize * 2];
        let base = zip_with(
            &[
                (MANIFEST, OK.as_bytes()),
                ("bomb.bin", &zeros),
                ("small.bin", b"tiny"),
            ],
            CompressionMethod::Deflated,
        );

        let mut bytes = base.clone();
        edit_both(&mut bytes, "bomb.bin", SIZE, &5u32.to_le_bytes());
        unreadable(&bytes, "bomb.bin");

        let mut bytes = base.clone();
        edit_both(&mut bytes, "bomb.bin", SIZE, &1024u32.to_le_bytes());
        unreadable(&bytes, "bomb.bin");

        let mut bytes = base.clone();
        let over = caps.unpacked as u32 + 1;
        edit_both(&mut bytes, "small.bin", SIZE, &over.to_le_bytes());
        let mut b = bundle(open(&bytes, &Options::default()));
        assert_eq!(b.bytes("small.bin", "manifest.json#a"), None);
        assert_eq!(codes(b.issues()), vec![IssueCode::UnpackedTooLarge]);

        let mut b = bundle(open(&base, &Options::default()));
        assert_eq!(b.json("bomb.bin", "manifest.json#a"), None);
        assert_eq!(b.bytes("bomb.bin", "manifest.json#a"), None);
        assert_eq!(
            codes(b.issues()),
            vec![IssueCode::JsonTooLarge, IssueCode::UnpackedTooLarge]
        );
    }
}
