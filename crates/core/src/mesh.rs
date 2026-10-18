//! Posed 3D quads, for hosts with their own camera, lighting and depth buffer (the game).

use crate::build::{cube_transform, face_uvs, Instance};
use crate::ir::Face;
use crate::math::{normalize, Affine};

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct MeshRun {
    pub texture: u32,
    /// In quads, four vertices per quad.
    pub start: u32,
    pub count: u32,
    /// 1 culls back faces (the player), 0 draws both sides (cosmetics).
    pub cull: u32,
}

/// Quads, four vertices each, in the host's space.
#[derive(Default)]
pub struct Mesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    /// Per-vertex minimum light level 0..=15 (0 if not glowing). Hosts pack `max(scene, glow)`.
    pub glow: Vec<u8>,
    pub runs: Vec<MeshRun>,
}

impl Mesh {
    /// `to_host` maps core model space (Y up, front +Z, model units) to the host's, e.g. `java::MODEL`.
    pub fn build(&mut self, instances: &[Instance], to_host: &Affine) {
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        self.glow.clear();
        self.runs.clear();

        for inst in instances {
            let start = (self.glow.len() / 4) as u32;

            for (bone, &bone_tf) in inst.model.bones.iter().zip(inst.transforms) {
                let glow = bone.glow.unwrap_or(0);
                for cube in &bone.cubes {
                    let tf = to_host.mul(&cube_transform(bone_tf, cube));
                    for face in Face::ALL {
                        let Some(uv) = cube.faces[face.index()] else {
                            continue;
                        };
                        let corners = cube.face_quad(face);
                        let uvs = face_uvs(inst, uv);
                        let normal = normalize(tf.vector(face.normal()));

                        // `face_quad` is clockwise from outside. GL wants CCW, in vanilla's corner order.
                        for i in [1, 0, 3, 2] {
                            self.positions.extend_from_slice(&tf.point(corners[i]));
                            self.normals.extend_from_slice(&normal);
                            self.uvs.extend_from_slice(&uvs[i]);
                            self.glow.push(glow);
                        }
                    }
                }
            }

            let count = (self.glow.len() / 4) as u32 - start;
            if count > 0 {
                self.runs.push(MeshRun {
                    texture: inst.texture,
                    start,
                    count,
                    cull: inst.cull as u32,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{Buffers, Camera, Light};
    use crate::math::dot;
    use crate::player::{self, Arms};
    use crate::pose::world_transforms;

    fn player_mesh() -> Mesh {
        let model = player::model(Arms::Wide);
        let mut transforms = Vec::new();
        world_transforms(&model, None, |_| None, &mut transforms);
        let mut mesh = Mesh::default();
        mesh.build(&[Instance::new(&model, &transforms, 0)], &Affine::IDENTITY);
        mesh
    }

    fn vertex(v: &[f32], i: usize) -> [f32; 3] {
        [v[i * 3], v[i * 3 + 1], v[i * 3 + 2]]
    }

    /// Every 2D vertex is a projected mesh vertex (geometry kept non-interpenetrating).
    #[test]
    fn projected_the_mesh_is_the_2d_build() {
        let model = crate::bedrock::parse_one(
            r#"{"format_version":"1.12.0","minecraft:geometry":[{
              "description":{"identifier":"g","texture_width":16,"texture_height":16},
              "bones":[{"name":"b","pivot":[0,4,0],"rotation":[20,-35,10],"cubes":[
                {"origin":[-6,0,-2],"size":[4,8,4],"uv":[0,0]},
                {"origin":[3,2,-1],"size":[3,3,3],"rotation":[0,0,30],"uv":[0,0]}]}]}]}"#,
        )
        .unwrap();

        let mut transforms = Vec::new();
        world_transforms(&model, None, |_| None, &mut transforms);
        let mut mesh = Mesh::default();
        mesh.build(&[Instance::new(&model, &transforms, 0)], &Affine::IDENTITY);

        let camera = Camera::orbit(30.0, 15.0, 400.0, [0.0, 4.0, 0.0]);
        let mut flat = Buffers::default();
        flat.build(
            &[Instance::new(&model, &transforms, 0)],
            &camera,
            &Light::default(),
            [512.0, 512.0],
        );

        let f = 1.0 / (22.5f32).to_radians().tan() * 256.0;
        let projected: Vec<[f32; 2]> = (0..mesh.glow.len())
            .map(|i| {
                let p = camera.view.point(vertex(&mesh.positions, i));
                [256.0 + p[0] * f / -p[2], 256.0 - p[1] * f / -p[2]]
            })
            .collect();

        for xy in flat.positions.chunks(2) {
            assert!(
                projected
                    .iter()
                    .any(|p| (p[0] - xy[0]).abs() < 1e-2 && (p[1] - xy[1]).abs() < 1e-2),
                "2D vertex {xy:?} has no 3D source"
            );
        }
    }

    #[test]
    fn quads_wind_counter_clockwise_from_outside() {
        let mesh = player_mesh();
        for q in 0..mesh.glow.len() / 4 {
            let [a, b, c] = [0, 1, 2].map(|k| vertex(&mesh.positions, q * 4 + k));
            let (ab, ac) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            let winding = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            assert!(dot(winding, vertex(&mesh.normals, q * 4)) > 0.0, "quad {q}");
        }

        assert_eq!(mesh.runs.len(), 1);
        assert_eq!(mesh.runs[0].count as usize * 4, mesh.glow.len());
    }
}
