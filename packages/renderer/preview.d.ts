export type Affine = number[];
export type Signals = Readonly<Record<string, number>>;
export type CustomColors = ReadonlyMap<number, number> | ReadonlyArray<readonly [number, number]> | Readonly<Record<string | number, number>>;
export type ImageLike = TexImageSource | { width: number; height: number; data: ArrayBufferView };
export type BundleSource = Uint8Array | ArrayBuffer | ArrayBufferView | Map<string, string | Uint8Array> | Record<string, string | Uint8Array>;
export type Interaction = "greet" | "follow" | "nap" | "play" | "chase";
export type HoldStep = "play" | "hold" | "end";

export interface CosmeticAssetLike {
    readonly status: string;
    readonly usable: boolean;
    readonly issues: unknown[];
    readonly handle: number;
    meta(): any;
    live(): number;
    free(): void;
    textureCount(): number;
    textureInfo(index: number): { width: number; height: number; indexed: boolean; bytes: number };
    resolveTexture(index: number, options?: { colorway?: number; custom?: CustomColors | null }): Uint8Array;
}

export interface Spot {
    x: number;
    y: number;
    z: number;
    yaw: number;
}

export interface Camera {
    yaw: number;
    pitch: number;
    distance: number;
    targetX?: number;
    targetY: number;
    targetZ?: number;
}

export interface Seat {
    seat: number;
    role: number;
    spot: Spot;
}

export interface RoleHold {
    holding: boolean;
    heldAge: number;
    heldFrom: number;
    stopping: boolean;
    stoppedAt: number;
}

export interface HoldInput {
    age: number;
    held: boolean;
    stopped: boolean;
}

export interface SessionStep {
    time: number;
    replay: boolean;
    ended: boolean;
}

export interface PetMotion {
    spots: Spot[];
    speeds: number[];
}

export interface PetClip {
    seconds: number;
    looping: boolean;
    startDelay: number;
}

export interface PetFacts {
    archetype?: string;
    interactions?: Readonly<Record<string, readonly string[]>>;
}

export interface ParticleBatch {
    texture: number;
    blend: number;
    emissive: number;
    start: number;
    count: number;
}

export interface ParticleBuffers {
    positions: Float32Array;
    uvs: Float32Array;
    colors: Float32Array;
    indices: Uint32Array;
    batches: ParticleBatch[];
    count: number;
    capacity: number;
}

export interface EffectRingExports {
    renderer_sample_effect_rings(renderer: number, viewPtr: number, viewLen: number): number;
}

export interface StageInput {
    signals?: Signals;
    variables?: Signals;
    events?: number;
    root?: ArrayLike<number>;
    play?: { clip: number; age: number } | null;
    role?: { index: number; age: number; anchor?: readonly [number, number, number]; yaw?: number } | null;
    held?: boolean;
    stopped?: boolean;
}

export interface StageFrame {
    bones: Float32Array;
    offsets: Float32Array;
    colorway: number;
    status: number;
}

export interface StageEvent {
    wearer: number;
    slot: number;
    controller: number;
    source: number;
    kind: number;
    effect: number;
    at: [number, number, number];
    volume: number;
    pitch: number;
}

export interface SkinRun {
    texture: number;
    start: number;
    count: number;
    cull: boolean;
}

export interface SkinMesh {
    layout: number;
    quads: number;
    boneCount: number;
    positions: Float32Array;
    normals: Float32Array;
    uvs: Float32Array;
    glow: Uint8Array;
    bones: Uint32Array;
    runs: SkinRun[];
}

export interface PixelImage {
    width: number;
    height: number;
    data: Uint8Array;
}

export interface PreviewSound {
    key: string;
    bytes: Uint8Array | null;
    vanilla: string | null;
}

export interface SoundOutput {
    decode(bytes: Uint8Array): Promise<unknown>;
    play(buffer: unknown, volume: number, pitch: number): void;
    stop(): void;
    close?(): void;
}

export const PIXELS_PER_BLOCK: 16;
export const AFFINE_FLOATS: 12;
export const QUAD_FLOATS: 27;
export const VIEW_FLOATS: 12;
export const EFFECT_EVENT_FLOATS: 5;
export const EFFECT_PARTICLE: 0;
export const EFFECT_SOUND: 1;
export const EFFECT_VANILLA_PARTICLE: 2;
export const LAYOUT_INTS: 12;
export const NO_PLAY: -1;
export const NO_ROLE: -1;
export const NO_COLORWAY: -1;
export const EVENT_JUMP: number;
export const EVENT_EQUIP: number;
export const EMOTE_ENDED: number;
export const EMOTE_CANCEL: number;
export const EMOTE_CANCEL_WALKING: number;
export const EMOTE_CANCEL_SPRINTING: number;
export const EMOTE_CANCEL_CROUCHING: number;
export const EMOTE_CANCEL_FALLING: number;
export const EMOTE_CANCEL_ELYTRA_FLYING: number;
export const EMOTE_CANCEL_SWIMMING: number;
export const EMOTE_CANCEL_SLEEPING: number;
export const EMOTE_CANCEL_SWINGING: number;
export const EMOTE_HELD: number;
export const HOLD_STEP_SECONDS: number;
export const FADE_SECONDS: number;
export const MAX_STEP_SECONDS: number;
export const MIN_SESSION_SECONDS: number;
export const EMOTE_CATCH_UP_SECONDS: number;
export const PET_CATCH_UP_SECONDS: number;
export const ROLE_IDS: readonly ["a", "b"];
export const INTERACTIONS: readonly Interaction[];
export const INTERACTION_SECONDS: Readonly<Record<Interaction, number>>;
export const PET_INTERACTION_VALUES: Readonly<Record<Interaction, number>>;
export const PET_PLAYING: number;
export const PET_SLEEPING: number;
export const PET_SCALE_MIN: number;
export const PET_SCALE_MAX: number;
export const PET_VARIABLES: readonly string[];
export const PET_DEFAULTS: Signals;
export const NEUTRAL_SIGNALS: Signals;
export const DEFAULT_CAMERA: Readonly<Required<Camera>>;
export const IDENTITY: readonly number[];

export function multiply(a: ArrayLike<number>, b: ArrayLike<number>): Affine;
export function invert(a: ArrayLike<number>): Affine;
export function transformPoint(a: ArrayLike<number>, p: ArrayLike<number>): [number, number, number];
export function scaleAffine(k: number): Affine;
export function wrapDegrees(degrees: number): number;
export function rootAt(spot?: Partial<Spot>, scale?: number): Affine;
export function offsetSpot(spot: Spot, origin: Spot | { x: number; y: number; z: number }): Spot;
export function markFrom(spot: Spot): { anchor: [number, number, number]; yaw: number };
export function orbitView(camera?: Camera): Affine;
export function projection(width: number, height: number): number[];
export function mat4(affine: ArrayLike<number>): Float32Array;
export function projectPoint(camera: Camera, width: number, height: number, p: ArrayLike<number>): [number, number, number];
export function effectView(view: ArrayLike<number>): number[];

export class FrameClock {
    constructor(source?: () => number);
    readonly source: () => number;
    offset: number;
    last: number;
    now(): number;
    reserve(span: number): number;
}

export interface InputSnapshot {
    readonly signals: Signals;
    readonly variables: Signals;
}

export interface InputLogEntry extends InputSnapshot {
    readonly time: number;
}

export interface InputLogEvent {
    readonly time: number;
    readonly bits: number;
}

export class InputLog {
    constructor(current: InputSnapshot, options?: { variableDefaults?: Signals; limit?: number });
    readonly variableDefaults: Signals;
    readonly limit: number;
    readonly entries: ReadonlyArray<InputLogEntry>;
    readonly events: ReadonlyArray<InputLogEvent>;
    at(age: number): InputLogEntry;
    reset(current: InputSnapshot): void;
    carry(span: number, window: number): void;
    trim(time: number): void;
    note(current: InputSnapshot, time: number, from?: number): void;
    pulse(time: number, bits: number): void;
    eventsIn(after: number, through: number): number;
    eventTimes(after: number, before: number): number[];
}

export function holdStep(status: number, holding: boolean, shared: () => boolean, partnerDone: () => boolean, onMark?: () => boolean): HoldStep;
export function freshHolds(count: number): RoleHold[];
export function holdInput(hold: RoleHold, age: number): HoldInput;
export function reactHolds(holds: RoleHold[], statuses: readonly number[], age: number, onMark?: () => boolean): void;
export function holdTicks(after: number, through: number): number[];
export function holdsFinished(holds: readonly RoleHold[], age: number): boolean;
export function advanceSession(time: number, length: number, playing: boolean, seconds: number, loop?: boolean): SessionStep;

export function startDelays(files: BundleSource | null | undefined): Map<string, number>;
export function clipDelays(meta: any): Map<string, number>;
export function clipOf(meta: any, name: string): { name: string; length: number; loop: string; start_delay?: number } | null;
export function mainClip(meta: any): { name: string; length: number; loop: string; start_delay?: number } | null;
export function emoteSeconds(meta: any, names: readonly (string | null | undefined)[], delays?: Map<string, number>): number;
export function roleIndex(roles: readonly { id?: string }[], seat: number): number;
export function spotOf(role: { offset?: readonly number[]; yaw?: number } | null | undefined): Spot;
export function emoteSeats(roles: readonly { id?: string; offset?: readonly number[]; yaw?: number }[]): Seat[];

export function petScale(scale: number): number;
export function supportsInteraction(pet: PetFacts | null | undefined, interaction: string, role?: number | null): boolean;
export function petInteractions(pet: PetFacts | null | undefined): Interaction[];
export function pairOrientation(seed: bigint | number, first: PetFacts | null | undefined, second: PetFacts | null | undefined, interaction: string): boolean;
export function pairChoice(seed: bigint | number, first: PetFacts, second: PetFacts): { interaction: Interaction; firstIsA: boolean } | null;
export function genericPetSpot(interaction: string, seat: number, scale: number): Spot;
export function pairSeats(first: { pet?: PetFacts; roles?: readonly any[] }, second: { pet?: PetFacts; roles?: readonly any[] }, interaction: string, scale: number): Seat[];
export function isGenericMove(interaction: string, seats: readonly Seat[]): boolean;
export function petSessionSeconds(interaction: string, clips: ReadonlyArray<PetClip | null>): number;
export function petRoleVariables(host: Signals, interaction: string, seat: number, time: number): Record<string, number>;
export function petPairSignals(values: Signals, interaction: string, seat: Seat): Signals;
export function genericPetMotion(interaction: string, scale: number, time: number, spin?: number): PetMotion;

export function particleCapacity(count: number): number;
export function particleBuffers(quads: Float32Array, camera: readonly number[], toWorld?: ArrayLike<number>, reuse?: ParticleBuffers | null): ParticleBuffers;

export function downmix<B extends { numberOfChannels: number; length: number; sampleRate: number; getChannelData(channel: number): Float32Array }>(buffer: B, create: (channels: number, length: number, rate: number) => B): B;
export function webAudio(): SoundOutput | null;

export class PreviewSounds {
    constructor(output?: SoundOutput | null);
    readonly output: SoundOutput | null;
    muted: boolean;
    onSkipped: ((names: string[]) => void) | null;
    readonly vanillaSkipped: string[];
    setMuted(muted: boolean): void;
    play(events: readonly { volume: number; pitch: number }[], lookup: (event: any) => PreviewSound | null): number;
    stop(): void;
    forget(): void;
    dispose(): void;
}

export function stageSections(options?: { play?: boolean; role?: boolean; hold?: boolean; variables?: number }): Record<string, number | boolean>;

export class PreviewStage {
    constructor(engine: { wasm: any; inputLayout(sections: any): Record<string, number> }, options?: { rig?: "player" | "actor" | "boat" | "minecart" | "horse"; body?: boolean; slim?: boolean; wearer?: bigint | number });
    readonly rig: string;
    readonly body: boolean;
    readonly slim: boolean;
    readonly wearer: bigint;
    readonly assets: Map<number, CosmeticAssetLike>;
    readonly emote: CosmeticAssetLike | null;
    readonly playsMoments: boolean;
    loadAsset(asset: CosmeticAssetLike, slot: number): number;
    loadEmote(asset: CosmeticAssetLike): void;
    variableId(name: string): number;
    layout(): any;
    frame(clock: number, input?: StageInput): StageFrame;
    skin(): SkinMesh;
    skinLayout(): number;
    fire(name: string): boolean;
    setEffectColors(colorway?: number, custom?: CustomColors | null): void;
    hasEffects(): boolean;
    sampleEffectRings(view: ArrayLike<number>): void;
    effects(view: ArrayLike<number>): Float32Array;
    effectTexture(id: number): { slot: number; path: string; image: PixelImage | null } | null;
    events(): StageEvent[];
    sound(event: StageEvent): PreviewSound | null;
    play(slot: number, at: number): void;
    forget(): void;
    breakRings(): boolean;
    dispose(): void;
}

export function effectTextureNames(meta: any): string[];
export function effectImage(asset: CosmeticAssetLike, path: string): PixelImage | null;
export function assetKey(asset: object): number;
export function assetSound(asset: CosmeticAssetLike, index: number): PreviewSound | null;
export function solidImage(width: number, height: number, rgba: readonly number[]): PixelImage;

export interface WornCosmetic {
    readonly asset: CosmeticAssetLike;
    readonly meta: any;
    readonly colorway: number;
    readonly custom: CustomColors | null;
    readonly removed: boolean;
    readonly namedEffects: string[];
    readonly moments: { name: string; clip: number | null; seconds: number }[];
    readonly colorways: string[];
    setColorway(colorway?: number): void;
    setCustom(custom?: CustomColors | null): void;
    fire(name: string): boolean;
    remove(): void;
}

export interface EmoteSession {
    readonly asset: CosmeticAssetLike;
    readonly meta: any;
    readonly seats: Seat[];
    readonly duo: boolean;
    length: number;
    readonly known: number;
    loop: boolean;
    readonly time: number;
    readonly playing: boolean;
    readonly finished: boolean;
    readonly stopping: boolean;
    readonly statuses: number[];
    readonly signalLog: ReadonlyArray<InputLogEntry>;
    readonly eventLog: ReadonlyArray<InputLogEvent>;
    play(): void;
    pause(): void;
    stop(): void;
    seek(time: number): void;
    restart(): void;
    dispose(): void;
}

export interface PetPaint {
    colorway: number;
    custom: CustomColors | null;
}

export interface PetSession {
    readonly metas: any[];
    readonly paints: ReadonlyArray<Readonly<PetPaint>>;
    readonly interaction: Interaction | null;
    readonly interactions: Interaction[];
    readonly seats: Seat[];
    readonly firstIsA: boolean;
    readonly count: number;
    readonly moving: boolean;
    readonly length: number;
    loop: boolean;
    readonly time: number;
    readonly playing: boolean;
    readonly signalLog: ReadonlyArray<InputLogEntry>;
    readonly eventLog: ReadonlyArray<InputLogEvent>;
    setInteraction(interaction: Interaction | "auto" | null): void;
    colorwayOf(pet?: number): number;
    customOf(pet?: number): CustomColors | null;
    setColorway(colorway?: number, pet?: number): void;
    setCustom(custom?: CustomColors | null, pet?: number): void;
    setPaint(paint?: Partial<PetPaint>, pet?: number): void;
    play(): void;
    pause(): void;
    seek(time: number): void;
    restart(): void;
    dispose(): void;
}

export interface PreviewActor {
    readonly kind: "player" | "pet";
    readonly index: number;
    readonly stage: PreviewStage;
    readonly seat: number;
    readonly spot: Spot | null;
    readonly root: Affine;
    readonly frame: StageFrame | null;
}

export interface PreviewOptions {
    wasm?: WebAssembly.Exports;
    renderer?: { wasm: WebAssembly.Exports };
    engine?: { wasm: WebAssembly.Exports };
    wasmUrl?: URL | string;
    clock?: FrameClock | (() => number);
    sounds?: SoundOutput | false | null;
    capabilities?: readonly string[];
    pixelRatio?: number;
    background?: [number, number, number, number];
    onError?: (error: unknown) => void;
}

export class PreviewPlayer {
    static create(canvas: HTMLCanvasElement, options?: PreviewOptions): Promise<PreviewPlayer>;
    constructor(canvas: HTMLCanvasElement, gl: WebGL2RenderingContext, wasm: WebAssembly.Exports, options?: PreviewOptions);
    readonly canvas: HTMLCanvasElement;
    readonly gl: WebGL2RenderingContext;
    readonly wasm: WebAssembly.Exports;
    readonly engine: any;
    readonly clock: FrameClock;
    readonly sounds: PreviewSounds;
    camera: Required<Camera>;
    background: [number, number, number, number];
    signals: Record<string, number>;
    variables: Record<string, number>;
    readonly lightLevel: number;
    readonly players: ReadonlyArray<{ skin: ImageLike | null; slim: boolean; version: number }>;
    readonly worn: WornCosmetic[];
    readonly emote: EmoteSession | null;
    readonly pets: PetSession | null;
    readonly actors: PreviewActor[];
    readonly stats: { actors: number; draws: number; particles: number };
    onError: ((error: unknown) => void) | null;
    muted: boolean;
    readonly vanillaSoundsSkipped: string[];
    parse(source: BundleSource, options?: { mode?: "authoring" | "client"; identity?: bigint | number }): CosmeticAssetLike;
    setPlayers(players: ReadonlyArray<{ skin?: ImageLike | null; slim?: boolean }>): void;
    setSkin(skin: ImageLike | null, options?: { index?: number; slim?: boolean }): void;
    wear(source: BundleSource | CosmeticAssetLike, options?: { colorway?: number; custom?: CustomColors | null }): WornCosmetic;
    unwear(worn: WornCosmetic): void;
    clearWorn(): void;
    playEmote(source: BundleSource | CosmeticAssetLike, options?: { loop?: boolean; length?: number | null; files?: BundleSource | CosmeticAssetLike | null }): EmoteSession;
    clearEmote(): void;
    showPet(source: BundleSource | CosmeticAssetLike, options?: {
        partner?: BundleSource | CosmeticAssetLike | null;
        interaction?: Interaction | "auto" | null;
        loop?: boolean;
        owners?: [bigint | number, bigint | number];
        ids?: [number, number];
        files?: BundleSource | CosmeticAssetLike | null;
        partnerFiles?: BundleSource | CosmeticAssetLike | null;
        length?: number | null;
        colorway?: number;
        custom?: CustomColors | null;
        partnerColorway?: number;
        partnerCustom?: CustomColors | null;
    }): PetSession;
    clearPets(): void;
    setSignal(name: string, value: number): void;
    setSignals(values: Signals): void;
    setVariable(name: string, value: number): void;
    setVariables(values: Signals): void;
    pulse(events: number): void;
    setLightLevel(level: number): void;
    fire(name: string): boolean;
    focus(): Required<Camera>;
    resize(scale?: number): [number, number];
    tick(): number;
    draw(): { actors: number; draws: number; particles: number };
    render(): { actors: number; draws: number; particles: number };
    start(): void;
    stop(): void;
    attachOrbit(element?: HTMLElement, options?: { yawSpeed?: number; pitchSpeed?: number; zoomSpeed?: number; minDistance?: number; maxDistance?: number }): () => void;
    dispose(): void;
}
