//! C-005: syntax highlighting produces exact token spans for a small Rust and a small
//! JSON fixture, through the kit's tree-sitter highlighter and the default light theme.

use gpui_kit::HighlightStyle;
use gpui_kit::component::highlighter::{HighlightTheme, Language, SyntaxHighlighter};
use gpui_kit::component::input::Rope;
use sfoglia::text::LANGUAGES;

const RUST: &str = "fn main() {\n    let answer: u32 = 42;\n    println!(\"answer = {answer}\");\n}\n";
const JSON: &str = "{\n  \"name\": \"Sfoglia\",\n  \"families\": 17,\n  \"open\": true\n}\n";

/// The highlighted spans of `code`: every span the engine emits, as its text and style.
fn spans(language: &str, code: &str) -> Vec<(String, HighlightStyle)> {
    let theme = HighlightTheme::default_light();
    let rope = Rope::from(code);
    let mut highlighter = SyntaxHighlighter::new(language);
    assert!(highlighter.update(None, &rope, None), "the parse ran to completion");
    highlighter
        .styles(&(0..code.len()), &*theme)
        .into_iter()
        .map(|(range, style)| (code[range].to_string(), style))
        .collect()
}

/// The exact spans expected: the text of each span and the theme name that styles it,
/// `None` for unstyled text between tokens.
fn expected(parts: &[(&str, Option<&str>)]) -> Vec<(String, HighlightStyle)> {
    let theme = HighlightTheme::default_light();
    parts
        .iter()
        .map(|(text, name)| {
            let style = name
                .map(|n| theme.style(n).unwrap_or_else(|| panic!("the theme styles {n}")))
                .unwrap_or_default();
            (text.to_string(), style)
        })
        .collect()
}

#[test]
fn every_language_mapping_names_a_known_grammar() {
    for language in LANGUAGES {
        let resolved = Language::from_str(language);
        assert_eq!(resolved.name(), *language, "the kit must know {language}");
        let highlighter = SyntaxHighlighter::new(language);
        assert_eq!(highlighter.language().as_ref(), *language);
    }
    assert_eq!(Language::from_str("no-such-language").name(), "text");
    assert_eq!(Language::from_str("text").name(), "text");
}

#[test]
fn rust_and_json_highlight_with_the_exact_token_spans() {
    let rust = expected(&[
        ("fn", Some("keyword")),
        (" ", None),
        ("main", Some("function")),
        ("(", None),
        (")", None),
        (" {", None),
        ("\n    ", None),
        ("let", Some("keyword")),
        (" answer:", None),
        (" ", None),
        ("u32", Some("type")),
        (" = ", None),
        ("42", Some("constant")),
        (";", None),
        ("\n    ", None),
        ("println", Some("function.macro")),
        ("!", Some("function.macro")),
        ("(", None),
        ("\"answer = {answer}\"", Some("string")),
        (")", None),
        (";", None),
        ("\n}", None),
        ("\n", None),
    ]);
    assert_eq!(spans("rust", RUST), rust);
    // The spans tile the whole text in order.
    let tiled: String = spans("rust", RUST).into_iter().map(|(t, _)| t).collect();
    assert_eq!(tiled, RUST);

    let json = expected(&[
        ("{", None),
        ("\n  ", None),
        ("\"name\"", Some("property")),
        (":", None),
        (" ", None),
        ("\"Sfoglia\"", Some("string")),
        (",", None),
        ("\n  ", None),
        ("\"families\"", Some("property")),
        (":", None),
        (" ", None),
        ("17", Some("number")),
        (",", None),
        ("\n  ", None),
        ("\"open\"", Some("property")),
        (":", None),
        (" ", None),
        ("true", Some("boolean")),
        ("\n}", None),
        ("\n", None),
    ]);
    assert_eq!(spans("json", JSON), json);

    // Plain text is one unstyled span, and a language with no grammar behaves the same.
    let plain = vec![(RUST.to_string(), HighlightStyle::default())];
    assert_eq!(spans("text", RUST), plain);
    assert_eq!(spans("no-such-language", RUST), plain);

    // The dark theme styles the same spans with its own colours.
    let dark = HighlightTheme::default_dark();
    let light = HighlightTheme::default_light();
    assert_ne!(dark.style("keyword"), light.style("keyword"));
    assert!(dark.style("keyword").is_some() && dark.style("string").is_some());
}
