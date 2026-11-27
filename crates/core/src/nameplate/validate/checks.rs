use crate::issues::{path, Issue, IssueCode, Report};

use super::super::flag::fits;
use super::super::geom::Rect;
use super::super::layout::{body_rect, build_block, reference_inputs, scaled_corners};
use super::super::material::cpu::loops;
use super::super::material::glsl::unsupported;
use super::super::material::ir::{
    Dialect, EdgeLane, Lane, MaterialFrame, MaterialIr, LANE_COUNT, MAX_SAMPLES,
};
use super::super::palette::{banding_issues, base_frame};
use super::super::types::{Fill, LayerContent, NameplateAsset, Of, PaletteFrame};
use super::sim::{breakpoint_inputs, distinct_variants, Stage};
use super::FLAGS;

fn lanes(
    ir: &MaterialIr,
    stage: &Stage<'_>,
    params: &[f64],
    edge: [f64; 5],
    frame: i64,
) -> [f64; LANE_COUNT] {
    let mut out = [0.0; LANE_COUNT];
    for (lane, value) in ir.lanes.iter().zip(out.iter_mut()) {
        *value = match *lane {
            Lane::Param(index) => params.get(usize::from(index)).copied().unwrap_or(0.0),
            Lane::Expr(program) => {
                let raw = stage.eval.raw(program, Some(frame));
                if raw.is_finite() {
                    raw
                } else {
                    0.0
                }
            }
            Lane::BodyEdge(which) => {
                edge[match which {
                    EdgeLane::TopLeft => 0,
                    EdgeLane::TopRight => 1,
                    EdgeLane::BottomRight => 2,
                    EdgeLane::BottomLeft => 3,
                    EdgeLane::Rings => 4,
                }]
            }
        };
    }
    out
}

fn edge(asset: &NameplateAsset, of: Of, body: Rect) -> [f64; 5] {
    let Of::Layer(target) = of else {
        return [0.0; 5];
    };
    let Some(LayerContent::Body(target)) =
        asset.layers.get(usize::from(target)).map(|l| &l.content)
    else {
        return [0.0; 5];
    };
    let rings = target.rings.len() as f64;
    if target.art.is_some() {
        return [0.0, 0.0, 0.0, 0.0, rings];
    }
    let [tl, tr, br, bl] = scaled_corners(target.corners, body).map(f64::from);
    [tl, tr, br, bl, rings]
}

type ParamSet = (Vec<f64>, Option<(usize, bool)>);

fn param_sets(asset: &NameplateAsset, ir: &MaterialIr, base: &[f64]) -> Vec<ParamSet> {
    let mut sets = vec![(base.to_vec(), None)];
    let mut used: Vec<usize> = ir
        .lanes
        .iter()
        .filter_map(|l| match l {
            Lane::Param(i) => Some(usize::from(*i)),
            _ => None,
        })
        .collect();
    used.sort_unstable();
    used.dedup();
    for index in used {
        let Some(param) = asset.params.get(index) else {
            continue;
        };
        for (value, high) in [(param.min, false), (param.max, true)] {
            let mut set = base.to_vec();
            if let Some(slot) = set.get_mut(index) {
                *slot = value;
            }
            sets.push((set, Some((index, high))));
        }
    }
    sets
}

pub fn material_loops(asset: &NameplateAsset, report: &mut Report) {
    let mut inputs = reference_inputs();
    inputs.extend(breakpoint_inputs(asset));
    let mut failed = vec![false; asset.layers.len()];
    for variant in distinct_variants(asset) {
        for input in &inputs {
            let Some(stage) = Stage::new(asset, variant, input) else {
                continue;
            };
            let mut palette = PaletteFrame::default();
            base_frame(stage.variant, &mut palette);
            for (layer, spec) in asset.layers.iter().enumerate() {
                let LayerContent::Material(content) = &spec.content else {
                    continue;
                };
                if content.drift || failed[layer] {
                    continue;
                }
                let Some(ir) = asset.materials.get(usize::from(content.material)) else {
                    continue;
                };
                let body = match spec.of {
                    Of::Block => stage.placement.block.background,
                    _ => stage.layout.targets.get(layer).copied().unwrap_or_default(),
                };
                let step = i64::from(spec.step.max(1));
                let cycle = (i64::from(asset.loop_frames) / step).max(1);
                let frame = MaterialFrame {
                    area: ir.area(body),
                    body,
                    clock: [
                        0,
                        i32::try_from(cycle).unwrap_or(1),
                        i32::from(asset.fps),
                        i32::try_from(step).unwrap_or(1),
                    ],
                    lanes: [0.0; LANE_COUNT],
                    tones: &palette.tones,
                    atlas: [[0.0; 4]; MAX_SAMPLES],
                };
                let edges = edge(asset, spec.of, body);
                for (params, which) in param_sets(asset, ir, &stage.eval.params) {
                    let mut at =
                        |state: i32| lanes(ir, &stage, &params, edges, i64::from(state) * step);
                    if loops(ir, &frame, &asset.textures, &mut at) {
                        continue;
                    }
                    failed[layer] = true;
                    let mut issue = Issue::error(
                        IssueCode::MaterialLoop,
                        path(&format!("layers/{}.json", spec.id), "material"),
                    )
                    .value("cycle", cycle as f64)
                    .value("name_width", f64::from(stage.placement.block.name_width))
                    .value("lines", f64::from(stage.placement.block.line_count))
                    .name("variant", &stage.variant.key);
                    if let Some((index, high)) = which {
                        issue = issue
                            .name("param", &asset.params[index].id)
                            .name("at", if high { "max" } else { "min" })
                            .value("value", params[index]);
                    }
                    report.push(issue);
                    break;
                }
            }
        }
    }
}

pub fn flags(asset: &NameplateAsset, report: &mut Report) {
    if asset.flags.is_empty() {
        return;
    }
    let mut inputs = reference_inputs();
    inputs.extend(breakpoint_inputs(asset));
    let mut reported: Vec<(usize, usize)> = Vec::new();
    for (v, variant) in asset.variants.iter().enumerate() {
        let flag_index = usize::from(variant.flag.unwrap_or(0));
        let Some(flag) = asset.flags.get(flag_index) else {
            continue;
        };
        for input in &inputs {
            let Some(block) = build_block(input) else {
                continue;
            };
            for (layer, spec) in asset.layers.iter().enumerate() {
                let LayerContent::Body(body) = &spec.content else {
                    continue;
                };
                if reported.contains(&(flag_index, layer)) {
                    continue;
                }
                let rect = body_rect(body, spec.of, &block);
                let mut areas = Vec::new();
                if body.fill == Some(Fill::Flag) {
                    areas.push(rect);
                }
                for (ring, fill) in body.rings.iter().enumerate() {
                    if *fill == Fill::Flag {
                        let grow = i32::try_from(ring + 1).unwrap_or(0);
                        areas.push(rect.grow(grow, grow, grow, grow));
                    }
                }
                let clamp = |v: i32| u16::try_from(v.max(0)).unwrap_or(u16::MAX);
                let unfit = areas
                    .iter()
                    .find(|a| !fits(flag, clamp(a.width()), clamp(a.height())));
                if let Some(area) = unfit {
                    reported.push((flag_index, layer));
                    report.push(
                        Issue::error(
                            IssueCode::FlagUnfit,
                            path(FLAGS, &format!("flags.{}", flag.id)),
                        )
                        .name("flag", &flag.id)
                        .name("layer", &spec.id)
                        .name("variant", &asset.variants[v].key)
                        .value("width", f64::from(area.width()))
                        .value("height", f64::from(area.height()))
                        .value("name_width", f64::from(block.name_width))
                        .value("lines", f64::from(block.line_count)),
                    );
                }
            }
        }
    }
}

pub fn banding(asset: &NameplateAsset, report: &mut Report) {
    for issue in banding_issues(asset) {
        let seen = report
            .issues()
            .iter()
            .any(|i| i.code == issue.code && i.path == issue.path);
        if !seen {
            report.push(issue);
        }
    }
}

pub fn dialect_issues(asset: &NameplateAsset, dialect: Dialect) -> Vec<Issue> {
    let mut out = Vec::new();
    for spec in &asset.layers {
        let LayerContent::Material(content) = &spec.content else {
            continue;
        };
        let Some(ir) = asset.materials.get(usize::from(content.material)) else {
            continue;
        };
        for op in unsupported(ir, dialect) {
            out.push(
                Issue::error(
                    IssueCode::DialectUnsupported,
                    path(&format!("layers/{}.json", spec.id), "material"),
                )
                .name("dialect", dialect.as_str())
                .name("node", op.name()),
            );
        }
    }
    out
}
