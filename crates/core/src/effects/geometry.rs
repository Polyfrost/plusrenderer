use libm::{roundf, sqrtf};

use super::particles::{
    for_each_live, particle_at, particle_parts, size_at, tint_at, unpack, uv_at, Budget,
};
use super::ring::{rotate, Ring, RingMut, Turn, NO_TURN, RING_SPAN_SECONDS};
use super::trail;
use super::{
    Blend, EffectSet, Facing, LocatorRef, Spawn, Tint, View, CORNER_FLOATS,
    EMITTER_ACTIVATION_TIMES, EMITTER_LOCATOR, EMITTER_START, EMITTER_STOP, QUAD_BLEND, QUAD_COLOR,
    QUAD_CORNERS, QUAD_FLAGS, QUAD_FLAG_EMISSIVE, QUAD_FLOATS, QUAD_TEXTURE,
};
use crate::cost::Features;
use crate::math::Affine;
use crate::palette::{base_tone, custom_tones, tone_index, Palette, MAX_TONES};
use crate::signals::SignalState;
use crate::wearer::{RING_CLOCK_FLOATS, RING_POINTS, RING_POINT_FLOATS};

pub const PIXELS_PER_BLOCK: f32 = 16.0;
pub const BLEND_ALPHA: f32 = 0.0;
pub const BLEND_ADDITIVE: f32 = 1.0;

const UP: [f32; 3] = [0.0, 1.0, 0.0];
const TINY: f32 = 1e-6;
const VELOCITY_PROBE: f32 = 1.0 / 60.0;
const SEEN_SLACK: f32 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub texture: u32,
    pub blend: Blend,
    pub color: [f32; 4],
    pub corners: [[f32; CORNER_FLOATS]; 4],
    pub emissive: bool,
}

impl Quad {
    pub fn write(&self, out: &mut [f32]) {
        out[QUAD_TEXTURE] = self.texture as f32;
        out[QUAD_BLEND] = match self.blend {
            Blend::Alpha => BLEND_ALPHA,
            Blend::Additive => BLEND_ADDITIVE,
        };
        out[QUAD_COLOR..QUAD_COLOR + 4].copy_from_slice(&self.color);
        for (i, corner) in self.corners.iter().enumerate() {
            let at = QUAD_CORNERS + i * CORNER_FLOATS;
            out[at..at + CORNER_FLOATS].copy_from_slice(corner);
        }
        let flags = if self.emissive { QUAD_FLAG_EMISSIVE } else { 0 };
        out[QUAD_FLAGS] = flags as f32;
    }
}

pub struct QuadSink<'a> {
    out: &'a mut [f32],
    count: usize,
}

impl<'a> QuadSink<'a> {
    pub fn new(out: &'a mut [f32]) -> QuadSink<'a> {
        QuadSink { out, count: 0 }
    }

    pub fn push(&mut self, quad: &Quad) {
        let at = self.count * QUAD_FLOATS;
        if let Some(slot) = self.out.get_mut(at..at + QUAD_FLOATS) {
            quad.write(slot);
        }
        self.count += 1;
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn capacity(&self) -> usize {
        self.out.len()
    }

    pub fn needed(&self) -> usize {
        self.count.saturating_mul(QUAD_FLOATS)
    }

    pub fn fits(&self) -> bool {
        self.needed() <= self.out.len()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Locators<'a> {
    pub model: &'a [Affine],
    pub player: &'a [Affine],
    pub root: Affine,
}

impl Locators<'_> {
    pub fn frame(&self, locator: LocatorRef) -> Affine {
        match locator {
            LocatorRef::Model(i) => self.model.get(i as usize),
            LocatorRef::Player(p) => self.player.get(p.index()),
        }
        .copied()
        .unwrap_or(self.root)
    }
}

#[derive(Clone, Copy)]
pub struct WearerEffects<'a> {
    pub locators: Locators<'a>,
    pub times: &'a [f32],
    pub rings: &'a [f32],
    pub signals: &'a SignalState,
    pub now: f32,
    pub epoch: f64,
    pub seen: f32,
    pub seed_epoch: f64,
    pub seed: u64,
    pub tones: &'a [u32],
    pub features: Features,
}

pub fn texture_count(e: &EffectSet) -> usize {
    e.particles.len() + e.trails.len()
}

pub fn texture_path(e: &EffectSet, local: usize) -> Option<&str> {
    match local.checked_sub(e.particles.len()) {
        None => e.particles.get(local).map(|p| p.texture.as_str()),
        Some(trail) => e.trails.get(trail).map(|t| t.texture.as_str()),
    }
}

pub fn draw(
    e: &EffectSet,
    local: usize,
    w: &WearerEffects<'_>,
    view: &View,
    texture: u32,
    sink: &mut QuadSink<'_>,
    budget: &mut Budget,
) {
    if !w.features.effects {
        return;
    }
    match local.checked_sub(e.particles.len()) {
        None if w.features.particles => particles(e, local as u16, w, view, texture, sink, budget),
        Some(index) if w.features.trails => {
            if let Some(def) = e.trails.get(index) {
                trail::draw(def, ring_of(e, w.rings, def.ring), w, view, texture, sink);
            }
        }
        _ => {}
    }
}

pub fn ring_points(e: &EffectSet, index: usize) -> usize {
    e.ring_points.get(index).map_or(RING_POINTS, |&points| {
        usize::from(points).clamp(1, RING_POINTS)
    })
}

pub fn ring_turns(e: &EffectSet, index: usize) -> usize {
    if e.ring_turns.get(index).copied().unwrap_or(false) {
        ring_points(e, index)
    } else {
        0
    }
}

pub fn ring_gates(e: &EffectSet, index: usize) -> usize {
    if e.ring_gates.get(index).copied().unwrap_or(false) {
        ring_points(e, index)
    } else {
        0
    }
}

fn ring_point_floats(e: &EffectSet, index: usize) -> usize {
    ring_points(e, index) * RING_POINT_FLOATS + ring_turns(e, index) + ring_gates(e, index)
}

pub fn ring_floats(e: &EffectSet) -> usize {
    let points: usize = (0..e.rings.len()).map(|i| ring_point_floats(e, i)).sum();
    e.rings.len() * RING_CLOCK_FLOATS + points
}

struct RingSpan {
    clock: usize,
    start: usize,
    turns: usize,
    gates: usize,
    end: usize,
}

fn ring_span(e: &EffectSet, index: usize) -> Option<RingSpan> {
    if index >= e.rings.len() {
        return None;
    }
    let before: usize = (0..index).map(|i| ring_point_floats(e, i)).sum();
    let start = e.rings.len() * RING_CLOCK_FLOATS + before;
    let turns = start + ring_points(e, index) * RING_POINT_FLOATS;
    let gates = turns + ring_turns(e, index);
    let end = gates + ring_gates(e, index);
    Some(RingSpan {
        clock: index * RING_CLOCK_FLOATS,
        start,
        turns,
        gates,
        end,
    })
}

pub fn ring_of<'a>(e: &EffectSet, rings: &'a [f32], index: u16) -> Ring<'a> {
    match ring_span(e, usize::from(index)) {
        Some(s) if s.end <= rings.len() => Ring {
            clock: &rings[s.clock..s.clock + RING_CLOCK_FLOATS],
            points: &rings[s.start..s.turns],
            turns: &rings[s.turns..s.gates],
            gates: &rings[s.gates..s.end],
        },
        _ => Ring::default(),
    }
}

pub fn ring_mut<'a>(e: &EffectSet, rings: &'a mut [f32], index: usize) -> Option<RingMut<'a>> {
    let s = ring_span(e, index).filter(|s| s.end <= rings.len())?;
    let (clocks, rest) = rings.split_at_mut(s.start);
    let (points, rest) = rest.split_at_mut(s.turns - s.start);
    let (turns, rest) = rest.split_at_mut(s.gates - s.turns);
    Some(RingMut {
        clock: &mut clocks[s.clock..s.clock + RING_CLOCK_FLOATS],
        points,
        turns,
        gates: &mut rest[..s.end - s.gates],
    })
}

pub fn sample_rings(
    e: &EffectSet,
    rings: &mut [f32],
    locators: &Locators<'_>,
    signals: &SignalState,
    now: f32,
) -> bool {
    let mut restarted = false;
    let block = block_scale(&locators.root);
    for (index, &locator) in e.rings.iter().enumerate() {
        if let Some(mut ring) = ring_mut(e, rings, index) {
            let newest = ring.as_ring().newest_time();
            restarted |= ring.as_ring().capacity() > 0
                && now.is_finite()
                && newest.is_none_or(|time| now - time > RING_SPAN_SECONDS);
            let frame = locators.frame(locator);
            if ring.as_ring().jumps_to(origin(&frame), now, block) {
                ring.split();
            }
            ring.sample_gated(
                origin(&frame),
                turn_of(&frame),
                |at| signals.gates_at(at),
                now,
            );
        }
    }
    restarted
}

pub fn break_rings(e: &EffectSet, rings: &mut [f32]) -> bool {
    let mut broke = false;
    for index in 0..e.rings.len() {
        if let Some(mut ring) = ring_mut(e, rings, index) {
            broke |= ring.split();
        }
    }
    broke
}

pub fn rebase_rings(e: &EffectSet, rings: &mut [f32], shift: [f32; 3]) {
    for index in 0..e.rings.len() {
        if let Some(mut ring) = ring_mut(e, rings, index) {
            ring.rebase(shift);
        }
    }
}

pub fn clear_rings(e: &EffectSet, rings: &mut [f32]) {
    for index in 0..e.rings.len() {
        if let Some(mut ring) = ring_mut(e, rings, index) {
            ring.clear();
        }
    }
}

fn ring_index(e: &EffectSet, locator: LocatorRef) -> Option<u16> {
    let index = e.rings.iter().position(|&r| r == locator)?;
    u16::try_from(index).ok()
}

pub fn palette_tones(p: &Palette, colorway: Option<usize>) -> Vec<u32> {
    palette_tones_with(p, colorway, &[])
}

pub fn palette_tones_with(p: &Palette, colorway: Option<usize>, custom: &[(u8, u32)]) -> Vec<u32> {
    let mut out = if custom.is_empty() {
        p.resolve(colorway)
    } else {
        custom_tones(p, colorway, custom)
    };
    for slot in 0..p.slots.len() {
        let count = p.tone_count(slot, colorway);
        out.push(out[tone_index(slot, base_tone(count))]);
    }
    out
}

pub fn slot_tone(tones: &[u32], slot: u8, tone: Option<u8>) -> u32 {
    let slots = tones.len() / (MAX_TONES + 1);
    let slot = usize::from(slot);
    if slot >= slots {
        return u32::MAX;
    }
    match tone {
        Some(tone) => tones[tone_index(slot, usize::from(tone))],
        None => tones[slots * MAX_TONES + slot],
    }
}

pub fn tint_color(tint: &Tint, tones: &[u32], fixed: impl FnOnce() -> u32) -> [f32; 4] {
    match tint {
        Tint::Slot { slot, tone } => unpack(slot_tone(tones, *slot, *tone)),
        _ => unpack(fixed()),
    }
}

pub fn origin(frame: &Affine) -> [f32; 3] {
    [frame.0[3], frame.0[7], frame.0[11]]
}

pub fn axes(frame: &Affine) -> [[f32; 3]; 3] {
    let m = &frame.0;
    [0, 1, 2].map(|c| unit([m[c], m[4 + c], m[8 + c]]).unwrap_or(UP))
}

pub fn turn_of(frame: &Affine) -> Turn {
    let [x, y, z] = axes(frame);
    let (m00, m01, m02) = (x[0], y[0], z[0]);
    let (m10, m11, m12) = (x[1], y[1], z[1]);
    let (m20, m21, m22) = (x[2], y[2], z[2]);
    let trace = m00 + m11 + m22;
    let q = if trace > 0.0 {
        let s = sqrtf(trace + 1.0) * 2.0;
        [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]
    } else if m00 > m11 && m00 > m22 {
        let s = sqrtf((1.0 + m00 - m11 - m22).max(TINY)) * 2.0;
        [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
    } else if m11 > m22 {
        let s = sqrtf((1.0 + m11 - m00 - m22).max(TINY)) * 2.0;
        [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]
    } else {
        let s = sqrtf((1.0 + m22 - m00 - m11).max(TINY)) * 2.0;
        [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]
    };
    let len = sqrtf(q.iter().map(|c| c * c).sum());
    if len > TINY && len.is_finite() {
        q.map(|c| c / len)
    } else {
        NO_TURN
    }
}

pub fn block_scale(frame: &Affine) -> f32 {
    let m = &frame.0;
    length([m[0], m[4], m[8]]) * PIXELS_PER_BLOCK
}

pub fn facing_axes(
    facing: Facing,
    center: [f32; 3],
    view: &View,
    emitter: &[[f32; 3]; 3],
    velocity: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let camera = (view.right, view.up);
    let upright = || match unit([view.right[0], 0.0, view.right[2]]) {
        Some(right) => (right, UP),
        None => ([1.0, 0.0, 0.0], UP),
    };
    let toward = sub(view.camera, center);
    match facing {
        Facing::RotateXyz => camera,
        Facing::RotateY => upright(),
        Facing::LookatXyz => match unit(toward) {
            Some(dir) => match unit(cross(UP, dir)) {
                Some(right) => (right, cross(dir, right)),
                None => camera,
            },
            None => camera,
        },
        Facing::LookatY => match unit([toward[0], 0.0, toward[2]]) {
            Some(dir) => (cross(UP, dir), UP),
            None => upright(),
        },
        Facing::Direction => {
            let side = unit(velocity).zip(unit(toward)).and_then(|(up, dir)| {
                let right = unit(cross(up, dir))?;
                Some((right, up))
            });
            side.unwrap_or(camera)
        }
        Facing::EmitterXy => (emitter[0], emitter[1]),
        Facing::EmitterXz => (emitter[0], emitter[2]),
        Facing::EmitterYz => (emitter[2], emitter[1]),
    }
}

pub fn billboard(
    center: [f32; 3],
    (right, up): ([f32; 3], [f32; 3]),
    half: [f32; 2],
    uv: [f32; 4],
    view: &View,
) -> [[f32; CORNER_FLOATS]; 4] {
    let r = scale(right, half[0]);
    let u = scale(up, half[1]);
    let at = sub(center, view.camera);
    let corner = |sr: f32, su: f32, tu: f32, tv: f32| {
        let p = add(at, add(scale(r, sr), scale(u, su)));
        [p[0], p[1], p[2], tu, tv]
    };
    [
        corner(-1.0, -1.0, uv[0], uv[3]),
        corner(1.0, -1.0, uv[2], uv[3]),
        corner(1.0, 1.0, uv[2], uv[1]),
        corner(-1.0, 1.0, uv[0], uv[1]),
    ]
}

fn particles(
    e: &EffectSet,
    particle: u16,
    w: &WearerEffects<'_>,
    view: &View,
    texture: u32,
    sink: &mut QuadSink<'_>,
    budget: &mut Budget,
) {
    let Some(def) = e.particles.get(particle as usize) else {
        return;
    };
    for (index, emitter) in e.emitters.iter().enumerate() {
        if emitter.particle != Some(particle) {
            continue;
        }
        let (first, runs) = e.runs(index);
        for activation in 0..runs {
            let base = first + activation * EMITTER_ACTIVATION_TIMES;
            let (Some(&start), Some(&stop)) = (
                w.times.get(base + EMITTER_START),
                w.times.get(base + EMITTER_STOP),
            ) else {
                continue;
            };
            let chosen = w
                .times
                .get(base + EMITTER_LOCATOR)
                .and_then(|&code| LocatorRef::decode(roundf(code - start)));
            let locator = chosen.unwrap_or(emitter.locator);
            let frame = w.locators.frame(locator);
            let block = block_scale(&frame);
            let emitter_axes = axes(&frame);
            let world = match (emitter.world, chosen) {
                (false, _) => None,
                (true, None) => emitter.ring,
                (true, Some(locator)) => ring_index(e, locator),
            };
            let head = (origin(&frame), turn_of(&frame));
            let place = |spawn: &Spawn, age: f32, budget: &mut Budget| match world {
                Some(ring) => {
                    let ring = ring_of(e, w.rings, ring);
                    let from = ring.position_at(head.0, w.now, spawn.time);
                    let turn = ring.turn_at(head.1, w.now, spawn.time);
                    let (local, drift) = particle_parts(def, spawn, age, budget);
                    add(from, scale(add(rotate(turn, local), drift), block))
                }
                None => frame.point(scale(
                    particle_at(def, spawn, age, budget),
                    PIXELS_PER_BLOCK,
                )),
            };
            let live = |_, spawn: &Spawn, age, budget: &mut Budget| {
                if world.is_some() && spawn.time < w.seen - SEEN_SLACK {
                    return;
                }
                let center = place(spawn, age, budget);
                let velocity = match def.facing {
                    Facing::Direction => sub(place(spawn, age + VELOCITY_PROBE, budget), center),
                    _ => [0.0; 3],
                };
                let size = size_at(def, spawn, age, budget);
                let half = [size[0] * block, size[1] * block];
                let orient = facing_axes(def.facing, center, view, &emitter_axes, velocity);
                let color =
                    tint_color(&def.tint, w.tones, || tint_at(def, spawn, age, &[], budget));
                if budget.starved() {
                    return;
                }
                let quad = Quad {
                    texture,
                    blend: def.blend,
                    color,
                    corners: billboard(center, orient, half, uv_at(def, spawn, age), view),
                    emissive: def.emissive,
                };
                sink.push(&quad);
            };
            for_each_live(
                e,
                index as u16,
                start,
                stop,
                w.now,
                w.epoch,
                w.seed_epoch,
                w.seed,
                budget,
                live,
            );
        }
    }
}

pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn length(a: [f32; 3]) -> f32 {
    sqrtf(a[0] * a[0] + a[1] * a[1] + a[2] * a[2])
}

pub fn unit(a: [f32; 3]) -> Option<[f32; 3]> {
    let len = length(a);
    (len > TINY && len.is_finite()).then(|| scale(a, 1.0 / len))
}

#[cfg(test)]
pub(in crate::effects) mod tests {
    use super::*;
    use crate::attach::PlayerLocator;

    pub(in crate::effects) fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    pub(in crate::effects) fn view() -> View {
        View {
            camera: [0.0, 0.0, 10.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            origin_shift: [0.0; 3],
        }
    }

    #[test]
    fn effects_quads_write_texture_blend_color_then_four_corners() {
        let corners = billboard(
            [1.0, 2.0, 3.0],
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            [0.5, 0.25],
            [0.0, 0.5, 0.5, 1.0],
            &view(),
        );
        let quad = Quad {
            texture: 3,
            blend: Blend::Additive,
            color: [1.0, 0.5, 0.25, 0.75],
            corners,
            emissive: true,
        };
        let mut out = [f32::NAN; QUAD_FLOATS];
        quad.write(&mut out);
        assert_eq!(QUAD_FLOATS, 6 + 4 * CORNER_FLOATS + 1);
        assert_eq!(QUAD_FLAGS, 6 + 4 * CORNER_FLOATS);
        let want = [
            3.0,
            BLEND_ADDITIVE,
            1.0,
            0.5,
            0.25,
            0.75,
            0.5,
            1.75,
            -7.0,
            0.0,
            1.0,
            1.5,
            1.75,
            -7.0,
            0.5,
            1.0,
            1.5,
            2.25,
            -7.0,
            0.5,
            0.5,
            0.5,
            2.25,
            -7.0,
            0.0,
            0.5,
            1.0,
        ];
        assert!(close(&out, &want), "{out:?}");

        let mut short = [0.0; QUAD_FLOATS + 3];
        let mut sink = QuadSink::new(&mut short);
        sink.push(&quad);
        assert!(sink.fits());
        sink.push(&quad);
        assert_eq!(
            (sink.count(), sink.needed(), sink.fits()),
            (2, 2 * QUAD_FLOATS, false)
        );
        assert_eq!(
            short[QUAD_FLOATS..],
            [0.0; 3],
            "nothing past the last whole quad"
        );
    }

    pub(in crate::effects) fn locators(root: Affine) -> (Vec<Affine>, Vec<Affine>) {
        let player = PlayerLocator::ALL
            .iter()
            .map(|l| root.mul(&Affine::translation(l.offset())))
            .collect();
        let model = vec![root.mul(&Affine::translation([0.0, 4.0, -6.0]))];
        (model, player)
    }

    pub(in crate::effects) fn centers(out: &[f32], quads: usize) -> Vec<[f32; 3]> {
        (0..quads)
            .map(|q| {
                let quad = &out[q * QUAD_FLOATS..];
                [0, 1, 2].map(|i| {
                    (quad[QUAD_CORNERS + i] + quad[QUAD_CORNERS + 2 * CORNER_FLOATS + i]) / 2.0
                })
            })
            .collect()
    }
}
