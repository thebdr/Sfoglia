# Sfoglia, working notes for agents

- Governed by IMPLICATION→MACHINE: the contract is `.zen/contract.md`, evidence logs are `.zen/evidence/<ID>.md`, screenshots go to `.zen/evidence/runs/` (git-ignored, overwritten per clause).
- UI stack is **gpui-kit 0.7.3** (`gpui_kit::*` re-exports GPUI). The gpui-kit and design-guide skills are vendored under `.claude/skills/`; read `references/coding-guides.md` before any state-ownership or public-API decision.
- Commands: `cargo run` (app), `cargo test` (unit + headless UI + checklist freshness; the workspace default members cover xtask), `cargo run -p xtask -- formats` (regenerate `FORMATS.md` after touching a format clause).
- `FORMATS.md` is generated. Never edit it by hand; change the clause and regenerate.
- Headless UI tests live in `tests/ui.rs` and need the `test-support` dev-dependency feature; the first `cargo test` after a `cargo build` recompiles the gpui stack once.
- Screenshots from the agent shell: `CopyFromScreen` returns black; use `PrintWindow(hwnd, 2)` on the process's `MainWindowHandle` (see `.zen/evidence/C-001.md`).
- Linux is not reachable from the agent session on this machine; Linux proof comes from CI (P-024).
