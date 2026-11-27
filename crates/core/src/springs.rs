use std::f32::consts::{PI, TAU};

use libm::{asinf, atan2f, cosf, expf, floorf, sinf, sqrtf};

use crate::attach::RigKind;
use crate::cost::{check_springs, Springs};
use crate::ir::{Cube, Model};
use crate::issues::{join, Issue, IssueCode, Report};
use crate::limits::{
    SPRING_BONES_PER_ITEM, SPRING_CHAIN_DEPTH, SPRING_LIMIT_DEGREES, SPRING_MAX_STEPS,
    SPRING_STEP_HZ, SPRING_TELEPORT_BLOCKS, SPRING_WATER_DRAG, SPRING_WATER_GRAVITY,
};
use crate::math::Affine;
use crate::pose::Pose;
use crate::wearer::{SPRING_BONE_FLOATS, SPRING_CLOCK, SPRING_HEADER_FLOATS, TIME_NEVER};

pub use crate::ir::SpringParams;

#[derive(Clone, Debug, PartialEq)]
pub struct SpringBone {
    pub bone: u16,
    pub parent: Option<u16>,
    pub params: SpringParams,
    pub pivot: [f32; 3],
    pub rotation: [f32; 3],
    pub tip: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpringSet {
    pub bones: Vec<SpringBone>,
    pub depth: u8,
}

impl SpringSet {
    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub bone: &'static str,
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub radius: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpringInput<'a> {
    pub world: &'a [Affine],
    pub root: &'a Affine,
    pub capsules: &'a [Capsule],
    pub capsule_frames: &'a [Affine],
    pub in_water: bool,
    pub scale: f32,
    pub now: f32,
}

type V3 = [f32; 3];
type M3 = [[f32; 3]; 3];

const MAX_BONES: usize = SPRING_BONES_PER_ITEM as usize;
const MAX_CAPSULES: usize = 8;
const STEP_HZ: f32 = SPRING_STEP_HZ as f32;
const STEP: f32 = 1.0 / STEP_HZ;
const MAX_STEPS: f32 = SPRING_MAX_STEPS as f32;
const STEP_SLACK: f32 = 1e-3;
const GRAVITY: f32 = 16.0;
const STIFFNESS_HZ: f32 = 6.0;
const BASE_DAMPING: f32 = 2.0;
const DAMPING_RATE: f32 = 30.0;
const DRAG_RATE: f32 = 40.0;
const SKIN_PIXELS: f32 = 0.1;
const MIN_TIP_PIXELS: f32 = 0.5;
const REST_RADIANS: f32 = 1e-5;
const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

const fn capsule(bone: &'static str, from: V3, to: V3, radius: f32) -> Capsule {
    Capsule {
        bone,
        from,
        to,
        radius,
    }
}

const PLAYER_CAPSULES: [Capsule; 6] = [
    capsule("head", [0.0, 27.0, 0.0], [0.0, 29.0, 0.0], 4.0),
    capsule("body", [0.0, 14.5, 0.0], [0.0, 21.5, 0.0], 2.5),
    capsule("rightArm", [-6.0, 14.0, 0.0], [-6.0, 22.0, 0.0], 2.0),
    capsule("leftArm", [6.0, 14.0, 0.0], [6.0, 22.0, 0.0], 2.0),
    capsule("rightLeg", [-1.9, 2.0, 0.0], [-1.9, 10.0, 0.0], 2.0),
    capsule("leftLeg", [1.9, 2.0, 0.0], [1.9, 10.0, 0.0], 2.0),
];

pub fn build(m: &Model, r: &mut Report) -> SpringSet {
    let mut set = SpringSet::default();
    let mut depths = vec![0u32; m.bones.len()];
    let mut kept: Vec<Option<u16>> = vec![None; m.bones.len()];
    let (mut marked, mut deepest) = (0u32, 0u32);
    for (i, bone) in m.bones.iter().enumerate() {
        let Some(params) = bone.spring else {
            continue;
        };
        let parent = bone.parent.filter(|&p| depths[p] > 0);
        if parent.is_none() && bone.parent.is_some_and(|p| marked_above(m, &depths, p)) {
            r.push(Issue::warning(
                IssueCode::SpringChainBroken,
                join(&join(&m.identifier, "bones"), &bone.name),
            ));
        }
        let depth = parent.map_or(1, |p| depths[p] + 1);
        depths[i] = depth;
        marked += 1;
        deepest = deepest.max(depth);
        let link = parent.map(|p| kept[p]);
        let Ok(index) = u16::try_from(i) else {
            continue;
        };
        if depth > SPRING_CHAIN_DEPTH || set.bones.len() >= MAX_BONES || link == Some(None) {
            continue;
        }
        kept[i] = Some(set.bones.len() as u16);
        set.depth = set.depth.max(depth as u8);
        set.bones.push(SpringBone {
            bone: index,
            parent: link.flatten(),
            params,
            pivot: bone.pivot,
            rotation: bone.rotation,
            tip: tip(m, i),
        });
    }
    check_springs(
        &Springs {
            path: join(&m.identifier, "bones"),
            bones: marked,
            chain_depth: deepest,
        },
        r,
    );
    set
}

fn marked_above(m: &Model, depths: &[u32], from: usize) -> bool {
    let mut at = Some(from);
    while let Some(i) = at {
        if depths[i] > 0 {
            return true;
        }
        at = m.bones[i].parent;
    }
    false
}

fn tip(m: &Model, i: usize) -> V3 {
    let bone = &m.bones[i];
    let far = |p: V3| norm(sub(p, bone.pivot)) > MIN_TIP_PIXELS;
    for springs_only in [true, false] {
        let (sum, count) = m.bones[i + 1..]
            .iter()
            .filter(|b| b.parent == Some(i) && (b.spring.is_some() || !springs_only))
            .filter(|b| far(b.pivot))
            .fold(([0.0; 3], 0u32), |(sum, n), b| (add(sum, b.pivot), n + 1));
        if count > 0 {
            return scale(sum, 1.0 / count as f32);
        }
    }
    let bounds = bone.cubes.iter().map(Cube::bounds).reduce(|a, b| {
        (
            [a.0[0].min(b.0[0]), a.0[1].min(b.0[1]), a.0[2].min(b.0[2])],
            [a.1[0].max(b.1[0]), a.1[1].max(b.1[1]), a.1[2].max(b.1[2])],
        )
    });
    let reach = match bounds {
        Some((lo, hi)) => {
            let center = scale(add(lo, hi), 0.5);
            if far(center) {
                return center;
            }
            ((hi[1] - lo[1]) * 0.5).max(1.0)
        }
        None => 1.0,
    };
    [bone.pivot[0], bone.pivot[1] - reach, bone.pivot[2]]
}

pub fn settle(
    s: &SpringSet,
    header: &mut [f32],
    state: &mut [f32],
    input: &SpringInput<'_>,
    pose: &mut Pose,
) {
    let mut anim = [Anim::default(); MAX_BONES];
    let Some(count) = animated(s, input, &mut anim) else {
        return;
    };
    if count == 0 || header.len() < SPRING_HEADER_FLOATS || state.len() < count * SPRING_BONE_FLOATS
    {
        return;
    }
    let anim = &anim[..count];
    let state = &mut state[..count * SPRING_BONE_FLOATS];
    let origin = translation(input.root);
    let start = [header[3], header[7], header[11]];
    let (clock, now) = (header[SPRING_CLOCK], input.now);
    let fresh = !clock.is_finite()
        || !now.is_finite()
        || now < clock
        || header[..SPRING_CLOCK].iter().any(|v| !v.is_finite())
        || input.root.0.iter().any(|v| !v.is_finite())
        || state.iter().any(|v| !v.is_finite())
        || norm(sub(origin, start)) > SPRING_TELEPORT_BLOCKS;
    if fresh {
        rest(anim, state);
        header[..SPRING_CLOCK].copy_from_slice(&input.root.0);
        header[SPRING_CLOCK] = if now.is_finite() { now } else { TIME_NEVER };
        return;
    }

    let elapsed = now - clock;
    let whole = floorf(elapsed * STEP_HZ + STEP_SLACK);
    if whole >= 1.0 {
        let steps = whole.min(MAX_STEPS);
        let leftover = if whole > MAX_STEPS {
            0.0
        } else {
            (elapsed - steps * STEP).max(0.0)
        };
        let velocity = scale(sub(origin, start), 1.0 / elapsed);
        let mut solids = [Solid::default(); MAX_CAPSULES];
        let solid_count = solids_in(input, &mut solids);
        let context = Context {
            anim,
            solids: &solids[..solid_count],
            velocity,
            in_water: input.in_water,
        };
        let span = steps * STEP + leftover;
        carry(state, scale(velocity, -span));
        for k in 1..=steps as u32 {
            advance(s, &context, state, span - k as f32 * STEP);
        }
        let behind = scale(velocity, leftover);
        carry(state, behind);
        header[..SPRING_CLOCK].copy_from_slice(&input.root.0);
        header[3] = origin[0] - behind[0];
        header[7] = origin[1] - behind[1];
        header[11] = origin[2] - behind[2];
        header[SPRING_CLOCK] = now - leftover;
    }
    display(s, anim, state, input.scale, pose);
}

pub fn capsules(k: RigKind) -> &'static [Capsule] {
    match k {
        RigKind::Player => &PLAYER_CAPSULES,
        RigKind::Actor | RigKind::Boat | RigKind::Minecart | RigKind::Horse => &[],
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Anim {
    pivot: V3,
    tip: V3,
    length: f32,
    linear: M3,
}

#[derive(Clone, Copy, Debug, Default)]
struct Solid {
    from: V3,
    to: V3,
    reach: f32,
}

struct Context<'a> {
    anim: &'a [Anim],
    solids: &'a [Solid],
    velocity: V3,
    in_water: bool,
}

#[derive(Clone, Copy, Debug)]
struct Rigid {
    r: M3,
    t: V3,
}

impl Rigid {
    const IDENTITY: Rigid = Rigid {
        r: IDENTITY,
        t: [0.0; 3],
    };

    fn apply(&self, p: V3) -> V3 {
        add(mat_vec(&self.r, p), self.t)
    }

    fn turned(&self, pivot: V3, q: &M3) -> Rigid {
        Rigid {
            r: mat_mul(q, &self.r),
            t: add(pivot, mat_vec(q, sub(self.t, pivot))),
        }
    }
}

fn animated(s: &SpringSet, input: &SpringInput<'_>, out: &mut [Anim; MAX_BONES]) -> Option<usize> {
    let count = s.bones.len().min(MAX_BONES);
    let root = linear(input.root);
    let to_world = |p: V3| mat_vec(&root, p);
    for (a, b) in out.iter_mut().zip(&s.bones[..count]) {
        let w = input.world.get(b.bone as usize)?;
        let pivot = to_world(w.point(b.pivot));
        let tip = to_world(w.point(b.tip));
        *a = Anim {
            pivot,
            tip,
            length: norm(sub(tip, pivot)),
            linear: mat_mul(&root, &linear(w)),
        };
    }
    Some(count)
}

fn solids_in(input: &SpringInput<'_>, out: &mut [Solid; MAX_CAPSULES]) -> usize {
    let root = linear(input.root);
    let to_world = |p: V3| mat_vec(&root, p);
    let unit = norm(to_world([1.0, 0.0, 0.0]));
    let pairs = input.capsules.iter().zip(input.capsule_frames);
    let mut count = 0;
    for (solid, (c, frame)) in out.iter_mut().zip(pairs) {
        *solid = Solid {
            from: to_world(frame.point(c.from)),
            to: to_world(frame.point(c.to)),
            reach: (c.radius + SKIN_PIXELS) * unit,
        };
        count += 1;
    }
    count
}

fn rest(anim: &[Anim], state: &mut [f32]) {
    for (a, slot) in anim.iter().zip(state.chunks_exact_mut(SPRING_BONE_FLOATS)) {
        slot.copy_from_slice(&[a.tip[0], a.tip[1], a.tip[2], 0.0, 0.0, 0.0]);
    }
}

fn carry(state: &mut [f32], by: V3) {
    for slot in state.chunks_exact_mut(SPRING_BONE_FLOATS) {
        for (p, d) in slot.iter_mut().zip(by) {
            *p += d;
        }
    }
}

fn link(b: &SpringBone, i: usize, chain: &[Rigid]) -> Rigid {
    b.parent
        .map(usize::from)
        .filter(|&p| p < i)
        .map_or(Rigid::IDENTITY, |p| chain[p])
}

fn advance(s: &SpringSet, c: &Context<'_>, state: &mut [f32], behind: f32) {
    let shift = scale(c.velocity, behind);
    let mut chain = [Rigid::IDENTITY; MAX_BONES];
    let slots = state.chunks_exact_mut(SPRING_BONE_FLOATS);
    for (i, ((b, a), slot)) in s.bones.iter().zip(c.anim).zip(slots).enumerate() {
        let parent = link(b, i, &chain);
        let pivot = parent.apply(sub(a.pivot, shift));
        let target = parent.apply(sub(a.tip, shift));
        let pos = [slot[0], slot[1], slot[2]];
        let vel = [slot[3], slot[4], slot[5]];
        let p = &b.params;
        let (gravity, drag) = if c.in_water {
            (p.gravity * SPRING_WATER_GRAVITY, p.drag * SPRING_WATER_DRAG)
        } else {
            (p.gravity, p.drag)
        };
        let pull = TAU * STIFFNESS_HZ * p.stiffness.clamp(0.0, 1.0);
        let force = add(
            scale(sub(target, pos), pull * pull),
            [0.0, -GRAVITY * gravity, 0.0],
        );
        let damping = BASE_DAMPING + DAMPING_RATE * p.damping.clamp(0.0, 1.0);
        let mut v = add(vel, scale(force, STEP));
        v = add(c.velocity, scale(sub(v, c.velocity), expf(-damping * STEP)));
        v = scale(v, expf(-DRAG_RATE * drag.max(0.0) * STEP));
        let mut next = add(pos, scale(v, STEP));
        next = constrain(next, pivot, target, a.length, p.limit_deg);
        if p.collide && !c.solids.is_empty() {
            next = push_out(next, target, c.solids, shift);
            next = constrain(next, pivot, target, a.length, p.limit_deg);
        }
        let v = scale(sub(next, pos), STEP_HZ);
        slot.copy_from_slice(&[next[0], next[1], next[2], v[0], v[1], v[2]]);
        let (axis, angle) = swing(sub(target, pivot), sub(next, pivot));
        chain[i] = parent.turned(pivot, &axis_angle(axis, angle));
    }
}

fn display(s: &SpringSet, anim: &[Anim], state: &[f32], amount: f32, pose: &mut Pose) {
    if !above(amount, 0.0) {
        return;
    }
    let amount = amount.min(1.0);
    let mut chain = [Rigid::IDENTITY; MAX_BONES];
    let slots = state.chunks_exact(SPRING_BONE_FLOATS);
    for (i, ((b, a), slot)) in s.bones.iter().zip(anim).zip(slots).enumerate() {
        let parent = link(b, i, &chain);
        let pivot = parent.apply(a.pivot);
        let target = parent.apply(a.tip);
        let at = [slot[0], slot[1], slot[2]];
        let (axis, angle) = swing(sub(target, pivot), sub(at, pivot));
        chain[i] = parent.turned(pivot, &axis_angle(axis, angle));
        let shown = angle * amount;
        if shown > REST_RADIANS {
            let local = mat_vec(&transpose(&parent.r), axis);
            bend(pose, b, &a.linear, local, shown);
        }
    }
}

fn bend(pose: &mut Pose, b: &SpringBone, world: &M3, axis: V3, angle: f32) {
    let Some(bone) = pose.0.get_mut(b.bone as usize) else {
        return;
    };
    let Some(inverse) = invert(world) else {
        return;
    };
    if bone.scale.iter().any(|s| !above(s.abs(), 1e-6)) {
        return;
    }
    let mut turn = mat_mul(&mat_mul(&inverse, &axis_angle(axis, angle)), world);
    for (r, row) in turn.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v *= bone.scale[r] / bone.scale[c];
        }
    }
    let total = add(b.rotation, bone.rotation);
    let euler = matrix_euler(&mat_mul(&euler_matrix(total), &turn));
    bone.rotation = sub(euler, b.rotation);
}

fn constrain(p: V3, pivot: V3, target: V3, length: f32, limit_deg: f32) -> V3 {
    if !above(length, 0.0) {
        return target;
    }
    let home = sub(target, pivot);
    let mut dir = sub(p, pivot);
    if !above(norm(dir), 0.0) {
        dir = home;
    }
    let limit = limit_deg.clamp(0.0, SPRING_LIMIT_DEGREES).to_radians();
    let (axis, angle) = swing(home, dir);
    let turned = mat_vec(&axis_angle(axis, angle.min(limit)), normalized(home));
    add(pivot, scale(turned, length))
}

fn push_out(mut p: V3, target: V3, solids: &[Solid], shift: V3) -> V3 {
    for c in solids {
        let (a, b) = (sub(c.from, shift), sub(c.to, shift));
        if norm(sub(target, closest(target, a, b))) < c.reach {
            continue;
        }
        let q = closest(p, a, b);
        let d = sub(p, q);
        let l = norm(d);
        if l >= c.reach {
            continue;
        }
        let n = if l > 1e-6 {
            scale(d, 1.0 / l)
        } else {
            normalized(sub(target, q))
        };
        p = add(q, scale(n, c.reach));
    }
    p
}

fn closest(p: V3, a: V3, b: V3) -> V3 {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    let t = if len2 > 0.0 {
        (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    add(a, scale(ab, t))
}

fn swing(u: V3, v: V3) -> (V3, f32) {
    let (lu, lv) = (norm(u), norm(v));
    if !above(lu, 0.0) || !above(lv, 0.0) {
        return ([1.0, 0.0, 0.0], 0.0);
    }
    let c = cross(u, v);
    let s = norm(c);
    let d = dot(u, v);
    if s <= f32::EPSILON * lu * lv {
        return if d >= 0.0 {
            ([1.0, 0.0, 0.0], 0.0)
        } else {
            (perpendicular(u), PI)
        };
    }
    (scale(c, 1.0 / s), atan2f(s, d))
}

fn perpendicular(u: V3) -> V3 {
    let other = if u[0].abs() < 0.9 * norm(u) {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    normalized(cross(u, other))
}

fn axis_angle(k: V3, angle: f32) -> M3 {
    let (s, c) = (sinf(angle), cosf(angle));
    let t = 1.0 - c;
    let [x, y, z] = k;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

fn euler_matrix(deg: V3) -> M3 {
    let [a, b, c] = [
        deg[0].to_radians(),
        -deg[1].to_radians(),
        -deg[2].to_radians(),
    ];
    let (sa, ca) = (sinf(a), cosf(a));
    let (sb, cb) = (sinf(b), cosf(b));
    let (sc, cc) = (sinf(c), cosf(c));
    [
        [cc * cb, cc * sb * sa - sc * ca, cc * sb * ca + sc * sa],
        [sc * cb, sc * sb * sa + cc * ca, sc * sb * ca - cc * sa],
        [-sb, cb * sa, cb * ca],
    ]
}

fn matrix_euler(m: &M3) -> V3 {
    let sb = (-m[2][0]).clamp(-1.0, 1.0);
    let b = asinf(sb);
    let (a, c) = if sb.abs() < 0.99999 {
        (atan2f(m[2][1], m[2][2]), atan2f(m[1][0], m[0][0]))
    } else {
        (atan2f(-m[1][2], m[1][1]), 0.0)
    };
    [a.to_degrees(), -b.to_degrees(), -c.to_degrees()]
}

fn translation(a: &Affine) -> V3 {
    [a.0[3], a.0[7], a.0[11]]
}

fn linear(a: &Affine) -> M3 {
    let m = &a.0;
    [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]]
}

fn mat_vec(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

fn mat_mul(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (row, a_row) in out.iter_mut().zip(a) {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a_row[0] * b[0][c] + a_row[1] * b[1][c] + a_row[2] * b[2][c];
        }
    }
    out
}

fn mat_scale(m: &M3, k: f32) -> M3 {
    m.map(|row| scale(row, k))
}

fn transpose(m: &M3) -> M3 {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

fn invert(m: &M3) -> Option<M3> {
    let minor =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adjugate = [
        [minor(1, 2, 1, 2), -minor(0, 2, 1, 2), minor(0, 1, 1, 2)],
        [-minor(1, 2, 0, 2), minor(0, 2, 0, 2), -minor(0, 1, 0, 2)],
        [minor(1, 2, 0, 1), -minor(0, 2, 0, 1), minor(0, 1, 0, 1)],
    ];
    let det = m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
    if !above(det.abs(), 1e-12) {
        return None;
    }
    Some(mat_scale(&adjugate, 1.0 / det))
}

fn above(v: f32, floor: f32) -> bool {
    v > floor
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(v: V3, k: f32) -> V3 {
    [v[0] * k, v[1] * k, v[2] * k]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(v: V3) -> f32 {
    sqrtf(dot(v, v))
}

fn normalized(v: V3) -> V3 {
    let l = norm(v);
    if l > 0.0 {
        scale(v, 1.0 / l)
    } else {
        [0.0, 1.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::asset::{CosmeticAsset, Kind};
    use crate::bedrock::parse_one;
    use crate::cost::Detail;
    use crate::issues::{IssueCode, Mode};
    use crate::player::Arms;
    use crate::pose::{world_transforms, BonePose};
    use crate::scene::{FrameSections, Physics, Scene, NEUTRAL_SIGNALS};
    use crate::wearer::{SPRING_BONE_FLOATS, SPRING_CLOCK, SPRING_HEADER_FLOATS, TIME_NEVER};

    const STEP: f32 = 1.0 / 60.0;

    fn geometry(bones: &str) -> Model {
        parse_one(&format!(
            r#"{{"format_version":"1.12.0","minecraft:geometry":[{{
                "description":{{"identifier":"geometry.test","texture_width":16,"texture_height":16}},
                "bones":[{bones}]}}]}}"#
        ))
        .unwrap()
    }

    fn link(name: &str, parent: &str, z: u32, spring: &str) -> String {
        format!(
            r#"{{"name":"{name}","parent":"{parent}","pivot":[0,12,{z}],"polyplus:spring":{spring},
                "cubes":[{{"origin":[-0.5,11.5,{z}],"size":[1,1,4],"uv":[0,0]}}]}}"#
        )
    }

    fn tail_on(parent: &str, spring: &str) -> Model {
        geometry(&format!("{},{}", link("tail_1", parent, 2, spring), link("tail_2", "tail_1", 6, spring)))
    }

    fn tail(spring: &str) -> Model {
        geometry(&format!(
            r#"{{"name":"base","pivot":[0,12,2]}},{},{}"#,
            link("tail_1", "base", 2, spring),
            link("tail_2", "tail_1", 6, spring)
        ))
    }

    struct Sim {
        model: Model,
        set: SpringSet,
        header: Vec<f32>,
        state: Vec<f32>,
        pose: Pose,
        world: Vec<Affine>,
        capsules: Vec<Capsule>,
        frames: Vec<Affine>,
    }

    impl Sim {
        fn new(model: Model) -> Sim {
            let set = build(&model, &mut Report::new(Mode::Authoring));
            let mut header = vec![f32::NAN; SPRING_HEADER_FLOATS];
            header[SPRING_CLOCK] = TIME_NEVER;
            Sim {
                state: vec![0.0; set.bones.len() * SPRING_BONE_FLOATS],
                pose: Pose::rest(&model),
                header,
                set,
                model,
                world: Vec::new(),
                capsules: Vec::new(),
                frames: Vec::new(),
            }
        }

        fn frame(&mut self, now: f32, root: Affine) {
            self.pose.reset();
            world_transforms(&self.model, Some(&self.pose), |_| None, &mut self.world);
            let root = root.mul(&Affine::scale([1.0 / 16.0; 3]));
            let input = SpringInput {
                world: &self.world,
                root: &root,
                capsules: &self.capsules,
                capsule_frames: &self.frames,
                in_water: false,
                scale: 1.0,
                now,
            };
            settle(&self.set, &mut self.header, &mut self.state, &input, &mut self.pose);
        }

        fn run(&mut self, from: f32, to: f32, dt: f32, root: impl Fn(f32) -> Affine) -> Vec<f32> {
            let mut swings = Vec::new();
            let mut now = from;
            while now <= to {
                self.frame(now, root(now));
                swings.push(self.swing(0));
                now += dt;
            }
            swings
        }

        fn swing(&self, spring: usize) -> f32 {
            let bone = &self.set.bones[spring];
            let (mut animated, mut posed) = (Vec::new(), Vec::new());
            world_transforms(&self.model, None, |_| None, &mut animated);
            world_transforms(&self.model, Some(&self.pose), |_| None, &mut posed);
            let dir = |w: &Affine| {
                let (a, b) = (w.point(bone.pivot), w.point(bone.tip));
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
            };
            let (a, b) = (dir(&animated[bone.bone as usize]), dir(&posed[bone.bone as usize]));
            let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            (dot / (len(a) * len(b))).clamp(-1.0, 1.0).acos().to_degrees()
        }

        fn rested(&self) -> bool {
            self.pose.0.iter().all(|p| *p == BonePose::default())
        }
    }

    fn dash(t: f32) -> Affine {
        Affine::translation([5.0 * (t - 0.5).clamp(0.0, 0.5), 0.0, 0.0])
    }

    #[test]
    fn springs_build_chains_from_marked_bones_within_their_budget() {
        let model = tail(r#"{"stiffness":0.5}"#);
        let mut report = Report::new(Mode::Authoring);
        let set = build(&model, &mut report);
        assert!(report.issues().is_empty(), "{:?}", report.issues());
        assert_eq!(set.depth, 2);
        assert_eq!(set.bones.iter().map(|b| (b.bone, b.parent)).collect::<Vec<_>>(), [(1, None), (2, Some(0))]);
        assert_eq!((set.bones[0].params.stiffness, set.bones[0].tip), (0.5, model.bones[2].pivot));
        let last = set.bones[1].tip;
        assert!((last[2] + 8.0).abs() < 1e-4 && last[1] == 12.0, "{last:?}");

        let bones = |n: usize, chained: bool| {
            let list: Vec<String> = (0..n)
                .map(|i| {
                    let parent = if chained && i > 0 { format!(r#""parent":"c{}","#, i - 1) } else { String::new() };
                    format!(r#"{{"name":"c{i}",{parent}"pivot":[{i},12,{}],"polyplus:spring":{{}}}}"#, 2 * i)
                })
                .collect();
            let mut report = Report::new(Mode::Client);
            let set = build(&geometry(&list.join(",")), &mut report);
            let codes: Vec<_> = report.issues().iter().map(|i| i.code).collect();
            (codes, set.bones.len(), set.depth)
        };
        assert_eq!(bones(10, true), (vec![IssueCode::SpringChainTooDeep], 8, 8));
        assert_eq!(bones(20, false), (vec![IssueCode::SpringBonesOverBudget], 16, 1));

        let gap = geometry(
            r#"{"name":"a","pivot":[0,12,0],"polyplus:spring":{}},{"name":"b","parent":"a","pivot":[0,12,2]},
               {"name":"c","parent":"b","pivot":[0,12,4],"polyplus:spring":{}}"#,
        );
        let mut report = Report::new(Mode::Authoring);
        let set = build(&gap, &mut report);
        assert_eq!((report.issues()[0].code, report.issues()[0].path.as_str()), (IssueCode::SpringChainBroken, "geometry.test.bones.c"));
        assert_eq!((set.bones[1].parent, set.depth), (None, 1));
    }

    #[test]
    fn springs_start_at_rest_trail_motion_and_restart_on_jumps() {
        let mut sim = Sim::new(tail(r#"{"gravity":0}"#));
        sim.frame(0.25, Affine::translation([3.0, 64.0, -2.0]));
        assert!(sim.rested());
        assert_eq!(sim.header[SPRING_CLOCK], 0.25);
        let swings = sim.run(0.3, 1.3, STEP, |t| Affine::translation([5.0 * t, 64.0, -2.0]));
        assert!(swings.iter().any(|&s| s > 3.0), "{swings:?}");
        let turns = Sim::new(tail(r#"{"gravity":0}"#)).run(0.0, 0.5, STEP, |t| Affine::rot_y(4.0 * t));
        assert!(turns.iter().any(|&s| s > 3.0), "{turns:?}");

        let mut sim = Sim::new(tail("{}"));
        sim.run(0.0, 0.8, STEP, dash);
        assert!(sim.swing(0) > 1.0);
        sim.frame(0.8 + STEP, Affine::translation([40.0, 0.0, 0.0]));
        assert!(sim.rested() && sim.state.iter().skip(3).step_by(SPRING_BONE_FLOATS).all(|v| *v == 0.0));
        sim.run(1.0, 1.6, STEP, dash);
        assert!(!sim.rested());
        sim.frame(0.5, dash(1.6));
        assert_eq!((sim.rested(), sim.header[SPRING_CLOCK]), (true, 0.5));
        sim.run(0.6, 1.0, STEP, dash);
        sim.header[SPRING_CLOCK] = TIME_NEVER;
        sim.frame(1.1, dash(1.0));
        assert!(sim.rested());
    }

    #[test]
    fn springs_stay_inside_their_cone_and_out_of_body_capsules() {
        let mut sim = Sim::new(tail(r#"{"stiffness":0,"gravity":1,"limit":15}"#));
        let mut widest = 0.0f32;
        for i in 0..120 {
            let now = i as f32 * STEP;
            sim.frame(now, dash(now));
            assert!(sim.swing(0) <= 15.01 && sim.swing(1) <= 30.02, "{}", sim.swing(0));
            widest = widest.max(sim.swing(0));
        }
        assert!(widest > 14.0, "{widest}");

        let fall = |collide: bool| {
            let mut sim = Sim::new(geometry(&link("flap", "", 2, &format!(r#"{{"stiffness":0,"gravity":1,"limit":180,"collide":{collide}}}"#)).replace(r#""parent":"","#, "")));
            sim.capsules = vec![Capsule { bone: "body", from: [-4.0, 9.5, -4.0], to: [4.0, 9.5, -4.0], radius: 2.3 }];
            sim.frames = vec![Affine::IDENTITY];
            let mut nearest = f32::INFINITY;
            for i in 0..120 {
                sim.frame(i as f32 * STEP, Affine::IDENTITY);
                let p = &sim.state;
                nearest = nearest.min(((p[1] * 16.0 - 9.5).powi(2) + (p[2] * 16.0 + 4.0).powi(2)).sqrt());
            }
            nearest
        };
        assert!(fall(true) >= 2.3 - 1e-3, "{}", fall(true));
        assert!(fall(false) < 2.2, "{}", fall(false));
        assert_eq!(capsules(RigKind::Player).iter().map(|c| c.bone).collect::<Vec<_>>(), ["head", "body", "rightArm", "leftArm", "rightLeg", "leftLeg"]);
    }

    #[test]
    fn springs_are_deterministic() {
        let run = || {
            let mut sim = Sim::new(tail(r#"{"limit":30}"#));
            sim.capsules = capsules(RigKind::Player).to_vec();
            sim.frames = vec![Affine::IDENTITY; sim.capsules.len()];
            let swings = sim.run(0.0, 2.0, 1.0 / 144.0, |t| dash(t).mul(&Affine::rot_y(t)));
            let rotations = sim.pose.0.iter().flat_map(|p| p.rotation);
            sim.state.iter().copied().chain(swings).chain(rotations).map(f32::to_bits).collect::<Vec<u32>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn springs_follow_the_physics_setting_and_detail() {
        let scene = |physics: Physics| {
            let model = tail_on("body", "{}");
            let mut asset = CosmeticAsset::new("tail", 3, Kind::Item);
            asset.springs = build(&model, &mut Report::new(Mode::Client));
            asset.model = Some(model);
            let mut scene = Scene::default();
            scene.load_player(0, Arms::Wide);
            scene.load_asset(1, Arc::new(asset)).unwrap();
            scene.set_physics(physics);
            scene
        };
        let frame = |scene: &mut Scene, i: u32, detail: Detail| {
            let sections = FrameSections { root: true, ..FrameSections::default() };
            let mut row = NEUTRAL_SIGNALS.to_vec();
            row.extend_from_slice(&[0.0, detail.value() as f32]);
            row.extend_from_slice(&dash(i as f32 / 60.0).mul(&Affine::scale([1.0 / 16.0; 3])).0);
            let mut out = vec![0.0; scene.output_layout(&sections).stride()];
            scene.frame_wearers_shaped(f64::from(i) / 60.0, &[5], &sections, &row, &Affine::IDENTITY, &mut out).unwrap();
            scene.bone_pose(1, "tail_1").unwrap().rotation != [0.0; 3]
        };
        for (physics, swings) in [(Physics::On, true), (Physics::Own(5), true), (Physics::Own(6), false), (Physics::Off, false)] {
            let mut scene = scene(physics);
            assert_eq!((0..40).any(|i| frame(&mut scene, i, Detail::Full)), swings, "{physics:?}");
        }
        let mut scene = scene(Physics::default());
        assert!((0..40).any(|i| frame(&mut scene, i, Detail::Full)));
        assert!(!(40..50).any(|i| frame(&mut scene, i, Detail::Reduced)));
        assert!(!frame(&mut scene, 50, Detail::Full));
        assert!(frame(&mut scene, 52, Detail::Full));
    }
}
