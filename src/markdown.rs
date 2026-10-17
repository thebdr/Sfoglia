//! Markdown as C-011 reads it: the outline of blocks the status row, the toggle and the tests use,
//! the targets of links and the sources of images. The rendering itself is the kit's rich text,
//! which parses the same source with the same crate, block for block.

use std::ops::Range;
use std::path::{Path, PathBuf};

use markdown::mdast::Node;
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
        let wanted = fragment.trim_start_matches('#');
        let slug = slugify(wanted);
        self.headings.iter().find(|heading| heading.slug == wanted || heading.slug == slug)
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

/// The outline of `source`.
pub fn outline(source: &str) -> Outline {
    let mut outline = Outline::default();
    let Ok(root) = to_mdast(source, &parse_options()) else { return outline };
    let Some(children) = root.children() else { return outline };
    let mut slugs: Vec<String> = Vec::new();
    for child in children {
        walk(child, 0, source, &mut outline, &mut slugs);
    }
    outline
}

fn walk(node: &Node, depth: u32, source: &str, outline: &mut Outline, slugs: &mut Vec<String>) {
    let span = span_of(node, source);
    let line = line_of(source, span.start);
    let push = |outline: &mut Outline, kind: Kind, text: String| {
        outline.blocks.push(Block { kind, depth, text, span: span.clone(), line });
    };
    match node {
        Node::Heading(heading) => {
            let text = inline_text(&heading.children);
            let slug = unique_slug(&text, slugs);
            outline.headings.push(Heading { level: heading.depth, text: text.clone(), slug, span: span.clone(), line });
            push(outline, Kind::Heading(heading.depth), text);
        }
        Node::Paragraph(paragraph) => push(outline, Kind::Paragraph, inline_text(&paragraph.children)),
        Node::List(list) => {
            push(outline, Kind::List, if list.ordered { "ordered".to_string() } else { "unordered".to_string() });
            for item in &list.children {
                walk(item, depth + 1, source, outline, slugs);
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
                walk(child, depth + 1, source, outline, slugs);
            }
        }
        Node::Blockquote(quote) => {
            push(outline, Kind::Quote, String::new());
            for child in &quote.children {
                walk(child, depth + 1, source, outline, slugs);
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
                walk(child, depth + 1, source, outline, slugs);
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
    let mut text = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => text.push_str(&t.value),
            Node::InlineCode(code) => text.push_str(&code.value),
            Node::InlineMath(math) => text.push_str(&math.value),
            Node::Image(image) => text.push_str(&image.alt),
            Node::ImageReference(image) => text.push_str(&image.alt),
            Node::Break(_) => text.push(' '),
            Node::Html(html) => text.push_str(&html.value),
            Node::FootnoteReference(note) => {
                text.push('[');
                text.push_str(&note.identifier);
                text.push(']');
            }
            other => {
                if let Some(children) = other.children() {
                    text.push_str(&inline_text(children));
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

fn unique_slug(text: &str, slugs: &mut Vec<String>) -> String {
    let base = slugify(text);
    let mut slug = base.clone();
    let mut n = 0;
    while slugs.contains(&slug) {
        n += 1;
        slug = format!("{base}-{n}");
    }
    slugs.push(slug.clone());
    slug
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
        return if fragment.is_empty() { LinkTarget::None } else { LinkTarget::Heading(fragment.to_string()) };
    }
    if let Some(rest) = scheme_rest(href) {
        // `file:` addresses are files like any other; everything else goes outside.
        if href[..href.len() - rest.len() - 1].eq_ignore_ascii_case("file") {
            let path = percent_decode(rest.trim_start_matches("//").trim_start_matches('/'));
            let (path, fragment) = split_fragment(&path);
            return LinkTarget::File { path: PathBuf::from(path), fragment };
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
        Some((path, fragment)) => (path.to_string(), (!fragment.is_empty()).then(|| fragment.to_string())),
        None => (href.to_string(), None),
    }
}

/// `%20` and friends decoded, as UTF-8; a malformed sequence stays as written.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%' && ix + 2 < bytes.len() + 0 && ix + 2 <= bytes.len() - 1 {
            if let Ok(value) = u8::from_str_radix(&text[ix + 1..ix + 3], 16) {
                out.push(value);
                ix += 3;
                continue;
            }
        }
        out.push(bytes[ix]);
        ix += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
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
    if let Some(rest) = src.strip_prefix("data:") {
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
            ImageSrc::File(PathBuf::from(percent_decode(rest.trim_start_matches("//").trim_start_matches('/'))))
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
    let bytes = if base64 { decode_base64(data)? } else { percent_decode(data).into_bytes() };
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
            (0, "footnote", "1"),
            (1, "paragraph", "A footnote."),
            (0, "definition", "readme"),
        ];
        let expected: Vec<(u32, String, String)> = expected.into_iter().map(|(d, k, t)| (d, k.to_string(), t.to_string())).collect();
        assert_eq!(kinds(&outline), expected);
        assert_eq!(
            outline.headings.iter().map(|h| (h.level, h.slug.as_str(), h.line)).collect::<Vec<_>>(),
            vec![(1, "sfoglia-guide", 1), (2, "install", 5), (2, "features", 13), (3, "formats", 22), (2, "code", 30), (2, "links-and-images", 42)]
        );
        assert_eq!(outline.summary(), "Markdown, 18 blocks, 6 headings");
        assert_eq!(outline.top_block_at_line(1), Some(0));
        assert_eq!(outline.top_block_at_line(9), Some(3), "a line inside the nested list belongs to the ordered list");
        assert_eq!(outline.top_block_at_line(1000), Some(17));
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
    fn links_are_sorted_into_browser_file_and_heading_targets() {
        let folder = Path::new(if cfg!(windows) { r"C:\docs\book" } else { "/docs/book" });
        let file = |p: &str, fragment: Option<&str>| LinkTarget::File { path: PathBuf::from(p), fragment: fragment.map(str::to_string) };
        assert_eq!(link_target(folder, "https://example.com/a?b=1#c"), LinkTarget::Outside("https://example.com/a?b=1#c".into()));
        assert_eq!(link_target(folder, "mailto:someone@example.com"), LinkTarget::Outside("mailto:someone@example.com".into()));
        assert_eq!(link_target(folder, "#install"), LinkTarget::Heading("install".into()));
        assert_eq!(link_target(folder, "#"), LinkTarget::None);
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
            assert_eq!(link_target(folder, "file:///z/w.md#s"), file("z/w.md", Some("s")));
        }
        assert_eq!(slugify("Links and images"), "links-and-images");
        assert_eq!(slugify("  C++ & Rust: 2 things! "), "c--rust-2-things");
        assert_eq!(slugify("Ünïcödé Köpfe"), "ünïcödé-köpfe");
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
        // A 1x1 red PNG as a data URL decodes to its 70 bytes, which start with the PNG signature.
        let data = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        match image_source(folder, data) {
            ImageSrc::Data { mime, bytes } => {
                assert_eq!(mime, "image/png");
                assert_eq!(bytes.len(), 70);
                assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(image_source(folder, "data:image/png;base64,!!!"), ImageSrc::Invalid);
        assert_eq!(image_source(folder, "data:,hi%20there"), ImageSrc::Data { mime: "text/plain".into(), bytes: b"hi there".to_vec() });
        assert_eq!(decode_base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGV sbG8=\n"), Some(b"hello".to_vec()));
    }
}
