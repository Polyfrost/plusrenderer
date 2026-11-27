use crate::container::CAPABILITIES;
use crate::limits::{
    FLASHES_PER_SECOND, FLASH_AREA_PIXELS, FLASH_LUMINANCE_CHANGE, NAMEPLATE_APPEAR_SECONDS,
    NAMEPLATE_APPEAR_SECONDS_MIN, NAMEPLATE_BADGE_OFFSET, NAMEPLATE_BAND_WEIGHT,
    NAMEPLATE_BLEND_ROWS, NAMEPLATE_BREAKPOINTS, NAMEPLATE_BURST, NAMEPLATE_CACHED_STATES,
    NAMEPLATE_CAPS, NAMEPLATE_CORNER_RADIUS, NAMEPLATE_COST_CYCLE_TRACK, NAMEPLATE_COST_KEYS_TRACK,
    NAMEPLATE_COST_LETTER_FACTOR, NAMEPLATE_COST_MATERIAL_LAYER, NAMEPLATE_COST_MIRROR_FACTOR,
    NAMEPLATE_COST_PALETTE, NAMEPLATE_COST_PARTICLE, NAMEPLATE_COST_REPEAT_COPY,
    NAMEPLATE_COST_SPAWN_CHECK, NAMEPLATE_COST_WALKER, NAMEPLATE_CPU_UNITS,
    NAMEPLATE_DECORATION_MARGIN, NAMEPLATE_EMITTER_POOL, NAMEPLATE_FADE_STEPS,
    NAMEPLATE_FALLBACK_BYTES, NAMEPLATE_FLAGS, NAMEPLATE_FLAG_BANDS, NAMEPLATE_FLASH_FRAMES,
    NAMEPLATE_FLASH_SCHEDULE_REPEATS, NAMEPLATE_FPS_MAX, NAMEPLATE_FPS_MIN, NAMEPLATE_IMAGE_BYTES,
    NAMEPLATE_IMAGE_OVERHEAD_BYTES, NAMEPLATE_INTERPOLATE, NAMEPLATE_LAPS, NAMEPLATE_LAYERS,
    NAMEPLATE_LETTERS, NAMEPLATE_LETTER_OFFSET, NAMEPLATE_LETTER_SCALE_MAX_QUARTERS,
    NAMEPLATE_LETTER_SCALE_MIN_QUARTERS, NAMEPLATE_LOOP_FRAMES, NAMEPLATE_MATERIAL_COST,
    NAMEPLATE_MATERIAL_INPUTS, NAMEPLATE_MATERIAL_MARGIN, NAMEPLATE_MATERIAL_NODES,
    NAMEPLATE_MATERIAL_OCTAVES, NAMEPLATE_MATERIAL_OUTLINE_FACTOR, NAMEPLATE_MATERIAL_REPEAT,
    NAMEPLATE_MATERIAL_SAMPLES, NAMEPLATE_MIN_WIDTH, NAMEPLATE_OFFSET, NAMEPLATE_ONE_SHOTS,
    NAMEPLATE_OVERLAY_COLORS, NAMEPLATE_OVERLAY_WIDTH, NAMEPLATE_OWNED_GLYPHS, NAMEPLATE_PADDING,
    NAMEPLATE_PARAMS, NAMEPLATE_PARTICLE_GRAVITY, NAMEPLATE_PARTICLE_LIFETIME,
    NAMEPLATE_PARTICLE_SPEED, NAMEPLATE_PATH_POINTS, NAMEPLATE_PATH_POINTS_MIN,
    NAMEPLATE_PATH_SPEED, NAMEPLATE_PATH_STOPS, NAMEPLATE_PAUSE_FRAMES, NAMEPLATE_RECT_BYTES,
    NAMEPLATE_REPEAT_COPIES, NAMEPLATE_RINGS, NAMEPLATE_SCHEDULE_WEIGHT, NAMEPLATE_SLOTS,
    NAMEPLATE_SPAWN_COUNT, NAMEPLATE_SPRITE_COLORS, NAMEPLATE_SPRITE_FRAMES, NAMEPLATE_SPRITE_SIZE,
    NAMEPLATE_STRIP_BYTES, NAMEPLATE_TEXTURE_MAX_SIZE, NAMEPLATE_TONES, NAMEPLATE_WALKERS,
    NAMEPLATE_WOBBLE, QUADS_PER_NAMEPLATE,
};
use crate::signals::SIGNALS;

use super::abi::*;
use super::geom::{
    Anchor, BADGE_ADVANCE, BADGE_HEIGHT, BADGE_WIDTH, GLYPH_HEIGHT, LINE_PITCH, MAX_LINES,
};
use super::material::graph::{DEFAULT_OCTAVES, MAX_CELL_SIZE, MAX_DIRECTION, MAX_PERIOD};
use super::material::ir::{Op, OpKind, Reduce, Reg, Ty, LANE_COUNT, UNIFORM_ATLAS, UNIFORM_TONES};
use super::material::nodes::{cost as op_cost, sockets, Accepts, Socket, SocketDefault, TONE_COST};
use super::raster::{
    INFO_BADGE_OPACITY, INFO_BADGE_SHOWN, INFO_BADGE_X, INFO_BADGE_Y, INFO_HEIGHT, INFO_LETTERS,
    INFO_LETTER_COUNT, INFO_PLATE_DX, INFO_PLATE_DY, INFO_TEXT_MODE, INFO_WIDTH, INFO_WORDS,
    INFO_X, INFO_Y,
};
use super::types::{MAX_LAYERS, MAX_LETTERS, PALETTE_TONES};

pub const CONSTANTS: &[(&str, i64)] = &[
    ("tag_ints", TAG_INTS as i64),
    ("tag_asset", TAG_ASSET as i64),
    ("tag_variant", TAG_VARIANT as i64),
    ("tag_flags", TAG_FLAGS as i64),
    ("tag_detail", TAG_DETAIL as i64),
    ("tag_line_count", TAG_LINE_COUNT as i64),
    ("tag_name_line", TAG_NAME_LINE as i64),
    ("tag_align", TAG_ALIGN as i64),
    ("tag_name_x", TAG_NAME_X as i64),
    ("tag_name_width", TAG_NAME_WIDTH as i64),
    ("tag_prefix_width", TAG_PREFIX_WIDTH as i64),
    ("tag_line_width_0", TAG_LINE_WIDTH_0 as i64),
    ("tag_line_width_1", TAG_LINE_WIDTH_1 as i64),
    ("tag_line_width_2", TAG_LINE_WIDTH_2 as i64),
    ("tag_bg_x0", TAG_BG_X0 as i64),
    ("tag_bg_y0", TAG_BG_Y0 as i64),
    ("tag_bg_x1", TAG_BG_X1 as i64),
    ("tag_bg_y1", TAG_BG_Y1 as i64),
    ("tag_external_color", TAG_EXTERNAL_COLOR as i64),
    ("tag_sneak_alpha", TAG_SNEAK_ALPHA as i64),
    ("tag_glyph_offset", TAG_GLYPH_OFFSET as i64),
    ("tag_glyph_count", TAG_GLYPH_COUNT as i64),
    ("tag_color_offset", TAG_COLOR_OFFSET as i64),
    ("tag_color_count", TAG_COLOR_COUNT as i64),
    ("tag_reserved", TAG_RESERVED as i64),
    ("flag_display", FLAG_DISPLAY as i64),
    ("flag_own", FLAG_OWN as i64),
    ("flag_animated_names_off", FLAG_ANIMATED_NAMES_OFF as i64),
    ("flag_effects_cpu", FLAG_EFFECTS_CPU as i64),
    ("flag_effects_off", FLAG_EFFECTS_OFF as i64),
    ("flag_external_color", FLAG_EXTERNAL_COLOR as i64),
    ("flag_badge", FLAG_BADGE as i64),
    ("glyph_floats", GLYPH_FLOATS as i64),
    ("glyph_x", GLYPH_X as i64),
    ("glyph_width", GLYPH_WIDTH as i64),
    ("signal_floats", SIGNAL_FLOATS as i64),
    ("signal_events", SIGNAL_EVENTS as i64),
    ("out_tags", OUT_TAGS as i64),
    ("out_words", OUT_WORDS as i64),
    ("out_strip_layout", OUT_STRIP_LAYOUT as i64),
    ("out_header", OUT_HEADER as i64),
    ("dir_words", DIR_WORDS as i64),
    ("dir_offset", DIR_OFFSET as i64),
    ("dir_len", DIR_LEN as i64),
    ("dir_status", DIR_STATUS as i64),
    ("status_ok", STATUS_OK as i64),
    ("status_vanilla", STATUS_VANILLA as i64),
    ("status_unknown_asset", STATUS_UNKNOWN_ASSET as i64),
    ("status_bad_row", STATUS_BAD_ROW as i64),
    ("status_disabled", STATUS_DISABLED as i64),
    ("status_no_room", STATUS_NO_ROOM as i64),
    ("tag_head", TAG_HEAD as i64),
    ("head_plate_dx", HEAD_PLATE_DX as i64),
    ("head_plate_dy", HEAD_PLATE_DY as i64),
    ("head_owns_background", HEAD_OWNS_BACKGROUND as i64),
    ("head_text_mode", HEAD_TEXT_MODE as i64),
    ("head_ops", HEAD_OPS as i64),
    ("head_materials", HEAD_MATERIALS as i64),
    ("head_material_words", HEAD_MATERIAL_WORDS as i64),
    ("head_events", HEAD_EVENTS as i64),
    ("text_mode_vanilla", TEXT_MODE_VANILLA as i64),
    ("text_mode_motion", TEXT_MODE_MOTION as i64),
    ("text_mode_owned", TEXT_MODE_OWNED as i64),
    ("op_words", OP_WORDS as i64),
    ("op_x0", OP_X0 as i64),
    ("op_y0", OP_Y0 as i64),
    ("op_x1", OP_X1 as i64),
    ("op_y1", OP_Y1 as i64),
    ("op_argb", OP_ARGB as i64),
    ("op_band_layer_kind", OP_BAND_LAYER_KIND as i64),
    ("op_aux0", OP_AUX0 as i64),
    ("op_aux1", OP_AUX1 as i64),
    ("op_kind_rect", OP_KIND_RECT as i64),
    ("op_kind_strip", OP_KIND_STRIP as i64),
    ("op_kind_image", OP_KIND_IMAGE as i64),
    ("aux_low", AUX_LOW as i64),
    ("aux_axis_y", AUX_AXIS_Y as i64),
    ("aux_mirror", AUX_MIRROR as i64),
    ("aux_additive", AUX_ADDITIVE as i64),
    ("band_behind", BAND_BEHIND as i64),
    ("band_background", BAND_BACKGROUND as i64),
    ("band_over_background", BAND_OVER_BACKGROUND as i64),
    ("band_over_text", BAND_OVER_TEXT as i64),
    ("band_badge", BAND_BADGE as i64),
    ("mat_shader", MAT_SHADER as i64),
    ("mat_band_layer", MAT_BAND_LAYER as i64),
    ("mat_run", MAT_RUN as i64),
    ("mat_area_x0", MAT_AREA_X0 as i64),
    ("mat_area_y0", MAT_AREA_Y0 as i64),
    ("mat_area_x1", MAT_AREA_X1 as i64),
    ("mat_area_y1", MAT_AREA_Y1 as i64),
    ("mat_clip_count", MAT_CLIP_COUNT as i64),
    ("mat_uniform_words", MAT_UNIFORM_WORDS as i64),
    ("mat_fixed", MAT_FIXED as i64),
    ("clip_words", CLIP_WORDS as i64),
    ("mat_mirrored", MAT_MIRRORED as i64),
    ("mat_opacity_shift", MAT_OPACITY_SHIFT as i64),
    ("text_color", TEXT_COLOR as i64),
    ("text_shadow", TEXT_SHADOW as i64),
    ("text_shadow_use", TEXT_SHADOW_USE as i64),
    ("text_outline", TEXT_OUTLINE as i64),
    ("text_letter_count", TEXT_LETTER_COUNT as i64),
    ("text_fixed", TEXT_FIXED as i64),
    ("shadow_leave", SHADOW_LEAVE as i64),
    ("shadow_remove", SHADOW_REMOVE as i64),
    ("shadow_always", SHADOW_ALWAYS as i64),
    ("shadow_with_drop", SHADOW_WITH_DROP as i64),
    ("letter_words", LETTER_WORDS as i64),
    ("letter_dx", LETTER_DX as i64),
    ("letter_dy", LETTER_DY as i64),
    ("letter_argb", LETTER_ARGB as i64),
    ("letter_shadow", LETTER_SHADOW as i64),
    ("letter_opacity", LETTER_OPACITY as i64),
    ("letter_scale_rot", LETTER_SCALE_ROT as i64),
    ("letter_outline", LETTER_OUTLINE as i64),
    ("badge_words", BADGE_WORDS as i64),
    ("badge_x", BADGE_X as i64),
    ("badge_y", BADGE_Y as i64),
    ("badge_opacity", BADGE_OPACITY as i64),
    ("badge_tint", BADGE_TINT as i64),
    ("badge_decoration", BADGE_DECORATION as i64),
    ("decoration_none", DECORATION_NONE as i64),
    ("decoration_shadow", DECORATION_SHADOW as i64),
    ("decoration_outline", DECORATION_OUTLINE as i64),
    ("decoration_kind_shift", DECORATION_KIND_SHIFT as i64),
    ("event_words", EVENT_WORDS as i64),
    ("event_code", EVENT_CODE as i64),
    ("event_layer", EVENT_LAYER as i64),
    ("event_layer_disabled", EVENT_LAYER_DISABLED as i64),
    ("event_molang_over_budget", EVENT_MOLANG_OVER_BUDGET as i64),
    ("event_asset_disabled", EVENT_ASSET_DISABLED as i64),
    ("event_target_tag", EVENT_TARGET_TAG as i64),
    ("event_target_palette", EVENT_TARGET_PALETTE as i64),
    ("event_target_letters", EVENT_TARGET_LETTERS as i64),
    ("event_target_badge", EVENT_TARGET_BADGE as i64),
    ("event_target_plate", EVENT_TARGET_PLATE as i64),
    ("max_layers", MAX_LAYERS as i64),
    ("max_letters", MAX_LETTERS as i64),
    ("max_lines", MAX_LINES as i64),
    ("palette_tones", PALETTE_TONES as i64),
    ("glyph_height", GLYPH_HEIGHT as i64),
    ("line_pitch", LINE_PITCH as i64),
    ("badge_width", BADGE_WIDTH as i64),
    ("badge_height", BADGE_HEIGHT as i64),
    ("badge_advance", BADGE_ADVANCE as i64),
    ("uniform_lanes", LANE_COUNT as i64),
    ("uniform_atlas", UNIFORM_ATLAS as i64),
    ("uniform_tones", UNIFORM_TONES as i64),
    ("raster_info_words", INFO_WORDS as i64),
    ("raster_info_width", INFO_WIDTH as i64),
    ("raster_info_height", INFO_HEIGHT as i64),
    ("raster_info_x", INFO_X as i64),
    ("raster_info_y", INFO_Y as i64),
    ("raster_info_plate_dx", INFO_PLATE_DX as i64),
    ("raster_info_plate_dy", INFO_PLATE_DY as i64),
    ("raster_info_badge_shown", INFO_BADGE_SHOWN as i64),
    ("raster_info_badge_x", INFO_BADGE_X as i64),
    ("raster_info_badge_y", INFO_BADGE_Y as i64),
    ("raster_info_badge_opacity", INFO_BADGE_OPACITY as i64),
    ("raster_info_text_mode", INFO_TEXT_MODE as i64),
    ("raster_info_letter_count", INFO_LETTER_COUNT as i64),
    ("raster_info_letters", INFO_LETTERS as i64),
];

pub fn constants() -> &'static [(&'static str, i64)] {
    CONSTANTS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Int,
    Pair,
    Choice,
    Literal,
    Slot,
    Param,
    Molang,
    Node,
    Sprite,
    Texture,
    Atlas,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub name: &'static str,
    pub kind: SettingKind,
    pub required: bool,
    pub min: i64,
    pub max: i64,
    pub default: [i64; 2],
    pub choices: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostBy {
    Flat,
    Width,
    Inputs,
    Octaves,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeInfo {
    pub name: &'static str,
    pub kind: Option<OpKind>,
    pub sockets: &'static [Socket],
    pub settings: &'static [Setting],
    pub cost_by: CostBy,
    pub costs: Vec<u32>,
}

pub const LIMITS: &[(&str, f64)] = &[
    ("download_bytes", NAMEPLATE_CAPS.download as f64),
    ("unpacked_bytes", NAMEPLATE_CAPS.unpacked as f64),
    ("entries", NAMEPLATE_CAPS.entries as f64),
    ("file_bytes", file_bytes()),
    ("texture_max_size", NAMEPLATE_TEXTURE_MAX_SIZE as f64),
    ("quads", QUADS_PER_NAMEPLATE as f64),
    ("flashes_per_second", FLASHES_PER_SECOND as f64),
    ("flash_area_pixels", FLASH_AREA_PIXELS as f64),
    ("flash_luminance_change", FLASH_LUMINANCE_CHANGE),
    ("material_tone_cost", TONE_COST as f64),
    ("fps_min", NAMEPLATE_FPS_MIN as f64),
    ("fps_max", NAMEPLATE_FPS_MAX as f64),
    ("loop_frames", NAMEPLATE_LOOP_FRAMES as f64),
    ("layers", NAMEPLATE_LAYERS as f64),
    ("slots", NAMEPLATE_SLOTS as f64),
    ("tones", NAMEPLATE_TONES as f64),
    ("params", NAMEPLATE_PARAMS as f64),
    ("offset", NAMEPLATE_OFFSET as f64),
    ("badge_offset", NAMEPLATE_BADGE_OFFSET as f64),
    ("padding", NAMEPLATE_PADDING as f64),
    ("min_width", NAMEPLATE_MIN_WIDTH as f64),
    ("corner_radius", NAMEPLATE_CORNER_RADIUS as f64),
    ("rings", NAMEPLATE_RINGS as f64),
    ("sprite_size", NAMEPLATE_SPRITE_SIZE as f64),
    ("sprite_frames", NAMEPLATE_SPRITE_FRAMES as f64),
    ("sprite_colors", NAMEPLATE_SPRITE_COLORS as f64),
    ("cached_states", NAMEPLATE_CACHED_STATES as f64),
    ("path_points_min", NAMEPLATE_PATH_POINTS_MIN as f64),
    ("path_points", NAMEPLATE_PATH_POINTS as f64),
    ("path_speed", NAMEPLATE_PATH_SPEED as f64),
    ("walkers", NAMEPLATE_WALKERS as f64),
    ("laps", NAMEPLATE_LAPS as f64),
    ("path_stops", NAMEPLATE_PATH_STOPS as f64),
    ("pause_frames", NAMEPLATE_PAUSE_FRAMES as f64),
    ("repeat_copies", NAMEPLATE_REPEAT_COPIES as f64),
    ("breakpoints", NAMEPLATE_BREAKPOINTS as f64),
    ("emitter_pool", NAMEPLATE_EMITTER_POOL as f64),
    ("spawn_count", NAMEPLATE_SPAWN_COUNT as f64),
    ("burst", NAMEPLATE_BURST as f64),
    ("particle_lifetime", NAMEPLATE_PARTICLE_LIFETIME as f64),
    ("particle_speed", NAMEPLATE_PARTICLE_SPEED as f64),
    ("particle_gravity", NAMEPLATE_PARTICLE_GRAVITY as f64),
    ("wobble", NAMEPLATE_WOBBLE as f64),
    ("fade_steps", NAMEPLATE_FADE_STEPS as f64),
    ("flags", NAMEPLATE_FLAGS as f64),
    ("flag_bands", NAMEPLATE_FLAG_BANDS as f64),
    ("band_weight", NAMEPLATE_BAND_WEIGHT as f64),
    ("overlay_colors", NAMEPLATE_OVERLAY_COLORS as f64),
    ("overlay_width", NAMEPLATE_OVERLAY_WIDTH as f64),
    ("image_bytes", NAMEPLATE_IMAGE_BYTES as f64),
    (
        "image_overhead_bytes",
        NAMEPLATE_IMAGE_OVERHEAD_BYTES as f64,
    ),
    ("rect_bytes", NAMEPLATE_RECT_BYTES as f64),
    ("strip_bytes", NAMEPLATE_STRIP_BYTES as f64),
    ("cpu_units", NAMEPLATE_CPU_UNITS as f64),
    ("cost_walker", NAMEPLATE_COST_WALKER as f64),
    ("cost_repeat_copy", NAMEPLATE_COST_REPEAT_COPY as f64),
    ("cost_particle", NAMEPLATE_COST_PARTICLE as f64),
    ("cost_spawn_check", NAMEPLATE_COST_SPAWN_CHECK as f64),
    ("cost_palette", NAMEPLATE_COST_PALETTE as f64),
    ("cost_cycle_track", NAMEPLATE_COST_CYCLE_TRACK as f64),
    ("cost_keys_track", NAMEPLATE_COST_KEYS_TRACK as f64),
    ("cost_material_layer", NAMEPLATE_COST_MATERIAL_LAYER as f64),
    ("cost_mirror_factor", NAMEPLATE_COST_MIRROR_FACTOR as f64),
    ("cost_letter_factor", NAMEPLATE_COST_LETTER_FACTOR as f64),
    ("decoration_margin", NAMEPLATE_DECORATION_MARGIN as f64),
    ("material_nodes", NAMEPLATE_MATERIAL_NODES as f64),
    ("material_samples", NAMEPLATE_MATERIAL_SAMPLES as f64),
    ("material_inputs", NAMEPLATE_MATERIAL_INPUTS as f64),
    ("material_octaves", NAMEPLATE_MATERIAL_OCTAVES as f64),
    ("material_cost", NAMEPLATE_MATERIAL_COST as f64),
    (
        "material_outline_factor",
        NAMEPLATE_MATERIAL_OUTLINE_FACTOR as f64,
    ),
    ("material_margin", NAMEPLATE_MATERIAL_MARGIN as f64),
    ("material_repeat", NAMEPLATE_MATERIAL_REPEAT as f64),
    ("fallback_bytes", NAMEPLATE_FALLBACK_BYTES as f64),
    ("owned_glyphs", NAMEPLATE_OWNED_GLYPHS as f64),
    ("letters", NAMEPLATE_LETTERS as f64),
    ("letter_offset", NAMEPLATE_LETTER_OFFSET as f64),
    (
        "letter_scale_min_quarters",
        NAMEPLATE_LETTER_SCALE_MIN_QUARTERS as f64,
    ),
    (
        "letter_scale_max_quarters",
        NAMEPLATE_LETTER_SCALE_MAX_QUARTERS as f64,
    ),
    ("one_shots", NAMEPLATE_ONE_SHOTS as f64),
    ("appear_seconds_min", NAMEPLATE_APPEAR_SECONDS_MIN as f64),
    ("appear_seconds", NAMEPLATE_APPEAR_SECONDS as f64),
    ("blend_rows", NAMEPLATE_BLEND_ROWS as f64),
    ("interpolate", NAMEPLATE_INTERPOLATE as f64),
    ("schedule_weight", NAMEPLATE_SCHEDULE_WEIGHT as f64),
    ("flash_frames", NAMEPLATE_FLASH_FRAMES as f64),
    (
        "flash_schedule_repeats",
        NAMEPLATE_FLASH_SCHEDULE_REPEATS as f64,
    ),
];

const fn file_bytes() -> f64 {
    match NAMEPLATE_CAPS.per_file {
        Some(bytes) => bytes as f64,
        None => 0.0,
    }
}

const fn int(name: &'static str, min: i64, max: i64, default: i64, required: bool) -> Setting {
    Setting {
        name,
        kind: SettingKind::Int,
        required,
        min,
        max,
        default: [default, 0],
        choices: &[],
    }
}

const fn pair(name: &'static str, min: i64, max: i64, default: [i64; 2]) -> Setting {
    Setting {
        name,
        kind: SettingKind::Pair,
        required: false,
        min,
        max,
        default,
        choices: &[],
    }
}

const fn choice(
    name: &'static str,
    choices: &'static [&'static str],
    default: i64,
    required: bool,
) -> Setting {
    Setting {
        name,
        kind: SettingKind::Choice,
        required,
        min: 0,
        max: choices.len() as i64 - 1,
        default: [default, 0],
        choices,
    }
}

const fn named(name: &'static str, kind: SettingKind) -> Setting {
    Setting {
        name,
        kind,
        required: true,
        min: 0,
        max: 0,
        default: [0, 0],
        choices: &[],
    }
}

const SEED: Setting = int("seed", 0, i32::MAX as i64, 0, false);
const NOISE_PERIOD: Setting = pair("period", 0, MAX_PERIOD, [0, 0]);
const VANILLA: [&str; 6] = [
    "end_sky",
    "end_portal",
    "glint",
    "beacon_beam",
    "creeper_armor",
    "breeze_wind",
];
const REDUCE: [&str; 4] = ["sum", "min", "max", "first"];

static NO_SETTINGS: [Setting; 0] = [];
static SLOT_SETTINGS: [Setting; 1] = [named("slot", SettingKind::Slot)];
static CONSTANT_SETTINGS: [Setting; 1] = [named("value", SettingKind::Literal)];
static PARAM_SETTINGS: [Setting; 1] = [named("name", SettingKind::Param)];
static EXPR_SETTINGS: [Setting; 1] = [named("expr", SettingKind::Molang)];
static EDGE_SETTINGS: [Setting; 1] = [choice("edge", &["fill", "outer"], 0, false)];
static NOISE_SETTINGS: [Setting; 2] = [SEED, NOISE_PERIOD];
static FBM_SETTINGS: [Setting; 3] = [
    SEED,
    NOISE_PERIOD,
    int(
        "octaves",
        1,
        NAMEPLATE_MATERIAL_OCTAVES as i64,
        DEFAULT_OCTAVES,
        false,
    ),
];
static HASH_SETTINGS: [Setting; 1] = [SEED];
static CHECKER_SETTINGS: [Setting; 1] = [int("size", 1, MAX_CELL_SIZE, 1, false)];
static STRIPES_SETTINGS: [Setting; 3] = [
    int("period", 1, MAX_CELL_SIZE, 2, false),
    int("width", 1, MAX_CELL_SIZE, 1, false),
    pair("direction", -MAX_DIRECTION, MAX_DIRECTION, [1, 0]),
];
static COMPONENT_SETTINGS: [Setting; 1] = [choice("component", &["x", "y", "z", "w"], 0, false)];
static SAMPLE_AT_SETTINGS: [Setting; 1] = [named("source", SettingKind::Node)];
static REPEAT_SETTINGS: [Setting; 3] = [
    named("source", SettingKind::Node),
    int("count", 1, NAMEPLATE_MATERIAL_REPEAT as i64, 1, true),
    choice("reduce", &REDUCE, Reduce::Max as i64, false),
];
static SPRITE_SETTINGS: [Setting; 1] = [named("sprite", SettingKind::Sprite)];
static BUNDLE_TEXTURE_SETTINGS: [Setting; 1] = [named("texture", SettingKind::Texture)];
static VANILLA_TEXTURE_SETTINGS: [Setting; 1] = [choice("texture", &VANILLA, 0, true)];
static ATLAS_SETTINGS: [Setting; 1] = [named("sprite", SettingKind::Atlas)];
static OUTPUT_SETTINGS: [Setting; 1] = [choice("blend", &["alpha", "additive"], 0, false)];
static OUTPUT_SOCKETS: [Socket; 2] = [
    Socket {
        name: "color",
        accepts: Accepts::Surface,
        default: SocketDefault::Required,
    },
    Socket {
        name: "coverage",
        accepts: Accepts::Float,
        default: SocketDefault::Value([1.0, 0.0, 0.0, 0.0], 1),
    },
];

pub fn settings(kind: OpKind) -> &'static [Setting] {
    match kind {
        OpKind::Slot | OpKind::Quantize | OpKind::DitherQuantize | OpKind::Cycle => &SLOT_SETTINGS,
        OpKind::Constant => &CONSTANT_SETTINGS,
        OpKind::Param => &PARAM_SETTINGS,
        OpKind::Expr => &EXPR_SETTINGS,
        OpKind::BodyEdge => &EDGE_SETTINGS,
        OpKind::Noise => &NOISE_SETTINGS,
        OpKind::Fbm => &FBM_SETTINGS,
        OpKind::Hash => &HASH_SETTINGS,
        OpKind::Checker => &CHECKER_SETTINGS,
        OpKind::Stripes => &STRIPES_SETTINGS,
        OpKind::Component => &COMPONENT_SETTINGS,
        OpKind::SampleAt => &SAMPLE_AT_SETTINGS,
        OpKind::Repeat => &REPEAT_SETTINGS,
        OpKind::Sprite => &SPRITE_SETTINGS,
        OpKind::BundleTexture => &BUNDLE_TEXTURE_SETTINGS,
        OpKind::VanillaTexture => &VANILLA_TEXTURE_SETTINGS,
        OpKind::AtlasSprite => &ATLAS_SETTINGS,
        _ => &NO_SETTINGS,
    }
}

fn probe(kind: OpKind, ty: Ty, imm: [f64; 4]) -> u32 {
    op_cost(&Op {
        kind,
        ty,
        dst: Reg::NONE,
        args: [Reg::NONE; 4],
        imm,
    })
}

fn costs(kind: OpKind) -> (CostBy, Vec<u32>) {
    if kind == OpKind::Slot {
        return (CostBy::Flat, vec![probe(kind, Ty::Slot, [0.0; 4])]);
    }
    let widths: Vec<u32> = (1..=4u8)
        .filter_map(Ty::vector)
        .map(|ty| probe(kind, ty, [0.0; 4]))
        .collect();
    if widths.iter().any(|c| *c != widths[0]) {
        return (CostBy::Width, widths);
    }
    let inputs: Vec<u32> = (1..=4)
        .map(|n| probe(kind, Ty::Float, [f64::from(n), 0.0, 0.0, 0.0]))
        .collect();
    if inputs.iter().any(|c| *c != inputs[0]) {
        return (CostBy::Inputs, inputs);
    }
    let octaves: Vec<u32> = (1..=NAMEPLATE_MATERIAL_OCTAVES)
        .map(|n| probe(kind, Ty::Float, [0.0, 0.0, 0.0, f64::from(n)]))
        .collect();
    if octaves.iter().any(|c| *c != octaves[0]) {
        return (CostBy::Octaves, octaves);
    }
    (CostBy::Flat, vec![widths[0]])
}

pub fn nodes() -> Vec<NodeInfo> {
    let mut out: Vec<NodeInfo> = OpKind::ALL
        .into_iter()
        .map(|kind| {
            let (cost_by, costs) = costs(kind);
            NodeInfo {
                name: kind.name(),
                kind: Some(kind),
                sockets: sockets(kind),
                settings: settings(kind),
                cost_by,
                costs,
            }
        })
        .collect();
    out.push(NodeInfo {
        name: "Output",
        kind: None,
        sockets: &OUTPUT_SOCKETS,
        settings: &OUTPUT_SETTINGS,
        cost_by: CostBy::Flat,
        costs: vec![0],
    });
    out
}

pub fn limits() -> &'static [(&'static str, f64)] {
    LIMITS
}

pub fn anchors() -> Vec<&'static str> {
    Anchor::ALL.iter().map(|a| a.as_str()).collect()
}

pub fn signals() -> Vec<&'static str> {
    SIGNALS.iter().map(|(name, _)| *name).collect()
}

pub fn capabilities() -> &'static [&'static str] {
    CAPABILITIES
}

impl SettingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SettingKind::Int => "int",
            SettingKind::Pair => "pair",
            SettingKind::Choice => "choice",
            SettingKind::Literal => "literal",
            SettingKind::Slot => "slot",
            SettingKind::Param => "param",
            SettingKind::Molang => "molang",
            SettingKind::Node => "node",
            SettingKind::Sprite => "sprite",
            SettingKind::Texture => "texture",
            SettingKind::Atlas => "atlas",
        }
    }
}

pub fn accepts_name(accepts: Accepts) -> &'static str {
    match accepts {
        Accepts::Number => "number",
        Accepts::Float => "float",
        Accepts::Vec2 => "vec2",
        Accepts::Color => "color",
        Accepts::Surface => "surface",
        Accepts::Any => "any",
    }
}

impl CostBy {
    pub fn as_str(self) -> &'static str {
        match self {
            CostBy::Flat => "flat",
            CostBy::Width => "width",
            CostBy::Inputs => "inputs",
            CostBy::Octaves => "octaves",
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};

    use super::*;
    use crate::issues::{IssueCode, Mode, Report};
    use crate::nameplate::material::graph::build_named;
    use crate::nameplate::material::graph::test_support::{context, Fixture};

    fn unique<T: PartialEq + std::fmt::Debug>(names: &[T]) {
        for (i, name) in names.iter().enumerate() {
            assert!(!names[i + 1..].contains(name), "{name:?} listed twice");
        }
    }

    #[test]
    fn host_constants_and_limits_are_exported_once_with_their_values() {
        let names: Vec<&str> = constants().iter().map(|(n, _)| *n).collect();
        unique(&names);
        let wire = "tag_ints=24 signal_floats=24 op_words=8 letter_words=7 event_target_tag=-1 raster_info_words=180";
        for (name, value) in wire.split(' ').filter_map(|pair| pair.split_once('=')) {
            assert_eq!(
                constants()
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v.to_string())
                    .as_deref(),
                Some(value)
            );
        }
        let names: Vec<&str> = limits().iter().map(|(n, _)| *n).collect();
        unique(&names);
        let limit = |name: &str| limits().iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
        let source = include_str!("../limits.rs");
        let declared: Vec<&str> = source
            .lines()
            .filter_map(|line| {
                line.strip_prefix("pub const NAMEPLATE_")?
                    .split_once(": u32")
            })
            .map(|(n, _)| n)
            .collect();
        assert!(
            declared.len() > 60
                && declared
                    .iter()
                    .all(|name| limit(&name.to_ascii_lowercase()).is_some())
        );
        let values = "fps_max=24 cpu_units=4000 quads=800 download_bytes=2097152 flashes_per_second=3 flash_area_pixels=64 flash_luminance_change=0.1";
        for (name, value) in values.split(' ').filter_map(|pair| pair.split_once('=')) {
            assert_eq!(limit(name), value.parse().ok(), "{name}");
        }
    }

    #[test]
    fn the_node_catalogue_matches_the_graph_builder() {
        let list = nodes();
        assert_eq!(list.len(), OpKind::ALL.len() + 1);
        let node = |name: &str| list.iter().find(|n| n.name == name).unwrap();
        for kind in OpKind::ALL {
            let actual = &(node(kind.name()).kind, node(kind.name()).sockets);
            assert_eq!(*actual, (Some(kind), sockets(kind)));
        }
        let output: Vec<&str> = node("Output").sockets.iter().map(|s| s.name).collect();
        assert_eq!(output, ["color", "coverage"]);
        let costs = |name: &str| (node(name).cost_by, node(name).costs.clone());
        assert_eq!(costs("Noise"), (CostBy::Flat, vec![52]));
        assert_eq!(costs("length"), (CostBy::Inputs, vec![7, 9, 11, 13]));
        assert_eq!(costs("Fbm").1[7], 52 * 8);

        let valid = |setting: &Setting| match setting.kind {
            SettingKind::Int => json!(setting.default[0]),
            SettingKind::Pair => json!(setting.default),
            SettingKind::Choice => json!(setting.choices[setting.default[0] as usize]),
            SettingKind::Literal => json!(1),
            SettingKind::Slot => json!("fill"),
            SettingKind::Param => json!("speed"),
            SettingKind::Molang => json!("1"),
            SettingKind::Node => json!("x"),
            SettingKind::Sprite => json!("star"),
            SettingKind::Texture => json!("dots"),
            SettingKind::Atlas => json!("minecraft:block/stone"),
        };
        let rejected = |kind: &str, set: Value| {
            let mut fixture = Fixture::new();
            let mut report = Report::new(Mode::Authoring);
            let file = json!({ "nodes": [{ "id": "n", "type": kind, "set": set }] });
            let _ = build_named("m", &file, &mut context(&mut fixture), &mut report);
            report
                .issues()
                .iter()
                .filter(|i| i.code == IssueCode::NodeSetting)
                .map(|i| i.path.clone())
                .collect::<Vec<_>>()
        };
        let mut every: Vec<&str> = list
            .iter()
            .flat_map(|n| n.settings.iter().map(|s| s.name))
            .collect();
        every.sort_unstable();
        every.dedup();
        assert!(every.len() > 10);
        for info in &list {
            let declared: Map<String, Value> = info
                .settings
                .iter()
                .map(|s| (s.name.to_string(), valid(s)))
                .collect();
            let holds = rejected(info.name, Value::Object(declared)).is_empty();
            assert!(holds, "{}", info.name);
            for name in every
                .iter()
                .filter(|n| !info.settings.iter().any(|s| s.name == **n))
            {
                assert!(
                    rejected(info.name, json!({ *name: 1 }))
                        .iter()
                        .any(|p| p.ends_with(name)),
                    "{} reads {name} undeclared",
                    info.name
                );
            }
        }
    }
}
