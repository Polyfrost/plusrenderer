import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative, sep } from "node:path";
import { AUTHORING_CAPABILITIES, CosmeticEngine } from "./cosmetic.js";
import { storeZip } from "./nameplate.js";
import {
    advanceSession, PreviewPlayer, EMOTE_CANCEL, EMOTE_CANCEL_CROUCHING, EMOTE_CANCEL_WALKING, EMOTE_ENDED,
    EMOTE_HELD, EVENT_EQUIP, EVENT_JUMP, effectView, emoteSeats, emoteSeconds, FADE_SECONDS, FrameClock, freshHolds, genericPetMotion,
    genericPetSpot, holdInput, holdsFinished, holdStep, holdTicks, IDENTITY, invert, markFrom,
    multiply, NO_ROLE, orbitView, pairChoice, pairOrientation, pairSeats, particleBuffers, petInteractions, petSessionSeconds,
    InputLog, PIXELS_PER_BLOCK, PreviewSounds, PreviewStage, projectPoint, QUAD_FLOATS, reactHolds, rootAt, startDelays, clipDelays,
    supportsInteraction, transformPoint,
} from "./preview.js";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = join(here, "..", "..");
const data = join(root, "crates", "core", "tests", "data");
const ORIGIN = rootAt({ x: 0, y: 0, z: 0, yaw: 0 });
const CONTROLLERS = "animation_controllers/latch.animation_controllers.json";

function close(a, b, epsilon = 1e-4) {
    return Math.abs(a - b) <= epsilon;
}

function assertClose(actual, expected, epsilon = 1e-4, what = "") {
    assert.equal(actual.length, expected.length, `${what} length`);
    actual.forEach((value, i) => assert.ok(close(value, expected[i], epsilon), `${what}[${i}]: ${value} vs ${expected[i]}`));
}

function readTree(dir) {
    const out = {};
    const walk = (at) => {
        for (const name of readdirSync(at)) {
            const path = join(at, name);
            if (statSync(path).isDirectory()) walk(path);
            else out[relative(dir, path).split(sep).join("/")] = new Uint8Array(readFileSync(path));
        }
    };
    walk(dir);
    return out;
}

function loadWasm() {
    for (const path of [process.env.PLUSRENDERER_WASM, join(here, "plusrenderer.wasm"), join(root, "target", "wasm32-unknown-unknown", "wasm", "plusrenderer_ffi.wasm")]) {
        if (!path || !existsSync(path)) continue;
        const module = new WebAssembly.Module(readFileSync(path));
        const names = new Set(WebAssembly.Module.exports(module).map((entry) => entry.name));
        if (names.has("renderer_frame_wearers_shaped") && names.has("renderer_build_f64")) return new WebAssembly.Instance(module, {}).exports;
    }
    return null;
}

const wasm = loadWasm();
const engineTest = wasm ? test : test.skip;
const engine = () => new CosmeticEngine(wasm);
const json = (value) => new TextEncoder().encode(JSON.stringify(value));
const decode = (bytes) => JSON.parse(new TextDecoder().decode(bytes));
const wings = () => readTree(join(data, "sample_wings"));
const spark = () => readTree(join(data, "sample_spark"));
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function parse(files) {
    const asset = engine().parse(files, { mode: "authoring", capabilities: AUTHORING_CAPABILITIES });
    assert.equal(asset.status, "asset", JSON.stringify(asset.issues));
    return asset;
}

function edit(files, path, change) {
    const value = decode(files[path]);
    change(value);
    return { ...files, [path]: json(value) };
}

function clips(id, animations, extra = {}) {
    return json({ format_version: "1.8.0", animations: Object.fromEntries(Object.entries(animations).map(([name, clip]) => [`animation.${id}.${name}`, clip])), ...extra });
}

function swingClip(seconds, extra = {}) {
    return { animation_length: seconds, bones: { wing: { rotation: { "0.0": [0, 0, 0], [String(seconds / 2)]: [0, 0, 80], [String(seconds)]: [0, 0, 0] } } }, ...extra };
}

const DUO_ROLES = [{ id: "a", animation: "left" }, { id: "b", animation: "right", offset: [0.5, 0, 1.5], yaw: 180 }];

function duoFiles({ requires = ["roles"], left = {}, extra = {} } = {}) {
    return {
        "manifest.json": json({ format: 1, kind: "emote", id: "duo", requires, animations: ["duo"], main: "left", roles: DUO_ROLES, ...extra }),
        "animations/duo.animation.json": clips("duo", {
            left: { animation_length: 1, bones: { rightArm: { rotation: [-90, 0, 0] } }, ...left },
            right: { animation_length: 1, bones: { leftArm: { rotation: [-90, 0, 0] } } },
        }),
        "models/duo.geo.json": wings()["models/wings.geo.json"],
    };
}

function duoSoundFiles() {
    const sounds = { sound_effects: { "0.3": { effect: "pop" }, "0.6": { effect: "pop" } } };
    return { ...duoFiles({ requires: ["roles", "sounds"], left: sounds, extra: { main: undefined, sounds: { pop: {} } } }), "sounds/pop.ogg": spark()["sounds/pop.ogg"] };
}

function petFiles(manifest = {}) {
    const tree = wings();
    return {
        "manifest.json": json({
            format: 1, kind: "pet", id: "cat", requires: ["controllers", "pets", "roles"], geometry: "wings", textures: ["wings"], archetype: "walking", animations: ["wings"],
            interactions: { greet: ["a", "b"] },
            roles: [{ interaction: "greet", id: "a", animation: "idle", offset: [0, 0, -0.75] }, { interaction: "greet", id: "b", animation: "droop", offset: [0, 0, 0.75], yaw: 180 }],
            ...manifest,
        }),
        "models/wings.geo.json": tree["models/wings.geo.json"],
        "textures/wings.png": tree["textures/wings.png"],
        "animations/wings.animation.json": tree["animations/wings.animation.json"],
    };
}

function worldSparkFiles() {
    const components = {
        "minecraft:emitter_rate_instant": { num_particles: 4 }, "minecraft:emitter_lifetime_once": { active_time: 0.5 }, "minecraft:emitter_shape_point": {},
        "minecraft:particle_lifetime_expression": { max_lifetime: 2 }, "minecraft:particle_appearance_billboard": { size: [0.25, 0.25] },
    };
    const description = { identifier: "sample:world_spark", basic_render_parameters: { material: "particles_add", texture: "textures/particle/spark" } };
    return { "particles/spark.json": json({ format_version: "1.10.0", particle_effect: { description, components } }), "textures/particle/spark.png": spark()["textures/particle/spark.png"] };
}

function sparkPet(manifest, files = petFiles()) {
    const sparks = { particles: { spark: {} }, effects: { pop: [{ particle: "spark" }] } };
    return { ...edit(files, "manifest.json", (m) => Object.assign(m, sparks, manifest(m))), ...worldSparkFiles() };
}

function latchEmoteFiles(condition, clip = { loop: true, animation_length: 1 }) {
    return {
        "manifest.json": json({ format: 1, kind: "emote", id: "latch", requires: ["controllers"], animations: ["latch"], animationControllers: ["latch"], controllers: ["controller.animation.latch.hurt"] }),
        "animations/latch.animation.json": clips("latch", { wave: { ...clip, bones: { rightArm: { rotation: [-90, 0, 0] } } }, slump: { ...clip, bones: { head: { rotation: [40, 0, 0] } } } }),
        [CONTROLLERS]: latchControllers(condition),
        "models/latch.geo.json": wings()["models/wings.geo.json"],
    };
}

function latchControllers(condition) {
    const states = { default: { animations: ["wave"], transitions: [{ slumped: condition }] }, slumped: { animations: ["slump"] } };
    return json({ format_version: "1.10.0", animation_controllers: { "controller.animation.latch.hurt": { initial_state: "default", states } } });
}

function latchPetFiles(condition) {
    const files = petFiles();
    return {
        "manifest.json": json({
            format: 1, kind: "pet", id: "latch", requires: ["controllers", "pets"], geometry: "wings", textures: ["wings"], archetype: "walking", animations: ["latch"],
            animationControllers: ["latch"], controllers: ["controller.animation.latch.hurt"],
        }),
        "models/wings.geo.json": files["models/wings.geo.json"],
        "textures/wings.png": files["textures/wings.png"],
        "animations/latch.animation.json": clips("latch", { wave: { loop: true, animation_length: 1, bones: { wing: { rotation: [-90, 0, 0] } } }, slump: { loop: true, animation_length: 1, bones: { wing: { rotation: [40, 0, 0] } } } }),
        [CONTROLLERS]: latchControllers(condition),
    };
}

test("emote holds end solo emotes on their clip and duos when both partners are done", () => {
    const solo = freshHolds(1);
    reactHolds(solo, [0], 0.5);
    assert.equal(solo[0].stopping, false);
    reactHolds(solo, [EMOTE_ENDED], 1.6);
    assert.deepEqual([solo[0].stopping, solo[0].stoppedAt], [true, 1.6]);
    assert.deepEqual([holdsFinished(solo, 1.6), holdsFinished(solo, 1.6 + FADE_SECONDS)], [false, true]);
    assert.deepEqual(holdInput(solo[0], 2), { age: 2, held: false, stopped: true });
    const duo = freshHolds(2);
    reactHolds(duo, [EMOTE_ENDED, 0], 1);
    assert.deepEqual([duo[0].holding, duo[0].heldAge], [true, 1]);
    assert.deepEqual(holdInput(duo[0], 1.5), { age: 1, held: true, stopped: false });
    reactHolds(duo, [EMOTE_HELD, EMOTE_ENDED], 1.2);
    assert.deepEqual([duo[1].stopping, duo[0].stopping], [true, false], "the partner was already holding, so the second ends");
    reactHolds(duo, [EMOTE_HELD, EMOTE_ENDED], 1.25);
    assert.equal(duo[0].stopping, true, "the partner is done, so the first stops holding");
    const walking = EMOTE_CANCEL | EMOTE_CANCEL_WALKING;
    const never = () => false;
    assert.equal(holdStep(walking, false, never, never, () => true), "play");
    assert.equal(holdStep(walking, false, never, never, () => false), "end");
    assert.equal(holdStep(walking | EMOTE_CANCEL_CROUCHING, false, never, never, () => true), "end");
    assert.equal(holdStep(EMOTE_HELD, true, () => true, never), "hold");
    assert.equal(holdStep(-1, false, never, never), "end");
    assert.deepEqual(holdTicks(-0.05, 0.12).map((t) => Math.round(t * 100)), [0, 5, 10]);
    assert.deepEqual(holdTicks(0.1, 0.2).map((t) => Math.round(t * 100)), [15, 20]);
    assert.deepEqual(holdTicks(0.2, 0.2), []);
});

test("sessions loop by replaying and the frame clock never goes backwards", () => {
    assert.deepEqual(advanceSession(0.5, 1, true, 0.1), { time: 0.6, replay: false, ended: false });
    assert.deepEqual(advanceSession(0.95, 1, true, 0.1), { time: 0, replay: true, ended: false });
    assert.deepEqual(advanceSession(0.95, 1, true, 0.1, false), { time: 1, replay: false, ended: true });
    assert.deepEqual(advanceSession(0.5, 1, false, 0.1), { time: 0.5, replay: false, ended: false });
    let wall = 1000;
    const clock = new FrameClock(() => wall);
    assert.equal(clock.now(), 1000);
    wall = 999;
    assert.equal(clock.now(), 1000);
    wall = 1001;
    assert.equal(clock.now(), 1001);
    assert.equal(clock.reserve(3), 1001);
    assert.ok(clock.now() >= 1004);
    wall = 1002;
    assert.ok(clock.now() >= 1005);
});

test("particle quads sort into blend and texture batches in world pixels", () => {
    const quad = (texture, blend, flags, x) => {
        const q = new Float32Array(QUAD_FLOATS);
        q.set([texture, blend, 1, 0.5, 0.25, 1]);
        for (let c = 0; c < 4; c++) q.set([x + c, 0, 0, c / 4, 0], 6 + c * 5);
        q[26] = flags;
        return q;
    };
    const quads = new Float32Array([...quad(2, 1, 0, 0), ...quad(1, 0, 0, 10), ...quad(1, 0, 1, 20), ...quad(1, 0, 0, 30)]);
    const built = particleBuffers(quads, [1, 2, 3]);
    assert.equal(built.count, 4);
    assert.deepEqual(built.batches.map(({ texture, blend, emissive, start, count }) => [texture, blend, emissive, start, count]), [[1, 0, 0, 0, 12], [1, 0, 1, 12, 6], [2, 1, 0, 18, 6]]);
    assertClose(Array.from(built.positions.subarray(0, 3)), [(10 + 1) * 16, 2 * 16, 3 * 16]);
    assertClose(Array.from(built.colors.subarray(0, 4)), [1, 0.5, 0.25, 1]);
    assert.deepEqual(Array.from(built.indices.subarray(0, 6)), [0, 1, 2, 0, 2, 3]);
    assert.equal(particleBuffers(quads.subarray(0, QUAD_FLOATS), [0, 0, 0], undefined, built), built);
});

test("the orbit camera matches the core and marks bring a seated actor back onto its spot", () => {
    const view = orbitView({ yaw: 30, pitch: 10, distance: 70, targetY: 18 });
    assertClose(transformPoint(view, [0, 18, 0]), [0, 0, -70]);
    const fx = effectView(view);
    assertClose(fx.slice(0, 3), transformPoint(invert(view), [0, 0, 0]).map((v) => v / PIXELS_PER_BLOCK));
    assert.ok(close(fx[3] * fx[6] + fx[4] * fx[7] + fx[5] * fx[8], 0), "right and up are perpendicular");
    const spot = { x: 0.5, y: 0.25, z: 1.5, yaw: 180 };
    const { anchor, yaw } = markFrom(spot);
    const radians = (yaw * Math.PI) / 180;
    const turned = [spot.x * Math.cos(radians) - spot.z * Math.sin(radians), spot.y, spot.x * Math.sin(radians) + spot.z * Math.cos(radians)];
    assertClose([anchor[0] + turned[0], anchor[1] + turned[1], anchor[2] + turned[2]], [0, 0, 0]);
    assert.ok(close(yaw + spot.yaw, 0) || close(Math.abs(yaw + spot.yaw), 360));
    const placed = rootAt(spot, 2);
    assertClose(transformPoint(placed, [16, 0, 0]), [0.5 - 2, 0.25, 1.5]);
    assertClose(multiply(placed, invert(placed)), IDENTITY);
});

test("pet pairs seat authored roles, fall back to generic spots and pick the interaction by seed", () => {
    const authored = {
        pet: { archetype: "walking", interactions: { greet: ["a", "b"] } },
        roles: [{ interaction: "greet", id: "a", animation: "x", offset: [0, 0, -0.75], yaw: 0 }, { interaction: "greet", id: "b", animation: "y", offset: [0, 0, 0.75], yaw: 180 }],
    };
    assert.deepEqual(pairSeats(authored, authored, "greet", 1).map((seat) => [seat.seat, seat.role, seat.spot.z]), [[0, 0, -0.75], [1, 1, 0.75]]);
    const chase = pairSeats(authored, authored, "chase", 1);
    assert.ok(chase.every((seat) => seat.role === NO_ROLE));
    assert.deepEqual(chase[0].spot, genericPetSpot("chase", 0, 1));
    const moved = genericPetMotion("chase", 1, 1);
    assert.ok(Math.hypot(moved.spots[0].x - chase[0].spot.x, moved.spots[0].z - chase[0].spot.z) > 0.1 && moved.speeds[0] > 0);
    assert.equal(supportsInteraction({ archetype: "shoulder", interactions: {} }, "chase"), false);
    assert.deepEqual(petInteractions({ archetype: "walking", interactions: { greet: [] } }), ["follow", "nap", "chase"]);
    assert.equal(petSessionSeconds("greet", [{ seconds: 1.5, looping: false, startDelay: 0.5 }, null]), 2);
    assert.equal(petSessionSeconds("nap", [{ seconds: 2, looping: true, startDelay: 0 }]), 30);
    const both = { archetype: "walking", interactions: { greet: ["a", "b"], nap: ["a"], follow: [], chase: [] } };
    const onlyB = { archetype: "walking", interactions: { greet: ["b"], nap: ["b"], follow: [], chase: [] } };
    const onlyA = { archetype: "walking", interactions: { greet: ["a"] } };
    assert.deepEqual(pairChoice(0n, both, onlyB), { interaction: "greet", firstIsA: true });
    assert.deepEqual(pairChoice(1n, both, onlyB), { interaction: "nap", firstIsA: true });
    assert.deepEqual(pairChoice(0n, onlyB, both), { interaction: "greet", firstIsA: false });
    assert.deepEqual(pairChoice(1n << 32n, both, both), { interaction: "greet", firstIsA: false });
    assert.equal(pairChoice(0n, { archetype: "shoulder", interactions: { greet: [] } }, both), null);
    const orientations = [[0n, onlyB, onlyA, false], [0n, onlyA, onlyB, true], [0n, both, both, true], [1n << 32n, both, both, false], [0n, onlyB, onlyB, true]];
    for (const [seed, a, b, expected] of orientations) assert.equal(pairOrientation(seed, a, b, "greet"), expected);
});

test("emote lengths add start delays and fade, and loops run open-ended", () => {
    const delays = startDelays({ "animations/a.animation.json": json({ animations: { "animation.a.wave": { start_delay: "0.5", animation_length: 1 } } }) });
    assert.equal(delays.get("animation.a.wave"), 0.5);
    const meta = { clips: [{ name: "animation.a.wave", length: 1, loop: "once" }, { name: "animation.a.spin", length: 2, loop: "repeat" }] };
    assert.equal(emoteSeconds(meta, ["animation.a.wave"], delays), 1.5 + FADE_SECONDS);
    assert.equal(emoteSeconds(meta, ["animation.a.spin"]), Infinity);
    assert.deepEqual(emoteSeats([{ id: "b", offset: [0, 0, 1], yaw: 180 }, { id: "a" }]).map((seat) => [seat.seat, seat.role]), [[0, 1], [1, 0]]);
    assert.deepEqual([...clipDelays({ clips: [{ name: "a", start_delay: 2 }, { name: "b" }, { name: "c", start_delay: 0 }] })], [["a", 2]]);
});

test("preview sounds decode once, clamp, skip vanilla sounds and respect mute", async () => {
    const played = [];
    let decodes = 0;
    const sounds = new PreviewSounds({ decode: async (bytes) => (decodes++, { bytes }), play: (_, volume, pitch) => played.push([volume, pitch]), stop() {} });
    const skipped = [];
    sounds.onSkipped = (names) => skipped.push(...names);
    const bytes = new Uint8Array([1, 2, 3]);
    const lookup = (event) => (event.effect === 0 ? { key: "a", bytes } : { key: "b", bytes: null, vanilla: "minecraft:chime" });
    sounds.play([{ effect: 0, volume: 9, pitch: 0 }, { effect: 0, volume: 0.5, pitch: 2 }, { effect: 1, volume: 1, pitch: 1 }], lookup);
    await settle();
    assert.deepEqual([decodes, played, skipped], [1, [[4, 0.1], [0.5, 2]], ["minecraft:chime"]]);
    sounds.setMuted(true);
    assert.equal(sounds.play([{ effect: 0, volume: 1, pitch: 1 }], lookup), 0);
});

test("the input log merges pulses per time, drops later ones on a trim and reads them by range", () => {
    const current = { signals: {}, variables: {} };
    const log = new InputLog(current, { limit: 4 });
    log.pulse(1, 0);
    assert.equal(log.events.length, 0, "an empty pulse is not logged");
    log.pulse(1, 1 << 6);
    log.pulse(1, 1 << 7);
    log.pulse(2, 1 << 6);
    assert.deepEqual(log.events, [{ time: 1, bits: (1 << 6) | (1 << 7) }, { time: 2, bits: 1 << 6 }]);
    assert.deepEqual([log.eventsIn(0, 1), log.eventsIn(1, 1.5), log.eventTimes(0, 2)], [(1 << 6) | (1 << 7), 0, [1]]);
    log.trim(1);
    assert.deepEqual(log.events.map((event) => event.time), [1], "a pulse at the seek time stays");
    for (let k = 2; k <= 9; k++) log.pulse(k, 1 << 6);
    assert.deepEqual([log.events.length, log.events[0].time], [4, 6], "bounded, oldest first out");
    log.trim(0);
    log.pulse(0, 1 << 6);
    log.trim(0);
    assert.deepEqual(log.events, [{ time: 0, bits: 1 << 6 }], "a pulse at time zero survives a replay from zero");
    log.reset(current);
    assert.equal(log.events.length, 0, "only a restart clears it");
    log.note({ signals: { is_hurt: 1 }, variables: {} }, 0.5);
    log.pulse(0.6, 1 << 7);
    log.pulse(0.9, 1 << 6);
    log.note({ signals: { is_hurt: 0 }, variables: {} }, 0.95);
    log.carry(1.05, 0.3);
    assert.deepEqual(log.events.map((event) => event.bits), [1 << 6], "only events inside the window move back a lap");
    assert.ok(close(log.events[0].time, -0.15, 1e-9));
    assert.deepEqual(log.entries.map((entry) => entry.signals.is_hurt), [1, 0], "older changes fold into the starting state");
    log.carry(1, 0);
    assert.deepEqual([log.events.length, log.entries.length], [0, 1]);
});

engineTest("a player stage skins the body and a worn cosmetic and projects like the 2D renderer", () => {
    const asset = parse(wings());
    const stage = new PreviewStage(engine());
    stage.loadAsset(asset, 2);
    const skin = stage.skin();
    assert.equal(skin.boneCount, 15);
    assert.deepEqual(skin.runs.map((run) => [run.texture, run.cull]), [[0, true], [2, false]]);
    const frame = stage.frame(1000, { root: ORIGIN });
    assert.deepEqual([frame.bones.length, frame.offsets.length, frame.colorway], [15 * 12, 2, -1]);
    stage.dispose();
    asset.free();

    const body = new PreviewStage(engine());
    const posed = body.frame(10, { root: ORIGIN });
    const bodySkin = body.skin();
    const camera = { yaw: 25, pitch: 12, distance: 70, targetY: 18 };
    const toPixels = multiply([16, 0, 0, 0, 0, 16, 0, 0, 0, 0, 16, 0], ORIGIN);
    const bounds = (points) => points.reduce((box, [x, y]) => [Math.min(box[0], x), Math.min(box[1], y), Math.max(box[2], x), Math.max(box[3], y)], [Infinity, Infinity, -Infinity, -Infinity]);
    const box = bounds(Array.from({ length: bodySkin.quads * 4 }, (_, v) => {
        const bone = Array.from(posed.bones.subarray(bodySkin.bones[v] * 12, bodySkin.bones[v] * 12 + 12));
        return projectPoint(camera, 320, 240, transformPoint(multiply(toPixels, bone), Array.from(bodySkin.positions.subarray(v * 3, v * 3 + 3))));
    }));
    const flat = wasm.renderer_new();
    wasm.renderer_load_player(flat, 0, 0);
    assert.ok(wasm.renderer_build_f64(flat, camera.yaw, camera.pitch, camera.distance, camera.targetY, 320, 240, 10) > 0);
    const positions = new Float32Array(wasm.memory.buffer, wasm.renderer_positions(flat), wasm.renderer_vertex_count(flat) * 2);
    const flatBox = bounds(Array.from({ length: positions.length / 2 }, (_, i) => [positions[i * 2], positions[i * 2 + 1]]));
    wasm.renderer_free(flat);
    body.dispose();
    assertClose(box, flatBox, 0.05, "screen bounds");
});

engineTest("a stage recolours through a while-reaction and back", () => {
    const asset = parse(wings());
    const stage = new PreviewStage(engine());
    stage.loadAsset(asset, 2);
    stage.play(2, 500);
    let frame = null;
    for (let t = 0; t <= 1; t += 0.05) frame = stage.frame(500 + t, { root: ORIGIN, signals: { is_in_rain: 1 } });
    assert.equal(frame.colorway, asset.meta().palette.colorways.indexOf("storm"));
    for (let t = 0; t <= 4; t += 0.05) frame = stage.frame(502 + t, { root: ORIGIN });
    assert.equal(frame.colorway, -1);
    stage.dispose();
    asset.free();
});

function stubPlayer(options = {}) {
    let wall = 2000;
    const calls = [];
    const gl = new Proxy({}, {
        get(_, name) {
            if (typeof name !== "string") return undefined;
            if (/^[A-Z][A-Z0-9_]*$/.test(name)) return name;
            return () => (calls.push(name), name.startsWith("get") || name.startsWith("create") ? {} : undefined);
        },
    });
    const played = [];
    const sounds = { decode: async (bytes) => bytes, play: (_, volume) => played.push(volume), stop() {} };
    const canvas = { clientWidth: 320, clientHeight: 240, width: 0, height: 0 };
    const player = new PreviewPlayer(canvas, gl, wasm, { clock: () => wall, sounds, pixelRatio: 1, ...options });
    const step = (seconds, frames = 1) => {
        for (let i = 0; i < frames; i++) {
            wall += seconds;
            player.render();
        }
    };
    return { player, calls, played, step };
}

const actorsOf = (player, kind) => player.actors.filter((actor) => actor.kind === kind);
const poses = (player) => actorsOf(player, "player").filter((actor) => actor.seat >= 0).sort((a, b) => a.seat - b.seat).map((actor) => Array.from(actor.frame.bones));
const petPoses = (player) => actorsOf(player, "pet").map((actor) => Array.from(actor.frame.bones));
const moved = (now, then, epsilon = 1e-3) => now[0].some((value, i) => Math.abs(value - then[0][i]) > epsilon);

function assertPoses(actual, expected, what) {
    assert.equal(actual.length, expected.length, `${what} actors`);
    actual.forEach((bones, i) => assertClose(bones, expected[i], 1e-4, `${what} actor ${i}`));
}

function lockstep(count, show) {
    const runs = Array.from({ length: count }, () => stubPlayer());
    const sessions = runs.map(({ player }, i) => show(player, i));
    const all = (seconds, frames = 1) => {
        for (let f = 0; f < frames; f++) runs.forEach(({ step }) => step(seconds));
    };
    return { runs, sessions, all, dispose: () => runs.forEach(({ player }) => player.dispose()) };
}

function sparkCenters(player, kind) {
    const view = effectView(orbitView(player.camera));
    const centers = [];
    for (const actor of actorsOf(player, kind)) {
        const quads = actor.stage.effects(view);
        for (let at = 0; at + QUAD_FLOATS <= quads.length; at += QUAD_FLOATS) {
            const corner = (n, i) => quads[at + 6 + n * 5 + i];
            centers.push([0, 1, 2].map((i) => (corner(0, i) + corner(2, i)) / 2));
        }
    }
    return centers.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
}

function assertSparks(actual, expected, what) {
    assert.equal(actual.length, expected.length, `${what}: ${actual.length} sparks vs ${expected.length}`);
    actual.forEach((center, i) => assertClose(center, expected[i], 1e-3, `${what} spark ${i}`));
}

engineTest("the player wears, recolours and fires effects through one frame loop", async () => {
    const { player, played, step } = stubPlayer();
    const worn = player.wear(wings(), { colorway: 0 });
    const sparks = player.wear(spark());
    step(0.05, 2);
    assert.ok(player.actors.length === 1 && player.stats.draws >= 2, "body and wings drawn");
    assert.deepEqual(worn.colorways, ["storm"]);
    assert.equal(player.fire("join"), true);
    step(0.05, 6);
    await settle();
    assert.ok(played.length >= 1, "the join sound played");
    worn.setCustom({ 0: 0xff0000 });
    step(0.05);
    sparks.remove();
    assert.equal(player.worn.length, 1);
    player.dispose();
});

engineTest("duo emotes seat two players, end by their statuses, seek and loop", () => {
    const { player, step } = stubPlayer();
    const session = player.playEmote(duoFiles(), { loop: false });
    step(0.05, 2);
    assert.deepEqual(actorsOf(player, "player").map((actor) => actor.spot.z), [0, 1.5]);
    step(0.05, 40);
    assert.deepEqual([session.finished, session.playing], [true, false]);
    session.play();
    step(0.05, 2);
    assert.ok(!session.finished && session.time > 0 && session.time < 0.5);
    session.stop();
    step(0.05, 10);
    assert.equal(session.finished, true);
    player.dispose();
});

engineTest("duo emote sounds play once each and seeks or rebuilds replay no earlier sounds", async () => {
    const { player, played, step } = stubPlayer();
    const session = player.playEmote(duoSoundFiles(), { loop: false });
    step(1 / 60, 45);
    await settle();
    assert.equal(played.length, 2);
    player.setPlayers([{}, {}]);
    step(1 / 60, 3);
    session.seek(0.7);
    step(1 / 60, 3);
    await settle();
    assert.equal(played.length, 2, "a rebuild or a seek replays no earlier sounds");
    session.pause();
    session.seek(0.2);
    player.setPlayers([{}, {}]);
    step(1 / 60);
    session.play();
    step(1 / 60, 30);
    await settle();
    assert.equal(played.length, 4, "both keys sound after a paused rebuild");
    player.dispose();
});

engineTest("pets idle beside the player, pair up, take a twin and seat each pet in a role it declares", () => {
    const { player, step } = stubPlayer();
    const pets = player.showPet(petFiles(), { partner: petFiles() });
    step(0.05, 2);
    assert.deepEqual([actorsOf(player, "pet").length, pets.interaction], [2, null]);
    pets.setInteraction("greet");
    step(0.05, 4);
    assert.deepEqual(actorsOf(player, "pet").map((actor) => actor.petSeat.role).sort(), [0, 1]);
    player.setPlayers([]);
    step(0.05);
    assert.ok(player.actors.every((actor) => actor.kind === "pet"));

    const twin = player.showPet(petFiles(), { interaction: "greet" });
    step(0.05, 2);
    assert.equal(actorsOf(player, "pet").length, 2);
    twin.setInteraction(null);
    step(0.05);
    assert.equal(actorsOf(player, "pet").length, 1, "the twin goes when the pet idles");

    const roleOnly = (role) => edit(petFiles(), "manifest.json", (m) => void (m.interactions = { greet: [role] }));
    assert.equal(player.showPet(roleOnly("b"), { partner: roleOnly("a"), interaction: "greet" }).firstIsA, false);
    step(0.05, 3);
    assert.deepEqual(actorsOf(player, "pet").map((actor) => actor.petSeat?.seat), [1, 0], "the first pet takes role b");
    player.dispose();
});

engineTest("role delays come from each pet's own bundle, given as zips, parsed assets or file maps", () => {
    const { player } = stubPlayer();
    const delayed = edit(petFiles(), "animations/wings.animation.json", (a) => void (a.animations["animation.wings.droop"].start_delay = 5));
    const asset = parse(delayed);
    const lengthOf = (source, options = {}) => player.showPet(source, { interaction: "greet", ...options }).length;
    assert.equal(lengthOf(petFiles(), { partner: delayed }), 6, "the partner's droop waits 5 s and runs 1 s");
    for (const [source, options] of [[petFiles(), { partner: storeZip(delayed) }], [asset, {}], [storeZip(petFiles()), { files: storeZip(delayed) }]]) {
        assert.equal(lengthOf(source, options), 6);
    }
    const late = edit(duoFiles(), "animations/duo.animation.json", (a) => void (a.animations["animation.duo.left"].start_delay = 2));
    assert.ok(player.playEmote(storeZip(late)).length > player.playEmote(duoFiles()).length);
    player.dispose();
    asset.free();
});

const SIGNAL = { condition: "query.is_hurt", on: (p) => p.setSignal("is_hurt", 1), off: (p) => p.setSignal("is_hurt", 0), log: "signalLog", kept: 1 };
const VARIABLE = { condition: "variable.pet_mood > 0.5", on: (p) => p.setVariable("pet_mood", 1), off: (p) => p.setVariable("pet_mood", 0), log: "signalLog", kept: 1 };
const JUMP = { condition: "query.is_jumping", on: (p) => p.pulse(EVENT_JUMP), off: () => {}, log: "eventLog", kept: 0 };

engineTest("scrubbed and rebuilt emotes and pets replay past signal, variable and jump changes at their own times", () => {
    const shows = {
        emote: [poses, (input) => (player) => player.playEmote(latchEmoteFiles(input.condition))],
        paired: [petPoses, (input) => (player) => player.showPet(latchPetFiles(input.condition), { partner: latchPetFiles(input.condition), interaction: "chase", loop: false })],
        solo: [petPoses, (input) => (player) => player.showPet(latchPetFiles(input.condition))],
    };
    for (const [kind, input] of [["emote", SIGNAL], ["emote", JUMP], ["paired", SIGNAL], ["paired", VARIABLE], ["paired", JUMP], ["solo", SIGNAL], ["solo", VARIABLE], ["solo", JUMP]]) {
        const [read, showOf] = shows[kind];
        const what = `${kind} ${input.condition}`;
        const show = (player) => (player.setVariables({ pet_mood: 0 }), showOf(input)(player));
        const { runs: [scrubbed, reference], sessions: [session, straight], all, dispose } = lockstep(2, show);
        all(0.05, 10);
        const waving = read(reference.player);
        [scrubbed, reference].forEach(({ player }) => input.on(player));
        all(0.05, 2);
        [scrubbed, reference].forEach(({ player }) => input.off(player));
        all(0.05, 10);
        assert.ok(moved(read(reference.player), waving), `${what}: the change moved the controller on`);
        session.seek(0.8);
        scrubbed.step(0.05);
        session.play();
        scrubbed.step(0.05, 5);
        assert.ok(close(session.time, straight.time, 1e-6), `${what}: ${session.time} vs ${straight.time}`);
        assertPoses(read(scrubbed.player), read(reference.player), `${what} after scrubbing back`);
        scrubbed.player.setPlayers([{ slim: true }]);
        all(0.05, 3);
        assertPoses(read(scrubbed.player), read(reference.player), `${what} after a rebuild`);
        session.seek(0.3);
        scrubbed.step(0.05);
        assert.equal(session[input.log].length, input.kept, `${what}: seeking before the change drops it from the history`);
        session.seek(1.1);
        scrubbed.step(0.05);
        const fresh = stubPlayer();
        show(fresh.player).seek(1.1);
        fresh.step(0.05);
        assertPoses(read(scrubbed.player), read(fresh.player), `${what}: a later scrub keeps the unchanged history`);
        dispose();
        fresh.player.dispose();
    }
});

engineTest("pet reactions, appears, effects and delayed clips run through interaction laps as an unlooped pet shows them", () => {
    const still = ({ reactions = [], clips: extra = {}, sparks = false }) => {
        const base = edit(petFiles(), "animations/wings.animation.json", (a) => Object.assign(a.animations, extra));
        const manifest = (m) => ({ roles: m.roles.map((role) => ({ ...role, animation: "droop" })), requires: [...m.requires, "reactions", ...(sparks ? ["effects", "particles"] : [])], reactions });
        return sparks ? sparkPet(manifest, base) : edit(base, "manifest.json", (m) => Object.assign(m, manifest(m)));
    };
    const each = (frame, at, act) => (players) => frame === at && players.forEach(act);
    const cases = [
        ["a fired effect", still({ sparks: true }), 60, (f) => each(f, 15, (p) => p.fire("pop")), 10],
        ["a short clip's sparks", still({ sparks: true, reactions: [{ on: "equip", play: "flick", blend: 0 }], clips: { "animation.wings.flick": swingClip(0.5, { particle_effects: { "0.1": { effect: "pop" } } }) } }), 70, (f) => each(f, 15, (p) => p.pulse(EVENT_EQUIP)), 20],
        ["a delayed clip", still({ reactions: [{ on: "equip", play: "late", blend: 0 }], clips: { "animation.wings.late": swingClip(0.5, { start_delay: "query.is_in_rain * 2.0" }) } }), 80,
            (f) => (players) => (each(f, 1, (p) => p.setSignal("is_in_rain", 1))(players), each(f, 15, (p) => p.pulse(EVENT_EQUIP))(players))],
    ];
    for (const [what, files, frames, act, sparkFrames] of cases) {
        const { runs: [looped, steady], sessions, all, dispose } = lockstep(2, (player, i) => player.showPet(files, { interaction: "greet", loop: true, length: i === 0 ? 1 : 100 }));
        all(0.05);
        const first = petPoses(steady.player);
        let [laps, swung, sparked] = [0, false, 0];
        for (let frame = 1; frame <= frames; frame++) {
            act(frame)([looped, steady].map(({ player }) => player));
            const before = sessions[0].time;
            all(0.05);
            if (sessions[0].time < before) laps++;
            swung ||= moved(petPoses(steady.player), first, 1e-2);
            assertPoses(petPoses(looped.player), petPoses(steady.player), `${what} frame ${frame} (lap ${laps})`);
            if (!sparkFrames) continue;
            const shown = sparkCenters(steady.player, "pet");
            if (shown.length && laps > 0) sparked++;
            assertSparks(sparkCenters(looped.player, "pet"), shown, `${what} frame ${frame} (lap ${laps})`);
        }
        assert.ok(laps >= 2, `${what}: the looped pet ran ${laps} laps`);
        assert.ok(sparkFrames ? sparked > sparkFrames : swung, `${what}: ${sparkFrames ? `sparks after a lap on ${sparked} frames` : "the wing moved"}`);
        sessions.forEach((session) => session.seek(session.time));
        all(0.05);
        assertPoses(petPoses(looped.player), petPoses(steady.player), `${what}: a scrub after the laps`);
        dispose();
    }
});

