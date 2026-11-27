use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LossKind {
    Rotation,
    Bounds,
    Emissive,
    Animations,
    Controllers,
    Reactions,
    Effects,
    Springs,
    Colorways,
    CustomColors,
    Textures,
    Attach,
    Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Adjusted,
    Approximated,
    Dropped,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Loss {
    pub kind: LossKind,
    pub severity: Severity,
    pub at: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ConversionReport {
    pub losses: Vec<Loss>,
}

impl ConversionReport {
    pub fn push(
        &mut self,
        kind: LossKind,
        severity: Severity,
        at: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.losses.push(Loss {
            kind,
            severity,
            at: at.into(),
            detail: detail.into(),
        });
    }

    pub fn has(&self, kind: LossKind) -> bool {
        self.losses.iter().any(|l| l.kind == kind)
    }

    pub fn of(&self, kind: LossKind) -> impl Iterator<Item = &Loss> {
        self.losses.iter().filter(move |l| l.kind == kind)
    }

    pub fn worst(&self) -> Option<Severity> {
        self.losses.iter().map(|l| l.severity).max()
    }

    pub fn offerable(&self) -> bool {
        self.worst().is_none_or(|s| s == Severity::Adjusted)
    }
}
