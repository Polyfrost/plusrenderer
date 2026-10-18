//! MoLang, compiled once with names interned to slots and evaluated per frame without allocation.
//! Rejected at compile time: `loop`, `for_each`, arrays, `context.` and string literals.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "molang: {}", self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// Shared by every program of one model, so they all see the same `variable.` storage.
#[derive(Debug, Default)]
pub struct Interner {
    index: [HashMap<String, u16>; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Space {
    Var,
    Temp,
    Query,
}

impl Interner {
    fn intern(&mut self, space: Space, name: &str) -> u16 {
        let map = &mut self.index[space as usize];
        let next = map.len() as u16;
        *map.entry(name.to_string()).or_insert(next)
    }

    fn count(&self, space: Space) -> usize {
        self.index[space as usize].len()
    }

    pub fn query_slot(&self, name: &str) -> Option<u16> {
        self.index[Space::Query as usize].get(name).copied()
    }
}

/// `variables` persist across frames, `temps` do not. The host writes `queries` before each
/// frame.
#[derive(Debug, Clone)]
pub struct Runtime {
    variables: Vec<f32>,
    temps: Vec<f32>,
    queries: Vec<f32>,
    rng: u64,
}

impl Runtime {
    /// Fixed RNG seed, so every backend produces identical output.
    pub fn new(interner: &Interner) -> Runtime {
        Runtime {
            variables: vec![0.0; interner.count(Space::Var)],
            temps: vec![0.0; interner.count(Space::Temp)],
            // NaN marks a query the host never supplied, for `??`.
            queries: vec![f32::NAN; interner.count(Space::Query)],
            rng: 0x2545_f491_4f6c_dd1d,
        }
    }

    pub fn set_query(&mut self, slot: u16, value: f32) {
        grow_with(&mut self.queries, slot, f32::NAN);
        self.queries[slot as usize] = value;
    }

    fn next_random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u32 << 24) as f32
    }
}

fn grow_with(v: &mut Vec<f32>, slot: u16, fill: f32) {
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
    Ceil,
    Clamp,
    Cos,
    DieRoll,
    DieRollInt,
    Exp,
    Floor,
    HermiteBlend,
    Lerp,
    LerpRotate,
    Ln,
    Max,
    Min,
    Mod,
    Pow,
    Random,
    RandomInt,
    Round,
    Sin,
    Sqrt,
    Trunc,
}

impl Func {
    fn lookup(name: &str) -> Option<(Func, usize)> {
        Some(match name {
            "abs" => (Func::Abs, 1),
            "acos" => (Func::Acos, 1),
            "asin" => (Func::Asin, 1),
            "atan" => (Func::Atan, 1),
            "atan2" => (Func::Atan2, 2),
            "ceil" => (Func::Ceil, 1),
            "clamp" => (Func::Clamp, 3),
            "cos" => (Func::Cos, 1),
            "die_roll" => (Func::DieRoll, 3),
            "die_roll_integer" => (Func::DieRollInt, 3),
            "exp" => (Func::Exp, 1),
            "floor" => (Func::Floor, 1),
            "hermite_blend" => (Func::HermiteBlend, 1),
            "lerp" => (Func::Lerp, 3),
            "lerprotate" => (Func::LerpRotate, 3),
            "ln" => (Func::Ln, 1),
            "max" => (Func::Max, 2),
            "min" => (Func::Min, 2),
            "mod" => (Func::Mod, 2),
            "pow" => (Func::Pow, 2),
            "random" => (Func::Random, 2),
            "random_integer" => (Func::RandomInt, 2),
            "round" => (Func::Round, 1),
            "sin" => (Func::Sin, 1),
            "sqrt" => (Func::Sqrt, 1),
            "trunc" => (Func::Trunc, 1),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
enum Expr {
    Num(f32),
    Var(u16),
    Temp(u16),
    Query(u16),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Binary(Bin, Box<Expr>, Box<Expr>),
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
}

#[derive(Debug, Clone)]
pub struct Program {
    stmts: Vec<Stmt>,
    /// A bare expression yields its value. A statement list yields 0 unless it returns.
    single: bool,
}

impl Program {
    pub fn number(value: f32) -> Program {
        Program {
            stmts: vec![Stmt::Expr(Expr::Num(value))],
            single: true,
        }
    }

    pub fn eval(&self, rt: &mut Runtime) -> f32 {
        rt.temps.fill(0.0);

        for stmt in &self.stmts {
            match stmt {
                Stmt::Return(e) => return eval(e, rt),
                Stmt::Expr(e) => {
                    let v = eval(e, rt);
                    if self.single {
                        return v;
                    }
                }
            }
        }

        0.0
    }
}

pub fn compile(source: &str, interner: &mut Interner) -> Result<Program> {
    let tokens = lex(source)?;
    Parser {
        tokens,
        pos: 0,
        interner,
    }
    .program()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f32),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
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
    Comma,
    Semi,
    End,
}

fn lex(src: &str) -> Result<Vec<Tok>> {
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
            let n = text
                .parse::<f32>()
                .map_err(|_| Error(format!("bad number `{text}`")))?;
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
            (b'<', _) => (Tok::Lt, 1),
            (b'>', _) => (Tok::Gt, 1),
            (b'!', _) => (Tok::Not, 1),
            (b'?', _) => (Tok::Question, 1),
            (b':', _) => (Tok::Colon, 1),
            (b'=', _) => (Tok::Assign, 1),
            (b'(', _) => (Tok::LParen, 1),
            (b')', _) => (Tok::RParen, 1),
            (b',', _) => (Tok::Comma, 1),
            (b';', _) => (Tok::Semi, 1),
            _ => return Err(Error(format!("unexpected character `{}`", c as char))),
        };

        out.push(tok);
        i += len;
    }

    out.push(Tok::End);
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    interner: &'a mut Interner,
}

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos]
    }

    fn bump(&mut self) -> Tok {
        let t = self.tokens[self.pos].clone();
        self.pos += 1;
        t
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == tok {
            self.pos += 1;
            return true;
        }
        false
    }

    fn expect(&mut self, tok: Tok) -> Result<()> {
        if self.eat(&tok) {
            Ok(())
        } else {
            Err(Error(format!("expected {tok:?}, found {:?}", self.peek())))
        }
    }

    fn program(&mut self) -> Result<Program> {
        let mut stmts = Vec::new();
        let mut terminated = false;

        while *self.peek() != Tok::End {
            let is_return = matches!(self.peek(), Tok::Name(n) if n == "return");
            if is_return {
                self.bump();
                stmts.push(Stmt::Return(self.expr()?));
            } else {
                stmts.push(Stmt::Expr(self.expr()?));
            }

            terminated = self.eat(&Tok::Semi);
            if !terminated {
                break;
            }
        }

        self.expect(Tok::End)?;
        if stmts.is_empty() {
            return Ok(Program::number(0.0));
        }

        let single = stmts.len() == 1 && !terminated && matches!(stmts[0], Stmt::Expr(_));
        Ok(Program { stmts, single })
    }

    fn expr(&mut self) -> Result<Expr> {
        self.ternary()
    }

    fn ternary(&mut self) -> Result<Expr> {
        let cond = self.coalesce()?;
        if self.eat(&Tok::Question) {
            let a = self.expr()?;
            // `a ? b` with no else yields 0 when false.
            let b = if self.eat(&Tok::Colon) {
                self.expr()?
            } else {
                Expr::Num(0.0)
            };
            return Ok(Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Ok(cond)
    }

    fn coalesce(&mut self) -> Result<Expr> {
        let mut lhs = self.binary(0)?;
        while self.eat(&Tok::Coalesce) {
            let rhs = self.binary(0)?;
            lhs = Expr::Coalesce(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn binary(&mut self, min_level: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, level)) = binary_op(self.peek()) {
            if level < min_level {
                break;
            }
            self.bump();
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat(&Tok::Not) {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.eat(&Tok::Plus);
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr> {
        match self.bump() {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::LParen => {
                let e = self.expr()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Tok::Name(name) => self.name(&name),
            tok => Err(Error(format!("unexpected {tok:?}"))),
        }
    }

    fn name(&mut self, name: &str) -> Result<Expr> {
        let (head, rest) = name.split_once('.').unwrap_or((name, ""));
        match head {
            "math" => {
                let (func, arity) = Func::lookup(rest)
                    .ok_or_else(|| Error(format!("unknown function `{name}`")))?;
                let args = self.args()?;
                if args.len() != arity {
                    return Err(Error(format!(
                        "`{name}` takes {arity} arguments, got {}",
                        args.len()
                    )));
                }
                Ok(Expr::Call(func, args))
            }
            "variable" | "v" => {
                let slot = self.interner.intern(Space::Var, rest);
                if self.eat(&Tok::Assign) {
                    return Ok(Expr::AssignVar(slot, Box::new(self.expr()?)));
                }
                Ok(Expr::Var(slot))
            }
            "temp" | "t" => {
                let slot = self.interner.intern(Space::Temp, rest);
                if self.eat(&Tok::Assign) {
                    return Ok(Expr::AssignTemp(slot, Box::new(self.expr()?)));
                }
                Ok(Expr::Temp(slot))
            }
            "query" | "q" => {
                let slot = self.interner.intern(Space::Query, rest);
                // Query call arguments are parsed and dropped.
                if *self.peek() == Tok::LParen {
                    self.args()?;
                }
                Ok(Expr::Query(slot))
            }
            "true" => Ok(Expr::Num(1.0)),
            "false" => Ok(Expr::Num(0.0)),
            _ => Err(Error(format!("unsupported identifier `{name}`"))),
        }
    }

    fn args(&mut self) -> Result<Vec<Expr>> {
        let mut args = Vec::new();
        if !self.eat(&Tok::LParen) {
            return Ok(args);
        }
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }

        loop {
            args.push(self.expr()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }

        self.expect(Tok::RParen)?;
        Ok(args)
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
        _ => return None,
    })
}

fn eval(expr: &Expr, rt: &mut Runtime) -> f32 {
    match expr {
        Expr::Num(n) => *n,
        Expr::Var(s) => rt.variables.get(*s as usize).copied().unwrap_or(0.0),
        Expr::Temp(s) => rt.temps.get(*s as usize).copied().unwrap_or(0.0),
        Expr::Query(s) => query(rt, *s).unwrap_or(0.0),
        Expr::Neg(e) => -eval(e, rt),
        Expr::Not(e) => bool_of(eval(e, rt) == 0.0),
        Expr::Binary(op, a, b) => {
            let (x, y) = (eval(a, rt), eval(b, rt));
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
        Expr::Ternary(c, a, b) => {
            if eval(c, rt) != 0.0 {
                eval(a, rt)
            } else {
                eval(b, rt)
            }
        }
        Expr::Coalesce(a, b) => match a.as_ref() {
            Expr::Query(s) => query(rt, *s).unwrap_or_else(|| eval(b, rt)),
            _ => eval(a, rt),
        },
        Expr::AssignVar(slot, e) => {
            let v = eval(e, rt);
            grow_with(&mut rt.variables, *slot, 0.0);
            rt.variables[*slot as usize] = v;
            v
        }
        Expr::AssignTemp(slot, e) => {
            let v = eval(e, rt);
            grow_with(&mut rt.temps, *slot, 0.0);
            rt.temps[*slot as usize] = v;
            v
        }
        Expr::Call(func, args) => call(*func, args, rt),
    }
}

fn query(rt: &Runtime, slot: u16) -> Option<f32> {
    rt.queries
        .get(slot as usize)
        .copied()
        .filter(|v| !v.is_nan())
}

fn bool_of(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

/// `math.*` trigonometry is in degrees.
fn call(func: Func, args: &[Expr], rt: &mut Runtime) -> f32 {
    let mut a = [0.0f32; 3];
    for (slot, arg) in a.iter_mut().zip(args) {
        *slot = eval(arg, rt);
    }

    match func {
        Func::Abs => a[0].abs(),
        Func::Acos => a[0].clamp(-1.0, 1.0).acos().to_degrees(),
        Func::Asin => a[0].clamp(-1.0, 1.0).asin().to_degrees(),
        Func::Atan => a[0].atan().to_degrees(),
        Func::Atan2 => a[0].atan2(a[1]).to_degrees(),
        Func::Ceil => a[0].ceil(),
        Func::Clamp => a[0].clamp(a[1], a[2].max(a[1])),
        Func::Cos => a[0].to_radians().cos(),
        Func::Sin => a[0].to_radians().sin(),
        Func::DieRoll => (0..a[0].max(0.0) as u32)
            .map(|_| a[1] + rt.next_random() * (a[2] - a[1]))
            .sum(),
        Func::DieRollInt => (0..a[0].max(0.0) as u32)
            .map(|_| random_integer(a[1], a[2], rt))
            .sum(),
        Func::Exp => a[0].exp(),
        Func::Floor => a[0].floor(),
        Func::HermiteBlend => {
            let t = a[0];
            3.0 * t * t - 2.0 * t * t * t
        }
        Func::Lerp => a[0] + (a[1] - a[0]) * a[2].clamp(0.0, 1.0),
        Func::LerpRotate => {
            let delta = ((a[1] - a[0]) % 360.0 + 540.0) % 360.0 - 180.0;
            a[0] + delta * a[2].clamp(0.0, 1.0)
        }
        Func::Ln => a[0].ln(),
        Func::Max => a[0].max(a[1]),
        Func::Min => a[0].min(a[1]),
        Func::Mod => {
            if a[1] == 0.0 {
                0.0
            } else {
                a[0] % a[1]
            }
        }
        Func::Pow => a[0].powf(a[1]),
        Func::Random => a[0] + rt.next_random() * (a[1] - a[0]),
        Func::RandomInt => random_integer(a[0], a[1], rt),
        Func::Round => a[0].round(),
        Func::Sqrt => a[0].max(0.0).sqrt(),
        Func::Trunc => a[0].trunc(),
    }
}

fn random_integer(low: f32, high: f32, rt: &mut Runtime) -> f32 {
    let span = (high.floor() - low.floor() + 1.0).max(1.0);
    low.floor() + (rt.next_random() * span).floor().min(span - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> f32 {
        let mut interner = Interner::default();
        let program = compile(source, &mut interner).expect(source);
        program.eval(&mut Runtime::new(&interner))
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn arithmetic_and_precedence() {
        let cases = [
            ("1 + 2 * 3", 7.0),
            ("(1 + 2) * 3", 9.0),
            ("10 - 4 - 3", 3.0),
            ("-2 * -3", 6.0),
            ("7 / 2", 3.5),
            ("1 / 0", 0.0),
            ("2 > 1", 1.0),
            ("1 == 1.0", 1.0),
            ("!0", 1.0),
            ("1 && 0 || 1", 1.0),
            ("0 ? 5 : 9", 9.0),
            ("1 ? 5 : 9", 5.0),
            ("0 ? 5", 0.0),
        ];
        for (src, want) in cases {
            assert!(close(run(src), want), "{src} = {} want {want}", run(src));
        }
    }

    #[test]
    fn trigonometry_is_in_degrees() {
        assert!(close(run("math.sin(90)"), 1.0));
        assert!(close(run("math.cos(180)"), -1.0));
        assert!(close(run("math.asin(1)"), 90.0));
        assert!(close(run("math.atan2(1, 0)"), 90.0));
    }

    #[test]
    fn math_library() {
        let cases = [
            ("math.abs(-3)", 3.0),
            ("math.clamp(5, 0, 1)", 1.0),
            ("math.floor(1.7)", 1.0),
            ("math.ceil(1.2)", 2.0),
            ("math.round(1.5)", 2.0),
            ("math.trunc(-1.7)", -1.0),
            ("math.lerp(0, 10, 0.25)", 2.5),
            ("math.lerprotate(350, 10, 0.5)", 360.0),
            ("math.hermite_blend(0.5)", 0.5),
            ("math.mod(7, 4)", 3.0),
            ("math.pow(2, 10)", 1024.0),
            ("math.sqrt(9)", 3.0),
            ("math.min(2, 3) + math.max(2, 3)", 5.0),
        ];
        for (src, want) in cases {
            assert!(close(run(src), want), "{src} = {} want {want}", run(src));
        }
    }

    #[test]
    fn variables_persist_and_temps_do_not() {
        let mut interner = Interner::default();
        let write = compile("v.spin = v.spin + 1;", &mut interner).unwrap();
        let read = compile("v.spin", &mut interner).unwrap();
        let temp = compile("t.scratch = 5; return t.scratch;", &mut interner).unwrap();
        let temp_read = compile("t.scratch", &mut interner).unwrap();

        let mut rt = Runtime::new(&interner);
        for _ in 0..3 {
            write.eval(&mut rt);
        }
        assert_eq!(read.eval(&mut rt), 3.0);
        assert_eq!(temp.eval(&mut rt), 5.0);
        assert_eq!(temp_read.eval(&mut rt), 0.0);
    }

    #[test]
    fn short_prefixes_alias_long_ones() {
        let mut interner = Interner::default();
        let short = compile("v.a = 4;", &mut interner).unwrap();
        let long = compile("variable.a", &mut interner).unwrap();
        let mut rt = Runtime::new(&interner);
        short.eval(&mut rt);
        assert_eq!(long.eval(&mut rt), 4.0);
    }

    #[test]
    fn queries_come_from_the_host_and_default_to_zero() {
        let mut interner = Interner::default();
        let program = compile("q.anim_time * 2", &mut interner).unwrap();
        let slot = interner.query_slot("anim_time").unwrap();

        let mut rt = Runtime::new(&interner);
        assert_eq!(program.eval(&mut rt), 0.0);
        rt.set_query(slot, 1.5);
        assert_eq!(program.eval(&mut rt), 3.0);
    }

    #[test]
    fn coalesce_falls_back_only_for_unsupplied_queries() {
        let mut interner = Interner::default();
        let program = compile("q.is_on_fire ?? 7", &mut interner).unwrap();
        let slot = interner.query_slot("is_on_fire").unwrap();

        let mut rt = Runtime::new(&interner);
        assert_eq!(program.eval(&mut rt), 7.0);
        rt.set_query(slot, 0.0);
        assert_eq!(program.eval(&mut rt), 0.0);
    }

    #[test]
    fn statement_lists_need_a_return() {
        assert_eq!(run("1 + 1"), 2.0);
        assert_eq!(run("v.x = 3; v.x * 2;"), 0.0);
        assert_eq!(run("v.x = 3; return v.x * 2;"), 6.0);
    }

    #[test]
    fn random_is_deterministic_across_runs() {
        let mut interner = Interner::default();
        let program = compile("math.random(0, 100)", &mut interner).unwrap();
        let first: Vec<f32> = (0..8)
            .map(|_| program.eval(&mut Runtime::new(&interner)))
            .collect();
        assert!(first.iter().all(|v| (0.0..100.0).contains(v)));
        assert!(
            first.windows(2).all(|w| w[0] == w[1]),
            "fresh runtime, same seed"
        );

        let mut rt = Runtime::new(&interner);
        let stream: Vec<f32> = (0..8).map(|_| program.eval(&mut rt)).collect();
        assert!(stream.windows(2).any(|w| w[0] != w[1]), "must advance");
    }

    #[test]
    fn unsupported_constructs_are_rejected() {
        let mut interner = Interner::default();
        for src in [
            "loop(10, { v.x = v.x + 1; })",
            "context.entity_count",
            "math.nope(1)",
            "math.sin(1, 2)",
            "$bad",
        ] {
            assert!(
                compile(src, &mut interner).is_err(),
                "{src} should not compile"
            );
        }
    }
}
