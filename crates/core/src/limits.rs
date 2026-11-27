use serde::Serialize;

const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Caps {
    pub download: u64,
    pub unpacked: u64,
    pub entries: u32,
    pub per_file: Option<u64>,
}

pub const CAPE_CAPS: Caps = Caps {
    download: 16 * MIB,
    unpacked: 32 * MIB,
    entries: 16,
    per_file: None,
};

pub const ITEM_CAPS: Caps = Caps {
    download: 8 * MIB,
    unpacked: 16 * MIB,
    entries: 64,
    per_file: None,
};

pub const PET_CAPS: Caps = ITEM_CAPS;

pub const EMOTE_CAPS: Caps = Caps {
    download: 8 * MIB,
    unpacked: 16 * MIB,
    entries: 128,
    per_file: None,
};

pub const EFFECT_CAPS: Caps = Caps {
    download: 4 * MIB,
    unpacked: 8 * MIB,
    entries: 128,
    per_file: None,
};

pub const NAMEPLATE_CAPS: Caps = Caps {
    download: 2 * MIB,
    unpacked: 8 * MIB,
    entries: 128,
    per_file: Some(MIB),
};

pub const JSON_MAX_BYTES: u64 = 256 * 1024;
pub const JSON_MAX_DEPTH: u32 = 32;

pub const ID_MIN_LEN: usize = 1;
pub const ID_MAX_LEN: usize = 64;

pub const TEXTURE_MAX_SIZE: u32 = 1024;
pub const NAMEPLATE_TEXTURE_MAX_SIZE: u32 = 256;

pub const QUADS_PER_WEARER: u32 = 2000;
pub const QUADS_PER_NAMEPLATE: u32 = 800;

pub const BONES_PER_ITEM: u32 = 64;
pub const BONES_PER_PET: u32 = 128;

pub const SPRING_BONES_PER_ITEM: u32 = 16;
pub const SPRING_CHAIN_DEPTH: u32 = 8;

pub const PARTICLES_PER_EMITTER: u32 = 64;
pub const PARTICLES_PER_COSMETIC: u32 = 128;
pub const TRAIL_POINTS: u32 = 64;

pub const MOLANG_AST_NODES: u32 = 256;
pub const MOLANG_DEPTH: u32 = 64;
pub const MOLANG_SOURCE_CHARS: u32 = 4096;
pub const MOLANG_VARIABLES: u32 = 64;
pub const MOLANG_CALL_ARGUMENTS: u32 = 4;
pub const MOLANG_STEPS_PER_EVALUATION: u32 = 2000;

pub const CPU_UNITS_PER_WEARER: u32 = 4000;

pub const WEARER_RECORD_BYTES: u32 = 2560;

pub const FLASHES_PER_SECOND: u32 = 3;
pub const FLASH_AREA_PIXELS: u32 = 64;
pub const FLASH_LUMINANCE_CHANGE: f64 = 0.1;

pub const DETAIL_DISTANCE_BLOCKS: f64 = 24.0;
pub const DETAIL_NEAREST_WEARERS: u32 = 32;
pub const MINIMAL_DETAIL_BLOCKS: f64 = 64.0;
pub const MINIMAL_ANIMATION_HZ: u32 = 10;
pub const REDUCED_MOTION_SPRING_SCALE: f32 = 0.5;

pub const NAMEPLATE_FPS_MIN: u32 = 1;
pub const NAMEPLATE_FPS_MAX: u32 = 24;
pub const NAMEPLATE_LOOP_FRAMES: u32 = 600;
pub const NAMEPLATE_LAYERS: u32 = 32;
pub const NAMEPLATE_SLOTS: u32 = 32;
pub const NAMEPLATE_TONES: u32 = 16;
pub const NAMEPLATE_PARAMS: u32 = 64;
pub const NAMEPLATE_OFFSET: u32 = 64;
pub const NAMEPLATE_BADGE_OFFSET: u32 = 32;
pub const NAMEPLATE_PADDING: u32 = 32;
pub const NAMEPLATE_MIN_WIDTH: u32 = 256;
pub const NAMEPLATE_CORNER_RADIUS: u32 = 16;
pub const NAMEPLATE_RINGS: u32 = 4;
pub const NAMEPLATE_SPRITE_SIZE: u32 = 64;
pub const NAMEPLATE_SPRITE_FRAMES: u32 = 120;
pub const NAMEPLATE_SPRITE_COLORS: u32 = 32;
pub const NAMEPLATE_CACHED_STATES: u32 = 120;
pub const NAMEPLATE_PATH_POINTS_MIN: u32 = 2;
pub const NAMEPLATE_PATH_POINTS: u32 = 32;
pub const NAMEPLATE_PATH_SPEED: u32 = 16;
pub const NAMEPLATE_WALKERS: u32 = 32;
pub const NAMEPLATE_LAPS: u32 = 64;
pub const NAMEPLATE_PATH_STOPS: u32 = 32;
pub const NAMEPLATE_PAUSE_FRAMES: u32 = 600;
pub const NAMEPLATE_REPEAT_COPIES: u32 = 64;
pub const NAMEPLATE_BREAKPOINTS: u32 = 8;
pub const NAMEPLATE_EMITTER_POOL: u32 = 8;
pub const NAMEPLATE_SPAWN_COUNT: u32 = 32;
pub const NAMEPLATE_BURST: u32 = 64;
pub const NAMEPLATE_PARTICLE_LIFETIME: u32 = 240;
pub const NAMEPLATE_PARTICLE_SPEED: u32 = 16;
pub const NAMEPLATE_PARTICLE_GRAVITY: u32 = 4;
pub const NAMEPLATE_WOBBLE: u32 = 8;
pub const NAMEPLATE_FADE_STEPS: u32 = 16;
pub const NAMEPLATE_FLAGS: u32 = 16;
pub const NAMEPLATE_FLAG_BANDS: u32 = 12;
pub const NAMEPLATE_BAND_WEIGHT: u32 = 1000;
pub const NAMEPLATE_OVERLAY_COLORS: u32 = 8;
pub const NAMEPLATE_OVERLAY_WIDTH: u32 = 8;
pub const NAMEPLATE_IMAGE_BYTES: u32 = 256 * 1024;
pub const NAMEPLATE_IMAGE_OVERHEAD_BYTES: u32 = 64;
pub const NAMEPLATE_RECT_BYTES: u32 = 20;
pub const NAMEPLATE_STRIP_BYTES: u32 = 24;
pub const NAMEPLATE_CPU_UNITS: u32 = CPU_UNITS_PER_WEARER;
pub const NAMEPLATE_COST_WALKER: u32 = 20;
pub const NAMEPLATE_COST_REPEAT_COPY: u32 = 5;
pub const NAMEPLATE_COST_PARTICLE: u32 = 18;
pub const NAMEPLATE_COST_SPAWN_CHECK: u32 = 2;
pub const NAMEPLATE_COST_PALETTE: u32 = 10;
pub const NAMEPLATE_COST_CYCLE_TRACK: u32 = 25;
pub const NAMEPLATE_COST_KEYS_TRACK: u32 = 3;
pub const NAMEPLATE_COST_MATERIAL_LAYER: u32 = 18;
pub const NAMEPLATE_COST_MIRROR_FACTOR: u32 = 2;
pub const NAMEPLATE_COST_LETTER_FACTOR: u32 = 24;
pub const NAMEPLATE_DECORATION_MARGIN: u32 = 32;
pub const NAMEPLATE_MATERIAL_NODES: u32 = 64;
pub const NAMEPLATE_MATERIAL_SAMPLES: u32 = 4;
pub const NAMEPLATE_MATERIAL_INPUTS: u32 = 16;
pub const NAMEPLATE_MATERIAL_OCTAVES: u32 = 8;
pub const NAMEPLATE_MATERIAL_COST: u32 = 2000;
pub const NAMEPLATE_MATERIAL_OUTLINE_FACTOR: u32 = 5;
pub const NAMEPLATE_MATERIAL_MARGIN: u32 = 32;
pub const NAMEPLATE_MATERIAL_REPEAT: u32 = 16;
pub const NAMEPLATE_FALLBACK_BYTES: u32 = 2 * 1024 * 1024;
pub const NAMEPLATE_OWNED_GLYPHS: u32 = 24;
pub const NAMEPLATE_LETTERS: u32 = 24;
pub const NAMEPLATE_LETTER_OFFSET: u32 = 32;
pub const NAMEPLATE_LETTER_SCALE_MIN_QUARTERS: u32 = 1;
pub const NAMEPLATE_LETTER_SCALE_MAX_QUARTERS: u32 = 12;
pub const NAMEPLATE_ONE_SHOTS: u32 = 8;
pub const NAMEPLATE_APPEAR_SECONDS_MIN: u32 = 1;
pub const NAMEPLATE_APPEAR_SECONDS: u32 = 30;
pub const NAMEPLATE_BLEND_ROWS: u32 = 1024;
pub const NAMEPLATE_INTERPOLATE: u32 = 16;
pub const NAMEPLATE_SCHEDULE_WEIGHT: u32 = 1000;
pub const NAMEPLATE_FLASH_FRAMES: u32 = 4800;
pub const NAMEPLATE_FLASH_SCHEDULE_REPEATS: u32 = 4;
pub const NAMEPLATE_FLASH_WORK: u64 = 2_500_000_000;

pub const SPRING_STEP_HZ: u32 = 60;
pub const SPRING_MAX_STEPS: u32 = 4;
pub const SPRING_TELEPORT_BLOCKS: f32 = 4.0;
pub const SPRING_WATER_DRAG: f32 = 3.0;
pub const SPRING_WATER_GRAVITY: f32 = 0.2;
pub const SPRING_LIMIT_DEGREES: f32 = 180.0;
pub const EFFECT_RING_POINTS: u32 = 64;
pub const EFFECT_RING_HZ: u32 = 20;
pub const EFFECT_PARTICLE_LIFE_SECONDS: f32 =
    (EFFECT_RING_POINTS - 2) as f32 / EFFECT_RING_HZ as f32;
pub const EFFECT_FOOTSTEP_BLOCKS: f32 = 0.6;
pub const EVENT_EFFECT_SECONDS: f32 = 3.0;
pub const SOUND_SECONDS: f32 = EVENT_EFFECT_SECONDS;
pub const SOUND_DECODED_BYTES: u64 = 8 * MIB;
pub const SOUND_RATE: u32 = 96_000;
pub const SOUND_CHANNELS: u32 = 2;
pub const EFFECT_CLIPS: u32 = 32;
pub const EFFECT_EMITTER_RUNS: u32 = 8;
pub const EVENT_COOLDOWN_SECONDS: f32 = 10.0;
pub const EVENT_JOIN_QUIET_SECONDS: f32 = 5.0;
pub const EVENT_JOIN_WINDOW_SECONDS: f32 = 10.0;
pub const EVENT_RESPAWN_WINDOW_SECONDS: f32 = 60.0;
pub const PET_LEASH_RADIUS: f32 = 3.0;
pub const PET_CATCH_UP_LEASHES: f32 = 4.0;
pub const PET_SNAP_LEASHES: f32 = 8.0;
pub const PET_FOLLOW_EASE: f32 = 0.15;
pub const PET_SPAWN_BEHIND_BLOCKS: f32 = 1.5;
pub const PET_FLYING_HEIGHT_BLOCKS: f32 = 1.4;
pub const PET_SIT_SECONDS: f32 = 3.0;
pub const PET_HURT_CLOSER_SECONDS: f32 = 5.0;
pub const PET_PAIR_START_BLOCKS: f32 = 8.0;
pub const PET_PAIR_END_BLOCKS: f32 = 8.0;
pub const PET_PAIR_WAIT_SECONDS: f32 = 3.0;
pub const PET_PAIR_COOLDOWN_SECONDS: f32 = 60.0;
pub const PET_PAIR_NAP_SECONDS: f32 = 30.0;
pub const PET_PAIRS: u32 = 2;
pub const PET_MOVING_SPEED: f32 = 0.25;
pub const PET_MOVE_CLIP_SPEED: f32 = 3.0;
pub const PET_RUN_CLIP_SPEED: f32 = 5.5;
pub const PET_CLIP_RATE_MAX: f32 = 2.5;
pub const PET_SCALE_MIN: f64 = 0.01;
pub const PET_SCALE_MAX: f64 = 8.0;
pub const EMOTE_BLEND_SECONDS: f32 = 0.14;
pub const ROLES_PER_EMOTE: u32 = 2;
pub const SHARED_RANGE_BLOCKS: f32 = 8.0;
pub const SHARED_PROMPT_SECONDS: f32 = 10.0;
pub const SHARED_MARK_BLOCKS: f32 = 0.75;
pub const SHARED_MARK_DEGREES: f32 = 30.0;
pub const SHARED_MARK_SECONDS: f32 = 0.5;
pub const SHARED_BREAK_BLOCKS: f32 = 1.0;

pub const CONSTANTS: &[(&str, f64)] = &[
    ("cape_download_bytes", CAPE_CAPS.download as f64),
    ("cape_unpacked_bytes", CAPE_CAPS.unpacked as f64),
    ("cape_entries", CAPE_CAPS.entries as f64),
    ("item_download_bytes", ITEM_CAPS.download as f64),
    ("item_unpacked_bytes", ITEM_CAPS.unpacked as f64),
    ("item_entries", ITEM_CAPS.entries as f64),
    ("pet_download_bytes", PET_CAPS.download as f64),
    ("pet_unpacked_bytes", PET_CAPS.unpacked as f64),
    ("pet_entries", PET_CAPS.entries as f64),
    ("emote_download_bytes", EMOTE_CAPS.download as f64),
    ("emote_unpacked_bytes", EMOTE_CAPS.unpacked as f64),
    ("emote_entries", EMOTE_CAPS.entries as f64),
    ("effect_download_bytes", EFFECT_CAPS.download as f64),
    ("effect_unpacked_bytes", EFFECT_CAPS.unpacked as f64),
    ("effect_entries", EFFECT_CAPS.entries as f64),
    ("nameplate_download_bytes", NAMEPLATE_CAPS.download as f64),
    ("nameplate_unpacked_bytes", NAMEPLATE_CAPS.unpacked as f64),
    ("nameplate_entries", NAMEPLATE_CAPS.entries as f64),
    ("nameplate_file_bytes", MIB as f64),
    ("json_max_bytes", JSON_MAX_BYTES as f64),
    ("json_max_depth", JSON_MAX_DEPTH as f64),
    ("id_min_len", ID_MIN_LEN as f64),
    ("id_max_len", ID_MAX_LEN as f64),
    ("texture_max_size", TEXTURE_MAX_SIZE as f64),
    (
        "nameplate_texture_max_size",
        NAMEPLATE_TEXTURE_MAX_SIZE as f64,
    ),
    ("texture_power_of_two", 1.0),
    ("quads_per_wearer", QUADS_PER_WEARER as f64),
    ("quads_per_nameplate", QUADS_PER_NAMEPLATE as f64),
    ("bones_per_item", BONES_PER_ITEM as f64),
    ("bones_per_pet", BONES_PER_PET as f64),
    ("spring_bones_per_item", SPRING_BONES_PER_ITEM as f64),
    ("spring_chain_depth", SPRING_CHAIN_DEPTH as f64),
    ("particles_per_emitter", PARTICLES_PER_EMITTER as f64),
    ("particles_per_cosmetic", PARTICLES_PER_COSMETIC as f64),
    ("trail_points", TRAIL_POINTS as f64),
    ("molang_ast_nodes", MOLANG_AST_NODES as f64),
    ("molang_depth", MOLANG_DEPTH as f64),
    ("molang_source_chars", MOLANG_SOURCE_CHARS as f64),
    ("molang_variables", MOLANG_VARIABLES as f64),
    ("molang_call_arguments", MOLANG_CALL_ARGUMENTS as f64),
    (
        "molang_steps_per_evaluation",
        MOLANG_STEPS_PER_EVALUATION as f64,
    ),
    ("cpu_units_per_wearer", CPU_UNITS_PER_WEARER as f64),
    ("wearer_record_bytes", WEARER_RECORD_BYTES as f64),
    ("flashes_per_second", FLASHES_PER_SECOND as f64),
    ("flash_area_pixels", FLASH_AREA_PIXELS as f64),
    ("flash_luminance_change", FLASH_LUMINANCE_CHANGE),
    ("detail_distance_blocks", DETAIL_DISTANCE_BLOCKS),
    ("detail_nearest_wearers", DETAIL_NEAREST_WEARERS as f64),
    ("minimal_detail_blocks", MINIMAL_DETAIL_BLOCKS),
    ("minimal_animation_hz", MINIMAL_ANIMATION_HZ as f64),
    (
        "reduced_motion_spring_scale",
        REDUCED_MOTION_SPRING_SCALE as f64,
    ),
    ("nameplate_fps_min", NAMEPLATE_FPS_MIN as f64),
    ("nameplate_fps_max", NAMEPLATE_FPS_MAX as f64),
    ("nameplate_loop_frames", NAMEPLATE_LOOP_FRAMES as f64),
    ("nameplate_layers", NAMEPLATE_LAYERS as f64),
    ("nameplate_slots", NAMEPLATE_SLOTS as f64),
    ("nameplate_tones", NAMEPLATE_TONES as f64),
    ("nameplate_params", NAMEPLATE_PARAMS as f64),
    ("nameplate_offset", NAMEPLATE_OFFSET as f64),
    ("nameplate_badge_offset", NAMEPLATE_BADGE_OFFSET as f64),
    ("nameplate_padding", NAMEPLATE_PADDING as f64),
    ("nameplate_min_width", NAMEPLATE_MIN_WIDTH as f64),
    ("nameplate_corner_radius", NAMEPLATE_CORNER_RADIUS as f64),
    ("nameplate_rings", NAMEPLATE_RINGS as f64),
    ("nameplate_sprite_size", NAMEPLATE_SPRITE_SIZE as f64),
    ("nameplate_sprite_frames", NAMEPLATE_SPRITE_FRAMES as f64),
    ("nameplate_sprite_colors", NAMEPLATE_SPRITE_COLORS as f64),
    ("nameplate_cached_states", NAMEPLATE_CACHED_STATES as f64),
    (
        "nameplate_path_points_min",
        NAMEPLATE_PATH_POINTS_MIN as f64,
    ),
    ("nameplate_path_points", NAMEPLATE_PATH_POINTS as f64),
    ("nameplate_path_speed", NAMEPLATE_PATH_SPEED as f64),
    ("nameplate_walkers", NAMEPLATE_WALKERS as f64),
    ("nameplate_laps", NAMEPLATE_LAPS as f64),
    ("nameplate_path_stops", NAMEPLATE_PATH_STOPS as f64),
    ("nameplate_pause_frames", NAMEPLATE_PAUSE_FRAMES as f64),
    ("nameplate_repeat_copies", NAMEPLATE_REPEAT_COPIES as f64),
    ("nameplate_breakpoints", NAMEPLATE_BREAKPOINTS as f64),
    ("nameplate_emitter_pool", NAMEPLATE_EMITTER_POOL as f64),
    ("nameplate_spawn_count", NAMEPLATE_SPAWN_COUNT as f64),
    ("nameplate_burst", NAMEPLATE_BURST as f64),
    (
        "nameplate_particle_lifetime",
        NAMEPLATE_PARTICLE_LIFETIME as f64,
    ),
    ("nameplate_particle_speed", NAMEPLATE_PARTICLE_SPEED as f64),
    (
        "nameplate_particle_gravity",
        NAMEPLATE_PARTICLE_GRAVITY as f64,
    ),
    ("nameplate_wobble", NAMEPLATE_WOBBLE as f64),
    ("nameplate_fade_steps", NAMEPLATE_FADE_STEPS as f64),
    ("nameplate_flags", NAMEPLATE_FLAGS as f64),
    ("nameplate_flag_bands", NAMEPLATE_FLAG_BANDS as f64),
    ("nameplate_band_weight", NAMEPLATE_BAND_WEIGHT as f64),
    ("nameplate_overlay_colors", NAMEPLATE_OVERLAY_COLORS as f64),
    ("nameplate_overlay_width", NAMEPLATE_OVERLAY_WIDTH as f64),
    ("nameplate_image_bytes", NAMEPLATE_IMAGE_BYTES as f64),
    (
        "nameplate_image_overhead_bytes",
        NAMEPLATE_IMAGE_OVERHEAD_BYTES as f64,
    ),
    ("nameplate_rect_bytes", NAMEPLATE_RECT_BYTES as f64),
    ("nameplate_strip_bytes", NAMEPLATE_STRIP_BYTES as f64),
    ("nameplate_cpu_units", NAMEPLATE_CPU_UNITS as f64),
    ("nameplate_cost_walker", NAMEPLATE_COST_WALKER as f64),
    (
        "nameplate_cost_repeat_copy",
        NAMEPLATE_COST_REPEAT_COPY as f64,
    ),
    ("nameplate_cost_particle", NAMEPLATE_COST_PARTICLE as f64),
    (
        "nameplate_cost_spawn_check",
        NAMEPLATE_COST_SPAWN_CHECK as f64,
    ),
    ("nameplate_cost_palette", NAMEPLATE_COST_PALETTE as f64),
    (
        "nameplate_cost_cycle_track",
        NAMEPLATE_COST_CYCLE_TRACK as f64,
    ),
    (
        "nameplate_cost_keys_track",
        NAMEPLATE_COST_KEYS_TRACK as f64,
    ),
    (
        "nameplate_cost_material_layer",
        NAMEPLATE_COST_MATERIAL_LAYER as f64,
    ),
    (
        "nameplate_cost_mirror_factor",
        NAMEPLATE_COST_MIRROR_FACTOR as f64,
    ),
    (
        "nameplate_cost_letter_factor",
        NAMEPLATE_COST_LETTER_FACTOR as f64,
    ),
    (
        "nameplate_decoration_margin",
        NAMEPLATE_DECORATION_MARGIN as f64,
    ),
    ("nameplate_material_nodes", NAMEPLATE_MATERIAL_NODES as f64),
    (
        "nameplate_material_samples",
        NAMEPLATE_MATERIAL_SAMPLES as f64,
    ),
    (
        "nameplate_material_inputs",
        NAMEPLATE_MATERIAL_INPUTS as f64,
    ),
    (
        "nameplate_material_octaves",
        NAMEPLATE_MATERIAL_OCTAVES as f64,
    ),
    ("nameplate_material_cost", NAMEPLATE_MATERIAL_COST as f64),
    (
        "nameplate_material_outline_factor",
        NAMEPLATE_MATERIAL_OUTLINE_FACTOR as f64,
    ),
    (
        "nameplate_material_margin",
        NAMEPLATE_MATERIAL_MARGIN as f64,
    ),
    (
        "nameplate_material_repeat",
        NAMEPLATE_MATERIAL_REPEAT as f64,
    ),
    ("nameplate_fallback_bytes", NAMEPLATE_FALLBACK_BYTES as f64),
    ("nameplate_owned_glyphs", NAMEPLATE_OWNED_GLYPHS as f64),
    ("nameplate_letters", NAMEPLATE_LETTERS as f64),
    ("nameplate_letter_offset", NAMEPLATE_LETTER_OFFSET as f64),
    (
        "nameplate_letter_scale_min_quarters",
        NAMEPLATE_LETTER_SCALE_MIN_QUARTERS as f64,
    ),
    (
        "nameplate_letter_scale_max_quarters",
        NAMEPLATE_LETTER_SCALE_MAX_QUARTERS as f64,
    ),
    ("nameplate_one_shots", NAMEPLATE_ONE_SHOTS as f64),
    (
        "nameplate_appear_seconds_min",
        NAMEPLATE_APPEAR_SECONDS_MIN as f64,
    ),
    ("nameplate_appear_seconds", NAMEPLATE_APPEAR_SECONDS as f64),
    ("nameplate_blend_rows", NAMEPLATE_BLEND_ROWS as f64),
    ("nameplate_interpolate", NAMEPLATE_INTERPOLATE as f64),
    (
        "nameplate_schedule_weight",
        NAMEPLATE_SCHEDULE_WEIGHT as f64,
    ),
    ("nameplate_flash_frames", NAMEPLATE_FLASH_FRAMES as f64),
    (
        "nameplate_flash_schedule_repeats",
        NAMEPLATE_FLASH_SCHEDULE_REPEATS as f64,
    ),
    ("nameplate_flash_work", NAMEPLATE_FLASH_WORK as f64),
    ("spring_step_hz", SPRING_STEP_HZ as f64),
    ("spring_max_steps", SPRING_MAX_STEPS as f64),
    ("spring_teleport_blocks", SPRING_TELEPORT_BLOCKS as f64),
    ("spring_water_drag", SPRING_WATER_DRAG as f64),
    ("spring_water_gravity", SPRING_WATER_GRAVITY as f64),
    ("spring_limit_degrees", SPRING_LIMIT_DEGREES as f64),
    ("effect_ring_points", EFFECT_RING_POINTS as f64),
    ("effect_ring_hz", EFFECT_RING_HZ as f64),
    (
        "effect_particle_life_seconds",
        EFFECT_PARTICLE_LIFE_SECONDS as f64,
    ),
    ("effect_footstep_blocks", EFFECT_FOOTSTEP_BLOCKS as f64),
    ("event_effect_seconds", EVENT_EFFECT_SECONDS as f64),
    ("sound_seconds", SOUND_SECONDS as f64),
    ("sound_decoded_bytes", SOUND_DECODED_BYTES as f64),
    ("sound_rate", SOUND_RATE as f64),
    ("sound_channels", SOUND_CHANNELS as f64),
    ("effect_clips", EFFECT_CLIPS as f64),
    ("effect_emitter_runs", EFFECT_EMITTER_RUNS as f64),
    ("event_cooldown_seconds", EVENT_COOLDOWN_SECONDS as f64),
    ("event_join_quiet_seconds", EVENT_JOIN_QUIET_SECONDS as f64),
    (
        "event_join_window_seconds",
        EVENT_JOIN_WINDOW_SECONDS as f64,
    ),
    (
        "event_respawn_window_seconds",
        EVENT_RESPAWN_WINDOW_SECONDS as f64,
    ),
    ("pet_leash_radius", PET_LEASH_RADIUS as f64),
    ("pet_catch_up_leashes", PET_CATCH_UP_LEASHES as f64),
    ("pet_snap_leashes", PET_SNAP_LEASHES as f64),
    ("pet_follow_ease", PET_FOLLOW_EASE as f64),
    ("pet_spawn_behind_blocks", PET_SPAWN_BEHIND_BLOCKS as f64),
    ("pet_flying_height_blocks", PET_FLYING_HEIGHT_BLOCKS as f64),
    ("pet_sit_seconds", PET_SIT_SECONDS as f64),
    ("pet_hurt_closer_seconds", PET_HURT_CLOSER_SECONDS as f64),
    ("pet_pair_start_blocks", PET_PAIR_START_BLOCKS as f64),
    ("pet_pair_end_blocks", PET_PAIR_END_BLOCKS as f64),
    ("pet_pair_wait_seconds", PET_PAIR_WAIT_SECONDS as f64),
    (
        "pet_pair_cooldown_seconds",
        PET_PAIR_COOLDOWN_SECONDS as f64,
    ),
    ("pet_pair_nap_seconds", PET_PAIR_NAP_SECONDS as f64),
    ("pet_pairs", PET_PAIRS as f64),
    ("pet_moving_speed", PET_MOVING_SPEED as f64),
    ("pet_move_clip_speed", PET_MOVE_CLIP_SPEED as f64),
    ("pet_run_clip_speed", PET_RUN_CLIP_SPEED as f64),
    ("pet_clip_rate_max", PET_CLIP_RATE_MAX as f64),
    ("pet_scale_min", PET_SCALE_MIN),
    ("pet_scale_max", PET_SCALE_MAX),
    ("emote_blend_seconds", EMOTE_BLEND_SECONDS as f64),
    ("roles_per_emote", ROLES_PER_EMOTE as f64),
    ("shared_range_blocks", SHARED_RANGE_BLOCKS as f64),
    ("shared_prompt_seconds", SHARED_PROMPT_SECONDS as f64),
    ("shared_mark_blocks", SHARED_MARK_BLOCKS as f64),
    ("shared_mark_degrees", SHARED_MARK_DEGREES as f64),
    ("shared_mark_seconds", SHARED_MARK_SECONDS as f64),
    ("shared_break_blocks", SHARED_BREAK_BLOCKS as f64),
];

pub fn constants() -> &'static [(&'static str, f64)] {
    CONSTANTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_export_every_cap_once_with_its_value() {
        let table = constants();
        for (i, (name, _)) in table.iter().enumerate() {
            assert!(!table[i + 1..].iter().any(|(n, _)| n == name), "{name} listed twice");
        }
        let value = |name: &str| table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
        assert_eq!(value("json_max_bytes"), Some(262_144.0));
        assert_eq!(value("wearer_record_bytes"), Some(2560.0));
        assert_eq!(value("effect_particle_life_seconds"), Some(3.1f32 as f64));
        assert_eq!(value("nameplate_file_bytes"), Some(NAMEPLATE_CAPS.per_file.unwrap() as f64));
    }
}
