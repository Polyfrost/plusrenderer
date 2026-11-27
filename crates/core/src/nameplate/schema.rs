#[cfg(feature = "schema")]
pub fn schema_of<T: schemars::JsonSchema>() -> serde_json::Value {
    schemars::schema_for!(T).to_value()
}

#[cfg(feature = "schema")]
pub fn schemas() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("manifest.json", file_schema::<shapes::Manifest>()),
        ("palettes.json", file_schema::<shapes::PaletteFile>()),
        ("flags.json", file_schema::<shapes::FlagsFile>()),
        ("layers/<id>.json", file_schema::<shapes::LayerFile>()),
        ("sprites/<id>.json", file_schema::<shapes::SpriteFile>()),
        ("materials/<id>.json", file_schema::<shapes::MaterialFile>()),
    ]
}

#[cfg(feature = "schema")]
fn file_schema<T: schemars::JsonSchema>() -> serde_json::Value {
    let mut schema = schema_of::<T>();
    shapes::forbid_null(&mut schema);
    schema
}

#[cfg(feature = "schema")]
#[allow(dead_code)]
mod shapes {
    use std::borrow::Cow;
    use std::marker::PhantomData;

    use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
    use serde_json::{Map, Value};

    use crate::container::{CAPABILITIES, FORMAT};
    use crate::limits::{
        ID_MAX_LEN, ID_MIN_LEN, NAMEPLATE_APPEAR_SECONDS, NAMEPLATE_APPEAR_SECONDS_MIN,
        NAMEPLATE_BADGE_OFFSET, NAMEPLATE_BAND_WEIGHT, NAMEPLATE_BREAKPOINTS, NAMEPLATE_BURST,
        NAMEPLATE_CORNER_RADIUS, NAMEPLATE_EMITTER_POOL, NAMEPLATE_FADE_STEPS, NAMEPLATE_FLAGS,
        NAMEPLATE_FLAG_BANDS, NAMEPLATE_FPS_MAX, NAMEPLATE_FPS_MIN, NAMEPLATE_LAPS,
        NAMEPLATE_LAYERS, NAMEPLATE_LOOP_FRAMES, NAMEPLATE_MATERIAL_MARGIN,
        NAMEPLATE_MATERIAL_NODES, NAMEPLATE_MATERIAL_OCTAVES, NAMEPLATE_MATERIAL_REPEAT,
        NAMEPLATE_MIN_WIDTH, NAMEPLATE_OFFSET, NAMEPLATE_OVERLAY_COLORS, NAMEPLATE_OVERLAY_WIDTH,
        NAMEPLATE_PADDING, NAMEPLATE_PARAMS, NAMEPLATE_PARTICLE_GRAVITY,
        NAMEPLATE_PARTICLE_LIFETIME, NAMEPLATE_PARTICLE_SPEED, NAMEPLATE_PATH_POINTS,
        NAMEPLATE_PATH_POINTS_MIN, NAMEPLATE_PATH_SPEED, NAMEPLATE_PATH_STOPS,
        NAMEPLATE_PAUSE_FRAMES, NAMEPLATE_REPEAT_COPIES, NAMEPLATE_RINGS,
        NAMEPLATE_SCHEDULE_WEIGHT, NAMEPLATE_SLOTS, NAMEPLATE_SPAWN_COUNT, NAMEPLATE_SPRITE_COLORS,
        NAMEPLATE_SPRITE_FRAMES, NAMEPLATE_SPRITE_SIZE, NAMEPLATE_TONES, NAMEPLATE_WALKERS,
        NAMEPLATE_WOBBLE,
    };
    use crate::nameplate::format::{
        BADGE_TARGET, NAME_TARGET, PALETTE_PREFIX, PLATE_TARGET, TEXT_TARGET,
    };
    use crate::nameplate::geom::{Anchor, Edge};
    use crate::nameplate::material::graph::{
        MAX_CELL_SIZE, MAX_DIRECTION, MAX_LITERAL, MAX_PERIOD,
    };
    use crate::nameplate::material::ir::{OpKind, Reduce, VanillaTexture};
    use crate::nameplate::material::nodes::{sockets, SocketDefault};
    use crate::nameplate::types::Depth;
    use crate::signals::{is_bool_signal, SIGNALS};

    const MAX_WIDTH: u32 = 4096;
    const MAX_SPACING: u32 = 256;
    const LAST_FRAME: u32 = NAMEPLATE_LOOP_FRAMES - 1;
    const LAST_SPRITE_FRAME: u32 = NAMEPLATE_SPRITE_FRAMES - 1;
    const MIN_PATH_SPEED: f64 = 1.0 / 512.0;

    pub fn forbid_null(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(Value::Object(properties)) = map.get_mut("properties") {
                    for property in properties.values_mut() {
                        if let Value::Object(field) = property {
                            strip_null(field);
                        }
                    }
                }
                for child in map.values_mut() {
                    forbid_null(child);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(forbid_null),
            _ => {}
        }
    }

    fn strip_null(field: &mut Map<String, Value>) {
        let null = json_null();
        if let Some(Value::Array(branches)) = field.get("anyOf") {
            if branches.len() == 2 && branches.contains(&null) {
                let kept = branches.iter().find(|b| **b != null).cloned();
                field.remove("anyOf");
                if let Some(Value::Object(kept)) = kept {
                    for (key, value) in kept {
                        field.entry(key).or_insert(value);
                    }
                }
            }
        }
        if let Some(Value::Array(types)) = field.get_mut("type") {
            types.retain(|t| t != "null");
            if types.len() == 1 {
                let only = types.remove(0);
                field.insert("type".into(), only);
            }
        }
        if let Some(Value::Array(options)) = field.get_mut("enum") {
            options.retain(|o| !o.is_null());
        }
    }

    fn json_null() -> Value {
        serde_json::json!({ "type": "null" })
    }

    fn string_pattern(pattern: String) -> Schema {
        json_schema!({ "type": "string", "pattern": pattern })
    }

    fn string_enum(names: Vec<&str>) -> Schema {
        json_schema!({ "type": "string", "enum": names })
    }

    fn id_pattern() -> String {
        format!("[a-z0-9_]{{{ID_MIN_LEN},{ID_MAX_LEN}}}")
    }

    fn hex_pattern() -> &'static str {
        "#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})"
    }

    fn slot_pattern() -> String {
        format!("{}(\\[[0-9]{{1,2}}\\])?", id_pattern())
    }

    macro_rules! string_schema {
        ($name:ident, $schema:expr) => {
            pub struct $name;

            impl JsonSchema for $name {
                fn schema_name() -> Cow<'static, str> {
                    Cow::Borrowed(stringify!($name))
                }

                fn json_schema(_: &mut SchemaGenerator) -> Schema {
                    $schema
                }
            }
        };
    }

    string_schema!(Id, string_pattern(format!("^{}$", id_pattern())));
    string_schema!(Hex, string_pattern(format!("^{}$", hex_pattern())));
    string_schema!(SlotRef, string_pattern(format!("^{}$", slot_pattern())));
    string_schema!(
        AnyColor,
        string_pattern(format!("^({}|{})$", hex_pattern(), slot_pattern()))
    );
    string_schema!(
        AtlasId,
        string_pattern("^[a-z0-9_.-]{1,64}:[a-z0-9_./-]{1,128}$".into())
    );
    string_schema!(
        AnchorName,
        string_enum(Anchor::ALL.iter().map(|a| a.as_str()).collect())
    );
    string_schema!(
        BlockAnchorName,
        string_enum(
            Anchor::ALL
                .iter()
                .filter(|a| !a.is_name())
                .map(|a| a.as_str())
                .collect()
        )
    );
    string_schema!(
        EdgeName,
        string_enum(Edge::ALL.iter().map(|e| e.as_str()).collect())
    );
    string_schema!(
        DepthName,
        string_enum(Depth::ALL.iter().map(|d| d.as_str()).collect())
    );
    string_schema!(
        BoolSignal,
        string_enum(
            SIGNALS
                .iter()
                .enumerate()
                .filter(|(i, _)| is_bool_signal(*i))
                .map(|(_, (name, _))| *name)
                .collect()
        )
    );
    string_schema!(Capability, string_enum(CAPABILITIES.to_vec()));
    string_schema!(
        VanillaTextureName,
        string_enum(VanillaTexture::ALL.iter().map(|t| t.as_str()).collect())
    );
    string_schema!(
        ReduceName,
        string_enum(Reduce::ALL.iter().map(|r| r.as_str()).collect())
    );
    string_schema!(
        NameTarget,
        json_schema!({ "type": "string", "const": NAME_TARGET })
    );
    string_schema!(True, json_schema!({ "type": "boolean", "const": true }));

    pub struct Nullable<T>(PhantomData<T>);

    impl<T: JsonSchema> JsonSchema for Nullable<T> {
        fn inline_schema() -> bool {
            true
        }

        fn schema_name() -> Cow<'static, str> {
            Cow::Owned(format!("Nullable{}", T::schema_name()))
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            let inner = generator.subschema_for::<T>();
            json_schema!({ "oneOf": [inner, json_null()] })
        }
    }

    pub struct IdMap<T>(PhantomData<T>);

    impl<T: JsonSchema> JsonSchema for IdMap<T> {
        fn inline_schema() -> bool {
            true
        }

        fn schema_name() -> Cow<'static, str> {
            Cow::Owned(format!("IdMap{}", T::schema_name()))
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            let names = generator.subschema_for::<Id>();
            let values = generator.subschema_for::<T>();
            json_schema!({
                "type": "object",
                "propertyNames": names,
                "additionalProperties": values
            })
        }
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum NameplateKind {
        Nameplate,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Phase {
        Global,
        PerPlayer,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Manifest {
        #[schemars(range(min = 1, max = FORMAT))]
        format: u64,
        kind: NameplateKind,
        id: Id,
        requires: Option<Vec<Capability>>,
        #[schemars(range(min = NAMEPLATE_FPS_MIN, max = NAMEPLATE_FPS_MAX))]
        fps: u32,
        #[schemars(range(min = 1, max = NAMEPLATE_LOOP_FRAMES))]
        r#loop: u32,
        phase: Option<Phase>,
        owns_background: Option<bool>,
        #[schemars(extend("maxProperties" = NAMEPLATE_PARAMS))]
        params: Option<IdMap<Param>>,
        text: Option<TextStyle>,
        badge: Option<BadgeStyle>,
        #[schemars(length(max = NAMEPLATE_LAYERS))]
        layers: Option<Vec<LayerEntry>>,
        groups: Option<Vec<Group>>,
        animations: Option<Animations>,
        variants: Option<IdMap<Variant>>,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Param {
        Fixed(f64),
        Ranged(ParamRange),
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ParamRange {
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum TextMode {
        Vanilla,
        Motion,
        Owned,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Formatting {
        Adjust,
        Keep,
        Override,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum PaintKeyword {
        None,
        Auto,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Paint {
        Keyword(PaintKeyword),
        Color(AnyColor),
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    pub struct TextStyle {
        mode: Option<TextMode>,
        color: Option<Paint>,
        shadow: Option<Paint>,
        outline: Option<Paint>,
        formatting: Option<Formatting>,
        #[schemars(length(min = 1))]
        candidates: Option<Vec<AnyColor>>,
        backdrop_hint: Option<Vec<AnyColor>>,
        motion: Option<Nullable<String>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Placement {
        Reserve,
        Carried,
        Placed,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum NoTint {
        None,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Tint {
        None(NoTint),
        Color(AnyColor),
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Decoration {
        None,
        Auto,
        Shadow,
        Outline,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum EdgeFollow {
        Outside,
        Line,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Follow {
        Outside,
        Line,
        Inside,
    }

    #[derive(JsonSchema)]
    pub struct BadgeOffset(
        #[schemars(range(min = -(NAMEPLATE_BADGE_OFFSET as i64), max = NAMEPLATE_BADGE_OFFSET))]
        i64,
        #[schemars(range(min = -(NAMEPLATE_BADGE_OFFSET as i64), max = NAMEPLATE_BADGE_OFFSET))]
        i64,
    );

    #[derive(JsonSchema)]
    pub struct Offset(
        #[schemars(range(min = -(NAMEPLATE_OFFSET as i64), max = NAMEPLATE_OFFSET))] i64,
        #[schemars(range(min = -(NAMEPLATE_OFFSET as i64), max = NAMEPLATE_OFFSET))] i64,
    );

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    pub struct BadgeStyle {
        placement: Option<Placement>,
        anchor: Option<AnchorName>,
        offset: Option<BadgeOffset>,
        tint: Option<Tint>,
        decoration: Option<Decoration>,
        decoration_color: Option<AnyColor>,
        #[schemars(length(min = 1))]
        candidates: Option<Vec<AnyColor>>,
        of: Option<Id>,
        follow: Option<EdgeFollow>,
        #[schemars(range(min = 0.0, max = 1.0))]
        at: Option<f64>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Clip {
        Fill,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum LayerOf {
        Name(NameTarget),
        Layer(Id),
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct LayerEntry {
        id: Id,
        depth: Option<DepthName>,
        #[schemars(range(min = 1, max = NAMEPLATE_LOOP_FRAMES))]
        step: Option<u32>,
        mask: Option<LayerOf>,
        clip: Option<Clip>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Breakpoint {
        #[schemars(range(min = 0, max = MAX_WIDTH))]
        min: Option<u32>,
        #[schemars(range(min = 1, max = MAX_WIDTH))]
        max: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct SpriteBreakpoint {
        #[schemars(range(min = 0, max = MAX_WIDTH))]
        min: Option<u32>,
        #[schemars(range(min = 1, max = MAX_WIDTH))]
        max: Option<u32>,
        sprite: Option<Id>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Group {
        id: Id,
        parent: Option<Id>,
        layers: Option<Vec<Id>>,
        #[schemars(length(min = 1, max = NAMEPLATE_BREAKPOINTS))]
        breakpoints: Option<Vec<Breakpoint>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Variant {
        palette: Option<Id>,
        hide: Option<Vec<Id>>,
        swap: Option<IdMap<Id>>,
        flag: Option<Id>,
        owns_background: Option<bool>,
        text: Option<TextStyle>,
        badge: Option<BadgeStyle>,
        params: Option<IdMap<f64>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Easing {
        Step,
        Linear,
        Ease,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum StepEasing {
        Step,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged, rename = "Key_{V}_{E}")]
    pub enum Key<V, E> {
        Plain(#[schemars(range(min = 0, max = LAST_FRAME))] u32, V),
        Eased(#[schemars(range(min = 0, max = LAST_FRAME))] u32, V, E),
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Coordinate {
        Fixed(#[schemars(range(min = -(NAMEPLATE_OFFSET as i64), max = NAMEPLATE_OFFSET))] i64),
        Molang(String),
    }

    #[derive(JsonSchema)]
    pub struct Position(Coordinate, Coordinate);

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Opacity {
        Fixed(#[schemars(range(min = 0.0, max = 1.0))] f64),
        Molang(String),
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Visible {
        Fixed(bool),
        Molang(String),
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum SpriteFrame {
        Fixed(#[schemars(range(min = 0, max = LAST_SPRITE_FRAME))] u32),
        Molang(String),
    }

    #[derive(JsonSchema)]
    pub struct Burst(
        #[schemars(range(min = 0, max = LAST_FRAME))] u32,
        #[schemars(range(min = 1, max = NAMEPLATE_BURST))] u32,
    );

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct PlateChannels {
        #[schemars(length(min = 1))]
        position: Option<Vec<Key<Position, Easing>>>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct BadgeChannels {
        #[schemars(length(min = 1))]
        position: Option<Vec<Key<Offset, Easing>>>,
        #[schemars(length(min = 1))]
        opacity: Option<Vec<Key<Fraction, Easing>>>,
        #[schemars(length(min = 1))]
        visible: Option<Vec<Key<bool, StepEasing>>>,
    }

    #[derive(JsonSchema)]
    pub struct Fraction(#[schemars(range(min = 0.0, max = 1.0))] f64);

    #[derive(JsonSchema)]
    pub struct Speed(
        #[schemars(range(min = -(NAMEPLATE_PATH_SPEED as f64), max = NAMEPLATE_PATH_SPEED))] f64,
    );

    #[derive(JsonSchema)]
    pub struct ForwardSpeed(#[schemars(range(min = 0.0, max = NAMEPLATE_PATH_SPEED))] f64);

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct LayerChannels {
        #[schemars(length(min = 1))]
        position: Option<Vec<Key<Position, Easing>>>,
        #[schemars(length(min = 1))]
        opacity: Option<Vec<Key<Opacity, Easing>>>,
        #[schemars(length(min = 1))]
        visible: Option<Vec<Key<Visible, StepEasing>>>,
        #[schemars(length(min = 1))]
        frame: Option<Vec<Key<SpriteFrame, StepEasing>>>,
        #[schemars(length(min = 1))]
        burst: Option<Vec<Burst>>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct TriggeredChannels {
        #[schemars(length(min = 1))]
        position: Option<Vec<Key<Position, Easing>>>,
        #[schemars(length(min = 1))]
        opacity: Option<Vec<Key<Opacity, Easing>>>,
        #[schemars(length(min = 1))]
        visible: Option<Vec<Key<Visible, StepEasing>>>,
        #[schemars(length(min = 1))]
        frame: Option<Vec<Key<SpriteFrame, StepEasing>>>,
        #[schemars(length(min = 1))]
        speed: Option<Vec<Key<Speed, Easing>>>,
        #[schemars(length(min = 1))]
        burst: Option<Vec<Burst>>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct TextTrack {
        motion: Option<Nullable<String>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum CycleMode {
        Forward,
        Backward,
        PingPong,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Cycle {
        #[schemars(range(min = 1, max = NAMEPLATE_LOOP_FRAMES))]
        every: u32,
        mode: CycleMode,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum PaletteValue {
        One(String),
        Tones(#[schemars(length(min = 1, max = NAMEPLATE_TONES))] Vec<String>),
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct PaletteCycle {
        cycle: Cycle,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct PaletteKeys {
        #[schemars(length(min = 1))]
        keys: Vec<Key<PaletteValue, Easing>>,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum PaletteTrack {
        Cycle(PaletteCycle),
        Keys(PaletteKeys),
    }

    pub trait ClipKind {
        const NAME: &'static str;
        const BADGE: bool;
        const PALETTE: bool;
        const TEXT: bool;
        type Layer: JsonSchema;
    }

    pub struct Looped;
    pub struct Triggered;
    pub struct Scheduled;

    impl ClipKind for Looped {
        const NAME: &'static str = "LoopTracks";
        const BADGE: bool = true;
        const PALETTE: bool = true;
        const TEXT: bool = false;
        type Layer = LayerChannels;
    }

    impl ClipKind for Triggered {
        const NAME: &'static str = "TriggeredTracks";
        const BADGE: bool = true;
        const PALETTE: bool = true;
        const TEXT: bool = true;
        type Layer = TriggeredChannels;
    }

    impl ClipKind for Scheduled {
        const NAME: &'static str = "ScheduledTracks";
        const BADGE: bool = false;
        const PALETTE: bool = false;
        const TEXT: bool = false;
        type Layer = LayerChannels;
    }

    pub struct Tracks<K>(PhantomData<K>);

    impl<K: ClipKind> JsonSchema for Tracks<K> {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed(K::NAME)
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            let mut properties = Map::new();
            let mut names: Vec<Value> = vec![generator.subschema_for::<Id>().to_value()];
            let mut specials: Vec<&str> = vec![PLATE_TARGET];
            properties.insert(
                PLATE_TARGET.into(),
                generator.subschema_for::<PlateChannels>().to_value(),
            );
            if K::BADGE {
                specials.push(BADGE_TARGET);
                properties.insert(
                    BADGE_TARGET.into(),
                    generator.subschema_for::<BadgeChannels>().to_value(),
                );
            }
            if K::TEXT {
                specials.push(TEXT_TARGET);
                properties.insert(
                    TEXT_TARGET.into(),
                    generator.subschema_for::<TextTrack>().to_value(),
                );
            }
            names.push(serde_json::json!({ "enum": specials }));
            let mut schema = json_schema!({
                "type": "object",
                "properties": properties,
                "additionalProperties": generator.subschema_for::<K::Layer>()
            });
            if K::PALETTE {
                let pattern = format!("^{}", PALETTE_PREFIX.replace('.', "\\."));
                names.push(serde_json::json!({
                    "pattern": format!("{pattern}{}$", id_pattern())
                }));
                let mut patterns = Map::new();
                patterns.insert(
                    pattern,
                    generator.subschema_for::<PaletteTrack>().to_value(),
                );
                schema.insert("patternProperties".into(), Value::Object(patterns));
            }
            schema.insert(
                "propertyNames".into(),
                serde_json::json!({ "anyOf": names }),
            );
            schema
        }
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct LoopAnimation {
        tracks: Option<Tracks<Looped>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum WhileOn {
        While,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum EdgeOn {
        Start,
        Stop,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Reduced {
        Hold,
        Drop,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct WhileTrigger {
        reaction: BoolSignal,
        on: WhileOn,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct EdgeTrigger {
        reaction: BoolSignal,
        on: EdgeOn,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct AppearTrigger {
        #[schemars(range(min = NAMEPLATE_APPEAR_SECONDS_MIN, max = NAMEPLATE_APPEAR_SECONDS))]
        appear: u32,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum NamedTrigger {
        Equip,
        Appear,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum OneShotTrigger {
        Named(NamedTrigger),
        Reaction(EdgeTrigger),
        Appear(AppearTrigger),
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct WhileAnimation {
        trigger: WhileTrigger,
        tracks: Option<Tracks<Triggered>>,
        reduced: Option<Reduced>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct OneShotAnimation {
        trigger: OneShotTrigger,
        #[schemars(range(min = 1, max = NAMEPLATE_LOOP_FRAMES))]
        length: u32,
        tracks: Option<Tracks<Triggered>>,
        restart: Option<bool>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Schedule {
        #[schemars(range(min = 1, max = u16::MAX))]
        window: u32,
        #[schemars(range(min = 0, max = u16::MAX))]
        jitter: Option<u32>,
        #[schemars(range(min = 0.0, max = 1.0))]
        chance: Option<f64>,
        id: Option<Id>,
        #[schemars(range(min = 1, max = NAMEPLATE_SCHEDULE_WEIGHT))]
        weight: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ScheduledAnimation {
        schedule: Schedule,
        #[schemars(range(min = 1, max = NAMEPLATE_LOOP_FRAMES))]
        length: u32,
        tracks: Option<Tracks<Scheduled>>,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Animation {
        While(WhileAnimation),
        OneShot(OneShotAnimation),
        Scheduled(ScheduledAnimation),
    }

    pub struct Animations;

    impl JsonSchema for Animations {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed("Animations")
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            json_schema!({
                "type": "object",
                "propertyNames": generator.subschema_for::<Id>(),
                "properties": { "loop": generator.subschema_for::<LoopAnimation>() },
                "additionalProperties": generator.subschema_for::<Animation>()
            })
        }
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Slot {
        id: Id,
        name: Option<String>,
        #[schemars(length(min = 1, max = NAMEPLATE_TONES))]
        ramp: Vec<Hex>,
        #[schemars(range(min = 2, max = NAMEPLATE_TONES))]
        interpolate: Option<u32>,
        customizable: Option<bool>,
    }

    pub struct Ramp;

    impl JsonSchema for Ramp {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed("Ramp")
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            json_schema!({
                "type": "array",
                "items": generator.subschema_for::<Hex>(),
                "minItems": 1,
                "maxItems": NAMEPLATE_TONES
            })
        }
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct PaletteFile {
        #[schemars(length(min = 1, max = NAMEPLATE_SLOTS))]
        slots: Vec<Slot>,
        colorways: Option<IdMap<IdMap<Ramp>>>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Band {
        color: SlotRef,
        #[schemars(range(min = 1, max = NAMEPLATE_BAND_WEIGHT))]
        weight: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum ChevronKind {
        Chevron,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum TriangleKind {
        Triangle,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum CircleKind {
        Circle,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ChevronOverlay {
        kind: ChevronKind,
        #[schemars(length(min = 1, max = NAMEPLATE_OVERLAY_COLORS))]
        colors: Vec<SlotRef>,
        #[schemars(range(min = 1, max = NAMEPLATE_OVERLAY_WIDTH))]
        width: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct TriangleOverlay {
        kind: TriangleKind,
        #[schemars(length(min = 1, max = 1))]
        colors: Vec<SlotRef>,
        #[schemars(range(min = 1, max = NAMEPLATE_OVERLAY_WIDTH))]
        width: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct CircleOverlay {
        kind: CircleKind,
        #[schemars(length(min = 1, max = 2))]
        colors: Vec<SlotRef>,
        #[schemars(range(min = 1, max = NAMEPLATE_OVERLAY_WIDTH))]
        width: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Overlay {
        Chevron(ChevronOverlay),
        Triangle(TriangleOverlay),
        Circle(CircleOverlay),
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum FlagAxis {
        Rows,
        Columns,
    }

    pub struct RowsOverride;

    impl JsonSchema for RowsOverride {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed("RowsOverride")
        }

        fn json_schema(_: &mut SchemaGenerator) -> Schema {
            json_schema!({
                "type": "object",
                "propertyNames": { "type": "string", "pattern": "^[0-9]+$" },
                "additionalProperties": {
                    "type": "array",
                    "items": { "type": "integer", "minimum": 1, "maximum": u16::MAX }
                }
            })
        }
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Flag {
        id: Id,
        #[schemars(length(min = 1, max = NAMEPLATE_FLAG_BANDS))]
        bands: Vec<Band>,
        overlay: Option<Overlay>,
        rows_override: Option<RowsOverride>,
        symmetric: Option<bool>,
        axis: Option<FlagAxis>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct FlagsFile {
        #[schemars(length(min = 1, max = NAMEPLATE_FLAGS))]
        flags: Vec<Flag>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum BodyKind {
        Body,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum SpriteKind {
        Sprite,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum EmitterKind {
        Emitter,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum MaterialKind {
        Material,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum SweepAxis {
        X,
        Y,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ColorFill {
        color: SlotRef,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct GradientFill {
        gradient: Id,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct BlendFill {
        blend: Id,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct SweepFill {
        sweep: Id,
        axis: SweepAxis,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct FlagFill {
        flag: True,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Fill {
        Color(ColorFill),
        Gradient(GradientFill),
        Blend(BlendFill),
        Sweep(SweepFill),
        Flag(FlagFill),
    }

    #[derive(JsonSchema)]
    pub struct Columns(
        #[schemars(range(min = 0, max = NAMEPLATE_SPRITE_SIZE))] u32,
        #[schemars(range(min = 0, max = NAMEPLATE_SPRITE_SIZE))] u32,
    );

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct BodyArt {
        sprite: Id,
        #[schemars(range(min = 0, max = NAMEPLATE_SPRITE_SIZE))]
        left: Option<u32>,
        #[schemars(range(min = 0, max = NAMEPLATE_SPRITE_SIZE))]
        right: Option<u32>,
        mirror: Option<bool>,
        segment: Option<Columns>,
        center: Option<Columns>,
        #[schemars(range(min = 0, max = NAMEPLATE_SPRITE_SIZE))]
        top: Option<u32>,
        #[schemars(range(min = 0, max = NAMEPLATE_SPRITE_SIZE))]
        bottom: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    #[schemars(extend("not" = { "required": ["radius", "corners"] }))]
    pub struct BodyLayer {
        kind: BodyKind,
        of: Option<NameTarget>,
        #[schemars(inner(range(min = 0, max = NAMEPLATE_PADDING)))]
        padding: Option<[u32; 4]>,
        #[schemars(range(min = 0, max = NAMEPLATE_MIN_WIDTH))]
        min_width: Option<u32>,
        #[schemars(range(min = 0, max = NAMEPLATE_CORNER_RADIUS))]
        radius: Option<u32>,
        #[schemars(inner(range(min = 0, max = NAMEPLATE_CORNER_RADIUS)))]
        corners: Option<[u32; 4]>,
        #[schemars(length(max = NAMEPLATE_RINGS))]
        rings: Option<Vec<Fill>>,
        fill: Option<Fill>,
        art: Option<BodyArt>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Hold {
        #[schemars(range(min = 0, max = LAST_SPRITE_FRAME))]
        frame: u32,
        #[schemars(range(min = 1, max = NAMEPLATE_LOOP_FRAMES))]
        hold: u32,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Preset {
        Top,
        Bottom,
        Perimeter,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct PathPoint {
        anchor: BlockAnchorName,
        offset: Option<Offset>,
        #[schemars(range(min = 0, max = NAMEPLATE_PAUSE_FRAMES))]
        pause: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum PathSpeed {
        Fixed(#[schemars(range(min = MIN_PATH_SPEED, max = NAMEPLATE_PATH_SPEED))] f64),
        Keys(#[schemars(length(min = 1))] Vec<Key<ForwardSpeed, Easing>>),
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum PathEnd {
        Wrap,
        Bounce,
        Stop,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct PathStop {
        #[schemars(range(min = 0.0, max = 1.0))]
        at: f64,
        #[schemars(range(min = 1, max = NAMEPLATE_PAUSE_FRAMES))]
        pause: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    #[schemars(extend("anyOf" = [{ "required": ["left"] }, { "required": ["right"] }]))]
    pub struct Facings {
        #[schemars(length(min = 1, max = NAMEPLATE_SPRITE_FRAMES))]
        #[schemars(inner(range(min = 0, max = LAST_SPRITE_FRAME)))]
        right: Option<Vec<u32>>,
        #[schemars(length(min = 1, max = NAMEPLATE_SPRITE_FRAMES))]
        #[schemars(inner(range(min = 0, max = LAST_SPRITE_FRAME)))]
        left: Option<Vec<u32>>,
        #[schemars(length(min = 1, max = NAMEPLATE_SPRITE_FRAMES))]
        #[schemars(inner(range(min = 0, max = LAST_SPRITE_FRAME)))]
        up: Option<Vec<u32>>,
        #[schemars(length(min = 1, max = NAMEPLATE_SPRITE_FRAMES))]
        #[schemars(inner(range(min = 0, max = LAST_SPRITE_FRAME)))]
        down: Option<Vec<u32>>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Walkers {
        #[schemars(range(min = 1, max = NAMEPLATE_WALKERS))]
        count: Option<u32>,
        #[schemars(range(min = 0, max = MAX_SPACING))]
        spacing: Option<u32>,
        even: Option<bool>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    #[schemars(extend("oneOf" = [{ "required": ["preset"] }, { "required": ["points"] }]))]
    #[schemars(extend("not" = { "required": ["speed", "laps"] }))]
    pub struct Path {
        preset: Option<Preset>,
        #[schemars(length(min = NAMEPLATE_PATH_POINTS_MIN, max = NAMEPLATE_PATH_POINTS))]
        points: Option<Vec<PathPoint>>,
        follow: Option<Follow>,
        speed: Option<PathSpeed>,
        #[schemars(range(min = 1, max = NAMEPLATE_LAPS))]
        laps: Option<u32>,
        end: Option<PathEnd>,
        reverse: Option<bool>,
        #[schemars(length(min = 1, max = NAMEPLATE_PATH_STOPS))]
        stops: Option<Vec<PathStop>>,
        frames: Option<Facings>,
        walkers: Option<Walkers>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Repeat {
        edge: EdgeName,
        #[schemars(range(min = 1, max = MAX_SPACING))]
        spacing: u32,
        #[schemars(range(min = 1, max = NAMEPLATE_REPEAT_COPIES))]
        min: Option<u32>,
        #[schemars(range(min = 1, max = NAMEPLATE_REPEAT_COPIES))]
        max: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    #[schemars(extend("not" = { "required": ["path", "repeat"] }))]
    pub struct SpriteLayer {
        kind: SpriteKind,
        sprite: Id,
        of: Option<LayerOf>,
        anchor: Option<AnchorName>,
        #[schemars(range(min = 0.0, max = 1.0))]
        at: Option<f64>,
        pivot: Option<BlockAnchorName>,
        offset: Option<Offset>,
        mirror: Option<bool>,
        #[schemars(range(min = 0, max = NAMEPLATE_LOOP_FRAMES))]
        mirror_delay: Option<u32>,
        mirror_seed: Option<bool>,
        timeline: Option<Vec<Hold>>,
        follow: Option<EdgeFollow>,
        path: Option<Path>,
        repeat: Option<Repeat>,
        #[schemars(length(min = 1, max = NAMEPLATE_BREAKPOINTS))]
        breakpoints: Option<Vec<SpriteBreakpoint>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum SpawnShape {
        Point,
        Line,
        Area,
        Border,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Spawn {
        shape: Option<SpawnShape>,
        anchor: Option<BlockAnchorName>,
        edge: Option<EdgeName>,
        offset: Option<Offset>,
        follow: Option<Follow>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Rate {
        #[schemars(range(min = 1, max = NAMEPLATE_LOOP_FRAMES))]
        every: u32,
        #[schemars(range(min = 1, max = NAMEPLATE_SPAWN_COUNT))]
        count: Option<u32>,
        #[schemars(range(min = 0, max = LAST_FRAME))]
        jitter: Option<u32>,
    }

    #[derive(JsonSchema)]
    pub struct Lifetime(
        #[schemars(range(min = 1, max = NAMEPLATE_PARTICLE_LIFETIME))] u32,
        #[schemars(range(min = 1, max = NAMEPLATE_PARTICLE_LIFETIME))] u32,
    );

    #[derive(JsonSchema)]
    pub struct SpeedRange(
        #[schemars(range(min = -(NAMEPLATE_PARTICLE_SPEED as f64), max = NAMEPLATE_PARTICLE_SPEED))]
         f64,
        #[schemars(range(min = -(NAMEPLATE_PARTICLE_SPEED as f64), max = NAMEPLATE_PARTICLE_SPEED))]
         f64,
    );

    #[derive(JsonSchema)]
    pub struct GravityRange(
        #[schemars(range(min = -(NAMEPLATE_PARTICLE_GRAVITY as f64), max = NAMEPLATE_PARTICLE_GRAVITY))]
         f64,
        #[schemars(range(min = -(NAMEPLATE_PARTICLE_GRAVITY as f64), max = NAMEPLATE_PARTICLE_GRAVITY))]
         f64,
    );

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Velocity {
        x: Option<SpeedRange>,
        y: Option<SpeedRange>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Wobble {
        #[schemars(range(min = 0, max = NAMEPLATE_WOBBLE))]
        x: Option<u32>,
        #[schemars(range(min = 0, max = NAMEPLATE_WOBBLE))]
        y: Option<u32>,
        #[schemars(range(min = 1, max = NAMEPLATE_PARTICLE_LIFETIME))]
        period: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum ParticleFrames {
        Loop,
        Life,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct Fade {
        #[schemars(range(min = 0.0, max = 1.0))]
        #[schemars(extend("exclusiveMaximum" = 1.0))]
        after: Option<f64>,
        #[schemars(range(min = 0.0, max = 1.0))]
        to: Option<f64>,
        #[schemars(range(min = 1, max = NAMEPLATE_FADE_STEPS))]
        steps: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct EmitterLayer {
        kind: EmitterKind,
        #[schemars(length(min = 1, max = NAMEPLATE_EMITTER_POOL))]
        sprites: Vec<Id>,
        of: Option<Id>,
        spawn: Option<Spawn>,
        rate: Option<Rate>,
        lifetime: Option<Lifetime>,
        velocity: Option<Velocity>,
        gravity: Option<GravityRange>,
        drift: Option<SpeedRange>,
        wobble: Option<Wobble>,
        frames: Option<ParticleFrames>,
        fade: Option<Fade>,
        #[schemars(length(min = 1, max = NAMEPLATE_BREAKPOINTS))]
        breakpoints: Option<Vec<Breakpoint>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    pub struct MaterialLayer {
        kind: MaterialKind,
        material: Id,
        of: Option<LayerOf>,
        drift: Option<bool>,
        mirror: Option<bool>,
        #[schemars(range(min = 0, max = NAMEPLATE_LOOP_FRAMES))]
        mirror_delay: Option<u32>,
        mirror_seed: Option<bool>,
    }

    pub struct LayerFile;

    impl JsonSchema for LayerFile {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed("LayerFile")
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            let mut kinds: Vec<Value> = [
                BodyLayer::json_schema(generator),
                SpriteLayer::json_schema(generator),
                EmitterLayer::json_schema(generator),
                MaterialLayer::json_schema(generator),
            ]
            .into_iter()
            .map(Schema::to_value)
            .collect();
            kinds.iter_mut().for_each(forbid_null);
            let mut properties: Map<String, Value> = Map::new();
            for kind in &kinds {
                let Some(Value::Object(fields)) = kind.get("properties") else {
                    continue;
                };
                for (key, field) in fields {
                    match properties.get_mut(key) {
                        None => {
                            properties.insert(key.clone(), serde_json::json!({ "anyOf": [field] }));
                        }
                        Some(Value::Object(known)) => {
                            if let Some(Value::Array(options)) = known.get_mut("anyOf") {
                                if !options.contains(field) {
                                    options.push(field.clone());
                                }
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
            for property in properties.values_mut() {
                let single = match property.get("anyOf") {
                    Some(Value::Array(options)) if options.len() == 1 => Some(options[0].clone()),
                    _ => None,
                };
                if let Some(single) = single {
                    *property = single;
                }
            }
            json_schema!({
                "type": "object",
                "properties": properties,
                "required": ["kind"],
                "additionalProperties": false,
                "oneOf": kinds
            })
        }
    }

    pub struct SpriteColors;

    impl JsonSchema for SpriteColors {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed("SpriteColors")
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            json_schema!({
                "type": "object",
                "propertyNames": { "type": "string", "pattern": "^[^.\\s]$" },
                "maxProperties": NAMEPLATE_SPRITE_COLORS,
                "additionalProperties": generator.subschema_for::<SlotRef>()
            })
        }
    }

    pub struct Rows;

    impl JsonSchema for Rows {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed("Rows")
        }

        fn json_schema(_: &mut SchemaGenerator) -> Schema {
            json_schema!({
                "type": "array",
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": NAMEPLATE_SPRITE_SIZE
                },
                "minItems": 1,
                "maxItems": NAMEPLATE_SPRITE_SIZE
            })
        }
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    #[schemars(extend("oneOf" = [{ "required": ["rows"] }, { "required": ["frames"] }]))]
    pub struct SpriteFile {
        colors: SpriteColors,
        rows: Option<Rows>,
        #[schemars(length(min = 1, max = NAMEPLATE_SPRITE_FRAMES))]
        frames: Option<Vec<Rows>>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Resolution {
        Pixel,
        Smooth,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum Literal {
        Scalar(#[schemars(range(min = -MAX_LITERAL, max = MAX_LITERAL))] f64),
        Vector(
            #[schemars(length(min = 1, max = 4))]
            #[schemars(inner(range(min = -MAX_LITERAL, max = MAX_LITERAL)))]
            Vec<f64>,
        ),
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    pub enum NodeInput {
        Node(Id),
        Literal(Literal),
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct SlotSettings {
        slot: SlotRef,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ConstantSettings {
        value: Literal,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ParamSettings {
        name: Id,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ExprSettings {
        expr: String,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum BodyEdgeName {
        Fill,
        Outer,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct BodyEdgeSettings {
        edge: Option<BodyEdgeName>,
    }

    #[derive(JsonSchema)]
    pub struct Period(
        #[schemars(range(min = 0, max = MAX_PERIOD))] i64,
        #[schemars(range(min = 0, max = MAX_PERIOD))] i64,
    );

    #[derive(JsonSchema)]
    pub struct Direction(
        #[schemars(range(min = -MAX_DIRECTION, max = MAX_DIRECTION))] i64,
        #[schemars(range(min = -MAX_DIRECTION, max = MAX_DIRECTION))] i64,
    );

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct NoiseSettings {
        #[schemars(range(min = 0, max = i32::MAX))]
        seed: Option<i64>,
        period: Option<Period>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct FbmSettings {
        #[schemars(range(min = 0, max = i32::MAX))]
        seed: Option<i64>,
        period: Option<Period>,
        #[schemars(range(min = 1, max = NAMEPLATE_MATERIAL_OCTAVES))]
        octaves: Option<u32>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct HashSettings {
        #[schemars(range(min = 0, max = i32::MAX))]
        seed: Option<i64>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct CheckerSettings {
        #[schemars(range(min = 1, max = MAX_CELL_SIZE))]
        size: Option<i64>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct StripesSettings {
        #[schemars(range(min = 1, max = MAX_CELL_SIZE))]
        period: Option<i64>,
        #[schemars(range(min = 1, max = MAX_CELL_SIZE))]
        width: Option<i64>,
        direction: Option<Direction>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum ComponentName {
        X,
        Y,
        Z,
        W,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct ComponentSettings {
        component: Option<ComponentName>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct SampleAtSettings {
        source: Id,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct RepeatSettings {
        source: Id,
        #[schemars(range(min = 1, max = NAMEPLATE_MATERIAL_REPEAT))]
        count: u32,
        reduce: Option<ReduceName>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct SpriteSettings {
        sprite: Id,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct BundleTextureSettings {
        texture: Id,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct VanillaTextureSettings {
        texture: VanillaTextureName,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct AtlasSpriteSettings {
        sprite: AtlasId,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Blend {
        Alpha,
        Additive,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    pub struct OutputSettings {
        blend: Option<Blend>,
    }

    fn settings(kind: Option<OpKind>, generator: &mut SchemaGenerator) -> (Schema, bool) {
        let Some(kind) = kind else {
            return (generator.subschema_for::<OutputSettings>(), false);
        };
        match kind {
            OpKind::Slot | OpKind::Quantize | OpKind::DitherQuantize | OpKind::Cycle => {
                (generator.subschema_for::<SlotSettings>(), true)
            }
            OpKind::Constant => (generator.subschema_for::<ConstantSettings>(), true),
            OpKind::Param => (generator.subschema_for::<ParamSettings>(), true),
            OpKind::Expr => (generator.subschema_for::<ExprSettings>(), true),
            OpKind::BodyEdge => (generator.subschema_for::<BodyEdgeSettings>(), false),
            OpKind::Noise => (generator.subschema_for::<NoiseSettings>(), false),
            OpKind::Fbm => (generator.subschema_for::<FbmSettings>(), false),
            OpKind::Hash => (generator.subschema_for::<HashSettings>(), false),
            OpKind::Checker => (generator.subschema_for::<CheckerSettings>(), false),
            OpKind::Stripes => (generator.subschema_for::<StripesSettings>(), false),
            OpKind::Component => (generator.subschema_for::<ComponentSettings>(), false),
            OpKind::SampleAt => (generator.subschema_for::<SampleAtSettings>(), true),
            OpKind::Repeat => (generator.subschema_for::<RepeatSettings>(), true),
            OpKind::Sprite => (generator.subschema_for::<SpriteSettings>(), true),
            OpKind::BundleTexture => (generator.subschema_for::<BundleTextureSettings>(), true),
            OpKind::VanillaTexture => (generator.subschema_for::<VanillaTextureSettings>(), true),
            OpKind::AtlasSprite => (generator.subschema_for::<AtlasSpriteSettings>(), true),
            _ => (
                json_schema!({ "type": "object", "additionalProperties": false }),
                false,
            ),
        }
    }

    fn node_schema(
        name: &str,
        inputs: &[(&str, bool)],
        set: (Schema, bool),
        generator: &mut SchemaGenerator,
    ) -> Value {
        let input = generator.subschema_for::<NodeInput>().to_value();
        let mut sockets = Map::new();
        let mut needed: Vec<&str> = Vec::new();
        for (socket, required) in inputs {
            sockets.insert((*socket).into(), input.clone());
            if *required {
                needed.push(socket);
            }
        }
        let mut required = vec!["id", "type"];
        if !needed.is_empty() {
            required.push("in");
        }
        if set.1 {
            required.push("set");
        }
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": generator.subschema_for::<Id>(),
                "type": { "type": "string", "const": name },
                "in": {
                    "type": "object",
                    "properties": sockets,
                    "required": needed,
                    "additionalProperties": false
                },
                "set": set.0
            },
            "required": required,
            "additionalProperties": false
        })
    }

    pub struct MaterialNode;

    impl JsonSchema for MaterialNode {
        fn schema_name() -> Cow<'static, str> {
            Cow::Borrowed("MaterialNode")
        }

        fn json_schema(generator: &mut SchemaGenerator) -> Schema {
            let output = settings(None, generator);
            let mut kinds = vec![node_schema(
                "Output",
                &[("color", true), ("coverage", false)],
                output,
                generator,
            )];
            for kind in OpKind::ALL {
                let inputs: Vec<(&str, bool)> = sockets(kind)
                    .iter()
                    .map(|s| (s.name, s.default == SocketDefault::Required))
                    .collect();
                let set = settings(Some(kind), generator);
                kinds.push(node_schema(kind.name(), &inputs, set, generator));
            }
            json_schema!({ "oneOf": kinds })
        }
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "camelCase", deny_unknown_fields)]
    pub struct MaterialFile {
        resolution: Option<Resolution>,
        #[schemars(inner(range(min = 0, max = NAMEPLATE_MATERIAL_MARGIN)))]
        margins: Option<[u32; 4]>,
        outline: Option<Nullable<SlotRef>>,
        emissive: Option<bool>,
        tone_hint: Option<Vec<AnyColor>>,
        #[schemars(length(max = NAMEPLATE_MATERIAL_NODES))]
        nodes: Option<Vec<MaterialNode>>,
    }
}

#[cfg(test)]
#[cfg(feature = "schema")]
mod tests {
    use super::*;

    #[test]
    fn every_bundle_file_kind_has_a_closed_object_schema() {
        let names: Vec<&str> = schemas().iter().map(|(n, _)| *n).collect();
        let kinds = "manifest.json palettes.json flags.json layers/<id>.json sprites/<id>.json materials/<id>.json";
        assert_eq!(names.join(" "), kinds);
        for (name, schema) in schemas() {
            let closed =
                schema["additionalProperties"] == false && schema["properties"].is_object();
            assert!(closed, "{name}");
        }
    }
}
