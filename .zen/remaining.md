# What is still missing (snapshot of 2026-10-18, evening)

The contract (`.zen/contract.md`) is the source of truth; this is a dated reading of it for the user. Verified
and on main: C-001 to C-014 (shell, checklist, CI, open and route, text, rules, hex, edit and save, images,
data trees, Markdown, closing with modified tabs, spreadsheets and delimited tables, PDF and XPS through
SumatraPDF or a page renderer with the tools outside Sfoglia).

## In flight

- Nothing: C-014 merged to main (b7fa3ac) after twelve refuter passes (fourteen defects found and fixed on the
  way, some forty promises pinned, 29 tests), Windows captures and the Linux pass with the real pdftoppm in the
  VM. Next in line: P-021 (archives, read side) as C-015.

## Pending clauses, in the order I plan to take them

| Pending | What it is | Stack decided or likely | Size |
| --- | --- | --- | --- |
| P-021 → C-015 | Archives, read side: list entries, open an entry inside Sfoglia, extract; container and codec traits | `zip`, `tar`, `flate2`, `bzip2`, `xz`/`lzma-rs`, `zstd`, `sevenz-rust2` (all in the registry); rar and exotic containers through an installed 7-Zip | large |
| P-030 | Archive writing (create, add, remove, update; 7z first, AES) | the same crates' writers | large |
| P-009 | PDF in-document links jump and blink (the user's specific ask) | the jump is SumatraPDF's own (its links work inside the hosted window already); the blink cannot be drawn inside Sumatra's window: options to put to the user | medium |
| P-014 | Email messages (eml, msg, winmail.dat) | `mail-parser`, `cfb` for msg, TNEF to write | medium |
| P-015 | Web pages (structural renderer, mhtml) | `html5ever`; a webview is out of reach in gpui | medium |
| P-007 | Word-processing documents (docx, odt, rtf, epub native; doc via LibreOffice) | `docx-rs`/own OOXML reader, `zip` | large |
| P-011 | Presentations (pptx, odp native; ppt via LibreOffice) | own OOXML reader | large |
| P-012 | Visio diagrams (vsdx native on a canvas) | own OOXML reader, gpui canvas | large |
| P-026 | LibreOffice fidelity rendering to PDF | needs C-014 (the PDF view and the tools) | medium |
| P-016 | CAD drawings (dxf read and write, dwg via converter) | `dxf` crate, gpui canvas | large |
| P-018 | Camera RAW images | `rawloader`/`libraw` bindings: to evaluate | medium |
| P-019 | Audio playback through libVLC | `vlc-rs` bindings, libVLC installed (found as C-014 finds tools) | medium |
| P-020 | Video playback through VLC | `vlc-rs`, a window surface in gpui: hardest integration | large |
| P-013 | Microsoft Project files (mpx first) | own parser | small |
| P-027 | Release artifacts (Windows zip, Linux tarball) from CI | GitHub Actions on tags | small |
| P-029 | A workbook's own column widths and number formats | sheet XML beside calamine | medium |

## Decisions that are the user's

- The PDF stack: SumatraPDF, decided on 2026-10-18, now C-014. One caveat for P-009: the blink on a link's
  destination cannot be drawn inside Sumatra's own window (the jump itself is Sumatra's); the choices when P-009
  comes: a flash of Sfoglia's own border around the panel, a short zoom nudge through Sumatra's commands, or
  leaving the blink to the Linux page column only.
- The disk: the system pagefile (about 22 GB) and the Linux VM's 66 GB disk keep C: tight; both are the user's
  settings, worked around with pruning and polling.

## Outside the contract, noted

- The desktop-control tools now press chords in the real window (`request_access` with `sfoglia.exe`), so the
  Windows captures no longer need the user's keys; Excel was driven the same way for the xls fixtures.
