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
// Port the lightmap if a host needs lit renders to match the game exactly.
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
    /// Spent id lists, reused so a frame's sort doesn't allocate.
    pool: Vec<Vec<u32>>,
    /// `split`'s back piece while the front is written.
    scratch: Vec<Vertex>,
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
    /// The source face's, kept by every piece cut from it. `None` if degenerate.
    plane: Option<([f32; 3], f32)>,
}

/// Far/near depth ratio above which a polygon is sliced for texture accuracy.
const MAX_DEPTH_RATIO: f32 = 1.08;

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

        let mut all = self.pool.pop().unwrap_or_default();
        all.extend(0..self.polys.len() as u32);
        let mut sorted = std::mem::take(&mut self.order);
        self.back_to_front(all, &mut sorted);

        // Sliced after sorting: pieces of one polygon are coplanar, so any order among them is exact.
        let mut order = self.pool.pop().unwrap_or_default();
        for &id in &sorted {
            self.slice(id, &mut order);
        }
        self.emit(&order, &proj);

        order.clear();
        self.pool.push(order);
        self.order = sorted;
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

                        let start = self.verts.len() as u32;
                        for (pos, uv) in view.into_iter().zip(uvs) {
                            self.verts.push(Vertex { pos, uv });
                        }
                        self.polys.push(Poly {
                            start,
                            len: 4,
                            shade,
                            texture: inst.texture,
                            plane: plane(&view),
                        });
                    }
                }
            }
        }
    }

    /// Painter's order by BSP, exact where geometry interpenetrates. The eye is the view origin.
    // ponytail: rebuilt every frame, splitter picked from a few sampled candidates.
    // Cache the tree per pose if scenes grow.
    fn back_to_front(&mut self, mut ids: Vec<u32>, out: &mut Vec<u32>) {
        if ids.len() <= 1 {
            out.extend_from_slice(&ids);
            ids.clear();
            self.pool.push(ids);
            return;
        }

        let best = self.best_splitter(&ids);
        ids.swap(0, best);
        let splitter = ids[0];
        let Some((normal, d)) = self.polys[splitter as usize].plane else {
            // Degenerate: covers nothing, can go anywhere.
            out.push(splitter);
            ids.remove(0);
            return self.back_to_front(ids, out);
        };

        let mut on = self.pool.pop().unwrap_or_default();
        let mut front = self.pool.pop().unwrap_or_default();
        let mut back = self.pool.pop().unwrap_or_default();
        on.push(splitter);

        for &id in &ids[1..] {
            match self.side(id, normal, d) {
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

        for mut spent in [ids, on] {
            spent.clear();
            self.pool.push(spent);
        }
    }

    /// Index into `ids` of the candidate plane that splits least and halves best.
    fn best_splitter(&self, ids: &[u32]) -> usize {
        const CANDIDATES: usize = 4;
        const SPLIT_COST: usize = 4;
        let step = (ids.len() / CANDIDATES).max(1);
        let mut best = (usize::MAX, 0);

        for i in (0..ids.len()).step_by(step).take(CANDIDATES) {
            let Some((normal, d)) = self.polys[ids[i] as usize].plane else {
                continue;
            };

            let (mut front, mut back, mut splits) = (0usize, 0usize, 0usize);
            for (seen, &id) in ids.iter().enumerate() {
                match self.side(id, normal, d) {
                    (true, false) => front += 1,
                    (false, true) => back += 1,
                    (true, true) => splits += 1,
                    _ => {}
                }

                // The rest can only even out the imbalance, so stop once this can't win.
                let left = ids.len() - seen - 1;
                if splits * SPLIT_COST + front.abs_diff(back).saturating_sub(left) >= best.0 {
                    break;
                }
            }

            let score = splits * SPLIT_COST + front.abs_diff(back);
            if score < best.0 {
                best = (score, i);
            }
        }

        best.1
    }

    /// Whether any vertex is ahead of the plane, and whether any is behind it.
    fn side(&self, id: u32, normal: [f32; 3], d: f32) -> (bool, bool) {
        let poly = self.polys[id as usize];
        let (mut ahead, mut behind) = (false, false);
        for v in &self.verts[poly.start as usize..(poly.start + poly.len) as usize] {
            let dist = dot(normal, v.pos) + d;
            ahead |= dist > PLANE_EPSILON;
            behind |= dist < -PLANE_EPSILON;
        }

        (ahead, behind)
    }

    /// Cut at constant depths until no piece's far/near ratio exceeds `MAX_DEPTH_RATIO`.
    /// Skia interpolates UVs affinely, exact only along lines of constant depth.
    fn slice(&mut self, id: u32, out: &mut Vec<u32>) {
        let poly = self.polys[id as usize];
        let (mut near, mut far) = (f32::MAX, 0.0f32);
        for v in &self.verts[poly.start as usize..(poly.start + poly.len) as usize] {
            near = near.min(-v.pos[2]);
            far = far.max(-v.pos[2]);
        }

        if far <= near * MAX_DEPTH_RATIO {
            out.push(id);
            return;
        }

        // Geometric middle, so both halves have the same ratio.
        let (a, b) = self.split(id, [0.0, 0.0, 1.0], (near * far).sqrt());
        self.slice(a, out);
        self.slice(b, out);
    }

    /// Front piece is written straight after the existing vertices, back piece after it.
    fn split(&mut self, id: u32, normal: [f32; 3], d: f32) -> (u32, u32) {
        let poly = self.polys[id as usize];
        let (start, len) = (poly.start as usize, poly.len as usize);
        let mut back = std::mem::take(&mut self.scratch);
        let front_start = self.verts.len();

        for i in 0..len {
            let (a, b) = (self.verts[start + i], self.verts[start + (i + 1) % len]);
            let (da, db) = (dot(normal, a.pos) + d, dot(normal, b.pos) + d);

            if da >= -PLANE_EPSILON {
                self.verts.push(a);
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
                self.verts.push(cut);
                back.push(cut);
            }
        }

        let back_start = self.verts.len();
        self.verts.extend_from_slice(&back);
        self.polys.push(Poly {
            start: front_start as u32,
            len: (back_start - front_start) as u32,
            ..poly
        });
        self.polys.push(Poly {
            start: back_start as u32,
            len: back.len() as u32,
            ..poly
        });

        back.clear();
        self.scratch = back;
        let n = self.polys.len() as u32;
        (n - 2, n - 1)
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

/// Most cubes are unrotated, so they skip the trig and the pivot round trip.
pub(crate) fn cube_transform(bone_tf: Affine, cube: &Cube) -> Affine {
    if cube.rotation == [0.0; 3] {
        return bone_tf;
    }

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

/// Unit normal and offset, by Newell's method.
fn plane(verts: &[[f32; 3]]) -> Option<([f32; 3], f32)> {
    let mut n = [0.0f32; 3];
    let mut centre = [0.0f32; 3];

    for (i, &a) in verts.iter().enumerate() {
        let b = verts[(i + 1) % verts.len()];
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
                locators: Vec::new(),
                spring: None,
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
}
