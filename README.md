# Sfoglia

*Sfoglia* (Italian: leaf through) is a desktop viewer for any file, with editing where it is worth the effort: source code with syntax highlighting plus your own regex highlighting rules, documents, PDFs, spreadsheets, images, email, CAD drawings, archives, audio and video. One Rust code base for Windows and Linux, built on [gpui-kit](https://gpui-kit.com).

## Status

Early. The application shell opens a window; everything else is planned. [FORMATS.md](FORMATS.md) is the checklist of every extension Sfoglia intends to read or write and how far each family has come.

## Open files

Give paths on the command line, press Ctrl+O for the Open dialog, or drop files onto the window. Each file opens in its own tab; opening a file that is already open just activates its tab. Sfoglia identifies a file by its extension first and then by its content, so a PNG renamed to `.txt` still opens as an image, and a binary file nobody claims opens in the hex view rather than failing. Until a family's viewer is built, its tab shows the file's name, path, size, detected family and the clause that will bring the viewer.

## Text and source files

Any text file opens in an editor with line numbers; source files are highlighted by language (35 grammars, from Rust and JSON to SQL and Zig). The status row under the text names the encoding (UTF-8, UTF-8 with BOM, UTF-16 LE or BE, or Latin-1 or Windows-1252 when the bytes are not valid UTF-8), the line endings, the line count and the language. Ctrl+Shift+T switches between the light and the dark theme.

## Editing and saving

The text is editable: type, cut, copy and paste, undo and redo, and find and replace with the editor's own panel (Ctrl+F, Ctrl+H). A modified tab shows • after its name and will not close until you save (Ctrl+S) or click Revert in the status row. Saving writes the file back in the encoding, byte-order mark and line endings it was read with; click the encoding or the line-endings item in the status row to choose another for the next save. A character the chosen encoding cannot hold stops the save and names the character and its line. A file with mixed line endings is written with LF once edited, as its status row says. When another program changes the file on disk, a bar offers to reload it or to keep your edits. Closing the window while tabs are modified asks about each in turn, first tab first: Save, Discard changes or Cancel, and Save all or Discard all while more remain; the window closes once every tab is saved or restored; a save or a restore that fails keeps it open with the reason, and a second Discard changes then drops that edit.

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

## Images

A raster image (PNG, APNG, JPEG, GIF, BMP, ICO, TIFF, WebP, TGA, PNM, DDS, EXR, HDR, QOI, Farbfeld) opens on a checkerboard, fitted to the view and centred. Press `+` and `-` to zoom, `1` for 100 %, `0` to fit again, `R` to turn it a quarter; scroll to pan when it is larger than the view. Animated GIF, APNG and WebP play. The side column lists the dimensions, pixel format, frame count, file size and the EXIF fields the file carries. SVG renders as vectors and stays sharp at any zoom; a TrueType font shows a specimen in its own face. Ctrl+Shift+S saves the first frame as another raster format, chosen by the extension you give the new file. Image extensions without a decoder yet (PSD, HEIC, AVIF, JPEG 2000, JXL, EMF, WMF and others) say so in their tab.

## Structured data

A JSON, JSON Lines, XML, YAML, TOML or INI file opens as a tree with key, type and value columns. A key or value of several lines shows on its one row, with ⏎ for each line break, and the key column widens for deep rows. The root is open; Enter, Space or a click on the arrow opens and closes a node, the arrow keys move and walk in and out, and the status row names the selected node's path (`$.items[2].name`, `/root/child[2]/@id`, `section.key`). Ctrl+Shift+C copies that path. Ctrl+Shift+F focuses the search field: Enter finds the next key or value containing the text, opens its ancestors and reports how many match. Ctrl+Shift+D switches to the raw text in the text viewer, where you edit and save as usual, and back to the tree rebuilt from your edit. Ctrl+Shift+P pretty-prints the text as one undoable edit. A file that does not parse opens on its text with the error's line and column in the status row and the cursor on that line. When another program changes the file, the Reload / Keep mine bar shows on the tree side too, and a Reload that brings in a text that does not parse opens on the text the same way.

## Markdown

A Markdown file opens rendered: headings, emphasis, lists (ordered, unordered and task lists), block quotes, tables, fenced code painted by its grammar, footnotes, links and images, the relative ones read from the file's own folder. A click on a web or mail link opens it outside Sfoglia, a link to a file opens that file in Sfoglia, and a `#fragment` link scrolls to its heading. Ctrl+Shift+D switches to the raw text and back, keeping the place: the text opens with the cursor on the block that was at the top, and the rendered view scrolls to the block holding the cursor. The raw text is edited and saved as any text; the Reload / Keep mine bar shows on the rendered side too. Code fences name their grammar in any spelling (`Rust`, `rust,ignore`, `shell`), and a file past 2 MB opens as text.

## Spreadsheets

A csv or tsv opens as a grid: the delimiter is detected from the first lines (a `.tsv` is tabs), quoted fields keep their commas, line breaks and doubled quotes, and the first record names the columns until the `Header` item in the status row turns it into a row; a csv or tsv past 64 MB opens as text. A click on the arrow in a column header sorts the rows by that column (descending first, then ascending, then the file's order; numbers by value before text, text regardless of case, empty cells last), Ctrl+Shift+F focuses the search field and Enter finds the next cell containing the text, and a click selects a cell whose text the edit field above the grid holds (a line break shown as ⏎): Enter writes it back into its own record, and Ctrl+S saves the file as usual. Ctrl+Shift+D switches to the raw text and back. An Excel file (xls, xlsx, xlsm and the templates) opens read-only on its first sheet with a bar to switch sheets, numbers, dates, booleans and errors as Excel shows them, a formula as its cached value and merged cells shown once.

## PDF and XPS

A PDF, XPS or OXPS opens through a tool outside Sfoglia. On Windows that is SumatraPDF when it is installed: it runs
inside the tab with its own toolbar (page, find, zoom, rotation, view modes), one SumatraPDF per tab, gone when the tab
closes. Otherwise, and on Linux, an installed `mutool` (package mupdf-tools) or `pdftoppm` with `pdfinfo` (package
poppler-utils) draws the pages into a scrollable column: fitted to the width, `+` and `-` zoom by a quarter, `0` fits
again; the status row names the tool and its version, the page count, the page at the top and the zoom, and a tool's
complaint when it has one. With none of them installed the status row says what to install, with a `Look again` button
for afterwards (offered on a failed listing too), and the bytes show meanwhile. Every tool is looked for the same way:
an environment variable `SFOGLIA_SUMATRAPDF`, `SFOGLIA_MUTOOL`, `SFOGLIA_PDFTOPPM` or `SFOGLIA_PDFINFO` set to a path
wins (a path to nothing switches the tool off), then the handler registered for the file class and the usual install
folders, then the search path. A PDF is a PDF by its header whatever its name; one SumatraPDF runs per tab, hidden
while its tab is behind another or the closing dialog is up, and ends with the tab (Ctrl+W closes the active tab; the
hosted window covers Sfoglia's own menus over the panel, the tab bar's included; keys typed into the document are
SumatraPDF's, and a click anywhere in Sfoglia's window brings the keyboard back), or with Sfoglia however Sfoglia ends.

## Hex view

A binary file that no family claims opens as rows of 16 bytes: the offset, the bytes in two groups of eight, and the printable ASCII, like `00000010  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 0a 00 ff 7f  |Hello, world....|`. Any other open file switches to its bytes and back with Ctrl+Shift+B. The file is memory-mapped, so a multi-gigabyte file opens at once and only the rows on screen are laid out. Ctrl+Shift+G goes to an offset (decimal, or `0x` and hex); Ctrl+Shift+F finds text, or bytes as hex pairs when Ctrl+M has switched the mode, and Enter steps to the next hit. The hit is marked in both columns and the status row names its offset.

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
