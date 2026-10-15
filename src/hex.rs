//! The hex view's model (C-007): a memory-mapped file rendered as rows of 16 bytes, with
//! offset parsing and byte or text search. No window here; the view paints the rows.

use std::fs::File;
use std::ops::Range;
use std::path::Path;

use memmap2::Mmap;

/// Bytes per row.
pub const ROW_BYTES: usize = 16;

/// The fewest digits an offset is written with.
pub const MIN_OFFSET_DIGITS: usize = 8;

enum Bytes {
    Mapped(Mmap),
    Owned(Vec<u8>),
}

/// A file's bytes, mapped rather than read.
pub struct HexFile {
    bytes: Bytes,
}

impl HexFile {
    /// Map `path`. An empty file cannot be mapped and is held as no bytes.
    pub fn open(path: &Path) -> std::io::Result<HexFile> {
        let file = File::open(path)?;
        if file.metadata()?.len() == 0 {
            return Ok(HexFile::from_bytes(Vec::new()));
        }
        // SAFETY: the mapping is read-only. A file truncated by another process while it is
        // on screen could fault the viewer; that is the price of a mapped view, which the
        // clause chooses so that a multi-gigabyte file opens at once.
        let map = unsafe { Mmap::map(&file)? };
        Ok(HexFile { bytes: Bytes::Mapped(map) })
    }

    pub fn from_bytes(bytes: Vec<u8>) -> HexFile {
        HexFile { bytes: Bytes::Owned(bytes) }
    }

    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            Bytes::Mapped(map) => map,
            Bytes::Owned(vec) => vec,
        }
    }

    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of rows; an empty file has none.
    pub fn rows(&self) -> usize {
        self.len().div_ceil(ROW_BYTES)
    }

    pub fn offset_digits(&self) -> usize {
        offset_digits(self.len())
    }

    /// The bytes of row `index` (fewer than 16 on the last row).
    pub fn row_bytes(&self, index: usize) -> &[u8] {
        let start = (index * ROW_BYTES).min(self.len());
        let end = (start + ROW_BYTES).min(self.len());
        &self.bytes()[start..end]
    }

    /// Row `index` rendered as one line, with `mark` (byte offsets) splitting the columns.
    pub fn row(&self, index: usize, mark: Option<&Range<usize>>) -> HexRow {
        render_row(index * ROW_BYTES, self.row_bytes(index), self.offset_digits(), mark)
    }

    /// The first occurrence of `needle` at or after `from`.
    pub fn find(&self, needle: &[u8], from: usize) -> Option<usize> {
        let bytes = self.bytes();
        if needle.is_empty() || from >= bytes.len() {
            return None;
        }
        memchr::memmem::find(&bytes[from..], needle).map(|at| at + from)
    }
}

/// How many hexadecimal digits offsets in a file of `len` bytes are written with: enough
/// for the last offset, and never fewer than eight.
pub fn offset_digits(len: usize) -> usize {
    let last = len.saturating_sub(1);
    let needed = if last == 0 { 1 } else { (usize::BITS - last.leading_zeros()).div_ceil(4) as usize };
    needed.max(MIN_OFFSET_DIGITS)
}

/// The row that holds byte `offset`.
pub fn row_of(offset: usize) -> usize {
    offset / ROW_BYTES
}

/// One rendered row. `hex` and `ascii` are each split in three at the mark: before, marked,
/// after (the middle part is empty when the mark misses the row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    pub offset: String,
    pub hex: [String; 3],
    pub ascii: [String; 3],
}

impl HexRow {
    /// The whole line, as the clause spells it.
    pub fn line(&self) -> String {
        format!("{}  {}  |{}|", self.offset, self.hex.concat(), self.ascii.concat())
    }
}

/// Render a row starting at `offset` holding `bytes` (16 or, on the last row, fewer).
pub fn render_row(offset: usize, bytes: &[u8], digits: usize, mark: Option<&Range<usize>>) -> HexRow {
    let cells: Vec<String> = (0..ROW_BYTES)
        .map(|i| bytes.get(i).map_or_else(|| "  ".to_string(), |b| format!("{b:02x}")))
        .collect();
    let separator = |i: usize| match i {
        7 => "  ",
        15 => "",
        _ => " ",
    };
    let chars: Vec<char> = (0..ROW_BYTES)
        .map(|i| match bytes.get(i) {
            Some(b) if (0x20..0x7f).contains(b) => *b as char,
            Some(_) => '.',
            None => ' ',
        })
        .collect();
    // The marked cells of this row, clipped to the bytes it holds.
    let marked = mark.map_or(0..0, |m| {
        let start = m.start.clamp(offset, offset + bytes.len()) - offset;
        let end = m.end.clamp(offset, offset + bytes.len()) - offset;
        start..end.max(start)
    });
    let mut hex = [String::new(), String::new(), String::new()];
    let mut ascii = [String::new(), String::new(), String::new()];
    for i in 0..ROW_BYTES {
        let part = if i < marked.start || marked.is_empty() {
            0
        } else if i < marked.end {
            1
        } else {
            2
        };
        hex[part].push_str(&cells[i]);
        // The separator after the last marked cell belongs to what follows the mark.
        let separator_part = if part == 1 && i + 1 == marked.end { 2 } else { part };
        hex[separator_part].push_str(separator(i));
        ascii[part].push(chars[i]);
    }
    HexRow { offset: format!("{offset:0digits$x}"), hex, ascii }
}

/// An offset typed by the user: decimal, or hexadecimal with a `0x` prefix.
pub fn parse_offset(text: &str) -> Result<usize, String> {
    let text = text.trim();
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => usize::from_str_radix(hex, 16),
        None => text.parse::<usize>(),
    };
    parsed.map_err(|_| format!("`{text}` is not an offset: use decimal or 0x and hexadecimal digits"))
}

/// A byte pattern typed by the user: hexadecimal pairs, spaces optional.
pub fn parse_bytes(text: &str) -> Result<Vec<u8>, String> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() {
        return Err("type the bytes to find as hexadecimal pairs, such as `ff d8 ff`".into());
    }
    if digits.len() % 2 != 0 {
        return Err(format!("`{}` has an odd number of hexadecimal digits", text.trim()));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16)
                .map_err(|_| format!("`{}` is not hexadecimal", &digits[i..i + 2]))
        })
        .collect()
}

/// What the search field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Text,
    Bytes,
}

impl SearchMode {
    pub fn label(self) -> &'static str {
        match self {
            SearchMode::Text => "text",
            SearchMode::Bytes => "bytes",
        }
    }

    pub fn other(self) -> SearchMode {
        match self {
            SearchMode::Text => SearchMode::Bytes,
            SearchMode::Bytes => SearchMode::Text,
        }
    }

    /// The bytes to look for.
    pub fn needle(self, text: &str) -> Result<Vec<u8>, String> {
        match self {
            SearchMode::Text if text.is_empty() => Err("type the text to find".into()),
            SearchMode::Text => Ok(text.as_bytes().to_vec()),
            SearchMode::Bytes => parse_bytes(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"Hello, world\n\x00\xff\x7fHello, world\n\x00\xff\x7f\x20\x7e\x1f\x80";

    #[test]
    fn rows_render_offset_hex_and_ascii_columns_exactly() {
        let file = HexFile::from_bytes(SAMPLE.to_vec());
        assert_eq!((file.len(), file.rows(), file.offset_digits()), (36, 3, 8));
        assert_eq!(
            file.row(0, None).line(),
            "00000000  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 0a 00 ff 7f  |Hello, world....|"
        );
        assert_eq!(
            file.row(1, None).line(),
            "00000010  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 0a 00 ff 7f  |Hello, world....|"
        );
        // The last row holds four bytes; the columns stay aligned.
        assert_eq!(
            file.row(2, None).line(),
            format!("00000020  20 7e 1f 80{}  | ~..            |", " ".repeat(37))
        );
        assert_eq!(file.row(2, None).line().len(), file.row(0, None).line().len());
        // The mark splits both columns; the separator after the mark follows it.
        let marked = file.row(0, Some(&(7..12)));
        assert_eq!(marked.hex, ["48 65 6c 6c 6f 2c 20 ".to_string(), "77  6f 72 6c 64".to_string(), " 0a 00 ff 7f".to_string()]);
        assert_eq!(marked.ascii, ["Hello, ".to_string(), "world".to_string(), "....".to_string()]);
        assert_eq!(marked.line(), file.row(0, None).line());
        // A mark across the group gap, at a row end, and off the row.
        let marked = file.row(0, Some(&(14..18)));
        assert_eq!(marked.hex, ["48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 0a 00 ".to_string(), "ff 7f".to_string(), String::new()]);
        assert_eq!(marked.ascii[1], "..");
        let marked = file.row(1, Some(&(14..18)));
        assert_eq!(marked.hex, [String::new(), "48 65".to_string(), " 6c 6c 6f 2c 20 77  6f 72 6c 64 0a 00 ff 7f".to_string()]);
        let marked = file.row(2, Some(&(0..3)));
        assert_eq!(marked.hex[1], "");
        assert_eq!(marked.line(), file.row(2, None).line());
        let marked = file.row(2, Some(&(35..40)));
        assert_eq!(marked.hex, ["20 7e 1f ".to_string(), "80".to_string(), " ".repeat(37)]);
        // Empty file: no rows; a single byte: one row.
        let empty = HexFile::from_bytes(Vec::new());
        assert_eq!((empty.rows(), empty.offset_digits(), empty.is_empty()), (0, 8, true));
        let one = HexFile::from_bytes(vec![0x41]);
        assert_eq!(one.rows(), 1);
        assert_eq!(one.row(0, None).line(), format!("00000000  41{}  |A               |", " ".repeat(46)));
        // Offsets grow past eight digits only when the last offset needs it.
        assert_eq!(offset_digits(0x1_0000_0000), 8);
        assert_eq!(offset_digits(0x1_0000_0001), 9);
        assert_eq!(offset_digits(0x8000_0010), 8);
        assert_eq!(render_row(0x1_0000_0000, b"\x00", 9, None).offset, "100000000");
        assert_eq!(row_of(0), 0);
        assert_eq!(row_of(15), 0);
        assert_eq!(row_of(16), 1);
        assert_eq!(row_of(0x8000_0010), 0x800_0001);
    }

    #[test]
    fn offsets_and_byte_patterns_parse() {
        assert_eq!(parse_offset("0"), Ok(0));
        assert_eq!(parse_offset(" 496 "), Ok(496));
        assert_eq!(parse_offset("0x1F0"), Ok(496));
        assert_eq!(parse_offset("0X1f0"), Ok(496));
        assert!(parse_offset("1F0").unwrap_err().contains("not an offset"));
        assert!(parse_offset("").is_err());
        assert!(parse_offset("-1").is_err());
        assert_eq!(parse_bytes("ff d8 ff"), Ok(vec![0xff, 0xd8, 0xff]));
        assert_eq!(parse_bytes("FFD8FF"), Ok(vec![0xff, 0xd8, 0xff]));
        assert_eq!(parse_bytes(" 00  01\t02 "), Ok(vec![0, 1, 2]));
        assert_eq!(parse_bytes("abc").unwrap_err(), "`abc` has an odd number of hexadecimal digits");
        assert_eq!(parse_bytes("zz").unwrap_err(), "`zz` is not hexadecimal");
        assert!(parse_bytes("  ").unwrap_err().starts_with("type the bytes"));
        assert_eq!(SearchMode::Text.needle("é"), Ok(vec![0xc3, 0xa9]));
        assert_eq!(SearchMode::Bytes.needle("c3 a9"), Ok(vec![0xc3, 0xa9]));
        assert!(SearchMode::Text.needle("").is_err());
        assert_eq!((SearchMode::Text.label(), SearchMode::Bytes.label()), ("text", "bytes"));
        assert_eq!(SearchMode::Text.other(), SearchMode::Bytes);
    }

    #[test]
    fn search_finds_exact_offsets_in_text_and_bytes() {
        let file = HexFile::from_bytes(SAMPLE.to_vec());
        assert_eq!(file.find(b"world", 0), Some(7));
        assert_eq!(file.find(b"world", 7), Some(7), "a search from the hit finds it again");
        assert_eq!(file.find(b"world", 8), Some(23), "the next hit after the first");
        assert_eq!(file.find(b"world", 24), None, "no hit after the last");
        assert_eq!(file.find(&[0x00, 0xff, 0x7f], 0), Some(13));
        assert_eq!(file.find(&[0x00, 0xff, 0x7f], 14), Some(29));
        assert_eq!(file.find(&[0x7e, 0x1f, 0x80], 0), Some(33), "a hit ending on the last byte");
        assert_eq!(file.find(b"", 0), None, "nothing to look for");
        assert_eq!(file.find(b"H", 36), None, "a search from the end finds nothing");
        assert_eq!(file.find(b"H", 1000), None);
        assert_eq!(HexFile::from_bytes(Vec::new()).find(b"H", 0), None);
    }

    #[test]
    fn a_file_is_mapped_and_an_empty_one_is_held_as_no_bytes() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("sfoglia-hex-{}.bin", std::process::id()));
        std::fs::write(&path, SAMPLE).unwrap();
        let file = HexFile::open(&path).unwrap();
        assert_eq!(file.bytes(), SAMPLE);
        assert!(matches!(file.bytes, Bytes::Mapped(_)));
        assert_eq!(file.find(b"world", 8), Some(23));
        drop(file);
        std::fs::write(&path, b"").unwrap();
        let empty = HexFile::open(&path).unwrap();
        assert!(empty.is_empty());
        assert!(matches!(empty.bytes, Bytes::Owned(_)));
        let _ = std::fs::remove_file(&path);
        assert!(HexFile::open(&dir.join("sfoglia-does-not-exist.bin")).is_err());
    }
}
