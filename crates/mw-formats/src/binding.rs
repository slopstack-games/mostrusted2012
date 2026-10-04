//! UI data bindings: `Path.To.Value[argument](OP:operand, FLAG, ...)`, optionally prefixed with `~` (the
//! path is relative to the current list item). The game's code supplies the values; operators post-process
//! them in order. Seen across the UI (counts over all screens):
//!
//! - logic: `NOT`, `EQUALS:x`, `GTEQ:x`, `LTEQ:x`, `BETWEEN:a:b`, `IS_EMPTY`
//! - arithmetic: `MULT:x`, `ADD:x` / `PLUS:x`, `MINUS:x`, `DIV:x`, `MIN:x` (upper clamp), `MAX:x`,
//!   `CEIL_TO:step`, `INT`, `POS`, `LERP` with `MINVALUE` / `MAXVALUE` / `MINRANGE` / `MAXRANGE`
//! - formatting (text output): `SPEED`, `DIST`, `TIME`, `DECIMAL`, `DIGITS`, `SPEEDPOINTS`, `LONG`,
//!   `WITHORDINAL`; `AXES:X` / `AXES:Y` select a component.
//!
//! `Signals.GetSineWave[period ms]` is a 0–1 sine used for pulsing (`(MULT:8,MINUS:4)` → ±4°).

/// A bound value.
#[derive(Debug, Clone, PartialEq)]
pub enum BindValue {
    Num(f64),
    Text(String),
}

impl BindValue {
    pub fn truthy(&self) -> bool {
        match self {
            Self::Num(n) => *n != 0.0,
            Self::Text(s) => !s.is_empty(),
        }
    }

    pub fn num(&self) -> f64 {
        match self {
            Self::Num(n) => *n,
            Self::Text(s) => s.trim().parse().unwrap_or(0.0),
        }
    }
}

impl From<bool> for BindValue {
    fn from(b: bool) -> Self {
        Self::Num(b as u8 as f64)
    }
}

impl From<f64> for BindValue {
    fn from(n: f64) -> Self {
        Self::Num(n)
    }
}

impl From<f32> for BindValue {
    fn from(n: f32) -> Self {
        Self::Num(n as f64)
    }
}

impl From<String> for BindValue {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

/// One operator: name (upper case) and its `:`-separated operands.
#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub name: String,
    pub args: Vec<String>,
}

impl Op {
    fn arg(&self, i: usize) -> f64 {
        self.args.get(i).and_then(|a| a.trim().parse().ok()).unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// `~` prefix: relative to the current item of a list.
    pub relative: bool,
    pub path: String,
    /// `[...]` argument (`Signals.GetSineWave[7500]`).
    pub argument: Option<String>,
    pub ops: Vec<Op>,
}

impl Binding {
    /// Parse a binding string; `None` for an empty one.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (relative, s) = match s.strip_prefix('~') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (head, ops) = match s.find('(') {
            Some(i) => (&s[..i], s[i + 1..].trim_end().trim_end_matches(')')),
            None => (s, ""),
        };
        let (path, argument) = match head.find('[') {
            Some(i) => (head[..i].trim(), Some(head[i + 1..].trim_end_matches(']').to_string())),
            None => (head.trim(), None),
        };
        let ops = ops
            .split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(|o| {
                let mut parts = o.split(':');
                Op {
                    name: parts.next().unwrap_or_default().trim().to_ascii_uppercase(),
                    args: parts.map(|p| p.trim().to_string()).collect(),
                }
            })
            .collect();
        Some(Self { relative, path: path.to_string(), argument, ops })
    }

    pub fn has(&self, op: &str) -> bool {
        self.ops.iter().any(|o| o.name == op)
    }

    fn named(&self, op: &str) -> Option<f64> {
        self.ops.iter().find(|o| o.name == op).map(|o| o.arg(0))
    }

    /// Evaluate with `lookup(path, argument)` supplying the raw value. Unknown paths give `None`.
    pub fn eval(&self, lookup: &mut dyn FnMut(&str, Option<&str>) -> Option<BindValue>) -> Option<BindValue> {
        let mut v = lookup(&self.path, self.argument.as_deref())?;
        if self.has("LERP") {
            let (lo, hi) = (self.named("MINVALUE").unwrap_or(0.0), self.named("MAXVALUE").unwrap_or(1.0));
            let (a, b) = (self.named("MINRANGE").unwrap_or(0.0), self.named("MAXRANGE").unwrap_or(1.0));
            let t = if hi != lo { ((v.num() - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
            v = BindValue::Num(a + (b - a) * t);
        }
        for op in &self.ops {
            v = match op.name.as_str() {
                "NOT" => (!v.truthy()).into(),
                "EQUALS" => match (&v, op.args.first()) {
                    (BindValue::Text(s), Some(a)) if a.trim().parse::<f64>().is_err() => {
                        s.eq_ignore_ascii_case(a).into()
                    }
                    _ => (v.num() == op.arg(0)).into(),
                },
                "GTEQ" => (v.num() >= op.arg(0)).into(),
                "LTEQ" => (v.num() <= op.arg(0)).into(),
                "BETWEEN" => (v.num() >= op.arg(0) && v.num() <= op.arg(1)).into(),
                "IS_EMPTY" => (!v.truthy()).into(),
                "MULT" => BindValue::Num(v.num() * op.arg(0)),
                "ADD" | "PLUS" => BindValue::Num(v.num() + op.arg(0)),
                "MINUS" => BindValue::Num(v.num() - op.arg(0)),
                "DIV" if op.arg(0) != 0.0 => BindValue::Num(v.num() / op.arg(0)),
                "MIN" => BindValue::Num(v.num().min(op.arg(0))),
                "MAX" => BindValue::Num(v.num().max(op.arg(0))),
                "CEIL_TO" if op.arg(0) > 0.0 => BindValue::Num((v.num() / op.arg(0)).ceil() * op.arg(0)),
                "INT" => BindValue::Num(v.num().trunc()),
                "POS" => BindValue::Num(v.num().abs()),
                _ => v,
            };
        }
        Some(v)
    }

    /// Text for a `{binding}` placeholder: numbers print as integers unless `DECIMAL`, distances (`DIST`)
    /// with one decimal.
    pub fn format(&self, v: &BindValue) -> String {
        match v {
            BindValue::Text(s) => s.clone(),
            BindValue::Num(n) if self.has("DIST") => format!("{n:.1}"),
            BindValue::Num(n) if self.has("DECIMAL") => format!("{n:.2}"),
            BindValue::Num(n) if self.has("SPEEDPOINTS") => group_thousands(n.round() as i64),
            BindValue::Num(n) => format!("{}", n.round() as i64),
        }
    }
}

/// `12345` → `12,345`.
pub fn group_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(s: &str, v: BindValue) -> Option<BindValue> {
        Binding::parse(s)?.eval(&mut |_, _| Some(v.clone()))
    }

    #[test]
    fn parses() {
        let b = Binding::parse("~Signals.GetSineWave[7500](MULT:8, MINUS:4)").unwrap();
        assert!(b.relative);
        assert_eq!(b.path, "Signals.GetSineWave");
        assert_eq!(b.argument.as_deref(), Some("7500"));
        assert_eq!(b.ops.len(), 2);
        assert_eq!(b.ops[1], Op { name: "MINUS".into(), args: vec!["4".into()] });
        assert_eq!(Binding::parse(""), None);
    }

    #[test]
    fn evaluates() {
        // Nitrous bar fill position: amount 0.5 → 246 * 0.5 + 893, amount rounded up to 1/36.
        let v = eval("Players.LocalPlayer.NitrousState.NitrousAmount(MULT:246,ADD:893,CEIL_TO:0.0278)", 0.5.into());
        assert!((v.unwrap().num() - 1016.0).abs() < 0.01);
        assert_eq!(eval("Language.IsMetric(NOT)", true.into()), Some(false.into()));
        assert_eq!(eval("X(GTEQ:0.98)", 0.99.into()), Some(true.into()));
        let lerp = "Players.LocalPlayer.HeatLevelInt(LERP,MINVALUE:0,MAXVALUE:6,MINRANGE:0.0,MAXRANGE:1.0)";
        assert_eq!(eval(lerp, 3.0.into()), Some(0.5.into()));
        let speed = Binding::parse("Players.GetLocalCompetitor.GetPlayerSpeed(MULT:1.60934,INT,POS)").unwrap();
        let v = speed.eval(&mut |_, _| Some((-100.0).into())).unwrap();
        assert_eq!(speed.format(&v), "160");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }
}
