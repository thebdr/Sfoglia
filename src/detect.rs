//! What a file is, and which viewer gets it (C-004).
//!
//! Identification is by extension first, through the table generated from the
//! contract's family clauses, then by content: a definitive signature at the start
//! of the file overrides a misleading extension, an extension shared by several
//! families is settled by content and otherwise by contract order, and a file with no
//! known extension is text when its bytes decode as text and binary otherwise.

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
    /// (PNG, PDF) or only names a container shared by many families (ZIP, OLE).
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

/// A leading-bytes signature. `offset` is where `magic` must sit.
struct Signature {
    magic: &'static [u8],
    offset: usize,
    family: Family,
    label: &'static str,
    definitive: bool,
}

const SIGNATURES: &[Signature] = &[
    Signature { magic: b"\x89PNG\r\n\x1a\n", offset: 0, family: Family::Images, label: "PNG", definitive: true },
    Signature { magic: b"\xff\xd8\xff", offset: 0, family: Family::Images, label: "JPEG", definitive: true },
    Signature { magic: b"GIF87a", offset: 0, family: Family::Images, label: "GIF", definitive: true },
    Signature { magic: b"GIF89a", offset: 0, family: Family::Images, label: "GIF", definitive: true },
    Signature { magic: b"WEBP", offset: 8, family: Family::Images, label: "WebP", definitive: true },
    Signature { magic: b"II*\x00", offset: 0, family: Family::Images, label: "TIFF", definitive: true },
    Signature { magic: b"MM\x00*", offset: 0, family: Family::Images, label: "TIFF", definitive: true },
    Signature { magic: b"8BPS", offset: 0, family: Family::Images, label: "Photoshop", definitive: true },
    Signature { magic: b"\x76\x2f\x31\x01", offset: 0, family: Family::Images, label: "OpenEXR", definitive: true },
    Signature { magic: b"%PDF-", offset: 0, family: Family::PdfXps, label: "PDF", definitive: true },
    Signature { magic: b"7z\xbc\xaf\x27\x1c", offset: 0, family: Family::Archives, label: "7-Zip", definitive: true },
    Signature { magic: b"\x1f\x8b", offset: 0, family: Family::Archives, label: "gzip", definitive: true },
    Signature { magic: b"\xfd7zXZ\x00", offset: 0, family: Family::Archives, label: "xz", definitive: true },
    Signature { magic: b"BZh", offset: 0, family: Family::Archives, label: "bzip2", definitive: true },
    Signature { magic: b"Rar!\x1a\x07", offset: 0, family: Family::Archives, label: "RAR", definitive: true },
    Signature { magic: b"ustar", offset: 257, family: Family::Archives, label: "tar", definitive: true },
    Signature { magic: b"PK\x03\x04", offset: 0, family: Family::Archives, label: "ZIP container", definitive: false },
    Signature { magic: b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", offset: 0, family: Family::WordProcessing, label: "OLE compound document", definitive: false },
    Signature { magic: b"\x78\x9f\x3e\x22", offset: 0, family: Family::Email, label: "TNEF (winmail.dat)", definitive: true },
    Signature { magic: b"ID3", offset: 0, family: Family::Audio, label: "MP3 (ID3)", definitive: true },
    Signature { magic: b"fLaC", offset: 0, family: Family::Audio, label: "FLAC", definitive: true },
    Signature { magic: b"OggS", offset: 0, family: Family::Audio, label: "Ogg", definitive: true },
    Signature { magic: b"WAVE", offset: 8, family: Family::Audio, label: "WAV", definitive: true },
    Signature { magic: b"AVI ", offset: 8, family: Family::Video, label: "AVI", definitive: true },
    Signature { magic: b"ftyp", offset: 4, family: Family::Video, label: "MP4 family", definitive: true },
    Signature { magic: b"\x1a\x45\xdf\xa3", offset: 0, family: Family::Video, label: "Matroska / WebM", definitive: true },
    Signature { magic: b"\x00\x00\x01\xba", offset: 0, family: Family::Video, label: "MPEG program stream", definitive: true },
];

/// The signature at the start of `head`, if any.
pub fn signature(head: &[u8]) -> Option<Content> {
    SIGNATURES.iter().find_map(|s| {
        let end = s.offset + s.magic.len();
        (head.len() >= end && &head[s.offset..end] == s.magic).then_some(Content::Signature {
            family: s.family,
            label: s.label,
            definitive: s.definitive,
        })
    })
}

/// Whether `head` reads as text, and in which encoding. UTF-8 is accepted when every
/// byte decodes (a cut-off final character is forgiven) and no control characters
/// other than tab, newline, carriage return, form feed and escape occur. UTF-16 is
/// recognised by its byte-order mark.
pub fn text_encoding(head: &[u8]) -> Option<TextEncoding> {
    if head.starts_with(&[0xff, 0xfe]) {
        return Some(TextEncoding::Utf16Le);
    }
    if head.starts_with(&[0xfe, 0xff]) {
        return Some(TextEncoding::Utf16Be);
    }
    let (body, bom) = match head.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        Some(rest) => (rest, true),
        None => (head, false),
    };
    let valid = match std::str::from_utf8(body) {
        Ok(text) => text,
        Err(e) => {
            // Only a character cut off by the end of the sample is forgiven.
            if e.error_len().is_some() {
                return None;
            }
            std::str::from_utf8(&body[..e.valid_up_to()]).ok()?
        }
    };
    let clean = valid
        .chars()
        .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r' | '\x0c' | '\x1b'));
    clean.then_some(if bom { TextEncoding::Utf8Bom } else { TextEncoding::Utf8 })
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

    let family = match content {
        Content::Signature { family, definitive: true, .. }
            if by_extension.is_empty() || !by_extension.contains(&family) =>
        {
            Some(family)
        }
        _ => match by_extension {
            [only] => Some(*only),
            [first, ..] => match content {
                Content::Signature { family, .. } if by_extension.contains(&family) => Some(family),
                Content::Text(_) if by_extension.contains(&Family::Text) => Some(Family::Text),
                _ => Some(*first),
            },
            [] => match content {
                Content::Empty | Content::Text(_) => Some(Family::Text),
                Content::Signature { family, .. } => Some(family),
                Content::Binary => None,
            },
        },
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

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    const ZIP: &[u8] = b"PK\x03\x04\x14\x00\x00\x00";
    const NOISE: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0xfe, 0xff, 0x10, 0x80, 0x00, 0x7f];

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
        assert_eq!(detect("ARCHIVE.TAR.GZ", b"").family, Some(Family::Archives));
        assert_eq!(detect("notes.backup.txt", b"").extension.as_deref(), Some("txt"));
        assert_eq!(detect("C:\\dir.v2\\report.PDF", b"").family, Some(Family::PdfXps));
        assert_eq!(detect("/tmp/some.dir/photo.jpeg", b"").family, Some(Family::Images));
        assert_eq!(extension_of("Makefile"), None);
        assert_eq!(extension_of("trailing."), None);
        assert_eq!(extension_of(".bashrc").as_deref(), Some("bashrc"));
        assert!(EXTENSIONS.windows(2).all(|w| w[0].0 < w[1].0), "table is sorted");
    }

    #[test]
    fn a_definitive_signature_overrides_a_misleading_extension() {
        let d = detect("photo.txt", PNG);
        assert_eq!(d.by_extension, &[Family::Text]);
        assert_eq!(d.content, Content::Signature { family: Family::Images, label: "PNG", definitive: true });
        assert_eq!(d.family, Some(Family::Images));
        assert_eq!(d.route.id(), "images");
        // A container signature does not: a docx is a ZIP, and stays a document.
        let d = detect("report.docx", ZIP);
        assert_eq!(d.content, Content::Signature { family: Family::Archives, label: "ZIP container", definitive: false });
        assert_eq!(d.family, Some(Family::WordProcessing));
        // Text content does not override a known extension either.
        assert_eq!(detect("notes.pdf", b"just words\n").family, Some(Family::PdfXps));
        // A signature of the extension's own family changes nothing.
        assert_eq!(detect("photo.png", PNG).family, Some(Family::Images));
        // A truncated signature is not a signature.
        assert_eq!(detect("photo.txt", &PNG[..4]).family, Some(Family::Text));
    }

    #[test]
    fn unknown_extension_is_settled_by_content() {
        assert_eq!(detect("README", b"Sfoglia\n").family, Some(Family::Text));
        assert_eq!(detect("blob.xyz", NOISE).family, None);
        assert_eq!(detect("blob.xyz", NOISE).route, Route::Hex);
        assert_eq!(detect("a.unknown", PNG).family, Some(Family::Images));
        assert_eq!(detect("x.q", ZIP).family, Some(Family::Archives));
        assert_eq!(detect("x.q", b"%PDF-1.7\n").family, Some(Family::PdfXps));
        assert_eq!(detect("x.q", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1rest").family, Some(Family::WordProcessing));
    }

    #[test]
    fn shared_extensions_are_settled_by_content_then_contract_order() {
        assert_eq!(families_for_extension("dat"), &[Family::Email, Family::Video]);
        assert_eq!(detect("winmail.dat", b"\x78\x9f\x3e\x22\x01\x00").family, Some(Family::Email));
        assert_eq!(detect("movie.dat", b"\x00\x00\x01\xba\x44\x00").family, Some(Family::Video));
        assert_eq!(detect("x.dat", b"").family, Some(Family::Email));
        assert_eq!(detect("x.dat", NOISE).family, Some(Family::Email));
        assert_eq!(families_for_extension("mpc"), &[Family::Audio, Family::Video]);
        assert_eq!(detect("x.mpc", b"").family, Some(Family::Audio));
        assert_eq!(families_for_extension("cin"), &[Family::Images, Family::Video]);
        assert_eq!(detect("x.cin", b"").family, Some(Family::Images));
        assert_eq!(families_for_extension("nothing-known"), &[]);
    }

    #[test]
    fn empty_and_text_files_go_to_text_and_unknown_binary_to_hex() {
        let d = detect("empty", b"");
        assert_eq!((d.content, d.family, d.route.id()), (Content::Empty, Some(Family::Text), "text"));
        assert_eq!(detect("x.unknownext", b"").family, Some(Family::Text));
        // A known extension keeps its family even when the file is empty.
        assert_eq!(detect("x.png", b"").family, Some(Family::Images));
        let d = detect("a.bin", NOISE);
        assert_eq!((d.content, d.family, d.route, d.route.clause()), (Content::Binary, None, Route::Hex, "P-005"));
        assert_eq!(text_encoding(b"\xef\xbb\xbfhi"), Some(TextEncoding::Utf8Bom));
        assert_eq!(text_encoding(b"\xff\xfeh\x00i\x00"), Some(TextEncoding::Utf16Le));
        assert_eq!(text_encoding(b"\xfe\xff\x00h\x00i"), Some(TextEncoding::Utf16Be));
        assert_eq!(text_encoding("caffè\tlatte\r\n".as_bytes()), Some(TextEncoding::Utf8));
        assert_eq!(text_encoding(b"abc\x00def"), None);
        assert_eq!(text_encoding(b"\x1b[31mred\x1b[0m\n"), Some(TextEncoding::Utf8));
        // A multi-byte character cut by the end of the sample is still text.
        let cut = &"héllo wörld".as_bytes()[..7];
        assert_eq!(text_encoding(cut), Some(TextEncoding::Utf8));
        assert_eq!(text_encoding(b"\xc3\x28"), None);
        assert_eq!(detect("notes.txt", b"\xff\xfeh\x00i\x00").content, Content::Text(TextEncoding::Utf16Le));
        assert_eq!(sniff(b""), Content::Empty);
    }
}
