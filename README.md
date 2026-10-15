# Sfoglia

*Sfoglia* (Italian: leaf through) is a desktop viewer for any file, with editing where it is worth the effort: source code with syntax highlighting plus your own regex highlighting rules, documents, PDFs, spreadsheets, images, email, CAD drawings, archives, audio and video. One Rust code base for Windows and Linux, built on [gpui-kit](https://gpui-kit.com).

## Status

Early. The application shell opens a window; everything else is planned. [FORMATS.md](FORMATS.md) is the checklist of every extension Sfoglia intends to read or write and how far each family has come.

## Open files

Give paths on the command line, press Ctrl+O for the Open dialog, or drop files onto the window. Each file opens in its own tab; opening a file that is already open just activates its tab. Sfoglia identifies a file by its extension first and then by its content, so a PNG renamed to `.txt` still opens as an image, and a binary file nobody claims opens in the hex view rather than failing. Until a family's viewer is built, its tab shows the file's name, path, size, detected family and the clause that will bring the viewer.

## Text and source files

Any text file opens in a read-only editor with line numbers; source files are highlighted by language (35 grammars, from Rust and JSON to SQL and Zig). The status row under the text names the encoding (UTF-8, UTF-8 with BOM, UTF-16 LE or BE, or Latin-1 or Windows-1252 when the bytes are not valid UTF-8), the line endings, the line count and the language. Ctrl+Shift+T switches between the light and the dark theme.

## Highlighting rules

Your own rules paint on top of the syntax colours. Ctrl+Shift+H opens the rule file in a panel on the right; every change re-highlights the open files at once and is saved to `highlight-rules.toml` under `sfoglia` in your configuration directory (`%APPDATA%` on Windows, `~/.config` on Linux). A `[[set]]` names the files it applies to with globs (`files = ["*.log", "plc_*.txt"]`, matched case-insensitively against the file name) and holds `[[set.rule]]` tables:

```toml
[[set]]
name = "logs"
files = ["*.log"]

[[set.rule]]
name = "error"
pattern = "(?i)\\b(error|fatal)\\b.*"   # Rust regex syntax; ^ and $ match at line boundaries
priority = 10                           # higher wins where matches overlap; 0 when absent
foreground = "#ff3b30"                  # #rgb, #rrggbb or #rrggbbaa
background = "#3a000080"
bold = true
italic = false
underline = false
```

Where matches overlap, the higher priority wins; at equal priority the earlier start wins, then the longer match, then the rule declared first. The winner keeps its whole match and a loser keeps only what no winner covers, so a word rule can sit inside a line rule. A rule with a bad pattern, colour or field type is listed under the editor with its line and the other rules still load; an unknown field is listed and ignored; a syntax error is listed with its line and loads nothing until it is fixed. The first run starts from a commented example.

## Build and run

Requirements: stable Rust (1.96 or newer), and on Windows the MSVC build tools with a Windows 10 or 11 SDK and CMake. On Linux install the system libraries gpui needs (see Zed's `script/linux` list: Vulkan, Wayland, X11-xcb, xkbcommon, fontconfig, ALSA, OpenSSL, zstd, clang, cmake).

```bash
cargo run
```

## Test

```bash
cargo test
```

`tests/ui.rs` renders the real window headlessly through gpui-kit's test support and asserts on what it shows. The `xtask` tests also fail when `FORMATS.md` no longer matches the contract.

## How the project is run

Every behaviour is a clause in [`.zen/contract.md`](.zen/contract.md), kept honest by the IMPLICATION→MACHINE protocol: a clause is only *verified* when a recorded test run and a screenshot of the real thing say so. `FORMATS.md` is generated from that contract:

```bash
cargo run -p xtask -- formats
```

Format requests, your own or from others, enter as pending clauses and show up in the checklist as *planned* rows.
