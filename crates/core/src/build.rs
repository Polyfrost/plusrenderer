//! Posed geometry to final 2D vertex buffers: screen-space, lit, back-to-front.
//! No depth buffer: the outer skin layer is translucent, so draw order is exact.

use crate::ir::{corner_uv, Cube, Face, FaceUv, Model};
use crate::math::{bedrock_rotation, dot, normalize, Affine};

pub struct Camera {
    /// Model space to view space. View space looks down -Z.
    pub view: Affine,
}

const FOV_Y_DEG: f32 = 45.0;
const NEAR: f32 = 0.1;

impl Camera {
    /// Yaw 0 faces the model's front, putting its right arm on screen left.
    pub fn orbit(yaw_deg: f32, pitch_deg: f32, distance: f32, target: [f32; 3]) -> Camera {
        let view = Affine::translation([0.0, 0.0, -distance])
            .mul(&Affine::rot_x(pitch_deg.to_radians()))
            .mul(&Affine::rot_y(yaw_deg.to_radians()))
            .mul(&Affine::translation([-target[0], -target[1], -target[2]]));
        Camera { view }
    }
}

/// In view space, so the light follows the camera.
const LIGHT_DIRECTION: [f32; 3] = [-0.35, 0.8, 1.0];
const AMBIENT: f32 = 0.62;
const DIFFUSE: f32 = 0.38;

#[derive(Clone, Copy, Debug)]
pub struct Light {
    /// Minecraft light level 0..=15, where 15 is daylight.
    pub level: f32,
}

impl Default for Light {
    fn default() -> Self {
        Light { level: 15.0 }
    }
}

// ponytail: vanilla light curve without the lightmap's gamma or dimension ambient.
fn brightness(level: f32) -> f32 {
    let f = (level / 15.0).clamp(0.0, 1.0);
    f / (4.0 - 3.0 * f)
}

pub struct Instance<'a> {
    pub model: &'a Model,
    /// One per bone, from `pose::world_transforms`.
    pub transforms: &'a [Affine],
    pub texture: u32,
    /// Off for cosmetics, which the in-game client draws two-sided.
    pub cull: bool,
    /// The texture is a vertical sheet of `frames` frames, and `frame` picks one.
    pub frames: u32,
    pub frame: u32,
}

impl<'a> Instance<'a> {
    pub fn new(model: &'a Model, transforms: &'a [Affine], texture: u32) -> Instance<'a> {
        Instance {
            model,
            transforms,
            texture,
            cull: true,
            frames: 1,
            frame: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DrawRun {
    pub texture: u32,
    pub start: u32,
    pub count: u32,
}

#[derive(Default)]
pub struct Buffers {
    /// Screen-space x, y pairs.
    pub positions: Vec<f32>,
    /// 0..1 across the bound image.
    pub uvs: Vec<f32>,
    /// 0xAARRGGBB, lighting baked in.
    pub colors: Vec<u32>,
    pub indices: Vec<u16>,
    pub runs: Vec<DrawRun>,
    verts: Vec<Vertex>,
    polys: Vec<Poly>,
    order: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

#[derive(Clone, Copy)]
struct Poly {
    start: u32,
    len: u32,
    shade: f32,
    texture: u32,
}

/// Far/near corner depth ratios above which a quad is subdivided.
const SUBDIVIDE_2X: f32 = 1.08;
const SUBDIVIDE_4X: f32 = 1.35;

const PLANE_EPSILON: f32 = 1e-3;

/// u16 indices for Skia.
// ponytail: stops emitting at the cap. Split index buffers if a model ever hits it.
const MAX_VERTICES: usize = u16::MAX as usize + 1;

impl Buffers {
    pub fn build(
        &mut self,
        instances: &[Instance],
        camera: &Camera,
        light: &Light,
        viewport: [f32; 2],
    ) {
        self.positions.clear();
        self.uvs.clear();
        self.colors.clear();
        self.indices.clear();
        self.runs.clear();
        self.verts.clear();
        self.polys.clear();
        self.order.clear();

        let proj = Proj::new(viewport);
        self.collect(instances, camera, light, &proj);

        let all: Vec<u32> = (0..self.polys.len() as u32).collect();
        let mut order = std::mem::take(&mut self.order);
        self.back_to_front(all, &mut order);
        self.emit(&order, &proj);
        self.order = order;
    }

    fn collect(&mut self, instances: &[Instance], camera: &Camera, light: &Light, proj: &Proj) {
        let light_direction = normalize(LIGHT_DIRECTION);

        for inst in instances {
            for (bone, &bone_tf) in inst.model.bones.iter().zip(inst.transforms) {
                let level_brightness =
                    brightness(light.level.max(bone.glow.map_or(0.0, f32::from)));

                for cube in &bone.cubes {
                    let to_view = camera.view.mul(&cube_transform(bone_tf, cube));

                    for face in Face::ALL {
                        let Some(uv) = cube.faces[face.index()] else {
                            continue;
                        };

                        let corners = cube.face_quad(face);
                        let mut view = [[0.0f32; 3]; 4];
                        let mut visible = true;
                        for (v, c) in view.iter_mut().zip(corners) {
                            *v = to_view.point(c);
                            visible &= v[2] < -NEAR;
                        }

                        if !visible || (inst.cull && !front_facing(&view, proj)) {
                            continue;
                        }

                        let normal = normalize(to_view.vector(face.normal()));
                        // Back faces keep their own normal, shading unlit as in game.
                        let shade = (AMBIENT + DIFFUSE * dot(normal, light_direction).max(0.0))
                            * level_brightness;
                        let uvs = face_uvs(inst, uv);

                        let n = subdivisions(&view);
                        let step = 1.0 / n as f32;

                        for row in 0..n {
                            for col in 0..n {
                                let (s0, t0) = (col as f32 * step, row as f32 * step);
                                let (s1, t1) = (s0 + step, t0 + step);
                                let start = self.verts.len() as u32;
                                for (s, t) in [(s0, t0), (s1, t0), (s1, t1), (s0, t1)] {
                                    self.verts.push(Vertex {
                                        pos: bilinear(&view, s, t),
                                        uv: bilinear(&uvs, s, t),
                                    });
                                }
                                self.polys.push(Poly {
                                    start,
                                    len: 4,
                                    shade,
                                    texture: inst.texture,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// Painter's order by BSP, exact where geometry interpenetrates. The eye is the view origin.
    // ponytail: first polygon as splitter, rebuilt every frame.
    // Pick by fewest splits if scenes grow.
    fn back_to_front(&mut self, ids: Vec<u32>, out: &mut Vec<u32>) {
        let Some((&splitter, rest)) = ids.split_first() else {
            return;
        };
        if rest.is_empty() {
            out.push(splitter);
            return;
        }

        let Some((normal, d)) = self.plane(splitter) else {
            // Degenerate: covers nothing, can go anywhere.
            out.push(splitter);
            return self.back_to_front(rest.to_vec(), out);
        };

        let mut on = vec![splitter];
        let (mut front, mut back) = (Vec::new(), Vec::new());
        for &id in rest {
            let poly = self.polys[id as usize];
            let (mut ahead, mut behind) = (false, false);
            for v in &self.verts[poly.start as usize..(poly.start + poly.len) as usize] {
                let dist = dot(normal, v.pos) + d;
                ahead |= dist > PLANE_EPSILON;
                behind |= dist < -PLANE_EPSILON;
            }

            match (ahead, behind) {
                (false, false) => on.push(id),
                (true, false) => front.push(id),
                (false, true) => back.push(id),
                (true, true) => {
                    let (f, b) = self.split(id, normal, d);
                    front.push(f);
                    back.push(b);
                }
            }
        }

        // Coplanar: later instances (cosmetics over the player) draw last.
        on.sort_by_key(|&id| self.polys[id as usize].texture);

        // The eye at the origin is in front of the plane when d > 0.
        let (far, near) = if d > 0.0 {
            (back, front)
        } else {
            (front, back)
        };

        self.back_to_front(far, out);
        out.extend_from_slice(&on);
        self.back_to_front(near, out);
    }

    /// Unit normal and offset, by Newell's method (stable on split slivers).
    fn plane(&self, id: u32) -> Option<([f32; 3], f32)> {
        let poly = self.polys[id as usize];
        let verts = &self.verts[poly.start as usize..(poly.start + poly.len) as usize];
        let mut n = [0.0f32; 3];
        let mut centre = [0.0f32; 3];

        for (i, a) in verts.iter().enumerate() {
            let b = verts[(i + 1) % verts.len()];
            let (a, b) = (a.pos, b.pos);
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
            for k in 0..3 {
                centre[k] += a[k] / verts.len() as f32;
            }
        }

        let len = dot(n, n).sqrt();
        if len < 1e-9 {
            return None;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        Some((n, -dot(n, centre)))
    }

    fn split(&mut self, id: u32, normal: [f32; 3], d: f32) -> (u32, u32) {
        let poly = self.polys[id as usize];
        let src: Vec<Vertex> =
            self.verts[poly.start as usize..(poly.start + poly.len) as usize].to_vec();
        let (mut front, mut back) = (Vec::with_capacity(6), Vec::with_capacity(6));

        for (i, &a) in src.iter().enumerate() {
            let b = src[(i + 1) % src.len()];
            let (da, db) = (dot(normal, a.pos) + d, dot(normal, b.pos) + d);

            if da >= -PLANE_EPSILON {
                front.push(a);
            }
            if da <= PLANE_EPSILON {
                back.push(a);
            }
            if (da > PLANE_EPSILON && db < -PLANE_EPSILON)
                || (da < -PLANE_EPSILON && db > PLANE_EPSILON)
            {
                let t = da / (da - db);
                let cut = Vertex {
                    pos: std::array::from_fn(|k| a.pos[k] + (b.pos[k] - a.pos[k]) * t),
                    uv: std::array::from_fn(|k| a.uv[k] + (b.uv[k] - a.uv[k]) * t),
                };
                front.push(cut);
                back.push(cut);
            }
        }

        let mut push = |verts: Vec<Vertex>| {
            let start = self.verts.len() as u32;
            self.verts.extend(verts.iter());
            self.polys.push(Poly {
                start,
                len: (self.verts.len() as u32) - start,
                ..poly
            });
            self.polys.len() as u32 - 1
        };

        (push(front), push(back))
    }

    fn emit(&mut self, order: &[u32], proj: &Proj) {
        for &id in order {
            let poly = self.polys[id as usize];
            let len = poly.len as usize;
            if len < 3 {
                continue;
            }
            if self.positions.len() / 2 + len > MAX_VERTICES {
                break;
            }

            let base = (self.positions.len() / 2) as u16;
            let color = pack_shade(poly.shade);
            for v in &self.verts[poly.start as usize..poly.start as usize + len] {
                let [x, y] = proj.point(v.pos);
                self.positions.extend_from_slice(&[x, y]);
                self.uvs.extend_from_slice(&v.uv);
                self.colors.push(color);
            }

            for k in 1..len as u16 - 1 {
                self.indices
                    .extend_from_slice(&[base, base + k, base + k + 1]);
            }

            let count = ((len - 2) * 3) as u32;
            match self.runs.last_mut() {
                Some(run) if run.texture == poly.texture => run.count += count,
                _ => self.runs.push(DrawRun {
                    texture: poly.texture,
                    start: self.indices.len() as u32 - count,
                    count,
                }),
            }
        }
    }
}

pub(crate) fn cube_transform(bone_tf: Affine, cube: &Cube) -> Affine {
    bone_tf.mul(&Affine::about(cube.pivot, &bedrock_rotation(cube.rotation)))
}

pub(crate) fn face_uvs(inst: &Instance, uv: FaceUv) -> [[f32; 2]; 4] {
    let [w, h] = inst.model.texture_size;
    let frames = inst.frames.max(1) as f32;
    std::array::from_fn(|i| {
        let [u, v] = corner_uv(uv, i);
        [u / w, (v / h + inst.frame as f32) / frames]
    })
}

/// Perspective divide into screen pixels, y down.
#[derive(Clone, Copy)]
struct Proj {
    scale: f32,
    cx: f32,
    cy: f32,
}

impl Proj {
    fn new(viewport: [f32; 2]) -> Proj {
        let f = 1.0 / (FOV_Y_DEG.to_radians() * 0.5).tan();
        Proj {
            scale: f * viewport[1] * 0.5,
            cx: viewport[0] * 0.5,
            cy: viewport[1] * 0.5,
        }
    }

    fn point(&self, p: [f32; 3]) -> [f32; 2] {
        let k = self.scale / -p[2];
        [self.cx + p[0] * k, self.cy - p[1] * k]
    }
}

/// Texture-order corners wind clockwise from outside: positive area in y-down screen space.
fn front_facing(view: &[[f32; 3]; 4], proj: &Proj) -> bool {
    let p = [
        proj.point(view[0]),
        proj.point(view[1]),
        proj.point(view[2]),
        proj.point(view[3]),
    ];

    let mut area = 0.0;
    for i in 0..4 {
        let (a, b) = (p[i], p[(i + 1) % 4]);
        area += a[0] * b[1] - b[0] * a[1];
    }

    area > 0.0
}

/// Skia interpolates UVs affinely, so subdividing bounds the depth range per patch.
fn subdivisions(view: &[[f32; 3]; 4]) -> usize {
    let mut near = f32::MAX;
    let mut far: f32 = 0.0;
    for v in view {
        let w = -v[2];
        near = near.min(w);
        far = far.max(w);
    }

    let ratio = far / near;
    if ratio > SUBDIVIDE_4X {
        4
    } else if ratio > SUBDIVIDE_2X {
        2
    } else {
        1
    }
}

/// Corners TL, TR, BR, BL. Exact since box faces stay parallelograms.
fn bilinear<const N: usize>(q: &[[f32; N]; 4], s: f32, t: f32) -> [f32; N] {
    std::array::from_fn(|i| {
        let top = q[0][i] + (q[1][i] - q[0][i]) * s;
        let bot = q[3][i] + (q[2][i] - q[3][i]) * s;
        top + (bot - top) * t
    })
}

fn pack_shade(shade: f32) -> u32 {
    let c = (shade.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    0xff00_0000 | (c << 16) | (c << 8) | c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Bone;
    use crate::pose::world_transforms;

    /// Face `i` gets u = `tag + i`, so with a 1x1 texture a vertex's u names its face.
    fn cube(origin: [f32; 3], size: [f32; 3], tag: f32) -> Cube {
        Cube {
            origin,
            size,
            inflate: 0.0,
            rotation: [0.0; 3],
            pivot: [0.0; 3],
            faces: std::array::from_fn(|i| {
                Some(FaceUv {
                    uv: [tag + i as f32, 0.0],
                    size: [1.0, 1.0],
                })
            }),
            box_uv: false,
            mirror: false,
        }
    }

    fn model(cubes: Vec<Cube>) -> Model {
        Model {
            identifier: "test".into(),
            texture_size: [1.0, 1.0],
            bones: vec![Bone {
                name: "root".into(),
                parent: None,
                parent_name: None,
                pivot: [0.0; 3],
                rotation: [0.0; 3],
                cubes,
                glow: None,
            }],
        }
    }

    fn render(m: &Model, camera: &Camera, cull: bool, light: &Light) -> Buffers {
        let mut transforms = Vec::new();
        world_transforms(m, None, |_| None, &mut transforms);
        let mut inst = Instance::new(m, &transforms, 0);
        inst.cull = cull;
        let mut b = Buffers::default();
        b.build(&[inst], camera, light, [512.0, 512.0]);
        b
    }

    fn orbit(yaw: f32, pitch: f32, distance: f32) -> Camera {
        Camera::orbit(yaw, pitch, distance, [0.0; 3])
    }

    /// Face tags in draw order. Only valid when nothing subdivided or split.
    fn face_tags(b: &Buffers) -> Vec<f32> {
        b.uvs.chunks(2).step_by(4).map(|uv| uv[0]).collect()
    }

    /// Cube tag (`u / 100`) of the topmost triangle at `p`'s screen point.
    fn visible_at(b: &Buffers, camera: &Camera, p: [f32; 3]) -> Option<u32> {
        let [x, y] = Proj::new([512.0, 512.0]).point(camera.view.point(p));
        let pos = |i: u16| [b.positions[i as usize * 2], b.positions[i as usize * 2 + 1]];
        let side =
            |a: [f32; 2], c: [f32; 2]| (c[0] - a[0]) * (y - a[1]) - (c[1] - a[1]) * (x - a[0]);

        b.indices
            .chunks(3)
            .rev()
            .find(|t| {
                let [a, c, e] = [pos(t[0]), pos(t[1]), pos(t[2])];
                let (s0, s1, s2) = (side(a, c), side(c, e), side(e, a));
                (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0)
            })
            .map(|t| (b.uvs[t[0] as usize * 2] / 100.0) as u32)
    }

    #[test]
    fn a_closed_cube_shows_exactly_three_faces() {
        let m = model(vec![cube([-8.0; 3], [16.0; 3], 0.0)]);
        let tags = face_tags(&render(
            &m,
            &orbit(30.0, 20.0, 400.0),
            true,
            &Light::default(),
        ));
        assert_eq!(tags.len(), 3, "got {tags:?}");
        for face in [Face::South, Face::Up, Face::West] {
            assert!(
                tags.contains(&(face.index() as f32)),
                "{face:?} in {tags:?}"
            );
        }
    }

    #[test]
    fn unculled_instances_show_faces_from_behind() {
        let m = model(vec![cube([-8.0; 3], [16.0; 3], 0.0)]);
        let culled = face_tags(&render(
            &m,
            &orbit(0.0, 0.0, 400.0),
            true,
            &Light::default(),
        ));
        let both = face_tags(&render(
            &m,
            &orbit(0.0, 0.0, 400.0),
            false,
            &Light::default(),
        ));
        assert_eq!(culled, vec![Face::South.index() as f32]);
        assert!(both.contains(&(Face::North.index() as f32)), "{both:?}");
    }

    #[test]
    fn an_inflated_layer_draws_over_what_it_wraps() {
        let inner = cube([-8.0; 3], [16.0; 3], 0.0);
        let mut outer = cube([-8.0; 3], [16.0; 3], 100.0);
        outer.inflate = 0.25;
        let camera = orbit(0.0, 0.0, 400.0);
        let b = render(&model(vec![outer, inner]), &camera, true, &Light::default());
        assert_eq!(visible_at(&b, &camera, [0.0, 0.0, 8.0]), Some(1));
    }

    /// Seen from above, face-centre sorting would paint the leg over the boot.
    #[test]
    fn a_close_fitting_cosmetic_stays_in_front_from_above() {
        let leg = cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], 0.0);
        let boot = cube([-2.3, 0.0, -2.3], [4.6, 3.0, 4.6], 100.0);
        let camera = Camera::orbit(0.0, 40.0, 40.0, [0.0, 6.0, 0.0]);
        let b = render(&model(vec![leg, boot]), &camera, true, &Light::default());
        assert_eq!(
            visible_at(&b, &camera, [0.0, 1.5, 2.3]),
            Some(1),
            "the boot's front"
        );
    }

    #[test]
    fn interpenetrating_faces_are_cut_and_interleaved() {
        let a = cube([-6.0, -6.0, -0.5], [12.0, 12.0, 1.0], 0.0);
        let mut b = cube([-6.0, -6.0, -0.5], [12.0, 12.0, 1.0], 100.0);
        b.rotation = [0.0, 50.0, 0.0];
        // Far enough that nothing subdivides.
        let camera = orbit(0.0, 0.0, 400.0);
        let buffers = render(&model(vec![a, b]), &camera, true, &Light::default());
        assert_eq!(
            visible_at(&buffers, &camera, [-2.0, 0.0, 0.5]),
            Some(0),
            "a wins left"
        );
        assert_eq!(
            visible_at(&buffers, &camera, [2.0, 0.0, 0.5]),
            Some(1),
            "b wins right"
        );
    }

    #[test]
    fn oblique_faces_subdivide_when_close() {
        let m = model(vec![cube([-8.0; 3], [16.0; 3], 0.0)]);
        let light = Light::default();
        let far = render(&m, &orbit(45.0, 0.0, 400.0), true, &light)
            .indices
            .len();
        let close = render(&m, &orbit(45.0, 0.0, 22.0), true, &light)
            .indices
            .len();
        assert!(close > far, "{close} vs {far}");
    }

    #[test]
    fn glowing_bones_stay_lit_in_the_dark() {
        let mut m = model(vec![cube([-8.0; 3], [16.0; 3], 0.0)]);
        let camera = orbit(0.0, 0.0, 400.0);
        let shade = |m: &Model, light: Light| render(m, &camera, true, &light).colors[0] & 0xff;
        let dark = Light { level: 2.0 };
        let unlit = shade(&m, dark);
        m.bones[0].glow = Some(15);
        assert!(unlit < 40, "level 2 is near black: {unlit}");
        assert_eq!(
            shade(&m, dark),
            shade(&m, Light::default()),
            "glow 15 looks like daylight"
        );
    }

    #[test]
    fn runs_cover_every_index_once() {
        let m = model(vec![cube([-8.0; 3], [16.0; 3], 0.0)]);
        let b = render(&m, &orbit(30.0, 20.0, 400.0), true, &Light::default());
        let covered: u32 = b.runs.iter().map(|r| r.count).sum();
        assert_eq!(covered as usize, b.indices.len());
        assert_eq!(b.runs[0].start, 0);
        assert_eq!(b.colors.len(), b.positions.len() / 2);
        assert_eq!(b.uvs.len(), b.positions.len());
    }
}
