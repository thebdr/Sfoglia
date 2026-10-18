# What is still missing (snapshot of 2026-10-18, 01:30)

The contract (`.zen/contract.md`) is the source of truth; this is a dated reading of it for the user. Verified
and on main: C-001 to C-012 (shell, checklist, CI, open and route, text, rules, hex, edit and save, images,
data trees, Markdown, closing with modified tabs).

## In flight

- **C-013 Spreadsheets and delimited tables** (branch `c-013-spreadsheets`): implemented and tested (15 tests,
  Windows and Linux captures). Three refuter passes found and fixed real bugs; passes four to seven found no code defect, only promises without a test, all pinned since (24 tests); passes eight and nine mutated the code and pinned what survived (29 tests); passes ten and eleven pinned eight more; the twelfth is running (29 tests). Left: the holds verdict, `zen verify`, a last Linux run, the merge to main, CI.
- Run evidence of C-002, C-004 and C-008 refreshed on 2026-10-18; `zen status` reports no drift.

## Pending clauses, in the order I plan to take them

| Pending | What it is | Stack decided or likely | Size |
| --- | --- | --- | --- |
| P-021 → C-014 | Archives, read side: list entries, open an entry inside Sfoglia, extract; container and codec traits | `zip`, `tar`, `flate2`, `bzip2`, `xz`/`lzma-rs`, `zstd`, `sevenz-rust2` (all in the registry); rar and exotic containers through an installed 7-Zip | large |
| P-030 | Archive writing (create, add, remove, update; 7z first, AES) | the same crates' writers | large |
| P-025 | External tools detected with graceful fallback (7-Zip, LibreOffice, VLC, a DWG converter) | own detection, status messages | small |
| P-008 | PDF and XPS viewing | decided by the user: SumatraPDF embedded on Windows (plugin mode, command line and DDE), `mutool` / `pdftoppm` page images on Linux, both detected as external tools (P-025) | large |
| P-009 | PDF in-document links jump and blink (the user's specific ask) | depends on P-008's stack exposing annotations | medium |
| P-014 | Email messages (eml, msg, winmail.dat) | `mail-parser`, `cfb` for msg, TNEF to write | medium |
| P-015 | Web pages (structural renderer, mhtml) | `html5ever`; a webview is out of reach in gpui | medium |
| P-007 | Word-processing documents (docx, odt, rtf, epub native; doc via LibreOffice) | `docx-rs`/own OOXML reader, `zip` | large |
| P-011 | Presentations (pptx, odp native; ppt via LibreOffice) | own OOXML reader | large |
| P-012 | Visio diagrams (vsdx native on a canvas) | own OOXML reader, gpui canvas | large |
| P-026 | LibreOffice fidelity rendering to PDF | needs P-008 and P-025 | medium |
| P-016 | CAD drawings (dxf read and write, dwg via converter) | `dxf` crate, gpui canvas | large |
| P-018 | Camera RAW images | `rawloader`/`libraw` bindings: to evaluate | medium |
| P-019 | Audio playback through libVLC | `vlc-rs` bindings, libVLC installed (P-025) | medium |
| P-020 | Video playback through VLC | `vlc-rs`, a window surface in gpui: hardest integration | large |
| P-013 | Microsoft Project files (mpx first) | own parser | small |
| P-027 | Release artifacts (Windows zip, Linux tarball) from CI | GitHub Actions on tags | small |
| P-029 | A workbook's own column widths and number formats | sheet XML beside calamine | medium |

## Decisions that are the user's

- The PDF stack (P-008): SumatraPDF, decided on 2026-10-18. One caveat: P-009's blink on the link's destination
  cannot be drawn inside Sumatra's own window; the jump itself is Sumatra's.
- The disk: the system pagefile (about 22 GB) and the Linux VM's 66 GB disk keep C: tight; both are the user's
  settings, worked around with pruning and polling.

## Outside the contract, noted

- The desktop-control tools now press chords in the real window (`request_access` with `sfoglia.exe`), so the
  Windows captures no longer need the user's keys; Excel was driven the same way for the xls fixtures.
