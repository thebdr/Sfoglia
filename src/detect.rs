//! What a file is, and which viewer gets it (C-004).
//!
//! Identification is by extension first, through the table generated from the
//! contract's family clauses, then by content. A *definitive* signature at the start
//! of the file (PNG, PDF, 7z, ...) pins one family and overrides a misleading
//! extension. A *container* signature shared by several families (ZIP, OLE, TIFF,
//! Ogg, Matroska, gzip, ISO media without a telling brand) never overrides a known
//! extension: it only settles an unknown one, or picks among the families that share
//! an extension. A file with no known extension and no signature is text when its
//! bytes decode as text and binary otherwise.

use std::path::Path;

use crate::generated::families::{EXTENSIONS, Family};

/// How many leading bytes `detect` wants to see. A caller may pass fewer.
pub const SNIFF_BYTES: usize = 8 * 1024;

/// How the leading bytes of a file read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// Zero bytes.
    Empty,
    /// Decodes as text in the given encoding.
    Text(TextEncoding),
    /// A known signature; `definitive` says whether it pins one family on its own
    /// (PNG, PDF) or only names a container shared by many families (ZIP, TIFF).
    /// For a container, `family` is the family an unknown extension defaults to.
    Signature {
        family: Family,
        label: &'static str,
        definitive: bool,
    },
    /// Bytes that are neither text nor a known signature.
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// Not valid UTF-8 and without a byte-order mark, but no byte is a control byte
    /// beyond whitespace and escape, nor DEL: single-byte text, Latin-1 or a Windows
    /// code page (bytes 0x80 to 0x9F are Windows-1252 punctuation and count as text).
    Latin1,
}

/// Where a file goes: a family's viewer, or the hex view for anything binary that no
/// family claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    Family(Family),
    Hex,
}

impl Route {
    /// The viewer id: the family slug, or `hex`.
    pub fn id(self) -> &'static str {
        match self {
            Route::Family(family) => family.slug(),
            Route::Hex => "hex",
        }
    }

    /// The clause that brings the viewer for this route.
    pub fn clause(self) -> &'static str {
        match self {
            Route::Family(family) => family.clause(),
            Route::Hex => "P-005",
        }
    }
}

/// The outcome of identifying one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    /// The extension that was looked up, lower case, without the dot; the longest
    /// multi-part suffix the table knows (`tar.gz`) or else the last suffix.
    pub extension: Option<String>,
    /// The families the table lists for that extension, in contract order.
    pub by_extension: &'static [Family],
    pub content: Content,
    /// The family that won, if any.
    pub family: Option<Family>,
    pub route: Route,
}

/// A leading-bytes signature: `magic` at `offset`, and optionally a second run
/// (`RIFF` at 0 together with `WAVE` at 8).
struct Signature {
    magic: &'static [u8],
    offset: usize,
    also: Option<(usize, &'static [u8])>,
    family: Family,
    label: &'static str,
    definitive: bool,
}

macro_rules! sig {
    ($magic:expr, $offset:expr, $family:ident, $label:expr, $definitive:expr) => {
        Signature { magic: $magic, offset: $offset, also: None, family: Family::$family, label: $label, definitive: $definitive }
    };
    ($magic:expr, $offset:expr, also $also:expr, $family:ident, $label:expr, $definitive:expr) => {
        Signature { magic: $magic, offset: $offset, also: Some($also), family: Family::$family, label: $label, definitive: $definitive }
    };
}

const SIGNATURES: &[Signature] = &[
    // Images
    sig!(b"\x89PNG\r\n\x1a\n", 0, Images, "PNG", true),
    sig!(b"\xff\xd8\xff", 0, Images, "JPEG", true),
    sig!(b"GIF87a", 0, Images, "GIF", true),
    sig!(b"GIF89a", 0, Images, "GIF", true),
    sig!(b"RIFF", 0, also (8, b"WEBP"), Images, "WebP", true),
    sig!(b"8BPS", 0, Images, "Photoshop", true),
    sig!(b"\x76\x2f\x31\x01", 0, Images, "OpenEXR", true),
    // TIFF is also the container of most camera RAW formats (DNG, NEF, CR2, ARW).
    sig!(b"II*\x00", 0, Images, "TIFF container", false),
    sig!(b"MM\x00*", 0, Images, "TIFF container", false),
    // Documents (PDF has its own header check, see `pdf_signature`)
    sig!(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", 0, WordProcessing, "OLE compound container", false),
    // Archives
    sig!(b"7z\xbc\xaf\x27\x1c", 0, Archives, "7-Zip", true),
    sig!(b"\xfd7zXZ\x00", 0, Archives, "xz", true),
    sig!(b"BZh", 0, also (4, b"1AY&SY"), Archives, "bzip2", true),
    sig!(b"Rar!\x1a\x07", 0, Archives, "RAR", true),
    sig!(b"ustar\x00", 257, Archives, "tar", true),
    sig!(b"ustar ", 257, Archives, "tar", true),
    // gzip also wraps svgz and tar.gz; ZIP wraps docx, xlsx, pptx, odt, vsdx, jar, epub.
    sig!(b"\x1f\x8b\x08", 0, Archives, "gzip container", false),
    sig!(b"PK\x03\x04", 0, Archives, "ZIP container", false),
    // Email
    sig!(b"\x78\x9f\x3e\x22", 0, Email, "TNEF (winmail.dat)", true),
    // Audio
    sig!(b"fLaC", 0, Audio, "FLAC", true),
    sig!(b"RIFF", 0, also (8, b"WAVE"), Audio, "WAV", true),
    // Ogg carries Vorbis and Opus audio as well as Theora video.
    sig!(b"OggS", 0, Audio, "Ogg container", false),
    // Video
    sig!(b"RIFF", 0, also (8, b"AVI "), Video, "AVI", true),
    sig!(b"\x00\x00\x01\xba", 0, Video, "MPEG program stream", true),
    // Matroska carries video (mkv, webm) and audio-only (mka) files.
    sig!(b"\x1a\x45\xdf\xa3", 0, Video, "Matroska container", false),
];

/// ISO base media files (`ftyp` at offset 4) are told apart by their major brand:
/// still images (HEIF, AVIF), audio (M4A), camera RAW (Canon CR3) and movies share
/// the container.
fn iso_media_signature(head: &[u8]) -> Option<Content> {
    // The file opens with the ftyp box's big-endian size, a small number whose three
    // leading bytes are zero; prose with "ftyp" at offset 4 has letters there instead.
    if head.len() < 12 || &head[4..8] != b"ftyp" || head[0..3] != [0, 0, 0] {
        return None;
    }
    let brand = &head[8..12];
    let (family, label, definitive) = match brand {
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"heim" | b"heis" => {
            (Family::Images, "HEIF (ISO media)", true)
        }
        b"avif" | b"avis" => (Family::Images, "AVIF (ISO media)", true),
        b"M4A " | b"M4B " => (Family::Audio, "M4A (ISO media)", true),
        b"crx " => (Family::CameraRaw, "Canon CR3 (ISO media)", true),
        b"M4V " | b"qt  " => (Family::Video, "Movie (ISO media)", true),
        // Generic brands (isom, mp42, dash, 3gp...) label audio-only files as often as
        // movies, so they only settle an unknown extension, defaulting to video.
        _ => (Family::Video, "ISO media container", false),
    };
    Some(Content::Signature { family, label, definitive })
}

/// MP3 files tagged with ID3v2: `ID3`, a major version 2 to 4, revision 0.
fn id3_signature(head: &[u8]) -> Option<Content> {
    (head.len() >= 5 && &head[0..3] == b"ID3" && (2..=4).contains(&head[3]) && head[4] == 0).then_some(
        Content::Signature { family: Family::Audio, label: "MP3 (ID3)", definitive: true },
    )
}

/// An untagged MP3 starts with an MPEG audio frame: eleven sync bits followed by the
/// Layer III bits (`FF FB`, `FF FA`, `FF F3`, `FF F2`, `FF E3`, ...). JPEG's `FF D8` does
/// not satisfy the layer bits.
fn mp3_frame_signature(head: &[u8]) -> Option<Content> {
    (head.len() >= 4 && head[0] == 0xff && head[1] & 0xe6 == 0xe2).then_some(Content::Signature {
        family: Family::Audio,
        label: "MP3 (frame sync)",
        definitive: true,
    })
}

/// A PDF opens with its full header line, `%PDF-1.7` followed by a line ending. A whole
/// PDF can be printable, so this is the one signature believed inside text, and prose
/// that merely starts with `%PDF-` does not match it.
fn pdf_signature(head: &[u8]) -> Option<Content> {
    let shaped = head.len() >= 9
        && head.starts_with(b"%PDF-")
        && head[5].is_ascii_digit()
        && head[6] == b'.'
        && head[7].is_ascii_digit()
        && matches!(head[8], b'\r' | b'\n');
    shaped.then_some(Content::Signature { family: Family::PdfXps, label: "PDF", definitive: true })
}

/// The signature at the start of `head`, if any. A signature spelled entirely in
/// printable text (`GIF89a`, `8BPS`, `fLaC`, `OggS`, `BZh…`) is only believed when the
/// sample does not read as text: a real file of that kind carries binary bytes right
/// after its magic, while a note that merely begins with those letters is text.
pub fn signature(head: &[u8]) -> Option<Content> {
    if let Some(found) = pdf_signature(head)
        .or_else(|| iso_media_signature(head))
        .or_else(|| id3_signature(head))
        .or_else(|| mp3_frame_signature(head))
    {
        return Some(found);
    }
    let reads_as_text = text_encoding(head).is_some();
    let printable = |bytes: &[u8]| bytes.iter().all(|b| (0x20..0x7f).contains(b));
    let matches_at = |offset: usize, magic: &[u8]| {
        head.len() >= offset + magic.len() && &head[offset..offset + magic.len()] == magic
    };
    SIGNATURES.iter().find_map(|s| {
        let hit = matches_at(s.offset, s.magic)
            && s.also.is_none_or(|(offset, magic)| matches_at(offset, magic));
        let spelled_in_text = printable(s.magic) && s.also.is_none_or(|(_, m)| printable(m));
        let believed = hit && (!spelled_in_text || !reads_as_text);
        believed.then_some(Content::Signature {
            family: s.family,
            label: s.label,
            definitive: s.definitive,
        })
    })
}

/// A character text may contain: anything but a control character, except the usual
/// whitespace controls and escape (terminal colour codes appear in logs).
fn is_text_char(c: char) -> bool {
    !c.is_control() || matches!(c, '\t' | '\n' | '\r' | '\x0c' | '\x1b')
}

/// Bytes that are not UTF-8 still read as text when none of them is a control byte
/// other than ordinary whitespace and escape, and none is DEL: that is Latin-1 or a
/// Windows code page (whose bytes 0x80 to 0x9F are punctuation, so they pass), which the
/// text viewer decodes byte by byte. A byte-order mark promises its encoding, so a sample
/// that starts with one never falls back here.
fn looks_like_latin1(body: &[u8]) -> bool {
    !body.is_empty()
        && body
            .iter()
            .all(|&b| b >= 0x20 && b != 0x7f || matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b))
}

/// Whether `head` reads as text, and in which encoding. UTF-8 is accepted when every
/// byte decodes (a cut-off final character is forgiven) and every character passes
/// `is_text_char`. UTF-16 is recognised by its byte-order mark and then held to the same
/// rule: every unit after the mark must decode (a cut-off final unit or surrogate pair
/// is forgiven) to text characters. Bytes that are not UTF-8 are Latin-1 text when they
/// contain no control bytes beyond whitespace and escape.
pub fn text_encoding(head: &[u8]) -> Option<TextEncoding> {
    let utf16 = |body: &[u8], encoding: TextEncoding| -> Option<TextEncoding> {
        let whole = &body[..body.len() - body.len() % 2];
        let units = whole.chunks_exact(2).map(|pair| match encoding {
            TextEncoding::Utf16Le => u16::from_le_bytes([pair[0], pair[1]]),
            _ => u16::from_be_bytes([pair[0], pair[1]]),
        });
        let mut decoded = char::decode_utf16(units).peekable();
        while let Some(result) = decoded.next() {
            match result {
                Ok(c) if is_text_char(c) => {}
                // An unpaired surrogate is forgiven only as the sample's last unit.
                Err(_) if decoded.peek().is_none() => {}
                _ => return None,
            }
        }
        Some(encoding)
    };
    if let Some(body) = head.strip_prefix(&[0xff, 0xfe]) {
        return utf16(body, TextEncoding::Utf16Le);
    }
    if let Some(body) = head.strip_prefix(&[0xfe, 0xff]) {
        return utf16(body, TextEncoding::Utf16Be);
    }
    let (body, bom) = match head.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        Some(rest) => (rest, true),
        None => (head, false),
    };
    let valid = match std::str::from_utf8(body) {
        Ok(text) => text,
        Err(e) => {
            // Only a character cut off by the end of the sample is forgiven; anything
            // else is not UTF-8, and may still be Latin-1 text.
            if e.error_len().is_some() {
                return (!bom && looks_like_latin1(body)).then_some(TextEncoding::Latin1);
            }
            std::str::from_utf8(&body[..e.valid_up_to()]).ok()?
        }
    };
    valid
        .chars()
        .all(is_text_char)
        .then_some(if bom { TextEncoding::Utf8Bom } else { TextEncoding::Utf8 })
}

/// Classify the leading bytes of a file.
pub fn sniff(head: &[u8]) -> Content {
    if head.is_empty() {
        return Content::Empty;
    }
    if let Some(found) = signature(head) {
        return found;
    }
    match text_encoding(head) {
        Some(encoding) => Content::Text(encoding),
        None => Content::Binary,
    }
}

/// The families the table lists for an extension (lower case, no dot).
pub fn families_for_extension(extension: &str) -> &'static [Family] {
    EXTENSIONS
        .binary_search_by_key(&extension, |(e, _)| e)
        .map(|i| EXTENSIONS[i].1)
        .unwrap_or(&[])
}

/// The extension to look up for a file name: the longest dotted suffix the table
/// knows (`tar.gz` before `gz`), else the last suffix, lower case. A name without a
/// dot, or ending in one, has none.
pub fn extension_of(name: &str) -> Option<String> {
    let name = Path::new(name).file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    let mut candidates: Vec<&str> = lower
        .match_indices('.')
        .map(|(i, _)| &lower[i + 1..])
        .filter(|s| !s.is_empty())
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // Longest first; the last entry is the plain final suffix.
    let known = candidates.iter().find(|c| !families_for_extension(c).is_empty());
    let chosen = known.copied().unwrap_or_else(|| candidates.pop().unwrap());
    Some(chosen.to_string())
}

/// Identify a file from its name and its leading bytes (up to `SNIFF_BYTES`).
pub fn detect(name: &str, head: &[u8]) -> Detection {
    let extension = extension_of(name);
    let by_extension = extension
        .as_deref()
        .map(families_for_extension)
        .unwrap_or(&[]);
    let content = sniff(head);

    let family = match (content, by_extension) {
        // A definitive signature pins its family, whatever the name says.
        (Content::Signature { family, definitive: true, .. }, _) => Some(family),
        // Any signature settles an extension shared by several families when it is one of them.
        (Content::Signature { family, .. }, candidates) if candidates.contains(&family) => Some(family),
        // A known extension wins over containers, text and binary alike; one shared by
        // several families and settled by no signature goes to the first in contract order.
        (_, [only]) => Some(*only),
        (_, [first, ..]) => Some(*first),
        // No known extension: content decides.
        (Content::Signature { family, .. }, []) => Some(family),
        (Content::Empty | Content::Text(_), []) => Some(Family::Text),
        (Content::Binary, []) => None,
    };
    let route = family.map_or(Route::Hex, Route::Family);
    Detection {
        extension,
        by_extension,
        content,
        family,
        route,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Family::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    const ZIP: &[u8] = b"PK\x03\x04\x14\x00\x00\x00";
    const NOISE: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0xfe, 0xff, 0x10, 0x80, 0x00, 0x7f];

    fn sig(family: Family, label: &'static str, definitive: bool) -> Content {
        Content::Signature { family, label, definitive }
    }

    #[test]
    fn every_family_extension_routes_to_its_family() {
        for family in Family::ALL {
            let owned: Vec<&str> = EXTENSIONS
                .iter()
                .filter(|(_, owners)| owners.first() == Some(family))
                .map(|(e, _)| *e)
                .collect();
            assert!(!owned.is_empty(), "{} lists at least one extension", family.slug());
            for ext in owned {
                let d = detect(&format!("sample.{ext}"), b"");
                assert_eq!(d.extension.as_deref(), Some(ext));
                assert_eq!(d.family, Some(*family), "sample.{ext}");
                assert_eq!(d.route, Route::Family(*family));
                assert_eq!(d.route.id(), family.slug());
                assert_eq!(d.route.clause(), family.clause());
            }
        }
        assert_eq!(detect("archive.tar.gz", b"").extension.as_deref(), Some("tar.gz"));
        assert_eq!(detect("ARCHIVE.TAR.GZ", b"").family, Some(Archives));
        assert_eq!(detect("notes.backup.txt", b"").extension.as_deref(), Some("txt"));
        assert_eq!(detect("C:\\dir.v2\\report.PDF", b"").family, Some(PdfXps));
        assert_eq!(detect("/tmp/some.dir/photo.jpeg", b"").family, Some(Images));
        assert_eq!(extension_of("Makefile"), None);
        assert_eq!(extension_of("trailing."), None);
        assert_eq!(extension_of(".bashrc").as_deref(), Some("bashrc"));
        assert!(EXTENSIONS.windows(2).all(|w| w[0].0 < w[1].0), "table is sorted");
    }

    #[test]
    fn a_definitive_signature_overrides_a_misleading_extension() {
        let d = detect("photo.txt", PNG);
        assert_eq!(d.by_extension, &[Text]);
        assert_eq!(d.content, sig(Images, "PNG", true));
        assert_eq!(d.family, Some(Images));
        assert_eq!(d.route.id(), "images");
        assert_eq!(detect("song.docx", b"fLaC\x00\x00\x00\x22").family, Some(Audio));
        assert_eq!(detect("scan.csv", b"%PDF-1.7\n").family, Some(PdfXps));
        // A container signature does not: a docx is a ZIP, and stays a document.
        let d = detect("report.docx", ZIP);
        assert_eq!(d.content, sig(Archives, "ZIP container", false));
        assert_eq!(d.family, Some(WordProcessing));
        // Text content does not override a known extension either.
        assert_eq!(detect("notes.pdf", b"just words\n").family, Some(PdfXps));
        // A signature of the extension's own family changes nothing.
        assert_eq!(detect("photo.png", PNG).family, Some(Images));
        // A truncated signature is not a signature.
        assert_eq!(detect("photo.txt", &PNG[..4]).family, Some(Text));
        // Plain text that merely starts with the letters of a signature stays text.
        for note in [
            b"BZh is how I start this note\n".as_slice(),
            b"BZh91AY&SY looks like a bzip2 block but this is prose\n",
            b"ID3 tags explained\n",
            b"RIFF is a container format\n",
            b"RIFF....WEBP spelled out in a note\n",
            b"GIF89a was released in 1989.\n",
            b"GIF87a came first.\n",
            b"8BPS is the Photoshop magic.\n",
            b"fLaC means FLAC.\n",
            b"OggS is the Ogg page marker.\n",
            b"Rar! is how RAR archives begin.\n",
            b"7z is a format; this line is text.\n",
            b"The ftypisom brand marks an MP4 file.\n",
            b"The ftyp box opens every MP4 file.\n",
            b"%PDF-1.7 is the version we target.\n",
            b"%PDF- without a version is not a header\n",
        ] {
            assert_eq!(sniff(note), Content::Text(TextEncoding::Utf8), "{}", String::from_utf8_lossy(note));
            assert_eq!(detect("notes.txt", note).family, Some(Text), "{}", String::from_utf8_lossy(note));
            assert_eq!(detect("README", note).family, Some(Text), "{}", String::from_utf8_lossy(note));
        }
        // A whole PDF can be printable, so its full header line is believed even in text.
        assert_eq!(detect("x.q", b"%PDF-1.4\n1 0 obj\n").family, Some(PdfXps));
        assert_eq!(detect("notes.txt", b"%PDF-1.7\r\n%\xe2\xe3\xcf\xd3\n").family, Some(PdfXps));
        assert_eq!(sniff(b"%PDF-1.7\n"), sig(PdfXps, "PDF", true));
        assert_eq!(sniff(b"%PDF-1.7"), Content::Text(TextEncoding::Utf8), "no line ending yet");
    }

    #[test]
    fn container_signatures_never_override_a_known_extension() {
        let ftyp = |brand: &[u8]| [b"\x00\x00\x00\x18ftyp".as_slice(), brand, b"\x00\x00\x00\x00"].concat();
        let tiff: &[u8] = b"II*\x00\x08\x00\x00\x00";
        let ogg: &[u8] = b"OggS\x00\x02\x00\x00";
        let ebml: &[u8] = b"\x1a\x45\xdf\xa3\x93\x42\x82\x88";
        let gzip: &[u8] = b"\x1f\x8b\x08\x00\x00\x00\x00\x00";
        // Correctly named files keep their family, whatever shared container they use.
        for (name, head, family) in [
            ("p.heic", ftyp(b"heic"), Images),
            ("p.avif", ftyp(b"avif"), Images),
            ("s.m4a", ftyp(b"M4A "), Audio),
            ("s.m4a", ftyp(b"isom"), Audio),
            ("s.m4a", ftyp(b"mp42"), Audio),
            ("s.m4a", ftyp(b"dash"), Audio),
            ("b.m4b", ftyp(b"isom"), Audio),
            ("r.cr3", ftyp(b"crx "), CameraRaw),
            ("m.mp4", ftyp(b"isom"), Video),
            ("m.mp4", ftyp(b"mp42"), Video),
            ("m.mov", ftyp(b"qt  "), Video),
            ("v.m4v", ftyp(b"M4V "), Video),
            ("r.dng", tiff.to_vec(), CameraRaw),
            ("r.nef", tiff.to_vec(), CameraRaw),
            ("p.tif", tiff.to_vec(), Images),
            ("v.ogv", ogg.to_vec(), Video),
            ("a.ogg", ogg.to_vec(), Audio),
            ("a.mka", ebml.to_vec(), Audio),
            ("v.mkv", ebml.to_vec(), Video),
            ("i.svgz", gzip.to_vec(), Images),
            ("t.tar.gz", gzip.to_vec(), Archives),
            ("d.docx", ZIP.to_vec(), WordProcessing),
            ("b.epub", ZIP.to_vec(), WordProcessing),
            ("s.xlsx", ZIP.to_vec(), Spreadsheets),
            ("m.msg", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".to_vec(), Email),
            ("w.doc", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".to_vec(), WordProcessing),
        ] {
            assert_eq!(detect(name, &head).family, Some(family), "{name}");
        }
        // Under an unknown extension a container falls back to its default family, and an
        // ISO media brand that tells the family is definitive.
        assert_eq!(detect("x.q", tiff).family, Some(Images));
        assert_eq!(detect("x.q", ogg).family, Some(Audio));
        assert_eq!(detect("x.q", ebml).family, Some(Video));
        assert_eq!(detect("x.q", gzip).family, Some(Archives));
        assert_eq!(detect("x.q", &ftyp(b"zzzz")).content, sig(Video, "ISO media container", false));
        assert_eq!(detect("x.q", &ftyp(b"isom")).content, sig(Video, "ISO media container", false));
        assert_eq!(detect("x.q", &ftyp(b"isom")).family, Some(Video));
        assert_eq!(detect("x.q", &ftyp(b"heic")).content, sig(Images, "HEIF (ISO media)", true));
        assert_eq!(detect("p.txt", &ftyp(b"heic")).family, Some(Images));
        assert_eq!(detect("s.txt", &ftyp(b"M4A ")).family, Some(Audio));
        assert_eq!(detect("m.txt", &ftyp(b"qt  ")).family, Some(Video));
        assert_eq!(detect("p.txt", &ftyp(b"zzzz")).family, Some(Text));
        assert_eq!(detect("s.txt", &ftyp(b"isom")).family, Some(Text), "a generic brand never overrides");
    }

    #[test]
    fn every_signature_is_recognised_with_its_real_header() {
        let mut tar = vec![0u8; 257];
        tar.extend_from_slice(b"ustar\x00" as &[u8]);
        let mut gnu_tar = vec![0u8; 257];
        gnu_tar.extend_from_slice(b"ustar  \x00" as &[u8]);
        let cases: Vec<(&str, Vec<u8>, Content, Family)> = vec![
            ("png", PNG.to_vec(), sig(Images, "PNG", true), Images),
            ("jpg", b"\xff\xd8\xff\xe0\x00\x10JFIF".to_vec(), sig(Images, "JPEG", true), Images),
            ("gif", b"GIF89a\x01\x00".to_vec(), sig(Images, "GIF", true), Images),
            ("gif", b"GIF87a\x01\x00".to_vec(), sig(Images, "GIF", true), Images),
            ("webp", b"RIFF\x24\x00\x00\x00WEBPVP8 ".to_vec(), sig(Images, "WebP", true), Images),
            ("psd", b"8BPS\x00\x01".to_vec(), sig(Images, "Photoshop", true), Images),
            ("exr", b"\x76\x2f\x31\x01\x02\x00".to_vec(), sig(Images, "OpenEXR", true), Images),
            ("tif", b"II*\x00\x08\x00".to_vec(), sig(Images, "TIFF container", false), Images),
            ("tif", b"MM\x00*\x00\x00".to_vec(), sig(Images, "TIFF container", false), Images),
            ("pdf", b"%PDF-1.4\n".to_vec(), sig(PdfXps, "PDF", true), PdfXps),
            ("doc", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00".to_vec(), sig(WordProcessing, "OLE compound container", false), WordProcessing),
            ("7z", b"7z\xbc\xaf\x27\x1c\x00\x04".to_vec(), sig(Archives, "7-Zip", true), Archives),
            ("xz", b"\xfd7zXZ\x00\x00".to_vec(), sig(Archives, "xz", true), Archives),
            ("bz2", b"BZh91AY&SY\xf1\x9c\x8b\x12".to_vec(), sig(Archives, "bzip2", true), Archives),
            ("rar", b"Rar!\x1a\x07\x00".to_vec(), sig(Archives, "RAR", true), Archives),
            ("tar", tar, sig(Archives, "tar", true), Archives),
            ("tar", gnu_tar, sig(Archives, "tar", true), Archives),
            ("gz", b"\x1f\x8b\x08\x00".to_vec(), sig(Archives, "gzip container", false), Archives),
            ("zip", ZIP.to_vec(), sig(Archives, "ZIP container", false), Archives),
            ("dat", b"\x78\x9f\x3e\x22\x01\x00".to_vec(), sig(Email, "TNEF (winmail.dat)", true), Email),
            ("mp3", b"ID3\x04\x00\x00".to_vec(), sig(Audio, "MP3 (ID3)", true), Audio),
            ("mp3", b"ID3\x03\x00\x00".to_vec(), sig(Audio, "MP3 (ID3)", true), Audio),
            ("mp3", b"\xff\xfb\x90\x64\x00\x00".to_vec(), sig(Audio, "MP3 (frame sync)", true), Audio),
            ("mp3", b"\xff\xf3\x80\x00\x00\x00".to_vec(), sig(Audio, "MP3 (frame sync)", true), Audio),
            ("mp3", b"\xff\xe3\x18\xc4\x00\x00".to_vec(), sig(Audio, "MP3 (frame sync)", true), Audio),
            ("flac", b"fLaC\x00\x00".to_vec(), sig(Audio, "FLAC", true), Audio),
            ("wav", b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec(), sig(Audio, "WAV", true), Audio),
            ("ogg", b"OggS\x00\x02".to_vec(), sig(Audio, "Ogg container", false), Audio),
            ("m4a", b"\x00\x00\x00\x18ftypM4A \x00\x00".to_vec(), sig(Audio, "M4A (ISO media)", true), Audio),
            ("avi", b"RIFF\x24\x00\x00\x00AVI LIST".to_vec(), sig(Video, "AVI", true), Video),
            ("mp4", b"\x00\x00\x00\x18ftypmp42\x00\x00".to_vec(), sig(Video, "ISO media container", false), Video),
            ("mov", b"\x00\x00\x00\x14ftypqt  \x00\x00".to_vec(), sig(Video, "Movie (ISO media)", true), Video),
            ("mkv", b"\x1a\x45\xdf\xa3\x93".to_vec(), sig(Video, "Matroska container", false), Video),
            ("mpg", b"\x00\x00\x01\xba\x44".to_vec(), sig(Video, "MPEG program stream", true), Video),
            ("heic", b"\x00\x00\x00\x18ftypheic\x00\x00".to_vec(), sig(Images, "HEIF (ISO media)", true), Images),
            ("avif", b"\x00\x00\x00\x18ftypavif\x00\x00".to_vec(), sig(Images, "AVIF (ISO media)", true), Images),
            ("cr3", b"\x00\x00\x00\x18ftypcrx \x00\x00".to_vec(), sig(CameraRaw, "Canon CR3 (ISO media)", true), CameraRaw),
        ];
        for (ext, head, content, family) in &cases {
            assert_eq!(sniff(head), *content, "{ext}");
            let neutral = detect("x.q", head);
            assert_eq!(neutral.content, *content, "x.q as {ext}");
            assert_eq!(neutral.family, Some(*family), "x.q as {ext}");
            let own = detect(&format!("x.{ext}"), head);
            assert_eq!(own.family, Some(*family), "x.{ext}");
            assert_eq!(own.content, *content, "x.{ext}");
        }
        // An untagged MP3 under a misleading or unknown name is still audio; JPEG is not.
        assert_eq!(detect("song.txt", b"\xff\xfb\x90\x64\x00\x00").family, Some(Audio));
        assert_eq!(detect("x.q", b"\xff\xfb\x90\x64\x00\x00").family, Some(Audio));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0\x00\x10"), sig(Images, "JPEG", true));
        assert_eq!(sniff(b"\xff\xf9\x00\x00"), Content::Binary, "layer bits 00 are reserved");
        // Near misses: wrong version bytes, a missing second run, a signature cut short.
        assert_eq!(sniff(b"ID3\x09\x00\x00"), Content::Binary);
        assert_eq!(sniff(b"RIFF\x24\x00\x00\x00ACON"), Content::Binary);
        assert_eq!(sniff(b"BZh9"), Content::Text(TextEncoding::Utf8));
        assert_eq!(sniff(b"BZh91AY&SY"), Content::Text(TextEncoding::Utf8), "block magic alone, all printable");
        assert_eq!(sniff(b"\x00\x00\x00\x18fty"), Content::Binary);
        assert_eq!(sniff(&vec![0u8; 257]), Content::Binary);
        assert_eq!(signature(b""), None);
    }

    #[test]
    fn unknown_extension_is_settled_by_content() {
        assert_eq!(detect("README", b"Sfoglia\n").family, Some(Text));
        assert_eq!(detect("blob.xyz", NOISE).family, None);
        assert_eq!(detect("blob.xyz", NOISE).route, Route::Hex);
        assert_eq!(detect("a.unknown", PNG).family, Some(Images));
        assert_eq!(detect("x.q", ZIP).family, Some(Archives));
        assert_eq!(detect("x.q", b"%PDF-1.7\n").family, Some(PdfXps));
        assert_eq!(detect("x.q", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1rest").family, Some(WordProcessing));
    }

    #[test]
    fn shared_extensions_are_settled_by_content_then_contract_order() {
        assert_eq!(families_for_extension("dat"), &[Email, Video]);
        assert_eq!(detect("winmail.dat", b"\x78\x9f\x3e\x22\x01\x00").family, Some(Email));
        assert_eq!(detect("movie.dat", b"\x00\x00\x01\xba\x44\x00").family, Some(Video));
        assert_eq!(detect("x.dat", b"").family, Some(Email));
        assert_eq!(detect("x.dat", NOISE).family, Some(Email));
        assert_eq!(families_for_extension("mpc"), &[Audio, Video]);
        assert_eq!(detect("x.mpc", b"").family, Some(Audio));
        assert_eq!(families_for_extension("cin"), &[Images, Video]);
        assert_eq!(detect("x.cin", b"").family, Some(Images));
        // A container shared by two candidates picks the candidate it names.
        assert_eq!(detect("x.cin", b"\x1a\x45\xdf\xa3\x93").family, Some(Video));
        assert_eq!(families_for_extension("nothing-known"), &[]);
    }

    #[test]
    fn empty_and_text_files_go_to_text_and_unknown_binary_to_hex() {
        let d = detect("empty", b"");
        assert_eq!((d.content, d.family, d.route.id()), (Content::Empty, Some(Text), "text"));
        assert_eq!(detect("x.unknownext", b"").family, Some(Text));
        // A known extension keeps its family even when the file is empty.
        assert_eq!(detect("x.png", b"").family, Some(Images));
        let d = detect("a.bin", NOISE);
        assert_eq!((d.content, d.family, d.route, d.route.clause()), (Content::Binary, None, Route::Hex, "P-005"));
        assert_eq!(text_encoding(b"\xef\xbb\xbfhi"), Some(TextEncoding::Utf8Bom));
        assert_eq!(text_encoding(b"\xff\xfeh\x00i\x00"), Some(TextEncoding::Utf16Le));
        assert_eq!(text_encoding(b"\xfe\xff\x00h\x00i"), Some(TextEncoding::Utf16Be));
        // A byte-order mark followed by binary is not text; a cut-off unit or pair is forgiven.
        assert_eq!(text_encoding(b"\xff\xfe\x00\x00\x01\x00\x02\x00"), None);
        assert_eq!(text_encoding(b"\xfe\xff\x00\x00\x00\x01"), None);
        assert_eq!(detect("x.q", b"\xff\xfe\x00\x00\x01\x00\x02\x00").route, Route::Hex);
        assert_eq!(text_encoding(b"\xff\xfeh\x00i"), Some(TextEncoding::Utf16Le), "odd trailing byte");
        assert_eq!(text_encoding(b"\xff\xfeh\x00\x3d\xd8"), Some(TextEncoding::Utf16Le), "cut-off surrogate pair");
        assert_eq!(text_encoding(b"\xff\xfe\x3d\xd8h\x00"), None, "unpaired surrogate mid-text");
        assert_eq!(text_encoding(b"\xff\xfe"), Some(TextEncoding::Utf16Le), "mark alone");
        assert_eq!(text_encoding("\u{feff}caffè\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>().as_slice()), Some(TextEncoding::Utf16Le));
        assert_eq!(text_encoding("caffè\tlatte\r\n".as_bytes()), Some(TextEncoding::Utf8));
        assert_eq!(text_encoding(b"abc\x00def"), None);
        // Latin-1 prose is text; Latin-1 bytes mixed with control bytes are binary.
        assert_eq!(text_encoding(b"caff\xe8 latte \xe0 Roma\n"), Some(TextEncoding::Latin1));
        assert_eq!(text_encoding(b"// caff\xe8 latte\nfn main() {}\n"), Some(TextEncoding::Latin1));
        assert_eq!(text_encoding(b"caff\xe8\x00latte"), None);
        assert_eq!(text_encoding(b"caff\xe8\x7flatte"), None);
        assert_eq!(text_encoding(b"\xef\xbb\xbfcaff\xe8 latte"), None, "a UTF-8 mark promises UTF-8");
        assert_eq!(text_encoding(b"\xff\xfeA\xd8AA"), None, "a UTF-16 mark promises UTF-16");
        // Windows-1252 punctuation (0x80..0x9F) and its C1 range count as text.
        assert_eq!(text_encoding(b"caff\xe8 \x93latte\x94 \x96 3\x80\n"), Some(TextEncoding::Latin1));
        assert_eq!(text_encoding(b"caff\xe8 \x85 and so on"), Some(TextEncoding::Latin1));
        assert_eq!(text_encoding(b"caff\xe8\x08"), None, "backspace is a control byte");
        assert_eq!(text_encoding(b"caff\xe8"), Some(TextEncoding::Utf8), "a trailing lead byte is a cut-off character");
        let latin_source = detect("main.rs", b"// caff\xe8 latte\nfn main() {}\n");
        assert_eq!(latin_source.content, Content::Text(TextEncoding::Latin1));
        assert_eq!(latin_source.family, Some(Text));
        assert_eq!(detect("README", b"caff\xe8 latte\n").family, Some(Text));
        assert_eq!(detect("notes.md", b"caff\xe8 latte\n").family, Some(Markdown));
        assert_eq!(detect("blob.xyz", b"caff\xe8\x00latte").route, Route::Hex);
        assert_eq!(text_encoding(b"\x1b[31mred\x1b[0m\n"), Some(TextEncoding::Utf8));
        // A multi-byte character cut by the end of the sample is still text.
        let cut = &"héllo wörld".as_bytes()[..7];
        assert_eq!(text_encoding(cut), Some(TextEncoding::Utf8));
        assert_eq!(text_encoding(b"\xc3\x28"), Some(TextEncoding::Latin1), "not UTF-8, but printable Latin-1");
        assert_eq!(detect("notes.txt", b"\xff\xfeh\x00i\x00").content, Content::Text(TextEncoding::Utf16Le));
        assert_eq!(sniff(b""), Content::Empty);
    }
}
