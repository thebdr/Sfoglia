//! Archives, read side (C-015): what container a file is and through which codec, the list of its entries,
//! one entry's bytes, and extraction into a folder. Containers (zip, 7z, tar, a lone compressed file, or a
//! format left to an installed 7-Zip) and codecs (gzip, bzip2, xz, lzma, zstd) are separate kinds: a tar
//! through gzip and a lone gzip file go through the same `decode`, so a codec of the user's own plugs in there.

use std::fs::File;
use std::io::{self, BufReader, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A compression codec, applied to a tar stream or to a lone file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    Gzip,
    Bzip2,
    Xz,
    Lzma,
    Zstd,
    /// Unix `compress` (`.Z`): no library here reads it, 7-Zip does.
    Compress,
}

impl Codec {
    pub fn label(self) -> &'static str {
        match self {
            Codec::Gzip => "gzip",
            Codec::Bzip2 => "bzip2",
            Codec::Xz => "xz",
            Codec::Lzma => "lzma",
            Codec::Zstd => "zstd",
            Codec::Compress => "compress",
        }
    }

    /// The suffix the codec puts on a file name.
    fn suffixes(self) -> &'static [&'static str] {
        match self {
            Codec::Gzip => &["gz", "gzip"],
            Codec::Bzip2 => &["bz2", "bzip2"],
            Codec::Xz => &["xz"],
            Codec::Lzma => &["lzma"],
            Codec::Zstd => &["zst", "zstd"],
            Codec::Compress => &["z"],
        }
    }

    fn by_suffix(suffix: &str) -> Option<Codec> {
        [Codec::Gzip, Codec::Bzip2, Codec::Xz, Codec::Lzma, Codec::Zstd, Codec::Compress]
            .into_iter()
            .find(|codec| codec.suffixes().contains(&suffix))
    }

    /// The codec a stream's first bytes announce (lzma has no magic; `compress` is `1f 9d`).
    fn by_signature(head: &[u8]) -> Option<Codec> {
        if head.starts_with(b"\x1f\x8b\x08") {
            Some(Codec::Gzip)
        } else if head.starts_with(b"BZh") && head.get(4..10) == Some(b"1AY&SY") {
            Some(Codec::Bzip2)
        } else if head.starts_with(b"\xfd7zXZ\x00") {
            Some(Codec::Xz)
        } else if head.starts_with(b"\x28\xb5\x2f\xfd") {
            Some(Codec::Zstd)
        } else if head.starts_with(b"\x1f\x9d") {
            Some(Codec::Compress)
        } else {
            None
        }
    }
}

/// What holds the entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Zip,
    /// A tar, plain or through a codec.
    Tar(Option<Codec>),
    SevenZ,
    /// One compressed file: a one-entry archive named after the file without its suffix.
    Lone(Codec),
    /// rar and the formats no library here reads: an installed 7-Zip lists and extracts them.
    External,
}

impl Container {
    /// `ZIP`, `TAR.GZ`, `7Z`, `GZ`, or the extension upper-cased for an external format.
    pub fn label(self, name: &str) -> String {
        match self {
            Container::Zip => "ZIP".to_string(),
            Container::SevenZ => "7Z".to_string(),
            Container::Tar(None) => "TAR".to_string(),
            Container::Tar(Some(codec)) => format!("TAR.{}", codec.suffixes()[0].to_ascii_uppercase()),
            Container::Lone(codec) => codec.suffixes()[0].to_ascii_uppercase(),
            Container::External => extension(name).to_ascii_uppercase(),
        }
    }
}

/// The lower-cased last extension of a name, or an empty string.
fn extension(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Whether the name says the compressed stream holds a tar: `x.tar.gz`, `x.tgz`, `x.tbz2`, `x.taz`, …
fn names_a_tar(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = Path::new(&lower).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    stem.ends_with(".tar") || matches!(extension(&lower).as_str(), "tgz" | "tbz" | "tbz2" | "txz" | "taz" | "tzst" | "tlz")
}

/// The container of a file, by its first bytes first (a `.zip` that holds a 7z opens as a 7z) and by its
/// name otherwise; `None` for a name outside the family.
pub fn container_of(name: &str, head: &[u8]) -> Option<Container> {
    if head.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return Some(Container::SevenZ);
    }
    if head.starts_with(b"Rar!\x1a\x07") {
        return Some(Container::External);
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Some(Container::Zip);
    }
    if head.len() > 262 && (&head[257..262] == b"ustar") {
        return Some(Container::Tar(None));
    }
    if let Some(codec) = Codec::by_signature(head) {
        if codec == Codec::Compress {
            return Some(Container::External);
        }
        return Some(if names_a_tar(name) { Container::Tar(Some(codec)) } else { Container::Lone(codec) });
    }
    let ext = extension(name);
    Some(match ext.as_str() {
        "zip" | "zipx" | "jar" | "war" | "apk" | "cbz" | "pk3" | "pk4" | "pkz" => Container::Zip,
        "7z" => Container::SevenZ,
        "tar" => Container::Tar(None),
        "tgz" | "taz" => Container::Tar(Some(Codec::Gzip)),
        "tbz" | "tbz2" => Container::Tar(Some(Codec::Bzip2)),
        "txz" => Container::Tar(Some(Codec::Xz)),
        "tzst" => Container::Tar(Some(Codec::Zstd)),
        "tlz" => Container::Tar(Some(Codec::Lzma)),
        "z" => Container::External,
        "rar" | "cbr" | "cab" | "iso" | "rpm" | "deb" | "cpio" | "lha" | "lzh" | "arj" | "chm" | "xar" | "wim"
        | "dmg" | "vhd" | "hfs" | "dd" | "deskthemepack" => Container::External,
        other => match Codec::by_suffix(other) {
            Some(Codec::Compress) => Container::External,
            Some(codec) => {
                if names_a_tar(name) { Container::Tar(Some(codec)) } else { Container::Lone(codec) }
            }
            None => return None,
        },
    })
}

/// One entry as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The path inside the archive, `/`-separated.
    pub name: String,
    pub size: u64,
    pub packed: Option<u64>,
    /// `YYYY-MM-DD HH:MM`, when the archive records one.
    pub modified: Option<String>,
    pub method: String,
    pub crc: Option<u32>,
    pub is_dir: bool,
    /// The entry's index inside the container, for the containers read by index; none for a folder the listing
    /// fills in because the archive records no entry for it.
    pub raw: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub container: Container,
    pub entries: Vec<Entry>,
    /// Some entries need a password to read.
    pub encrypted: bool,
}

impl Listing {
    pub fn unpacked_total(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }

    /// The packed sizes summed, when the container records them per entry.
    pub fn packed_total(&self) -> Option<u64> {
        let known: Vec<u64> = self.entries.iter().filter(|e| !e.is_dir).filter_map(|e| e.packed).collect();
        (!known.is_empty()).then(|| known.iter().sum())
    }

    /// `ZIP, 12 entries, 1.2 MB packed from 4.5 MB`.
    pub fn describe(&self, name: &str, file_size: Option<u64>) -> String {
        let n = self.entries.len();
        let noun = if n == 1 { "entry" } else { "entries" };
        let packed = self.packed_total().or(file_size);
        match packed {
            Some(packed) => {
                format!("{}, {n} {noun}, {} packed from {}", self.container.label(name), human(packed), human(self.unpacked_total()))
            }
            None => format!("{}, {n} {noun}, {}", self.container.label(name), human(self.unpacked_total())),
        }
    }
}

#[derive(Debug)]
pub enum ArchiveError {
    /// Encrypted, and no password was given.
    NeedsPassword,
    /// Encrypted, and the password given is wrong.
    WrongPassword,
    /// The format needs an installed 7-Zip and none serves.
    NoTool,
    /// The file could not be read as its container.
    Bad(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::NeedsPassword => write!(f, "the archive is encrypted: it needs its password"),
            ArchiveError::WrongPassword => write!(f, "wrong password"),
            ArchiveError::NoTool => write!(f, "this format needs 7-Zip"),
            ArchiveError::Bad(reason) => write!(f, "{reason}"),
        }
    }
}

impl From<io::Error> for ArchiveError {
    fn from(error: io::Error) -> Self {
        ArchiveError::Bad(error.to_string())
    }
}

impl From<zip::result::ZipError> for ArchiveError {
    fn from(error: zip::result::ZipError) -> Self {
        use zip::result::ZipError;
        match error {
            ZipError::InvalidPassword => ArchiveError::WrongPassword,
            ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => ArchiveError::NeedsPassword,
            other => ArchiveError::Bad(other.to_string()),
        }
    }
}

impl From<sevenz_rust2::Error> for ArchiveError {
    fn from(error: sevenz_rust2::Error) -> Self {
        use sevenz_rust2::Error;
        match error {
            Error::PasswordRequired => ArchiveError::NeedsPassword,
            Error::MaybeBadPassword(_) | Error::ChecksumVerificationFailed => ArchiveError::WrongPassword,
            other => ArchiveError::Bad(other.to_string()),
        }
    }
}

/// The codec path: the decoded stream over `reader`. A tar and a lone file both pass through here.
pub fn decode(codec: Codec, reader: Box<dyn Read>) -> Result<Box<dyn Read>, ArchiveError> {
    Ok(match codec {
        Codec::Gzip => Box::new(flate2::read::MultiGzDecoder::new(reader)),
        Codec::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(reader)),
        Codec::Zstd => Box::new(zstd::stream::read::Decoder::new(reader)?),
        Codec::Xz => {
            let mut out = Vec::new();
            lzma_rs::xz_decompress(&mut BufReader::new(reader), &mut out).map_err(|e| ArchiveError::Bad(e.to_string()))?;
            Box::new(Cursor::new(out))
        }
        Codec::Lzma => {
            let mut out = Vec::new();
            lzma_rs::lzma_decompress(&mut BufReader::new(reader), &mut out).map_err(|e| ArchiveError::Bad(e.to_string()))?;
            Box::new(Cursor::new(out))
        }
        Codec::Compress => return Err(ArchiveError::NoTool),
    })
}

/// The 7-Zip executable to use for the external formats, if any; set by the viewer from the tool set.
pub struct SevenZipTool(pub PathBuf);

/// What a lone file's one entry is called: the name without the codec's suffix (`note.txt.gz` → `note.txt`).
pub fn lone_entry_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for codec in [Codec::Gzip, Codec::Bzip2, Codec::Xz, Codec::Lzma, Codec::Zstd, Codec::Compress] {
        for suffix in codec.suffixes() {
            let dotted = format!(".{suffix}");
            if lower.ends_with(&dotted) && lower.len() > dotted.len() {
                return name[..name.len() - dotted.len()].to_string();
            }
        }
    }
    name.to_string()
}

/// List the entries of `path`; `tool` is the installed 7-Zip for the external formats.
pub fn list(path: &Path, password: Option<&str>, tool: Option<&SevenZipTool>) -> Result<Listing, ArchiveError> {
    let name = file_name(path);
    let head = head_of(path)?;
    let container = container_of(&name, &head).ok_or_else(|| ArchiveError::Bad(format!("{name} is no archive")))?;
    let entries = match container {
        Container::Zip => list_zip(path)?,
        Container::Tar(codec) => list_tar(path, codec)?,
        Container::SevenZ => list_seven_z(path, password)?,
        Container::Lone(codec) => vec![lone_entry(path, &name, codec)?],
        Container::External => {
            let tool = tool.ok_or(ArchiveError::NoTool)?;
            list_external(path, password, tool)?
        }
    };
    let entries = fill_in_folders(entries);
    let encrypted = match container {
        Container::Zip => zip_has_encrypted(path)?,
        Container::SevenZ => seven_z_wants_password(path),
        _ => false,
    };
    // A password given is checked now, on the smallest encrypted entry, so a wrong one is said at once.
    if let Some(password) = password
        && encrypted
    {
        verify_password(path, container, &entries, password)?;
    }
    Ok(Listing { container, entries, encrypted })
}

/// A folder the archive records no entry for (7-Zip and others store the files alone) gets a row of its own,
/// before its first child; a folder recorded after its children (as 7-Zip lists them) stays where it is.
fn fill_in_folders(entries: Vec<Entry>) -> Vec<Entry> {
    let mut known: std::collections::HashSet<String> = entries.iter().filter(|e| e.is_dir).map(|e| e.name.clone()).collect();
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = entry.name.trim_end_matches('/');
        let mut start = 0;
        for (at, ch) in path.char_indices() {
            if ch == '/' {
                let component = &path[start..at];
                start = at + 1;
                // A `.` or `..` is no folder of the archive's (such an entry is skipped on extraction anyway).
                if component.is_empty() || component == "." || component == ".." {
                    continue;
                }
                let folder = format!("{}/", &path[..at]);
                if known.insert(folder.clone()) {
                    out.push(Entry { name: folder, size: 0, packed: None, modified: None, method: String::new(), crc: None, is_dir: true, raw: None });
                }
            }
        }
        out.push(entry);
    }
    out
}

/// Whether entry `raw` of a 7z sits in a block with an AES coder.
fn seven_z_entry_encrypted(archive: &sevenz_rust2::Archive, raw: usize) -> bool {
    let Some(Some(block)) = archive.stream_map.file_block_index.get(raw) else { return false };
    archive.blocks.get(*block).is_some_and(|block| {
        block.coders.iter().any(|coder| coder.encoder_method_id() == sevenz_rust2::EncoderMethod::AES256_SHA256.id())
    })
}

/// Check `password` on the smallest encrypted entry: a zip through `by_index_decrypt` (ZipCrypto's check byte
/// or AES's verifier refuse a wrong one, a checksum catches the rest), a 7z through its reader. Nothing
/// encrypted to try it on passes.
fn verify_password(path: &Path, container: Container, entries: &[Entry], password: &str) -> Result<(), ArchiveError> {
    match container {
        Container::Zip => {
            let mut archive = zip::ZipArchive::new(File::open(path)?)?;
            let mut smallest: Option<(u64, usize)> = None;
            for i in 0..archive.len() {
                let file = archive.by_index_raw(i)?;
                if file.encrypted() && !file.is_dir() && smallest.is_none_or(|(size, _)| file.size() < size) {
                    smallest = Some((file.size(), i));
                }
            }
            if let Some((_, i)) = smallest {
                let mut sink = Vec::new();
                let read = archive.by_index_decrypt(i, password.as_bytes()).and_then(|mut file| file.read_to_end(&mut sink).map_err(Into::into));
                if read.is_err() {
                    return Err(ArchiveError::WrongPassword);
                }
            }
            Ok(())
        }
        Container::SevenZ => {
            let mut reader = seven_z_reader(path, Some(password))?;
            let smallest = entries
                .iter()
                .filter(|e| !e.is_dir)
                .filter(|e| e.raw.is_some_and(|raw| seven_z_entry_encrypted(reader.archive(), raw)))
                .min_by_key(|e| e.size)
                .map(|e| e.name.trim_end_matches('/').to_string());
            if let Some(name) = smallest && reader.read_file(&name).is_err() {
                return Err(ArchiveError::WrongPassword);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The bytes of entry `index` of the listing.
pub fn read_entry(path: &Path, listing: &Listing, index: usize, password: Option<&str>, tool: Option<&SevenZipTool>) -> Result<Vec<u8>, ArchiveError> {
    let entry = listing.entries.get(index).ok_or_else(|| ArchiveError::Bad(format!("no entry {index}")))?;
    if entry.is_dir {
        return Err(ArchiveError::Bad(format!("{} is a folder", entry.name)));
    }
    let raw = || entry.raw.ok_or_else(|| ArchiveError::Bad(format!("{} has no entry of its own", entry.name)));
    match listing.container {
        Container::Zip => {
            let mut archive = zip::ZipArchive::new(File::open(path)?)?;
            let mut out = Vec::new();
            match password {
                Some(password) => archive.by_index_decrypt(raw()?, password.as_bytes())?.read_to_end(&mut out)?,
                None => archive.by_index(raw()?)?.read_to_end(&mut out)?,
            };
            Ok(out)
        }
        Container::Tar(codec) => {
            let wanted = raw()?;
            let mut archive = tar::Archive::new(tar_stream(path, codec)?);
            for (i, member) in archive.entries()?.enumerate() {
                let mut member = member?;
                if i == wanted {
                    let mut out = Vec::new();
                    member.read_to_end(&mut out)?;
                    return Ok(out);
                }
            }
            Err(ArchiveError::Bad(format!("no entry {index}")))
        }
        Container::SevenZ => {
            let mut reader = seven_z_reader(path, password)?;
            let encrypted = seven_z_entry_encrypted(reader.archive(), raw()?);
            if encrypted && password.is_none() {
                return Err(ArchiveError::NeedsPassword);
            }
            match reader.read_file(&entry.name) {
                Ok(bytes) => Ok(bytes),
                // A wrong password makes the stream garbage: whatever the decoder then says, that is the reason.
                Err(_) if encrypted => Err(ArchiveError::WrongPassword),
                Err(error) => Err(error.into()),
            }
        }
        Container::Lone(codec) => {
            let mut out = Vec::new();
            decode(codec, Box::new(BufReader::new(File::open(path)?)))?.read_to_end(&mut out)?;
            Ok(out)
        }
        Container::External => {
            let tool = tool.ok_or(ArchiveError::NoTool)?;
            let mut command = Command::new(&tool.0);
            // `--` ends the switches, so a name starting with `-` or `@` is a name; `-spd` keeps it from being a pattern.
            command.args(["e", "-so", "-y", "-sccUTF-8", "-spd"]).arg(password_flag(password)).arg("--").arg(path).arg(&entry.name);
            command.stdin(Stdio::null());
            let output = command.output()?;
            if !output.status.success() {
                return Err(external_error(&output.stderr, &output.stdout, password.is_some()));
            }
            Ok(output.stdout)
        }
    }
}

/// What an extraction did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Extracted {
    pub written: Vec<PathBuf>,
    /// Entries whose path would have climbed out of the folder, left unwritten.
    pub skipped: Vec<String>,
}

/// Extract the entries `indices` (all when `None`) into `into`, never outside it.
pub fn extract(
    path: &Path,
    listing: &Listing,
    indices: Option<&[usize]>,
    into: &Path,
    password: Option<&str>,
    tool: Option<&SevenZipTool>,
) -> Result<Extracted, ArchiveError> {
    std::fs::create_dir_all(into)?;
    let wanted = |i: usize| indices.is_none_or(|chosen| chosen.contains(&i));
    let mut done = Extracted::default();
    // Where an entry goes, or its name on the skipped list when its path climbs out.
    let place = |done: &mut Extracted, entry: &Entry| -> Option<PathBuf> {
        match safe_join(into, &entry.name) {
            Some(target) => Some(target),
            None => {
                done.skipped.push(entry.name.clone());
                None
            }
        }
    };
    let make_folder = |done: &mut Extracted, target: PathBuf| -> Result<(), ArchiveError> {
        std::fs::create_dir_all(&target)?;
        done.written.push(target);
        Ok(())
    };
    match listing.container {
        Container::External => {
            let tool = tool.ok_or(ArchiveError::NoTool)?;
            // The entries go to 7-Zip by name through an include list (`-i@`, UTF-8, no wildcards): every wanted
            // one that stays inside the folder. A name 7-Zip would read as a switch or a list file is harmless
            // there, and a climbing entry is never asked for (7-Zip itself would write it inside with the `..`
            // stripped, which is not what the status row would say).
            let mut names = Vec::new();
            for (i, entry) in listing.entries.iter().enumerate() {
                if wanted(i) && place(&mut done, entry).is_some() {
                    names.push(entry.name.trim_end_matches('/').to_string());
                }
            }
            if names.is_empty() {
                return Ok(done);
            }
            let scratch = TempFolder::new()?;
            let list = scratch.path().join("entries.txt");
            std::fs::write(&list, names.join("
") + "
")?;
            let mut command = Command::new(&tool.0);
            command
                .arg("x")
                .arg(format!("-o{}", into.display()))
                .args(["-y", "-sccUTF-8", "-scsUTF-8", "-spd"])
                .arg(format!("-i@{}", list.display()))
                .arg(password_flag(password))
                .arg("--")
                .arg(path);
            command.stdin(Stdio::null());
            let output = command.output()?;
            if !output.status.success() {
                return Err(external_error(&output.stderr, &output.stdout, password.is_some()));
            }
            for (i, entry) in listing.entries.iter().enumerate() {
                if wanted(i) && let Some(target) = safe_join(into, &entry.name) && target.exists() {
                    done.written.push(target);
                }
            }
        }
        Container::Tar(codec) => {
            // The folders the listing filled in first, then the stream, each member matched to its row.
            let mut by_raw = std::collections::HashMap::new();
            for (i, entry) in listing.entries.iter().enumerate() {
                match entry.raw {
                    Some(raw) => {
                        by_raw.insert(raw, i);
                    }
                    None if wanted(i) => {
                        if let Some(target) = place(&mut done, entry) {
                            make_folder(&mut done, target)?;
                        }
                    }
                    None => {}
                }
            }
            let mut archive = tar::Archive::new(tar_stream(path, codec)?);
            for (raw, member) in archive.entries()?.enumerate() {
                let mut member = member?;
                let Some(&i) = by_raw.get(&raw) else { continue };
                if !wanted(i) {
                    continue;
                }
                let entry = &listing.entries[i];
                let Some(target) = place(&mut done, entry) else { continue };
                if entry.is_dir {
                    make_folder(&mut done, target)?;
                } else {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut file = File::create(&target)?;
                    io::copy(&mut member, &mut file)?;
                    done.written.push(target);
                }
            }
        }
        _ => {
            for (i, entry) in listing.entries.iter().enumerate() {
                if !wanted(i) {
                    continue;
                }
                let Some(target) = place(&mut done, entry) else { continue };
                if entry.is_dir {
                    make_folder(&mut done, target)?;
                } else {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = read_entry(path, listing, i, password, tool)?;
                    std::fs::write(&target, bytes)?;
                    done.written.push(target);
                }
            }
        }
    }
    Ok(done)
}

/// `root/name` when `name` stays inside `root`: no absolute path, no `..` component, and on Windows no `:` (a drive;
/// elsewhere a colon is just a character of a name).
pub fn safe_join(root: &Path, name: &str) -> Option<PathBuf> {
    let unified = name.replace('\\', "/");
    if unified.starts_with('/') || (cfg!(windows) && unified.contains(':')) {
        return None;
    }
    let mut target = root.to_path_buf();
    for part in unified.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            part => target.push(part),
        }
    }
    (target != root).then_some(target)
}

/// `1.2 MB`, `640 KB`, `12 B`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Unix seconds as `YYYY-MM-DD HH:MM` (UTC).
pub fn format_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rest / 3600, (rest % 3600) / 60)
}

/// Howard Hinnant's days-to-civil, for the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn head_of(path: &Path) -> io::Result<Vec<u8>> {
    let mut head = Vec::new();
    File::open(path)?.take(512).read_to_end(&mut head)?;
    Ok(head)
}

fn method_label(method: zip::CompressionMethod) -> String {
    use zip::CompressionMethod;
    match method {
        CompressionMethod::Stored => "stored".to_string(),
        CompressionMethod::Deflated => "deflate".to_string(),
        CompressionMethod::Bzip2 => "bzip2".to_string(),
        CompressionMethod::Zstd => "zstd".to_string(),
        CompressionMethod::Lzma => "lzma".to_string(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

fn list_zip(path: &Path) -> Result<Vec<Entry>, ArchiveError> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let file = archive.by_index_raw(i)?;
        let name = file.name()?.into_owned();
        let modified = file.last_modified().map(|d| {
            format!("{:04}-{:02}-{:02} {:02}:{:02}", d.year(), d.month(), d.day(), d.hour(), d.minute())
        });
        let is_dir = file.is_dir();
        entries.push(Entry {
            name: name.trim_end_matches('/').to_string() + if is_dir { "/" } else { "" },
            size: file.size(),
            packed: Some(file.compressed_size()),
            modified,
            method: if is_dir { String::new() } else { method_label(file.compression()) },
            crc: (!is_dir).then_some(file.crc32()),
            is_dir,
            raw: Some(i),
        });
    }
    Ok(entries)
}

fn zip_has_encrypted(path: &Path) -> Result<bool, ArchiveError> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    for i in 0..archive.len() {
        if archive.by_index_raw(i)?.encrypted() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn tar_stream(path: &Path, codec: Option<Codec>) -> Result<Box<dyn Read>, ArchiveError> {
    let file: Box<dyn Read> = Box::new(BufReader::new(File::open(path)?));
    match codec {
        Some(codec) => decode(codec, file),
        None => Ok(file),
    }
}

fn list_tar(path: &Path, codec: Option<Codec>) -> Result<Vec<Entry>, ArchiveError> {
    let mut archive = tar::Archive::new(tar_stream(path, codec)?);
    let method = codec.map_or("stored".to_string(), |c| c.label().to_string());
    let mut entries = Vec::new();
    for (i, member) in archive.entries()?.enumerate() {
        let member = member?;
        let header = member.header();
        let is_dir = header.entry_type().is_dir();
        let mut name = member.path()?.to_string_lossy().replace('\\', "/");
        if is_dir && !name.ends_with('/') {
            name.push('/');
        }
        entries.push(Entry {
            name,
            size: header.size().unwrap_or(0),
            packed: None,
            modified: header.mtime().ok().map(format_unix),
            method: if is_dir { String::new() } else { method.clone() },
            crc: None,
            is_dir,
            raw: Some(i),
        });
    }
    Ok(entries)
}

fn seven_z_reader(path: &Path, password: Option<&str>) -> Result<sevenz_rust2::ArchiveReader<File>, ArchiveError> {
    let password = match password {
        Some(password) => sevenz_rust2::Password::from(password),
        None => sevenz_rust2::Password::empty(),
    };
    Ok(sevenz_rust2::ArchiveReader::new(File::open(path)?, password)?)
}

/// Whether the 7z asks for a password (its headers or its streams encrypted).
fn seven_z_wants_password(path: &Path) -> bool {
    match seven_z_reader(path, None) {
        Err(ArchiveError::NeedsPassword) => true,
        Ok(reader) => reader.archive().blocks.iter().any(|block| {
            block.coders.iter().any(|coder| coder.encoder_method_id() == sevenz_rust2::EncoderMethod::AES256_SHA256.id())
        }),
        Err(_) => false,
    }
}

fn seven_z_method(archive: &sevenz_rust2::Archive, index: usize) -> String {
    let Some(Some(block)) = archive.stream_map.file_block_index.get(index) else { return String::new() };
    let Some(block) = archive.blocks.get(*block) else { return String::new() };
    let names: Vec<&str> = block
        .coders
        .iter()
        .map(|coder| match coder.encoder_method_id() {
            id if id == sevenz_rust2::EncoderMethod::LZMA2.id() => "LZMA2",
            id if id == sevenz_rust2::EncoderMethod::LZMA.id() => "LZMA",
            id if id == sevenz_rust2::EncoderMethod::BCJ_X86_FILTER.id() => "BCJ",
            id if id == sevenz_rust2::EncoderMethod::AES256_SHA256.id() => "AES",
            id if id == sevenz_rust2::EncoderMethod::COPY.id() => "stored",
            _ => "?",
        })
        .collect();
    // The coders are stored from the outside in; 7-Zip prints them the other way (`BCJ LZMA`, `LZMA2 7zAES`).
    let mut names = names;
    names.reverse();
    names.join("+")
}

fn list_seven_z(path: &Path, password: Option<&str>) -> Result<Vec<Entry>, ArchiveError> {
    let reader = seven_z_reader(path, password)?;
    let archive = reader.archive();
    let mut entries = Vec::new();
    for (i, entry) in archive.files.iter().enumerate() {
        let is_dir = entry.is_directory;
        let modified = entry.has_last_modified_date.then(|| {
            let system: std::time::SystemTime = entry.last_modified_date.into();
            format_unix(system.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
        });
        let mut name = entry.name().replace('\\', "/");
        if is_dir && !name.ends_with('/') {
            name.push('/');
        }
        entries.push(Entry {
            name,
            size: entry.size,
            packed: (entry.compressed_size > 0).then_some(entry.compressed_size),
            modified,
            method: if is_dir { String::new() } else { seven_z_method(archive, i) },
            crc: (entry.has_crc && !is_dir).then_some(entry.crc as u32),
            is_dir,
            raw: Some(i),
        });
    }
    Ok(entries)
}

fn lone_entry(path: &Path, name: &str, codec: Codec) -> Result<Entry, ArchiveError> {
    let packed = std::fs::metadata(path)?.len();
    let mut out = Vec::new();
    decode(codec, Box::new(BufReader::new(File::open(path)?)))?.read_to_end(&mut out)?;
    let modified = (codec == Codec::Gzip)
        .then(|| {
            let decoder = flate2::read::GzDecoder::new(BufReader::new(File::open(path).ok()?));
            decoder.header().map(|h| h.mtime()).filter(|m| *m > 0).map(|m| format_unix(m as u64))
        })
        .flatten();
    Ok(Entry {
        name: lone_entry_name(name),
        size: out.len() as u64,
        packed: Some(packed),
        modified,
        method: codec.label().to_string(),
        crc: None,
        is_dir: false,
        raw: Some(0),
    })
}

fn password_flag(password: Option<&str>) -> String {
    // 7-Zip asks on the console for a password it is not given, and `-p` alone is the empty one (the real
    // tool then says "Cannot open encrypted archive. Wrong password?" instead of waiting).
    format!("-p{}", password.unwrap_or(""))
}

fn external_error(stderr: &[u8], stdout: &[u8], had_password: bool) -> ArchiveError {
    let text = format!("{}{}", String::from_utf8_lossy(stderr), String::from_utf8_lossy(stdout));
    // `Cannot open encrypted archive. Wrong password?`, `Wrong password?` on an entry, `Enter password`: the
    // one given is wrong, or none was given.
    if text.to_lowercase().contains("password") || text.contains("encrypted") {
        if had_password { ArchiveError::WrongPassword } else { ArchiveError::NeedsPassword }
    } else {
        let line = text.lines().map(str::trim).find(|l| l.starts_with("ERROR") || l.contains("Error") || !l.is_empty()).unwrap_or("7-Zip failed");
        ArchiveError::Bad(format!("7-Zip: {line}"))
    }
}

fn list_external(path: &Path, password: Option<&str>, tool: &SevenZipTool) -> Result<Vec<Entry>, ArchiveError> {
    let mut command = Command::new(&tool.0);
    command.args(["l", "-slt", "-y", "-sccUTF-8"]).arg(password_flag(password)).arg("--").arg(path);
    command.stdin(Stdio::null());
    let output = command.output()?;
    if !output.status.success() {
        return Err(external_error(&output.stderr, &output.stdout, password.is_some()));
    }
    parse_seven_zip_listing(&String::from_utf8_lossy(&output.stdout)).map_err(ArchiveError::Bad)
}

/// `7z l -slt` output: after the `----------` line, one block per entry with `Path = `, `Size = `,
/// `Packed Size = `, `Modified = 2026-10-18 12:00:00`, `Attributes = D....` (or `Folder = +`), `CRC = `,
/// `Method = `; blank lines between blocks.
pub fn parse_seven_zip_listing(text: &str) -> Result<Vec<Entry>, String> {
    let body = match text.find("\n----------") {
        Some(at) => &text[at..],
        None => return Err("7-Zip listed no entries".to_string()),
    };
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    let finish = |current: &mut Option<Entry>, entries: &mut Vec<Entry>| {
        if let Some(mut entry) = current.take() {
            if entry.is_dir && !entry.name.ends_with('/') {
                entry.name.push('/');
            }
            if entry.is_dir {
                entry.method.clear();
                entry.crc = None;
            }
            entries.push(entry);
        }
    };
    for line in body.lines().skip(1) {
        let line = line.trim_end();
        if line.is_empty() {
            finish(&mut current, &mut entries);
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else { continue };
        let value = value.trim();
        match key.trim() {
            "Path" => {
                finish(&mut current, &mut entries);
                current = Some(Entry {
                    name: value.replace('\\', "/"),
                    size: 0,
                    packed: None,
                    modified: None,
                    method: String::new(),
                    crc: None,
                    is_dir: false,
                    raw: None,
                });
            }
            "Size" => {
                if let Some(entry) = current.as_mut() {
                    entry.size = value.parse().unwrap_or(0);
                }
            }
            "Packed Size" => {
                if let Some(entry) = current.as_mut() {
                    entry.packed = value.parse().ok();
                }
            }
            "Modified" => {
                if let Some(entry) = current.as_mut() {
                    entry.modified = (value.len() >= 16).then(|| value[..16].to_string());
                }
            }
            "Attributes" => {
                if let Some(entry) = current.as_mut() {
                    entry.is_dir = entry.is_dir || value.starts_with('D');
                }
            }
            "Folder" => {
                if let Some(entry) = current.as_mut() {
                    entry.is_dir = entry.is_dir || value == "+";
                }
            }
            "CRC" => {
                if let Some(entry) = current.as_mut() {
                    entry.crc = u32::from_str_radix(value, 16).ok();
                }
            }
            "Method" => {
                if let Some(entry) = current.as_mut() {
                    entry.method = value.to_string();
                }
            }
            _ => {}
        }
    }
    finish(&mut current, &mut entries);
    if entries.is_empty() { Err("7-Zip listed no entries".to_string()) } else { Ok(entries) }
}

/// A folder under the system's temporary folder, removed when dropped: the home of an entry opened inside
/// Sfoglia, which goes with its tab.
#[derive(Debug, PartialEq, Eq)]
pub struct TempFolder(PathBuf);

impl TempFolder {
    /// A fresh folder of its own under `<temp>/sfoglia`.
    pub fn new() -> io::Result<TempFolder> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join("sfoglia").join(format!("entry-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(TempFolder(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write entry `index` into a folder of its own, under its own file name, for a tab to open: the folder,
/// and the file in it.
pub fn materialize(
    path: &Path,
    listing: &Listing,
    index: usize,
    password: Option<&str>,
    tool: Option<&SevenZipTool>,
) -> Result<(TempFolder, PathBuf), ArchiveError> {
    let entry = listing.entries.get(index).ok_or_else(|| ArchiveError::Bad(format!("no entry {index}")))?;
    let bytes = read_entry(path, listing, index, password, tool)?;
    let folder = TempFolder::new()?;
    let unified = entry.name.replace('\\', "/");
    let name = unified
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .unwrap_or("entry");
    let file = folder.path().join(name);
    std::fs::write(&file, bytes)?;
    Ok((folder, file))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
    }

    #[test]
    fn containers_and_codecs_are_told_by_name_and_signature() {
        assert_eq!(container_of("a.zip", b"PK\x03\x04"), Some(Container::Zip));
        assert_eq!(container_of("a.jar", b""), Some(Container::Zip));
        assert_eq!(container_of("a.zip", b"7z\xbc\xaf\x27\x1c"), Some(Container::SevenZ), "the bytes win over the name");
        assert_eq!(container_of("a.7z", b""), Some(Container::SevenZ));
        assert_eq!(container_of("a.rar", b"Rar!\x1a\x07\x01\x00"), Some(Container::External));
        assert_eq!(container_of("a.cbr", b""), Some(Container::External));
        assert_eq!(container_of("a.tar", b""), Some(Container::Tar(None)));
        let mut tar = vec![0u8; 263];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(container_of("odd.bin", &tar), Some(Container::Tar(None)), "ustar at 257");
        assert_eq!(container_of("a.tar.gz", b"\x1f\x8b\x08\x00"), Some(Container::Tar(Some(Codec::Gzip))));
        assert_eq!(container_of("a.tgz", b""), Some(Container::Tar(Some(Codec::Gzip))));
        assert_eq!(container_of("a.taz", b""), Some(Container::Tar(Some(Codec::Gzip))));
        assert_eq!(container_of("a.tar.bz2", b"BZh91AY&SY"), Some(Container::Tar(Some(Codec::Bzip2))));
        assert_eq!(container_of("a.tbz2", b""), Some(Container::Tar(Some(Codec::Bzip2))));
        assert_eq!(container_of("a.tar.xz", b"\xfd7zXZ\x00"), Some(Container::Tar(Some(Codec::Xz))));
        assert_eq!(container_of("a.txz", b""), Some(Container::Tar(Some(Codec::Xz))));
        assert_eq!(container_of("a.tar.zst", b"\x28\xb5\x2f\xfd"), Some(Container::Tar(Some(Codec::Zstd))));
        assert_eq!(container_of("a.tzst", b""), Some(Container::Tar(Some(Codec::Zstd))));
        assert_eq!(container_of("a.tar.lzma", b""), Some(Container::Tar(Some(Codec::Lzma))));
        assert_eq!(container_of("note.txt.gz", b"\x1f\x8b\x08\x00"), Some(Container::Lone(Codec::Gzip)));
        assert_eq!(container_of("note.txt.bz2", b""), Some(Container::Lone(Codec::Bzip2)));
        assert_eq!(container_of("note.txt.xz", b""), Some(Container::Lone(Codec::Xz)));
        assert_eq!(container_of("note.txt.lzma", b""), Some(Container::Lone(Codec::Lzma)));
        assert_eq!(container_of("note.txt.zst", b""), Some(Container::Lone(Codec::Zstd)));
        assert_eq!(container_of("note.txt.Z", b"\x1f\x9d\x90"), Some(Container::External), "compress is 7-Zip's");
        assert_eq!(container_of("a.tar.z", b""), Some(Container::External));
        for ext in ["cab", "iso", "rpm", "deb", "cpio", "lha", "lzh", "arj", "chm", "xar", "wim", "dmg", "vhd", "hfs", "dd", "deskthemepack", "apk", "war", "cbz", "pk3", "pk4", "pkz", "zipx"] {
            assert!(container_of(&format!("a.{ext}"), b"").is_some(), "{ext}");
        }
        assert_eq!(container_of("a.txt", b"hello"), None);
        assert_eq!(lone_entry_name("note.txt.gz"), "note.txt");
        assert_eq!(lone_entry_name("archive.tar.xz"), "archive.tar");
        assert_eq!(lone_entry_name("NOTE.TXT.GZ"), "NOTE.TXT");
        assert_eq!(lone_entry_name("gz"), "gz", "a bare suffix stays");
        assert_eq!(Container::Tar(Some(Codec::Zstd)).label("x.tzst"), "TAR.ZST");
        assert_eq!(Container::Lone(Codec::Bzip2).label("n.bz2"), "BZ2");
        assert_eq!(Container::External.label("disk.iso"), "ISO");
        assert_eq!(human(12), "12 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(1_258_291), "1.2 MB");
        assert_eq!(human(150 * 1024 * 1024), "150 MB");
        assert_eq!(format_unix(1_792_324_800), "2026-10-18 12:00");
        assert_eq!(format_unix(0), "1970-01-01 00:00");
        assert_eq!(format_unix(951_782_400), "2000-02-29 00:00", "the leap day of 2000");
    }

    #[test]
    fn seven_zip_listings_parse_with_their_fields() {
        let text = "\n7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11\n\nScanning the drive for archives:\n1 file, 1234 bytes (2 KiB)\n\nListing archive: x.rar\n\n--\nPath = x.rar\nType = Rar5\n\n----------\nPath = docs\\readme.md\nFolder = -\nSize = 42\nPacked Size = 40\nModified = 2026-10-18 12:00:00\nAttributes = A\nCRC = 1A2B3C4D\nMethod = RAR5:LZSS\n\nPath = docs\nFolder = +\nSize = 0\nPacked Size = 0\nModified = 2026-10-18 12:00:00\nAttributes = D\nCRC = \nMethod = \n\nPath = big.bin\nFolder = -\nSize = 1048576\nPacked Size = 1000\nModified = 2026-10-18 12:00:00\nAttributes = A\nCRC = 00000000\nMethod = RAR5:LZSS\n";
        let entries = parse_seven_zip_listing(text).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0],
            Entry {
                name: "docs/readme.md".to_string(),
                size: 42,
                packed: Some(40),
                modified: Some("2026-10-18 12:00".to_string()),
                method: "RAR5:LZSS".to_string(),
                crc: Some(0x1A2B3C4D),
                is_dir: false,
                raw: None,
            }
        );
        assert_eq!(entries[1].name, "docs/");
        assert!(entries[1].is_dir);
        assert_eq!(entries[1].crc, None);
        assert_eq!(entries[1].method, "");
        assert_eq!(entries[2].size, 1_048_576);
        assert_eq!(entries[2].crc, Some(0));
        assert_eq!(parse_seven_zip_listing("7-Zip\nno archive here\n"), Err("7-Zip listed no entries".to_string()));
        let listing = Listing { container: Container::External, entries, encrypted: false };
        assert_eq!(listing.describe("x.rar", Some(1234)), "RAR, 3 entries, 1.0 KB packed from 1.0 MB");
        assert_eq!(listing.packed_total(), Some(1040));
        assert_eq!(listing.unpacked_total(), 1_048_618);
    }

    #[test]
    fn entries_climbing_out_of_the_folder_are_skipped_on_extraction() {
        let root = Path::new("/out");
        assert_eq!(safe_join(root, "a/b.txt"), Some(root.join("a").join("b.txt")));
        assert_eq!(safe_join(root, "./a/./b.txt"), Some(root.join("a").join("b.txt")));
        assert_eq!(safe_join(root, "a\\b.txt"), Some(root.join("a").join("b.txt")), "backslashes are separators too");
        assert_eq!(safe_join(root, "../escape.txt"), None);
        assert_eq!(safe_join(root, "a/../../escape.txt"), None);
        assert_eq!(safe_join(root, "/etc/passwd"), None);
        if cfg!(windows) {
            assert_eq!(safe_join(root, "C:/windows/win.ini"), None, "a drive");
            assert_eq!(safe_join(root, &"C:/windows/win.ini".replace('/', "\\")), None);
        } else {
            assert_eq!(safe_join(root, "a:b.txt"), Some(root.join("a:b.txt")), "a colon is a character of a name on Unix");
        }
        assert_eq!(safe_join(root, ""), None);
        assert_eq!(safe_join(root, "."), None);
        // The fixture holds `../escape.txt` beside `ok.txt`: one is written, the other named.
        let listing = list(&fixture("escape.zip"), None, None).unwrap();
        assert_eq!(listing.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["../escape.txt", "ok.txt"]);
        let into = std::env::temp_dir().join(format!("sfoglia-archive-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&into);
        let done = extract(&fixture("escape.zip"), &listing, None, &into, None, None).unwrap();
        assert_eq!(done.skipped, ["../escape.txt"]);
        assert_eq!(done.written, [into.join("ok.txt")]);
        assert_eq!(std::fs::read_to_string(into.join("ok.txt")).unwrap(), "fine\n");
        assert!(!into.parent().unwrap().join("escape.txt").exists(), "nothing climbed out");
        let _ = std::fs::remove_dir_all(&into);
    }

    #[test]
    fn zip_entries_list_with_sizes_dates_methods_and_crcs() {
        let path = fixture("bundle.zip");
        let listing = list(&path, None, None).unwrap();
        assert_eq!(listing.container, Container::Zip);
        assert!(!listing.encrypted);
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["readme.md", "data/", "data/table.csv", "data/dot.png", "naïve.txt"]);
        let readme = &listing.entries[0];
        assert_eq!(readme.size, 36);
        assert_eq!(readme.method, "deflate");
        assert_eq!(readme.modified.as_deref(), Some("2026-10-18 12:00"));
        assert_eq!(readme.crc, Some(crc32(b"# Bundle\n\nThree files and a folder.\n")));
        assert_eq!(readme.packed, Some(38));
        let folder = &listing.entries[1];
        assert!(folder.is_dir);
        assert_eq!((folder.size, folder.method.as_str(), folder.crc), (0, "", None));
        let table = &listing.entries[2];
        assert_eq!(table.size, std::fs::metadata(fixture("table.csv")).unwrap().len());
        assert_eq!(table.method, "bzip2");
        assert_eq!(table.packed, Some(160));
        let dot = &listing.entries[3];
        assert_eq!(dot.method, "stored");
        assert_eq!(dot.size, 67);
        assert_eq!(dot.packed, Some(67), "stored: packed as large as the entry");
        assert_eq!(listing.entries[4].method, "zstd");
        assert_eq!(listing.entries[4].size, 13);
        assert_eq!(listing.entries[4].packed, Some(22));
        assert_eq!(read_entry(&path, &listing, 0, None, None).unwrap(), b"# Bundle\n\nThree files and a folder.\n");
        assert_eq!(read_entry(&path, &listing, 3, None, None).unwrap(), std::fs::read(fixture("dot.png")).unwrap());
        assert_eq!(read_entry(&path, &listing, 4, None, None).unwrap(), "Unicode name\n".as_bytes());
        assert!(matches!(read_entry(&path, &listing, 1, None, None), Err(ArchiveError::Bad(_))), "a folder has no bytes");
        let summary = listing.describe("bundle.zip", std::fs::metadata(&path).ok().map(|m| m.len()));
        assert_eq!(summary, "ZIP, 5 entries, 287 B packed from 258 B");
        assert_eq!(listing.packed_total(), Some(287));
        // Extract all into a folder: the paths kept, the folder entry made.
        let into = std::env::temp_dir().join(format!("sfoglia-archive-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&into);
        let done = extract(&path, &listing, None, &into, None, None).unwrap();
        assert_eq!(done.skipped, Vec::<String>::new());
        assert_eq!(done.written.len(), 5);
        assert_eq!(std::fs::read(into.join("data").join("dot.png")).unwrap(), std::fs::read(fixture("dot.png")).unwrap());
        assert!(into.join("data").is_dir());
        // Extract one: only it.
        let one = std::env::temp_dir().join(format!("sfoglia-archive-zip-one-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&one);
        let done = extract(&path, &listing, Some(&[4]), &one, None, None).unwrap();
        assert_eq!(done.written, [one.join("naïve.txt")]);
        assert!(!one.join("readme.md").exists());
        let _ = std::fs::remove_dir_all(&into);
        let _ = std::fs::remove_dir_all(&one);
    }

    #[test]
    fn tar_streams_through_each_codec_list_the_same_entries() {
        let plain = list(&fixture("bundle.tar"), None, None).unwrap();
        assert_eq!(plain.container, Container::Tar(None));
        let names: Vec<&str> = plain.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["readme.md", "data/", "data/table.csv", "data/dot.png", "naïve.txt"]);
        assert_eq!(plain.entries[0].method, "stored");
        assert_eq!(plain.entries[0].modified.as_deref(), Some("2026-10-18 12:00"));
        assert_eq!(plain.entries[0].packed, None, "a tar packs nothing");
        assert_eq!(plain.entries[0].crc, None);
        assert!(plain.entries[1].is_dir);
        assert_eq!(plain.packed_total(), None);
        assert_eq!(plain.describe("bundle.tar", Some(10_240)), format!("TAR, 5 entries, 10.0 KB packed from {}", human(plain.unpacked_total())));
        for (file, codec) in [
            ("bundle.tar.gz", Codec::Gzip),
            ("bundle.tar.bz2", Codec::Bzip2),
            ("bundle.tar.xz", Codec::Xz),
            ("bundle.tar.zst", Codec::Zstd),
            ("bundle.tgz", Codec::Gzip),
        ] {
            let listing = list(&fixture(file), None, None).unwrap();
            assert_eq!(listing.container, Container::Tar(Some(codec)), "{file}");
            let through: Vec<(&str, u64, bool)> = listing.entries.iter().map(|e| (e.name.as_str(), e.size, e.is_dir)).collect();
            let expected: Vec<(&str, u64, bool)> = plain.entries.iter().map(|e| (e.name.as_str(), e.size, e.is_dir)).collect();
            assert_eq!(through, expected, "{file}");
            assert_eq!(listing.entries[0].method, codec.label(), "{file}");
            assert_eq!(read_entry(&fixture(file), &listing, 4, None, None).unwrap(), "Unicode name\n".as_bytes(), "{file}");
        }
        // Extract from the xz tar: the folder and the files, in one pass.
        let into = std::env::temp_dir().join(format!("sfoglia-archive-tar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&into);
        let listing = list(&fixture("bundle.tar.xz"), None, None).unwrap();
        let done = extract(&fixture("bundle.tar.xz"), &listing, Some(&[1, 2]), &into, None, None).unwrap();
        assert_eq!(done.written, [into.join("data"), into.join("data").join("table.csv")]);
        assert_eq!(std::fs::read(into.join("data").join("table.csv")).unwrap(), std::fs::read(fixture("table.csv")).unwrap());
        let _ = std::fs::remove_dir_all(&into);
        // A tar entry climbing out is skipped too.
        let climb = list(&fixture("climb.tar"), None, None).unwrap();
        assert_eq!(climb.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["../climb.txt", "ok.txt"]);
        let out = std::env::temp_dir().join(format!("sfoglia-archive-climb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let done = extract(&fixture("climb.tar"), &climb, None, &out, None, None).unwrap();
        assert_eq!(done.skipped, ["../climb.txt"]);
        assert_eq!(done.written, [out.join("ok.txt")]);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_seven_zip_lists_reads_and_needs_its_password() {
        let path = fixture("bundle.7z");
        let listing = list(&path, None, None).unwrap();
        assert_eq!(listing.container, Container::SevenZ);
        assert!(!listing.encrypted);
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["readme.md", "data/", "data/table.csv", "data/dot.png", "naïve.txt"]);
        assert_eq!(listing.entries[0].method, "LZMA2");
        assert_eq!(listing.entries[0].modified.as_deref(), Some("2026-10-18 12:00"));
        assert_eq!(listing.entries[0].size, 36);
        assert_eq!(listing.entries[0].crc, Some(crc32(b"# Bundle\n\nThree files and a folder.\n")));
        assert!(listing.entries[1].is_dir);
        assert_eq!(listing.entries.iter().map(|e| e.packed).collect::<Vec<_>>(), [Some(40), None, Some(139), Some(71), Some(17)]);
        assert_eq!(listing.describe("bundle.7z", Some(500)), "7Z, 5 entries, 267 B packed from 258 B");
        assert_eq!(read_entry(&path, &listing, 0, None, None).unwrap(), b"# Bundle\n\nThree files and a folder.\n");
        assert_eq!(read_entry(&path, &listing, 3, None, None).unwrap(), std::fs::read(fixture("dot.png")).unwrap());
        // Encrypted headers: no listing without the password, a wrong one says so, the right one lists and reads.
        let secret = fixture("secret.7z");
        assert!(matches!(list(&secret, None, None), Err(ArchiveError::NeedsPassword)));
        assert!(matches!(list(&secret, Some("nope"), None), Err(ArchiveError::WrongPassword)));
        let listing = list(&secret, Some("sfoglia"), None).unwrap();
        assert!(listing.encrypted);
        assert_eq!(listing.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["secret.txt"]);
        assert_eq!(listing.entries[0].method, "LZMA2+AES", "as 7-Zip prints it: the cipher after the compressor");
        assert_eq!(listing.entries[0].packed, Some(32));
        assert_eq!(listing.describe("secret.7z", Some(276)), "7Z, 1 entry, 32 B packed from 25 B");
        assert_eq!(read_entry(&secret, &listing, 0, Some("sfoglia"), None).unwrap(), b"The password is sfoglia.\n");
        assert_eq!(ArchiveError::NeedsPassword.to_string(), "the archive is encrypted: it needs its password");
        assert_eq!(ArchiveError::WrongPassword.to_string(), "wrong password");
    }

    #[test]
    fn a_lone_compressed_file_is_a_one_entry_archive() {
        for (file, codec) in [
            ("note.txt.gz", Codec::Gzip),
            ("note.txt.bz2", Codec::Bzip2),
            ("note.txt.xz", Codec::Xz),
            ("note.txt.lzma", Codec::Lzma),
            ("note.txt.zst", Codec::Zstd),
        ] {
            let path = fixture(file);
            let listing = list(&path, None, None).unwrap();
            assert_eq!(listing.container, Container::Lone(codec), "{file}");
            assert_eq!(listing.entries.len(), 1);
            let entry = &listing.entries[0];
            assert_eq!(entry.name, "note.txt", "{file}");
            assert_eq!(entry.size, 26, "{file}");
            assert_eq!(entry.method, codec.label());
            assert_eq!(entry.packed, Some(std::fs::metadata(&path).unwrap().len()));
            assert!(!entry.is_dir);
            assert_eq!(read_entry(&path, &listing, 0, None, None).unwrap(), b"A note, compressed alone.\n", "{file}");
            let summary = listing.describe(file, Some(entry.packed.unwrap()));
            assert!(summary.starts_with(&format!("{}, 1 entry, ", codec.suffixes()[0].to_ascii_uppercase())), "{summary}");
        }
        let gz = list(&fixture("note.txt.gz"), None, None).unwrap();
        assert_eq!(gz.entries[0].modified.as_deref(), Some("2026-10-18 12:00"), "gzip keeps a time in its header");
        assert_eq!(list(&fixture("note.txt.bz2"), None, None).unwrap().entries[0].modified, None, "bzip2 keeps none");
    }

    #[test]
    fn encrypted_entries_check_the_password_in_zips_and_clear_header_seven_zips() {
        // AES-256 (`secret.zip`), ZipCrypto (`legacy.zip`) and a 7z whose headers are in the clear (`entries.7z`,
        // 7-Zip's own default) all list without a password and say what they need when an entry is read.
        for (file, container, name, size, packed, method, bytes) in [
            ("secret.zip", Container::Zip, "secret.txt", 25, 55, "deflate", &b"The password is sfoglia.\n"[..]),
            ("legacy.zip", Container::Zip, "legacy.txt", 22, 36, "deflate", &b"ZipCrypto keeps this.\n"[..]),
            ("entries.7z", Container::SevenZ, "secret.txt", 25, 32, "LZMA2+AES", &b"The password is sfoglia.\n"[..]),
        ] {
            let path = fixture(file);
            let listing = list(&path, None, None).unwrap();
            assert_eq!(listing.container, container, "{file}");
            assert!(listing.encrypted, "{file} says it is encrypted");
            assert_eq!(listing.entries.len(), 1, "{file}");
            let entry = &listing.entries[0];
            assert_eq!((entry.name.as_str(), entry.size, entry.packed, entry.method.as_str()), (name, size, Some(packed), method), "{file}");
            assert_eq!(entry.modified.as_deref(), Some("2026-10-18 12:00"), "{file}");
            assert_eq!(entry.crc, Some(crc32(bytes)), "{file}");
            assert!(matches!(read_entry(&path, &listing, 0, None, None), Err(ArchiveError::NeedsPassword)), "{file}: no password");
            assert!(matches!(read_entry(&path, &listing, 0, Some("nope"), None), Err(ArchiveError::WrongPassword)), "{file}: a wrong one");
            assert_eq!(read_entry(&path, &listing, 0, Some("sfoglia"), None).unwrap(), bytes, "{file}");
            // A password given at listing time is checked then, so Unlock can say it is wrong.
            assert!(matches!(list(&path, Some("nope"), None), Err(ArchiveError::WrongPassword)), "{file}: listed with a wrong one");
            let unlocked = list(&path, Some("sfoglia"), None).unwrap();
            assert_eq!(unlocked.entries, listing.entries, "{file}: the same listing with the right one");
            assert!(unlocked.encrypted);
            // Extraction with the right password writes the bytes; with none it refuses before writing anything.
            let into = std::env::temp_dir().join(format!("sfoglia-archive-secret-{}-{}", std::process::id(), file.replace('.', "-")));
            let _ = std::fs::remove_dir_all(&into);
            assert!(matches!(extract(&path, &listing, None, &into, None, None), Err(ArchiveError::NeedsPassword)), "{file}");
            assert!(!into.join(name).exists());
            let done = extract(&path, &listing, None, &into, Some("sfoglia"), None).unwrap();
            assert_eq!(done.written, [into.join(name)]);
            assert_eq!(std::fs::read(into.join(name)).unwrap(), bytes);
            let _ = std::fs::remove_dir_all(&into);
        }
        // A password given for an archive with nothing encrypted is simply unused.
        assert_eq!(list(&fixture("bundle.zip"), Some("nope"), None).unwrap().entries.len(), 5);
        assert_eq!(list(&fixture("bundle.7z"), Some("nope"), None).unwrap().entries.len(), 5);
        assert_eq!(read_entry(&fixture("bundle.zip"), &list(&fixture("bundle.zip"), None, None).unwrap(), 0, Some("nope"), None).unwrap().len(), 36);
    }

    #[test]
    fn lzma_and_bcj_entries_read_through_the_library() {
        // `lzma.zip` and `bcj.7z` were made once with the real 7-Zip (`-mm=LZMA`; `-m0=BCJ -m1=LZMA`).
        let lzma = fixture("lzma.zip");
        let listing = list(&lzma, None, None).unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!((listing.entries[0].name.as_str(), listing.entries[0].size, listing.entries[0].method.as_str()), ("lines.txt", 1400, "lzma"));
        let bytes = read_entry(&lzma, &listing, 0, None, None).unwrap();
        assert_eq!(bytes.len(), 1400);
        assert!(bytes.starts_with(b"Line 01: a note, compressed alone.\nLine 02: a note"), "{}", String::from_utf8_lossy(&bytes[..60]));
        assert_eq!(listing.entries[0].crc, Some(crc32(&bytes)));
        let bcj = fixture("bcj.7z");
        let listing = list(&bcj, None, None).unwrap();
        assert_eq!(listing.entries.len(), 1);
        let entry = &listing.entries[0];
        assert_eq!((entry.name.as_str(), entry.size, entry.packed, entry.method.as_str()), ("readme.md", 36, Some(40), "BCJ+LZMA"));
        assert_eq!(read_entry(&bcj, &listing, 0, None, None).unwrap(), b"# Bundle\n\nThree files and a folder.\n");
        assert_eq!(listing.describe("bcj.7z", Some(162)), "7Z, 1 entry, 40 B packed from 36 B");
    }

    #[test]
    fn folders_the_archive_does_not_record_get_rows_of_their_own() {
        // `more.zip` holds `d/n.txt` with no entry for `d/`: the listing adds the row before its first child.
        let path = fixture("more.zip");
        let listing = list(&path, None, None).unwrap();
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["data.json", "three.pdf", "inner.zip", "noise.bin", "d/", "d/n.txt"]);
        let folder = &listing.entries[4];
        assert_eq!(
            folder,
            &Entry { name: "d/".to_string(), size: 0, packed: None, modified: None, method: String::new(), crc: None, is_dir: true, raw: None }
        );
        assert_eq!(listing.entries[5].raw, Some(4), "the file keeps its own index inside the zip");
        assert_eq!(listing.entries.iter().map(|e| e.packed).collect::<Vec<_>>(), [Some(106), Some(531), Some(140), Some(25), None, Some(9)]);
        assert_eq!(listing.entries.iter().map(|e| e.modified.as_deref()).collect::<Vec<_>>(), [Some("2026-10-18 12:00"), Some("2026-10-18 12:01"), Some("2026-10-18 12:02"), Some("2026-10-18 12:03"), None, Some("2026-10-18 12:04")]);
        assert_eq!(listing.entries.iter().map(|e| e.method.as_str()).collect::<Vec<_>>(), ["deflate", "bzip2", "stored", "zstd", "", "deflate"]);
        assert_eq!(listing.describe("more.zip", std::fs::metadata(&path).ok().map(|m| m.len())), "ZIP, 6 entries, 811 B packed from 1.6 KB");
        assert!(matches!(read_entry(&path, &listing, 4, None, None), Err(ArchiveError::Bad(reason)) if reason == "d/ is a folder"));
        assert_eq!(read_entry(&path, &listing, 5, None, None).unwrap(), b"nested\n");
        assert_eq!(read_entry(&path, &listing, 2, None, None).unwrap().len(), 140, "the nested zip's bytes");
        let into = std::env::temp_dir().join(format!("sfoglia-archive-more-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&into);
        let done = extract(&path, &listing, None, &into, None, None).unwrap();
        assert_eq!(done.skipped, Vec::<String>::new());
        assert_eq!(done.written.len(), 6);
        assert!(into.join("d").is_dir(), "the filled-in folder is made");
        assert_eq!(std::fs::read(into.join("d").join("n.txt")).unwrap(), b"nested\n");
        let _ = std::fs::remove_dir_all(&into);
        // Extracting the folder row alone makes the folder, nothing inside it.
        let _ = std::fs::remove_dir_all(&into);
        let done = extract(&path, &listing, Some(&[4]), &into, None, None).unwrap();
        assert_eq!(done.written, [into.join("d")]);
        assert!(!into.join("d").join("n.txt").exists());
        let _ = std::fs::remove_dir_all(&into);
        // The fill-in itself: every missing parent, once, before the first child; `.` and `..` never.
        let file = |name: &str| Entry { name: name.to_string(), size: 1, packed: None, modified: None, method: String::new(), crc: None, is_dir: false, raw: Some(0) };
        let filled = fill_in_folders(vec![file("a/b/c.txt"), file("a/d.txt"), file("../x.txt"), file("./y.txt"), file("a/b/e.txt")]);
        assert_eq!(filled.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a/", "a/b/", "a/b/c.txt", "a/d.txt", "../x.txt", "./y.txt", "a/b/e.txt"]);
        assert!(filled[0].is_dir && filled[1].is_dir && filled[0].raw.is_none());
        // A folder recorded after its children (7-Zip's order for a rar) stays where it is, with no double.
        let mut dir = file("data/");
        dir.is_dir = true;
        let filled = fill_in_folders(vec![file("data/a.txt"), dir]);
        assert_eq!(filled.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["data/a.txt", "data/"]);
        // The same in a tar (`nodir.tar`: `d/n.txt` alone): the folder row, extracted on its own, the file by its raw index.
        let tar = fixture("nodir.tar");
        let listing = list(&tar, None, None).unwrap();
        assert_eq!(listing.entries.iter().map(|e| (e.name.as_str(), e.is_dir, e.raw)).collect::<Vec<_>>(), [("d/", true, None), ("d/n.txt", false, Some(0))]);
        assert_eq!(read_entry(&tar, &listing, 1, None, None).unwrap(), b"nested
");
        let into = std::env::temp_dir().join(format!("sfoglia-archive-nodir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&into);
        let done = extract(&tar, &listing, Some(&[0]), &into, None, None).unwrap();
        assert_eq!(done.written, [into.join("d")], "the folder alone");
        assert!(into.join("d").is_dir() && !into.join("d").join("n.txt").exists());
        let done = extract(&tar, &listing, None, &into, None, None).unwrap();
        assert_eq!(done.written, [into.join("d"), into.join("d").join("n.txt")]);
        assert_eq!(std::fs::read(into.join("d").join("n.txt")).unwrap(), b"nested
");
        let _ = std::fs::remove_dir_all(&into);
    }

    /// The CRC-32 the zip and 7z formats record (IEEE), for the expected values.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }
}
