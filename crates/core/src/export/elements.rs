use core::f32::consts::PI;

use libm::{asinf, atan2f, cosf, fabsf, round, sinf};
use serde_json::{json, Map, Value};

use super::{
    ConversionReport, LossKind, Rotations, Severity, Target, ANGLE_LIMIT, ANGLE_STEPS,
    MODEL_CENTER, MODEL_MAX, MODEL_MIN, MODEL_SPAN, TEXTURE_KEY,
};
use crate::ir::{Face, FaceUv, Model};
use crate::math::Affine;

pub type Mat3 = [[f32; 3]; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    None,
    Axis(usize, f32),
    Euler([f32; 3]),
}

const FLIP: [f32; 3] = [-1.0, 1.0, -1.0];
const EXACT: f32 = 1e-4;
const BOUNDS_SLACK: f32 = 1e-3;
const AXES: [&str; 3] = ["x", "y", "z"];
const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const JAVA_FACES: [(&str, Face); 6] = [
    ("north", Face::South),
    ("east", Face::West),
    ("south", Face::North),
    ("west", Face::East),
    ("up", Face::Up),
    ("down", Face::Down),
];

fn rad(deg: f32) -> f32 {
    deg * PI / 180.0
}

fn deg(rad: f32) -> f32 {
    rad * 180.0 / PI
}

fn affine(m: &Mat3) -> Affine {
    let [a, b, c] = *m;
    Affine([
        a[0], a[1], a[2], 0.0, b[0], b[1], b[2], 0.0, c[0], c[1], c[2], 0.0,
    ])
}

fn linear(a: &Affine) -> Mat3 {
    let m = &a.0;
    [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]]
}

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0f32; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    out
}

fn axis_matrix(axis: usize, degrees: f32) -> Mat3 {
    let (s, c) = (sinf(rad(degrees)), cosf(rad(degrees)));
    match axis {
        0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

pub fn euler_matrix(xyz: [f32; 3]) -> Mat3 {
    let x = axis_matrix(0, xyz[0]);
    let y = axis_matrix(1, xyz[1]);
    let z = axis_matrix(2, xyz[2]);
    mul(&z, &mul(&y, &x))
}

pub fn turn_matrix(turn: Turn) -> Mat3 {
    match turn {
        Turn::None => IDENTITY,
        Turn::Axis(axis, degrees) => axis_matrix(axis, degrees),
        Turn::Euler(xyz) => euler_matrix(xyz),
    }
}

fn bedrock(rotation: [f32; 3]) -> Affine {
    let x = axis_matrix(0, rotation[0]);
    let y = axis_matrix(1, -rotation[1]);
    let z = axis_matrix(2, -rotation[2]);
    affine(&mul(&z, &mul(&y, &x)))
}

fn rest_world(model: &Model) -> Vec<Affine> {
    let mut out: Vec<Affine> = Vec::with_capacity(model.bones.len());
    for bone in &model.bones {
        let local = Affine::about(bone.pivot, &bedrock(bone.rotation));
        let parent = bone.parent.map_or(Affine::IDENTITY, |p| out[p]);
        out.push(parent.mul(&local));
    }
    out
}

fn distance(a: &Mat3, b: &Mat3) -> f32 {
    let mut worst = 0.0f32;
    for r in 0..3 {
        for c in 0..3 {
            worst = worst.max(fabsf(a[r][c] - b[r][c]));
        }
    }
    worst
}

fn score(r: &Mat3, m: &Mat3) -> f32 {
    let mut sum = 0.0;
    for i in 0..3 {
        for j in 0..3 {
            sum += r[i][j] * m[i][j];
        }
    }
    sum
}

fn best_angle(r: &Mat3, axis: usize) -> f32 {
    let (s, c) = match axis {
        0 => (r[2][1] - r[1][2], r[1][1] + r[2][2]),
        1 => (r[0][2] - r[2][0], r[0][0] + r[2][2]),
        _ => (r[1][0] - r[0][1], r[0][0] + r[1][1]),
    };
    deg(atan2f(s, c))
}

fn circular(a: f32, b: f32) -> f32 {
    let d = fabsf(a - b) % 360.0;
    d.min(360.0 - d)
}

fn allowed(theta: f32, rule: Rotations) -> f32 {
    match rule {
        Rotations::Free => theta,
        Rotations::Clamped => theta.clamp(-ANGLE_LIMIT, ANGLE_LIMIT),
        Rotations::Steps => {
            let mut best = ANGLE_STEPS[0];
            for step in ANGLE_STEPS {
                if circular(theta, step) < circular(theta, best) {
                    best = step;
                }
            }
            best
        }
    }
}

pub fn euler_angles(r: &Mat3) -> [f32; 3] {
    let sy = (-r[2][0]).clamp(-1.0, 1.0);
    let y = asinf(sy);
    if fabsf(sy) < 0.99999 {
        [
            deg(atan2f(r[2][1], r[2][2])),
            deg(y),
            deg(atan2f(r[1][0], r[0][0])),
        ]
    } else {
        [0.0, deg(y), deg(atan2f(-r[0][1], r[1][1]))]
    }
}

pub fn fit_rotation(r: &Mat3, rule: Rotations) -> (Turn, bool) {
    if distance(r, &IDENTITY) < EXACT {
        return (Turn::None, true);
    }
    let mut best = Turn::None;
    let mut best_score = score(r, &IDENTITY);
    for axis in 0..3 {
        let angle = allowed(best_angle(r, axis), rule);
        let s = score(r, &axis_matrix(axis, angle));
        if s > best_score + 1e-6 {
            best = Turn::Axis(axis, angle);
            best_score = s;
        }
    }
    if let Turn::Axis(_, angle) = best {
        if fabsf(angle) < EXACT {
            best = Turn::None;
        }
    }
    let exact = distance(r, &turn_matrix(best)) < EXACT;
    if !exact && rule == Rotations::Free {
        return (Turn::Euler(euler_angles(r)), true);
    }
    (best, exact)
}

pub fn tidy(v: f32) -> f64 {
    let r = round(f64::from(v) * 10_000.0) / 10_000.0;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

pub fn vec3(v: [f32; 3]) -> Value {
    json!([tidy(v[0]), tidy(v[1]), tidy(v[2])])
}

fn describe(turn: Turn) -> String {
    match turn {
        Turn::None => "none".to_string(),
        Turn::Axis(axis, angle) => format!("{} {}", AXES[axis], tidy(angle)),
        Turn::Euler([x, y, z]) => format!("x {} y {} z {}", tidy(x), tidy(y), tidy(z)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub center: [f32; 3],
}

impl Fit {
    pub const IDENTITY: Fit = Fit {
        scale: 1.0,
        center: MODEL_CENTER,
    };

    pub fn apply(&self, q: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| self.scale * (q[i] - self.center[i]) + MODEL_CENTER[i])
    }

    pub fn anchor(&self) -> [f32; 3] {
        self.apply(MODEL_CENTER)
    }
}

struct Part {
    center: [f32; 3],
    half: [f32; 3],
    turn: Turn,
    faces: Vec<(&'static str, [f32; 4])>,
    light: Option<u8>,
}

pub struct Built {
    pub elements: Vec<Value>,
    pub fit: Fit,
}

fn java_uv(face: Face, uv: FaceUv, texture_size: [f32; 2]) -> [f32; 4] {
    let scale = |size: f32| if size > 0.0 { 16.0 / size } else { 1.0 };
    let (ku, kv) = (scale(texture_size[0]), scale(texture_size[1]));
    let u0 = uv.uv[0] * ku;
    let v0 = uv.uv[1] * kv;
    let u1 = (uv.uv[0] + uv.size[0]) * ku;
    let v1 = (uv.uv[1] + uv.size[1]) * kv;
    match face {
        Face::Up | Face::Down => [u1, v1, u0, v0],
        _ => [u0, v0, u1, v1],
    }
}

fn glow_levels(model: &Model) -> Vec<Option<u8>> {
    let mut out: Vec<Option<u8>> = Vec::with_capacity(model.bones.len());
    for bone in &model.bones {
        let inherited = bone.parent.and_then(|p| out[p]);
        out.push(bone.glow.or(inherited));
    }
    out
}

pub fn build(model: &Model, anchor: [f32; 3], target: Target, r: &mut ConversionReport) -> Built {
    let world = rest_world(model);
    let glow = glow_levels(model);
    let mut parts = Vec::new();

    for (b, bone) in model.bones.iter().enumerate() {
        let mut drawn = false;
        for (c, cube) in bone.cubes.iter().enumerate() {
            if cube.faces.iter().all(Option::is_none) {
                continue;
            }
            drawn = true;
            let a = world[b].mul(&Affine::about(cube.pivot, &bedrock(cube.rotation)));
            let l = linear(&a);
            let java: Mat3 =
                std::array::from_fn(|i| std::array::from_fn(|j| FLIP[i] * FLIP[j] * l[i][j]));

            let i = cube.inflate;
            let mut mid = [0.0f32; 3];
            let mut half = [0.0f32; 3];
            for axis in 0..3 {
                let p = cube.origin[axis] - i;
                let q = cube.origin[axis] + cube.size[axis] + i;
                mid[axis] = (p + q) * 0.5;
                half[axis] = fabsf(q - p) * 0.5;
            }
            let w = a.point(mid);
            let center = std::array::from_fn(|k| FLIP[k] * (w[k] - anchor[k]) + MODEL_CENTER[k]);

            let (turn, exact) = fit_rotation(&java, target.rotations);
            if !exact {
                r.push(
                    LossKind::Rotation,
                    Severity::Approximated,
                    format!("{}/cubes/{c}", bone.name),
                    format!(
                        "needs {}; exported {}",
                        describe(Turn::Euler(euler_angles(&java))),
                        describe(turn)
                    ),
                );
            }

            let faces = JAVA_FACES
                .iter()
                .filter_map(|(name, face)| {
                    cube.faces[face.index()]
                        .map(|uv| (*name, java_uv(*face, uv, model.texture_size)))
                })
                .collect();

            parts.push(Part {
                center,
                half,
                turn,
                faces,
                light: glow[b].filter(|l| *l > 0),
            });
        }
        if let Some(level) = glow[b].filter(|l| *l > 0 && drawn) {
            if target.light_emission {
                r.push(
                    LossKind::Emissive,
                    Severity::Approximated,
                    bone.name.as_str(),
                    format!("glow {level} becomes light_emission {level}"),
                );
            } else {
                r.push(
                    LossKind::Emissive,
                    Severity::Dropped,
                    bone.name.as_str(),
                    format!("glow {level} has no equivalent"),
                );
            }
        }
    }

    let fit = fit_bounds(&parts, r);
    let elements = parts.iter().map(|p| element(p, &fit, target)).collect();
    Built { elements, fit }
}

fn fit_bounds(parts: &[Part], r: &mut ConversionReport) -> Fit {
    if parts.is_empty() {
        return Fit::IDENTITY;
    }
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in parts {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(p.center[axis] - p.half[axis]);
            hi[axis] = hi[axis].max(p.center[axis] + p.half[axis]);
        }
    }
    let inside =
        (0..3).all(|k| lo[k] >= MODEL_MIN - BOUNDS_SLACK && hi[k] <= MODEL_MAX + BOUNDS_SLACK);
    if inside {
        return Fit::IDENTITY;
    }
    let extent: [f32; 3] = std::array::from_fn(|k| hi[k] - lo[k]);
    let widest = extent[0].max(extent[1]).max(extent[2]);
    let mid: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5);
    if widest <= MODEL_SPAN + BOUNDS_SLACK {
        let center = std::array::from_fn(|k| {
            if lo[k] >= MODEL_MIN - BOUNDS_SLACK && hi[k] <= MODEL_MAX + BOUNDS_SLACK {
                MODEL_CENTER[k]
            } else {
                mid[k]
            }
        });
        return Fit { scale: 1.0, center };
    }
    let scale = MODEL_SPAN / widest;
    r.push(
        LossKind::Bounds,
        Severity::Adjusted,
        "geometry",
        format!(
            "{} x {} x {} exceeds {MODEL_SPAN}; scaled by {}, scale the display by {} to restore",
            tidy(extent[0]),
            tidy(extent[1]),
            tidy(extent[2]),
            tidy(scale),
            tidy(1.0 / scale)
        ),
    );
    Fit { scale, center: mid }
}

fn bounded(v: [f32; 3]) -> Value {
    vec3(v.map(|x| x.clamp(MODEL_MIN, MODEL_MAX)))
}

fn element(p: &Part, fit: &Fit, target: Target) -> Value {
    let c = fit.apply(p.center);
    let h = p.half.map(|x| x * fit.scale);
    let mut out = Map::new();
    out.insert("from".into(), bounded(std::array::from_fn(|k| c[k] - h[k])));
    out.insert("to".into(), bounded(std::array::from_fn(|k| c[k] + h[k])));
    match p.turn {
        Turn::None => {}
        Turn::Axis(axis, angle) => {
            out.insert(
                "rotation".into(),
                json!({ "origin": vec3(c), "axis": AXES[axis], "angle": tidy(angle) }),
            );
        }
        Turn::Euler([x, y, z]) => {
            out.insert(
                "rotation".into(),
                json!({ "origin": vec3(c), "x": tidy(x), "y": tidy(y), "z": tidy(z) }),
            );
        }
    }
    if let Some(level) = p.light.filter(|_| target.light_emission) {
        out.insert("light_emission".into(), json!(level.min(15)));
    }
    let texture = format!("#{TEXTURE_KEY}");
    let mut faces = Map::new();
    for (name, uv) in &p.faces {
        faces.insert(
            (*name).into(),
            json!({ "uv": [tidy(uv[0]), tidy(uv[1]), tidy(uv[2]), tidy(uv[3])], "texture": texture }),
        );
    }
    out.insert("faces".into(), Value::Object(faces));
    Value::Object(out)
}
