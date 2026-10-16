//! Structured data trees (C-010): JSON, JSON Lines, XML, YAML, TOML and INI parsed into one
//! pre-order arena of nodes with a key, a type and a value; paths, search and pretty-printing
//! over that arena. Pure: no window, no file system.

use std::fmt;

use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

/// The formats the family decodes, one per extension group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Json,
    JsonLines,
    Xml,
    Yaml,
    Toml,
    Ini,
}

impl Format {
    /// The extensions of the family, as the contract lists them.
    pub const EXTENSIONS: &'static [&'static str] = &["json", "jsonl", "xml", "yaml", "yml", "toml", "ini"];

    /// The format a file name's extension names, if it is one of the family's.
    pub fn for_name(name: &str) -> Option<Format> {
        let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match extension.as_str() {
            "json" => Format::Json,
            "jsonl" => Format::JsonLines,
            "xml" => Format::Xml,
            "yaml" | "yml" => Format::Yaml,
            "toml" => Format::Toml,
            "ini" => Format::Ini,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Json => "JSON",
            Format::JsonLines => "JSON Lines",
            Format::Xml => "XML",
            Format::Yaml => "YAML",
            Format::Toml => "TOML",
            Format::Ini => "INI",
        }
    }
}

/// The type shown in a node's type column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Object,
    Array,
    String,
    Number,
    Integer,
    Float,
    Boolean,
    Null,
    DateTime,
    Table,
    Element,
    Attribute,
    Text,
    Comment,
    Instruction,
    Section,
    Key,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Object => "object",
            Kind::Array => "array",
            Kind::String => "string",
            Kind::Number => "number",
            Kind::Integer => "integer",
            Kind::Float => "float",
            Kind::Boolean => "boolean",
            Kind::Null => "null",
            Kind::DateTime => "date-time",
            Kind::Table => "table",
            Kind::Element => "element",
            Kind::Attribute => "attribute",
            Kind::Text => "text",
            Kind::Comment => "comment",
            Kind::Instruction => "instruction",
            Kind::Section => "section",
            Kind::Key => "key",
        }
    }

    /// Whether nodes of this kind can hold children (and open or close in the view).
    pub fn is_container(self) -> bool {
        matches!(self, Kind::Object | Kind::Array | Kind::Table | Kind::Element | Kind::Section)
    }
}

/// One row of the tree: its depth, where its subtree ends, and the three columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub depth: u32,
    /// One past the last index of this node's subtree; `ix + 1` for a leaf.
    pub end: usize,
    pub key: String,
    pub kind: Kind,
    /// The full value; containers carry their entry count.
    pub value: String,
}

/// Where and why a file does not parse. Line and column are 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

/// The parsed file: nodes in pre-order, each with the end of its subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub format: Format,
    pub nodes: Vec<Node>,
}

impl Tree {
    /// Parse `text` as `format`.
    pub fn parse(format: Format, text: &str) -> Result<Tree, ParseError> {
        let mut nodes = Vec::new();
        match format {
            Format::Json => parse_json(text, &mut nodes)?,
            Format::JsonLines => parse_json_lines(text, &mut nodes)?,
            Format::Xml => parse_xml(text, &mut nodes)?,
            Format::Yaml => parse_yaml(text, &mut nodes)?,
            Format::Toml => parse_toml(text, &mut nodes)?,
            Format::Ini => parse_ini(text, &mut nodes)?,
        }
        Ok(Tree { format, nodes })
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The deepest row's depth plus one: how many levels the tree has.
    pub fn levels(&self) -> usize {
        self.nodes.iter().map(|n| n.depth as usize + 1).max().unwrap_or(0)
    }

    /// The root rows, in order.
    pub fn roots(&self) -> Vec<usize> {
        let mut roots = Vec::new();
        let mut ix = 0;
        while ix < self.nodes.len() {
            roots.push(ix);
            ix = self.nodes[ix].end;
        }
        roots
    }

    /// The direct children of `ix`, in order.
    pub fn children(&self, ix: usize) -> Vec<usize> {
        let mut children = Vec::new();
        let end = self.nodes[ix].end;
        let mut child = ix + 1;
        while child < end {
            children.push(child);
            child = self.nodes[child].end;
        }
        children
    }

    /// The parent of `ix`: the nearest earlier row one level up.
    pub fn parent(&self, ix: usize) -> Option<usize> {
        let depth = self.nodes[ix].depth;
        if depth == 0 {
            return None;
        }
        (0..ix).rev().find(|&j| self.nodes[j].depth == depth - 1)
    }

    /// The ancestors of `ix`, root first.
    pub fn ancestors(&self, ix: usize) -> Vec<usize> {
        let mut ancestors = Vec::new();
        let mut current = ix;
        while let Some(parent) = self.parent(current) {
            ancestors.push(parent);
            current = parent;
        }
        ancestors.reverse();
        ancestors
    }

    /// The path of `ix` in the file's own notation: `$.items[2].name` for JSON, JSON Lines,
    /// YAML and TOML; `/root/child[2]/@id` for XML; `section.key` for INI.
    pub fn path(&self, ix: usize) -> String {
        let mut chain = self.ancestors(ix);
        chain.push(ix);
        match self.format {
            Format::Ini => chain.iter().map(|&i| self.nodes[i].key.as_str()).collect::<Vec<_>>().join("."),
            Format::Xml => {
                let mut path = String::new();
                for &i in &chain {
                    path.push('/');
                    path.push_str(&self.nodes[i].key);
                    if let Some(position) = self.xml_position(i) {
                        path.push_str(&format!("[{position}]"));
                    }
                }
                path
            }
            _ => {
                let mut path = String::from("$");
                for &i in chain.iter().skip(1) {
                    let node = &self.nodes[i];
                    let parent = self.parent(i).map(|p| self.nodes[p].kind);
                    if parent == Some(Kind::Array) {
                        path.push_str(&format!("[{}]", node.key));
                    } else if is_identifier(&node.key) {
                        path.push('.');
                        path.push_str(&node.key);
                    } else {
                        path.push_str(&format!("[\"{}\"]", node.key.replace('\\', "\\\\").replace('"', "\\\"")));
                    }
                }
                path
            }
        }
    }

    /// The 1-based position of an XML node among its siblings of the same key, or `None`
    /// when it is the only one (XPath leaves the predicate out then).
    fn xml_position(&self, ix: usize) -> Option<usize> {
        let siblings = match self.parent(ix) {
            Some(parent) => self.children(parent),
            None => self.roots(),
        };
        let key = &self.nodes[ix].key;
        let same: Vec<usize> = siblings.into_iter().filter(|&s| &self.nodes[s].key == key).collect();
        (same.len() > 1).then(|| same.iter().position(|&s| s == ix).unwrap_or(0) + 1)
    }

    /// The nodes whose key or value contains `text`, regardless of case, in order.
    pub fn search(&self, text: &str) -> Vec<usize> {
        if text.is_empty() {
            return Vec::new();
        }
        let needle = text.to_lowercase();
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| contains_ignoring_case(&node.key, &needle) || contains_ignoring_case(&node.value, &needle))
            .map(|(ix, _)| ix)
            .collect()
    }

    /// One line about the tree for the status row.
    pub fn summary(&self) -> String {
        format!("{}, {} nodes, {} levels", self.format.label(), self.len(), self.levels())
    }
}

/// Whether `text` contains the already lower-cased `needle`; ASCII text is compared in place.
fn contains_ignoring_case(text: &str, needle: &str) -> bool {
    if text.is_ascii() && needle.is_ascii() {
        let text = text.as_bytes();
        let needle = needle.as_bytes();
        needle.len() <= text.len()
            && text.windows(needle.len()).any(|window| window.eq_ignore_ascii_case(needle))
    } else {
        text.to_lowercase().contains(needle)
    }
}

fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The value a container shows: how many entries it holds.
pub fn entries(count: usize) -> String {
    if count == 1 { "1 entry".to_string() } else { format!("{count} entries") }
}

fn push(nodes: &mut Vec<Node>, depth: u32, key: String, kind: Kind, value: String) -> usize {
    let ix = nodes.len();
    nodes.push(Node { depth, end: ix + 1, key, kind, value });
    ix
}

/// Close a container pushed at `ix`: its subtree ends here and its value is its entry count.
fn close(nodes: &mut [Node], ix: usize, count: usize) {
    let end = nodes.len();
    nodes[ix].end = end;
    nodes[ix].value = entries(count);
}

/// How a float prints: always with a fraction or an exponent, so `1.0` stays `1.0`.
fn float_text(value: f64) -> String {
    format!("{value:?}")
}

/// The 1-based line and column of a byte offset.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

// ---- JSON and JSON Lines: a serde seed that builds the arena while the parser runs ----

/// The map key under which serde_json hands over a number's own text (`arbitrary_precision`).
const JSON_NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// Builds one value (and its subtree) straight into the arena, without an intermediate tree.
struct Builder<'a> {
    nodes: &'a mut Vec<Node>,
    key: String,
    depth: u32,
}

impl<'de, 'a> DeserializeSeed<'de> for Builder<'a> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de, 'a> Visitor<'de> for Builder<'a> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<(), E> {
        push(self.nodes, self.depth, self.key, Kind::Boolean, v.to_string());
        Ok(())
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<(), E> {
        push(self.nodes, self.depth, self.key, Kind::Number, v.to_string());
        Ok(())
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<(), E> {
        push(self.nodes, self.depth, self.key, Kind::Number, v.to_string());
        Ok(())
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<(), E> {
        push(self.nodes, self.depth, self.key, Kind::Number, float_text(v));
        Ok(())
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<(), E> {
        push(self.nodes, self.depth, self.key, Kind::String, v.to_string());
        Ok(())
    }

    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<(), E> {
        push(self.nodes, self.depth, self.key, Kind::String, v);
        Ok(())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        push(self.nodes, self.depth, self.key, Kind::Null, "null".to_string());
        Ok(())
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        self.visit_unit()
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let Builder { nodes, key, depth } = self;
        let ix = push(nodes, depth, key, Kind::Array, String::new());
        let mut count = 0;
        while seq
            .next_element_seed(Builder { nodes: &mut *nodes, key: count.to_string(), depth: depth + 1 })?
            .is_some()
        {
            count += 1;
        }
        close(nodes, ix, count);
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let Builder { nodes, key, depth } = self;
        let Some(first) = map.next_key::<String>()? else {
            let ix = push(nodes, depth, key, Kind::Object, String::new());
            close(nodes, ix, 0);
            return Ok(());
        };
        // A number arrives as a one-entry map under serde_json's private key: keep its text.
        if first == JSON_NUMBER_TOKEN {
            let text: String = map.next_value()?;
            push(nodes, depth, key, Kind::Number, text);
            return Ok(());
        }
        let ix = push(nodes, depth, key, Kind::Object, String::new());
        map.next_value_seed(Builder { nodes: &mut *nodes, key: first, depth: depth + 1 })?;
        let mut count = 1;
        while let Some(key) = map.next_key::<String>()? {
            map.next_value_seed(Builder { nodes: &mut *nodes, key, depth: depth + 1 })?;
            count += 1;
        }
        close(nodes, ix, count);
        Ok(())
    }
}

/// serde_json's message without its own " at line N column M" tail, which the error's
/// line and column carry.
fn json_message(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.rfind(" at line ") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

fn json_error(error: serde_json::Error, line_offset: usize) -> ParseError {
    ParseError {
        line: error.line() + line_offset,
        column: error.column().max(1),
        message: json_message(&error),
    }
}

fn parse_json_value(text: &str, nodes: &mut Vec<Node>, key: &str, depth: u32) -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    Builder { nodes, key: key.to_string(), depth }.deserialize(&mut deserializer)?;
    deserializer.end()
}

fn parse_json(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    parse_json_value(text, nodes, "$", 0).map_err(|e| json_error(e, 0))
}

/// One record per non-blank line under a root `$` array; a bad line is reported by its number.
fn parse_json_lines(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    let root = push(nodes, 0, "$".to_string(), Kind::Array, String::new());
    let mut count = 0;
    for (line_ix, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        parse_json_value(line, nodes, &count.to_string(), 1).map_err(|e| json_error(e, line_ix))?;
        count += 1;
    }
    close(nodes, root, count);
    Ok(())
}

/// Check that `text` is one JSON value and nothing else, with the parser's own position.
fn check_json(text: &str, line_offset: usize) -> Result<(), ParseError> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    <serde::de::IgnoredAny as serde::Deserialize>::deserialize(&mut deserializer)
        .and_then(|_| deserializer.end())
        .map_err(|e| json_error(e, line_offset))
}

/// Re-space valid JSON token by token: every string, number and literal keeps its own text
/// (duplicate keys and long numbers included). `indent` is `Some` for the pretty form,
/// `None` for one compact line.
fn respace_json(text: &str, indent: Option<&str>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() * 2);
    let mut depth = 0usize;
    let newline = |out: &mut String, depth: usize| {
        if let Some(unit) = indent {
            out.push('\n');
            for _ in 0..depth {
                out.push_str(unit);
            }
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => break,
                        _ => i += 1,
                    }
                }
                i = (i + 1).min(bytes.len());
                out.push_str(&text[start..i]);
            }
            open @ (b'{' | b'[') => {
                out.push(open as char);
                let close = if open == b'{' { b'}' } else { b']' };
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == close {
                    out.push(close as char);
                    i = j + 1;
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                    i += 1;
                }
            }
            close @ (b'}' | b']') => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(close as char);
                i += 1;
            }
            b',' => {
                out.push(',');
                newline(&mut out, depth);
                i += 1;
            }
            b':' => {
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                i += 1;
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < bytes.len() && !b"{}[],:\"".contains(&bytes[i]) && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                out.push_str(&text[start..i]);
            }
        }
    }
    out
}

// ---- YAML: the document tree, with its typed keys ----

use saphyr::{LoadableYamlNode as _, Scalar, Yaml};

/// A scalar as the tree shows it: the kind and the text.
fn yaml_scalar(scalar: &Scalar) -> (Kind, String) {
    match scalar {
        Scalar::Null => (Kind::Null, "null".to_string()),
        Scalar::Boolean(b) => (Kind::Boolean, b.to_string()),
        Scalar::Integer(i) => (Kind::Number, i.to_string()),
        Scalar::FloatingPoint(f) => (Kind::Number, yaml_float_text(f.into_inner())),
        Scalar::String(s) => (Kind::String, s.to_string()),
    }
}

/// YAML's own spelling of a float: `.inf`, `-.inf` and `.nan` for the non-finite ones.
fn yaml_float_text(value: f64) -> String {
    if value.is_nan() {
        ".nan".to_string()
    } else if value.is_infinite() {
        if value > 0. { ".inf".to_string() } else { "-.inf".to_string() }
    } else {
        float_text(value)
    }
}

/// The text of a mapping key: its scalar, or its whole form for a complex key.
fn yaml_key_text(key: &Yaml) -> String {
    match key {
        Yaml::Value(scalar) => yaml_scalar(scalar).1,
        Yaml::Representation(text, _, _) => text.to_string(),
        Yaml::Tagged(_, inner) => yaml_key_text(inner),
        other => {
            let mut out = String::new();
            let _ = saphyr::YamlEmitter::new(&mut out).dump(other);
            out.trim_start_matches("---").trim().replace('\n', " ")
        }
    }
}

fn walk_yaml(value: &Yaml, nodes: &mut Vec<Node>, key: String, depth: u32) {
    match value {
        Yaml::Value(scalar) => {
            let (kind, text) = yaml_scalar(scalar);
            push(nodes, depth, key, kind, text);
        }
        Yaml::Representation(text, _, _) => {
            push(nodes, depth, key, Kind::String, text.to_string());
        }
        Yaml::Tagged(_, inner) => walk_yaml(inner, nodes, key, depth),
        Yaml::Alias(_) | Yaml::BadValue => {
            push(nodes, depth, key, Kind::Null, "null".to_string());
        }
        Yaml::Sequence(items) => {
            let ix = push(nodes, depth, key, Kind::Array, String::new());
            for (i, item) in items.iter().enumerate() {
                walk_yaml(item, nodes, i.to_string(), depth + 1);
            }
            close(nodes, ix, items.len());
        }
        Yaml::Mapping(map) => {
            let ix = push(nodes, depth, key, Kind::Object, String::new());
            for (k, v) in map.iter() {
                walk_yaml(v, nodes, yaml_key_text(k), depth + 1);
            }
            close(nodes, ix, map.len());
        }
    }
}

fn load_yaml(text: &str) -> Result<Vec<Yaml<'_>>, ParseError> {
    Yaml::load_from_str(text).map_err(|error| {
        let marker = error.marker();
        ParseError { line: marker.line().max(1), column: marker.col() + 1, message: error.info().to_string() }
    })
}

/// One document is the root `$`; a stream of several is a root `$` with one entry each.
fn parse_yaml(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    let documents = load_yaml(text)?;
    match documents.as_slice() {
        [] => {
            push(nodes, 0, "$".to_string(), Kind::Null, "null".to_string());
        }
        [document] => walk_yaml(document, nodes, "$".to_string(), 0),
        documents => {
            let ix = push(nodes, 0, "$".to_string(), Kind::Array, String::new());
            for (i, document) in documents.iter().enumerate() {
                walk_yaml(document, nodes, i.to_string(), 1);
            }
            close(nodes, ix, documents.len());
        }
    }
    Ok(())
}

/// The emitter writes non-finite floats as Rust does; give it their YAML spelling instead.
fn yaml_for_emit(value: &Yaml<'_>) -> Yaml<'static> {
    match value {
        Yaml::Value(Scalar::FloatingPoint(f)) if !f.is_finite() => {
            Yaml::Representation(yaml_float_text(f.into_inner()).into(), saphyr::ScalarStyle::Plain, None)
        }
        Yaml::Value(scalar) => Yaml::Value(match scalar {
            Scalar::Null => Scalar::Null,
            Scalar::Boolean(b) => Scalar::Boolean(*b),
            Scalar::Integer(i) => Scalar::Integer(*i),
            Scalar::FloatingPoint(f) => Scalar::FloatingPoint(*f),
            Scalar::String(s) => Scalar::String(s.to_string().into()),
        }),
        Yaml::Representation(text, style, tag) => {
            Yaml::Representation(text.to_string().into(), *style, tag.as_ref().map(|t| std::borrow::Cow::Owned(t.clone().into_owned())))
        }
        Yaml::Tagged(tag, inner) => Yaml::Tagged(std::borrow::Cow::Owned(tag.clone().into_owned()), Box::new(yaml_for_emit(inner))),
        Yaml::Alias(a) => Yaml::Alias(*a),
        Yaml::BadValue => Yaml::BadValue,
        Yaml::Sequence(items) => Yaml::Sequence(items.iter().map(yaml_for_emit).collect()),
        Yaml::Mapping(map) => Yaml::Mapping(map.iter().map(|(k, v)| (yaml_for_emit(k), yaml_for_emit(v))).collect()),
    }
}

fn pretty_yaml(text: &str) -> Result<String, ParseError> {
    let documents = load_yaml(text)?;
    let mut out = String::new();
    for (i, document) in documents.iter().enumerate() {
        let mut dumped = String::new();
        saphyr::YamlEmitter::new(&mut dumped)
            .dump(&yaml_for_emit(document))
            .map_err(|e| ParseError { line: 1, column: 1, message: e.to_string() })?;
        // The emitter opens every document with `---`; the first keeps it only when the file had it.
        if i == 0 && !text.trim_start().starts_with("---") {
            dumped = dumped.strip_prefix("---\n").unwrap_or(&dumped).to_string();
        }
        out.push_str(dumped.trim_end_matches('\n'));
        out.push('\n');
    }
    Ok(out)
}

// ---- TOML: the typed value tree, in file order ----

fn walk_toml_value(value: &toml::Value, nodes: &mut Vec<Node>, key: String, depth: u32) {
    use toml::Value;
    match value {
        Value::String(s) => {
            push(nodes, depth, key, Kind::String, s.clone());
        }
        Value::Integer(i) => {
            push(nodes, depth, key, Kind::Integer, i.to_string());
        }
        Value::Float(f) => {
            push(nodes, depth, key, Kind::Float, float_text(*f));
        }
        Value::Boolean(b) => {
            push(nodes, depth, key, Kind::Boolean, b.to_string());
        }
        Value::Datetime(d) => {
            push(nodes, depth, key, Kind::DateTime, d.to_string());
        }
        Value::Array(items) => {
            let ix = push(nodes, depth, key, Kind::Array, String::new());
            for (i, item) in items.iter().enumerate() {
                walk_toml_value(item, nodes, i.to_string(), depth + 1);
            }
            close(nodes, ix, items.len());
        }
        Value::Table(table) => {
            let ix = push(nodes, depth, key, Kind::Table, String::new());
            for (k, v) in table {
                walk_toml_value(v, nodes, k.clone(), depth + 1);
            }
            close(nodes, ix, table.len());
        }
    }
}

fn load_toml(text: &str) -> Result<toml::Table, ParseError> {
    text.parse().map_err(|error: toml::de::Error| {
        let (line, column) = error.span().map(|span| line_column(text, span.start)).unwrap_or((1, 1));
        ParseError { line, column, message: error.message().to_string() }
    })
}

fn parse_toml(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    let table = load_toml(text)?;
    walk_toml_value(&toml::Value::Table(table), nodes, "$".to_string(), 0);
    Ok(())
}

// ---- XML: elements, attributes, text, comments and processing instructions ----

/// The name as written: with its prefix when the element or attribute has one.
fn xml_name(node: roxmltree::Node, namespace: Option<&str>, local: &str) -> String {
    match namespace.and_then(|uri| node.lookup_prefix(uri)) {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local}"),
        _ => local.to_string(),
    }
}

fn xml_children<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| !(child.is_text() && child.text().is_some_and(|t| t.trim().is_empty())))
        .collect()
}

fn walk_xml(node: roxmltree::Node, nodes: &mut Vec<Node>, depth: u32) {
    use roxmltree::NodeType;
    match node.node_type() {
        NodeType::Root => {}
        NodeType::Element => {
            let name = xml_name(node, node.tag_name().namespace(), node.tag_name().name());
            let ix = push(nodes, depth, name, Kind::Element, String::new());
            let mut count = 0;
            for attribute in node.attributes() {
                let name = xml_name(node, attribute.namespace(), attribute.name());
                push(nodes, depth + 1, format!("@{name}"), Kind::Attribute, attribute.value().to_string());
                count += 1;
            }
            let children = xml_children(node);
            if children.len() == 1 && children[0].is_text() {
                let text = children[0].text().unwrap_or_default().trim().to_string();
                let end = nodes.len();
                nodes[ix].end = end;
                nodes[ix].value = text;
                return;
            }
            for child in children {
                walk_xml(child, nodes, depth + 1);
                count += 1;
            }
            close(nodes, ix, count);
        }
        NodeType::Text => {
            let text = node.text().unwrap_or_default().trim().to_string();
            if !text.is_empty() {
                push(nodes, depth, "text()".to_string(), Kind::Text, text);
            }
        }
        NodeType::Comment => {
            push(nodes, depth, "comment()".to_string(), Kind::Comment, node.text().unwrap_or_default().trim().to_string());
        }
        NodeType::PI => {
            let pi = node.pi().map(|pi| match pi.value {
                Some(value) => format!("{} {}", pi.target, value.trim()),
                None => pi.target.to_string(),
            });
            push(nodes, depth, "processing-instruction()".to_string(), Kind::Instruction, pi.unwrap_or_default());
        }
    }
}

fn load_xml(text: &str) -> Result<roxmltree::Document<'_>, ParseError> {
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(text, options).map_err(|error| {
        let at = error.pos();
        ParseError { line: at.row as usize, column: at.col as usize, message: xml_message(&error.to_string()) }
    })
}

fn parse_xml(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    let document = load_xml(text)?;
    for child in document.root().children() {
        walk_xml(child, nodes, 0);
    }
    Ok(())
}

/// roxmltree's message without its own " at R:C" tail.
fn xml_message(text: &str) -> String {
    match text.rfind(" at ") {
        Some(at) if text[at + 4..].chars().all(|c| c.is_ascii_digit() || c == ':') => text[..at].to_string(),
        _ => text.to_string(),
    }
}

// ---- INI: sections with keys, keys before the first section at the root ----

fn parse_ini(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    let mut section: Option<(usize, usize)> = None;
    for (line_ix, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            if let Some((ix, count)) = section.take() {
                close(nodes, ix, count);
            }
            let ix = push(nodes, 0, name.trim().to_string(), Kind::Section, String::new());
            section = Some((ix, 0));
            continue;
        }
        let Some(at) = line.find(['=', ':']) else {
            return Err(ParseError {
                line: line_ix + 1,
                column: 1,
                message: "expected `key = value` or `[section]`".to_string(),
            });
        };
        let (key, value) = (line[..at].trim(), line[at + 1..].trim());
        if key.is_empty() {
            return Err(ParseError { line: line_ix + 1, column: 1, message: "a key is missing before the separator".to_string() });
        }
        let depth = if section.is_some() { 1 } else { 0 };
        push(nodes, depth, key.to_string(), Kind::Key, value.to_string());
        if let Some((_, count)) = section.as_mut() {
            *count += 1;
        }
    }
    if let Some((ix, count)) = section {
        close(nodes, ix, count);
    }
    Ok(())
}

// ---- Pretty-printing ----

/// The pretty form of `text` in `format`, or why it does not parse.
pub fn pretty(format: Format, text: &str) -> Result<String, ParseError> {
    match format {
        Format::Json => {
            check_json(text, 0)?;
            let mut out = respace_json(text, Some("  "));
            out.push('\n');
            Ok(out)
        }
        Format::JsonLines => {
            let mut out = String::new();
            for (line_ix, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                check_json(line, line_ix)?;
                out.push_str(&respace_json(line, None));
                out.push('\n');
            }
            Ok(out)
        }
        Format::Xml => pretty_xml(text),
        Format::Yaml => pretty_yaml(text),
        Format::Toml => {
            let table = load_toml(text)?;
            toml::to_string_pretty(&table).map_err(|e| ParseError { line: 1, column: 1, message: e.to_string() })
        }
        Format::Ini => pretty_ini(text),
    }
}

/// The `<!DOCTYPE ...>` of `text`, internal subset included, as written.
fn xml_doctype(text: &str) -> Option<&str> {
    let start = text.find("<!DOCTYPE")?;
    let rest = &text[start..];
    let end = match (rest.find('['), rest.find('>')) {
        (Some(bracket), Some(close)) if bracket < close => rest.find("]>").map(|e| e + 2)?,
        (_, Some(close)) => close + 1,
        _ => return None,
    };
    Some(&rest[..end])
}

fn pretty_xml(text: &str) -> Result<String, ParseError> {
    let document = load_xml(text)?;
    let mut out = String::new();
    if text.trim_start().starts_with("<?xml") {
        let end = text.find("?>").map_or(0, |e| e + 2);
        out.push_str(text[..end].trim_start());
        out.push('\n');
    }
    if let Some(doctype) = xml_doctype(text) {
        out.push_str(doctype);
        out.push('\n');
    }
    for child in document.root().children() {
        write_xml(child, 0, &mut out);
    }
    Ok(out)
}

fn escape_xml(text: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// The namespace declarations this element makes: those in scope here and not on its parent.
fn xml_declarations(node: roxmltree::Node) -> Vec<(Option<String>, String)> {
    let inherited: Vec<(Option<&str>, &str)> = node
        .parent_element()
        .map(|parent| parent.namespaces().map(|ns| (ns.name(), ns.uri())).collect())
        .unwrap_or_default();
    node.namespaces()
        .filter(|ns| ns.name() != Some("xml"))
        .filter(|ns| !inherited.contains(&(ns.name(), ns.uri())))
        .map(|ns| (ns.name().map(str::to_string), ns.uri().to_string()))
        .collect()
}

fn write_xml(node: roxmltree::Node, depth: usize, out: &mut String) {
    use roxmltree::NodeType;
    let indent = "  ".repeat(depth);
    match node.node_type() {
        NodeType::Root => {}
        NodeType::Element => {
            let name = xml_name(node, node.tag_name().namespace(), node.tag_name().name());
            out.push_str(&indent);
            out.push('<');
            out.push_str(&name);
            for (prefix, uri) in xml_declarations(node) {
                match prefix {
                    Some(prefix) => out.push_str(&format!(" xmlns:{prefix}=\"{}\"", escape_xml(&uri, true))),
                    None => out.push_str(&format!(" xmlns=\"{}\"", escape_xml(&uri, true))),
                }
            }
            for attribute in node.attributes() {
                let attribute_name = xml_name(node, attribute.namespace(), attribute.name());
                out.push_str(&format!(" {attribute_name}=\"{}\"", escape_xml(attribute.value(), true)));
            }
            let children = xml_children(node);
            if children.is_empty() {
                out.push_str("/>\n");
            } else if children.len() == 1 && children[0].is_text() {
                out.push('>');
                out.push_str(&escape_xml(children[0].text().unwrap_or_default().trim(), false));
                out.push_str(&format!("</{name}>\n"));
            } else {
                out.push_str(">\n");
                for child in children {
                    write_xml(child, depth + 1, out);
                }
                out.push_str(&format!("{indent}</{name}>\n"));
            }
        }
        NodeType::Text => {
            let text = node.text().unwrap_or_default().trim();
            if !text.is_empty() {
                out.push_str(&format!("{indent}{}\n", escape_xml(text, false)));
            }
        }
        NodeType::Comment => {
            out.push_str(&format!("{indent}<!-- {} -->\n", node.text().unwrap_or_default().trim()));
        }
        NodeType::PI => {
            if let Some(pi) = node.pi() {
                match pi.value {
                    Some(value) => out.push_str(&format!("{indent}<?{} {}?>\n", pi.target, value.trim())),
                    None => out.push_str(&format!("{indent}<?{}?>\n", pi.target)),
                }
            }
        }
    }
}

/// `[section]` lines with `key = value` under them, comments kept, one blank line before
/// each section.
fn pretty_ini(text: &str) -> Result<String, ParseError> {
    let mut nodes = Vec::new();
    parse_ini(text, &mut nodes)?;
    let mut out = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with(';') || line.starts_with('#') {
            out.push_str(line);
            out.push('\n');
        } else if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{}]\n", name.trim()));
        } else {
            let at = line.find(['=', ':']).unwrap_or(line.len());
            let (key, value) = (line[..at].trim(), line[at + 1..].trim());
            out.push_str(&format!("{key} = {value}\n"));
        }
    }
    Ok(out)
}

/// A JSON document of about `target` bytes for the tests: `records`, each with ten nodes,
/// and a count. Gives the text, the node count and the path of the last record's last tag.
pub fn big_json(target: usize) -> (String, usize, String) {
    let mut text = String::with_capacity(target + 1024);
    text.push_str("{\"records\": [");
    let note = "x".repeat(300);
    let mut records = 0;
    while text.len() < target {
        if records > 0 {
            text.push(',');
        }
        text.push_str(&format!(
            "{{\"id\": {records}, \"name\": \"record-{records}\", \"tags\": [\"alpha\", \"beta\", \"gamma\"], \"ok\": true, \"score\": {records}.5, \"note\": \"{note}\"}}"
        ));
        records += 1;
    }
    text.push_str(&format!("], \"count\": {records}}}"));
    // root, records, count, and ten nodes per record (object, id, name, tags, 3 tags, ok, score, note)
    let nodes = 3 + records * 10;
    (text, nodes, format!("$.records[{}].tags[2]", records - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{"name": "Sfoglia", "version": 1.5, "tags": ["rust", "gpui"], "ok": true, "none": null, "nested": {"a": {"b": [1, {"c": "deep"}]}}, "odd key": "x"}"#;

    fn rows(tree: &Tree) -> Vec<(u32, &str, &str, &str)> {
        tree.nodes.iter().map(|n| (n.depth, n.key.as_str(), n.kind.label(), n.value.as_str())).collect()
    }

    #[test]
    fn every_extension_has_its_format() {
        let expected = [
            ("json", Format::Json),
            ("jsonl", Format::JsonLines),
            ("xml", Format::Xml),
            ("yaml", Format::Yaml),
            ("yml", Format::Yaml),
            ("toml", Format::Toml),
            ("ini", Format::Ini),
        ];
        assert_eq!(Format::EXTENSIONS.len(), expected.len());
        for (ext, format) in expected {
            assert!(Format::EXTENSIONS.contains(&ext));
            assert_eq!(Format::for_name(&format!("file.{ext}")), Some(format), "{ext}");
            assert_eq!(Format::for_name(&format!("FILE.{}", ext.to_uppercase())), Some(format), "{ext} upper");
        }
        for name in ["file.txt", "file.json5", "json", "file.", "file.xml.bak"] {
            assert_eq!(Format::for_name(name), None, "{name}");
        }
    }

    #[test]
    fn json_builds_the_exact_arena() {
        let tree = Tree::parse(Format::Json, JSON).unwrap();
        assert_eq!(
            rows(&tree),
            vec![
                (0, "$", "object", "7 entries"),
                (1, "name", "string", "Sfoglia"),
                (1, "version", "number", "1.5"),
                (1, "tags", "array", "2 entries"),
                (2, "0", "string", "rust"),
                (2, "1", "string", "gpui"),
                (1, "ok", "boolean", "true"),
                (1, "none", "null", "null"),
                (1, "nested", "object", "1 entry"),
                (2, "a", "object", "1 entry"),
                (3, "b", "array", "2 entries"),
                (4, "0", "number", "1"),
                (4, "1", "object", "1 entry"),
                (5, "c", "string", "deep"),
                (1, "odd key", "string", "x"),
            ]
        );
        assert_eq!(tree.nodes[0].end, 15);
        assert_eq!(tree.nodes[3].end, 6);
        assert_eq!(tree.nodes[8].end, 14);
        assert_eq!(tree.children(0), vec![1, 2, 3, 6, 7, 8, 14]);
        assert_eq!(tree.children(8), vec![9]);
        assert_eq!(tree.parent(13), Some(12));
        assert_eq!(tree.ancestors(13), vec![0, 8, 9, 10, 12]);
        assert_eq!(tree.roots(), vec![0]);
        assert_eq!(tree.levels(), 6);
        assert_eq!(tree.summary(), "JSON, 15 nodes, 6 levels");
        // Scalars at the root; numbers keep the file's own digits, however long.
        let scalar = Tree::parse(Format::Json, "42").unwrap();
        assert_eq!(rows(&scalar), vec![(0, "$", "number", "42")]);
        let numbers = Tree::parse(Format::Json, "[1.0, 2.5e10, -0.0, 12345678901234567890123, 0.1000]").unwrap();
        assert_eq!(numbers.nodes[1].value, "1.0");
        assert_eq!(numbers.nodes[2].value, "2.5e+10", "the exponent gets its sign, the digits stay");
        assert_eq!(numbers.nodes[3].value, "-0.0");
        assert_eq!(numbers.nodes[4].value, "12345678901234567890123");
        assert_eq!(numbers.nodes[5].value, "0.1000");
        let empty = Tree::parse(Format::Json, r#"{"a": [], "b": {}}"#).unwrap();
        assert_eq!(rows(&empty), vec![(0, "$", "object", "2 entries"), (1, "a", "array", "0 entries"), (1, "b", "object", "0 entries")]);
        // Duplicate keys are two nodes, as the file has them.
        let twice = Tree::parse(Format::Json, r#"{"a": 1, "a": 2}"#).unwrap();
        assert_eq!(rows(&twice), vec![(0, "$", "object", "2 entries"), (1, "a", "number", "1"), (1, "a", "number", "2")]);
    }

    #[test]
    fn jsonl_is_one_entry_per_line_and_names_the_bad_line() {
        let text = "{\"id\": 1}\n\n{\"id\": 2, \"tags\": [\"a\"]}\n";
        let tree = Tree::parse(Format::JsonLines, text).unwrap();
        assert_eq!(
            rows(&tree),
            vec![
                (0, "$", "array", "2 entries"),
                (1, "0", "object", "1 entry"),
                (2, "id", "number", "1"),
                (1, "1", "object", "2 entries"),
                (2, "id", "number", "2"),
                (2, "tags", "array", "1 entry"),
                (3, "0", "string", "a"),
            ]
        );
        assert_eq!(tree.path(6), "$[1].tags[0]");
        let bad = "{\"id\": 1}\n{\"id\": 2}\n{\"id\": }\n";
        let error = Tree::parse(Format::JsonLines, bad).unwrap_err();
        assert_eq!((error.line, error.column), (3, 8));
        assert_eq!(error.message, "expected value");
        let empty = Tree::parse(Format::JsonLines, "\n\n").unwrap();
        assert_eq!(rows(&empty), vec![(0, "$", "array", "0 entries")]);
        // Records may be scalars or arrays too.
        let mixed = Tree::parse(Format::JsonLines, "1\n[true]\n\"s\"\n").unwrap();
        assert_eq!(rows(&mixed), vec![(0, "$", "array", "3 entries"), (1, "0", "number", "1"), (1, "1", "array", "1 entry"), (2, "0", "boolean", "true"), (1, "2", "string", "s")]);
    }

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- the catalogue -->
<catalogue year="2026">
  <book id="b1">
    <title>Leafing</title>
    <price currency="EUR">12.50</price>
  </book>
  <book id="b2"><title>Pages &amp; Bytes</title></book>
  <note>mixed <b>content</b> here</note>
  <?render fast?>
  <empty/>
</catalogue>
"#;

    #[test]
    fn xml_elements_attributes_text_and_comments_become_nodes() {
        let tree = Tree::parse(Format::Xml, XML).unwrap();
        assert_eq!(
            rows(&tree),
            vec![
                (0, "comment()", "comment", "the catalogue"),
                (0, "catalogue", "element", "6 entries"),
                (1, "@year", "attribute", "2026"),
                (1, "book", "element", "3 entries"),
                (2, "@id", "attribute", "b1"),
                (2, "title", "element", "Leafing"),
                (2, "price", "element", "12.50"),
                (3, "@currency", "attribute", "EUR"),
                (1, "book", "element", "2 entries"),
                (2, "@id", "attribute", "b2"),
                (2, "title", "element", "Pages & Bytes"),
                (1, "note", "element", "3 entries"),
                (2, "text()", "text", "mixed"),
                (2, "b", "element", "content"),
                (2, "text()", "text", "here"),
                (1, "processing-instruction()", "instruction", "render fast"),
                (1, "empty", "element", "0 entries"),
            ]
        );
        assert_eq!(tree.nodes[6].end, 8, "a text-only element with attributes ends after them");
        assert_eq!(tree.roots(), vec![0, 1]);
        assert_eq!(tree.path(0), "/comment()");
        assert_eq!(tree.path(1), "/catalogue");
        assert_eq!(tree.path(2), "/catalogue/@year");
        assert_eq!(tree.path(4), "/catalogue/book[1]/@id");
        assert_eq!(tree.path(7), "/catalogue/book[1]/price/@currency");
        assert_eq!(tree.path(10), "/catalogue/book[2]/title");
        assert_eq!(tree.path(14), "/catalogue/note/text()[2]");
        assert_eq!(tree.path(15), "/catalogue/processing-instruction()");
        assert_eq!(tree.path(16), "/catalogue/empty");
        // A DOCTYPE is allowed, and prefixed names keep their prefixes.
        let typed = Tree::parse(Format::Xml, "<?xml version=\"1.0\"?>\n<!DOCTYPE note SYSTEM \"note.dtd\">\n<note><to>x</to></note>\n").unwrap();
        assert_eq!(rows(&typed), vec![(0, "note", "element", "1 entry"), (1, "to", "element", "x")]);
        let spaced = Tree::parse(Format::Xml, "<a:r xmlns:a=\"urn:a\" xmlns:b=\"urn:b\" xmlns=\"urn:d\"><a:x a:id=\"1\" b:id=\"2\" xml:lang=\"en\"/><y/></a:r>").unwrap();
        assert_eq!(
            rows(&spaced),
            vec![
                (0, "a:r", "element", "2 entries"),
                (1, "a:x", "element", "3 entries"),
                (2, "@a:id", "attribute", "1"),
                (2, "@b:id", "attribute", "2"),
                (2, "@xml:lang", "attribute", "en"),
                (1, "y", "element", "0 entries"),
            ]
        );
        assert_eq!(spaced.path(3), "/a:r/a:x/@b:id");
    }

    #[test]
    fn yaml_and_toml_keep_key_order_and_name_their_types() {
        let yaml = "zeta: 1\nalpha:\n  - x\n  - 2.5\n  - true\n  - null\nmid: text\n";
        let tree = Tree::parse(Format::Yaml, yaml).unwrap();
        assert_eq!(
            rows(&tree),
            vec![
                (0, "$", "object", "3 entries"),
                (1, "zeta", "number", "1"),
                (1, "alpha", "array", "4 entries"),
                (2, "0", "string", "x"),
                (2, "1", "number", "2.5"),
                (2, "2", "boolean", "true"),
                (2, "3", "null", "null"),
                (1, "mid", "string", "text"),
            ]
        );
        assert_eq!(tree.path(4), "$.alpha[1]");
        // Typed keys, non-finite floats, anchors and a stream of documents.
        let typed = Tree::parse(Format::Yaml, "200: ok\ntrue: yes\nbase: &b\n  k: 1\ncopy: *b\ninf: .inf\nnan: .nan\n").unwrap();
        assert_eq!(
            rows(&typed),
            vec![
                (0, "$", "object", "6 entries"),
                (1, "200", "string", "ok"),
                (1, "true", "string", "yes"),
                (1, "base", "object", "1 entry"),
                (2, "k", "number", "1"),
                (1, "copy", "object", "1 entry"),
                (2, "k", "number", "1"),
                (1, "inf", "number", ".inf"),
                (1, "nan", "number", ".nan"),
            ]
        );
        assert_eq!(typed.path(1), "$[\"200\"]");
        let stream = Tree::parse(Format::Yaml, "a: 1\n---\nb: 2\n").unwrap();
        assert_eq!(rows(&stream), vec![(0, "$", "array", "2 entries"), (1, "0", "object", "1 entry"), (2, "a", "number", "1"), (1, "1", "object", "1 entry"), (2, "b", "number", "2")]);
        assert_eq!(stream.path(4), "$[1].b");
        let scalar = Tree::parse(Format::Yaml, "just text\n").unwrap();
        assert_eq!(rows(&scalar), vec![(0, "$", "string", "just text")]);
        let toml = "zeta = 1\nalpha = [\"x\", 2.5, true]\nwhen = 2026-10-17T08:00:00Z\n\n[server]\nhost = \"localhost\"\nport = 8080\n\n[[items]]\nname = \"a\"\n\n[[items]]\nname = \"b\"\n";
        let tree = Tree::parse(Format::Toml, toml).unwrap();
        assert_eq!(
            rows(&tree),
            vec![
                (0, "$", "table", "5 entries"),
                (1, "zeta", "integer", "1"),
                (1, "alpha", "array", "3 entries"),
                (2, "0", "string", "x"),
                (2, "1", "float", "2.5"),
                (2, "2", "boolean", "true"),
                (1, "when", "date-time", "2026-10-17T08:00:00Z"),
                (1, "server", "table", "2 entries"),
                (2, "host", "string", "localhost"),
                (2, "port", "integer", "8080"),
                (1, "items", "array", "2 entries"),
                (2, "0", "table", "1 entry"),
                (3, "name", "string", "a"),
                (2, "1", "table", "1 entry"),
                (3, "name", "string", "b"),
            ]
        );
        assert_eq!(tree.path(14), "$.items[1].name");
        assert_eq!(tree.path(9), "$.server.port");
        let inline = Tree::parse(Format::Toml, "point = { x = 1, y = 2 }\n").unwrap();
        assert_eq!(rows(&inline), vec![(0, "$", "table", "1 entry"), (1, "point", "table", "2 entries"), (2, "x", "integer", "1"), (2, "y", "integer", "2")]);
    }

    #[test]
    fn ini_sections_keys_and_the_bad_line() {
        let ini = "; a comment\nname = top\n\n[server]\nhost = localhost\nport: 8080\nurl = http://x/y?a=b\n# another\n\n[empty]\n\n[paths]\nroot = C:\\data\n";
        let tree = Tree::parse(Format::Ini, ini).unwrap();
        assert_eq!(
            rows(&tree),
            vec![
                (0, "name", "key", "top"),
                (0, "server", "section", "3 entries"),
                (1, "host", "key", "localhost"),
                (1, "port", "key", "8080"),
                (1, "url", "key", "http://x/y?a=b"),
                (0, "empty", "section", "0 entries"),
                (0, "paths", "section", "1 entry"),
                (1, "root", "key", "C:\\data"),
            ]
        );
        assert_eq!(tree.roots(), vec![0, 1, 5, 6]);
        assert_eq!(tree.path(0), "name");
        assert_eq!(tree.path(3), "server.port");
        assert_eq!(tree.path(7), "paths.root");
        let error = Tree::parse(Format::Ini, "[a]\nkey = 1\nnonsense\n").unwrap_err();
        assert_eq!((error.line, error.column), (3, 1));
        assert_eq!(error.message, "expected `key = value` or `[section]`");
        let error = Tree::parse(Format::Ini, "= 1\n").unwrap_err();
        assert_eq!((error.line, error.column), (1, 1));
    }

    #[test]
    fn paths_are_exact_for_every_kind_of_node() {
        let tree = Tree::parse(Format::Json, JSON).unwrap();
        let paths: Vec<String> = (0..tree.len()).map(|ix| tree.path(ix)).collect();
        assert_eq!(
            paths,
            vec![
                "$",
                "$.name",
                "$.version",
                "$.tags",
                "$.tags[0]",
                "$.tags[1]",
                "$.ok",
                "$.none",
                "$.nested",
                "$.nested.a",
                "$.nested.a.b",
                "$.nested.a.b[0]",
                "$.nested.a.b[1]",
                "$.nested.a.b[1].c",
                "$[\"odd key\"]",
            ]
        );
        let quoted = Tree::parse(Format::Json, r#"{"a\"b": {"1st": 0, "x\\y": 1}}"#).unwrap();
        assert_eq!(quoted.path(1), "$[\"a\\\"b\"]");
        assert_eq!(quoted.path(2), "$[\"a\\\"b\"][\"1st\"]");
        assert_eq!(quoted.path(3), "$[\"a\\\"b\"][\"x\\\\y\"]");
        let toml = Tree::parse(Format::Toml, "[a.b]\n\"c.d\" = 1\n").unwrap();
        assert_eq!(toml.path(3), "$.a.b[\"c.d\"]");
        let xml = Tree::parse(Format::Xml, "<r><a/><b/><a x=\"1\"/><!--c--><!--d--></r>").unwrap();
        assert_eq!((0..xml.len()).map(|ix| xml.path(ix)).collect::<Vec<_>>(), vec!["/r", "/r/a[1]", "/r/b", "/r/a[2]", "/r/a[2]/@x", "/r/comment()[1]", "/r/comment()[2]"]);
    }

    #[test]
    fn search_matches_keys_and_values_case_insensitively() {
        let tree = Tree::parse(Format::Json, JSON).unwrap();
        assert_eq!(tree.search("NAME"), vec![1]);
        assert_eq!(tree.search("sfoglia"), vec![1]);
        assert_eq!(tree.search("e"), vec![0, 1, 2, 3, 6, 7, 8, 9, 10, 12, 13, 14]);
        assert_eq!(tree.search("deep"), vec![13]);
        assert_eq!(tree.search("entr"), vec![0, 3, 8, 9, 10, 12], "container values count too");
        assert_eq!(tree.search("missing"), Vec::<usize>::new());
        assert_eq!(tree.search(""), Vec::<usize>::new());
        let xml = Tree::parse(Format::Xml, XML).unwrap();
        assert_eq!(xml.search("B2"), vec![9]);
        assert_eq!(xml.search("@"), vec![2, 4, 7, 9]);
        let accented = Tree::parse(Format::Json, r#"{"Città": "ÉCOLE"}"#).unwrap();
        assert_eq!(accented.search("città"), vec![1]);
        assert_eq!(accented.search("école"), vec![1]);
    }

    #[test]
    fn pretty_print_reformats_every_format_and_keeps_its_tree() {
        let compact = "{\"b\":[1,{\"c\":null}],\"a\":\"x\"}";
        let json = pretty(Format::Json, compact).unwrap();
        assert_eq!(json, "{\n  \"b\": [\n    1,\n    {\n      \"c\": null\n    }\n  ],\n  \"a\": \"x\"\n}\n");
        assert_eq!(Tree::parse(Format::Json, &json).unwrap().nodes, Tree::parse(Format::Json, compact).unwrap().nodes);
        // Duplicate keys, long numbers, escapes and empty containers come through untouched.
        let tricky = "{\"a\" : 1,\"a\":2, \"big\":12345678901234567890123,\"s\":\"q\\\"{}[],:\", \"e\":{ } ,\"f\":[ ]}";
        let pretty_tricky = pretty(Format::Json, tricky).unwrap();
        assert_eq!(
            pretty_tricky,
            "{\n  \"a\": 1,\n  \"a\": 2,\n  \"big\": 12345678901234567890123,\n  \"s\": \"q\\\"{}[],:\",\n  \"e\": {},\n  \"f\": []\n}\n"
        );
        assert_eq!(Tree::parse(Format::Json, &pretty_tricky).unwrap().nodes, Tree::parse(Format::Json, tricky).unwrap().nodes);
        assert_eq!(pretty(Format::Json, "  [ ]\n").unwrap(), "[]\n");
        let lines = pretty(Format::JsonLines, "{ \"id\" : 1 , \"t\":[ ] }\n\n  {\"id\":2, \"id\":3}\n").unwrap();
        assert_eq!(lines, "{\"id\":1,\"t\":[]}\n{\"id\":2,\"id\":3}\n");
        let xml = pretty(Format::Xml, XML).unwrap();
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- the catalogue -->\n<catalogue year=\"2026\">\n  <book id=\"b1\">\n    <title>Leafing</title>\n    <price currency=\"EUR\">12.50</price>\n  </book>\n  <book id=\"b2\">\n    <title>Pages &amp; Bytes</title>\n  </book>\n  <note>\n    mixed\n    <b>content</b>\n    here\n  </note>\n  <?render fast?>\n  <empty/>\n</catalogue>\n"
        );
        assert_eq!(Tree::parse(Format::Xml, &xml).unwrap().nodes, Tree::parse(Format::Xml, XML).unwrap().nodes);
        // A DOCTYPE and namespace declarations are written out, so the pretty form is still well-formed.
        let typed = "<?xml version=\"1.0\"?><!DOCTYPE note [<!ENTITY e \"x\">]><note>&e;</note>";
        assert_eq!(pretty(Format::Xml, typed).unwrap(), "<?xml version=\"1.0\"?>\n<!DOCTYPE note [<!ENTITY e \"x\">]>\n<note>x</note>\n");
        let spaced = "<a:r xmlns:a=\"urn:a\" xmlns=\"urn:d\"><a:x a:id=\"1\" xml:lang=\"en\"/><y xmlns:b=\"urn:b\" b:k=\"v\"/></a:r>";
        let pretty_spaced = pretty(Format::Xml, spaced).unwrap();
        assert_eq!(
            pretty_spaced,
            "<a:r xmlns:a=\"urn:a\" xmlns=\"urn:d\">\n  <a:x a:id=\"1\" xml:lang=\"en\"/>\n  <y xmlns:b=\"urn:b\" b:k=\"v\"/>\n</a:r>\n"
        );
        assert_eq!(Tree::parse(Format::Xml, &pretty_spaced).unwrap().nodes, Tree::parse(Format::Xml, spaced).unwrap().nodes);
        let yaml_in = "zeta: 1\nalpha: [x, 2.5]\n200:   ok\ntrue: yes\ninf: .inf\n";
        let yaml = pretty(Format::Yaml, yaml_in).unwrap();
        // The emitter quotes what would read as another type (`yes`, `inf`); the data is the same.
        assert_eq!(yaml, "zeta: 1\nalpha:\n  - x\n  - 2.5\n200: ok\ntrue: \"yes\"\n\"inf\": .inf\n");
        assert_eq!(Tree::parse(Format::Yaml, &yaml).unwrap().nodes, Tree::parse(Format::Yaml, yaml_in).unwrap().nodes);
        let stream = pretty(Format::Yaml, "---\na: 1\n---\nb: [1,2]\n").unwrap();
        assert_eq!(stream, "---\na: 1\n---\nb:\n  - 1\n  - 2\n");
        let toml_in = "zeta=1\n[server]\nhost=\"h\"\n";
        let toml = pretty(Format::Toml, toml_in).unwrap();
        assert_eq!(toml, "zeta = 1\n\n[server]\nhost = \"h\"\n");
        assert_eq!(Tree::parse(Format::Toml, &toml).unwrap().nodes, Tree::parse(Format::Toml, toml_in).unwrap().nodes);
        let ini_in = "name=top\n; note\n[server]\n  host :localhost  \nport=8080\n[next]\n";
        let ini = pretty(Format::Ini, ini_in).unwrap();
        assert_eq!(ini, "name = top\n; note\n\n[server]\nhost = localhost\nport = 8080\n\n[next]\n");
        assert_eq!(Tree::parse(Format::Ini, &ini).unwrap().nodes, Tree::parse(Format::Ini, ini_in).unwrap().nodes);
        // A file that does not parse is refused with its position, not reformatted.
        let error = pretty(Format::Json, "{\"a\": }").unwrap_err();
        assert_eq!((error.line, error.column), (1, 7));
        let error = pretty(Format::JsonLines, "{\"a\": 1}\n{\"a\": 1} x\n").unwrap_err();
        assert_eq!((error.line, error.column), (2, 10));
    }

    #[test]
    fn parse_errors_name_their_line_and_column() {
        let json = Tree::parse(Format::Json, "{\n  \"a\": 1,\n  \"b\": [1, 2,\n}\n").unwrap_err();
        assert_eq!((json.line, json.column, json.message.as_str()), (4, 1, "expected value"));
        assert_eq!(json.to_string(), "line 4, column 1: expected value");
        let xml = Tree::parse(Format::Xml, "<a>\n  <b>\n</a>\n").unwrap_err();
        assert_eq!(xml.line, 3, "{xml}");
        assert!(xml.column >= 1, "{xml}");
        assert!(!xml.message.contains(" at "), "{xml}");
        let yaml = Tree::parse(Format::Yaml, "a: 1\nb: [1, 2\nc: 3\n").unwrap_err();
        assert!(yaml.line >= 2, "{yaml}");
        assert!(yaml.column >= 1 && !yaml.message.is_empty(), "{yaml}");
        let toml = Tree::parse(Format::Toml, "a = 1\n[server\nhost = 1\n").unwrap_err();
        assert_eq!(toml.line, 2, "{toml}");
        assert!(toml.column >= 1 && !toml.message.is_empty(), "{toml}");
        let ini = Tree::parse(Format::Ini, "[a]\nb = 1\n\nbroken line\n").unwrap_err();
        assert_eq!((ini.line, ini.column), (4, 1));
        // CRLF endings count lines the same way.
        let crlf = Tree::parse(Format::Json, "{\r\n  \"a\": 1,\r\n  \"b\": [1, 2,\r\n}\r\n").unwrap_err();
        assert_eq!((crlf.line, crlf.column), (4, 1));
        let crlf_toml = Tree::parse(Format::Toml, "a = 1\r\n[server\r\nhost = 1\r\n").unwrap_err();
        assert_eq!(crlf_toml.line, 2, "{crlf_toml}");
    }

    #[test]
    fn a_50_mb_json_parses_with_every_node_and_a_deep_path() {
        let (text, nodes, deep_path) = big_json(50 * 1024 * 1024);
        assert!(text.len() >= 50 * 1024 * 1024);
        let tree = Tree::parse(Format::Json, &text).unwrap();
        assert_eq!(tree.len(), nodes);
        let last_tag = tree.len() - 5; // ..., tags[2], ok, score, note, then count
        assert_eq!(tree.nodes[last_tag].value, "gamma");
        assert_eq!(tree.path(last_tag), deep_path);
        assert_eq!(tree.nodes[tree.len() - 1].key, "count");
        assert_eq!(tree.nodes[tree.len() - 1].value, ((nodes - 3) / 10).to_string());
        assert_eq!(tree.levels(), 5);
    }
}
