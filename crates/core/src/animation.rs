//! Bedrock `.animation.json`: keyframes, interpolation and blending into a `Pose`.
//! `anim_time_update`, `relative_to` and sound/particle/timeline sections are ignored, not rejected.

use std::fmt;

use serde_json::{Map, Value};

use crate::ir::Model;
use crate::molang::{self, Interner, Program, Runtime};
use crate::pose::Pose;

#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    Molang(molang::Error),
    NoAnimations,
    BadValue(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(e) => write!(f, "{e}"),
            Error::Molang(e) => write!(f, "{e}"),
            Error::NoAnimations => f.write_str("no `animations` object"),
            Error::BadValue(what) => write!(f, "cannot read {what} as an animation value"),
        }
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Error {
        Error::Json(e)
    }
}

impl From<molang::Error> for Error {
    fn from(e: molang::Error) -> Error {
        Error::Molang(e)
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loop {
    /// Holds the last frame, as the client does for cosmetics. `hold_on_last_frame` reads the same.
    Once,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lerp {
    Linear,
    Step,
    CatmullRom,
    Bezier,
}

#[derive(Debug, Clone)]
struct Handles {
    left_time: [f32; 3],
    left_value: [f32; 3],
    right_time: [f32; 3],
    right_value: [f32; 3],
}

#[derive(Debug, Clone)]
struct Keyframe {
    time: f32,
    pre: [Program; 3],
    post: [Program; 3],
    lerp: Lerp,
    handles: Option<Box<Handles>>,
}

/// Sorted by time. Empty means the channel is absent.
#[derive(Debug, Clone, Default)]
struct Channel(Vec<Keyframe>);

#[derive(Debug, Clone)]
pub struct BoneChannels {
    pub name: String,
    rotation: Channel,
    position: Channel,
    scale: Channel,
}

#[derive(Debug, Clone)]
pub struct Animation {
    pub name: String,
    pub length: f32,
    pub loop_mode: Loop,
    start_delay: Program,
    loop_delay: Program,
    blend_weight: Program,
    pub bones: Vec<BoneChannels>,
}

impl Animation {
    /// Time within the animation, or `None` before it starts.
    // ponytail: delays re-evaluate per call, so a random delay re-rolls. Cache them if that bites.
    pub fn local_time(&self, elapsed: f32, rt: &mut Runtime) -> Option<f32> {
        let t = elapsed - self.start_delay.eval(rt);
        if t < 0.0 {
            return None;
        }

        match self.loop_mode {
            Loop::Once => Some(t.min(self.length)),
            Loop::Repeat => {
                let period = self.length + self.loop_delay.eval(rt).max(0.0);
                if period <= 0.0 {
                    return Some(0.0);
                }
                Some((t % period).min(self.length))
            }
        }
    }

    /// A `Once` animation past its end only repeats its last frame.
    pub fn holds_at(&self, time: f32) -> bool {
        self.loop_mode == Loop::Once && time >= self.length
    }

    pub fn weight(&self, rt: &mut Runtime) -> f32 {
        self.blend_weight.eval(rt)
    }

    pub fn bind(&self, model: &Model) -> Vec<Option<usize>> {
        self.bones.iter().map(|b| model.bone(&b.name)).collect()
    }
}

/// Blends over the rest pose: rotation and position scale by `weight`, scale eases towards 1.
pub fn evaluate(
    animation: &Animation,
    bind: &[Option<usize>],
    time: f32,
    weight: f32,
    rt: &mut Runtime,
    pose: &mut Pose,
) {
    pose.reset();

    for (channels, bone) in animation.bones.iter().zip(bind) {
        let Some(target) = bone.and_then(|i| pose.0.get_mut(i)) else {
            continue;
        };

        accumulate(&mut target.rotation, &channels.rotation, time, weight, rt);
        accumulate(&mut target.position, &channels.position, time, weight, rt);

        if let Some(scale) = sample(&channels.scale, time, rt) {
            for (axis, value) in target.scale.iter_mut().zip(scale) {
                *axis *= 1.0 + (value - 1.0) * weight;
            }
        }
    }
}

fn accumulate(target: &mut [f32; 3], channel: &Channel, time: f32, weight: f32, rt: &mut Runtime) {
    if let Some(value) = sample(channel, time, rt) {
        for i in 0..3 {
            target[i] += value[i] * weight;
        }
    }
}

fn sample(channel: &Channel, time: f32, rt: &mut Runtime) -> Option<[f32; 3]> {
    let frames = &channel.0;
    let last = frames.len().checked_sub(1)?;

    if time <= frames[0].time {
        return Some(triple(&frames[0].pre, rt));
    }
    if time >= frames[last].time {
        return Some(triple(&frames[last].post, rt));
    }

    let next = frames.partition_point(|k| k.time <= time);
    let (a, b) = (&frames[next - 1], &frames[next]);
    let span = b.time - a.time;
    let t = if span > 0.0 {
        (time - a.time) / span
    } else {
        0.0
    };

    let from = triple(&a.post, rt);
    let to = triple(&b.pre, rt);
    Some(match a.lerp {
        Lerp::Step => from,
        Lerp::Linear => lerp3(from, to, t),
        Lerp::CatmullRom => {
            let before = triple(&frames[next.saturating_sub(2)].post, rt);
            let after = triple(&frames[(next + 1).min(last)].pre, rt);
            std::array::from_fn(|i| catmull_rom(before[i], from[i], to[i], after[i], t))
        }
        Lerp::Bezier => std::array::from_fn(|i| {
            bezier(a, b, from[i], to[i], i, time).unwrap_or_else(|| from[i] + (to[i] - from[i]) * t)
        }),
    })
}

fn triple(programs: &[Program; 3], rt: &mut Runtime) -> [f32; 3] {
    std::array::from_fn(|i| programs[i].eval(rt))
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((2.0 * p1)
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Blockbench handles are time/value offsets from their key. Time is solved by bisection
/// for determinism.
fn bezier(a: &Keyframe, b: &Keyframe, from: f32, to: f32, axis: usize, time: f32) -> Option<f32> {
    let (ah, bh) = (a.handles.as_ref()?, b.handles.as_ref()?);
    let x = [
        a.time,
        a.time + ah.right_time[axis],
        b.time + bh.left_time[axis],
        b.time,
    ];
    let y = [
        from,
        from + ah.right_value[axis],
        to + bh.left_value[axis],
        to,
    ];

    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..16 {
        let mid = 0.5 * (lo + hi);
        if cubic(&x, mid) < time {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    Some(cubic(&y, 0.5 * (lo + hi)))
}

fn cubic(p: &[f32; 4], s: f32) -> f32 {
    let u = 1.0 - s;
    u * u * u * p[0] + 3.0 * u * u * s * p[1] + 3.0 * u * s * s * p[2] + s * s * s * p[3]
}

pub fn parse(json: &str, interner: &mut Interner) -> Result<Vec<Animation>> {
    let root: Value = serde_json::from_str(json)?;
    let animations = root
        .get("animations")
        .and_then(Value::as_object)
        .ok_or(Error::NoAnimations)?;

    animations
        .iter()
        .map(|(name, body)| parse_animation(name, body, interner))
        .collect()
}

fn parse_animation(name: &str, body: &Value, interner: &mut Interner) -> Result<Animation> {
    let loop_mode = match body.get("loop") {
        Some(Value::Bool(true)) => Loop::Repeat,
        _ => Loop::Once,
    };

    let bones = match body.get("bones").and_then(Value::as_object) {
        Some(map) => map
            .iter()
            .map(|(bone, channels)| parse_bone(bone, channels, interner))
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };

    let explicit = body.get("animation_length").and_then(Value::as_f64);
    let length = explicit.map(|v| v as f32).unwrap_or_else(|| {
        bones
            .iter()
            .flat_map(|b| [&b.rotation, &b.position, &b.scale])
            .filter_map(|c| c.0.last())
            .fold(0.0f32, |acc, k| acc.max(k.time))
    });

    Ok(Animation {
        name: name.to_string(),
        length,
        loop_mode,
        start_delay: scalar(body.get("start_delay"), 0.0, interner)?,
        loop_delay: scalar(body.get("loop_delay"), 0.0, interner)?,
        blend_weight: scalar(body.get("blend_weight"), 1.0, interner)?,
        bones,
    })
}

fn parse_bone(name: &str, channels: &Value, interner: &mut Interner) -> Result<BoneChannels> {
    let mut channel = |key: &str| match channels.get(key) {
        Some(v) => parse_channel(v, interner),
        None => Ok(Channel::default()),
    };

    Ok(BoneChannels {
        name: name.to_string(),
        rotation: channel("rotation")?,
        position: channel("position")?,
        scale: channel("scale")?,
    })
}

fn parse_channel(value: &Value, interner: &mut Interner) -> Result<Channel> {
    if let Some(map) = value.as_object() {
        if is_keyframe_map(map) {
            let mut frames: Vec<Keyframe> = map
                .iter()
                .map(|(time, v)| {
                    let time = time.parse::<f32>().unwrap_or(0.0);
                    parse_keyframe(time, v, interner)
                })
                .collect::<Result<_>>()?;
            frames.sort_by(|a, b| a.time.total_cmp(&b.time));
            return Ok(Channel(frames));
        }
    }

    Ok(Channel(vec![parse_keyframe(0.0, value, interner)?]))
}

fn is_keyframe_map(map: &Map<String, Value>) -> bool {
    !map.is_empty() && map.keys().all(|k| k.parse::<f32>().is_ok())
}

fn parse_keyframe(time: f32, value: &Value, interner: &mut Interner) -> Result<Keyframe> {
    let detailed = value
        .as_object()
        .filter(|m| !is_keyframe_map(m))
        .filter(|m| m.contains_key("pre") || m.contains_key("post") || m.contains_key("lerp_mode"));

    let Some(map) = detailed else {
        let v = vector(value, interner)?;
        return Ok(Keyframe {
            time,
            pre: v.clone(),
            post: v,
            lerp: Lerp::Linear,
            handles: None,
        });
    };

    let pre = match map.get("pre") {
        Some(v) => vector(v, interner)?,
        None => vector(map.get("post").unwrap_or(&Value::Null), interner)?,
    };
    let post = match map.get("post") {
        Some(v) => vector(v, interner)?,
        None => pre.clone(),
    };

    let lerp = match map.get("lerp_mode").and_then(Value::as_str) {
        Some("catmullrom") => Lerp::CatmullRom,
        Some("step") => Lerp::Step,
        Some("bezier") => Lerp::Bezier,
        _ => Lerp::Linear,
    };
    let handles = (lerp == Lerp::Bezier).then(|| {
        Box::new(Handles {
            left_time: numbers(map.get("bezier_left_time"), 0.0),
            left_value: numbers(map.get("bezier_left_value"), 0.0),
            right_time: numbers(map.get("bezier_right_time"), 0.0),
            right_value: numbers(map.get("bezier_right_value"), 0.0),
        })
    });

    Ok(Keyframe {
        time,
        pre,
        post,
        lerp,
        handles,
    })
}

/// A scalar broadcasts to every axis.
fn vector(value: &Value, interner: &mut Interner) -> Result<[Program; 3]> {
    match value {
        Value::Null => Ok(std::array::from_fn(|_| Program::number(0.0))),
        Value::Array(items) if items.len() == 3 => {
            let x = scalar(Some(&items[0]), 0.0, interner)?;
            let y = scalar(Some(&items[1]), 0.0, interner)?;
            let z = scalar(Some(&items[2]), 0.0, interner)?;
            Ok([x, y, z])
        }
        Value::Number(_) | Value::String(_) => {
            let p = scalar(Some(value), 0.0, interner)?;
            Ok([p.clone(), p.clone(), p])
        }
        _ => Err(Error::BadValue(value.to_string())),
    }
}

fn scalar(value: Option<&Value>, default: f32, interner: &mut Interner) -> Result<Program> {
    Ok(match value {
        None | Some(Value::Null) => Program::number(default),
        Some(Value::Number(n)) => Program::number(n.as_f64().unwrap_or(0.0) as f32),
        Some(Value::String(s)) => molang::compile(s, interner)?,
        Some(Value::Bool(b)) => Program::number(if *b { 1.0 } else { 0.0 }),
        Some(other) => return Err(Error::BadValue(other.to_string())),
    })
}

fn numbers(value: Option<&Value>, default: f32) -> [f32; 3] {
    let items = value.and_then(Value::as_array);
    std::array::from_fn(|i| {
        items
            .and_then(|a| a.get(i))
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .unwrap_or(default)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{self, Arms};
    use crate::pose::BonePose;

    fn load(json: &str) -> (Vec<Animation>, Interner) {
        let mut interner = Interner::default();
        let animations = parse(json, &mut interner).expect("parse");
        (animations, interner)
    }

    fn arm(json: &str, time: f32) -> [f32; 3] {
        let (animations, interner) = load(json);
        let model = player::model(Arms::Wide);
        let bind = animations[0].bind(&model);
        let mut rt = Runtime::new(&interner);
        let mut pose = Pose::rest(&model);
        evaluate(&animations[0], &bind, time, 1.0, &mut rt, &mut pose);
        pose.0[model.bone("rightArm").unwrap()].rotation
    }

    const LINEAR: &str = r#"{"animations":{"animation.wave":{"animation_length":2.0,
        "bones":{"rightArm":{"rotation":{"0.0":[0,0,0],"2.0":[90,0,0]}}}}}}"#;

    #[test]
    fn keyframes_interpolate_linearly() {
        assert_eq!(arm(LINEAR, 0.0)[0], 0.0);
        assert_eq!(arm(LINEAR, 1.0)[0], 45.0);
        assert_eq!(arm(LINEAR, 2.0)[0], 90.0);
        assert_eq!(arm(LINEAR, 5.0)[0], 90.0, "clamps past the end");
    }

    #[test]
    fn step_holds_until_the_next_key() {
        let json = r#"{"animations":{"a":{"animation_length":2.0,"bones":{"rightArm":{
            "rotation":{"0.0":{"post":[0,0,0],"lerp_mode":"step"},"2.0":[90,0,0]}}}}}}"#;
        assert_eq!(arm(json, 1.9)[0], 0.0);
        assert_eq!(arm(json, 2.0)[0], 90.0);
    }

    #[test]
    fn pre_and_post_make_a_jump() {
        let json = r#"{"animations":{"a":{"animation_length":2.0,"bones":{"rightArm":{
            "rotation":{"0.0":[0,0,0],"1.0":{"pre":[10,0,0],"post":[90,0,0]},"2.0":[90,0,0]}}}}}}"#;
        assert_eq!(arm(json, 0.5)[0], 5.0, "approaches the pre value");
        assert_eq!(arm(json, 1.5)[0], 90.0, "leaves at the post value");
    }

    #[test]
    fn a_scalar_broadcasts_to_every_axis() {
        let json = r#"{"animations":{"a":{"bones":{"rightArm":{"rotation":30}}}}}"#;
        assert_eq!(arm(json, 0.0), [30.0; 3]);
    }

    #[test]
    fn molang_keyframes_are_evaluated_per_frame() {
        let json = r#"{"animations":{"a":{"bones":{"rightArm":{
            "rotation":["math.sin(query.anim_time * 90) * 20",0,0]}}}}}"#;
        let (animations, interner) = load(json);
        let model = player::model(Arms::Wide);
        let bind = animations[0].bind(&model);
        let slot = interner.query_slot("anim_time").unwrap();
        let mut rt = Runtime::new(&interner);
        let mut pose = Pose::rest(&model);
        let bone = model.bone("rightArm").unwrap();

        let mut sampled = Vec::new();
        for step in 0..3 {
            rt.set_query(slot, step as f32);
            evaluate(&animations[0], &bind, 0.0, 1.0, &mut rt, &mut pose);
            sampled.push(pose.0[bone].rotation[0]);
        }

        assert_eq!(sampled[0], 0.0);
        assert!((sampled[1] - 20.0).abs() < 1e-4, "{sampled:?}");
        assert!(sampled[2].abs() < 1e-4, "{sampled:?}");
    }

    #[test]
    fn looping_wraps_and_a_one_shot_holds() {
        let (looping, interner) =
            load(r#"{"animations":{"a":{"loop":true,"animation_length":2.0,"bones":{}}}}"#);
        let mut rt = Runtime::new(&interner);
        assert_eq!(looping[0].local_time(5.0, &mut rt), Some(1.0));

        let (once, interner) = load(r#"{"animations":{"a":{"animation_length":2.0,"bones":{}}}}"#);
        let mut rt = Runtime::new(&interner);
        assert_eq!(once[0].local_time(1.0, &mut rt), Some(1.0));
        assert_eq!(once[0].local_time(3.0, &mut rt), Some(2.0));

        let (held, interner) = load(
            r#"{"animations":{"a":{"loop":"hold_on_last_frame","animation_length":2.0,"bones":{}}}}"#,
        );
        let mut rt = Runtime::new(&interner);
        assert_eq!(held[0].local_time(9.0, &mut rt), Some(2.0));
    }

    #[test]
    fn bezier_hits_its_endpoints_and_stays_between_them() {
        let json = r#"{"animations":{"a":{"animation_length":1.0,"bones":{"rightArm":{
            "rotation":{
              "0.0":{"post":[0,0,0],"lerp_mode":"bezier",
                     "bezier_right_time":[0.3,0.3,0.3],"bezier_right_value":[0,0,0],
                     "bezier_left_time":[0,0,0],"bezier_left_value":[0,0,0]},
              "1.0":{"pre":[90,0,0],"lerp_mode":"bezier",
                     "bezier_left_time":[-0.3,-0.3,-0.3],"bezier_left_value":[0,0,0],
                     "bezier_right_time":[0,0,0],"bezier_right_value":[0,0,0]}}}}}}}"#;
        assert!(arm(json, 0.0)[0].abs() < 1e-3);
        assert!((arm(json, 1.0)[0] - 90.0).abs() < 1e-3);
        assert!((arm(json, 0.5)[0] - 45.0).abs() < 1e-2, "symmetric handles");
        let quarter = arm(json, 0.25)[0];
        assert!(
            quarter < 22.5,
            "ease-in should lag the straight line, got {quarter}"
        );
    }

    #[test]
    fn catmullrom_differs_from_linear_between_keys() {
        let smooth = r#"{"animations":{"a":{"animation_length":3.0,"bones":{"rightArm":{
            "rotation":{"0.0":[0,0,0],
                        "1.0":{"post":[10,0,0],"lerp_mode":"catmullrom"},
                        "2.0":{"post":[90,0,0],"lerp_mode":"catmullrom"},
                        "3.0":[100,0,0]}}}}}}"#;
        // Catmull-Rom is symmetric, so only an off-centre sample separates it from the chord.
        let value = arm(smooth, 1.25)[0];
        assert!(
            (value - 30.0).abs() > 1.0,
            "should not track the chord: {value}"
        );
        assert!((0.0..100.0).contains(&value), "{value}");
    }

    #[test]
    fn animation_length_defaults_to_the_last_keyframe() {
        let (animations, _) =
            load(r#"{"animations":{"a":{"bones":{"rightArm":{"rotation":{"0.0":0,"4.5":90}}}}}}"#);
        assert_eq!(animations[0].length, 4.5);
    }

    /// Hosts play `animations[0]`, so it must be the file's first.
    #[test]
    fn animations_keep_file_order() {
        let (animations, _) = load(r#"{"animations":{"b":{"bones":{}},"a":{"bones":{}}}}"#);
        let names: Vec<_> = animations.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
    }

    #[test]
    fn unknown_bones_and_extra_sections_are_ignored() {
        let (animations, interner) = load(
            r#"{"animations":{"a":{"animation_length":1.0,
                "sound_effects":{"0.0":{"effect":"step"}},
                "bones":{"tail":{"rotation":[45,0,0]}}}}}"#,
        );
        let model = player::model(Arms::Wide);
        let bind = animations[0].bind(&model);
        assert_eq!(bind, vec![None]);

        let mut pose = Pose::rest(&model);
        let mut rt = Runtime::new(&interner);
        evaluate(&animations[0], &bind, 0.0, 1.0, &mut rt, &mut pose);
        assert!(pose.0.iter().all(|b| *b == BonePose::default()));
    }
}
