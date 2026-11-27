import { test } from "node:test";
import assert from "node:assert/strict";
import { OUT } from "./nameplate.js";
import { NameplateDrawer, disjointRects } from "./nameplate-gl.js";

const UNPACK = ["ALIGNMENT", "ROW_LENGTH", "SKIP_ROWS", "SKIP_PIXELS", "IMAGE_HEIGHT", "SKIP_IMAGES", "FLIP_Y_WEBGL", "PREMULTIPLY_ALPHA_WEBGL", "COLORSPACE_CONVERSION_WEBGL"];
const SHADER = { key: "k", material: 0, vertex: "", fragment: "", samplers: [{ unit: 3, name: "Sampler0", texture: "vanilla:x" }] };
const IDENTITY = { x: 0, y: 0, scale: 1 };

function area(rects) {
    return rects.reduce((sum, [x0, y0, x1, y1]) => sum + (x1 - x0) * (y1 - y0), 0);
}

function covered(rects, x, y) {
    return rects.filter(([x0, y0, x1, y1]) => x >= x0 && x < x1 && y >= y0 && y < y1).length;
}

function mockGl({ width = 100, height = 100 } = {}) {
    const constants = new Map([["TEXTURE0", 0x84c0]]);
    const constant = (name) => {
        if (!constants.has(name)) constants.set(name, 0x1000 + constants.size);
        return constants.get(name);
    };
    const c = new Proxy({}, { get: (_, name) => constant(name) });
    let created = 0;
    const state = {
        framebuffer: null, enabled: new Set(), scissorBox: [0, 0, width, height], viewport: [0, 0, width, height],
        equation: [c.FUNC_ADD, c.FUNC_ADD], func: [c.ONE, c.ZERO, c.ONE, c.ZERO], program: null, vao: null,
        buffers: new Map(), indexed: new Map(), activeTexture: c.TEXTURE0, textures: new Map(), samplers: new Map(),
        unpack: new Map(UNPACK.map((name, i) => [c[`UNPACK_${name}`], [4, 0, 0, 0, 0, 0, false, false, c.BROWSER_DEFAULT_WEBGL][i]])),
        uploads: [], calls: [], draws: [],
    };
    const parameters = {
        DRAW_FRAMEBUFFER_BINDING: () => state.framebuffer, FRAMEBUFFER_BINDING: () => state.framebuffer, STENCIL_BITS: () => 8,
        VIEWPORT: () => Int32Array.from(state.viewport), SCISSOR_BOX: () => Int32Array.from(state.scissorBox),
        BLEND_EQUATION_RGB: () => state.equation[0], BLEND_EQUATION_ALPHA: () => state.equation[1],
        BLEND_SRC_RGB: () => state.func[0], BLEND_DST_RGB: () => state.func[1], BLEND_SRC_ALPHA: () => state.func[2], BLEND_DST_ALPHA: () => state.func[3],
        CURRENT_PROGRAM: () => state.program, VERTEX_ARRAY_BINDING: () => state.vao, ACTIVE_TEXTURE: () => state.activeTexture,
        ARRAY_BUFFER_BINDING: () => state.buffers.get(c.ARRAY_BUFFER) ?? null,
        UNIFORM_BUFFER_BINDING: () => state.buffers.get(c.UNIFORM_BUFFER) ?? null,
        PIXEL_UNPACK_BUFFER_BINDING: () => state.buffers.get(c.PIXEL_UNPACK_BUFFER) ?? null,
        SAMPLER_BINDING: () => state.samplers.get(state.activeTexture - c.TEXTURE0) ?? null,
        TEXTURE_BINDING_2D: () => state.textures.get(state.activeTexture - c.TEXTURE0) ?? null,
    };
    const byValue = new Map(Object.entries(parameters).map(([name, read]) => [constant(name), read]));
    const unpackState = () => ({ buffer: state.buffers.get(c.PIXEL_UNPACK_BUFFER) ?? null, params: Object.fromEntries(state.unpack) });
    const methods = {
        getParameter: (name) => (byValue.has(name) ? byValue.get(name)() : state.unpack.has(name) ? state.unpack.get(name) : 0),
        getIndexedParameter(name, index) {
            const binding = state.indexed.get(index);
            if (name === c.UNIFORM_BUFFER_BINDING) return binding?.buffer ?? null;
            return (name === c.UNIFORM_BUFFER_START ? binding?.start : name === c.UNIFORM_BUFFER_SIZE ? binding?.size : 0) ?? 0;
        },
        isEnabled: (cap) => state.enabled.has(cap),
        enable: (cap) => void state.enabled.add(cap),
        disable: (cap) => void state.enabled.delete(cap),
        scissor: (x, y, w, h) => void (state.scissorBox = [x, y, w, h]),
        viewport: (x, y, w, h) => void (state.viewport = [x, y, w, h]),
        blendEquationSeparate: (rgb, alpha) => void (state.equation = [rgb, alpha]),
        blendFuncSeparate: (...func) => void (state.func = func),
        useProgram: (program) => void (state.program = program),
        bindVertexArray: (vao) => void (state.vao = vao),
        bindBuffer: (target, buffer) => void state.buffers.set(target, buffer),
        bindBufferBase: (target, index, buffer) => methods.bindBufferRange(target, index, buffer, 0, 0),
        bindBufferRange(target, index, buffer, start, size) {
            state.indexed.set(index, { buffer, start, size });
            state.buffers.set(target, buffer);
        },
        activeTexture: (unit) => void (state.activeTexture = unit),
        bindTexture: (_, texture) => void state.textures.set(state.activeTexture - c.TEXTURE0, texture),
        pixelStorei: (name, value) => void state.unpack.set(name, value),
        bindSampler: (unit, sampler) => void state.samplers.set(unit, sampler),
        texImage2D: () => void state.uploads.push(unpackState()),
        texSubImage2D: () => void state.uploads.push(unpackState()),
        bindFramebuffer: (_, framebuffer) => void (state.framebuffer = framebuffer),
        drawArrays: () => void state.draws.push({
            scissor: state.enabled.has(c.SCISSOR_TEST) ? [...state.scissorBox] : null, depth: state.enabled.has(c.DEPTH_TEST),
            blend: state.enabled.has(c.BLEND), cull: state.enabled.has(c.CULL_FACE), samplers: new Map(state.samplers),
        }),
        getAttribLocation: () => 0,
        getUniformBlockIndex: () => c.INVALID_INDEX,
        getShaderParameter: () => true,
        getProgramParameter: () => true,
    };
    const gl = new Proxy({}, {
        get(_, name) {
            if (typeof name !== "string") return undefined;
            if (/^[A-Z0-9_]+$/.test(name)) return constant(name);
            if (name === "drawingBufferWidth") return width;
            if (name === "drawingBufferHeight") return height;
            return (...args) => {
                state.calls.push([name, ...args]);
                if (methods[name]) return methods[name](...args);
                return name.startsWith("create") ? { kind: name.slice(6), id: ++created } : {};
            };
        },
    });
    return { gl, state };
}

function snapshot(state) {
    const sorted = (map) => [...map].sort((a, b) => a[0] - b[0]);
    const { enabled, scissorBox, viewport, equation, func, program, vao, activeTexture } = state;
    return {
        enabled: [...enabled].sort(), scissorBox: [...scissorBox], viewport: [...viewport], equation: [...equation], func: [...func],
        program, vao, activeTexture, buffers: sorted(state.buffers), indexed: sorted(state.indexed), textures: sorted(state.textures),
        samplers: sorted(state.samplers), unpack: sorted(state.unpack),
    };
}

function setHostState(gl) {
    const host = Object.fromEntries(["program", "vao", "array", "ubo", "generic", "tex0", "tex3", "sampler0", "sampler3", "unpack"].map((name) => [name, { host: name }]));
    [gl.DEPTH_TEST, gl.CULL_FACE, gl.SCISSOR_TEST].forEach((cap) => gl.enable(cap));
    gl.scissor(0, 0, 100, 100);
    gl.disable(gl.BLEND);
    gl.blendEquationSeparate(gl.FUNC_SUBTRACT, gl.MIN);
    gl.blendFuncSeparate(gl.DST_COLOR, gl.ZERO, gl.ONE, gl.ZERO);
    gl.useProgram(host.program);
    gl.bindVertexArray(host.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, host.array);
    gl.bindBufferRange(gl.UNIFORM_BUFFER, 0, host.ubo, 16, 64);
    gl.bindBuffer(gl.UNIFORM_BUFFER, host.generic);
    for (const unit of [0, 3]) {
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, host[`tex${unit}`]);
        gl.bindSampler(unit, host[`sampler${unit}`]);
    }
    UNPACK.forEach((name, i) => gl.pixelStorei(gl[`UNPACK_${name}`], [1, 7, 2, 3, 5, 1, true, true, gl.BROWSER_DEFAULT_WEBGL][i]));
    gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, host.unpack);
    return host;
}

function material(clips, run = 0, band = 0) {
    const out = new Int32Array(OUT.MAT_FIXED + clips.length * OUT.CLIP_WORDS + 8);
    out[OUT.MAT_BAND_LAYER] = (0xff << OUT.MAT_OPACITY_SHIFT) | band;
    out[OUT.MAT_RUN] = run;
    out[OUT.MAT_AREA_X1] = out[OUT.MAT_AREA_Y1] = 20;
    out[OUT.MAT_CLIP_COUNT] = clips.length;
    out[OUT.MAT_UNIFORM_WORDS] = 8;
    clips.forEach((clip, k) => out.set(clip, OUT.MAT_FIXED + k * OUT.CLIP_WORDS));
    return out;
}

function tag(ops, mats, status = OUT.STATUS_OK) {
    const matWords = mats.reduce((sum, mat) => sum + mat.length, 0);
    const len = OUT.TAG_HEAD + ops.length * OUT.OP_WORDS + matWords + OUT.TEXT_FIXED + OUT.BADGE_WORDS;
    const at = OUT.OUT_HEADER + OUT.DIR_WORDS;
    const out = new Int32Array(at + len);
    [out[OUT.OUT_TAGS], out[OUT.OUT_WORDS]] = [1, out.length];
    [out[OUT.OUT_HEADER + OUT.DIR_OFFSET], out[OUT.OUT_HEADER + OUT.DIR_LEN], out[OUT.OUT_HEADER + OUT.DIR_STATUS]] = [at, len, status];
    [out[at + OUT.HEAD_OPS], out[at + OUT.HEAD_MATERIALS], out[at + OUT.HEAD_MATERIAL_WORDS]] = [ops.length, mats.length, matWords];
    ops.forEach(([kind, band], k) => {
        const op = at + OUT.TAG_HEAD + k * OUT.OP_WORDS;
        [out[op + OUT.OP_X1], out[op + OUT.OP_Y1], out[op + OUT.OP_ARGB]] = [5, 5, -1];
        out[op + OUT.OP_BAND_LAYER_KIND] = (kind << 16) | band;
        out[op + OUT.OP_AUX0] = 7;
    });
    let matAt = at + OUT.TAG_HEAD + ops.length * OUT.OP_WORDS;
    for (const mat of mats) {
        out.set(mat, matAt);
        matAt += mat.length;
    }
    return out;
}

const clipped = (status) => tag([[OUT.OP_KIND_STRIP, OUT.BAND_BEHIND]], [material([[0, 0, 10, 10]], 1)], status);
const texts = { assetId: 1, viewport: [100, 100], text: (ctx) => ctx.drawer.fillRect(0, 0, 1, 1, 0xffffffff) };

function drawer(gl = null, size = {}, options) {
    const mock = gl ? null : mockGl(size);
    const context = gl ?? mock.gl;
    const nameplates = { stripLayout: () => 0, stripTexels: () => new Uint32Array(6) };
    const d = new NameplateDrawer(context, nameplates, options);
    d.setAssetShaders(1, [SHADER]);
    return { gl: context, state: mock?.state, drawer: d };
}

function draw(setup, clips, { viewport = [100, 100], transform = IDENTITY } = {}) {
    const { state, drawer: d } = setup;
    state.calls.length = state.draws.length = 0;
    d.drawMaterial(1, material(clips), 0, viewport, transform);
    return state.draws.map((call) => call.scissor);
}

const touchesStencil = (state) => state.calls.some(([name]) => /stencil|colorMask|^clear/i.test(name));

test("disjoint rects cover the union exactly once and drop empty ones", () => {
    const rects = [[0, 0, 10, 10], [5, 5, 15, 15], [2, 2, 4, 4], [20, 0, 25, 5], [0, 0, 0, 5]];
    const parts = disjointRects(rects);
    assert.equal(area(parts), 100 + 100 - 25 + 25);
    for (let x = -1; x < 26; x++) {
        for (let y = -1; y < 16; y++) assert.equal(covered(parts, x, y), covered(rects, x, y) ? 1 : 0, `${x},${y}`);
    }
    assert.deepEqual(disjointRects([[1, 2, 1, 4], [5, 5, 6, 5]]), []);
});

test("clips scissor to their own rects in GL pixels without touching the stencil", () => {
    const setup = drawer();
    const { gl, state } = setup;
    const [first, second] = [{}, {}];
    for (const [framebuffer, clip, expected] of [[null, [0, 0, 10, 10], [0, 90, 10, 10]], [first, [0, 0, 10, 10], [0, 90, 10, 10]],
        [second, [20, 20, 30, 30], [20, 70, 10, 10]], [first, [0, 0, 10, 10], [0, 90, 10, 10]]]) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
        assert.deepEqual(draw(setup, [clip]), [expected]);
        assert.equal(touchesStencil(state), false);
    }
    assert.equal(state.enabled.has(gl.SCISSOR_TEST), false);
    const rects = draw(setup, [[0, 0, 10, 10], [5, 5, 15, 15], [0, 0, 10, 10]]).map(([x, y, w, h]) => [x, y, x + w, y + h]);
    assert.equal(area(rects), 175, "overlapping clips draw each pixel once");
    assert.deepEqual(draw(setup, [[1, 1, 3, 3]], { transform: { x: 0, y: 0, scale: 1.5 } }), [[1, 96, 3, 3]]);
    assert.deepEqual(draw(setup, [[0, 0, 10, 10]], { transform: { x: 50, y: 50, scale: -1 } }), [[40, 50, 10, 10]]);
    gl.viewport(10, 20, 200, 200);
    assert.deepEqual(draw(setup, [[0, 0, 10, 10]]), [[10, 200, 20, 20]]);
    gl.viewport(0, 0, 100, 100);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(5, 0, 90, 100);
    assert.deepEqual(draw(setup, [[0, 0, 10, 10]]), [[5, 90, 5, 10]], "clips stay inside the host scissor");
    assert.deepEqual(draw(setup, [[96, 0, 100, 10]]), []);
    assert.deepEqual(state.scissorBox, [5, 0, 90, 100]);
});

test("drawing restores host GL state on every path and draws without depth, culling or samplers", () => {
    const unscissored = drawer();
    unscissored.drawer.drawTag(clipped(), 0, texts);
    assert.deepEqual(unscissored.state.draws.map((call) => call.scissor), [null, [0, 90, 10, 10], null], "draws after a clipped material are not scissored");
    const { gl, state, drawer: d } = drawer();
    const host = setHostState(gl);
    const before = snapshot(state);
    state.calls.length = state.draws.length = 0;
    assert.equal(d.drawTag(clipped(), 0, texts), OUT.STATUS_OK);
    assert.deepEqual(state.draws.map((call) => call.scissor), [[0, 0, 100, 100], [0, 90, 10, 10], [0, 0, 100, 100]]);
    assert.ok(state.draws.every((call) => !call.depth && call.blend && !call.cull));
    assert.equal(state.draws[0].samplers.get(0), null);
    assert.equal(state.draws[1].samplers.get(3), null);
    assert.equal(touchesStencil(state), false);
    assert.deepEqual(snapshot(state), before);
    assert.equal(state.samplers.get(3), host.sampler3);
    assert.equal(d.drawTag(clipped(OUT.STATUS_VANILLA), 0, texts), OUT.STATUS_VANILLA);
    assert.deepEqual(snapshot(state), before);
    assert.throws(() => d.drawTag(clipped(), 0, { ...texts, text: () => { throw new Error("hook"); } }), /hook/);
    assert.deepEqual(snapshot(state), before);
    assert.equal(d.depth, 0);

    const fresh = mockGl();
    setHostState(fresh.gl);
    const saved = snapshot(fresh.state);
    const uploader = drawer(fresh.gl).drawer;
    uploader.setTexture("vanilla:x", new Uint8Array(4), 1, 1);
    uploader.setBundleTextures(1, [{ rgba: new Uint8Array(16), width: 2, height: 2 }, {}]);
    uploader.drawImage({}, 0, 0, 1, 1);
    uploader.drawTag(clipped(), 0, { assetId: 1, viewport: [100, 100] });
    assert.deepEqual(snapshot(fresh.state), saved);
    assert.equal(fresh.state.uploads.length, 6);
    const clean = Object.fromEntries(UNPACK.map((name, i) => [fresh.gl[`UNPACK_${name}`], [4, 0, 0, 0, 0, 0, false, false, fresh.gl.NONE][i]]));
    for (const upload of fresh.state.uploads) assert.deepEqual(upload, { buffer: null, params: clean });
});

test("the screen size is the physical target size, not the logical viewport", () => {
    const sizes = (state) => state.calls.filter(([name]) => name === "uniform2f").map(([, , x, y]) => [x, y]);
    const into = (options, bound = { id: "target" }) => {
        const setup = drawer(null, { width: 300, height: 300 }, options);
        setup.gl.bindFramebuffer(setup.gl.FRAMEBUFFER, bound);
        setup.gl.viewport(0, 0, 128, 128);
        setup.state.calls.length = 0;
        return setup;
    };
    const asked = [];
    for (const [setup, expected] of [[into(), [128, 128]], [into({ targetSize: (framebuffer) => (asked.push(framebuffer), [512, 256]) }), [512, 256]]]) {
        setup.drawer.drawMaterial(1, material([]), 0, [100, 100], IDENTITY);
        assert.deepEqual(sizes(setup.state), [expected]);
    }
    assert.deepEqual(asked, [{ id: "target" }]);
    const perCall = into({ targetSize: [64, 64] });
    perCall.drawer.drawMaterial(1, material([]), 0, [100, 100], IDENTITY, [512, 512]);
    perCall.drawer.drawTag(clipped(), 0, { assetId: 1, viewport: [100, 100], targetSize: [256, 256] });
    assert.deepEqual(sizes(perCall.state), [[512, 512], [256, 256]]);
});

test("the vanilla background sits between the background and over-background bands", () => {
    const order = (out) => {
        const { state, drawer: d } = drawer();
        const seen = [];
        const at = (name) => () => seen.push([name, state.draws.length]);
        d.drawTag(out, 0, { assetId: 1, viewport: [100, 100], background: at("background"), text: at("text"), badge: at("badge") });
        return [...seen, ["end", state.draws.length]];
    };
    const rects = (...bands) => bands.map((band) => [OUT.OP_KIND_RECT, band]);
    const { BAND_BEHIND, BAND_BACKGROUND, BAND_OVER_BACKGROUND, BAND_OVER_TEXT } = OUT;
    assert.deepEqual(order(tag(rects(BAND_BEHIND, BAND_BACKGROUND, BAND_OVER_BACKGROUND, BAND_OVER_TEXT), [])),
        [["background", 1], ["text", 2], ["badge", 2], ["end", 3]]);
    assert.deepEqual(order(tag(rects(BAND_BACKGROUND), [material([], 1, BAND_OVER_BACKGROUND)])),
        [["background", 1], ["text", 2], ["badge", 2], ["end", 2]]);
    assert.deepEqual(order(tag(rects(BAND_OVER_TEXT), [])), [["background", 0], ["text", 0], ["badge", 0], ["end", 1]]);
    const owned = tag(rects(BAND_BACKGROUND, BAND_OVER_BACKGROUND), []);
    owned[OUT.OUT_HEADER + OUT.DIR_WORDS + OUT.HEAD_OWNS_BACKGROUND] = 1;
    assert.deepEqual(order(owned), [["text", 2], ["badge", 2], ["end", 2]]);
});
