//! Structured data trees (C-010): JSON, JSON Lines, XML, YAML, TOML and INI parsed into one
//! pre-order arena of nodes with a key, a type and a value; paths, search and pretty-printing
//! over that arena. Pure: no window, no file system.

use std::fmt;

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
        on_deep_stack(|| {
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
        })
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

/// The stack the parsers get. roxmltree, saphyr and toml recurse per level of nesting, and a
/// debug build of roxmltree overflows the window thread's megabyte at six hundred levels, so
/// every parse and pretty-print runs on a scoped thread with this reservation (address space,
/// committed only as used).
const DEEP_STACK: usize = 256 * 1024 * 1024;

fn on_deep_stack<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DEEP_STACK)
            .spawn_scoped(scope, work)
            .expect("a thread for the parser")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// How deep YAML and XML may nest. roxmltree recurses per element with no limit of its own, and
/// saphyr's loader limits flow collections only (compact block sequences put a level in two
/// bytes), so the text is checked before it is built; toml stops at about 80 with its own message.
const MAX_DEPTH: usize = 200;

/// How much YAML anchors and aliases may stand for in one file, in nodes and bytes of scalar
/// text together: the loader keeps a copy of every anchored node and copies it again for
/// every alias, so a few lines can ask for billions. A big file may anchor big things, so the
/// budget is this or eight times the text, whichever is more.
const ALIAS_BUDGET: usize = 1_000_000;

fn alias_budget(text: &str) -> usize {
    ALIAS_BUDGET.max(text.len().saturating_mul(8))
}

/// Refuse YAML nested past `MAX_DEPTH`, or whose anchors and aliases stand for more than
/// `ALIAS_BUDGET`, by counting the parser's events, which it produces without recursion. An
/// alias counts as the depth and the size of what it names; an anchored collection counts
/// once more at its end, for the copy the loader keeps. A scan error here is left to the
/// loader, which reports it.
fn check_yaml_depth(text: &str) -> Result<(), ParseError> {
    use saphyr_parser::Event;
    // Every open collection: where it starts, its anchor, the depth of its own node, the
    // deepest level inside it, and its size in nodes and bytes.
    struct Open {
        line: usize,
        column: usize,
        anchor: usize,
        depth: usize,
        deepest: usize,
        size: usize,
    }
    let mut open: Vec<Open> = Vec::new();
    // Per anchor id: the levels below the anchored node, and what it holds.
    let mut heights: Vec<usize> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();
    let mut expanded = 0usize;
    let budget = alias_budget(text);
    let at = |span: &saphyr_parser::Span| (span.start.line().max(1), span.start.col() + 1);
    let too_deep = |line: usize, column: usize, suffix: &str| ParseError {
        line,
        column,
        message: format!("values nested deeper than {MAX_DEPTH} levels{suffix}"),
    };
    let too_much = |line: usize, column: usize| ParseError {
        line,
        column,
        message: format!("anchors and aliases stand for more than {budget} nodes and bytes"),
    };
    let remember = |heights: &mut Vec<usize>, sizes: &mut Vec<usize>, anchor: usize, height: usize, size: usize| {
        if anchor > 0 {
            if heights.len() <= anchor {
                heights.resize(anchor + 1, 0);
                sizes.resize(anchor + 1, 0);
            }
            heights[anchor] = height;
            sizes[anchor] = size;
        }
    };
    for event in saphyr_parser::Parser::new_from_str(text) {
        let Ok((event, span)) = event else { return Ok(()) };
        let depth = open.len();
        let (line, column) = at(&span);
        match event {
            Event::SequenceStart(anchor, _) | Event::MappingStart(anchor, _) => {
                if depth + 1 > MAX_DEPTH {
                    return Err(too_deep(line, column, ""));
                }
                open.push(Open { line, column, anchor, depth: depth + 1, deepest: depth + 1, size: 1 });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let Some(done) = open.pop() else { continue };
                remember(&mut heights, &mut sizes, done.anchor, done.deepest - done.depth, done.size);
                if done.anchor > 0 {
                    expanded += done.size;
                    if expanded > budget {
                        return Err(too_much(done.line, done.column));
                    }
                }
                if let Some(parent) = open.last_mut() {
                    parent.deepest = parent.deepest.max(done.deepest);
                    parent.size += done.size;
                }
            }
            Event::Scalar(value, _, anchor, _) => {
                let size = 1 + value.len();
                remember(&mut heights, &mut sizes, anchor, 0, size);
                if anchor > 0 {
                    expanded += size;
                    if expanded > budget {
                        return Err(too_much(line, column));
                    }
                }
                if let Some(parent) = open.last_mut() {
                    parent.deepest = parent.deepest.max(depth + 1);
                    parent.size += size;
                }
            }
            Event::Alias(anchor) => {
                let height = heights.get(anchor).copied().unwrap_or(0);
                let size = sizes.get(anchor).copied().unwrap_or(1);
                if depth + 1 + height > MAX_DEPTH {
                    return Err(too_deep(line, column, " through an alias"));
                }
                expanded += size;
                if expanded > budget {
                    return Err(too_much(line, column));
                }
                if let Some(parent) = open.last_mut() {
                    parent.deepest = parent.deepest.max(depth + 1 + height);
                    parent.size += size;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn too_deep(text: &str, offset: usize, what: &str) -> ParseError {
    let (line, column) = line_column(text, offset);
    ParseError { line, column, message: format!("{what} nested deeper than {MAX_DEPTH} levels") }
}

/// How many nodes an XML document may expand to beyond its own size: the file's own markup
/// never makes more nodes than it has bytes, so only entity expansion can reach this.
const XML_EXPANSION_LIMIT: u32 = 1_000_000;

fn xml_nodes_limit(text: &str) -> u32 {
    u32::try_from(text.len()).unwrap_or(u32::MAX).saturating_add(XML_EXPANSION_LIMIT)
}

/// Refuse `text` when its elements nest deeper than `MAX_DEPTH`, in the document and inside any
/// entity value of its DOCTYPE (the markup there is parsed where the entity is referenced).
fn check_xml_depth(text: &str) -> Result<(), ParseError> {
    if let Some(offset) = xml_tag_overflow(text) {
        return Err(too_deep(text, offset, "elements"));
    }
    // Markup inside an entity's value is parsed where the entity is referenced: each value must
    // stay within the bound on its own (character references for `<` included).
    if let Some(doctype) = xml_prolog(text).1 {
        let base = doctype.as_ptr() as usize - text.as_ptr() as usize;
        for entity in xml_entities(doctype) {
            if xml_tag_overflow(&decode_lt_references(&entity.value)).is_some() {
                return Err(too_deep(text, base + entity.offset, "elements"));
            }
        }
    }
    Ok(())
}

/// An entity value with `&#60;` or `&#x3c;` holds a `<` that becomes markup where it is referenced.
fn decode_lt_references(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find("&#") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 2..];
        let (digits, radix) = match tail.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16),
            None => (tail, 10),
        };
        let len = digits.find(';').unwrap_or(0);
        match u32::from_str_radix(&digits[..len], radix) {
            Ok(60) if len > 0 => {
                out.push('<');
                rest = &digits[len + 1..];
            }
            _ => {
                out.push_str("&#");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// How many bytes XML entities may stand for in one file: a million, or eight times the text,
/// whichever is more. roxmltree bounds the nodes an expansion makes, not its text.
fn xml_expansion_budget(text: &str) -> usize {
    1_000_000usize.max(text.len().saturating_mul(8))
}

/// XML's own whitespace: space, tab, carriage return and line feed (Rust's `is_whitespace`
/// takes in characters XML allows inside names).
fn xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Past the `-->` that closes the comment starting `text`, looked for after the opener: a
/// comment written `<!-->` does not close itself (roxmltree reads it the same way).
fn xml_comment_end(text: &str) -> Option<usize> {
    text.get(4..)?.find("-->").map(|e| e + 7)
}

/// A general entity the DOCTYPE declares inline: its name, its value as written, and where its
/// declaration starts in the DOCTYPE.
struct Entity {
    name: String,
    value: String,
    offset: usize,
}

/// The length of the `<!...>` declaration that starts `text`, as roxmltree reads it: an
/// `<!ENTITY` ends at its `>` outside quotes, every other declaration at its first `>`.
fn xml_declaration_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    if !text.starts_with("<!ENTITY") {
        return text.find('>').map_or(text.len(), |e| e + 1);
    }
    let mut i = "<!ENTITY".len();
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                i += 1;
            }
            b'>' => return i + 1,
            _ => i += 1,
        }
    }
    text.len()
}

/// Where a `<!DOCTYPE ...>` that starts `text` ends: after its `>`, or after the `>` that
/// follows its internal subset's `]`. The subset is walked declaration by declaration, as
/// roxmltree walks it, so a `]`, a quote or a `>` inside a declaration, a comment or a
/// processing instruction ends nothing.
fn doctype_end(text: &str) -> Option<usize> {
    let header = xml_doctype_header_end(text);
    if !text[..header].ends_with('[') {
        return Some(header);
    }
    let mut i = header;
    loop {
        let rest = &text[i..];
        let rest_trimmed = rest.trim_start_matches(xml_space);
        i += rest.len() - rest_trimmed.len();
        if rest_trimmed.is_empty() {
            return None;
        }
        if rest_trimmed.starts_with("<!--") {
            i += xml_comment_end(rest_trimmed)?;
        } else if rest_trimmed.starts_with("<?") {
            i += rest_trimmed.find("?>")? + 2;
        } else if rest_trimmed.starts_with("<!") {
            i += xml_declaration_end(rest_trimmed);
        } else if rest_trimmed.starts_with(']') {
            let close = rest_trimmed.find('>')?;
            return Some(i + close + 1);
        } else {
            // A parameter entity reference or stray text: step to the next marker.
            i += rest_trimmed.find(['<', ']']).filter(|&p| p > 0).unwrap_or_else(|| rest_trimmed.chars().next().map_or(1, char::len_utf8));
        }
    }
}

/// The general entities a DOCTYPE declares inline, by name, with their values as written;
/// the subset is walked as `doctype_end` walks it, so a `<!ENTITY` inside a comment, a
/// processing instruction or another declaration is not taken for a declaration.
fn xml_entities(doctype: &str) -> Vec<Entity> {
    let mut entities = Vec::new();
    let header = xml_doctype_header_end(doctype);
    if !doctype[..header].ends_with('[') {
        return entities;
    }
    let mut i = header;
    while i < doctype.len() {
        let rest = &doctype[i..];
        let rest_trimmed = rest.trim_start_matches(xml_space);
        i += rest.len() - rest_trimmed.len();
        if rest_trimmed.starts_with("<!--") {
            i += xml_comment_end(rest_trimmed).unwrap_or(rest_trimmed.len());
        } else if rest_trimmed.starts_with("<?") {
            i += rest_trimmed.find("?>").map_or(rest_trimmed.len(), |e| e + 2);
        } else if rest_trimmed.starts_with("<!ENTITY") {
            let after = rest_trimmed["<!ENTITY".len()..].trim_start_matches(xml_space);
            let (name, after_name) = after.split_once(xml_space).unwrap_or((after, ""));
            let after_name = after_name.trim_start_matches(xml_space);
            if let Some(quote) = after_name.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let value = after_name[1..].split(quote).next().unwrap_or("");
                if !name.starts_with('%') {
                    entities.push(Entity { name: name.to_string(), value: value.to_string(), offset: i });
                }
            }
            i += xml_declaration_end(rest_trimmed);
        } else if rest_trimmed.starts_with("<!") {
            i += xml_declaration_end(rest_trimmed);
        } else if rest_trimmed.starts_with(']') || rest_trimmed.is_empty() {
            break;
        } else {
            i += rest_trimmed.find(['<', ']']).filter(|&p| p > 0).unwrap_or_else(|| rest_trimmed.chars().next().map_or(1, char::len_utf8));
        }
    }
    entities
}

/// The end of the `<!DOCTYPE ...` header that starts `text`: just past its `[` outside quotes,
/// or past its `>` when it has no internal subset.
fn xml_doctype_header_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut i = "<!DOCTYPE".len();
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                i += 1;
            }
            b'[' | b'>' => return i + 1,
            _ => i += 1,
        }
    }
    text.len()
}

/// The bytes each entity stands for once the entities inside its value are expanded too,
/// computed once per entity with an index by name (the first declaration wins, as in
/// roxmltree) and an explicit stack, so many declarations or a long chain cost no more than
/// their text. A chain back to itself counts its text once; roxmltree refuses such a file.
fn xml_expanded_lengths(entities: &[Entity]) -> std::collections::HashMap<String, usize> {
    use std::collections::HashMap;
    let mut index: HashMap<&str, &str> = HashMap::new();
    for entity in entities {
        index.entry(entity.name.as_str()).or_insert(entity.value.as_str());
    }
    let references = |value: &str| -> Vec<String> {
        let mut refs = Vec::new();
        let mut rest = value;
        while let Some(at) = rest.find('&') {
            let tail = &rest[at + 1..];
            match tail.find(';') {
                Some(end) if !tail.starts_with('#') && index.contains_key(&tail[..end]) => {
                    refs.push(tail[..end].to_string());
                    rest = &tail[end + 1..];
                }
                _ => rest = tail,
            }
        }
        refs
    };
    let mut memo: HashMap<String, usize> = HashMap::new();
    for Entity { name, .. } in entities {
        if memo.contains_key(name.as_str()) {
            continue;
        }
        let value = index[name.as_str()];
        memo.insert(name.clone(), value.len());
        // Frames: the entity, the entities its value references, the next one to take, the total so far.
        let mut stack: Vec<(String, Vec<String>, usize, usize)> = vec![(name.clone(), references(value), 0, value.len())];
        loop {
            let Some(top) = stack.last_mut() else { break };
            if top.2 < top.1.len() {
                let reference = top.1[top.2].clone();
                top.2 += 1;
                match memo.get(&reference).copied() {
                    Some(known) => top.3 = top.3.saturating_sub(reference.len() + 2).saturating_add(known),
                    None => {
                        let value = index[reference.as_str()];
                        memo.insert(reference.clone(), value.len());
                        stack.push((reference, references(value), 0, value.len()));
                    }
                }
            } else {
                let (done, _, _, total) = stack.pop().expect("a frame");
                memo.insert(done.clone(), total);
                if let Some(parent) = stack.last_mut() {
                    parent.3 = parent.3.saturating_sub(done.len() + 2).saturating_add(total);
                }
            }
        }
    }
    memo
}

/// Refuse `text` when the entities its content references stand for more than the budget.
fn check_xml_expansion(text: &str) -> Result<(), ParseError> {
    let Some(doctype) = xml_prolog(text).1 else { return Ok(()) };
    let entities = xml_entities(doctype);
    if entities.is_empty() {
        return Ok(());
    }
    let lengths = xml_expanded_lengths(&entities);
    let budget = xml_expansion_budget(text);
    let doctype_end = doctype.as_ptr() as usize - text.as_ptr() as usize + doctype.len();
    let body = &text[doctype_end..];
    let bytes = body.as_bytes();
    let mut total = 0usize;
    // The text is sliced at `<` and `&` only, so a step over a multi-byte character is safe.
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                let rest = &body[i..];
                if rest.starts_with("<!--") {
                    i += xml_comment_end(rest).unwrap_or(rest.len());
                } else if rest.starts_with("<![CDATA[") {
                    i += rest.find("]]>").map_or(rest.len(), |e| e + 3);
                } else if rest.starts_with("<?") {
                    i += rest.find("?>").map_or(rest.len(), |e| e + 2);
                } else {
                    i += 1;
                }
            }
            b'&' => {
                let rest = &body[i..];
                let Some(end) = rest.find(';') else { break };
                if !rest.starts_with("&#")
                    && let Some(&expanded) = lengths.get(&rest[1..end])
                {
                    total = total.saturating_add(expanded);
                    if total > budget {
                        let (line, column) = line_column(text, doctype_end + i);
                        return Err(ParseError { line, column, message: format!("entities expand to more than {budget} bytes") });
                    }
                }
                i += end + 1;
            }
            _ => i += 1,
        }
    }
    Ok(())
}

/// The offset of the start tag that nests deeper than `MAX_DEPTH`, if any; comments, CDATA,
/// processing instructions and the DOCTYPE are skipped, quotes inside tags honoured.
fn xml_tag_overflow(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += xml_comment_end(rest).unwrap_or(rest.len());
        } else if rest.starts_with("<![CDATA[") {
            i += rest.find("]]>").map_or(rest.len(), |e| e + 3);
        } else if rest.starts_with("<?") {
            i += rest.find("?>").map_or(rest.len(), |e| e + 2);
        } else if rest.starts_with("<!DOCTYPE") {
            // A DOCTYPE whose end the scan cannot find is scanned through, never skipped.
            i += doctype_end(rest).unwrap_or("<!DOCTYPE".len());
        } else if rest.starts_with("</") {
            depth = depth.saturating_sub(1);
            i += rest.find('>').map_or(rest.len(), |e| e + 1);
        } else {
            // A start tag: find its `>` outside quotes; `/>` closes the element at once.
            let start = i;
            let mut j = i + 1;
            let mut self_closing = false;
            while j < bytes.len() {
                match bytes[j] {
                    quote @ (b'"' | b'\'') => {
                        j += 1;
                        while j < bytes.len() && bytes[j] != quote {
                            j += 1;
                        }
                    }
                    b'>' => {
                        self_closing = j > 0 && bytes[j - 1] == b'/';
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            if !self_closing {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Some(start);
                }
            }
            i = j + 1;
        }
    }
    None
}

/// The 1-based line and column of a byte offset.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

// ---- JSON and JSON Lines: serde_json checks the text, a scan of the valid text fills the arena ----

/// serde_json's message without its own " at line N column M" tail, which the error's
/// line and column carry.
fn json_message(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.rfind(" at line ") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

/// serde_json counts columns in bytes; the editor and the status row count characters.
fn json_error(error: serde_json::Error, text: &str, line_offset: usize) -> ParseError {
    let line_text = text.lines().nth(error.line().saturating_sub(1)).unwrap_or("");
    let byte_column = error.column().max(1);
    let column = line_text.get(..byte_column - 1).map_or(byte_column, |before| before.chars().count() + 1);
    ParseError { line: error.line() + line_offset, column, message: json_message(&error) }
}

/// A scan of JSON that serde_json has accepted: numbers keep the file's own text, strings
/// are unescaped, and the arena is filled with no tree in between.
struct JsonScan<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl JsonScan<'_> {
    fn skip_whitespace(&mut self) {
        while self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.bytes.get(self.pos).copied().unwrap_or(0)
    }

    /// Scan one value with its subtree into the arena. Containers are kept on an explicit
    /// stack, so a file nested a hundred thousand levels deep costs no call frames.
    fn value(&mut self, nodes: &mut Vec<Node>, key: String, depth: u32) {
        // Open containers: the node index, whether it is an object, and its entries so far.
        let mut open: Vec<(usize, bool, usize)> = Vec::new();
        let mut key = key;
        loop {
            self.skip_whitespace();
            let depth = depth + open.len() as u32;
            match self.peek() {
                b'{' => {
                    self.pos += 1;
                    let ix = push(nodes, depth, key, Kind::Object, String::new());
                    open.push((ix, true, 0));
                }
                b'[' => {
                    self.pos += 1;
                    let ix = push(nodes, depth, key, Kind::Array, String::new());
                    open.push((ix, false, 0));
                }
                b'"' => {
                    let text = self.string();
                    push(nodes, depth, key, Kind::String, text);
                }
                b't' => {
                    self.pos += 4;
                    push(nodes, depth, key, Kind::Boolean, "true".to_string());
                }
                b'f' => {
                    self.pos += 5;
                    push(nodes, depth, key, Kind::Boolean, "false".to_string());
                }
                b'n' => {
                    self.pos += 4;
                    push(nodes, depth, key, Kind::Null, "null".to_string());
                }
                _ => {
                    let start = self.pos;
                    while self.pos < self.bytes.len()
                        && !matches!(self.bytes[self.pos], b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r')
                    {
                        self.pos += 1;
                    }
                    push(nodes, depth, key, Kind::Number, self.text[start..self.pos].to_string());
                }
            }
            // The next entry of the innermost open container, closing containers on the way.
            loop {
                let Some(&(ix, is_object, count)) = open.last() else { return };
                self.skip_whitespace();
                match self.peek() {
                    b',' => {
                        self.pos += 1;
                    }
                    b'}' | b']' => {
                        self.pos += 1;
                        close(nodes, ix, count);
                        open.pop();
                    }
                    _ => {
                        key = if is_object {
                            let name = self.string();
                            self.skip_whitespace();
                            self.pos += 1; // the colon
                            name
                        } else {
                            count.to_string()
                        };
                        open.last_mut().expect("an open container").2 += 1;
                        break;
                    }
                }
            }
        }
    }

    /// The string at the cursor (on its opening quote), unescaped; the cursor ends after
    /// the closing quote.
    fn string(&mut self) -> String {
        self.pos += 1;
        let mut out = String::new();
        let mut start = self.pos;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    out.push_str(&self.text[start..self.pos]);
                    self.pos += 1;
                    return out;
                }
                b'\\' => {
                    out.push_str(&self.text[start..self.pos]);
                    self.pos += 1;
                    let escape = self.peek();
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => self.unicode_escape(&mut out),
                        _ => {}
                    }
                    start = self.pos;
                }
                _ => self.pos += 1,
            }
        }
        out.push_str(&self.text[start.min(self.text.len())..]);
        out
    }

    /// The four hex digits after `\u`, as a char; a high surrogate takes the low one that
    /// follows, and stands as U+FFFD when none does (the next escape is then read on its own).
    fn unicode_escape(&mut self, out: &mut String) {
        let first = self.hex4();
        if (0xD800..=0xDBFF).contains(&first) {
            let after = self.pos.min(self.text.len());
            if self.text[after..].starts_with("\\u") {
                self.pos += 2;
                let second = self.hex4();
                if (0xDC00..=0xDFFF).contains(&second) {
                    let code = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                    out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    return;
                }
                self.pos = after;
            }
            out.push('\u{FFFD}');
            return;
        }
        out.push(char::from_u32(first).unwrap_or('\u{FFFD}'));
    }

    fn hex4(&mut self) -> u32 {
        let digits = self.text.get(self.pos..self.pos + 4).unwrap_or("");
        self.pos += 4;
        u32::from_str_radix(digits, 16).unwrap_or(0xFFFD)
    }
}

/// Fill the arena from `text`, which `check_json` has accepted.
fn scan_json(text: &str, nodes: &mut Vec<Node>, key: &str, depth: u32) {
    let mut scan = JsonScan { text, bytes: text.as_bytes(), pos: 0 };
    scan.value(nodes, key.to_string(), depth);
}

fn parse_json(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    check_json(text, 0)?;
    scan_json(text, nodes, "$", 0);
    Ok(())
}

/// One record per non-blank line under a root `$` array; a bad line is reported by its number.
fn parse_json_lines(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    let root = push(nodes, 0, "$".to_string(), Kind::Array, String::new());
    let mut count = 0;
    for (line_ix, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        check_json(line, line_ix)?;
        scan_json(line, nodes, &count.to_string(), 1);
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
        .map_err(|e| json_error(e, text, line_offset))
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
    check_yaml_depth(text)?;
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

/// An element's name as written in the file, prefix included: the namespace lookup cannot
/// tell two prefixes bound to one URI apart, the source text can.
fn xml_element_name(node: roxmltree::Node) -> String {
    let input = node.document().input_text();
    let range = node.range();
    let after = &input[(range.start + 1).min(input.len())..range.end.min(input.len())];
    after.split(|c: char| xml_space(c) || c == '/' || c == '>').next().unwrap_or_default().to_string()
}

/// An attribute's name as written in the file, prefix included.
fn xml_attribute_name(input: &str, attribute: &roxmltree::Attribute) -> String {
    let range = attribute.range();
    let text = &input[range.start.min(input.len())..range.end.min(input.len())];
    text.split('=').next().unwrap_or_default().trim_matches(xml_space).to_string()
}

fn xml_children<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| !(child.is_text() && child.text().is_some_and(|t| t.trim_matches(xml_space).is_empty())))
        .collect()
}

fn walk_xml(node: roxmltree::Node, nodes: &mut Vec<Node>, depth: u32) -> Result<(), ParseError> {
    use roxmltree::NodeType;
    match node.node_type() {
        NodeType::Root => {}
        NodeType::Element => {
            if depth as usize >= MAX_DEPTH {
                return Err(too_deep(node.document().input_text(), node.range().start, "elements"));
            }
            let ix = push(nodes, depth, xml_element_name(node), Kind::Element, String::new());
            let mut count = 0;
            let input = node.document().input_text();
            for attribute in node.attributes() {
                let name = xml_attribute_name(input, &attribute);
                push(nodes, depth + 1, format!("@{name}"), Kind::Attribute, attribute.value().to_string());
                count += 1;
            }
            let children = xml_children(node);
            if children.len() == 1 && children[0].is_text() {
                let text = children[0].text().unwrap_or_default().trim_matches(xml_space).to_string();
                let end = nodes.len();
                nodes[ix].end = end;
                nodes[ix].value = text;
                return Ok(());
            }
            for child in children {
                walk_xml(child, nodes, depth + 1)?;
                count += 1;
            }
            close(nodes, ix, count);
        }
        NodeType::Text => {
            let text = node.text().unwrap_or_default().trim_matches(xml_space).to_string();
            if !text.is_empty() {
                push(nodes, depth, "text()".to_string(), Kind::Text, text);
            }
        }
        NodeType::Comment => {
            push(nodes, depth, "comment()".to_string(), Kind::Comment, node.text().unwrap_or_default().trim_matches(xml_space).to_string());
        }
        NodeType::PI => {
            let pi = node.pi().map(|pi| match pi.value {
                Some(value) => format!("{} {}", pi.target, value.trim_matches(xml_space)),
                None => pi.target.to_string(),
            });
            push(nodes, depth, "processing-instruction()".to_string(), Kind::Instruction, pi.unwrap_or_default());
        }
    }
    Ok(())
}

fn load_xml(text: &str) -> Result<roxmltree::Document<'_>, ParseError> {
    check_xml_depth(text)?;
    check_xml_expansion(text)?;
    // Entities may add a million nodes at most: a few lines can ask for billions otherwise.
    let limit = xml_nodes_limit(text);
    let options = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: limit, ..Default::default() };
    roxmltree::Document::parse_with_options(text, options).map_err(|error| {
        let at = error.pos();
        let message = match error {
            roxmltree::Error::NodesLimitReached => format!("the document expands to more than {limit} nodes"),
            _ => xml_message(&error.to_string()),
        };
        ParseError { line: at.row as usize, column: at.col as usize, message }
    })
}

/// The byte range of the DOCTYPE, whose inner comments and processing instructions roxmltree
/// reports as children of the document: they belong to the DOCTYPE, not to the tree.
fn xml_doctype_range(text: &str) -> Option<std::ops::Range<usize>> {
    let doctype = xml_prolog(text).1?;
    let start = doctype.as_ptr() as usize - text.as_ptr() as usize;
    Some(start..start + doctype.len())
}

fn parse_xml(text: &str, nodes: &mut Vec<Node>) -> Result<(), ParseError> {
    let document = load_xml(text)?;
    let doctype = xml_doctype_range(text);
    for child in document.root().children() {
        if doctype.as_ref().is_some_and(|range| range.contains(&child.range().start)) {
            continue;
        }
        walk_xml(child, nodes, 0)?;
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
    on_deep_stack(|| pretty_on_this_stack(format, text))
}

fn pretty_on_this_stack(format: Format, text: &str) -> Result<String, ParseError> {
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
        Format::Toml => pretty_toml(text),
        Format::Ini => pretty_ini(text),
    }
}

/// The XML declaration and the DOCTYPE of the prolog, as written: a scan of what comes
/// before the root element, skipping processing instructions and comments.
fn xml_prolog(text: &str) -> (Option<&str>, Option<&str>) {
    let mut declaration = None;
    let mut doctype = None;
    let mut rest = text;
    let mut first = true;
    loop {
        // A byte-order mark that decoding left in place is skipped here, as roxmltree skips it.
        let trimmed = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}');
        // The declaration is `<?xml ` (a space) at the very start and nothing else, as roxmltree
        // reads it; `<?xml` followed by a tab or a line break is a processing instruction.
        let at_start = first && rest.trim_start_matches('\u{FEFF}').len() == trimmed.len();
        first = false;
        if let Some(after) = trimmed.strip_prefix("<?xml ").filter(|_| at_start) {
            let Some(end) = after.find("?>") else { break };
            let end = "<?xml ".len() + end + "?>".len();
            declaration = Some(&trimmed[..end]);
            rest = &trimmed[end..];
        } else if trimmed.starts_with("<?") {
            let Some(end) = trimmed.find("?>") else { break };
            rest = &trimmed[end + 2..];
        } else if trimmed.starts_with("<!--") {
            let Some(end) = xml_comment_end(trimmed) else { break };
            rest = &trimmed[end..];
        } else if trimmed.starts_with("<!DOCTYPE") {
            let Some(end) = doctype_end(trimmed) else { break };
            doctype = Some(&trimmed[..end]);
            rest = &trimmed[end..];
        } else {
            break;
        }
    }
    (declaration, doctype)
}

fn pretty_xml(text: &str) -> Result<String, ParseError> {
    // The same walk as the tree's, so a file too deep for it is refused here too.
    parse_xml(text, &mut Vec::new())?;
    let document = load_xml(text)?;
    let mut out = String::new();
    let (declaration, doctype) = xml_prolog(text);
    if let Some(declaration) = declaration {
        out.push_str(declaration);
        out.push('\n');
    }
    if let Some(doctype) = doctype {
        out.push_str(doctype);
        out.push('\n');
    }
    let doctype = xml_doctype_range(text);
    for child in document.root().children() {
        if doctype.as_ref().is_some_and(|range| range.contains(&child.range().start)) {
            continue;
        }
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
            '\t' if attribute => out.push_str("&#9;"),
            '\n' if attribute => out.push_str("&#10;"),
            // A carriage return is normalised to a line feed wherever it is read back, so it is
            // written as a reference in text too.
            '\r' => out.push_str("&#13;"),
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
            let name = xml_element_name(node);
            let input = node.document().input_text();
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
                let attribute_name = xml_attribute_name(input, &attribute);
                out.push_str(&format!(" {attribute_name}=\"{}\"", escape_xml(attribute.value(), true)));
            }
            let children = xml_children(node);
            if children.is_empty() {
                out.push_str("/>\n");
            } else if children.len() == 1 && children[0].is_text() {
                out.push('>');
                out.push_str(&escape_xml(children[0].text().unwrap_or_default().trim_matches(xml_space), false));
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
            let text = node.text().unwrap_or_default().trim_matches(xml_space);
            if !text.is_empty() {
                out.push_str(&format!("{indent}{}\n", escape_xml(text, false)));
            }
        }
        NodeType::Comment => {
            out.push_str(&format!("{indent}<!-- {} -->\n", node.text().unwrap_or_default().trim_matches(xml_space)));
        }
        NodeType::PI => {
            if let Some(pi) = node.pi() {
                // An instruction whose target is `xml` is written with a tab after it: with a space it
                // would read as a declaration, which may stand at the start only.
                let separator = if pi.target.eq_ignore_ascii_case("xml") { '\t' } else { ' ' };
                match pi.value.map(|value| value.trim_matches(xml_space)).filter(|value| !value.is_empty()) {
                    Some(value) => out.push_str(&format!("{indent}<?{}{separator}{value}?>\n", pi.target)),
                    None => out.push_str(&format!("{indent}<?{}?>\n", pi.target)),
                }
            }
        }
    }
}

/// The file's own layout through the TOML document model: keys and tables keep their order
/// and values their spelling, the whitespace is the canonical one; comments do not survive.
fn pretty_toml(text: &str) -> Result<String, ParseError> {
    load_toml(text)?;
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| ParseError { line: 1, column: 1, message: error.message().to_string() })?;
    fmt_toml_table(document.as_table_mut());
    // Comments after the last line live in the document's trailing decor, not in any table.
    document.set_trailing("");
    let out = document.to_string();
    let mut out = out.trim_start_matches('\n').to_string();
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

fn fmt_toml_table(table: &mut toml_edit::Table) {
    table.fmt();
    table.decor_mut().clear();
    for (mut key, item) in table.iter_mut() {
        key.fmt();
        fmt_toml_item(item);
    }
}

fn fmt_toml_item(item: &mut toml_edit::Item) {
    match item {
        toml_edit::Item::Table(table) => fmt_toml_table(table),
        toml_edit::Item::ArrayOfTables(tables) => {
            for table in tables.iter_mut() {
                fmt_toml_table(table);
            }
        }
        toml_edit::Item::Value(value) => fmt_toml_value(value),
        toml_edit::Item::None => {}
    }
}

fn fmt_toml_value(value: &mut toml_edit::Value) {
    match value {
        toml_edit::Value::InlineTable(table) => {
            // Rebuilt from its entries: a multi-line inline table keeps comments and line breaks
            // in a preamble that `fmt` leaves alone.
            let mut fresh = toml_edit::InlineTable::new();
            for (key, value) in table.iter() {
                fresh.insert(key, value.clone());
            }
            *table = fresh;
            table.fmt();
            for (mut key, value) in table.iter_mut() {
                key.fmt();
                fmt_toml_value(value);
            }
        }
        toml_edit::Value::Array(array) => {
            array.fmt();
            for value in array.iter_mut() {
                fmt_toml_value(value);
            }
        }
        other => other.decor_mut().clear(),
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
        // Scalars at the root; numbers keep the file's own text, exponents and long digit runs included.
        let scalar = Tree::parse(Format::Json, "42").unwrap();
        assert_eq!(rows(&scalar), vec![(0, "$", "number", "42")]);
        let numbers = Tree::parse(Format::Json, "[1.0, 2.5e10, -0.0, 12345678901234567890123, 0.1000, 1E5, 1e400, 1.0e1, -1E-0]").unwrap();
        let values: Vec<&str> = numbers.nodes[1..].iter().map(|n| n.value.as_str()).collect();
        assert_eq!(values, vec!["1.0", "2.5e10", "-0.0", "12345678901234567890123", "0.1000", "1E5", "1e400", "1.0e1", "-1E-0"]);
        assert!(numbers.nodes[1..].iter().all(|n| n.kind == Kind::Number));
        // Strings are unescaped, surrogate pairs included.
        let escaped = Tree::parse(Format::Json, r#"["a\"b\\c\/d\n\u00e9\ud83d\ude00", "\u0041"]"#).unwrap();
        assert_eq!(escaped.nodes[1].value, "a\"b\\c/d\né😀");
        assert_eq!(escaped.nodes[2].value, "A");
        // A high surrogate without its low one stands as U+FFFD, and the escape after it is read on its own.
        let lone = Tree::parse(Format::Json, r#"["\uD800\u0041", "\uD800\uD83D\uDE00", "\uD800", "\uDC00A"]"#).unwrap();
        let values: Vec<&str> = lone.nodes[1..].iter().map(|n| n.value.as_str()).collect();
        assert_eq!(values, vec!["\u{FFFD}A", "\u{FFFD}😀", "\u{FFFD}", "\u{FFFD}A"]);
        // Nesting costs no call frames: a hundred thousand levels build their arena on a small stack.
        let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
        let tree = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || Tree::parse(Format::Json, &deep).unwrap())
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(tree.len(), 100_001);
        assert_eq!(tree.levels(), 100_001);
        assert_eq!(tree.nodes[100_000].value, "1");
        assert_eq!(tree.nodes[0].end, 100_001);
        let empty = Tree::parse(Format::Json, r#"{"a": [], "b": {}}"#).unwrap();
        assert_eq!(rows(&empty), vec![(0, "$", "object", "2 entries"), (1, "a", "array", "0 entries"), (1, "b", "object", "0 entries")]);
        // Duplicate keys are two nodes, as the file has them.
        let twice = Tree::parse(Format::Json, r#"{"a": 1, "a": 2}"#).unwrap();
        assert_eq!(rows(&twice), vec![(0, "$", "object", "2 entries"), (1, "a", "number", "1"), (1, "a", "number", "2")]);
        // An object whose first key is serde_json's private number marker is an object like any other.
        let marker = "$serde_json::private::Number";
        let object = Tree::parse(Format::Json, &format!("{{\"{marker}\": 5}}")).unwrap();
        assert_eq!(rows(&object), vec![(0, "$", "object", "1 entry"), (1, marker, "number", "5")]);
        let object = Tree::parse(Format::Json, &format!("{{\"{marker}\": \"abc\"}}")).unwrap();
        assert_eq!(rows(&object), vec![(0, "$", "object", "1 entry"), (1, marker, "string", "abc")]);
        let object = Tree::parse(Format::Json, &format!("{{\"{marker}\": \"1\", \"b\": 2}}")).unwrap();
        assert_eq!(rows(&object), vec![(0, "$", "object", "2 entries"), (1, marker, "string", "1"), (1, "b", "number", "2")]);
        let object = Tree::parse(Format::Json, &format!("{{\"{marker}\": \"1\"}}")).unwrap();
        assert_eq!(rows(&object), vec![(0, "$", "object", "1 entry"), (1, marker, "string", "1")]);
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
        // Two prefixes bound to one URI, and the default namespace beside a prefix: the names stay as written.
        let twice = Tree::parse(Format::Xml, "<a:r xmlns:a=\"u\" xmlns:b=\"u\"><b:x/></a:r>").unwrap();
        assert_eq!(rows(&twice), vec![(0, "a:r", "element", "1 entry"), (1, "b:x", "element", "0 entries")]);
        let shared = Tree::parse(Format::Xml, "<r xmlns=\"u\" xmlns:p=\"u\" p:k=\"1\"><p:c/></r>").unwrap();
        assert_eq!(rows(&shared), vec![(0, "r", "element", "2 entries"), (1, "@p:k", "attribute", "1"), (1, "p:c", "element", "0 entries")]);
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
        // Comments and processing instructions inside the DOCTYPE stay there: they are not rows and are not written twice.
        let subset = "<!DOCTYPE r [<!-- c --><?pi x?>]><r/>";
        assert_eq!(rows(&Tree::parse(Format::Xml, subset).unwrap()), vec![(0, "r", "element", "0 entries")]);
        assert_eq!(pretty(Format::Xml, subset).unwrap(), "<!DOCTYPE r [<!-- c --><?pi x?>]>\n<r/>\n");
        assert_eq!(Tree::parse(Format::Xml, &pretty(Format::Xml, subset).unwrap()).unwrap().nodes, Tree::parse(Format::Xml, subset).unwrap().nodes);
        // A comment that opens with a dash does not close itself, and XML's whitespace is the only whitespace:
        // an ogham space mark stays inside names and values.
        let dashed = "<!-->--><!DOCTYPE r [<!-- inside --><?pi inside?>]><r/>";
        assert_eq!(rows(&Tree::parse(Format::Xml, dashed).unwrap()), vec![(0, "comment()", "comment", ">"), (0, "r", "element", "0 entries")]);
        assert_eq!(pretty(Format::Xml, dashed).unwrap(), "<!DOCTYPE r [<!-- inside --><?pi inside?>]>\n<!-- > -->\n<r/>\n");
        let ogham = format!("<a{0}b c{0}d=\"1\"><?p {0}?></a{0}b>", '\u{1680}');
        let tree = Tree::parse(Format::Xml, &ogham).unwrap();
        assert_eq!(rows(&tree), vec![(0, "a\u{1680}b", "element", "2 entries"), (1, "@c\u{1680}d", "attribute", "1"), (1, "processing-instruction()", "instruction", "p \u{1680}")]);
        let pretty_ogham = pretty(Format::Xml, &ogham).unwrap();
        assert_eq!(pretty_ogham, format!("<a{0}b c{0}d=\"1\">\n  <?p {0}?>\n</a{0}b>\n", '\u{1680}'));
        assert_eq!(Tree::parse(Format::Xml, &pretty_ogham).unwrap().nodes, tree.nodes);
        // An instruction whose target is `xml` is not the declaration (roxmltree reads only `<?xml ` at the
        // very start as one) and is written back so that it stays an instruction; a carriage-return
        // reference in text survives the round trip.
        for (xml_in, xml_out) in [
            ("<?xml\tversion=\"1.0\"?><r/>", "<?xml\tversion=\"1.0\"?>\n<r/>\n"),
            ("<r><?xml\tx?></r>", "<r>\n  <?xml\tx?>\n</r>\n"),
            ("<?xml version=\"1.0\"?><r/><?xml\nx?>", "<?xml version=\"1.0\"?>\n<r/>\n<?xml\tx?>\n"),
            ("<?xml\r\n?><r/>", "<?xml?>\n<r/>\n"),
            ("<r>a&#13;b</r>", "<r>a&#13;b</r>\n"),
        ] {
            let pretty_xml = pretty(Format::Xml, xml_in).unwrap();
            assert_eq!(pretty_xml, xml_out, "{xml_in:?}");
            assert_eq!(Tree::parse(Format::Xml, &pretty_xml).unwrap().nodes, Tree::parse(Format::Xml, xml_in).unwrap().nodes, "{xml_in:?}");
        }
        assert_eq!(Tree::parse(Format::Xml, "<r>a&#13;b</r>").unwrap().nodes[0].value, "a\rb");
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
        // TOML keeps the file's order and the values' spelling: dotted keys and inline tables stay where they are.
        let toml_order = "a.b.c = 1\nf = 1e3\npoint = {x=1}\nz = 2\n";
        let pretty_order = pretty(Format::Toml, toml_order).unwrap();
        assert_eq!(pretty_order, "a.b.c = 1\nf = 1e3\npoint = { x = 1 }\nz = 2\n");
        assert_eq!(Tree::parse(Format::Toml, &pretty_order).unwrap().nodes, Tree::parse(Format::Toml, toml_order).unwrap().nodes);
        // A multi-line inline table loses its comment and line break too, and dotted keys lose their spaces.
        let toml_inline = "[[arr]]\nz = { a = 1, # no\n}\nq . r = 2\n";
        let pretty_inline = pretty(Format::Toml, toml_inline).unwrap();
        assert_eq!(pretty_inline, "[[arr]]\nz = { a = 1 }\nq.r = 2\n");
        assert_eq!(Tree::parse(Format::Toml, &pretty_inline).unwrap().nodes, Tree::parse(Format::Toml, toml_inline).unwrap().nodes);
        // Comments after the last line go too; a file of comments alone comes back empty.
        assert_eq!(pretty(Format::Toml, "a = 1\n# end\n").unwrap(), "a = 1\n");
        assert_eq!(pretty(Format::Toml, "# only a comment\n").unwrap(), "");
        // XML: a processing instruction at the start is not the declaration, a DOCTYPE inside a comment is
        // not a DOCTYPE, an internal subset may end in `] >`, two prefixes for one URI keep their names, and
        // whitespace in attribute values is written as character references.
        for (xml_in, xml_out) in [
            ("<?xml-stylesheet href=\"a\"?><r/>", "<?xml-stylesheet href=\"a\"?>\n<r/>\n"),
            ("<!-- see <!DOCTYPE in docs --><r/>", "<!-- see <!DOCTYPE in docs -->\n<r/>\n"),
            ("<!DOCTYPE r [<!ENTITY e \"a>b\"> ] ><r>&e;</r>", "<!DOCTYPE r [<!ENTITY e \"a>b\"> ] >\n<r>a&gt;b</r>\n"),
            ("<a:r xmlns:a=\"u\" xmlns:b=\"u\"><b:x/></a:r>", "<a:r xmlns:a=\"u\" xmlns:b=\"u\">\n  <b:x/>\n</a:r>\n"),
            ("<r xmlns=\"u\" xmlns:p=\"u\" p:k=\"1\"><p:c/></r>", "<r xmlns=\"u\" xmlns:p=\"u\" p:k=\"1\">\n  <p:c/>\n</r>\n"),
            ("<a k=\"x&#9;y&#10;z\"/>", "<a k=\"x&#9;y&#10;z\"/>\n"),
        ] {
            let pretty_xml = pretty(Format::Xml, xml_in).unwrap();
            assert_eq!(pretty_xml, xml_out, "{xml_in}");
            assert_eq!(Tree::parse(Format::Xml, &pretty_xml).unwrap().nodes, Tree::parse(Format::Xml, xml_in).unwrap().nodes, "{xml_in}");
        }
    }

    #[test]
    fn parse_errors_name_their_line_and_column() {
        let json = Tree::parse(Format::Json, "{\n  \"a\": 1,\n  \"b\": [1, 2,\n}\n").unwrap_err();
        assert_eq!((json.line, json.column, json.message.as_str()), (4, 1, "expected value"));
        assert_eq!(json.to_string(), "line 4, column 1: expected value");
        let xml = Tree::parse(Format::Xml, "<a>\n  <b>\n</a>\n").unwrap_err();
        assert_eq!((xml.line, xml.column), (3, 1), "{xml}");
        assert!(!xml.message.contains(" at "), "{xml}");
        let yaml = Tree::parse(Format::Yaml, "a: 1\nb: [1, 2\nc: 3\n").unwrap_err();
        assert_eq!((yaml.line, yaml.column), (3, 2), "{yaml}");
        assert!(!yaml.message.is_empty(), "{yaml}");
        let twice = Tree::parse(Format::Yaml, "a: 1\na: 2\n").unwrap_err();
        assert_eq!((twice.line, twice.column), (2, 1), "{twice}");
        assert!(twice.message.contains("duplicate"), "{twice}");
        let toml = Tree::parse(Format::Toml, "a = 1\n[server\nhost = 1\n").unwrap_err();
        assert_eq!((toml.line, toml.column), (2, 8), "{toml}");
        assert!(!toml.message.is_empty(), "{toml}");
        // Columns count characters, as the editor does, not bytes.
        let accented = Tree::parse(Format::Json, "{\"é\": }").unwrap_err();
        assert_eq!((accented.line, accented.column), (1, 7), "{accented}");
        let ini = Tree::parse(Format::Ini, "[a]\nb = 1\n\nbroken line\n").unwrap_err();
        assert_eq!((ini.line, ini.column), (4, 1));
        // CRLF endings count lines the same way.
        let crlf = Tree::parse(Format::Json, "{\r\n  \"a\": 1,\r\n  \"b\": [1, 2,\r\n}\r\n").unwrap_err();
        assert_eq!((crlf.line, crlf.column), (4, 1));
        let crlf_toml = Tree::parse(Format::Toml, "a = 1\r\n[server\r\nhost = 1\r\n").unwrap_err();
        assert_eq!(crlf_toml.line, 2, "{crlf_toml}");
    }

    #[test]
    fn deep_nesting_in_every_format_parses_or_is_refused_but_never_crashes() {
        // The window's main thread has a megabyte of stack on Windows; each parser is called from a thread
        // of that size and must come back with the exact tree or the exact refusal, however the input nests.
        // JSON nests without limit; YAML stops at the check's 200 in block form and at the scanner's own 256
        // in flow form (its tokens run ahead of the events); XML at 200, entity values included; TOML at its
        // parser's limit of about eighty.
        let flow = |open: &str, close: &str, n: usize| format!("{}1{}\n", open.repeat(n), close.repeat(n));
        let block = |n: usize| (0..n).map(|i| format!("{}a:\n", "  ".repeat(i))).collect::<String>() + &"  ".repeat(n) + "1\n";
        let toml_limit = "line 1, column 85: cannot recurse further; max recursion depth met";
        let cases: Vec<(&str, Format, String, Result<(usize, usize), &str>)> = vec![
            ("json 50000", Format::Json, flow("[", "]", 50_000), Ok((50_001, 50_001))),
            ("jsonl 50000", Format::JsonLines, flow("[", "]", 50_000), Ok((50_002, 50_002))),
            ("yaml flow 5000", Format::Yaml, flow("[", "]", 5_000), Err("line 1, column 256: recursion limit exceeded")),
            ("yaml compact block 200000", Format::Yaml, "- ".repeat(200_000) + "x\n", Err("line 1, column 401: values nested deeper than 200 levels")),
            ("yaml compact block 150", Format::Yaml, "- ".repeat(150) + "x\n", Ok((151, 151))),
            ("yaml flow 150", Format::Yaml, flow("[", "]", 150), Ok((151, 151))),
            ("yaml block 1000", Format::Yaml, block(1000), Err("line 201, column 401: values nested deeper than 200 levels")),
            ("yaml block 150", Format::Yaml, block(150), Ok((151, 151))),
            ("yaml apostrophe then 150", Format::Yaml, format!("it's: 1\nb: {}", flow("[", "]", 150)), Ok((153, 152))),
            ("yaml apostrophe then 230", Format::Yaml, format!("it's: 1\nb: {}", flow("[", "]", 230)), Err("line 2, column 203: values nested deeper than 200 levels")),
            ("yaml block scalar of brackets", Format::Yaml, format!("a: |\n  {}\n", "[".repeat(250)), Ok((2, 2))),
            ("toml 5000", Format::Toml, format!("a = {}", flow("[", "]", 5_000)), Err(toml_limit)),
            ("toml 150", Format::Toml, format!("a = {}", flow("[", "]", 150)), Err(toml_limit)),
            ("toml 79", Format::Toml, format!("a = {}", flow("[", "]", 79)), Ok((81, 81))),
            ("xml 20000", Format::Xml, format!("{}x{}", "<a>".repeat(20_000), "</a>".repeat(20_000)), Err("line 1, column 601: elements nested deeper than 200 levels")),
            ("xml 150", Format::Xml, format!("{}x{}", "<a>".repeat(150), "</a>".repeat(150)), Ok((150, 150))),
            ("xml entity 20000", Format::Xml, format!("<!DOCTYPE r [<!ENTITY e \"{}x{}\">]><r>&e;</r>", "<a>".repeat(20_000), "</a>".repeat(20_000)), Err("line 1, column 14: elements nested deeper than 200 levels")),
            ("xml entity by char refs 20000", Format::Xml, format!("<!DOCTYPE r [<!ENTITY e \"{}x{}\">]><r>&e;</r>", "&#60;a>".repeat(20_000), "&#x3C;/a>".repeat(20_000)), Err("line 1, column 14: elements nested deeper than 200 levels")),
            ("xml pi with apostrophe in the doctype 20000", Format::Xml, format!("<!DOCTYPE r [<?pi it's?>]>{}x{}", "<a>".repeat(20_000), "</a>".repeat(20_000)), Err("line 1, column 627: elements nested deeper than 200 levels")),
            ("yaml alias chain", Format::Yaml, format!("a0: &a0 {}1{}\na1: &a1 {}*a0{}\n", "[".repeat(190), "]".repeat(190), "[".repeat(190), "]".repeat(190)), Err("line 2, column 199: values nested deeper than 200 levels through an alias")),
            ("yaml alias within bounds", Format::Yaml, format!("a0: &a0 {}1{}\na1: {}*a0{}\n", "[".repeat(100), "]".repeat(100), "[".repeat(50), "]".repeat(50)), Ok((253, 152))),
            ("yaml alias bomb", Format::Yaml, (1..6).fold("l0: &l0 [x, x, x, x, x, x, x, x, x, x]\n".to_string(), |text, i| text + &format!("l{i}: &l{i} [{}]\n", vec![format!("*l{}", i - 1); 10].join(", "))), Err("line 6, column 20: anchors and aliases stand for more than 1000000 nodes and bytes")),
            ("yaml alias bomb on long scalars", Format::Yaml, (1..5).fold(format!("l0: &l0 [{}]\n", vec!["y".repeat(1000); 10].join(", ")), |text, i| text + &format!("l{i}: &l{i} [{}]\n", vec![format!("*l{}", i - 1); 10].join(", "))) + "l5: [*l4, *l4, *l4, *l4, *l4, *l4, *l4]\n", Err("line 3, column 45: anchors and aliases stand for more than 1000000 nodes and bytes")),
            ("yaml nested anchors over a long list", Format::Yaml, format!("x: {}{}{}\n", (0..190).map(|i| format!("&a{i} [")).collect::<String>(), vec!["1"; 100_000].join(", "), "]".repeat(190)), Err("line 1, column 1139: anchors and aliases stand for more than 2411296 nodes and bytes")),
            ("xml entity amplification", Format::Xml, format!("<!DOCTYPE r [<!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r>{}</r>", "<x/>".repeat(1000), "&e0;".repeat(255), "&e1;".repeat(10)), Err("line 1, column 5069: entities expand to more than 1000000 bytes")),
            ("xml entity amplification into text", Format::Xml, format!("<!DOCTYPE r [<!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r>{}</r>", "a".repeat(10_000), "&e0;".repeat(255), "&e1;".repeat(40)), Err("line 1, column 11069: entities expand to more than 1000000 bytes")),
            ("xml entity amplification into an attribute", Format::Xml, format!("<!DOCTYPE r [<!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r a=\"{}\"/>", "a".repeat(10_000), "&e0;".repeat(255), "&e1;".repeat(40)), Err("line 1, column 11072: entities expand to more than 1000000 bytes")),
            ("xml decoy entity in a doctype comment", Format::Xml, format!("<!DOCTYPE r [<!-- <!ENTITY e1 \"a\"> --><!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r>{}</r>", "a".repeat(10_000), "&e0;".repeat(255), "&e1;".repeat(40)), Err("line 1, column 11094: entities expand to more than 1000000 bytes")),
            ("xml decoy entity in a doctype instruction", Format::Xml, format!("<!DOCTYPE r [<?x <!ENTITY e1 \"a\"> ?><!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r>{}</r>", "a".repeat(10_000), "&e0;".repeat(255), "&e1;".repeat(40)), Err("line 1, column 11092: entities expand to more than 1000000 bytes")),
            ("xml decoy entity in another value", Format::Xml, format!("<!DOCTYPE r [<!ENTITY d '<!ENTITY e1 \"a\">'><!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r>{}</r>", "a".repeat(10_000), "&e0;".repeat(255), "&e1;".repeat(40)), Err("line 1, column 11099: entities expand to more than 1000000 bytes")),
            ("xml billion laughs", Format::Xml, (1..=9).fold("<!DOCTYPE r [<!ENTITY lol0 \"lol\">".to_string(), |text, i| text + &format!("<!ENTITY lol{i} \"{}\">", format!("&lol{};", i - 1).repeat(10))) + "]><r>&lol9;</r>", Err("line 1, column 732: entities expand to more than 1000000 bytes")),
            ("xml entity 20000 behind a stray byte-order mark", Format::Xml, format!("{}<!DOCTYPE r [<!ENTITY e \"{}x{}\">]><r>&e;</r>", '\u{FEFF}', "<a>".repeat(20_000), "</a>".repeat(20_000)), Err("line 1, column 15: elements nested deeper than 200 levels")),
            ("xml amplification behind a stray byte-order mark", Format::Xml, format!("{}<!DOCTYPE r [<!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r>{}</r>", '\u{FEFF}', "a".repeat(10_000), "&e0;".repeat(255), "&e1;".repeat(40)), Err("line 1, column 11070: entities expand to more than 1000000 bytes")),
            ("xml decoy entity in the system literal", Format::Xml, format!("<!DOCTYPE r SYSTEM \"[<!ENTITY e1 'a'>\" [<!ENTITY e0 \"{}\"><!ENTITY e1 \"{}\">]><r>{}</r>", "a".repeat(10_000), "&e0;".repeat(255), "&e1;".repeat(40)), Err("line 1, column 11096: entities expand to more than 1000000 bytes")),
            ("xml fifty thousand chained entities", Format::Xml, (0..50_000).map(|i| format!("<!ENTITY e{i} \"&e{};\">", i + 1)).fold("<!DOCTYPE r [".to_string(), |text, d| text + &d) + "<!ENTITY e50000 \"x\">]><r>&e0;</r>", Err("line 1, column 203: a possible entity reference loop is detected")),
            ("xml eight chained entities", Format::Xml, (0..8).map(|i| format!("<!ENTITY e{i} \"&e{};\">", i + 1)).fold("<!DOCTYPE r [".to_string(), |text, d| text + &d) + "<!ENTITY e8 \"x\">]><r>&e0;</r>", Ok((1, 1))),
            ("xml non-ascii text after an entity declaration", Format::Xml, "<!DOCTYPE r [<!ENTITY e \"x\">]><r>café</r>".to_string(), Ok((1, 1))),
            ("xml non-ascii attribute after an entity declaration", Format::Xml, "<!DOCTYPE r [<!ENTITY e \"x\">]><r a=\"é\">&e;</r>".to_string(), Ok((2, 2))),
            ("xml attlist with a quote before the entity", Format::Xml, format!("<!DOCTYPE r [\n<!ATTLIST r a CDATA '>\n<!ENTITY e \"{}\">\n<!-- ' -->\n]>\n<r>{}</r>", "A".repeat(100_000), "&e;".repeat(1000)), Err("line 6, column 34: entities expand to more than 1000000 bytes")),
            ("xml element declaration holding a subset end", Format::Xml, format!("<!DOCTYPE r [\n<!ELEMENT r ANY]>\n<!ENTITY e \"{}\">\n]>\n<r>{}</r>", "A".repeat(100_000), "&e;".repeat(1000)), Err("line 5, column 34: entities expand to more than 1000000 bytes")),
            ("xml entity name with an ogham space mark", Format::Xml, format!("<!DOCTYPE r [\n<!ENTITY e{0}x \"{1}\">\n]>\n<r>{2}</r>", '\u{1680}', "A".repeat(100_000), format!("&e{}x;", '\u{1680}').repeat(1000)), Err("line 4, column 54: entities expand to more than 1000000 bytes")),
            ("xml comment that opens with a dash", Format::Xml, format!("<!-->--> <!DOCTYPE r [<!ENTITY a \"{}\"><!ENTITY b \"{}\">]><r>{}</r>", "x".repeat(4000), "&a;".repeat(250), "&b;".repeat(40)), Err("line 1, column 4809: entities expand to more than 1000000 bytes")),
            ("xml comment that opens with a dash inside the subset", Format::Xml, format!("<!DOCTYPE r [<!-->]>--> <!ENTITY d \"{}x{}\">]><r>&d;</r>", "<a>".repeat(20_000), "</a>".repeat(20_000)), Err("line 1, column 25: elements nested deeper than 200 levels")),
            ("xml entity name with an ogham space mark and a deep value", Format::Xml, format!("<!DOCTYPE r [<!ENTITY d{0}x \"{1}x{2}\">]><r>&d{0}x;</r>", '\u{1680}', "<a>".repeat(20_000), "</a>".repeat(20_000)), Err("line 1, column 14: elements nested deeper than 200 levels")),
            ("xml small entity used a thousand times", Format::Xml, format!("<!DOCTYPE r [<!ENTITY e \"hello\">]><r>{}</r>", "&e;".repeat(1000)), Ok((1, 1))),
            ("xml big and plain", Format::Xml, format!("<r>\n{}</r>\n", "  <i>v</i>\n".repeat(340_000)), Ok((340_001, 2))),
            ("xml 1.2 million empty elements", Format::Xml, format!("<r>{}</r>", "<i/>".repeat(1_200_000)), Ok((1_200_001, 2))),
            ("yaml big anchor without alias", Format::Yaml, (0..100_000).fold("big: &big\n".to_string(), |text, i| text + &format!("  k{i}: value number {i}\n")), Ok((100_002, 3))),
            ("yaml anchored 2 MB scalar", Format::Yaml, format!("a: &s {}\n", "y".repeat(2_000_000)), Ok((2, 2))),
            ("xml entity 100 in 50", Format::Xml, format!("<!DOCTYPE r [<!ENTITY e \"{}x{}\">]>{}&e;{}", "<a>".repeat(100), "</a>".repeat(100), "<r>".repeat(50), "</r>".repeat(50)), Ok((150, 150))),
        ];
        for (name, format, text, expected) in cases {
            let outcome = std::thread::Builder::new()
                .stack_size(1024 * 1024)
                .spawn(move || Tree::parse(format, &text).map(|tree| (tree.len(), tree.levels())).map_err(|e| e.to_string()))
                .unwrap()
                .join()
                .unwrap_or_else(|_| panic!("{name} panicked"));
            assert_eq!(outcome, expected.map_err(String::from), "{name}");
        }
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
