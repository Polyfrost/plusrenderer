use serde_json::Value;

use crate::animation::Animation;
use crate::container::{is_id, MANIFEST};
use crate::issues::{join, path, Issue, IssueCode, Mode, Report};
use crate::limits::ROLES_PER_EMOTE;
use crate::math::Affine;
use crate::pet::{InteractionDecl, INTERACTIONS};
use crate::reactions;

const MIN_SCALE: f32 = 1.0e-6;

pub const ROLE_KEYS: [&str; 4] = ["id", "animation", "offset", "yaw"];
pub const PET_ROLE_KEYS: [&str; 5] = ["interaction", "id", "animation", "offset", "yaw"];
pub const PET_ROLE_IDS: [&str; 2] = ["a", "b"];
pub const ROLES_PER_PET: usize = INTERACTIONS.len() * ROLES_PER_EMOTE as usize;
pub const PIXELS_PER_BLOCK: f32 = 16.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Role {
    pub id: String,
    pub interaction: Option<String>,
    pub animation: u16,
    pub offset: [f32; 3],
    pub yaw: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoleInput {
    pub role: u16,
    pub anchor: [f32; 3],
    pub yaw: f32,
}

impl RoleInput {
    pub fn from_floats(floats: &[f32]) -> Option<RoleInput> {
        let [role, dx, dy, dz, yaw] = *floats else {
            return None;
        };
        if !role.is_finite() || role < 0.0 || role > u16::MAX as f32 {
            return None;
        }
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        Some(RoleInput {
            role: role as u16,
            anchor: [finite(dx), finite(dy), finite(dz)],
            yaw: finite(yaw),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    pub offset: [f32; 3],
    pub yaw: f32,
}

impl Placement {
    pub fn root_position(&self) -> [f32; 3] {
        let [x, y, z] = self.offset;
        [
            x * PIXELS_PER_BLOCK,
            y * PIXELS_PER_BLOCK,
            -z * PIXELS_PER_BLOCK,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selected {
    pub animation: Option<u16>,
    pub placement: Placement,
}

pub fn turn(v: [f32; 3], yaw: f32) -> [f32; 3] {
    let radians = yaw.to_radians();
    let (s, c) = (libm::sinf(radians), libm::cosf(radians));
    let [x, y, z] = v;
    [x * c - z * s, y, x * s + z * c]
}

pub fn place(input: &RoleInput, role: Option<&Role>) -> Placement {
    let Some(role) = role else {
        return Placement {
            offset: input.anchor,
            yaw: input.yaw,
        };
    };
    let turned = turn(role.offset, input.yaw);
    Placement {
        offset: [
            input.anchor[0] + turned[0],
            input.anchor[1] + turned[1],
            input.anchor[2] + turned[2],
        ],
        yaw: input.yaw + role.yaw,
    }
}

pub fn select(roles: &[Role], input: &RoleInput) -> Option<Selected> {
    if roles.is_empty() {
        return Some(Selected {
            animation: None,
            placement: place(input, None),
        });
    }
    let role = roles.get(usize::from(input.role))?;
    Some(Selected {
        animation: Some(role.animation),
        placement: place(input, Some(role)),
    })
}

fn determinant(m: &Affine) -> f32 {
    let a = &m.0;
    a[0] * (a[5] * a[10] - a[6] * a[9]) - a[1] * (a[4] * a[10] - a[6] * a[8])
        + a[2] * (a[4] * a[9] - a[5] * a[8])
}

pub fn uniform_scale(m: &Affine) -> Option<f32> {
    let k = libm::cbrtf(determinant(m));
    (k.is_finite() && k > MIN_SCALE).then_some(k)
}

pub fn mark_root(placement: &Placement, scale: f32) -> Affine {
    Affine::translation(placement.offset)
        .mul(&Affine::rot_y(-placement.yaw.to_radians()))
        .mul(&Affine::scale([scale; 3]))
}

pub fn partway(local: &Affine, weight: f32) -> Affine {
    if weight >= 1.0 {
        return *local;
    }
    let Some(k) = uniform_scale(local).filter(|_| weight > 0.0) else {
        return Affine::IDENTITY;
    };
    let a = &local.0;
    let r = [
        a[0] / k,
        a[1] / k,
        a[2] / k,
        a[4] / k,
        a[5] / k,
        a[6] / k,
        a[8] / k,
        a[9] / k,
        a[10] / k,
    ];
    let [w, x, y, z] = quaternion(&r);
    let along = libm::sqrtf(x * x + y * y + z * z);
    let [w, x, y, z] = if along > 1.0e-7 {
        let angle = libm::atan2f(along, w) * weight;
        let s = libm::sinf(angle) / along;
        [libm::cosf(angle), x * s, y * s, z * s]
    } else {
        [1.0, 0.0, 0.0, 0.0]
    };
    let s = 1.0 + (k - 1.0) * weight;
    let m = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y - w * z),
        2.0 * (x * z + w * y),
        2.0 * (x * y + w * z),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z - w * x),
        2.0 * (x * z - w * y),
        2.0 * (y * z + w * x),
        1.0 - 2.0 * (x * x + y * y),
    ];
    Affine([
        m[0] * s,
        m[1] * s,
        m[2] * s,
        a[3] * weight,
        m[3] * s,
        m[4] * s,
        m[5] * s,
        a[7] * weight,
        m[6] * s,
        m[7] * s,
        m[8] * s,
        a[11] * weight,
    ])
}

fn quaternion(r: &[f32; 9]) -> [f32; 4] {
    let trace = r[0] + r[4] + r[8];
    let q = if trace > 0.0 {
        let s = libm::sqrtf(trace + 1.0) * 2.0;
        [
            0.25 * s,
            (r[7] - r[5]) / s,
            (r[2] - r[6]) / s,
            (r[3] - r[1]) / s,
        ]
    } else if r[0] > r[4] && r[0] > r[8] {
        let s = libm::sqrtf(1.0 + r[0] - r[4] - r[8]) * 2.0;
        [
            (r[7] - r[5]) / s,
            0.25 * s,
            (r[1] + r[3]) / s,
            (r[2] + r[6]) / s,
        ]
    } else if r[4] > r[8] {
        let s = libm::sqrtf(1.0 + r[4] - r[0] - r[8]) * 2.0;
        [
            (r[2] - r[6]) / s,
            (r[1] + r[3]) / s,
            0.25 * s,
            (r[5] + r[7]) / s,
        ]
    } else {
        let s = libm::sqrtf(1.0 + r[8] - r[0] - r[4]) * 2.0;
        [
            (r[3] - r[1]) / s,
            (r[2] + r[6]) / s,
            (r[5] + r[7]) / s,
            0.25 * s,
        ]
    };
    let len = libm::sqrtf(q.iter().map(|v| v * v).sum());
    let q = q.map(|v| v / len);
    if q[0] < 0.0 {
        q.map(|v| -v)
    } else {
        q
    }
}

fn manifest_at(key: &str) -> String {
    path(MANIFEST, key)
}

fn vector(value: Option<&Value>) -> Option<[f32; 3]> {
    let Some(value) = value else {
        return Some([0.0; 3]);
    };
    let items = value.as_array()?;
    if items.len() != 3 {
        return None;
    }
    let mut out = [0.0f32; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64().map(|n| n as f32).filter(|n| n.is_finite())?;
    }
    Some(out)
}

fn listed<'a>(manifest: &'a Value, limit: usize, r: &mut Report) -> &'a [Value] {
    let Some(value) = manifest.get("roles") else {
        return &[];
    };
    let Some(items) = value.as_array() else {
        r.push(Issue::error(IssueCode::RoleInvalid, manifest_at("roles")));
        return &[];
    };
    if items.len() > limit {
        r.push(
            Issue::error(IssueCode::RolesTooMany, manifest_at("roles"))
                .value("count", items.len() as f64)
                .value("limit", limit as f64),
        );
    }
    &items[..items.len().min(limit)]
}

fn placed(
    map: &serde_json::Map<String, Value>,
    at: &str,
    id: &str,
    interaction: Option<&str>,
    anims: &[Animation],
    r: &mut Report,
) -> Option<Role> {
    let animation = match map.get("animation").and_then(Value::as_str) {
        Some(name) => reactions::resolve(anims, name).and_then(|i| u16::try_from(i).ok()),
        None => None,
    };
    let Some(animation) = animation else {
        let mut issue = Issue::error(
            IssueCode::RoleAnimationMissing,
            manifest_at(&join(at, "animation")),
        );
        if let Some(name) = map.get("animation").and_then(Value::as_str) {
            issue = issue.name("animation", name);
        }
        r.push(issue);
        return None;
    };
    let offset = vector(map.get("offset"));
    let yaw = match map.get("yaw") {
        None => Some(0.0),
        Some(value) => value.as_f64().map(|n| n as f32).filter(|n| n.is_finite()),
    };
    let (Some(offset), Some(yaw)) = (offset, yaw) else {
        r.push(Issue::error(IssueCode::RoleInvalid, manifest_at(at)).name("id", id));
        return None;
    };
    Some(Role {
        id: id.to_string(),
        interaction: interaction.map(str::to_string),
        animation,
        offset,
        yaw,
    })
}

pub fn parse(manifest: &Value, anims: &[Animation], r: &mut Report) -> Vec<Role> {
    let items = listed(manifest, ROLES_PER_EMOTE as usize, r);
    let mut out: Vec<Role> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at = join("roles", &i.to_string());
        let Some(map) = item.as_object() else {
            r.push(Issue::error(IssueCode::RoleInvalid, manifest_at(&at)));
            continue;
        };
        r.check_keys(MANIFEST, &at, map, &ROLE_KEYS);
        let id = map.get("id").and_then(Value::as_str).unwrap_or("");
        if !is_id(id) {
            r.push(
                Issue::error(IssueCode::RoleInvalid, manifest_at(&join(&at, "id"))).name("id", id),
            );
            continue;
        }
        if out.iter().any(|role| role.id == id) {
            r.push(
                Issue::error(IssueCode::RoleDuplicate, manifest_at(&join(&at, "id")))
                    .name("id", id),
            );
            continue;
        }
        if let Some(role) = placed(map, &at, id, None, anims, r) {
            out.push(role);
        }
    }
    out
}

pub fn parse_pet(
    manifest: &Value,
    anims: &[Animation],
    declared: &[InteractionDecl],
    r: &mut Report,
) -> Vec<Role> {
    let items = listed(manifest, ROLES_PER_PET, r);
    let mut out: Vec<Role> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at = join("roles", &i.to_string());
        let Some(map) = item.as_object() else {
            r.push(Issue::error(IssueCode::RoleInvalid, manifest_at(&at)));
            continue;
        };
        r.check_keys(MANIFEST, &at, map, &PET_ROLE_KEYS);
        let interaction = map.get("interaction").and_then(Value::as_str).unwrap_or("");
        if !INTERACTIONS.contains(&interaction) {
            if r.mode() == Mode::Authoring || interaction.is_empty() {
                r.push(
                    Issue::error(
                        IssueCode::RoleInvalid,
                        manifest_at(&join(&at, "interaction")),
                    )
                    .name("interaction", interaction),
                );
            }
            continue;
        }
        let id = map.get("id").and_then(Value::as_str).unwrap_or("");
        let decl = declared.iter().find(|d| d.id == interaction);
        let allowed = match id {
            "a" => decl.is_some_and(|d| d.as_a),
            "b" => decl.is_some_and(|d| d.as_b),
            _ => false,
        };
        if !allowed {
            r.push(
                Issue::error(IssueCode::RoleInvalid, manifest_at(&join(&at, "id")))
                    .name("id", id)
                    .name("interaction", interaction),
            );
            continue;
        }
        if out
            .iter()
            .any(|role| role.id == id && role.interaction.as_deref() == Some(interaction))
        {
            r.push(
                Issue::error(IssueCode::RoleDuplicate, manifest_at(&join(&at, "id")))
                    .name("id", id)
                    .name("interaction", interaction),
            );
            continue;
        }
        if let Some(role) = placed(map, &at, id, Some(interaction), anims, r) {
            out.push(role);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn role_inputs_need_a_role_and_five_floats() {
        assert_eq!(
            RoleInput::from_floats(&[1.0, 0.5, 0.0, -2.0, 90.0]),
            Some(RoleInput {
                role: 1,
                anchor: [0.5, 0.0, -2.0],
                yaw: 90.0
            })
        );
        assert_eq!(RoleInput::from_floats(&[-1.0, 0.0, 0.0, 0.0, 0.0]), None);
        assert_eq!(
            RoleInput::from_floats(&[f32::NAN, 0.0, 0.0, 0.0, 0.0]),
            None
        );
        assert_eq!(RoleInput::from_floats(&[0.0, 0.0, 0.0, 0.0]), None);
        assert_eq!(
            RoleInput::from_floats(&[0.0, f32::INFINITY, 1.0, 0.0, f32::NAN]),
            Some(RoleInput {
                role: 0,
                anchor: [0.0, 1.0, 0.0],
                yaw: 0.0
            })
        );
    }

    fn duet() -> Vec<Role> {
        vec![
            Role {
                id: "a".to_string(),
                interaction: None,
                animation: 0,
                offset: [0.0, 0.0, -0.75],
                yaw: 0.0,
            },
            Role {
                id: "b".to_string(),
                interaction: None,
                animation: 1,
                offset: [0.0, 0.0, 0.75],
                yaw: 180.0,
            },
        ]
    }

    #[test]
    fn roles_stand_at_their_offset_turned_by_the_anchor() {
        let roles = duet();
        let anchor = |role: u16| RoleInput {
            role,
            anchor: [1.0, 0.0, 2.0],
            yaw: 90.0,
        };
        let a = select(&roles, &anchor(0)).unwrap();
        assert_eq!(a.animation, Some(0));
        assert!(
            close(a.placement.offset, [1.75, 0.0, 2.0]),
            "{:?}",
            a.placement
        );
        assert_eq!(a.placement.yaw, 90.0);

        let b = select(&roles, &anchor(1)).unwrap();
        assert_eq!(b.animation, Some(1));
        assert!(
            close(b.placement.offset, [0.25, 0.0, 2.0]),
            "{:?}",
            b.placement
        );
        assert_eq!(b.placement.yaw, 270.0, "b faces back towards a");

        assert_eq!(select(&roles, &anchor(2)), None, "only declared roles");
    }
}
