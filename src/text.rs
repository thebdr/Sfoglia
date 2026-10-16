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
    /// Not valid UTF-8, and bytes 0x80 to 0x9F occur: those are the Windows-1252
    /// punctuation (smart quotes, dashes, the euro sign) rather than Latin-1 controls.
    Windows1252,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 with BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Latin1 => "Latin-1",
            Encoding::Windows1252 => "Windows-1252",
        }
    }
}

/// Windows-1252 for bytes 0x80 to 0x9F; the five undefined bytes keep their Latin-1 value.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// One byte of a single-byte file as a character: Windows-1252 where it differs from Latin-1.
fn single_byte_char(b: u8) -> char {
    match b {
        0x80..=0x9f => CP1252_HIGH[(b - 0x80) as usize],
        _ => b as char,
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
/// stripped; bytes that are not valid UTF-8 are read one character per byte, as Latin-1,
/// or as Windows-1252 when any byte from 0x80 to 0x9F occurs.
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
            Err(_) => {
                let windows = bytes.iter().any(|b| (0x80..=0x9f).contains(b));
                let text: String = bytes.iter().map(|&b| single_byte_char(b)).collect();
                (text, if windows { Encoding::Windows1252 } else { Encoding::Latin1 }, 0)
            }
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

/// A character that an encoding cannot hold, and the line it is on (C-008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError {
    pub character: char,
    pub line: usize,
}

impl EncodeError {
    pub fn message(&self, encoding: Encoding) -> String {
        format!(
            "{} cannot hold {:?} (U+{:04X}) on line {}; choose another encoding",
            encoding.label(),
            self.character,
            self.character as u32,
            self.line
        )
    }
}

/// `text` with every line break (CRLF, LF or lone CR) written as `ending`. `Mixed` and
/// `None` write LF: a file whose breaks disagreed is written uniformly once edited.
pub fn with_line_endings(text: &str, ending: LineEndings) -> String {
    let ending = match ending {
        LineEndings::CrLf => "\r\n",
        LineEndings::Cr => "\r",
        LineEndings::Lf | LineEndings::Mixed | LineEndings::None => "\n",
    };
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                out.push_str(&text[start..i]);
                out.push_str(ending);
                i += if bytes.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                start = i;
            }
            b'\n' => {
                out.push_str(&text[start..i]);
                out.push_str(ending);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&text[start..]);
    out
}

/// The bytes of `text` in `encoding`, every line break as `ending`: the byte-order mark
/// the encoding carries, then the text. A character the encoding cannot hold is an error
/// naming it and its line.
pub fn encode(text: &str, encoding: Encoding, ending: LineEndings) -> Result<Vec<u8>, EncodeError> {
    let text = with_line_endings(text, ending);
    match encoding {
        Encoding::Utf8 => Ok(text.into_bytes()),
        Encoding::Utf8Bom => {
            let mut out = vec![0xef, 0xbb, 0xbf];
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        Encoding::Utf16Le => {
            let mut out = vec![0xff, 0xfe];
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            Ok(out)
        }
        Encoding::Utf16Be => {
            let mut out = vec![0xfe, 0xff];
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            Ok(out)
        }
        Encoding::Latin1 | Encoding::Windows1252 => {
            let mut out = Vec::with_capacity(text.len());
            let mut line = 1;
            for c in text.chars() {
                let byte = match (encoding, c as u32) {
                    (_, code) if code < 0x80 => Some(code as u8),
                    (Encoding::Latin1, code) if code <= 0xff => Some(code as u8),
                    (Encoding::Windows1252, code) if (0xa0..=0xff).contains(&code) => Some(code as u8),
                    (Encoding::Windows1252, _) => {
                        CP1252_HIGH.iter().position(|&h| h == c).map(|i| 0x80 + i as u8)
                    }
                    _ => None,
                };
                match byte {
                    Some(byte) => out.push(byte),
                    None => return Err(EncodeError { character: c, line }),
                }
                if c == '\n' || (c == '\r' && ending == LineEndings::Cr) {
                    line += 1;
                }
            }
            Ok(out)
        }
    }
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
        "json" | "jsonc" | "json5" | "jsonl" => "json",
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
        // No XML grammar in the kit: the HTML one colours tags, attributes and text alike (C-010).
        "xml" => "html",
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

        // Bytes 0x80 to 0x9F make it Windows-1252: smart quotes, an en dash, the euro sign.
        let d = decode(b"caff\xe8 \x93latte\x94 \x96 3\x80\n");
        assert_eq!(d.text, "caffè \u{201C}latte\u{201D} \u{2013} 3\u{20AC}\n");
        assert_eq!((d.encoding, d.lines), (Encoding::Windows1252, 2));
        assert_eq!(Encoding::Windows1252.label(), "Windows-1252");
        // The five bytes Windows-1252 leaves undefined keep their Latin-1 control value.
        assert_eq!(decode(b"a\x81b\x8d\x8f\x90\x9dc\xe8").text, "a\u{81}b\u{8d}\u{8f}\u{90}\u{9d}cè");
        // Without any 0x80..0x9F byte the file stays Latin-1, byte for byte.
        assert_eq!(decode(b"\xa9 2026 \xff").text, "\u{a9} 2026 \u{ff}");
        assert_eq!(decode(b"\xa9 2026 \xff").encoding, Encoding::Latin1);
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

    /// C-008: decoding then encoding with the encoding and ending found gives the bytes back.
    #[test]
    fn saving_round_trips_every_encoding_and_line_ending() {
        for name in ["hello.txt", "utf8-bom.txt", "bom-only.txt", "empty.txt", "utf16le.txt", "utf16be.txt",
                     "latin1.txt", "latin1.rs", "cr-only.txt", "no-final-newline.txt", "sample.rs", "data.json"] {
            let bytes = fixture(name);
            let d = decode(&bytes);
            assert_eq!(encode(&d.text, d.encoding, d.line_endings), Ok(bytes), "{name}");
        }
        let windows: &[u8] = b"caff\xe8 \x93latte\x94 \x96 3\x80\r\na\x81b\x8d\x8f\x90\x9dc\r\n";
        let d = decode(windows);
        assert_eq!(d.encoding, Encoding::Windows1252);
        assert_eq!(encode(&d.text, Encoding::Windows1252, LineEndings::CrLf), Ok(windows.to_vec()));
        let utf16: Vec<u8> = [0xff, 0xfe].into_iter().chain("é\r\n😀".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let d = decode(&utf16);
        assert_eq!(encode(&d.text, Encoding::Utf16Le, LineEndings::CrLf), Ok(utf16));
        // Mixed endings are written as LF once the file is saved; the ending can be changed.
        let d = decode(&fixture("mixed-endings.txt"));
        assert_eq!(d.text, "one\r\ntwo\nthree\r\n");
        assert_eq!(encode(&d.text, Encoding::Utf8, LineEndings::Mixed), Ok(b"one\ntwo\nthree\n".to_vec()));
        assert_eq!(encode(&d.text, Encoding::Utf8, LineEndings::CrLf), Ok(b"one\r\ntwo\r\nthree\r\n".to_vec()));
        assert_eq!(encode("a\nb\r\nc\rd", Encoding::Utf8, LineEndings::Cr), Ok(b"a\rb\rc\rd".to_vec()));
        assert_eq!(encode("no break", Encoding::Utf8, LineEndings::None), Ok(b"no break".to_vec()));
        assert_eq!(with_line_endings("\r\n\n\r", LineEndings::CrLf), "\r\n\r\n\r\n");
        // Changing the encoding: the mark follows, and so does the byte order.
        assert_eq!(encode("hi\n", Encoding::Utf8Bom, LineEndings::Lf), Ok(b"\xef\xbb\xbfhi\n".to_vec()));
        assert_eq!(encode("hi", Encoding::Utf16Be, LineEndings::Lf), Ok(b"\xfe\xff\x00h\x00i".to_vec()));
        assert_eq!(encode("caffè", Encoding::Latin1, LineEndings::Lf), Ok(b"caff\xe8".to_vec()));
        assert_eq!(encode("3\u{20AC}", Encoding::Windows1252, LineEndings::Lf), Ok(b"3\x80".to_vec()));
        // A character the encoding cannot hold is refused, with its line.
        assert_eq!(
            encode("ok\n3\u{20AC}", Encoding::Latin1, LineEndings::Lf),
            Err(EncodeError { character: '\u{20AC}', line: 2 })
        );
        assert_eq!(
            encode("a\rb\r😀", Encoding::Latin1, LineEndings::Cr),
            Err(EncodeError { character: '😀', line: 3 })
        );
        assert_eq!(
            encode("x\u{85}", Encoding::Windows1252, LineEndings::Lf),
            Err(EncodeError { character: '\u{85}', line: 1 }),
            "U+0085 is not a Windows-1252 character: that byte is the ellipsis"
        );
        assert_eq!(encode("x\u{85}", Encoding::Latin1, LineEndings::Lf), Ok(b"x\x85".to_vec()));
        assert_eq!(
            EncodeError { character: '\u{20AC}', line: 2 }.message(Encoding::Latin1),
            "Latin-1 cannot hold '€' (U+20AC) on line 2; choose another encoding"
        );
    }

    /// C-008: exact bytes after a known edit, in the file's own encoding and endings.
    #[test]
    fn an_edit_is_written_in_the_original_encoding_and_endings() {
        let d = decode(&fixture("utf16le.txt"));
        let edited = d.text.replace("hi", "hi there");
        let expected: Vec<u8> = [0xff, 0xfe].into_iter().chain("hi there\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(encode(&edited, d.encoding, d.line_endings), Ok(expected));
        let d = decode(&fixture("latin1.txt"));
        let edited = format!("{}seconda riga\n", d.text);
        assert_eq!(encode(&edited, d.encoding, d.line_endings), Ok(b"caff\xe8 latte \xe0 Roma\nseconda riga\n".to_vec()));
        let d = decode(&fixture("utf8-bom.txt"));
        let edited = d.text.replace("bom", "mark");
        assert_eq!(encode(&edited, d.encoding, d.line_endings), Ok(b"\xef\xbb\xbfmark then text\n".to_vec()));
        // The editor shows a CR-only file with LF breaks (C-005); saving writes CR again.
        let d = decode(&fixture("cr-only.txt"));
        let shown = d.text.replace('\r', "\n");
        let edited = shown.replace("beta", "BETA");
        assert_eq!(encode(&edited, d.encoding, d.line_endings), Ok(b"alpha\rBETA\rgamma".to_vec()));
    }
}
