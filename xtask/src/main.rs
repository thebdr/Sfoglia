//! Repository tasks.
//!
//! `cargo run -p xtask -- formats [--check]` renders two files from the format-family
//! clauses of `.zen/contract.md`: `FORMATS.md`, the user-facing checklist (C-002), and
//! `src/generated/families.rs`, the extension table the application routes by (C-004).
//! With `--check` it only compares and exits non-zero when a committed file is stale.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("formats") => {
            let check = args.iter().any(|a| a == "--check");
            match formats(&repo_root(), check) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("{message}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("fixtures") => match fixtures(&repo_root()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo run -p xtask -- formats [--check] | fixtures");
            ExitCode::FAILURE
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the repository root")
        .to_path_buf()
}

/// The two outputs, as (path relative to the repository root, renderer).
const OUTPUTS: [(&str, fn(&str) -> Result<String, String>); 2] = [
    ("FORMATS.md", render_formats),
    ("src/generated/families.rs", render_families_rs),
];

fn formats(root: &Path, check: bool) -> Result<(), String> {
    let contract_path = root.join(".zen").join("contract.md");
    let contract = std::fs::read_to_string(&contract_path)
        .map_err(|e| format!("cannot read {}: {e}", contract_path.display()))?;
    for (relative, render) in OUTPUTS {
        let output_path = root.join(relative);
        let rendered = render(&contract)?;
        if check {
            let committed = std::fs::read_to_string(&output_path)
                .map_err(|e| format!("cannot read {}: {e}", output_path.display()))?;
            if committed.replace("\r\n", "\n") != rendered {
                return Err(format!(
                    "{} is stale: run `cargo run -p xtask -- formats`",
                    output_path.display()
                ));
            }
        } else {
            if let Some(dir) = output_path.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            std::fs::write(&output_path, &rendered)
                .map_err(|e| format!("cannot write {}: {e}", output_path.display()))?;
            println!("wrote {}", output_path.display());
        }
    }
    Ok(())
}

/// One contract clause, as far as the generators need it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Clause {
    id: String,
    title: String,
    date: String,
    source: String,
    family: String,
    description: String,
    status: String,
}

/// Parse `## C-xxx:` and `### P-xxx:` clauses with their `- **Field:** value` lines.
fn parse_clauses(contract: &str) -> Vec<Clause> {
    let mut clauses = Vec::new();
    let mut current: Option<Clause> = None;
    for line in contract.lines() {
        let heading = line.strip_prefix("### ").or_else(|| line.strip_prefix("## "));
        if let Some(rest) = heading {
            if let Some((id, title)) = rest.split_once(": ") {
                let is_clause = (id.starts_with("C-") || id.starts_with("P-"))
                    && id.len() > 2
                    && id[2..].chars().all(|c| c.is_ascii_digit());
                if is_clause {
                    if let Some(done) = current.take() {
                        clauses.push(done);
                    }
                    current = Some(Clause {
                        id: id.to_string(),
                        title: title.trim().to_string(),
                        date: String::new(),
                        source: String::new(),
                        family: String::new(),
                        description: String::new(),
                        status: String::new(),
                    });
                    continue;
                }
            }
            // Any other heading ends the current clause.
            if let Some(done) = current.take() {
                clauses.push(done);
            }
            continue;
        }
        let Some(clause) = current.as_mut() else { continue };
        if let Some(field) = line.strip_prefix("- **") {
            if let Some((name, value)) = field.split_once(":** ") {
                let value = value.trim();
                match name {
                    "Date" => clause.date = value.to_string(),
                    "Source" => clause.source = value.to_string(),
                    "Family" => clause.family = value.to_string(),
                    "Description" => clause.description = value.to_string(),
                    "Status" => clause.status = value.to_string(),
                    _ => {}
                }
            }
        }
    }
    if let Some(done) = current.take() {
        clauses.push(done);
    }
    clauses
}

/// An extension token: lowercase letters, digits, dots and dashes, starting with a letter or digit.
fn is_extension_token(word: &str) -> bool {
    let mut chars = word.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

/// A note after a token: exactly one parenthesised group that closes the entry.
fn is_single_parenthesised_note(rest: &str) -> bool {
    rest.len() >= 2
        && rest.starts_with('(')
        && rest.ends_with(')')
        && !rest[1..rest.len() - 1].contains(['(', ')'])
}

/// The extensions a format-family clause enumerates. A family's description *starts*
/// with `Extensions:`; a clause that merely mentions the word in prose is not a family.
/// A token is one extension, optionally followed by one parenthesised note, so
/// `dat (winmail.dat)` yields `dat`; order is kept and duplicates are dropped. Anything
/// else, such as a missing comma (`doc docx`) or prose (`pdf and xps`), is an error.
fn extensions(description: &str) -> Result<Vec<String>, String> {
    let Some(list) = description.strip_prefix("Extensions:") else {
        return Ok(Vec::new());
    };
    let list = match list.find(". ") {
        Some(end) => &list[..end],
        None => list.trim_end_matches('.'),
    };
    let mut seen = Vec::new();
    for token in list.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let (word, rest) = token.split_once(' ').unwrap_or((token, ""));
        let word = word.trim_matches('.');
        if !is_extension_token(word) {
            return Err(format!("not an extension: {word:?}"));
        }
        let rest = rest.trim();
        if !rest.is_empty() && !is_single_parenthesised_note(rest) {
            return Err(format!("not an extension list entry: {token:?}"));
        }
        if !seen.iter().any(|s| s == word) {
            seen.push(word.to_string());
        }
    }
    Ok(seen)
}

/// A family slug: lowercase words joined by single dashes, such as `camera-raw`.
fn is_family_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.split('-').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// `camera-raw` becomes `CameraRaw`; `pdf-xps` becomes `PdfXps`.
fn camel_case(slug: &str) -> String {
    slug.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn read_flag(clause: &Clause) -> &'static str {
    if clause.id.starts_with("P-") {
        "planned"
    } else if clause.status.starts_with("verified") {
        "done"
    } else {
        "in progress"
    }
}

/// Openings of a write sentence that mean writing is not offered.
const WRITE_DENIALS: [&str; 5] = ["out of scope", "no", "none", "not supported", "read-only"];

/// The Write flag follows the clause's own `Write:` sentence; only the part before the
/// first semicolon decides, so a deferred remainder (`Write: txt now; docx is a later
/// extension.`) does not demote the present grant. No sentence (including an empty one), or
/// one that opens with `out of scope`, `no`, `none`, `not supported` or `read-only`, means `no`. One that opens
/// with `later` or speaks of a `later extension` means `later`. Anything else is a present
/// grant and mirrors the Read flag.
fn write_flag(clause: &Clause) -> &'static str {
    let Some(start) = clause.description.find("Write:") else {
        return "no";
    };
    let sentence = &clause.description[start + "Write:".len()..];
    let sentence = sentence.split_once(". ").map_or(sentence, |(s, _)| s);
    let now = sentence.split(';').next().unwrap_or(sentence).trim();
    let now = now.trim_end_matches('.').trim().to_ascii_lowercase();
    if now.is_empty() {
        return "no";
    }
    let opens_with = |word: &str| {
        now.strip_prefix(word)
            .is_some_and(|after| after.is_empty() || !after.starts_with(|c: char| c.is_alphanumeric()))
    };
    if WRITE_DENIALS.iter().any(|w| opens_with(w)) {
        "no"
    } else if opens_with("later") || now.contains("later extension") {
        "later"
    } else {
        read_flag(clause)
    }
}

/// Who asked for the family and when. The origin is the part before `#` of an
/// `ingest (...)` provenance, or the bare source (`discovery`); a date found inside
/// the origin (`user-request-2026-10-13`, `issue-2026-09-01`) is the request date and
/// is lifted out of the label; otherwise the clause's own date is used. A family with
/// no origin, or with neither date, is an error.
fn requested_by(clause: &Clause) -> Result<String, String> {
    let origin = match clause
        .source
        .strip_prefix("ingest (")
        .and_then(|s| s.strip_suffix(')'))
    {
        Some(inner) => inner.split('#').next().unwrap_or(inner),
        None => clause.source.as_str(),
    }
    .trim();
    if origin.is_empty() {
        return Err(format!("{}: no request origin", clause.id));
    }
    let (label, embedded_date) = match find_date(origin) {
        Some(date) => (origin.replace(date, "").trim_matches('-').to_string(), Some(date)),
        None => (origin.to_string(), None),
    };
    let label = label.replace('-', " ");
    let label = label.trim();
    if label.is_empty() {
        return Err(format!("{}: no request origin", clause.id));
    }
    let date = match embedded_date {
        Some(date) => date,
        None if is_date(&clause.date) => clause.date.as_str(),
        None => return Err(format!("{}: no request date", clause.id)),
    };
    Ok(format!("{label} {date}"))
}

/// `YYYY-MM-DD` with a real year (2000 or later), month and day for that month.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    let shaped = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit());
    if !shaped {
        return false;
    }
    let (year, month, day): (u32, u32, u32) = (
        s[0..4].parse().unwrap_or(0),
        s[5..7].parse().unwrap_or(0),
        s[8..10].parse().unwrap_or(0),
    );
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    year >= 2000 && day >= 1 && day <= days_in_month
}

/// The first `YYYY-MM-DD` inside `text`, if any.
fn find_date(text: &str) -> Option<&str> {
    (0..text.len().saturating_sub(9))
        .filter(|&i| text.is_char_boundary(i) && text.is_char_boundary(i + 10))
        .map(|i| &text[i..i + 10])
        .find(|candidate| is_date(candidate))
}

/// A format family: a clause whose description enumerates extensions. It must carry a
/// `Family:` slug, which names it in the generated code.
struct Family {
    clause: Clause,
    extensions: Vec<String>,
}

fn families(contract: &str) -> Result<Vec<Family>, String> {
    let mut out: Vec<Family> = Vec::new();
    for clause in parse_clauses(contract) {
        let extensions = extensions(&clause.description).map_err(|e| format!("{}: {e}", clause.id))?;
        if extensions.is_empty() {
            continue;
        }
        if !is_family_slug(&clause.family) {
            return Err(format!("{}: missing or malformed Family slug {:?}", clause.id, clause.family));
        }
        if out.iter().any(|f| f.clause.family == clause.family) {
            return Err(format!("{}: Family slug {:?} is used twice", clause.id, clause.family));
        }
        out.push(Family { clause, extensions });
    }
    Ok(out)
}

fn render_formats(contract: &str) -> Result<String, String> {
    let families = families(contract)?;
    let extension_count: usize = families.iter().map(|f| f.extensions.len()).sum();

    let mut out = String::new();
    out.push_str("# Sfoglia format support\n\n");
    out.push_str(
        "Generated from `.zen/contract.md` by `cargo run -p xtask -- formats`. \
         Do not edit by hand: `cargo test` fails while this file is stale.\n\n",
    );
    out.push_str(
        "Legend: **done** = proven by a verified clause. **in progress** = clause exists, \
         not yet verified. **planned** = pending clause. **later** = a later extension of the \
         clause. **no** = out of scope.\n\n",
    );
    out.push_str(&format!(
        "{} families, {} extensions.\n\n",
        families.len(),
        extension_count
    ));
    out.push_str("| Family | Clause | Read | Write | Requested | Extensions |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for family in &families {
        let clause = &family.clause;
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            clause.title,
            clause.id,
            read_flag(clause),
            write_flag(clause),
            requested_by(clause)?,
            family.extensions.join(", ")
        ));
    }
    Ok(out)
}

/// The Rust extension table: one `Family` variant per family clause, in contract order,
/// and every extension mapped to the families that list it, sorted by extension.
fn render_families_rs(contract: &str) -> Result<String, String> {
    let families = families(contract)?;
    if families.is_empty() {
        return Err("the contract names no format family".to_string());
    }
    let variants: Vec<String> = families.iter().map(|f| camel_case(&f.clause.family)).collect();

    let mut table: Vec<(&str, Vec<&str>)> = Vec::new();
    for (family, variant) in families.iter().zip(&variants) {
        for ext in &family.extensions {
            match table.iter_mut().find(|(e, _)| e == ext) {
                Some((_, owners)) => owners.push(variant),
                None => table.push((ext, vec![variant])),
            }
        }
    }
    table.sort_by(|a, b| a.0.cmp(b.0));

    let mut out = String::new();
    out.push_str("//! Generated from `.zen/contract.md` by `cargo run -p xtask -- formats`. Do not edit.\n");
    out.push_str("//! One variant per format-family clause, in contract order; every extension the\n");
    out.push_str("//! families enumerate, sorted, with its owners in contract order.\n\n");
    out.push_str("/// A format family, as named by a clause of the contract.\n");
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    out.push_str("pub enum Family {\n");
    for (family, variant) in families.iter().zip(&variants) {
        out.push_str(&format!("    /// {} ({}).\n    {},\n", family.clause.title, family.clause.id, variant));
    }
    out.push_str("}\n\n");
    out.push_str("impl Family {\n");
    out.push_str("    /// Every family, in contract order.\n");
    out.push_str("    pub const ALL: &[Family] = &[\n");
    for variant in &variants {
        out.push_str(&format!("        Family::{variant},\n"));
    }
    out.push_str("    ];\n\n");
    out.push_str("    /// The slug the contract names the family by.\n");
    out.push_str("    pub fn slug(self) -> &'static str {\n        match self {\n");
    for (family, variant) in families.iter().zip(&variants) {
        out.push_str(&format!("            Family::{variant} => \"{}\",\n", family.clause.family));
    }
    out.push_str("        }\n    }\n\n");
    out.push_str("    /// The clause's title.\n");
    out.push_str("    pub fn title(self) -> &'static str {\n        match self {\n");
    for (family, variant) in families.iter().zip(&variants) {
        out.push_str(&format!("            Family::{variant} => \"{}\",\n", family.clause.title.replace('"', "\\\"")));
    }
    out.push_str("        }\n    }\n\n");
    out.push_str("    /// The clause that defines the family (a P- clause until its viewer is verified).\n");
    out.push_str("    pub fn clause(self) -> &'static str {\n        match self {\n");
    for (family, variant) in families.iter().zip(&variants) {
        out.push_str(&format!("            Family::{variant} => \"{}\",\n", family.clause.id));
    }
    out.push_str("        }\n    }\n}\n\n");
    out.push_str("/// Extension (lower case, no leading dot) to the families that list it, in contract order;\n");
    out.push_str("/// sorted by extension so it can be searched with `binary_search_by_key`.\n");
    out.push_str("pub static EXTENSIONS: &[(&str, &[Family])] = &[\n");
    for (ext, owners) in &table {
        let owners: Vec<String> = owners.iter().map(|v| format!("Family::{v}")).collect();
        out.push_str(&format!("    (\"{ext}\", &[{}]),\n", owners.join(", ")));
    }
    out.push_str("];\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
# Contract Items

## C-007: Word-processing documents
- **Date:** 2026-10-14
- **Source:** ingest (user-request-2026-10-13#text-documents)
- **Family:** word-processing
- **Description:** Extensions: docx, odt, txt. Read: structure. Write: txt now; docx is a later extension.
- **Verification:** automated → docx_fixture
- **Status:** verified (2026-10-14)

## C-008: PDF viewing
- **Date:** 2026-10-14
- **Source:** discovery
- **Family:** pdf-xps
- **Description:** Extensions: pdf. Pages render. Write: out of scope.
- **Verification:** manual: look
- **Status:** pending verification

## C-009: Checklist generator
- **Date:** 2026-10-14
- **Source:** discovery
- **Description:** Generated from the clauses that enumerate `Extensions:` by one command. Write: anything.
- **Verification:** manual: look
- **Status:** pending verification

## Pending Contract Deltas

### P-014: Email messages
- **Date:** 2026-10-14
- **Source:** ingest (user-request-2026-10-13#email)
- **Family:** email
- **Description:** Extensions: eml, msg, dat (winmail.dat), eml, tar.gz. Headers show. Write: out of scope.
- **Proposed verification:** automated: headers

### P-021: Archives
- **Date:** 2026-10-20
- **Source:** ingest (issue-42#archives)
- **Family:** archives
- **Description:** Extensions: 7z, zip, tar.gz, dat. Read: list. Write: create and update entries for 7z and zip.
- **Proposed verification:** automated: round trip

### P-030: Not a format family
- **Date:** 2026-10-14
- **Source:** discovery
- **Description:** Something without an extension list.
- **Proposed verification:** TBD
";

    fn clause(id: &str) -> Clause {
        parse_clauses(FIXTURE)
            .into_iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("{id} in fixture"))
    }

    fn with_write(sentence: &str) -> Clause {
        let mut c = clause("C-007");
        c.description = format!("Extensions: x. Read: yes. {sentence}");
        c
    }

    #[test]
    fn parses_every_clause_with_its_fields() {
        let clauses = parse_clauses(FIXTURE);
        let ids: Vec<&str> = clauses.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["C-007", "C-008", "C-009", "P-014", "P-021", "P-030"]);
        assert_eq!(clauses[0].title, "Word-processing documents");
        assert_eq!(clauses[0].date, "2026-10-14");
        assert_eq!(clauses[0].family, "word-processing");
        assert_eq!(clauses[0].status, "verified (2026-10-14)");
        assert_eq!(clauses[2].family, "");
        assert_eq!(clauses[3].source, "ingest (user-request-2026-10-13#email)");
        assert_eq!(clauses[4].date, "2026-10-20");
        assert_eq!(clauses[5].status, "");
        assert_eq!(parse_clauses(""), Vec::<Clause>::new());
        assert_eq!(parse_clauses("## C-: bad id\n## Notes: text\n"), Vec::<Clause>::new());
    }

    #[test]
    fn extension_lists_keep_order_drop_duplicates_and_parentheses() {
        assert_eq!(extensions(&clause("C-007").description).unwrap(), ["docx", "odt", "txt"]);
        assert_eq!(extensions(&clause("C-008").description).unwrap(), ["pdf"]);
        assert_eq!(
            extensions(&clause("P-014").description).unwrap(),
            ["eml", "msg", "dat", "tar.gz"]
        );
        assert_eq!(extensions(&clause("P-030").description).unwrap(), Vec::<String>::new());
        assert_eq!(extensions("").unwrap(), Vec::<String>::new());
        assert_eq!(extensions("Extensions:").unwrap(), Vec::<String>::new());
        assert_eq!(extensions("Extensions: a, b.").unwrap(), ["a", "b"]);
        assert_eq!(extensions("Extensions: dat (winmail.dat).").unwrap(), ["dat"]);
        assert_eq!(extensions("Extensions: dat (winmail.dat), eml.").unwrap(), ["dat", "eml"]);
    }

    #[test]
    fn a_clause_that_mentions_extensions_in_prose_is_not_a_family() {
        assert_eq!(extensions(&clause("C-009").description).unwrap(), Vec::<String>::new());
        let ids: Vec<String> = families(FIXTURE)
            .unwrap()
            .into_iter()
            .map(|f| f.clause.id)
            .collect();
        assert_eq!(ids, ["C-007", "C-008", "P-014", "P-021"]);
    }

    #[test]
    fn a_malformed_extension_token_is_an_error_not_a_row() {
        assert_eq!(
            extensions("Extensions: `, cargo, so."),
            Err("not an extension: \"`\"".to_string())
        );
        assert_eq!(
            extensions("Extensions: PDF, doc."),
            Err("not an extension: \"PDF\"".to_string())
        );
        assert_eq!(
            extensions("Extensions: doc docx, odt."),
            Err("not an extension list entry: \"doc docx\"".to_string())
        );
        assert_eq!(
            extensions("Extensions: pdf and xps."),
            Err("not an extension list entry: \"pdf and xps\"".to_string())
        );
        assert_eq!(
            extensions("Extensions: dat (a) and prose (b)."),
            Err("not an extension list entry: \"dat (a) and prose (b)\"".to_string())
        );
        assert_eq!(
            extensions("Extensions: dat (a (b))."),
            Err("not an extension list entry: \"dat (a (b))\"".to_string())
        );
        assert!(render_formats("## C-001: Bad\n- **Family:** bad\n- **Description:** Extensions: pdf and xps.\n").is_err());
        assert!(is_extension_token("tar.bz2"));
        assert!(is_extension_token("dvr-ms"));
        assert!(is_extension_token("3fr"));
        assert!(!is_extension_token(""));
        assert!(!is_extension_token(".hidden"));
        assert!(!is_extension_token("a b"));
    }

    #[test]
    fn a_family_needs_one_well_formed_slug() {
        let no_slug = "## C-001: Images\n- **Description:** Extensions: png.\n";
        assert_eq!(
            families(no_slug).err(),
            Some("C-001: missing or malformed Family slug \"\"".to_string())
        );
        let bad_slug = "## C-001: Images\n- **Family:** Camera Raw\n- **Description:** Extensions: png.\n";
        assert_eq!(
            families(bad_slug).err(),
            Some("C-001: missing or malformed Family slug \"Camera Raw\"".to_string())
        );
        let twice = "## C-001: A\n- **Family:** images\n- **Description:** Extensions: png.\n\
                     ## C-002: B\n- **Family:** images\n- **Description:** Extensions: jpg.\n";
        assert_eq!(
            families(twice).err(),
            Some("C-002: Family slug \"images\" is used twice".to_string())
        );
        for slug in ["text", "camera-raw", "pdf-xps", "mp3-audio"] {
            assert!(is_family_slug(slug), "{slug}");
        }
        for slug in ["", "-", "camera--raw", "Camera", "camera raw", "camera_raw", "raw-"] {
            assert!(!is_family_slug(slug), "{slug}");
        }
        assert_eq!(camel_case("camera-raw"), "CameraRaw");
        assert_eq!(camel_case("pdf-xps"), "PdfXps");
        assert_eq!(camel_case("text"), "Text");
    }

    #[test]
    fn flags_follow_clause_kind_status_and_write_sentence() {
        let (c007, c008, p014, p021) = (clause("C-007"), clause("C-008"), clause("P-014"), clause("P-021"));
        // "txt now; docx is a later extension": the present grant wins, so Write mirrors Read.
        assert_eq!((read_flag(&c007), write_flag(&c007)), ("done", "done"));
        assert_eq!((read_flag(&c008), write_flag(&c008)), ("in progress", "no"));
        assert_eq!((read_flag(&p014), write_flag(&p014)), ("planned", "no"));
        assert_eq!((read_flag(&p021), write_flag(&p021)), ("planned", "planned"));
        // Denials, in every spelling the clause allows, never mirror Read.
        for denial in [
            "Write: out of scope.",
            "Write: no.",
            "Write: none.",
            "Write: not supported.",
            "Write: read-only.",
            "Write: out of scope; later maybe.",
            "Write: No.",
            "Write: None.",
            "Write: Out of scope.",
            "Write: Not supported.",
            "Write: Read-only.",
        ] {
            assert_eq!(write_flag(&with_write(denial)), "no", "{denial}");
        }
        // Deferrals.
        for deferral in [
            "Write: later.",
            "Write: Later.",
            "Write: later, once the parser is stable.",
            "Write: cell edits are a later extension.",
            "Write: cell edits are a Later Extension.",
        ] {
            assert_eq!(write_flag(&with_write(deferral)), "later", "{deferral}");
        }
        // Present grants, including ones that merely contain the letters "later".
        for grant in [
            "Write: txt now, docx later.",
            "Write: collateral edits.",
            "Write: save-as between raster formats.",
            "Write: nothing is said about no here.",
        ] {
            assert_eq!(write_flag(&with_write(grant)), "done", "{grant}");
        }
        assert_eq!(write_flag(&with_write("Nothing about writing.")), "no");
        // An empty write sentence is no sentence.
        for empty in ["Write:", "Write: .", "Write: ; docx is a later extension.", "Write:   "] {
            assert_eq!(write_flag(&with_write(empty)), "no", "{empty:?}");
        }
    }

    #[test]
    fn requested_by_names_the_origin_and_a_real_date() {
        assert_eq!(requested_by(&clause("P-014")).unwrap(), "user request 2026-10-13");
        assert_eq!(requested_by(&clause("C-008")).unwrap(), "discovery 2026-10-14");
        assert_eq!(requested_by(&clause("P-021")).unwrap(), "issue 42 2026-10-20");
        let mut dated_issue = clause("P-021");
        dated_issue.source = "ingest (issue-2026-09-01#x)".into();
        assert_eq!(requested_by(&dated_issue).unwrap(), "issue 2026-09-01");
        let mut undated = clause("C-008");
        undated.date = String::new();
        assert_eq!(requested_by(&undated), Err("C-008: no request date".to_string()));
        for empty in ["", "ingest ()", "ingest (#x)", "ingest (2026-01-02#x)"] {
            let mut no_origin = clause("C-008");
            no_origin.source = empty.into();
            assert_eq!(requested_by(&no_origin), Err("C-008: no request origin".to_string()), "{empty:?}");
        }
        let mut bad_embedded = clause("P-014");
        bad_embedded.source = "ingest (user-request-yesterday#x)".into();
        assert_eq!(requested_by(&bad_embedded).unwrap(), "user request yesterday 2026-10-14");
        assert!(is_date("2026-10-14"));
        assert!(is_date("2024-02-29"));
        assert!(!is_date("2026-1-14"));
        assert!(!is_date("yesterday"));
        for not_a_date in ["2026-13-01", "2026-00-10", "2026-04-31", "2026-02-29", "0000-00-00", "1999-12-31"] {
            assert!(!is_date(not_a_date), "{not_a_date}");
            let mut bad_date = clause("C-008");
            bad_date.date = not_a_date.into();
            assert_eq!(requested_by(&bad_date), Err("C-008: no request date".to_string()), "{not_a_date}");
        }
        assert_eq!(find_date("issue-2026-09-01"), Some("2026-09-01"));
        assert_eq!(find_date("issue-2026-99-99"), None);
        assert_eq!(find_date("plan.md"), None);
        let mut shaped_not_date = clause("C-008");
        shaped_not_date.source = "discovery-2026-99-99".into();
        assert_eq!(requested_by(&shaped_not_date).unwrap(), "discovery 2026 99 99 2026-10-14");
    }

    #[test]
    fn renders_the_expected_table_for_the_fixture() {
        let expected = "\
# Sfoglia format support

Generated from `.zen/contract.md` by `cargo run -p xtask -- formats`. Do not edit by hand: `cargo test` fails while this file is stale.

Legend: **done** = proven by a verified clause. **in progress** = clause exists, not yet verified. **planned** = pending clause. **later** = a later extension of the clause. **no** = out of scope.

4 families, 12 extensions.

| Family | Clause | Read | Write | Requested | Extensions |
|---|---|---|---|---|---|
| Word-processing documents | C-007 | done | done | user request 2026-10-13 | docx, odt, txt |
| PDF viewing | C-008 | in progress | no | discovery 2026-10-14 | pdf |
| Email messages | P-014 | planned | no | user request 2026-10-13 | eml, msg, dat, tar.gz |
| Archives | P-021 | planned | planned | issue 42 2026-10-20 | 7z, zip, tar.gz, dat |
";
        assert_eq!(render_formats(FIXTURE).unwrap(), expected);
        assert!(render_formats("## C-001: Bad\n- **Family:** bad\n- **Description:** Extensions: pdf.\n").is_err());
    }

    #[test]
    fn renders_the_expected_rust_table_for_the_fixture() {
        let expected = "\
//! Generated from `.zen/contract.md` by `cargo run -p xtask -- formats`. Do not edit.
//! One variant per format-family clause, in contract order; every extension the
//! families enumerate, sorted, with its owners in contract order.

/// A format family, as named by a clause of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Word-processing documents (C-007).
    WordProcessing,
    /// PDF viewing (C-008).
    PdfXps,
    /// Email messages (P-014).
    Email,
    /// Archives (P-021).
    Archives,
}

impl Family {
    /// Every family, in contract order.
    pub const ALL: &[Family] = &[
        Family::WordProcessing,
        Family::PdfXps,
        Family::Email,
        Family::Archives,
    ];

    /// The slug the contract names the family by.
    pub fn slug(self) -> &'static str {
        match self {
            Family::WordProcessing => \"word-processing\",
            Family::PdfXps => \"pdf-xps\",
            Family::Email => \"email\",
            Family::Archives => \"archives\",
        }
    }

    /// The clause's title.
    pub fn title(self) -> &'static str {
        match self {
            Family::WordProcessing => \"Word-processing documents\",
            Family::PdfXps => \"PDF viewing\",
            Family::Email => \"Email messages\",
            Family::Archives => \"Archives\",
        }
    }

    /// The clause that defines the family (a P- clause until its viewer is verified).
    pub fn clause(self) -> &'static str {
        match self {
            Family::WordProcessing => \"C-007\",
            Family::PdfXps => \"C-008\",
            Family::Email => \"P-014\",
            Family::Archives => \"P-021\",
        }
    }
}

/// Extension (lower case, no leading dot) to the families that list it, in contract order;
/// sorted by extension so it can be searched with `binary_search_by_key`.
pub static EXTENSIONS: &[(&str, &[Family])] = &[
    (\"7z\", &[Family::Archives]),
    (\"dat\", &[Family::Email, Family::Archives]),
    (\"docx\", &[Family::WordProcessing]),
    (\"eml\", &[Family::Email]),
    (\"msg\", &[Family::Email]),
    (\"odt\", &[Family::WordProcessing]),
    (\"pdf\", &[Family::PdfXps]),
    (\"tar.gz\", &[Family::Email, Family::Archives]),
    (\"txt\", &[Family::WordProcessing]),
    (\"zip\", &[Family::Archives]),
];
";
        assert_eq!(render_families_rs(FIXTURE).unwrap(), expected);
        assert!(render_families_rs("# Contract Items\n").is_err());
    }

    #[test]
    fn committed_checklist_matches_the_committed_contract() {
        formats(&repo_root(), true).expect("FORMATS.md and src/generated/families.rs are up to date");
    }

    #[test]
    fn generated_family_table_matches_the_committed_contract() {
        let root = repo_root();
        let contract = std::fs::read_to_string(root.join(".zen/contract.md")).unwrap();
        let committed = std::fs::read_to_string(root.join("src/generated/families.rs"))
            .expect("src/generated/families.rs exists")
            .replace("\r\n", "\n");
        assert_eq!(committed, render_families_rs(&contract).unwrap());
        // Every family slug in the contract has a variant, and the table is sorted.
        let families = families(&contract).unwrap();
        for family in &families {
            assert!(committed.contains(&format!("    {},\n", camel_case(&family.clause.family))));
        }
        let keys: Vec<&str> = committed
            .lines()
            .filter_map(|l| l.trim().strip_prefix("(\""))
            .filter_map(|l| l.split('"').next())
            .collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "extension table sorted");
    }

    #[test]
    fn committed_contract_families_are_exactly_the_clauses_that_start_with_extensions() {
        let contract = std::fs::read_to_string(repo_root().join(".zen/contract.md")).unwrap();
        let expected: Vec<String> = parse_clauses(&contract)
            .into_iter()
            .filter(|c| c.description.starts_with("Extensions:"))
            .map(|c| c.id)
            .collect();
        let families = families(&contract).expect("every committed family parses");
        let actual: Vec<String> = families.iter().map(|f| f.clause.id.clone()).collect();
        assert_eq!(actual, expected);
        assert!(!actual.is_empty(), "the contract enumerates at least one format family");
        for family in &families {
            for ext in &family.extensions {
                assert!(is_extension_token(ext), "{}: {ext:?}", family.clause.id);
            }
            requested_by(&family.clause).expect("every committed family has an origin and a date");
        }
    }
}

/// `cargo run -p xtask -- fixtures`: write the binary test fixtures the tests read, so they can be rebuilt
/// from source instead of being hand-made (C-013: tests/fixtures/book.xlsx).
fn fixtures(root: &Path) -> Result<(), String> {
    use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, FormatAlign, Formula, Workbook};
    // A fixed creation time, so the generated bytes are the same on every run.
    let made = ExcelDateTime::from_ymd(2026, 10, 18).map_err(|e: rust_xlsxwriter::XlsxError| e.to_string())?;
    let properties = DocProperties::new().set_creation_datetime(&made);
    let mut book = Workbook::new();
    book.set_properties(&properties);
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let stamp = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
    let money = Format::new().set_num_format("0.00");
    let bold = Format::new().set_bold();
    let centre = Format::new().set_align(FormatAlign::Center).set_bold();
    let err = |e: rust_xlsxwriter::XlsxError| e.to_string();

    let data = book.add_worksheet();
    data.set_name("Data").map_err(err)?;
    for (col, head) in ["id", "name", "amount", "when", "paid", "note"].iter().enumerate() {
        data.write_string_with_format(0, col as u16, *head, &bold).map_err(err)?;
    }
    data.write_number(1, 0, 1.0).map_err(err)?;
    data.write_string(1, 1, "Rossi, Mario").map_err(err)?;
    data.write_number_with_format(1, 2, 12.5, &money).map_err(err)?;
    data.write_datetime_with_format(1, 3, &ExcelDateTime::from_ymd(2026, 1, 5).map_err(err)?, &date).map_err(err)?;
    data.write_boolean(1, 4, true).map_err(err)?;
    data.write_string(1, 5, "first line\nsecond line").map_err(err)?;
    data.write_number(2, 0, 2.0).map_err(err)?;
    data.write_string(2, 1, "Élodie").map_err(err)?;
    data.write_number_with_format(2, 2, 7.0, &money).map_err(err)?;
    data.write_datetime_with_format(
        2,
        3,
        &ExcelDateTime::from_ymd(2026, 2, 28).map_err(err)?.and_hms(14, 30, 0).map_err(err)?,
        &stamp,
    )
    .map_err(err)?;
    data.write_boolean(2, 4, false).map_err(err)?;
    data.write_number(3, 0, 3.0).map_err(err)?;
    data.write_string(3, 1, "quote \"inside\"").map_err(err)?;
    data.write_number_with_format(3, 2, -3.25, &money).map_err(err)?;
    // A formula with its cached result, as Excel saves it.
    data.write_formula(4, 2, Formula::new("=SUM(C2:C4)").set_result("16.25")).map_err(err)?;
    data.write_string(4, 1, "total").map_err(err)?;
    // An error cell, as Excel stores a division by zero (the legacy saves predate it and read it blank).
    data.write_formula(4, 5, Formula::new("=1/0").set_result("#DIV/0!")).map_err(err)?;
    // A merged title row under the data and a column width Excel would show.
    data.merge_range(6, 0, 6, 2, "merged across three", &centre).map_err(err)?;
    data.set_column_width(1, 24.0).map_err(err)?;

    let notes = book.add_worksheet();
    notes.set_name("Notes").map_err(err)?;
    notes.write_string(0, 0, "a note").map_err(err)?;
    notes.write_number(1, 1, 42.0).map_err(err)?;

    let empty = book.add_worksheet();
    empty.set_name("Empty").map_err(err)?;

    let path = root.join("tests").join("fixtures").join("book.xlsx");
    book.save(&path).map_err(err)?;
    println!("wrote {}", path.display());

    // A second, different book: a reload test tells a re-read from a re-show by it.
    let mut other = Workbook::new();
    other.set_properties(&properties);
    let only = other.add_worksheet();
    only.set_name("Only").map_err(err)?;
    only.write_string(0, 0, "changed").map_err(err)?;
    let path = root.join("tests").join("fixtures").join("book2.xlsx");
    other.save(&path).map_err(err)?;
    println!("wrote {}", path.display());
    Ok(())
}
