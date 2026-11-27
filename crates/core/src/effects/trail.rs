use super::geometry::{
    add, block_scale, cross, length, origin, scale, sub, tint_color, unit, Quad, QuadSink,
    WearerEffects,
};
use super::particles::gradient;
use super::ring::{Ring, RING_STEP_SECONDS};
use super::{Tint, TrailDef, View, CORNER_FLOATS};
use crate::limits::{EFFECT_RING_HZ, TRAIL_POINTS};
use crate::signals::SignalState;
use crate::wearer::RING_POINTS;

pub const MAX_POINTS: usize = TRAIL_POINTS as usize;
pub const MAX_WINDOWS: usize = RING_POINTS + 1;

const SAME_TIME: f32 = 1e-4;
const SAME_PLACE: f32 = 1e-6;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrailPoint {
    pub at: [f32; 3],
    pub age: f32,
}

fn gate_mask(gates: &[u8]) -> u32 {
    gates
        .iter()
        .filter_map(|&gate| 1u32.checked_shl(u32::from(gate)))
        .fold(0, |mask, bit| mask | bit)
}

pub fn windows(
    gates: &[u8],
    ring: Ring<'_>,
    signals: &SignalState,
    now: f32,
    out: &mut [[f32; 2]; MAX_WINDOWS],
) -> usize {
    let mask = gate_mask(gates);
    let open = |bits: Option<u32>| gates.is_empty() || bits.is_some_and(|bits| bits & mask != 0);
    let mut n = 0usize;
    let mut close = |run: &mut Option<[f32; 2]>, out: &mut [[f32; 2]; MAX_WINDOWS]| {
        if let Some(window) = run.take().filter(|_| n < MAX_WINDOWS) {
            out[n] = window;
            n += 1;
        }
    };
    let mut run = open(Some(signals.gates_at(now))).then_some([0.0f32, 0.0]);
    if let Some(newest) = ring.newest_time() {
        if ring.breaks_before_head() {
            close(&mut run, out);
        }
        for back in 0..ring.len() {
            let age = (now - (newest - back as f32 * RING_STEP_SECONDS)).max(0.0);
            if open(ring.gate(back)) {
                match &mut run {
                    Some(window) => window[1] = age,
                    None => run = Some([age, age]),
                }
            } else {
                close(&mut run, out);
            }
            if ring.starts_segment(back) {
                close(&mut run, out);
            }
        }
    }
    if let Some(window) = run.as_mut().filter(|_| gates.is_empty()) {
        window[1] = f32::INFINITY;
    }
    close(&mut run, out);
    n
}

pub fn points(
    def: &TrailDef,
    ring: Ring<'_>,
    head: [f32; 3],
    now: f32,
    window: [f32; 2],
    out: &mut [TrailPoint; MAX_POINTS],
) -> usize {
    let fade = def.fade_seconds;
    if fade.is_nan() || fade <= 0.0 {
        return 0;
    }
    let mut n = 0;
    let newest = ring.newest_time();
    let head_is_newest = newest.is_some_and(|t| now - t <= SAME_TIME);
    if window[0] <= 0.0 && !head_is_newest {
        out[n] = TrailPoint { at: head, age: 0.0 };
        n += 1;
    }
    let Some(newest) = newest else {
        return n;
    };
    for back in 0..ring.len() {
        if n == MAX_POINTS {
            break;
        }
        let age = (now - (newest - back as f32 * RING_STEP_SECONDS)).max(0.0);
        if age >= fade || age > window[1] {
            break;
        }
        if age < window[0] {
            continue;
        }
        if let Some(at) = ring.point(back) {
            out[n] = TrailPoint { at, age };
            n += 1;
        }
    }
    n
}

pub fn draw(
    def: &TrailDef,
    ring: Ring<'_>,
    w: &WearerEffects<'_>,
    view: &View,
    texture: u32,
    sink: &mut QuadSink<'_>,
) {
    let frame = w.locators.frame(def.locator);
    let mut open = [[0.0; 2]; MAX_WINDOWS];
    let count = windows(&def.gates, ring, w.signals, w.now, &mut open);
    let mut found = [TrailPoint::default(); MAX_POINTS];
    for &window in &open[..count] {
        let n = points(def, ring, origin(&frame), w.now, window, &mut found);
        ribbon(
            def,
            &found[..n],
            block_scale(&frame),
            view,
            texture,
            w.tones,
            sink,
        );
    }
}

pub fn ribbon(
    def: &TrailDef,
    points: &[TrailPoint],
    block: f32,
    view: &View,
    texture: u32,
    tones: &[u32],
    sink: &mut QuadSink<'_>,
) {
    let fade = def.fade_seconds;
    if points.len() < 2 || fade.is_nan() || fade <= 0.0 {
        return;
    }
    let last = points.len() - 1;
    let side = |i: usize| {
        let ahead = points[i.saturating_sub(1)].at;
        let behind = points[(i + 1).min(last)].at;
        let tangent = unit(sub(ahead, behind))?;
        let toward = unit(sub(view.camera, points[i].at))?;
        let across = unit(cross(tangent, toward))?;
        let along = (points[i].age / fade).clamp(0.0, 1.0);
        let width = def.width[0] + (def.width[1] - def.width[0]) * along;
        Some(scale(across, width * block / 2.0))
    };
    for i in 0..last {
        let (a, b) = (points[i], points[i + 1]);
        let span = b.age - a.age;
        let distance = length(sub(a.at, b.at));
        if distance <= SAME_PLACE {
            continue;
        }
        if def.min_speed > 0.0 && (span <= 0.0 || distance / block / span < def.min_speed) {
            continue;
        }
        let (Some(sa), Some(sb)) = (side(i), side(i + 1)) else {
            continue;
        };
        let middle = ((a.age + b.age) / 2.0 / fade).clamp(0.0, 1.0);
        let mut color = tint_color(&def.tint, tones, || match &def.tint {
            Tint::Fixed(color) => *color,
            Tint::Gradient { keys, .. } => gradient(keys, middle),
            _ => u32::MAX,
        });
        color[3] *= 1.0 - middle;
        let (ua, ub) = if def.tile {
            ((1.0 - span * EFFECT_RING_HZ as f32).clamp(0.0, 1.0), 1.0)
        } else {
            (a.age / fade, b.age / fade)
        };
        let corner = |p: [f32; 3], u: f32, v: f32| -> [f32; CORNER_FLOATS] {
            let p = sub(p, view.camera);
            [p[0], p[1], p[2], u, v]
        };
        let quad = Quad {
            texture,
            blend: def.blend,
            color,
            corners: [
                corner(sub(a.at, sa), ua, 1.0),
                corner(sub(b.at, sb), ub, 1.0),
                corner(add(b.at, sb), ub, 0.0),
                corner(add(a.at, sa), ua, 0.0),
            ],
            emissive: false,
        };
        sink.push(&quad);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::format::tests::sample;
    use crate::effects::geometry::tests::{close, view};
    use crate::effects::ring::tests::Owned;
    use crate::effects::{QUAD_COLOR, QUAD_CORNERS, QUAD_FLOATS};

    fn trail(fade: f32) -> TrailDef {
        let mut def = sample().trails.remove(0);
        def.fade_seconds = fade;
        def.gates.clear();
        def
    }

    fn walked_in(room: usize, seconds: f32, now: f32) -> Owned {
        let mut ring = Owned::new(room);
        let mut t = now - seconds;
        while t <= now + 1e-4 {
            ring.sample([t, 0.0, 0.0], t);
            t += 1.0 / 120.0;
        }
        ring
    }

    fn walked(seconds: f32, now: f32) -> Owned {
        walked_in(RING_POINTS, seconds, now)
    }

    #[test]
    fn effects_trails_fade_and_narrow_along_their_length() {
        let def = trail(1.0);
        let now = 20.0;
        let ring = walked(2.0, now);
        let mut found = [TrailPoint::default(); MAX_POINTS];
        let n = points(
            &def,
            ring.ring(),
            [now, 0.0, 0.0],
            now,
            [0.0, f32::INFINITY],
            &mut found,
        );
        let mut out = vec![0.0; 64 * QUAD_FLOATS];
        let mut sink = QuadSink::new(&mut out);
        let camera = View {
            camera: [now, 0.0, 10.0],
            ..view()
        };
        ribbon(&def, &found[..n], 1.0, &camera, 4, &[], &mut sink);
        let quads = sink.count();
        assert_eq!(quads, n - 1);
        let alpha: Vec<f32> = (0..quads)
            .map(|q| out[q * QUAD_FLOATS + QUAD_COLOR + 3])
            .collect();
        assert!(alpha.windows(2).all(|a| a[0] > a[1]), "{alpha:?}");
        assert!(alpha[0] > 0.9 && alpha[quads - 1] < 0.1);
        let corner = |q: usize, c: usize, k: usize| out[q * QUAD_FLOATS + QUAD_CORNERS + c * 5 + k];
        let first_width = (corner(0, 3, 1) - corner(0, 0, 1)).abs();
        assert!(
            (first_width - 0.3).abs() < 1e-3,
            "head is 0.3 wide: {first_width}"
        );
        let tail_width = (corner(quads - 1, 2, 1) - corner(quads - 1, 1, 1)).abs();
        assert!(tail_width < 0.02, "tail narrows to 0: {tail_width}");
        assert!(
            close(&[corner(0, 0, 2), corner(0, 3, 2)], &[-10.0, -10.0]),
            "faces the camera"
        );
        assert!(close(
            &[corner(0, 0, 3), corner(0, 0, 4), corner(0, 3, 4)],
            &[0.0, 1.0, 0.0]
        ));
    }
}
