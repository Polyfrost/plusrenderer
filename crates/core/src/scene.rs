//! A player and its cosmetics: numbered slots (slot number = texture id), clocks and sheets.
//! Each frame: `pose`, then `build` (2D, back-to-front) or `build_mesh` (3D quads for the game).

use crate::animation::{self, Animation};
use crate::build::{Buffers, Camera, Instance, Light};
use crate::cosmetic::{self, BodySlot};
use crate::java;
use crate::math::Affine;
use crate::mesh::Mesh;
use crate::molang::{Interner, Runtime};
use crate::player::{self, Arms};
use crate::pose::{world_transforms, BonePose, Pose};
use crate::Model;

/// As the in-game client: a cosmetic's sheet advances every 4 ticks.
const COSMETIC_FRAME_SECONDS: f32 = 0.2;

struct Slot {
    model: Option<Model>,
    visible: bool,
    cull: bool,
    /// Only the file's first animation plays, as in game.
    animation: Option<Animation>,
    bind: Vec<Option<usize>>,
    /// Per slot, so one cosmetic's `variable.` writes can't leak into another's.
    interner: Interner,
    runtime: Runtime,
    anim_time: Option<u16>,
    life_time: Option<u16>,
    /// Scene time the animation started at. `None` holds the rest pose.
    started: Option<f32>,
    pose: Pose,
    transforms: Vec<Affine>,
    frames: u32,
    frame_seconds: f32,
    scale: f32,
    /// The last `pose` was mid-animation, so the next one may differ.
    moving: bool,
}

impl Slot {
    fn new() -> Slot {
        let interner = Interner::default();
        let runtime = Runtime::new(&interner);

        Slot {
            model: None,
            visible: true,
            cull: true,
            animation: None,
            bind: Vec::new(),
            interner,
            runtime,
            anim_time: None,
            life_time: None,
            started: None,
            pose: Pose(Vec::new()),
            transforms: Vec::new(),
            frames: 1,
            frame_seconds: COSMETIC_FRAME_SECONDS,
            scale: 1.0,
            moving: false,
        }
    }

    fn set_model(&mut self, model: Model, cull: bool, frames: u32, frame_seconds: f32) {
        self.pose = Pose::rest(&model);
        self.bind = self
            .animation
            .as_ref()
            .map(|a| a.bind(&model))
            .unwrap_or_default();

        self.model = Some(model);
        self.cull = cull;
        self.frames = frames.max(1);
        self.frame_seconds = frame_seconds;
    }

    fn animate(&mut self, seconds: f32, queries: &[(String, f32)]) {
        if let Some(slot) = self.life_time {
            self.runtime.set_query(slot, seconds);
        }

        for (name, value) in queries {
            if let Some(slot) = self.interner.query_slot(name) {
                self.runtime.set_query(slot, *value);
            }
        }

        let time = (self.animation.as_ref().zip(self.started))
            .and_then(|(a, started)| a.local_time(seconds - started, &mut self.runtime));
        let (Some(animation), Some(time)) = (&self.animation, time) else {
            self.pose.reset();
            // Played but still in its start delay.
            self.moving = self.animation.is_some() && self.started.is_some();
            return;
        };

        self.moving = !animation.holds_at(time);
        if let Some(slot) = self.anim_time {
            self.runtime.set_query(slot, time);
        }

        let weight = animation.weight(&mut self.runtime);
        animation::evaluate(
            animation,
            &self.bind,
            time,
            weight,
            &mut self.runtime,
            &mut self.pose,
        );
    }

    fn frame(&self, seconds: f32) -> u32 {
        let elapsed = seconds - self.started.unwrap_or(0.0);
        (elapsed.max(0.0) / self.frame_seconds) as u32 % self.frames
    }
}

#[derive(Default)]
pub struct Scene {
    slots: Vec<Slot>,
    /// The game's posed player parts. Cosmetics follow these ahead of any player slot.
    game_parts: Vec<GamePart>,
    queries: Vec<(String, f32)>,
    seconds: f32,
    pub light: Light,
    pub buffers: Buffers,
    pub mesh: Mesh,
}

impl Scene {
    fn slot(&mut self, index: usize) -> &mut Slot {
        if self.slots.len() <= index {
            self.slots.resize_with(index + 1, Slot::new);
        }
        &mut self.slots[index]
    }

    pub fn load_player(&mut self, slot: usize, arms: Arms) {
        self.slot(slot)
            .set_model(player::model(arms), true, 1, COSMETIC_FRAME_SECONDS);
    }

    /// Prepared as the in-game client does (`cosmetic::prepare`), drawn two-sided.
    pub fn load_cosmetic(
        &mut self,
        slot: usize,
        mut model: Model,
        body: BodySlot,
        anchor: Option<&str>,
        image_size: Option<[u32; 2]>,
    ) {
        let frames = cosmetic::prepare(&mut model, body, anchor, image_size);
        self.slot(slot)
            .set_model(model, false, frames, COSMETIC_FRAME_SECONDS);
    }

    /// Multi-frame textures come from `bundle::cape_sheet`.
    pub fn load_cape(&mut self, slot: usize, frames: u32, frame_seconds: f32) {
        self.slot(slot)
            .set_model(player::cape(), true, frames, frame_seconds);
    }

    /// Uses the file's first animation, which stays at rest until `play`.
    pub fn load_animation(&mut self, slot: usize, json: &str) -> Result<(), animation::Error> {
        let s = self.slot(slot);
        s.animation = animation::parse(json, &mut s.interner)?.into_iter().next();
        s.runtime = Runtime::new(&s.interner);
        s.anim_time = s.interner.query_slot("anim_time");
        s.life_time = s.interner.query_slot("life_time");
        s.started = None;

        if let Some(model) = &s.model {
            s.bind = s
                .animation
                .as_ref()
                .map(|a| a.bind(model))
                .unwrap_or_default();
        }

        Ok(())
    }

    pub fn play(&mut self, slot: usize, at: f32) {
        self.slot(slot).started = Some(at);
    }

    pub fn stop(&mut self, slot: usize) {
        self.slot(slot).started = None;
    }

    /// About the bone it hangs from, as the in-game client scales a shoulder pet.
    pub fn set_scale(&mut self, slot: usize, scale: f32) {
        self.slot(slot).scale = scale;
    }

    pub fn set_visible(&mut self, slot: usize, visible: bool) {
        self.slot(slot).visible = visible;
    }

    pub fn clear(&mut self, slot: usize) {
        if let Some(s) = self.slots.get_mut(slot) {
            *s = Slot::new();
        }
    }

    /// Applies to every slot from the next `pose` and overrides the core's `life_time`.
    pub fn set_query(&mut self, name: &str, value: f32) {
        match self.queries.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = value,
            None => self.queries.push((name.to_string(), value)),
        }
    }

    /// Hidden slots too: a hidden player can still drive an emote and its cosmetics.
    pub fn pose(&mut self, seconds: f32) {
        self.seconds = seconds;
        for slot in self.slots.iter_mut().filter(|s| s.model.is_some()) {
            slot.animate(seconds, &self.queries);
        }
    }

    /// Whether a later `pose` could differ from the last one.
    /// Hosts can skip frames while it is false.
    /// Host-side changes (camera, `set_query`, loads) are the host's to redraw for.
    pub fn is_animating(&self) -> bool {
        self.slots
            .iter()
            .any(|s| s.model.is_some() && (s.moving || s.frames > 1))
    }

    /// `bone` is a core name (`head`, `rightArm`, ..., `root`).
    /// Cosmetics then follow the game's limbs.
    pub fn set_game_part(&mut self, bone: &str, rest: [f32; 3], part: &java::Part) {
        let posed = GamePart {
            bone: bone.to_string(),
            transform: java::part_transform(rest, part),
            pivot: java::rest_pivot(rest),
        };

        match self.game_parts.iter_mut().find(|p| p.bone == bone) {
            Some(entry) => *entry = posed,
            None => self.game_parts.push(posed),
        }
    }

    pub fn clear_game_parts(&mut self) {
        self.game_parts.clear();
    }

    /// A game host adds `java::part_delta` of this to the vanilla part to play the emote.
    pub fn bone_pose(&self, slot: usize, bone: &str) -> Option<BonePose> {
        let s = self.slots.get(slot)?;
        s.pose.0.get(s.model.as_ref()?.bone(bone)?).copied()
    }

    pub fn build(&mut self, camera: &Camera, viewport: [f32; 2]) -> &Buffers {
        self.transform();
        let instances = instances(&self.slots, self.seconds);
        self.buffers
            .build(&instances, camera, &self.light, viewport);
        &self.buffers
    }

    /// `to_host` is `java::MODEL` for the game.
    pub fn build_mesh(&mut self, to_host: &Affine) -> &Mesh {
        self.transform();
        let instances = instances(&self.slots, self.seconds);
        self.mesh.build(&instances, to_host);
        &self.mesh
    }

    fn transform(&mut self) {
        let game = &self.game_parts;

        for i in 0..self.slots.len() {
            // Lower slots are done, so a cosmetic bone parented to `head` follows the player's.
            let (lower, rest) = self.slots.split_at_mut(i);
            let slot = &mut rest[0];
            let Some(model) = &slot.model else {
                continue;
            };

            let scale = Affine::scale([slot.scale; 3]);
            world_transforms(
                model,
                Some(&slot.pose),
                |name| {
                    let (host, pivot) = game_part(game, name).or_else(|| slot_bone(lower, name))?;
                    Some(host.mul(&Affine::about(pivot, &scale)))
                },
                &mut slot.transforms,
            );
        }
    }
}

fn instances(slots: &[Slot], seconds: f32) -> Vec<Instance<'_>> {
    slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.visible)
        .filter_map(|(i, s)| {
            let mut instance = Instance::new(s.model.as_ref()?, &s.transforms, i as u32);
            instance.cull = s.cull;
            instance.frames = s.frames;
            instance.frame = s.frame(seconds);
            Some(instance)
        })
        .collect()
}

struct GamePart {
    bone: String,
    transform: Affine,
    pivot: [f32; 3],
}

/// Composed down vanilla's chain (`root`, `head` for `hat`, the part). Parts are parent-relative.
fn game_part(parts: &[GamePart], name: &str) -> Option<(Affine, [f32; 3])> {
    let own = parts.iter().find(|p| p.bone == name)?;
    let parent = match name {
        "root" => None,
        "hat" => game_part(parts, "head").map(|(tf, _)| tf),
        _ => game_part(parts, "root").map(|(tf, _)| tf),
    };

    Some((
        parent.map_or(own.transform, |p| p.mul(&own.transform)),
        own.pivot,
    ))
}

fn slot_bone(lower: &[Slot], name: &str) -> Option<(Affine, [f32; 3])> {
    lower.iter().rev().find_map(|s| {
        let model = s.model.as_ref()?;
        let i = model.bone(name)?;
        Some((*s.transforms.get(i)?, model.bones[i].pivot))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAT: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.hat","texture_width":16,"texture_height":16},
        "bones":[{"name":"brim","pivot":[0,24,0],
          "cubes":[{"origin":[-5,32,-5],"size":[10,1,10],"uv":[0,0]}]}]}]}"#;

    const NOD: &str = r#"{"animations":{"nod":{"loop":true,"animation_length":2,
        "bones":{"head":{"rotation":{"0.0":[0,0,0],"1.0":[30,0,0],"2.0":[0,0,0]}}}}}}"#;

    fn draw(scene: &mut Scene, seconds: f32) -> &Buffers {
        scene.pose(seconds);
        scene.build(
            &Camera::orbit(0.0, 10.0, 62.0, [0.0, 18.0, 0.0]),
            [512.0, 512.0],
        )
    }

    fn scene_with_hat() -> Scene {
        let mut scene = Scene::default();
        scene.load_player(0, Arms::Wide);
        scene.load_animation(0, NOD).unwrap();
        let hat = crate::bedrock::parse_one(HAT).unwrap();
        scene.load_cosmetic(1, hat, BodySlot::Hat, None, None);
        scene
    }

    fn head(scene: &Scene) -> Affine {
        let player = &scene.slots[0];
        player.transforms[player.model.as_ref().unwrap().bone("head").unwrap()]
    }

    #[test]
    fn cosmetics_follow_the_bone_their_slot_names() {
        let mut scene = scene_with_hat();
        scene.play(0, 0.0);
        draw(&mut scene, 1.0);
        assert_ne!(head(&scene), Affine::IDENTITY, "the nod must move it");
        assert_eq!(scene.slots[1].transforms[0], head(&scene));
    }

    #[test]
    fn slots_share_one_pass_and_keep_their_textures() {
        let mut scene = scene_with_hat();
        let runs = &draw(&mut scene, 0.0).runs;
        assert!(runs.iter().any(|r| r.texture == 0) && runs.iter().any(|r| r.texture == 1));

        scene.set_visible(1, false);
        let runs = &draw(&mut scene, 0.0).runs;
        assert!(runs.iter().all(|r| r.texture == 0));
    }

    #[test]
    fn animations_count_from_their_own_start() {
        let mut scene = scene_with_hat();
        draw(&mut scene, 7.0);
        assert_eq!(head(&scene), Affine::IDENTITY, "rest until played");

        scene.play(0, 7.0);
        draw(&mut scene, 7.0);
        assert_eq!(head(&scene), Affine::IDENTITY, "frame 0 of the nod is rest");
        draw(&mut scene, 8.0);
        let peak = head(&scene);

        scene.stop(0);
        draw(&mut scene, 8.0);
        assert_eq!(head(&scene), Affine::IDENTITY);

        scene.play(0, 100.0);
        draw(&mut scene, 101.0);
        assert_eq!(
            head(&scene),
            peak,
            "same pose one second in, whenever it started"
        );
    }

    #[test]
    fn sheets_step_through_their_frames() {
        let mut scene = Scene::default();
        scene.load_cape(0, 16, 0.1);
        scene.play(0, 0.0);

        let top_v = |scene: &mut Scene, t: f32| {
            scene.pose(t);
            let b = scene.build(
                &Camera::orbit(180.0, 0.0, 62.0, [0.0, 18.0, 0.0]),
                [256.0, 256.0],
            );
            b.uvs.chunks(2).map(|uv| uv[1]).fold(f32::MAX, f32::min)
        };

        let first = top_v(&mut scene, 0.0);
        assert!(
            (top_v(&mut scene, 0.15) - first - 1.0 / 16.0).abs() < 1e-6,
            "one 100 ms frame down"
        );
        assert_eq!(top_v(&mut scene, 1.6), first, "16 frames wrap");
    }

    #[test]
    fn cosmetics_follow_the_games_parts_when_given() {
        let part = |offset, rotation| java::Part {
            offset,
            rotation,
            scale: [1.0; 3],
        };
        let crouch = part([0.0, 3.2, 0.0], [0.0; 3]);
        let nod = part([0.0; 3], [0.5, 0.0, 0.0]);

        let mut scene = scene_with_hat();
        scene.set_game_part("root", [0.0; 3], &crouch);
        scene.set_game_part("head", [0.0; 3], &nod);
        draw(&mut scene, 0.0);

        let tf = |p| java::part_transform([0.0; 3], p);
        assert_eq!(scene.slots[1].transforms[0], tf(&crouch).mul(&tf(&nod)));

        scene.clear_game_parts();
        draw(&mut scene, 0.0);
        assert_eq!(
            scene.slots[1].transforms[0],
            Affine::IDENTITY,
            "back on the slot's rest pose"
        );
    }

    #[test]
    fn scale_is_about_the_attach_bones_pivot() {
        let mut scene = scene_with_hat();
        scene.set_scale(1, 2.0);
        draw(&mut scene, 0.0);

        let head_pivot = [0.0, 24.0, 0.0];
        assert_eq!(scene.slots[1].transforms[0].point(head_pivot), head_pivot);
        assert_eq!(
            scene.slots[1].transforms[0].point([0.0, 25.0, 0.0]),
            [0.0, 26.0, 0.0]
        );
    }

    #[test]
    fn host_queries_reach_every_slot() {
        let sway = r#"{"animations":{"sway":{"loop":true,"bones":{"brim":{"rotation":["q.is_moving * 15",0,0]}}}}}"#;
        let mut scene = scene_with_hat();
        scene.load_animation(1, sway).unwrap();
        scene.play(1, 0.0);

        scene.pose(0.0);
        assert_eq!(scene.bone_pose(1, "brim").unwrap().rotation[0], 0.0);

        scene.set_query("is_moving", 1.0);
        scene.pose(0.0);
        assert_eq!(scene.bone_pose(1, "brim").unwrap().rotation[0], 15.0);
    }

    #[test]
    fn a_hidden_player_still_plays_its_emote() {
        let mut scene = scene_with_hat();
        scene.set_visible(0, false);
        scene.play(0, 0.0);
        draw(&mut scene, 1.0);

        assert_eq!(
            scene.bone_pose(0, "head").unwrap().rotation,
            [30.0, 0.0, 0.0]
        );
        assert_eq!(
            scene.slots[1].transforms[0],
            head(&scene),
            "its cosmetics still follow"
        );
        assert!(
            draw(&mut scene, 1.0).runs.iter().all(|r| r.texture == 1),
            "but it is not drawn"
        );
    }

    #[test]
    fn only_running_animations_and_sheets_need_frames() {
        let mut scene = scene_with_hat();
        scene.pose(0.0);
        assert!(!scene.is_animating(), "rest pose");
        scene.play(0, 0.0);
        scene.pose(1.0);
        assert!(scene.is_animating(), "looping nod");
        scene.stop(0);
        scene.pose(1.0);
        assert!(!scene.is_animating());

        let once = NOD.replace(r#""loop":true"#, r#""loop":false,"start_delay":1"#);
        scene.load_animation(0, &once).unwrap();
        scene.play(0, 0.0);
        scene.pose(0.5);
        assert!(scene.is_animating(), "waiting out its delay");
        scene.pose(2.0);
        assert!(scene.is_animating());
        scene.pose(3.0);
        assert!(!scene.is_animating(), "holding its last frame");

        scene.load_cape(2, 16, 0.1);
        assert!(scene.is_animating(), "sheets always step");
    }

    #[test]
    fn meshes_carry_each_slots_culling() {
        let mut scene = scene_with_hat();
        scene.pose(0.0);
        let runs = &scene.build_mesh(&java::MODEL).runs;
        let cull: Vec<(u32, u32)> = runs.iter().map(|r| (r.texture, r.cull)).collect();
        assert_eq!(cull, [(0, 1), (1, 0)], "player culled, cosmetic two-sided");
    }
}
