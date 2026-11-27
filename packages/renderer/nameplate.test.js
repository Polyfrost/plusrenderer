import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import {
    blendPixel, flattenRasterLayers, layerBits, NameplateEngine, RASTER_BAND_ALL, RASTER_BAND_OVER_TEXT,
    RASTER_BAND_UNDER_TEXT, rasterBand, SIGNALS,
} from "./nameplate.js";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = join(here, "..", "..");

function loadWasm() {
    for (const path of [process.env.PLUSRENDERER_WASM, join(here, "plusrenderer.wasm"), join(root, "target", "wasm32-unknown-unknown", "wasm", "plusrenderer_ffi.wasm")]) {
        if (!path || !existsSync(path)) continue;
        const module = new WebAssembly.Module(readFileSync(path));
        if (WebAssembly.Module.exports(module).some((entry) => entry.name === "nameplate_raster_ex")) return new WebAssembly.Instance(module, {}).exports;
    }
    return null;
}

const wasm = loadWasm();
const engineTest = wasm && wasm.nameplate_raster_ex.length >= 24 && wasm.nameplate_raster_images ? test : test.skip;

const [RED, GREEN, HALF_BLUE] = [0xffff0000, 0xff00ff00, 0x800000ff];

const json = (value) => new TextEncoder().encode(JSON.stringify(value));
const body = (fill) => ({ kind: "body", padding: [2, 1, 2, 1], fill: { color: fill } });
const constant = (color) => ({ resolution: "pixel", margins: [4, 4, 4, 4], nodes: [{ id: "out", type: "Output", in: { color } }] });

function clipFiles() {
    return {
        "manifest.json": json({
            format: 1, kind: "nameplate", id: "clip", requires: ["nameplate", "nameplate_materials", "nameplate_masks"],
            fps: 6, loop: 12, text: { color: "ink" },
            layers: [
                { id: "back", depth: "background", mask: "front" },
                { id: "front", depth: "over_text" },
                { id: "alt", depth: "over_text", mask: "front" },
            ],
            variants: { plain: { hide: ["alt"] }, swapped: { hide: ["alt"], swap: { back: "alt" } } },
        }),
        "palettes.json": json({ slots: [{ id: "ink", ramp: ["#FFFFFF"] }, { id: "blue", ramp: ["#800000FF"] }] }),
        "materials/red.json": json(constant([1, 0, 0, 1])),
        "materials/green.json": json(constant([0, 1, 0, 1])),
        "layers/back.json": json({ kind: "material", material: "red" }),
        "layers/front.json": json(body("blue")),
        "layers/alt.json": json({ kind: "material", material: "green" }),
    };
}

function load(files) {
    const asset = new NameplateEngine(wasm).loadSync(files, { mode: "authoring" });
    assert.equal(asset.kind, "asset", JSON.stringify(asset.issues));
    return asset;
}

function pixels(image) {
    const out = [];
    for (let i = 0; i < image.data.length; i += 4) {
        const [r, g, b, a] = image.data.subarray(i, i + 4);
        out.push(((a << 24) | (r << 16) | (g << 8) | b) >>> 0);
    }
    return out;
}

function raster(asset, options = {}) {
    const variant = options.variant ?? 0;
    const image = asset.raster({ asset: 0, variant, nameWidth: 40 }, 0, options);
    return { image, colors: pixels(image) };
}

test("raster bands and layer sets read names, numbers and slot lists", () => {
    assert.equal(rasterBand(undefined), RASTER_BAND_ALL);
    assert.equal(rasterBand("under_text"), RASTER_BAND_UNDER_TEXT);
    assert.equal(rasterBand("over_text"), RASTER_BAND_OVER_TEXT);
    assert.throws(() => rasterBand("above"), /no raster band above/);
    assert.throws(() => rasterBand(3), /no raster band 3/);
    assert.equal(layerBits([0, 2]), 0b101);
    assert.equal(layerBits(null), 0);
    assert.throws(() => layerBits([32]), /no layer slot 32/);
});

engineTest("a material masked by a translucent front body keeps its clip in each band", () => {
    const asset = load(clipFiles());
    const plain = asset.meta.variants.findIndex((variant) => variant.key === "plain");
    const all = raster(asset, { variant: plain });
    const under = raster(asset, { variant: plain, band: "under_text" });
    const top = raster(asset, { variant: plain, band: RASTER_BAND_OVER_TEXT });
    assert.deepEqual([under.image.width, under.image.height, ...under.image.origin], [all.image.width, all.image.height, ...all.image.origin]);
    const front = top.colors.map((color) => color >>> 24 !== 0);
    assert.ok(front.some(Boolean), "the front body draws in its band");
    top.colors.forEach((color, i) => {
        assert.equal(color, front[i] ? HALF_BLUE : 0, `over_text pixel ${i}`);
        assert.equal(under.colors[i], front[i] ? RED : 0, `under_text pixel ${i}`);
        assert.equal(all.colors[i], front[i] ? blendPixel(HALF_BLUE, RED) : 0, `full pixel ${i}`);
    });
    const omitted = raster(asset, { variant: plain, omit: [1] });
    omitted.colors.forEach((color, i) => assert.equal(color, front[i] ? RED : 0, `omitted pixel ${i}`));
    const hidden = raster(asset, { variant: plain, hidden: [1] });
    assert.ok(hidden.colors.every((color) => color === 0), "a hidden mask target clips its material away");
    asset.free();
});

function glowFiles(blend, { alpha = 1, mirror = false, masked = true } = {}) {
    return {
        "manifest.json": json({
            format: 1, kind: "nameplate", id: "glow", requires: ["nameplate", "nameplate_materials", "nameplate_masks"],
            fps: 6, loop: 12, text: { color: "ink" },
            layers: [{ id: "back", depth: "background" }, { id: "glow", depth: "over_text", ...(masked ? { mask: "back" } : {}) }],
        }),
        "palettes.json": json({ slots: [{ id: "ink", ramp: ["#FFFFFF"] }, { id: "green", ramp: ["#00FF00"] }] }),
        "materials/glow.json": json({
            resolution: "pixel", margins: [4, 4, 4, 4],
            nodes: [{ id: "out", type: "Output", in: { color: [1, 0, 0, alpha] }, set: { blend } }],
        }),
        "layers/back.json": json(body("green")),
        "layers/glow.json": json({ kind: "material", material: "glow", ...(mirror ? { mirror: true } : {}) }),
    };
}

function layered(asset, options = {}) {
    return asset.rasterLayers({ asset: 0, variant: options.variant ?? 0, nameWidth: 40 }, 0, options);
}

engineTest("an additive over-text material comes back as its own layer with its blend", () => {
    const asset = load(glowFiles("additive"));
    const whole = raster(asset);
    const result = layered(asset);
    assert.deepEqual([result.width, result.height, ...result.origin], [whole.image.width, whole.image.height, ...whole.image.origin]);
    assert.deepEqual(result.layers.map(({ slot, id, depth, band, blend, composite }) => ({ slot, id, depth, band, blend, composite })), [
        { slot: 0, id: "back", depth: "background", band: "under_text", blend: "alpha", composite: "source-over" },
        { slot: 1, id: "glow", depth: "over_text", band: "over_text", blend: "additive", composite: "lighter" },
    ]);
    const back = pixels(result.layers[0]);
    const glow = pixels(result.layers[1]);
    assert.ok(back.some((color) => color === GREEN));
    back.forEach((color, i) => {
        assert.equal(glow[i], color === GREEN ? RED : 0, `glow pixel ${i}`);
        assert.equal(whole.colors[i], color === GREEN ? 0xffffff00 : color, `whole pixel ${i}`);
    });
    assert.deepEqual(flattenRasterLayers(result.layers), whole.image.data);
    const under = flattenRasterLayers(result.layers.filter((layer) => layer.band === "under_text"));
    assert.deepEqual(flattenRasterLayers(result.layers.filter((layer) => layer.band === "over_text"), under), whole.image.data);
    asset.free();
});

engineTest("overlapping draws inside a layer flatten exactly through their passes", () => {
    for (const blend of ["additive", "alpha"]) {
        for (const alpha of [0.5, 0.25, 1]) {
            for (const masked of [true, false]) {
                const asset = load(glowFiles(blend, { alpha, mirror: true, masked }));
                for (const scale of [1, 2]) {
                    const whole = raster(asset, { scale });
                    const result = layered(asset, { scale });
                    const label = `${blend} ${alpha} ${masked} x${scale}`;
                    assert.equal(result.layers[0].passes.length, 1, label);
                    assert.equal(result.layers[0].passes[0], result.layers[0].data, label);
                    assert.equal(result.layers[1].passes.length, 2, label);
                    assert.ok(result.layers[1].passes.every((pass) => pass.length === whole.image.data.length), label);
                    assert.deepEqual(flattenRasterLayers(result.layers), whole.image.data, label);
                    const under = flattenRasterLayers(result.layers.filter((layer) => layer.band === "under_text"));
                    assert.deepEqual(flattenRasterLayers(result.layers.filter((layer) => layer.band === "over_text"), under), whole.image.data, label);
                }
                asset.free();
            }
        }
    }
});

function timedFiles() {
    return {
        "manifest.json": json({
            format: 1, kind: "nameplate", id: "timed", requires: ["nameplate"],
            fps: 6, loop: 12, text: { color: "ink" },
            layers: [{ id: "back", depth: "background" }, { id: "front", depth: "over_text" }],
            animations: { loop: { tracks: { front: { position: [[0, ["q.is_sneaking_time * 4", 0]]] } } } },
        }),
        "palettes.json": json({ slots: [
            { id: "ink", ramp: ["#FFFFFF"] }, { id: "green", ramp: ["#00FF00"] }, { id: "blue", ramp: ["#0000FF"] },
        ] }),
        "layers/back.json": json(body("green")),
        "layers/front.json": json(body("blue")),
    };
}

engineTest("stopped signals time their queries and take names, indexes or entries", () => {
    const asset = load(timedFiles());
    const at = (stopped) => pixels(asset.raster({ asset: 0, variant: 0, nameWidth: 40 }, 20, { stopped }));
    const fresh = at([["is_sneaking", 6, 0]]);
    const later = at([["is_sneaking", 6, 3]]);
    assert.notDeepEqual(fresh, later);
    assert.deepEqual(at([{ signal: SIGNALS.indexOf("is_sneaking"), heldFor: 6, ago: 3 }]), later);
    assert.throws(() => at([["no_such_signal", 1, 1]]), /no signal/);
    asset.free();
});

engineTest("unchanged signals count their time from when the wearer was first observed", () => {
    const asset = load(timedFiles());
    const row = { asset: 0, variant: 0, nameWidth: 40 };
    const at = (options) => pixels(asset.raster(row, 6_000_000, options));
    assert.ok(asset.engine.observedRaster);
    assert.deepEqual(at({}), at({ observedFor: 6_000_000 }));
    assert.deepEqual(at({ observedFor: null }), at({}));
    assert.notDeepEqual(at({ observedFor: 3 }), at({}));
    assert.deepEqual(at({ observedFor: 3 }), at({ stopped: [["is_sneaking", 0, 3]] }));
    asset.free();
});

engineTest("an older wasm without the observation origin rasters as before", () => {
    const asset = load(timedFiles());
    const row = { asset: 0, variant: 0, nameWidth: 40 };
    const engine = asset.engine;
    const real = engine.wasm;
    const before = pixels(asset.raster(row, 6_000_000));
    const legacy = new NameplateEngine({
        ...real,
        nameplate_raster_images: undefined,
        nameplate_raster_ex(a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w) {
            return real.nameplate_raster_ex(a, b, c, d, e, f, g, h, i, 0xffffffff, j, k, l, m, n, o, p, q, r, s, t, u, v, w);
        },
    });
    assert.equal(legacy.stoppedRaster, true);
    assert.equal(legacy.observedRaster, false);
    engine.wasm = legacy.wasm;
    engine.observedRaster = false;
    try {
        assert.deepEqual(pixels(asset.raster(row, 6_000_000, { observedFor: 3 })), before);
    } finally {
        engine.wasm = real;
        engine.observedRaster = true;
    }
    asset.free();
});
