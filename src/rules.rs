//! User-defined highlighting rules (C-006): regular expressions with a style and a
//! priority, grouped in sets scoped by file-name globs, read from a TOML file.
//!
//! ```toml
//! [[set]]
//! name = "logs"
//! files = ["*.log", "plc_*.txt"]
//!
//! [[set.rule]]
//! name = "error"
//! pattern = "(?i)\\berror\\b.*"
//! priority = 10
//! foreground = "#ff3b30"
//! bold = true
//! ```
//!
//! This module knows nothing about the window: it turns a file's text into styled
//! spans, and the text viewer paints them.

use std::collections::BTreeMap;
use std::ops::Range;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use toml::Spanned;

/// The name of the rule file inside the configuration directory.
pub const FILE_NAME: &str = "highlight-rules.toml";

/// Where the rule file lives: `highlight-rules.toml` under `sfoglia` in the user's
/// configuration directory (`%APPDATA%` on Windows, `~/.config` on Linux).
pub fn default_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("sfoglia").join(FILE_NAME))
}

/// The rule file a first run starts from: every field shown once, as comments.
pub const STARTER: &str = r##"# Sfoglia highlighting rules. Each [[set]] applies to the files its globs name;
# each [[set.rule]] is a regular expression with a style and a priority.
# Where matches overlap, the higher priority wins; at equal priority the earlier
# start wins, then the longer match. Colours are #rgb, #rrggbb or #rrggbbaa.
# ^ and $ match at line boundaries. Saved on every change.

[[set]]
name = "logs"
files = ["*.log"]

[[set.rule]]
name = "error"
pattern = "(?i)\\b(error|fatal)\\b.*"
priority = 10
foreground = "#ff3b30"
bold = true

[[set.rule]]
name = "warning"
pattern = "(?i)\\bwarn(ing)?\\b.*"
priority = 5
foreground = "#ff9f0a"

[[set.rule]]
name = "timestamp"
pattern = "^\\d{4}-\\d\\d-\\d\\d[ T]\\d\\d:\\d\\d:\\d\\d"
priority = 20
underline = true
"##;

/// A colour given as `#rgb`, `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub fn parse(text: &str) -> Result<Color, String> {
        let Some(hex) = text.trim().strip_prefix('#') else {
            return Err(format!("colour `{text}` must start with `#`"));
        };
        let digit = |i: usize| -> Result<u8, String> {
            hex.as_bytes()
                .get(i)
                .and_then(|b| (*b as char).to_digit(16))
                .map(|d| d as u8)
                .ok_or_else(|| format!("colour `{text}` is not hexadecimal"))
        };
        let byte = |i: usize| -> Result<u8, String> { Ok(digit(i)? * 16 + digit(i + 1)?) };
        match hex.len() {
            3 => Ok(Color { r: digit(0)? * 17, g: digit(1)? * 17, b: digit(2)? * 17, a: 255 }),
            6 => Ok(Color { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: 255 }),
            8 => Ok(Color { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: byte(6)? }),
            _ => Err(format!("colour `{text}` must be #rgb, #rrggbb or #rrggbbaa")),
        }
    }
}

/// How a rule paints its matches. Unset colours leave the syntax colour in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub pattern: String,
    pub priority: i64,
    pub style: Style,
    /// The line of the rule's `[[set.rule]]` header in the file.
    pub line: usize,
    regex: Regex,
}

#[derive(Debug, Clone)]
pub struct RuleSet {
    pub name: String,
    /// The globs as written, valid ones only.
    pub files: Vec<String>,
    pub rules: Vec<Rule>,
    /// The line of the set's `[[set]]` header in the file.
    pub line: usize,
    matcher: GlobSet,
}

impl RuleSet {
    /// Whether this set applies to a file of this name (case-insensitively, on the
    /// name alone, never the directory).
    pub fn applies_to(&self, file_name: &str) -> bool {
        self.matcher.is_match(file_name)
    }
}

/// Something in the rule file that could not be used, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// 1-based line in the rule file.
    pub line: usize,
    pub message: String,
}

impl Problem {
    pub fn label(&self) -> String {
        format!("line {}: {}", self.line, self.message)
    }
}

/// The parsed rule file: every usable set, plus every problem found on the way.
#[derive(Debug, Clone, Default)]
pub struct RuleFile {
    pub sets: Vec<RuleSet>,
    pub problems: Vec<Problem>,
}

/// One styled piece of text after overlap resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub set: usize,
    pub rule: usize,
}

/// A match before overlap resolution: its range, its priority and the declaration
/// order of its rule (sets first, then rules), the final tiebreak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub range: Range<usize>,
    pub priority: i64,
    pub set: usize,
    pub rule: usize,
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    set: Vec<Spanned<SetDocument>>,
}

#[derive(Deserialize)]
struct SetDocument {
    name: Option<String>,
    #[serde(default)]
    files: OneOrMany,
    #[serde(default)]
    rule: Vec<Spanned<toml::Table>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl Default for OneOrMany {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(one) => vec![one],
            OneOrMany::Many(many) => many,
        }
    }
}

/// A rule's typed fields; `name` is read from the table itself so that it names the
/// rule in problems even when the other fields do not parse.
#[derive(Deserialize)]
struct RuleDocument {
    pattern: String,
    #[serde(default)]
    priority: i64,
    foreground: Option<String>,
    background: Option<String>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    underline: bool,
}

const RULE_FIELDS: [&str; 8] =
    ["name", "pattern", "priority", "foreground", "background", "bold", "italic", "underline"];

/// 1-based line of a byte offset in `source`.
fn line_of(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    source[..offset].matches('\n').count() + 1
}

impl RuleFile {
    /// Parse the file. A syntax error, or a set whose fields have the wrong type, is
    /// reported with its line and loads nothing; a rule that cannot be used (bad
    /// pattern, colour or field type) is reported with the line of its header and the
    /// other rules load; an unknown field is reported and the rule loads without it.
    pub fn parse(source: &str) -> RuleFile {
        let mut file = RuleFile::default();
        let document: Document = match toml::from_str(source) {
            Ok(document) => document,
            Err(error) => {
                let line = error.span().map_or(1, |span| line_of(source, span.start));
                file.problems.push(Problem { line, message: error.message().to_string() });
                return file;
            }
        };
        for (set_index, set) in document.set.into_iter().enumerate() {
            let set_line = line_of(source, set.span().start);
            let set = set.into_inner();
            let name = set.name.unwrap_or_else(|| format!("set {}", set_index + 1));
            let mut files = Vec::new();
            let mut matcher = GlobSetBuilder::new();
            for glob in set.files.into_vec() {
                match GlobBuilder::new(&glob).case_insensitive(true).build() {
                    Ok(built) => {
                        matcher.add(built);
                        files.push(glob);
                    }
                    Err(error) => file.problems.push(Problem {
                        line: set_line,
                        message: format!("set `{name}`: glob `{glob}`: {error}"),
                    }),
                }
            }
            let matcher = match matcher.build() {
                Ok(matcher) => matcher,
                Err(error) => {
                    file.problems.push(Problem { line: set_line, message: error.to_string() });
                    continue;
                }
            };
            let mut rules = Vec::new();
            for (rule_index, rule) in set.rule.into_iter().enumerate() {
                let line = line_of(source, rule.span().start);
                let table = rule.into_inner();
                let unknown: Vec<&String> =
                    table.keys().filter(|k| !RULE_FIELDS.contains(&k.as_str())).collect();
                let rule_name = table
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map_or_else(|| format!("rule {}", rule_index + 1), str::to_string);
                let label = format!("set `{name}`, rule `{rule_name}`");
                for field in unknown {
                    file.problems.push(Problem {
                        line,
                        message: format!("{label}: unknown field `{field}` is ignored"),
                    });
                }
                let parsed: RuleDocument = match table.try_into() {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        file.problems.push(Problem {
                            line,
                            message: format!("{label}: {}", error.message()),
                        });
                        continue;
                    }
                };
                let mut usable = true;
                let mut colour = |text: Option<String>| -> Option<Color> {
                    let text = text?;
                    match Color::parse(&text) {
                        Ok(colour) => Some(colour),
                        Err(message) => {
                            file.problems.push(Problem { line, message: format!("{label}: {message}") });
                            usable = false;
                            None
                        }
                    }
                };
                let style = Style {
                    foreground: colour(parsed.foreground),
                    background: colour(parsed.background),
                    bold: parsed.bold,
                    italic: parsed.italic,
                    underline: parsed.underline,
                };
                let regex = match RegexBuilder::new(&parsed.pattern).multi_line(true).build() {
                    Ok(regex) => regex,
                    Err(error) => {
                        // The regex crate draws its errors over several lines; one line will do.
                        let message = error.to_string().split_whitespace().collect::<Vec<_>>().join(" ");
                        file.problems.push(Problem { line, message: format!("{label}: {message}") });
                        continue;
                    }
                };
                if !usable {
                    continue;
                }
                rules.push(Rule {
                    name: rule_name,
                    pattern: parsed.pattern,
                    priority: parsed.priority,
                    style,
                    line,
                    regex,
                });
            }
            file.sets.push(RuleSet { name, files, rules, line: set_line, matcher });
        }
        file
    }

    /// The indices of the sets that apply to a file of this name, in file order.
    pub fn sets_for(&self, file_name: &str) -> Vec<usize> {
        (0..self.sets.len()).filter(|&i| self.sets[i].applies_to(file_name)).collect()
    }

    /// Every match of every applicable rule in `text`, before overlap resolution.
    /// Empty matches are not candidates.
    pub fn candidates(&self, file_name: &str, text: &str) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        for set_index in self.sets_for(file_name) {
            for (rule_index, rule) in self.sets[set_index].rules.iter().enumerate() {
                for found in rule.regex.find_iter(text) {
                    if found.range().is_empty() {
                        continue;
                    }
                    candidates.push(Candidate {
                        range: found.range(),
                        priority: rule.priority,
                        set: set_index,
                        rule: rule_index,
                    });
                }
            }
        }
        candidates
    }

    /// The styled spans of `text` for a file of this name: every applicable rule's
    /// matches, overlaps resolved.
    pub fn spans(&self, file_name: &str, text: &str) -> Vec<Span> {
        resolve(self.candidates(file_name, text))
    }

    pub fn rule(&self, span: &Span) -> &Rule {
        &self.sets[span.set].rules[span.rule]
    }
}

/// Resolve overlaps: the higher priority wins; at equal priority the earlier start,
/// then the longer match, then the rule declared first. The winner keeps its whole
/// range; a loser keeps only what no winner covers, which may split it. The result is
/// sorted by start and never overlaps.
pub fn resolve(mut candidates: Vec<Candidate>) -> Vec<Span> {
    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.range.start.cmp(&b.range.start))
            .then(b.range.len().cmp(&a.range.len()))
            .then((a.set, a.rule).cmp(&(b.set, b.rule)))
    });
    // Occupied ranges, disjoint, keyed by start.
    let mut occupied: BTreeMap<usize, usize> = BTreeMap::new();
    let mut spans = Vec::new();
    for candidate in candidates {
        let Range { start, end } = candidate.range;
        // The occupied ranges that overlap [start, end): contiguous in the map, ending
        // at the last one that starts before `end`.
        let overlapping: Vec<(usize, usize)> = occupied
            .range(..end)
            .rev()
            .take_while(|&(_, &taken_end)| taken_end > start)
            .map(|(&s, &e)| (s, e))
            .collect();
        let mut free = Vec::new();
        let mut cursor = start;
        for (taken_start, taken_end) in overlapping.into_iter().rev() {
            if taken_start > cursor {
                free.push(cursor..taken_start);
            }
            cursor = cursor.max(taken_end);
        }
        if cursor < end {
            free.push(cursor..end);
        }
        for range in free {
            occupied.insert(range.start, range.end);
            spans.push(Span { range, set: candidate.set, rule: candidate.rule });
        }
    }
    spans.sort_by_key(|span| span.range.start);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(range: Range<usize>, priority: i64, rule: usize) -> Candidate {
        Candidate { range, priority, set: 0, rule }
    }

    fn pieces(spans: &[Span]) -> Vec<(Range<usize>, usize)> {
        spans.iter().map(|s| (s.range.clone(), s.rule)).collect()
    }

    #[test]
    fn overlap_resolution_keeps_the_winner_whole_and_clips_the_loser() {
        // Nested: a higher priority inside a lower one splits the outer in two.
        let spans = resolve(vec![candidate(0..20, 1, 0), candidate(5..10, 2, 1)]);
        assert_eq!(pieces(&spans), [(0..5, 0), (5..10, 1), (10..20, 0)]);
        // Nested the other way: the lower priority inside is hidden entirely.
        let spans = resolve(vec![candidate(0..20, 2, 0), candidate(5..10, 1, 1)]);
        assert_eq!(pieces(&spans), [(0..20, 0)]);
        // Partial overlap: the loser keeps its tail.
        let spans = resolve(vec![candidate(0..10, 1, 0), candidate(5..15, 3, 1)]);
        assert_eq!(pieces(&spans), [(0..5, 0), (5..15, 1)]);
        let spans = resolve(vec![candidate(0..10, 3, 0), candidate(5..15, 1, 1)]);
        assert_eq!(pieces(&spans), [(0..10, 0), (10..15, 1)]);
        // Adjacent matches do not overlap: both show whole.
        let spans = resolve(vec![candidate(0..5, 1, 0), candidate(5..10, 9, 1)]);
        assert_eq!(pieces(&spans), [(0..5, 0), (5..10, 1)]);
        // Equal priority: the earlier start wins, then the longer match.
        let spans = resolve(vec![candidate(3..12, 5, 1), candidate(0..6, 5, 0)]);
        assert_eq!(pieces(&spans), [(0..6, 0), (6..12, 1)]);
        let spans = resolve(vec![candidate(4..8, 5, 0), candidate(4..20, 5, 1)]);
        assert_eq!(pieces(&spans), [(4..20, 1)]);
        // Identical range and priority: the rule declared first wins.
        let spans = resolve(vec![candidate(4..8, 5, 1), candidate(4..8, 5, 0)]);
        assert_eq!(pieces(&spans), [(4..8, 0)]);
        // A loser under several winners is cut into every free piece.
        let spans = resolve(vec![
            candidate(0..30, 0, 0),
            candidate(2..4, 1, 1),
            candidate(10..12, 1, 2),
            candidate(28..30, 1, 3),
        ]);
        assert_eq!(
            pieces(&spans),
            [(0..2, 0), (2..4, 1), (4..10, 0), (10..12, 2), (12..28, 0), (28..30, 3)]
        );
        // Priority is an integer: a negative one loses to zero.
        let spans = resolve(vec![candidate(0..4, -1, 0), candidate(2..6, 0, 1)]);
        assert_eq!(pieces(&spans), [(0..2, 0), (2..6, 1)]);
        assert_eq!(resolve(Vec::new()), Vec::<Span>::new());
    }

    const RULES: &str = r##"
[[set]]
name = "logs"
files = ["*.log", "plc_*.txt"]

[[set.rule]]
name = "error"
pattern = "ERROR.*"
priority = 1
foreground = "#f00"

[[set.rule]]
name = "number"
pattern = "\\d+"
priority = 5
bold = true

[[set]]
name = "everything"
files = "*"

[[set.rule]]
name = "todo"
pattern = "TODO"
priority = 9
background = "#ffff0080"
italic = true
underline = true
"##;

    #[test]
    fn glob_scoping_picks_the_sets_that_name_the_file() {
        let file = RuleFile::parse(RULES);
        assert_eq!(file.problems, Vec::new());
        assert_eq!(file.sets.len(), 2);
        assert_eq!(file.sets[0].files, ["*.log", "plc_*.txt"]);
        assert_eq!(file.sets[1].files, ["*"], "a single glob may be written as a string");
        assert_eq!(file.sets_for("app.log"), [0, 1]);
        assert_eq!(file.sets_for("APP.LOG"), [0, 1], "globs match case-insensitively");
        assert_eq!(file.sets_for("plc_7.txt"), [0, 1]);
        assert_eq!(file.sets_for("notes.txt"), [1]);
        assert_eq!(file.sets_for("main.rs"), [1]);
        let only_logs = RuleFile::parse(&RULES[..RULES.find("[[set]]\nname = \"everything\"").unwrap()]);
        assert_eq!(only_logs.sets_for("main.rs"), Vec::<usize>::new());
        assert_eq!(only_logs.spans("main.rs", "ERROR 1"), Vec::new());
    }

    #[test]
    fn spans_come_from_every_applicable_set_with_the_rule_styles() {
        let file = RuleFile::parse(RULES);
        let text = "ERROR 42 at TODO\nok 7\n";
        let spans = file.spans("app.log", text);
        let shown: Vec<(&str, &str)> = spans
            .iter()
            .map(|s| (&text[s.range.clone()], file.rule(s).name.as_str()))
            .collect();
        assert_eq!(
            shown,
            [("ERROR ", "error"), ("42", "number"), (" at ", "error"), ("TODO", "todo"), ("7", "number")]
        );
        let error = file.rule(&spans[0]);
        assert_eq!(error.style, Style { foreground: Some(Color { r: 255, g: 0, b: 0, a: 255 }), ..Style::default() });
        assert_eq!(error.line, 6);
        let number = file.rule(&spans[1]);
        assert_eq!(number.style, Style { bold: true, ..Style::default() });
        assert_eq!(number.priority, 5);
        let todo = file.rule(&spans[3]);
        assert_eq!(
            todo.style,
            Style {
                background: Some(Color { r: 255, g: 255, b: 0, a: 128 }),
                italic: true,
                underline: true,
                ..Style::default()
            }
        );
        assert_eq!((todo.line, file.sets[1].line), (22, 18));
        // Only the universal set applies to a Rust file.
        let spans = file.spans("main.rs", text);
        assert_eq!(spans.iter().map(|s| s.range.clone()).collect::<Vec<_>>(), [12..16]);
    }

    #[test]
    fn anchors_match_at_line_boundaries_and_empty_matches_are_skipped() {
        let file = RuleFile::parse(
            "[[set]]\nfiles = \"*\"\n[[set.rule]]\npattern = \"^\\\\w+\"\n[[set.rule]]\npattern = \"x*\"\nname = \"empty\"\n",
        );
        assert_eq!(file.problems, Vec::new());
        assert_eq!(file.sets[0].name, "set 1", "an unnamed set is numbered");
        assert_eq!(file.sets[0].rules[1].name, "empty");
        assert_eq!(file.sets[0].rules[0].name, "rule 1", "an unnamed rule is numbered");
        let spans = file.spans("a.txt", "one two\nthree\n");
        assert_eq!(spans.iter().map(|s| s.range.clone()).collect::<Vec<_>>(), [0..3, 8..13]);
        // `x*` matches only emptily in "abc": the word rule alone paints.
        assert_eq!(file.spans("a.txt", "abc"), [Span { range: 0..3, set: 0, rule: 0 }]);
        assert_eq!(file.spans("a.txt", " -- "), Vec::new());
        assert_eq!(file.spans("a.txt", " xx"), [Span { range: 1..3, set: 0, rule: 1 }]);
    }

    #[test]
    fn a_malformed_rule_file_loads_the_valid_rules_and_reports_the_bad_ones_by_line() {
        let source = r##"[[set]]
name = "logs"
files = ["*.log", "[unclosed"]

[[set.rule]]
name = "broken"
pattern = "(unclosed"

[[set.rule]]
name = "fine"
pattern = "ok"
colour = "#fff"

[[set.rule]]
name = "wrong-type"
pattern = "x"
priority = "high"

[[set.rule]]
name = "bad-colour"
pattern = "y"
foreground = "red"

[[set.rule]]
name = "last"
pattern = "z"
priority = 3
"##;
        let file = RuleFile::parse(source);
        assert_eq!(file.sets.len(), 1);
        assert_eq!(file.sets[0].files, ["*.log"], "the broken glob is dropped, the others kept");
        let names: Vec<&str> = file.sets[0].rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["fine", "last"]);
        assert_eq!(file.sets[0].rules[0].line, 9);
        assert_eq!(file.sets[0].rules[1].line, 24);
        let lines: Vec<usize> = file.problems.iter().map(|p| p.line).collect();
        assert_eq!(lines, [1, 5, 9, 14, 19]);
        let messages: Vec<&str> = file.problems.iter().map(|p| p.message.as_str()).collect();
        assert!(messages[0].starts_with("set `logs`: glob `[unclosed`:"), "{}", messages[0]);
        assert!(messages[1].starts_with("set `logs`, rule `broken`: regex parse error"), "{}", messages[1]);
        assert_eq!(messages[2], "set `logs`, rule `fine`: unknown field `colour` is ignored");
        assert!(messages[3].starts_with("set `logs`, rule `wrong-type`: invalid type: string \"high\""), "{}", messages[3]);
        assert_eq!(messages[4], "set `logs`, rule `bad-colour`: colour `red` must start with `#`");
        assert_eq!(file.problems[1].label(), format!("line 5: {}", messages[1]));

        // A rule without a pattern is reported on its header line.
        let file = RuleFile::parse("[[set]]\nfiles = \"*\"\n\n[[set.rule]]\nname = \"nothing\"\n");
        assert_eq!(file.problems.len(), 1);
        assert_eq!(file.problems[0].line, 4);
        assert!(file.problems[0].message.contains("missing field `pattern`"), "{}", file.problems[0].message);
        assert!(file.sets[0].rules.is_empty());

        // A syntax error loads nothing and names its line.
        let file = RuleFile::parse("[[set]]\nname = \"a\"\n\n[[set.rule]]\npattern = \"x\n");
        assert!(file.sets.is_empty());
        assert_eq!(file.problems.len(), 1);
        assert_eq!(file.problems[0].line, 5, "{:?}", file.problems[0]);

        // A set field of the wrong type is a whole-file problem too, with its line.
        let file = RuleFile::parse("[[set]]\nname = 7\n");
        assert!(file.sets.is_empty());
        assert_eq!(file.problems.len(), 1);
        assert_eq!(file.problems[0].line, 2, "{:?}", file.problems[0]);

        // An empty file is no rules and no problems; so is the starter file.
        assert!(RuleFile::parse("").sets.is_empty());
        assert!(RuleFile::parse("").problems.is_empty());
        let starter = RuleFile::parse(STARTER);
        assert_eq!(starter.problems, Vec::new());
        assert_eq!(starter.sets.len(), 1);
        assert_eq!(starter.sets[0].rules.len(), 3);
    }

    #[test]
    fn colours_parse_in_three_lengths() {
        assert_eq!(Color::parse("#fff"), Ok(Color { r: 255, g: 255, b: 255, a: 255 }));
        assert_eq!(Color::parse("#1A2b3C"), Ok(Color { r: 26, g: 43, b: 60, a: 255 }));
        assert_eq!(Color::parse(" #00000080 "), Ok(Color { r: 0, g: 0, b: 0, a: 128 }));
        assert_eq!(Color::parse("fff"), Err("colour `fff` must start with `#`".into()));
        assert_eq!(Color::parse("#ffff"), Err("colour `#ffff` must be #rgb, #rrggbb or #rrggbbaa".into()));
        assert_eq!(Color::parse("#ggg"), Err("colour `#ggg` is not hexadecimal".into()));
    }
}
