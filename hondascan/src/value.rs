//! What the Python package passes around as bare dict values: a number, a state text, or None.
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Num(f64),
    Text(String),
    Null,
}

/// Symbol -> decoded value, as handed to the trip, tire, map and alert maths.
pub type Named = HashMap<String, Val>;

impl Val {
    /// A decoded value as a number, or None (a signal that could not be computed, or a state text).
    pub fn num(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn is_text(&self, s: &str) -> bool {
        matches!(self, Val::Text(t) if t == s)
    }

    /// Python's `v in ('ON', 1)`.
    pub fn is_on(&self) -> bool {
        self.is_text("ON") || *self == Val::Num(1.0)
    }

    pub fn from_json(v: &Value) -> Val {
        match v {
            Value::Number(n) => Val::Num(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => Val::Text(s.clone()),
            _ => Val::Null,
        }
    }
}

pub fn named_from_json(v: &Value) -> Named {
    v.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), Val::from_json(v))).collect()).unwrap_or_default()
}

/// Python's `round(x, nd)`: the exact decimal value rounded half to even, which is what Rust's `{:.N}` does too.
pub fn round_to(x: f64, nd: usize) -> f64 {
    format!("{:.*}", nd, x).parse().unwrap_or(x)
}

/// A number for JSON output: whole values print without a fraction like Python's ints would not, so tests compare by value.
pub fn jnum(x: f64) -> Value {
    json!(x)
}

pub fn jopt(x: Option<f64>) -> Value {
    x.map_or(Value::Null, jnum)
}

/// Value equality that treats 100 and 100.0 as the same, as Python's `==` does.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        _ => a == b,
    }
}

/// Python's `sum()` of floats: CPython 3.12+ adds with Neumaier compensation, so a plain loop differs in the last digit.
pub fn pysum(xs: &[f64]) -> f64 {
    let (mut s, mut c) = (0.0_f64, 0.0_f64);
    for &x in xs {
        let t = s + x;
        if s.abs() >= x.abs() {
            c += (s - t) + x;
        } else {
            c += (x - t) + s;
        }
        s = t;
    }
    if c != 0.0 && c.is_finite() { s + c } else { s }
}

/// How Python's `json.dumps` lays a value out.
#[derive(Clone, Copy)]
pub enum Json {
    /// `separators=(',', ':')`
    Compact,
    /// the default: `", "` and `": "`
    Spaced,
    /// `indent=n`
    Indent(usize),
}

/// Python's `json.dumps(value, ...)` with its default `ensure_ascii=True`: non-ASCII becomes `\uXXXX` (surrogate pairs
/// above the BMP), which serde_json would write as raw UTF-8.
pub fn dumps(v: &Value, style: Json) -> String {
    use serde::Serialize;
    use serde_json::ser::{CompactFormatter, Formatter, PrettyFormatter, Serializer};
    use std::io;

    struct Ascii<F: Formatter>(F);

    /// Python's default separators: ", " between items and ": " after a key.
    struct Spaced;

    impl Formatter for Spaced {
        fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
            if first { Ok(()) } else { w.write_all(b", ") }
        }
        fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
            if first { Ok(()) } else { w.write_all(b", ") }
        }
        fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            w.write_all(b": ")
        }
    }

    impl<F: Formatter> Formatter for Ascii<F> {
        fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.begin_array(w)
        }
        fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.end_array(w)
        }
        fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
            self.0.begin_array_value(w, first)
        }
        fn end_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.end_array_value(w)
        }
        fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.begin_object(w)
        }
        fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.end_object(w)
        }
        fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
            self.0.begin_object_key(w, first)
        }
        fn end_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.end_object_key(w)
        }
        fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.begin_object_value(w)
        }
        fn end_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
            self.0.end_object_value(w)
        }
        fn write_string_fragment<W: ?Sized + io::Write>(&mut self, w: &mut W, fragment: &str) -> io::Result<()> {
            let mut buf = [0u16; 2];
            for c in fragment.chars() {
                if c.is_ascii() {
                    w.write_all(&[c as u8])?;
                } else {
                    for u in c.encode_utf16(&mut buf) {
                        write!(w, "\\u{u:04x}")?;
                    }
                }
            }
            Ok(())
        }
    }

    let mut out = Vec::new();
    match style {
        Json::Indent(n) => {
            let pad = " ".repeat(n);
            let mut s = Serializer::with_formatter(&mut out, Ascii(PrettyFormatter::with_indent(pad.as_bytes())));
            v.serialize(&mut s).ok();
        }
        Json::Spaced => {
            let mut s = Serializer::with_formatter(&mut out, Ascii(Spaced));
            v.serialize(&mut s).ok();
        }
        Json::Compact => {
            let mut s = Serializer::with_formatter(&mut out, Ascii(CompactFormatter));
            v.serialize(&mut s).ok();
        }
    }
    String::from_utf8(out).unwrap_or_default()
}
