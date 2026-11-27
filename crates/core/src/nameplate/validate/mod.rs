mod budget;
mod checks;
mod cover;
mod flash;
mod review;
mod sim;

use crate::issues::{path, Issue, Mode, Report};

use super::geom::BlockInput;
use super::material::ir::Dialect;
use super::text::Backdrop;
use super::types::NameplateAsset;
use super::Analysis;

pub(crate) const MANIFEST: &str = "manifest.json";
pub(crate) const FLAGS: &str = "flags.json";

pub(crate) fn layer_path(id: &str) -> String {
    format!("layers/{id}.json")
}

pub(crate) fn animation_path(name: &str) -> String {
    path(MANIFEST, &format!("animations.{name}"))
}

pub fn validate(asset: &NameplateAsset, report: &mut Report) {
    let measured = budget::measure(asset);
    budget::check(asset, &measured, report);
    if report.mode() != Mode::Authoring {
        return;
    }
    checks::flags(asset, report);
    checks::banding(asset, report);
    checks::material_loops(asset, report);
    flash::check(asset, &measured, report);
    review::readability(asset, report);
    review::letters_readable(asset, report);
    review::badge_tracks(asset, report);
    review::badge_covered(asset, report);
    review::art_coverage(asset, report);
    review::fixed_height(asset, report);
    review::translucent_overlap(asset, report);
    review::position_clamped(asset, &measured, report);
    review::plate_background(asset, report);
}

pub fn analysis(asset: &NameplateAsset) -> Analysis {
    budget::measure(asset).analysis
}

pub fn dialect_issues(asset: &NameplateAsset, dialect: Dialect) -> Vec<Issue> {
    checks::dialect_issues(asset, dialect)
}

pub fn sample_backdrop(asset: &NameplateAsset, variant: u16, block: &BlockInput) -> Backdrop {
    review::sample_backdrop(asset, variant, block)
}

#[cfg(test)]
mod tests;
