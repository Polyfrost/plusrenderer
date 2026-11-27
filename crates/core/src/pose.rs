//! Bone hierarchy to world transforms, the only place animation touches rendering.

use crate::animation::Animation;
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

pub const POSE_CHANNELS: usize = 3;

pub fn held_channels(
    animations: &[Animation],
    groups: &[(&[Vec<Option<usize>>], usize)],
) -> Vec<u32> {
    let mut held: Vec<u32> = groups
        .iter()
        .flat_map(|&(binds, first)| {
            animations
                .iter()
                .zip(binds)
                .flat_map(move |(animation, bind)| {
                    animation
                        .keyed_channels(bind)
                        .map(move |channel| first * POSE_CHANNELS + channel)
                })
        })
        .filter_map(|channel| u32::try_from(channel).ok())
        .collect();
    held.sort_unstable();
    held.dedup();
    held
}

fn channel_of(bone: &BonePose, kind: usize) -> [f32; 3] {
    match kind {
        0 => bone.rotation,
        1 => bone.position,
        _ => bone.scale,
    }
}

fn channel_mut(bone: &mut BonePose, kind: usize) -> &mut [f32; 3] {
    match kind {
        0 => &mut bone.rotation,
        1 => &mut bone.position,
        _ => &mut bone.scale,
    }
}

pub fn hold_channels(held: &[u32], bones: &[BonePose], props: &[BonePose], out: &mut [[f32; 3]]) {
    for (&code, slot) in held.iter().zip(out) {
        let (bone, kind) = (code as usize / POSE_CHANNELS, code as usize % POSE_CHANNELS);
        let pose = match bone.checked_sub(bones.len()) {
            None => bones.get(bone),
            Some(prop) => props.get(prop),
        };
        if let Some(pose) = pose {
            *slot = channel_of(pose, kind);
        }
    }
}

pub fn restore_channels(
    held: &[u32],
    stored: &[[f32; 3]],
    bones: &mut [BonePose],
    props: &mut [BonePose],
) {
    bones.fill(BonePose::default());
    props.fill(BonePose::default());
    for (&code, value) in held.iter().zip(stored) {
        let (bone, kind) = (code as usize / POSE_CHANNELS, code as usize % POSE_CHANNELS);
        let pose = match bone.checked_sub(bones.len()) {
            None => bones.get_mut(bone),
            Some(prop) => props.get_mut(prop),
        };
        if let Some(pose) = pose {
            *channel_mut(pose, kind) = *value;
        }
    }
}

/// One per bone. `external` resolves parents outside this model, e.g. an attachable's `head`.
pub fn world_transforms<F>(model: &Model, pose: Option<&Pose>, external: F, out: &mut Vec<Affine>)
where
    F: FnMut(&str) -> Option<Affine>,
{
    world_transforms_carried(model, pose, external, &[], out);
}

pub fn world_transforms_carried<F>(
    model: &Model,
    pose: Option<&Pose>,
    mut external: F,
    carried: &[(usize, usize)],
    out: &mut Vec<Affine>,
) where
    F: FnMut(&str) -> Option<Affine>,
{
    world_transforms_resolved(
        model,
        pose,
        |bone| external(model.bones[bone].parent_name.as_deref()?),
        carried,
        out,
    );
}

pub fn world_transforms_resolved<F>(
    model: &Model,
    pose: Option<&Pose>,
    mut external: F,
    carried: &[(usize, usize)],
    out: &mut Vec<Affine>,
) where
    F: FnMut(usize) -> Option<Affine>,
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

        let carrier = carried
            .iter()
            .find(|&&(child, carrier)| child == i && carrier < i)
            .map(|&(_, carrier)| carrier);
        let parent = match carrier.or(bone.parent) {
            // Bones are topologically sorted, so the parent is already done.
            Some(idx) => out[idx],
            None => bone
                .parent_name
                .as_ref()
                .and_then(|_| external(i))
                .unwrap_or(Affine::IDENTITY),
        };

        out.push(parent.mul(&local));
    }
}

pub fn inverse(a: &Affine) -> Option<Affine> {
    let m = &a.0;
    let c00 = m[5] * m[10] - m[6] * m[9];
    let c01 = m[6] * m[8] - m[4] * m[10];
    let c02 = m[4] * m[9] - m[5] * m[8];
    let det = m[0] * c00 + m[1] * c01 + m[2] * c02;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    let linear = [
        c00 * d,
        (m[2] * m[9] - m[1] * m[10]) * d,
        (m[1] * m[6] - m[2] * m[5]) * d,
        c01 * d,
        (m[0] * m[10] - m[2] * m[8]) * d,
        (m[2] * m[4] - m[0] * m[6]) * d,
        c02 * d,
        (m[1] * m[8] - m[0] * m[9]) * d,
        (m[0] * m[5] - m[1] * m[4]) * d,
    ];
    let t = [m[3], m[7], m[11]];
    let mut out = [0.0f32; 12];
    for r in 0..3 {
        let row = &linear[r * 3..r * 3 + 3];
        out[r * 4..r * 4 + 3].copy_from_slice(row);
        out[r * 4 + 3] = -(row[0] * t[0] + row[1] * t[1] + row[2] * t[2]);
    }
    Some(Affine(out))
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
