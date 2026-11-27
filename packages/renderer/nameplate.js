export const defaultWasmUrl = new URL("./plusrenderer.wasm", import.meta.url);

export const OUT = Object.freeze({
    TAG_INTS: 24,
    TAG_ASSET: 0,
    TAG_VARIANT: 1,
    TAG_FLAGS: 2,
    TAG_DETAIL: 3,
    TAG_LINE_COUNT: 4,
    TAG_NAME_LINE: 5,
    TAG_ALIGN: 6,
    TAG_NAME_X: 7,
    TAG_NAME_WIDTH: 8,
    TAG_PREFIX_WIDTH: 9,
    TAG_LINE_WIDTH_0: 10,
    TAG_BG_X0: 13,
    TAG_BG_Y0: 14,
    TAG_BG_X1: 15,
    TAG_BG_Y1: 16,
    TAG_EXTERNAL_COLOR: 17,
    TAG_SNEAK_ALPHA: 18,
    TAG_GLYPH_OFFSET: 19,
    TAG_GLYPH_COUNT: 20,
    TAG_COLOR_OFFSET: 21,
    TAG_COLOR_COUNT: 22,
    GLYPH_FLOATS: 2,
    SIGNAL_FLOATS: 24,
    SIGNAL_EVENTS: 23,
    OUT_TAGS: 0,
    OUT_WORDS: 1,
    OUT_STRIP_LAYOUT: 2,
    OUT_HEADER: 3,
    DIR_WORDS: 3,
    DIR_OFFSET: 0,
    DIR_LEN: 1,
    DIR_STATUS: 2,
    STATUS_OK: 0,
    STATUS_VANILLA: 1,
    STATUS_UNKNOWN_ASSET: 2,
    STATUS_BAD_ROW: 3,
    STATUS_DISABLED: 4,
    STATUS_NO_ROOM: 5,
    TAG_HEAD: 8,
    HEAD_PLATE_DX: 0,
    HEAD_PLATE_DY: 1,
    HEAD_OWNS_BACKGROUND: 2,
    HEAD_TEXT_MODE: 3,
    HEAD_OPS: 4,
    HEAD_MATERIALS: 5,
    HEAD_MATERIAL_WORDS: 6,
    HEAD_EVENTS: 7,
    OP_WORDS: 8,
    OP_X0: 0,
    OP_Y0: 1,
    OP_X1: 2,
    OP_Y1: 3,
    OP_ARGB: 4,
    OP_BAND_LAYER_KIND: 5,
    OP_AUX0: 6,
    OP_AUX1: 7,
    OP_KIND_RECT: 0,
    OP_KIND_STRIP: 1,
    OP_KIND_IMAGE: 2,
    AUX_LOW: 0xffff,
    AUX_AXIS_Y: 1 << 16,
    AUX_MIRROR: 1 << 17,
    AUX_ADDITIVE: 1 << 18,
    BAND_BEHIND: 0,
    BAND_BACKGROUND: 1,
    BAND_OVER_BACKGROUND: 2,
    BAND_OVER_TEXT: 3,
    BAND_BADGE: 4,
    MAT_SHADER: 0,
    MAT_BAND_LAYER: 1,
    MAT_RUN: 2,
    MAT_AREA_X0: 3,
    MAT_AREA_Y0: 4,
    MAT_AREA_X1: 5,
    MAT_AREA_Y1: 6,
    MAT_CLIP_COUNT: 7,
    MAT_UNIFORM_WORDS: 8,
    MAT_FIXED: 9,
    CLIP_WORDS: 4,
    MAT_MIRRORED: 1 << 16,
    MAT_OPACITY_SHIFT: 24,
    TEXT_COLOR: 0,
    TEXT_SHADOW: 1,
    TEXT_SHADOW_USE: 2,
    TEXT_OUTLINE: 3,
    TEXT_LETTER_COUNT: 4,
    TEXT_FIXED: 5,
    LETTER_WORDS: 7,
    LETTER_DX: 0,
    LETTER_DY: 1,
    LETTER_ARGB: 2,
    LETTER_SHADOW: 3,
    LETTER_OPACITY: 4,
    LETTER_SCALE_ROT: 5,
    LETTER_OUTLINE: 6,
    BADGE_WORDS: 5,
    BADGE_X: 0,
    BADGE_Y: 1,
    BADGE_OPACITY: 2,
    BADGE_TINT: 3,
    BADGE_DECORATION: 4,
    DECORATION_KIND_SHIFT: 24,
    EVENT_WORDS: 2,
    EVENT_CODE: 0,
    EVENT_LAYER: 1,
});

export const FLAG_DISPLAY = 1 << 0;
export const FLAG_OWN = 1 << 1;
export const FLAG_ANIMATED_NAMES_OFF = 1 << 2;
export const FLAG_EFFECTS_CPU = 1 << 3;
export const FLAG_EFFECTS_OFF = 1 << 4;
export const FLAG_EXTERNAL_COLOR = 1 << 5;
export const FLAG_BADGE = 1 << 6;

export const TEXT_MODE_VANILLA = 0;
export const TEXT_MODE_MOTION = 1;
export const TEXT_MODE_OWNED = 2;

export const ALIGN_LEFT = 0;
export const ALIGN_RIGHT = 1;
export const ALIGN_CENTER = 2;

export const DETAIL_FULL = 0;
export const DETAIL_REDUCED = 1;
export const DETAIL_MINIMAL = 2;

export const MOTION_FULL = 0;
export const MOTION_REDUCED = 1;
export const MOTION_OFF = 2;

export const GLYPH_HEIGHT = 8;

export const SIGNALS = Object.freeze([
    "is_sneaking", "is_sprinting", "is_swimming", "is_in_water", "is_in_rain", "is_in_snow", "is_on_fire",
    "is_hurt", "is_gliding", "is_flying", "is_falling", "is_on_ground", "is_riding", "is_sleeping",
    "is_emoting", "is_in_combat", "is_moving", "ground_speed", "vertical_speed", "time_of_day",
    "biome_temperature", "is_in_ocean", "dimension",
]);

const MODES = { client: 0, authoring: 1 };
const DIALECTS = ["core_330", "core_330_ubo", "legacy_150", "compat_130", "es_300", "sksl"];
const LOADED = ["asset", "draft", "skipped", "invalid"];

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function wearerId(wearer) {
    return BigInt.asUintN(64, BigInt(wearer));
}

export function startClock() {
    const wall = Date.now() / 1000;
    const start = performance.now();
    return () => wall + (performance.now() - start) / 1000;
}

const CRC_TABLE = (() => {
    const table = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
        let c = n;
        for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
        table[n] = c >>> 0;
    }
    return table;
})();

function crc32(bytes) {
    let crc = 0xffffffff;
    for (let i = 0; i < bytes.length; i++) crc = CRC_TABLE[(crc ^ bytes[i]) & 0xff] ^ (crc >>> 8);
    return (crc ^ 0xffffffff) >>> 0;
}

function fileEntries(files) {
    const entries = files instanceof Map ? [...files] : Array.isArray(files) ? files : Object.entries(files);
    return entries
        .map(([name, data]) => [name.replace(/\\/g, "/"), typeof data === "string" ? encoder.encode(data) : toBytes(data)])
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

function toBytes(data) {
    if (data instanceof Uint8Array) return data;
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    return new Uint8Array(data);
}

export function storeZip(files) {
    const entries = fileEntries(files).map(([name, data]) => ({ name: encoder.encode(name), data, crc: crc32(data) }));
    let size = 22;
    for (const e of entries) size += 30 + e.name.length + e.data.length + 46 + e.name.length;
    const zip = new Uint8Array(size);
    const view = new DataView(zip.buffer);
    let at = 0;
    for (const e of entries) {
        e.offset = at;
        view.setUint32(at, 0x04034b50, true);
        view.setUint16(at + 4, 20, true);
        view.setUint16(at + 6, 0x0800, true);
        view.setUint16(at + 8, 0, true);
        view.setUint32(at + 10, 0x00210000, true);
        view.setUint32(at + 14, e.crc, true);
        view.setUint32(at + 18, e.data.length, true);
        view.setUint32(at + 22, e.data.length, true);
        view.setUint16(at + 26, e.name.length, true);
        view.setUint16(at + 28, 0, true);
        zip.set(e.name, at + 30);
        zip.set(e.data, at + 30 + e.name.length);
        at += 30 + e.name.length + e.data.length;
    }
    const central = at;
    for (const e of entries) {
        view.setUint32(at, 0x02014b50, true);
        view.setUint16(at + 4, 20, true);
        view.setUint16(at + 6, 20, true);
        view.setUint16(at + 8, 0x0800, true);
        view.setUint16(at + 10, 0, true);
        view.setUint32(at + 12, 0x00210000, true);
        view.setUint32(at + 16, e.crc, true);
        view.setUint32(at + 20, e.data.length, true);
        view.setUint32(at + 24, e.data.length, true);
        view.setUint16(at + 28, e.name.length, true);
        view.setUint32(at + 30, 0, true);
        view.setUint32(at + 34, 0, true);
        view.setUint32(at + 38, 0, true);
        view.setUint32(at + 42, e.offset, true);
        zip.set(e.name, at + 46);
        at += 46 + e.name.length;
    }
    view.setUint32(at, 0x06054b50, true);
    view.setUint16(at + 8, entries.length, true);
    view.setUint16(at + 10, entries.length, true);
    view.setUint32(at + 12, at - central, true);
    view.setUint32(at + 16, central, true);
    return zip;
}

export function tagRow({
    asset,
    variant = 0,
    flags = 0,
    detail = DETAIL_FULL,
    lines,
    nameLine = 0,
    align = ALIGN_LEFT,
    nameX = 0,
    nameWidth,
    prefixWidth = 0,
    background,
    externalColor = 0,
    sneakAlpha = 255,
    glyphOffset = 0,
    glyphCount = 0,
    colorOffset = 0,
    colorCount = 0,
}, row = new Int32Array(OUT.TAG_INTS)) {
    const widths = lines ?? [nameWidth];
    const name = nameWidth ?? widths[nameLine];
    const bg = background ?? [-1, -1, widths[0], GLYPH_HEIGHT + 1];
    row.fill(0);
    row[OUT.TAG_ASSET] = asset;
    row[OUT.TAG_VARIANT] = variant;
    row[OUT.TAG_FLAGS] = flags;
    row[OUT.TAG_DETAIL] = detail;
    row[OUT.TAG_LINE_COUNT] = widths.length;
    row[OUT.TAG_NAME_LINE] = nameLine;
    row[OUT.TAG_ALIGN] = align;
    row[OUT.TAG_NAME_X] = nameX;
    row[OUT.TAG_NAME_WIDTH] = name;
    row[OUT.TAG_PREFIX_WIDTH] = prefixWidth;
    for (let i = 0; i < Math.min(widths.length, 3); i++) row[OUT.TAG_LINE_WIDTH_0 + i] = widths[i];
    row[OUT.TAG_BG_X0] = bg[0];
    row[OUT.TAG_BG_Y0] = bg[1];
    row[OUT.TAG_BG_X1] = bg[2];
    row[OUT.TAG_BG_Y1] = bg[3];
    row[OUT.TAG_EXTERNAL_COLOR] = externalColor | 0;
    row[OUT.TAG_SNEAK_ALPHA] = sneakAlpha;
    row[OUT.TAG_GLYPH_OFFSET] = glyphOffset;
    row[OUT.TAG_GLYPH_COUNT] = glyphCount;
    row[OUT.TAG_COLOR_OFFSET] = colorOffset;
    row[OUT.TAG_COLOR_COUNT] = colorCount;
    return row;
}

export async function instantiate(source = defaultWasmUrl) {
    let bytes = source;
    if (source instanceof URL || typeof source === "string") {
        bytes = await (await fetch(source)).arrayBuffer();
    }
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return instance.exports;
}

export class NameplateEngine {
    static async create(source = defaultWasmUrl) {
        return new NameplateEngine(await instantiate(source));
    }

    constructor(wasm) {
        this.wasm = wasm;
        this.stoppedRaster = (wasm?.nameplate_raster_ex?.length ?? 0) >= 23;
        this.observedRaster = (wasm?.nameplate_raster_ex?.length ?? 0) >= 24;
    }

    memory() {
        return this.wasm.memory.buffer;
    }

    put(bytes, nonNull = false) {
        const size = Math.max(bytes.length, nonNull ? 1 : 0);
        if (size === 0) return 0;
        const ptr = this.wasm.alloc(size);
        new Uint8Array(this.memory(), ptr, bytes.length).set(bytes);
        return ptr;
    }

    text(ptr, len) {
        return decoder.decode(new Uint8Array(this.memory(), ptr, len));
    }

    error() {
        return this.text(this.wasm.nameplate_error(), this.wasm.nameplate_error_len());
    }

    json(len) {
        if (len < 0) throw new Error(this.error());
        return JSON.parse(this.text(this.wasm.nameplate_text(), len));
    }

    async load(source, options = {}) {
        return this.loadSync(source, options);
    }

    loadSync(source, { mode = "client", capabilities, identity } = {}) {
        const zip = ArrayBuffer.isView(source) || source instanceof ArrayBuffer ? toBytes(source) : storeZip(source);
        const modeValue = MODES[mode] ?? mode;
        const caps = capabilities ? encoder.encode(capabilities.join("\n")) : null;
        const wasm = this.wasm;
        const zipPtr = this.put(zip);
        const capsPtr = caps ? this.put(caps, true) : 0;
        let handle;
        try {
            handle = wasm.nameplate_parse(zipPtr, zip.length, modeValue, capsPtr, caps ? caps.length : 0,
                identity === undefined || identity === null ? 0n : wearerId(identity));
        } finally {
            if (zipPtr) wasm.dealloc(zipPtr, zip.length);
            if (capsPtr) wasm.dealloc(capsPtr, Math.max(caps.length, 1));
        }
        if (!handle) throw new Error(this.error());
        return new NameplateAsset(this, handle);
    }

    constants() {
        return (this.constantsCache ??= this.json(this.wasm.nameplate_constants_json()));
    }

    checkMolang(source, context = "channel") {
        const index = typeof context === "number" ? context : MOLANG_CONTEXTS.indexOf(context);
        if (index < 0) throw new Error(`nameplate: no MoLang context ${context}`);
        const bytes = encoder.encode(source);
        const ptr = this.put(bytes);
        try {
            return this.json(this.wasm.nameplate_check_molang_json(ptr, bytes.length, index));
        } finally {
            if (ptr) this.wasm.dealloc(ptr, bytes.length);
        }
    }

    nameplates() {
        return new Nameplates(this);
    }

    rasterTextures() {
        return (this.sharedTextures ??= new RasterTextures(this));
    }

    setRasterTexture(name, width, height, rgba) {
        this.rasterTextures().set(name, width, height, rgba);
    }
}

export const MOLANG_CONTEXTS = Object.freeze(["channel", "slot_color", "field", "text", "material"]);

export const RASTER_IDLE = 0;
export const RASTER_WHILE = 1;
export const RASTER_ONE_SHOT = 2;

export const RASTER_BAND_ALL = 0;
export const RASTER_BAND_UNDER_TEXT = 1;
export const RASTER_BAND_OVER_TEXT = 2;
const RASTER_BANDS = Object.freeze({ all: RASTER_BAND_ALL, under_text: RASTER_BAND_UNDER_TEXT, over_text: RASTER_BAND_OVER_TEXT });

export function rasterBand(band) {
    if (band === undefined || band === null) return RASTER_BAND_ALL;
    const value = typeof band === "string" ? RASTER_BANDS[band] : band;
    if (value !== RASTER_BAND_ALL && value !== RASTER_BAND_UNDER_TEXT && value !== RASTER_BAND_OVER_TEXT) {
        throw new Error(`nameplate: no raster band ${band}`);
    }
    return value;
}

export function layerBits(layers) {
    if (layers === undefined || layers === null) return 0;
    if (typeof layers === "number") return layers >>> 0;
    let bits = 0;
    for (const slot of layers) {
        if (!Number.isInteger(slot) || slot < 0 || slot > 31) throw new Error(`nameplate: no layer slot ${slot}`);
        bits |= 1 << slot;
    }
    return bits >>> 0;
}

export const RASTER_INFO = Object.freeze({
    WORDS: 180,
    WIDTH: 0,
    HEIGHT: 1,
    X: 2,
    Y: 3,
    PLATE_DX: 4,
    PLATE_DY: 5,
    BADGE_SHOWN: 6,
    BADGE_X: 7,
    BADGE_Y: 8,
    BADGE_OPACITY: 9,
    TEXT_MODE: 10,
    LETTER_COUNT: 11,
    LETTERS: 12,
});

const HELD_FOREVER = 0xffffffff;
const TEXT_MODES = ["vanilla", "motion", "owned"];
const RASTER_DEPTHS = Object.freeze(["behind", "background", "over_background", "over_text"]);
const RASTER_BLENDS = Object.freeze(["alpha", "additive"]);
const RASTER_COMPOSITES = Object.freeze(["source-over", "lighter"]);

function signalBits(reactions) {
    if (reactions === undefined || reactions === null) return 0;
    if (typeof reactions === "number") return reactions >>> 0;
    let bits = 0;
    for (const entry of reactions) {
        const index = typeof entry === "number" ? entry : SIGNALS.indexOf(entry);
        if (index < 0 || index > 31) throw new Error(`nameplate: no signal ${entry}`);
        bits |= 1 << index;
    }
    return bits >>> 0;
}

function signalIndex(signal) {
    const index = typeof signal === "number" ? signal : SIGNALS.indexOf(signal);
    if (!Number.isInteger(index) || index < 0 || index > 31) throw new Error(`nameplate: no signal ${signal}`);
    return index;
}

function stoppedTriples(stopped) {
    const words = [];
    for (const entry of stopped) {
        const [signal, heldFor, ago] = Array.isArray(entry) ? entry : [entry.signal, entry.heldFor, entry.ago];
        words.push(signalIndex(signal), heldFor >>> 0, ago >>> 0);
    }
    return Uint32Array.from(words);
}

function glyphFloats(glyphs) {
    if (!glyphs) return EMPTY_FLOATS;
    if (glyphs instanceof Float32Array) return glyphs;
    return Float32Array.from(glyphs.flat ? glyphs.flat() : glyphs);
}

export class RasterTextures {
    constructor(engine) {
        this.engine = engine;
        this.handle = engine.wasm.nameplate_textures_new();
    }

    live() {
        if (!this.handle) throw new Error("nameplate: the raster textures were freed");
        return this.handle;
    }

    set(name, width, height, rgba) {
        const handle = this.live();
        const engine = this.engine;
        const wasm = engine.wasm;
        const id = encoder.encode(name);
        const bytes = rgba ? toBytes(rgba) : new Uint8Array(0);
        const idPtr = engine.put(id);
        const ptr = engine.put(bytes);
        try {
            if (wasm.nameplate_textures_set(handle, idPtr, id.length, width, height, ptr, bytes.length) < 0) {
                throw new Error(engine.error());
            }
        } finally {
            if (idPtr) wasm.dealloc(idPtr, id.length);
            if (ptr) wasm.dealloc(ptr, bytes.length);
        }
    }

    remove(name) {
        this.set(name, 0, 0, null);
    }

    free() {
        if (!this.handle) return;
        this.engine.wasm.nameplate_textures_free(this.handle);
        this.handle = 0;
        if (this.engine.sharedTextures === this) this.engine.sharedTextures = null;
    }
}

export class NameplateAsset {
    constructor(engine, handle) {
        this.engine = engine;
        this.handle = handle;
        this.status = engine.wasm.nameplate_asset_status(handle);
        this.kind = LOADED[this.status] ?? "invalid";
        const report = engine.json(engine.wasm.nameplate_asset_issues_json(handle));
        this.issues = report.issues ?? [];
        this.skip = report.skip ?? null;
        const meta = this.usable ? engine.json(engine.wasm.nameplate_meta_json(handle)) : {};
        this.meta = { ...meta, status: this.kind, skip: this.skip };
    }

    get usable() {
        return this.kind === "asset" || this.kind === "draft";
    }

    live() {
        if (!this.handle) throw new Error("nameplate: the asset was freed");
        return this.handle;
    }

    shaders(dialect = "es_300") {
        const value = typeof dialect === "number" ? dialect : DIALECTS.indexOf(dialect);
        if (value < 0) throw new Error(`nameplate: no shader dialect ${dialect}`);
        return this.engine.json(this.engine.wasm.nameplate_shaders_json(this.live(), value));
    }

    analyze() {
        return this.engine.json(this.engine.wasm.nameplate_analyze_json(this.live()));
    }

    dialectIssues(dialect = "es_300") {
        const value = typeof dialect === "number" ? dialect : DIALECTS.indexOf(dialect);
        if (value < 0) throw new Error(`nameplate: no shader dialect ${dialect}`);
        return this.engine.json(this.engine.wasm.nameplate_dialect_issues_json(this.live(), value));
    }

    variantIndex(variant) {
        if (typeof variant === "number") return variant;
        const index = (this.meta.variants ?? []).findIndex((entry) => entry.key === variant);
        if (index < 0) throw new Error(`nameplate: no variant ${variant}`);
        return index;
    }

    paramCurve(layer, param, variant = 0, count = 1) {
        const handle = this.live();
        const engine = this.engine;
        const wasm = engine.wasm;
        const layerBytes = encoder.encode(layer);
        const paramBytes = encoder.encode(param);
        const frames = Math.max(0, count >>> 0);
        const outBytes = Math.max(frames * 8, 8);
        const layerPtr = engine.put(layerBytes);
        const paramPtr = engine.put(paramBytes);
        const out = wasm.alloc(outBytes);
        try {
            const len = wasm.nameplate_param_curve(handle, layerPtr, layerBytes.length, paramPtr, paramBytes.length,
                this.variantIndex(variant), frames, out, frames);
            if (len < 0) throw new Error(engine.error());
            return Array.from(new Float64Array(engine.memory(), out, len));
        } finally {
            wasm.dealloc(out, outBytes);
            if (paramPtr) wasm.dealloc(paramPtr, paramBytes.length);
            if (layerPtr) wasm.dealloc(layerPtr, layerBytes.length);
        }
    }

    oneShotIndex(shot) {
        if (typeof shot === "number") return shot;
        const index = (this.meta.animations?.one_shots ?? []).findIndex((entry) => entry.name === shot);
        if (index < 0) throw new Error(`nameplate: no one-shot ${shot}`);
        return index;
    }

    raster(row, frame = 0, options = {}) {
        return this.rasterRun(row, frame, options, false);
    }

    rasterLayers(row, frame = 0, options = {}) {
        return this.rasterRun(row, frame, options, true);
    }

    rasterRun(row, frame, {
        scale = 1,
        seed = 0n,
        state = RASTER_IDLE,
        index = 0,
        hidden = 0,
        omit = 0,
        band = RASTER_BAND_ALL,
        reactions,
        heldFor,
        observedFor,
        stopped = [],
        oneShots = [],
        glyphs,
        detail,
        motion = MOTION_FULL,
        textures = this.engine.sharedTextures,
    }, perLayer) {
        const handle = this.live();
        const engine = this.engine;
        const wasm = engine.wasm;
        if (textures && textures.engine?.wasm !== wasm) {
            throw new Error("nameplate: the raster textures belong to another wasm instance");
        }
        if (perLayer && !wasm.nameplate_raster_images) {
            throw new Error("nameplate: this wasm build has no per-layer raster");
        }
        const store = textures ? textures.live() : 0;
        const bandValue = rasterBand(band);
        const hiddenBits = layerBits(hidden);
        const omitBits = layerBits(omit);
        const tag = Int32Array.from(row instanceof Int32Array ? row : tagRow(row));
        if (detail !== undefined) tag[OUT.TAG_DETAIL] = detail;
        let bits = signalBits(reactions);
        const pairs = [];
        for (const shot of oneShots) {
            const [which, age] = Array.isArray(shot) ? shot : [shot.index ?? shot.name, shot.age];
            pairs.push(this.oneShotIndex(which), age >>> 0);
        }
        if (state === RASTER_WHILE) bits = (bits | (1 << index)) >>> 0;
        if (state === RASTER_ONE_SHOT) pairs.push(index, frame >>> 0);
        const shots = Uint32Array.from(pairs);
        const ended = stoppedTriples(stopped);
        const takesStopped = engine.stoppedRaster;
        if (ended.length && !takesStopped) {
            throw new Error("nameplate: this wasm build has no stopped-signal raster");
        }
        const floats = glyphFloats(glyphs);
        const held = heldFor === undefined || heldFor === null ? HELD_FOREVER : heldFor >>> 0;
        const observed = observedFor === undefined || observedFor === null ? HELD_FOREVER : observedFor >>> 0;
        const observedArgs = engine.observedRaster ? [observed] : [];
        const rowPtr = engine.put(new Uint8Array(tag.buffer, tag.byteOffset, OUT.TAG_INTS * 4));
        const shotPtr = engine.put(new Uint8Array(shots.buffer, shots.byteOffset, shots.byteLength));
        const endedPtr = engine.put(new Uint8Array(ended.buffer, ended.byteOffset, ended.byteLength));
        const glyphPtr = engine.put(new Uint8Array(floats.buffer, floats.byteOffset, floats.byteLength));
        const endedArgs = takesStopped ? [endedPtr, ended.length] : [];
        const args = [handle, store, rowPtr, OUT.TAG_INTS, frame >>> 0, scale, wearerId(seed), bits, held,
            ...observedArgs, shotPtr, shots.length, ...endedArgs, glyphPtr, floats.length, motion, hiddenBits, omitBits, bandValue];
        try {
            if (!wasm.nameplate_raster_images) return this.rasterTwice(args);
            const images = wasm.nameplate_raster_images(...args, perLayer ? 1 : 0);
            if (!images) throw new Error(engine.error());
            try {
                return this.readImages(images, perLayer);
            } finally {
                wasm.nameplate_raster_images_free(images);
            }
        } finally {
            if (glyphPtr) wasm.dealloc(glyphPtr, floats.byteLength);
            if (endedPtr) wasm.dealloc(endedPtr, ended.byteLength);
            if (shotPtr) wasm.dealloc(shotPtr, shots.byteLength);
            wasm.dealloc(rowPtr, OUT.TAG_INTS * 4);
        }
    }

    readImages(images, perLayer) {
        const engine = this.engine;
        const wasm = engine.wasm;
        const words = Int32Array.from(new Int32Array(engine.memory(), wasm.nameplate_raster_images_info(images), RASTER_INFO.WORDS));
        const info = readRasterInfo(words);
        const count = wasm.nameplate_raster_images_count(images) >>> 0;
        const list = [];
        for (let i = 0; i < count; i++) {
            const [slot, depth, blend, bytes] = new Int32Array(engine.memory(), wasm.nameplate_raster_image_meta(images, i), 4);
            const data = new Uint8ClampedArray(bytes);
            data.set(new Uint8Array(engine.memory(), wasm.nameplate_raster_image_rgba(images, i), bytes));
            const pass = wasm.nameplate_raster_image_pass ? wasm.nameplate_raster_image_pass(images, i) : -1;
            if (pass >= 0 && list.length) list[list.length - 1].passes.push(data);
            else list.push({ slot, depth, blend, data, passes: [] });
        }
        if (!perLayer) return { ...info, data: list[0]?.data ?? new Uint8ClampedArray(0) };
        const layers = this.meta.layers ?? [];
        return {
            ...info,
            layers: list.map(({ slot, depth, blend, data, passes }) => ({
                slot,
                id: layers[slot]?.id ?? null,
                depth: RASTER_DEPTHS[depth],
                band: RASTER_DEPTHS[depth] === "over_text" ? "over_text" : "under_text",
                blend: RASTER_BLENDS[blend],
                composite: RASTER_COMPOSITES[blend],
                data,
                passes: passes.length ? passes : [data],
            })),
        };
    }

    rasterTwice(args) {
        const engine = this.engine;
        const wasm = engine.wasm;
        const infoBytes = RASTER_INFO.WORDS * 4;
        const info = wasm.alloc(infoBytes);
        let pixels = 0;
        let size = 0;
        try {
            const call = (out, len) => wasm.nameplate_raster_ex(...args, info, RASTER_INFO.WORDS, out, len);
            size = call(0, 0);
            if (size < 0) throw new Error(engine.error());
            pixels = wasm.alloc(Math.max(size, 1));
            if (call(pixels, size) < 0) throw new Error(engine.error());
            const words = Int32Array.from(new Int32Array(engine.memory(), info, RASTER_INFO.WORDS));
            const data = new Uint8ClampedArray(size);
            data.set(new Uint8Array(engine.memory(), pixels, size));
            return { ...readRasterInfo(words), data };
        } finally {
            if (pixels) wasm.dealloc(pixels, Math.max(size, 1));
            wasm.dealloc(info, infoBytes);
        }
    }

    textureCount() {
        if (!this.usable) return 0;
        const count = this.engine.wasm.nameplate_texture_count(this.live());
        if (count < 0) throw new Error(this.engine.error());
        return count;
    }

    texture(index) {
        const handle = this.live();
        const engine = this.engine;
        const wasm = engine.wasm;
        const info = wasm.alloc(8);
        let pixels = 0;
        let size = 0;
        try {
            size = wasm.nameplate_texture(handle, index >>> 0, info, 0, 0);
            if (size < 0) throw new Error(engine.error());
            pixels = wasm.alloc(Math.max(size, 1));
            if (wasm.nameplate_texture(handle, index >>> 0, info, pixels, size) < 0) throw new Error(engine.error());
            const [width, height] = new Int32Array(engine.memory(), info, 2);
            const rgba = new Uint8Array(size);
            rgba.set(new Uint8Array(engine.memory(), pixels, size));
            return { width, height, rgba };
        } finally {
            if (pixels) wasm.dealloc(pixels, Math.max(size, 1));
            wasm.dealloc(info, 8);
        }
    }

    textures() {
        const list = [];
        for (let i = 0, count = this.textureCount(); i < count; i++) list.push(this.texture(i));
        return list;
    }

    free() {
        if (!this.handle) return;
        this.engine.wasm.nameplate_asset_free(this.handle);
        this.handle = 0;
    }
}

class Region {
    constructor(engine, Type) {
        this.engine = engine;
        this.Type = Type;
        this.ptr = 0;
        this.capacity = 0;
        this.view = null;
        this.buffer = null;
    }

    reserve(count) {
        if (count > this.capacity) {
            const wasm = this.engine.wasm;
            if (this.ptr) wasm.dealloc(this.ptr, this.capacity * this.Type.BYTES_PER_ELEMENT);
            let capacity = Math.max(this.capacity, 16);
            while (capacity < count) capacity *= 2;
            this.ptr = wasm.alloc(capacity * this.Type.BYTES_PER_ELEMENT);
            this.capacity = capacity;
            this.buffer = null;
        }
        return this.current();
    }

    current() {
        const buffer = this.engine.memory();
        if (this.buffer !== buffer) {
            this.buffer = buffer;
            this.view = this.capacity ? new this.Type(buffer, this.ptr, this.capacity) : new this.Type(0);
        }
        return this.view;
    }

    write(source, count) {
        const view = this.reserve(count);
        if (count === 0) return view;
        if (source instanceof this.Type) view.set(count === source.length ? source : source.subarray(0, count));
        else for (let i = 0; i < count; i++) view[i] = source[i];
        return view;
    }

    free() {
        if (this.ptr) this.engine.wasm.dealloc(this.ptr, this.capacity * this.Type.BYTES_PER_ELEMENT);
        this.ptr = 0;
        this.capacity = 0;
        this.view = null;
        this.buffer = null;
    }
}

export class Nameplates {
    constructor(engine) {
        this.engine = engine;
        this.wasm = engine.wasm;
        this.handle = this.wasm.nameplates_new();
        this.wearers = new Region(engine, BigUint64Array);
        this.tags = new Region(engine, Int32Array);
        this.signals = new Region(engine, Float32Array);
        this.glyphs = new Region(engine, Float32Array);
        this.colors = new Region(engine, Int32Array);
        this.out = new Region(engine, Int32Array);
        this.customSlots = new Region(engine, Uint32Array);
        this.customRgb = new Region(engine, Uint32Array);
        this.lengthSlot = new Region(engine, Uint32Array);
        this.lengthSlot.reserve(1);
        this.texelBuffer = null;
        this.texelViews = new Map();
    }

    live() {
        if (!this.handle) throw new Error("nameplate: the nameplates were freed");
        return this.handle;
    }

    check(code) {
        if (code >= 0) return code;
        const handle = this.live();
        const ptr = this.wasm.nameplates_error(handle);
        const len = this.wasm.nameplates_error_len(handle);
        throw new Error(this.engine.text(ptr, len));
    }

    addAsset(asset) {
        const handle = this.live();
        if (asset?.engine?.wasm !== this.wasm) throw new Error("nameplate: the asset was loaded by another wasm instance");
        if (!asset.handle) throw new Error("nameplate: the asset was freed");
        return this.check(this.wasm.nameplates_add_asset(handle, asset.handle));
    }

    removeAsset(id) {
        this.wasm.nameplates_remove_asset(this.live(), id);
    }

    setMotion(motion) {
        this.check(this.wasm.nameplates_set_motion(this.live(), motion));
    }

    setVanillaTexture(name, width, height, rgba) {
        const handle = this.live();
        const engine = this.engine;
        const id = encoder.encode(name);
        const bytes = rgba ? toBytes(rgba) : new Uint8Array(0);
        const idPtr = engine.put(id);
        const ptr = engine.put(bytes);
        try {
            this.check(this.wasm.nameplates_set_vanilla_texture(handle, idPtr, id.length, width, height, ptr, bytes.length));
        } finally {
            if (idPtr) this.wasm.dealloc(idPtr, id.length);
            if (ptr) this.wasm.dealloc(ptr, bytes.length);
        }
    }

    forgetWearer(wearer) {
        return this.wasm.nameplates_forget_wearer(this.live(), wearerId(wearer)) !== 0;
    }

    setSignalOverride(wearer, signal, value) {
        const handle = this.live();
        const index = typeof signal === "string" ? SIGNALS.indexOf(signal) : signal;
        this.check(this.wasm.nameplates_set_signal_override(handle, wearerId(wearer), index, value));
    }

    setCustomColors(wearer, asset, slots, rgb) {
        const handle = this.live();
        if (slots.length !== rgb.length) throw new Error(`${slots.length} custom slots for ${rgb.length} colors`);
        this.customSlots.write(slots, slots.length);
        this.customRgb.write(rgb, rgb.length);
        this.check(this.wasm.nameplates_set_custom_colors(
            handle,
            wearerId(wearer),
            asset,
            this.customSlots.ptr,
            this.customRgb.ptr,
            slots.length,
        ));
    }

    clearCustomColors(wearer, asset) {
        return this.wasm.nameplates_clear_custom_colors(this.live(), wearerId(wearer), asset) !== 0;
    }

    fire(wearer, asset, oneShot) {
        return this.check(this.wasm.nameplates_fire(this.live(), wearerId(wearer), asset, oneShot)) > 0;
    }

    stripLayout() {
        return this.wasm.nameplates_strip_layout(this.live()) >>> 0;
    }

    stripTexels(id) {
        const handle = this.live();
        const slot = this.lengthSlot;
        const ptr = this.wasm.nameplates_strip_texels(handle, id, slot.ptr);
        const len = slot.current()[0];
        if (len === 0) return EMPTY_TEXELS;
        const buffer = this.engine.memory();
        const layout = this.stripLayout();
        if (this.texelBuffer !== buffer || this.texelLayout !== layout) {
            this.texelBuffer = buffer;
            this.texelLayout = layout;
            this.texelViews.clear();
        }
        let view = this.texelViews.get(id);
        if (!view || view.byteOffset !== ptr || view.length !== len) {
            view = new Uint32Array(buffer, ptr, len);
            this.texelViews.set(id, view);
        }
        return view;
    }

    wearerCount() {
        return this.wasm.nameplates_wearer_count(this.live()) >>> 0;
    }

    wearerBytes() {
        return this.wasm.nameplates_wearer_bytes(this.live()) >>> 0;
    }

    pendingFallbacks() {
        return this.wasm.nameplates_pending_fallbacks(this.live()) >>> 0;
    }

    runFallbacks(limit = 0xffffffff) {
        return this.check(this.wasm.nameplates_run_fallbacks(this.live(), limit >>> 0));
    }

    frame(clock, wearers, tags, signals, glyphs = EMPTY_FLOATS, colors = EMPTY_INTS) {
        const handle = this.live();
        const count = Math.floor(tags.length / OUT.TAG_INTS);
        const tagLen = count * OUT.TAG_INTS;
        const wearerView = this.wearers.reserve(count);
        if ((wearers instanceof BigUint64Array || wearers instanceof BigInt64Array) && wearers.length === count) {
            wearerView.set(wearers);
        } else {
            for (let i = 0; i < count; i++) {
                const w = wearers[i];
                wearerView[i] = typeof w === "bigint" ? w : BigInt(w);
            }
        }
        this.tags.write(tags, tagLen);
        this.signals.write(signals, signals.length);
        this.glyphs.write(glyphs, glyphs.length);
        this.colors.write(colors, colors.length);
        const needed = this.check(this.wasm.nameplates_needed_words(handle, this.tags.ptr, tagLen));
        this.out.reserve(needed);
        const words = this.check(this.wasm.nameplates_frame(
            handle,
            clock,
            this.wearers.ptr, count,
            this.tags.ptr, tagLen,
            this.signals.ptr, signals.length,
            this.glyphs.ptr, glyphs.length,
            this.colors.ptr, colors.length,
            this.out.ptr, this.out.capacity,
        ));
        return this.out.current().slice(0, words);
    }

    free() {
        if (!this.handle) return;
        for (const region of [this.wearers, this.tags, this.signals, this.glyphs, this.colors, this.out, this.customSlots, this.customRgb, this.lengthSlot]) {
            region.free();
        }
        this.texelViews.clear();
        this.wasm.nameplates_free(this.handle);
        this.handle = 0;
    }
}

const EMPTY_TEXELS = new Uint32Array(0);
const EMPTY_FLOATS = new Float32Array(0);
const EMPTY_INTS = new Int32Array(0);

export function readRasterInfo(words) {
    const I = RASTER_INFO;
    const count = words[I.LETTER_COUNT];
    const letters = [];
    for (let i = 0; i < count; i++) {
        const at = I.LETTERS + i * OUT.LETTER_WORDS;
        const packed = words[at + OUT.LETTER_SCALE_ROT];
        letters.push({
            dx: words[at + OUT.LETTER_DX],
            dy: words[at + OUT.LETTER_DY],
            color: words[at + OUT.LETTER_ARGB] >>> 0,
            shadow: words[at + OUT.LETTER_SHADOW] >>> 0,
            opacity: words[at + OUT.LETTER_OPACITY],
            scale: (packed & 0xff) / 4,
            rotation: packed >> 8,
            outline: words[at + OUT.LETTER_OUTLINE] >>> 0,
        });
    }
    return {
        width: words[I.WIDTH],
        height: words[I.HEIGHT],
        origin: [words[I.X], words[I.Y]],
        plate: [words[I.PLATE_DX], words[I.PLATE_DY]],
        badge: words[I.BADGE_SHOWN]
            ? { x: words[I.BADGE_X], y: words[I.BADGE_Y], opacity: words[I.BADGE_OPACITY] }
            : null,
        textMode: TEXT_MODES[words[I.TEXT_MODE]] ?? "vanilla",
        letters,
    };
}

export function blendPixel(source, destination, blend = "alpha") {
    const sa = source >>> 24;
    if (sa === 0) return destination >>> 0;
    const da = destination >>> 24;
    if (blend === "additive") {
        const alpha = Math.max(sa, da);
        let out = alpha * 0x1000000;
        for (const shift of [16, 8, 0]) {
            const sc = (source >>> shift) & 0xff;
            const dc = (destination >>> shift) & 0xff;
            const premul = Math.floor((dc * da) / 255) + Math.floor((sc * sa + 127) / 255);
            out += Math.min(255, Math.floor((premul * 255 + Math.floor(alpha / 2)) / alpha)) * 2 ** shift;
        }
        return out >>> 0;
    }
    if (sa === 255 || da === 0) return source >>> 0;
    const oa = sa + Math.floor((da * (255 - sa) + 127) / 255);
    let out = oa * 0x1000000;
    for (const shift of [16, 8, 0]) {
        const sc = (source >>> shift) & 0xff;
        const dc = (destination >>> shift) & 0xff;
        out += Math.floor((sc * sa * 255 + dc * da * (255 - sa) + Math.floor((oa * 255) / 2)) / (oa * 255)) * 2 ** shift;
    }
    return out >>> 0;
}

export function flattenRasterLayers(layers, into) {
    let out = into;
    for (const layer of layers) {
        for (const data of layer.passes?.length ? layer.passes : [layer.data]) {
            if (!out) out = new Uint8ClampedArray(data.length);
            if (data.length !== out.length) throw new Error("nameplate: raster layers differ in size");
            for (let i = 0; i < data.length; i += 4) {
                const alpha = data[i + 3];
                if (alpha === 0) continue;
                const source = (alpha * 0x1000000 + (data[i] << 16) + (data[i + 1] << 8) + data[i + 2]) >>> 0;
                const below = (out[i + 3] * 0x1000000 + (out[i] << 16) + (out[i + 1] << 8) + out[i + 2]) >>> 0;
                const blended = blendPixel(source, below, layer.blend);
                out[i] = (blended >>> 16) & 0xff;
                out[i + 1] = (blended >>> 8) & 0xff;
                out[i + 2] = blended & 0xff;
                out[i + 3] = blended >>> 24;
            }
        }
    }
    return out ?? new Uint8ClampedArray(0);
}

export function section(out, tag) {
    const dir = OUT.OUT_HEADER + tag * OUT.DIR_WORDS;
    return { offset: out[dir + OUT.DIR_OFFSET], length: out[dir + OUT.DIR_LEN], status: out[dir + OUT.DIR_STATUS] };
}

export function events(out, tag) {
    const { offset, length } = section(out, tag);
    if (length < OUT.TAG_HEAD) return [];
    const count = out[offset + OUT.HEAD_EVENTS];
    const start = offset + length - count * OUT.EVENT_WORDS;
    const list = [];
    for (let i = 0; i < count; i++) {
        list.push({ code: out[start + i * 2 + OUT.EVENT_CODE], target: out[start + i * 2 + OUT.EVENT_LAYER] });
    }
    return list;
}
