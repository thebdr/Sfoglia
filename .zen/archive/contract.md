# Archived Contract Items

Retired clauses, preserved — never deleted (the audit trace, C-023). Each carries the date it left the live contract.

<!-- archived 2026-10-18: shelved -->
### P-025: External tools detected with graceful fallback
- **Date:** 2026-10-14
- **Source:** ingest (user-request-2026-10-13#external-tools)
- **Description:** Folded into C-014 on 2026-10-18, which states the search order, the version, the hint and the fallback for every tool outside Sfoglia. Backends that depend on software outside Sfoglia (libVLC, LibreOffice, a DWG converter, 7-Zip) are detected at startup and on demand; when one is missing, the viewer that needs it explains what to install instead of failing silently, and the rest of the application is unaffected.
- **Proposed verification:** automated: with a tool path pointed at nothing, the viewer reports the exact missing tool and the file still opens in the fallback (hex or text).
