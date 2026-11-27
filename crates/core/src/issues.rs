use serde::Serialize;
use serde_json::{Map, Value};

use crate::molang::ErrorCode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Client,
    Authoring,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    ZipInvalid,
    DownloadTooLarge,
    TooManyEntries,
    UnpackedTooLarge,
    FileTooLarge,
    UnsafeEntryName,
    EntryUnreadable,
    JsonTooLarge,
    JsonTooDeep,
    JsonInvalid,
    ManifestNotObject,
    FormatMissing,
    FormatInvalid,
    RequiresInvalid,
    KindMissing,
    KindUnknown,
    IdMissing,
    IdInvalid,
    UnknownKey,
    BadPath,
    MissingFile,
    UnreferencedFile,
    PngInvalid,
    TextureTooLarge,
    TextureNotPowerOfTwo,
    QuadsOverBudget,
    BonesOverBudget,
    WearerRecordOverBudget,
    WearerVariablesOverBudget,
    WearerOneShotsOverBudget,
    MolangNodesOverBudget,
    EmitterParticlesOverBudget,
    CosmeticParticlesOverBudget,
    TrailPointsOverBudget,
    SpringBonesOverBudget,
    SpringChainTooDeep,
    ControllerInvalid,
    ControllerUnsupported,
    ControllerTooManyStates,
    ControllerTooManyControllers,
    ControllerServerSide,
    ControllerUnknownState,
    ControllerMolangInvalid,
    ControllerMissing,
    ControllerUnknownAnimation,
    Manifest,
    Layers,
    Layer,
    Palette,
    Colorway,
    ColorwayTones,
    Variant,
    VariantFlagMissing,
    BackgroundWithoutBody,
    Text,
    LetterProgram,
    LetterOwned,
    LetterBudget,
    LetterReadable,
    Badge,
    Corners,
    CornersBoth,
    Flags,
    FlagUnfit,
    Sprite,
    SpriteFrames,
    Path,
    Repeat,
    Widths,
    Emitter,
    Animation,
    KeyStart,
    KeyOrder,
    KeyRange,
    SpeedRange,
    SpeedTrigger,
    SpeedPath,
    BurstCount,
    BadgeSchedule,
    BadgeChannel,
    BadgeMolang,
    Group,
    Reaction,
    ReducedWhile,
    ReducedChoice,
    PaletteAnimation,
    PaletteCycle,
    PaletteMolang,
    Molang,
    MolangEmpty,
    MolangUnexpected,
    MolangTooLong,
    MolangTooDeep,
    MolangTooBig,
    MolangUnterminated,
    MolangCharacter,
    MolangString,
    MolangValue,
    MolangUncalled,
    MolangUnknownName,
    MolangNotQuery,
    MolangUnknownQuery,
    MolangQueryPlace,
    MolangAssign,
    MolangVariableName,
    MolangVariables,
    MolangUnknownFunction,
    MolangUnknownMath,
    MolangParamCall,
    MolangUnknownParam,
    MolangArguments,
    MolangCost,
    MolangConstant,
    MolangUnknownRoot,
    MolangUnsupported,
    Material,
    MaterialLoop,
    MaterialNodes,
    MaterialCost,
    MaterialSamples,
    MaterialInputs,
    MaterialOutputs,
    MaterialCycle,
    MaterialMargins,
    NodeId,
    NodeDuplicate,
    NodeType,
    NodeInput,
    NodeLink,
    NodeOutputLink,
    NodeLiteral,
    NodeRequired,
    NodeSetting,
    NodeParam,
    NodeTexture,
    NodeSprite,
    NodeTyping,
    NodeRepeat,
    Texture,
    Layout,
    Mask,
    BodyArt,
    ArtOutsideWindow,
    Anchor,
    Follow,
    PathTiming,
    NameTarget,
    PlateChannel,
    OverCpu,
    OverCache,
    OverMargin,
    OverStates,
    OverFallback,
    Flash,
    FlashUnchecked,
    Readability,
    BadgeHidden,
    BadgeFaded,
    BadgeCovered,
    ArtCoverage,
    FixedHeight,
    TranslucentOverlap,
    PositionClamped,
    PlateBackground,
    FallbackCache,
    FallbackStill,
    FallbackRebuild,
    FallbackShots,
    SpringInvalid,
    SpringSettingRange,
    SpringChainBroken,
    EffectInvalid,
    EffectUnsupported,
    EffectMissing,
    EffectTooLong,
    EffectClipsTooMany,
    EffectEventUnknown,
    ParticleMissing,
    ParticleTextureMissing,
    SoundMissing,
    SoundNotAllowed,
    SoundUnsupported,
    SoundTooLong,
    SoundTooBig,
    VanillaParticleNotAllowed,
    LocatorMissing,
    TrailInvalid,
    ReactionInvalid,
    ReactionSignalUnknown,
    ReactionMissing,
    AttachInvalid,
    RigUnknown,
    AttachBoneUnknown,
    ReplacementUnknown,
    AssetKindUnsupported,
    GeometryMissing,
    GeometryInvalid,
    AnimationsInvalid,
    PetInvalid,
    PetArchetypeUnknown,
    PetFieldRange,
    PetScaleRange,
    PetInteractionUnknown,
    PetLegacyInvalid,
    PetPlainQuery,
    RoleInvalid,
    RoleUnknown,
    RoleDuplicate,
    RolesTooMany,
    RoleAnimationMissing,
    EmoteRulesInvalid,
    TextureIndexSize,
    TextureIndexInvalid,
    TextureIndexOffPalette,
    DialectUnsupported,
    SoundRateUnsupported,
    SoundChannelsUnsupported,
    EffectRunsTooMany,
    EmoteMainUnnamed,
}

impl IssueCode {
    pub const ALL: [IssueCode; 169] = [
        IssueCode::ZipInvalid,
        IssueCode::DownloadTooLarge,
        IssueCode::TooManyEntries,
        IssueCode::UnpackedTooLarge,
        IssueCode::FileTooLarge,
        IssueCode::UnsafeEntryName,
        IssueCode::EntryUnreadable,
        IssueCode::JsonTooLarge,
        IssueCode::JsonTooDeep,
        IssueCode::JsonInvalid,
        IssueCode::ManifestNotObject,
        IssueCode::FormatMissing,
        IssueCode::FormatInvalid,
        IssueCode::RequiresInvalid,
        IssueCode::KindMissing,
        IssueCode::KindUnknown,
        IssueCode::IdMissing,
        IssueCode::IdInvalid,
        IssueCode::UnknownKey,
        IssueCode::BadPath,
        IssueCode::MissingFile,
        IssueCode::UnreferencedFile,
        IssueCode::PngInvalid,
        IssueCode::TextureTooLarge,
        IssueCode::TextureNotPowerOfTwo,
        IssueCode::QuadsOverBudget,
        IssueCode::BonesOverBudget,
        IssueCode::WearerRecordOverBudget,
        IssueCode::WearerVariablesOverBudget,
        IssueCode::WearerOneShotsOverBudget,
        IssueCode::MolangNodesOverBudget,
        IssueCode::EmitterParticlesOverBudget,
        IssueCode::CosmeticParticlesOverBudget,
        IssueCode::TrailPointsOverBudget,
        IssueCode::SpringBonesOverBudget,
        IssueCode::SpringChainTooDeep,
        IssueCode::ControllerInvalid,
        IssueCode::ControllerUnsupported,
        IssueCode::ControllerTooManyStates,
        IssueCode::ControllerTooManyControllers,
        IssueCode::ControllerServerSide,
        IssueCode::ControllerUnknownState,
        IssueCode::ControllerMolangInvalid,
        IssueCode::ControllerMissing,
        IssueCode::ControllerUnknownAnimation,
        IssueCode::Manifest,
        IssueCode::Layers,
        IssueCode::Layer,
        IssueCode::Palette,
        IssueCode::Colorway,
        IssueCode::ColorwayTones,
        IssueCode::Variant,
        IssueCode::VariantFlagMissing,
        IssueCode::BackgroundWithoutBody,
        IssueCode::Text,
        IssueCode::LetterProgram,
        IssueCode::LetterOwned,
        IssueCode::LetterBudget,
        IssueCode::LetterReadable,
        IssueCode::Badge,
        IssueCode::Corners,
        IssueCode::CornersBoth,
        IssueCode::Flags,
        IssueCode::FlagUnfit,
        IssueCode::Sprite,
        IssueCode::SpriteFrames,
        IssueCode::Path,
        IssueCode::Repeat,
        IssueCode::Widths,
        IssueCode::Emitter,
        IssueCode::Animation,
        IssueCode::KeyStart,
        IssueCode::KeyOrder,
        IssueCode::KeyRange,
        IssueCode::SpeedRange,
        IssueCode::SpeedTrigger,
        IssueCode::SpeedPath,
        IssueCode::BurstCount,
        IssueCode::BadgeSchedule,
        IssueCode::BadgeChannel,
        IssueCode::BadgeMolang,
        IssueCode::Group,
        IssueCode::Reaction,
        IssueCode::ReducedWhile,
        IssueCode::ReducedChoice,
        IssueCode::PaletteAnimation,
        IssueCode::PaletteCycle,
        IssueCode::PaletteMolang,
        IssueCode::Molang,
        IssueCode::MolangEmpty,
        IssueCode::MolangUnexpected,
        IssueCode::MolangTooLong,
        IssueCode::MolangTooDeep,
        IssueCode::MolangTooBig,
        IssueCode::MolangUnterminated,
        IssueCode::MolangCharacter,
        IssueCode::MolangString,
        IssueCode::MolangValue,
        IssueCode::MolangUncalled,
        IssueCode::MolangUnknownName,
        IssueCode::MolangNotQuery,
        IssueCode::MolangUnknownQuery,
        IssueCode::MolangQueryPlace,
        IssueCode::MolangAssign,
        IssueCode::MolangVariableName,
        IssueCode::MolangVariables,
        IssueCode::MolangUnknownFunction,
        IssueCode::MolangUnknownMath,
        IssueCode::MolangParamCall,
        IssueCode::MolangUnknownParam,
        IssueCode::MolangArguments,
        IssueCode::MolangCost,
        IssueCode::MolangConstant,
        IssueCode::MolangUnknownRoot,
        IssueCode::MolangUnsupported,
        IssueCode::Material,
        IssueCode::MaterialLoop,
        IssueCode::MaterialNodes,
        IssueCode::MaterialCost,
        IssueCode::MaterialSamples,
        IssueCode::MaterialInputs,
        IssueCode::MaterialOutputs,
        IssueCode::MaterialCycle,
        IssueCode::MaterialMargins,
        IssueCode::NodeId,
        IssueCode::NodeDuplicate,
        IssueCode::NodeType,
        IssueCode::NodeInput,
        IssueCode::NodeLink,
        IssueCode::NodeOutputLink,
        IssueCode::NodeLiteral,
        IssueCode::NodeRequired,
        IssueCode::NodeSetting,
        IssueCode::NodeParam,
        IssueCode::NodeTexture,
        IssueCode::NodeSprite,
        IssueCode::NodeTyping,
        IssueCode::NodeRepeat,
        IssueCode::Texture,
        IssueCode::Layout,
        IssueCode::Mask,
        IssueCode::BodyArt,
        IssueCode::ArtOutsideWindow,
        IssueCode::Anchor,
        IssueCode::Follow,
        IssueCode::PathTiming,
        IssueCode::NameTarget,
        IssueCode::PlateChannel,
        IssueCode::OverCpu,
        IssueCode::OverCache,
        IssueCode::OverMargin,
        IssueCode::OverStates,
        IssueCode::OverFallback,
        IssueCode::Flash,
        IssueCode::Readability,
        IssueCode::BadgeHidden,
        IssueCode::BadgeFaded,
        IssueCode::BadgeCovered,
        IssueCode::ArtCoverage,
        IssueCode::FixedHeight,
        IssueCode::TranslucentOverlap,
        IssueCode::PositionClamped,
        IssueCode::PlateBackground,
        IssueCode::FallbackCache,
        IssueCode::FallbackStill,
        IssueCode::FallbackRebuild,
        IssueCode::FallbackShots,
        IssueCode::DialectUnsupported,
        IssueCode::FlashUnchecked,
    ];

    pub const ENGINE: [IssueCode; 50] = [
        IssueCode::SpringInvalid,
        IssueCode::SpringSettingRange,
        IssueCode::SpringChainBroken,
        IssueCode::EffectInvalid,
        IssueCode::EffectUnsupported,
        IssueCode::EffectMissing,
        IssueCode::EffectTooLong,
        IssueCode::EffectClipsTooMany,
        IssueCode::EffectEventUnknown,
        IssueCode::ParticleMissing,
        IssueCode::ParticleTextureMissing,
        IssueCode::SoundMissing,
        IssueCode::SoundNotAllowed,
        IssueCode::SoundUnsupported,
        IssueCode::SoundTooLong,
        IssueCode::SoundTooBig,
        IssueCode::VanillaParticleNotAllowed,
        IssueCode::LocatorMissing,
        IssueCode::TrailInvalid,
        IssueCode::ReactionInvalid,
        IssueCode::ReactionSignalUnknown,
        IssueCode::ReactionMissing,
        IssueCode::AttachInvalid,
        IssueCode::RigUnknown,
        IssueCode::AttachBoneUnknown,
        IssueCode::ReplacementUnknown,
        IssueCode::AssetKindUnsupported,
        IssueCode::GeometryMissing,
        IssueCode::GeometryInvalid,
        IssueCode::AnimationsInvalid,
        IssueCode::PetInvalid,
        IssueCode::PetArchetypeUnknown,
        IssueCode::PetFieldRange,
        IssueCode::PetScaleRange,
        IssueCode::PetInteractionUnknown,
        IssueCode::PetLegacyInvalid,
        IssueCode::PetPlainQuery,
        IssueCode::RoleInvalid,
        IssueCode::RoleUnknown,
        IssueCode::RoleDuplicate,
        IssueCode::RolesTooMany,
        IssueCode::RoleAnimationMissing,
        IssueCode::EmoteRulesInvalid,
        IssueCode::TextureIndexSize,
        IssueCode::TextureIndexInvalid,
        IssueCode::TextureIndexOffPalette,
        IssueCode::SoundRateUnsupported,
        IssueCode::SoundChannelsUnsupported,
        IssueCode::EffectRunsTooMany,
        IssueCode::EmoteMainUnnamed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IssueCode::ZipInvalid => "zip_invalid",
            IssueCode::DownloadTooLarge => "download_too_large",
            IssueCode::TooManyEntries => "too_many_entries",
            IssueCode::UnpackedTooLarge => "unpacked_too_large",
            IssueCode::FileTooLarge => "file_too_large",
            IssueCode::UnsafeEntryName => "unsafe_entry_name",
            IssueCode::EntryUnreadable => "entry_unreadable",
            IssueCode::JsonTooLarge => "json_too_large",
            IssueCode::JsonTooDeep => "json_too_deep",
            IssueCode::JsonInvalid => "json_invalid",
            IssueCode::ManifestNotObject => "manifest_not_object",
            IssueCode::FormatMissing => "format_missing",
            IssueCode::FormatInvalid => "format_invalid",
            IssueCode::RequiresInvalid => "requires_invalid",
            IssueCode::KindMissing => "kind_missing",
            IssueCode::KindUnknown => "kind_unknown",
            IssueCode::IdMissing => "id_missing",
            IssueCode::IdInvalid => "id_invalid",
            IssueCode::UnknownKey => "unknown_key",
            IssueCode::BadPath => "bad_path",
            IssueCode::MissingFile => "missing_file",
            IssueCode::UnreferencedFile => "unreferenced_file",
            IssueCode::PngInvalid => "png_invalid",
            IssueCode::TextureTooLarge => "texture_too_large",
            IssueCode::TextureNotPowerOfTwo => "texture_not_power_of_two",
            IssueCode::QuadsOverBudget => "quads_over_budget",
            IssueCode::BonesOverBudget => "bones_over_budget",
            IssueCode::WearerRecordOverBudget => "wearer_record_over_budget",
            IssueCode::WearerVariablesOverBudget => "wearer_variables_over_budget",
            IssueCode::WearerOneShotsOverBudget => "wearer_one_shots_over_budget",
            IssueCode::MolangNodesOverBudget => "molang_nodes_over_budget",
            IssueCode::EmitterParticlesOverBudget => "emitter_particles_over_budget",
            IssueCode::CosmeticParticlesOverBudget => "cosmetic_particles_over_budget",
            IssueCode::TrailPointsOverBudget => "trail_points_over_budget",
            IssueCode::SpringBonesOverBudget => "spring_bones_over_budget",
            IssueCode::SpringChainTooDeep => "spring_chain_too_deep",
            IssueCode::ControllerInvalid => "controller_invalid",
            IssueCode::ControllerUnsupported => "controller_unsupported",
            IssueCode::ControllerTooManyStates => "controller_too_many_states",
            IssueCode::ControllerTooManyControllers => "controller_too_many_controllers",
            IssueCode::ControllerServerSide => "controller_server_side",
            IssueCode::ControllerUnknownState => "controller_unknown_state",
            IssueCode::ControllerMolangInvalid => "controller_molang_invalid",
            IssueCode::ControllerMissing => "controller_missing",
            IssueCode::ControllerUnknownAnimation => "controller_unknown_animation",
            IssueCode::Manifest => "manifest",
            IssueCode::Layers => "layers",
            IssueCode::Layer => "layer",
            IssueCode::Palette => "palette",
            IssueCode::Colorway => "colorway",
            IssueCode::ColorwayTones => "colorway_tones",
            IssueCode::Variant => "variant",
            IssueCode::VariantFlagMissing => "variant_flag_missing",
            IssueCode::BackgroundWithoutBody => "background_without_body",
            IssueCode::Text => "text",
            IssueCode::LetterProgram => "letter_program",
            IssueCode::LetterOwned => "letter_owned",
            IssueCode::LetterBudget => "letter_budget",
            IssueCode::LetterReadable => "letter_readable",
            IssueCode::Badge => "badge",
            IssueCode::Corners => "corners",
            IssueCode::CornersBoth => "corners_both",
            IssueCode::Flags => "flags",
            IssueCode::FlagUnfit => "flag_unfit",
            IssueCode::Sprite => "sprite",
            IssueCode::SpriteFrames => "sprite_frames",
            IssueCode::Path => "path",
            IssueCode::Repeat => "repeat",
            IssueCode::Widths => "widths",
            IssueCode::Emitter => "emitter",
            IssueCode::Animation => "animation",
            IssueCode::KeyStart => "key_start",
            IssueCode::KeyOrder => "key_order",
            IssueCode::KeyRange => "key_range",
            IssueCode::SpeedRange => "speed_range",
            IssueCode::SpeedTrigger => "speed_trigger",
            IssueCode::SpeedPath => "speed_path",
            IssueCode::BurstCount => "burst_count",
            IssueCode::BadgeSchedule => "badge_schedule",
            IssueCode::BadgeChannel => "badge_channel",
            IssueCode::BadgeMolang => "badge_molang",
            IssueCode::Group => "group",
            IssueCode::Reaction => "reaction",
            IssueCode::ReducedWhile => "reduced_while",
            IssueCode::ReducedChoice => "reduced_choice",
            IssueCode::PaletteAnimation => "palette_animation",
            IssueCode::PaletteCycle => "palette_cycle",
            IssueCode::PaletteMolang => "palette_molang",
            IssueCode::Molang => "molang",
            IssueCode::MolangEmpty => "molang_empty",
            IssueCode::MolangUnexpected => "molang_unexpected",
            IssueCode::MolangTooLong => "molang_too_long",
            IssueCode::MolangTooDeep => "molang_too_deep",
            IssueCode::MolangTooBig => "molang_too_big",
            IssueCode::MolangUnterminated => "molang_unterminated",
            IssueCode::MolangCharacter => "molang_character",
            IssueCode::MolangString => "molang_string",
            IssueCode::MolangValue => "molang_value",
            IssueCode::MolangUncalled => "molang_uncalled",
            IssueCode::MolangUnknownName => "molang_unknown_name",
            IssueCode::MolangNotQuery => "molang_not_query",
            IssueCode::MolangUnknownQuery => "molang_unknown_query",
            IssueCode::MolangQueryPlace => "molang_query_place",
            IssueCode::MolangAssign => "molang_assign",
            IssueCode::MolangVariableName => "molang_variable_name",
            IssueCode::MolangVariables => "molang_variables",
            IssueCode::MolangUnknownFunction => "molang_unknown_function",
            IssueCode::MolangUnknownMath => "molang_unknown_math",
            IssueCode::MolangParamCall => "molang_param_call",
            IssueCode::MolangUnknownParam => "molang_unknown_param",
            IssueCode::MolangArguments => "molang_arguments",
            IssueCode::MolangCost => "molang_cost",
            IssueCode::MolangConstant => "molang_constant",
            IssueCode::MolangUnknownRoot => "molang_unknown_root",
            IssueCode::MolangUnsupported => "molang_unsupported",
            IssueCode::Material => "material",
            IssueCode::MaterialLoop => "material_loop",
            IssueCode::MaterialNodes => "material_nodes",
            IssueCode::MaterialCost => "material_cost",
            IssueCode::MaterialSamples => "material_samples",
            IssueCode::MaterialInputs => "material_inputs",
            IssueCode::MaterialOutputs => "material_outputs",
            IssueCode::MaterialCycle => "material_cycle",
            IssueCode::MaterialMargins => "material_margins",
            IssueCode::NodeId => "node_id",
            IssueCode::NodeDuplicate => "node_duplicate",
            IssueCode::NodeType => "node_type",
            IssueCode::NodeInput => "node_input",
            IssueCode::NodeLink => "node_link",
            IssueCode::NodeOutputLink => "node_output_link",
            IssueCode::NodeLiteral => "node_literal",
            IssueCode::NodeRequired => "node_required",
            IssueCode::NodeSetting => "node_setting",
            IssueCode::NodeParam => "node_param",
            IssueCode::NodeTexture => "node_texture",
            IssueCode::NodeSprite => "node_sprite",
            IssueCode::NodeTyping => "node_typing",
            IssueCode::NodeRepeat => "node_repeat",
            IssueCode::Texture => "texture",
            IssueCode::Layout => "layout",
            IssueCode::Mask => "mask",
            IssueCode::BodyArt => "body_art",
            IssueCode::ArtOutsideWindow => "art_outside_window",
            IssueCode::Anchor => "anchor",
            IssueCode::Follow => "follow",
            IssueCode::PathTiming => "path_timing",
            IssueCode::NameTarget => "name_target",
            IssueCode::PlateChannel => "plate_channel",
            IssueCode::OverCpu => "over_cpu",
            IssueCode::OverCache => "over_cache",
            IssueCode::OverMargin => "over_margin",
            IssueCode::OverStates => "over_states",
            IssueCode::OverFallback => "over_fallback",
            IssueCode::Flash => "flash",
            IssueCode::FlashUnchecked => "flash_unchecked",
            IssueCode::Readability => "readability",
            IssueCode::BadgeHidden => "badge_hidden",
            IssueCode::BadgeFaded => "badge_faded",
            IssueCode::BadgeCovered => "badge_covered",
            IssueCode::ArtCoverage => "art_coverage",
            IssueCode::FixedHeight => "fixed_height",
            IssueCode::TranslucentOverlap => "translucent_overlap",
            IssueCode::PositionClamped => "position_clamped",
            IssueCode::PlateBackground => "plate_background",
            IssueCode::FallbackCache => "fallback_cache",
            IssueCode::FallbackStill => "fallback_still",
            IssueCode::FallbackRebuild => "fallback_rebuild",
            IssueCode::FallbackShots => "fallback_shots",
            IssueCode::DialectUnsupported => "dialect_unsupported",
            IssueCode::SpringInvalid => "spring_invalid",
            IssueCode::SpringSettingRange => "spring_setting_range",
            IssueCode::SpringChainBroken => "spring_chain_broken",
            IssueCode::EffectInvalid => "effect_invalid",
            IssueCode::EffectUnsupported => "effect_unsupported",
            IssueCode::EffectMissing => "effect_missing",
            IssueCode::EffectTooLong => "effect_too_long",
            IssueCode::EffectClipsTooMany => "effect_clips_too_many",
            IssueCode::EffectEventUnknown => "effect_event_unknown",
            IssueCode::ParticleMissing => "particle_missing",
            IssueCode::ParticleTextureMissing => "particle_texture_missing",
            IssueCode::SoundMissing => "sound_missing",
            IssueCode::SoundNotAllowed => "sound_not_allowed",
            IssueCode::SoundUnsupported => "sound_unsupported",
            IssueCode::SoundTooLong => "sound_too_long",
            IssueCode::SoundTooBig => "sound_too_big",
            IssueCode::SoundRateUnsupported => "sound_rate_unsupported",
            IssueCode::SoundChannelsUnsupported => "sound_channels_unsupported",
            IssueCode::EffectRunsTooMany => "effect_runs_too_many",
            IssueCode::VanillaParticleNotAllowed => "vanilla_particle_not_allowed",
            IssueCode::LocatorMissing => "locator_missing",
            IssueCode::TrailInvalid => "trail_invalid",
            IssueCode::ReactionInvalid => "reaction_invalid",
            IssueCode::ReactionSignalUnknown => "reaction_signal_unknown",
            IssueCode::ReactionMissing => "reaction_missing",
            IssueCode::AttachInvalid => "attach_invalid",
            IssueCode::RigUnknown => "rig_unknown",
            IssueCode::AttachBoneUnknown => "attach_bone_unknown",
            IssueCode::ReplacementUnknown => "replacement_unknown",
            IssueCode::AssetKindUnsupported => "asset_kind_unsupported",
            IssueCode::GeometryMissing => "geometry_missing",
            IssueCode::GeometryInvalid => "geometry_invalid",
            IssueCode::AnimationsInvalid => "animations_invalid",
            IssueCode::PetInvalid => "pet_invalid",
            IssueCode::PetArchetypeUnknown => "pet_archetype_unknown",
            IssueCode::PetFieldRange => "pet_field_range",
            IssueCode::PetScaleRange => "pet_scale_range",
            IssueCode::PetInteractionUnknown => "pet_interaction_unknown",
            IssueCode::PetLegacyInvalid => "pet_legacy_invalid",
            IssueCode::PetPlainQuery => "pet_plain_query",
            IssueCode::RoleInvalid => "role_invalid",
            IssueCode::RoleUnknown => "role_unknown",
            IssueCode::RoleDuplicate => "role_duplicate",
            IssueCode::RolesTooMany => "roles_too_many",
            IssueCode::RoleAnimationMissing => "role_animation_missing",
            IssueCode::EmoteRulesInvalid => "emote_rules_invalid",
            IssueCode::EmoteMainUnnamed => "emote_main_unnamed",
            IssueCode::TextureIndexSize => "texture_index_size",
            IssueCode::TextureIndexInvalid => "texture_index_invalid",
            IssueCode::TextureIndexOffPalette => "texture_index_off_palette",
        }
    }
}

impl From<ErrorCode> for IssueCode {
    fn from(code: ErrorCode) -> IssueCode {
        match code {
            ErrorCode::Other => IssueCode::Molang,
            ErrorCode::TooLong => IssueCode::MolangTooLong,
            ErrorCode::TooDeep => IssueCode::MolangTooDeep,
            ErrorCode::TooBig => IssueCode::MolangTooBig,
            ErrorCode::Variables => IssueCode::MolangVariables,
            ErrorCode::Arguments => IssueCode::MolangArguments,
            ErrorCode::TooManyNames => IssueCode::MolangTooBig,
            ErrorCode::Character => IssueCode::MolangCharacter,
            ErrorCode::Number => IssueCode::MolangValue,
            ErrorCode::Unterminated => IssueCode::MolangUnterminated,
            ErrorCode::Unexpected => IssueCode::MolangUnexpected,
            ErrorCode::Unsupported => IssueCode::MolangUnsupported,
            ErrorCode::StringUse => IssueCode::MolangString,
            ErrorCode::UnknownFunction => IssueCode::MolangUnknownFunction,
            ErrorCode::UnknownMath => IssueCode::MolangUnknownMath,
            ErrorCode::UnknownQuery => IssueCode::MolangUnknownQuery,
            ErrorCode::UnknownName => IssueCode::MolangUnknownName,
            ErrorCode::UnknownRoot => IssueCode::MolangUnknownRoot,
            ErrorCode::Arity => IssueCode::MolangArguments,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Issue {
    pub code: IssueCode,
    pub level: Level,
    pub path: String,
    pub values: Vec<(String, f64)>,
    pub names: Vec<(String, String)>,
}

impl Issue {
    pub fn new(code: IssueCode, level: Level, path: impl Into<String>) -> Self {
        Issue {
            code,
            level,
            path: path.into(),
            values: Vec::new(),
            names: Vec::new(),
        }
    }

    pub fn error(code: IssueCode, path: impl Into<String>) -> Self {
        Issue::new(code, Level::Error, path)
    }

    pub fn warning(code: IssueCode, path: impl Into<String>) -> Self {
        Issue::new(code, Level::Warning, path)
    }

    pub fn info(code: IssueCode, path: impl Into<String>) -> Self {
        Issue::new(code, Level::Info, path)
    }

    pub fn value(mut self, key: &str, value: f64) -> Self {
        self.values.push((key.to_string(), value));
        self
    }

    pub fn name(mut self, key: &str, name: &str) -> Self {
        self.names.push((key.to_string(), sanitize(name)));
        self
    }
}

pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

pub fn path(file: &str, at: &str) -> String {
    if at.is_empty() {
        file.to_string()
    } else {
        format!("{file}#{at}")
    }
}

pub fn join(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_string()
    } else {
        format!("{at}.{key}")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    mode: Mode,
    issues: Vec<Issue>,
}

impl Report {
    pub fn new(mode: Mode) -> Self {
        Report {
            mode,
            issues: Vec::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn push(&mut self, issue: Issue) {
        if self.mode == Mode::Authoring || issue.level == Level::Error {
            self.issues.push(issue);
        }
    }

    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    pub fn into_issues(self) -> Vec<Issue> {
        self.issues
    }

    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.level == Level::Error)
    }

    pub fn check_keys(
        &mut self,
        file: &str,
        at: &str,
        object: &Map<String, Value>,
        known: &[&str],
    ) {
        if self.mode != Mode::Authoring {
            return;
        }
        for key in object.keys() {
            if !known.contains(&key.as_str()) {
                self.push(
                    Issue::error(IssueCode::UnknownKey, path(file, &join(at, key)))
                        .name("key", key),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codes_keep_their_wire_names_and_numbers() {
        let all: Vec<IssueCode> = IssueCode::ALL.iter().chain(IssueCode::ENGINE.iter()).copied().collect();
        let names: Vec<&str> = all.iter().map(|c| c.as_str()).collect();
        for (i, code) in all.iter().enumerate() {
            assert_eq!(serde_json::to_value(code).unwrap(), json!(names[i]));
            assert!(!names[i + 1..].contains(&names[i]), "{} listed twice", names[i]);
        }
        assert_eq!(IssueCode::ALL.len(), 169);
        assert_eq!((IssueCode::ALL[45], IssueCode::ALL[168]), (IssueCode::Manifest, IssueCode::FlashUnchecked));
        assert_eq!(IssueCode::ENGINE[0], IssueCode::SpringInvalid);
        assert_eq!(IssueCode::TextureNotPowerOfTwo.as_str(), "texture_not_power_of_two");
        for code in ErrorCode::ALL {
            assert!(IssueCode::from(code).as_str().starts_with("molang"), "{code:?}");
        }
        assert_eq!(IssueCode::from(ErrorCode::Arity), IssueCode::MolangArguments);
    }

    #[test]
    fn reports_keep_what_their_mode_shows() {
        let mut client = Report::new(Mode::Client);
        client.push(Issue::warning(IssueCode::UnreferencedFile, "a"));
        client.push(Issue::error(IssueCode::MissingFile, "a"));
        assert_eq!((client.issues().len(), client.has_errors()), (1, true));

        let object = json!({ "format": 1, "colourways": [] });
        client.check_keys("manifest.json", "", object.as_object().unwrap(), &["format", "colorways"]);
        assert_eq!(client.issues().len(), 1);
        let mut authoring = Report::new(Mode::Authoring);
        authoring.check_keys("manifest.json", "", object.as_object().unwrap(), &["format", "colorways"]);
        let issue = &authoring.issues()[0];
        assert_eq!((issue.code, issue.level, issue.path.as_str()), (IssueCode::UnknownKey, Level::Error, "manifest.json#colourways"));
        assert_eq!(issue.names, vec![("key".to_string(), "colourways".to_string())]);

        let named = Issue::error(IssueCode::IdInvalid, "manifest.json#id").name("id", "a\nb\u{7}c\u{9b}d");
        assert_eq!(named.names[0].1, "a?b?c?d");
        assert_eq!(path("manifest.json", &join(&join("variants", "mint"), "palette")), "manifest.json#variants.mint.palette");
    }
}
