//! The vanilla player as ordinary bedrock geometry. The outer layer is cubes with `inflate`.

use crate::bedrock;
use crate::ir::Model;

const PLAYER_GEO: &str = include_str!("../assets/player.geo.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arms {
    Wide,
    Slim,
}

const CAPE_GEO: &str = include_str!("../assets/cape.geo.json");

/// Vanilla's cape, parented to `body`, leaning 6 degrees back, outside (uv (1,1)) facing away.
// ponytail: fixed at the standing lean, but a host can swing it through the `cape` bone's pose.
pub fn cape() -> Model {
    bedrock::parse_one(CAPE_GEO).expect("baked cape geometry must parse")
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
    use crate::ir::{corner_uv, Face};

    /// Every skin's head front is the 8x8 square at (8,8).
    #[test]
    fn head_front_face_is_the_face() {
        let m = model(Arms::Wide);
        let head = &m.bones[m.bone("head").unwrap()];
        let uv = head.cubes[0].faces[Face::South.index()].unwrap();
        assert_eq!(uv.uv, [8.0, 8.0]);
        assert_eq!(uv.size, [8.0, 8.0]);
        // Corners run across then down.
        assert_eq!(corner_uv(uv, 0), [8.0, 8.0]);
        assert_eq!(corner_uv(uv, 2), [16.0, 16.0]);
    }

    #[test]
    fn box_uv_wraps_the_whole_cross() {
        let m = model(Arms::Wide);
        let head = &m.bones[m.bone("head").unwrap()];
        let f = &head.cubes[0].faces;
        assert_eq!(f[Face::Up.index()].unwrap().uv, [8.0, 0.0]);
        assert_eq!(f[Face::West.index()].unwrap().uv, [0.0, 8.0]);
        assert_eq!(f[Face::East.index()].unwrap().uv, [16.0, 8.0]);
        assert_eq!(f[Face::North.index()].unwrap().uv, [24.0, 8.0]);
        assert!(f[Face::Down.index()].unwrap().size[1] < 0.0);
    }

    #[test]
    fn slim_arms_are_three_wide() {
        let wide = model(Arms::Wide);
        let slim = model(Arms::Slim);
        let w = &wide.bones[wide.bone("rightArm").unwrap()].cubes[0];
        let s = &slim.bones[slim.bone("rightArm").unwrap()].cubes[0];
        assert_eq!(w.size[0], 4.0);
        assert_eq!(s.size[0], 3.0);
        assert_eq!(w.origin[0] + w.size[0], s.origin[0] + s.size[0]);
    }

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
    fn bones_are_topologically_sorted() {
        for arms in [Arms::Wide, Arms::Slim] {
            let m = model(arms);
            for (i, b) in m.bones.iter().enumerate() {
                if let Some(p) = b.parent {
                    assert!(p < i, "{} comes before its parent", b.name);
                }
            }
        }
    }

    #[test]
    fn the_cape_hangs_behind_and_leans_away() {
        let cape = cape();
        assert_eq!(cape.bones[0].parent_name.as_deref(), Some("body"));
        let mut out = Vec::new();
        crate::pose::world_transforms(&cape, None, |_| None, &mut out);
        let c = &cape.bones[0].cubes[0];
        let (min, max) = c.bounds();
        let top = out[0].point([0.0, max[1], max[2]]);
        let hem = out[0].point([0.0, min[1], max[2]]);

        assert!(
            top[2] <= -2.0 + 1e-4,
            "behind the body's back at z=-2: {top:?}"
        );
        assert!(
            hem[2] < top[2],
            "the hem leans further back: {hem:?} vs {top:?}"
        );
        assert_eq!(c.faces[Face::North.index()].unwrap().uv, [1.0, 1.0]);
    }
}
