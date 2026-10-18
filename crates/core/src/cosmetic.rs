//! Prepares a parsed geometry as a cosmetic, as PolyPlus `AttachedCosmeticParser` does:
//! `attach` reparents attachment roots onto the slot's player bone. `prepare` adds texture and glow.

use crate::ir::{Bone, Cube, Model};

/// The client's cosmetic steps in order. `anchor` overrides the slot's bone. `image_size` is the PNG's pixels.
/// Returns the frame count: above 1, a vertical sheet of `texture_size` frames cycled every 4 ticks.
pub fn prepare(
    model: &mut Model,
    slot: BodySlot,
    anchor: Option<&str>,
    image_size: Option<[u32; 2]>,
) -> u32 {
    attach(model, slot, anchor.and_then(player_bone));
    auto_mirror(model);

    let frames = image_size.map_or(1, |size| frame_count(model, size));
    // The client trusts the PNG's size over the declared one, except for sheets.
    if let (Some([w, h]), 1) = (image_size, frames) {
        model.texture_size = [w as f32, h as f32];
    }

    for i in 0..model.bones.len() {
        let inherited = model.bones[i].parent.and_then(|p| model.bones[p].glow);
        let bone = &mut model.bones[i];
        bone.glow = glow_level(&bone.name).or(inherited);
    }

    frames
}

/// As the client does: unless the file mirrors anything, mirror the +X bone of each left/right pair.
fn auto_mirror(model: &mut Model) {
    if model.bones.iter().flat_map(|b| &b.cubes).any(|c| c.mirror) {
        return;
    }

    let pivots: Vec<[f32; 3]> = model.bones.iter().map(|b| b.pivot).collect();
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;

    for bone in &mut model.bones {
        let [x, y, z] = bone.pivot;
        let paired = x > 0.0
            && pivots
                .iter()
                .any(|p| p[0] < 0.0 && near(-p[0], x) && near(p[1], y) && near(p[2], z));
        if paired {
            for cube in bone.cubes.iter_mut().filter(|c| c.box_uv) {
                cube.toggle_mirror();
            }
        }
    }
}

/// `glow` anywhere in a bone name means full bright and `glowN` means level N.
fn glow_level(name: &str) -> Option<u8> {
    let lower = name.to_ascii_lowercase();
    let rest = &lower[lower.find("glow")? + 4..];
    let digits: String = rest
        .chars()
        .take(2)
        .take_while(char::is_ascii_digit)
        .collect();
    Some(digits.parse::<u8>().map_or(15, |n| n.min(15)))
}

/// A sheet of declared-size frames (three or more) only if UVs stay inside the first frame.
fn frame_count(model: &Model, image: [u32; 2]) -> u32 {
    let [w, h] = model.texture_size;
    if max_uv_v(model) > h {
        return 1;
    }
    crate::bundle::vertical_frames([w as u32, h as u32], image, 3)
}

fn max_uv_v(model: &Model) -> f32 {
    model
        .bones
        .iter()
        .flat_map(|b| &b.cubes)
        .flat_map(|c| c.faces.iter().flatten())
        .map(|f| f.uv[1].max(f.uv[1] + f.size[1]))
        .fold(0.0, f32::max)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodySlot {
    Cape,
    Backpack,
    Glasses,
    Wings,
    LeftHand,
    RightHand,
    Hat,
    Aura,
    Boots,
    Shoulder,
    Pet,
}

impl BodySlot {
    pub fn from_name(name: &str) -> Option<BodySlot> {
        Some(match name {
            "cape" => BodySlot::Cape,
            "backpack" => BodySlot::Backpack,
            "glasses" => BodySlot::Glasses,
            "wings" => BodySlot::Wings,
            "left_hand" => BodySlot::LeftHand,
            "right_hand" => BodySlot::RightHand,
            "hat" => BodySlot::Hat,
            "aura" => BodySlot::Aura,
            "boots" => BodySlot::Boots,
            "shoulder" => BodySlot::Shoulder,
            "pet" => BodySlot::Pet,
            _ => return None,
        })
    }

    fn default_bone(self) -> Option<&'static str> {
        match self {
            BodySlot::Glasses | BodySlot::Hat => Some("head"),
            BodySlot::LeftHand => Some("leftArm"),
            BodySlot::RightHand => Some("rightArm"),
            BodySlot::Backpack
            | BodySlot::Wings
            | BodySlot::Aura
            | BodySlot::Boots
            | BodySlot::Shoulder
            | BodySlot::Pet => Some("body"),
            BodySlot::Cape => None,
        }
    }
}

/// Matched as the client does: case-insensitive, underscores ignored. Its `helmet` is our `hat`.
fn player_bone(name: &str) -> Option<&'static str> {
    let key: String = name
        .chars()
        .filter(|&c| c != '_')
        .collect::<String>()
        .to_lowercase();
    Some(match key.as_str() {
        "root" => "root",
        "head" => "head",
        "helmet" => "hat",
        "body" => "body",
        "rightarm" => "rightArm",
        "leftarm" => "leftArm",
        "rightleg" => "rightLeg",
        "leftleg" => "leftLeg",
        _ => return None,
    })
}

fn attach(model: &mut Model, slot: BodySlot, anchor: Option<&'static str>) {
    // A file that already names a player bone is trusted, so only the spelling is normalised.
    let mut authored = false;
    for bone in &mut model.bones {
        if let Some(canonical) = bone.parent_name.as_deref().and_then(player_bone) {
            bone.parent_name = Some(canonical.to_string());
            authored = true;
        }
    }

    if authored {
        return;
    }

    if slot == BodySlot::Boots {
        split_straddling_leaves(model);
    }

    let has_cubes = subtree_has_cubes(model);
    for (i, &renderable) in has_cubes.iter().enumerate() {
        if !renderable || !is_attachment_root(model, i) {
            continue;
        }
        let Some(target) = anchor.or_else(|| target_for(model, i, slot)) else {
            continue;
        };

        // Detached from any in-file parent, as the client does.
        let bone = &mut model.bones[i];
        bone.parent = None;
        bone.parent_name = Some(target.to_string());
    }
}

fn is_attachment_root(model: &Model, i: usize) -> bool {
    if model.bones[i].cubes.is_empty() {
        return false;
    }

    let mut parent = model.bones[i].parent;
    while let Some(p) = parent {
        if !model.bones[p].cubes.is_empty() {
            return false;
        }
        parent = model.bones[p].parent;
    }

    true
}

fn subtree_has_cubes(model: &Model) -> Vec<bool> {
    let mut has: Vec<bool> = model.bones.iter().map(|b| !b.cubes.is_empty()).collect();
    for i in (0..model.bones.len()).rev() {
        if let Some(p) = model.bones[i].parent {
            has[p] |= has[i];
        }
    }
    has
}

fn target_for(model: &Model, i: usize, slot: BodySlot) -> Option<&'static str> {
    let side = |negative, positive| {
        let x = subtree_center_x(model, i)?;
        if x < 0.0 {
            Some(negative)
        } else if x > 0.0 {
            Some(positive)
        } else {
            None
        }
    };

    let by_side = match slot {
        BodySlot::Boots => side("rightLeg", "leftLeg"),
        BodySlot::LeftHand | BodySlot::RightHand | BodySlot::Shoulder => {
            side("rightArm", "leftArm")
        }
        _ => None,
    };

    by_side.or(slot.default_bone())
}

fn subtree_center_x(model: &Model, root: usize) -> Option<f32> {
    let (mut sum, mut n) = (0.0, 0);

    for (i, bone) in model.bones.iter().enumerate() {
        if !descends_from(model, i, root) {
            continue;
        }
        for cube in &bone.cubes {
            sum += center_x(cube);
            n += 1;
        }
    }

    (n > 0).then(|| sum / n as f32)
}

fn descends_from(model: &Model, mut i: usize, root: usize) -> bool {
    loop {
        if i == root {
            return true;
        }
        match model.bones[i].parent {
            Some(p) => i = p,
            None => return false,
        }
    }
}

fn center_x(cube: &Cube) -> f32 {
    cube.origin[0] + cube.size[0] * 0.5
}

/// Splits each childless bone with cubes on both sides of X into `<name>_r` and `<name>_l`.
fn split_straddling_leaves(model: &mut Model) {
    let has_cubes = subtree_has_cubes(model);
    let mut has_child = vec![false; model.bones.len()];
    for bone in &model.bones {
        if let Some(p) = bone.parent {
            has_child[p] = true;
        }
    }

    // Only leaves split, so parent indices just shift by the bones inserted before them.
    let mut bones: Vec<Bone> = Vec::with_capacity(model.bones.len() + 2);
    let mut new_index = Vec::with_capacity(model.bones.len());
    for (i, bone) in model.bones.drain(..).enumerate() {
        let remap = |b: &mut Bone, map: &[usize]| b.parent = b.parent.map(|p| map[p]);
        let (right, left): (Vec<Cube>, Vec<Cube>) =
            bone.cubes.iter().cloned().partition(|c| center_x(c) < 0.0);
        new_index.push(bones.len());

        if !has_cubes[i] || has_child[i] || right.is_empty() || left.is_empty() {
            let mut kept = bone;
            remap(&mut kept, &new_index);
            bones.push(kept);
            continue;
        }

        for (suffix, cubes) in [("_r", right), ("_l", left)] {
            let mut half = Bone {
                name: format!("{}{suffix}", bone.name),
                cubes,
                ..bone.clone()
            };
            remap(&mut half, &new_index);
            bones.push(half);
        }
    }

    model.bones = bones;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bedrock;
    use crate::ir::FaceUv;

    fn model(bones: &str) -> Model {
        bedrock::parse_one(&format!(
            r#"{{"format_version":"1.12.0","minecraft:geometry":[{{
                "description":{{"identifier":"g","texture_width":32,"texture_height":32}},
                "bones":{bones}}}]}}"#
        ))
        .unwrap()
    }

    fn parent_of<'a>(m: &'a Model, name: &str) -> Option<&'a str> {
        m.bones[m.bone(name).unwrap()].parent_name.as_deref()
    }

    #[test]
    fn hats_go_on_the_head_and_wings_on_the_body() {
        let mut hat = model(
            r#"[{"name":"bb_main","pivot":[0,0,0],"cubes":[{"origin":[-4,32,-4],"size":[8,2,8],"uv":[0,0]}]}]"#,
        );
        attach(&mut hat, BodySlot::Hat, None);
        assert_eq!(parent_of(&hat, "bb_main"), Some("head"));

        // The cube-less grouping bone is left behind, as in the client.
        let mut wings = model(
            r#"[{"name":"wings","pivot":[0,3,-1]},
                {"name":"leftWing","parent":"wings","pivot":[-1,20,2],"cubes":[{"origin":[-33,20,2],"size":[32,16,0],"uv":[0,0]}]}]"#,
        );
        attach(&mut wings, BodySlot::Wings, None);
        let left = wings.bone("leftWing").unwrap();
        assert_eq!(wings.bones[left].parent, None);
        assert_eq!(parent_of(&wings, "leftWing"), Some("body"));
        assert_eq!(parent_of(&wings, "wings"), None);
    }

    #[test]
    fn boots_split_per_leg_and_follow_it() {
        let mut boots = model(
            r#"[{"name":"pair","pivot":[0,0,0],"cubes":[
                  {"origin":[-4,0,-2],"size":[4,4,4],"uv":[0,0]},
                  {"origin":[0,0,-2],"size":[4,4,4],"uv":[0,0]}]},
                {"name":"boot_left","pivot":[0,0,0],"cubes":[{"origin":[-4,0,-2],"size":[4,8,4],"uv":[0,8]}]}]"#,
        );
        attach(&mut boots, BodySlot::Boots, None);
        assert_eq!(parent_of(&boots, "pair_r"), Some("rightLeg"));
        assert_eq!(parent_of(&boots, "pair_l"), Some("leftLeg"));
        // Side comes from position, not name.
        assert_eq!(parent_of(&boots, "boot_left"), Some("rightLeg"));
        assert!(boots.bone("pair").is_none());
    }

    #[test]
    fn split_keeps_parent_indices_valid() {
        let mut boots = model(
            r#"[{"name":"group","pivot":[0,0,0]},
                {"name":"pair","parent":"group","pivot":[0,0,0],"cubes":[
                  {"origin":[-4,0,-2],"size":[4,4,4],"uv":[0,0]},
                  {"origin":[0,0,-2],"size":[4,4,4],"uv":[0,0]}]},
                {"name":"lace","parent":"group","pivot":[0,0,0],"cubes":[{"origin":[1,4,-2],"size":[1,1,1],"uv":[0,0]}]}]"#,
        );
        split_straddling_leaves(&mut boots);
        let group = boots.bone("group").unwrap();
        for name in ["pair_r", "pair_l", "lace"] {
            assert_eq!(
                boots.bones[boots.bone(name).unwrap()].parent,
                Some(group),
                "{name}"
            );
        }
    }

    #[test]
    fn authored_parents_are_kept_and_normalised() {
        let mut m = model(
            r#"[{"name":"brim","parent":"Right_Leg","pivot":[0,0,0],"cubes":[{"origin":[0,0,0],"size":[1,1,1],"uv":[0,0]}]},
                {"name":"visor","parent":"helmet","pivot":[0,0,0],"cubes":[{"origin":[0,0,0],"size":[1,1,1],"uv":[0,0]}]}]"#,
        );
        attach(&mut m, BodySlot::Hat, None);
        assert_eq!(parent_of(&m, "brim"), Some("rightLeg"));
        assert_eq!(parent_of(&m, "visor"), Some("hat"));
    }

    const PLANE: &str = r#"[{"name":"wing","pivot":[0,0,0],"cubes":[{"origin":[0,20,2],"size":[32,16,0],"uv":{"north":{"uv":[0,0],"uv_size":[32,16]}}}]}]"#;

    #[test]
    fn the_real_image_size_wins_over_the_declared_one() {
        // Declares 32x32 (see `model`).
        let mut m = model(PLANE);
        assert_eq!(prepare(&mut m, BodySlot::Wings, None, Some([128, 128])), 1);
        assert_eq!(m.texture_size, [128.0, 128.0]);

        let mut unknown = model(PLANE);
        prepare(&mut unknown, BodySlot::Wings, None, None);
        assert_eq!(unknown.texture_size, [32.0, 32.0]);
    }

    #[test]
    fn a_vertical_sheet_keeps_its_frame_size() {
        let mut m = model(PLANE);
        assert_eq!(prepare(&mut m, BodySlot::Wings, None, Some([32, 128])), 4);
        assert_eq!(m.texture_size, [32.0, 32.0]);

        // Two frames is below the client's threshold.
        let mut tall = model(PLANE);
        assert_eq!(prepare(&mut tall, BodySlot::Wings, None, Some([32, 64])), 1);
        assert_eq!(tall.texture_size, [32.0, 64.0]);
    }

    #[test]
    fn glow_comes_from_names_and_runs_down_the_tree() {
        assert_eq!(glow_level("right_wing.glow15"), Some(15));
        assert_eq!(
            glow_level("left_wing_glow.15"),
            Some(15),
            "bare glow is full bright"
        );
        assert_eq!(glow_level("Halo_GLOW7"), Some(7));
        assert_eq!(glow_level("aura_glow42"), Some(15), "clamped");
        assert_eq!(glow_level("wing"), None);

        let mut m = model(
            r#"[{"name":"halo_glow8","pivot":[0,0,0],"cubes":[{"origin":[0,32,0],"size":[1,1,1],"uv":[0,0]}]},
                {"name":"ring","parent":"halo_glow8","pivot":[0,0,0],"cubes":[{"origin":[0,33,0],"size":[1,1,1],"uv":[0,0]}]}]"#,
        );
        prepare(&mut m, BodySlot::Hat, None, None);
        assert_eq!(m.bones[m.bone("ring").unwrap()].glow, Some(8));
    }

    const EARS: &str = r#"[
        {"name":"ear_l","pivot":[3,30,0],"cubes":[{"origin":[2,30,0],"size":[2,3,1],"uv":[0,0]}]},
        {"name":"ear_r","pivot":[-3,30,0],"cubes":[{"origin":[-4,30,0],"size":[2,3,1],"uv":[0,0]MIRROR}]}]"#;

    fn south_u(m: &Model, bone: &str) -> FaceUv {
        m.bones[m.bone(bone).unwrap()].cubes[0].faces[crate::ir::Face::South.index()].unwrap()
    }

    #[test]
    fn an_unmirrored_pair_gets_its_plus_x_side_mirrored() {
        let mut ears = model(&EARS.replace("MIRROR", ""));
        let before = south_u(&ears, "ear_l");
        prepare(&mut ears, BodySlot::Hat, None, None);
        let after = south_u(&ears, "ear_l");
        assert_eq!(after.size[0], -before.size[0], "+X ear runs backwards in u");
        assert_eq!(
            south_u(&ears, "ear_r").size[0],
            before.size[0],
            "-X ear untouched"
        );

        let mut authored = model(&EARS.replace("MIRROR", r#","mirror":true"#));
        prepare(&mut authored, BodySlot::Hat, None, None);
        assert_eq!(south_u(&authored, "ear_l").size[0], before.size[0]);
    }

    #[test]
    fn an_anchor_overrides_the_slots_bone() {
        let mut pet = model(
            r#"[{"name":"pet","pivot":[0,0,0],"cubes":[{"origin":[4,22,-1],"size":[2,2,2],"uv":[0,0]}]}]"#,
        );
        prepare(&mut pet, BodySlot::Pet, Some("left_arm"), None);
        assert_eq!(parent_of(&pet, "pet"), Some("leftArm"));
    }

    #[test]
    fn shoulders_pick_the_arm_on_their_side_and_capes_stay_put() {
        let mut pad = model(
            r#"[{"name":"pad","pivot":[0,0,0],"cubes":[{"origin":[-8,22,-3],"size":[4,3,6],"uv":[0,0]}]}]"#,
        );
        attach(&mut pad, BodySlot::Shoulder, None);
        assert_eq!(parent_of(&pad, "pad"), Some("rightArm"));

        let mut cape = model(
            r#"[{"name":"cape","pivot":[0,24,2],"cubes":[{"origin":[-5,8,2],"size":[10,16,1],"uv":[0,0]}]}]"#,
        );
        attach(&mut cape, BodySlot::Cape, None);
        assert_eq!(parent_of(&cape, "cape"), None);
    }
}
