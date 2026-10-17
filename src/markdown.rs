//! Markdown as C-011 reads it: the outline of blocks the status row, the toggle and the tests use,
//! the targets of links and the sources of images. The rendering itself is the kit's rich text,
//! which parses the same source with the same crate, block for block.

use std::ops::Range;
use std::path::{Path, PathBuf};

use markdown::mdast::{Node, ReferenceKind};
use markdown::{Constructs, ParseOptions, to_mdast};

/// What a block is, as the outline names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Heading(u8),
    Paragraph,
    List,
    ListItem,
    Quote,
    Code,
    Table,
    Rule,
    Html,
    Footnote,
    Definition,
    Math,
    Other,
}

impl Kind {
    pub fn label(self) -> String {
        match self {
            Kind::Heading(level) => format!("heading {level}"),
            Kind::Paragraph => "paragraph".to_string(),
            Kind::List => "list".to_string(),
            Kind::ListItem => "item".to_string(),
            Kind::Quote => "quote".to_string(),
            Kind::Code => "code".to_string(),
            Kind::Table => "table".to_string(),
            Kind::Rule => "rule".to_string(),
            Kind::Html => "html".to_string(),
            Kind::Footnote => "footnote".to_string(),
            Kind::Definition => "definition".to_string(),
            Kind::Math => "math".to_string(),
            Kind::Other => "other".to_string(),
        }
    }
}

/// One block of the outline: its kind, its nesting depth (0 at the root), its text on one line, its
/// source byte range and the 1-based line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub depth: u32,
    pub text: String,
    pub span: Range<usize>,
    pub line: usize,
}

/// A heading, with the slug a `#fragment` link names it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub slug: String,
    pub span: Range<usize>,
    pub line: usize,
}

/// The blocks of a document in reading order, nested ones after their container with a deeper depth.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    pub blocks: Vec<Block>,
    pub headings: Vec<Heading>,
}

impl Outline {
    /// The root's own blocks, which are the kit's rendered blocks one for one.
    pub fn top_level(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter().filter(|block| block.depth == 0)
    }

    /// The index among the root's blocks of the one holding `line` (1-based), or of the last one
    /// starting before it, when any starts at or before it.
    pub fn top_block_at_line(&self, line: usize) -> Option<usize> {
        let mut found = None;
        for (ix, block) in self.top_level().enumerate() {
            if block.line <= line {
                found = Some(ix);
            } else {
                break;
            }
        }
        found
    }

    /// The root's `ix`th block.
    pub fn top_block(&self, ix: usize) -> Option<&Block> {
        self.top_level().nth(ix)
    }

    /// The heading a fragment names, by slug, the fragment taken as written or slugified.
    pub fn heading_for_fragment(&self, fragment: &str) -> Option<&Heading> {
        heading_for_fragment(&self.headings, fragment)
    }

    /// `Markdown, N blocks, H headings`.
    pub fn summary(&self) -> String {
        let blocks = self.top_level().count();
        let headings = self.headings.len();
        format!(
            "Markdown, {blocks} {}, {headings} {}",
            if blocks == 1 { "block" } else { "blocks" },
            if headings == 1 { "heading" } else { "headings" }
        )
    }
}

/// The parse options the kit uses too: GitHub flavoured, with math.
pub fn parse_options() -> ParseOptions {
    ParseOptions {
        constructs: Constructs { math_flow: true, math_text: true, ..Constructs::gfm() },
        ..ParseOptions::gfm()
    }
}

fn walk(node: &Node, depth: u32, source: &str, lines: &LineIndex, outline: &mut Outline, slugs: &mut Slugs) {
    let span = span_of(node, source);
    let line = lines.line_of(span.start);
    let push = |outline: &mut Outline, kind: Kind, text: String| {
        outline.blocks.push(Block { kind, depth, text, span: span.clone(), line });
    };
    match node {
        Node::Heading(heading) => {
            let text = heading_text(&heading.children);
            let slug = slugs.unique(&text);
            outline.headings.push(Heading { level: heading.depth, text: text.clone(), slug, span: span.clone(), line });
            push(outline, Kind::Heading(heading.depth), text);
        }
        Node::Paragraph(paragraph) => push(outline, Kind::Paragraph, inline_text(&paragraph.children)),
        Node::List(list) => {
            push(outline, Kind::List, if list.ordered { "ordered".to_string() } else { "unordered".to_string() });
            for item in &list.children {
                walk(item, depth + 1, source, lines, outline, slugs);
            }
        }
        Node::ListItem(item) => {
            // The item's first paragraph is its text; anything else it holds is a block of its own.
            let mut rest = item.children.iter();
            let text = match rest.next() {
                Some(Node::Paragraph(paragraph)) => inline_text(&paragraph.children),
                Some(other) => {
                    rest = item.children.iter();
                    let _ = other;
                    String::new()
                }
                None => String::new(),
            };
            let text = match item.checked {
                Some(true) => format!("[x] {text}"),
                Some(false) => format!("[ ] {text}"),
                None => text,
            };
            push(outline, Kind::ListItem, text);
            for child in rest {
                walk(child, depth + 1, source, lines, outline, slugs);
            }
        }
        Node::Blockquote(quote) => {
            push(outline, Kind::Quote, String::new());
            for child in &quote.children {
                walk(child, depth + 1, source, lines, outline, slugs);
            }
        }
        Node::Code(code) => push(outline, Kind::Code, code.lang.clone().unwrap_or_default()),
        Node::Table(table) => {
            // The header row's cells, and how many rows follow.
            let header = table.children.first().and_then(|row| row.children()).map(|cells| {
                cells.iter().map(|cell| cell.children().map(|c| inline_text(c)).unwrap_or_default()).collect::<Vec<_>>().join(" | ")
            });
            let rows = table.children.len().saturating_sub(1);
            push(
                outline,
                Kind::Table,
                format!("{} ({rows} {})", header.unwrap_or_default(), if rows == 1 { "row" } else { "rows" }),
            );
        }
        Node::ThematicBreak(_) => push(outline, Kind::Rule, String::new()),
        Node::Html(html) => push(outline, Kind::Html, one_line(&html.value)),
        Node::FootnoteDefinition(note) => {
            push(outline, Kind::Footnote, note.identifier.clone());
            for child in &note.children {
                walk(child, depth + 1, source, lines, outline, slugs);
            }
        }
        Node::Definition(definition) => push(outline, Kind::Definition, definition.identifier.clone()),
        Node::Math(math) => push(outline, Kind::Math, one_line(&math.value)),
        Node::Yaml(_) | Node::Toml(_) => push(outline, Kind::Other, "front matter".to_string()),
        _ => push(outline, Kind::Other, String::new()),
    }
}

/// The text of inline nodes on one line: emphasis and links keep their words, code its value, an
/// image its alternative text, a break a space.
fn inline_text(nodes: &[Node]) -> String {
    inline_text_with(nodes, true)
}

/// A heading's text as GitHub anchors it: an image is no text (`# ![logo](l.png) Project` slugifies to
/// `project`), inline HTML neither.
fn heading_text(nodes: &[Node]) -> String {
    inline_text_with(nodes, false)
}

fn inline_text_with(nodes: &[Node], with_alt: bool) -> String {
    let mut text = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => text.push_str(&t.value),
            Node::InlineCode(code) => text.push_str(&code.value),
            Node::InlineMath(math) => text.push_str(&math.value),
            Node::Image(image) if with_alt => text.push_str(&image.alt),
            Node::ImageReference(image) if with_alt => text.push_str(&image.alt),
            Node::Image(_) | Node::ImageReference(_) => {}
            Node::Break(_) => text.push(' '),
            Node::FootnoteReference(note) => {
                text.push('[');
                text.push_str(&note.identifier);
                text.push(']');
            }
            other => {
                if let Some(children) = other.children() {
                    text.push_str(&inline_text_with(children, with_alt));
                }
            }
        }
    }
    one_line(&text)
}

fn one_line(text: &str) -> String {
    text.split(['\n', '\r']).map(str::trim).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ")
}

fn span_of(node: &Node, source: &str) -> Range<usize> {
    match node.position() {
        Some(position) => position.start.offset.min(source.len())..position.end.offset.min(source.len()),
        None => 0..0,
    }
}

/// The 1-based line `offset` is on.
pub fn line_of(source: &str, offset: usize) -> usize {
    source.as_bytes()[..offset.min(source.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

/// The byte offset where `line` (1-based) starts, or the end of the text for a line past it.
pub fn offset_of_line(source: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut seen = 1;
    for (ix, b) in source.bytes().enumerate() {
        if b == b'\n' {
            seen += 1;
            if seen == line {
                return ix + 1;
            }
        }
    }
    source.len()
}

/// A heading's slug as GitHub makes it: lower case, letters, digits, hyphens, underscores and spaces
/// kept, spaces turned to hyphens.
pub fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            for lower in ch.to_lowercase() {
                slug.push(lower);
            }
        } else if ch == ' ' {
            slug.push('-');
        }
    }
    slug
}

/// The slugs given so far, so a repeated heading gets `-1`, `-2` and so on as on GitHub, each in about
/// constant time: a changelog repeats `Added` and `Fixed` thousands of times.
#[derive(Default)]
struct Slugs {
    used: std::collections::HashSet<String>,
    next: std::collections::HashMap<String, usize>,
}

impl Slugs {
    fn unique(&mut self, text: &str) -> String {
        let base = slugify(text);
        let mut n = self.next.get(&base).copied().unwrap_or(0);
        let mut slug = if n == 0 { base.clone() } else { format!("{base}-{n}") };
        while !self.used.insert(slug.clone()) {
            n += 1;
            slug = format!("{base}-{n}");
        }
        self.next.insert(base, n + 1);
        slug
    }
}

/// The definitions of a document and every image or link written with a reference to one: the
/// renderer resolves neither, so both are rewritten inline. A reference inside a link's text is left
/// to the next round, once the link itself is inline.
fn collect_images(
    node: &Node,
    source: &str,
    definitions: &mut Vec<(String, String)>,
    references: &mut Vec<(Range<usize>, String, ReferenceKind, bool)>,
) {
    match node {
        Node::Definition(definition) => definitions.push((definition.identifier.clone(), definition.url.clone())),
        Node::ImageReference(image) => {
            references.push((span_of(node, source), image.identifier.clone(), image.reference_kind, true))
        }
        Node::LinkReference(link) => {
            references.push((span_of(node, source), link.identifier.clone(), link.reference_kind, false));
            return;
        }
        _ => {}
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_images(child, source, definitions, references);
        }
    }
}

/// The path a `file:` address names, `rest` being what follows `file:`: `///C:/x`, `///x`, `/C:/x` and
/// `//localhost/x` are local paths, `//host/share/x` stays a network path.
fn file_url_path(rest: &str) -> PathBuf {
    let decoded = percent_decode(rest);
    let path = match decoded.strip_prefix("//") {
        Some(after) => {
            let (host, path) = match after.split_once('/') {
                Some((host, path)) => (host, format!("/{path}")),
                None => (after, String::new()),
            };
            if host.is_empty() || host.eq_ignore_ascii_case("localhost") { local_path(&path) } else { format!("//{after}") }
        }
        None => local_path(&decoded),
    };
    PathBuf::from(path)
}

/// `/C:/x` is the drive path `C:/x`; any other path stays as it is.
fn local_path(path: &str) -> String {
    match path.strip_prefix('/') {
        Some(rest) if rest.len() >= 2 && rest.as_bytes()[0].is_ascii_alphabetic() && rest.as_bytes()[1] == b':' => rest.to_string(),
        _ => path.to_string(),
    }
}

/// The byte offsets where lines start, so a block's line is a binary search and not a rescan of the
/// text from the top for every block.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(source.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(ix, _)| ix + 1));
        Self { starts }
    }

    /// The 1-based line `offset` is on.
    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset).max(1)
    }
}

/// The outline of `source`.
pub fn outline(source: &str) -> Outline {
    match to_mdast(source, &parse_options()) {
        Ok(root) => outline_from(&root, source),
        Err(_) => Outline::default(),
    }
}

fn outline_from(root: &Node, source: &str) -> Outline {
    let mut outline = Outline::default();
    let Some(children) = root.children() else { return outline };
    let lines = LineIndex::new(source);
    let mut slugs = Slugs::default();
    for child in children {
        walk(child, 0, source, &lines, &mut outline, &mut slugs);
    }
    outline
}

/// The source as the renderer gets it, and its outline.
pub struct Prepared {
    pub text: String,
    pub outline: Outline,
}

/// Prepare `source` for the renderer: images and links written with a reference become inline and fenced
/// code names its grammar as the kit knows it, all without moving a line, and the outline is built on the
/// result. One parse when nothing needs rewriting; a rewrite is parsed again, and a reference inside a
/// rewritten link's text gets its turn in the next round.
pub fn prepare(source: &str) -> Prepared {
    let Ok(mut root) = to_mdast(source, &parse_options()) else {
        return Prepared { text: source.to_string(), outline: Outline::default() };
    };
    let mut text = source.to_string();
    for _ in 0..4 {
        let mut edits = image_edits(&root, &text);
        collect_fences(&root, &text, &mut edits);
        if edits.is_empty() {
            break;
        }
        text = apply_edits(&text, edits);
        match to_mdast(&text, &parse_options()) {
            Ok(parsed) => root = parsed,
            Err(_) => return Prepared { outline: Outline::default(), text },
        }
    }
    let outline = outline_from(&root, &text);
    Prepared { text, outline }
}

fn apply_edits(source: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    let mut out = source.to_string();
    edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    for (span, replacement) in edits {
        out.replace_range(span, &replacement);
    }
    out
}

/// The grammar a fenced code block's info string names, as C-005 names grammars: its first word, in
/// any case, taken as a grammar name (`Rust`), a file extension (`py`, `yml`) or a shell alias.
pub fn grammar_for_info(info: &str) -> Option<&'static str> {
    let token = info.trim().split([' ', '\t', ',', '{', ';', ':']).next().unwrap_or("").trim().to_ascii_lowercase();
    if token.is_empty() {
        return None;
    }
    if let Some(name) = crate::text::LANGUAGES.iter().find(|name| **name == token) {
        return Some(name);
    }
    match token.as_str() {
        "shell" | "shell-session" | "console" | "sh" | "zsh" | "bash" | "dockerfile" => return Some("bash"),
        "c++" => return Some("cpp"),
        "c#" => return Some("csharp"),
        "xml" | "svg" | "htm" | "vue" => return Some("html"),
        "mk" | "makefile" => return Some("make"),
        _ => {}
    }
    crate::text::language_for(&format!("f.{token}"))
}

/// The edits that make every fenced code block name its grammar as the kit knows it: the info string
/// is replaced by the grammar name on the fence's own line, so no line moves.
fn collect_fences(node: &Node, source: &str, edits: &mut Vec<(Range<usize>, String)>) {
    if let Node::Code(code) = node
        && let Some(lang) = &code.lang
    {
        let span = span_of(node, source);
        let line_end = source[span.start..].find('\n').map_or(source.len(), |ix| span.start + ix);
        let line = &source[span.start..line_end];
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        let fence = &line[indent..];
        let fence_char = fence.chars().next();
        if matches!(fence_char, Some('`') | Some('~')) {
            let fence_len = fence.chars().take_while(|c| Some(*c) == fence_char).count();
            let info_start = span.start + indent + fence_len;
            let rest = &source[info_start..line_end];
            let leading = rest.len() - rest.trim_start().len();
            if let Some(name) = grammar_for_info(lang)
                && rest.trim() != name
            {
                edits.push((info_start + leading..line_end, name.to_string()));
            }
        }
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_fences(child, source, edits);
        }
    }
}

/// `![alt][id]` images written with a reference, rewritten as `![alt](url)` with the address their
/// definition gives, since the renderer draws inline images only. The alt is copied as written
/// (it is valid between `![` and `]` already), the dropped label's line break stays inside it and
/// titles are left out, so no line moves and the outline still agrees with the editor line for line.
pub fn resolve_image_references(source: &str) -> String {
    let Ok(root) = to_mdast(source, &parse_options()) else { return source.to_string() };
    let edits = image_edits(&root, source);
    if edits.is_empty() { source.to_string() } else { apply_edits(source, edits) }
}

fn image_edits(root: &Node, source: &str) -> Vec<(Range<usize>, String)> {
    let mut definitions: Vec<(String, String)> = Vec::new();
    let mut references: Vec<(Range<usize>, String, ReferenceKind, bool)> = Vec::new();
    collect_images(root, source, &mut definitions, &mut references);
    let mut edits = Vec::new();
    for (span, identifier, kind, image) in references {
        let Some((_, url)) = definitions.iter().find(|(id, _)| *id == identifier) else { continue };
        let text = &source[span.clone()];
        let opener = if image { "![" } else { "[" };
        if !text.starts_with(opener) || text.len() < opener.len() + 1 {
            continue;
        }
        let alt_end = match kind {
            ReferenceKind::Full => text.rfind("][").unwrap_or(text.len() - 1),
            ReferenceKind::Collapsed => text.len() - 3,
            ReferenceKind::Shortcut => text.len() - 1,
        };
        if alt_end < opener.len() {
            continue;
        }
        let alt = &text[opener.len()..alt_end];
        let kept_breaks = "\n".repeat(text[alt_end..].matches('\n').count());
        // A `|` would split a table cell, so it travels percent-encoded, as any address may. The parser has
        // unescaped the definition's address, so a backslash or an ampersand goes back escaped, or the second
        // parse would read another address (`..\` ends up as `..\)` and no link at all).
        // A control character would end the destination or count as a line ending: percent-encoded, as every
        // consumer decodes the address again.
        let url: String = url
            .replace('\\', "\\\\")
            .replace('&', "\\&")
            .replace('|', "%7C")
            .chars()
            .map(|c| if c.is_ascii_control() { format!("%{:02X}", c as u32) } else { c.to_string() })
            .collect();
        let destination = if url.is_empty() || url.contains([' ', '(', ')', '<', '>']) {
            format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
        } else {
            url
        };
        edits.push((span, format!("{opener}{alt}{kept_breaks}]({destination})")));
    }
    edits
}

/// The heading a fragment names among `headings`, by slug, the fragment taken as written or slugified,
/// hyphens at either end set aside (GitHub anchors `# ![logo](l.png) Project` as `-project`).
pub fn heading_for_fragment<'a>(headings: &'a [Heading], fragment: &str) -> Option<&'a Heading> {
    let wanted = fragment.trim_start_matches('#');
    let slug = slugify(wanted);
    let bare = |slug: &str| slug.trim_matches('-').to_string();
    // A fragment that slugifies to nothing names no heading, not even one whose own slug is empty (an
    // image alone): only a non-empty form is ever compared.
    let same = |heading: &str, form: &str| !form.is_empty() && heading == form;
    headings.iter().find(|heading| same(&heading.slug, wanted) || same(&heading.slug, &slug)).or_else(|| {
        let wanted = bare(wanted);
        let slug = bare(&slug);
        headings.iter().find(|heading| {
            let heading = bare(&heading.slug);
            same(&heading, &wanted) || same(&heading, &slug)
        })
    })
}

/// Where a link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// An address with a scheme (`https:`, `mailto:` and the like): opened outside Sfoglia.
    Outside(String),
    /// A file, relative to the document's folder unless absolute, with the fragment after `#` if any.
    File { path: PathBuf, fragment: Option<String> },
    /// A heading of the document itself.
    Heading(String),
    /// Nothing to follow (an empty link).
    None,
}

/// Sort `href` into what it points at, `folder` being the document's own folder.
pub fn link_target(folder: &Path, href: &str) -> LinkTarget {
    let href = href.trim();
    if href.is_empty() {
        return LinkTarget::None;
    }
    if let Some(fragment) = href.strip_prefix('#') {
        return if fragment.is_empty() { LinkTarget::None } else { LinkTarget::Heading(percent_decode(fragment)) };
    }
    if let Some(rest) = scheme_rest(href) {
        // `file:` addresses are files like any other; everything else goes outside.
        if href[..href.len() - rest.len() - 1].eq_ignore_ascii_case("file") {
            let (path, fragment) = split_fragment(rest);
            return LinkTarget::File { path: file_url_path(&path), fragment };
        }
        return LinkTarget::Outside(href.to_string());
    }
    let (path, fragment) = split_fragment(href);
    let path = percent_decode(&path);
    let path = Path::new(&path);
    let path = if path.is_absolute() { path.to_path_buf() } else { normalise(&folder.join(path)) };
    LinkTarget::File { path, fragment }
}

/// The part after the scheme when `href` starts with one (a Windows drive letter is not a scheme).
fn scheme_rest(href: &str) -> Option<&str> {
    let colon = href.find(':')?;
    let scheme = &href[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    if scheme.len() == 1 && href[colon + 1..].starts_with(['\\', '/']) && !href[colon + 1..].starts_with("//") {
        return None;
    }
    Some(&href[colon + 1..])
}

fn split_fragment(href: &str) -> (String, Option<String>) {
    match href.split_once('#') {
        Some((path, fragment)) => (path.to_string(), (!fragment.is_empty()).then(|| percent_decode(fragment))),
        None => (href.to_string(), None),
    }
}

/// `%20` and friends decoded, as UTF-8; a malformed sequence stays as written, and so does a text whose
/// decoded bytes are no UTF-8 (a data URL's binary payload takes `percent_decode_bytes`).
pub fn percent_decode(text: &str) -> String {
    String::from_utf8(percent_decode_bytes(text)).unwrap_or_else(|_| text.to_string())
}

/// `%20` and friends decoded to bytes; a `%` without two hex digits stays as written.
pub fn percent_decode_bytes(text: &str) -> Vec<u8> {
    // Byte by byte: a `%` before a multi-byte character is kept as written, never sliced.
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%' && ix + 2 < bytes.len() && bytes[ix + 1].is_ascii_hexdigit() && bytes[ix + 2].is_ascii_hexdigit() {
            let digits = std::str::from_utf8(&bytes[ix + 1..ix + 3]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(digits, 16) {
                out.push(value);
                ix += 3;
                continue;
            }
        }
        out.push(bytes[ix]);
        ix += 1;
    }
    out
}

/// `a/./b/../c` folded, without touching the disk.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Where an image's bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSrc {
    /// A file, relative to the document's folder unless absolute.
    File(PathBuf),
    /// An `http(s)` address, fetched.
    Remote(String),
    /// A `data:` URL, decoded: the media type and the bytes.
    Data { mime: String, bytes: Vec<u8> },
    /// An address that cannot be read (a malformed data URL, another scheme).
    Invalid,
}

/// Sort `src` into where its bytes come from, `folder` being the document's own folder.
pub fn image_source(folder: &Path, src: &str) -> ImageSrc {
    let src = src.trim();
    if src.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:")) {
        let rest = &src[5..];
        return match data_url(rest) {
            Some((mime, bytes)) => ImageSrc::Data { mime, bytes },
            None => ImageSrc::Invalid,
        };
    }
    if let Some(rest) = scheme_rest(src) {
        let scheme = &src[..src.len() - rest.len() - 1];
        return if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            ImageSrc::Remote(src.to_string())
        } else if scheme.eq_ignore_ascii_case("file") {
            ImageSrc::File(file_url_path(rest.split('#').next().unwrap_or(rest)))
        } else {
            ImageSrc::Invalid
        };
    }
    let path = percent_decode(src.split('#').next().unwrap_or(src));
    let path = Path::new(&path);
    ImageSrc::File(if path.is_absolute() { path.to_path_buf() } else { normalise(&folder.join(path)) })
}

/// `[<mime>][;base64],<data>` decoded.
fn data_url(rest: &str) -> Option<(String, Vec<u8>)> {
    let (header, data) = rest.split_once(',')?;
    let mut parts = header.split(';');
    let mime = parts.next().unwrap_or("").trim();
    let mime = if mime.is_empty() { "text/plain".to_string() } else { mime.to_ascii_lowercase() };
    let base64 = parts.any(|part| part.trim().eq_ignore_ascii_case("base64"));
    // Percent-encoding comes off first, as the data URL rules say: a reference image's address written back
    // carries a control character as `%09`.
    let bytes = if base64 { decode_base64(&percent_decode(data))? } else { percent_decode_bytes(data) };
    Some((mime, bytes))
}

/// Standard and URL-safe base64, padding optional, whitespace ignored.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for ch in text.bytes() {
        let value = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(outline: &Outline) -> Vec<(u32, String, String)> {
        outline.blocks.iter().map(|b| (b.depth, b.kind.label(), b.text.clone())).collect()
    }

    #[test]
    fn the_outline_lists_every_block_with_its_kind_depth_and_text() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/guide.md")).unwrap();
        let outline = super::outline(&source);
        let expected: Vec<(u32, &str, &str)> = vec![
            (0, "heading 1", "Sfoglia guide"),
            (0, "paragraph", "Sfoglia opens anything. This guide shows the rendered Markdown with emphasis, strong, strike and code."),
            (0, "heading 2", "Install"),
            (0, "list", "ordered"),
            (1, "item", "Install Rust"),
            (1, "item", "Clone the repository"),
            (2, "list", "unordered"),
            (3, "item", "with git"),
            (3, "item", "or as a zip"),
            (1, "item", "Run cargo run"),
            (0, "heading 2", "Features"),
            (0, "list", "unordered"),
            (1, "item", "[x] text and hex"),
            (1, "item", "[ ] office documents"),
            (0, "quote", ""),
            (1, "paragraph", "Nothing is true until reality shows it."),
            (0, "rule", ""),
            (0, "heading 3", "Formats"),
            (0, "table", "Format | Read | Write (3 rows)"),
            (0, "heading 2", "Code"),
            (0, "code", "rust"),
            (0, "code", ""),
            (0, "heading 2", "Links and images"),
            (0, "paragraph", "The site, the readme, the install section, a note[1] and the dot."),
            (0, "paragraph", "dot"),
            (0, "paragraph", "dot too"),
            (0, "footnote", "1"),
            (1, "paragraph", "A footnote."),
            (0, "definition", "readme"),
            (0, "definition", "dot"),
        ];
        let expected: Vec<(u32, String, String)> = expected.into_iter().map(|(d, k, t)| (d, k.to_string(), t.to_string())).collect();
        assert_eq!(kinds(&outline), expected);
        assert_eq!(
            outline.headings.iter().map(|h| (h.level, h.slug.as_str(), h.line)).collect::<Vec<_>>(),
            vec![(1, "sfoglia-guide", 1), (2, "install", 5), (2, "features", 13), (3, "formats", 22), (2, "code", 30), (2, "links-and-images", 42)]
        );
        assert_eq!(outline.summary(), "Markdown, 20 blocks, 6 headings");
        assert_eq!(outline.top_block_at_line(1), Some(0));
        assert_eq!(outline.top_block_at_line(9), Some(3), "a line inside the nested list belongs to the ordered list");
        assert_eq!(outline.top_block_at_line(1000), Some(19));
        assert_eq!(outline.heading_for_fragment("install").map(|h| h.line), Some(5));
        assert_eq!(outline.heading_for_fragment("#Links and images").map(|h| h.line), Some(42));
        assert!(outline.heading_for_fragment("missing").is_none());
        // A repeated heading gets a numbered slug, as on GitHub.
        let twice = super::outline("# A\n\n# A\n\n# a b!\n");
        assert_eq!(twice.headings.iter().map(|h| h.slug.as_str()).collect::<Vec<_>>(), vec!["a", "a-1", "a-b"]);
        assert_eq!(super::outline("").summary(), "Markdown, 0 blocks, 0 headings");
        assert_eq!(super::outline("# One\n").summary(), "Markdown, 1 block, 1 heading");
    }


    #[test]
    fn reference_images_are_written_inline_for_the_renderer() {
        fn images(text: &str) -> Vec<String> {
            fn walk(node: &Node, out: &mut Vec<String>) {
                if let Node::Image(image) = node {
                    out.push(format!("{}|{}", image.alt, image.url));
                }
                if let Some(children) = node.children() {
                    for child in children {
                        walk(child, out);
                    }
                }
            }
            let mut out = Vec::new();
            walk(&to_mdast(text, &parse_options()).unwrap(), &mut out);
            out
        }
        let source = "A ![alt][logo] B\n\n![x][]\n\n![y]\n\n![nope][zz] and ![a\\]b][logo]\n\n[logo]: beta.png \"Be\\\"ta\"\n[x]: <a b.png>\n[y]: y.png\n";
        let shown = resolve_image_references(source);
        assert_eq!(
            shown,
            "A ![alt](beta.png) B\n\n![x](<a b.png>)\n\n![y](y.png)\n\n![nope][zz] and ![a\\]b](beta.png)\n\n[logo]: beta.png \"Be\\\"ta\"\n[x]: <a b.png>\n[y]: y.png\n"
        );
        assert_eq!(shown.lines().count(), source.lines().count(), "no line moves");
        assert_eq!(images(&shown), vec!["alt|beta.png", "x|a b.png", "y|y.png", "a]b|beta.png"]);
        // An alt or a label over two lines (the parser drops a label's line ending when it matches a definition, so
        // `lo⏎go` is `logo`), balanced brackets in the alt, a title ending in a backslash: every heading keeps its
        // line and every image comes back as an image.
        let tricky = "![first\nsecond][logo] tail\n\n# Install\n\n![x][lo\ngo] and ![a [b] c][logo] and ![t][slash]\n\n## Next\n\n[logo]: a.png \"one\ntwo\"\n[slash]: c.png \"t\\\\\"\n";
        let shown = resolve_image_references(tricky);
        assert_eq!(shown.lines().count(), tricky.lines().count(), "{shown}");
        assert_eq!(
            outline(&shown).headings.iter().map(|h| h.line).collect::<Vec<_>>(),
            outline(tricky).headings.iter().map(|h| h.line).collect::<Vec<_>>(),
            "{shown}"
        );
        assert_eq!(images(&shown), vec!["first\nsecond|a.png", "x\n|a.png", "a [b] c|a.png", "t|c.png"], "{shown}");
        // The fixture's reference image becomes inline too, and a source without references comes
        // back as it is, byte for byte.
        let plain = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/guide.md")).unwrap();
        let shown = resolve_image_references(&plain);
        assert!(shown.contains("![dot too](dot.png)"), "{shown}");
        assert_eq!(shown.lines().count(), plain.lines().count());
        assert_eq!(resolve_image_references("# Plain\n\ntext\n"), "# Plain\n\ntext\n");
        // A `|` in the address would split a table cell: it travels percent-encoded, and the cell keeps its image.
        let table = "| ![x] | b |\n| --- | --- |\n\n[x]: a|b.png\n";
        let shown = resolve_image_references(table);
        assert_eq!(shown, "| ![x](a%7Cb.png) | b |\n| --- | --- |\n\n[x]: a|b.png\n");
        assert_eq!(images(&shown), vec!["x|a%7Cb.png"]);
        assert_eq!(outline(&shown).blocks[0].text, "x | b (0 rows)");
        // Links written with a reference are rewritten the same way: the renderer resolves no definition, so a
        // click on `[text][id]` would hand over an empty address.
        // (A destination with a space is only a definition between angle brackets.)
        let links = "See [the docs][d], [d][] and [d] here, and [none][zz].\n\n[d]: <docs/a b.md#top>\n";
        let shown = resolve_image_references(links);
        assert_eq!(shown, "See [the docs](<docs/a b.md#top>), [d](<docs/a b.md#top>) and [d](<docs/a b.md#top>) here, and [none][zz].\n\n[d]: <docs/a b.md#top>\n");
        assert_eq!(shown.lines().count(), links.lines().count());
        // A reference link holding a reference image: the link is rewritten first and the image inside it in the
        // next round of the preparation.
        let nested = "[![badge][b]][site]\n\n[b]: b.png\n[site]: https://example.com\n";
        assert_eq!(resolve_image_references(nested), "[![badge][b]](https://example.com)\n\n[b]: b.png\n[site]: https://example.com\n");
        assert_eq!(prepare(nested).text, "[![badge](b.png)](https://example.com)\n\n[b]: b.png\n[site]: https://example.com\n");
        // A backslash or an ampersand in an address, which the parser unescapes, is written back escaped, so
        // the second parse reads the definition's own address: `..\` links a folder, `a\_b.md` and `a&amp;b.md`
        // stay those files, and nothing grows round after round.
        fn link_urls(text: &str) -> Vec<String> {
            fn walk(node: &Node, out: &mut Vec<String>) {
                if let Node::Link(link) = node {
                    out.push(link.url.clone());
                }
                if let Some(children) = node.children() {
                    for child in children {
                        walk(child, out);
                    }
                }
            }
            let mut out = Vec::new();
            walk(&to_mdast(text, &parse_options()).unwrap(), &mut out);
            out
        }
        let escaped = "Open [root] or [root][] now, [t][d] and [u][e].\n\n[root]: ..\\\n[d]: a\\\\_b.md\n[e]: a&amp;amp;b.md\n";
        assert!(link_urls(escaped).is_empty(), "before the rewrite they are references, which the renderer cannot follow, not links");
        let prepared = prepare(escaped).text;
        assert_eq!(
            prepared,
            "Open [root](..\\\\) or [root](..\\\\) now, [t](a\\\\_b.md) and [u](a\\&amp;b.md).\n\n[root]: ..\\\n[d]: a\\\\_b.md\n[e]: a&amp;amp;b.md\n"
        );
        assert_eq!(link_urls(&prepared), vec!["..\\", "..\\", "a\\_b.md", "a&amp;b.md"], "the same addresses after the rewrite");
        assert_eq!(prepared.lines().count(), escaped.lines().count());
        // A control character in an address (a tab, a carriage return, a form feed, written as a character
        // reference) ends an unbracketed destination and counts as a line ending inside brackets: it travels
        // percent-encoded, which every consumer decodes again.
        let control = "[t][d] ![i][e]\n\n[d]: a&#9;b.png\n[e]: a&#13;b&#x0C;.png\n";
        let prepared = prepare(control).text;
        assert_eq!(link_urls(&prepared), vec!["a%09b.png"]);
        assert_eq!(images(&prepared), vec!["i|a%0Db%0C.png"]);
        assert_eq!(percent_decode("a%09b.png"), "a\tb.png");
        assert_eq!(percent_decode("a%0Db%0C.png"), "a\rb\x0C.png");
        assert_eq!(prepared.lines().count(), control.lines().count());
    }

    #[test]
    fn repeated_headings_get_numbered_slugs_in_linear_time() {
        // A changelog repeats its headings thousands of times: each gets its number without a search through
        // every slug given before, so the outline of such a file takes about as long as its parse.
        let mut source = String::from("# Changelog\n\n");
        for release in 0..2000 {
            source.push_str(&format!("## {release}.0\n\n### Added\n\n- a\n\n### Fixed\n\n- b\n\n"));
        }
        let started = std::time::Instant::now();
        let outline = outline(&source);
        let took = started.elapsed();
        assert_eq!(outline.headings.len(), 1 + 3 * 2000);
        assert_eq!(outline.headings[2].slug, "added");
        assert_eq!(outline.headings[3].slug, "fixed");
        assert_eq!(outline.headings[5].slug, "added-1");
        assert_eq!(outline.headings[6].slug, "fixed-1");
        assert_eq!(outline.headings[1 + 3 * 1999 + 1].slug, "added-1999");
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
        // An explicit `a-1` and a repeated `a` stay apart, as on GitHub.
        let apart = super::outline("# a\n\n# a-1\n\n# a\n\n# a\n");
        assert_eq!(apart.headings.iter().map(|h| h.slug.as_str()).collect::<Vec<_>>(), vec!["a", "a-1", "a-2", "a-3"]);
    }

    #[test]
    fn code_fences_name_their_grammar() {
        for (info, grammar) in [
            ("rust", Some("rust")),
            ("Rust", Some("rust")),
            ("rust,ignore", Some("rust")),
            ("rust {.class}", Some("rust")),
            ("JSON", Some("json")),
            ("Python", Some("python")),
            ("py", Some("python")),
            ("rs", Some("rust")),
            ("yml", Some("yaml")),
            ("shell", Some("bash")),
            ("zsh", Some("bash")),
            ("console", Some("bash")),
            ("sh", Some("bash")),
            ("c++", Some("cpp")),
            ("c#", Some("csharp")),
            ("xml", Some("html")),
            ("js", Some("javascript")),
            ("", None),
            ("   ", None),
            ("zzz", None),
            ("text", None),
        ] {
            assert_eq!(grammar_for_info(info), grammar, "{info:?}");
        }
        // The fence line names the grammar as the kit knows it (the space after the fence kept as
        // written); an unknown word and an indented block stay as they are; no line moves; the outline
        // says the grammar.
        let source = "# Code\n\n```rust,ignore\nfn x() {}\n```\n\n~~~ Python {.x}\ncode\n~~~\n\n- item\n\n  ```Shell\n  ls\n  ```\n\n```zzz\nq\n```\n\n    indented\n";
        let prepared = prepare(source);
        assert_eq!(
            prepared.text,
            "# Code\n\n```rust\nfn x() {}\n```\n\n~~~ python\ncode\n~~~\n\n- item\n\n  ```bash\n  ls\n  ```\n\n```zzz\nq\n```\n\n    indented\n"
        );
        assert_eq!(prepared.text.lines().count(), source.lines().count());
        let codes: Vec<(u32, String)> =
            prepared.outline.blocks.iter().filter(|b| b.kind == Kind::Code).map(|b| (b.depth, b.text.clone())).collect();
        assert_eq!(codes, vec![(0, "rust".into()), (0, "python".into()), (2, "bash".into()), (0, "zzz".into()), (0, String::new())]);
        assert_eq!(prepared.outline.headings[0].line, 1);
        // Nothing to rewrite: the text comes back as it is.
        let plain = "# A\n\n```rust\nx\n```\n";
        assert_eq!(prepare(plain).text, plain);
        assert_eq!(prepare(plain).outline, outline(plain));
    }

    #[test]
    fn links_are_sorted_into_browser_file_and_heading_targets() {
        let folder = Path::new(if cfg!(windows) { r"C:\docs\book" } else { "/docs/book" });
        let file = |p: &str, fragment: Option<&str>| LinkTarget::File { path: PathBuf::from(p), fragment: fragment.map(str::to_string) };
        assert_eq!(link_target(folder, "https://example.com/a?b=1#c"), LinkTarget::Outside("https://example.com/a?b=1#c".into()));
        assert_eq!(link_target(folder, "mailto:someone@example.com"), LinkTarget::Outside("mailto:someone@example.com".into()));
        assert_eq!(link_target(folder, "#install"), LinkTarget::Heading("install".into()));
        assert_eq!(link_target(folder, "#"), LinkTarget::None);
        // A percent-encoded fragment is decoded, in a file link too; inline HTML is no part of a heading's slug.
        assert_eq!(link_target(folder, "#%C3%BCber-uns"), LinkTarget::Heading("über-uns".into()));
        assert_eq!(link_target(folder, "a.md#%C3%BC").clone(), link_target(folder, "a.md#ü"));
        let anchored = super::outline("## <a name=\"install\"></a> Install\n\n# <img src=\"l.png\" width=32> Project\n\nSee <b>bold</b> text.\n");
        assert_eq!(anchored.headings.iter().map(|h| (h.slug.as_str(), h.text.as_str())).collect::<Vec<_>>(), vec![("install", "Install"), ("project", "Project")]);
        assert_eq!(anchored.heading_for_fragment("%C3%BCber").is_none(), true);
        assert_eq!(anchored.blocks[2].text, "See bold text.");
        // An image in a heading is no part of its text or slug, and GitHub's own `#-project` form finds it too.
        let pictured = prepare("# ![logo](l.png) Project\n\n## ![badge][b] Install\n\n[b]: b.png\n").outline;
        assert_eq!(pictured.headings.iter().map(|h| (h.slug.as_str(), h.text.as_str())).collect::<Vec<_>>(), vec![("project", "Project"), ("install", "Install")]);
        assert_eq!(pictured.heading_for_fragment("-project").map(|h| h.line), Some(1));
        assert_eq!(pictured.heading_for_fragment("#install").map(|h| h.line), Some(3));
        assert_eq!(pictured.blocks[0].text, "Project");
        // A fragment that slugifies to nothing names no heading, not even the image-only one whose slug is empty.
        let bare = super::outline("# ![x](a.png)\n\n# Real\n");
        assert_eq!(bare.headings.iter().map(|h| h.slug.as_str()).collect::<Vec<_>>(), vec!["", "real"]);
        for fragment in ["!!!", "?", "-", "--", ""] {
            assert!(bare.heading_for_fragment(fragment).is_none(), "{fragment:?}");
        }
        assert_eq!(bare.heading_for_fragment("real").map(|h| h.line), Some(3));
        assert_eq!(link_target(folder, ""), LinkTarget::None);
        if cfg!(windows) {
            assert_eq!(link_target(folder, "README.md"), file(r"C:\docs\book\README.md", None));
            assert_eq!(link_target(folder, "../notes/a%20b.md#top"), file(r"C:\docs\notes\a b.md", Some("top")));
            assert_eq!(link_target(folder, "./img/../x.md"), file(r"C:\docs\book\x.md", None));
            assert_eq!(link_target(folder, r"D:\other\y.md"), file(r"D:\other\y.md", None));
            assert_eq!(link_target(folder, "file:///C:/z/w.md#s"), file("C:/z/w.md", Some("s")));
        } else {
            assert_eq!(link_target(folder, "README.md"), file("/docs/book/README.md", None));
            assert_eq!(link_target(folder, "../notes/a%20b.md#top"), file("/docs/notes/a b.md", Some("top")));
            assert_eq!(link_target(folder, "./img/../x.md"), file("/docs/book/x.md", None));
            assert_eq!(link_target(folder, "/other/y.md"), file("/other/y.md", None));
            assert_eq!(link_target(folder, "file:///z/w.md#s"), file("/z/w.md", Some("s")));
        }
        // A network share keeps its host on both systems.
        assert_eq!(link_target(folder, "file://server/share/x.md"), file("//server/share/x.md", None));
        // `localhost` is no host, and a single slash (`file:/C:/x`, as Java writes it) names a local path too.
        if cfg!(windows) {
            assert_eq!(link_target(folder, "file://localhost/C:/z/w.md"), file("C:/z/w.md", None));
            assert_eq!(link_target(folder, "file:/C:/z/w.md"), file("C:/z/w.md", None));
        } else {
            assert_eq!(link_target(folder, "file://localhost/z/w.md"), file("/z/w.md", None));
            assert_eq!(link_target(folder, "file:/z/w.md"), file("/z/w.md", None));
        }
        assert_eq!(link_target(folder, "FILE://LOCALHOST/share/x.md"), file("/share/x.md", None));
        assert_eq!(slugify("Links and images"), "links-and-images");
        assert_eq!(slugify("  C++ & Rust: 2 things! "), "c--rust-2-things");
        assert_eq!(slugify("Ünïcödé Köpfe"), "ünïcödé-köpfe");
        // A percent sign before a multi-byte character, or without two hex digits, stays as written.
        assert_eq!(percent_decode("%aé.png"), "%aé.png");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%41%zz%"), "A%zz%");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        let odd = if cfg!(windows) { r"C:\docs\book\%aé.md" } else { "/docs/book/%aé.md" };
        assert_eq!(link_target(folder, "%aé.md"), file(odd, None));
    }

    #[test]
    fn image_sources_resolve_against_the_file() {
        let folder = Path::new(if cfg!(windows) { r"C:\docs\book" } else { "/docs/book" });
        let expected = if cfg!(windows) { r"C:\docs\book\img\dot.png" } else { "/docs/book/img/dot.png" };
        assert_eq!(image_source(folder, "img/dot.png"), ImageSrc::File(PathBuf::from(expected)));
        assert_eq!(image_source(folder, "./img/dot.png"), ImageSrc::File(PathBuf::from(expected)));
        let absolute = if cfg!(windows) { r"D:\pics\a.png" } else { "/pics/a.png" };
        assert_eq!(image_source(folder, absolute), ImageSrc::File(PathBuf::from(absolute)));
        assert_eq!(image_source(folder, "https://example.com/a.png"), ImageSrc::Remote("https://example.com/a.png".into()));
        assert_eq!(image_source(folder, "ftp://example.com/a.png"), ImageSrc::Invalid);
        let from_url = if cfg!(windows) { "file:///C:/pics/a%20b.png" } else { "file:///pics/a%20b.png" };
        let on_disk = if cfg!(windows) { "C:/pics/a b.png" } else { "/pics/a b.png" };
        assert_eq!(image_source(folder, from_url), ImageSrc::File(PathBuf::from(on_disk)));
        assert_eq!(image_source(folder, "file://server/share/x.png"), ImageSrc::File(PathBuf::from("//server/share/x.png")));
        let local = if cfg!(windows) { "file://localhost/C:/pics/a.png" } else { "file://localhost/pics/a.png" };
        let on_disk = if cfg!(windows) { "C:/pics/a.png" } else { "/pics/a.png" };
        assert_eq!(image_source(folder, local), ImageSrc::File(PathBuf::from(on_disk)));
        let single = if cfg!(windows) { "file:/C:/pics/a.png" } else { "file:/pics/a.png" };
        assert_eq!(image_source(folder, single), ImageSrc::File(PathBuf::from(on_disk)));
        // A 1x1 red PNG as a data URL decodes to its 70 bytes, which start with the PNG signature.
        let data = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        match image_source(folder, data) {
            ImageSrc::Data { mime, bytes } => {
                assert_eq!(mime, "image/png");
                assert_eq!(bytes.len(), 70);
                assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
                let decoded = image::load_from_memory(&bytes).expect("a PNG the image crate reads");
                assert_eq!((decoded.width(), decoded.height()), (1, 1), "the exact size");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(image_source(folder, "data:image/png;base64,!!!"), ImageSrc::Invalid);
        let odd = if cfg!(windows) { r"C:\docs\book\%aé.png" } else { "/docs/book/%aé.png" };
        assert_eq!(image_source(folder, "%aé.png"), ImageSrc::File(PathBuf::from(odd)));
        assert_eq!(image_source(folder, "data:,hi%20there"), ImageSrc::Data { mime: "text/plain".into(), bytes: b"hi there".to_vec() });
        assert_eq!(decode_base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGV sbG8=\n"), Some(b"hello".to_vec()));
        // A data URL without base64 may carry any byte percent-encoded (RFC 2397): the bytes come back as
        // bytes, not through a text round trip; and the scheme is matched in any case.
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dot.png")).unwrap();
        let encoded = format!("data:image/png,{}", png.iter().map(|b| format!("%{b:02X}")).collect::<String>());
        match image_source(folder, &encoded) {
            ImageSrc::Data { mime, bytes } => {
                assert_eq!(mime, "image/png");
                assert_eq!(bytes, png, "every byte of the PNG, as encoded");
                let decoded = image::load_from_memory(&bytes).expect("the PNG decodes");
                assert_eq!((decoded.width(), decoded.height()), (1, 1));
            }
            other => panic!("{other:?}"),
        }
        match image_source(folder, "DATA:Image/PNG;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==") {
            ImageSrc::Data { mime, bytes } => assert_eq!((mime.as_str(), bytes.len()), ("image/png", 70)),
            other => panic!("{other:?}"),
        }
        assert_eq!(percent_decode_bytes("%89PNG%0D%0A"), b"\x89PNG\r\n".to_vec());
        // A control character percent-encoded inside the base64 (a reference image's address written back)
        // is decoded first, as the data URL rules say, so the image still decodes.
        let tabbed = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ%09AAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        match image_source(folder, tabbed) {
            ImageSrc::Data { bytes, .. } => assert_eq!(bytes.len(), 70),
            other => panic!("{other:?}"),
        }
        let by_reference = prepare("![j][e]\n\n[e]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ\tAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==>\n").text;
        let url = by_reference.split("](").nth(1).unwrap().split(')').next().unwrap().to_string();
        assert!(url.contains("%09"), "{by_reference}");
        match image_source(folder, &url) {
            ImageSrc::Data { bytes, .. } => assert_eq!(bytes.len(), 70),
            other => panic!("{other:?}"),
        }
    }
}
