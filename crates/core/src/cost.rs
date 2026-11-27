use crate::animation::Animation;
use crate::container::Kind;
use crate::ir::Model;
use crate::issues::{join, path, Issue, IssueCode, Mode, Report};
use crate::limits::{
    BONES_PER_ITEM, BONES_PER_PET, DETAIL_DISTANCE_BLOCKS, DETAIL_NEAREST_WEARERS,
    MINIMAL_ANIMATION_HZ, MINIMAL_DETAIL_BLOCKS, MOLANG_AST_NODES, PARTICLES_PER_COSMETIC,
    PARTICLES_PER_EMITTER, QUADS_PER_NAMEPLATE, QUADS_PER_WEARER, REDUCED_MOTION_SPRING_SCALE,
    SPRING_BONES_PER_ITEM, SPRING_CHAIN_DEPTH, TRAIL_POINTS, WEARER_RECORD_BYTES,
};
use crate::wearer::{
    DeclarationError, RecordLayout, WearerDeclarations, MAX_ONE_SHOTS, MAX_VARIABLE_SLOTS,
};

const ROUNDING_SLACK: f64 = 1e-9;

#[derive(Clone, Debug)]
pub struct Geometry<'a> {
    pub file: String,
    pub at: String,
    pub model: &'a Model,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub path: String,
    pub declarations: WearerDeclarations,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expression {
    pub path: String,
    pub nodes: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Emitter {
    pub path: String,
    pub rate: f64,
    pub longest_life: f64,
    pub bursts: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trail {
    pub path: String,
    pub points: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Springs {
    pub path: String,
    pub bones: u32,
    pub chain_depth: u32,
}

#[derive(Clone, Debug)]
pub struct Content<'a> {
    pub kind: Kind,
    pub path: String,
    pub geometry: Option<Geometry<'a>>,
    pub textures: Vec<Texture>,
    pub record: Option<Record>,
    pub expressions: Vec<Expression>,
    pub emitters: Vec<Emitter>,
    pub trails: Vec<Trail>,
    pub springs: Option<Springs>,
}

impl Content<'_> {
    pub fn new(kind: Kind, path: impl Into<String>) -> Self {
        Content {
            kind,
            path: path.into(),
            geometry: None,
            textures: Vec::new(),
            record: None,
            expressions: Vec::new(),
            emitters: Vec::new(),
            trails: Vec::new(),
            springs: None,
        }
    }
}

pub fn quad_limit(kind: Kind) -> u32 {
    match kind {
        Kind::Nameplate => QUADS_PER_NAMEPLATE,
        _ => QUADS_PER_WEARER,
    }
}

pub fn bone_limit(kind: Kind) -> u32 {
    match kind {
        Kind::Pet => BONES_PER_PET,
        _ => BONES_PER_ITEM,
    }
}

pub fn quads(model: &Model) -> u32 {
    let faces: usize = model
        .bones
        .iter()
        .flat_map(|bone| &bone.cubes)
        .map(|cube| cube.faces.iter().flatten().count())
        .sum();
    saturate(faces)
}

pub fn bones(model: &Model) -> u32 {
    saturate(model.bones.len())
}

pub fn record_bytes(declarations: WearerDeclarations) -> u64 {
    let capped = WearerDeclarations {
        variables: declarations.variables.min(MAX_VARIABLE_SLOTS),
        one_shots: declarations.one_shots.min(MAX_ONE_SHOTS),
        ..declarations
    };
    let empty = layout_bytes(WearerDeclarations::default());
    let per_variable = layout_bytes(WearerDeclarations {
        variables: 1,
        ..WearerDeclarations::default()
    })
    .saturating_sub(empty);
    let per_one_shot = layout_bytes(WearerDeclarations {
        one_shots: 1,
        ..WearerDeclarations::default()
    })
    .saturating_sub(empty);
    let extra_variables = (declarations.variables - capped.variables) as u64;
    let extra_one_shots = (declarations.one_shots - capped.one_shots) as u64;
    layout_bytes(capped)
        .saturating_add(extra_variables.saturating_mul(per_variable))
        .saturating_add(extra_one_shots.saturating_mul(per_one_shot))
}

pub fn live_particles(emitter: &Emitter) -> f64 {
    let steady = emitter.rate * emitter.longest_life;
    let steady = if steady.is_nan() || steady <= 0.0 || emitter.rate <= 0.0 {
        0.0
    } else if steady.is_infinite() {
        steady
    } else {
        libm::ceil(steady * (1.0 - ROUNDING_SLACK))
    };
    let bursts: f64 = emitter.bursts.iter().map(|&size| size as f64).sum();
    steady + bursts
}

pub fn check(content: &Content<'_>, report: &mut Report) {
    if let Some(geometry) = &content.geometry {
        check_geometry(content.kind, geometry, report);
    }
    for texture in &content.textures {
        check_texture(content.kind, texture, report);
    }
    if let Some(record) = &content.record {
        check_record(record, report);
    }
    check_emitters(&content.path, &content.emitters, report);
    for trail in &content.trails {
        check_trail(trail, report);
    }
    if let Some(springs) = &content.springs {
        check_springs(springs, report);
    }
    if report.mode() == Mode::Authoring {
        for expression in &content.expressions {
            check_expression(expression, report);
        }
    }
}

pub fn check_geometry(kind: Kind, geometry: &Geometry<'_>, report: &mut Report) {
    over_limit(
        report,
        IssueCode::QuadsOverBudget,
        path(&geometry.file, &geometry.at),
        quad_limit(kind) as f64,
        quads(geometry.model) as f64,
    );
    over_limit(
        report,
        IssueCode::BonesOverBudget,
        path(&geometry.file, &join(&geometry.at, "bones")),
        bone_limit(kind) as f64,
        bones(geometry.model) as f64,
    );
}

pub fn check_texture(kind: Kind, texture: &Texture, report: &mut Report) {
    let limit = kind.max_texture_size();
    let actual = texture.width.max(texture.height);
    if actual > limit {
        report.push(
            over(
                IssueCode::TextureTooLarge,
                &texture.path,
                limit as f64,
                actual as f64,
            )
            .value("width", texture.width as f64)
            .value("height", texture.height as f64),
        );
    }
    if !texture.width.is_power_of_two() || !texture.height.is_power_of_two() {
        report.push(
            Issue::error(IssueCode::TextureNotPowerOfTwo, &texture.path)
                .value("width", texture.width as f64)
                .value("height", texture.height as f64),
        );
    }
}

pub fn check_record(record: &Record, report: &mut Report) {
    let declarations = record.declarations;
    over_limit(
        report,
        IssueCode::WearerVariablesOverBudget,
        record.path.clone(),
        MAX_VARIABLE_SLOTS as f64,
        declarations.variables as f64,
    );
    over_limit(
        report,
        IssueCode::WearerOneShotsOverBudget,
        record.path.clone(),
        MAX_ONE_SHOTS as f64,
        declarations.one_shots as f64,
    );
    over_limit(
        report,
        IssueCode::WearerRecordOverBudget,
        record.path.clone(),
        WEARER_RECORD_BYTES as f64,
        record_bytes(declarations) as f64,
    );
}

pub fn expressions(animation: &Animation, file: &str) -> Vec<Expression> {
    let base = join("animations", &animation.name);
    animation
        .programs()
        .into_iter()
        .map(|(at, program)| Expression {
            path: path(file, &join(&base, &at)),
            nodes: program.nodes(),
        })
        .collect()
}

pub fn check_expression(expression: &Expression, report: &mut Report) {
    over_limit(
        report,
        IssueCode::MolangNodesOverBudget,
        expression.path.clone(),
        MOLANG_AST_NODES as f64,
        expression.nodes as f64,
    );
}

pub fn check_emitters(path: &str, emitters: &[Emitter], report: &mut Report) {
    let mut total = 0.0;
    for emitter in emitters {
        let live = live_particles(emitter);
        over_limit(
            report,
            IssueCode::EmitterParticlesOverBudget,
            emitter.path.clone(),
            PARTICLES_PER_EMITTER as f64,
            live,
        );
        total += live;
    }
    over_limit(
        report,
        IssueCode::CosmeticParticlesOverBudget,
        path.to_string(),
        PARTICLES_PER_COSMETIC as f64,
        total,
    );
}

pub fn check_trail(trail: &Trail, report: &mut Report) {
    over_limit(
        report,
        IssueCode::TrailPointsOverBudget,
        trail.path.clone(),
        TRAIL_POINTS as f64,
        trail.points as f64,
    );
}

pub fn check_springs(springs: &Springs, report: &mut Report) {
    over_limit(
        report,
        IssueCode::SpringBonesOverBudget,
        springs.path.clone(),
        SPRING_BONES_PER_ITEM as f64,
        springs.bones as f64,
    );
    over_limit(
        report,
        IssueCode::SpringChainTooDeep,
        springs.path.clone(),
        SPRING_CHAIN_DEPTH as f64,
        springs.chain_depth as f64,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimationRate {
    EveryFrame,
    Hz(u32),
    FirstFrame,
}

impl AnimationRate {
    pub fn time(self, seconds: f64) -> f64 {
        match self {
            AnimationRate::EveryFrame => seconds,
            AnimationRate::FirstFrame | AnimationRate::Hz(0) => 0.0,
            AnimationRate::Hz(hz) => libm::floor(seconds * hz as f64) / hz as f64,
        }
    }

    pub fn slower(self, other: AnimationRate) -> AnimationRate {
        match (self, other) {
            (AnimationRate::FirstFrame, _) | (_, AnimationRate::FirstFrame) => {
                AnimationRate::FirstFrame
            }
            (AnimationRate::Hz(a), AnimationRate::Hz(b)) => AnimationRate::Hz(a.min(b)),
            (AnimationRate::Hz(hz), AnimationRate::EveryFrame)
            | (AnimationRate::EveryFrame, AnimationRate::Hz(hz)) => AnimationRate::Hz(hz),
            (AnimationRate::EveryFrame, AnimationRate::EveryFrame) => AnimationRate::EveryFrame,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Features {
    pub animation: AnimationRate,
    pub palette_animation: bool,
    pub effects: bool,
    pub particles: bool,
    pub trails: bool,
    pub shakes: bool,
    pub flashes: bool,
    pub letter_motion: bool,
    pub nameplate_paths: bool,
    pub spring_scale: f32,
}

impl Features {
    pub const ALL: Features = Features {
        animation: AnimationRate::EveryFrame,
        palette_animation: true,
        effects: true,
        particles: true,
        trails: true,
        shakes: true,
        flashes: true,
        letter_motion: true,
        nameplate_paths: true,
        spring_scale: 1.0,
    };

    pub fn springs(self) -> bool {
        self.spring_scale > 0.0
    }

    pub fn combine(self, other: Features) -> Features {
        Features {
            animation: self.animation.slower(other.animation),
            palette_animation: self.palette_animation && other.palette_animation,
            effects: self.effects && other.effects,
            particles: self.particles && other.particles,
            trails: self.trails && other.trails,
            shakes: self.shakes && other.shakes,
            flashes: self.flashes && other.flashes,
            letter_motion: self.letter_motion && other.letter_motion,
            nameplate_paths: self.nameplate_paths && other.nameplate_paths,
            spring_scale: self.spring_scale.min(other.spring_scale),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Detail {
    Full = 0,
    Reduced = 1,
    Minimal = 2,
}

impl Detail {
    pub const ALL: [Detail; 3] = [Detail::Full, Detail::Reduced, Detail::Minimal];

    pub const fn value(self) -> u8 {
        self as u8
    }

    pub fn from_value(value: u8) -> Option<Detail> {
        Detail::ALL.get(value as usize).copied()
    }

    pub fn features(self) -> Features {
        match self {
            Detail::Full => Features::ALL,
            Detail::Reduced => Features {
                particles: false,
                trails: false,
                nameplate_paths: false,
                spring_scale: 0.0,
                ..Features::ALL
            },
            Detail::Minimal => Features {
                animation: AnimationRate::Hz(MINIMAL_ANIMATION_HZ),
                effects: false,
                ..Detail::Reduced.features()
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Motion {
    #[default]
    Full = 0,
    Reduced = 1,
    Off = 2,
}

impl Motion {
    pub const ALL: [Motion; 3] = [Motion::Full, Motion::Reduced, Motion::Off];

    pub const fn value(self) -> u8 {
        self as u8
    }

    pub fn from_value(value: u8) -> Option<Motion> {
        Motion::ALL.get(value as usize).copied()
    }

    pub fn features(self) -> Features {
        match self {
            Motion::Full => Features::ALL,
            Motion::Reduced => Features {
                particles: false,
                trails: false,
                shakes: false,
                flashes: false,
                letter_motion: false,
                spring_scale: REDUCED_MOTION_SPRING_SCALE,
                ..Features::ALL
            },
            Motion::Off => Features {
                animation: AnimationRate::FirstFrame,
                palette_animation: false,
                effects: false,
                spring_scale: 0.0,
                ..Motion::Reduced.features()
            },
        }
    }
}

pub fn features(detail: Detail, motion: Motion) -> Features {
    detail.features().combine(motion.features())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailSettings {
    pub distance: f64,
    pub nearest: u32,
}

impl Default for DetailSettings {
    fn default() -> Self {
        DetailSettings {
            distance: DETAIL_DISTANCE_BLOCKS,
            nearest: DETAIL_NEAREST_WEARERS,
        }
    }
}

pub fn assign_detail(
    distances: &[f64],
    local: Option<usize>,
    settings: DetailSettings,
    over_budget: bool,
) -> Vec<Detail> {
    let distance = |i: usize| {
        let d = distances[i];
        if d.is_nan() {
            f64::INFINITY
        } else {
            d
        }
    };
    let mut levels = vec![Detail::Full; distances.len()];
    let mut others: Vec<usize> = (0..distances.len()).filter(|&i| Some(i) != local).collect();
    others.sort_by(|&a, &b| distance(a).total_cmp(&distance(b)));

    let mut full = 0;
    for i in others {
        let d = distance(i);
        levels[i] = if over_budget || d > MINIMAL_DETAIL_BLOCKS {
            Detail::Minimal
        } else if d <= settings.distance && full < settings.nearest {
            full += 1;
            Detail::Full
        } else {
            Detail::Reduced
        };
    }
    levels
}

fn saturate(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn layout_bytes(declarations: WearerDeclarations) -> u64 {
    match RecordLayout::new(declarations) {
        Ok(layout) => layout.bytes() as u64,
        Err(DeclarationError::OverBudget { bytes, .. }) => bytes as u64,
        Err(_) => u64::MAX,
    }
}

fn over(code: IssueCode, at: &str, limit: f64, actual: f64) -> Issue {
    Issue::error(code, at)
        .value("limit", limit)
        .value("actual", actual)
}

fn over_limit(report: &mut Report, code: IssueCode, at: String, limit: f64, actual: f64) {
    if actual > limit {
        report.push(over(code, &at, limit, actual));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Bone, Cube, FaceUv};
    use crate::issues::Level;
    use crate::wearer::RECORD_HEADER_BYTES;

    fn cube(faces: usize) -> Cube {
        let mut all = [None; 6];
        for face in all.iter_mut().take(faces) {
            *face = Some(FaceUv {
                uv: [0.0, 0.0],
                size: [1.0, 1.0],
            });
        }
        Cube {
            origin: [0.0; 3],
            size: [1.0; 3],
            inflate: 0.0,
            rotation: [0.0; 3],
            pivot: [0.0; 3],
            faces: all,
            box_uv: false,
            mirror: false,
        }
    }

    fn model(bone_count: usize, quad_count: usize) -> Model {
        let mut bones: Vec<Bone> = (0..bone_count.max(1))
            .map(|i| Bone {
                name: format!("b{i}"),
                parent: None,
                parent_name: None,
                pivot: [0.0; 3],
                rotation: [0.0; 3],
                cubes: Vec::new(),
                glow: None,
                locators: Vec::new(),
                spring: None,
            })
            .collect();
        bones.truncate(bone_count);
        let mut left = quad_count;
        while left > 0 {
            let n = left.min(6);
            bones[0].cubes.push(cube(n));
            left -= n;
        }
        Model {
            identifier: "geometry.test".to_string(),
            texture_size: [64.0, 64.0],
            bones,
        }
    }

    fn geometry(model: &Model) -> Geometry<'_> {
        Geometry {
            file: "geo.json".to_string(),
            at: "minecraft:geometry.0".to_string(),
            model,
        }
    }

    fn codes(report: &Report) -> Vec<IssueCode> {
        report.issues().iter().map(|i| i.code).collect()
    }

    fn value(issue: &Issue, key: &str) -> Option<f64> {
        issue.values.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }

    fn authoring() -> Report {
        Report::new(Mode::Authoring)
    }

    fn emitter(rate: f64, longest_life: f64, bursts: &[u32]) -> Emitter {
        Emitter {
            path: "effects/spark.json#emitters.0".to_string(),
            rate,
            longest_life,
            bursts: bursts.to_vec(),
        }
    }

    fn declarations_of(bytes: usize) -> WearerDeclarations {
        WearerDeclarations {
            effect_floats: (bytes - RECORD_HEADER_BYTES) / 4,
            ..WearerDeclarations::default()
        }
    }

    fn everything_over(m: &Model) -> Content<'_> {
        let mut content = Content::new(Kind::Item, "manifest.json");
        content.geometry = Some(geometry(m));
        content.textures = vec![Texture {
            path: "textures/a.png".to_string(),
            width: 2048,
            height: 1000,
        }];
        content.record = Some(Record {
            path: "manifest.json#wearer".to_string(),
            declarations: declarations_of(2564),
        });
        content.expressions = vec![Expression {
            path: "animations.json#animations.idle.blend_weight".to_string(),
            nodes: 300,
        }];
        content.emitters = vec![emitter(100.0, 1.0, &[]), emitter(100.0, 1.0, &[])];
        content.trails = vec![Trail {
            path: "effects/trail.json#trails.0".to_string(),
            points: 100,
        }];
        content.springs = Some(Springs {
            path: "springs.json#springs".to_string(),
            bones: 20,
            chain_depth: 10,
        });
        content
    }

    #[test]
    fn cost_check_reports_every_issue_in_one_pass() {
        let m = model(65, 2001);
        let content = everything_over(&m);
        let mut report = authoring();
        check(&content, &mut report);
        assert_eq!(
            codes(&report),
            vec![
                IssueCode::QuadsOverBudget,
                IssueCode::BonesOverBudget,
                IssueCode::TextureTooLarge,
                IssueCode::TextureNotPowerOfTwo,
                IssueCode::WearerRecordOverBudget,
                IssueCode::EmitterParticlesOverBudget,
                IssueCode::EmitterParticlesOverBudget,
                IssueCode::CosmeticParticlesOverBudget,
                IssueCode::TrailPointsOverBudget,
                IssueCode::SpringBonesOverBudget,
                IssueCode::SpringChainTooDeep,
                IssueCode::MolangNodesOverBudget,
            ]
        );
        assert!(report.issues().iter().all(|i| i.level == Level::Error));
        assert!(report
            .issues()
            .iter()
            .filter(|i| i.code != IssueCode::TextureNotPowerOfTwo)
            .all(|i| value(i, "limit").is_some() && value(i, "actual").is_some()));
    }

    #[test]
    fn cost_detail_levels_have_fixed_values() {
        assert_eq!(Detail::Full.value(), 0);
        assert_eq!(Detail::Reduced.value(), 1);
        assert_eq!(Detail::Minimal.value(), 2);
        for level in Detail::ALL {
            assert_eq!(Detail::from_value(level.value()), Some(level));
        }
        assert_eq!(Detail::from_value(3), None);
    }

    #[test]
    fn cost_detail_assignment_gives_full_to_the_nearest_n_within_distance() {
        let settings = DetailSettings {
            distance: 24.0,
            nearest: 2,
        };
        let distances = [10.0, 0.0, 3.0, 5.0, 24.0, 24.5, 64.0, 64.5, f64::NAN];
        let levels = assign_detail(&distances, Some(1), settings, false);
        assert_eq!(
            levels,
            vec![
                Detail::Reduced,
                Detail::Full,
                Detail::Full,
                Detail::Full,
                Detail::Reduced,
                Detail::Reduced,
                Detail::Reduced,
                Detail::Minimal,
                Detail::Minimal,
            ]
        );

        let ties = assign_detail(&[4.0, 4.0, 4.0], None, settings, false);
        assert_eq!(ties, vec![Detail::Full, Detail::Full, Detail::Reduced]);
    }
}
