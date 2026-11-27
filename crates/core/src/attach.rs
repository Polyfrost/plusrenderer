use serde_json::Value;

use crate::container::MANIFEST;
use crate::cosmetic::player_bone;
use crate::issues::{path, Issue, IssueCode, Report};
use crate::math::Affine;
use crate::molang::{Interner, Rules};
use crate::player::{player_locator, PLAYER_LOCATORS};
use crate::signals::{SignalKind, SIGNALS, TIME_SUFFIX};
use crate::wearer::{seed, seed_key};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum RigKind {
    #[default]
    Player = 0,
    Actor = 2,
    Boat = 3,
    Minecart = 4,
    Horse = 5,
}

impl RigKind {
    pub const ALL: [RigKind; 5] = [
        RigKind::Player,
        RigKind::Actor,
        RigKind::Boat,
        RigKind::Minecart,
        RigKind::Horse,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RigKind::Player => "player",
            RigKind::Actor => "actor",
            RigKind::Boat => "boat",
            RigKind::Minecart => "minecart",
            RigKind::Horse => "horse",
        }
    }

    pub fn parse(name: &str) -> Option<RigKind> {
        RigKind::ALL.into_iter().find(|k| k.as_str() == name)
    }

    pub fn index(self) -> u32 {
        self as u32
    }

    pub fn from_index(index: u32) -> Option<RigKind> {
        RigKind::ALL.into_iter().find(|k| k.index() == index)
    }

    pub fn is_vehicle(self) -> bool {
        matches!(self, RigKind::Boat | RigKind::Minecart | RigKind::Horse)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigBone {
    pub name: &'static str,
    pub parent: Option<u8>,
    pub pivot: [f32; 3],
}

const fn bone(name: &'static str, parent: Option<u8>, pivot: [f32; 3]) -> RigBone {
    RigBone {
        name,
        parent,
        pivot,
    }
}

const fn vanilla(x: f32, y: f32, z: f32) -> [f32; 3] {
    [x, 24.0 - y, -z]
}

const PLAYER_RIG: [RigBone; 8] = [
    bone("root", None, [0.0, 0.0, 0.0]),
    bone("body", Some(0), [0.0, 24.0, 0.0]),
    bone("head", Some(0), [0.0, 24.0, 0.0]),
    bone("hat", Some(2), [0.0, 24.0, 0.0]),
    bone("rightArm", Some(0), [-5.0, 22.0, 0.0]),
    bone("leftArm", Some(0), [5.0, 22.0, 0.0]),
    bone("rightLeg", Some(0), [-1.9, 12.0, 0.0]),
    bone("leftLeg", Some(0), [1.9, 12.0, 0.0]),
];

const ACTOR_RIG: [RigBone; 1] = [bone("root", None, [0.0, 0.0, 0.0])];

const BOAT_RIG: [RigBone; 11] = [
    bone("root", None, vanilla(0.0, 0.0, 0.0)),
    bone("bottom", Some(0), vanilla(0.0, 3.0, 1.0)),
    bone("back", Some(0), vanilla(-15.0, 4.0, 4.0)),
    bone("front", Some(0), vanilla(15.0, 4.0, 0.0)),
    bone("right", Some(0), vanilla(0.0, 4.0, -9.0)),
    bone("left", Some(0), vanilla(0.0, 4.0, 9.0)),
    bone("left_paddle", Some(0), vanilla(3.0, -5.0, 9.0)),
    bone("right_paddle", Some(0), vanilla(3.0, -5.0, -9.0)),
    bone("chest_bottom", Some(0), vanilla(-2.0, -5.0, -6.0)),
    bone("chest_lid", Some(0), vanilla(-2.0, -9.0, -6.0)),
    bone("chest_lock", Some(0), vanilla(-1.0, -6.0, -1.0)),
];

const MINECART_RIG: [RigBone; 6] = [
    bone("root", None, vanilla(0.0, 0.0, 0.0)),
    bone("bottom", Some(0), vanilla(0.0, 4.0, 0.0)),
    bone("front", Some(0), vanilla(-9.0, 4.0, 0.0)),
    bone("back", Some(0), vanilla(9.0, 4.0, 0.0)),
    bone("left", Some(0), vanilla(0.0, 4.0, -7.0)),
    bone("right", Some(0), vanilla(0.0, 4.0, 7.0)),
];

const HORSE_BODY: [f32; 3] = vanilla(0.0, 11.0, 5.0);
const HORSE_HEAD: [f32; 3] = vanilla(0.0, 4.0, -12.0);

const HORSE_RIG: [RigBone; 20] = [
    bone("root", None, vanilla(0.0, 0.0, 0.0)),
    bone("body", Some(0), HORSE_BODY),
    bone("tail", Some(1), vanilla(0.0, 6.0, 7.0)),
    bone("saddle", Some(1), HORSE_BODY),
    bone("head_parts", Some(0), HORSE_HEAD),
    bone("head", Some(4), HORSE_HEAD),
    bone("left_ear", Some(5), HORSE_HEAD),
    bone("right_ear", Some(5), HORSE_HEAD),
    bone("mane", Some(4), HORSE_HEAD),
    bone("upper_mouth", Some(4), HORSE_HEAD),
    bone("left_saddle_mouth", Some(4), HORSE_HEAD),
    bone("right_saddle_mouth", Some(4), HORSE_HEAD),
    bone("left_saddle_line", Some(4), HORSE_HEAD),
    bone("right_saddle_line", Some(4), HORSE_HEAD),
    bone("head_saddle", Some(4), HORSE_HEAD),
    bone("mouth_saddle_wrap", Some(4), HORSE_HEAD),
    bone("left_hind_leg", Some(0), vanilla(4.0, 14.0, 7.0)),
    bone("right_hind_leg", Some(0), vanilla(-4.0, 14.0, 7.0)),
    bone("left_front_leg", Some(0), vanilla(4.0, 14.0, -10.0)),
    bone("right_front_leg", Some(0), vanilla(-4.0, 14.0, -10.0)),
];

pub fn rig(k: RigKind) -> &'static [RigBone] {
    match k {
        RigKind::Player => &PLAYER_RIG,
        RigKind::Actor => &ACTOR_RIG,
        RigKind::Boat => &BOAT_RIG,
        RigKind::Minecart => &MINECART_RIG,
        RigKind::Horse => &HORSE_RIG,
    }
}

pub fn rig_bone(k: RigKind, name: &str) -> Option<usize> {
    rig(k).iter().position(|b| b.name == name)
}

#[derive(Clone, Debug, PartialEq)]
pub struct GamePart {
    pub bone: String,
    pub transform: Affine,
    pub pivot: [f32; 3],
    pub active: bool,
}

fn fed<'a>(parts: &'a [GamePart], name: &str) -> Option<&'a GamePart> {
    parts.iter().find(|p| p.active && p.bone == name)
}

pub fn compose(k: RigKind, parts: &[GamePart], bone: &str) -> Option<(Affine, [f32; 3])> {
    let bones = rig(k);
    let own = fed(parts, bone);
    let index = rig_bone(k, bone);
    let pivot = match (own, index) {
        (Some(part), _) => part.pivot,
        (None, Some(i)) if k != RigKind::Player => bones[i].pivot,
        (None, _) => return None,
    };
    let mut transform = own.map_or(Affine::IDENTITY, |p| p.transform);
    let mut found = own.is_some();
    let mut parent = index.map_or(Some(0), |i| bones[i].parent.map(usize::from));
    while let Some(at) = parent {
        let entry = &bones[at];
        if let Some(part) = fed(parts, entry.name) {
            transform = part.transform.mul(&transform);
            found = true;
        }
        parent = entry.parent.map(usize::from);
    }
    found.then_some((transform, pivot))
}

pub fn game_part_index(parts: &[GamePart], bone: &str) -> Option<usize> {
    parts.iter().position(|p| p.bone == bone)
}

pub fn rig_parts(k: RigKind, parts: &[GamePart], out: &mut Vec<Option<usize>>) {
    out.clear();
    out.extend(
        rig(k)
            .iter()
            .map(|entry| game_part_index(parts, entry.name)),
    );
}

fn fed_at(parts: &[GamePart], index: Option<usize>) -> Option<&GamePart> {
    parts.get(index?).filter(|p| p.active)
}

pub fn compose_at(
    k: RigKind,
    parts: &[GamePart],
    rig_parts: &[Option<usize>],
    own: Option<usize>,
    index: Option<usize>,
) -> Option<(Affine, [f32; 3])> {
    let bones = rig(k);
    let own = fed_at(parts, own);
    let pivot = match (own, index) {
        (Some(part), _) => part.pivot,
        (None, Some(i)) if k != RigKind::Player => bones[i].pivot,
        (None, _) => return None,
    };
    let mut transform = own.map_or(Affine::IDENTITY, |p| p.transform);
    let mut found = own.is_some();
    let mut parent = index.map_or(Some(0), |i| bones[i].parent.map(usize::from));
    while let Some(at) = parent {
        let entry = &bones[at];
        if let Some(part) = fed_at(parts, rig_parts.get(at).copied().flatten()) {
            transform = part.transform.mul(&transform);
            found = true;
        }
        parent = entry.parent.map(usize::from);
    }
    found.then_some((transform, pivot))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attach {
    pub rig: RigKind,
    pub bone: Option<String>,
    pub replaces: Option<String>,
    pub anchor: Option<PlayerLocator>,
}

const ATTACH: &str = "attach";
const WORN_ANCHORS: [PlayerLocator; 1] = [PlayerLocator::Tail];
const ATTACH_KEYS: [&str; 3] = ["rig", "bone", "replaces"];

fn attach_path(key: &str) -> String {
    path(MANIFEST, &format!("{ATTACH}.{key}"))
}

impl Attach {
    pub fn offset(&self) -> [f32; 3] {
        self.anchor.map_or([0.0; 3], PlayerLocator::offset)
    }

    pub fn parse(value: &Value, r: &mut Report) -> Attach {
        let Some(object) = value.as_object() else {
            r.push(Issue::error(
                IssueCode::AttachInvalid,
                path(MANIFEST, ATTACH),
            ));
            return Attach::default();
        };
        r.check_keys(MANIFEST, ATTACH, object, &ATTACH_KEYS);
        let mut text = |key: &str| match object.get(key) {
            None => None,
            Some(Value::String(s)) => Some(s.as_str()),
            Some(_) => {
                r.push(Issue::error(IssueCode::AttachInvalid, attach_path(key)));
                None
            }
        };
        let (rig_name, bone, replaces) = (text("rig"), text("bone"), text("replaces"));

        let kind = match rig_name.map(|name| (name, RigKind::parse(name))) {
            None => RigKind::Player,
            Some((_, Some(kind))) => kind,
            Some((name, None)) => {
                r.push(Issue::error(IssueCode::RigUnknown, attach_path("rig")).name("rig", name));
                return Attach::default();
            }
        };
        let anchor = bone
            .filter(|_| kind == RigKind::Player)
            .and_then(|name| WORN_ANCHORS.into_iter().find(|a| a.as_str() == name));
        let bone = bone.and_then(|name| {
            let found = match (kind, anchor) {
                (_, Some(anchor)) => Some(anchor.bone()),
                (RigKind::Player, None) => player_bone(name),
                _ => rig_bone(kind, name).map(|i| rig(kind)[i].name),
            };
            if found.is_none() {
                r.push(
                    Issue::error(IssueCode::AttachBoneUnknown, attach_path("bone"))
                        .name("rig", kind.as_str())
                        .name("bone", name),
                );
            }
            found
        });
        let replaces = replaces.and_then(|name| {
            let found = match kind {
                k if k.is_vehicle() => rig_bone(k, name).filter(|&i| i > 0).map(|i| rig(k)[i].name),
                _ => {
                    r.push(
                        Issue::error(IssueCode::AttachInvalid, attach_path("replaces"))
                            .name("rig", kind.as_str()),
                    );
                    return None;
                }
            };
            if found.is_none() {
                r.push(
                    Issue::error(IssueCode::ReplacementUnknown, attach_path("replaces"))
                        .name("rig", kind.as_str())
                        .name("item", name),
                );
            }
            found
        });

        Attach {
            rig: kind,
            bone: bone.map(str::to_string),
            replaces: replaces.map(str::to_string),
            anchor,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlayerLocator {
    Feet,
    Body,
    Head,
    RightHand,
    LeftHand,
    Back,
    AboveHead,
    LeftWingTip,
    RightWingTip,
    Tail,
}

impl PlayerLocator {
    pub const ALL: [PlayerLocator; 10] = [
        PlayerLocator::Feet,
        PlayerLocator::Body,
        PlayerLocator::Head,
        PlayerLocator::RightHand,
        PlayerLocator::LeftHand,
        PlayerLocator::Back,
        PlayerLocator::AboveHead,
        PlayerLocator::LeftWingTip,
        PlayerLocator::RightWingTip,
        PlayerLocator::Tail,
    ];

    pub const COUNT: usize = PlayerLocator::ALL.len();

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        PLAYER_LOCATORS[self.index()].name
    }

    pub fn bone(self) -> &'static str {
        PLAYER_LOCATORS[self.index()].bone
    }

    pub fn offset(self) -> [f32; 3] {
        PLAYER_LOCATORS[self.index()].offset
    }

    pub fn wing(self) -> Option<&'static str> {
        PLAYER_LOCATORS[self.index()].wing
    }

    pub fn parse(name: &str) -> Option<PlayerLocator> {
        PlayerLocator::ALL.get(player_locator(name)?).copied()
    }
}

pub fn actor_wearer(owner: u64, slot: u8) -> u64 {
    seed(owner, seed_key("actor"), u64::from(slot))
}

pub fn event_wearer(owner: u64, event: u8, seq: u32) -> u64 {
    seed(
        owner,
        seed_key("event"),
        (u64::from(event) << 32) | u64::from(seq),
    )
}

pub fn event_key(owner: u64, event: u8, seq: u32, nonce: u32) -> u64 {
    let seed_wearer = event_wearer(owner, event, seq);
    if nonce == 0 {
        return seed_wearer;
    }
    seed(seed_wearer, seed_key("event_key"), u64::from(nonce))
}

pub fn pair_seed(a: u64, b: u64, minute: i64) -> u64 {
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    seed(low, high, minute as u64 ^ seed_key("pair"))
}

pub const OWNER_PREFIX: &str = "owner.";

pub fn owner_signal(query: &str) -> Option<&str> {
    query.strip_prefix(OWNER_PREFIX)
}

pub fn warn_plain_signals(interner: &Interner, rules: Rules, file: &str, r: &mut Report) {
    if rules != Rules::Strict {
        return;
    }
    for (name, kind) in SIGNALS {
        let timed = format!("{name}{TIME_SUFFIX}");
        let queries = match kind {
            SignalKind::Bool => [Some(name), Some(timed.as_str())],
            SignalKind::Float => [Some(name), None],
        };
        for query in queries.into_iter().flatten() {
            if interner.query_slot(query).is_some() {
                r.push(
                    Issue::warning(IssueCode::PetPlainQuery, file)
                        .name("query", query)
                        .name("owner", &format!("{OWNER_PREFIX}{query}")),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issues::{IssueCode, Mode};

    #[test]
    fn rigs_keep_fixed_host_numbers() {
        let numbered: Vec<(u32, &str)> = RigKind::ALL
            .iter()
            .map(|k| (k.index(), k.as_str()))
            .collect();
        assert_eq!(
            numbered,
            [
                (0, "player"),
                (2, "actor"),
                (3, "boat"),
                (4, "minecart"),
                (5, "horse")
            ]
        );
        for kind in RigKind::ALL {
            assert_eq!(RigKind::from_index(kind.index()), Some(kind));
        }
        assert_eq!(RigKind::from_index(1), None);
        assert_eq!(RigKind::from_index(6), None);
        assert_eq!(RigKind::parse("held_item"), None);
    }

    #[test]
    fn pair_seeds_ignore_order_and_are_pinned() {
        let a = 0x0123_4567_89ab_cdef;
        let b = 0xfedc_ba98_7654_3210;
        assert_eq!(pair_seed(a, b, 29_000_000), pair_seed(b, a, 29_000_000));
        assert_ne!(pair_seed(a, b, 29_000_000), pair_seed(a, b, 29_000_001));
        assert_ne!(pair_seed(a, a, 0), pair_seed(a, b, 0));
        assert_eq!(pair_seed(a, b, 29_000_000), 0xf5d9_78ea_06d8_bb00);
        assert_eq!(pair_seed(1, 2, -1), 0xc723_1d6d_7f60_3f90);
        assert_eq!(pair_seed(0, 0, 0), 0x4fcb_c078_f1b2_c4ba);
    }

    #[test]
    fn attach_reads_a_rig_bone_and_what_it_replaces() {
        let parse = |json: &str| {
            let mut report = Report::new(Mode::Authoring);
            let attach = Attach::parse(&serde_json::from_str(json).unwrap(), &mut report);
            let codes: Vec<_> = report.issues().iter().map(|i| i.code).collect();
            (attach, codes)
        };
        let (attach, issues) = parse(r#"{"rig":"player","bone":"right_arm"}"#);
        assert_eq!((attach.rig, attach.bone.as_deref(), issues), (RigKind::Player, Some("rightArm"), vec![]));
        assert_eq!(parse(r#"{"rig":"boat","replaces":"left_paddle"}"#).0.replaces.as_deref(), Some("left_paddle"));
        let tail = Attach { rig: RigKind::Player, bone: Some("body".to_string()), replaces: None, anchor: Some(PlayerLocator::Tail) };
        assert_eq!(parse(r#"{"rig":"player","bone":"tail"}"#), (tail, vec![]));
        assert_eq!(parse(r#"{"rig":"player","bone":"tail"}"#).0.offset(), [0.0, 13.0, -2.0]);
        assert_eq!(parse(r#"{"bone":"body"}"#).0.offset(), [0.0; 3]);
        assert_eq!(parse(r#"{"rig":"horse","bone":"tail"}"#).0.anchor, None);
        assert_eq!(parse(r#"{"bone":"head"}"#).0.rig, RigKind::Player);
        for (json, code) in [
            (r#"{"rig":"held_item","replaces":"shield"}"#, IssueCode::RigUnknown),
            (r#"{"rig":"player","bone":"back"}"#, IssueCode::AttachBoneUnknown),
            (r#"{"rig":"boat","bone":"tail"}"#, IssueCode::AttachBoneUnknown),
            (r#"{"rig":"boat","replaces":"anvil"}"#, IssueCode::ReplacementUnknown),
            (r#"{"rig":"player","replaces":"head"}"#, IssueCode::AttachInvalid),
            (r#""head""#, IssueCode::AttachInvalid),
        ] {
            assert_eq!(parse(json).1, [code], "{json}");
        }
    }
}
