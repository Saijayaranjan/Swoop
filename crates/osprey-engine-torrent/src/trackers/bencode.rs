//! Minimal bencode codec (BEP-3) used for tracker responses and for re-wrapping an `info`
//! dictionary with a chosen tracker list. It is deliberately small: no serde, no borrowing,
//! bounded recursion, and it never panics on malformed input.

use std::collections::BTreeMap;
use std::fmt;

/// Maximum nesting depth accepted while decoding (tracker responses are flat; torrents are
/// at most a few levels deep).
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Dict(BTreeMap<Vec<u8>, Value>),
    /// Pre-encoded bytes, emitted verbatim by [`encode`]. Never produced by [`decode`].
    Raw(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BencodeError {
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for BencodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bencode error at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for BencodeError {}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(d) => d.get(key.as_bytes()),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<String> {
        self.as_bytes()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_dict(&self) -> Option<&BTreeMap<Vec<u8>, Value>> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
    /// Integer lookup that tolerates trackers encoding numbers as strings.
    pub fn get_int(&self, key: &str) -> Option<i64> {
        let v = self.get(key)?;
        v.as_int()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    }
}

/// Decode a complete bencoded document. Trailing bytes are rejected.
pub fn decode(input: &[u8]) -> Result<Value, BencodeError> {
    let (value, used) = decode_prefix(input)?;
    if used != input.len() {
        return Err(BencodeError {
            offset: used,
            message: "trailing data after value",
        });
    }
    Ok(value)
}

/// Decode a value at the start of `input`, returning it and the number of bytes consumed.
pub fn decode_prefix(input: &[u8]) -> Result<(Value, usize), BencodeError> {
    let mut parser = Parser { buf: input, pos: 0 };
    let v = parser.value(0)?;
    Ok((v, parser.pos))
}

struct Parser<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, message: &'static str) -> BencodeError {
        BencodeError {
            offset: self.pos,
            message,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    fn value(&mut self, depth: usize) -> Result<Value, BencodeError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.peek() {
            Some(b'i') => {
                self.pos += 1;
                let n = self.integer_until(b'e')?;
                Ok(Value::Int(n))
            }
            Some(b'l') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    match self.peek() {
                        Some(b'e') => {
                            self.pos += 1;
                            return Ok(Value::List(items));
                        }
                        Some(_) => items.push(self.value(depth + 1)?),
                        None => return Err(self.err("unterminated list")),
                    }
                }
            }
            Some(b'd') => {
                self.pos += 1;
                let mut map = BTreeMap::new();
                loop {
                    match self.peek() {
                        Some(b'e') => {
                            self.pos += 1;
                            return Ok(Value::Dict(map));
                        }
                        Some(c) if c.is_ascii_digit() => {
                            let key = self.byte_string()?;
                            let val = self.value(depth + 1)?;
                            map.insert(key, val);
                        }
                        Some(_) => return Err(self.err("dictionary key must be a string")),
                        None => return Err(self.err("unterminated dictionary")),
                    }
                }
            }
            Some(c) if c.is_ascii_digit() => Ok(Value::Bytes(self.byte_string()?)),
            Some(_) => Err(self.err("unexpected byte")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    /// Parse an optionally negative decimal integer terminated by `term` (consumed).
    fn integer_until(&mut self, term: u8) -> Result<i64, BencodeError> {
        let start = self.pos;
        let mut negative = false;
        if self.peek() == Some(b'-') {
            negative = true;
            self.pos += 1;
        }
        let mut value: i64 = 0;
        let mut digits = 0usize;
        while let Some(c) = self.peek() {
            if c == term {
                break;
            }
            if !c.is_ascii_digit() {
                return Err(self.err("invalid integer"));
            }
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(i64::from(c - b'0')))
                .ok_or_else(|| self.err("integer overflow"))?;
            digits += 1;
            self.pos += 1;
        }
        if self.peek() != Some(term) {
            self.pos = start;
            return Err(self.err("unterminated integer"));
        }
        self.pos += 1;
        if digits == 0 {
            return Err(self.err("integer without digits"));
        }
        Ok(if negative { -value } else { value })
    }

    fn byte_string(&mut self) -> Result<Vec<u8>, BencodeError> {
        let len = self.integer_until(b':')?;
        let len = usize::try_from(len).map_err(|_| self.err("negative string length"))?;
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| self.err("string length overflow"))?;
        if end > self.buf.len() {
            return Err(self.err("string extends past end of input"));
        }
        let s = self.buf[self.pos..end].to_vec();
        self.pos = end;
        Ok(s)
    }
}

/// Encode a value. Dictionary keys are emitted in byte order as the spec requires.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(value, &mut out);
    out
}

fn encode_into(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Int(i) => {
            out.push(b'i');
            out.extend_from_slice(i.to_string().as_bytes());
            out.push(b'e');
        }
        Value::Bytes(b) => {
            out.extend_from_slice(b.len().to_string().as_bytes());
            out.push(b':');
            out.extend_from_slice(b);
        }
        Value::List(l) => {
            out.push(b'l');
            for v in l {
                encode_into(v, out);
            }
            out.push(b'e');
        }
        Value::Dict(d) => {
            out.push(b'd');
            for (k, v) in d {
                encode_into(&Value::Bytes(k.clone()), out);
                encode_into(v, out);
            }
            out.push(b'e');
        }
        Value::Raw(r) => out.extend_from_slice(r),
    }
}

/// Convenience for building string-keyed dictionaries.
pub fn dict(entries: Vec<(&str, Value)>) -> Value {
    Value::Dict(
        entries
            .into_iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v))
            .collect(),
    )
}

pub fn bytes(s: &str) -> Value {
    Value::Bytes(s.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_scalars_and_containers() {
        assert_eq!(decode(b"i42e").unwrap(), Value::Int(42));
        assert_eq!(decode(b"i-7e").unwrap(), Value::Int(-7));
        assert_eq!(decode(b"4:spam").unwrap(), Value::Bytes(b"spam".to_vec()));
        assert_eq!(decode(b"0:").unwrap(), Value::Bytes(Vec::new()));
        assert_eq!(
            decode(b"l4:spami1ee").unwrap(),
            Value::List(vec![Value::Bytes(b"spam".to_vec()), Value::Int(1)])
        );
        let d = decode(b"d3:cow3:moo4:spam4:eggse").unwrap();
        assert_eq!(d.get("cow").and_then(Value::as_str).as_deref(), Some("moo"));
        assert_eq!(
            d.get("spam").and_then(Value::as_str).as_deref(),
            Some("eggs")
        );
        assert!(d.get("missing").is_none());
    }

    #[test]
    fn tracker_response_shape() {
        let body =
            b"d8:completei12e10:incompletei3e8:intervali1800e5:peers6:\x7f\x00\x00\x01\x1a\xe1e";
        let v = decode(body).unwrap();
        assert_eq!(v.get_int("complete"), Some(12));
        assert_eq!(v.get_int("incomplete"), Some(3));
        assert_eq!(v.get_int("interval"), Some(1800));
        assert_eq!(
            v.get("peers").and_then(Value::as_bytes).map(|b| b.len()),
            Some(6)
        );
        let err = decode(b"d14:failure reason9:forbiddene").unwrap();
        assert_eq!(
            err.get("failure reason").and_then(Value::as_str).as_deref(),
            Some("forbidden")
        );
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            &b"i42"[..],
            b"ie",
            b"i--1e",
            b"5:abc",
            b"l",
            b"d3:abce",
            b"di1e3:abce",
            b"i42ex",
            b"-1:",
            b"99999999999999999999:a",
            b"",
        ] {
            assert!(decode(bad).is_err(), "{bad:?} should fail");
        }
        let deep = "l".repeat(100) + &"e".repeat(100);
        assert!(decode(deep.as_bytes()).is_err());
    }

    #[test]
    fn encodes_sorted_and_roundtrips() {
        let v = dict(vec![
            ("zebra", Value::Int(1)),
            ("apple", bytes("x")),
            ("list", Value::List(vec![Value::Int(2), bytes("y")])),
        ]);
        let enc = encode(&v);
        assert_eq!(&enc, b"d5:apple1:x4:listli2e1:ye5:zebrai1ee");
        assert_eq!(decode(&enc).unwrap(), v);
        let raw = dict(vec![("info", Value::Raw(b"d1:ai1ee".to_vec()))]);
        assert_eq!(&encode(&raw), b"d4:infod1:ai1eee");
    }

    #[test]
    fn decode_prefix_reports_consumed_bytes() {
        let (v, used) = decode_prefix(b"i1etrailing").unwrap();
        assert_eq!(v, Value::Int(1));
        assert_eq!(used, 3);
        assert!(decode(b"i1etrailing").is_err());
    }
}
