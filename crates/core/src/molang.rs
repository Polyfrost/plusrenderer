//! MoLang, compiled once with names interned to slots and evaluated per frame without allocation.
//! Rejected at compile time: `loop`, `for_each`, arrays, `context.` and string literals.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    Other,
    TooLong,
    TooDeep,
    TooBig,
    Variables,
    Arguments,
    TooManyNames,
    Character,
    Number,
    Unterminated,
    Unexpected,
    Unsupported,
    StringUse,
    UnknownFunction,
    UnknownMath,
    UnknownQuery,
    UnknownName,
    UnknownRoot,
    Arity,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 19] = [
        ErrorCode::Other,
        ErrorCode::TooLong,
        ErrorCode::TooDeep,
        ErrorCode::TooBig,
        ErrorCode::Variables,
        ErrorCode::Arguments,
        ErrorCode::TooManyNames,
        ErrorCode::Character,
        ErrorCode::Number,
        ErrorCode::Unterminated,
        ErrorCode::Unexpected,
        ErrorCode::Unsupported,
        ErrorCode::StringUse,
        ErrorCode::UnknownFunction,
        ErrorCode::UnknownMath,
        ErrorCode::UnknownQuery,
        ErrorCode::UnknownName,
        ErrorCode::UnknownRoot,
        ErrorCode::Arity,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Other => "other",
            ErrorCode::TooLong => "too_long",
            ErrorCode::TooDeep => "too_deep",
            ErrorCode::TooBig => "too_big",
            ErrorCode::Variables => "variables",
            ErrorCode::Arguments => "arguments",
            ErrorCode::TooManyNames => "too_many_names",
            ErrorCode::Character => "character",
            ErrorCode::Number => "number",
            ErrorCode::Unterminated => "unterminated",
            ErrorCode::Unexpected => "unexpected",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::StringUse => "string_use",
            ErrorCode::UnknownFunction => "unknown_function",
            ErrorCode::UnknownMath => "unknown_math",
            ErrorCode::UnknownQuery => "unknown_query",
            ErrorCode::UnknownName => "unknown_name",
            ErrorCode::UnknownRoot => "unknown_root",
            ErrorCode::Arity => "arity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "molang: {}", self.0)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub code: ErrorCode,
    pub error: Error,
}

impl CompileError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> CompileError {
        CompileError {
            code,
            error: Error(message.into()),
        }
    }

    pub fn message(&self) -> &str {
        &self.error.0
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for CompileError {}

impl From<CompileError> for Error {
    fn from(e: CompileError) -> Error {
        e.error
    }
}

type Result<T> = std::result::Result<T, Error>;
type Coded<T> = std::result::Result<T, CompileError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rules {
    #[default]
    Bedrock,
    Strict,
}

struct Limits {
    chars: usize,
    nodes: usize,
    depth: usize,
    height: usize,
    variables: usize,
    args: usize,
}

impl Rules {
    fn limits(self) -> Limits {
        match self {
            Rules::Strict => Limits {
                chars: 4096,
                nodes: 256,
                depth: 64,
                height: 256,
                variables: 64,
                args: 4,
            },
            Rules::Bedrock => Limits {
                chars: 16_384,
                nodes: 4096,
                depth: 256,
                height: 1024,
                variables: usize::MAX,
                args: usize::MAX,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub enum QueryScope<'a> {
    Any,
    Only(&'a [&'a str]),
    Filter(&'a dyn Fn(&str) -> bool),
}

impl QueryScope<'_> {
    fn allows(&self, name: &str) -> bool {
        match self {
            QueryScope::Any => true,
            QueryScope::Only(names) => names.contains(&name),
            QueryScope::Filter(allowed) => allowed(name),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub rules: Rules,
    pub queries: QueryScope<'a>,
}

impl Options<'_> {
    pub fn bedrock() -> Options<'static> {
        Options {
            rules: Rules::Bedrock,
            queries: QueryScope::Any,
        }
    }

    pub fn strict() -> Options<'static> {
        Options {
            rules: Rules::Strict,
            queries: QueryScope::Any,
        }
    }
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options::bedrock()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    UnknownFunction,
    UnknownQuery,
    UnknownName,
    UnknownRoot,
    Arity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub kind: WarningKind,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct Warnings {
    list: Vec<Warning>,
}

impl Warnings {
    pub fn as_slice(&self) -> &[Warning] {
        &self.list
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn add(&mut self, kind: WarningKind, name: &str) {
        if !self.list.iter().any(|w| w.kind == kind && w.name == name) {
            self.list.push(Warning {
                kind,
                name: name.to_string(),
            });
        }
    }
}

/// Shared by every program of one model, so they all see the same `variable.` storage.
#[derive(Debug, Default)]
pub struct Interner {
    index: [HashMap<String, u16>; 4],
    texts: Vec<String>,
    programs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Space {
    Var,
    Temp,
    Query,
    Text,
}

impl Interner {
    fn intern(&mut self, space: Space, name: &str) -> Coded<u16> {
        let map = &mut self.index[space as usize];
        if let Some(&slot) = map.get(name) {
            return Ok(slot);
        }

        let next = u16::try_from(map.len())
            .map_err(|_| CompileError::new(ErrorCode::TooManyNames, "too many names"))?;
        map.insert(name.to_string(), next);
        if space == Space::Text {
            self.texts.push(name.to_string());
        }
        Ok(next)
    }

    fn count(&self, space: Space) -> usize {
        self.index[space as usize].len()
    }

    fn contains(&self, space: Space, name: &str) -> bool {
        self.index[space as usize].contains_key(name)
    }

    pub fn query_slot(&self, name: &str) -> Option<u16> {
        self.index[Space::Query as usize].get(name).copied()
    }

    pub fn query_count(&self) -> usize {
        self.count(Space::Query)
    }

    pub fn variable_slot(&self, name: &str) -> Option<u16> {
        self.index[Space::Var as usize].get(name).copied()
    }

    pub fn variable_count(&self) -> usize {
        self.count(Space::Var)
    }

    pub fn program_count(&self) -> u32 {
        self.programs
    }

    pub fn text_id(&self, text: &str) -> Option<u16> {
        self.index[Space::Text as usize].get(text).copied()
    }

    pub fn text(&self, id: u16) -> Option<&str> {
        self.texts.get(id as usize).map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arg {
    Number(f64),
    Text(u16),
}

pub trait Queries {
    fn number(&mut self, slot: u16, args: &[Arg]) -> Option<f64>;

    fn text(&mut self, _slot: u16, _args: &[Arg]) -> Option<u16> {
        None
    }
}

pub struct NoQueries;

impl Queries for NoQueries {
    fn number(&mut self, _slot: u16, _args: &[Arg]) -> Option<f64> {
        None
    }
}

/// `variables` persist across frames, `temps` do not. The host writes `queries` before each
/// frame.
pub const STEP_LIMIT: u32 = 2_000;
pub const FRAME_FUEL: u32 = 4_000;
pub const RANDOM_TICKS_PER_SECOND: f64 = 20.0;

#[derive(Debug, Clone)]
struct Work {
    temps: Vec<f64>,
    queries: Vec<f64>,
    args: Vec<Arg>,
}

impl Work {
    fn new(interner: &Interner) -> Work {
        Work {
            temps: vec![0.0; interner.count(Space::Temp)],
            // NaN marks a query the host never supplied, for `??`.
            queries: vec![f64::NAN; interner.count(Space::Query)],
            args: Vec::new(),
        }
    }

    fn query(&self, slot: u16) -> Option<f64> {
        self.queries
            .get(slot as usize)
            .copied()
            .filter(|v| !v.is_nan())
    }

    fn set_query(&mut self, slot: u16, value: f32) {
        grow_with(&mut self.queries, slot, f64::NAN);
        self.queries[slot as usize] = value as f64;
    }

    fn set_queries(&mut self, values: &[f32]) {
        self.queries.clear();
        self.queries.extend(values.iter().map(|v| *v as f64));
    }
}

/// `variables` persist across frames, `temps` do not. The host writes `queries` before each
/// frame.
#[derive(Debug, Clone)]
pub struct Runtime {
    variables: Vec<f64>,
    work: Work,
    seed: u64,
    frame: i64,
}

impl Runtime {
    pub fn new(interner: &Interner) -> Runtime {
        Runtime {
            variables: vec![0.0; interner.count(Space::Var)],
            work: Work::new(interner),
            seed: 0,
            frame: 0,
        }
    }

    pub fn set_query(&mut self, slot: u16, value: f32) {
        self.work.set_query(slot, value);
    }

    pub fn clear_temps(&mut self) {
        self.work.temps.fill(0.0);
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    pub fn set_clock(&mut self, seconds: f64) {
        self.frame = random_frame(seconds);
    }

    pub fn frame(&mut self) -> Scope<'_> {
        self.clear_temps();
        self.own_scope()
    }

    pub fn wearer_frame<'a>(
        &'a mut self,
        variables: &'a mut [f32],
        seed: u64,
        clock: f64,
    ) -> Scope<'a> {
        self.clear_temps();
        Scope {
            variables: Variables::Narrow(variables),
            work: &mut self.work,
            seed,
            frame: random_frame(clock),
            fuel: FRAME_FUEL,
        }
    }

    fn own_scope(&mut self) -> Scope<'_> {
        Scope {
            variables: Variables::Wide(&mut self.variables),
            work: &mut self.work,
            seed: self.seed,
            frame: self.frame,
            fuel: FRAME_FUEL,
        }
    }
}

#[derive(Debug)]
enum Variables<'a> {
    Wide(&'a mut Vec<f64>),
    Narrow(&'a mut [f32]),
}

impl Variables<'_> {
    fn get(&self, slot: u16) -> f64 {
        match self {
            Variables::Wide(v) => v.get(slot as usize).copied().unwrap_or(0.0),
            Variables::Narrow(v) => v.get(slot as usize).map_or(0.0, |x| *x as f64),
        }
    }

    fn set(&mut self, slot: u16, value: f64) {
        match self {
            Variables::Wide(v) => {
                grow_with(v, slot, 0.0);
                v[slot as usize] = value;
            }
            Variables::Narrow(v) => {
                if let Some(x) = v.get_mut(slot as usize) {
                    *x = value as f32;
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Scope<'a> {
    variables: Variables<'a>,
    work: &'a mut Work,
    seed: u64,
    frame: i64,
    fuel: u32,
}

impl Scope<'_> {
    pub fn fuel(&self) -> u32 {
        self.fuel
    }

    pub fn set_fuel(&mut self, fuel: u32) {
        self.fuel = fuel;
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn random_frame(&self) -> i64 {
        self.frame
    }

    pub fn set_random_frame(&mut self, frame: i64) {
        self.frame = frame;
    }

    pub fn set_query(&mut self, slot: u16, value: f32) {
        self.work.set_query(slot, value);
    }

    pub fn set_queries(&mut self, values: &[f32]) {
        self.work.set_queries(values);
    }

    pub fn variable(&self, slot: u16) -> f32 {
        self.variables.get(slot) as f32
    }

    pub fn copy_variables(&self, out: &mut Vec<f64>) {
        match &self.variables {
            Variables::Wide(v) => out.extend_from_slice(v),
            Variables::Narrow(v) => out.extend(v.iter().map(|&x| f64::from(x))),
        }
    }

    pub fn set_variable(&mut self, slot: u16, value: f32) {
        self.variables.set(slot, value as f64);
    }

    pub fn clear_temps(&mut self) {
        self.work.temps.fill(0.0);
    }

    pub fn copy_temps(&self, out: &mut Vec<f64>) {
        out.clear();
        out.extend_from_slice(&self.work.temps);
    }

    pub fn set_temps(&mut self, temps: &[f64]) {
        self.work.temps.clear();
        self.work.temps.extend_from_slice(temps);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub value: f64,
    pub failed: bool,
}

pub fn random_frame(seconds: f64) -> i64 {
    if seconds.is_finite() {
        libm::floor(seconds * RANDOM_TICKS_PER_SECOND) as i64
    } else {
        0
    }
}

const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn random_bits(seed: u64, expression: u32, call: u32, frame: i64) -> u64 {
    let site = ((expression as u64) << 32) | call as u64;
    let h = mix64(seed.wrapping_add(GOLDEN_GAMMA));
    let h = mix64(h ^ site.wrapping_add(GOLDEN_GAMMA.wrapping_mul(2)));
    mix64(h ^ (frame as u64).wrapping_add(GOLDEN_GAMMA.wrapping_mul(3)))
}

fn unit(bits: u64) -> f64 {
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

#[derive(Debug, Default)]
pub struct Env {
    rules: Rules,
    interner: Interner,
    programs: Vec<Program>,
    warnings: Warnings,
}

impl Env {
    pub fn new(rules: Rules) -> Env {
        Env {
            rules,
            ..Env::default()
        }
    }

    pub fn compile(&mut self, source: &str) -> Result<u32> {
        let options = Options {
            rules: self.rules,
            queries: QueryScope::Any,
        };
        let program = compile_with(source, &mut self.interner, &options, &mut self.warnings)?;
        let id = program.id;
        self.programs.push(program);
        Ok(id)
    }

    pub fn query_id(&self, name: &str) -> Option<u16> {
        self.interner
            .query_slot(&unprefixed(name, ["query.", "q."]))
    }

    pub fn variable_id(&self, name: &str) -> Option<u16> {
        self.interner
            .variable_slot(&unprefixed(name, ["variable.", "v."]))
    }

    pub fn query_count(&self) -> usize {
        self.interner.query_count()
    }

    pub fn variable_count(&self) -> usize {
        self.interner.variable_count()
    }

    pub fn program(&self, id: u32) -> Option<&Program> {
        self.programs.get(id as usize)
    }

    pub fn interner(&self) -> &Interner {
        &self.interner
    }

    pub fn warnings(&self) -> &Warnings {
        &self.warnings
    }
}

fn unprefixed(name: &str, roots: [&str; 2]) -> String {
    let name = name.to_ascii_lowercase();
    match roots.iter().find_map(|root| name.strip_prefix(root)) {
        Some(rest) => rest.to_string(),
        None => name,
    }
}

#[derive(Debug, Clone)]
pub struct EntityRuntime {
    variables: Vec<f32>,
    work: Work,
    seed: u64,
}

impl EntityRuntime {
    pub fn new(env: &Env, seed: u64) -> EntityRuntime {
        EntityRuntime {
            variables: vec![0.0; env.variable_count()],
            work: Work::new(&env.interner),
            seed,
        }
    }

    pub fn initialize(&mut self, env: &Env, script_ids: &[u32]) -> usize {
        self.work.queries.clear();
        self.run(env, script_ids, 0, &mut [], &mut [])
    }

    pub fn evaluate(
        &mut self,
        env: &Env,
        expr_ids: &[u32],
        queries: &[f32],
        clock: f64,
        out: &mut [f32],
        failed: &mut [bool],
    ) -> usize {
        self.work.set_queries(queries);
        self.run(env, expr_ids, random_frame(clock), out, failed)
    }

    pub fn variable(&self, id: u16) -> f32 {
        self.variables.get(id as usize).copied().unwrap_or(0.0)
    }

    pub fn set_variable(&mut self, id: u16, value: f32) {
        if let Some(v) = self.variables.get_mut(id as usize) {
            *v = value;
        }
    }

    pub fn fit(&mut self, env: &Env) {
        if self.variables.len() < env.variable_count() {
            self.variables.resize(env.variable_count(), 0.0);
        }
    }

    pub fn reset(&mut self) {
        self.variables.fill(0.0);
        self.work.temps.fill(0.0);
    }

    fn run(
        &mut self,
        env: &Env,
        ids: &[u32],
        frame: i64,
        out: &mut [f32],
        failed: &mut [bool],
    ) -> usize {
        self.fit(env);
        self.work.temps.fill(0.0);
        let mut scope = Scope {
            variables: Variables::Narrow(&mut self.variables),
            work: &mut self.work,
            seed: self.seed,
            frame,
            fuel: FRAME_FUEL,
        };
        let mut failures = 0;
        for (i, id) in ids.iter().enumerate() {
            let outcome = match env.program(*id) {
                Some(program) => program.evaluate(&mut scope, &mut NoQueries),
                None => Outcome {
                    value: 0.0,
                    failed: false,
                },
            };
            if let Some(slot) = out.get_mut(i) {
                *slot = outcome.value as f32;
            }
            if let Some(slot) = failed.get_mut(i) {
                *slot = outcome.failed;
            }
            failures += outcome.failed as usize;
        }
        failures
    }
}

fn grow_with(v: &mut Vec<f64>, slot: u16, fill: f64) {
    if v.len() <= slot as usize {
        v.resize(slot as usize + 1, fill);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Func {
    Abs,
    Acos,
    Asin,
    Atan,
    Atan2,
    Bayer,
    Ceil,
    Clamp,
    Cos,
    DieRoll,
    DieRollInt,
    Exp,
    Fbm,
    Floor,
    FloorMod,
    Frac,
    Hash,
    HermiteBlend,
    Hsv,
    Lerp,
    LerpRotate,
    Ln,
    Max,
    Min,
    Mod,
    Noise,
    Pow,
    Random,
    RandomInt,
    Round,
    Sign,
    Sin,
    Smoothstep,
    Sqrt,
    Step,
    Trunc,
}

impl Func {
    fn lookup(name: &str) -> Option<(Func, usize, usize)> {
        let (func, min, max) = match name {
            "abs" => (Func::Abs, 1, 1),
            "acos" => (Func::Acos, 1, 1),
            "asin" => (Func::Asin, 1, 1),
            "atan" => (Func::Atan, 1, 1),
            "atan2" => (Func::Atan2, 2, 2),
            "bayer" => (Func::Bayer, 2, 2),
            "ceil" => (Func::Ceil, 1, 1),
            "clamp" => (Func::Clamp, 3, 3),
            "cos" => (Func::Cos, 1, 1),
            "die_roll" => (Func::DieRoll, 3, 3),
            "die_roll_integer" => (Func::DieRollInt, 3, 3),
            "exp" => (Func::Exp, 1, 1),
            "fbm" => (Func::Fbm, 3, 4),
            "floor" => (Func::Floor, 1, 1),
            "floor_mod" => (Func::FloorMod, 2, 2),
            "frac" => (Func::Frac, 1, 1),
            "hash" => (Func::Hash, 3, 3),
            "hermite_blend" => (Func::HermiteBlend, 1, 1),
            "hsv" => (Func::Hsv, 3, 3),
            "lerp" => (Func::Lerp, 3, 3),
            "lerprotate" => (Func::LerpRotate, 3, 3),
            "ln" => (Func::Ln, 1, 1),
            "max" => (Func::Max, 2, 2),
            "min" => (Func::Min, 2, 2),
            "mod" => (Func::Mod, 2, 2),
            "noise" => (Func::Noise, 2, 3),
            "pow" => (Func::Pow, 2, 2),
            "random" => (Func::Random, 2, 2),
            "random_integer" => (Func::RandomInt, 2, 2),
            "round" => (Func::Round, 1, 1),
            "sign" => (Func::Sign, 1, 1),
            "sin" => (Func::Sin, 1, 1),
            "smoothstep" => (Func::Smoothstep, 3, 3),
            "sqrt" => (Func::Sqrt, 1, 1),
            "step" => (Func::Step, 2, 2),
            "trunc" => (Func::Trunc, 1, 1),
            _ => return None,
        };
        Some((func, min, max))
    }
}

#[derive(Debug, Clone)]
enum Expr {
    Num(f64),
    Text(u16),
    Var(u16),
    Temp(u16),
    Query(u16),
    QueryCall(u16, Vec<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Binary(Bin, Box<Expr>, Box<Expr>),
    TextEq(bool, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Coalesce(Box<Expr>, Box<Expr>),
    AssignVar(u16, Box<Expr>),
    AssignTemp(u16, Box<Expr>),
    Call(Func, Vec<Expr>),
}

#[derive(Debug, Clone)]
enum Stmt {
    Expr(Expr),
    Return(Expr),
    Block(Vec<Stmt>),
    If(Expr, Vec<Stmt>, Option<Box<Stmt>>),
}

impl Stmt {
    fn ends_with_brace(&self) -> bool {
        match self {
            Stmt::Block(_) => true,
            Stmt::If(_, _, None) => true,
            Stmt::If(_, _, Some(otherwise)) => otherwise.ends_with_brace(),
            Stmt::Expr(_) | Stmt::Return(_) => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reads {
    pub queries: Vec<u16>,
    pub variables: Vec<u16>,
    pub random: bool,
}

impl Reads {
    fn of(stmts: &[Stmt]) -> Reads {
        let mut reads = Reads::default();
        stmts.iter().for_each(|s| reads.stmt(s));
        for slots in [&mut reads.queries, &mut reads.variables] {
            slots.sort_unstable();
            slots.dedup();
        }
        reads
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(e) | Stmt::Return(e) => self.expr(e),
            Stmt::Block(body) => body.iter().for_each(|s| self.stmt(s)),
            Stmt::If(cond, then, otherwise) => {
                self.expr(cond);
                then.iter().for_each(|s| self.stmt(s));
                if let Some(otherwise) = otherwise {
                    self.stmt(otherwise);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Var(slot) => self.variables.push(*slot),
            Expr::Query(slot) => self.queries.push(*slot),
            Expr::QueryCall(slot, args) => {
                self.queries.push(*slot);
                args.iter().for_each(|a| self.expr(a));
            }
            Expr::Neg(a) | Expr::Not(a) | Expr::AssignVar(_, a) | Expr::AssignTemp(_, a) => {
                self.expr(a)
            }
            Expr::Binary(_, a, b) | Expr::TextEq(_, a, b) | Expr::Coalesce(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Ternary(c, a, b) => {
                self.expr(c);
                self.expr(a);
                self.expr(b);
            }
            Expr::Call(func, args) => {
                self.random |= matches!(
                    func,
                    Func::Random | Func::RandomInt | Func::DieRoll | Func::DieRollInt
                );
                args.iter().for_each(|a| self.expr(a));
            }
            Expr::Num(_) | Expr::Text(_) | Expr::Temp(_) => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    stmts: Vec<Stmt>,
    /// A bare expression yields its value. A statement list yields 0 unless it returns.
    single: bool,
    id: u32,
    nodes: u32,
    reads: Reads,
}

impl Program {
    pub fn number(value: f32) -> Program {
        Program {
            stmts: vec![Stmt::Expr(Expr::Num(value as f64))],
            single: true,
            id: 0,
            nodes: 0,
            reads: Reads::default(),
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn nodes(&self) -> u32 {
        self.nodes
    }

    pub fn reads(&self) -> &Reads {
        &self.reads
    }

    pub fn eval(&self, rt: &mut Runtime) -> f32 {
        self.eval_with(rt, &mut NoQueries)
    }

    pub fn literal(&self) -> Option<f64> {
        match self.stmts.as_slice() {
            [Stmt::Expr(Expr::Num(value))] if self.single => Some(*value),
            _ => None,
        }
    }

    pub fn bounds(&self, variable: &dyn Fn(u16) -> Option<(f64, f64)>) -> Option<(f64, f64)> {
        match self.stmts.as_slice() {
            [Stmt::Expr(e)] if self.single => Bounds { variable }.expr(e),
            _ => None,
        }
    }

    pub fn clock_scale(&self, time: u16, delta: u16) -> Option<f64> {
        let e = match self.stmts.as_slice() {
            [Stmt::Expr(e)] if self.single => e,
            [Stmt::Return(e)] => e,
            _ => return None,
        };
        let [constant, t, d] = Linear { time, delta }.expr(e)?;
        (constant == 0.0 && t == 1.0 && d.is_finite() && d >= 0.0).then_some(d)
    }

    pub fn eval_with(&self, rt: &mut Runtime, queries: &mut dyn Queries) -> f32 {
        rt.clear_temps();
        self.run(rt, queries) as f32
    }

    pub fn run(&self, rt: &mut Runtime, queries: &mut dyn Queries) -> f64 {
        self.evaluate(&mut rt.own_scope(), queries).value
    }

    pub fn evaluate(&self, scope: &mut Scope<'_>, queries: &mut dyn Queries) -> Outcome {
        let first = match self.stmts.first() {
            Some(Stmt::Expr(e)) if self.single => Some(e),
            _ => None,
        };
        if let Some(Expr::Num(n)) = first {
            return Outcome {
                value: *n,
                failed: false,
            };
        }

        let mut cx = Eval {
            scope,
            queries,
            id: self.id,
            calls: 0,
            steps: STEP_LIMIT,
            failed: false,
        };
        let value = match first {
            Some(e) => cx.expr(e),
            None => cx.block(&self.stmts).unwrap_or(0.0),
        };
        if cx.failed {
            return Outcome {
                value: 0.0,
                failed: true,
            };
        }
        Outcome {
            value,
            failed: false,
        }
    }
}

pub fn compile(source: &str, interner: &mut Interner) -> Result<Program> {
    compile_with(
        source,
        interner,
        &Options::bedrock(),
        &mut Warnings::default(),
    )
}

pub fn compile_with(
    source: &str,
    interner: &mut Interner,
    options: &Options<'_>,
    warnings: &mut Warnings,
) -> Result<Program> {
    compile_checked(source, interner, options, warnings).map_err(Error::from)
}

pub fn compile_checked(
    source: &str,
    interner: &mut Interner,
    options: &Options<'_>,
    warnings: &mut Warnings,
) -> Coded<Program> {
    let limits = options.rules.limits();
    if source.chars().count() > limits.chars {
        return Err(CompileError::new(
            ErrorCode::TooLong,
            format!("longer than {} characters", limits.chars),
        ));
    }

    let tokens = lex(source)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        interner,
        options,
        warnings,
        limits,
        nodes: 0,
        depth: 0,
        height: 0,
    };
    let mut program = parser.program()?;
    program.nodes = u32::try_from(parser.nodes).unwrap_or(u32::MAX);
    check_stmts(&program.stmts)?;
    program.reads = Reads::of(&program.stmts);
    program.id = interner.programs;
    interner.programs = interner.programs.wrapping_add(1);
    Ok(program)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Str(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    Not,
    Question,
    Colon,
    Coalesce,
    Assign,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    End,
}

fn lex(src: &str) -> Coded<Vec<Tok>> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }

            let text = &src[start..i];
            let n = text.parse::<f64>().map_err(|_| {
                CompileError::new(ErrorCode::Number, format!("bad number `{text}`"))
            })?;
            out.push(Tok::Num(n));
            continue;
        }

        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            out.push(Tok::Name(src[start..i].to_ascii_lowercase()));
            continue;
        }

        if c == b'\'' {
            let start = i + 1;
            let len = src[start..]
                .find('\'')
                .ok_or_else(|| CompileError::new(ErrorCode::Unterminated, "unterminated string"))?;
            out.push(Tok::Str(src[start..start + len].to_string()));
            i = start + len + 1;
            continue;
        }

        let (tok, len) = match (c, b.get(i + 1).copied()) {
            (b'<', Some(b'=')) => (Tok::Le, 2),
            (b'>', Some(b'=')) => (Tok::Ge, 2),
            (b'=', Some(b'=')) => (Tok::EqEq, 2),
            (b'!', Some(b'=')) => (Tok::Ne, 2),
            (b'&', Some(b'&')) => (Tok::AndAnd, 2),
            (b'|', Some(b'|')) => (Tok::OrOr, 2),
            (b'?', Some(b'?')) => (Tok::Coalesce, 2),
            (b'+', _) => (Tok::Plus, 1),
            (b'-', _) => (Tok::Minus, 1),
            (b'*', _) => (Tok::Star, 1),
            (b'/', _) => (Tok::Slash, 1),
            (b'%', _) => (Tok::Percent, 1),
            (b'<', _) => (Tok::Lt, 1),
            (b'>', _) => (Tok::Gt, 1),
            (b'!', _) => (Tok::Not, 1),
            (b'?', _) => (Tok::Question, 1),
            (b':', _) => (Tok::Colon, 1),
            (b'=', _) => (Tok::Assign, 1),
            (b'(', _) => (Tok::LParen, 1),
            (b')', _) => (Tok::RParen, 1),
            (b'{', _) => (Tok::LBrace, 1),
            (b'}', _) => (Tok::RBrace, 1),
            (b',', _) => (Tok::Comma, 1),
            (b';', _) => (Tok::Semi, 1),
            _ => {
                let ch = src[i..].chars().next().unwrap_or('?');
                return Err(CompileError::new(
                    ErrorCode::Character,
                    format!("unexpected character `{ch}`"),
                ));
            }
        };

        out.push(tok);
        i += len;
    }

    out.push(Tok::End);
    Ok(out)
}

struct Parser<'a, 'o> {
    tokens: Vec<Tok>,
    pos: usize,
    interner: &'a mut Interner,
    options: &'a Options<'o>,
    warnings: &'a mut Warnings,
    limits: Limits,
    nodes: usize,
    depth: usize,
    height: usize,
}

impl Parser<'_, '_> {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos]
    }

    fn peek_next(&self) -> &Tok {
        &self.tokens[(self.pos + 1).min(self.tokens.len() - 1)]
    }

    fn bump(&mut self) -> Tok {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == tok {
            self.bump();
            return true;
        }
        false
    }

    fn expect(&mut self, tok: Tok) -> Coded<()> {
        if self.eat(&tok) {
            Ok(())
        } else {
            Err(CompileError::new(
                ErrorCode::Unexpected,
                format!("expected {tok:?}, found {:?}", self.peek()),
            ))
        }
    }

    fn strict(&self) -> bool {
        self.options.rules == Rules::Strict
    }

    fn node(&mut self) -> Coded<()> {
        self.nodes += 1;
        if self.nodes > self.limits.nodes {
            return Err(CompileError::new(
                ErrorCode::TooBig,
                format!("more than {} nodes", self.limits.nodes),
            ));
        }
        Ok(())
    }

    fn enter(&mut self) -> Coded<()> {
        self.depth += 1;
        if self.depth > self.limits.depth {
            return Err(CompileError::new(
                ErrorCode::TooDeep,
                format!("nested deeper than {}", self.limits.depth),
            ));
        }
        self.rise()
    }

    fn rise(&mut self) -> Coded<()> {
        self.height += 1;
        if self.height > self.limits.height {
            return Err(CompileError::new(
                ErrorCode::TooDeep,
                format!("more than {} levels of operators", self.limits.height),
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
        self.height -= 1;
    }

    fn reject(&mut self, kind: WarningKind, name: &str, message: String) -> Coded<()> {
        if self.strict() {
            return Err(CompileError::new(rejected(kind, name), message));
        }
        self.warnings.add(kind, name);
        Ok(())
    }

    fn program(&mut self) -> Coded<Program> {
        let (stmts, terminated) = self.statements(&Tok::End)?;
        if stmts.is_empty() {
            return Ok(Program::number(0.0));
        }

        let single = stmts.len() == 1 && !terminated && matches!(stmts[0], Stmt::Expr(_));
        Ok(Program {
            stmts,
            single,
            id: 0,
            nodes: 0,
            reads: Reads::default(),
        })
    }

    fn statements(&mut self, end: &Tok) -> Coded<(Vec<Stmt>, bool)> {
        let mut stmts = Vec::new();
        let mut terminated = false;

        while self.eat(&Tok::Semi) {}
        while self.peek() != end {
            let stmt = self.statement()?;
            let braced = stmt.ends_with_brace();
            stmts.push(stmt);

            terminated = self.eat(&Tok::Semi);
            while self.eat(&Tok::Semi) {}
            if !terminated && !braced {
                break;
            }
        }

        self.expect(end.clone())?;
        Ok((stmts, terminated))
    }

    fn statement(&mut self) -> Coded<Stmt> {
        if matches!(self.peek(), Tok::Name(n) if n == "return") {
            self.bump();
            self.node()?;
            return Ok(Stmt::Return(self.expr()?));
        }
        if *self.peek() == Tok::LBrace {
            return Ok(Stmt::Block(self.block()?));
        }

        self.enter()?;
        let cond = self.coalesce()?;
        let stmt = if *self.peek() == Tok::Question && *self.peek_next() == Tok::LBrace {
            self.bump();
            self.node()?;
            let then = self.block()?;
            let otherwise = if self.eat(&Tok::Colon) {
                Some(Box::new(self.statement()?))
            } else {
                None
            };
            Stmt::If(cond, then, otherwise)
        } else {
            Stmt::Expr(self.ternary_rest(cond)?)
        };
        self.leave();
        Ok(stmt)
    }

    fn block(&mut self) -> Coded<Vec<Stmt>> {
        self.expect(Tok::LBrace)?;
        self.enter()?;
        self.node()?;
        let (stmts, _) = self.statements(&Tok::RBrace)?;
        self.leave();
        Ok(stmts)
    }

    fn expr(&mut self) -> Coded<Expr> {
        self.enter()?;
        let cond = self.coalesce()?;
        let e = self.ternary_rest(cond)?;
        self.leave();
        Ok(e)
    }

    fn ternary_rest(&mut self, cond: Expr) -> Coded<Expr> {
        if !self.eat(&Tok::Question) {
            return Ok(cond);
        }
        self.node()?;
        let a = self.expr()?;
        // `a ? b` with no else yields 0 when false.
        let b = if self.eat(&Tok::Colon) {
            self.expr()?
        } else {
            Expr::Num(0.0)
        };
        Ok(Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b)))
    }

    fn coalesce(&mut self) -> Coded<Expr> {
        let mut lhs = self.binary(0)?;
        let height = self.height;
        while self.eat(&Tok::Coalesce) {
            self.node()?;
            self.rise()?;
            let rhs = self.binary(0)?;
            lhs = Expr::Coalesce(Box::new(lhs), Box::new(rhs));
        }
        self.height = height;
        Ok(lhs)
    }

    fn binary(&mut self, min_level: u8) -> Coded<Expr> {
        let mut lhs = self.unary()?;
        let height = self.height;
        while let Some((op, level)) = binary_op(self.peek()) {
            if level < min_level {
                break;
            }
            self.bump();
            self.rise()?;
            let rhs = self.binary(level + 1)?;
            self.node()?;
            lhs = self.combine(op, lhs, rhs)?;
        }
        self.height = height;
        Ok(lhs)
    }

    fn combine(&mut self, op: Bin, lhs: Expr, rhs: Expr) -> Coded<Expr> {
        let text = matches!(lhs, Expr::Text(_)) || matches!(rhs, Expr::Text(_));
        if !text || !matches!(op, Bin::Eq | Bin::Ne) {
            return Ok(Expr::Binary(op, Box::new(lhs), Box::new(rhs)));
        }

        let textual = |e: &Expr| matches!(e, Expr::Text(_) | Expr::Query(_) | Expr::QueryCall(..));
        if self.strict() && !(textual(&lhs) && textual(&rhs)) {
            return Err(CompileError::new(
                ErrorCode::StringUse,
                "a string can only be compared with a string or a query",
            ));
        }
        Ok(Expr::TextEq(op == Bin::Eq, Box::new(lhs), Box::new(rhs)))
    }

    fn unary(&mut self) -> Coded<Expr> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Neg(Box::new(self.nested_unary()?)));
        }
        if self.eat(&Tok::Not) {
            return Ok(Expr::Not(Box::new(self.nested_unary()?)));
        }
        self.eat(&Tok::Plus);
        self.primary()
    }

    fn nested_unary(&mut self) -> Coded<Expr> {
        self.enter()?;
        self.node()?;
        let e = self.unary()?;
        self.leave();
        Ok(e)
    }

    fn primary(&mut self) -> Coded<Expr> {
        match self.bump() {
            Tok::Num(n) => {
                self.node()?;
                Ok(Expr::Num(n))
            }
            Tok::Str(s) => {
                self.node()?;
                Ok(Expr::Text(self.interner.intern(Space::Text, &s)?))
            }
            Tok::LParen => {
                let e = self.expr()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Tok::Name(name) => {
                self.node()?;
                self.name(&name)
            }
            tok => Err(CompileError::new(
                ErrorCode::Unexpected,
                format!("unexpected {tok:?}"),
            )),
        }
    }

    fn name(&mut self, name: &str) -> Coded<Expr> {
        let (head, rest) = name.split_once('.').unwrap_or((name, ""));
        match head {
            "math" => self.math(name, rest),
            "variable" | "v" => self.variable(Space::Var, name, rest),
            "temp" | "t" => self.variable(Space::Temp, name, rest),
            "query" | "q" => self.query(name, rest),
            "true" if rest.is_empty() => Ok(Expr::Num(1.0)),
            "false" if rest.is_empty() => Ok(Expr::Num(0.0)),
            "loop" | "for_each" | "break" | "continue" | "return" => Err(CompileError::new(
                ErrorCode::Unsupported,
                format!("unsupported `{name}`"),
            )),
            "context" | "c" if !rest.is_empty() => self.foreign(name, &format!("context.{rest}")),
            _ if !rest.is_empty() => self.foreign(name, name),
            _ => self.bare(name),
        }
    }

    fn math(&mut self, name: &str, rest: &str) -> Coded<Expr> {
        let Some((func, min, max)) = Func::lookup(rest) else {
            return self.unknown_call(name);
        };

        let mut args = self.call_args()?;
        if args.len() < min || args.len() > max {
            let count = if min == max {
                min.to_string()
            } else {
                format!("{min} to {max}")
            };
            self.reject(
                WarningKind::Arity,
                name,
                format!("`{name}` takes {count} arguments, got {}", args.len()),
            )?;
            args.truncate(max);
            args.resize(args.len().max(min), Expr::Num(0.0));
        }
        Ok(Expr::Call(func, args))
    }

    fn variable(&mut self, space: Space, name: &str, key: &str) -> Coded<Expr> {
        if key.is_empty() {
            return self.bare(name);
        }

        let slot = self.slot(space, key)?;
        if self.eat(&Tok::Assign) {
            self.node()?;
            let value = Box::new(self.expr()?);
            return Ok(match space {
                Space::Temp => Expr::AssignTemp(slot, value),
                _ => Expr::AssignVar(slot, value),
            });
        }
        Ok(match space {
            Space::Temp => Expr::Temp(slot),
            _ => Expr::Var(slot),
        })
    }

    fn slot(&mut self, space: Space, key: &str) -> Coded<u16> {
        if self.strict() && !self.interner.contains(space, key) {
            let used = self.interner.count(Space::Var) + self.interner.count(Space::Temp);
            if used >= self.limits.variables {
                return Err(CompileError::new(
                    ErrorCode::Variables,
                    format!("more than {} variables", self.limits.variables),
                ));
            }
        }
        self.interner.intern(space, key)
    }

    fn query(&mut self, name: &str, key: &str) -> Coded<Expr> {
        if key.is_empty() {
            return self.bare(name);
        }

        if !self.options.queries.allows(key) {
            self.reject(
                WarningKind::UnknownQuery,
                name,
                format!("`{name}` is not available here"),
            )?;
            self.call_args()?;
            return Ok(Expr::Num(0.0));
        }

        let slot = self.interner.intern(Space::Query, key)?;
        let args = self.call_args()?;
        if args.is_empty() {
            return Ok(Expr::Query(slot));
        }
        Ok(Expr::QueryCall(slot, args))
    }

    fn foreign(&mut self, name: &str, key: &str) -> Coded<Expr> {
        if *self.peek() == Tok::LParen {
            return self.unknown_call(name);
        }
        self.reject(
            WarningKind::UnknownRoot,
            key,
            format!("unknown root in `{name}`"),
        )?;
        self.variable(Space::Var, name, key)
    }

    fn bare(&mut self, name: &str) -> Coded<Expr> {
        if *self.peek() == Tok::LParen {
            return self.unknown_call(name);
        }
        self.reject(
            WarningKind::UnknownName,
            name,
            format!("unknown name `{name}`"),
        )?;
        if self.eat(&Tok::Assign) {
            return self.expr();
        }
        Ok(Expr::Num(0.0))
    }

    fn unknown_call(&mut self, name: &str) -> Coded<Expr> {
        self.reject(
            WarningKind::UnknownFunction,
            name,
            format!("unknown function `{name}`"),
        )?;
        self.call_args()?;
        Ok(Expr::Num(0.0))
    }

    fn call_args(&mut self) -> Coded<Vec<Expr>> {
        let mut args = Vec::new();
        if !self.eat(&Tok::LParen) {
            return Ok(args);
        }

        if !self.eat(&Tok::RParen) {
            loop {
                args.push(self.expr()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RParen)?;
        }

        if args.len() > self.limits.args {
            return Err(CompileError::new(
                ErrorCode::Arguments,
                format!("more than {} arguments in one call", self.limits.args),
            ));
        }
        Ok(args)
    }
}

fn rejected(kind: WarningKind, name: &str) -> ErrorCode {
    match kind {
        WarningKind::UnknownFunction if name.starts_with("math.") => ErrorCode::UnknownMath,
        WarningKind::UnknownFunction => ErrorCode::UnknownFunction,
        WarningKind::UnknownQuery => ErrorCode::UnknownQuery,
        WarningKind::UnknownName => ErrorCode::UnknownName,
        WarningKind::UnknownRoot => ErrorCode::UnknownRoot,
        WarningKind::Arity => ErrorCode::Arity,
    }
}

fn binary_op(tok: &Tok) -> Option<(Bin, u8)> {
    Some(match tok {
        Tok::OrOr => (Bin::Or, 0),
        Tok::AndAnd => (Bin::And, 1),
        Tok::EqEq => (Bin::Eq, 2),
        Tok::Ne => (Bin::Ne, 2),
        Tok::Lt => (Bin::Lt, 3),
        Tok::Le => (Bin::Le, 3),
        Tok::Gt => (Bin::Gt, 3),
        Tok::Ge => (Bin::Ge, 3),
        Tok::Plus => (Bin::Add, 4),
        Tok::Minus => (Bin::Sub, 4),
        Tok::Star => (Bin::Mul, 5),
        Tok::Slash => (Bin::Div, 5),
        Tok::Percent => (Bin::Mod, 5),
        _ => return None,
    })
}

fn check_stmts(stmts: &[Stmt]) -> Coded<()> {
    stmts.iter().try_for_each(check_stmt)
}

fn check_stmt(stmt: &Stmt) -> Coded<()> {
    match stmt {
        Stmt::Expr(e) | Stmt::Return(e) => check(e),
        Stmt::Block(body) => check_stmts(body),
        Stmt::If(cond, then, otherwise) => {
            check(cond)?;
            check_stmts(then)?;
            otherwise.as_deref().map_or(Ok(()), check_stmt)
        }
    }
}

fn check(e: &Expr) -> Coded<()> {
    match e {
        Expr::Text(_) => Err(CompileError::new(
            ErrorCode::StringUse,
            "a string can only be a query argument or compared with `==` or `!=`",
        )),
        Expr::QueryCall(_, args) => args
            .iter()
            .filter(|a| !matches!(a, Expr::Text(_)))
            .try_for_each(check),
        Expr::TextEq(_, a, b) => [a, b]
            .into_iter()
            .filter(|side| !matches!(***side, Expr::Text(_)))
            .try_for_each(|side| check(side)),
        Expr::Neg(a) | Expr::Not(a) | Expr::AssignVar(_, a) | Expr::AssignTemp(_, a) => check(a),
        Expr::Binary(_, a, b) | Expr::Coalesce(a, b) => {
            check(a)?;
            check(b)
        }
        Expr::Ternary(c, a, b) => {
            check(c)?;
            check(a)?;
            check(b)
        }
        Expr::Call(_, args) => args.iter().try_for_each(check),
        Expr::Num(_) | Expr::Var(_) | Expr::Temp(_) | Expr::Query(_) => Ok(()),
    }
}

struct Eval<'s, 'a> {
    scope: &'s mut Scope<'a>,
    queries: &'s mut dyn Queries,
    id: u32,
    calls: u32,
    steps: u32,
    failed: bool,
}

impl Eval<'_, '_> {
    fn step(&mut self) -> bool {
        if self.failed {
            return false;
        }
        if self.steps == 0 || self.scope.fuel == 0 {
            self.failed = true;
            return false;
        }
        self.steps -= 1;
        self.scope.fuel -= 1;
        true
    }

    fn random(&mut self) -> f64 {
        let bits = random_bits(self.scope.seed, self.id, self.calls, self.scope.frame);
        self.calls = self.calls.wrapping_add(1);
        unit(bits)
    }

    fn random_integer(&mut self, low: f64, high: f64) -> f64 {
        let low = libm::floor(low);
        let span = libm::fmax(libm::floor(high) - low + 1.0, 1.0);
        low + libm::fmin(libm::floor(self.random() * span), span - 1.0)
    }

    fn die_roll(&mut self, count: f64, low: f64, high: f64, integer: bool) -> f64 {
        let mut sum = 0.0;
        for _ in 0..libm::fmax(count, 0.0) as u32 {
            if !self.step() {
                break;
            }
            sum += if integer {
                self.random_integer(low, high)
            } else {
                low + self.random() * (high - low)
            };
        }
        sum
    }

    fn block(&mut self, stmts: &[Stmt]) -> Option<f64> {
        for stmt in stmts {
            if let Some(v) = self.stmt(stmt) {
                return Some(v);
            }
        }
        None
    }

    fn stmt(&mut self, stmt: &Stmt) -> Option<f64> {
        match stmt {
            Stmt::Expr(e) => {
                self.expr(e);
                None
            }
            Stmt::Return(e) => Some(self.expr(e)),
            Stmt::Block(body) => self.block(body),
            Stmt::If(cond, then, otherwise) => {
                if self.expr(cond) != 0.0 {
                    self.block(then)
                } else {
                    otherwise.as_deref().and_then(|s| self.stmt(s))
                }
            }
        }
    }

    fn expr(&mut self, expr: &Expr) -> f64 {
        if !self.step() {
            return 0.0;
        }
        match expr {
            Expr::Num(n) => *n,
            Expr::Text(_) => 0.0,
            Expr::Var(s) => self.scope.variables.get(*s),
            Expr::Temp(s) => self
                .scope
                .work
                .temps
                .get(*s as usize)
                .copied()
                .unwrap_or(0.0),
            Expr::Query(s) => self.scope.work.query(*s).unwrap_or(0.0),
            Expr::QueryCall(s, args) => self.query_call(*s, args).unwrap_or(0.0),
            Expr::Neg(e) => -self.expr(e),
            Expr::Not(e) => bool_of(self.expr(e) == 0.0),
            Expr::Binary(Bin::And, a, b) => bool_of(self.expr(a) != 0.0 && self.expr(b) != 0.0),
            Expr::Binary(Bin::Or, a, b) => bool_of(self.expr(a) != 0.0 || self.expr(b) != 0.0),
            Expr::Binary(op, a, b) => {
                let x = self.expr(a);
                let y = self.expr(b);
                binary(*op, x, y)
            }
            Expr::TextEq(eq, a, b) => {
                let x = self.text(a);
                let y = self.text(b);
                bool_of((x.is_some() && x == y) == *eq)
            }
            Expr::Ternary(c, a, b) => {
                if self.expr(c) != 0.0 {
                    self.expr(a)
                } else {
                    self.expr(b)
                }
            }
            Expr::Coalesce(a, b) => {
                let supplied = match a.as_ref() {
                    Expr::Query(s) => self.scope.work.query(*s),
                    Expr::QueryCall(s, args) => self.query_call(*s, args),
                    other => Some(self.expr(other)),
                };
                match supplied {
                    Some(v) => v,
                    None => self.expr(b),
                }
            }
            Expr::AssignVar(slot, e) => {
                let v = self.expr(e);
                self.scope.variables.set(*slot, v);
                v
            }
            Expr::AssignTemp(slot, e) => {
                let v = self.expr(e);
                let temps = &mut self.scope.work.temps;
                grow_with(temps, *slot, 0.0);
                temps[*slot as usize] = v;
                v
            }
            Expr::Call(func, args) => self.call(*func, args),
        }
    }

    fn push_args(&mut self, args: &[Expr]) -> usize {
        let start = self.scope.work.args.len();
        for arg in args {
            let value = match arg {
                Expr::Text(id) => Arg::Text(*id),
                e => Arg::Number(self.expr(e)),
            };
            self.scope.work.args.push(value);
        }
        start
    }

    fn query_call(&mut self, slot: u16, args: &[Expr]) -> Option<f64> {
        let start = self.push_args(args);
        let value = self.queries.number(slot, &self.scope.work.args[start..]);
        self.scope.work.args.truncate(start);
        value
            .filter(|v| !v.is_nan())
            .or_else(|| self.scope.work.query(slot))
    }

    fn text(&mut self, expr: &Expr) -> Option<u16> {
        match expr {
            Expr::Text(id) => Some(*id),
            Expr::Query(slot) => self.queries.text(*slot, &[]),
            Expr::QueryCall(slot, args) => {
                let start = self.push_args(args);
                let value = self.queries.text(*slot, &self.scope.work.args[start..]);
                self.scope.work.args.truncate(start);
                value
            }
            other => {
                self.expr(other);
                None
            }
        }
    }

    /// `math.*` trigonometry is in degrees.
    fn call(&mut self, func: Func, args: &[Expr]) -> f64 {
        let mut a = [0.0f64; 4];
        for (slot, arg) in a.iter_mut().zip(args) {
            *slot = self.expr(arg);
        }
        match func {
            Func::DieRoll => self.die_roll(a[0], a[1], a[2], false),
            Func::DieRollInt => self.die_roll(a[0], a[1], a[2], true),
            Func::Random => a[0] + self.random() * (a[1] - a[0]),
            Func::RandomInt => self.random_integer(a[0], a[1]),
            _ => pure(func, &a, args.len()).unwrap_or(0.0),
        }
    }
}

fn pure(func: Func, a: &[f64; 4], n: usize) -> Option<f64> {
    Some(match func {
        Func::Abs => libm::fabs(a[0]),
        Func::Acos => libm::acos(clamp(a[0], -1.0, 1.0)).to_degrees(),
        Func::Asin => libm::asin(clamp(a[0], -1.0, 1.0)).to_degrees(),
        Func::Atan => libm::atan(a[0]).to_degrees(),
        Func::Atan2 => libm::atan2(a[0], a[1]).to_degrees(),
        Func::Bayer => bayer(cell(a[0]), cell(a[1])),
        Func::Ceil => libm::ceil(a[0]),
        Func::Clamp => clamp(a[0], a[1], a[2]),
        Func::Cos => libm::cos(a[0].to_radians()),
        Func::Sin => libm::sin(a[0].to_radians()),
        Func::Exp => libm::exp(a[0]),
        Func::Fbm => fbm(
            a[0],
            a[1],
            octaves(a[2]),
            if n > 3 { cell(a[3]) } else { 0 },
        ),
        Func::Floor => libm::floor(a[0]),
        Func::FloorMod => floor_mod(a[0], a[1]),
        Func::Frac => a[0] - libm::floor(a[0]),
        Func::Hash => hash(cell(a[0]), cell(a[1]), cell(a[2])),
        Func::HermiteBlend => {
            let t = a[0];
            3.0 * t * t - 2.0 * t * t * t
        }
        Func::Hsv => hsv(a[0], a[1], a[2]),
        Func::Lerp => a[0] + (a[1] - a[0]) * a[2],
        Func::LerpRotate => {
            let delta = libm::fmod(libm::fmod(a[1] - a[0], 360.0) + 540.0, 360.0) - 180.0;
            a[0] + delta * clamp(a[2], 0.0, 1.0)
        }
        Func::Ln => libm::log(a[0]),
        Func::Max => libm::fmax(a[0], a[1]),
        Func::Min => libm::fmin(a[0], a[1]),
        Func::Mod => truncating_mod(a[0], a[1]),
        Func::Noise => noise(a[0], a[1], if n > 2 { cell(a[2]) } else { 0 }),
        Func::Pow => pow(a[0], a[1]),
        Func::Round => round(a[0]),
        Func::Sign => sign(a[0]),
        Func::Smoothstep => smoothstep(a[0], a[1], a[2]),
        Func::Sqrt => libm::sqrt(a[0]),
        Func::Step => bool_of(a[1] >= a[0]),
        Func::Trunc => libm::trunc(a[0]),
        Func::DieRoll | Func::DieRollInt | Func::Random | Func::RandomInt => return None,
    })
}

type Span = (f64, f64);

const UNIT: Span = (0.0, 1.0);

fn span(lo: f64, hi: f64) -> Option<Span> {
    (lo.is_finite() && hi.is_finite() && lo <= hi).then_some((lo, hi))
}

fn hull(values: &[f64]) -> Option<Span> {
    if values.iter().any(|v| v.is_nan()) {
        return None;
    }
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    span(lo, hi)
}

fn union(a: Span, b: Span) -> Span {
    (a.0.min(b.0), a.1.max(b.1))
}

fn rising(a: Span, f: impl Fn(f64) -> f64) -> Option<Span> {
    span(f(a.0), f(a.1))
}

fn truth(a: Span) -> Span {
    if a.0 > 0.0 || a.1 < 0.0 {
        (1.0, 1.0)
    } else if a == (0.0, 0.0) {
        (0.0, 0.0)
    } else {
        UNIT
    }
}

fn add_span(a: Span, b: Span) -> Option<Span> {
    span(a.0 + b.0, a.1 + b.1)
}

fn sub_span(a: Span, b: Span) -> Option<Span> {
    span(a.0 - b.1, a.1 - b.0)
}

fn mul_span(a: Span, b: Span) -> Option<Span> {
    hull(&[a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1])
}

fn div_span(a: Span, b: Span) -> Option<Span> {
    if b == (0.0, 0.0) {
        Some((0.0, 0.0))
    } else if b.0 > 0.0 || b.1 < 0.0 {
        hull(&[a.0 / b.0, a.0 / b.1, a.1 / b.0, a.1 / b.1])
    } else {
        None
    }
}

fn mod_span(a: Span, b: Span) -> Option<Span> {
    let m = libm::fmax(libm::fabs(b.0), libm::fabs(b.1));
    let lo = if a.0 < 0.0 { libm::fmax(a.0, -m) } else { 0.0 };
    let hi = if a.1 > 0.0 { libm::fmin(a.1, m) } else { 0.0 };
    span(lo, hi)
}

fn floor_mod_span(a: Span, b: Span) -> Option<Span> {
    let m = libm::fmax(libm::fabs(b.0), libm::fabs(b.1));
    if b.0 >= 0.0 {
        let hi = if a.0 >= 0.0 { libm::fmin(a.1, m) } else { m };
        span(0.0, hi)
    } else if b.1 <= 0.0 {
        span(-m, 0.0)
    } else {
        span(-m, m)
    }
}

fn power_span(x: Span, n: u32) -> Option<Span> {
    let (lo, hi) = (pow(x.0, n as f64), pow(x.1, n as f64));
    if n % 2 == 1 || x.0 >= 0.0 {
        hull(&[lo, hi])
    } else if x.1 <= 0.0 {
        hull(&[hi, lo])
    } else {
        hull(&[0.0, lo, hi])
    }
}

fn pow_span(x: Span, y: Span) -> Option<Span> {
    if y.0 == y.1 && y.0 == libm::floor(y.0) && libm::fabs(y.0) <= 1024.0 {
        let raised = power_span(x, libm::fabs(y.0) as u32)?;
        return if y.0 < 0.0 {
            div_span((1.0, 1.0), raised)
        } else {
            Some(raised)
        };
    }
    if x.0 > 0.0 || (x.0 >= 0.0 && y.0 > 0.0) {
        hull(&[pow(x.0, y.0), pow(x.0, y.1), pow(x.1, y.0), pow(x.1, y.1)])
    } else {
        None
    }
}

struct Linear {
    time: u16,
    delta: u16,
}

impl Linear {
    fn expr(&self, e: &Expr) -> Option<[f64; 3]> {
        let constant = |v: [f64; 3]| (v[1] == 0.0 && v[2] == 0.0).then_some(v[0]);
        match e {
            Expr::Num(n) => Some([*n, 0.0, 0.0]),
            Expr::Query(slot) if *slot == self.time => Some([0.0, 1.0, 0.0]),
            Expr::Query(slot) if *slot == self.delta => Some([0.0, 0.0, 1.0]),
            Expr::Neg(a) => Some(self.expr(a)?.map(|v| -v)),
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                match op {
                    Bin::Add => Some([a[0] + b[0], a[1] + b[1], a[2] + b[2]]),
                    Bin::Sub => Some([a[0] - b[0], a[1] - b[1], a[2] - b[2]]),
                    Bin::Mul => match (constant(a), constant(b)) {
                        (Some(k), _) => Some(b.map(|v| v * k)),
                        (_, Some(k)) => Some(a.map(|v| v * k)),
                        _ => None,
                    },
                    Bin::Div => constant(b).filter(|&k| k != 0.0).map(|k| a.map(|v| v / k)),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

struct Bounds<'v> {
    variable: &'v dyn Fn(u16) -> Option<Span>,
}

impl Bounds<'_> {
    fn expr(&self, e: &Expr) -> Option<Span> {
        match e {
            Expr::Num(n) => span(*n, *n),
            Expr::Text(_) => Some((0.0, 0.0)),
            Expr::Var(slot) => (self.variable)(*slot).and_then(|(lo, hi)| span(lo, hi)),
            Expr::Neg(a) => {
                let a = self.expr(a)?;
                span(-a.1, -a.0)
            }
            Expr::Not(a) => {
                let t = truth(self.expr(a)?);
                span(1.0 - t.1, 1.0 - t.0)
            }
            Expr::Binary(op, a, b) => self.binary(*op, self.expr(a)?, self.expr(b)?),
            Expr::TextEq(..) => Some(UNIT),
            Expr::Ternary(c, a, b) => {
                let t = truth(self.expr(c)?);
                if t.0 == 1.0 {
                    self.expr(a)
                } else if t.1 == 0.0 {
                    self.expr(b)
                } else {
                    Some(union(self.expr(a)?, self.expr(b)?))
                }
            }
            Expr::Call(func, args) => {
                let mut spans = [(0.0, 0.0); 4];
                for (slot, arg) in spans.iter_mut().zip(args) {
                    *slot = self.expr(arg)?;
                }
                self.call(*func, &spans, args.len())
            }
            Expr::Temp(_)
            | Expr::Query(_)
            | Expr::QueryCall(..)
            | Expr::Coalesce(..)
            | Expr::AssignVar(..)
            | Expr::AssignTemp(..) => None,
        }
    }

    fn binary(&self, op: Bin, a: Span, b: Span) -> Option<Span> {
        if a.0 == a.1 && b.0 == b.1 {
            let v = binary(op, a.0, b.0);
            return span(v, v);
        }
        match op {
            Bin::Add => add_span(a, b),
            Bin::Sub => sub_span(a, b),
            Bin::Mul => mul_span(a, b),
            Bin::Div => div_span(a, b),
            Bin::Mod => mod_span(a, b),
            Bin::And => {
                let (t, u) = (truth(a), truth(b));
                Some((t.0 * u.0, t.1 * u.1))
            }
            Bin::Or => {
                let (t, u) = (truth(a), truth(b));
                Some((t.0.max(u.0), t.1.max(u.1)))
            }
            Bin::Lt | Bin::Le | Bin::Gt | Bin::Ge | Bin::Eq | Bin::Ne => Some(UNIT),
        }
    }

    fn call(&self, func: Func, s: &[Span; 4], n: usize) -> Option<Span> {
        if s.iter().all(|a| a.0 == a.1) {
            if let Some(v) = pure(func, &s.map(|a| a.0), n) {
                return span(v, v);
            }
        }
        let degrees = |v: f64| v.to_degrees();
        let unit = |v: f64| clamp(v, -1.0, 1.0);
        let [a, b, c, _] = *s;
        match func {
            Func::Abs if a.0 >= 0.0 => Some(a),
            Func::Abs if a.1 <= 0.0 => span(-a.1, -a.0),
            Func::Abs => span(0.0, libm::fmax(-a.0, a.1)),
            Func::Acos => span(
                degrees(libm::acos(unit(a.1))),
                degrees(libm::acos(unit(a.0))),
            ),
            Func::Asin => rising(a, |v| degrees(libm::asin(unit(v)))),
            Func::Atan => rising(a, |v| degrees(libm::atan(v))),
            Func::Atan2 => Some((-180.0, 180.0)),
            Func::Bayer => Some((0.0, 15.0 / 16.0)),
            Func::Ceil => rising(a, libm::ceil),
            Func::Floor => rising(a, libm::floor),
            Func::Round => rising(a, round),
            Func::Trunc => rising(a, libm::trunc),
            Func::Exp => rising(a, libm::exp),
            Func::Sign => rising(a, sign),
            Func::Clamp => span(
                libm::fmax(b.0, libm::fmin(c.0, a.0)),
                libm::fmax(b.1, libm::fmin(c.1, a.1)),
            ),
            Func::Cos | Func::Sin => Some((-1.0, 1.0)),
            Func::DieRoll | Func::DieRollInt => {
                let count = (
                    libm::trunc(libm::fmax(a.0, 0.0)),
                    libm::trunc(libm::fmax(a.1, 0.0)),
                );
                let draw = if func == Func::DieRoll {
                    (libm::fmin(b.0, c.0), libm::fmax(b.1, c.1))
                } else {
                    (
                        libm::floor(b.0),
                        libm::fmax(libm::floor(c.1), libm::floor(b.1)),
                    )
                };
                mul_span(count, draw)
            }
            Func::Fbm | Func::Hash | Func::Noise | Func::Frac | Func::Smoothstep | Func::Step => {
                Some(UNIT)
            }
            Func::FloorMod => floor_mod_span(a, b),
            Func::Mod => mod_span(a, b),
            Func::HermiteBlend => {
                let squared = mul_span(a, a)?;
                let cubed = mul_span(squared, a)?;
                sub_span(mul_span(squared, (3.0, 3.0))?, mul_span(cubed, (2.0, 2.0))?)
            }
            Func::Hsv => Some((0.0, 16_777_215.0)),
            Func::Lerp => add_span(a, mul_span(sub_span(b, a)?, c)?),
            Func::LerpRotate => add_span(
                a,
                mul_span(
                    (-180.0, 180.0),
                    (clamp(c.0, 0.0, 1.0), clamp(c.1, 0.0, 1.0)),
                )?,
            ),
            Func::Ln if a.0 > 0.0 => rising(a, libm::log),
            Func::Ln => None,
            Func::Max => span(libm::fmax(a.0, b.0), libm::fmax(a.1, b.1)),
            Func::Min => span(libm::fmin(a.0, b.0), libm::fmin(a.1, b.1)),
            Func::Pow => pow_span(a, b),
            Func::Random => Some(union(a, b)),
            Func::RandomInt => span(
                libm::floor(a.0),
                libm::fmax(libm::floor(b.1), libm::floor(a.1)),
            ),
            Func::Sqrt if a.0 >= 0.0 => rising(a, libm::sqrt),
            Func::Sqrt => None,
        }
    }
}

fn binary(op: Bin, x: f64, y: f64) -> f64 {
    match op {
        Bin::Add => x + y,
        Bin::Sub => x - y,
        Bin::Mul => x * y,
        Bin::Div => {
            if y == 0.0 {
                0.0
            } else {
                x / y
            }
        }
        Bin::Mod => truncating_mod(x, y),
        Bin::Lt => bool_of(x < y),
        Bin::Le => bool_of(x <= y),
        Bin::Gt => bool_of(x > y),
        Bin::Ge => bool_of(x >= y),
        Bin::Eq => bool_of(x == y),
        Bin::Ne => bool_of(x != y),
        Bin::And => bool_of(x != 0.0 && y != 0.0),
        Bin::Or => bool_of(x != 0.0 || y != 0.0),
    }
}

fn bool_of(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    libm::fmax(lo, libm::fmin(hi, x))
}

fn truncating_mod(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        libm::fmod(a, b)
    }
}

fn floor_mod(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        return 0.0;
    }
    let m = libm::fmod(a, b);
    if m != 0.0 && (m < 0.0) != (b < 0.0) {
        m + b
    } else {
        m
    }
}

fn round(x: f64) -> f64 {
    libm::floor(x + 0.5)
}

fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    if edge0 == edge1 {
        return bool_of(x >= edge0);
    }
    smooth(clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0))
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn pow(x: f64, y: f64) -> f64 {
    if y == libm::floor(y) && libm::fabs(y) <= 1024.0 {
        let mut result = 1.0;
        let mut base = x;
        let mut e = libm::fabs(y) as u32;
        while e > 0 {
            if e & 1 == 1 {
                result *= base;
            }
            base *= base;
            e >>= 1;
        }
        return if y < 0.0 { 1.0 / result } else { result };
    }
    libm::pow(x, y)
}

fn cell(x: f64) -> i32 {
    libm::floor(x) as i32
}

fn octaves(x: f64) -> i32 {
    clamp(libm::floor(x), 1.0, 8.0) as i32
}

pub fn snap(x: f64) -> i32 {
    if x.is_finite() {
        round(x) as i32
    } else {
        0
    }
}

pub fn hash_bits(x: i32, y: i32, seed: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27D4_EB2D)
        ^ (y as u32).wrapping_mul(0x1656_67B1)
        ^ (seed as u32).wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    h
}

pub fn hash(x: i32, y: i32, seed: i32) -> f64 {
    (hash_bits(x, y, seed) >> 8) as f64 / 16_777_216.0
}

pub fn noise(x: f64, y: f64, seed: i32) -> f64 {
    let x0 = cell(x);
    let y0 = cell(y);
    let fx = smooth(x - x0 as f64);
    let fy = smooth(y - y0 as f64);
    let x1 = x0.wrapping_add(1);
    let y1 = y0.wrapping_add(1);
    let lerp = |t: f64, a: f64, b: f64| a + (b - a) * t;
    let top = lerp(fx, hash(x0, y0, seed), hash(x1, y0, seed));
    let bottom = lerp(fx, hash(x0, y1, seed), hash(x1, y1, seed));
    lerp(fy, top, bottom)
}

pub fn fbm(x: f64, y: f64, octaves: i32, seed: i32) -> f64 {
    let (mut px, mut py) = (x, y);
    let mut sum = 0.0;
    let mut weight = 1.0;
    let mut total = 0.0;
    for i in 0..octaves.max(1) {
        sum += noise(px, py, seed.wrapping_add(i)) * weight;
        total += weight;
        px *= 2.0;
        py *= 2.0;
        weight /= 2.0;
    }
    sum / total
}

const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

pub fn bayer(x: i32, y: i32) -> f64 {
    BAYER[((y & 3) * 4 + (x & 3)) as usize] as f64 / 16.0
}

pub fn hsv(hue: f64, saturation: f64, value: f64) -> f64 {
    let h = if hue.is_finite() {
        floor_mod(hue, 360.0) / 60.0
    } else {
        0.0
    };
    let s = clamp(saturation, 0.0, 1.0);
    let v = clamp(value, 0.0, 1.0);
    let channel = |n: f64| {
        let k = libm::fmod(n + h, 6.0);
        let c = v - v * s * libm::fmax(0.0, libm::fmin(libm::fmin(k, 4.0 - k), 1.0));
        round(c * 255.0) as u32
    };
    ((channel(5.0) << 16) | (channel(3.0) << 8) | channel(1.0)) as f64
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    fn run(source: &str) -> f32 {
        let mut interner = Interner::default();
        let program = compile(source, &mut interner).expect(source);
        program.eval(&mut Runtime::new(&interner))
    }

    fn run64(source: &str) -> f64 {
        let mut interner = Interner::default();
        let program = compile(source, &mut interner).expect(source);
        program.run(&mut Runtime::new(&interner), &mut NoQueries)
    }

    fn strict(source: &str) -> Result<Program> {
        compile_with(
            source,
            &mut Interner::default(),
            &Options::strict(),
            &mut Warnings::default(),
        )
    }

    fn lenient(source: &str) -> (f32, Vec<(WarningKind, String)>) {
        let mut interner = Interner::default();
        let mut warnings = Warnings::default();
        let program =
            compile_with(source, &mut interner, &Options::bedrock(), &mut warnings).expect(source);
        let list = warnings.as_slice().iter().map(|w| (w.kind, w.name.clone())).collect();
        (program.eval(&mut Runtime::new(&interner)), list)
    }

    fn code(source: &str, options: &Options) -> Option<ErrorCode> {
        compile_checked(source, &mut Interner::default(), options, &mut Warnings::default())
            .err()
            .map(|e| e.code)
    }

    fn wearer_eval(program: &Program, rt: &mut Runtime, seed: u64, clock: f64) -> Outcome {
        let mut variables = [0.0f32; 4];
        let mut scope = rt.wearer_frame(&mut variables, seed, clock);
        program.evaluate(&mut scope, &mut NoQueries)
    }

    #[test]
    fn expressions_evaluate_to_their_bedrock_values() {
        let cases: &[(&str, f64)] = &[
            ("1 + 2 * 3", 7.0),
            ("7 / 2", 3.5),
            ("5 / 0", 0.0),
            ("1 == 1.0", 1.0),
            ("1 && 0 || 1", 1.0),
            ("0 ? 5 : 9", 9.0),
            ("1 + 7 % 4 * 2", 7.0),
            ("-7 % 3", -1.0),
            ("5 % 0", 0.0),
            ("math.mod(5, 0)", 0.0),
            ("math.floor_mod(-7, 3)", 2.0),
            ("math.sin(90)", 1.0),
            ("math.round(-2.5)", -2.0),
            ("math.round(0.49999999999999994)", 1.0),
            ("math.lerp(0, 10, 2)", 20.0),
            ("math.lerprotate(350, 10, 0.5)", 360.0),
            ("math.pow(10, 22)", 1e22),
            ("math.pow(2, 1024)", f64::INFINITY),
            ("math.clamp(0.5, 3, 1)", 3.0),
            ("math.frac(-1.25)", 0.75),
            ("math.smoothstep(0, 1, 0.25)", 0.15625),
            ("math.smoothstep(1, 1, 0.5)", 0.0),
            ("math.hsv(-120, 1, 1)", 255.0),
            ("math.hsv(0, 1, 0.5)", 8_388_608.0),
            ("16777217 - 16777216", 1.0),
            ("{ return 4; }; return 5;", 4.0),
            ("v.n = 2; v.n == 1 ? { return 10; } : v.n == 2 ? { return 20; } : { return 30; }", 20.0),
            ("0 ? { return 3; } : { v.a = 1; } return 4;", 4.0),
            ("v.x = 3; v.x * 2;", 0.0),
            ("0 && (v.hit = 1); 1 || (v.hit = 1); return v.hit;", 0.0),
            ("1 && (v.hit = 2); 0 || (v.hit = v.hit + 1); return v.hit;", 3.0),
            ("'Red' == 'red'", 0.0),
        ];
        for (src, want) in cases {
            assert_eq!(run64(src), *want, "{src}");
        }
        assert!(run64("math.sqrt(-1)").is_nan());
        assert_eq!(snap(run64("math.sqrt(-1)")), 0);
        assert_eq!((snap(2.5), snap(-2.5), snap(f64::INFINITY)), (3, -2, 0));
        assert_eq!(run("0.1 + 0.2"), (0.1f64 + 0.2f64) as f32);
    }

    #[test]
    fn shared_maths_are_bit_exact_across_clients() {
        let cases: [(&str, f64); 6] = [
            ("math.sin(30)", libm::sin(30f64.to_radians())),
            ("math.cos(30)", libm::cos(30f64.to_radians())),
            ("math.atan2(1, 2)", libm::atan2(1.0, 2.0).to_degrees()),
            ("math.exp(1.5)", libm::exp(1.5)),
            ("math.ln(7)", libm::log(7.0)),
            ("math.pow(2, 0.5)", libm::pow(2.0, 0.5)),
        ];
        for (src, want) in cases {
            assert_eq!(run64(src).to_bits(), want.to_bits(), "{src}");
        }
        assert_eq!(hash_bits(1, 2, 3), 0xa76a_ca30);
        assert_eq!(hash_bits(-1, -1, 7), 0x43ba_5132);
        assert_eq!(run64("math.hash(1.9, 2.2, 3.7)"), hash(1, 2, 3));
        assert_eq!(run64("math.noise(3, 4, 9)"), hash(3, 4, 9));
        let mid = run64("math.noise(3.5, 4)");
        assert!((mid - (hash(3, 4, 0) + hash(4, 4, 0)) / 2.0).abs() < 1e-12);
        let two = (noise(1.3, 2.7, 0) + 0.5 * noise(2.6, 5.4, 1)) / 1.5;
        assert_eq!(run64("math.fbm(1.3, 2.7, 2)"), two);
        assert_eq!(run64("math.bayer(2.5, 1)"), 14.0 / 16.0);

        assert_eq!(random_bits(0, 0, 0, 0), 0x1e35_cdb4_bf4f_6f16);
        assert_eq!(random_bits(u64::MAX, u32::MAX, 7, -5), 0xc172_a10e_7c6d_10a4);
        let mut interner = Interner::default();
        let program = compile(
            "v.a = math.random(0, 100); v.b = math.random_integer(1, 6); v.c = math.die_roll(3, 0, 1); v.d = math.die_roll_integer(2, 1, 6); return v.a;",
            &mut interner,
        )
        .unwrap();
        let mut rt = Runtime::new(&interner);
        let mut variables = [0.0f32; 4];
        let mut scope = rt.wearer_frame(&mut variables, 0x1234_5678_9abc_def0, 12.5);
        program.evaluate(&mut scope, &mut NoQueries);
        assert_eq!(
            variables.map(f32::to_bits),
            [1111191238, 1084227584, 1074989837, 1091567616]
        );
    }

    #[test]
    fn bounds_hold_every_value_the_expression_can_take() {
        let sources = [
            "math.abs(math.sin(v.a * 360)) * 3",
            "!(v.a > 0.5) ? v.b : -v.b",
            "math.random_integer(v.a, 4 + v.b) + math.die_roll(3, v.a, 2)",
            "math.clamp(v.a * 10 - 3, 0, 4) + math.lerp(1, 5, v.b) / (1 + v.a)",
            "math.pow(v.a + 0.5, v.b * 2.5) - math.pow(v.b - 0.5, 2)",
            "math.sqrt(v.a) + math.exp(v.b) + math.ln(1 + v.a)",
            "math.mod(v.a * 7 - 3, 2) + math.floor_mod(v.b * 9 - 4, 3)",
            "math.hermite_blend(v.a * 2 - 0.5) * math.acos(v.b) - math.atan(v.a * 9)",
            "math.lerprotate(10, 350, v.a) + math.sign(v.b - 0.5) + math.frac(v.a * 3)",
            "(v.a > 0.2 && v.b < 0.7) + math.step(0.5, v.a) + math.hash(v.a, v.b, 1) + math.noise(v.a, v.b)",
        ];
        for source in sources {
            let mut interner = Interner::default();
            compile("v.a + v.b", &mut interner).unwrap();
            let program = compile(source, &mut interner).expect(source);
            let (a, b) = (interner.variable_slot("a").unwrap(), interner.variable_slot("b").unwrap());
            let unit = |slot: u16| (slot == a || slot == b).then_some((0.0, 1.0));
            let (low, high) = program.bounds(&unit).expect(source);
            let mut rt = Runtime::new(&interner);
            for step in 0..=40u32 {
                for other in 0..=40u32 {
                    let mut variables = vec![0.0f32; interner.variable_count()];
                    variables[a as usize] = step as f32 / 40.0;
                    variables[b as usize] = other as f32 / 40.0;
                    let mut scope = rt.wearer_frame(&mut variables, u64::from(step * 41 + other), 0.0);
                    let value = program.evaluate(&mut scope, &mut NoQueries).value;
                    assert!((low..=high).contains(&value), "{source}: {value} outside {low}..{high}");
                }
            }
        }
        let mut interner = Interner::default();
        let mut bound = |source: &str| compile(source, &mut interner).expect(source).bounds(&|_| Some((0.0, 1.0)));
        assert_eq!(bound("v.a > 0.5 ? 3 : 0.5"), Some((0.5, 3.0)));
        assert_eq!(bound("v.a / (v.a - 0.5)"), None);
        assert_eq!(bound("q.anything"), None);
    }

    #[test]
    fn bedrock_tolerates_what_strict_rejects() {
        let (value, warnings) = lenient("math.nope(1) + math.nope(2) + foo + foo + bar(1) + 2");
        assert_eq!(value, 2.0);
        assert_eq!(
            warnings,
            [
                (WarningKind::UnknownFunction, "math.nope".to_string()),
                (WarningKind::UnknownName, "foo".to_string()),
                (WarningKind::UnknownFunction, "bar".to_string()),
            ]
        );
        let (value, warnings) = lenient("math.sin(90, v.x = 5) + v.x + math.pow(2)");
        assert_eq!((value, warnings[0].0), (2.0, WarningKind::Arity));
        let (value, warnings) = lenient("c.count = 3; geometry.x = 1; return context.count + c.count + geometry.x;");
        assert_eq!(value, 7.0);
        assert_eq!(warnings[1], (WarningKind::UnknownRoot, "geometry.x".to_string()));
        for src in ["math.sin(1, 2)", "c.x = 1", "foo(1)", "v", "q."] {
            assert!(strict(src).is_err(), "{src} should not compile in strict");
            assert!(compile(src, &mut Interner::default()).is_ok(), "{src} should compile in bedrock");
        }
        for src in ["'a' + 1", "v.x = 'a'", "return 'a';", "'unterminated", "loop(10, { v.x = 1; })"] {
            assert!(strict(src).is_err() && compile(src, &mut Interner::default()).is_err(), "{src}");
        }
        assert_eq!(lenient("1 != 'a'").0, 1.0);
    }

    #[test]
    fn compile_limits_and_error_codes() {
        let strict = Options::strict();
        let bedrock = Options::bedrock();
        let nested = |n: usize| format!("{}1{}", "(".repeat(n), ")".repeat(n));
        let vars = |n: usize| (0..n).map(|i| format!("v.a{i} = 1;")).collect::<String>();
        let cases: Vec<(String, &Options, Option<ErrorCode>)> = vec![
            (format!("-1{}", " + 1".repeat(127)), &strict, None),
            (format!("1{}", " + 1".repeat(128)), &strict, Some(ErrorCode::TooBig)),
            (nested(63), &strict, None),
            (nested(64), &strict, Some(ErrorCode::TooDeep)),
            (format!("{}1", "-".repeat(64)), &strict, Some(ErrorCode::TooDeep)),
            (format!("1{}", " ".repeat(4096)), &strict, Some(ErrorCode::TooLong)),
            (format!("'{}' == 'a'", "é".repeat(4000)), &strict, None),
            (vars(64), &strict, None),
            (vars(65), &strict, Some(ErrorCode::Variables)),
            (format!("{} t.extra = 1;", vars(64)), &strict, Some(ErrorCode::Variables)),
            ("q.f(1, 2, 3, 4, 5)".into(), &strict, Some(ErrorCode::Arguments)),
            ("1 # 2".into(), &strict, Some(ErrorCode::Character)),
            ("1.2.3".into(), &strict, Some(ErrorCode::Number)),
            ("'open".into(), &strict, Some(ErrorCode::Unterminated)),
            ("(1 + 2".into(), &strict, Some(ErrorCode::Unexpected)),
            ("loop(2, {})".into(), &strict, Some(ErrorCode::Unsupported)),
            ("'a' == 1".into(), &strict, Some(ErrorCode::StringUse)),
            ("math.nope(1)".into(), &strict, Some(ErrorCode::UnknownMath)),
            ("c.thing".into(), &strict, Some(ErrorCode::UnknownRoot)),
            (format!("1{}", " + 1".repeat(400)), &bedrock, None),
            (nested(300), &bedrock, Some(ErrorCode::TooDeep)),
            (vars(100), &bedrock, None),
            ("q.f(1, 2, 3, 4, 5, 6)".into(), &bedrock, None),
            (format!("1{}", "+1".repeat(1000)), &bedrock, None),
            (format!("1{}", "+1".repeat(1100)), &bedrock, Some(ErrorCode::TooDeep)),
            (format!("{}1{}{}", "(".repeat(200), "+1".repeat(900), ")".repeat(200)), &bedrock, Some(ErrorCode::TooDeep)),
            (format!("1{}", " ".repeat(16_383)), &bedrock, None),
            (format!("1{}", " ".repeat(16_384)), &bedrock, Some(ErrorCode::TooLong)),
            (format!("math.max(1{})", ",1".repeat(4096)), &bedrock, Some(ErrorCode::TooBig)),
        ];
        for (source, options, want) in &cases {
            assert_eq!(code(source, options), *want, "{}", &source[..source.len().min(30)]);
        }
        assert_eq!(compile(&format!("1{}", " + 1".repeat(300)), &mut Interner::default()).unwrap().nodes(), 601);
        let names: Vec<&str> = ErrorCode::ALL.iter().map(|c| c.as_str()).collect();
        for (i, name) in names.iter().enumerate() {
            assert!(!names[i + 1..].contains(name), "{name} listed twice");
        }
    }

    #[test]
    fn the_tallest_programs_compile_run_and_drop_on_a_small_stack() {
        let small = |bytes: usize, work: fn()| {
            thread::Builder::new().stack_size(bytes).spawn(work).unwrap().join().unwrap();
        };
        small(1 << 20, || {
            assert!(code(&format!("1{}", "+1".repeat(120_000)), &Options::bedrock()).is_some());
        });
        small(4 << 20, || {
            for source in [
                format!("1{}", "+1".repeat(1020)),
                format!("q.a{}", " ?? q.a".repeat(1020)),
                format!("{}1{}{}", "(".repeat(200), "+1".repeat(800), ")".repeat(200)),
            ] {
                let mut interner = Interner::default();
                let program = compile(&source, &mut interner).unwrap();
                let copy = program.clone();
                copy.run(&mut Runtime::new(&interner), &mut NoQueries);
            }
        });
    }

    struct Host {
        radius: u16,
        calls: Vec<(u16, Vec<Arg>)>,
        name: Option<u16>,
    }

    impl Queries for Host {
        fn number(&mut self, slot: u16, args: &[Arg]) -> Option<f64> {
            self.calls.push((slot, args.to_vec()));
            match args {
                [Arg::Text(id)] if *id == self.radius => Some(7.0),
                [Arg::Number(a), Arg::Number(b)] => Some(a * b),
                _ => None,
            }
        }

        fn text(&mut self, _slot: u16, _args: &[Arg]) -> Option<u16> {
            self.name
        }
    }

    #[test]
    fn queries_reach_the_host_and_fall_back_when_unsupplied() {
        let mut interner = Interner::default();
        let program = compile_with(
            "q.variant_param('radius') + q.scale(2, 1 + 2) + q.variant_param(q.scale(1, 1), 'x')",
            &mut interner,
            &Options::strict(),
            &mut Warnings::default(),
        )
        .unwrap();
        let param = interner.query_slot("variant_param").unwrap();
        let scale = interner.query_slot("scale").unwrap();
        let radius = interner.text_id("radius").unwrap();
        let x = interner.text_id("x").unwrap();
        let mut host = Host { radius, calls: Vec::new(), name: None };
        let mut rt = Runtime::new(&interner);
        assert_eq!(program.eval_with(&mut rt, &mut host), 13.0);
        assert_eq!(
            host.calls,
            vec![
                (param, vec![Arg::Text(radius)]),
                (scale, vec![Arg::Number(2.0), Arg::Number(3.0)]),
                (scale, vec![Arg::Number(1.0), Arg::Number(1.0)]),
                (param, vec![Arg::Number(1.0), Arg::Text(x)]),
            ]
        );

        let text = compile("q.name == 'steve' ? 1 : 2", &mut interner).unwrap();
        let steve = interner.text_id("steve").unwrap();
        let mut host = Host { radius: u16::MAX, calls: Vec::new(), name: Some(steve) };
        assert_eq!(text.eval_with(&mut Runtime::new(&interner), &mut host), 1.0);

        let fallback = compile("q.speed(1) + (q.is_on_fire ?? 7)", &mut interner).unwrap();
        let mut rt = Runtime::new(&interner);
        assert_eq!(fallback.eval(&mut rt), 7.0);
        rt.set_query(interner.query_slot("speed").unwrap(), 2.0);
        rt.set_query(interner.query_slot("is_on_fire").unwrap(), 0.0);
        assert_eq!(fallback.eval(&mut rt), 2.0);

        let sliced = compile("q.a * 10 + (q.b ?? 3)", &mut interner).unwrap();
        let mut values = vec![f32::NAN; interner.query_count()];
        values[interner.query_slot("a").unwrap() as usize] = 2.0;
        let mut rt = Runtime::new(&interner);
        let mut scope = rt.wearer_frame(&mut [], 0, 0.0);
        scope.set_queries(&values);
        assert_eq!(sliced.evaluate(&mut scope, &mut NoQueries).value, 23.0);
    }

    #[test]
    fn rolls_hash_wearer_seed_expression_and_frame() {
        let mut interner = Interner::default();
        let a = compile("math.random(0, 1)", &mut interner).unwrap();
        let b = compile("math.random(0, 1)", &mut interner).unwrap();
        let dice = compile("math.die_roll_integer(3, 1, 6)", &mut interner).unwrap();
        let mut rt = Runtime::new(&interner);
        let mut other = Runtime::new(&interner);
        let roll = wearer_eval(&a, &mut rt, 42, 3.0).value;
        assert_eq!(roll, wearer_eval(&a, &mut other, 42, 3.0).value);
        assert_eq!(roll, wearer_eval(&a, &mut rt, 42, 3.049).value);
        assert_ne!(roll, wearer_eval(&a, &mut rt, 43, 3.0).value);
        assert_ne!(roll, wearer_eval(&a, &mut rt, 42, 3.05).value);
        assert_ne!(roll, wearer_eval(&b, &mut rt, 42, 3.0).value);
        for frame in 0..100 {
            let d = wearer_eval(&dice, &mut rt, 9, frame as f64 / 20.0).value;
            assert!(d == libm::floor(d) && (3.0..=18.0).contains(&d), "{d}");
        }

        let mut rt = Runtime::new(&interner);
        let first = a.eval(&mut rt);
        assert_eq!(a.eval(&mut Runtime::new(&interner)), first);
        rt.set_clock(0.049);
        assert_eq!(a.eval(&mut rt), first);
        rt.set_clock(0.0);
        rt.set_seed(7);
        assert_ne!(a.eval(&mut rt), first);
    }

    #[test]
    fn wearer_frames_keep_variables_per_record_and_temps_for_one_frame() {
        let mut interner = Interner::default();
        let bump = compile("v.count = v.count + 1; return v.count;", &mut interner).unwrap();
        let wide = compile("v.wide = 16777217; return v.wide - 16777216;", &mut interner).unwrap();
        let write = compile("t.x = 5;", &mut interner).unwrap();
        let read = compile("t.x", &mut interner).unwrap();
        let mut rt = Runtime::new(&interner);
        let mut alex = [0.0f32; 2];
        let mut steve = [0.0f32; 2];
        for frame in 0..3 {
            let mut scope = rt.wearer_frame(&mut alex, 1, frame as f64);
            assert_eq!(bump.evaluate(&mut scope, &mut NoQueries).value, frame as f64 + 1.0);
        }
        let mut scope = rt.wearer_frame(&mut steve, 2, 3.0);
        assert_eq!(bump.evaluate(&mut scope, &mut NoQueries).value, 1.0);
        assert_eq!(wide.evaluate(&mut scope, &mut NoQueries).value, 0.0);
        write.evaluate(&mut scope, &mut NoQueries);
        assert_eq!(read.evaluate(&mut scope, &mut NoQueries).value, 5.0);
        let mut scope = rt.wearer_frame(&mut alex, 1, 3.05);
        assert_eq!(read.evaluate(&mut scope, &mut NoQueries).value, 0.0);
        assert_eq!(wide.eval(&mut rt), 1.0);

        let mut plain = Runtime::new(&interner);
        write.run(&mut plain, &mut NoQueries);
        assert_eq!(read.run(&mut plain, &mut NoQueries), 5.0);
        plain.clear_temps();
        assert_eq!(read.run(&mut plain, &mut NoQueries), 0.0);
    }

    #[test]
    fn fuel_fails_one_evaluation_and_is_shared_by_a_wearer_frame() {
        let sum = |terms: usize| format!("1{}", " + 1".repeat(terms - 1));
        let mut interner = Interner::default();
        let roll = compile("math.die_roll(1000000000, 0, 1)", &mut interner).unwrap();
        let fits = compile(&sum(700), &mut interner).unwrap();
        let constant = compile("7", &mut interner).unwrap();
        let mut rt = Runtime::new(&interner);
        let outcome = wearer_eval(&roll, &mut rt, 1, 0.0);
        assert_eq!((outcome.value, outcome.failed), (0.0, true));
        assert_eq!(roll.eval(&mut rt), 0.0);
        let mut variables = [0.0f32; 0];
        let mut scope = rt.wearer_frame(&mut variables, 1, 0.0);
        let runs: Vec<bool> = (0..4).map(|_| fits.evaluate(&mut scope, &mut NoQueries).failed).collect();
        assert_eq!(runs, [false, false, true, true]);
        assert_eq!(constant.evaluate(&mut scope, &mut NoQueries).value, 7.0);
        let mut scope = rt.wearer_frame(&mut variables, 1, 0.05);
        assert!(!fits.evaluate(&mut scope, &mut NoQueries).failed);
    }

    #[test]
    fn env_and_entity_runtime_batch_one_frame() {
        let mut env = Env::new(Rules::Bedrock);
        let init = env.compile("v.base = 10; v.offset = math.random(0, 1);").unwrap();
        let pre = env.compile("t.speed = q.speed * 2;").unwrap();
        let spin = env.compile("v.spin = v.spin + t.speed; return v.spin + v.base;").unwrap();
        let temp = env.compile("t.speed").unwrap();
        let roll = env.compile("math.random(0, 1)").unwrap();
        let hang = env.compile("math.die_roll(1000000000, 0, 1)").unwrap();
        assert_eq!((pre, hang), (1, 5));
        assert_eq!(env.query_id("Q.Speed"), Some(0));
        assert_eq!(env.variable_id("variable.spin"), Some(2));
        let speed = env.query_id("speed").unwrap() as usize;
        let base = env.variable_id("base").unwrap();
        let offset = env.variable_id("offset").unwrap();

        let mut pet = EntityRuntime::new(&env, 5);
        let mut late = EntityRuntime::new(&env, 5);
        late.evaluate(&env, &[], &[], 999.0, &mut [], &mut []);
        assert_eq!(pet.initialize(&env, &[init]), 0);
        late.initialize(&env, &[init]);
        assert_eq!(pet.variable(base), 10.0);
        assert_eq!(pet.variable(offset), late.variable(offset));

        let mut queries = vec![0.0f32; env.query_count()];
        queries[speed] = 1.5;
        let ids = [pre, spin, temp, roll, hang, 99];
        let mut out = [f32::NAN; 6];
        let mut failed = [false; 6];
        assert_eq!(pet.evaluate(&env, &ids, &queries, 1.0, &mut out, &mut failed), 1);
        assert_eq!(out[..3], [0.0, 13.0, 3.0]);
        assert_eq!((out[4], out[5]), (0.0, 0.0));
        assert_eq!(failed, [false, false, false, false, true, false]);
        let mut again = [0.0f32; 3];
        pet.evaluate(&env, &ids[1..4], &queries, 1.0, &mut again, &mut []);
        assert_eq!(again, [13.0, 0.0, out[3]]);
        let mut cousin = [0.0f32];
        EntityRuntime::new(&env, 6).evaluate(&env, &[roll], &queries, 1.0, &mut cousin, &mut []);
        assert_ne!(cousin[0], out[3]);

        env.compile("v.after = 1;").unwrap();
        pet.fit(&env);
        pet.set_variable(env.variable_id("after").unwrap(), 5.0);
        assert_eq!(pet.variable(env.variable_id("after").unwrap()), 5.0);
    }
}
