use std::collections::BTreeSet;
use std::ops::Range;

use crate::nameplate::image::{ColorRef, RefKind};
use crate::nameplate::types::Sprite;

use super::ir::{
    Blend, Dialect, MaterialIr, Op, OpKind, Period, Reduce, Reg, Resolution, Sampler, ShaderSource,
    TexRef, Ty, COVERAGE_THRESHOLD, LANE_COUNT, MAX_SAMPLES, TONES_PER_SLOT, UNIFORM_AREA,
    UNIFORM_ATLAS, UNIFORM_BODY, UNIFORM_CLOCK, UNIFORM_LANES, UNIFORM_TONES, WORDS_PER_TONE,
};

pub const UNIFORM_BLOCK: &str = "NameplateMaterial";
pub const AREA_UNIFORM: &str = "NameplateArea";
pub const BODY_UNIFORM: &str = "NameplateBody";
pub const CLOCK_UNIFORM: &str = "NameplateClock";
pub const LANES_UNIFORM: &str = "NameplateLanes";
pub const ATLAS_UNIFORM: &str = "NameplateAtlas";
pub const TONES_UNIFORM: &str = "NameplateTones";
pub const TONE_UNIFORM: &str = "NameplateTone";
pub const SCREEN_FIRST_DEFINE: &str = "NAMEPLATE_SCREEN_FIRST";
pub const COMPAT_LIGHTMAP_UNIT: u8 = 1;
pub const LIGHTMAP_UNIT: u8 = 2;
pub const SKSL_TEXTURE: &str = "NameplateTexture";
pub const SKSL_COLOR_UNIFORM: &str = "NameplateColor";
pub const SKSL_SCREEN_UNIFORM: &str = "NameplateScreen";
pub const SKSL_SPRITE_RUNS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LooseUniform {
    pub name: String,
    pub ints: bool,
    pub word: u32,
    pub count: u32,
}

pub fn generate(ir: &MaterialIr, dialect: Dialect) -> ShaderSource {
    let vertex = vertex(dialect);
    let fragment = Gen::new(ir, dialect).fragment();
    let key = format!(
        "{}_{:016x}",
        dialect.as_str(),
        fnv1a(&[vertex.as_bytes(), b"\n", fragment.as_bytes()])
    );
    ShaderSource {
        key,
        material: 0,
        vertex,
        fragment,
        samplers: samplers(ir, dialect),
        uniform_words: ir.uniform_words(),
        tones: ir.tone_slots.clone(),
        blend: ir.blend,
        depth_prepass: dialect != Dialect::Sksl && opaque(ir),
    }
}

pub fn unsupported(ir: &MaterialIr, dialect: Dialect) -> Vec<OpKind> {
    if dialect != Dialect::Sksl {
        return Vec::new();
    }
    OpKind::ALL
        .into_iter()
        .filter(|kind| {
            ir.ops.iter().any(|op| {
                op.kind == *kind
                    && match op.kind {
                        OpKind::Hash | OpKind::Noise | OpKind::Fbm => true,
                        OpKind::Sprite => ir
                            .sprites
                            .get(int(op.imm[0]).max(0) as usize)
                            .is_some_and(|sprite| sprite_runs(sprite).len() > SKSL_SPRITE_RUNS),
                        _ => false,
                    }
            })
        })
        .collect()
}

fn sprite_runs(sprite: &Sprite) -> Vec<(usize, Option<ColorRef>)> {
    let (w, h) = (usize::from(sprite.width), usize::from(sprite.height));
    let mut runs: Vec<(usize, Option<ColorRef>)> = Vec::new();
    let mut i = 0;
    for f in 0..sprite.frames.len() {
        for y in 0..h {
            for x in 0..w {
                let cell = sprite.pixel(f, x, y);
                i += 1;
                match runs.last_mut() {
                    Some((end, value)) if *value == cell => *end = i,
                    _ => runs.push((i, cell)),
                }
            }
        }
    }
    runs
}

pub fn reads_screen(ir: &MaterialIr) -> bool {
    ir.ops
        .iter()
        .any(|op| matches!(op.kind, OpKind::ScreenPos | OpKind::PixelScreenPos))
}

pub fn sampler_name(dialect: Dialect, index: usize) -> String {
    match dialect {
        Dialect::Core330Ubo | Dialect::Legacy150 => {
            format!("Sampler{}", sampler_unit(dialect, index))
        }
        Dialect::Sksl => format!("{SKSL_TEXTURE}{index}"),
        _ => format!("NameplateSampler{index}"),
    }
}

pub fn sampler_unit(dialect: Dialect, index: usize) -> u8 {
    let index = index.min(u8::MAX as usize - 1) as u8;
    let skip = match dialect {
        Dialect::Core330Ubo | Dialect::Legacy150 => LIGHTMAP_UNIT,
        Dialect::Compat130 => COMPAT_LIGHTMAP_UNIT,
        Dialect::Core330 | Dialect::Es300 | Dialect::Sksl => return index,
    };
    if index < skip {
        index
    } else {
        index + 1
    }
}

pub fn samplers(ir: &MaterialIr, dialect: Dialect) -> Vec<Sampler> {
    ir.textures
        .iter()
        .take(MAX_SAMPLES)
        .enumerate()
        .map(|(k, texture)| Sampler {
            name: sampler_name(dialect, k),
            unit: sampler_unit(dialect, k),
            texture: texture.clone(),
        })
        .collect()
}

pub fn loose_uniforms(ir: &MaterialIr, dialect: Dialect) -> Vec<LooseUniform> {
    let tones = tone_count(ir) as u32;
    let lane_vectors = LANE_COUNT as u32 / 4;
    let one = |name: &str, ints: bool, word: u32, count: u32| LooseUniform {
        name: name.to_string(),
        ints,
        word,
        count,
    };
    let mut out = match dialect {
        Dialect::Compat130 | Dialect::Legacy150 => vec![
            one(AREA_UNIFORM, false, UNIFORM_AREA, 1),
            one(BODY_UNIFORM, false, UNIFORM_BODY, 1),
            one(CLOCK_UNIFORM, true, UNIFORM_CLOCK, 1),
        ],
        _ => return Vec::new(),
    };
    if dialect == Dialect::Compat130 {
        out.push(one(LANES_UNIFORM, false, UNIFORM_LANES, lane_vectors));
        out.push(one(ATLAS_UNIFORM, false, UNIFORM_ATLAS, MAX_SAMPLES as u32));
        if tones > 0 {
            out.push(one(TONES_UNIFORM, false, UNIFORM_TONES, tones));
        }
        return out;
    }
    for v in 0..lane_vectors {
        out.push(one(
            &format!("{LANES_UNIFORM}{v}"),
            false,
            UNIFORM_LANES + v * 4,
            1,
        ));
    }
    for k in 0..MAX_SAMPLES as u32 {
        out.push(one(
            &format!("{ATLAS_UNIFORM}{k}"),
            false,
            UNIFORM_ATLAS + k * 4,
            1,
        ));
    }
    for t in 0..tones {
        out.push(one(
            &format!("{TONE_UNIFORM}{t}"),
            false,
            UNIFORM_TONES + t * WORDS_PER_TONE,
            1,
        ));
    }
    out
}

pub fn vertex(dialect: Dialect) -> String {
    match dialect {
        Dialect::Core330 => CORE_VERTEX,
        Dialect::Core330Ubo => UBO_VERTEX,
        Dialect::Legacy150 => LEGACY_VERTEX,
        Dialect::Compat130 => COMPAT_VERTEX,
        Dialect::Es300 => ES_VERTEX,
        Dialect::Sksl => "",
    }
    .to_string()
}

pub fn opaque(ir: &MaterialIr) -> bool {
    if ir.blend != Blend::Alpha || ir.coverage.is_some() || ir.color.is_none() {
        return false;
    }
    let mut memo = vec![None; ir.ops.len()];
    Opacity {
        ir,
        memo: &mut memo,
    }
    .reg(ir.color, 0)
}

fn tone_count(ir: &MaterialIr) -> usize {
    ir.tone_slots.len() * TONES_PER_SLOT as usize
}

fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for part in parts {
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

fn float(value: f64) -> String {
    let value = value as f32;
    if value.is_finite() {
        format!("{value:?}")
    } else {
        "0.0".to_string()
    }
}

fn int(value: f64) -> i32 {
    if value.is_finite() {
        value as i32
    } else {
        0
    }
}

fn zero(ty: Ty) -> String {
    match ty {
        Ty::Float => "0.0".to_string(),
        Ty::Slot => "-1".to_string(),
        _ => format!("{}(0.0)", ty.glsl()),
    }
}

fn numeric(ty: Ty) -> Ty {
    if ty == Ty::Slot {
        Ty::Vec4
    } else {
        ty
    }
}

const SWIZZLE: [&str; 4] = ["x", "y", "z", "w"];

fn convert(expr: &str, from: Ty, to: Ty) -> String {
    if from == to {
        return expr.to_string();
    }
    if from == Ty::Slot {
        return convert(&format!("nTone({expr})"), Ty::Vec4, to);
    }
    if to == Ty::Slot {
        return if from == Ty::Float {
            format!("int({expr})")
        } else {
            format!("int({expr}.x)")
        };
    }
    let (have, want) = (from.width(), to.width());
    if have == 1 {
        return format!("{}({expr})", to.glsl());
    }
    if want < have {
        return format!("{expr}.{}", SWIZZLE[..want as usize].concat());
    }
    let pad = vec!["0.0"; usize::from(want - have)].join(", ");
    format!("{}({expr}, {pad})", to.glsl())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Helper {
    Wrap,
    Div,
    Mod,
    Pow,
    Atan2,
    Clamp,
    Mix,
    Smooth,
    Inverse,
    Hash,
    Noise,
    Fbm,
    Bayer,
    Checker,
    Stripes,
    Gradient,
    BodyEdge,
    Tone,
    Quantize,
    Dither,
    Cycle,
    Hsv,
    Lighten,
    Darken,
    Alpha,
    Polar,
    Sample,
    Atlas,
    TagUv,
    Seconds,
    Screen,
}

impl Helper {
    fn deps(self) -> &'static [Helper] {
        match self {
            Helper::Smooth => &[Helper::Div, Helper::Clamp],
            Helper::Noise => &[Helper::Wrap, Helper::Hash, Helper::Mix],
            Helper::Fbm => &[Helper::Noise],
            Helper::Bayer | Helper::Checker | Helper::Stripes | Helper::Cycle | Helper::Hsv => {
                &[Helper::Wrap]
            }
            Helper::Gradient => &[Helper::Div, Helper::Clamp],
            Helper::Dither => &[Helper::Bayer],
            Helper::Polar => &[Helper::Atan2],
            Helper::Sample | Helper::Atlas => &[Helper::Wrap],
            Helper::Seconds => &[Helper::Div],
            Helper::Screen => &[Helper::Inverse],
            _ => &[],
        }
    }

    fn vector(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Helper::Div => Some(("nDiv", &["a", "b"])),
            Helper::Mod => Some(("nMod", &["a", "b"])),
            Helper::Pow => Some(("nPow", &["a", "b"])),
            Helper::Atan2 => Some(("nAtan2", &["y", "x"])),
            Helper::Smooth => Some(("nSmooth", &["e0", "e1", "x"])),
            _ => None,
        }
    }

    fn code(self, out: &mut String, tones: usize, dialect: Dialect) {
        let sksl = dialect == Dialect::Sksl;
        let text = match self {
            Helper::Wrap if sksl => SKSL_WRAP,
            Helper::Wrap => WRAP,
            Helper::Div => "float nDiv(float a, float b) {\n    return b == 0.0 ? 0.0 : a / b;\n}\n",
            Helper::Mod => {
                "float nMod(float a, float b) {\n    return b == 0.0 ? 0.0 : a - b * floor(a / b);\n}\n"
            }
            Helper::Pow => "float nPow(float a, float b) {\n    return a > 0.0 ? pow(a, b) : 0.0;\n}\n",
            Helper::Atan2 => {
                "float nAtan2(float y, float x) {\n    return x == 0.0 && y == 0.0 ? 0.0 : atan(y, x);\n}\n"
            }
            Helper::Clamp => CLAMP,
            Helper::Mix => MIX,
            Helper::Smooth => SMOOTH,
            Helper::Inverse if sksl => "",
            Helper::Inverse => INVERSE,
            Helper::Hash | Helper::Noise | Helper::Fbm if sksl => "",
            Helper::Hash => HASH,
            Helper::Noise => NOISE,
            Helper::Fbm => FBM,
            Helper::Bayer if sksl => SKSL_BAYER,
            Helper::Bayer => BAYER,
            Helper::Checker => CHECKER,
            Helper::Stripes => STRIPES,
            Helper::Gradient => GRADIENT,
            Helper::BodyEdge => BODY_EDGE,
            Helper::Tone if sksl && tones > 0 => {
                out.push_str(&format!(
                    "vec4 nTone(int ref) {{\n    for (int i = 0; i < {tones}; i++) {{\n        if (i == ref) {{\n            return {TONES_UNIFORM}[i];\n        }}\n    }}\n    return vec4(0.0);\n}}\n"
                ));
                ""
            }
            Helper::Tone => {
                if tones == 0 {
                    "vec4 nTone(int ref) {\n    return vec4(0.0);\n}\n"
                } else {
                    out.push_str(&format!(
                        "vec4 nTone(int ref) {{\n    return ref < 0 || ref >= {tones} ? vec4(0.0) : {TONES_UNIFORM}[ref];\n}}\n"
                    ));
                    ""
                }
            }
            Helper::Quantize => QUANTIZE,
            Helper::Dither => DITHER,
            Helper::Cycle => CYCLE,
            Helper::Hsv => HSV,
            Helper::Lighten => {
                "vec4 nLighten(vec4 c, float a) {\n    return vec4(c.rgb + (1.0 - c.rgb) * a, c.a);\n}\n"
            }
            Helper::Darken => "vec4 nDarken(vec4 c, float a) {\n    return vec4(c.rgb * (1.0 - a), c.a);\n}\n",
            Helper::Alpha => "vec4 nAlpha(vec4 c, float a) {\n    return vec4(c.rgb, c.a * a);\n}\n",
            Helper::Polar => "vec2 nPolar(vec2 v) {\n    return vec2(length(v), nAtan2(v.y, v.x));\n}\n",
            Helper::Sample | Helper::Atlas if sksl => "",
            Helper::Sample => SAMPLE,
            Helper::Atlas => ATLAS,
            Helper::TagUv => {
                out.push_str(&format!(
                    "vec2 nTagUv(vec2 p) {{\n    return (p - {AREA_UNIFORM}.xy) / ({AREA_UNIFORM}.zw - {AREA_UNIFORM}.xy);\n}}\n"
                ));
                ""
            }
            Helper::Seconds => {
                out.push_str(&format!(
                    "float nSeconds(float frame) {{\n    return nDiv(frame * float({CLOCK_UNIFORM}.w), float({CLOCK_UNIFORM}.z));\n}}\n"
                ));
                ""
            }
            Helper::Screen if sksl => {
                out.push_str(&format!(
                    "vec2 nScreen(vec2 p) {{\n    return ({SKSL_SCREEN_UNIFORM} * vec3(p, 1.0)).xy;\n}}\n"
                ));
                ""
            }
            Helper::Screen => SCREEN,
        };
        out.push_str(text);
        if let Some((name, params)) = self.vector() {
            for width in 2..=4u8 {
                let ty = Ty::vector(width).unwrap_or(Ty::Vec4).glsl();
                let signature = params
                    .iter()
                    .map(|p| format!("{ty} {p}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let parts = SWIZZLE[..usize::from(width)]
                    .iter()
                    .map(|c| {
                        let args = params
                            .iter()
                            .map(|p| format!("{p}.{c}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{name}({args})")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "{ty} {name}({signature}) {{\n    return {ty}({parts});\n}}\n"
                ));
            }
        }
    }
}

const WRAP: &str = "int nWrap(int x, int p) {
    if (p <= 0) {
        return x;
    }
    int q = x >= 0 ? x / p : -((-x - 1) / p) - 1;
    return x - p * q;
}
";

const SKSL_WRAP: &str = "int nWrap(int x, int p) {
    if (p <= 0) {
        return x;
    }
    int r = x - p * int(floor(float(x) / float(p)));
    if (r < 0) {
        r += p;
    }
    if (r >= p) {
        r -= p;
    }
    return r;
}
";

const SKSL_BAYER: &str = "float nBayer(vec2 p) {
    vec2 c = floor(p);
    vec2 a = c - 2.0 * floor(c / 2.0);
    vec2 b = floor(c / 2.0) - 2.0 * floor(c / 4.0);
    return (8.0 * abs(a.x - a.y) + 4.0 * a.y + 2.0 * abs(b.x - b.y) + b.y) / 16.0;
}
";

const CLAMP: &str = "float nClamp(float x, float lo, float hi) {
    return min(max(x, lo), hi);
}
vec2 nClamp(vec2 x, vec2 lo, vec2 hi) {
    return min(max(x, lo), hi);
}
vec3 nClamp(vec3 x, vec3 lo, vec3 hi) {
    return min(max(x, lo), hi);
}
vec4 nClamp(vec4 x, vec4 lo, vec4 hi) {
    return min(max(x, lo), hi);
}
";

const MIX: &str = "float nMix(float a, float b, float t) {
    return a * (1.0 - t) + b * t;
}
vec2 nMix(vec2 a, vec2 b, vec2 t) {
    return a * (1.0 - t) + b * t;
}
vec3 nMix(vec3 a, vec3 b, vec3 t) {
    return a * (1.0 - t) + b * t;
}
vec4 nMix(vec4 a, vec4 b, vec4 t) {
    return a * (1.0 - t) + b * t;
}
";

const SMOOTH: &str = "float nSmooth(float e0, float e1, float x) {
    float t = nClamp(nDiv(x - e0, e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
";

const INVERSE: &str = "mat2 nInverse(mat2 m) {
    float d = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    return d == 0.0 ? mat2(0.0) : mat2(m[1][1], -m[0][1], -m[1][0], m[0][0]) / d;
}
";

const HASH: &str = "float nHash(ivec2 p, int seed) {
    uint h = uint(p.x) * 0x27D4EB2Du ^ uint(p.y) * 0x165667B1u ^ uint(seed) * 0x5BD1E995u;
    h ^= h >> 15u;
    h *= 0x2C1B3C6Du;
    h ^= h >> 12u;
    h *= 0x297A2D39u;
    h ^= h >> 15u;
    return float(h >> 8u) / 16777216.0;
}
float nHashAt(vec2 p, int seed) {
    return nHash(ivec2(floor(p)), seed);
}
";

const NOISE: &str = "float nNoise(vec2 p, ivec2 period, int seed) {
    vec2 cell = floor(p);
    vec2 f = p - cell;
    f = f * f * (3.0 - 2.0 * f);
    int x0 = nWrap(int(cell.x), period.x);
    int x1 = nWrap(int(cell.x) + 1, period.x);
    int y0 = nWrap(int(cell.y), period.y);
    int y1 = nWrap(int(cell.y) + 1, period.y);
    float top = nMix(nHash(ivec2(x0, y0), seed), nHash(ivec2(x1, y0), seed), f.x);
    float bottom = nMix(nHash(ivec2(x0, y1), seed), nHash(ivec2(x1, y1), seed), f.x);
    return nMix(top, bottom, f.y);
}
";

const FBM: &str = "float nFbm(vec2 p, int octaves, ivec2 period, int seed) {
    float sum = 0.0;
    float weight = 1.0;
    float total = 0.0;
    for (int i = 0; i < octaves; i++) {
        sum += nNoise(p, period, seed + i) * weight;
        total += weight;
        p *= 2.0;
        period *= 2;
        weight *= 0.5;
    }
    return total == 0.0 ? 0.0 : sum / total;
}
";

const BAYER: &str =
    "const int N_BAYER[16] = int[16](0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5);
float nBayer(vec2 p) {
    ivec2 c = ivec2(floor(p));
    return float(N_BAYER[nWrap(c.y, 4) * 4 + nWrap(c.x, 4)]) / 16.0;
}
";

const CHECKER: &str = "float nChecker(vec2 p, int size) {
    float s = float(size);
    return float(nWrap(int(floor(p.x / s)) + int(floor(p.y / s)), 2));
}
";

const STRIPES: &str = "float nStripes(vec2 p, int period, int width, ivec2 d) {
    int t = nWrap(int(floor(p.x)) * d.x + int(floor(p.y)) * d.y, period);
    return t < width ? 1.0 : 0.0;
}
";

const GRADIENT: &str = "float nGradient(vec2 p, vec2 a, vec2 b) {
    vec2 d = b - a;
    return nClamp(nDiv(dot(p - a, d), dot(d, d)), 0.0, 1.0);
}
";

const BODY_EDGE: &str = "float nBodyEdge(vec2 p, vec2 size, vec4 radii) {
    vec2 c = p - 0.5 * size;
    float r = c.x < 0.0 ? (c.y < 0.0 ? radii.x : radii.w) : (c.y < 0.0 ? radii.y : radii.z);
    vec2 q = abs(c) - 0.5 * size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}
";

const QUANTIZE: &str = "int nQuantize(float level, int base, int count) {
    float top = float(count - 1);
    return base + int(clamp(floor(level * top + 0.5), 0.0, top));
}
";

const DITHER: &str = "int nDither(float level, vec2 p, int base, int count) {
    float top = float(count - 1);
    return base + int(clamp(floor(level * top + nBayer(p)), 0.0, top));
}
";

const CYCLE: &str = "int nCycle(float index, float shift, int base, int count) {
    return base + nWrap(int(floor(index)) + int(floor(shift)), count);
}
";

const HSV: &str = "vec4 nHsv(float h, float s, float v) {
    float h6 = (h - floor(h)) * 6.0;
    float sector = floor(h6);
    float f = h6 - sector;
    float p = v * (1.0 - s);
    float q = v * (1.0 - s * f);
    float t = v * (1.0 - s * (1.0 - f));
    int i = nWrap(int(sector), 6);
    vec3 c = i == 0 ? vec3(v, t, p) : i == 1 ? vec3(q, v, p) : i == 2 ? vec3(p, v, t)
        : i == 3 ? vec3(p, q, v) : i == 4 ? vec3(t, p, v) : vec3(v, p, q);
    return vec4(c, 1.0);
}
";

const SAMPLE: &str = "vec4 nSample(sampler2D s, vec2 uv) {
    ivec2 size = textureSize(s, 0);
    ivec2 t = ivec2(floor(uv * vec2(size)));
    return texelFetch(s, ivec2(nWrap(t.x, size.x), nWrap(t.y, size.y)), 0);
}
";

const ATLAS: &str = "vec4 nAtlas(sampler2D s, vec4 rect, vec2 uv) {
    ivec2 size = ivec2(rect.zw);
    ivec2 t = ivec2(floor(uv * rect.zw));
    return texelFetch(s, ivec2(rect.xy) + ivec2(nWrap(t.x, size.x), nWrap(t.y, size.y)), 0);
}
";

const SCREEN: &str = "mat2 nScreenJ;
vec2 nFrag;
vec2 nScreen(vec2 p) {
    return (gl_FragCoord.xy + nScreenJ * (p - nFrag)) / ScreenSize;
}
";

const CORE_VERTEX: &str = "#version 330
#extension GL_ARB_separate_shader_objects : require

#include <minecraft:fog.glsl>
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
#include <minecraft:sample_lightmap.glsl>

layout(location = 0) in vec3 Position;
layout(location = 1) in vec2 UV0;
layout(location = 2) in vec4 Color;
layout(location = 3) in ivec2 UV2;

uniform sampler2D Sampler2;

layout(location = 0) out float sphericalVertexDistance;
layout(location = 1) out float cylindricalVertexDistance;
layout(location = 2) out vec2 pixelPos;
layout(location = 3) out vec4 vertexColor;
layout(location = 4) out vec4 lightColor;

void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    sphericalVertexDistance = fog_spherical_distance(Position);
    cylindricalVertexDistance = fog_cylindrical_distance(Position);
    pixelPos = UV0;
    vertexColor = Color;
    lightColor = sample_lightmap(Sampler2, UV2);
}
";

const UBO_TRANSFORMS: &str = "layout(std140) uniform DynamicTransforms {
    mat4 ModelViewMat;
    vec4 ColorModulator;
    vec3 ModelOffset;
    mat4 TextureMat;
};
";

const UBO_VERTEX: &str = "#version 150

layout(std140) uniform Projection {
    mat4 ProjMat;
};

layout(std140) uniform DynamicTransforms {
    mat4 ModelViewMat;
    vec4 ColorModulator;
    vec3 ModelOffset;
    mat4 TextureMat;
};

in vec3 Position;
in vec2 UV0;
in vec4 Color;
in ivec2 UV2;

uniform sampler2D Sampler2;

out float sphericalVertexDistance;
out float cylindricalVertexDistance;
out vec2 pixelPos;
out vec4 vertexColor;
out vec4 lightColor;

void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    sphericalVertexDistance = length(Position);
    cylindricalVertexDistance = max(length(Position.xz), abs(Position.y));
    pixelPos = UV0;
    vertexColor = Color;
    lightColor = texture(Sampler2, clamp(vec2(UV2) / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0)));
}
";

const UBO_FOG: &str = "layout(std140) uniform Fog {
    vec4 FogColor;
    float FogEnvironmentalStart;
    float FogEnvironmentalEnd;
    float FogRenderDistanceStart;
    float FogRenderDistanceEnd;
    float FogSkyEnd;
    float FogCloudsEnd;
};

float nFogValue(float distance, float start, float end) {
    if (distance <= start) {
        return 0.0;
    }
    return distance >= end ? 1.0 : (distance - start) / (end - start);
}

vec4 nFog(vec4 color, vec4 fogColor) {
    float value = max(nFogValue(sphericalVertexDistance, FogEnvironmentalStart, FogEnvironmentalEnd),
        nFogValue(cylindricalVertexDistance, FogRenderDistanceStart, FogRenderDistanceEnd));
    return vec4(mix(color.rgb, fogColor.rgb, value * fogColor.a), color.a);
}
";

const UBO_GLOBALS: &str = "#ifdef NAMEPLATE_SCREEN_FIRST
layout(std140) uniform Globals {
    vec2 ScreenSize;
    float GlintAlpha;
    float GameTime;
    int MenuBlurRadius;
};
#else
layout(std140) uniform Globals {
    ivec3 CameraBlockPos;
    vec3 CameraOffset;
    vec2 ScreenSize;
    float GlintAlpha;
    float GameTime;
    int MenuBlurRadius;
    int UseRgss;
};
#endif
";

const LEGACY_VERTEX: &str = "#version 150

in vec3 Position;
in vec2 UV0;
in vec4 Color;
in ivec2 UV2;

uniform sampler2D Sampler2;

uniform mat4 ModelViewMat;
uniform mat4 ProjMat;
uniform int FogShape;

out float vertexDistance;
out vec2 pixelPos;
out vec4 vertexColor;
out vec4 lightColor;

void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    vertexDistance = FogShape == 0 ? length(Position) : max(length(Position.xz), abs(Position.y));
    pixelPos = UV0;
    vertexColor = Color;
    lightColor = texelFetch(Sampler2, UV2 / 16, 0);
}
";

const COMPAT_VERTEX: &str = "#version 130

out float vertexDistance;
out vec2 pixelPos;
out vec4 vertexColor;
out vec2 lightUv;

void main() {
    vec4 eye = gl_ModelViewMatrix * gl_Vertex;
    gl_Position = gl_ProjectionMatrix * eye;
    vertexDistance = length(eye.xyz);
    pixelPos = gl_MultiTexCoord0.xy;
    vertexColor = gl_Color;
    lightUv = (gl_TextureMatrix[1] * gl_MultiTexCoord1).xy;
}
";

const ES_VERTEX: &str = "#version 300 es
precision highp float;

in vec3 Position;
in vec2 UV0;
in vec4 Color;

uniform mat4 ProjMat;
uniform mat4 ModelViewMat;

out vec2 pixelPos;
out vec4 vertexColor;

void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    pixelPos = UV0;
    vertexColor = Color;
}
";

struct Opacity<'a, 'm> {
    ir: &'a MaterialIr,
    memo: &'m mut Vec<Option<bool>>,
}

impl Opacity<'_, '_> {
    fn def(&self, reg: Reg) -> Option<usize> {
        if reg.is_none() {
            return None;
        }
        self.ir.ops.iter().rposition(|op| op.dst == reg)
    }

    fn constant(&self, reg: Reg) -> Option<f64> {
        let op = &self.ir.ops[self.def(reg)?];
        match (op.kind, op.ty) {
            (OpKind::Constant, Ty::Float) => Some(op.imm[0]),
            _ => None,
        }
    }

    fn reg(&mut self, reg: Reg, depth: u32) -> bool {
        let Some(i) = self.def(reg) else {
            return false;
        };
        if let Some(known) = self.memo[i] {
            return known;
        }
        if depth > 256 {
            return false;
        }
        let op = self.ir.ops[i];
        let [a, b, c, _] = op.args;
        let result = match op.kind {
            OpKind::Hsv => true,
            OpKind::Constant => op.ty == Ty::Vec4 && op.imm[3] >= 1.0,
            OpKind::Lighten | OpKind::Darken => op.ty == Ty::Vec4 && self.vec4(a, depth),
            OpKind::Alpha => self.vec4(a, depth) && self.constant(b).is_some_and(|v| v >= 1.0),
            OpKind::Mix | OpKind::Min => {
                op.ty == Ty::Vec4 && self.vec4(a, depth) && self.vec4(b, depth)
            }
            OpKind::Max => op.ty == Ty::Vec4 && (self.vec4(a, depth) || self.vec4(b, depth)),
            OpKind::Select => op.ty == Ty::Vec4 && self.vec4(b, depth) && self.vec4(c, depth),
            OpKind::Combine => {
                op.ty == Ty::Vec4 && self.constant(op.args[3]).is_some_and(|v| v >= 1.0)
            }
            OpKind::SampleAt => op.ty == Ty::Vec4 && self.vec4(op.body_result(), depth),
            OpKind::Repeat => {
                op.ty == Ty::Vec4
                    && Reduce::from_value(op.imm[3]) != Reduce::First
                    && int(op.imm[2]) >= 1
                    && self.vec4(op.body_result(), depth)
            }
            _ => false,
        };
        self.memo[i] = Some(result);
        result
    }

    fn vec4(&mut self, reg: Reg, depth: u32) -> bool {
        let Some(i) = self.def(reg) else {
            return false;
        };
        self.ir.ops[i].ty == Ty::Vec4 && self.reg(reg, depth + 1)
    }
}

struct Gen<'a> {
    ir: &'a MaterialIr,
    dialect: Dialect,
    types: Vec<Option<Ty>>,
    owner: Vec<usize>,
    bodies: Vec<Range<usize>>,
    globals: Vec<bool>,
    needs: BTreeSet<Helper>,
    sprites: BTreeSet<usize>,
    screen: bool,
}

impl<'a> Gen<'a> {
    fn new(ir: &'a MaterialIr, dialect: Dialect) -> Gen<'a> {
        let count = ir.ops.len();
        let regs = ir
            .ops
            .iter()
            .filter(|op| !op.dst.is_none())
            .map(|op| usize::from(op.dst.0) + 1)
            .max()
            .unwrap_or(0);
        let mut types = vec![None; regs];
        for op in &ir.ops {
            if !op.dst.is_none() {
                types[usize::from(op.dst.0)] = Some(op.ty);
            }
        }
        let mut bodies: Vec<Range<usize>> = ir
            .ops
            .iter()
            .filter_map(Op::body)
            .map(|r| r.start.min(count)..r.end.min(count))
            .filter(|r| !r.is_empty())
            .collect();
        bodies.sort_by_key(|r| (r.start, usize::MAX - r.end));
        bodies.dedup();
        let owner = (0..count)
            .map(|i| {
                bodies
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.contains(&i))
                    .min_by_key(|(_, r)| r.len())
                    .map_or(0, |(k, _)| k + 1)
            })
            .collect();
        let mut gen = Gen {
            ir,
            dialect,
            types,
            owner,
            bodies,
            globals: vec![false; regs],
            needs: BTreeSet::new(),
            sprites: BTreeSet::new(),
            screen: false,
        };
        gen.find_globals();
        gen
    }

    fn find_globals(&mut self) {
        let mut def = vec![usize::MAX; self.types.len()];
        for (i, op) in self.ir.ops.iter().enumerate() {
            if !op.dst.is_none() {
                def[usize::from(op.dst.0)] = self.owner[i];
            }
        }
        let read = |reg: Reg, context: usize, globals: &mut Vec<bool>| {
            if let Some(&d) = def.get(usize::from(reg.0)) {
                if !reg.is_none() && d != usize::MAX && d != context {
                    globals[usize::from(reg.0)] = true;
                }
            }
        };
        let mut globals = std::mem::take(&mut self.globals);
        for (i, op) in self.ir.ops.iter().enumerate() {
            let result = match op.kind {
                OpKind::SampleAt => 2,
                OpKind::Repeat => 0,
                _ => usize::MAX,
            };
            for (s, arg) in op.args.into_iter().enumerate() {
                if s != result {
                    read(arg, self.owner[i], &mut globals);
                }
            }
            if let Some(k) = self.body_of(op) {
                read(op.body_result(), k + 1, &mut globals);
            }
        }
        read(self.ir.color, 0, &mut globals);
        if let Some(coverage) = self.ir.coverage {
            read(coverage, 0, &mut globals);
        }
        self.globals = globals;
    }

    fn body_of(&self, op: &Op) -> Option<usize> {
        let count = self.ir.ops.len();
        let range = op.body()?;
        let range = range.start.min(count)..range.end.min(count);
        self.bodies.iter().position(|r| *r == range)
    }

    fn ty(&self, reg: Reg) -> Option<Ty> {
        if reg.is_none() {
            return None;
        }
        self.types.get(usize::from(reg.0)).copied().flatten()
    }

    fn global(&self, reg: Reg) -> bool {
        !reg.is_none()
            && self
                .globals
                .get(usize::from(reg.0))
                .copied()
                .unwrap_or(false)
    }

    fn need(&mut self, helper: Helper) {
        if self.needs.insert(helper) {
            for dep in helper.deps() {
                self.need(*dep);
            }
        }
    }

    fn cast(&mut self, reg: Reg, target: Ty) -> String {
        match self.ty(reg) {
            None => zero(target),
            Some(from) => {
                if from == Ty::Slot && target != Ty::Slot {
                    self.need(Helper::Tone);
                }
                convert(&format!("r{}", reg.0), from, target)
            }
        }
    }

    fn arg(&mut self, op: &Op, socket: usize, target: Ty) -> String {
        let reg = op.args[socket];
        if reg.is_none() || self.ty(reg).is_none() {
            let default = self.default(op.kind, socket);
            return default.map_or_else(|| zero(target), |(expr, ty)| convert(&expr, ty, target));
        }
        self.cast(reg, target)
    }

    fn default(&mut self, kind: OpKind, socket: usize) -> Option<(String, Ty)> {
        let cell = ("floor(pos)".to_string(), Ty::Vec2);
        match (kind, socket) {
            (OpKind::BodyEdge, 0) => Some(("pos".to_string(), Ty::Vec2)),
            (OpKind::Bayer | OpKind::Checker | OpKind::Stripes | OpKind::Sprite, 0) => Some(cell),
            (OpKind::DitherQuantize, 1) => Some(cell),
            (
                OpKind::Gradient
                | OpKind::BundleTexture
                | OpKind::VanillaTexture
                | OpKind::AtlasSprite,
                0,
            ) => {
                self.need(Helper::TagUv);
                Some(("nTagUv(pos)".to_string(), Ty::Vec2))
            }
            (OpKind::Gradient, 2) => Some(("vec2(1.0, 0.0)".to_string(), Ty::Vec2)),
            (OpKind::Hsv, 1 | 2) => Some(("1.0".to_string(), Ty::Float)),
            (OpKind::Cycle, 1) => Some(("frame".to_string(), Ty::Float)),
            _ => None,
        }
    }

    fn widest(&self, op: &Op, sockets: usize) -> Ty {
        let width = op.args[..sockets]
            .iter()
            .filter_map(|reg| self.ty(*reg))
            .map(|ty| numeric(ty).width())
            .max()
            .unwrap_or(1);
        Ty::vector(width).unwrap_or(Ty::Float)
    }

    fn lane(&self, index: f64) -> String {
        let lane = int(index).clamp(0, LANE_COUNT as i32 - 1) as usize;
        let (vector, component) = (lane / 4, SWIZZLE[lane % 4]);
        match self.dialect {
            Dialect::Legacy150 => format!("{LANES_UNIFORM}{vector}.{component}"),
            _ => format!("{LANES_UNIFORM}[{vector}].{component}"),
        }
    }

    fn atlas(&self, k: usize) -> String {
        match self.dialect {
            Dialect::Legacy150 => format!("{ATLAS_UNIFORM}{k}"),
            _ => format!("{ATLAS_UNIFORM}[{k}]"),
        }
    }

    fn tone_base(&self, slot: u32) -> Option<i32> {
        let index = self
            .ir
            .tone_slots
            .iter()
            .position(|s| u32::from(*s) == slot)?;
        Some(index as i32 * TONES_PER_SLOT as i32)
    }

    fn compact(&self, tone_ref: i64) -> i32 {
        if tone_ref < 0 {
            return -1;
        }
        let tone_ref = tone_ref as u64;
        let slot = (tone_ref / u64::from(TONES_PER_SLOT)) as u32;
        let tone = (tone_ref % u64::from(TONES_PER_SLOT)) as i32;
        self.tone_base(slot).map_or(-1, |base| base + tone)
    }

    fn compact_ref(&self, color: ColorRef) -> i32 {
        match color.kind() {
            RefKind::Slot => self.compact(i64::from(color.tone_ref())),
            RefKind::Blend => {
                self.compact(i64::from(color.slot_index()) * i64::from(TONES_PER_SLOT))
            }
            RefKind::Fixed => -1,
        }
    }

    fn natural(&self, op: &Op) -> Ty {
        match op.kind {
            OpKind::Slot if !op.args[0].is_none() => Ty::Vec4,
            OpKind::PixelPos
            | OpKind::PixelCell
            | OpKind::TagUv
            | OpKind::AreaSize
            | OpKind::ScreenPos
            | OpKind::PixelScreenPos
            | OpKind::Polar => Ty::Vec2,
            OpKind::BodyRect
            | OpKind::Hsv
            | OpKind::Lighten
            | OpKind::Darken
            | OpKind::Alpha
            | OpKind::BundleTexture
            | OpKind::VanillaTexture
            | OpKind::AtlasSprite => Ty::Vec4,
            OpKind::Frame
            | OpKind::SteppedTime
            | OpKind::Param
            | OpKind::Expr
            | OpKind::BodyEdge
            | OpKind::Length
            | OpKind::Dot
            | OpKind::Noise
            | OpKind::Fbm
            | OpKind::Hash
            | OpKind::Bayer
            | OpKind::Checker
            | OpKind::Stripes
            | OpKind::Gradient
            | OpKind::RepeatIndex
            | OpKind::Component => Ty::Float,
            OpKind::Slot
            | OpKind::Quantize
            | OpKind::DitherQuantize
            | OpKind::Cycle
            | OpKind::Sprite => Ty::Slot,
            OpKind::SampleAt | OpKind::Repeat => self.ty(op.body_result()).unwrap_or(op.ty),
            _ => op.ty,
        }
    }

    fn elementwise(&mut self, op: &Op, name: &str, sockets: usize) -> String {
        let ty = numeric(op.ty);
        let args: Vec<String> = (0..sockets).map(|s| self.arg(op, s, ty)).collect();
        format!("{name}({})", args.join(", "))
    }

    fn infix(&mut self, op: &Op, operator: &str) -> String {
        let ty = numeric(op.ty);
        let a = self.arg(op, 0, ty);
        let b = self.arg(op, 1, ty);
        format!("({a} {operator} {b})")
    }

    fn ramp(&mut self, op: &Op, helper: Helper, name: &str, inputs: &[Ty]) -> String {
        let Some(base) = self.tone_base(int(op.imm[0]).max(0) as u32) else {
            return "-1".to_string();
        };
        self.need(helper);
        let mut args: Vec<String> = inputs
            .iter()
            .enumerate()
            .map(|(s, ty)| self.arg(op, s, *ty))
            .collect();
        args.push(base.to_string());
        args.push(int(op.imm[1]).max(1).to_string());
        format!("{name}({})", args.join(", "))
    }

    fn texture(&mut self, op: &Op) -> String {
        let k = int(op.imm[0]).max(0) as usize;
        if k >= self.ir.textures.len().min(MAX_SAMPLES) {
            return "vec4(0.0)".to_string();
        }
        let uv = self.arg(op, 0, Ty::Vec2);
        if self.dialect == Dialect::Sksl {
            self.need(Helper::Wrap);
            return format!("nTexture{k}({uv})");
        }
        let sampler = sampler_name(self.dialect, k);
        if matches!(self.ir.textures[k], TexRef::Atlas(_)) || op.kind == OpKind::AtlasSprite {
            self.need(Helper::Atlas);
            format!("nAtlas({sampler}, {}, {uv})", self.atlas(k))
        } else {
            self.need(Helper::Sample);
            format!("nSample({sampler}, {uv})")
        }
    }

    fn expr(&mut self, op: &Op) -> String {
        match op.kind {
            OpKind::PixelPos => "pos".to_string(),
            OpKind::PixelCell => "floor(pos)".to_string(),
            OpKind::TagUv => {
                self.need(Helper::TagUv);
                "nTagUv(pos)".to_string()
            }
            OpKind::AreaSize => format!("({AREA_UNIFORM}.zw - {AREA_UNIFORM}.xy)"),
            OpKind::BodyRect => BODY_UNIFORM.to_string(),
            OpKind::Frame => "frame".to_string(),
            OpKind::SteppedTime => {
                self.need(Helper::Seconds);
                "nSeconds(frame)".to_string()
            }
            OpKind::ScreenPos => {
                self.need(Helper::Screen);
                self.screen = true;
                "nScreen(pos)".to_string()
            }
            OpKind::PixelScreenPos => {
                self.need(Helper::Screen);
                self.screen = true;
                "nScreen(floor(pos) + 0.5)".to_string()
            }
            OpKind::Slot if !op.args[0].is_none() => {
                self.need(Helper::Tone);
                format!("nTone({})", self.cast(op.args[0], Ty::Slot))
            }
            OpKind::Slot => self.compact(op.imm[0] as i64).to_string(),
            OpKind::Constant => match op.ty {
                Ty::Slot => self.compact(op.imm[0] as i64).to_string(),
                Ty::Float => float(op.imm[0]),
                ty => {
                    let values: Vec<String> = op.imm[..usize::from(ty.width())]
                        .iter()
                        .map(|v| float(*v))
                        .collect();
                    format!("{}({})", ty.glsl(), values.join(", "))
                }
            },
            OpKind::Param | OpKind::Expr => self.lane(op.imm[0]),
            OpKind::BodyEdge => {
                self.need(Helper::BodyEdge);
                let p = self.arg(op, 0, Ty::Vec2);
                let base = int(op.imm[0]).clamp(0, LANE_COUNT as i32 - 5);
                let radii: Vec<String> = (0..4).map(|i| self.lane(f64::from(base + i))).collect();
                let radii = format!("vec4({})", radii.join(", "));
                let size = format!("({BODY_UNIFORM}.zw - {BODY_UNIFORM}.xy)");
                if op.imm[1] != 0.0 {
                    let rings = self.lane(f64::from(base + 4));
                    format!("nBodyEdge({p} + {rings}, {size} + 2.0 * {rings}, {radii} + {rings})")
                } else {
                    format!("nBodyEdge({p}, {size}, {radii})")
                }
            }
            OpKind::Abs => self.elementwise(op, "abs", 1),
            OpKind::Floor => self.elementwise(op, "floor", 1),
            OpKind::Frac => self.elementwise(op, "fract", 1),
            OpKind::Sin => self.elementwise(op, "sin", 1),
            OpKind::Cos => self.elementwise(op, "cos", 1),
            OpKind::Exp => self.elementwise(op, "exp", 1),
            OpKind::Add => self.infix(op, "+"),
            OpKind::Sub => self.infix(op, "-"),
            OpKind::Mul => self.infix(op, "*"),
            OpKind::Div => {
                self.need(Helper::Div);
                self.elementwise(op, "nDiv", 2)
            }
            OpKind::Mod => {
                self.need(Helper::Mod);
                self.elementwise(op, "nMod", 2)
            }
            OpKind::Min => self.elementwise(op, "min", 2),
            OpKind::Max => self.elementwise(op, "max", 2),
            OpKind::Step => self.elementwise(op, "step", 2),
            OpKind::Pow => {
                self.need(Helper::Pow);
                self.elementwise(op, "nPow", 2)
            }
            OpKind::Atan2 => {
                self.need(Helper::Atan2);
                self.elementwise(op, "nAtan2", 2)
            }
            OpKind::Clamp => {
                self.need(Helper::Clamp);
                self.elementwise(op, "nClamp", 3)
            }
            OpKind::Mix => {
                self.need(Helper::Mix);
                self.elementwise(op, "nMix", 3)
            }
            OpKind::Smoothstep => {
                self.need(Helper::Smooth);
                self.elementwise(op, "nSmooth", 3)
            }
            OpKind::Length => {
                let ty = self.widest(op, 1);
                format!("length({})", self.arg(op, 0, ty))
            }
            OpKind::Dot => {
                let ty = self.widest(op, 2);
                let a = self.arg(op, 0, ty);
                let b = self.arg(op, 1, ty);
                format!("dot({a}, {b})")
            }
            OpKind::Polar => {
                self.need(Helper::Polar);
                format!("nPolar({})", self.arg(op, 0, Ty::Vec2))
            }
            OpKind::Combine => {
                let width = numeric(op.ty).width();
                let parts: Vec<String> = (0..usize::from(width))
                    .map(|s| self.arg(op, s, Ty::Float))
                    .collect();
                if width == 1 {
                    parts[0].clone()
                } else {
                    format!("{}({})", numeric(op.ty).glsl(), parts.join(", "))
                }
            }
            OpKind::Component => {
                let ty = self.widest(op, 1);
                let value = self.arg(op, 0, ty);
                if ty.width() == 1 {
                    value
                } else {
                    let c = int(op.imm[0]).clamp(0, i32::from(ty.width()) - 1) as usize;
                    format!("{value}.{}", SWIZZLE[c])
                }
            }
            OpKind::Select => {
                let condition = self.arg(op, 0, Ty::Float);
                let a = self.arg(op, 1, op.ty);
                let b = self.arg(op, 2, op.ty);
                format!("({condition} > 0.0 ? {b} : {a})")
            }
            OpKind::Noise | OpKind::Fbm => {
                let p = self.arg(op, 0, Ty::Vec2);
                let seed = int(op.imm[0]);
                let period = format!("ivec2({}, {})", int(op.imm[1]), int(op.imm[2]));
                if op.kind == OpKind::Fbm {
                    self.need(Helper::Fbm);
                    format!("nFbm({p}, {}, {period}, {seed})", int(op.imm[3]).max(1))
                } else {
                    self.need(Helper::Noise);
                    format!("nNoise({p}, {period}, {seed})")
                }
            }
            OpKind::Hash => {
                self.need(Helper::Hash);
                format!("nHashAt({}, {})", self.arg(op, 0, Ty::Vec2), int(op.imm[0]))
            }
            OpKind::Bayer => {
                self.need(Helper::Bayer);
                format!("nBayer({})", self.arg(op, 0, Ty::Vec2))
            }
            OpKind::Checker => {
                self.need(Helper::Checker);
                format!(
                    "nChecker({}, {})",
                    self.arg(op, 0, Ty::Vec2),
                    int(op.imm[0]).max(1)
                )
            }
            OpKind::Stripes => {
                self.need(Helper::Stripes);
                format!(
                    "nStripes({}, {}, {}, ivec2({}, {}))",
                    self.arg(op, 0, Ty::Vec2),
                    int(op.imm[0]).max(1),
                    int(op.imm[1]),
                    int(op.imm[2]),
                    int(op.imm[3])
                )
            }
            OpKind::Gradient => {
                self.need(Helper::Gradient);
                let p = self.arg(op, 0, Ty::Vec2);
                let a = self.arg(op, 1, Ty::Vec2);
                let b = self.arg(op, 2, Ty::Vec2);
                format!("nGradient({p}, {a}, {b})")
            }
            OpKind::Hsv => {
                self.need(Helper::Hsv);
                let h = self.arg(op, 0, Ty::Float);
                let s = self.arg(op, 1, Ty::Float);
                let v = self.arg(op, 2, Ty::Float);
                format!("nHsv({h}, {s}, {v})")
            }
            OpKind::Quantize => self.ramp(op, Helper::Quantize, "nQuantize", &[Ty::Float]),
            OpKind::DitherQuantize => {
                self.ramp(op, Helper::Dither, "nDither", &[Ty::Float, Ty::Vec2])
            }
            OpKind::Cycle => self.ramp(op, Helper::Cycle, "nCycle", &[Ty::Float, Ty::Float]),
            OpKind::Lighten | OpKind::Darken | OpKind::Alpha => {
                let (helper, name) = match op.kind {
                    OpKind::Lighten => (Helper::Lighten, "nLighten"),
                    OpKind::Darken => (Helper::Darken, "nDarken"),
                    _ => (Helper::Alpha, "nAlpha"),
                };
                self.need(helper);
                let color = self.arg(op, 0, Ty::Vec4);
                let amount = self.arg(op, 1, Ty::Float);
                format!("{name}({color}, {amount})")
            }
            OpKind::SampleAt => {
                let Some(k) = self.body_of(op) else {
                    return zero(op.ty);
                };
                let mut position = "pos".to_string();
                if !op.args[0].is_none() {
                    position = format!("pos + {}", self.cast(op.args[0], Ty::Vec2));
                }
                let mut time = "frame".to_string();
                if !op.args[1].is_none() {
                    time = format!("frame + {}", self.cast(op.args[1], Ty::Float));
                }
                format!("nBody{k}({position}, {time}, index)")
            }
            OpKind::Repeat => zero(op.ty),
            OpKind::RepeatIndex => "index".to_string(),
            OpKind::Sprite => {
                let s = int(op.imm[0]).max(0) as usize;
                if s >= self.ir.sprites.len() {
                    return "-1".to_string();
                }
                self.sprites.insert(s);
                self.need(Helper::Wrap);
                let at = self.arg(op, 0, Ty::Vec2);
                let frame = self.arg(op, 1, Ty::Float);
                format!("nSprite{s}({at}, {frame})")
            }
            OpKind::BundleTexture | OpKind::VanillaTexture | OpKind::AtlasSprite => {
                self.texture(op)
            }
        }
    }

    fn statement(&mut self, op: &Op, out: &mut Vec<String>) {
        if op.dst.is_none() {
            return;
        }
        let name = format!("r{}", op.dst.0);
        let decl = if self.global(op.dst) {
            name.clone()
        } else {
            format!("{} {name}", op.ty.glsl())
        };
        if op.kind == OpKind::Repeat {
            self.repeat(op, &name, &decl, out);
            return;
        }
        let natural = self.natural(op);
        let expr = self.expr(op);
        if natural == Ty::Slot && op.ty != Ty::Slot {
            self.need(Helper::Tone);
        }
        out.push(format!("{decl} = {};", convert(&expr, natural, op.ty)));
    }

    fn repeat(&mut self, op: &Op, name: &str, decl: &str, out: &mut Vec<String>) {
        let ty = op.ty;
        let Some(k) = self.body_of(op) else {
            out.push(format!("{decl} = {};", zero(ty)));
            return;
        };
        let from = self.natural(op);
        let call = convert(&format!("nBody{k}(pos, frame, float(i))"), from, ty);
        if from == Ty::Slot && ty != Ty::Slot {
            self.need(Helper::Tone);
        }
        let count = int(op.imm[2]).max(0);
        let reduce = Reduce::from_value(op.imm[3]);
        out.push(format!("{decl} = {};", zero(ty)));
        if reduce == Reduce::First {
            out.push(format!("bool {name}f = false;"));
        }
        out.push(format!("for (int i = 0; i < {count}; i++) {{"));
        out.push(format!("    {} {name}r = {call};", ty.glsl()));
        match reduce {
            Reduce::Sum => out.push(format!("    {name} += {name}r;")),
            Reduce::Min => out.push(format!(
                "    {name} = i == 0 ? {name}r : min({name}, {name}r);"
            )),
            Reduce::Max => out.push(format!(
                "    {name} = i == 0 ? {name}r : max({name}, {name}r);"
            )),
            Reduce::First => {
                let test = match ty {
                    Ty::Slot => format!("{name}r >= 0"),
                    Ty::Float => format!("{name}r > 0.0"),
                    _ => format!("{name}r.{} > 0.0", SWIZZLE[usize::from(ty.width()) - 1]),
                };
                out.push(format!("    if (!{name}f && {test}) {{"));
                out.push(format!("        {name} = {name}r;"));
                out.push(format!("        {name}f = true;"));
                out.push("    }".to_string());
            }
        }
        out.push("}".to_string());
    }

    fn context(&mut self, context: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for i in 0..self.ir.ops.len() {
            if self.owner[i] == context {
                let op = self.ir.ops[i];
                self.statement(&op, &mut lines);
            }
        }
        lines
    }

    fn sprite(&self, index: usize, sprite: &Sprite, out: &mut String) {
        let (w, h) = (usize::from(sprite.width), usize::from(sprite.height));
        let frames = sprite.frames.len();
        if w == 0 || h == 0 || frames == 0 {
            out.push_str(&format!(
                "int nSprite{index}(vec2 at, float frame) {{\n    return -1;\n}}\n"
            ));
            return;
        }
        let mut cells = Vec::with_capacity(frames * w * h);
        for f in 0..frames {
            for y in 0..h {
                for x in 0..w {
                    cells.push(sprite.pixel(f, x, y).map_or(-1, |c| self.compact_ref(c)));
                }
            }
        }
        if self.dialect == Dialect::Sksl {
            let mut runs: Vec<(usize, i32)> = Vec::new();
            for (i, cell) in cells.iter().enumerate() {
                match runs.last_mut() {
                    Some((end, value)) if *value == *cell => *end = i + 1,
                    _ => runs.push((i + 1, *cell)),
                }
            }
            out.push_str(&format!(
                "int nSprite{index}(vec2 at, float frame) {{
    ivec2 c = ivec2(floor(at));
    if (c.x < 0 || c.y < 0 || c.x >= {w} || c.y >= {h}) {{
        return -1;
    }}
    int i = (nWrap(int(floor(frame)), {frames}) * {h} + c.y) * {w} + c.x;
"
            ));
            let last = runs.len() - 1;
            for (k, (end, value)) in runs.into_iter().enumerate() {
                if k == last {
                    out.push_str(&format!("    return {value};\n}}\n"));
                } else {
                    out.push_str(&format!(
                        "    if (i < {end}) {{\n        return {value};\n    }}\n"
                    ));
                }
            }
            return;
        }
        let n = cells.len();
        out.push_str(&format!("const int nSprite{index}Cells[{n}] = int[{n}]("));
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                out.push_str(if i % w == 0 { ",\n    " } else { ", " });
            }
            out.push_str(&cell.to_string());
        }
        out.push_str(");\n");
        out.push_str(&format!(
            "int nSprite{index}(vec2 at, float frame) {{
    ivec2 c = ivec2(floor(at));
    if (c.x < 0 || c.y < 0 || c.x >= {w} || c.y >= {h}) {{
        return -1;
    }}
    int f = nWrap(int(floor(frame)), {frames});
    return nSprite{index}Cells[(f * {h} + c.y) * {w} + c.x];
}}
"
        ));
    }

    fn textures(&self, out: &mut String) {
        for k in 0..self.ir.textures.len().min(MAX_SAMPLES) {
            let child = sampler_name(self.dialect, k);
            let rect = self.atlas(k);
            out.push_str(&format!(
                "vec4 nTexture{k}(vec2 uv) {{
    vec4 rect = {rect};
    vec2 size = max(rect.zw, vec2(1.0));
    vec2 t = floor(uv * rect.zw);
    t -= size * floor(t / size);
    return vec4(unpremul({child}.eval(rect.xy + t + 0.5)));
}}
"
            ));
        }
    }

    fn fragment(mut self) -> String {
        self.need(Helper::Wrap);
        self.need(Helper::Tone);

        let mut bodies = Vec::new();
        for k in 0..self.bodies.len() {
            let lines = self.context(k + 1);
            bodies.push(lines);
        }
        let main = self.context(0);
        let color = match self.ty(self.ir.color) {
            None => "vec4(0.0)".to_string(),
            Some(_) => self.cast(self.ir.color, Ty::Vec4),
        };
        let result = match self.ir.coverage {
            Some(coverage) if self.ty(coverage).is_some() => {
                let coverage = self.cast(coverage, Ty::Float);
                format!("{coverage} > 0.0 ? {color} : vec4(0.0)")
            }
            _ => color,
        };
        let body_types: Vec<Ty> = (0..self.bodies.len())
            .map(|k| {
                let result = self
                    .ir
                    .ops
                    .iter()
                    .filter(|op| self.body_of(op) == Some(k))
                    .map(Op::body_result)
                    .next()
                    .unwrap_or(Reg::NONE);
                self.ty(result).unwrap_or(Ty::Float)
            })
            .collect();
        let body_results: Vec<String> = (0..self.bodies.len())
            .map(|k| {
                let result = self
                    .ir
                    .ops
                    .iter()
                    .filter(|op| self.body_of(op) == Some(k))
                    .map(Op::body_result)
                    .next()
                    .unwrap_or(Reg::NONE);
                self.cast(result, body_types[k])
            })
            .collect();

        let mut out = String::new();
        self.header(&mut out);
        self.material_uniforms(&mut out);
        let textures = self.ir.textures.len().min(MAX_SAMPLES);
        let sampler = if self.dialect == Dialect::Sksl {
            "shader"
        } else {
            "sampler2D"
        };
        for k in 0..textures {
            out.push_str(&format!(
                "uniform {sampler} {};\n",
                sampler_name(self.dialect, k)
            ));
        }
        if textures > 0 {
            out.push('\n');
        }
        if self.dialect == Dialect::Legacy150 {
            self.legacy_load(&mut out);
        }
        out.push_str(&format!(
            "const float nMinAlpha = {};\n\n",
            float(COVERAGE_THRESHOLD)
        ));
        let tones = tone_count(self.ir);
        for helper in self.needs.clone() {
            helper.code(&mut out, tones, self.dialect);
        }
        if self.dialect == Dialect::Sksl {
            self.textures(&mut out);
        }
        out.push('\n');
        for s in self.sprites.clone() {
            self.sprite(s, &self.ir.sprites[s], &mut out);
        }
        let globals: Vec<(usize, Ty)> = self
            .globals
            .iter()
            .enumerate()
            .filter(|(_, g)| **g)
            .filter_map(|(r, _)| self.types[r].map(|ty| (r, ty)))
            .collect();
        for (r, ty) in &globals {
            out.push_str(&format!("{} r{r};\n", ty.glsl()));
        }
        if !globals.is_empty() {
            out.push('\n');
        }
        for (k, ty) in body_types.iter().enumerate() {
            out.push_str(&format!(
                "{} nBody{k}(vec2 pos, float frame, float index);\n",
                ty.glsl()
            ));
        }
        if !body_types.is_empty() {
            out.push('\n');
        }
        for (k, lines) in bodies.iter().enumerate() {
            out.push_str(&format!(
                "{} nBody{k}(vec2 pos, float frame, float index) {{\n",
                body_types[k].glsl()
            ));
            for line in lines {
                out.push_str(&format!("    {line}\n"));
            }
            out.push_str(&format!("    return {};\n}}\n\n", body_results[k]));
        }
        out.push_str("vec4 material(vec2 pos, float frame) {\n    float index = 0.0;\n");
        for line in &main {
            out.push_str(&format!("    {line}\n"));
        }
        out.push_str(&format!("    return {result};\n}}\n\n"));
        self.emit(&mut out);
        self.main(&mut out);
        out
    }

    fn header(&self, out: &mut String) {
        match self.dialect {
            Dialect::Sksl => {}
            Dialect::Core330 => {
                out.push_str(
                    "#version 330
#extension GL_ARB_separate_shader_objects : require

#include <minecraft:fog.glsl>
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:oit.glsl>
",
                );
                if self.screen {
                    out.push_str("#include <minecraft:globals.glsl>\n");
                }
                out.push_str(
                    "
layout(location = 0) in float sphericalVertexDistance;
layout(location = 1) in float cylindricalVertexDistance;
layout(location = 2) in vec2 pixelPos;
layout(location = 3) in vec4 vertexColor;
layout(location = 4) in vec4 lightColor;

#ifndef OIT_ALPHA_ONLY
layout(location = 0) out vec4 fragColor;
#endif

",
                );
            }
            Dialect::Core330Ubo => {
                out.push_str("#version 150\n\n");
                out.push_str(
                    "in float sphericalVertexDistance;
in float cylindricalVertexDistance;
in vec2 pixelPos;
in vec4 vertexColor;
in vec4 lightColor;

out vec4 fragColor;

",
                );
                out.push_str(UBO_FOG);
                out.push('\n');
                out.push_str(UBO_TRANSFORMS);
                out.push('\n');
                if self.screen {
                    out.push_str(UBO_GLOBALS);
                    out.push('\n');
                }
            }
            Dialect::Legacy150 => {
                out.push_str(
                    "#version 150

in float vertexDistance;
in vec2 pixelPos;
in vec4 vertexColor;
in vec4 lightColor;

out vec4 fragColor;

uniform vec4 ColorModulator;
uniform float FogStart;
uniform float FogEnd;
uniform vec4 FogColor;
",
                );
                if self.screen {
                    out.push_str("uniform vec2 ScreenSize;\n");
                }
                out.push('\n');
            }
            Dialect::Compat130 => {
                out.push_str(
                    "#version 130

in float vertexDistance;
in vec2 pixelPos;
in vec4 vertexColor;
in vec2 lightUv;

uniform sampler2D NameplateLightmap;
uniform int NameplateLit;
uniform int NameplateFogMode;
",
                );
                if self.screen {
                    out.push_str("uniform vec2 ScreenSize;\n");
                }
                out.push('\n');
            }
            Dialect::Es300 => {
                out.push_str(
                    "#version 300 es
precision highp float;
precision highp int;

in vec2 pixelPos;
in vec4 vertexColor;

out vec4 fragColor;

uniform vec4 ColorModulator;
uniform vec4 NameplateLight;
",
                );
                if self.screen {
                    out.push_str("uniform vec2 ScreenSize;\n");
                }
                out.push('\n');
            }
        }
    }

    fn material_uniforms(&self, out: &mut String) {
        let tones = tone_count(self.ir);
        let lanes = LANE_COUNT / 4;
        match self.dialect {
            Dialect::Core330 | Dialect::Core330Ubo | Dialect::Es300 => {
                out.push_str(&format!(
                    "layout(std140) uniform {UNIFORM_BLOCK} {{
    vec4 {AREA_UNIFORM};
    vec4 {BODY_UNIFORM};
    ivec4 {CLOCK_UNIFORM};
    vec4 {LANES_UNIFORM}[{lanes}];
    vec4 {ATLAS_UNIFORM}[{MAX_SAMPLES}];
"
                ));
                if tones > 0 {
                    out.push_str(&format!("    vec4 {TONES_UNIFORM}[{tones}];\n"));
                }
                out.push_str("};\n\n");
            }
            Dialect::Compat130 => {
                out.push_str(&format!(
                    "uniform vec4 {AREA_UNIFORM};
uniform vec4 {BODY_UNIFORM};
uniform ivec4 {CLOCK_UNIFORM};
uniform vec4 {LANES_UNIFORM}[{lanes}];
uniform vec4 {ATLAS_UNIFORM}[{MAX_SAMPLES}];
"
                ));
                if tones > 0 {
                    out.push_str(&format!("uniform vec4 {TONES_UNIFORM}[{tones}];\n"));
                }
                out.push('\n');
            }
            Dialect::Sksl => {
                out.push_str(&format!(
                    "uniform vec4 {AREA_UNIFORM};
uniform vec4 {BODY_UNIFORM};
uniform ivec4 {CLOCK_UNIFORM};
uniform vec4 {LANES_UNIFORM}[{lanes}];
uniform vec4 {ATLAS_UNIFORM}[{MAX_SAMPLES}];
"
                ));
                if tones > 0 {
                    out.push_str(&format!("uniform vec4 {TONES_UNIFORM}[{tones}];\n"));
                }
                out.push_str(&format!("uniform vec4 {SKSL_COLOR_UNIFORM};\n"));
                if self.screen {
                    out.push_str(&format!("uniform mat3 {SKSL_SCREEN_UNIFORM};\n"));
                }
                out.push('\n');
            }
            Dialect::Legacy150 => {
                for uniform in loose_uniforms(self.ir, self.dialect) {
                    let ty = if uniform.ints { "ivec4" } else { "vec4" };
                    out.push_str(&format!("uniform {ty} {};\n", uniform.name));
                }
                out.push('\n');
            }
        }
    }

    fn legacy_load(&self, out: &mut String) {
        let tones = tone_count(self.ir);
        if tones > 0 {
            out.push_str(&format!("vec4 {TONES_UNIFORM}[{tones}];\n\n"));
        }
        out.push_str("void nameplateLoad() {\n");
        for t in 0..tones {
            out.push_str(&format!("    {TONES_UNIFORM}[{t}] = {TONE_UNIFORM}{t};\n"));
        }
        out.push_str("}\n\n");
    }

    fn emit(&self, out: &mut String) {
        let additive = self.ir.blend == Blend::Additive;
        let light = !self.ir.emissive;
        if self.dialect == Dialect::Sksl {
            out.push_str(&format!(
                "vec4 nameplateEmit(vec4 color) {{\n    color *= {SKSL_COLOR_UNIFORM};\n    return vec4(color.rgb * color.a, color.a);\n}}\n\n"
            ));
            return;
        }
        out.push_str("void nameplateEmit(vec4 color) {\n");
        match self.dialect {
            Dialect::Sksl => {}
            Dialect::Core330 => {
                if light {
                    out.push_str("    color.rgb *= lightColor.rgb;\n");
                }
                out.push_str("    color *= vertexColor * ColorModulator;\n");
                out.push_str("#ifdef OIT_ALPHA_ONLY\n    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);\n#else\n");
                if additive {
                    out.push_str("#ifdef OIT_ACCUMULATE\n    color = sampleColorForAccumulation(color);\n#endif\n");
                    out.push_str("    vec4 fogColor = vec4(0.0, 0.0, 0.0, FogColor.a);\n");
                } else {
                    out.push_str("#ifdef OIT_ACCUMULATE\n    color = sampleColorForAccumulation(color);\n    vec4 fogColor = vec4(FogColor.rgb * color.a, FogColor.a);\n#else\n    vec4 fogColor = FogColor;\n#endif\n");
                }
                out.push_str("    fragColor = apply_fog(color, sphericalVertexDistance, cylindricalVertexDistance, FogEnvironmentalStart,\n        FogEnvironmentalEnd, FogRenderDistanceStart, FogRenderDistanceEnd, fogColor);\n#endif\n");
            }
            Dialect::Core330Ubo => {
                if light {
                    out.push_str("    color.rgb *= lightColor.rgb;\n");
                }
                out.push_str("    color *= vertexColor * ColorModulator;\n");
                let fog = if additive {
                    "vec4(0.0, 0.0, 0.0, FogColor.a)"
                } else {
                    "FogColor"
                };
                out.push_str(&format!("    fragColor = nFog(color, {fog});\n"));
            }
            Dialect::Legacy150 => {
                if light {
                    out.push_str("    color.rgb *= lightColor.rgb;\n");
                }
                out.push_str("    color *= vertexColor * ColorModulator;\n");
                let fog = if additive {
                    "vec4(0.0, 0.0, 0.0, FogColor.a)"
                } else {
                    "FogColor"
                };
                out.push_str(&format!(
                    "    vec4 fogColor = {fog};
    if (vertexDistance > FogStart) {{
        float value = vertexDistance < FogEnd ? smoothstep(FogStart, FogEnd, vertexDistance) : 1.0;
        color.rgb = mix(color.rgb, fogColor.rgb, value * fogColor.a);
    }}
    fragColor = color;
"
                ));
            }
            Dialect::Compat130 => {
                if light {
                    out.push_str("    if (NameplateLit != 0) {\n        color.rgb *= texture(NameplateLightmap, lightUv).rgb;\n    }\n");
                }
                out.push_str("    color *= vertexColor;\n");
                out.push_str(
                    "    float fog = 1.0;
    if (NameplateFogMode == 1) {
        fog = clamp((gl_Fog.end - vertexDistance) * gl_Fog.scale, 0.0, 1.0);
    } else if (NameplateFogMode == 2) {
        fog = clamp(exp(-gl_Fog.density * vertexDistance), 0.0, 1.0);
    } else if (NameplateFogMode == 3) {
        float e = gl_Fog.density * vertexDistance;
        fog = clamp(exp(-e * e), 0.0, 1.0);
    }
",
                );
                let fog = if additive {
                    "vec3(0.0)"
                } else {
                    "gl_Fog.color.rgb"
                };
                out.push_str(&format!(
                    "    gl_FragColor = vec4(mix({fog}, color.rgb, fog), color.a);\n"
                ));
            }
            Dialect::Es300 => {
                if light {
                    out.push_str("    color.rgb *= NameplateLight.rgb;\n");
                }
                out.push_str("    fragColor = color * vertexColor * ColorModulator;\n");
            }
        }
        out.push_str("}\n\n");
    }

    fn main(&self, out: &mut String) {
        if self.ir.outline.is_some() {
            out.push_str("bool nCovered(vec2 pos, float frame) {\n    return material(pos, frame).a >= nMinAlpha;\n}\n\n");
        }
        let sksl = self.dialect == Dialect::Sksl;
        let discard = if sksl {
            "return half4(0.0);"
        } else {
            "discard;"
        };
        if sksl {
            out.push_str("half4 main(float2 coord) {\n    vec2 pixelPos = coord;\n");
        } else {
            out.push_str("void main() {\n");
        }
        if self.dialect == Dialect::Legacy150 {
            out.push_str("    nameplateLoad();\n");
        }
        if self.screen && !sksl {
            let inverse = "nInverse(mat2(dFdx(pixelPos), dFdy(pixelPos)))";
            out.push_str(&format!("    nScreenJ = {inverse};\n"));
            out.push_str(&format!("    nFrag = pixelPos - {BODY_UNIFORM}.xy;\n"));
        }
        match self.ir.resolution {
            Resolution::Pixel => out.push_str(&format!(
                "    vec2 pos = floor(pixelPos) + 0.5 - {BODY_UNIFORM}.xy;\n"
            )),
            Resolution::Smooth => {
                out.push_str(&format!("    vec2 pos = pixelPos - {BODY_UNIFORM}.xy;\n"))
            }
        }
        if self.ir.period == Period::Unknown {
            out.push_str(&format!("    float frame = float({CLOCK_UNIFORM}.x);\n"));
        } else {
            out.push_str(&format!(
                "    float frame = float(nWrap({CLOCK_UNIFORM}.x, {CLOCK_UNIFORM}.y));\n"
            ));
        }
        out.push_str("    vec4 color = material(pos, frame);\n");
        out.push_str("    if (color.a < nMinAlpha) {\n");
        match self.ir.outline {
            Some(outline) => {
                let tone = self.compact_ref(outline);
                out.push_str(&format!(
                    "        if (!(nCovered(pos + vec2(1.0, 0.0), frame) || nCovered(pos - vec2(1.0, 0.0), frame)
            || nCovered(pos + vec2(0.0, 1.0), frame) || nCovered(pos - vec2(0.0, 1.0), frame))) {{
            {discard}
        }}
"
                ));
                out.push_str(&format!("        color = nTone({tone});\n"));
            }
            None => out.push_str(&format!("        {discard}\n")),
        }
        if sksl {
            out.push_str("    }\n    return half4(nameplateEmit(color));\n}\n");
        } else {
            out.push_str("    }\n    nameplateEmit(color);\n}\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::nameplate::material::ir::{EdgeLane, Lane, VanillaTexture};

    struct Graph {
        ops: Vec<Op>,
    }

    impl Graph {
        fn new() -> Graph {
            Graph { ops: Vec::new() }
        }

        fn op(&mut self, kind: OpKind, ty: Ty, args: &[Reg], imm: &[f64]) -> Reg {
            let dst = Reg(self.ops.len() as u16);
            let mut a = [Reg::NONE; 4];
            a[..args.len()].copy_from_slice(args);
            let mut i = [0.0; 4];
            i[..imm.len()].copy_from_slice(imm);
            self.ops.push(Op {
                kind,
                ty,
                dst,
                args: a,
                imm: i,
            });
            dst
        }

        fn at(&self) -> f64 {
            self.ops.len() as f64
        }
    }

    const N: Reg = Reg::NONE;

    const GLSL: [Dialect; 5] = [
        Dialect::Core330,
        Dialect::Core330Ubo,
        Dialect::Legacy150,
        Dialect::Compat130,
        Dialect::Es300,
    ];

    fn shimmer() -> MaterialIr {
        let mut g = Graph::new();
        let cell = g.op(OpKind::PixelCell, Ty::Vec2, &[], &[]);
        let x = g.op(OpKind::Component, Ty::Float, &[cell], &[0.0]);
        let frame = g.op(OpKind::Frame, Ty::Float, &[], &[]);
        let tone = g.op(OpKind::Cycle, Ty::Slot, &[x, frame], &[3.0, 4.0]);
        MaterialIr {
            margins: [2, 1, 2, 1],
            outline: Some(ColorRef::slot(3, 0)),
            color: tone,
            tone_slots: vec![3],
            period: Period::Frames(4),
            cost: 12,
            ops: g.ops,
            ..MaterialIr::default()
        }
    }

    fn glow() -> MaterialIr {
        let mut g = Graph::new();
        let uv = g.op(OpKind::TagUv, Ty::Vec2, &[], &[]);
        let u = g.op(OpKind::Component, Ty::Float, &[uv], &[0.0]);
        let time = g.op(OpKind::SteppedTime, Ty::Float, &[], &[]);
        let speed = g.op(OpKind::Param, Ty::Float, &[], &[0.0]);
        let shift = g.op(OpKind::Mul, Ty::Float, &[time, speed], &[]);
        let hue = g.op(OpKind::Add, Ty::Float, &[u, shift], &[]);
        let color = g.op(OpKind::Hsv, Ty::Vec4, &[hue, N, N], &[]);
        MaterialIr {
            resolution: Resolution::Smooth,
            blend: Blend::Additive,
            emissive: true,
            margins: [4, 4, 4, 4],
            color,
            lanes: vec![Lane::Param(0)],
            period: Period::Unknown,
            ops: g.ops,
            ..MaterialIr::default()
        }
    }

    fn everything() -> MaterialIr {
        let mut g = Graph::new();
        let pos = g.op(OpKind::PixelPos, Ty::Vec2, &[], &[]);
        let cell = g.op(OpKind::PixelCell, Ty::Vec2, &[], &[]);
        let uv = g.op(OpKind::TagUv, Ty::Vec2, &[], &[]);
        let area = g.op(OpKind::AreaSize, Ty::Vec2, &[], &[]);
        let body = g.op(OpKind::BodyRect, Ty::Vec4, &[], &[]);
        let frame = g.op(OpKind::Frame, Ty::Float, &[], &[]);
        let secs = g.op(OpKind::SteppedTime, Ty::Float, &[], &[]);
        let screen = g.op(OpKind::ScreenPos, Ty::Vec2, &[], &[]);
        let pscreen = g.op(OpKind::PixelScreenPos, Ty::Vec2, &[], &[]);
        let slot = g.op(OpKind::Slot, Ty::Slot, &[], &[f64::from(2 * 16 + 3)]);
        let c4 = g.op(OpKind::Constant, Ty::Vec4, &[], &[0.25, 0.5, 0.75, 1.0]);
        let c1 = g.op(OpKind::Constant, Ty::Float, &[], &[4.0]);
        let c2 = g.op(OpKind::Constant, Ty::Vec2, &[], &[1.0, -1.0]);
        let param = g.op(OpKind::Param, Ty::Float, &[], &[0.0]);
        let expr = g.op(OpKind::Expr, Ty::Float, &[], &[1.0]);
        let edge = g.op(OpKind::BodyEdge, Ty::Float, &[pos], &[2.0, 1.0]);
        let inner = g.op(OpKind::BodyEdge, Ty::Float, &[N], &[2.0, 0.0]);
        let abs = g.op(OpKind::Abs, Ty::Float, &[param], &[]);
        let floor = g.op(OpKind::Floor, Ty::Vec2, &[uv], &[]);
        let frac = g.op(OpKind::Frac, Ty::Vec2, &[screen], &[]);
        let sin = g.op(OpKind::Sin, Ty::Float, &[frame], &[]);
        let cos = g.op(OpKind::Cos, Ty::Float, &[secs], &[]);
        let exp = g.op(OpKind::Exp, Ty::Float, &[abs], &[]);
        let add = g.op(OpKind::Add, Ty::Vec2, &[pos, expr], &[]);
        let sub = g.op(OpKind::Sub, Ty::Vec2, &[add, area], &[]);
        let mul = g.op(OpKind::Mul, Ty::Vec2, &[sub, frac], &[]);
        let div = g.op(OpKind::Div, Ty::Vec2, &[mul, floor], &[]);
        g.op(OpKind::Mod, Ty::Float, &[frame, c1], &[]);
        let min = g.op(OpKind::Min, Ty::Float, &[sin, cos], &[]);
        let max = g.op(OpKind::Max, Ty::Float, &[min, exp], &[]);
        let step = g.op(OpKind::Step, Ty::Float, &[c1, frame], &[]);
        let pow = g.op(OpKind::Pow, Ty::Float, &[abs, c1], &[]);
        let atan = g.op(OpKind::Atan2, Ty::Float, &[sin, cos], &[]);
        let clamp = g.op(OpKind::Clamp, Ty::Float, &[edge, min, max], &[]);
        let mix = g.op(OpKind::Mix, Ty::Vec4, &[c4, slot, clamp], &[]);
        let smooth = g.op(OpKind::Smoothstep, Ty::Float, &[min, inner, step], &[]);
        let length = g.op(OpKind::Length, Ty::Float, &[div], &[]);
        let dot = g.op(OpKind::Dot, Ty::Float, &[div, pscreen], &[]);
        let polar = g.op(OpKind::Polar, Ty::Vec2, &[div], &[]);
        let combine = g.op(OpKind::Combine, Ty::Vec3, &[length, dot, pow], &[]);
        let component = g.op(OpKind::Component, Ty::Float, &[combine], &[2.0]);
        let select = g.op(OpKind::Select, Ty::Vec4, &[atan, mix, body], &[]);
        let noise = g.op(OpKind::Noise, Ty::Float, &[div], &[7.0, 8.0, 0.0]);
        let fbm = g.op(OpKind::Fbm, Ty::Float, &[polar], &[3.0, 0.0, 0.0, 4.0]);
        let hash = g.op(OpKind::Hash, Ty::Float, &[cell], &[11.0]);
        let bayer = g.op(OpKind::Bayer, Ty::Float, &[N], &[]);
        let checker = g.op(OpKind::Checker, Ty::Float, &[cell], &[2.0]);
        let stripes = g.op(OpKind::Stripes, Ty::Float, &[N], &[4.0, 2.0, 1.0, -1.0]);
        let gradient = g.op(OpKind::Gradient, Ty::Float, &[uv, N, N], &[]);
        let hsv = g.op(OpKind::Hsv, Ty::Vec4, &[noise, fbm, N], &[]);
        let quantize = g.op(OpKind::Quantize, Ty::Slot, &[gradient], &[2.0, 5.0]);
        g.op(OpKind::DitherQuantize, Ty::Slot, &[hash, N], &[5.0, 4.0]);
        g.op(OpKind::Cycle, Ty::Slot, &[checker, N], &[2.0, 6.0]);
        let lighten = g.op(OpKind::Lighten, Ty::Vec4, &[quantize, bayer], &[]);
        let darken = g.op(OpKind::Darken, Ty::Vec4, &[hsv, stripes], &[]);
        let alpha = g.op(OpKind::Alpha, Ty::Vec4, &[lighten, smooth], &[]);

        let start = g.at();
        let local = g.op(OpKind::PixelPos, Ty::Vec2, &[], &[]);
        let sprite = g.op(OpKind::Sprite, Ty::Slot, &[local, N], &[0.0]);
        let end = g.at();
        let sample = g.op(OpKind::SampleAt, Ty::Slot, &[c2, c1, sprite], &[start, end]);

        let start = g.at();
        let index = g.op(OpKind::RepeatIndex, Ty::Float, &[], &[]);
        let moved = g.op(OpKind::Add, Ty::Vec2, &[pos, index], &[]);
        let texel = g.op(OpKind::BundleTexture, Ty::Vec4, &[moved], &[0.0]);
        let end = g.at();
        let repeat = g.op(
            OpKind::Repeat,
            Ty::Vec4,
            &[texel],
            &[start, end, 3.0, Reduce::Max.value()],
        );

        let start = g.at();
        let vanilla = g.op(OpKind::VanillaTexture, Ty::Vec4, &[N], &[1.0]);
        let end = g.at();
        let first = g.op(
            OpKind::Repeat,
            Ty::Vec4,
            &[vanilla],
            &[start, end, 2.0, Reduce::First.value()],
        );
        let atlas = g.op(OpKind::AtlasSprite, Ty::Vec4, &[uv], &[2.0]);
        let tinted = g.op(OpKind::Lighten, Ty::Vec4, &[sample, component], &[]);
        let a = g.op(OpKind::Mix, Ty::Vec4, &[select, darken, clamp], &[]);
        let b = g.op(OpKind::Mix, Ty::Vec4, &[repeat, first, smooth], &[]);
        let c = g.op(OpKind::Mix, Ty::Vec4, &[atlas, tinted, step], &[]);
        let d = g.op(OpKind::Max, Ty::Vec4, &[a, b], &[]);
        let e = g.op(OpKind::Max, Ty::Vec4, &[c, d], &[]);
        let color = g.op(OpKind::Max, Ty::Vec4, &[e, alpha], &[]);
        MaterialIr {
            outline: Some(ColorRef::slot(5, 2)),
            margins: [8, 5, 8, 5],
            color,
            coverage: Some(component),
            lanes: vec![
                Lane::Param(0),
                Lane::Expr(0),
                Lane::BodyEdge(EdgeLane::TopLeft),
                Lane::BodyEdge(EdgeLane::TopRight),
                Lane::BodyEdge(EdgeLane::BottomRight),
                Lane::BodyEdge(EdgeLane::BottomLeft),
                Lane::BodyEdge(EdgeLane::Rings),
            ],
            textures: vec![
                TexRef::Bundle(0),
                TexRef::Vanilla(VanillaTexture::EndPortal),
                TexRef::Atlas("minecraft:block/stone".to_string()),
            ],
            tone_slots: vec![2, 5],
            period: Period::Frames(24),
            cost: 900,
            sprites: vec![Sprite {
                id: "spark".to_string(),
                width: 2,
                height: 2,
                colors: vec![ColorRef::slot(2, 1), ColorRef::slot(5, 0)],
                frames: vec![vec![0, 255, 1, 0], vec![255, 1, 1, 255]],
            }],
            ops: g.ops,
            ..MaterialIr::default()
        }
    }

    fn opaque_hsv() -> MaterialIr {
        let mut g = Graph::new();
        let cell = g.op(OpKind::PixelCell, Ty::Vec2, &[], &[]);
        let hue = g.op(OpKind::Hash, Ty::Float, &[cell], &[5.0]);
        let color = g.op(OpKind::Hsv, Ty::Vec4, &[hue, N, N], &[]);
        MaterialIr {
            color,
            ops: g.ops,
            ..MaterialIr::default()
        }
    }

    fn balanced(text: &str) -> bool {
        let mut depth = [0i32; 3];
        for c in text.chars() {
            let (i, d) = match c {
                '(' => (0, 1),
                ')' => (0, -1),
                '{' => (1, 1),
                '}' => (1, -1),
                '[' => (2, 1),
                ']' => (2, -1),
                _ => continue,
            };
            depth[i] += d;
            if depth[i] < 0 {
                return false;
            }
        }
        depth == [0, 0, 0]
    }

    #[test]
    fn the_everything_fixture_uses_every_op_kind() {
        let used: BTreeSet<&str> = everything().ops.iter().map(|op| op.kind.name()).collect();
        let all: BTreeSet<&str> = OpKind::ALL.iter().map(|k| k.name()).collect();
        assert_eq!(used, all);
    }

    #[test]
    fn every_op_kind_emits_code_in_every_glsl_dialect() {
        let ir = everything();
        for dialect in GLSL {
            let shader = generate(&ir, dialect);
            let f = shader.fragment.as_str();
            assert!(balanced(f), "{dialect:?}");
            assert!(balanced(&shader.vertex), "{dialect:?}");
            assert!(!f.contains("NaN") && !f.contains("inf"), "{dialect:?}");
            let lane = |l: usize| match dialect {
                Dialect::Legacy150 => format!("NameplateLanes{}.{}", l / 4, SWIZZLE[l % 4]),
                _ => format!("NameplateLanes[{}].{}", l / 4, SWIZZLE[l % 4]),
            };
            let atlas = match dialect {
                Dialect::Legacy150 => "NameplateAtlas2".to_string(),
                _ => "NameplateAtlas[2]".to_string(),
            };
            let s = |k: usize| sampler_name(dialect, k);
            let edges = format!("vec4({}, {}, {}, {})", lane(2), lane(3), lane(4), lane(5));
            let expected = [
                "    r0 = pos;".to_string(),
                "vec2 r1 = floor(pos);".to_string(),
                "vec2 r2 = nTagUv(pos);".to_string(),
                "vec2 r3 = (NameplateArea.zw - NameplateArea.xy);".to_string(),
                "vec4 r4 = NameplateBody;".to_string(),
                "float r5 = frame;".to_string(),
                "float r6 = nSeconds(frame);".to_string(),
                "vec2 r7 = nScreen(pos);".to_string(),
                "vec2 r8 = nScreen(floor(pos) + 0.5);".to_string(),
                "int r9 = 3;".to_string(),
                "vec4 r10 = vec4(0.25, 0.5, 0.75, 1.0);".to_string(),
                "float r11 = 4.0;".to_string(),
                "vec2 r12 = vec2(1.0, -1.0);".to_string(),
                format!("float r13 = {};", lane(0)),
                format!("float r14 = {};", lane(1)),
                format!(
                    "float r15 = nBodyEdge(r0 + {r}, (NameplateBody.zw - NameplateBody.xy) + 2.0 * {r}, {edges} + {r});",
                    r = lane(6)
                ),
                format!("float r16 = nBodyEdge(pos, (NameplateBody.zw - NameplateBody.xy), {edges});"),
                "float r17 = abs(r13);".to_string(),
                "vec2 r18 = floor(r2);".to_string(),
                "vec2 r19 = fract(r7);".to_string(),
                "float r20 = sin(r5);".to_string(),
                "float r21 = cos(r6);".to_string(),
                "float r22 = exp(r17);".to_string(),
                "vec2 r23 = (r0 + vec2(r14));".to_string(),
                "vec2 r24 = (r23 - r3);".to_string(),
                "vec2 r25 = (r24 * r19);".to_string(),
                "vec2 r26 = nDiv(r25, r18);".to_string(),
                "float r27 = nMod(r5, r11);".to_string(),
                "float r28 = min(r20, r21);".to_string(),
                "float r29 = max(r28, r22);".to_string(),
                "float r30 = step(r11, r5);".to_string(),
                "float r31 = nPow(r17, r11);".to_string(),
                "float r32 = nAtan2(r20, r21);".to_string(),
                "float r33 = nClamp(r15, r28, r29);".to_string(),
                "vec4 r34 = nMix(r10, nTone(r9), vec4(r33));".to_string(),
                "float r35 = nSmooth(r28, r16, r30);".to_string(),
                "float r36 = length(r26);".to_string(),
                "float r37 = dot(r26, r8);".to_string(),
                "vec2 r38 = nPolar(r26);".to_string(),
                "vec3 r39 = vec3(r36, r37, r31);".to_string(),
                "float r40 = r39.z;".to_string(),
                "vec4 r41 = (r32 > 0.0 ? r4 : r34);".to_string(),
                "float r42 = nNoise(r26, ivec2(8, 0), 7);".to_string(),
                "float r43 = nFbm(r38, 4, ivec2(0, 0), 3);".to_string(),
                "float r44 = nHashAt(r1, 11);".to_string(),
                "float r45 = nBayer(floor(pos));".to_string(),
                "float r46 = nChecker(r1, 2);".to_string(),
                "float r47 = nStripes(floor(pos), 4, 2, ivec2(1, -1));".to_string(),
                "float r48 = nGradient(r2, vec2(0.0), vec2(1.0, 0.0));".to_string(),
                "vec4 r49 = nHsv(r42, r43, 1.0);".to_string(),
                "int r50 = nQuantize(r48, 0, 5);".to_string(),
                "int r51 = nDither(r44, floor(pos), 16, 4);".to_string(),
                "int r52 = nCycle(r46, frame, 0, 6);".to_string(),
                "vec4 r53 = nLighten(nTone(r50), r45);".to_string(),
                "vec4 r54 = nDarken(r49, r47);".to_string(),
                "vec4 r55 = nAlpha(r53, r35);".to_string(),
                "int nBody0(vec2 pos, float frame, float index) {\n    vec2 r56 = pos;\n    int r57 = nSprite0(r56, 0.0);\n    return r57;\n}".to_string(),
                "int r58 = nBody0(pos + r12, frame + r11, index);".to_string(),
                format!("vec4 nBody1(vec2 pos, float frame, float index) {{\n    float r59 = index;\n    vec2 r60 = (r0 + vec2(r59));\n    vec4 r61 = nSample({}, r60);\n    return r61;\n}}", s(0)),
                "    vec4 r62 = vec4(0.0);\n    for (int i = 0; i < 3; i++) {\n        vec4 r62r = nBody1(pos, frame, float(i));\n        r62 = i == 0 ? r62r : max(r62, r62r);\n    }".to_string(),
                format!("vec4 r63 = nSample({}, nTagUv(pos));", s(1)),
                "    bool r64f = false;\n    for (int i = 0; i < 2; i++) {\n        vec4 r64r = nBody2(pos, frame, float(i));\n        if (!r64f && r64r.w > 0.0) {\n            r64 = r64r;\n            r64f = true;\n        }\n    }".to_string(),
                format!("vec4 r65 = nAtlas({}, {atlas}, r2);", s(2)),
                "vec4 r66 = nLighten(nTone(r58), r40);".to_string(),
                "return r40 > 0.0 ? r72 : vec4(0.0);".to_string(),
                "const int nSprite0Cells[8] = int[8](1, -1,\n    16, 1,\n    -1, 16,\n    16, -1);".to_string(),
                "int f = nWrap(int(floor(frame)), 2);".to_string(),
                "vec2 r0;".to_string(),
                "color = nTone(18);".to_string(),
            ];
            for needle in expected {
                let holds = f.contains(&needle);
                assert!(holds, "{dialect:?} is missing:\n{needle}\n\n{f}");
            }
            for op in &ir.ops {
                let r = op.dst.0;
                let count = f.matches(&format!(" r{r} = ")).count();
                if op.kind == OpKind::Repeat {
                    assert!(count >= 2, "{dialect:?} r{r}");
                } else {
                    assert_eq!(count, 1, "{dialect:?} assigns r{r} once");
                }
            }
        }
    }

    fn portable() -> MaterialIr {
        let mut ir = everything();
        for op in &mut ir.ops {
            if matches!(op.kind, OpKind::Noise | OpKind::Fbm | OpKind::Hash) {
                op.kind = OpKind::Constant;
                op.args = [Reg::NONE; 4];
                op.imm = [0.5, 0.0, 0.0, 0.0];
            }
        }
        ir
    }

    fn fixture(name: &str) -> MaterialIr {
        let mut g = Graph::new();
        let (color, tone_slots) = match name {
            "shimmer" => return shimmer(),
            "glow" => return glow(),
            "everything" => return everything(),
            "portable" => return portable(),
            "still" => {
                return MaterialIr {
                    period: Period::Still,
                    ..shimmer()
                }
            }
            "defaults" => {
                let a = g.op(OpKind::Gradient, Ty::Float, &[N, N, N], &[]);
                let b = g.op(OpKind::Hsv, Ty::Vec4, &[a, N, N], &[]);
                g.op(OpKind::Cycle, Ty::Slot, &[a, N], &[1.0, 3.0]);
                g.op(OpKind::Sprite, Ty::Slot, &[N, N], &[4.0]);
                g.op(OpKind::Add, Ty::Vec3, &[N, a], &[]);
                g.op(OpKind::Quantize, Ty::Slot, &[a], &[9.0, 3.0]);
                (b, vec![1])
            }
            _ => return opaque_hsv(),
        };
        MaterialIr {
            color,
            tone_slots,
            ops: g.ops,
            ..MaterialIr::default()
        }
    }

    #[test]
    fn generated_shaders_contain_their_statements() {
        for row in [
            "shimmer all + vec2 pos = floor(pixelPos) + 0.5 - NameplateBody.xy;",
            "glow all + vec2 pos = pixelPos - NameplateBody.xy;",
            "shimmer all + float frame = float(nWrap(NameplateClock.x, NameplateClock.y));",
            "still all + float frame = float(nWrap(NameplateClock.x, NameplateClock.y));",
            "glow all + float frame = float(NameplateClock.x);",
            "shimmer all + return material(pos, frame).a >= nMinAlpha;",
            "glow all - nCovered",
            "glow glsl + if (color.a < nMinAlpha) {\n        discard;\n    }",
            "glow sksl + if (color.a < nMinAlpha) {\n        return half4(0.0);\n    }",
            "opaque es_300 +     return r2;\n}",
            "shimmer core_330,core_330_ubo,legacy_150 + color.rgb *= lightColor.rgb;",
            "glow core_330,core_330_ubo,legacy_150 - color.rgb *= lightColor.rgb;",
            "glow core_330,legacy_150 + vec4 fogColor = vec4(0.0, 0.0, 0.0, FogColor.a);",
            "glow core_330_ubo + nFog(color, vec4(0.0, 0.0, 0.0, FogColor.a))",
            "shimmer compat_130 + texture(NameplateLightmap, lightUv)",
            "glow compat_130 - texture(NameplateLightmap, lightUv)",
            "glow compat_130 + mix(vec3(0.0), color.rgb, fog)",
            "shimmer es_300 + color.rgb *= NameplateLight.rgb;",
            "glow es_300 + fragColor = color * vertexColor * ColorModulator;",
            "everything core_330,core_330_ubo,es_300 +     vec4 NameplateTones[32];\n",
            "shimmer core_330,core_330_ubo,es_300 +     vec4 NameplateTones[16];\n",
            "glow core_330,core_330_ubo,es_300 + vec4 nTone(int ref) {\n    return vec4(0.0);\n}",
            "everything core_330_ubo + #ifdef NAMEPLATE_SCREEN_FIRST",
            "everything core_330_ubo + nScreenJ = nInverse(mat2(dFdx(pixelPos), dFdy(pixelPos)));",
            "everything core_330 + #include <minecraft:globals.glsl>",
            "everything legacy_150,compat_130,es_300 + uniform vec2 ScreenSize;",
            "shimmer all - ScreenSize",
            "defaults es_300 + float r0 = nGradient(nTagUv(pos), vec2(0.0), vec2(1.0, 0.0));",
            "defaults es_300 + vec4 r1 = nHsv(r0, 1.0, 1.0);",
            "defaults es_300 + int r2 = nCycle(r0, frame, 0, 3);",
            "defaults es_300 + vec3 r4 = (vec3(0.0) + vec3(r0));",
            "portable sksl + half4 main(float2 coord) {\n    vec2 pixelPos = coord;\n",
            "portable sksl + uniform ivec4 NameplateClock;\nuniform vec4 NameplateLanes[4];\nuniform vec4 NameplateAtlas[4];\nuniform vec4 NameplateTones[32];\nuniform vec4 NameplateColor;\nuniform mat3 NameplateScreen;\n",
            "portable sksl + vec4 r61 = nTexture0(r60);",
            "portable sksl + vec4 r65 = nTexture2(r2);",
            "portable sksl + int r58 = nBody0(pos + r12, frame + r11, index);",
            "portable sksl +     color *= NameplateColor;\n    return vec4(color.rgb * color.a, color.a);",
            "portable sksl +     return half4(nameplateEmit(color));\n}",
            "glow sksl - NameplateScreen",
        ] {
            let mut words = row.splitn(4, ' ');
            let (name, dialects, sign) = (words.next().unwrap(), words.next().unwrap(), words.next().unwrap());
            let needle = words.next().unwrap();
            let chosen: Vec<Dialect> = match dialects {
                "all" => Dialect::ALL.to_vec(),
                "glsl" => GLSL.to_vec(),
                names => names.split(',').map(|n| Dialect::parse(n).unwrap()).collect(),
            };
            for dialect in chosen {
                let f = generate(&fixture(name), dialect).fragment;
                assert_eq!(f.contains(needle), sign == "+", "{row}\n{dialect:?}:\n{f}");
            }
        }
        let sksl = generate(&portable(), Dialect::Sksl).fragment;
        let banned = "#version|#include|#ifdef|discard|texelFetch|textureSize|texture(|sampler2D|dFdx|gl_|uint| % | ^ |>>|int[|Cells|layout(|precision|clamp(int(|lightColor|Fog|ScreenSize";
        for banned in banned.split('|') {
            assert!(!sksl.contains(banned), "SkSL contains {banned}");
        }
    }

    #[test]
    fn vertex_stages_feed_the_fragment_inputs() {
        for dialect in GLSL {
            let shader = generate(&shimmer(), dialect);
            assert_eq!(shader.vertex, vertex(dialect));
            for line in shader.fragment.lines() {
                let input = line
                    .strip_prefix("in ")
                    .or_else(|| line.split_once(") in ").map(|(_, rest)| rest));
                if let Some(input) = input {
                    let name = input.trim_end_matches(';').rsplit(' ').next().unwrap();
                    let holds = shader.vertex.contains(&format!(" {name};"));
                    assert!(holds, "{dialect:?} vertex lacks {name}");
                }
            }
        }
        let sksl = generate(&opaque_hsv(), Dialect::Sksl);
        assert!(sksl.vertex.is_empty() && !sksl.depth_prepass);
    }

    #[test]
    fn outlines_run_the_graph_five_times() {
        for dialect in Dialect::ALL {
            let f = generate(&shimmer(), dialect).fragment;
            let main = &f[f.find(" main(").unwrap()..];
            let count = |needle: &str| main.matches(needle).count();
            let counts = [
                count("material(pos, frame)"),
                count("nCovered(pos "),
                count("discard;") + count("return half4(0.0);"),
            ];
            assert_eq!(counts, [1, 4, 1], "{dialect:?}");
        }
        assert_eq!(float(COVERAGE_THRESHOLD), "0.0019607844");
    }

    #[test]
    fn opaque_materials_ask_for_a_depth_prepass() {
        assert!(generate(&opaque_hsv(), Dialect::Es300).depth_prepass);
        assert!(!generate(&shimmer(), Dialect::Core330).depth_prepass);
        assert!(!generate(&glow(), Dialect::Core330).depth_prepass);
        let mut g = Graph::new();
        let solid = g.op(OpKind::Constant, Ty::Vec4, &[], &[1.0, 0.0, 0.0, 1.0]);
        let glass = g.op(OpKind::Constant, Ty::Vec4, &[], &[1.0, 0.0, 0.0, 0.5]);
        let t = g.op(OpKind::Frame, Ty::Float, &[], &[]);
        let lit = g.op(OpKind::Darken, Ty::Vec4, &[solid, t], &[]);
        let faded = g.op(OpKind::Mix, Ty::Vec4, &[solid, glass, t], &[]);
        let ops = g.ops;
        let with = |color| {
            opaque(&MaterialIr {
                color,
                ops: ops.clone(),
                ..MaterialIr::default()
            })
        };
        assert_eq!([lit, faded, Reg::NONE].map(with), [true, false, false]);
    }

    #[test]
    fn samplers_uniforms_and_keys_follow_the_host_layout() {
        let ir = everything();
        let units = |d| {
            generate(&ir, d)
                .samplers
                .iter()
                .map(|s| format!("{}@{}", s.name, s.unit))
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            units(Dialect::Core330),
            "NameplateSampler0@0 NameplateSampler1@1 NameplateSampler2@2"
        );
        assert_eq!(
            units(Dialect::Compat130),
            "NameplateSampler0@0 NameplateSampler1@2 NameplateSampler2@3"
        );
        let actual = &units(Dialect::Legacy150);
        assert_eq!(*actual, "Sampler0@0 Sampler1@1 Sampler3@3");
        let mut keys = BTreeSet::new();
        for dialect in Dialect::ALL {
            let shader = generate(&ir, dialect);
            let actual = &(shader.uniform_words, shader.tones.as_slice());
            assert_eq!(*actual, (44 + 2 * 64, &[2, 5][..]));
            assert_eq!(shader, generate(&ir, dialect));
            assert!(
                shader.key.starts_with(dialect.as_str())
                    && shader.key.len() == dialect.as_str().len() + 17
            );
            keys.insert(shader.key);
            keys.insert(generate(&shimmer(), dialect).key);
        }
        assert_eq!(keys.len(), 12);
        for (dialect, count) in [
            (Dialect::Core330Ubo, 0),
            (Dialect::Compat130, 6),
            (Dialect::Legacy150, 43),
        ] {
            let list = loose_uniforms(&ir, dialect);
            let mut words: Vec<u32> = list
                .iter()
                .flat_map(|u| u.word..u.word + 4 * u.count)
                .collect();
            words.sort_unstable();
            assert!(
                list.len() == count
                    && (count == 0 || words == (0..ir.uniform_words()).collect::<Vec<_>>())
            );
            let f = generate(&ir, dialect).fragment;
            for u in list {
                let array = if u.count > 1 {
                    format!("[{}]", u.count)
                } else {
                    String::new()
                };
                let ty = if u.ints { "ivec4" } else { "vec4" };
                let holds = f.contains(&format!("uniform {ty} {}{array};", u.name));
                assert!(holds, "{}", u.name);
            }
        }
        let clock = &loose_uniforms(&ir, Dialect::Compat130)[2];
        let actual = &(clock.name.as_str(), clock.ints, clock.word);
        assert_eq!(*actual, ("NameplateClock", true, 8));
        let actual = &loose_uniforms(&ir, Dialect::Legacy150)[42].name;
        assert_eq!(*actual, "NameplateTone31");
        assert_eq!([float(1e20), float(f64::NAN)], ["1e20", "0.0"]);
        let actual = &ir.ops.iter().filter_map(Op::body).collect::<Vec<_>>();
        assert_eq!(*actual, [56..58, 59..62, 63..64]);
    }

    #[test]
    fn sksl_flags_ops_it_cannot_express() {
        let ir = portable();
        assert!(unsupported(&ir, Dialect::Sksl).is_empty());
        let f = generate(&ir, Dialect::Sksl).fragment;
        assert!(balanced(&f));
        for op in &ir.ops {
            let count = f.matches(&format!(" r{} = ", op.dst.0)).count();
            let holds = if op.kind == OpKind::Repeat {
                count >= 2
            } else {
                count == 1
            };
            assert!(holds, "r{}", op.dst.0);
        }
        assert!(GLSL
            .iter()
            .all(|d| unsupported(&everything(), *d).is_empty()));
        let actual = &unsupported(&everything(), Dialect::Sksl);
        assert_eq!(*actual, [OpKind::Noise, OpKind::Fbm, OpKind::Hash]);
        assert_eq!(unsupported(&opaque_hsv(), Dialect::Sksl), [OpKind::Hash]);
        assert!(unsupported(&shimmer(), Dialect::Sksl).is_empty());
        let mut busy = portable();
        let side = 64usize;
        let frames = vec![(0..side * side).map(|i| (i % 2) as u8).collect()];
        busy.sprites[0] = Sprite {
            id: "busy".to_string(),
            width: side as u8,
            height: side as u8,
            colors: vec![ColorRef::slot(2, 0), ColorRef::slot(2, 1)],
            frames,
        };
        assert!(sprite_runs(&busy.sprites[0]).len() > SKSL_SPRITE_RUNS);
        assert_eq!(unsupported(&busy, Dialect::Sksl), [OpKind::Sprite]);
        assert_eq!(sprite_runs(&portable().sprites[0]).len(), 7);
        assert!(reads_screen(&everything()) && !reads_screen(&shimmer()));
    }

    fn glslang() -> bool {
        Command::new("glslangValidator")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    const STUB_FOG: &str = "layout(std140) uniform Fog {
    vec4 FogColor;
    float FogEnvironmentalStart;
    float FogEnvironmentalEnd;
    float FogRenderDistanceStart;
    float FogRenderDistanceEnd;
    float FogSkyEnd;
    float FogCloudsEnd;
};
vec4 apply_fog(vec4 c, float s, float y, float es, float ee, float rs, float re, vec4 f) {
    return vec4(mix(c.rgb, f.rgb, clamp(max(s - es, y - rs), 0.0, 1.0) * f.a), c.a);
}
float fog_spherical_distance(vec3 p) {
    return length(p);
}
float fog_cylindrical_distance(vec3 p) {
    return max(length(p.xz), abs(p.y));
}
";

    fn stub(name: &str) -> &'static str {
        match name {
            "fog" => STUB_FOG,
            "dynamictransforms" => "layout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n};\n",
            "projection" => "layout(std140) uniform Projection {\n    mat4 ProjMat;\n};\n",
            "sample_lightmap" => "vec4 sample_lightmap(sampler2D m, ivec2 uv) {\n    return texture(m, vec2(uv) / 256.0);\n}\n",
            "globals" => "layout(std140) uniform Globals {\n    ivec3 CameraBlockPos;\n    float GlintAlpha;\n    vec3 CameraOffset;\n    float GameTime;\n    vec2 ScreenSize;\n    int MenuBlurRadius;\n    int UseRgss;\n};\n",
            "oit" => "void executeAlphaOnlyPhase(float depth, float alpha) {\n}\n",
            other => panic!("no stub for {other}"),
        }
    }

    fn inline_includes(source: &str) -> String {
        let mut out = String::new();
        for line in source.lines() {
            match line
                .strip_prefix("#include <minecraft:")
                .and_then(|rest| rest.strip_suffix(".glsl>"))
            {
                Some(name) => out.push_str(stub(name)),
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        out
    }

    fn validate(shader: &ShaderSource, dialect: Dialect, extra: &[&str]) -> Result<String, String> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "plusrenderer-glsl-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let vert = dir.join("material.vert");
        let frag = dir.join("material.frag");
        let prepare = |s: &str| {
            if dialect == Dialect::Core330 {
                inline_includes(s)
            } else {
                s.to_string()
            }
        };
        fs::write(&vert, prepare(&shader.vertex)).map_err(|e| e.to_string())?;
        fs::write(&frag, prepare(&shader.fragment)).map_err(|e| e.to_string())?;
        let output = Command::new("glslangValidator")
            .args(extra)
            .arg(&vert)
            .arg(&frag)
            .current_dir(&dir)
            .output()
            .map_err(|e| e.to_string())?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = fs::remove_dir_all(&dir);
        if output.status.success() && !text.contains("ERROR") {
            Ok(text)
        } else {
            Err(text)
        }
    }

    #[test]
    fn glslang_compiles_links_and_reflects_every_glsl_dialect() {
        if !glslang() {
            eprintln!("glslangValidator is not on PATH; skipping");
            return;
        }
        for (name, ir) in [
            ("shimmer", shimmer()),
            ("glow", glow()),
            ("opaque", opaque_hsv()),
            ("everything", everything()),
        ] {
            for dialect in GLSL {
                let shader = generate(&ir, dialect);
                if let Err(log) = validate(&shader, dialect, &["-l"]) {
                    panic!("{name} {dialect:?}:\n{log}\n{}", shader.fragment);
                }
            }
        }
        let spirv =
            std::env::temp_dir().join(format!("plusrenderer-glsl-{}.spv", std::process::id()));
        let out = spirv.to_string_lossy().to_string();
        let mut args = vec![
            "-l",
            "-G",
            "--auto-map-bindings",
            "--auto-map-locations",
            "-o",
        ];
        args.push(&out);
        let result = validate(
            &generate(&everything(), Dialect::Core330),
            Dialect::Core330,
            &args,
        );
        let _ = fs::remove_file(&spirv);
        result.unwrap_or_else(|log| panic!("core_330 as SPIR-V:\n{log}"));

        for dialect in [Dialect::Core330, Dialect::Core330Ubo, Dialect::Es300] {
            let shader = generate(&everything(), dialect);
            let log = validate(&shader, dialect, &["-l", "-q"]).unwrap();
            let line = |name: &str| {
                log.lines()
                    .find(|l| l.starts_with(name))
                    .unwrap_or_else(|| panic!("{dialect:?} has no {name}\n{log}"))
                    .to_string()
            };
            for (name, word) in [
                ("NameplateArea", 0),
                ("NameplateBody", 4),
                ("NameplateClock", 8),
                ("NameplateLanes", 12),
                ("NameplateAtlas", 28),
                ("NameplateTones", 44),
            ] {
                assert!(
                    line(&format!("{name}: offset"))
                        .starts_with(&format!("{name}: offset {},", word * 4)),
                    "{dialect:?} {name}"
                );
            }
            assert!(
                line("NameplateMaterial:").contains(&format!("size {},", shader.uniform_words * 4)),
                "{dialect:?}"
            );
        }
    }
}
