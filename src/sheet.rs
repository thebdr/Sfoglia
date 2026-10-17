//! Spreadsheets and delimited tables (C-013), the pure part: a delimited text parsed into a grid of cells with
//! each record's byte span (so an edit can be written back into the text as it stands), and an Excel workbook
//! read into sheets of typed cells through the `calamine` crate.

use std::ops::Range;
use std::path::Path;

use calamine::{Data, Reader, Sheets};

/// A grid of text cells, as a delimited file holds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    /// The records, each its fields as written (quotes and escapes undone).
    pub rows: Vec<Vec<String>>,
    /// Each record's byte span in the text it was parsed from, the line terminator left out.
    pub spans: Vec<Range<usize>>,
    /// The widest record's field count.
    pub columns: usize,
    pub delimiter: u8,
    /// The line terminator the text uses (`\r\n` when its first line ends so, `\n` otherwise).
    pub crlf: bool,
}

/// The delimiter a text is read with: a `.tsv` is tabs; otherwise the candidate (comma, semicolon, tab, pipe)
/// that splits the first lines into the most fields the same way, commas winning a tie.
pub fn detect_delimiter(text: &str, file_name: &str) -> u8 {
    if file_name.to_ascii_lowercase().ends_with(".tsv") {
        return b'\t';
    }
    let records = record_counts(text, 20);
    let mut best = (b',', 0usize, 0usize);
    for (ix, _) in CANDIDATES.iter().enumerate() {
        let candidate = CANDIDATES[ix];
        let counts: Vec<usize> = records.iter().map(|r| r[ix]).collect();
        // The count most lines share (a title line in front of the data does not decide), the larger
        // count on a tie; a candidate no line holds is out.
        let mut tally: Vec<(usize, usize)> = Vec::new();
        for &c in counts.iter().filter(|&&c| c > 0) {
            match tally.iter_mut().find(|(count, _)| *count == c) {
                Some(entry) => entry.1 += 1,
                None => tally.push((c, 1)),
            }
        }
        let Some(&(common, agreeing)) = tally.iter().max_by_key(|&&(count, lines)| (lines, count)) else {
            continue;
        };
        // More agreement, then more fields, win; an earlier candidate wins a tie.
        if (agreeing, common) > (best.2, best.1) {
            best = (candidate, common, agreeing);
        }
    }
    best.0
}

const CANDIDATES: [u8; 4] = [b',', b';', b'\t', b'|'];

/// For the first `limit` records (a record ending at a line break outside quotes, so a quoted field spanning
/// lines stays one record), how many of each candidate delimiter stand outside quotes; blank records left out.
fn record_counts(text: &str, limit: usize) -> Vec<[usize; 4]> {
    let mut records = Vec::new();
    let mut counts = [0usize; 4];
    let mut quoted = false;
    let mut blank = true;
    for b in text.bytes() {
        if b == b'"' {
            quoted = !quoted;
            blank = false;
        } else if (b == b'\n' || b == b'\r') && !quoted {
            if !blank {
                records.push(counts);
                if records.len() == limit {
                    return records;
                }
            }
            counts = [0; 4];
            blank = true;
        } else {
            if !b.is_ascii_whitespace() || b == b'\t' {
                blank = false;
            }
            if !quoted && let Some(ix) = CANDIDATES.iter().position(|&c| c == b) {
                counts[ix] += 1;
            }
        }
    }
    if !blank {
        records.push(counts);
    }
    records
}

/// Parse `text` with `delimiter`: RFC 4180 quoting (a quoted field may hold the delimiter, line breaks and
/// doubled quotes), ragged records kept as they are, a final line without a terminator included.
pub fn parse_delimited(text: &str, delimiter: u8) -> Grid {
    let crlf = text.split_once('\n').map(|(first, _)| first.ends_with('\r')).unwrap_or(false);
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .quoting(true)
        .from_reader(text.as_bytes());
    let bytes = text.as_bytes();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let mut columns = 0;
    let mut record = csv::StringRecord::new();
    while reader.read_record(&mut record).unwrap_or(false) {
        let fields: Vec<String> = record.iter().map(str::to_string).collect();
        // The reader's own position may sit on the line break that ended the record before: the record
        // itself starts after any break.
        let mut start = record.position().map(|p| p.byte() as usize).unwrap_or(0).min(bytes.len());
        while start < bytes.len() && (bytes[start] == b'\n' || bytes[start] == b'\r') {
            start += 1;
        }
        starts.push(start);
        rows.push(fields);
    }
    // Each record's span runs to the next record's start, the line break taken off.
    let mut spans = Vec::with_capacity(rows.len());
    for (ix, &start) in starts.iter().enumerate() {
        let mut end = starts.get(ix + 1).copied().unwrap_or(bytes.len()).max(start);
        while end > start && (bytes[end - 1] == b'\n' || bytes[end - 1] == b'\r') {
            end -= 1;
        }
        columns = columns.max(rows[ix].len());
        spans.push(start..end);
    }
    Grid { rows, spans, columns, delimiter, crlf }
}

/// `fields` written as one record in the grid's dialect: quoted only where needed, no terminator.
pub fn format_record(fields: &[String], delimiter: u8) -> String {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .quote_style(csv::QuoteStyle::Necessary)
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    writer.write_record(fields).ok();
    let mut bytes = writer.into_inner().unwrap_or_default();
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    String::from_utf8(bytes).unwrap_or_default()
}

/// What a cell holds, for sorting and for the look of the grid.
#[derive(Clone, Debug, PartialEq)]
pub enum CellKind {
    Empty,
    Text,
    Number(f64),
    Bool(bool),
    /// A date or time, `text` being its ISO form.
    Date,
    Error,
}

/// One cell of a sheet: its text as the grid shows it, and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub text: String,
    pub kind: CellKind,
}

impl Cell {
    pub fn empty() -> Self {
        Self { text: String::new(), kind: CellKind::Empty }
    }

    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        let kind = if text.is_empty() { CellKind::Empty } else { CellKind::Text };
        Self { text, kind }
    }
}

/// A merged region: its top-left cell holds the value, the others are covered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    pub rows: Range<usize>,
    pub columns: Range<usize>,
}

/// One sheet of a workbook.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
    pub columns: usize,
    pub merges: Vec<Merge>,
}

impl Sheet {
    pub fn cell(&self, row: usize, column: usize) -> Option<&Cell> {
        self.rows.get(row).and_then(|r| r.get(column))
    }

    /// The merge covering `(row, column)` whose top-left this is not, if any.
    pub fn covered_by(&self, row: usize, column: usize) -> Option<&Merge> {
        self.merges.iter().find(|m| {
            m.rows.contains(&row) && m.columns.contains(&column) && !(m.rows.start == row && m.columns.start == column)
        })
    }
}

/// A workbook: its sheets in file order, an empty sheet kept as empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
}

/// Read an Excel workbook (xls, xlsx, xlsm, xlsb and their templates) with every sheet's cells: numbers as
/// their own digits, dates and times in ISO form, booleans as `TRUE` / `FALSE`, errors as Excel writes them
/// (`#DIV/0!`), a formula as its cached value; merged regions for the xlsx family.
pub fn read_workbook(path: &Path) -> Result<Workbook, String> {
    let mut book = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    let epoch_1904 = match &book {
        Sheets::Xlsx(xlsx) => xlsx.has_1904_epoch(),
        Sheets::Xls(xls) => xls.has_1904_epoch(),
        _ => false,
    };
    let names = book.sheet_names();
    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let range = book.worksheet_range(&name).map_err(|e| e.to_string())?;
        let (height, width) = range.get_size();
        let (row_offset, col_offset) = range.start().map(|(r, c)| (r as usize, c as usize)).unwrap_or((0, 0));
        // Cells before the range's first row or column are empty: the grid starts at A1 as Excel shows it.
        let columns = if width == 0 { 0 } else { col_offset + width };
        let mut rows = vec![vec![Cell::empty(); columns]; if height == 0 { 0 } else { row_offset + height }];
        for (r, row) in range.rows().enumerate() {
            for (c, data) in row.iter().enumerate() {
                rows[row_offset + r][col_offset + c] = cell_of(data, epoch_1904);
            }
        }
        let merges = match &mut book {
            Sheets::Xlsx(xlsx) => xlsx
                .merge_cells_by_sheet_name(&name)
                .map(|dims| {
                    dims.iter()
                        .map(|d| Merge {
                            rows: d.start.0 as usize..d.end.0 as usize + 1,
                            columns: d.start.1 as usize..d.end.1 as usize + 1,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        sheets.push(Sheet { name, rows, columns, merges });
    }
    Ok(Workbook { sheets })
}

fn cell_of(data: &Data, epoch_1904: bool) -> Cell {
    match data {
        Data::Empty => Cell::empty(),
        Data::String(s) => Cell::text(s.clone()),
        Data::Int(i) => Cell { text: i.to_string(), kind: CellKind::Number(*i as f64) },
        Data::Float(f) => Cell { text: format_number(*f), kind: CellKind::Number(*f) },
        Data::Bool(b) => Cell { text: if *b { "TRUE" } else { "FALSE" }.to_string(), kind: CellKind::Bool(*b) },
        Data::DateTime(dt) => {
            let text = if dt.is_duration() { format_duration(dt.as_f64()) } else { format_serial(dt.as_f64(), epoch_1904) };
            Cell { text, kind: CellKind::Date }
        }
        Data::DateTimeIso(s) => Cell { text: iso_text(s), kind: CellKind::Date },
        Data::DurationIso(s) => Cell { text: s.clone(), kind: CellKind::Date },
        Data::Error(e) => Cell { text: e.to_string(), kind: CellKind::Error },
    }
}

/// An ISO date-time as written in a workbook, its `T` between date and time shown as a space.
pub fn iso_text(text: &str) -> String {
    match text.as_bytes().get(10) {
        Some(b'T') => format!("{} {}", &text[..10], &text[11..]),
        _ => text.to_string(),
    }
}

/// A number as its own digits: an integral value without a fraction, otherwise the shortest form that
/// reads back exactly, cut to fifteen significant digits as Excel shows them (so 0.1 + 0.2 is `0.3`).
pub fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        return if value.abs() < 1e15 { format!("{}", value as i64) } else { value.to_string() };
    }
    let shortest = value.to_string();
    let digits = shortest.chars().filter(|c| c.is_ascii_digit()).count();
    if digits <= 15 && !shortest.contains('e') {
        return shortest;
    }
    // Fifteen significant digits, written out without an exponent.
    let sci = format!("{value:.14e}");
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let point = exponent + 1;
    let mut text = if point <= 0 {
        format!("0.{}{}", "0".repeat((-point) as usize), digits)
    } else if (point as usize) >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
    };
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if negative { format!("-{text}") } else { text }
}

/// An Excel serial date as `YYYY-MM-DD`, with ` HH:MM:SS` when it has a time of day; a serial below one
/// (a time alone) as `HH:MM:SS`.
pub fn format_serial(serial: f64, epoch_1904: bool) -> String {
    let base = if epoch_1904 { 24107 } else { 25569 }; // days from 1899-12-30 / 1904-01-01 to 1970-01-01
    // Rounded to whole seconds before the split, so a time just short of midnight carries into the next day.
    let total = (serial * 86_400.0).round() as i64;
    let (days, seconds) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if serial < 1.0 {
        // A serial below one is a time of day in either system (calamine cannot tell a time cell from a
        // date cell, and the 1900 system's day 0 is no day).
        return format!("{h:02}:{m:02}:{s:02}");
    }
    if !epoch_1904 && days == 60 {
        // Excel's phantom 1900-02-29, kept as Excel shows it.
        return if seconds == 0 { "1900-02-29".to_string() } else { format!("1900-02-29 {h:02}:{m:02}:{s:02}") };
    }
    // Excel counts the phantom 1900-02-29 in the 1900 system: serials from 61 on are one day ahead.
    let unix_days = if epoch_1904 { days - base } else if days >= 61 { days - base } else { days - base + 1 };
    let (y, mo, d) = civil_from_days(unix_days);
    if seconds == 0 { format!("{y:04}-{mo:02}-{d:02}") } else { format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}") }
}

fn format_duration(serial: f64) -> String {
    let total = (serial * 86_400.0).round() as i64;
    format!("{:02}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60)
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The column's letters as Excel names it (`A`, `Z`, `AA`).
pub fn column_name(index: usize) -> String {
    let mut n = index + 1;
    let mut name = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        name.push(b'A' + rem as u8);
        n = (n - 1) / 26;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

/// A cell reference as Excel writes it (`B3`).
pub fn cell_name(row: usize, column: usize) -> String {
    format!("{}{}", column_name(column), row + 1)
}

/// How two cells order under a sort: numbers before text, by value then case-insensitively by text, empty
/// cells last whatever the direction asks.
pub fn compare_cells(a: &Cell, b: &Cell) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (&a.kind, &b.kind) {
        (CellKind::Empty, CellKind::Empty) => Ordering::Equal,
        (CellKind::Empty, _) => Ordering::Greater,
        (_, CellKind::Empty) => Ordering::Less,
        (CellKind::Number(x), CellKind::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (CellKind::Number(_), _) => Ordering::Less,
        (_, CellKind::Number(_)) => Ordering::Greater,
        _ => a.text.to_lowercase().cmp(&b.text.to_lowercase()).then_with(|| a.text.cmp(&b.text)),
    }
}

/// A text cell classified for sorting: a number when it parses as one, a date when it reads as ISO.
pub fn classify(text: &str) -> Cell {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Cell::empty();
    }
    if let Ok(n) = trimmed.parse::<f64>()
        && n.is_finite()
    {
        return Cell { text: text.to_string(), kind: CellKind::Number(n) };
    }
    Cell::text(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delimited_text_parses_with_quotes_breaks_and_spans() {
        let text = "id,name\r\n1,\"Rossi, Mario\"\r\n2,\"two\r\nlines\"\r\n3,\"say \"\"hi\"\"\"\r\n4\r\n";
        let grid = parse_delimited(text, b',');
        assert_eq!(grid.rows[1], vec!["1", "Rossi, Mario"]);
        assert_eq!(grid.rows[2], vec!["2", "two\r\nlines"]);
        assert_eq!(grid.rows[3], vec!["3", "say \"hi\""]);
        assert_eq!(grid.rows[4], vec!["4"], "a ragged record is kept");
        let trailing = parse_delimited("a,b,\nc,\"d,\"\n", b',');
        assert_eq!(trailing.rows, vec![vec!["a", "b", ""], vec!["c", "d,"]], "a trailing delimiter is an empty field, a quoted one is not");
        assert_eq!(grid.columns, 2);
        assert!(grid.crlf);
        let spans: Vec<&str> = grid.spans.iter().map(|s| &text[s.clone()]).collect();
        assert_eq!(spans, ["id,name", "1,\"Rossi, Mario\"", "2,\"two\r\nlines\"", "3,\"say \"\"hi\"\"\"", "4"]);
        assert_eq!(format_record(&["a,b".into(), "plain".into(), "q\"q".into()], b','), "\"a,b\",plain,\"q\"\"q\"");
    }

    #[test]
    fn the_delimiter_is_detected_from_the_first_lines() {
        assert_eq!(detect_delimiter("a;b;c\n1;2;3\n", "x.csv"), b';');
        assert_eq!(detect_delimiter("a,b\n1,2\n", "x.csv"), b',');
        assert_eq!(detect_delimiter("a\tb\n1\t2\n", "x.txt"), b'\t');
        assert_eq!(detect_delimiter("a,b\n1,2\n", "x.tsv"), b'\t', "a .tsv is tabs whatever it holds");
        assert_eq!(detect_delimiter("\"a,b\";c\n\"1,2\";3\n", "x.csv"), b';', "a delimiter inside quotes does not count");
        assert_eq!(detect_delimiter("one\ntwo\n", "x.csv"), b',', "nothing to split: commas");
        assert_eq!(detect_delimiter("a,b;c\n1,2;3\n", "x.csv"), b',', "an even tie: commas before the rest");
        assert_eq!(detect_delimiter("a,b|c|d\n1,2|3|4\n", "x.csv"), b'|', "the same agreement, more fields: the pipe");
        assert_eq!(detect_delimiter("title\n1;2;3\n4;5;6\n", "x.csv"), b';', "a title line in front does not decide");
        assert_eq!(detect_delimiter("a;b\n1,5;2,5\n3,5;4,5\n", "x.csv"), b';', "decimal commas do not outvote the semicolons");
        assert_eq!(
            detect_delimiter("id;note;x\n1;\"a, b\nc, d\";x\n2;\"e, f\ng, h\";y\n3;\"i, j\nk, l\";z\n", "x.csv"),
            b';',
            "a quoted field spanning lines is one record, its commas inside quotes"
        );
    }

    #[test]
    fn serial_dates_and_numbers_format_as_excel_shows_them() {
        assert_eq!(format_serial(46027.0, false), "2026-01-05");
        assert_eq!(format_serial(46081.604166666664, false), "2026-02-28 14:30:00");
        assert_eq!(format_serial(1.0, false), "1900-01-01");
        assert_eq!(format_serial(60.0, false), "1900-02-29", "Excel's phantom leap day keeps its serial");
        assert_eq!(format_serial(61.0, false), "1900-03-01");
        assert_eq!(format_serial(0.5, false), "12:00:00");
        assert_eq!(format_serial(0.5, true), "12:00:00", "a time of day in the 1904 system too");
        assert_eq!(format_serial(46027.999999999, false), "2026-01-06", "a second short of midnight rounds into the next day");
        assert_eq!(format_serial(0.9999999999, false), "00:00:00", "a time alone wraps");
        assert_eq!(iso_text("2026-02-28T14:30:00"), "2026-02-28 14:30:00");
        assert_eq!(iso_text("2026-02-28"), "2026-02-28");
        assert_eq!(format_serial(1.0, true), "1904-01-02");
        assert_eq!(format_serial(44565.0, true), "2026-01-05", "the 1904 system's serial of the fixture's date (1462 days fewer)");
        assert_eq!(format_number(12.5), "12.5");
        assert_eq!(format_number(7.0), "7");
        assert_eq!(format_number(-3.25), "-3.25");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1e20), "100000000000000000000");
        assert_eq!(format_number(0.000123456789012345), "0.000123456789012345", "fifteen significant digits kept");
        assert_eq!(format_number(1e-7), "0.0000001");
        assert_eq!(format_number(-2.0 / 3.0), "-0.666666666666667", "cut to fifteen, as Excel shows it");
        assert_eq!(format_number(123456789012345678.0), "123456789012345680");
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(cell_name(2, 1), "B3");
    }
}
