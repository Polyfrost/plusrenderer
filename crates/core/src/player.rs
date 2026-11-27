//! The vanilla player as ordinary bedrock geometry. The outer layer is cubes with `inflate`.

use crate::bedrock;
use crate::ir::Model;

const PLAYER_GEO: &str = include_str!("../assets/player.geo.json");
pub const PLAYER_BONES: usize = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arms {
    Wide,
    Slim,
}

const CAPE_GEO: &str = include_str!("../assets/cape.geo.json");

/// Vanilla's cape, parented to `body`, leaning 6 degrees back, outside (uv (1,1)) facing away.
// ponytail: fixed at the standing lean, but a host can swing it through the `cape` bone's pose.
// Simulate the swing in the core if a host without the game needs a moving cape.
pub fn cape() -> Model {
    bedrock::parse_one(CAPE_GEO).expect("baked cape geometry must parse")
}

const ELYTRA_GEO: &str = include_str!("../assets/elytra.geo.json");

/// Vanilla's elytra at its standing spread, on the cape texture's (22,0) box or `elytra.png`.
/// Parented to `root`, as vanilla draws it in entity space. The body's crouch lean would double up.
pub fn elytra() -> Model {
    bedrock::parse_one(ELYTRA_GEO).expect("baked elytra geometry must parse")
}

/// The wing poses of vanilla's `ElytraModel`.
/// Swimming keeps `Standing`. Its tilt, like flight's, is a whole-player turn on `root`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ElytraPose {
    Standing,
    Crouching,
    /// `fall` is how steeply the player dives, 0..=1: `max(0, -velocity.normalize().y)`.
    /// Level flight spreads the wings flat and a straight dive folds them back to standing.
    Flying {
        fall: f32,
    },
}

impl ElytraPose {
    /// The left wing's rotation in degrees and drop in pixels. The right wing mirrors Y and Z.
    fn left_wing(self) -> ([f32; 3], f32) {
        match self {
            ElytraPose::Standing => ([15.0, 0.0, -15.0], 0.0),
            ElytraPose::Crouching => ([40.0, 5.0, -45.0], 3.0),
            ElytraPose::Flying { fall } => {
                let spread = 1.0 - libm::powf(fall.clamp(0.0, 1.0), 1.5);
                let lerp = |from: f32, to: f32| from + (to - from) * spread;
                ([lerp(15.0, 20.0), 0.0, lerp(-15.0, -90.0)], 0.0)
            }
        }
    }
}

/// Sets an `elytra()` model's wings. Other models are left alone.
// ponytail: snaps to the pose, vanilla eases 30% of the way per tick. A host can ease `fall` itself.
pub fn pose_elytra(model: &mut Model, pose: ElytraPose) {
    let ([x, y, z], drop) = pose.left_wing();

    for (name, side) in [("leftWing", 1.0), ("rightWing", -1.0)] {
        let Some(i) = model.bone(name) else {
            continue;
        };

        // Wings hang from the neck, 24 up, and a crouch drops them.
        let bone = &mut model.bones[i];
        let shift = 24.0 - drop - bone.pivot[1];
        bone.pivot[1] += shift;
        bone.rotation = [x, y * side, z * side];

        for cube in &mut bone.cubes {
            cube.origin[1] += shift;
            cube.pivot[1] += shift;
        }
    }
}

pub fn elytra_pose(
    gliding: bool,
    sneaking: bool,
    ground_speed: f32,
    vertical_speed: f32,
) -> ElytraPose {
    if gliding {
        let speed = libm::sqrtf(ground_speed * ground_speed + vertical_speed * vertical_speed);
        let fall = -vertical_speed / speed;
        ElytraPose::Flying {
            fall: if fall > 0.0 { fall.min(1.0) } else { 0.0 },
        }
    } else if sneaking {
        ElytraPose::Crouching
    } else {
        ElytraPose::Standing
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerLocatorDef {
    pub name: &'static str,
    pub bone: &'static str,
    pub offset: [f32; 3],
    pub wing: Option<&'static str>,
}

pub const PLAYER_LOCATORS: [PlayerLocatorDef; 10] = [
    PlayerLocatorDef {
        name: "feet",
        bone: "root",
        offset: [0.0; 3],
        wing: None,
    },
    PlayerLocatorDef {
        name: "body",
        bone: "body",
        offset: [0.0, 18.0, 0.0],
        wing: None,
    },
    PlayerLocatorDef {
        name: "head",
        bone: "head",
        offset: [0.0, 28.0, 0.0],
        wing: None,
    },
    PlayerLocatorDef {
        name: "right_hand",
        bone: "rightArm",
        offset: [-6.0, 12.0, 0.0],
        wing: None,
    },
    PlayerLocatorDef {
        name: "left_hand",
        bone: "leftArm",
        offset: [6.0, 12.0, 0.0],
        wing: None,
    },
    PlayerLocatorDef {
        name: "back",
        bone: "body",
        offset: [0.0, 18.0, -2.0],
        wing: None,
    },
    PlayerLocatorDef {
        name: "above_head",
        bone: "head",
        offset: [0.0, 34.0, 0.0],
        wing: None,
    },
    PlayerLocatorDef {
        name: "left_wing_tip",
        bone: "root",
        offset: [5.0, 4.0, 4.0],
        wing: Some("leftWing"),
    },
    PlayerLocatorDef {
        name: "right_wing_tip",
        bone: "root",
        offset: [-5.0, 4.0, 4.0],
        wing: Some("rightWing"),
    },
    PlayerLocatorDef {
        name: "tail",
        bone: "body",
        offset: [0.0, 13.0, -2.0],
        wing: None,
    },
];

pub fn player_locator(name: &str) -> Option<usize> {
    PLAYER_LOCATORS.iter().position(|l| l.name == name)
}

pub fn model(arms: Arms) -> Model {
    let want = match arms {
        Arms::Wide => "geometry.player.wide",
        Arms::Slim => "geometry.player.slim",
    };

    bedrock::parse(PLAYER_GEO)
        .expect("baked player geometry must parse")
        .into_iter()
        .find(|m| m.identifier == want)
        .expect("baked player geometry must contain both arm variants")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_locators_are_named_in_a_fixed_order() {
        let names: Vec<_> = PLAYER_LOCATORS.iter().map(|l| l.name).collect();
        assert_eq!(
            names,
            [
                "feet",
                "body",
                "head",
                "right_hand",
                "left_hand",
                "back",
                "above_head",
                "left_wing_tip",
                "right_wing_tip",
                "tail"
            ]
        );
        for (i, l) in PLAYER_LOCATORS.iter().enumerate() {
            assert_eq!(player_locator(l.name), Some(i));
        }
        assert_eq!(player_locator("tailbone"), None);
    }

    /// Every skin's head front is the 8x8 square at (8,8).
    #[test]
    fn layers_are_inflated_cubes_and_parented_to_their_part() {
        let m = model(Arms::Wide);
        for (layer, base) in [
            ("hat", "head"),
            ("jacket", "body"),
            ("rightSleeve", "rightArm"),
            ("leftSleeve", "leftArm"),
            ("rightPants", "rightLeg"),
            ("leftPants", "leftLeg"),
        ] {
            let i = m.bone(layer).unwrap_or_else(|| panic!("missing {layer}"));
            let parent = m.bones[i].parent.expect("layer must be parented");
            assert_eq!(m.bones[parent].name, base);
            assert!(
                m.bones[i].cubes[0].inflate > 0.0,
                "{layer} must be inflated"
            );
        }
    }

    #[test]
    fn the_elytra_wings_mirror_and_spread_behind() {
        let elytra = elytra();
        let mut out = Vec::new();
        crate::pose::world_transforms(&elytra, None, |_| None, &mut out);

        let hem = |i: usize| {
            let (min, max) = elytra.bones[i].cubes[0].bounds();
            let x = if i == 0 { max[0] } else { min[0] };
            out[i].point([x, min[1], max[2]])
        };
        let (left, right) = (hem(0), hem(1));

        assert_eq!(elytra.bones[0].name, "leftWing");
        assert!((left[0] + right[0]).abs() < 1e-4, "{left:?} vs {right:?}");
        assert!(
            left[0] > 6.0,
            "the left hem swings out past the pivot: {left:?}"
        );
        assert!(left[2] < -2.0, "the hem hangs behind the back: {left:?}");

        // The right wing's box UV is mirrored, as vanilla's `mirror = true`.
        assert!(!elytra.bones[0].cubes[0].mirror);
        assert!(elytra.bones[1].cubes[0].mirror);
    }

    #[test]
    fn elytra_poses_follow_vanilla_and_return_to_standing() {
        let mut elytra = elytra();
        let wing = |m: &Model, i: usize| (m.bones[i].rotation, m.bones[i].pivot[1]);

        pose_elytra(&mut elytra, ElytraPose::Crouching);
        assert_eq!(wing(&elytra, 0), ([40.0, 5.0, -45.0], 21.0));
        assert_eq!(wing(&elytra, 1), ([40.0, -5.0, 45.0], 21.0));
        assert_eq!(elytra.bones[1].cubes[0].origin[1], 1.0);

        // Level flight spreads the wings flat, a straight dive folds them to standing.
        pose_elytra(&mut elytra, ElytraPose::Flying { fall: 0.0 });
        assert_eq!(wing(&elytra, 1), ([20.0, 0.0, 90.0], 24.0));
        pose_elytra(&mut elytra, ElytraPose::Flying { fall: 1.0 });
        assert_eq!(wing(&elytra, 0), ([15.0, 0.0, -15.0], 24.0));

        pose_elytra(&mut elytra, ElytraPose::Standing);
        let fresh = super::elytra();
        for (posed, rest) in elytra.bones.iter().zip(&fresh.bones) {
            assert_eq!(posed.rotation, rest.rotation);
            assert_eq!(posed.pivot, rest.pivot);
            assert_eq!(posed.cubes[0].origin, rest.cubes[0].origin);
        }
    }
}
