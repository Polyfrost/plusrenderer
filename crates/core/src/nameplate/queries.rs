use crate::molang::{Arg, Interner, Queries, QueryScope};
use crate::signals::{
    is_bool_signal, signal_id, signal_time_id, SIGNALS, SIGNAL_COUNT, TIME_SUFFIX,
};

use super::types::Param;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MolangUse {
    #[default]
    Channel,
    SlotColor,
    Field,
    Text,
    Material,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameplateQuery {
    Width,
    Height,
    LineCount,
    Frame,
    Loop,
    Fps,
    X,
    Y,
    NameX,
    NameY,
    NameWidth,
    PrefixWidth,
    BgX0,
    BgY0,
    BgX1,
    BgY1,
    GlyphIndex,
    GlyphCount,
    GlyphX,
    GlyphWidth,
    VariantParam,
}

impl NameplateQuery {
    pub const ALL: [NameplateQuery; 21] = [
        NameplateQuery::Width,
        NameplateQuery::Height,
        NameplateQuery::LineCount,
        NameplateQuery::Frame,
        NameplateQuery::Loop,
        NameplateQuery::Fps,
        NameplateQuery::X,
        NameplateQuery::Y,
        NameplateQuery::NameX,
        NameplateQuery::NameY,
        NameplateQuery::NameWidth,
        NameplateQuery::PrefixWidth,
        NameplateQuery::BgX0,
        NameplateQuery::BgY0,
        NameplateQuery::BgX1,
        NameplateQuery::BgY1,
        NameplateQuery::GlyphIndex,
        NameplateQuery::GlyphCount,
        NameplateQuery::GlyphX,
        NameplateQuery::GlyphWidth,
        NameplateQuery::VariantParam,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NameplateQuery::Width => "width",
            NameplateQuery::Height => "height",
            NameplateQuery::LineCount => "line_count",
            NameplateQuery::Frame => "frame",
            NameplateQuery::Loop => "loop",
            NameplateQuery::Fps => "fps",
            NameplateQuery::X => "x",
            NameplateQuery::Y => "y",
            NameplateQuery::NameX => "name_x",
            NameplateQuery::NameY => "name_y",
            NameplateQuery::NameWidth => "name_width",
            NameplateQuery::PrefixWidth => "prefix_width",
            NameplateQuery::BgX0 => "bg_x0",
            NameplateQuery::BgY0 => "bg_y0",
            NameplateQuery::BgX1 => "bg_x1",
            NameplateQuery::BgY1 => "bg_y1",
            NameplateQuery::GlyphIndex => "glyph_index",
            NameplateQuery::GlyphCount => "glyph_count",
            NameplateQuery::GlyphX => "glyph_x",
            NameplateQuery::GlyphWidth => "glyph_width",
            NameplateQuery::VariantParam => "variant_param",
        }
    }

    pub fn parse(name: &str) -> Option<NameplateQuery> {
        NameplateQuery::ALL.into_iter().find(|q| q.name() == name)
    }

    pub fn only(self) -> Option<MolangUse> {
        match self {
            NameplateQuery::X | NameplateQuery::Y => Some(MolangUse::Field),
            NameplateQuery::GlyphIndex
            | NameplateQuery::GlyphCount
            | NameplateQuery::GlyphX
            | NameplateQuery::GlyphWidth => Some(MolangUse::Text),
            _ => None,
        }
    }

    pub fn available_in(self, scope: MolangUse) -> bool {
        self.only().is_none_or(|only| only == scope)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

impl MolangUse {
    pub const ALL: [MolangUse; 5] = [
        MolangUse::Channel,
        MolangUse::SlotColor,
        MolangUse::Field,
        MolangUse::Text,
        MolangUse::Material,
    ];
}

pub fn is_known(name: &str) -> bool {
    NameplateQuery::parse(name).is_some()
        || signal_id(name).is_some()
        || signal_time_id(name).is_some()
}

pub fn allowed(name: &str, scope: MolangUse) -> bool {
    match NameplateQuery::parse(name) {
        Some(query) => query.available_in(scope),
        None => signal_id(name).is_some() || signal_time_id(name).is_some(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuerySlots {
    pub nameplate: Vec<Option<u16>>,
    pub signals: Vec<Option<u16>>,
    pub times: Vec<Option<u16>>,
    pub params: Vec<(u16, u16)>,
}

impl QuerySlots {
    pub fn new(interner: &Interner) -> QuerySlots {
        let nameplate = NameplateQuery::ALL
            .iter()
            .map(|q| interner.query_slot(q.name()))
            .collect();
        let signals = SIGNALS
            .iter()
            .map(|(name, _)| interner.query_slot(name))
            .collect();
        let times = (0..SIGNAL_COUNT)
            .map(|id| {
                if is_bool_signal(id) {
                    interner.query_slot(&format!("{}{TIME_SUFFIX}", SIGNALS[id].0))
                } else {
                    None
                }
            })
            .collect();
        QuerySlots {
            nameplate,
            signals,
            times,
            params: Vec::new(),
        }
    }

    pub fn with_params(interner: &Interner, params: &[Param]) -> QuerySlots {
        let mut slots = QuerySlots::new(interner);
        slots.params = params
            .iter()
            .enumerate()
            .filter_map(|(i, p)| Some((interner.text_id(&p.id)?, u16::try_from(i).ok()?)))
            .collect();
        slots
    }

    pub fn slot(&self, query: NameplateQuery) -> Option<u16> {
        self.nameplate.get(query.index()).copied().flatten()
    }

    pub fn signal(&self, id: usize) -> Option<u16> {
        self.signals.get(id).copied().flatten()
    }

    pub fn time(&self, id: usize) -> Option<u16> {
        self.times.get(id).copied().flatten()
    }

    pub fn param(&self, text: u16) -> Option<usize> {
        self.params
            .iter()
            .find(|(t, _)| *t == text)
            .map(|(_, i)| usize::from(*i))
    }
}

fn channel_allows(name: &str) -> bool {
    allowed(name, MolangUse::Channel)
}

fn slot_color_allows(name: &str) -> bool {
    allowed(name, MolangUse::SlotColor)
}

fn field_allows(name: &str) -> bool {
    allowed(name, MolangUse::Field)
}

fn text_allows(name: &str) -> bool {
    allowed(name, MolangUse::Text)
}

fn material_allows(name: &str) -> bool {
    allowed(name, MolangUse::Material)
}

fn known_allows(name: &str) -> bool {
    is_known(name)
}

pub fn scope_for(scope: MolangUse) -> QueryScope<'static> {
    match scope {
        MolangUse::Channel => QueryScope::Filter(&channel_allows),
        MolangUse::SlotColor => QueryScope::Filter(&slot_color_allows),
        MolangUse::Field => QueryScope::Filter(&field_allows),
        MolangUse::Text => QueryScope::Filter(&text_allows),
        MolangUse::Material => QueryScope::Filter(&material_allows),
    }
}

pub fn known_scope() -> QueryScope<'static> {
    QueryScope::Filter(&known_allows)
}

#[derive(Clone, Copy, Debug)]
pub struct ParamQueries<'a> {
    pub slots: &'a QuerySlots,
    pub values: &'a [f64],
}

impl Queries for ParamQueries<'_> {
    fn number(&mut self, slot: u16, args: &[Arg]) -> Option<f64> {
        if self.slots.slot(NameplateQuery::VariantParam) != Some(slot) {
            return None;
        }
        match args {
            [Arg::Text(text)] => self
                .slots
                .param(*text)
                .and_then(|i| self.values.get(i).copied()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::molang::{compile_checked, ErrorCode, Options, Program, Rules, Runtime, Warnings};

    fn compile(source: &str, scope: MolangUse, interner: &mut Interner) -> Option<Program> {
        let options = Options {
            rules: Rules::Strict,
            queries: scope_for(scope),
        };
        let compiled = compile_checked(source, interner, &options, &mut Warnings::default());
        assert!(compiled
            .as_ref()
            .err()
            .map_or(true, |e| e.code == ErrorCode::UnknownQuery));
        compiled.ok()
    }

    #[test]
    fn queries_are_scoped_to_where_they_make_sense() {
        let mut interner = Interner::default();
        for scope in MolangUse::ALL {
            assert!(compile("q.name_width + q.is_hurt_time", scope, &mut interner).is_some());
            let field = compile("q.x + q.y", scope, &mut interner);
            assert_eq!(field.is_some(), scope == MolangUse::Field, "{scope:?}");
            let glyph = compile("q.glyph_index", scope, &mut interner);
            assert_eq!(glyph.is_some(), scope == MolangUse::Text, "{scope:?}");
        }
        assert!(compile("q.ground_speed_time", MolangUse::Channel, &mut interner).is_none());
    }

    #[test]
    fn query_slots_find_signals_times_and_params() {
        let mut interner = Interner::default();
        let source = "q.variant_param('radius') * 10 + q.variant_param('missing') + q.is_hurt_time";
        let program = compile(source, MolangUse::Channel, &mut interner).unwrap();
        let param = |id: &str| Param {
            id: id.into(),
            value: 1.0,
            min: 0.0,
            max: 8.0,
        };
        let slots = QuerySlots::with_params(&interner, &[param("width"), param("radius")]);
        let hurt = signal_id("is_hurt").unwrap();
        assert_eq!(slots.time(hurt), interner.query_slot("is_hurt_time"));
        assert_eq!(slots.signal(hurt), None);
        let mut queries = ParamQueries {
            slots: &slots,
            values: &[1.0, 5.0],
        };
        assert_eq!(
            program.eval_with(&mut Runtime::new(&interner), &mut queries),
            50.0
        );
    }
}
