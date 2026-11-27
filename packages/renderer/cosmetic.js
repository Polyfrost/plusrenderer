import { defaultWasmUrl, instantiate, storeZip } from "./nameplate.js";

export const KINDS = Object.freeze(["item", "pet", "effect", "emote"]);
export const RIGS = Object.freeze(["player", "actor", "boat", "minecart", "horse"]);
export const RIG_VALUES = Object.freeze({ player: 0, actor: 2, boat: 3, minecart: 4, horse: 5 });
export const STATUSES = Object.freeze(["asset", "draft", "skipped", "invalid", "legacy"]);
export const ENGINE_CAPABILITIES = Object.freeze([
    "controllers", "springs", "effects", "particles", "trails", "sounds", "custom_colors", "rigs", "pets", "roles", "events",
]);
export const HOST_CAPABILITIES = Object.freeze(["reactions", "palettes"]);
export const AUTHORING_CAPABILITIES = Object.freeze([...HOST_CAPABILITIES, ...ENGINE_CAPABILITIES]);
export const NO_COLORWAY = -1;
export const TEXTURE_INFO_INTS = 3;
export const SECTION_INTS = 9;
export const INPUT_LAYOUT_INTS = 9;
export const SECTION_NAMES = Object.freeze(["parts", "queries", "root", "variables", "play", "role", "colorway", "part_deltas", "hold"]);
export const INPUT_LAYOUT_NAMES = Object.freeze(["signals", "events", "detail", "root_at", "variables_at", "play_at", "role_at", "stride", "hold_at"]);

const MODES = { client: 0, authoring: 1 };
const encoder = new TextEncoder();
const decoder = new TextDecoder();

function toBytes(data) {
    if (data instanceof Uint8Array) return data;
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    return new Uint8Array(data);
}

function u64(value) {
    return BigInt.asUintN(64, BigInt(value ?? 0));
}

function sectionInts(sections) {
    if (Array.isArray(sections) || ArrayBuffer.isView(sections)) return Int32Array.from(sections);
    return Int32Array.from(SECTION_NAMES, (name) => {
        const value = sections?.[name];
        return typeof value === "boolean" ? Number(value) : value ?? 0;
    });
}

export class CosmeticEngine {
    static async create(source = defaultWasmUrl) {
        return new CosmeticEngine(await instantiate(source));
    }

    constructor(wasm) {
        if (typeof wasm.cosmetic_parse !== "function") throw new Error("cosmetic: this plusrenderer build has no cosmetic exports");
        this.wasm = wasm;
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

    free(ptr, size) {
        if (ptr) this.wasm.dealloc(ptr, Math.max(size, 1));
    }

    text(ptr, len) {
        return decoder.decode(new Uint8Array(this.memory(), ptr, len));
    }

    error() {
        return this.text(this.wasm.cosmetic_error(), this.wasm.cosmetic_error_len());
    }

    json(len) {
        if (len < 0) throw new Error(this.error());
        return JSON.parse(this.text(this.wasm.cosmetic_text(), len));
    }

    parse(source, { mode = "client", capabilities, identity } = {}) {
        const zip = ArrayBuffer.isView(source) || source instanceof ArrayBuffer ? toBytes(source) : storeZip(source);
        const modeValue = MODES[mode] ?? mode;
        const caps = capabilities ? encoder.encode(capabilities.join("\n")) : null;
        const zipPtr = this.put(zip);
        const capsPtr = caps ? this.put(caps, true) : 0;
        let handle;
        try {
            const id = identity == null ? this.wasm.cosmetic_bundle_identity(zipPtr, zip.length) : u64(identity);
            handle = this.wasm.cosmetic_parse(zipPtr, zip.length, modeValue, capsPtr, caps ? caps.length : 0, id);
        } finally {
            this.free(zipPtr, zip.length);
            if (caps) this.free(capsPtr, caps.length);
        }
        if (!handle) throw new Error(this.error());
        return new CosmeticAsset(this, handle);
    }

    constants() {
        return (this.constantsCache ??= this.json(this.wasm.cosmetic_constants_json()));
    }

    inputLayout(sections) {
        const ints = sectionInts(sections);
        const out = new Int32Array(INPUT_LAYOUT_INTS);
        const inPtr = this.wasm.alloc(ints.byteLength);
        const outPtr = this.wasm.alloc(out.byteLength);
        try {
            new Int32Array(this.memory(), inPtr, ints.length).set(ints);
            const stride = this.wasm.cosmetic_input_layout(inPtr, ints.length, outPtr, out.length);
            if (stride < 0) throw new Error(this.error());
            out.set(new Int32Array(this.memory(), outPtr, out.length));
        } finally {
            this.free(inPtr, ints.byteLength);
            this.free(outPtr, out.byteLength);
        }
        return Object.fromEntries(INPUT_LAYOUT_NAMES.map((name, i) => [name, out[i]]));
    }

    pairSeed(a, b, minute) {
        return u64(this.wasm.cosmetic_pair_seed(u64(a), u64(b), BigInt(minute)));
    }

    actorWearer(owner, slot) {
        return u64(this.wasm.cosmetic_actor_wearer(u64(owner), slot));
    }

    eventWearer(owner, event, seq, nonce = 0) {
        return u64(this.wasm.cosmetic_event_key(u64(owner), event, seq >>> 0, nonce >>> 0));
    }
}

function paint(engine, custom) {
    const entries = custom instanceof Map ? [...custom] : Array.isArray(custom) ? custom : Object.entries(custom ?? {});
    const count = entries.length;
    if (count === 0) return { count: 0, slots: 0, rgb: 0, release() {} };
    const slots = Uint32Array.from(entries, ([slot]) => Number(slot));
    const rgb = Uint32Array.from(entries, ([, color]) => Number(color) & 0xffffff);
    const slotsPtr = engine.wasm.alloc(slots.byteLength);
    const rgbPtr = engine.wasm.alloc(rgb.byteLength);
    new Uint32Array(engine.memory(), slotsPtr, count).set(slots);
    new Uint32Array(engine.memory(), rgbPtr, count).set(rgb);
    return {
        count,
        slots: slotsPtr,
        rgb: rgbPtr,
        release() {
            engine.free(slotsPtr, slots.byteLength);
            engine.free(rgbPtr, rgb.byteLength);
        },
    };
}

export class CosmeticAsset {
    constructor(engine, handle) {
        this.engine = engine;
        this.handle = handle;
        this.status = STATUSES[engine.wasm.cosmetic_asset_status(handle)] ?? "invalid";
        const report = engine.json(engine.wasm.cosmetic_asset_issues_json(handle));
        this.issues = report.issues ?? [];
        this.skip = report.skip ?? null;
        this.legacy = report.legacy ?? null;
        this.metaCache = null;
        this.analysisCache = null;
    }

    get usable() {
        return this.status === "asset" || this.status === "draft";
    }

    live() {
        if (!this.handle) throw new Error("cosmetic: the asset was freed");
        return this.handle;
    }

    meta() {
        if (!this.usable) return null;
        return (this.metaCache ??= this.engine.json(this.engine.wasm.cosmetic_asset_meta_json(this.live())));
    }

    analyze() {
        if (!this.usable) return null;
        return (this.analysisCache ??= this.engine.json(this.engine.wasm.cosmetic_asset_analyze_json(this.live())));
    }

    textureCount() {
        if (!this.usable) return 0;
        const count = this.engine.wasm.cosmetic_texture_count(this.live());
        if (count < 0) throw new Error(this.engine.error());
        return count;
    }

    textureInfo(index) {
        const engine = this.engine;
        const ptr = engine.wasm.alloc(TEXTURE_INFO_INTS * 4);
        try {
            const bytes = engine.wasm.cosmetic_texture_info(this.live(), index, ptr);
            if (bytes < 0) throw new Error(engine.error());
            const [width, height, indexed] = new Int32Array(engine.memory(), ptr, TEXTURE_INFO_INTS);
            return { width, height, indexed: indexed !== 0, bytes };
        } finally {
            engine.free(ptr, TEXTURE_INFO_INTS * 4);
        }
    }

    resolveTexture(index, { colorway = NO_COLORWAY, custom } = {}) {
        const engine = this.engine;
        const { bytes } = this.textureInfo(index);
        const colors = paint(engine, custom);
        const out = engine.wasm.alloc(Math.max(bytes, 1));
        try {
            const written = engine.wasm.cosmetic_resolve_texture(this.live(), index, colorway, colors.slots, colors.rgb, colors.count, out, bytes);
            if (written < 0) throw new Error(engine.error());
            return new Uint8Array(engine.memory(), out, written).slice();
        } finally {
            engine.free(out, bytes);
            colors.release();
        }
    }

    textureKey(index, { colorway = NO_COLORWAY, custom } = {}) {
        const engine = this.engine;
        const colors = paint(engine, custom);
        const out = engine.wasm.alloc(8);
        try {
            const status = engine.wasm.cosmetic_texture_key(this.live(), index, colorway, colors.slots, colors.rgb, colors.count, out);
            if (status < 0) throw new Error(engine.error());
            return new DataView(engine.memory(), out, 8).getBigUint64(0, true);
        } finally {
            engine.free(out, 8);
            colors.release();
        }
    }

    free() {
        if (!this.handle) return;
        this.engine.wasm.cosmetic_asset_free(this.handle);
        this.handle = 0;
    }
}

export function parseCosmetic(engine, source, options) {
    return (engine instanceof CosmeticEngine ? engine : new CosmeticEngine(engine.wasm ?? engine)).parse(source, options);
}
