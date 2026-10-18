//! Bone hierarchy to world transforms, the only place animation touches rendering.

use crate::ir::Model;
use crate::math::{bedrock_rotation, Affine};

/// In file-space units and signs. `world_transforms` converts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePose {
    /// Degrees, added to the rest rotation.
    pub rotation: [f32; 3],
    /// Model units, offsetting the pivot.
    pub position: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for BonePose {
    fn default() -> Self {
        BonePose {
            rotation: [0.0; 3],
            position: [0.0; 3],
            scale: [1.0; 3],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pose(pub Vec<BonePose>);

impl Pose {
    pub fn rest(model: &Model) -> Pose {
        Pose(vec![BonePose::default(); model.bones.len()])
    }

    pub fn reset(&mut self) {
        self.0.fill(BonePose::default());
    }
}

/// One per bone. `external` resolves parents outside this model, e.g. an attachable's `head`.
pub fn world_transforms<F>(
    model: &Model,
    pose: Option<&Pose>,
    mut external: F,
    out: &mut Vec<Affine>,
) where
    F: FnMut(&str) -> Option<Affine>,
{
    out.clear();
    out.reserve(model.bones.len());

    for (i, bone) in model.bones.iter().enumerate() {
        let p = pose.and_then(|p| p.0.get(i)).copied().unwrap_or_default();

        let rotation = bedrock_rotation([
            bone.rotation[0] + p.rotation[0],
            bone.rotation[1] + p.rotation[1],
            bone.rotation[2] + p.rotation[2],
        ]);

        // File-space offsets (see `bedrock::to_ir`).
        let local = Affine::translation(crate::bedrock::to_ir(p.position)).mul(&Affine::about(
            bone.pivot,
            &rotation.mul(&Affine::scale(p.scale)),
        ));

        let parent = match bone.parent {
            // Bones are topologically sorted, so the parent is already done.
            Some(idx) => out[idx],
            None => bone
                .parent_name
                .as_deref()
                .and_then(&mut external)
                .unwrap_or(Affine::IDENTITY),
        };

        out.push(parent.mul(&local));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{self, Arms};

    fn transforms(model: &Model, pose: Option<&Pose>) -> Vec<Affine> {
        let mut out = Vec::new();
        world_transforms(model, pose, |_| None, &mut out);
        out
    }

    #[test]
    fn rotation_leaves_the_pivot_where_it_was() {
        let model = player::model(Arms::Wide);
        let head = model.bone("head").unwrap();
        let mut pose = Pose::rest(&model);
        pose.0[head].rotation = [30.0, 45.0, 10.0];

        let pivot = model.bones[head].pivot;
        let moved = transforms(&model, Some(&pose))[head].point(pivot);
        for i in 0..3 {
            assert!((moved[i] - pivot[i]).abs() < 1e-4, "{moved:?} vs {pivot:?}");
        }
    }

    #[test]
    fn child_bones_inherit_their_parent() {
        let model = player::model(Arms::Wide);
        let (head, hat) = (model.bone("head").unwrap(), model.bone("hat").unwrap());
        let mut pose = Pose::rest(&model);
        pose.0[head].position = [0.0, 5.0, 0.0];

        let out = transforms(&model, Some(&pose));
        let hat_origin = out[hat].point([0.0; 3]);
        assert!((hat_origin[1] - 5.0).abs() < 1e-4, "{hat_origin:?}");
    }

    fn hand(pose_rotation: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let model = player::model(Arms::Wide);
        let arm = model.bone("rightArm").unwrap();
        let mut pose = Pose::rest(&model);
        pose.0[arm].rotation = pose_rotation;
        let pivot = model.bones[arm].pivot;
        let hand =
            transforms(&model, Some(&pose))[arm].point([pivot[0], pivot[1] - 10.0, pivot[2]]);
        (hand, pivot)
    }

    /// Pinned to Java's zombie pose (`xRot = -90deg`, arms ahead). Emotes use the same signs.
    #[test]
    fn negative_x_raises_an_arm_forward() {
        let ([_, y, z], pivot) = hand([-90.0, 0.0, 0.0]);
        assert!(
            (y - pivot[1]).abs() < 1e-3 && z > 9.9,
            "forward is +Z: y={y} z={z}"
        );
    }

    /// As Java's idle sway (positive `zRot`). In IR the right is -X.
    #[test]
    fn positive_z_swings_the_right_arm_outward() {
        let ([x, ..], pivot) = hand([0.0, 0.0, 30.0]);
        assert!(x < pivot[0], "outward is further -X: {x} vs {}", pivot[0]);
    }

    #[test]
    fn animated_positions_follow_the_file_axes() {
        let model = player::model(Arms::Wide);
        let head = model.bone("head").unwrap();
        let mut pose = Pose::rest(&model);
        pose.0[head].position = [0.0, 0.0, 3.0];
        let moved = transforms(&model, Some(&pose))[head].point([0.0; 3]);
        assert_eq!(moved, [0.0, 0.0, -3.0], "file +Z is backwards");
    }
}
