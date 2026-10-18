# What is still missing (snapshot of 2026-10-18, 04:30)

The contract (`.zen/contract.md`) is the source of truth; this is a dated reading of it for the user. Verified
and on main: C-001 to C-013 (shell, checklist, CI, open and route, text, rules, hex, edit and save, images,
data trees, Markdown, closing with modified tabs, spreadsheets and delimited tables).

## In flight

- **C-014 PDF and XPS through SumatraPDF or a page renderer, and the tools outside Sfoglia** (branch
  `c-014-pdf`): P-008 promoted with P-025 folded in. SumatraPDF 3.6 embeds in the tab through its plugin mode
  (experiments logged in `.zen/evidence/C-014.md`), its frame placed on every paint, hidden with the tab,
  driven by `WM_COPYDATA`; mutool or pdftoppm draw a page column elsewhere; the hint and the bytes without a
  tool. Implemented with 25 tests (9 unit, 16 headless); seven refuter passes found some twenty unpinned promises and
  nine defects (a failed drawing retried on every paint, a tab behind another keeping its frame visible, a PDF
  called by its name, a closed tab leaving its drawings, zoom keys eating the hex fields' digits, a complaint
  cleared by another page, zero pages passing as a listing, a dead SumatraPDF never counted absent and its
  reason hiding the hint, Look again losing the focus), all fixed and pinned; the Windows captures (two tabs,
  the hex toggle, a resize, a page jump, a dead stand-in) and the Linux pass (the real pdftoppm in the VM) are
  in the evidence; the eighth pass, the run records and the merge remain.
- Run evidence of C-002, C-004 and C-008 refreshed on 2026-10-18; `zen status` reports no drift.

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
