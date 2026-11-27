use crate::container::Image;
use crate::limits::{NAMEPLATE_FALLBACK_BYTES, NAMEPLATE_IMAGE_OVERHEAD_BYTES};
use crate::molang::{bayer, hash};
use crate::nameplate::geom::Rect;
use crate::nameplate::image::RefKind;
use crate::nameplate::types::SPRITE_CLEAR;
use crate::palette::MAX_TONES;

use super::ir::{
    MaterialFrame, MaterialIr, Op, OpKind, Period, Reduce, Reg, TexRef, Ty, CLOCK_CYCLE, CLOCK_FPS,
    CLOCK_FRAME, CLOCK_STEP, COVERAGE_THRESHOLD, LANE_COUNT,
};

type V4 = [f64; 4];

const ZERO: V4 = [0.0; 4];
const NO_REF: V4 = [-1.0, 0.0, 0.0, 0.0];
const POS: u8 = 1;
const FRAME: u8 = 2;
const INDEX: u8 = 4;
const TOP: usize = usize::MAX;

pub trait TextureSource {
    fn texture(&self, texture: &TexRef) -> Option<&Image>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoTextures;

impl TextureSource for NoTextures {
    fn texture(&self, _texture: &TexRef) -> Option<&Image> {
        None
    }
}

pub fn div(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        a / b
    }
}

pub fn floor_mod(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        a - b * libm::floor(a / b)
    }
}

pub fn safe_pow(base: f64, power: f64) -> f64 {
    if base > 0.0 {
        libm::pow(base, power)
    } else {
        0.0
    }
}

pub fn safe_atan2(y: f64, x: f64) -> f64 {
    if x == 0.0 && y == 0.0 {
        0.0
    } else {
        libm::atan2(y, x)
    }
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    a * (1.0 - t) + b * t
}

fn saturate(v: f64) -> f64 {
    if v > 1.0 {
        1.0
    } else if v > 0.0 {
        v
    } else {
        0.0
    }
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = saturate(div(x - edge0, edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

fn smooth(f: f64) -> f64 {
    f * f * (3.0 - 2.0 * f)
}

fn cell(v: f64) -> i32 {
    libm::floor(v) as i32
}

fn wrap(x: i32, period: i32) -> i32 {
    if period <= 0 {
        x
    } else {
        x.rem_euclid(period)
    }
}

fn following(cell: i32, wrapped: i32, period: i32) -> i32 {
    if period <= 0 || cell == i32::MAX {
        wrap(cell.wrapping_add(1), period)
    } else if wrapped + 1 == period {
        0
    } else {
        wrapped + 1
    }
}

pub fn noise(x: f64, y: f64, period_x: i32, period_y: i32, seed: i32) -> f64 {
    let cell_x = libm::floor(x);
    let cell_y = libm::floor(y);
    let fx = smooth(x - cell_x);
    let fy = smooth(y - cell_y);
    let (ix, iy) = (cell_x as i32, cell_y as i32);
    let x0 = wrap(ix, period_x);
    let y0 = wrap(iy, period_y);
    let x1 = following(ix, x0, period_x);
    let y1 = following(iy, y0, period_y);
    let top = mix(hash(x0, y0, seed), hash(x1, y0, seed), fx);
    let bottom = mix(hash(x0, y1, seed), hash(x1, y1, seed), fx);
    mix(top, bottom, fy)
}

pub fn fbm(x: f64, y: f64, period_x: i32, period_y: i32, octaves: i32, seed: i32) -> f64 {
    let (mut px, mut py) = (x, y);
    let (mut tile_x, mut tile_y) = (period_x, period_y);
    let mut sum = 0.0;
    let mut weight = 1.0;
    let mut total = 0.0;
    for octave in 0..octaves.max(1) {
        sum += noise(px, py, tile_x, tile_y, seed.wrapping_add(octave)) * weight;
        total += weight;
        px *= 2.0;
        py *= 2.0;
        tile_x = tile_x.saturating_mul(2);
        tile_y = tile_y.saturating_mul(2);
        weight *= 0.5;
    }
    sum / total
}

fn hsv(h: f64, s: f64, v: f64) -> V4 {
    let h6 = (h - libm::floor(h)) * 6.0;
    let sector = libm::floor(h6);
    let f = h6 - sector;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let rgb = match (sector as i64).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    };
    [rgb[0], rgb[1], rgb[2], 1.0]
}

fn level(value: f64, tones: i64, threshold: f64) -> Option<i64> {
    let raw = libm::floor(value * (tones - 1) as f64 + threshold);
    raw.is_finite().then(|| (raw as i64).clamp(0, tones - 1))
}

fn unpack(argb: u32) -> V4 {
    let channel = |shift: u32| f64::from((argb >> shift) & 0xff) / 255.0;
    [channel(16), channel(8), channel(0), channel(24)]
}

fn byte(value: f64) -> u32 {
    let scaled = libm::floor(value * 255.0 + 0.5);
    if scaled.is_nan() || scaled <= 0.0 {
        0
    } else if scaled >= 255.0 {
        255
    } else {
        scaled as u32
    }
}

pub fn pack(color: [f64; 4]) -> u32 {
    (byte(color[3]) << 24) | (byte(color[0]) << 16) | (byte(color[1]) << 8) | byte(color[2])
}

fn reg_index(reg: Reg) -> Option<usize> {
    (!reg.is_none()).then_some(usize::from(reg.0))
}

#[derive(Clone, Copy, Debug)]
struct Env {
    x: f64,
    y: f64,
    frame: f64,
    index: f64,
}

struct Plan {
    bodies: Vec<Vec<usize>>,
    uniform: Vec<usize>,
    varying: Vec<usize>,
    registers: usize,
}

fn own_bits(kind: OpKind) -> u8 {
    match kind {
        OpKind::PixelPos
        | OpKind::PixelCell
        | OpKind::TagUv
        | OpKind::ScreenPos
        | OpKind::PixelScreenPos => POS,
        OpKind::Frame | OpKind::SteppedTime => FRAME,
        OpKind::RepeatIndex => INDEX,
        _ => 0,
    }
}

fn plan(ir: &MaterialIr) -> Plan {
    let count = ir.ops.len();
    let registers = ir
        .ops
        .iter()
        .flat_map(|op| op.args.iter().chain([&op.dst]))
        .filter_map(|r| reg_index(*r))
        .max()
        .map_or(count, |m| (m + 1).max(count));
    let mut owners: Vec<(usize, usize, usize)> = ir
        .ops
        .iter()
        .enumerate()
        .filter_map(|(k, op)| {
            op.body().map(|body| {
                let end = body.end.min(k);
                (k, body.start.min(end), end)
            })
        })
        .collect();
    owners.sort_by_key(|&(_, start, end)| std::cmp::Reverse(end - start));
    let mut scope = vec![TOP; count];
    for &(owner, start, end) in &owners {
        for slot in &mut scope[start..end] {
            *slot = owner;
        }
    }
    let mut masks = vec![0u8; registers];
    for op in &ir.ops {
        let mut mask = own_bits(op.kind);
        for (k, arg) in op.args.iter().enumerate() {
            let Some(at) = reg_index(*arg) else {
                continue;
            };
            let bits = masks.get(at).copied().unwrap_or(0);
            mask |= if op.kind == OpKind::Repeat && k == 0 {
                bits & !INDEX
            } else {
                bits
            };
        }
        if let Some(slot) = reg_index(op.dst).and_then(|d| masks.get_mut(d)) {
            *slot = mask;
        }
    }
    let mut bodies = vec![Vec::new(); count];
    let mut uniform = Vec::new();
    let mut varying = Vec::new();
    for (k, owner) in scope.iter().enumerate() {
        if *owner != TOP {
            bodies[*owner].push(k);
            continue;
        }
        let mask = reg_index(ir.ops[k].dst)
            .and_then(|d| masks.get(d))
            .copied()
            .unwrap_or(POS);
        if mask & POS != 0 {
            varying.push(k);
        } else {
            uniform.push(k);
        }
    }
    Plan {
        bodies,
        uniform,
        varying,
        registers,
    }
}

struct Machine<'m, 'f> {
    ir: &'m MaterialIr,
    frame: &'m MaterialFrame<'f>,
    textures: &'m [Image],
    source: &'m dyn TextureSource,
    plan: &'m Plan,
    regs: Vec<V4>,
    origin: [f64; 2],
    size: [f64; 2],
}

impl<'m, 'f> Machine<'m, 'f> {
    fn new(
        ir: &'m MaterialIr,
        frame: &'m MaterialFrame<'f>,
        textures: &'m [Image],
        source: &'m dyn TextureSource,
        plan: &'m Plan,
    ) -> Machine<'m, 'f> {
        Machine {
            ir,
            frame,
            textures,
            source,
            plan,
            regs: vec![ZERO; plan.registers],
            origin: [
                f64::from(frame.area.x0 - frame.body.x0),
                f64::from(frame.area.y0 - frame.body.y0),
            ],
            size: [
                f64::from(frame.area.width()),
                f64::from(frame.area.height()),
            ],
        }
    }

    fn get(&self, reg: Reg) -> V4 {
        reg_index(reg)
            .and_then(|i| self.regs.get(i))
            .copied()
            .unwrap_or(ZERO)
    }

    fn run(&mut self, list: &[usize], env: Env) {
        for &k in list {
            self.step(k, env);
        }
    }

    fn run_body(&mut self, owner: usize, env: Env) {
        let plan = self.plan;
        self.run(&plan.bodies[owner], env);
    }

    fn tone(&self, value: f64) -> V4 {
        if !value.is_finite() || value < 0.0 {
            return ZERO;
        }
        self.frame
            .tones
            .get(value as usize)
            .copied()
            .map_or(ZERO, unpack)
    }

    fn lane(&self, index: f64) -> f64 {
        if index.is_nan() || index < 0.0 {
            return 0.0;
        }
        self.frame.lanes.get(index as usize).copied().unwrap_or(0.0)
    }

    fn uv(&self, x: f64, y: f64) -> [f64; 2] {
        [
            div(x - self.origin[0], self.size[0]),
            div(y - self.origin[1], self.size[1]),
        ]
    }

    fn sample(&self, texture: f64, u: f64, v: f64) -> V4 {
        let Some(reference) = self.ir.textures.get(texture.max(0.0) as usize) else {
            return ZERO;
        };
        let image = match reference {
            TexRef::Bundle(index) => self.textures.get(usize::from(*index)),
            other => self.source.texture(other),
        };
        let Some(image) = image else {
            return ZERO;
        };
        let (w, h) = (i64::from(image.width), i64::from(image.height));
        if w == 0 || h == 0 {
            return ZERO;
        }
        let x = (libm::floor(u * w as f64) as i64).rem_euclid(w);
        let y = (libm::floor(v * h as f64) as i64).rem_euclid(h);
        let at = ((y * w + x) * 4) as usize;
        match image.rgba.get(at..at + 4) {
            Some(px) => [
                f64::from(px[0]) / 255.0,
                f64::from(px[1]) / 255.0,
                f64::from(px[2]) / 255.0,
                f64::from(px[3]) / 255.0,
            ],
            None => ZERO,
        }
    }

    fn sprite(&self, index: f64, at: V4, frame: f64) -> f64 {
        let Some(sprite) = self.ir.sprites.get(index.max(0.0) as usize) else {
            return -1.0;
        };
        let (x, y) = (libm::floor(at[0]), libm::floor(at[1]));
        let (w, h) = (f64::from(sprite.width), f64::from(sprite.height));
        if !(x >= 0.0 && y >= 0.0 && x < w && y < h) || sprite.frames.is_empty() {
            return -1.0;
        }
        let frames = sprite.frames.len() as i64;
        let f = if frame.is_finite() {
            (libm::floor(frame) as i64).rem_euclid(frames) as usize
        } else {
            0
        };
        let at = (y as usize) * usize::from(sprite.width) + x as usize;
        match sprite.frames[f].get(at) {
            Some(&c) if c != SPRITE_CLEAR => match sprite.colors.get(usize::from(c)) {
                Some(color) if color.kind() == RefKind::Slot => f64::from(color.tone_ref()),
                _ => -1.0,
            },
            _ => -1.0,
        }
    }

    fn body_edge(&self, p: V4, base: f64, outer: bool) -> f64 {
        let body = self.frame.body;
        let half = [
            f64::from(body.width()) / 2.0,
            f64::from(body.height()) / 2.0,
        ];
        let (cx, cy) = (p[0] - half[0], p[1] - half[1]);
        let corner = match (cx < 0.0, cy < 0.0) {
            (true, true) => 0.0,
            (false, true) => 1.0,
            (false, false) => 2.0,
            (true, false) => 3.0,
        };
        let grow = if outer { self.lane(base + 4.0) } else { 0.0 };
        let radius = self.lane(base + corner) + grow;
        let qx = cx.abs() - half[0] - grow + radius;
        let qy = cy.abs() - half[1] - grow + radius;
        let (ox, oy) = (qx.max(0.0), qy.max(0.0));
        qx.max(qy).min(0.0) + libm::sqrt(ox * ox + oy * oy) - radius
    }

    fn elementwise(op: &Op, a: V4, b: V4, c: V4) -> V4 {
        let mut out = ZERO;
        for (k, slot) in out.iter_mut().enumerate().take(usize::from(op.ty.width())) {
            let (x, y, z) = (a[k], b[k], c[k]);
            *slot = match op.kind {
                OpKind::Abs => x.abs(),
                OpKind::Floor => libm::floor(x),
                OpKind::Frac => x - libm::floor(x),
                OpKind::Sin => libm::sin(x),
                OpKind::Cos => libm::cos(x),
                OpKind::Exp => libm::exp(x),
                OpKind::Add => x + y,
                OpKind::Sub => x - y,
                OpKind::Mul => x * y,
                OpKind::Div => div(x, y),
                OpKind::Mod => floor_mod(x, y),
                OpKind::Min => x.min(y),
                OpKind::Max => x.max(y),
                OpKind::Step => {
                    if y < x {
                        0.0
                    } else {
                        1.0
                    }
                }
                OpKind::Pow => safe_pow(x, y),
                OpKind::Atan2 => safe_atan2(x, y),
                OpKind::Clamp => x.max(y).min(z),
                OpKind::Mix => mix(x, y, z),
                OpKind::Smoothstep => smoothstep(x, y, z),
                _ => 0.0,
            };
        }
        out
    }

    fn ramp(op: &Op, a: V4, b: V4) -> V4 {
        let base = op.imm[0] as i64 * MAX_TONES as i64;
        let tones = (op.imm[1] as i64).max(1);
        let picked = match op.kind {
            OpKind::Quantize => level(a[0], tones, 0.5),
            OpKind::DitherQuantize => level(a[0], tones, bayer(cell(b[0]), cell(b[1]))),
            _ => {
                let sum = libm::floor(a[0]) + libm::floor(b[0]);
                sum.is_finite().then(|| (sum as i64).rem_euclid(tones))
            }
        };
        match picked {
            Some(tone) => [(base + tone) as f64, 0.0, 0.0, 0.0],
            None => NO_REF,
        }
    }

    fn step(&mut self, k: usize, env: Env) {
        let op = self.ir.ops[k];
        let a = self.get(op.args[0]);
        let b = self.get(op.args[1]);
        let c = self.get(op.args[2]);
        let width = usize::from(op.ty.width());
        let clock = self.frame.clock;
        let scalar = |v: f64| [v, 0.0, 0.0, 0.0];
        let value: V4 = match op.kind {
            OpKind::PixelPos => [env.x, env.y, 0.0, 0.0],
            OpKind::PixelCell => [libm::floor(env.x), libm::floor(env.y), 0.0, 0.0],
            OpKind::TagUv | OpKind::ScreenPos => {
                let [u, v] = self.uv(env.x, env.y);
                [u, v, 0.0, 0.0]
            }
            OpKind::PixelScreenPos => {
                let [u, v] = self.uv(libm::floor(env.x) + 0.5, libm::floor(env.y) + 0.5);
                [u, v, 0.0, 0.0]
            }
            OpKind::AreaSize => [self.size[0], self.size[1], 0.0, 0.0],
            OpKind::BodyRect => {
                let body = self.frame.body;
                [body.x0, body.y0, body.x1, body.y1].map(f64::from)
            }
            OpKind::Frame => scalar(env.frame),
            OpKind::SteppedTime => scalar(div(
                env.frame * f64::from(clock[CLOCK_STEP]),
                f64::from(clock[CLOCK_FPS]),
            )),
            OpKind::RepeatIndex => scalar(env.index),
            OpKind::Slot if op.ty == Ty::Vec4 => self.tone(a[0]),
            OpKind::Slot => scalar(op.imm[0]),
            OpKind::Constant => {
                let mut out = op.imm;
                for slot in out.iter_mut().skip(width) {
                    *slot = 0.0;
                }
                out
            }
            OpKind::Param | OpKind::Expr => scalar(self.lane(op.imm[0])),
            OpKind::BodyEdge => scalar(self.body_edge(a, op.imm[0], op.imm[1] != 0.0)),
            OpKind::Length | OpKind::Dot => {
                let n = (op.imm[0] as usize).clamp(1, 4);
                let other = if op.kind == OpKind::Length { a } else { b };
                let sum: f64 = (0..n).map(|i| a[i] * other[i]).sum();
                scalar(if op.kind == OpKind::Length {
                    libm::sqrt(sum)
                } else {
                    sum
                })
            }
            OpKind::Polar => [
                libm::sqrt(a[0] * a[0] + a[1] * a[1]),
                safe_atan2(a[1], a[0]),
                0.0,
                0.0,
            ],
            OpKind::Combine => {
                let mut out = ZERO;
                for (i, slot) in out.iter_mut().enumerate().take(width) {
                    *slot = self.get(op.args[i])[0];
                }
                out
            }
            OpKind::Component => scalar(a[(op.imm[0] as usize).min(3)]),
            OpKind::Select => {
                if a[0] > 0.0 {
                    c
                } else {
                    b
                }
            }
            OpKind::Noise => scalar(noise(
                a[0],
                a[1],
                op.imm[1] as i32,
                op.imm[2] as i32,
                op.imm[0] as i32,
            )),
            OpKind::Fbm => scalar(fbm(
                a[0],
                a[1],
                op.imm[1] as i32,
                op.imm[2] as i32,
                op.imm[3] as i32,
                op.imm[0] as i32,
            )),
            OpKind::Hash => scalar(hash(cell(a[0]), cell(a[1]), op.imm[0] as i32)),
            OpKind::Bayer => scalar(bayer(cell(a[0]), cell(a[1]))),
            OpKind::Checker => {
                let size = op.imm[0];
                let sum = libm::floor(div(a[0], size)) + libm::floor(div(a[1], size));
                scalar((sum as i64).rem_euclid(2) as f64)
            }
            OpKind::Stripes => {
                let along = i64::from(cell(a[0])) * op.imm[2] as i64
                    + i64::from(cell(a[1])) * op.imm[3] as i64;
                let period = (op.imm[0] as i64).max(1);
                scalar(f64::from(u8::from(
                    along.rem_euclid(period) < op.imm[1] as i64,
                )))
            }
            OpKind::Gradient => {
                let (dx, dy) = (c[0] - b[0], c[1] - b[1]);
                let along = (a[0] - b[0]) * dx + (a[1] - b[1]) * dy;
                scalar(saturate(div(along, dx * dx + dy * dy)))
            }
            OpKind::Hsv => hsv(a[0], b[0], c[0]),
            OpKind::Quantize | OpKind::DitherQuantize | OpKind::Cycle => Self::ramp(&op, a, b),
            OpKind::Lighten => {
                let t = b[0];
                [
                    a[0] + (1.0 - a[0]) * t,
                    a[1] + (1.0 - a[1]) * t,
                    a[2] + (1.0 - a[2]) * t,
                    a[3],
                ]
            }
            OpKind::Darken => {
                let t = b[0];
                [a[0] * (1.0 - t), a[1] * (1.0 - t), a[2] * (1.0 - t), a[3]]
            }
            OpKind::Alpha => [a[0], a[1], a[2], a[3] * b[0]],
            OpKind::SampleAt => {
                let moved = Env {
                    x: env.x + a[0],
                    y: env.y + a[1],
                    frame: env.frame + b[0],
                    index: env.index,
                };
                self.run_body(k, moved);
                self.get(op.args[2])
            }
            OpKind::Repeat => self.repeat(k, &op, env),
            OpKind::Sprite => scalar(self.sprite(op.imm[0], a, b[0])),
            OpKind::BundleTexture | OpKind::VanillaTexture | OpKind::AtlasSprite => {
                self.sample(op.imm[0], a[0], a[1])
            }
            _ => Self::elementwise(&op, a, b, c),
        };
        if let Some(slot) = reg_index(op.dst).and_then(|d| self.regs.get_mut(d)) {
            *slot = value;
        }
    }

    fn repeat(&mut self, k: usize, op: &Op, env: Env) -> V4 {
        let count = if op.imm[2].is_finite() {
            op.imm[2].max(0.0) as usize
        } else {
            0
        };
        let reduce = Reduce::from_value(op.imm[3]);
        let width = usize::from(op.ty.width());
        let slot = op.ty == Ty::Slot;
        let mut acc = if slot { NO_REF } else { ZERO };
        let mut found = false;
        for i in 0..count {
            let inner = Env {
                index: i as f64,
                ..env
            };
            self.run_body(k, inner);
            let value = self.get(op.args[0]);
            match reduce {
                Reduce::Sum => {
                    for (sum, v) in acc.iter_mut().zip(value).take(width) {
                        *sum += v;
                    }
                }
                Reduce::Min | Reduce::Max if i == 0 => acc = value,
                Reduce::Min => {
                    for (low, v) in acc.iter_mut().zip(value).take(width) {
                        *low = low.min(v);
                    }
                }
                Reduce::Max => {
                    for (high, v) in acc.iter_mut().zip(value).take(width) {
                        *high = high.max(v);
                    }
                }
                Reduce::First => {
                    let last = value[width.max(1) - 1];
                    let hit = if slot { last >= 0.0 } else { last > 0.0 };
                    if !found && hit {
                        acc = value;
                        found = true;
                    }
                }
            }
        }
        acc
    }

    fn color_type(&self) -> Ty {
        reg_index(self.ir.color)
            .and_then(|at| self.ir.ops.iter().find(|op| reg_index(op.dst) == Some(at)))
            .map_or(Ty::Vec4, |op| op.ty)
    }

    fn pixel(&self, slot_color: bool) -> u32 {
        let coverage = self.ir.coverage.map_or(1.0, |reg| self.get(reg)[0]);
        if coverage.is_nan() || coverage <= 0.0 {
            return 0;
        }
        let raw = self.get(self.ir.color);
        let color = if slot_color { self.tone(raw[0]) } else { raw };
        if color[3].is_nan() || color[3] < COVERAGE_THRESHOLD {
            return 0;
        }
        pack(color)
    }
}

fn top_env(frame: &MaterialFrame<'_>) -> Env {
    Env {
        x: 0.5,
        y: 0.5,
        frame: f64::from(frame.clock[CLOCK_FRAME]),
        index: 0.0,
    }
}

pub fn evaluate(ir: &MaterialIr, frame: &MaterialFrame<'_>, textures: &[Image]) -> Vec<u32> {
    evaluate_with(ir, frame, textures, &NoTextures)
}

pub fn evaluate_with(
    ir: &MaterialIr,
    frame: &MaterialFrame<'_>,
    textures: &[Image],
    source: &dyn TextureSource,
) -> Vec<u32> {
    let area = frame.area;
    if area.is_empty() {
        return Vec::new();
    }
    let plan = plan(ir);
    let mut machine = Machine::new(ir, frame, textures, source, &plan);
    let slot_color = machine.color_type() == Ty::Slot;
    let grow = i32::from(ir.outline.is_some());
    let width = (area.width() + 2 * grow) as usize;
    let height = (area.height() + 2 * grow) as usize;
    let x0 = f64::from(area.x0 - frame.body.x0 - grow) + 0.5;
    let y0 = f64::from(area.y0 - frame.body.y0 - grow) + 0.5;
    let mut env = top_env(frame);
    env.x = x0;
    env.y = y0;
    machine.run(&plan.uniform, env);
    let mut grid = vec![0u32; width * height];
    for (j, row) in grid.chunks_mut(width).enumerate() {
        for (i, pixel) in row.iter_mut().enumerate() {
            env.x = x0 + i as f64;
            env.y = y0 + j as f64;
            machine.run(&plan.varying, env);
            *pixel = machine.pixel(slot_color);
        }
    }
    let Some(outline) = ir.outline else {
        return grid;
    };
    let ring = if outline.kind() == RefKind::Slot {
        pack(machine.tone(f64::from(outline.tone_ref())))
    } else {
        0
    };
    let mut out = Vec::with_capacity((width - 2) * (height - 2));
    for j in 1..height - 1 {
        for i in 1..width - 1 {
            let at = j * width + i;
            let edge = grid[at - 1] != 0
                || grid[at + 1] != 0
                || grid[at - width] != 0
                || grid[at + width] != 0;
            out.push(match grid[at] {
                0 if edge => ring,
                value => value,
            });
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FallbackBudget {
    cap: u32,
    used: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    pub area: Rect,
    pub pixels: Vec<u32>,
    pub bytes: u32,
}

impl FallbackBudget {
    pub const fn new() -> FallbackBudget {
        FallbackBudget::with_cap(NAMEPLATE_FALLBACK_BYTES)
    }

    pub const fn with_cap(cap: u32) -> FallbackBudget {
        FallbackBudget { cap, used: 0 }
    }

    pub const fn cap(&self) -> u32 {
        self.cap
    }

    pub const fn used(&self) -> u32 {
        self.used
    }

    pub fn release(&mut self, bytes: u32) {
        self.used = self.used.saturating_sub(bytes);
    }

    fn take(&mut self, bytes: u32) -> bool {
        match self.used.checked_add(bytes) {
            Some(total) if total <= self.cap => {
                self.used = total;
                true
            }
            _ => false,
        }
    }
}

impl Default for FallbackBudget {
    fn default() -> FallbackBudget {
        FallbackBudget::new()
    }
}

pub fn fallback_bytes(area: Rect) -> u32 {
    let pixels = if area.is_empty() {
        0
    } else {
        u64::from(area.width().unsigned_abs()) * u64::from(area.height().unsigned_abs())
    };
    let bytes = u64::from(NAMEPLATE_IMAGE_OVERHEAD_BYTES) + pixels * 4;
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

pub fn fallback(
    ir: &MaterialIr,
    frame: &MaterialFrame<'_>,
    textures: &[Image],
    budget: &mut FallbackBudget,
) -> Option<Fallback> {
    fallback_with(ir, frame, textures, &NoTextures, budget)
}

pub fn fallback_with(
    ir: &MaterialIr,
    frame: &MaterialFrame<'_>,
    textures: &[Image],
    source: &dyn TextureSource,
    budget: &mut FallbackBudget,
) -> Option<Fallback> {
    let bytes = fallback_bytes(frame.area);
    if !budget.take(bytes) {
        return None;
    }
    Some(Fallback {
        area: frame.area,
        pixels: evaluate_with(ir, frame, textures, source),
        bytes,
    })
}

pub fn loops(
    ir: &MaterialIr,
    frame: &MaterialFrame<'_>,
    textures: &[Image],
    lanes_at: &mut dyn FnMut(i32) -> [f64; LANE_COUNT],
) -> bool {
    let cycle = frame.clock[CLOCK_CYCLE];
    match ir.period {
        Period::Still => return true,
        Period::Frames(period) if period > 0 && i64::from(cycle) % i64::from(period) == 0 => {
            return true
        }
        _ => {}
    }
    let mut first = *frame;
    first.clock[CLOCK_FRAME] = 0;
    first.lanes = lanes_at(0);
    let mut wrapped = *frame;
    wrapped.clock[CLOCK_FRAME] = cycle;
    wrapped.lanes = lanes_at(cycle);
    evaluate(ir, &first, textures) == evaluate(ir, &wrapped, textures)
}

#[cfg(test)]
pub(crate) fn probe(
    ir: &MaterialIr,
    frame: &MaterialFrame<'_>,
    textures: &[Image],
    pixel: (i32, i32),
) -> Vec<[f64; 4]> {
    let plan = plan(ir);
    let mut machine = Machine::new(ir, frame, textures, &NoTextures, &plan);
    let mut env = top_env(frame);
    env.x = f64::from(frame.area.x0 - frame.body.x0 + pixel.0) + 0.5;
    env.y = f64::from(frame.area.y0 - frame.body.y0 + pixel.1) + 0.5;
    machine.run(&plan.uniform, env);
    machine.run(&plan.varying, env);
    machine.regs
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::issues::{Mode, Report};
    use crate::molang::hash;
    use crate::nameplate::material::graph::build_named;
    use crate::nameplate::material::graph::test_support::{context, nodes, Fixture, FILL, GLOW};
    use crate::nameplate::material::ir::{
        Lane, Period, Reg, VanillaTexture, CLOCK_CYCLE, CLOCK_FRAME,
    };
    use crate::nameplate::types::PALETTE_TONES;

    fn palette() -> Vec<u32> {
        let mut tones = vec![0u32; PALETTE_TONES];
        tones[..4].copy_from_slice(&FILL);
        tones[16..18].copy_from_slice(&GLOW);
        tones
    }

    fn dots() -> Vec<Image> {
        let rgba = vec![
            10, 20, 30, 255, 40, 50, 60, 128, 70, 80, 90, 0, 100, 110, 120, 255,
        ];
        vec![Image {
            width: 2,
            height: 2,
            rgba,
        }]
    }

    fn frame<'t>(ir: &MaterialIr, tones: &'t [u32]) -> MaterialFrame<'t> {
        let body = Rect::new(0, 0, 10, 4);
        let mut lanes = [0.0; LANE_COUNT];
        lanes[0] = 2.0;
        MaterialFrame {
            area: ir.area(body),
            body,
            clock: [3, 12, 12, 1],
            lanes,
            tones,
            atlas: [[0.0; 4]; 4],
        }
    }

    fn compile_file(file: Value) -> (MaterialIr, Vec<(String, Reg)>) {
        let mut fixture = Fixture::new();
        let mut report = Report::new(Mode::Authoring);
        let built = build_named("m", &file, &mut context(&mut fixture), &mut report);
        assert!(report.issues().is_empty(), "{:?}", report.issues());
        built.expect("built")
    }

    fn compile(source: &str) -> MaterialIr {
        compile_file(json!({ "nodes": nodes(source) })).0
    }

    fn pixels(source: &str) -> Vec<u32> {
        let ir = compile(source);
        evaluate(&ir, &frame(&ir, &palette()), &[])
    }

    fn value_at(source: &str, pixel: (i32, i32), lanes: &[(usize, f64)]) -> [f64; 4] {
        let (ir, named) = compile_file(
            json!({ "nodes": nodes(&format!("{source}; probe:Component v=r; out:Output color=[0,0,0,1] coverage=probe")) }),
        );
        let tones = palette();
        let mut frame = frame(&ir, &tones);
        for (lane, value) in lanes {
            frame.lanes[*lane] = *value;
        }
        let reg = named
            .iter()
            .find(|(id, _)| id == "r")
            .expect("node r is lowered at the top level")
            .1;
        probe(&ir, &frame, &dots(), pixel)[usize::from(reg.0)]
    }

    fn close(actual: [f64; 4], expected: &[f64], case: &str) {
        for (c, e) in expected.iter().enumerate() {
            assert!(
                (actual[c] - e).abs() < 1e-12,
                "{case}: component {c}: {actual:?} vs {expected:?}"
            );
        }
    }

    fn check(rows: &[&str]) {
        for row in rows {
            let (source, expected) = row.split_once(" => ").unwrap();
            let expected: Vec<f64> = expected.split(' ').map(|v| v.parse().unwrap()).collect();
            close(value_at(source, (2, 1), &[]), &expected, row);
        }
    }

    #[test]
    fn nodes_evaluate_by_the_safe_rules() {
        check(&[
            "r:PixelPos => 2.5 1.5",
            "r:PixelCell => 2 1",
            "r:TagUV => 0.25 0.375",
            "r:ScreenPos => 0.25 0.375",
            "r:Slot .slot=glow[1] => 17",
            "r:frac x=-0.25 => 0.75",
            "r:mul a=[2,3] b=[4,5] => 8 15",
            "r:div a=[6,1] b=[3,0] => 2 0",
            "r:mod a=[-1,5] b=[3,0] => 2 0",
            "r:step edge=1 x=[0.5,1,2] => 0 1 1",
            "r:pow x=[-2,0,2,-8] y=[2,2,3,0.3333] => 0 0 8 0",
            "r:atan2 y=[0,1] x=[0,0] => 0 1.5707963267948966",
            "r:clamp x=[-1,5,0.5] min=0 max=1 => 0 1 0.5",
            "r:smoothstep edge0=0 edge1=2 x=[1,3] => 0.5 1",
            "r:smoothstep edge0=1 edge1=1 x=2 => 0",
            "r:polar v=[0,2] => 2 1.5707963267948966",
            "r:polar v=[0,0] => 0 0",
            "r:Component v=[1,2,3] .component=z => 3",
            "r:Select condition=1 a=5 b=7 => 7",
            "r:Bayer => 0.875",
            "r:Stripes .period=2 .width=1 => 1",
            "r:Gradient from=[1,1] to=[1,1] => 0",
            "r:Hsv h=0 => 1 0 0 1",
            "r:Quantize level=0.5 .slot=fill => 2",
            "r:Cycle index=1 .slot=fill => 0",
            "r:Lighten color=[0.5,0.5,0.5,0.4] amount=0.5 => 0.75 0.75 0.75 0.4",
            "s:Slot .slot=glow; r:Alpha color=s alpha=1 => 1 0 0 0.5019607843137255",
            "p:PixelPos; r:SampleAt offset=[1,-1] .source=p => 3.5 0.5",
            "f:Frame; r:SampleAt frameOffset=2 .source=f => 5",
            "i:RepeatIndex; v:add a=i b=1; r:Repeat .source=v .count=3 .reduce=sum => 6",
            "i:RepeatIndex; v:add a=i b=1; r:Repeat .source=v .count=3 .reduce=max => 3",
            "i:RepeatIndex; v:add a=i b=-1; r:Repeat .source=v .count=3 .reduce=first => 1",
            "i:RepeatIndex; v:add a=i b=-5; r:Repeat .source=v .count=3 .reduce=first => 0",
            "p:PixelCell; i:RepeatIndex; x:Component v=p; v:add a=i b=x; s:Repeat .source=v .count=2 .reduce=sum; r:SampleAt offset=[3,0] .source=s => 11",
        ]);
        let h = |x, y, s| hash(x, y, s);
        for (source, expected) in [
            ("r:sin x=1", libm::sin(1.0)),
            ("r:cos x=1", libm::cos(1.0)),
            ("r:exp x=1", libm::exp(1.0)),
            ("r:Noise p=[0,0]", h(0, 0, 0)),
            ("r:Noise p=[3,0] .period=[3,0] .seed=4", h(0, 0, 4)),
            ("r:Noise p=[0.5,0] .period=[1,1]", h(0, 0, 0)),
            (
                "r:Fbm p=[0,0] .octaves=2",
                (h(0, 0, 0) + 0.5 * h(0, 0, 1)) / 1.5,
            ),
            ("r:Hash p=[3.7,-1.2] .seed=5", h(3, -2, 5)),
        ] {
            close(value_at(source, (2, 1), &[]), &[expected], source);
        }
        let ir = compile("r:Expr .expr='1+2'; out:Output color=[0,0,0,1] coverage=r");
        assert_eq!(ir.lanes, [Lane::Expr(0)]);
        close(
            value_at("r:Expr .expr='1+2'", (0, 0), &[(0, 3.0)]),
            &[3.0],
            "expr",
        );
        close(value_at("r:BodyEdge", (2, 1), &[]), &[-1.5], "edge");
        close(
            value_at("r:BodyEdge", (0, 0), &[(0, 2.0)]),
            &[1.5 * 2.0_f64.sqrt() - 2.0],
            "corner",
        );
        close(
            value_at("r:BodyEdge .edge=outer", (2, 1), &[(4, 1.0)]),
            &[-2.5],
            "outer",
        );
        let star = |frame: f64, pixel| {
            value_at(&format!("r:Sprite frame={frame} .sprite=star"), pixel, &[])[0]
        };
        let actual = &[
            star(0.0, (1, 0)),
            star(0.0, (0, 1)),
            star(0.0, (2, 1)),
            star(3.0, (0, 1)),
        ];
        assert_eq!(*actual, [16.0, -1.0, -1.0, 16.0]);
        let texel = |uv: &str| {
            value_at(
                &format!("r:BundleTexture uv={uv} .texture=dots"),
                (2, 1),
                &[],
            )
            .map(|c| (c * 255.0).round())
        };
        assert_eq!(texel("[0.75,0.25]"), [40.0, 50.0, 60.0, 128.0]);
        assert_eq!(texel("[-0.25,1.75]"), [100.0, 110.0, 120.0, 255.0]);
    }

    #[test]
    fn images_cover_the_area_and_never_show_nan() {
        let nan = "big:exp x=1000; nan:mul a=big b=0; c:Combine x=nan y=big z=0.5 w=1";
        assert!(pixels(&format!("{nan}; out:Output color=c coverage=nan"))
            .iter()
            .all(|p| *p == 0));
        assert!(pixels(&format!("{nan}; out:Output color=c"))
            .iter()
            .all(|p| *p == 0xff00_ff80));
        assert!(pixels(&format!(
            "{nan}; q:Quantize level=nan .slot=fill; out:Output color=q"
        ))
        .iter()
        .all(|p| *p == 0));

        let (ir, _) = compile_file(
            json!({ "margins": [1, 0, 1, 0], "nodes": nodes("c:Slot .slot=fill[1]; out:Output color=c") }),
        );
        let filled = evaluate(&ir, &frame(&ir, &palette()), &[]);
        assert_eq!(filled.len(), 12 * 4);
        assert!(filled.iter().all(|p| *p == FILL[1]));
        assert!(
            pixels("c:Slot .slot=fill[1]; out:Output color=c coverage=0")
                .iter()
                .all(|p| *p == 0)
        );
        assert!(pixels("out:Output color=[1,0,0,0.001]")
            .iter()
            .all(|p| *p == 0));
        assert!(pixels("out:Output color=[1,0.5,0,0.5]")
            .iter()
            .all(|p| *p == 0x80ff_8000));

        let dot = "p:PixelCell; d:sub a=p b=[2,1]; l:length v=d; inside:step edge=l x=0; out:Output color=[1,1,1,1] coverage=inside";
        let (ir, _) = compile_file(json!({ "outline": "glow[1]", "nodes": nodes(dot) }));
        let ring = evaluate(&ir, &frame(&ir, &palette()), &[]);
        let at = |x: usize, y: usize| ring[y * 10 + x];
        assert_eq!(at(2, 1), 0xffff_ffff);
        assert!([(1, 1), (3, 1), (2, 0), (2, 2)]
            .iter()
            .all(|(x, y)| at(*x, *y) == GLOW[1]));
        assert_eq!(ring.iter().filter(|p| **p != 0).count(), 5);
    }

    struct Host(Vec<(TexRef, Image)>);

    impl TextureSource for Host {
        fn texture(&self, texture: &TexRef) -> Option<&Image> {
            self.0.iter().find(|(t, _)| t == texture).map(|(_, i)| i)
        }
    }

    #[test]
    fn host_textures_feed_vanilla_and_atlas_nodes() {
        let dots = dots().remove(0);
        let host = Host(vec![
            (TexRef::Vanilla(VanillaTexture::Glint), dots.clone()),
            (TexRef::Atlas("minecraft:block/stone".into()), dots),
        ]);
        for node in [
            "r:VanillaTexture uv=uv .texture=glint",
            "r:AtlasSprite uv=uv .sprite=minecraft:block/stone",
        ] {
            let ir = compile(&format!("uv:TagUV; {node}; out:Output color=r"));
            let tones = palette();
            let frame = frame(&ir, &tones);
            assert!(
                evaluate(&ir, &frame, &[]).iter().all(|p| *p == 0),
                "{node} draws clear without the host"
            );
            let fed = evaluate_with(&ir, &frame, &[], &host);
            assert_eq!(
                fed[0],
                pack([10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0, 1.0]),
                "{node}"
            );
            let fallback =
                fallback_with(&ir, &frame, &[], &host, &mut FallbackBudget::new()).unwrap();
            assert_eq!(fallback.pixels, fed);
        }
    }

    #[test]
    fn loop_checks_compare_frame_zero_with_the_cycle() {
        let tones = palette();
        let looping = |modulo: u32| {
            let ir = compile(&format!("f:Frame; m:mod a=f b={modulo}; d:div a=m b={modulo}; h:Hsv h=d; out:Output color=h"));
            let frame = frame(&ir, &tones);
            assert_eq!(frame.clock[CLOCK_CYCLE], 12);
            (
                ir.period,
                loops(&ir, &frame, &[], &mut |_| [0.0; LANE_COUNT]),
            )
        };
        assert_eq!(looping(4), (Period::Frames(4), true));
        assert_eq!(looping(5).1, false);
        assert_eq!(looping(6).1, true);

        let drift = compile("t:SteppedTime; e:Expr .expr=q.frame; s:add a=t b=e; f:frac x=s; h:Hsv h=f; out:Output color=h");
        assert_eq!(drift.period, Period::Unknown);
        let mut seen = Vec::new();
        let mut lanes = |frame: i32| {
            seen.push(frame);
            let mut lanes = [0.0; LANE_COUNT];
            lanes[0] = f64::from(frame) * 0.1;
            lanes
        };
        assert!(!loops(&drift, &frame(&drift, &tones), &[], &mut lanes));
        assert_eq!(seen, [0, 12]);
        assert!(loops(&drift, &frame(&drift, &tones), &[], &mut |_| [0.0;
            LANE_COUNT]));
    }

    #[test]
    fn fallbacks_stop_at_the_bundle_cap() {
        assert_eq!(FallbackBudget::new().cap(), 2 * 1024 * 1024);
        let ir = compile("c:Slot .slot=fill; out:Output color=c");
        let tones = palette();
        let frame = frame(&ir, &tones);
        let each = fallback_bytes(frame.area);
        assert_eq!(each, 64 + 10 * 4 * 4);
        let mut budget = FallbackBudget::with_cap(each * 3 + 10);
        for _ in 0..3 {
            let image = fallback(&ir, &frame, &[], &mut budget).expect("fits");
            let actual = &(image.bytes, image.area, image.pixels.len());
            assert_eq!(*actual, (each, frame.area, 40));
        }
        assert_eq!(budget.used(), each * 3);
        assert!(fallback(&ir, &frame, &[], &mut budget).is_none());
        budget.release(each);
        assert!(fallback(&ir, &frame, &[], &mut budget).is_some());
        let mut big = frame;
        big.area = Rect::new(0, 0, 1024, 600);
        assert!(fallback_bytes(big.area) > NAMEPLATE_FALLBACK_BYTES);
        let mut full = FallbackBudget::new();
        assert!(fallback(&ir, &big, &[], &mut full).is_none() && full.used() == 0);
    }

    #[test]
    fn evaluation_is_deterministic_per_frame() {
        let ir = compile("p:PixelPos; f:Frame; x:Component v=p; s:add a=x b=f; c:Combine x=s y=0; n:Fbm p=c .period=[8,0] .octaves=4; \
            q:DitherQuantize level=n .slot=fill; out:Output color=q");
        let tones = palette();
        let mut a = frame(&ir, &tones);
        let first = evaluate(&ir, &a, &[]);
        assert_eq!(first, evaluate(&ir, &a, &[]));
        assert!(first.iter().all(|p| FILL.contains(p)));
        a.clock[CLOCK_FRAME] = 4;
        assert_ne!(first, evaluate(&ir, &a, &[]));
        a.clock[CLOCK_FRAME] = 3 + 8;
        assert_eq!(first, evaluate(&ir, &a, &[]));
    }
}
