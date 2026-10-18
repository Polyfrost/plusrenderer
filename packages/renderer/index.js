// WebGL2 backend. No depth test: the core's back-to-front order is exact.

const VERTEX_SHADER = `#version 300 es
in vec2 a_position;
in vec2 a_uv;
in vec4 a_color;
uniform vec2 u_viewport;
out vec2 v_uv;
out vec3 v_shade;
void main() {
    vec2 clip = a_position / u_viewport * 2.0 - 1.0;
    gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
    v_uv = a_uv;
    // Colours arrive as a little-endian 0xAARRGGBB word, so the bytes read BGRA.
    v_shade = a_color.bgr;
}`;

const FRAGMENT_SHADER = `#version 300 es
precision mediump float;
in vec2 v_uv;
in vec3 v_shade;
uniform sampler2D u_texture;
out vec4 fragColor;
void main() {
    vec4 texel = texture(u_texture, v_uv);
    if (texel.a == 0.0) discard;
    fragColor = vec4(texel.rgb * v_shade, texel.a);
}`;

export const defaultWasmUrl = new URL("./plusrenderer.wasm", import.meta.url);

function compile(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        throw new Error(gl.getShaderInfoLog(shader));
    }
    return shader;
}

export class Renderer {
    static async create(canvas, wasmUrl = defaultWasmUrl) {
        const gl = canvas.getContext("webgl2", {
            alpha: true,
            antialias: false,
            premultipliedAlpha: false,
        });
        if (!gl) throw new Error("WebGL2 is not available");

        // Not instantiateStreaming: static servers often get the application/wasm type wrong.
        const bytes = await (await fetch(wasmUrl)).arrayBuffer();
        const { instance } = await WebAssembly.instantiate(bytes, {});
        return new Renderer(canvas, gl, instance.exports);
    }

    constructor(canvas, gl, wasm) {
        this.canvas = canvas;
        this.gl = gl;
        this.wasm = wasm;
        this.handle = wasm.renderer_new();
        this.textures = [];

        const program = gl.createProgram();
        gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERTEX_SHADER));
        gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER));
        gl.linkProgram(program);
        if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
            throw new Error(gl.getProgramInfoLog(program));
        }
        this.program = program;
        this.viewport = gl.getUniformLocation(program, "u_viewport");

        this.vao = gl.createVertexArray();
        gl.bindVertexArray(this.vao);
        this.attributes = {
            position: this.attribute(program, "a_position", 2, gl.FLOAT, false),
            uv: this.attribute(program, "a_uv", 2, gl.FLOAT, false),
            color: this.attribute(program, "a_color", 4, gl.UNSIGNED_BYTE, true),
        };
        this.indexBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.indexBuffer);
        gl.bindVertexArray(null);

        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    }

    attribute(program, name, size, type, normalized) {
        const gl = this.gl;
        const location = gl.getAttribLocation(program, name);
        const buffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
        gl.enableVertexAttribArray(location);
        gl.vertexAttribPointer(location, size, type, normalized, 0, 0);
        return buffer;
    }

    // Never cache views: they go stale when wasm memory grows.
    memory() {
        return this.wasm.memory.buffer;
    }

    withStrings(strings, fn) {
        const args = [];
        for (const s of strings) {
            const bytes = new TextEncoder().encode(s);
            const ptr = this.wasm.alloc(bytes.length);
            new Uint8Array(this.memory(), ptr, bytes.length).set(bytes);
            args.push(ptr, bytes.length);
        }

        try {
            return fn(...args);
        } finally {
            for (let i = 0; i < args.length; i += 2) this.wasm.dealloc(args[i], args[i + 1]);
        }
    }

    check(code) {
        if (code >= 0) return;
        const ptr = this.wasm.renderer_error(this.handle);
        const len = this.wasm.renderer_error_len(this.handle);
        throw new Error(new TextDecoder().decode(new Uint8Array(this.memory(), ptr, len)));
    }

    // Cosmetics hang off the nearest lower slot with the bone they name, so the player goes lowest.

    loadPlayer(slot, slim) {
        this.wasm.renderer_load_player(this.handle, slot, slim ? 1 : 0);
    }

    loadCosmetic(slot, json, bodySlot, image, anchor = "") {
        this.check(this.withStrings([json, bodySlot, anchor], (ptr, len, bodyPtr, bodyLen, anchorPtr, anchorLen) =>
            this.wasm.renderer_load_cosmetic(
                this.handle, slot, ptr, len, bodyPtr, bodyLen, anchorPtr, anchorLen, image.width, image.height,
            ),
        ));
        this.setTexture(slot, image);
    }

    // Animates if `sheet` stacks frames of the still's size, and `sheetName` may set the pace.
    loadCape(slot, still, sheet, sheetName = "") {
        const frames = this.withStrings([sheetName], (ptr, len) =>
            this.wasm.renderer_load_cape(
                this.handle, slot, still.width, still.height,
                sheet?.width ?? 0, sheet?.height ?? 0, ptr, len,
            ),
        );
        this.setTexture(slot, frames > 1 ? sheet : still);
    }

    loadAnimation(slot, json) {
        this.check(this.withStrings([json], (ptr, len) =>
            this.wasm.renderer_load_animation(this.handle, slot, ptr, len),
        ));
    }

    // Indices into `paths` of [geometry, animation, texture, cape sheet], or -1.
    bundleParts(paths) {
        return this.withStrings([paths.join("\n")], (ptr, len) =>
            [0, 1, 2, 3].map((kind) => this.wasm.bundle_part(ptr, len, kind)),
        );
    }

    play(slot, at) {
        this.wasm.renderer_play(this.handle, slot, at);
    }

    stop(slot) {
        this.wasm.renderer_stop(this.handle, slot);
    }

    setVisible(slot, visible) {
        this.wasm.renderer_set_visible(this.handle, slot, visible ? 1 : 0);
    }

    clear(slot) {
        this.wasm.renderer_clear(this.handle, slot);
        if (this.textures[slot]) this.gl.deleteTexture(this.textures[slot]);
        this.textures[slot] = undefined;
    }

    // Light level 0..15. Glowing bones stay lit.
    setLightLevel(level) {
        this.wasm.renderer_set_light_level(this.handle, level);
    }

    setTexture(slot, image) {
        const gl = this.gl;
        this.textures[slot] ??= gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, this.textures[slot]);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);

        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }

    // ponytail: supersamples via an oversized canvas. Use a framebuffer if the downscale or phone cost matters.
    resize(scale = 2) {
        const width = Math.round(this.canvas.clientWidth * scale);
        const height = Math.round(this.canvas.clientHeight * scale);
        if (this.canvas.width !== width || this.canvas.height !== height) {
            this.canvas.width = width;
            this.canvas.height = height;
        }
        return [width, height];
    }

    draw(camera, seconds) {
        const gl = this.gl;
        const [width, height] = this.resize();

        const indexCount = this.wasm.renderer_build(
            this.handle,
            camera.yaw,
            camera.pitch,
            camera.distance,
            camera.targetY,
            width,
            height,
            seconds,
        );

        gl.viewport(0, 0, width, height);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        if (indexCount <= 0) return { triangles: 0, draws: 0 };

        const vertexCount = this.wasm.renderer_vertex_count(this.handle);
        const memory = this.memory();
        const view = (Type, ptr, count) => new Type(memory, ptr, count);

        gl.bindVertexArray(this.vao);
        this.upload(gl.ARRAY_BUFFER, this.attributes.position,
            view(Float32Array, this.wasm.renderer_positions(this.handle), vertexCount * 2));
        this.upload(gl.ARRAY_BUFFER, this.attributes.uv,
            view(Float32Array, this.wasm.renderer_uvs(this.handle), vertexCount * 2));
        this.upload(gl.ARRAY_BUFFER, this.attributes.color,
            view(Uint32Array, this.wasm.renderer_colors(this.handle), vertexCount));
        this.upload(gl.ELEMENT_ARRAY_BUFFER, this.indexBuffer,
            view(Uint16Array, this.wasm.renderer_indices(this.handle), indexCount));

        // texture (= slot), first index, index count
        const runs = view(Uint32Array, this.wasm.renderer_runs(this.handle),
            this.wasm.renderer_run_count(this.handle) * 3);

        gl.useProgram(this.program);
        gl.uniform2f(this.viewport, width, height);
        let triangles = 0;
        let draws = 0;

        for (let i = 0; i < runs.length; i += 3) {
            const texture = this.textures[runs[i]];
            if (!texture) continue;
            gl.bindTexture(gl.TEXTURE_2D, texture);
            gl.drawElements(gl.TRIANGLES, runs[i + 2], gl.UNSIGNED_SHORT, runs[i + 1] * 2);
            triangles += runs[i + 2] / 3;
            draws += 1;
        }

        gl.bindVertexArray(null);
        return { triangles, draws };
    }

    upload(target, buffer, data) {
        const gl = this.gl;
        gl.bindBuffer(target, buffer);
        gl.bufferData(target, data, gl.DYNAMIC_DRAW);
    }
}
