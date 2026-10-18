//! Java Edition model space: game parts in, emote deltas out (as PolyPlus `ModelPoseApplicator`).

use crate::math::Affine;
use crate::pose::BonePose;

/// Core model space to a player render layer's: Y down, origin at the neck (24 up), front -Z, blocks.
pub const MODEL: Affine = Affine([
    1.0 / 16.0,
    0.0,
    0.0,
    0.0, //
    0.0,
    -1.0 / 16.0,
    0.0,
    1.5, //
    0.0,
    0.0,
    -1.0 / 16.0,
    0.0,
]);

/// A `ModelPart` pose: pivot in pixels on Java's axes, rotation in radians applied Z, Y, X.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Part {
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

/// Java pixel axes <-> core space. It is its own inverse.
const FLIP: Affine = Affine([
    1.0, 0.0, 0.0, 0.0, //
    0.0, -1.0, 0.0, 24.0, //
    0.0, 0.0, -1.0, 0.0,
]);

/// How a posed vanilla part, at rest at `rest`, moves the core bone of that name. Parent-relative.
pub fn part_transform(rest: [f32; 3], part: &Part) -> Affine {
    let [x, y, z] = part.rotation;
    let java = Affine::translation(part.offset)
        .mul(&Affine::rot_z(z))
        .mul(&Affine::rot_y(y))
        .mul(&Affine::rot_x(x))
        .mul(&Affine::scale(part.scale))
        .mul(&Affine::translation(rest.map(|r| -r)));

    FLIP.mul(&java).mul(&FLIP)
}

pub fn rest_pivot(rest: [f32; 3]) -> [f32; 3] {
    FLIP.point(rest)
}

/// The bone's pose as a delta to the vanilla part: offset and rotation add, scale multiplies.
pub fn part_delta(pose: &BonePose) -> Part {
    let [x, y, z] = pose.position;
    Part {
        offset: [x, -y, z],
        rotation: pose.rotation.map(f32::to_radians),
        scale: pose.scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{self, Arms};
    use crate::pose::{world_transforms, Pose};

    fn rest(bone: &str, arms: Arms) -> [f32; 3] {
        let arm_y = if arms == Arms::Slim { 2.5 } else { 2.0 };
        match bone {
            "rightArm" => [-5.0, arm_y, 0.0],
            "leftArm" => [5.0, arm_y, 0.0],
            "rightLeg" => [-1.9, 12.0, 0.0],
            "leftLeg" => [1.9, 12.0, 0.0],
            _ => [0.0; 3],
        }
    }

    fn close(a: &Affine, b: &Affine) -> bool {
        a.0.iter().zip(&b.0).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    /// Core emote -> game part delta -> back moves a cosmetic as the core alone would.
    #[test]
    fn deltas_and_parts_agree_with_the_core() {
        for arms in [Arms::Wide, Arms::Slim] {
            let model = player::model(arms);
            for bone in ["head", "body", "rightArm", "leftArm", "rightLeg", "leftLeg"] {
                let i = model.bone(bone).unwrap();
                let mut pose = Pose::rest(&model);
                pose.0[i] = BonePose {
                    rotation: [30.0, -20.0, 10.0],
                    position: [1.0, 2.0, 3.0],
                    scale: [1.0, 1.1, 0.9],
                };
                let mut core = Vec::new();
                world_transforms(&model, Some(&pose), |_| None, &mut core);

                let rest = rest(bone, arms);
                let delta = part_delta(&pose.0[i]);
                let game = Part {
                    offset: std::array::from_fn(|k| rest[k] + delta.offset[k]),
                    rotation: delta.rotation,
                    scale: delta.scale,
                };
                let moved = part_transform(rest, &game);
                assert!(
                    close(&moved, &core[i]),
                    "{bone} {arms:?}:\n{moved:?}\n{:?}",
                    core[i]
                );
            }
        }
    }

    #[test]
    fn a_part_at_rest_moves_nothing() {
        let rest = Part {
            offset: [-5.0, 2.0, 0.0],
            rotation: [0.0; 3],
            scale: [1.0; 3],
        };
        assert!(close(
            &part_transform([-5.0, 2.0, 0.0], &rest),
            &Affine::IDENTITY
        ));
    }

    #[test]
    fn model_space_puts_the_feet_at_one_and_a_half_blocks_down() {
        assert_eq!(MODEL.point([0.0, 0.0, 0.0]), [0.0, 1.5, 0.0]);
        assert_eq!(MODEL.point([16.0, 24.0, 16.0]), [1.0, 0.0, -1.0]);
    }
}
