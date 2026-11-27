import { OUT, TEXT_MODE_VANILLA } from "./nameplate.js";

const CANVAS_VERTEX = `#version 300 es
precision highp float;
precision highp int;
in vec2 a_pos;
in vec4 a_rect;
in vec4 a_color;
in ivec4 a_info;
uniform mat4 ProjMat;
uniform mat4 ModelViewMat;
out vec2 v_pos;
flat out vec4 v_rect;
flat out vec4 v_color;
flat out ivec4 v_info;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(a_pos, 0.0, 1.0);
    v_pos = a_pos;
    v_rect = a_rect;
    v_color = a_color.bgra;
    v_info = a_info;
}`;

const CANVAS_FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
in vec2 v_pos;
flat in vec4 v_rect;
flat in vec4 v_color;
flat in ivec4 v_info;
uniform sampler2D u_texels;
out vec4 fragColor;
void main() {
    int kind = v_info.x;
    if (kind == 0) {
        fragColor = v_color;
        return;
    }
    ivec2 p = ivec2(floor(v_pos));
    int i = p.x - int(v_rect.x);
    int j = p.y - int(v_rect.y);
    int width = int(v_rect.z - v_rect.x);
    int aux = v_info.y;
    bool mirror = (aux & 0x20000) != 0;
    vec4 texel;
    if (kind == 1) {
        int along = (aux & 0x10000) != 0 ? j : (mirror ? width - 1 - i : i);
        int phase = (aux << 16) >> 16;
        int len = max(v_info.z, 1);
        int at = ((along + phase) % len + len) % len;
        texel = texelFetch(u_texels, ivec2(at, 0), 0).bgra;
    } else {
        int w = aux & 0xffff;
        int sx = mirror ? w - 1 - i : i;
        texel = texelFetch(u_texels, ivec2(sx, j), 0).bgra;
    }
    fragColor = vec4(texel.rgb, texel.a * v_color.a);
}`;

const IMAGE_VERTEX = `#version 300 es
precision highp float;
in vec2 a_pos;
in vec2 a_uv;
uniform mat4 ProjMat;
uniform mat4 ModelViewMat;
out vec2 v_uv;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(a_pos, 0.0, 1.0);
    v_uv = a_uv;
}`;

const IMAGE_FRAGMENT = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_image;
uniform vec4 u_tint;
out vec4 fragColor;
void main() {
    fragColor = texture(u_image, v_uv) * u_tint;
}`;

const FLOATS_PER_VERTEX = 11;
const VERTEX_BYTES = FLOATS_PER_VERTEX * 4;
const VERTICES_PER_OP = 6;
const MATERIAL_VERTEX_FLOATS = 9;
const UNIFORM_BINDING = 0;
const UNIFORM_ATLAS = 28;
const ATLAS_SLOTS = 4;
const ATLAS_TEXTURE = "atlas";
const CORNERS = [0, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 1];

function columnEdge(x) {
    return Math.ceil(x - 0.5) | 0;
}

function rowEdge(y) {
    return Math.floor(y + 0.5) | 0;
}

function toggle(gl, cap, on) {
    if (on) gl.enable(cap);
    else gl.disable(cap);
}

function compile(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        const log = gl.getShaderInfoLog(shader);
        gl.deleteShader(shader);
        throw new Error(log);
    }
    return shader;
}

function link(gl, vertex, fragment) {
    const program = gl.createProgram();
    const vs = compile(gl, gl.VERTEX_SHADER, vertex);
    const fs = compile(gl, gl.FRAGMENT_SHADER, fragment);
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        const log = gl.getProgramInfoLog(program);
        gl.deleteProgram(program);
        throw new Error(log);
    }
    return program;
}

function textureKey(texture) {
    if (texture == null) return "";
    if (typeof texture === "string") return texture;
    if (texture.kind && texture.name !== undefined) return `${texture.kind}:${texture.name}`;
    if (texture.kind && texture.index !== undefined) return `${texture.kind}:${texture.index}`;
    const [kind] = Object.keys(texture);
    return `${kind.toLowerCase()}:${texture[kind]}`;
}

export function disjointRects(rects) {
    const live = rects.filter(([x0, y0, x1, y1]) => x1 > x0 && y1 > y0);
    if (live.length <= 1) return live;
    const ys = [...new Set(live.flatMap((r) => [r[1], r[3]]))].sort((a, b) => a - b);
    const result = [];
    let open = [];
    for (let j = 0; j + 1 < ys.length; j++) {
        const y0 = ys[j];
        const y1 = ys[j + 1];
        const spans = live
            .filter((r) => r[1] <= y0 && r[3] >= y1)
            .map((r) => [r[0], r[2]])
            .sort((a, b) => a[0] - b[0]);
        const merged = [];
        for (const [x0, x1] of spans) {
            const last = merged[merged.length - 1];
            if (last && x0 <= last[1]) last[1] = Math.max(last[1], x1);
            else merged.push([x0, x1]);
        }
        const next = [];
        for (const [x0, x1] of merged) {
            let rect = open.find((r) => r[0] === x0 && r[2] === x1 && r[3] === y0);
            if (rect) {
                rect[3] = y1;
            } else {
                rect = [x0, y0, x1, y1];
                result.push(rect);
            }
            next.push(rect);
        }
        open = next;
    }
    return result;
}

export class NameplateDrawer {
    constructor(gl, nameplates, { light = [1, 1, 1, 1], targetSize = null } = {}) {
        this.gl = gl;
        this.nameplates = nameplates;
        this.light = Float32Array.from(light);
        this.targetSize = targetSize;
        this.depth = 0;
        this.saved = null;
        this.begin();
        try {
            this.setup();
        } finally {
            this.end();
        }
    }

    setup() {
        const gl = this.gl;
        this.canvasProgram = link(gl, CANVAS_VERTEX, CANVAS_FRAGMENT);
        this.canvasUniforms = {
            proj: gl.getUniformLocation(this.canvasProgram, "ProjMat"),
            view: gl.getUniformLocation(this.canvasProgram, "ModelViewMat"),
            texels: gl.getUniformLocation(this.canvasProgram, "u_texels"),
        };
        this.canvasVao = gl.createVertexArray();
        this.canvasBuffer = gl.createBuffer();
        gl.bindVertexArray(this.canvasVao);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.canvasBuffer);
        const attrib = (name, size, type, offset, integer = false, normalized = false) => {
            const location = gl.getAttribLocation(this.canvasProgram, name);
            gl.enableVertexAttribArray(location);
            if (integer) gl.vertexAttribIPointer(location, size, type, VERTEX_BYTES, offset);
            else gl.vertexAttribPointer(location, size, type, normalized, VERTEX_BYTES, offset);
        };
        attrib("a_pos", 2, gl.FLOAT, 0);
        attrib("a_rect", 4, gl.FLOAT, 8);
        attrib("a_color", 4, gl.UNSIGNED_BYTE, 24, false, true);
        attrib("a_info", 4, gl.INT, 28, true);
        gl.bindVertexArray(null);
        this.vertices = new ArrayBuffer(VERTEX_BYTES * VERTICES_PER_OP * 64);
        this.viewVertices();

        this.imageProgram = link(gl, IMAGE_VERTEX, IMAGE_FRAGMENT);
        this.imageUniforms = {
            proj: gl.getUniformLocation(this.imageProgram, "ProjMat"),
            view: gl.getUniformLocation(this.imageProgram, "ModelViewMat"),
            image: gl.getUniformLocation(this.imageProgram, "u_image"),
            tint: gl.getUniformLocation(this.imageProgram, "u_tint"),
        };
        this.imageVao = gl.createVertexArray();
        this.imageBuffer = gl.createBuffer();
        gl.bindVertexArray(this.imageVao);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.imageBuffer);
        for (const [name, offset] of [["a_pos", 0], ["a_uv", 8]]) {
            const location = gl.getAttribLocation(this.imageProgram, name);
            gl.enableVertexAttribArray(location);
            gl.vertexAttribPointer(location, 2, gl.FLOAT, false, 16, offset);
        }
        gl.bindVertexArray(null);
        this.imageVertices = new Float32Array(24);
        this.imageTextures = new WeakMap();
        this.ownedImageTextures = new Set();

        this.materialVao = gl.createVertexArray();
        this.materialBuffer = gl.createBuffer();
        this.materialVertices = new Float32Array(MATERIAL_VERTEX_FLOATS * 6);
        this.uniformBuffer = gl.createBuffer();
        this.programs = new Map();
        this.assetShaders = new Map();
        this.textures = new Map();
        this.atlasSprites = null;

        this.empty = gl.createTexture();
        this.bindTexture(0, this.empty);
        this.unpack();
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(4));
        this.nearest();

        this.texels = new Map();
        this.texelLayout = -1;

        this.proj = new Float32Array(16);
        this.view = new Float32Array(16);
        this.context = {
            tag: 0,
            status: 0,
            mode: TEXT_MODE_VANILLA,
            color: 0,
            shadow: 0,
            shadowUse: 0,
            outline: 0,
            letterCount: 0,
            letters: null,
            lettersAt: 0,
            colors: null,
            colorsAt: 0,
            colorCount: 0,
            plate: [0, 0],
            ownsBackground: false,
            badge: { visible: false, x: 0, y: 0, opacity: 0, tint: 0, decorationKind: 0, decorationRgb: 0 },
            row: null,
            transform: null,
            drawer: this,
        };
    }

    begin() {
        if (this.depth++ > 0) return;
        const gl = this.gl;
        this.saved = {
            blend: gl.isEnabled(gl.BLEND),
            depthTest: gl.isEnabled(gl.DEPTH_TEST),
            cullFace: gl.isEnabled(gl.CULL_FACE),
            scissorTest: gl.isEnabled(gl.SCISSOR_TEST),
            scissorBox: Array.from(gl.getParameter(gl.SCISSOR_BOX)),
            equationRgb: gl.getParameter(gl.BLEND_EQUATION_RGB),
            equationAlpha: gl.getParameter(gl.BLEND_EQUATION_ALPHA),
            srcRgb: gl.getParameter(gl.BLEND_SRC_RGB),
            dstRgb: gl.getParameter(gl.BLEND_DST_RGB),
            srcAlpha: gl.getParameter(gl.BLEND_SRC_ALPHA),
            dstAlpha: gl.getParameter(gl.BLEND_DST_ALPHA),
            program: gl.getParameter(gl.CURRENT_PROGRAM),
            vao: gl.getParameter(gl.VERTEX_ARRAY_BINDING),
            arrayBuffer: gl.getParameter(gl.ARRAY_BUFFER_BINDING),
            uniformBuffer: gl.getParameter(gl.UNIFORM_BUFFER_BINDING),
            uniformBinding: gl.getIndexedParameter(gl.UNIFORM_BUFFER_BINDING, UNIFORM_BINDING),
            uniformStart: gl.getIndexedParameter(gl.UNIFORM_BUFFER_START, UNIFORM_BINDING),
            uniformSize: gl.getIndexedParameter(gl.UNIFORM_BUFFER_SIZE, UNIFORM_BINDING),
            activeTexture: gl.getParameter(gl.ACTIVE_TEXTURE),
            textures: new Map(),
            samplers: new Map(),
            unpack: null,
        };
    }

    end() {
        if (--this.depth > 0) return;
        const gl = this.gl;
        const s = this.saved;
        this.saved = null;
        gl.useProgram(s.program);
        gl.bindVertexArray(s.vao);
        gl.bindBuffer(gl.ARRAY_BUFFER, s.arrayBuffer);
        if (s.uniformBinding && s.uniformSize > 0) {
            gl.bindBufferRange(gl.UNIFORM_BUFFER, UNIFORM_BINDING, s.uniformBinding, s.uniformStart, s.uniformSize);
        } else {
            gl.bindBufferBase(gl.UNIFORM_BUFFER, UNIFORM_BINDING, s.uniformBinding);
        }
        gl.bindBuffer(gl.UNIFORM_BUFFER, s.uniformBuffer);
        for (const [unit, texture] of s.textures) {
            gl.activeTexture(gl.TEXTURE0 + unit);
            gl.bindTexture(gl.TEXTURE_2D, texture);
        }
        for (const [unit, sampler] of s.samplers) gl.bindSampler(unit, sampler);
        gl.activeTexture(s.activeTexture);
        if (s.unpack) {
            for (const [name, value] of s.unpack.params) gl.pixelStorei(name, value);
            gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, s.unpack.buffer);
        }
        toggle(gl, gl.BLEND, s.blend);
        toggle(gl, gl.DEPTH_TEST, s.depthTest);
        toggle(gl, gl.CULL_FACE, s.cullFace);
        toggle(gl, gl.SCISSOR_TEST, s.scissorTest);
        gl.scissor(s.scissorBox[0], s.scissorBox[1], s.scissorBox[2], s.scissorBox[3]);
        gl.blendEquationSeparate(s.equationRgb, s.equationAlpha);
        gl.blendFuncSeparate(s.srcRgb, s.dstRgb, s.srcAlpha, s.dstAlpha);
    }

    bindTexture(unit, texture) {
        const gl = this.gl;
        gl.activeTexture(gl.TEXTURE0 + unit);
        const textures = this.saved.textures;
        if (!textures.has(unit)) textures.set(unit, gl.getParameter(gl.TEXTURE_BINDING_2D));
        const samplers = this.saved.samplers;
        if (!samplers.has(unit)) {
            samplers.set(unit, gl.getParameter(gl.SAMPLER_BINDING));
            gl.bindSampler(unit, null);
        }
        gl.bindTexture(gl.TEXTURE_2D, texture);
    }

    unpack() {
        const s = this.saved;
        if (s.unpack) return;
        const gl = this.gl;
        const wanted = [
            [gl.UNPACK_ALIGNMENT, 4],
            [gl.UNPACK_ROW_LENGTH, 0],
            [gl.UNPACK_SKIP_ROWS, 0],
            [gl.UNPACK_SKIP_PIXELS, 0],
            [gl.UNPACK_IMAGE_HEIGHT, 0],
            [gl.UNPACK_SKIP_IMAGES, 0],
            [gl.UNPACK_FLIP_Y_WEBGL, false],
            [gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false],
            [gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE],
        ];
        s.unpack = {
            buffer: gl.getParameter(gl.PIXEL_UNPACK_BUFFER_BINDING),
            params: wanted.map(([name]) => [name, gl.getParameter(name)]),
        };
        gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, null);
        for (const [name, value] of wanted) gl.pixelStorei(name, value);
    }

    quads() {
        this.gl.disable(this.gl.CULL_FACE);
    }

    viewVertices() {
        this.vertexFloats = new Float32Array(this.vertices);
        this.vertexInts = new Int32Array(this.vertices);
        this.vertexWords = new Uint32Array(this.vertices);
    }

    nearest() {
        const gl = this.gl;
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }

    setLight(rgba) {
        this.light.set(rgba);
    }

    setAssetShaders(assetId, shaders) {
        this.begin();
        try {
            this.loadShaders(assetId, shaders);
        } finally {
            this.end();
        }
    }

    loadShaders(assetId, shaders) {
        const byMaterial = [];
        const held = new Set();
        for (const shader of Array.isArray(shaders) ? shaders : shaders.shaders) {
            let entry = this.programs.get(shader.key);
            if (!entry) {
                entry = this.materialProgram(shader);
                entry.key = shader.key;
                entry.refs = 0;
                this.programs.set(shader.key, entry);
            }
            if (!held.has(entry)) {
                held.add(entry);
                entry.refs++;
            }
            byMaterial[shader.material] = {
                entry,
                additive: shader.blend === "additive" || shader.blend === 1,
                samplers: shader.samplers.map((sampler, slot) => {
                    const key = textureKey(sampler.texture);
                    return {
                        unit: sampler.unit,
                        key,
                        slot,
                        atlas: key.startsWith("atlas:") ? key.slice(6) : null,
                    };
                }),
            };
        }
        this.releaseShaders(assetId);
        this.assetShaders.set(assetId, byMaterial);
    }

    releaseShaders(assetId) {
        const byMaterial = this.assetShaders.get(assetId);
        if (!byMaterial) return;
        this.assetShaders.delete(assetId);
        for (const entry of new Set(byMaterial.map((material) => material?.entry))) {
            if (!entry || --entry.refs > 0) continue;
            this.gl.deleteProgram(entry.program);
            this.programs.delete(entry.key);
        }
    }

    forgetAsset(assetId) {
        this.releaseShaders(assetId);
        const prefix = `bundle:${assetId}:`;
        for (const [key, texture] of this.textures) {
            if (!key.startsWith(prefix)) continue;
            this.gl.deleteTexture(texture);
            this.textures.delete(key);
        }
    }

    materialProgram(shader) {
        const gl = this.gl;
        const program = link(gl, shader.vertex, shader.fragment);
        const block = gl.getUniformBlockIndex(program, "NameplateMaterial");
        if (block !== gl.INVALID_INDEX) gl.uniformBlockBinding(program, block, UNIFORM_BINDING);
        const location = (name) => gl.getUniformLocation(program, name);
        gl.useProgram(program);
        for (const sampler of shader.samplers) gl.uniform1i(location(sampler.name), sampler.unit);
        return {
            program,
            proj: location("ProjMat"),
            view: location("ModelViewMat"),
            modulator: location("ColorModulator"),
            light: location("NameplateLight"),
            screen: location("ScreenSize"),
            position: gl.getAttribLocation(program, "Position"),
            uv: gl.getAttribLocation(program, "UV0"),
            color: gl.getAttribLocation(program, "Color"),
        };
    }

    setTexture(key, source, width = 0, height = 0) {
        this.begin();
        try {
            this.uploadTexture(key, source, width, height);
        } finally {
            this.end();
        }
    }

    uploadTexture(key, source, width, height) {
        const gl = this.gl;
        let texture = this.textures.get(key);
        if (!texture) {
            texture = gl.createTexture();
            this.textures.set(key, texture);
        }
        this.bindTexture(0, texture);
        this.unpack();
        if (ArrayBuffer.isView(source)) {
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE,
                new Uint8Array(source.buffer, source.byteOffset, source.byteLength));
        } else {
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
        }
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
    }

    setVanillaTexture(name, source, width, height) {
        this.setTexture(`vanilla:${name}`, source, width, height);
    }

    setAtlas(source, width, height, sprites) {
        this.setTexture(ATLAS_TEXTURE, source, width, height);
        this.atlasSprites = sprites ?? null;
    }

    atlasRect(name) {
        const sprites = this.atlasSprites;
        if (!sprites) return null;
        const names = name.includes(":") ? [name] : [name, `minecraft:${name}`];
        for (const candidate of names) {
            const rect = typeof sprites === "function"
                ? sprites(candidate)
                : sprites instanceof Map ? sprites.get(candidate) : sprites[candidate];
            if (rect && rect.length >= 4) return rect;
        }
        return null;
    }

    fillAtlas(material, floats, at, words) {
        let missing = 0;
        for (const sampler of material.samplers) {
            if (sampler.atlas === null) continue;
            const word = UNIFORM_ATLAS + sampler.slot * 4;
            const fits = sampler.slot < ATLAS_SLOTS && word + 4 <= words;
            const rect = fits ? this.atlasRect(sampler.atlas) : null;
            if (!rect) missing |= 1 << sampler.slot;
            if (!fits) continue;
            for (let c = 0; c < 4; c++) floats[at + word + c] = rect ? rect[c] : 0;
        }
        return missing;
    }

    setBundleTextures(assetId, images) {
        images.forEach((image, index) => {
            if (!image) return;
            if (image.rgba) this.setTexture(`bundle:${assetId}:${index}`, image.rgba, image.width, image.height);
            else this.setTexture(`bundle:${assetId}:${index}`, image);
        });
    }

    forgetImage(source) {
        const texture = this.imageTextures.get(source);
        if (texture) {
            this.gl.deleteTexture(texture);
            this.ownedImageTextures.delete(texture);
        }
        this.imageTextures.delete(source);
    }

    texelTexture(id) {
        const gl = this.gl;
        const layout = this.nameplates.stripLayout();
        if (layout !== this.texelLayout) {
            this.texelLayout = layout;
            for (const entry of this.texels.values()) entry.stale = true;
        }
        let entry = this.texels.get(id);
        if (entry && !entry.stale) return entry;
        const texels = this.nameplates.stripTexels(id);
        if (!entry) {
            entry = { texture: gl.createTexture(), width: 0, height: 0, length: 0, stale: false };
            this.texels.set(id, entry);
        }
        entry.stale = false;
        entry.length = texels.length;
        return entry;
    }

    uploadTexels(entry, id, width) {
        const gl = this.gl;
        const texels = this.nameplates.stripTexels(id);
        const w = Math.max(1, width || texels.length);
        const h = Math.max(1, Math.floor(texels.length / w));
        this.bindTexture(0, entry.texture);
        const bytes = new Uint8Array(texels.buffer, texels.byteOffset, Math.min(texels.length, w * h) * 4);
        this.unpack();
        if (entry.width === w && entry.height === h) {
            gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
        } else {
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes.length ? bytes : null);
            this.nearest();
            entry.width = w;
            entry.height = h;
        }
        entry.uploaded = this.texelLayout;
    }

    bindTexels(id, width) {
        const entry = this.texelTexture(id);
        if (entry.uploaded !== this.texelLayout || entry.width !== Math.max(1, width || entry.length)) {
            this.uploadTexels(entry, id, width);
        }
        this.bindTexture(0, entry.texture);
        return entry;
    }

    matrices(transform, viewport) {
        const [width, height] = viewport;
        const p = this.proj;
        p.fill(0);
        p[0] = 2 / width;
        p[5] = -2 / height;
        p[10] = -1;
        p[12] = -1;
        p[13] = 1;
        p[15] = 1;
        const v = this.view;
        const scale = transform.scale ?? 1;
        v.fill(0);
        v[0] = scale;
        v[5] = scale;
        v[10] = 1;
        v[12] = transform.x ?? 0;
        v[13] = transform.y ?? 0;
        v[15] = 1;
    }

    reserve(ops) {
        const needed = ops * VERTICES_PER_OP * VERTEX_BYTES;
        if (this.vertices.byteLength >= needed) return;
        let size = this.vertices.byteLength;
        while (size < needed) size *= 2;
        this.vertices = new ArrayBuffer(size);
        this.viewVertices();
    }

    writeOp(slot, out, at, kind, aux, length) {
        const f = this.vertexFloats;
        const n = this.vertexInts;
        const u = this.vertexWords;
        const x0 = out[at];
        const y0 = out[at + 1];
        const x1 = out[at + 2];
        const y1 = out[at + 3];
        const argb = out[at + 4] >>> 0;
        let base = slot * VERTICES_PER_OP * FLOATS_PER_VERTEX;
        for (let c = 0; c < 12; c += 2) {
            f[base] = CORNERS[c] ? x1 : x0;
            f[base + 1] = CORNERS[c + 1] ? y1 : y0;
            f[base + 2] = x0;
            f[base + 3] = y0;
            f[base + 4] = x1;
            f[base + 5] = y1;
            u[base + 6] = argb;
            n[base + 7] = kind;
            n[base + 8] = aux;
            n[base + 9] = length;
            n[base + 10] = 0;
            base += FLOATS_PER_VERTEX;
        }
    }

    blend(additive) {
        const gl = this.gl;
        if (additive) {
            gl.blendEquationSeparate(gl.FUNC_ADD, gl.MAX);
            gl.blendFuncSeparate(gl.SRC_ALPHA, gl.ONE, gl.ONE, gl.ONE);
        } else {
            gl.blendEquationSeparate(gl.FUNC_ADD, gl.FUNC_ADD);
            gl.blendFuncSeparate(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
        }
    }

    floatsOf(out) {
        if (this.floatSource !== out.buffer || this.floatOffset !== out.byteOffset
            || this.floatView.length !== out.length) {
            this.floatSource = out.buffer;
            this.floatOffset = out.byteOffset;
            this.floatView = new Float32Array(out.buffer, out.byteOffset, out.length);
        }
        return this.floatView;
    }

    drawOps(out, opsAt, from, to) {
        if (to <= from) return;
        const gl = this.gl;
        gl.useProgram(this.canvasProgram);
        gl.uniformMatrix4fv(this.canvasUniforms.proj, false, this.proj);
        gl.uniformMatrix4fv(this.canvasUniforms.view, false, this.view);
        gl.uniform1i(this.canvasUniforms.texels, 0);
        gl.bindVertexArray(this.canvasVao);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.canvasBuffer);
        this.reserve(to - from);
        for (let k = from; k < to; k++) {
            const at = opsAt + k * OUT.OP_WORDS;
            const kind = (out[at + OUT.OP_BAND_LAYER_KIND] >> 16) & 0xff;
            const aux = out[at + OUT.OP_AUX1];
            const length = kind === OUT.OP_KIND_STRIP ? this.nameplates.stripTexels(out[at + OUT.OP_AUX0]).length : 0;
            this.writeOp(k - from, out, at, kind, aux, length);
        }
        gl.bufferData(gl.ARRAY_BUFFER, this.vertexFloats, gl.STREAM_DRAW, 0, (to - from) * VERTICES_PER_OP * FLOATS_PER_VERTEX);

        let start = from;
        while (start < to) {
            const at = opsAt + start * OUT.OP_WORDS;
            const kind = (out[at + OUT.OP_BAND_LAYER_KIND] >> 16) & 0xff;
            const texel = kind === OUT.OP_KIND_RECT ? -1 : out[at + OUT.OP_AUX0];
            const additive = kind === OUT.OP_KIND_IMAGE && (out[at + OUT.OP_AUX1] & OUT.AUX_ADDITIVE) !== 0;
            const width = kind === OUT.OP_KIND_IMAGE ? out[at + OUT.OP_AUX1] & OUT.AUX_LOW : 0;
            let end = start + 1;
            while (end < to) {
                const next = opsAt + end * OUT.OP_WORDS;
                const nextKind = (out[next + OUT.OP_BAND_LAYER_KIND] >> 16) & 0xff;
                if (texel < 0 ? nextKind !== OUT.OP_KIND_RECT : nextKind !== kind || out[next + OUT.OP_AUX0] !== texel) break;
                if (kind === OUT.OP_KIND_IMAGE && (out[next + OUT.OP_AUX1] & (OUT.AUX_ADDITIVE | OUT.AUX_LOW)) !== (out[at + OUT.OP_AUX1] & (OUT.AUX_ADDITIVE | OUT.AUX_LOW))) break;
                end++;
            }
            if (texel < 0) {
                this.bindTexture(0, this.empty);
            } else {
                this.bindTexels(texel, width);
            }
            this.blend(additive);
            gl.drawArrays(gl.TRIANGLES, (start - from) * VERTICES_PER_OP, (end - start) * VERTICES_PER_OP);
            start = end;
        }
        gl.bindVertexArray(null);
    }

    clipScissors(out, at, count, viewport, transform, hostScissor) {
        const gl = this.gl;
        const scale = transform.scale ?? 1;
        const tx = transform.x ?? 0;
        const ty = transform.y ?? 0;
        const port = gl.getParameter(gl.VIEWPORT);
        const [vx, vy, vw, vh] = port && port.length === 4 ? port : [0, 0, viewport[0], viewport[1]];
        const kx = vw / viewport[0];
        const ky = vh / viewport[1];
        const rects = [];
        for (let k = 0; k < count; k++) {
            const r = at + k * OUT.CLIP_WORDS;
            const ax = tx + out[r] * scale;
            const ay = ty + out[r + 1] * scale;
            const bx = tx + out[r + 2] * scale;
            const by = ty + out[r + 3] * scale;
            const sx0 = Math.min(ax, bx);
            const sy0 = Math.min(ay, by);
            const sx1 = Math.max(ax, bx);
            const sy1 = Math.max(ay, by);
            rects.push([
                columnEdge(vx + sx0 * kx),
                rowEdge(vy + vh - sy1 * ky),
                columnEdge(vx + sx1 * kx),
                rowEdge(vy + vh - sy0 * ky),
            ]);
        }
        const parts = disjointRects(rects);
        if (!hostScissor) return parts;
        const [bx, by, bw, bh] = hostScissor;
        const bounded = [];
        for (const [x0, y0, x1, y1] of parts) {
            const cx0 = Math.max(x0, bx);
            const cy0 = Math.max(y0, by);
            const cx1 = Math.min(x1, bx + bw);
            const cy1 = Math.min(y1, by + bh);
            if (cx1 > cx0 && cy1 > cy0) bounded.push([cx0, cy0, cx1, cy1]);
        }
        return bounded;
    }

    screenSize(targetSize = null) {
        const gl = this.gl;
        const sized = (size) => Array.isArray(size) || ArrayBuffer.isView(size)
            ? size.length >= 2 && size[0] > 0 && size[1] > 0
            : false;
        if (sized(targetSize)) return [targetSize[0], targetSize[1]];
        const framebuffer = gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING);
        if (framebuffer === null) {
            return [gl.drawingBufferWidth, gl.drawingBufferHeight];
        }
        const known = typeof this.targetSize === "function" ? this.targetSize(framebuffer) : this.targetSize;
        if (sized(known)) return [known[0], known[1]];
        const [x, y, width, height] = gl.getParameter(gl.VIEWPORT);
        return [x + width, y + height];
    }

    drawMaterial(assetId, out, at, viewport, transform, targetSize = null) {
        this.begin();
        try {
            this.renderMaterial(assetId, out, at, viewport, transform, targetSize);
        } finally {
            this.end();
        }
    }

    renderMaterial(assetId, out, at, viewport, transform, targetSize = null) {
        const gl = this.gl;
        const shader = out[at + OUT.MAT_SHADER];
        const word = out[at + OUT.MAT_BAND_LAYER];
        const clipCount = out[at + OUT.MAT_CLIP_COUNT];
        const uniformWords = out[at + OUT.MAT_UNIFORM_WORDS];
        const clipsAt = at + OUT.MAT_FIXED;
        const uniformsAt = clipsAt + clipCount * OUT.CLIP_WORDS;
        const material = this.assetShaders.get(assetId)?.[shader];
        if (!material || uniformWords <= 0) return;
        const entry = material.entry;

        let scissors = null;
        let hostScissorTest = false;
        let hostScissorBox = null;
        if (clipCount > 0) {
            hostScissorTest = gl.isEnabled(gl.SCISSOR_TEST);
            hostScissorBox = Array.from(gl.getParameter(gl.SCISSOR_BOX));
            scissors = this.clipScissors(out, clipsAt, clipCount, viewport, transform,
                hostScissorTest ? hostScissorBox : null);
            if (scissors.length === 0) return;
        }

        const floats = this.floatsOf(out);
        const bodyX = floats[uniformsAt + 4];
        const bodyY = floats[uniformsAt + 5];
        const u0 = floats[uniformsAt] + bodyX;
        const v0 = floats[uniformsAt + 1] + bodyY;
        const u1 = floats[uniformsAt + 2] + bodyX;
        const v1 = floats[uniformsAt + 3] + bodyY;
        const mirrored = (word & OUT.MAT_MIRRORED) !== 0;
        const opacity = ((word >>> OUT.MAT_OPACITY_SHIFT) & 0xff) / 255;
        const x0 = out[at + OUT.MAT_AREA_X0];
        const y0 = out[at + OUT.MAT_AREA_Y0];
        const x1 = out[at + OUT.MAT_AREA_X1];
        const y1 = out[at + OUT.MAT_AREA_Y1];
        const m = this.materialVertices;
        for (let c = 0, base = 0; c < 12; c += 2, base += MATERIAL_VERTEX_FLOATS) {
            const right = CORNERS[c] === 1;
            const bottom = CORNERS[c + 1] === 1;
            m[base] = right ? x1 : x0;
            m[base + 1] = bottom ? y1 : y0;
            m[base + 2] = 0;
            m[base + 3] = right !== mirrored ? u1 : u0;
            m[base + 4] = bottom ? v1 : v0;
            m[base + 5] = 1;
            m[base + 6] = 1;
            m[base + 7] = 1;
            m[base + 8] = opacity;
        }

        this.quads();
        gl.useProgram(entry.program);
        gl.bindVertexArray(this.materialVao);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.materialBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, m, gl.STREAM_DRAW);
        const stride = MATERIAL_VERTEX_FLOATS * 4;
        const pointer = (location, size, offset) => {
            if (location < 0) return;
            gl.enableVertexAttribArray(location);
            gl.vertexAttribPointer(location, size, gl.FLOAT, false, stride, offset);
        };
        pointer(entry.position, 3, 0);
        pointer(entry.uv, 2, 12);
        pointer(entry.color, 4, 20);

        const missingAtlas = this.fillAtlas(material, floats, uniformsAt, uniformWords);
        gl.bindBuffer(gl.UNIFORM_BUFFER, this.uniformBuffer);
        gl.bufferData(gl.UNIFORM_BUFFER, out, gl.STREAM_DRAW, uniformsAt, uniformWords);
        gl.bindBufferBase(gl.UNIFORM_BUFFER, UNIFORM_BINDING, this.uniformBuffer);

        gl.uniformMatrix4fv(entry.proj, false, this.proj);
        gl.uniformMatrix4fv(entry.view, false, this.view);
        if (entry.modulator) gl.uniform4f(entry.modulator, 1, 1, 1, 1);
        if (entry.light) gl.uniform4fv(entry.light, this.light);
        if (entry.screen) {
            const [width, height] = this.screenSize(targetSize);
            gl.uniform2f(entry.screen, width, height);
        }
        for (const sampler of material.samplers) {
            let key = sampler.key;
            if (sampler.atlas !== null) key = (missingAtlas & (1 << sampler.slot)) !== 0 ? null : ATLAS_TEXTURE;
            else if (key.startsWith("bundle:")) key = `bundle:${assetId}:${key.slice(7)}`;
            this.bindTexture(sampler.unit, this.textures.get(key) ?? this.empty);
        }
        gl.activeTexture(gl.TEXTURE0);
        this.blend(material.additive);

        if (scissors) {
            gl.enable(gl.SCISSOR_TEST);
            for (const [x0, y0, x1, y1] of scissors) {
                gl.scissor(x0, y0, x1 - x0, y1 - y0);
                gl.drawArrays(gl.TRIANGLES, 0, 6);
            }
            gl.scissor(hostScissorBox[0], hostScissorBox[1], hostScissorBox[2], hostScissorBox[3]);
            toggle(gl, gl.SCISSOR_TEST, hostScissorTest);
        } else {
            gl.drawArrays(gl.TRIANGLES, 0, 6);
        }
        gl.bindVertexArray(null);
    }

    drawImage(source, x, y, width, height, tint = 0xffffffff) {
        this.begin();
        try {
            this.renderImage(source, x, y, width, height, tint);
        } finally {
            this.end();
        }
    }

    renderImage(source, x, y, width, height, tint) {
        const gl = this.gl;
        let texture = typeof WebGLTexture !== "undefined" && source instanceof WebGLTexture
            ? source
            : this.imageTextures.get(source);
        if (!texture) {
            texture = gl.createTexture();
            this.imageTextures.set(source, texture);
            this.ownedImageTextures.add(texture);
            this.bindTexture(0, texture);
            this.unpack();
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
            this.nearest();
        }
        const v = this.imageVertices;
        for (let c = 0, base = 0; c < 12; c += 2, base += 4) {
            v[base] = x + CORNERS[c] * width;
            v[base + 1] = y + CORNERS[c + 1] * height;
            v[base + 2] = CORNERS[c];
            v[base + 3] = CORNERS[c + 1];
        }
        this.quads();
        gl.useProgram(this.imageProgram);
        gl.uniformMatrix4fv(this.imageUniforms.proj, false, this.proj);
        gl.uniformMatrix4fv(this.imageUniforms.view, false, this.view);
        gl.uniform1i(this.imageUniforms.image, 0);
        const t = tint >>> 0;
        gl.uniform4f(this.imageUniforms.tint, ((t >>> 16) & 0xff) / 255, ((t >>> 8) & 0xff) / 255, (t & 0xff) / 255, (t >>> 24) / 255);
        this.bindTexture(0, texture);
        gl.bindVertexArray(this.imageVao);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.imageBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, v, gl.STREAM_DRAW);
        this.blend(false);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
        gl.bindVertexArray(null);
    }

    fillRect(x0, y0, x1, y1, argb) {
        this.begin();
        try {
            this.renderRect(x0, y0, x1, y1, argb);
        } finally {
            this.end();
        }
    }

    renderRect(x0, y0, x1, y1, argb) {
        const gl = this.gl;
        this.quads();
        gl.useProgram(this.canvasProgram);
        gl.uniformMatrix4fv(this.canvasUniforms.proj, false, this.proj);
        gl.uniformMatrix4fv(this.canvasUniforms.view, false, this.view);
        gl.bindVertexArray(this.canvasVao);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.canvasBuffer);
        this.reserve(1);
        const f = this.vertexFloats;
        const n = this.vertexInts;
        const u = this.vertexWords;
        for (let c = 0, base = 0; c < 12; c += 2, base += FLOATS_PER_VERTEX) {
            f[base] = CORNERS[c] ? x1 : x0;
            f[base + 1] = CORNERS[c + 1] ? y1 : y0;
            u[base + 6] = argb >>> 0;
            n[base + 7] = 0;
        }
        gl.bufferData(gl.ARRAY_BUFFER, f, gl.STREAM_DRAW, 0, VERTICES_PER_OP * FLOATS_PER_VERTEX);
        this.blend(false);
        gl.drawArrays(gl.TRIANGLES, 0, VERTICES_PER_OP);
        gl.bindVertexArray(null);
    }

    drawTag(out, tag, options = {}) {
        this.begin();
        try {
            return this.renderTag(out, tag, options);
        } finally {
            this.end();
        }
    }

    renderTag(out, tag, { assetId, row = null, transform = {}, viewport, text, badge, background, targetSize = null }) {
        const gl = this.gl;
        const view = viewport ?? [gl.drawingBufferWidth, gl.drawingBufferHeight];
        this.matrices(transform, view);
        gl.enable(gl.BLEND);
        gl.disable(gl.DEPTH_TEST);
        this.quads();

        const dir = OUT.OUT_HEADER + tag * OUT.DIR_WORDS;
        const at = out[dir + OUT.DIR_OFFSET];
        const len = out[dir + OUT.DIR_LEN];
        const status = out[dir + OUT.DIR_STATUS];
        const id = assetId ?? (row ? row[OUT.TAG_ASSET] : 0);
        const ctx = this.context;
        ctx.tag = tag;
        ctx.status = status;
        ctx.row = row;
        ctx.transform = transform;
        ctx.assetId = id;

        if (status !== OUT.STATUS_OK || len < OUT.TAG_HEAD) {
            ctx.mode = TEXT_MODE_VANILLA;
            ctx.color = 0;
            ctx.shadow = 0;
            ctx.shadowUse = 0;
            ctx.outline = 0;
            ctx.letterCount = 0;
            ctx.colorCount = 0;
            ctx.plate[0] = 0;
            ctx.plate[1] = 0;
            ctx.ownsBackground = false;
            ctx.badge.visible = false;
            background?.(ctx);
            text?.(ctx);
            return status;
        }

        const opCount = out[at + OUT.HEAD_OPS];
        const materialCount = out[at + OUT.HEAD_MATERIALS];
        const materialWords = out[at + OUT.HEAD_MATERIAL_WORDS];
        const eventCount = out[at + OUT.HEAD_EVENTS];
        const opsAt = at + OUT.TAG_HEAD;
        const materialsAt = opsAt + opCount * OUT.OP_WORDS;
        const textAt = materialsAt + materialWords;
        const letterCount = out[textAt + OUT.TEXT_LETTER_COUNT];
        const lettersAt = textAt + OUT.TEXT_FIXED;
        const colorsAt = lettersAt + letterCount * OUT.LETTER_WORDS;
        const badgeAt = at + len - eventCount * OUT.EVENT_WORDS - OUT.BADGE_WORDS;

        ctx.plate[0] = out[at + OUT.HEAD_PLATE_DX];
        ctx.plate[1] = out[at + OUT.HEAD_PLATE_DY];
        ctx.ownsBackground = out[at + OUT.HEAD_OWNS_BACKGROUND] !== 0;
        ctx.mode = out[at + OUT.HEAD_TEXT_MODE];
        ctx.color = out[textAt + OUT.TEXT_COLOR] >>> 0;
        ctx.shadow = out[textAt + OUT.TEXT_SHADOW] >>> 0;
        ctx.shadowUse = out[textAt + OUT.TEXT_SHADOW_USE];
        ctx.outline = out[textAt + OUT.TEXT_OUTLINE] >>> 0;
        ctx.letterCount = letterCount;
        ctx.letters = out;
        ctx.lettersAt = lettersAt;
        ctx.colors = out;
        ctx.colorsAt = colorsAt;
        ctx.colorCount = Math.max(0, badgeAt - colorsAt);
        const b = ctx.badge;
        b.opacity = out[badgeAt + OUT.BADGE_OPACITY];
        b.visible = b.opacity > 0;
        b.x = out[badgeAt + OUT.BADGE_X];
        b.y = out[badgeAt + OUT.BADGE_Y];
        b.tint = out[badgeAt + OUT.BADGE_TINT] >>> 0;
        b.decorationKind = (out[badgeAt + OUT.BADGE_DECORATION] >>> OUT.DECORATION_KIND_SHIFT) & 0xff;
        b.decorationRgb = out[badgeAt + OUT.BADGE_DECORATION] & 0xffffff;

        let opCursor = 0;
        let materialAt = materialsAt;
        let backgroundDone = false;
        let textDone = false;
        const finishBackground = () => {
            if (backgroundDone) return;
            backgroundDone = true;
            if (!ctx.ownsBackground) background?.(ctx);
        };
        const finishBelowText = () => {
            if (textDone) return;
            finishBackground();
            textDone = true;
            text?.(ctx);
            badge?.(ctx);
        };
        const bandOfOp = (k) => out[opsAt + k * OUT.OP_WORDS + OUT.OP_BAND_LAYER_KIND] & 0xff;
        const flushOps = (until) => {
            while (opCursor < until) {
                let end = opCursor;
                if (!textDone) {
                    const boundary = backgroundDone ? OUT.BAND_OVER_TEXT : OUT.BAND_OVER_BACKGROUND;
                    while (end < until && bandOfOp(end) < boundary) end++;
                    if (end === opCursor) {
                        if (backgroundDone) finishBelowText();
                        else finishBackground();
                        continue;
                    }
                } else {
                    end = until;
                }
                this.drawOps(out, opsAt, opCursor, end);
                opCursor = end;
            }
        };
        for (let m = 0; m < materialCount; m++) {
            const run = out[materialAt + OUT.MAT_RUN];
            flushOps(Math.min(run, opCount));
            const band = out[materialAt + OUT.MAT_BAND_LAYER] & 0xff;
            if (band >= OUT.BAND_OVER_BACKGROUND) finishBackground();
            if (band >= OUT.BAND_OVER_TEXT) finishBelowText();
            this.drawMaterial(id, out, materialAt, view, transform, targetSize);
            materialAt += OUT.MAT_FIXED + out[materialAt + OUT.MAT_CLIP_COUNT] * OUT.CLIP_WORDS
                + out[materialAt + OUT.MAT_UNIFORM_WORDS];
        }
        flushOps(opCount);
        finishBelowText();
        return status;
    }

    dispose() {
        const gl = this.gl;
        for (const entry of this.programs.values()) gl.deleteProgram(entry.program);
        for (const texture of this.textures.values()) gl.deleteTexture(texture);
        for (const entry of this.texels.values()) gl.deleteTexture(entry.texture);
        for (const texture of this.ownedImageTextures) gl.deleteTexture(texture);
        gl.deleteProgram(this.canvasProgram);
        gl.deleteProgram(this.imageProgram);
        gl.deleteBuffer(this.canvasBuffer);
        gl.deleteBuffer(this.imageBuffer);
        gl.deleteBuffer(this.materialBuffer);
        gl.deleteBuffer(this.uniformBuffer);
        gl.deleteTexture(this.empty);
        gl.deleteVertexArray(this.canvasVao);
        gl.deleteVertexArray(this.imageVao);
        gl.deleteVertexArray(this.materialVao);
        this.programs.clear();
        this.textures.clear();
        this.texels.clear();
        this.ownedImageTextures.clear();
        this.imageTextures = new WeakMap();
    }
}

export function letterAt(ctx, index) {
    const at = ctx.lettersAt + index * OUT.LETTER_WORDS;
    const out = ctx.letters;
    const scaleRotation = out[at + OUT.LETTER_SCALE_ROT];
    return {
        dx: out[at + OUT.LETTER_DX],
        dy: out[at + OUT.LETTER_DY],
        argb: out[at + OUT.LETTER_ARGB] >>> 0,
        shadow: out[at + OUT.LETTER_SHADOW] >>> 0,
        opacity: out[at + OUT.LETTER_OPACITY],
        scaleQ: scaleRotation & 0xff,
        rotation: scaleRotation >> 8,
        outline: out[at + OUT.LETTER_OUTLINE] >>> 0,
    };
}
