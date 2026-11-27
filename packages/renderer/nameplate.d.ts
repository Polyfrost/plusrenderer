export type Wasm = WebAssembly.Exports & Record<string, any>;
export type WasmSource = URL | string | BufferSource;
export type BundleBytes = Uint8Array | ArrayBuffer | ArrayBufferView;
export type BundleFiles = Map<string, string | BundleBytes> | ReadonlyArray<readonly [string, string | BundleBytes]> | Readonly<Record<string, string | BundleBytes>>;
export type LoadMode = "client" | "authoring" | 0 | 1;
export type LoadedKind = "asset" | "draft" | "skipped" | "invalid";
export type ShaderDialect = "core_330" | "core_330_ubo" | "legacy_150" | "compat_130" | "es_300" | "sksl" | number;
export type MolangContext = "channel" | "slot_color" | "field" | "text" | "material" | number;
export type SignalName =
    | "is_sneaking" | "is_sprinting" | "is_swimming" | "is_in_water" | "is_in_rain" | "is_in_snow" | "is_on_fire"
    | "is_hurt" | "is_gliding" | "is_flying" | "is_falling" | "is_on_ground" | "is_riding" | "is_sleeping"
    | "is_emoting" | "is_in_combat" | "is_moving" | "ground_speed" | "vertical_speed" | "time_of_day"
    | "biome_temperature" | "is_in_ocean" | "dimension";
export type SignalSet = number | Iterable<SignalName | number>;
export type LayerSet = number | Iterable<number>;
export type RasterBandName = "all" | "under_text" | "over_text";
export type RasterBandValue = 0 | 1 | 2;
export type RasterBandInput = RasterBandName | RasterBandValue | null | undefined;
export type RasterStateValue = 0 | 1 | 2;
export type LayerDepth = "behind" | "background" | "over_background" | "over_text";
export type TextBand = "under_text" | "over_text";
export type LayerBlend = "alpha" | "additive";
export type CanvasComposite = "source-over" | "lighter";
export type TextModeName = "vanilla" | "motion" | "owned";
export type Wearer = bigint | number | string;

export declare const defaultWasmUrl: URL;

export declare const OUT: Readonly<{
    TAG_INTS: number;
    TAG_ASSET: number;
    TAG_VARIANT: number;
    TAG_FLAGS: number;
    TAG_DETAIL: number;
    TAG_LINE_COUNT: number;
    TAG_NAME_LINE: number;
    TAG_ALIGN: number;
    TAG_NAME_X: number;
    TAG_NAME_WIDTH: number;
    TAG_PREFIX_WIDTH: number;
    TAG_LINE_WIDTH_0: number;
    TAG_BG_X0: number;
    TAG_BG_Y0: number;
    TAG_BG_X1: number;
    TAG_BG_Y1: number;
    TAG_EXTERNAL_COLOR: number;
    TAG_SNEAK_ALPHA: number;
    TAG_GLYPH_OFFSET: number;
    TAG_GLYPH_COUNT: number;
    TAG_COLOR_OFFSET: number;
    TAG_COLOR_COUNT: number;
    GLYPH_FLOATS: number;
    SIGNAL_FLOATS: number;
    SIGNAL_EVENTS: number;
    OUT_TAGS: number;
    OUT_WORDS: number;
    OUT_STRIP_LAYOUT: number;
    OUT_HEADER: number;
    DIR_WORDS: number;
    DIR_OFFSET: number;
    DIR_LEN: number;
    DIR_STATUS: number;
    STATUS_OK: number;
    STATUS_VANILLA: number;
    STATUS_UNKNOWN_ASSET: number;
    STATUS_BAD_ROW: number;
    STATUS_DISABLED: number;
    STATUS_NO_ROOM: number;
    TAG_HEAD: number;
    HEAD_PLATE_DX: number;
    HEAD_PLATE_DY: number;
    HEAD_OWNS_BACKGROUND: number;
    HEAD_TEXT_MODE: number;
    HEAD_OPS: number;
    HEAD_MATERIALS: number;
    HEAD_MATERIAL_WORDS: number;
    HEAD_EVENTS: number;
    OP_WORDS: number;
    OP_X0: number;
    OP_Y0: number;
    OP_X1: number;
    OP_Y1: number;
    OP_ARGB: number;
    OP_BAND_LAYER_KIND: number;
    OP_AUX0: number;
    OP_AUX1: number;
    OP_KIND_RECT: number;
    OP_KIND_STRIP: number;
    OP_KIND_IMAGE: number;
    AUX_LOW: number;
    AUX_AXIS_Y: number;
    AUX_MIRROR: number;
    AUX_ADDITIVE: number;
    BAND_BEHIND: number;
    BAND_BACKGROUND: number;
    BAND_OVER_BACKGROUND: number;
    BAND_OVER_TEXT: number;
    BAND_BADGE: number;
    MAT_SHADER: number;
    MAT_BAND_LAYER: number;
    MAT_RUN: number;
    MAT_AREA_X0: number;
    MAT_AREA_Y0: number;
    MAT_AREA_X1: number;
    MAT_AREA_Y1: number;
    MAT_CLIP_COUNT: number;
    MAT_UNIFORM_WORDS: number;
    MAT_FIXED: number;
    CLIP_WORDS: number;
    MAT_MIRRORED: number;
    MAT_OPACITY_SHIFT: number;
    TEXT_COLOR: number;
    TEXT_SHADOW: number;
    TEXT_SHADOW_USE: number;
    TEXT_OUTLINE: number;
    TEXT_LETTER_COUNT: number;
    TEXT_FIXED: number;
    LETTER_WORDS: number;
    LETTER_DX: number;
    LETTER_DY: number;
    LETTER_ARGB: number;
    LETTER_SHADOW: number;
    LETTER_OPACITY: number;
    LETTER_SCALE_ROT: number;
    LETTER_OUTLINE: number;
    BADGE_WORDS: number;
    BADGE_X: number;
    BADGE_Y: number;
    BADGE_OPACITY: number;
    BADGE_TINT: number;
    BADGE_DECORATION: number;
    DECORATION_KIND_SHIFT: number;
    EVENT_WORDS: number;
    EVENT_CODE: number;
    EVENT_LAYER: number;
}>;

export declare const FLAG_DISPLAY: number;
export declare const FLAG_OWN: number;
export declare const FLAG_ANIMATED_NAMES_OFF: number;
export declare const FLAG_EFFECTS_CPU: number;
export declare const FLAG_EFFECTS_OFF: number;
export declare const FLAG_EXTERNAL_COLOR: number;
export declare const FLAG_BADGE: number;

export declare const TEXT_MODE_VANILLA: 0;
export declare const TEXT_MODE_MOTION: 1;
export declare const TEXT_MODE_OWNED: 2;

export declare const ALIGN_LEFT: 0;
export declare const ALIGN_RIGHT: 1;
export declare const ALIGN_CENTER: 2;

export declare const DETAIL_FULL: 0;
export declare const DETAIL_REDUCED: 1;
export declare const DETAIL_MINIMAL: 2;

export declare const MOTION_FULL: 0;
export declare const MOTION_REDUCED: 1;
export declare const MOTION_OFF: 2;

export declare const GLYPH_HEIGHT: 8;

export declare const SIGNALS: readonly SignalName[];
export declare const MOLANG_CONTEXTS: readonly ["channel", "slot_color", "field", "text", "material"];

export declare const RASTER_IDLE: 0;
export declare const RASTER_WHILE: 1;
export declare const RASTER_ONE_SHOT: 2;

export declare const RASTER_BAND_ALL: 0;
export declare const RASTER_BAND_UNDER_TEXT: 1;
export declare const RASTER_BAND_OVER_TEXT: 2;

export declare const RASTER_INFO: Readonly<{
    WORDS: 180;
    WIDTH: 0;
    HEIGHT: 1;
    X: 2;
    Y: 3;
    PLATE_DX: 4;
    PLATE_DY: 5;
    BADGE_SHOWN: 6;
    BADGE_X: 7;
    BADGE_Y: 8;
    BADGE_OPACITY: 9;
    TEXT_MODE: 10;
    LETTER_COUNT: 11;
    LETTERS: 12;
}>;

export declare function startClock(): () => number;
export declare function storeZip(files: BundleFiles): Uint8Array;

export interface TagRowInput {
    asset: number;
    variant?: number;
    flags?: number;
    detail?: number;
    lines?: readonly number[];
    nameLine?: number;
    align?: number;
    nameX?: number;
    nameWidth?: number;
    prefixWidth?: number;
    background?: readonly [number, number, number, number];
    externalColor?: number;
    sneakAlpha?: number;
    glyphOffset?: number;
    glyphCount?: number;
    colorOffset?: number;
    colorCount?: number;
}

export declare function tagRow(input: TagRowInput, row?: Int32Array): Int32Array;
export declare function instantiate(source?: WasmSource): Promise<Wasm>;
export declare function rasterBand(band: RasterBandInput): RasterBandValue;
export declare function layerBits(layers: LayerSet | null | undefined): number;

export interface LoadOptions {
    mode?: LoadMode;
    capabilities?: readonly string[];
    identity?: Wearer | null;
}

export interface MolangWarning {
    kind: string;
    name: string;
}

export type MolangCheck =
    | { ok: true; cost: number; warnings: MolangWarning[]; issues: unknown[] }
    | { ok: false; code: string; message: string; [key: string]: unknown };

export declare class NameplateEngine {
    static create(source?: WasmSource): Promise<NameplateEngine>;
    constructor(wasm: Wasm);
    readonly wasm: Wasm;
    readonly stoppedRaster: boolean;
    readonly observedRaster: boolean;
    sharedTextures?: RasterTextures | null;
    memory(): ArrayBuffer;
    put(bytes: Uint8Array, nonNull?: boolean): number;
    text(ptr: number, len: number): string;
    error(): string;
    json(len: number): any;
    load(source: BundleBytes | BundleFiles, options?: LoadOptions): Promise<NameplateAsset>;
    loadSync(source: BundleBytes | BundleFiles, options?: LoadOptions): NameplateAsset;
    constants(): any;
    checkMolang(source: string, context?: MolangContext): MolangCheck;
    nameplates(): Nameplates;
    rasterTextures(): RasterTextures;
    setRasterTexture(name: string, width: number, height: number, rgba: BundleBytes | null): void;
}

export declare class RasterTextures {
    constructor(engine: NameplateEngine);
    readonly engine: NameplateEngine;
    handle: number;
    live(): number;
    set(name: string, width: number, height: number, rgba: BundleBytes | null): void;
    remove(name: string): void;
    free(): void;
}

export interface NameplateVariantMeta {
    key: string;
    identity: string;
    colorway: number | null;
    owns_background: boolean;
}

export interface NameplateLayerMeta {
    slot: number;
    id: string;
    content: "body" | "sprite" | "emitter" | "material";
    depth: LayerDepth;
}

export interface NameplateReactionMeta {
    name: string;
    signal: SignalName | null;
    signal_index: number;
}

export interface NameplateOneShotMeta {
    name: string;
    trigger: "start" | "stop" | "appear" | "equip";
    signal: SignalName | null;
    signal_index?: number;
    restart: boolean;
}

export interface NameplateScheduleMeta {
    id: string;
    events: string[];
}

export interface NameplateParamMeta {
    id: string;
    value: number;
    min: number;
    max: number;
}

export interface NameplateMeta {
    id?: string;
    identity?: string;
    fps?: number;
    loop_frames?: number;
    phase?: "global" | "per_player";
    requires?: string[];
    variants?: NameplateVariantMeta[];
    layers?: NameplateLayerMeta[];
    animations?: {
        reactions: NameplateReactionMeta[];
        one_shots: NameplateOneShotMeta[];
        schedules: NameplateScheduleMeta[];
    };
    params?: NameplateParamMeta[];
    textures?: { width: number; height: number }[];
    status: LoadedKind;
    skip: unknown;
}

export interface RasterOneShotEntry {
    index?: number;
    name?: string;
    age: number;
}

export type RasterOneShot = readonly [number | string, number] | RasterOneShotEntry;

export interface RasterStoppedEntry {
    signal: SignalName | number;
    heldFor: number;
    ago: number;
}

export type RasterStopped = readonly [SignalName | number, number, number] | RasterStoppedEntry;

export interface RasterOptions {
    scale?: number;
    seed?: Wearer;
    state?: RasterStateValue;
    index?: number;
    hidden?: LayerSet;
    omit?: LayerSet;
    band?: RasterBandInput;
    reactions?: SignalSet | null;
    heldFor?: number | null;
    observedFor?: number | null;
    stopped?: readonly RasterStopped[];
    oneShots?: readonly RasterOneShot[];
    glyphs?: Float32Array | readonly number[] | readonly (readonly number[])[];
    detail?: number;
    motion?: number;
    textures?: RasterTextures | null;
}

export interface RasterLetter {
    dx: number;
    dy: number;
    color: number;
    shadow: number;
    opacity: number;
    scale: number;
    rotation: number;
    outline: number;
}

export interface RasterInfo {
    width: number;
    height: number;
    origin: [number, number];
    plate: [number, number];
    badge: { x: number; y: number; opacity: number } | null;
    textMode: TextModeName;
    letters: RasterLetter[];
}

export interface RasterImage extends RasterInfo {
    data: Uint8ClampedArray<ArrayBuffer>;
}

export interface RasterLayerImage {
    slot: number;
    id: string | null;
    depth: LayerDepth;
    band: TextBand;
    blend: LayerBlend;
    composite: CanvasComposite;
    data: Uint8ClampedArray<ArrayBuffer>;
    passes: Uint8ClampedArray<ArrayBuffer>[];
}

export interface RasterLayerSet extends RasterInfo {
    layers: RasterLayerImage[];
}

export interface FlattenLayer {
    blend: LayerBlend;
    data: Uint8ClampedArray | Uint8Array;
    passes?: readonly (Uint8ClampedArray | Uint8Array)[];
}

export interface TextureImage {
    width: number;
    height: number;
    rgba: Uint8Array;
}

export declare class NameplateAsset {
    constructor(engine: NameplateEngine, handle: number);
    readonly engine: NameplateEngine;
    handle: number;
    readonly status: number;
    readonly kind: LoadedKind;
    readonly issues: any[];
    readonly skip: unknown;
    readonly meta: NameplateMeta;
    get usable(): boolean;
    live(): number;
    shaders(dialect?: ShaderDialect): any;
    analyze(): any;
    dialectIssues(dialect?: ShaderDialect): any;
    variantIndex(variant: number | string): number;
    paramCurve(layer: string, param: string, variant?: number | string, count?: number): number[];
    oneShotIndex(shot: number | string): number;
    raster(row: TagRowInput | Int32Array, frame?: number, options?: RasterOptions): RasterImage;
    rasterLayers(row: TagRowInput | Int32Array, frame?: number, options?: RasterOptions): RasterLayerSet;
    textureCount(): number;
    texture(index: number): TextureImage;
    textures(): TextureImage[];
    free(): void;
}

export declare class Nameplates {
    constructor(engine: NameplateEngine);
    readonly engine: NameplateEngine;
    readonly wasm: Wasm;
    handle: number;
    live(): number;
    check(code: number): number;
    addAsset(asset: NameplateAsset): number;
    removeAsset(id: number): void;
    setMotion(motion: number): void;
    setVanillaTexture(name: string, width: number, height: number, rgba: BundleBytes | null): void;
    forgetWearer(wearer: Wearer): boolean;
    setSignalOverride(wearer: Wearer, signal: SignalName | number, value: number): void;
    setCustomColors(wearer: Wearer, asset: number, slots: ArrayLike<number>, rgb: ArrayLike<number>): void;
    clearCustomColors(wearer: Wearer, asset: number): boolean;
    fire(wearer: Wearer, asset: number, oneShot: number): boolean;
    stripLayout(): number;
    stripTexels(id: number): Uint32Array;
    wearerCount(): number;
    wearerBytes(): number;
    pendingFallbacks(): number;
    runFallbacks(limit?: number): number;
    frame(
        clock: number,
        wearers: BigUint64Array | BigInt64Array | ArrayLike<bigint | number>,
        tags: Int32Array | ArrayLike<number>,
        signals: Float32Array | ArrayLike<number>,
        glyphs?: Float32Array | ArrayLike<number>,
        colors?: Int32Array | ArrayLike<number>,
    ): Int32Array;
    free(): void;
}

export interface Section {
    offset: number;
    length: number;
    status: number;
}

export interface FrameEvent {
    code: number;
    target: number;
}

export declare function readRasterInfo(words: ArrayLike<number>): RasterInfo;
export declare function blendPixel(source: number, destination: number, blend?: LayerBlend): number;
export declare function flattenRasterLayers<T extends Uint8ClampedArray = Uint8ClampedArray<ArrayBuffer>>(layers: readonly FlattenLayer[], into?: T): T;
export declare function section(out: ArrayLike<number>, tag: number): Section;
export declare function events(out: ArrayLike<number>, tag: number): FrameEvent[];
