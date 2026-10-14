//! Text files: how they are decoded, what their line endings are, and which grammar
//! highlights them (C-005).

use std::path::Path;

/// The encoding a file was decoded with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// The bytes were not valid UTF-8; every byte became the character of that value.
    Latin1,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 with BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Latin1 => "Latin-1",
        }
    }
}

/// The line endings a text uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndings {
    Lf,
    CrLf,
    Cr,
    /// More than one kind occurs.
    Mixed,
    /// The text has no line break at all.
    None,
}

impl LineEndings {
    pub fn label(self) -> &'static str {
        match self {
            LineEndings::Lf => "LF",
            LineEndings::CrLf => "CRLF",
            LineEndings::Cr => "CR",
            LineEndings::Mixed => "mixed",
            LineEndings::None => "none",
        }
    }
}

/// A decoded text file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub encoding: Encoding,
    pub line_endings: LineEndings,
    /// Lines as an editor counts them: one more than the number of line breaks, and
    /// zero for an empty text.
    pub lines: usize,
    /// UTF-16 units that did not decode and became U+FFFD.
    pub replaced: usize,
}

/// Decode the whole file. A UTF-16 byte-order mark decides UTF-16; a UTF-8 mark is
/// stripped; bytes that are not valid UTF-8 are read as Latin-1, one character per byte.
pub fn decode(bytes: &[u8]) -> Decoded {
    let (text, encoding, replaced) = if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let (text, replaced) = decode_utf16(body, u16::from_le_bytes);
        (text, Encoding::Utf16Le, replaced)
    } else if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let (text, replaced) = decode_utf16(body, u16::from_be_bytes);
        (text, Encoding::Utf16Be, replaced)
    } else {
        let (body, bom) = match bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
            Some(rest) => (rest, true),
            None => (bytes, false),
        };
        match std::str::from_utf8(body) {
            Ok(text) => (text.to_string(), if bom { Encoding::Utf8Bom } else { Encoding::Utf8 }, 0),
            Err(_) => (bytes.iter().map(|&b| b as char).collect(), Encoding::Latin1, 0),
        }
    };
    let (line_endings, lines) = line_endings(&text);
    Decoded { text, encoding, line_endings, lines, replaced }
}

fn decode_utf16(body: &[u8], unit: fn([u8; 2]) -> u16) -> (String, usize) {
    let mut replaced = 0;
    let units = body.chunks_exact(2).map(|pair| unit([pair[0], pair[1]]));
    let text: String = char::decode_utf16(units)
        .map(|r| {
            r.unwrap_or_else(|_| {
                replaced += 1;
                char::REPLACEMENT_CHARACTER
            })
        })
        .collect();
    if body.len() % 2 == 1 {
        replaced += 1;
    }
    (text, replaced)
}

/// The kind of line endings in `text`, and the editor's line count: one more than the
/// number of line breaks, zero for an empty text.
pub fn line_endings(text: &str) -> (LineEndings, usize) {
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 2;
                continue;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = [crlf, lf, cr].iter().filter(|&&n| n > 0).count();
    let endings = match (kinds, crlf, lf, cr) {
        (0, ..) => LineEndings::None,
        (1, n, _, _) if n > 0 => LineEndings::CrLf,
        (1, _, n, _) if n > 0 => LineEndings::Lf,
        (1, ..) => LineEndings::Cr,
        _ => LineEndings::Mixed,
    };
    let lines = if text.is_empty() { 0 } else { crlf + lf + cr + 1 };
    (endings, lines)
}

/// The highlighting language for a file name: the grammar name the kit knows, by
/// extension or by a well-known file name; `None` for plain text or an unknown language.
pub fn language_for(name: &str) -> Option<&'static str> {
    let file = Path::new(name).file_name()?.to_str()?;
    match file {
        "Makefile" | "makefile" | "GNUmakefile" => return Some("make"),
        "CMakeLists.txt" => return Some("cmake"),
        "Dockerfile" => return Some("bash"),
        _ => {}
    }
    let ext = file.rsplit_once('.')?.1.to_ascii_lowercase();
    let language = match ext.as_str() {
        "rs" => "rust",
        "json" | "jsonc" | "json5" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "py" | "pyi" => "python",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ino" => "cpp",
        "cs" => "csharp",
        "sh" | "bash" | "zsh" => "bash",
        "cmake" => "cmake",
        "css" | "scss" => "css",
        "diff" | "patch" => "diff",
        "go" => "go",
        "java" => "java",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "html" | "htm" | "xhtml" => "html",
        "lua" => "lua",
        "mk" => "make",
        "php" => "php",
        "proto" => "proto",
        "rb" | "rake" => "ruby",
        "scala" | "sc" => "scala",
        "sql" => "sql",
        "swift" => "swift",
        "zig" => "zig",
        "ex" | "exs" => "elixir",
        "graphql" | "gql" => "graphql",
        "kt" | "kts" => "kotlin",
        "astro" => "astro",
        "svelte" => "svelte",
        "ejs" => "ejs",
        "erb" => "erb",
        _ => return None,
    };
    Some(language)
}

/// Every grammar name `language_for` can return, for the test that checks the kit knows them.
pub const LANGUAGES: &[&str] = &[
    "rust", "json", "toml", "yaml", "markdown", "python", "c", "cpp", "csharp", "bash", "cmake",
    "css", "diff", "go", "java", "javascript", "typescript", "tsx", "html", "lua", "make", "php",
    "proto", "ruby", "scala", "sql", "swift", "zig", "elixir", "graphql", "kotlin", "astro",
    "svelte", "ejs", "erb",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
    }

    #[test]
    fn decoding_names_the_encoding_and_falls_back_to_latin1() {
        let d = decode(&fixture("hello.txt"));
        assert_eq!(d.text, "Sfoglia reads anything.\nSecond line.\n");
        assert_eq!((d.encoding, d.line_endings, d.lines, d.replaced), (Encoding::Utf8, LineEndings::Lf, 3, 0));

        let d = decode(&fixture("utf8-bom.txt"));
        assert_eq!(d.text, "bom then text\n");
        assert_eq!((d.encoding, d.lines), (Encoding::Utf8Bom, 2));

        let d = decode(&fixture("bom-only.txt"));
        assert_eq!(d.text, "");
        assert_eq!((d.encoding, d.line_endings, d.lines), (Encoding::Utf8Bom, LineEndings::None, 0));

        let d = decode(&fixture("empty.txt"));
        assert_eq!((d.text.as_str(), d.encoding, d.line_endings, d.lines), ("", Encoding::Utf8, LineEndings::None, 0));

        let d = decode(&fixture("utf16le.txt"));
        assert_eq!(d.text, "hi\r\n");
        assert_eq!((d.encoding, d.line_endings, d.lines, d.replaced), (Encoding::Utf16Le, LineEndings::CrLf, 2, 0));

        let d = decode(&fixture("utf16be.txt"));
        assert_eq!(d.text, "hi\n");
        assert_eq!((d.encoding, d.line_endings, d.lines), (Encoding::Utf16Be, LineEndings::Lf, 2));

        // Invalid UTF-8: every byte is read as Latin-1, so 0xE8 is è and 0xE0 is à.
        let d = decode(&fixture("latin1.txt"));
        assert_eq!(d.text, "caffè latte à Roma\n");
        assert_eq!((d.encoding, d.lines), (Encoding::Latin1, 2));

        // A UTF-16 unit that cannot decode becomes U+FFFD and is counted; an odd trailing byte too.
        let d = decode(b"\xff\xfe\x3d\xd8h\x00");
        assert_eq!(d.text, "\u{fffd}h");
        assert_eq!((d.encoding, d.replaced), (Encoding::Utf16Le, 1));
        let d = decode(b"\xff\xfeh\x00i");
        assert_eq!((d.text.as_str(), d.replaced), ("h", 1));

        assert_eq!(Encoding::Latin1.label(), "Latin-1");
        assert_eq!(Encoding::Utf8Bom.label(), "UTF-8 with BOM");
    }

    #[test]
    fn line_endings_and_line_counts_are_exact() {
        assert_eq!(line_endings(""), (LineEndings::None, 0));
        assert_eq!(line_endings("a"), (LineEndings::None, 1));
        assert_eq!(line_endings("a\n"), (LineEndings::Lf, 2));
        assert_eq!(line_endings("a\nb"), (LineEndings::Lf, 2));
        assert_eq!(line_endings("a\r\nb\r\n"), (LineEndings::CrLf, 3));
        assert_eq!(line_endings("a\rb\rc"), (LineEndings::Cr, 3));
        assert_eq!(line_endings("one\r\ntwo\nthree\r\n"), (LineEndings::Mixed, 4));
        assert_eq!(line_endings("a\n\r"), (LineEndings::Mixed, 3), "LF then a lone CR");
        assert_eq!(line_endings("\r\n"), (LineEndings::CrLf, 2));
        let d = decode(&fixture("mixed-endings.txt"));
        assert_eq!((d.line_endings, d.lines), (LineEndings::Mixed, 4));
        let d = decode(&fixture("cr-only.txt"));
        assert_eq!((d.text.as_str(), d.line_endings, d.lines), ("alpha\rbeta\rgamma", LineEndings::Cr, 3));
        let d = decode(&fixture("no-final-newline.txt"));
        assert_eq!((d.line_endings, d.lines), (LineEndings::None, 1));
        let big: String = (0..300_000).map(|i| format!("line {i}\n")).collect();
        assert_eq!(line_endings(&big), (LineEndings::Lf, 300_001));
        assert_eq!(LineEndings::Mixed.label(), "mixed");
    }

    #[test]
    fn language_mapping_follows_extension_and_file_name() {
        assert_eq!(language_for("src/main.rs"), Some("rust"));
        assert_eq!(language_for("C:\\x\\data.JSON"), Some("json"));
        assert_eq!(language_for("Makefile"), Some("make"));
        assert_eq!(language_for("CMakeLists.txt"), Some("cmake"));
        assert_eq!(language_for("notes.txt"), None);
        assert_eq!(language_for("README"), None);
        assert_eq!(language_for("archive.tar.gz"), None);
        assert_eq!(language_for("weird.unknownext"), None);
        assert_eq!(language_for("index.tsx"), Some("tsx"));
        assert_eq!(language_for("Dockerfile"), Some("bash"));
        for language in LANGUAGES {
            assert!(!language.is_empty());
        }
    }
}
