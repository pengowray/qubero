//! A JSON reader that keeps the place of everything it reads.
//!
//! Formats embed JSON: a safetensors file is a header of it and then the
//! weights it describes. Reading such a header with a library would give the
//! values and lose where in the file each one sits, and a hex editor needs
//! both. So this parses by hand and hands back the byte range of every value,
//! counted from the start of the text it was given.
//!
//! Numbers come back as integers when they are written as integers and fit,
//! since a length or an offset read as a float would lose digits at the sizes
//! these files reach.

/// One JSON value and where its text sits: `start..end` are byte offsets from
/// the start of the text parsed, and cover the value alone. A member's key and
/// the punctuation around it are outside its value's range.
#[derive(Debug, Clone, PartialEq)]
pub struct Val {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Members in the order the file wrote them, which is the order the
    /// offsets in a safetensors header run in.
    Object(Vec<(String, Val)>),
    Array(Vec<Val>),
    Text(String),
    Int(i128),
    Float(f64),
    Bool(bool),
    Null,
}

impl Kind {
    /// What to call this in a type column.
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Object(_) => "object",
            Kind::Array(_) => "array",
            Kind::Text(_) => "string",
            Kind::Int(_) | Kind::Float(_) => "number",
            Kind::Bool(_) => "bool",
            Kind::Null => "null",
        }
    }
}

impl Val {
    /// The values inside this one, in order, with the name each is known by:
    /// a member's key, or an element's index written out.
    pub fn children(&self) -> Vec<(String, &Val)> {
        match &self.kind {
            Kind::Object(m) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
            Kind::Array(a) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
            _ => Vec::new(),
        }
    }

    pub fn child_count(&self) -> usize {
        match &self.kind {
            Kind::Object(m) => m.len(),
            Kind::Array(a) => a.len(),
            _ => 0,
        }
    }

    pub fn child(&self, i: usize) -> Option<&Val> {
        match &self.kind {
            Kind::Object(m) => m.get(i).map(|(_, v)| v),
            Kind::Array(a) => a.get(i),
            _ => None,
        }
    }

    /// What child `i` is called: its key, or its index written out.
    pub fn child_name(&self, i: usize) -> Option<String> {
        match &self.kind {
            Kind::Object(m) => m.get(i).map(|(k, _)| k.clone()),
            Kind::Array(a) => (i < a.len()).then(|| i.to_string()),
            _ => None,
        }
    }

    /// Which child is called `name`: a key for an object, an index written as
    /// a number for an array. This is what lets a template say
    /// `data_offsets.0` and reach the first of the two.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        match &self.kind {
            Kind::Object(m) => m.iter().position(|(k, _)| k == name),
            Kind::Array(a) => name.parse::<usize>().ok().filter(|i| *i < a.len()),
            _ => None,
        }
    }
}

/// How deep one value may sit inside another. Deeper than this is beyond
/// anything a file format writes, and following it would run the stack out.
const MAX_DEPTH: u32 = 64;

/// Read one JSON value from `text`, which may be followed by whitespace and
/// nothing else. The error says what was expected and where.
pub fn parse(text: &[u8]) -> Result<Val, String> {
    let mut p = Parser { b: text, at: 0 };
    let v = p.value(0)?;
    p.spaces();
    if p.at < p.b.len() {
        return Err(p.wrong("nothing after the end of the value"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn wrong(&self, want: &str) -> String {
        match self.b.get(self.at) {
            Some(c) => format!("expected {want} at byte {}, found {:?}", self.at, *c as char),
            None => format!("expected {want}, and the text ends at byte {}", self.at),
        }
    }

    fn spaces(&mut self) {
        while matches!(self.b.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.b.get(self.at) == Some(&c) {
            self.at += 1;
            return true;
        }
        false
    }

    fn value(&mut self, depth: u32) -> Result<Val, String> {
        if depth > MAX_DEPTH {
            return Err(format!("values nested more than {MAX_DEPTH} deep at byte {}", self.at));
        }
        self.spaces();
        let start = self.at;
        let kind = match self.b.get(self.at) {
            Some(b'{') => self.object(depth)?,
            Some(b'[') => self.array(depth)?,
            Some(b'"') => Kind::Text(self.string()?),
            Some(b't') => self.word(b"true", Kind::Bool(true))?,
            Some(b'f') => self.word(b"false", Kind::Bool(false))?,
            Some(b'n') => self.word(b"null", Kind::Null)?,
            _ => self.number()?,
        };
        Ok(Val { kind, start, end: self.at })
    }

    fn word(&mut self, want: &[u8], kind: Kind) -> Result<Kind, String> {
        if self.b[self.at..].starts_with(want) {
            self.at += want.len();
            return Ok(kind);
        }
        Err(self.wrong(&String::from_utf8_lossy(want)))
    }

    fn object(&mut self, depth: u32) -> Result<Kind, String> {
        self.at += 1; // {
        let mut members = Vec::new();
        self.spaces();
        if self.eat(b'}') {
            return Ok(Kind::Object(members));
        }
        loop {
            self.spaces();
            let key = self.string()?;
            self.spaces();
            if !self.eat(b':') {
                return Err(self.wrong("a colon after the key"));
            }
            members.push((key, self.value(depth + 1)?));
            self.spaces();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(Kind::Object(members));
            }
            return Err(self.wrong("a comma or the end of the object"));
        }
    }

    fn array(&mut self, depth: u32) -> Result<Kind, String> {
        self.at += 1; // [
        let mut items = Vec::new();
        self.spaces();
        if self.eat(b']') {
            return Ok(Kind::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.spaces();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Ok(Kind::Array(items));
            }
            return Err(self.wrong("a comma or the end of the array"));
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if !self.eat(b'"') {
            return Err(self.wrong("a string"));
        }
        // Gathered as bytes: what the file holds is UTF-8, and an escape is
        // written back as the UTF-8 of the character it stands for.
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.at) else {
                return Err(self.wrong("the end of the string"));
            };
            self.at += 1;
            match c {
                b'"' => return Ok(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let Some(&e) = self.b.get(self.at) else {
                        return Err(self.wrong("what the backslash escapes"));
                    };
                    self.at += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let ch = self.escape()?;
                            out.extend_from_slice(ch.encode_utf8(&mut [0u8; 4]).as_bytes());
                        }
                        _ => return Err(format!("\\{} is not an escape, at byte {}", e as char, self.at - 1)),
                    }
                }
                _ => out.push(c),
            }
        }
    }

    /// The four hex digits after a `\u`, and the low half of a surrogate pair
    /// when this is the high half.
    fn escape(&mut self) -> Result<char, String> {
        let n = self.hex4()?;
        if !(0xd800..0xdc00).contains(&n) {
            return char::from_u32(n).ok_or_else(|| format!("\\u{n:04x} is not a character, at byte {}", self.at - 4));
        }
        if !(self.eat(b'\\') && self.eat(b'u')) {
            return Err(self.wrong("the second half of a surrogate pair"));
        }
        let low = self.hex4()?;
        if !(0xdc00..0xe000).contains(&low) {
            return Err(format!("\\u{low:04x} is not the second half of a surrogate pair, at byte {}", self.at - 4));
        }
        let c = 0x10000 + ((n - 0xd800) << 10) + (low - 0xdc00);
        char::from_u32(c).ok_or_else(|| format!("\\u{n:04x}\\u{low:04x} is not a character, at byte {}", self.at - 10))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let Some(digits) = self.b.get(self.at..self.at + 4) else {
            return Err(self.wrong("four hex digits"));
        };
        let text = std::str::from_utf8(digits).map_err(|_| self.wrong("four hex digits"))?;
        let n = u32::from_str_radix(text, 16).map_err(|_| self.wrong("four hex digits"))?;
        self.at += 4;
        Ok(n)
    }

    fn number(&mut self) -> Result<Kind, String> {
        let start = self.at;
        let mut float = false;
        if matches!(self.b.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        while let Some(&c) = self.b.get(self.at) {
            match c {
                b'0'..=b'9' => {}
                b'.' | b'e' | b'E' => float = true,
                b'-' | b'+' if matches!(self.b[self.at - 1], b'e' | b'E') => {}
                _ => break,
            }
            self.at += 1;
        }
        if self.at == start {
            return Err(self.wrong("a value"));
        }
        let text = std::str::from_utf8(&self.b[start..self.at]).map_err(|_| self.wrong("a number"))?;
        if !float {
            if let Ok(v) = text.parse::<i128>() {
                return Ok(Kind::Int(v));
            }
        }
        // Too many digits for an integer, or written with a point or an
        // exponent. Either way what it is worth is a float.
        match text.parse::<f64>() {
            Ok(v) => Ok(Kind::Float(v)),
            Err(_) => Err(format!("{text} is not a number, at byte {start}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(text: &str) -> Val {
        parse(text.as_bytes()).expect("parses")
    }

    #[test]
    fn a_value_knows_where_its_text_sits() {
        let v = val(r#"{"a": 1, "b": [2, 30]}"#);
        assert_eq!((v.start, v.end), (0, 22));
        let a = v.child(0).expect("first member");
        assert_eq!((a.start, a.end, &a.kind), (6, 7, &Kind::Int(1)));
        let b = v.child(1).expect("second member");
        assert_eq!((b.start, b.end), (14, 21));
        assert_eq!(b.child(1).expect("second element").start, 18);
    }

    #[test]
    fn members_keep_the_order_they_were_written_in() {
        let v = val(r#"{"z": 1, "a": 2}"#);
        let names: Vec<_> = v.children().into_iter().map(|(k, _)| k).collect();
        assert_eq!(names, ["z", "a"]);
        assert_eq!(v.index_of("a"), Some(1));
    }

    #[test]
    fn an_element_is_named_by_its_index() {
        let v = val("[10, 20]");
        assert_eq!(v.index_of("1"), Some(1));
        assert_eq!(v.index_of("2"), None);
        assert_eq!(v.children().into_iter().map(|(k, _)| k).collect::<Vec<_>>(), ["0", "1"]);
    }

    #[test]
    fn a_whole_number_stays_whole() {
        // 20 GB of weights, which is past what a float holds exactly.
        assert_eq!(val("20430698424").kind, Kind::Int(20430698424));
        assert_eq!(val("-3").kind, Kind::Int(-3));
        assert_eq!(val("1.5").kind, Kind::Float(1.5));
        assert_eq!(val("2e3").kind, Kind::Float(2000.0));
    }

    #[test]
    fn text_comes_back_with_its_escapes_undone() {
        assert_eq!(val(r#""a\"b\nA""#).kind, Kind::Text("a\"b\nA".into()));
        assert_eq!(val(r#""😀""#).kind, Kind::Text("\u{1f600}".into()));
    }

    #[test]
    fn utf8_in_the_file_survives_the_trip() {
        assert_eq!(val("\"h\u{e9}llo\"").kind, Kind::Text("h\u{e9}llo".into()));
    }

    #[test]
    fn the_empty_object_and_array_hold_nothing() {
        assert_eq!(val("{}").child_count(), 0);
        assert_eq!(val("[]").child_count(), 0);
    }

    #[test]
    fn what_went_wrong_says_where() {
        let e = parse(br#"{"a": }"#).expect_err("no value");
        assert!(e.contains("byte 6"), "{e}");
        let e = parse(br#"{"a": 1"#).expect_err("no end");
        assert!(e.contains("comma"), "{e}");
        let e = parse(br#"{"a": 1} tail"#).expect_err("trailing");
        assert!(e.contains("nothing after"), "{e}");
    }

    #[test]
    fn nesting_stops_before_the_stack_does() {
        let deep = "[".repeat(500) + &"]".repeat(500);
        assert!(parse(deep.as_bytes()).expect_err("too deep").contains("nested"));
    }
}
