import { defaultWasmUrl, EFFECT_EVENT_INTS, EFFECT_PARTICLE, EFFECT_SOUND } from "./index.js";
import { AUTHORING_CAPABILITIES, CosmeticAsset, CosmeticEngine, NO_COLORWAY, RIG_VALUES, SECTION_NAMES } from "./cosmetic.js";
import { instantiate, SIGNALS, startClock } from "./nameplate.js";

export const PIXELS_PER_BLOCK = 16;
export const AFFINE_FLOATS = 12;
export const QUAD_FLOATS = 27;
export const VIEW_FLOATS = 12;
export const EFFECT_EVENT_FLOATS = 5;
export const EFFECT_VANILLA_PARTICLE = 2;
export const LAYOUT_INTS = 12;
export const NO_PLAY = -1;
export const NO_ROLE = -1;
export const EVENT_JUMP = 1 << 6;
export const EVENT_EQUIP = 1 << 7;
export const EMOTE_ENDED = 1 << 1;
export const EMOTE_CANCEL = 1 << 2;
export const EMOTE_CANCEL_WALKING = 1 << 4;
export const EMOTE_CANCEL_SPRINTING = 1 << 5;
export const EMOTE_CANCEL_CROUCHING = 1 << 6;
export const EMOTE_CANCEL_FALLING = 1 << 7;
export const EMOTE_CANCEL_ELYTRA_FLYING = 1 << 8;
export const EMOTE_CANCEL_SWIMMING = 1 << 9;
export const EMOTE_CANCEL_SLEEPING = 1 << 10;
export const EMOTE_CANCEL_SWINGING = 1 << 11;
export const EMOTE_HELD = 1 << 12;
export const HOLD_STEP_SECONDS = 1 / 20;
export const FADE_SECONDS = 0.25;
export const MAX_STEP_SECONDS = 0.1;
export const MIN_SESSION_SECONDS = 0.25;
const SIGNAL_LOG_LIMIT = 256;
export const EMOTE_CATCH_UP_SECONDS = 120;
export const PET_CATCH_UP_SECONDS = 120;
export const ROLE_IDS = Object.freeze(["a", "b"]);
export const INTERACTIONS = Object.freeze(["greet", "follow", "nap", "play", "chase"]);
export const INTERACTION_SECONDS = Object.freeze({ greet: 3, follow: 6, nap: 30, play: 8, chase: 12 });
export const PET_INTERACTION_VALUES = Object.freeze({ greet: 1, follow: 2, nap: 3, play: 4, chase: 5 });
export const PET_PLAYING = 7;
export const PET_SLEEPING = 5;
export const PET_SCALE_MIN = 0.01;
export const PET_SCALE_MAX = 8;
export const PET_VARIABLES = Object.freeze([
    "pet_mode", "pet_action", "pet_look_yaw", "pet_look_pitch", "pet_speed", "pet_distance", "pet_on_ground",
    "pet_in_water", "pet_mood", "pet_interaction", "pet_role", "pet_interaction_time",
]);
export const PET_DEFAULTS = Object.freeze({ pet_on_ground: 1 });
export const NEUTRAL_SIGNALS = Object.freeze({ is_on_ground: 1, time_of_day: 0.25 });
export const DEFAULT_CAMERA = Object.freeze({ yaw: 0, pitch: 8, distance: 70, targetX: 0, targetY: 18, targetZ: 0 });
export const IDENTITY = Object.freeze([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]);

const PHYSICS_ON = 0;
const PLAYER_SLOT = 0;
const PROPS_SLOT = 1;
const FIRST_WORN_SLOT = 2;
const PET_SLOT = 0;
const COMBAT_SIGNAL = "is_in_combat";
const CELEBRATE_SIGNAL = "is_emoting";
const LAYOUT_BONES = 0;
const LAYOUT_RUNS = 1;
const LAYOUT_COLORWAY = 3;
const LAYOUT_RUNS_AT = 5;
const LAYOUT_COLORWAY_AT = 7;
const LAYOUT_EMOTE = 10;
const LAYOUT_EMOTE_AT = 11;
const TEXTURE_INFO_INTS = 3;
const MESH_RUN_INTS = 4;
const QUAD_TEXTURE = 0;
const QUAD_BLEND = 1;
const QUAD_COLOR = 2;
const QUAD_CORNERS = 6;
const CORNER_FLOATS = 5;
const QUAD_FLAGS = 26;
const BLEND_ADDITIVE = 1;
const FLAG_EMISSIVE = 1;
const MIN_PARTICLE_CAPACITY = 64;
const FOV_Y_DEG = 45;
const NEAR = 0.5;
const FAR = 4096;
const AMBIENT = 0.62;
const DIFFUSE = 0.38;
const LIGHT_DIRECTION = Object.freeze(normalize([-0.35, 0.8, 1.0]));
const MAX_VOLUME = 4;
const MIN_PITCH = 0.1;
const MAX_PITCH = 4;
const PLAYER_GAP = 0.6;
const PET_BESIDE = Object.freeze({ x: 1.1, y: 0, z: 0.35, yaw: 0 });
const IDLE_PET_GAP = 0.7;
const PET_BODY_WIDTH = 0.5;
const PET_GAP = 0.2;
const PET_CIRCLE_GAP = 0.6;
const PAIRED_MOVES = new Set(["follow", "chase"]);
const MOVE_CANCELS = EMOTE_CANCEL_WALKING | EMOTE_CANCEL_SPRINTING;
const KEPT_CANCELS = EMOTE_CANCEL_CROUCHING | EMOTE_CANCEL_FALLING | EMOTE_CANCEL_ELYTRA_FLYING | EMOTE_CANCEL_SWIMMING
    | EMOTE_CANCEL_SLEEPING | EMOTE_CANCEL_SWINGING;
const TICKS_PER_SECOND = 20;
const WALK = 0.194;
const RUN = 0.3;
const FOLLOW_CIRCLE_SPEED = 1.8;
const CHASE_SWAP_SECONDS = 4;
const CHASE_RADIUS = 2.5;
const CHASER_PACE = 0.92;
const CHASER_GAP = 0.7;
const CIRCLE_STOP = 0.05;
const GROUND_FRICTION = 0.546;
const ARRIVE_SHARE = 0.5;
const MIN_SPEED = 0.02;
const REST_SPEED = 0.003;
const MAX_TURN = 90;
const CONSTANT_DELAY = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?[fFdD]?$/;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function normalize(v) {
    const length = Math.hypot(v[0], v[1], v[2]) || 1;
    return [v[0] / length, v[1] / length, v[2] / length];
}

function finite(value, fallback = 0) {
    const number = Number(value);
    return Number.isFinite(number) ? number : fallback;
}

function isRecord(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function multiply(a, b) {
    const out = new Array(12).fill(0);
    for (let row = 0; row < 3; row++) {
        for (let col = 0; col < 4; col++) {
            let sum = col === 3 ? a[row * 4 + 3] : 0;
            for (let k = 0; k < 3; k++) sum += a[row * 4 + k] * b[k * 4 + col];
            out[row * 4 + col] = sum;
        }
    }
    return out;
}

export function invert(a) {
    const [m00, m01, m02, tx, m10, m11, m12, ty, m20, m21, m22, tz] = Array.from(a);
    const c00 = m11 * m22 - m12 * m21;
    const c01 = m02 * m21 - m01 * m22;
    const c02 = m01 * m12 - m02 * m11;
    const c10 = m12 * m20 - m10 * m22;
    const c11 = m00 * m22 - m02 * m20;
    const c12 = m02 * m10 - m00 * m12;
    const c20 = m10 * m21 - m11 * m20;
    const c21 = m01 * m20 - m00 * m21;
    const c22 = m00 * m11 - m01 * m10;
    const det = m00 * c00 + m01 * c10 + m02 * c20;
    const k = Math.abs(det) < 1e-12 ? 0 : 1 / det;
    const r = [c00 * k, c01 * k, c02 * k, c10 * k, c11 * k, c12 * k, c20 * k, c21 * k, c22 * k];
    return [
        r[0], r[1], r[2], -(r[0] * tx + r[1] * ty + r[2] * tz),
        r[3], r[4], r[5], -(r[3] * tx + r[4] * ty + r[5] * tz),
        r[6], r[7], r[8], -(r[6] * tx + r[7] * ty + r[8] * tz),
    ];
}

export function transformPoint(a, p) {
    return [
        a[0] * p[0] + a[1] * p[1] + a[2] * p[2] + a[3],
        a[4] * p[0] + a[5] * p[1] + a[6] * p[2] + a[7],
        a[8] * p[0] + a[9] * p[1] + a[10] * p[2] + a[11],
    ];
}

export function scaleAffine(k) {
    return [k, 0, 0, 0, 0, k, 0, 0, 0, 0, k, 0];
}

export function wrapDegrees(degrees) {
    let a = degrees % 360;
    if (a >= 180) a -= 360;
    if (a < -180) a += 360;
    return a;
}

export function rootAt(spot = {}, scale = 1) {
    const radians = (finite(spot.yaw) * Math.PI) / 180;
    const c = Math.cos(radians);
    const s = Math.sin(radians);
    const k = finite(scale, 1) / PIXELS_PER_BLOCK;
    return [c * k, 0, -s * k, finite(spot.x), 0, k, 0, finite(spot.y), s * k, 0, c * k, finite(spot.z)];
}

export function offsetSpot(spot, origin) {
    return { x: spot.x + origin.x, y: spot.y + origin.y, z: spot.z + origin.z, yaw: spot.yaw };
}

export function orbitView(camera = DEFAULT_CAMERA) {
    const yaw = (finite(camera.yaw) * Math.PI) / 180;
    const pitch = (finite(camera.pitch) * Math.PI) / 180;
    const distance = finite(camera.distance, DEFAULT_CAMERA.distance);
    const target = [finite(camera.targetX), finite(camera.targetY, DEFAULT_CAMERA.targetY), finite(camera.targetZ)];
    const cp = Math.cos(pitch);
    const sp = Math.sin(pitch);
    const cy = Math.cos(yaw);
    const sy = Math.sin(yaw);
    const back = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, -distance];
    const tilt = [1, 0, 0, 0, 0, cp, -sp, 0, 0, sp, cp, 0];
    const turnY = [cy, 0, sy, 0, 0, 1, 0, 0, -sy, 0, cy, 0];
    const centre = [1, 0, 0, -target[0], 0, 1, 0, -target[1], 0, 0, 1, -target[2]];
    return multiply(back, multiply(tilt, multiply(turnY, centre)));
}

export function projection(width, height) {
    const f = 1 / Math.tan(((FOV_Y_DEG * Math.PI) / 180) / 2);
    const aspect = width / Math.max(1, height);
    return [
        f / aspect, 0, 0, 0,
        0, f, 0, 0,
        0, 0, (FAR + NEAR) / (NEAR - FAR), -1,
        0, 0, (2 * FAR * NEAR) / (NEAR - FAR), 0,
    ];
}

export function mat4(affine) {
    const a = affine;
    return new Float32Array([a[0], a[4], a[8], 0, a[1], a[5], a[9], 0, a[2], a[6], a[10], 0, a[3], a[7], a[11], 1]);
}

export function projectPoint(camera, width, height, p) {
    const v = transformPoint(orbitView(camera), p);
    const f = 1 / Math.tan(((FOV_Y_DEG * Math.PI) / 180) / 2);
    const k = (f * height * 0.5) / -v[2];
    return [width * 0.5 + v[0] * k, height * 0.5 - v[1] * k, -v[2]];
}

export function effectView(view) {
    const eye = transformPoint(invert(view), [0, 0, 0]);
    return [
        eye[0] / PIXELS_PER_BLOCK, eye[1] / PIXELS_PER_BLOCK, eye[2] / PIXELS_PER_BLOCK,
        ...normalize([view[0], view[1], view[2]]),
        ...normalize([view[4], view[5], view[6]]),
        0, 0, 0,
    ];
}

export class FrameClock {
    constructor(source = startClock()) {
        this.source = source;
        this.offset = 0;
        this.last = -Infinity;
    }

    now() {
        const value = this.source() + this.offset;
        if (value > this.last) this.last = value;
        return this.last;
    }

    reserve(span) {
        const base = this.now();
        const end = base + Math.max(0, finite(span));
        const lag = end - (this.source() + this.offset);
        if (lag > 0) this.offset += lag;
        this.last = end;
        return base;
    }
}

function movedOnly(status) {
    return (status & MOVE_CANCELS) !== 0 && (status & KEPT_CANCELS) === 0;
}

export function holdStep(status, holding, shared, partnerDone, onMark = () => false) {
    if (status < 0) return "end";
    if ((status & EMOTE_CANCEL) !== 0 && !(movedOnly(status) && onMark())) return "end";
    if (!holding) {
        if ((status & (EMOTE_ENDED | EMOTE_HELD)) === 0) return "play";
        if (!shared()) return "end";
    }
    return partnerDone() ? "end" : "hold";
}

export function freshHolds(count) {
    return Array.from({ length: count }, () => ({ holding: false, heldAge: 0, heldFrom: 0, stopping: false, stoppedAt: 0 }));
}

export function holdInput(hold, age) {
    return { age: hold.holding ? hold.heldAge : age, held: hold.holding && !hold.stopping, stopped: hold.stopping };
}

export function reactHolds(holds, statuses, age, onMark = () => holds.length > 1) {
    const finished = holds.map((hold) => hold.stopping || hold.holding);
    holds.forEach((hold, i) => {
        if (hold.stopping) return;
        const shared = () => holds.length > 1;
        const step = holdStep(statuses[i] ?? 0, hold.holding, shared, () => finished.some((done, j) => j !== i && done), onMark);
        if (step === "hold" && !hold.holding) {
            hold.holding = true;
            hold.heldAge = age;
            hold.heldFrom = age;
        } else if (step === "end") {
            hold.stopping = true;
            hold.stoppedAt = age;
        }
    });
}

export function holdTicks(after, through) {
    const ticks = [];
    const first = Math.floor(after / HOLD_STEP_SECONDS + 1e-9) + 1;
    const last = Math.floor(through / HOLD_STEP_SECONDS + 1e-9);
    for (let k = Math.max(0, first); k <= last; k++) ticks.push(k * HOLD_STEP_SECONDS);
    return ticks;
}

function replayPoints(ticks, quiet, times) {
    const points = new Map();
    for (const age of times) points.set(age, false);
    if (Number.isFinite(quiet) && !points.has(quiet)) points.set(quiet, false);
    for (const age of ticks) points.set(age, true);
    return [...points].sort((a, b) => a[0] - b[0]).map(([age, tick]) => ({ age, tick }));
}

export function holdsFinished(holds, age) {
    return holds.length > 0 && holds.every((hold) => hold.stopping && age - hold.stoppedAt >= FADE_SECONDS);
}

export function advanceSession(time, length, playing, seconds, loop = true) {
    if (!playing || !(seconds > 0)) return { time, replay: false, ended: false };
    const next = time + seconds;
    if (next > length) return loop ? { time: 0, replay: true, ended: false } : { time: length, replay: false, ended: true };
    return { time: next, replay: false, ended: false };
}

function parsedJson(text) {
    try {
        return text ? JSON.parse(typeof text === "string" ? text : decoder.decode(text)) : null;
    } catch {
        return null;
    }
}

function entries(value, key) {
    return isRecord(value) && isRecord(value[key]) ? Object.entries(value[key]) : [];
}

function constantDelay(value) {
    if (typeof value === "number") return value;
    if (typeof value !== "string") return null;
    const text = value.trim();
    return CONSTANT_DELAY.test(text) ? Number(text.replace(/[fFdD]$/, "")) : null;
}

function fileEntries(files) {
    if (!files) return [];
    return files instanceof Map ? [...files] : Array.isArray(files) ? files : Object.entries(files);
}

export function startDelays(files) {
    const out = new Map();
    for (const [path, text] of fileEntries(files)) {
        if (!String(path).toLowerCase().endsWith(".json")) continue;
        for (const [clip, body] of entries(parsedJson(text), "animations")) {
            const seconds = isRecord(body) ? constantDelay(body.start_delay) : null;
            if (seconds !== null && Number.isFinite(seconds) && seconds > 0) out.set(clip, seconds);
        }
    }
    return out;
}

export function clipDelays(meta) {
    const out = new Map();
    for (const clip of meta?.clips ?? []) {
        const seconds = finite(clip?.start_delay);
        if (seconds > 0) out.set(clip.name, seconds);
    }
    return out;
}

export function clipOf(meta, name) {
    return (meta?.clips ?? []).find((clip) => clip.name === name) ?? null;
}

export function mainClip(meta) {
    const fallback = meta?.fallback ?? null;
    return (meta?.clips ?? []).find((clip) => clip.name !== fallback) ?? null;
}

export function emoteSeconds(meta, names, delays = new Map()) {
    let longest = 0;
    let looping = false;
    for (const name of names) {
        const clip = name ? clipOf(meta, name) : null;
        if (!clip) continue;
        if (clip.loop === "repeat") looping = true;
        longest = Math.max(longest, (delays.get(name) ?? 0) + finite(clip.length) + (clip.loop === "repeat" ? 0 : FADE_SECONDS));
    }
    if (looping) return Infinity;
    return Math.max(MIN_SESSION_SECONDS, longest);
}

export function roleIndex(roles, seat) {
    const id = ROLE_IDS[seat];
    const found = id === undefined ? -1 : roles.findIndex((role) => role.id === id);
    return found >= 0 ? found : seat;
}

export function spotOf(role) {
    const offset = role?.offset ?? [0, 0, 0];
    return { x: finite(offset[0]), y: finite(offset[1]), z: finite(offset[2]), yaw: wrapDegrees(finite(role?.yaw)) };
}

export function emoteSeats(roles) {
    return ROLE_IDS.map((_, seat) => {
        const role = roleIndex(roles, seat);
        const data = roles[role];
        if (!data) return null;
        return { seat, role, spot: spotOf(data) };
    }).filter((seat) => seat !== null);
}

export function markFrom(spot) {
    const radians = (-finite(spot.yaw) * Math.PI) / 180;
    const s = Math.sin(radians);
    const c = Math.cos(radians);
    const x = -finite(spot.x);
    const z = -finite(spot.z);
    return { anchor: [x * c - z * s, -finite(spot.y), x * s + z * c], yaw: wrapDegrees(-finite(spot.yaw)) };
}

export function petScale(scale) {
    return Math.min(PET_SCALE_MAX, Math.max(PET_SCALE_MIN, finite(scale, 1)));
}

export function supportsInteraction(pet, interaction, role = null) {
    if (!INTERACTIONS.includes(interaction)) return false;
    if (pet?.archetype === "shoulder" && interaction !== "greet") return false;
    const declared = pet?.interactions?.[interaction];
    if (declared) return role === null ? declared.includes("a") || declared.includes("b") : declared.includes(ROLE_IDS[role]);
    return interaction !== "play";
}

export function petInteractions(pet) {
    return INTERACTIONS.filter((id) => supportsInteraction(pet, id));
}

function leads(first, second, play) {
    return supportsInteraction(first, play, 0) && supportsInteraction(second, play, 1);
}

export function pairOrientation(seed, first, second, interaction) {
    const firstLeads = leads(first, second, interaction);
    const secondLeads = leads(second, first, interaction);
    if (firstLeads && secondLeads) return ((BigInt.asUintN(64, BigInt(seed)) >> 32n) & 1n) === 0n;
    return firstLeads || !secondLeads;
}

export function pairChoice(seed, first, second) {
    const candidates = INTERACTIONS.filter((play) => leads(first, second, play) || leads(second, first, play));
    if (!candidates.length) return null;
    const value = BigInt.asUintN(64, BigInt(seed));
    const interaction = candidates[Number(value % BigInt(candidates.length))];
    return { interaction, firstIsA: pairOrientation(seed, first, second, interaction) };
}

export function genericPetSpot(interaction, seat, scale) {
    const half = (PET_BODY_WIDTH * scale) / 2;
    const first = seat === 0;
    if (interaction === "follow") return first ? { x: 0, y: 0, z: -(2 * half + PET_CIRCLE_GAP), yaw: 0 } : { x: 0, y: 0, z: 0, yaw: wrapDegrees(180) };
    const reach = half + PET_GAP / 2;
    const yaw = interaction === "nap" ? 90 : first ? 0 : wrapDegrees(180);
    return { x: 0, y: 0, z: first ? -reach : reach, yaw };
}

function genericSeat(interaction, seat, scale) {
    return { seat, role: NO_ROLE, spot: genericPetSpot(interaction, seat, scale) };
}

function petSeat(roles, interaction, seat, declared) {
    const id = ROLE_IDS[seat];
    if (declared && !declared.includes(id)) return null;
    const role = roles.findIndex((candidate) => candidate.interaction === interaction && candidate.id === id);
    if (role < 0) return null;
    return { seat, role, spot: spotOf(roles[role]) };
}

export function pairSeats(first, second, interaction, scale) {
    if (!INTERACTIONS.includes(interaction)) return [];
    const pets = [first, second];
    const placed = [0, 1]
        .map((seat) => petSeat(pets[seat]?.roles ?? [], interaction, seat, pets[seat]?.pet?.interactions?.[interaction]))
        .filter((seat) => seat !== null);
    const generic = () => [genericSeat(interaction, 0, scale), genericSeat(interaction, 1, scale)];
    if (placed.length === 0) return generic();
    if (placed.length === 2) return placed;
    if (PAIRED_MOVES.has(interaction)) return generic();
    const other = 1 - placed[0].seat;
    return [placed[0], genericSeat(interaction, other, scale)].sort((a, b) => a.seat - b.seat);
}

export function isGenericMove(interaction, seats) {
    return PAIRED_MOVES.has(interaction) && seats.length === 2 && seats.every((seat) => seat.role === NO_ROLE);
}

function leadTicks(from) {
    const ticks = [];
    for (let k = Math.ceil(from / HOLD_STEP_SECONDS - 1e-9); k < 0; k++) ticks.push(k * HOLD_STEP_SECONDS);
    return ticks;
}

export function petSessionSeconds(interaction, clips) {
    let longest = 0;
    let looping = false;
    for (const clip of clips) {
        if (!clip || !(clip.seconds > 0)) continue;
        longest = Math.max(longest, (clip.startDelay > 0 ? clip.startDelay : 0) + clip.seconds);
        if (clip.looping) looping = true;
    }
    const base = INTERACTION_SECONDS[interaction] ?? 0;
    if (longest <= 0) return base;
    return looping ? Math.max(longest, base) : longest;
}

export function petRoleVariables(host, interaction, seat, time) {
    return {
        ...host,
        pet_mode: interaction === "nap" && time >= 0 ? PET_SLEEPING : PET_PLAYING,
        pet_interaction: PET_INTERACTION_VALUES[interaction] ?? 0,
        pet_role: seat,
        pet_interaction_time: time,
    };
}

export function petPairSignals(values, interaction, seat) {
    const celebrating = interaction === "greet" || (interaction === "play" && seat.role === NO_ROLE);
    return celebrating ? { ...values, [CELEBRATE_SIGNAL]: 1 } : values;
}

function headingOf(dx, dz) {
    if (dx * dx + dz * dz < 1.0e-8) return null;
    return (Math.atan2(-dx, dz) * 180) / Math.PI;
}

function stepMover(mover, goal) {
    let ax = 0;
    let az = 0;
    if (goal) {
        const dx = goal.x - mover.x;
        const dz = goal.z - mover.z;
        const remaining = Math.sqrt(dx * dx + dz * dz);
        if (remaining > goal.stop && remaining > 1.0e-4) {
            const heading = headingOf(dx, dz) ?? mover.yaw;
            mover.yaw += Math.max(-MAX_TURN, Math.min(MAX_TURN, wrapDegrees(heading - mover.yaw)));
            const speed = Math.min(goal.speed, Math.max(MIN_SPEED, (remaining - goal.stop) * ARRIVE_SHARE));
            const accel = speed * (1 - GROUND_FRICTION);
            const radians = (mover.yaw * Math.PI) / 180;
            ax = -Math.sin(radians) * accel;
            az = Math.cos(radians) * accel;
        }
    }
    mover.vx += ax;
    mover.vz += az;
    mover.x += mover.vx;
    mover.z += mover.vz;
    mover.stride = Math.sqrt(mover.vx * mover.vx + mover.vz * mover.vz);
    mover.vx *= GROUND_FRICTION;
    mover.vz *= GROUND_FRICTION;
    if (ax === 0 && az === 0 && mover.vx * mover.vx + mover.vz * mover.vz < REST_SPEED * REST_SPEED) {
        mover.vx = 0;
        mover.vz = 0;
    }
}

function circleGoal(radius, phase, angle, speed) {
    return { x: Math.cos(phase + angle) * radius, z: Math.sin(phase + angle) * radius, speed, stop: CIRCLE_STOP };
}

function goalsAt(interaction, movers, elapsed, radius, spin) {
    if (interaction === "follow") return [circleGoal(radius, -Math.PI / 2, elapsed * FOLLOW_CIRCLE_SPEED, WALK * 1.3), null];
    const runner = Math.floor(elapsed / CHASE_SWAP_SECONDS) & 1;
    const omega = (RUN / radius) * TICKS_PER_SECOND;
    return movers.map((_, role) => {
        if (role === runner) return circleGoal(radius, role === 0 ? -Math.PI / 2 : Math.PI / 2, spin * elapsed * omega, RUN);
        const partner = movers[runner];
        return { x: partner.x, z: partner.z, speed: RUN * CHASER_PACE, stop: CHASER_GAP };
    });
}

export function genericPetMotion(interaction, scale, time, spin = 1) {
    const start = [genericPetSpot(interaction, 0, scale), genericPetSpot(interaction, 1, scale)];
    if (!PAIRED_MOVES.has(interaction)) return { spots: start, speeds: [0, 0] };
    const radius = interaction === "follow" ? -start[0].z : CHASE_RADIUS;
    const movers = start.map((spot) => ({ x: spot.x, z: spot.z, yaw: spot.yaw, vx: 0, vz: 0, stride: 0 }));
    const ticks = Math.max(0, time) * TICKS_PER_SECOND;
    const whole = Math.floor(ticks);
    let before = movers.map((mover) => ({ ...mover }));
    for (let tick = 0; tick <= whole; tick++) {
        before = movers.map((mover) => ({ ...mover }));
        const goals = goalsAt(interaction, movers, tick / TICKS_PER_SECOND, radius, spin);
        movers.forEach((mover, role) => stepMover(mover, goals[role]));
    }
    const t = ticks - whole;
    const spots = movers.map((mover, role) => {
        const from = before[role];
        return {
            x: from.x + (mover.x - from.x) * t,
            y: 0,
            z: from.z + (mover.z - from.z) * t,
            yaw: wrapDegrees(from.yaw + wrapDegrees(mover.yaw - from.yaw) * t),
        };
    });
    return { spots, speeds: movers.map((mover) => mover.stride * TICKS_PER_SECOND) };
}

export function particleCapacity(count) {
    let capacity = MIN_PARTICLE_CAPACITY;
    while (capacity < count) capacity *= 2;
    return capacity;
}

function particleArrays(capacity) {
    return {
        positions: new Float32Array(capacity * 12),
        uvs: new Float32Array(capacity * 8),
        colors: new Float32Array(capacity * 16),
        indices: new Uint32Array(capacity * 6),
        batches: [],
        count: 0,
        capacity,
    };
}

export function particleBuffers(quads, camera, toWorld = scaleAffine(PIXELS_PER_BLOCK), reuse = null) {
    const count = Math.floor(quads.length / QUAD_FLOATS);
    const out = reuse && reuse.capacity >= count ? reuse : particleArrays(particleCapacity(count));
    const additive = (i) => (quads[i * QUAD_FLOATS + QUAD_BLEND] === BLEND_ADDITIVE ? 1 : 0);
    const glows = (i) => (Number(quads[i * QUAD_FLOATS + QUAD_FLAGS]) & FLAG_EMISSIVE ? 1 : 0);
    const order = Array.from({ length: count }, (_, i) => i);
    order.sort((a, b) => additive(a) - additive(b) || quads[a * QUAD_FLOATS + QUAD_TEXTURE] - quads[b * QUAD_FLOATS + QUAD_TEXTURE] || glows(a) - glows(b) || a - b);
    const { positions, uvs, colors, indices } = out;
    const batches = [];
    for (let n = 0; n < count; n++) {
        const at = order[n] * QUAD_FLOATS;
        const texture = quads[at + QUAD_TEXTURE];
        const blend = additive(order[n]);
        const emissive = glows(order[n]);
        const last = batches[batches.length - 1];
        if (last && last.texture === texture && last.blend === blend && last.emissive === emissive) last.count += 6;
        else batches.push({ texture, blend, emissive, start: n * 6, count: 6 });
        for (let c = 0; c < 4; c++) {
            const corner = at + QUAD_CORNERS + c * CORNER_FLOATS;
            const p = transformPoint(toWorld, [quads[corner] + camera[0], quads[corner + 1] + camera[1], quads[corner + 2] + camera[2]]);
            const v = n * 4 + c;
            positions[v * 3] = p[0];
            positions[v * 3 + 1] = p[1];
            positions[v * 3 + 2] = p[2];
            uvs[v * 2] = quads[corner + 3];
            uvs[v * 2 + 1] = quads[corner + 4];
            for (let k = 0; k < 4; k++) colors[v * 4 + k] = quads[at + QUAD_COLOR + k];
        }
        const v = n * 4;
        const i = n * 6;
        indices[i] = v;
        indices[i + 1] = v + 1;
        indices[i + 2] = v + 2;
        indices[i + 3] = v;
        indices[i + 4] = v + 2;
        indices[i + 5] = v + 3;
    }
    out.batches = batches;
    out.count = count;
    return out;
}

export function downmix(buffer, create) {
    if (buffer.numberOfChannels !== 2) return buffer;
    const left = buffer.getChannelData(0);
    const right = buffer.getChannelData(1);
    const mono = create(1, buffer.length, buffer.sampleRate);
    const out = mono.getChannelData(0);
    for (let i = 0; i < out.length; i++) out[i] = (left[i] + right[i]) / 2;
    return mono;
}

export function webAudio() {
    const Context = globalThis.AudioContext ?? globalThis.webkitAudioContext;
    if (typeof Context !== "function") return null;
    let context = null;
    const audio = () => {
        if (context) return context;
        try {
            context = new Context();
        } catch {
            context = null;
        }
        return context;
    };
    const active = new Set();
    return {
        async decode(bytes) {
            const ctx = audio();
            if (!ctx) return null;
            const decoded = await ctx.decodeAudioData(bytes.slice().buffer);
            return downmix(decoded, (channels, length, rate) => ctx.createBuffer(channels, length, rate));
        },
        play(buffer, volume, pitch) {
            const ctx = audio();
            if (!ctx) return;
            if (ctx.state === "suspended") ctx.resume().catch(() => {});
            const source = ctx.createBufferSource();
            source.buffer = buffer;
            source.playbackRate.value = pitch;
            const gain = ctx.createGain();
            gain.gain.value = volume;
            source.connect(gain).connect(ctx.destination);
            active.add(source);
            source.onended = () => {
                active.delete(source);
                source.disconnect();
                gain.disconnect();
            };
            source.start();
        },
        stop() {
            for (const source of Array.from(active)) {
                try {
                    source.stop();
                } catch {
                    active.delete(source);
                }
            }
        },
        close() {
            this.stop();
            context?.close?.().catch?.(() => {});
            context = null;
        },
    };
}

function clamp(value, min, max, fallback) {
    return Number.isFinite(value) ? Math.max(min, Math.min(max, value)) : fallback;
}

export class PreviewSounds {
    constructor(output = webAudio()) {
        this.output = output;
        this.muted = false;
        this.decoded = new Map();
        this.skipped = new Set();
        this.disposed = false;
        this.onSkipped = null;
    }

    get vanillaSkipped() {
        return Array.from(this.skipped);
    }

    setMuted(muted) {
        this.muted = !!muted;
        if (this.muted) this.stop();
    }

    play(events, lookup) {
        const output = this.output;
        if (this.disposed || this.muted || !output) return 0;
        let started = 0;
        for (const event of events) {
            const sound = lookup(event);
            if (!sound) continue;
            if (!sound.bytes || !sound.bytes.length) {
                if (sound.vanilla && !this.skipped.has(sound.vanilla)) {
                    this.skipped.add(sound.vanilla);
                    this.onSkipped?.(this.vanillaSkipped);
                }
                continue;
            }
            const volume = clamp(event.volume, 0, MAX_VOLUME, 1);
            const pitch = clamp(event.pitch, MIN_PITCH, MAX_PITCH, 1);
            if (volume <= 0) continue;
            const decoding = this.buffer(sound.key, sound.bytes);
            started += 1;
            decoding.then((buffer) => {
                if (buffer && !this.muted && !this.disposed) output.play(buffer, volume, pitch);
            });
        }
        return started;
    }

    buffer(key, bytes) {
        const known = this.decoded.get(key);
        if (known) return known;
        const decoding = Promise.resolve()
            .then(() => this.output.decode(bytes))
            .catch(() => null);
        this.decoded.set(key, decoding);
        return decoding;
    }

    stop() {
        this.output?.stop();
    }

    forget() {
        this.decoded.clear();
        this.skipped.clear();
    }

    dispose() {
        if (this.disposed) return;
        this.disposed = true;
        this.stop();
        this.output?.close?.();
        this.decoded.clear();
    }
}

function sectionInts(sections) {
    return Int32Array.from(SECTION_NAMES, (name) => {
        const value = sections[name];
        return typeof value === "boolean" ? Number(value) : value ?? 0;
    });
}

export function stageSections({ play = false, role = false, hold = false, variables = 0 } = {}) {
    return { parts: 0, queries: 0, root: true, variables, play: play || role, role, colorway: true, part_deltas: false, hold };
}

export class PreviewStage {
    constructor(engine, { rig = "player", body = rig === "player", slim = false, wearer = 1n } = {}) {
        this.engine = engine;
        this.wasm = engine.wasm;
        this.rig = rig;
        this.body = body;
        this.slim = slim;
        this.wearer = BigInt.asUintN(64, BigInt(wearer));
        this.assets = new Map();
        this.emote = null;
        this.variableIds = new Map();
        this.allocations = new Map();
        this.shape = null;
        this.disposed = false;
        this.handle = this.wasm.renderer_new();
        try {
            this.check(this.wasm.renderer_set_rig(this.handle, RIG_VALUES[rig] ?? RIG_VALUES.player));
            this.check(this.wasm.renderer_set_physics(this.handle, PHYSICS_ON, this.wearer));
            if (body) this.wasm.renderer_load_player(this.handle, PLAYER_SLOT, slim ? 1 : 0);
        } catch (error) {
            this.wasm.renderer_free(this.handle);
            this.handle = 0;
            throw error;
        }
    }

    memory() {
        return this.wasm.memory.buffer;
    }

    error() {
        const wasm = this.wasm;
        return decoder.decode(new Uint8Array(this.memory(), wasm.renderer_error(this.handle), wasm.renderer_error_len(this.handle)));
    }

    check(code) {
        if (code < 0) throw new Error(this.error());
        return code;
    }

    live() {
        if (this.disposed) throw new Error("preview: this stage was disposed");
        return this.handle;
    }

    space(name, bytes) {
        const size = Math.max(1, Math.ceil(bytes));
        const known = this.allocations.get(name);
        if (known && known.bytes >= size) return known.ptr;
        if (known) this.wasm.dealloc(known.ptr, known.bytes);
        const ptr = this.wasm.alloc(size);
        this.allocations.set(name, { ptr, bytes: size });
        return ptr;
    }

    withBytes(bytes, fn) {
        const size = Math.max(1, bytes.length);
        const ptr = this.wasm.alloc(size);
        try {
            new Uint8Array(this.memory(), ptr, bytes.length).set(bytes);
            return fn(ptr, bytes.length);
        } finally {
            this.wasm.dealloc(ptr, size);
        }
    }

    loadAsset(asset, slot) {
        this.check(this.wasm.renderer_load_asset(this.live(), slot, asset.live()));
        this.assets.set(slot, asset);
        this.shape = null;
        return slot;
    }

    loadEmote(asset) {
        if (!this.body) throw new Error("preview: emotes need a player stage");
        this.check(this.wasm.renderer_load_emote(this.live(), PLAYER_SLOT, PROPS_SLOT, asset.live()));
        this.assets.set(PLAYER_SLOT, asset);
        this.emote = asset;
        this.shape = null;
    }

    variableId(name) {
        const known = this.variableIds.get(name);
        if (known !== undefined) return known;
        const id = this.check(this.withBytes(encoder.encode(name), (ptr, len) => this.wasm.renderer_host_variable_id(this.live(), ptr, len)));
        this.variableIds.set(name, id);
        this.shape = null;
        return id;
    }

    plays() {
        let moments = false;
        let roles = false;
        for (const asset of this.assets.values()) {
            const meta = asset.meta();
            if (!meta) continue;
            if (meta.kind !== "emote" && (meta.effects?.moments?.length ?? 0) > 0) moments = true;
            if (meta.kind === "pet" && (meta.roles?.length ?? 0) > 0) roles = true;
        }
        return { moments, roles, emote: !!this.emote };
    }

    get playsMoments() {
        const plays = this.plays();
        return plays.moments && !plays.emote;
    }

    layout() {
        if (this.shape) return this.shape;
        const plays = this.plays();
        const variables = this.variableIds.size ? Math.max(...this.variableIds.values()) + 1 : 0;
        const sections = stageSections({
            play: plays.emote || plays.moments || plays.roles,
            role: plays.emote || plays.roles,
            hold: plays.emote,
            variables,
        });
        const ints = sectionInts(sections);
        const sectionsPtr = this.space("sections", ints.byteLength);
        new Int32Array(this.memory(), sectionsPtr, ints.length).set(ints);
        const shapePtr = this.space("shape", LAYOUT_INTS * 4);
        const stride = this.check(this.wasm.renderer_output_layout(this.live(), sectionsPtr, ints.length, shapePtr, LAYOUT_INTS));
        const out = new Int32Array(this.memory(), shapePtr, LAYOUT_INTS).slice();
        const input = this.engine.inputLayout(sections);
        this.shape = {
            sections,
            sectionCount: ints.length,
            bones: out[LAYOUT_BONES],
            runs: out[LAYOUT_RUNS],
            runsAt: out[LAYOUT_RUNS_AT],
            colorwayAt: out[LAYOUT_COLORWAY] ? out[LAYOUT_COLORWAY_AT] : -1,
            statusAt: out[LAYOUT_EMOTE] ? out[LAYOUT_EMOTE_AT] : -1,
            outputLength: stride,
            input,
            signalCount: Math.max(0, input.events - input.signals),
        };
        return this.shape;
    }

    fillRow(row, shape, input) {
        const layout = shape.input;
        const signals = input.signals ?? {};
        SIGNALS.forEach((name, i) => {
            if (i < shape.signalCount) row[layout.signals + i] = finite(signals[name] ?? NEUTRAL_SIGNALS[name]);
        });
        row[layout.events] = (input.events ?? 0) >>> 0;
        for (const [name, id] of this.variableIds) row[layout.variables_at + id] = finite(input.variables?.[name]);
        const root = input.root ?? IDENTITY;
        for (let i = 0; i < AFFINE_FLOATS; i++) row[layout.root_at + i] = finite(root[i], IDENTITY[i]);
        const sections = shape.sections;
        const role = input.role && sections.role ? input.role : null;
        if (sections.play) {
            if (input.stopped) {
                row[layout.play_at] = NO_PLAY;
                row[layout.play_at + 1] = finite(role?.age);
            } else if (role) {
                row[layout.play_at] = 0;
                row[layout.play_at + 1] = finite(role.age);
            } else if (input.play) {
                row[layout.play_at] = input.play.clip;
                row[layout.play_at + 1] = finite(input.play.age);
            } else {
                row[layout.play_at] = NO_PLAY;
                row[layout.play_at + 1] = 0;
            }
        }
        if (sections.role) {
            if (role) {
                const anchor = role.anchor ?? [0, 0, 0];
                row[layout.role_at] = role.index;
                row[layout.role_at + 1] = finite(anchor[0]);
                row[layout.role_at + 2] = finite(anchor[1]);
                row[layout.role_at + 3] = finite(anchor[2]);
                row[layout.role_at + 4] = finite(role.yaw);
            } else {
                row[layout.role_at] = NO_ROLE;
            }
        }
        if (sections.hold) row[layout.hold_at] = input.held ? 1 : 0;
    }

    frame(clock, input = {}) {
        const shape = this.layout();
        const wasm = this.wasm;
        const combat = SIGNALS.indexOf(COMBAT_SIGNAL);
        if (combat >= 0 && combat < shape.signalCount) {
            this.check(wasm.renderer_set_signal_override(this.live(), this.wearer, combat, finite(input.signals?.[COMBAT_SIGNAL]) ? 1 : 0));
        }
        const inputLength = shape.input.stride;
        const wearerPtr = this.space("wearer", 8);
        const inputPtr = this.space("input", inputLength * 4);
        const outputPtr = this.space("output", shape.outputLength * 4);
        const sectionsPtr = this.allocations.get("sections").ptr;
        new BigUint64Array(this.memory(), wearerPtr, 1)[0] = this.wearer;
        const row = new Float32Array(this.memory(), inputPtr, inputLength);
        row.fill(0);
        this.fillRow(row, shape, input);
        this.check(wasm.renderer_frame_wearers_shaped(this.live(), clock, wearerPtr, 1, sectionsPtr, shape.sectionCount, inputPtr, inputLength, 0, outputPtr, shape.outputLength));
        const out = new Float32Array(this.memory(), outputPtr, shape.outputLength);
        return {
            bones: out.slice(0, shape.bones * AFFINE_FLOATS),
            offsets: out.slice(shape.runsAt, shape.runsAt + shape.runs),
            colorway: shape.colorwayAt >= 0 ? Math.round(out[shape.colorwayAt]) : NO_COLORWAY,
            status: shape.statusAt >= 0 ? Math.trunc(out[shape.statusAt]) : 0,
        };
    }

    skin() {
        const wasm = this.wasm;
        const handle = this.live();
        const quads = this.check(wasm.renderer_build_skin(handle));
        const layout = wasm.renderer_layout(handle);
        const vertices = quads * 4;
        const memory = this.memory();
        const copy = (Type, ptr, count) => (count ? new Type(memory, ptr, count).slice() : new Type(0));
        const runCount = wasm.renderer_skin_run_count(handle);
        const runInts = copy(Uint32Array, wasm.renderer_skin_runs(handle), runCount * MESH_RUN_INTS);
        const runs = [];
        for (let i = 0; i < runCount; i++) {
            const at = i * MESH_RUN_INTS;
            runs.push({ texture: runInts[at], start: runInts[at + 1], count: runInts[at + 2], cull: runInts[at + 3] !== 0 });
        }
        return {
            layout,
            quads,
            boneCount: wasm.renderer_skin_bone_count(handle),
            positions: copy(Float32Array, wasm.renderer_skin_positions(handle), vertices * 3),
            normals: copy(Float32Array, wasm.renderer_skin_normals(handle), vertices * 3),
            uvs: copy(Float32Array, wasm.renderer_skin_uvs(handle), vertices * 2),
            glow: copy(Uint8Array, wasm.renderer_skin_glow(handle), vertices),
            bones: copy(Uint32Array, wasm.renderer_skin_bones(handle), vertices),
            runs,
        };
    }

    skinLayout() {
        this.check(this.wasm.renderer_build_skin(this.live()));
        return this.wasm.renderer_layout(this.live());
    }

    fire(name) {
        return this.check(this.withBytes(encoder.encode(name), (ptr, len) => this.wasm.renderer_fire_effect(this.live(), this.wearer, ptr, len))) > 0;
    }

    setEffectColors(colorway = NO_COLORWAY, custom = null) {
        const pairs = custom instanceof Map ? [...custom] : Array.isArray(custom) ? custom : Object.entries(custom ?? {});
        const count = pairs.length;
        const slots = this.space("paint-slots", count * 4);
        const rgb = this.space("paint-rgb", count * 4);
        new Uint32Array(this.memory(), slots, count).set(pairs.map(([slot]) => Number(slot) >>> 0));
        new Uint32Array(this.memory(), rgb, count).set(pairs.map(([, color]) => Number(color) & 0xffffff));
        this.check(this.wasm.renderer_set_effect_colors(this.live(), this.wearer, colorway, slots, rgb, count));
    }

    hasEffects() {
        return !this.disposed && this.wasm.renderer_has_effects(this.handle) !== 0;
    }

    viewAt(view) {
        const viewPtr = this.space("view", VIEW_FLOATS * 4);
        new Float32Array(this.memory(), viewPtr, VIEW_FLOATS).set(Array.from({ length: VIEW_FLOATS }, (_, i) => finite(view[i])));
        return viewPtr;
    }

    sampleEffectRings(view) {
        if (this.disposed || !this.hasEffects()) return;
        this.check(this.wasm.renderer_sample_effect_rings(this.handle, this.viewAt(view), VIEW_FLOATS));
    }

    effects(view) {
        if (this.disposed) return new Float32Array(0);
        const wasm = this.wasm;
        const viewPtr = this.viewAt(view);
        const needed = this.check(wasm.renderer_effect_geometry(this.handle, viewPtr, VIEW_FLOATS, 0, 0));
        if (needed === 0) return new Float32Array(0);
        const floats = needed * QUAD_FLOATS * 2;
        const quadsPtr = this.space("quads", floats * 4);
        const capacity = Math.floor(this.allocations.get("quads").bytes / 4);
        const count = this.check(wasm.renderer_effect_geometry(this.handle, viewPtr, VIEW_FLOATS, quadsPtr, capacity));
        return new Float32Array(this.memory(), quadsPtr, count * QUAD_FLOATS).slice();
    }

    effectTexture(id) {
        if (this.disposed) return null;
        const wasm = this.wasm;
        const slotPtr = this.space("texture-slot", 4);
        const length = wasm.renderer_effect_texture(this.handle, id, slotPtr, 0, 0);
        if (length < 0) return null;
        const slot = new Uint32Array(this.memory(), slotPtr, 1)[0];
        const textPtr = this.space("texture-path", length);
        wasm.renderer_effect_texture(this.handle, id, 0, textPtr, length);
        const path = decoder.decode(new Uint8Array(this.memory(), textPtr, length));
        const asset = this.assets.get(slot) ?? null;
        if (!asset) return { slot, path, image: null };
        return { slot, path, image: effectImage(asset, path) };
    }

    events() {
        if (this.disposed) return [];
        const wasm = this.wasm;
        const count = wasm.renderer_effect_events(this.handle, 0, 0);
        if (count <= 0) return [];
        const intsPtr = this.space("event-ints", count * EFFECT_EVENT_INTS * 4);
        const floatsPtr = this.space("event-floats", count * EFFECT_EVENT_FLOATS * 4);
        wasm.renderer_effect_events(this.handle, intsPtr, count * EFFECT_EVENT_INTS);
        wasm.renderer_effect_event_floats(this.handle, floatsPtr, count * EFFECT_EVENT_FLOATS);
        const ints = new Uint32Array(this.memory(), intsPtr, count * EFFECT_EVENT_INTS);
        const floats = new Float32Array(this.memory(), floatsPtr, count * EFFECT_EVENT_FLOATS);
        const out = [];
        for (let i = 0; i < count; i++) {
            const e = i * EFFECT_EVENT_INTS;
            const f = i * EFFECT_EVENT_FLOATS;
            out.push({
                wearer: ints[e],
                slot: ints[e + 1],
                controller: ints[e + 2],
                source: ints[e + 3],
                kind: ints[e + 4],
                effect: ints[e + 5],
                at: [floats[f], floats[f + 1], floats[f + 2]],
                volume: floats[f + 3],
                pitch: floats[f + 4],
            });
        }
        return out;
    }

    sound(event) {
        const asset = this.assets.get(event.slot);
        if (!asset || event.kind !== EFFECT_SOUND) return null;
        return assetSound(asset, event.effect);
    }

    play(slot, at) {
        if (!this.disposed) this.wasm.renderer_play_f64(this.handle, slot, at);
    }

    forget() {
        if (!this.disposed) this.wasm.renderer_forget_wearer(this.handle, this.wearer);
    }

    breakRings() {
        return !this.disposed && this.wasm.renderer_break_rings(this.handle, this.wearer) !== 0;
    }

    dispose() {
        if (this.disposed) return;
        this.disposed = true;
        for (const { ptr, bytes } of this.allocations.values()) this.wasm.dealloc(ptr, bytes);
        this.allocations.clear();
        this.wasm.renderer_free(this.handle);
        this.handle = 0;
    }
}

export function effectTextureNames(meta) {
    const effects = meta?.effects ?? {};
    return [...(effects.particles ?? []).map((p) => p.texture), ...(effects.trails ?? []).map((t) => t.texture)];
}

export function effectImage(asset, path) {
    if (!asset?.usable) return null;
    const names = effectTextureNames(asset.meta());
    const local = names.findIndex((name) => name === path || name === `${path}.png` || `${name}.png` === path);
    if (local < 0) return null;
    const engine = asset.engine;
    const wasm = engine.wasm;
    const info = wasm.alloc(TEXTURE_INFO_INTS * 4);
    try {
        const bytes = wasm.cosmetic_effect_texture_info(asset.live(), local, info);
        if (bytes <= 0) return null;
        const [width, height] = new Int32Array(engine.memory(), info, TEXTURE_INFO_INTS);
        if (!(width > 0 && height > 0)) return null;
        const out = wasm.alloc(bytes);
        try {
            const written = wasm.cosmetic_effect_texture(asset.live(), local, out, bytes);
            if (written < 0) return null;
            return { width, height, data: new Uint8Array(engine.memory(), out, written).slice() };
        } finally {
            wasm.dealloc(out, bytes);
        }
    } finally {
        wasm.dealloc(info, TEXTURE_INFO_INTS * 4);
    }
}

const soundCache = new WeakMap();
const assetKeys = new WeakMap();
let assetCount = 0;

export function assetKey(asset) {
    if (!assetKeys.has(asset)) assetKeys.set(asset, ++assetCount);
    return assetKeys.get(asset);
}

function readSound(asset, index) {
    const meta = asset?.meta();
    const entry = meta?.effects?.sounds?.[index];
    if (!entry) return null;
    const key = `${assetKey(asset)}:${index}`;
    const vanilla = entry.vanilla ?? null;
    if (!entry.file) return { key, bytes: null, vanilla };
    const engine = asset.engine;
    const wasm = engine.wasm;
    const size = wasm.cosmetic_sound(asset.live(), index, 0, 0);
    if (size <= 0) return { key, bytes: null, vanilla };
    const out = wasm.alloc(size);
    try {
        const written = wasm.cosmetic_sound(asset.live(), index, out, size);
        if (written < 0) return null;
        return { key, bytes: new Uint8Array(engine.memory(), out, written).slice(), vanilla };
    } finally {
        wasm.dealloc(out, size);
    }
}

export function assetSound(asset, index) {
    if (!asset?.usable) return null;
    let known = soundCache.get(asset);
    if (!known) soundCache.set(asset, (known = new Map()));
    if (!known.has(index)) known.set(index, readSound(asset, index));
    return known.get(index);
}

const MESH_VERTEX = `#version 300 es
in vec3 a_position;
in vec3 a_normal;
in vec2 a_uv;
in float a_glow;
in uint a_bone;
uniform highp sampler2D u_bones;
uniform mat4 u_model;
uniform mat4 u_view;
uniform mat4 u_projection;
uniform vec3 u_light;
uniform float u_level;
uniform float u_offset;
out vec2 v_uv;
out float v_shade;
float brightness(float level) {
    float f = clamp(level / 15.0, 0.0, 1.0);
    return f / (4.0 - 3.0 * f);
}
void main() {
    int bone = int(a_bone);
    vec4 r0 = texelFetch(u_bones, ivec2(0, bone), 0);
    vec4 r1 = texelFetch(u_bones, ivec2(1, bone), 0);
    vec4 r2 = texelFetch(u_bones, ivec2(2, bone), 0);
    vec4 rest = vec4(a_position, 1.0);
    vec3 posed = vec3(dot(r0, rest), dot(r1, rest), dot(r2, rest));
    vec3 turned = vec3(dot(r0.xyz, a_normal), dot(r1.xyz, a_normal), dot(r2.xyz, a_normal));
    mat4 modelView = u_view * u_model;
    gl_Position = u_projection * modelView * vec4(posed, 1.0);
    vec3 normal = normalize(mat3(modelView) * turned);
    v_shade = (${AMBIENT.toFixed(2)} + ${DIFFUSE.toFixed(2)} * max(dot(normal, u_light), 0.0)) * brightness(max(u_level, a_glow));
    v_uv = vec2(a_uv.x, a_uv.y + u_offset);
}`;

const MESH_FRAGMENT = `#version 300 es
precision mediump float;
in vec2 v_uv;
in float v_shade;
uniform sampler2D u_texture;
out vec4 fragColor;
void main() {
    vec4 texel = texture(u_texture, v_uv);
    if (texel.a < 0.1) discard;
    fragColor = vec4(texel.rgb * v_shade, texel.a);
}`;

const PARTICLE_VERTEX = `#version 300 es
in vec3 a_position;
in vec2 a_uv;
in vec4 a_color;
uniform mat4 u_view;
uniform mat4 u_projection;
out vec2 v_uv;
out vec4 v_color;
void main() {
    gl_Position = u_projection * u_view * vec4(a_position, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}`;

const PARTICLE_FRAGMENT = `#version 300 es
precision mediump float;
in vec2 v_uv;
in vec4 v_color;
uniform sampler2D u_texture;
uniform float u_light;
uniform float u_cutoff;
out vec4 fragColor;
void main() {
    vec4 color = texture(u_texture, v_uv) * v_color;
    color.rgb *= u_light;
    if (color.a <= u_cutoff) discard;
    fragColor = color;
}`;

const MESH_ATTRIBUTES = ["a_position", "a_normal", "a_uv", "a_glow", "a_bone"];
const PARTICLE_ATTRIBUTES = ["a_position", "a_uv", "a_color"];
const DEFAULT_SKIN_RGBA = [196, 198, 206, 255];
const WHITE_RGBA = [255, 255, 255, 255];

function compileShader(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
    return shader;
}

function linkProgram(gl, vertex, fragment, attributes) {
    const program = gl.createProgram();
    const shaders = [compileShader(gl, gl.VERTEX_SHADER, vertex), compileShader(gl, gl.FRAGMENT_SHADER, fragment)];
    for (const shader of shaders) gl.attachShader(program, shader);
    attributes.forEach((name, location) => gl.bindAttribLocation(program, location, name));
    gl.linkProgram(program);
    for (const shader of shaders) gl.deleteShader(shader);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
    return program;
}

function uniformsOf(gl, program, names) {
    return Object.fromEntries(names.map((name) => [name, gl.getUniformLocation(program, name)]));
}

function isPixels(source) {
    if (typeof ImageData !== "undefined" && source instanceof ImageData) return false;
    return !!source && ArrayBuffer.isView(source.data) && Number.isInteger(source.width) && Number.isInteger(source.height);
}

export function solidImage(width, height, rgba) {
    const data = new Uint8Array(width * height * 4);
    for (let i = 0; i < data.length; i += 4) data.set(rgba, i);
    return { width, height, data };
}

function uploadTexture(gl, texture, source) {
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
    if (isPixels(source)) {
        const data = source.data instanceof Uint8Array ? source.data : new Uint8Array(source.data.buffer, source.data.byteOffset, source.data.byteLength);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, source.width, source.height, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
    } else {
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
    }
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
}

class SkinDrawer {
    constructor(gl) {
        this.gl = gl;
        this.program = linkProgram(gl, MESH_VERTEX, MESH_FRAGMENT, MESH_ATTRIBUTES);
        this.uniforms = uniformsOf(gl, this.program, ["u_bones", "u_model", "u_view", "u_projection", "u_light", "u_level", "u_offset", "u_texture"]);
        this.boneTexture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, this.boneTexture);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }

    upload(skin) {
        const gl = this.gl;
        const vao = gl.createVertexArray();
        gl.bindVertexArray(vao);
        const buffers = [];
        const attribute = (location, data, size, type, integer = false) => {
            const buffer = gl.createBuffer();
            buffers.push(buffer);
            gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
            gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
            gl.enableVertexAttribArray(location);
            if (integer) gl.vertexAttribIPointer(location, size, type, 0, 0);
            else gl.vertexAttribPointer(location, size, type, false, 0, 0);
        };
        attribute(0, skin.positions, 3, gl.FLOAT);
        attribute(1, skin.normals, 3, gl.FLOAT);
        attribute(2, skin.uvs, 2, gl.FLOAT);
        attribute(3, skin.glow, 1, gl.UNSIGNED_BYTE);
        attribute(4, skin.bones, 1, gl.UNSIGNED_INT, true);
        const indices = new Uint32Array(skin.quads * 6);
        for (let q = 0; q < skin.quads; q++) {
            const v = q * 4;
            indices.set([v, v + 1, v + 2, v, v + 2, v + 3], q * 6);
        }
        const index = gl.createBuffer();
        buffers.push(index);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, index);
        gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.STATIC_DRAW);
        gl.bindVertexArray(null);
        return { vao, buffers, runs: skin.runs, layout: skin.layout, boneCount: skin.boneCount };
    }

    release(gpu) {
        if (!gpu) return;
        const gl = this.gl;
        gl.deleteVertexArray(gpu.vao);
        for (const buffer of gpu.buffers) gl.deleteBuffer(buffer);
    }

    draw(gpu, frame, model, view, projection, textureOf, level) {
        if (!gpu || gpu.boneCount === 0 || frame.bones.length < gpu.boneCount * AFFINE_FLOATS) return 0;
        const gl = this.gl;
        gl.useProgram(this.program);
        gl.bindVertexArray(gpu.vao);
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_2D, this.boneTexture);
        gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, 3, gpu.boneCount, 0, gl.RGBA, gl.FLOAT, frame.bones.subarray(0, gpu.boneCount * AFFINE_FLOATS));
        const u = this.uniforms;
        gl.uniform1i(u.u_bones, 1);
        gl.uniform1i(u.u_texture, 0);
        gl.uniformMatrix4fv(u.u_model, false, model);
        gl.uniformMatrix4fv(u.u_view, false, view);
        gl.uniformMatrix4fv(u.u_projection, false, projection);
        gl.uniform3fv(u.u_light, LIGHT_DIRECTION);
        gl.uniform1f(u.u_level, level);
        gl.activeTexture(gl.TEXTURE0);
        let draws = 0;
        gpu.runs.forEach((run, i) => {
            const texture = textureOf(run.texture);
            if (!texture || run.count === 0) return;
            gl.bindTexture(gl.TEXTURE_2D, texture);
            if (run.cull) gl.enable(gl.CULL_FACE);
            else gl.disable(gl.CULL_FACE);
            gl.uniform1f(u.u_offset, frame.offsets[i] ?? 0);
            gl.drawElements(gl.TRIANGLES, run.count * 6, gl.UNSIGNED_INT, run.start * 6 * 4);
            draws += 1;
        });
        gl.disable(gl.CULL_FACE);
        gl.bindVertexArray(null);
        return draws;
    }

    dispose() {
        this.gl.deleteProgram(this.program);
        this.gl.deleteTexture(this.boneTexture);
    }
}

class ParticleDrawer {
    constructor(gl) {
        this.gl = gl;
        this.program = linkProgram(gl, PARTICLE_VERTEX, PARTICLE_FRAGMENT, PARTICLE_ATTRIBUTES);
        this.uniforms = uniformsOf(gl, this.program, ["u_view", "u_projection", "u_texture", "u_light", "u_cutoff"]);
        this.vao = gl.createVertexArray();
        gl.bindVertexArray(this.vao);
        this.attributes = [3, 2, 4].map((size, location) => {
            const buffer = gl.createBuffer();
            gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
            gl.enableVertexAttribArray(location);
            gl.vertexAttribPointer(location, size, gl.FLOAT, false, 0, 0);
            return buffer;
        });
        this.index = gl.createBuffer();
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.index);
        gl.bindVertexArray(null);
        this.cpu = null;
    }

    draw(quads, camera, view, projection, textureOf, light) {
        if (!quads.length) return 0;
        const gl = this.gl;
        const built = particleBuffers(quads, camera, scaleAffine(PIXELS_PER_BLOCK), this.cpu);
        this.cpu = built;
        gl.useProgram(this.program);
        gl.bindVertexArray(this.vao);
        const arrays = [built.positions.subarray(0, built.count * 12), built.uvs.subarray(0, built.count * 8), built.colors.subarray(0, built.count * 16)];
        arrays.forEach((data, i) => {
            gl.bindBuffer(gl.ARRAY_BUFFER, this.attributes[i]);
            gl.bufferData(gl.ARRAY_BUFFER, data, gl.DYNAMIC_DRAW);
        });
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.index);
        gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, built.indices.subarray(0, built.count * 6), gl.DYNAMIC_DRAW);
        const u = this.uniforms;
        gl.uniformMatrix4fv(u.u_view, false, view);
        gl.uniformMatrix4fv(u.u_projection, false, projection);
        gl.uniform1i(u.u_texture, 0);
        gl.activeTexture(gl.TEXTURE0);
        gl.depthMask(false);
        gl.disable(gl.CULL_FACE);
        for (const batch of built.batches) {
            gl.bindTexture(gl.TEXTURE_2D, textureOf(batch.texture));
            gl.blendFunc(gl.SRC_ALPHA, batch.blend ? gl.ONE : gl.ONE_MINUS_SRC_ALPHA);
            gl.uniform1f(u.u_cutoff, batch.blend ? 0 : 0.01);
            gl.uniform1f(u.u_light, batch.emissive ? 1 : light);
            gl.drawElements(gl.TRIANGLES, batch.count, gl.UNSIGNED_INT, batch.start * 4);
        }
        gl.depthMask(true);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.bindVertexArray(null);
        return built.count;
    }

    dispose() {
        const gl = this.gl;
        gl.deleteProgram(this.program);
        gl.deleteVertexArray(this.vao);
        for (const buffer of this.attributes) gl.deleteBuffer(buffer);
        gl.deleteBuffer(this.index);
    }
}

function paintKey(colorway, custom) {
    const pairs = custom instanceof Map ? [...custom] : Array.isArray(custom) ? custom : Object.entries(custom ?? {});
    const text = pairs.map(([slot, rgb]) => `${Number(slot)}=${Number(rgb) & 0xffffff}`).sort().join(",");
    return `${colorway}|${text}`;
}

function isAssetSource(source) {
    return source instanceof CosmeticAsset;
}

function isFileMap(source) {
    return !!source && !(source instanceof ArrayBuffer) && !ArrayBuffer.isView(source) && !isAssetSource(source);
}

function lightLevelBrightness(level) {
    const f = Math.min(1, Math.max(0, level / 15));
    return f / (4 - 3 * f);
}

class WornCosmetic {
    constructor(player, entry, colorway, custom) {
        this.player = player;
        this.entry = entry;
        this.asset = entry.asset;
        this.meta = entry.asset.meta();
        this.colorway = colorway;
        this.custom = custom;
        this.removed = false;
    }

    get namedEffects() {
        return (this.meta.effects?.named ?? []).slice();
    }

    get moments() {
        return (this.meta.effects?.moments ?? []).map((moment) => ({ ...moment }));
    }

    get colorways() {
        return (this.meta.palette?.colorways ?? []).slice();
    }

    setColorway(colorway = NO_COLORWAY) {
        this.colorway = Number.isInteger(colorway) ? colorway : NO_COLORWAY;
        this.player.paintChanged();
    }

    setCustom(custom = null) {
        this.custom = custom;
        this.player.paintChanged();
    }

    fire(name) {
        return this.player.fire(name);
    }

    remove() {
        this.player.unwear(this);
    }
}

class EmoteSession {
    constructor(player, entry, { loop = true, length = null, delays = new Map() } = {}) {
        this.player = player;
        this.entry = entry;
        this.asset = entry.asset;
        this.meta = entry.asset.meta();
        const roles = this.meta.roles ?? [];
        const seats = roles.length >= 2 ? emoteSeats(roles) : [];
        this.seats = seats.length >= 2 ? seats : [{ seat: 0, role: roles.length ? roleIndex(roles, 0) : 0, spot: { x: 0, y: 0, z: 0, yaw: 0 } }];
        this.duo = this.seats.length >= 2;
        const names = roles.length ? this.seats.map((seat) => roles[seat.role]?.animation) : [mainClip(this.meta)?.name];
        this.length = length > 0 ? length : emoteSeconds(this.meta, names, delays);
        this.loop = loop;
        this.time = 0;
        this.reached = 0;
        this.playing = true;
        this.finished = false;
        this.stopping = false;
        this.replay = true;
        this.quiet = null;
        this.holds = freshHolds(this.seats.length);
        this.heldTo = -HOLD_STEP_SECONDS;
        this.statuses = [];
        this.base = 0;
        this.clockEnd = -Infinity;
        this.inputs = new InputLog(player);
    }

    get signalLog() {
        return this.inputs.entries;
    }

    get eventLog() {
        return this.inputs.events;
    }

    clockAt(age) {
        return this.base + age;
    }

    play() {
        if (this.finished || this.stopping) this.restart();
        else this.playing = true;
    }

    pause() {
        this.playing = false;
    }

    stop() {
        if (this.finished) return;
        this.stopping = true;
        this.playing = true;
        for (const hold of this.holds) {
            if (hold.stopping) continue;
            hold.stopping = true;
            hold.stoppedAt = this.time;
        }
    }

    get known() {
        const length = this.length > 0 ? this.length : 0;
        return Math.min(EMOTE_CATCH_UP_SECONDS, Math.max(length, this.reached));
    }

    seek(time) {
        const next = Math.max(0, Math.min(this.known, finite(time)));
        this.playing = false;
        this.finished = false;
        this.stopping = false;
        if (next !== this.time || this.replay) {
            this.replay = true;
            this.quiet = next;
        }
        this.time = next;
    }

    resync() {
        if (!this.replay) this.quiet = this.time;
        this.replay = true;
    }

    rewind() {
        this.time = 0;
        this.replay = true;
        this.quiet = null;
        this.heldTo = -HOLD_STEP_SECONDS;
        this.inputs.reset(this.player);
    }

    restart() {
        this.rewind();
        this.playing = true;
        this.finished = false;
        this.stopping = false;
    }

    dispose() {
        if (this.player.emote === this) this.player.clearEmote();
    }
}

function inputValue(values, key, neutral) {
    return finite(values[key] ?? neutral[key]);
}

function sameValues(a, b, neutral) {
    return [...Object.keys(a), ...Object.keys(b)].every((key) => inputValue(a, key, neutral) === inputValue(b, key, neutral));
}

function switchedValues(a, b, neutral) {
    return [...Object.keys(a), ...Object.keys(b)].some((key) => (inputValue(a, key, neutral) === 0) !== (inputValue(b, key, neutral) === 0));
}

export class InputLog {
    constructor(current, { variableDefaults = {}, limit = SIGNAL_LOG_LIMIT } = {}) {
        this.variableDefaults = variableDefaults;
        this.limit = Math.max(4, limit);
        this.entries = [this.snapshot(-Infinity, current)];
        this.events = [];
    }

    snapshot(time, current) {
        return { time, signals: { ...current.signals }, variables: { ...current.variables } };
    }

    same(a, b) {
        return sameValues(a.signals, b.signals, NEUTRAL_SIGNALS) && sameValues(a.variables, b.variables, this.variableDefaults);
    }

    switched(a, b) {
        return switchedValues(a.signals, b.signals, NEUTRAL_SIGNALS) || switchedValues(a.variables, b.variables, this.variableDefaults);
    }

    at(age) {
        const log = this.entries;
        for (let i = log.length - 1; i > 0; i--) {
            if (log[i].time < age) return log[i];
        }
        return log[0];
    }

    reset(current) {
        this.entries = [this.snapshot(-Infinity, current)];
        this.events = [];
    }

    carry(span, window) {
        const from = -Math.max(0, finite(window));
        const shift = finite(span);
        this.events = this.events.map((event) => ({ time: event.time - shift, bits: event.bits })).filter((event) => event.time > from);
        const log = this.entries.map((entry) => ({ ...entry, time: entry.time - shift }));
        let old = 0;
        while (old + 1 < log.length && log[old + 1].time <= from) old++;
        this.entries = [{ ...log[old], time: -Infinity }, ...log.slice(old + 1)];
    }

    trim(time) {
        const log = this.entries;
        while (log.length > 1 && log[log.length - 1].time >= time) log.pop();
        while (this.events.length && this.events[this.events.length - 1].time > time) this.events.pop();
    }

    pulse(time, bits) {
        bits >>>= 0;
        if (!bits) return;
        const events = this.events;
        while (events.length && events[events.length - 1].time > time) events.pop();
        const last = events[events.length - 1];
        if (last && last.time === time) last.bits = (last.bits | bits) >>> 0;
        else events.push({ time, bits });
        if (events.length > this.limit) events.splice(0, events.length - this.limit);
    }

    eventsIn(after, through) {
        let bits = 0;
        for (const event of this.events) {
            if (event.time > after && event.time <= through) bits |= event.bits;
        }
        return bits >>> 0;
    }

    eventTimes(after, before) {
        return this.events.filter((event) => event.time > after && event.time < before).map((event) => event.time);
    }

    note(current, time, from = 0) {
        const log = this.entries;
        if (this.same(log[log.length - 1], current)) return;
        if (log.length > 1 && log[log.length - 1].time >= time) log.pop();
        if (log.length > 1 && this.same(log[log.length - 1], current)) return;
        log.push(this.snapshot(time, current));
        if (log.length > this.limit) this.compact(from);
    }

    compact(from) {
        let log = this.entries;
        let old = 0;
        while (old + 1 < log.length && log[old + 1].time < from) old++;
        if (old > 0) log = [{ ...log[old], time: -Infinity }, ...log.slice(old + 1)];
        if (log.length > this.limit) {
            const kept = log.map((_, i) => i === 0 || i === log.length - 1);
            for (let i = 1; i < log.length; i++) {
                if (!this.switched(log[i - 1], log[i])) continue;
                kept[i] = true;
                kept[i - 1] = true;
            }
            const loose = kept.flatMap((keep, i) => (keep ? [] : [i]));
            const room = Math.min(loose.length, Math.max(0, (this.limit >> 1) - (log.length - loose.length)));
            const every = room > 0 ? loose.length / room : Infinity;
            for (let n = 0; n < room; n++) kept[loose[Math.floor(n * every)]] = true;
            log = log.filter((_, i) => kept[i]);
        }
        if (log.length > this.limit) {
            const drop = log.length - this.limit;
            log = [{ ...log[drop], time: -Infinity }, ...log.slice(drop + 1)];
        }
        this.entries = log;
    }
}

function petPaint(paint) {
    return {
        colorway: Number.isInteger(paint?.colorway) ? paint.colorway : NO_COLORWAY,
        custom: paint?.custom ?? null,
    };
}

class PetSession {
    constructor(player, entries, { interaction = null, loop = true, owners = [1n, 2n], ids = [1, 2], delays = [], length = null, paints = [] } = {}) {
        this.player = player;
        this.entries = entries;
        this.metas = entries.map((entry) => entry.asset.meta());
        this.paints = [0, 1].map((i) => petPaint(paints[i]));
        this.twinShared = entries.length === 1 && !paints[1];
        this.owners = owners;
        this.ids = ids;
        this.delays = entries.map((_, i) => delays[i] ?? new Map());
        this.fixedLength = length > 0 ? length : null;
        this.loop = loop;
        this.time = 0;
        this.base = 0;
        this.playing = true;
        this.replay = true;
        this.inputs = new InputLog(player, { variableDefaults: PET_DEFAULTS });
        this.seen = 0;
        this.origin = 0;
        this.laps = [];
        this.choose(interaction);
    }

    get signalLog() {
        return this.inputs.entries;
    }

    get eventLog() {
        return this.inputs.events;
    }

    clockAt(age) {
        return this.base + age;
    }

    pairSeed() {
        return this.player.engine.pairSeed(this.owners[0], this.owners[1], (BigInt(this.ids[0] >>> 0) << 32n) | BigInt(this.ids[1] >>> 0));
    }

    get paired() {
        return this.metas.length === 2 || this.interaction !== null;
    }

    get interactions() {
        const [first, second] = this.metas;
        if (!second) return petInteractions(first.pet);
        return INTERACTIONS.filter((id) => supportsInteraction(first.pet, id) && supportsInteraction(second.pet, id));
    }

    choose(interaction) {
        const [first] = this.metas;
        const second = this.metas[1] ?? first;
        this.firstIsA = true;
        this.interaction = null;
        if (interaction === "auto") {
            const picked = pairChoice(this.pairSeed(), first.pet, second.pet);
            if (picked) {
                this.interaction = picked.interaction;
                this.firstIsA = picked.firstIsA;
            }
        } else if (interaction && INTERACTIONS.includes(interaction)) {
            this.interaction = interaction;
            this.firstIsA = pairOrientation(this.pairSeed(), first.pet, second.pet, interaction);
        }
        const scale = Math.max(...this.metas.map((meta) => petScale(meta.pet?.scale)));
        this.scale = scale;
        if (this.interaction) {
            const delays = [this.delays[0], this.delays[1] ?? this.delays[0]];
            const byRole = this.firstIsA ? [first, second] : [second, first];
            const delaysByRole = this.firstIsA ? delays : [delays[1], delays[0]];
            this.seats = pairSeats(byRole[0], byRole[1], this.interaction, scale);
            this.moving = isGenericMove(this.interaction, this.seats);
            const clips = this.seats.map((seat) => {
                if (seat.role === NO_ROLE) return null;
                const meta = byRole[seat.seat];
                const name = meta.roles?.[seat.role]?.animation;
                const clip = name ? clipOf(meta, name) : null;
                if (!clip) return null;
                return { seconds: finite(clip.length), looping: clip.loop === "repeat", startDelay: delaysByRole[seat.seat].get(name) ?? 0 };
            });
            this.length = this.fixedLength ?? Math.max(MIN_SESSION_SECONDS, petSessionSeconds(this.interaction, clips));
        } else {
            this.seats = [];
            this.moving = false;
            this.length = Infinity;
        }
        const count = this.interaction && this.metas.length === 1 ? 2 : this.metas.length;
        if (count !== this.count) {
            this.count = count;
            this.player.dirty = true;
        }
        this.time = 0;
        this.replay = true;
        this.quiet = null;
        this.seen = 0;
        this.origin = 0;
        this.laps = [];
        this.inputs.reset(this.player);
    }

    setInteraction(interaction) {
        this.choose(interaction);
    }

    paintOf(index) {
        const pet = Math.min(Math.max(0, index), this.paints.length - 1);
        return this.paints[pet === 1 && this.twinShared ? 0 : pet] ?? petPaint(null);
    }

    colorwayOf(pet = 0) {
        return this.paintOf(pet).colorway;
    }

    customOf(pet = 0) {
        return this.paintOf(pet).custom;
    }

    setColorway(colorway = NO_COLORWAY, pet = 0) {
        this.setPaint({ colorway }, pet);
    }

    setCustom(custom = null, pet = 0) {
        this.setPaint({ custom }, pet);
    }

    setPaint(paint = {}, pet = 0) {
        const current = this.paints[pet];
        if (!current) return;
        if (pet === 1 && this.twinShared) {
            this.twinShared = false;
            Object.assign(current, this.paints[0]);
        }
        Object.assign(current, petPaint({
            colorway: "colorway" in paint ? paint.colorway : current.colorway,
            custom: "custom" in paint ? paint.custom : current.custom,
        }));
        this.player.paintChanged();
    }

    actorFor(seat) {
        if (this.metas.length < 2) return seat;
        return this.firstIsA ? seat : 1 - seat;
    }

    play() {
        this.playing = true;
    }

    pause() {
        this.playing = false;
    }

    seek(time) {
        const limit = Number.isFinite(this.length) ? this.length : Infinity;
        const next = Math.max(0, Math.min(limit, finite(time)));
        this.playing = false;
        if (next !== this.time) {
            this.replay = true;
            this.quiet = next;
        }
        this.time = next;
    }

    resync() {
        if (!this.replay) this.quiet = this.time;
        this.replay = true;
    }

    restart() {
        this.time = 0;
        this.replay = true;
        this.quiet = null;
        this.playing = true;
        this.seen = 0;
        this.origin = 0;
        this.laps = [];
        this.inputs.reset(this.player);
    }

    nextLap(span) {
        this.base += span;
        this.origin -= span;
        const laps = [...this.laps, 0].map((start) => start - span);
        while (laps.length > 1 && laps[1] <= -PET_CATCH_UP_SECONDS) laps.shift();
        this.laps = laps;
        this.inputs.carry(span, PET_CATCH_UP_SECONDS);
    }

    dispose() {
        if (this.player.pets === this) this.player.clearPets();
    }
}

export class PreviewPlayer {
    static async create(canvas, options = {}) {
        let wasm = options.wasm ?? options.renderer?.wasm ?? options.engine?.wasm ?? null;
        if (!wasm) wasm = await instantiate(options.wasmUrl ?? defaultWasmUrl);
        const gl = canvas.getContext("webgl2", { alpha: true, antialias: true, premultipliedAlpha: false, depth: true });
        if (!gl) throw new Error("WebGL2 is not available");
        return new PreviewPlayer(canvas, gl, wasm, options);
    }

    constructor(canvas, gl, wasm, options = {}) {
        if (typeof wasm.renderer_frame_wearers_shaped !== "function") throw new Error("preview: this plusrenderer build has no frame path exports");
        this.canvas = canvas;
        this.gl = gl;
        this.wasm = wasm;
        this.engine = options.engine instanceof CosmeticEngine ? options.engine : new CosmeticEngine(wasm);
        this.capabilities = options.capabilities ?? AUTHORING_CAPABILITIES;
        this.clock = options.clock instanceof FrameClock ? options.clock : new FrameClock(options.clock ?? startClock());
        this.sounds = new PreviewSounds(options.sounds === false ? null : options.sounds ?? webAudio());
        this.pixelRatio = options.pixelRatio ?? null;
        this.camera = { ...DEFAULT_CAMERA };
        this.lightLevel = 15;
        this.background = options.background ?? [0, 0, 0, 0];
        this.signals = {};
        this.variables = {};
        this.pendingEvents = 0;
        this.players = [{ skin: null, slim: false, version: 0 }];
        this.worn = [];
        this.emote = null;
        this.pets = null;
        this.actors = [];
        this.dirty = true;
        this.painted = true;
        this.lastSource = null;
        this.frameRequest = null;
        this.onError = options.onError ?? null;
        this.disposed = false;
        this.stats = { actors: 0, draws: 0, particles: 0 };
        this.skins = new SkinDrawer(gl);
        this.particles = new ParticleDrawer(gl);
        this.defaultSkin = gl.createTexture();
        uploadTexture(gl, this.defaultSkin, solidImage(64, 64, DEFAULT_SKIN_RGBA));
        this.white = gl.createTexture();
        uploadTexture(gl, this.white, solidImage(1, 1, WHITE_RGBA));
        this.skinTextures = new Map();
    }

    get muted() {
        return this.sounds.muted;
    }

    set muted(muted) {
        this.sounds.setMuted(muted);
    }

    get vanillaSoundsSkipped() {
        return this.sounds.vanillaSkipped;
    }

    parse(source, { mode = "authoring", identity } = {}) {
        const asset = this.engine.parse(source, { mode, capabilities: this.capabilities, identity });
        if (!asset.usable) {
            const { status, issues } = asset;
            asset.free();
            const error = new Error(`preview: the bundle can't be shown (${status})`);
            error.status = status;
            error.issues = issues;
            throw error;
        }
        return asset;
    }

    take(source) {
        if (isAssetSource(source)) {
            if (!source.usable) throw new Error(`preview: the bundle can't be shown (${source.status})`);
            return { asset: source, owned: false };
        }
        return { asset: this.parse(source), owned: true };
    }

    release(entry) {
        if (entry?.owned) entry.asset.free();
    }

    setPlayers(players) {
        this.players = players.slice(0, 2).map((player) => ({ skin: player?.skin ?? null, slim: !!player?.slim, version: 0 }));
        for (const texture of this.skinTextures.values()) this.gl.deleteTexture(texture.texture);
        this.skinTextures.clear();
        this.dirty = true;
    }

    setSkin(skin, { index = 0, slim } = {}) {
        const player = this.players[index];
        if (!player) return;
        player.skin = skin;
        player.version += 1;
        if (slim !== undefined && !!slim !== player.slim) {
            player.slim = !!slim;
            this.dirty = true;
        }
    }

    wear(source, { colorway = NO_COLORWAY, custom = null } = {}) {
        const entry = this.take(source);
        const meta = entry.asset.meta();
        const reason = meta.kind === "emote" ? "use playEmote for an emote"
            : meta.kind === "pet" ? "use showPet for a pet"
            : (meta.attach?.rig ?? "player") !== "player" ? `the ${meta.attach.rig} rig has no preview body`
            : null;
        if (reason) {
            this.release(entry);
            throw new Error(`preview: ${reason}`);
        }
        const worn = new WornCosmetic(this, entry, colorway, custom);
        this.worn.push(worn);
        this.dirty = true;
        return worn;
    }

    unwear(worn) {
        const at = this.worn.indexOf(worn);
        if (at < 0) return;
        this.worn.splice(at, 1);
        worn.removed = true;
        this.dirty = true;
        this.rebuild();
        this.release(worn.entry);
    }

    clearWorn() {
        const removed = this.worn.splice(0);
        if (!removed.length) return;
        this.dirty = true;
        this.rebuild();
        for (const worn of removed) {
            worn.removed = true;
            this.release(worn.entry);
        }
    }

    playEmote(source, { loop = true, length = null, files = null } = {}) {
        const entry = this.take(source);
        if (entry.asset.meta().kind !== "emote") {
            this.release(entry);
            throw new Error("preview: this bundle is not an emote");
        }
        this.clearEmote();
        try {
            this.emote = new EmoteSession(this, entry, { loop, length, delays: this.delaysOf(entry, files ?? (isFileMap(source) ? source : null)) });
        } catch (error) {
            this.release(entry);
            throw error;
        }
        this.dirty = true;
        return this.emote;
    }

    clearEmote() {
        const session = this.emote;
        if (!session) return;
        this.emote = null;
        this.dirty = true;
        this.rebuild();
        this.release(session.entry);
    }

    showPet(source, {
        partner = null, interaction = null, loop = true, owners, ids, files = null, partnerFiles = null, length = null,
        colorway = NO_COLORWAY, custom = null, partnerColorway, partnerCustom,
    } = {}) {
        const entries = [];
        try {
            for (const item of partner ? [source, partner] : [source]) {
                const entry = this.take(item);
                entries.push(entry);
                if (entry.asset.meta().kind !== "pet") throw new Error("preview: this bundle is not a pet");
            }
        } catch (error) {
            entries.forEach((entry) => this.release(entry));
            throw error;
        }
        let delays;
        try {
            delays = [
                this.delaysOf(entries[0], files ?? (isFileMap(source) ? source : null)),
                entries[1] ? this.delaysOf(entries[1], partnerFiles ?? (isFileMap(partner) ? partner : null)) : null,
            ];
        } catch (error) {
            entries.forEach((entry) => this.release(entry));
            throw error;
        }
        this.clearPets();
        this.pets = new PetSession(this, entries, {
            interaction, loop, owners: owners ?? [1n, 2n], ids: ids ?? [1, 2], length, delays,
            paints: [
                { colorway, custom },
                partner || partnerColorway !== undefined || partnerCustom !== undefined
                    ? { colorway: partnerColorway ?? NO_COLORWAY, custom: partnerCustom ?? null }
                    : null,
            ],
        });
        this.dirty = true;
        return this.pets;
    }

    delaysOf(entry, files) {
        const out = clipDelays(entry.asset.meta());
        if (!files) return out;
        if (isFileMap(files)) {
            for (const [clip, seconds] of startDelays(files)) out.set(clip, seconds);
            return out;
        }
        if (files === entry.asset) return out;
        const other = this.take(files);
        try {
            for (const [clip, seconds] of clipDelays(other.asset.meta())) out.set(clip, seconds);
        } finally {
            this.release(other);
        }
        return out;
    }

    clearPets() {
        const session = this.pets;
        if (!session) return;
        this.pets = null;
        this.dirty = true;
        this.rebuild();
        session.entries.forEach((entry) => this.release(entry));
    }

    setSignal(name, value) {
        this.signals[name] = finite(value);
    }

    setSignals(values) {
        this.signals = { ...values };
    }

    setVariable(name, value) {
        if (!(name in this.variables)) this.dirty = true;
        this.variables[name] = finite(value);
    }

    setVariables(values) {
        this.variables = { ...values };
        this.dirty = true;
    }

    pulse(events) {
        this.pendingEvents |= events >>> 0;
    }

    setLightLevel(level) {
        this.lightLevel = Math.min(15, Math.max(0, finite(level, 15)));
    }

    paintChanged() {
        this.painted = true;
    }

    fire(name) {
        if (this.dirty) this.rebuild();
        let fired = false;
        const now = this.clock.now();
        for (const actor of this.actors) {
            if (!actor.stage.fire(name)) continue;
            fired = true;
            if (actor.kind !== "player") continue;
            for (const worn of this.worn) {
                const plan = (worn.meta.effects?.moments ?? []).find((moment) => moment.name === name);
                if (plan && plan.clip !== null && actor.stage.playsMoments) {
                    actor.moment = { clip: plan.clip, started: now, until: now + finite(plan.seconds) };
                }
            }
        }
        return fired;
    }

    focus() {
        this.rebuild();
        this.arrange();
        const spots = this.actors.map((actor) => actor.spot).filter(Boolean);
        if (!spots.length) return this.camera;
        const xs = spots.map((spot) => spot.x);
        const zs = spots.map((spot) => spot.z);
        const centre = (values) => (Math.min(...values) + Math.max(...values)) / 2;
        const spread = Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...zs) - Math.min(...zs));
        const players = this.actors.some((actor) => actor.kind === "player");
        this.camera.targetX = centre(xs) * PIXELS_PER_BLOCK;
        this.camera.targetZ = centre(zs) * PIXELS_PER_BLOCK;
        this.camera.targetY = players ? DEFAULT_CAMERA.targetY : 6;
        this.camera.distance = (players ? DEFAULT_CAMERA.distance : 40) + spread * PIXELS_PER_BLOCK * 1.4;
        return this.camera;
    }

    disposeActors() {
        for (const actor of this.actors) {
            this.skins.release(actor.gpu);
            for (const texture of actor.textures.values()) this.gl.deleteTexture(texture.texture);
            for (const texture of actor.effectTextures.values()) if (texture) this.gl.deleteTexture(texture);
            actor.stage.dispose();
        }
        this.actors = [];
    }

    rebuild() {
        if (!this.dirty) return;
        this.dirty = false;
        this.disposeActors();
        const now = this.clock.now();
        const emote = this.emote;
        const emoting = emote ? (emote.duo ? 2 : 1) : 0;
        const playerCount = Math.max(this.players.length, emoting);
        const variableNames = Object.keys(this.variables);
        for (let i = 0; i < playerCount; i++) {
            const player = this.players[i] ?? this.players[0] ?? { skin: null, slim: false, version: 0 };
            const stage = new PreviewStage(this.engine, { rig: "player", slim: player.slim, wearer: BigInt(i + 1) });
            const actor = this.actorOf("player", i, stage, player);
            this.actors.push(actor);
            variableNames.forEach((name) => stage.variableId(name));
            this.worn.forEach((worn, n) => {
                stage.loadAsset(worn.asset, FIRST_WORN_SLOT + n);
                stage.play(FIRST_WORN_SLOT + n, now);
            });
            if (emote && i < emoting) {
                stage.loadEmote(emote.asset);
                actor.seat = i;
            }
        }
        if (this.pets) {
            const entries = this.pets.entries;
            Array.from({ length: this.pets.count }, (_, i) => entries[i] ?? entries[0]).forEach((entry, i) => {
                const stage = new PreviewStage(this.engine, { rig: "actor", body: false, wearer: BigInt(101 + i) });
                const actor = this.actorOf("pet", i, stage, null);
                this.actors.push(actor);
                [...PET_VARIABLES, ...variableNames].forEach((name) => stage.variableId(name));
                stage.loadAsset(entry.asset, PET_SLOT);
                stage.play(PET_SLOT, now);
            });
            this.pets.resync();
        }
        if (emote) {
            emote.resync();
        }
        this.painted = true;
    }

    actorOf(kind, index, stage, player) {
        return { kind, index, stage, player, seat: -1, clock: -Infinity, gpu: null, textures: new Map(), effectTextures: new Map(), moment: null, spot: null, root: IDENTITY, frame: null };
    }

    paintFor(actor, asset) {
        if (actor.kind === "pet" && this.pets) {
            const paint = this.pets.paintOf(actor.index);
            if (paint.colorway !== NO_COLORWAY || paint.custom) return paint;
        }
        return this.worn.find((item) => item.asset === asset) ?? null;
    }

    lastPaint() {
        return [...this.worn].reverse().find((worn) => worn.colorway !== NO_COLORWAY || worn.custom) ?? null;
    }

    paintActor(actor, last = this.lastPaint()) {
        if (!actor.stage.hasEffects()) return;
        const paint = actor.kind === "pet" ? this.paintFor(actor, actor.stage.assets.get(PET_SLOT)) : last;
        actor.stage.setEffectColors(paint?.colorway ?? NO_COLORWAY, paint?.custom ?? null);
    }

    forgetActor(actor) {
        actor.stage.forget();
        this.paintActor(actor);
    }

    applyPaint() {
        if (!this.painted) return;
        this.painted = false;
        const last = this.lastPaint();
        for (const actor of this.actors) this.paintActor(actor, last);
    }

    arrange() {
        const players = this.actors.filter((actor) => actor.kind === "player");
        const pets = this.actors.filter((actor) => actor.kind === "pet");
        const emote = this.emote;
        players.forEach((actor, i) => {
            if (emote?.duo && actor.seat >= 0) actor.spot = emote.seats[actor.seat].spot;
            else if (players.length === 1) actor.spot = { x: 0, y: 0, z: 0, yaw: 0 };
            else actor.spot = { x: i === 0 ? -PLAYER_GAP : PLAYER_GAP, y: 0, z: 0, yaw: 0 };
            if (emote && !emote.duo && actor.seat === 0) actor.spot = { x: players.length > 1 ? -PLAYER_GAP : 0, y: 0, z: 0, yaw: 0 };
            actor.root = rootAt(actor.spot);
        });
        const origin = players.length ? PET_BESIDE : { x: 0, y: 0, z: 0, yaw: 0 };
        const session = this.pets;
        if (!session) return;
        const motion = session.moving ? genericPetMotion(session.interaction, session.scale, session.time) : null;
        pets.forEach((actor) => {
            const meta = session.metas[actor.index] ?? session.metas[0];
            const scale = petScale(meta?.pet?.scale);
            let spot;
            if (session.interaction) {
                const seatIndex = session.actorFor(actor.index);
                const seat = session.seats[seatIndex];
                actor.petSeat = seat ?? null;
                spot = offsetSpot(motion?.spots[seatIndex] ?? seat?.spot ?? { x: 0, y: 0, z: 0, yaw: 0 }, origin);
                actor.speed = motion?.speeds[seatIndex] ?? 0;
            } else {
                actor.petSeat = null;
                actor.speed = 0;
                const side = pets.length > 1 ? (actor.index === 0 ? -IDLE_PET_GAP : IDLE_PET_GAP) : 0;
                spot = offsetSpot({ x: side, y: 0, z: 0, yaw: 0 }, origin);
            }
            actor.spot = spot;
            actor.origin = origin;
            actor.root = rootAt(spot, scale);
        });
    }

    baseInput(actor, events) {
        return { signals: this.signals, variables: this.variables, events, root: actor.root };
    }

    frameActor(actor, clock, input, heard) {
        actor.clock = Math.max(actor.clock, clock);
        const frame = actor.stage.frame(actor.clock, input);
        const events = actor.stage.events();
        if (events.length && heard) heard.push({ actor, events });
        return frame;
    }

    replayActor(actor, clock, input, heard, view) {
        const frame = this.frameActor(actor, clock, input, heard);
        actor.stage.sampleEffectRings(view);
        return frame;
    }

    emoteInput(actor, session, age, events, inputs = this) {
        const hold = holdInput(session.holds[actor.seat], age);
        const seat = session.seats[actor.seat];
        const mark = session.duo ? markFrom(seat.spot) : { anchor: [0, 0, 0], yaw: 0 };
        return {
            ...this.baseInput(actor, events),
            signals: inputs.signals,
            variables: inputs.variables,
            role: { index: seat.role, age: hold.age, anchor: mark.anchor, yaw: mark.yaw },
            held: hold.held,
            stopped: hold.stopped,
        };
    }

    petInput(actor, session, time, events, inputs = this) {
        const host = { ...PET_DEFAULTS, ...inputs.variables };
        const seat = actor.petSeat;
        if (!session.interaction || !seat) return { ...this.baseInput(actor, events), signals: inputs.signals, variables: host };
        const variables = petRoleVariables(host, session.interaction, seat.seat, time);
        if (session.moving) variables.pet_speed = actor.speed ?? 0;
        const input = {
            ...this.baseInput(actor, events),
            signals: time > 1e-9 ? petPairSignals(inputs.signals, session.interaction, seat) : inputs.signals,
            variables,
        };
        if (seat.role !== NO_ROLE) {
            const origin = actor.origin ?? { x: 0, y: 0, z: 0 };
            input.role = { index: seat.role, age: time, anchor: [origin.x, origin.y, origin.z], yaw: 0 };
        }
        return input;
    }

    stepSessions(seconds) {
        const emote = this.emote;
        if (emote && !emote.finished && emote.playing && seconds > 0) {
            emote.time += seconds;
            emote.reached = Math.max(emote.reached, emote.time);
        }
        const pets = this.pets;
        if (pets?.interaction) {
            const elapsed = pets.time + seconds;
            const step = advanceSession(pets.time, pets.length, pets.playing, seconds, pets.loop);
            pets.time = step.time;
            if (step.replay) {
                pets.nextLap(elapsed);
                this.breakPetRings();
            }
            if (step.ended) pets.playing = false;
        } else if (pets && pets.playing && seconds > 0) {
            pets.time += seconds;
        }
    }

    runEmote(now, events, heard) {
        const session = this.emote;
        const actors = this.actors.filter((actor) => actor.kind === "player" && actor.seat >= 0).sort((a, b) => a.seat - b.seat);
        if (!session || !actors.length) return;
        let quiet = -Infinity;
        let replayed = false;
        const since = Math.min(session.heldTo, session.time);
        if (session.replay) {
            session.replay = false;
            replayed = true;
            if (session.quiet !== null) quiet = Math.min(session.quiet, session.time);
            session.quiet = null;
            session.base = Math.max(now, session.clockEnd);
            for (const actor of actors) this.forgetActor(actor);
            const from = Math.max(0, session.time - EMOTE_CATCH_UP_SECONDS);
            const stopping = session.stopping;
            session.holds = freshHolds(actors.length);
            if (stopping) for (const hold of session.holds) Object.assign(hold, { stopping: true, stoppedAt: from });
            session.heldTo = from - HOLD_STEP_SECONDS;
            session.inputs.trim(session.time);
        }
        session.inputs.note(this, since, session.time - EMOTE_CATCH_UP_SECONDS);
        const view = effectView(orbitView(this.camera));
        const sample = (age, listen, bits) => actors.map((actor) => this.replayActor(actor, session.clockAt(age), this.emoteInput(actor, session, age, bits, session.inputs.at(age)), listen ? heard : null, view).status);
        let sampled = session.heldTo;
        const ticks = holdTicks(session.heldTo, session.time);
        const times = replayed ? session.inputs.eventTimes(sampled, session.time) : [];
        for (const { age, tick } of replayPoints(ticks, quiet, times)) {
            const statuses = sample(age, age > quiet, replayed ? session.inputs.eventsIn(sampled, age) : 0);
            sampled = Math.max(sampled, age);
            if (tick) reactHolds(session.holds, statuses, age);
        }
        const pulsed = replayed ? (events | session.inputs.eventsIn(sampled, session.time)) >>> 0 : events;
        session.heldTo = Math.max(session.heldTo, session.time);
        actors.forEach((actor) => {
            actor.frame = this.frameActor(actor, session.clockAt(session.time), this.emoteInput(actor, session, session.time, pulsed), heard);
        });
        session.inputs.pulse(session.time, events);
        session.clockEnd = Math.max(session.clockEnd, session.clockAt(session.time));
        session.statuses = actors.map((actor) => actor.frame.status);
        if (!session.finished && holdsFinished(session.holds, session.time)) {
            if (session.loop && !session.stopping) {
                session.rewind();
            } else {
                session.finished = true;
                session.playing = false;
            }
        }
        return now;
    }

    breakPetRings() {
        for (const actor of this.actors) if (actor.kind === "pet") actor.stage.breakRings();
    }

    runPets(now, events, heard) {
        const session = this.pets;
        const actors = this.actors.filter((actor) => actor.kind === "pet");
        if (!session || !actors.length) return now;
        const since = Math.min(session.seen, session.time);
        if (session.replay) session.inputs.trim(session.time);
        session.inputs.note(this, since, session.time - PET_CATCH_UP_SECONDS);
        let pulsed = events;
        if (session.replay) {
            session.replay = false;
            const time = session.time;
            const quiet = session.quiet === null ? -Infinity : Math.min(session.quiet, time);
            session.quiet = null;
            const origin = session.interaction ? session.origin : 0;
            const from = Math.max(origin, time - PET_CATCH_UP_SECONDS);
            const base = this.clock.reserve(time - from) - from;
            session.base = base;
            const starts = [...(session.interaction ? session.laps : []), 0];
            let lap = 0;
            const lapStart = (age) => {
                while (lap + 1 < starts.length && starts[lap + 1] <= age) lap++;
                return starts[lap];
            };
            for (const actor of actors) {
                this.forgetActor(actor);
                actor.stage.play(PET_SLOT, base + origin);
            }
            const view = effectView(orbitView(this.camera));
            let sampledLap = null;
            const enterLap = (age) => {
                const start = lapStart(age);
                if (sampledLap !== null && start !== sampledLap) this.breakPetRings();
                sampledLap = start;
                return start;
            };
            const sample = (age, listen, bits) => {
                const lapAge = age - enterLap(age);
                session.time = lapAge;
                this.arrange();
                actors.forEach((actor) => this.replayActor(actor, base + age, this.petInput(actor, session, lapAge, bits, session.inputs.at(age)), listen ? heard : null, view));
            };
            let sampled = from - HOLD_STEP_SECONDS;
            const ticks = [...leadTicks(from), ...holdTicks(sampled, time)].filter((tick) => tick < time);
            const marks = [...session.inputs.eventTimes(sampled, time), ...starts.filter((start) => start > sampled && start < time)];
            for (const { age } of replayPoints(ticks, quiet, marks)) {
                sample(age, age >= 0 && age > quiet, session.inputs.eventsIn(sampled, age));
                sampled = Math.max(sampled, age);
            }
            pulsed = (pulsed | session.inputs.eventsIn(sampled, time)) >>> 0;
            enterLap(time);
            session.time = time;
            this.arrange();
            now = this.clock.now();
        }
        actors.forEach((actor) => {
            actor.frame = this.frameActor(actor, session.clockAt(session.time), this.petInput(actor, session, session.time, pulsed), heard);
        });
        session.inputs.pulse(session.time, events);
        session.seen = session.time;
        return now;
    }

    tick() {
        this.rebuild();
        const source = this.clock.source();
        const seconds = this.lastSource === null ? 0 : Math.min(MAX_STEP_SECONDS, Math.max(0, source - this.lastSource));
        this.lastSource = source;
        this.stepSessions(seconds);
        this.arrange();
        this.applyPaint();
        const events = this.pendingEvents;
        this.pendingEvents = 0;
        const heard = [];
        let now = this.clock.now();
        if (this.emote) now = this.runEmote(now, events, heard) ?? now;
        if (this.pets) now = this.runPets(now, events, heard);
        for (const actor of this.actors) {
            if (actor.kind !== "player" || actor.seat >= 0) continue;
            const input = this.baseInput(actor, events);
            if (actor.moment) {
                if (now <= actor.moment.until) input.play = { clip: actor.moment.clip, age: now - actor.moment.started };
                else actor.moment = null;
            }
            actor.frame = this.frameActor(actor, now, input, heard);
        }
        for (const { actor, events: list } of heard) {
            this.sounds.play(list.filter((event) => event.kind === EFFECT_SOUND), (event) => actor.stage.sound(event));
        }
        return now;
    }

    resize(scale = this.pixelRatio ?? globalThis.devicePixelRatio ?? 1) {
        const width = Math.max(1, Math.round(this.canvas.clientWidth * scale));
        const height = Math.max(1, Math.round(this.canvas.clientHeight * scale));
        if (this.canvas.width !== width || this.canvas.height !== height) {
            this.canvas.width = width;
            this.canvas.height = height;
        }
        return [width, height];
    }

    textureOf(actor, slot, frame) {
        const gl = this.gl;
        let key;
        let source;
        if (actor.kind === "player" && slot === PLAYER_SLOT) {
            const player = actor.player;
            if (!player?.skin) return this.defaultSkin;
            const known = this.skinTextures.get(player);
            if (known && known.version === player.version) return known.texture;
            const texture = known?.texture ?? gl.createTexture();
            uploadTexture(gl, texture, player.skin);
            this.skinTextures.set(player, { texture, version: player.version });
            return texture;
        }
        const asset = actor.kind === "player" && slot === PROPS_SLOT ? this.emote?.asset : actor.stage.assets.get(slot);
        if (!asset?.usable || asset.textureCount() === 0) return this.white;
        const paint = this.paintFor(actor, asset);
        let colorway = paint?.colorway ?? NO_COLORWAY;
        if (frame.colorway >= 0 && asset === this.reactingAsset(actor)) colorway = frame.colorway;
        const custom = paint?.custom ?? null;
        key = paintKey(colorway, custom);
        const known = actor.textures.get(slot);
        if (known && known.key === key) return known.texture;
        const info = asset.textureInfo(0);
        source = { width: info.width, height: info.height, data: asset.resolveTexture(0, { colorway, custom }) };
        const texture = known?.texture ?? gl.createTexture();
        uploadTexture(gl, texture, source);
        actor.textures.set(slot, { texture, key });
        return texture;
    }

    reactingAsset(actor) {
        for (const [slot, asset] of [...actor.stage.assets].sort((a, b) => a[0] - b[0])) {
            if (slot === PLAYER_SLOT && actor.kind === "player") continue;
            if ((asset.meta()?.reactions ?? []).some((reaction) => reaction.colorway)) return asset;
        }
        return null;
    }

    effectTextureOf(actor, id) {
        if (actor.effectTextures.has(id)) return actor.effectTextures.get(id) ?? this.white;
        const found = actor.stage.effectTexture(id);
        let texture = null;
        if (found?.image) {
            texture = this.gl.createTexture();
            uploadTexture(this.gl, texture, found.image);
        }
        actor.effectTextures.set(id, texture);
        return texture ?? this.white;
    }

    draw() {
        const gl = this.gl;
        const [width, height] = this.resize();
        gl.viewport(0, 0, width, height);
        gl.clearColor(...this.background);
        gl.clearDepth(1);
        gl.enable(gl.DEPTH_TEST);
        gl.depthFunc(gl.LEQUAL);
        gl.depthMask(true);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.frontFace(gl.CCW);
        gl.cullFace(gl.BACK);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        const viewAffine = orbitView(this.camera);
        const view = mat4(viewAffine);
        const project = new Float32Array(projection(width, height));
        let draws = 0;
        for (const actor of this.actors) {
            if (!actor.frame) continue;
            const layout = actor.stage.skinLayout();
            if (!actor.gpu || actor.gpu.layout !== layout) {
                this.skins.release(actor.gpu);
                actor.gpu = this.skins.upload(actor.stage.skin());
            }
            const toPixels = scaleAffine(PIXELS_PER_BLOCK);
            const model = mat4(actor.kind === "player" ? multiply(toPixels, actor.root) : toPixels);
            draws += this.skins.draw(actor.gpu, actor.frame, model, view, project, (slot) => this.textureOf(actor, slot, actor.frame), this.lightLevel);
        }
        const particleView = effectView(viewAffine);
        const light = lightLevelBrightness(this.lightLevel);
        let particles = 0;
        for (const actor of this.actors) {
            if (!actor.frame || !actor.stage.hasEffects()) continue;
            const quads = actor.stage.effects(particleView);
            particles += this.particles.draw(quads, particleView.slice(0, 3), view, project, (id) => this.effectTextureOf(actor, id), light);
        }
        this.stats = { actors: this.actors.length, draws, particles };
        return this.stats;
    }

    render() {
        if (this.disposed) return this.stats;
        this.tick();
        return this.draw();
    }

    start() {
        if (this.frameRequest !== null || this.disposed) return;
        const loop = () => {
            this.frameRequest = null;
            try {
                this.render();
                this.frameFailing = false;
            } catch (error) {
                if (!this.frameFailing) {
                    if (this.onError) this.onError(error);
                    else console.error(error);
                }
                this.frameFailing = true;
            }
            this.frameRequest = requestAnimationFrame(loop);
        };
        this.frameRequest = requestAnimationFrame(loop);
    }

    stop() {
        if (this.frameRequest !== null) cancelAnimationFrame(this.frameRequest);
        this.frameRequest = null;
    }

    attachOrbit(element = this.canvas, { yawSpeed = 0.6, pitchSpeed = 0.4, zoomSpeed = 0.08, minDistance = 20, maxDistance = 400 } = {}) {
        let drag = null;
        const down = (event) => {
            drag = { x: event.clientX, y: event.clientY, id: event.pointerId };
            element.setPointerCapture?.(event.pointerId);
        };
        const move = (event) => {
            if (!drag || event.pointerId !== drag.id) return;
            this.camera.yaw = wrapDegrees(this.camera.yaw + (event.clientX - drag.x) * yawSpeed);
            this.camera.pitch = Math.max(-89, Math.min(89, this.camera.pitch - (event.clientY - drag.y) * pitchSpeed));
            drag.x = event.clientX;
            drag.y = event.clientY;
        };
        const up = (event) => {
            if (drag && event.pointerId === drag.id) drag = null;
        };
        const wheel = (event) => {
            event.preventDefault();
            const factor = Math.exp(Math.sign(event.deltaY) * zoomSpeed);
            this.camera.distance = Math.max(minDistance, Math.min(maxDistance, this.camera.distance * factor));
        };
        element.addEventListener("pointerdown", down);
        element.addEventListener("pointermove", move);
        element.addEventListener("pointerup", up);
        element.addEventListener("pointercancel", up);
        element.addEventListener("wheel", wheel, { passive: false });
        return () => {
            element.removeEventListener("pointerdown", down);
            element.removeEventListener("pointermove", move);
            element.removeEventListener("pointerup", up);
            element.removeEventListener("pointercancel", up);
            element.removeEventListener("wheel", wheel);
        };
    }

    dispose() {
        if (this.disposed) return;
        this.stop();
        this.disposeActors();
        for (const worn of this.worn.splice(0)) this.release(worn.entry);
        if (this.emote) this.release(this.emote.entry);
        this.emote = null;
        if (this.pets) this.pets.entries.forEach((entry) => this.release(entry));
        this.pets = null;
        const gl = this.gl;
        for (const texture of this.skinTextures.values()) gl.deleteTexture(texture.texture);
        this.skinTextures.clear();
        gl.deleteTexture(this.defaultSkin);
        gl.deleteTexture(this.white);
        this.skins.dispose();
        this.particles.dispose();
        this.sounds.dispose();
        this.disposed = true;
    }
}

export { EFFECT_PARTICLE, EFFECT_SOUND, NO_COLORWAY };
