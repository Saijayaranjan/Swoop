//! A deliberately tiny expression language for "sandboxed scripts": no I/O, no loops, no
//! recursion, bounded evaluation steps and wall time. Scripts read the task variables and return
//! a JSON value — typically `{ name: "...", directory: "..." }` — which the automation runner
//! applies as a rename/move.
//!
//! Grammar (precedence low → high):
//! ```text
//! expr    := ternary
//! ternary := or ( '?' expr ':' expr )?
//! or      := and ( '||' and )*
//! and     := eq ( '&&' eq )*
//! eq      := cmp ( ('==' | '!=') cmp )*
//! cmp     := add ( ('<' | '<=' | '>' | '>=') add )*
//! add     := mul ( ('+' | '-') mul )*
//! mul     := unary ( ('*' | '/' | '%') unary )*
//! unary   := '!' unary | '-' unary | postfix
//! postfix := primary ( '.' ident ( '(' args ')' )? )*
//! primary := number | string | 'true' | 'false' | 'null' | ident | '(' expr ')' | '{' (ident ':' expr (',' ...)* )? '}'
//! ```
//! Methods on strings: `lower()`, `upper()`, `trim()`, `contains(s)`, `startsWith(s)`,
//! `endsWith(s)`, `replace(a, b)`, `length`, `slice(start, end)`, `matches(glob)`.
//! Functions on numbers: `round()`, `floor()`. Built-in variables: everything in
//! [`swoop_domain::automation::AutomationContext::as_env`] without the `SWOOP_` prefix,
//! lower-cased (`file_name`, `directory`, `size`, …).

use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use swoop_domain::{ErrorKind, TaskError};

const MAX_SCRIPT_LEN: usize = 8 * 1024;
const MAX_STEPS: u32 = 20_000;
const MAX_STRING: usize = 64 * 1024;

pub fn evaluate(
    script: &str,
    vars: &BTreeMap<String, Value>,
    max_time: Duration,
) -> Result<Value, TaskError> {
    if script.len() > MAX_SCRIPT_LEN {
        return Err(err("script too long"));
    }
    let tokens = lex(script)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        vars,
        steps: 0,
        deadline: Instant::now() + max_time,
    };
    let v = p.expr()?;
    if p.pos != p.tokens.len() {
        return Err(err(format!("unexpected token {:?}", p.tokens[p.pos])));
    }
    Ok(v)
}

fn err(msg: impl Into<String>) -> TaskError {
    TaskError::new(ErrorKind::ParseError, format!("script: {}", msg.into()))
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, TaskError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push(Tok::Num(
                s.parse().map_err(|_| err(format!("bad number {s}")))?,
            ));
        } else if c == '"' || c == '\'' {
            let q = c;
            i += 1;
            let mut s = String::new();
            loop {
                let Some(&ch) = chars.get(i) else {
                    return Err(err("unterminated string"));
                };
                i += 1;
                if ch == '\\' {
                    let Some(&e) = chars.get(i) else {
                        return Err(err("bad escape"));
                    };
                    i += 1;
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else if ch == q {
                    break;
                } else {
                    s.push(ch);
                }
            }
            out.push(Tok::Str(s));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let op = match two.as_str() {
                "==" | "!=" | "<=" | ">=" | "&&" | "||" => Some(match two.as_str() {
                    "==" => "==",
                    "!=" => "!=",
                    "<=" => "<=",
                    ">=" => ">=",
                    "&&" => "&&",
                    _ => "||",
                }),
                _ => None,
            };
            if let Some(op) = op {
                out.push(Tok::Op(op));
                i += 2;
                continue;
            }
            let op = match c {
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '%' => "%",
                '<' => "<",
                '>' => ">",
                '!' => "!",
                '?' => "?",
                ':' => ":",
                '(' => "(",
                ')' => ")",
                '{' => "{",
                '}' => "}",
                ',' => ",",
                '.' => ".",
                _ => return Err(err(format!("unexpected character {c:?}"))),
            };
            out.push(Tok::Op(op));
            i += 1;
        }
        if out.len() > 4096 {
            return Err(err("script too complex"));
        }
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    vars: &'a BTreeMap<String, Value>,
    steps: u32,
    deadline: Instant,
}

impl Parser<'_> {
    fn step(&mut self) -> Result<(), TaskError> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(err("step limit exceeded"));
        }
        if self.steps.is_multiple_of(256) && Instant::now() > self.deadline {
            return Err(err("time limit exceeded"));
        }
        Ok(())
    }
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }
    fn eat(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if *o == op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, op: &str) -> Result<(), TaskError> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(err(format!("expected {op:?}")))
        }
    }

    fn expr(&mut self) -> Result<Value, TaskError> {
        self.step()?;
        let cond = self.or()?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(if truthy(&cond) { a } else { b });
        }
        Ok(cond)
    }
    fn or(&mut self) -> Result<Value, TaskError> {
        let mut l = self.and()?;
        while self.eat("||") {
            let r = self.and()?;
            l = Value::Bool(truthy(&l) || truthy(&r));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Value, TaskError> {
        let mut l = self.eq()?;
        while self.eat("&&") {
            let r = self.eq()?;
            l = Value::Bool(truthy(&l) && truthy(&r));
        }
        Ok(l)
    }
    fn eq(&mut self) -> Result<Value, TaskError> {
        let mut l = self.cmp()?;
        loop {
            if self.eat("==") {
                let r = self.cmp()?;
                l = Value::Bool(loose_eq(&l, &r));
            } else if self.eat("!=") {
                let r = self.cmp()?;
                l = Value::Bool(!loose_eq(&l, &r));
            } else {
                return Ok(l);
            }
        }
    }
    fn cmp(&mut self) -> Result<Value, TaskError> {
        let mut l = self.add()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) if ["<", "<=", ">", ">="].contains(o) => *o,
                _ => return Ok(l),
            };
            self.pos += 1;
            let r = self.add()?;
            let (a, b) = (num(&l)?, num(&r)?);
            l = Value::Bool(match op {
                "<" => a < b,
                "<=" => a <= b,
                ">" => a > b,
                _ => a >= b,
            });
        }
    }
    fn add(&mut self) -> Result<Value, TaskError> {
        let mut l = self.mul()?;
        loop {
            if self.eat("+") {
                let r = self.mul()?;
                l = match (&l, &r) {
                    (Value::String(a), b) | (b, Value::String(a))
                        if !(matches!(b, Value::Number(_)) && matches!(l, Value::Number(_))) =>
                    {
                        let s = if let Value::String(x) = &l {
                            format!("{x}{}", to_str(b))
                        } else {
                            format!("{a}{}", to_str(&r))
                        };
                        if s.len() > MAX_STRING {
                            return Err(err("string too long"));
                        }
                        Value::String(s)
                    }
                    _ => number(num(&l)? + num(&r)?),
                };
            } else if self.eat("-") {
                let r = self.mul()?;
                l = number(num(&l)? - num(&r)?);
            } else {
                return Ok(l);
            }
        }
    }
    fn mul(&mut self) -> Result<Value, TaskError> {
        let mut l = self.unary()?;
        loop {
            if self.eat("*") {
                let r = self.unary()?;
                l = number(num(&l)? * num(&r)?);
            } else if self.eat("/") {
                let r = self.unary()?;
                let d = num(&r)?;
                if d == 0.0 {
                    return Err(err("division by zero"));
                }
                l = number(num(&l)? / d);
            } else if self.eat("%") {
                let r = self.unary()?;
                let d = num(&r)?;
                if d == 0.0 {
                    return Err(err("division by zero"));
                }
                l = number(num(&l)? % d);
            } else {
                return Ok(l);
            }
        }
    }
    fn unary(&mut self) -> Result<Value, TaskError> {
        self.step()?;
        if self.eat("!") {
            let v = self.unary()?;
            return Ok(Value::Bool(!truthy(&v)));
        }
        if self.eat("-") {
            let v = self.unary()?;
            return Ok(number(-num(&v)?));
        }
        self.postfix()
    }
    fn postfix(&mut self) -> Result<Value, TaskError> {
        let mut v = self.primary()?;
        while self.eat(".") {
            let name = match self.tokens.get(self.pos) {
                Some(Tok::Ident(n)) => n.clone(),
                _ => return Err(err("expected member name")),
            };
            self.pos += 1;
            let mut args = Vec::new();
            if self.eat("(") && !self.eat(")") {
                loop {
                    args.push(self.expr()?);
                    if self.eat(")") {
                        break;
                    }
                    self.expect(",")?;
                }
            }
            v = call(&v, &name, &args)?;
        }
        Ok(v)
    }
    fn primary(&mut self) -> Result<Value, TaskError> {
        let tok = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or_else(|| err("unexpected end"))?;
        self.pos += 1;
        match tok {
            Tok::Num(n) => Ok(number(n)),
            Tok::Str(s) => Ok(Value::String(s)),
            Tok::Ident(id) => match id.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                "null" => Ok(Value::Null),
                _ => self
                    .vars
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| err(format!("unknown variable {id}"))),
            },
            Tok::Op("(") => {
                let v = self.expr()?;
                self.expect(")")?;
                Ok(v)
            }
            Tok::Op("{") => {
                let mut map = serde_json::Map::new();
                if !self.eat("}") {
                    loop {
                        let key = match self.tokens.get(self.pos) {
                            Some(Tok::Ident(k)) => k.clone(),
                            Some(Tok::Str(k)) => k.clone(),
                            _ => return Err(err("expected key")),
                        };
                        self.pos += 1;
                        self.expect(":")?;
                        let v = self.expr()?;
                        map.insert(key, v);
                        if self.eat("}") {
                            break;
                        }
                        self.expect(",")?;
                        if map.len() > 64 {
                            return Err(err("object too large"));
                        }
                    }
                }
                Ok(Value::Object(map))
            }
            other => Err(err(format!("unexpected {other:?}"))),
        }
    }
}

fn number(n: f64) -> Value {
    serde_json::Number::from_f64(n)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn num(v: &Value) -> Result<f64, TaskError> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| err("bad number")),
        Value::String(s) => s
            .trim()
            .parse()
            .map_err(|_| err(format!("not a number: {s:?}"))),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        _ => Err(err("not a number")),
    }
}

fn to_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::String(_)) | (Value::String(_), Value::Number(_)) => {
            num(a).ok() == num(b).ok()
        }
        _ => a == b,
    }
}

fn call(recv: &Value, name: &str, args: &[Value]) -> Result<Value, TaskError> {
    let arg = |i: usize| -> Result<String, TaskError> {
        args.get(i)
            .map(to_str)
            .ok_or_else(|| err(format!("{name}: missing argument")))
    };
    match recv {
        Value::String(s) => Ok(match name {
            "lower" => Value::String(s.to_lowercase()),
            "upper" => Value::String(s.to_uppercase()),
            "trim" => Value::String(s.trim().to_owned()),
            "length" => number(s.chars().count() as f64),
            "contains" => Value::Bool(s.contains(&arg(0)?)),
            "startsWith" => Value::Bool(s.starts_with(&arg(0)?)),
            "endsWith" => Value::Bool(s.ends_with(&arg(0)?)),
            "replace" => {
                let out = s.replace(&arg(0)?, &arg(1)?);
                if out.len() > MAX_STRING {
                    return Err(err("string too long"));
                }
                Value::String(out)
            }
            "matches" => Value::Bool(swoop_domain::rules::glob_match(&arg(0)?, s)),
            "slice" => {
                let chars: Vec<char> = s.chars().collect();
                let start = args.first().map(num).transpose()?.unwrap_or(0.0).max(0.0) as usize;
                let end = args
                    .get(1)
                    .map(num)
                    .transpose()?
                    .map(|e| e as usize)
                    .unwrap_or(chars.len())
                    .min(chars.len());
                Value::String(chars[start.min(end)..end].iter().collect())
            }
            _ => return Err(err(format!("unknown string method {name}"))),
        }),
        Value::Number(_) => {
            let n = num(recv)?;
            Ok(match name {
                "round" => number(n.round()),
                "floor" => number(n.floor()),
                "ceil" => number(n.ceil()),
                _ => return Err(err(format!("unknown number method {name}"))),
            })
        }
        Value::Object(o) => o
            .get(name)
            .cloned()
            .ok_or_else(|| err(format!("no field {name}"))),
        _ => Err(err(format!("cannot call {name} on this value"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("file_name".into(), Value::String("Report Q3.PDF".into())),
            ("extension".into(), Value::String("pdf".into())),
            ("size".into(), Value::String("1500000".into())),
            ("domain".into(), Value::String("cdn.example.com".into())),
        ])
    }

    #[test]
    fn evaluates_expressions() {
        let t = Duration::from_millis(500);
        let v = evaluate(
            "{ name: file_name.lower().replace(' ', '_'), big: size > 1000000 }",
            &vars(),
            t,
        )
        .unwrap();
        assert_eq!(v["name"], "report_q3.pdf");
        assert_eq!(v["big"], true);
        assert_eq!(
            evaluate("extension == 'pdf' ? 'Documents' : 'Other'", &vars(), t).unwrap(),
            "Documents"
        );
        assert_eq!(
            evaluate("domain.matches('*.example.com') && !false", &vars(), t).unwrap(),
            true
        );
        assert_eq!(
            evaluate("(size / 1000000).round()", &vars(), t).unwrap(),
            2.0
        );
        assert_eq!(evaluate("'a' + 1 + 'b'", &vars(), t).unwrap(), "a1b");
        assert_eq!(
            evaluate("file_name.slice(0, 6)", &vars(), t).unwrap(),
            "Report"
        );
    }

    #[test]
    fn rejects_bad_scripts() {
        let t = Duration::from_millis(500);
        assert!(evaluate("unknown_var", &vars(), t).is_err());
        assert!(evaluate("1 / 0", &vars(), t).is_err());
        assert!(evaluate("file_name.exec()", &vars(), t).is_err());
        assert!(evaluate("{ a: 1", &vars(), t).is_err());
        assert!(evaluate(&"x".repeat(9000), &vars(), t).is_err());
        // no way to grow strings without bound
        let grow = ".replace('x', 'xxxxxxxx')".repeat(7);
        assert!(evaluate(&format!("'x'{grow}"), &vars(), t).is_err());
    }
}
