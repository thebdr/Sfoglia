# Sfoglia, working notes for agents

- Governed by IMPLICATION→MACHINE: the contract is `.zen/contract.md`, evidence logs are `.zen/evidence/<ID>.md`, screenshots go to `.zen/evidence/runs/` (git-ignored, overwritten per clause).
- UI stack is **gpui-kit 0.7.3** (`gpui_kit::*` re-exports GPUI). The gpui-kit and design-guide skills are vendored under `.claude/skills/`; read `references/coding-guides.md` before any state-ownership or public-API decision.
- Commands: `cargo run` (app), `cargo test` (unit + headless UI + checklist freshness; the workspace default members cover xtask), `cargo run -p xtask -- formats` (regenerate `FORMATS.md` after touching a format clause).
- `FORMATS.md` is generated. Never edit it by hand; change the clause and regenerate.
- Headless UI tests live in `tests/ui.rs` and need the `test-support` dev-dependency feature; the first `cargo test` after a `cargo build` recompiles the gpui stack once.
- Screenshots from the agent shell: `CopyFromScreen` returns black; use `PrintWindow(hwnd, 2)` on the process's `MainWindowHandle` (see `.zen/evidence/C-001.md`).
- Linux verification runs in the VMware VM `C:\_VMs\LMint\LinuxMint64.vmx` (Linux Mint 22.3, X11, llvmpipe Vulkan): start it with `vmrun -T ws start <vmx>`, address via `vmrun getGuestIPAddress <vmx> -wait`, then `ssh -i ~/.ssh/sfoglia_vm hyper@<ip>`. The shared folder `C:\_VMs\_Shared` is `/mnt/hgfs/_Shared` in the guest. WSL is unusable on this machine (company policy denies the VM service logon).
- CI: `.github/workflows/ci.yml`, jobs `test-windows` and `test-linux`, on the public repo https://github.com/thebdr/Sfoglia.
- Linux verification in the VM: copy `scripts/linux-verify.sh` to the VM with scp, run it there over SSH (`bash linux-verify.sh`), then fetch `~/sfoglia-linux.png` and `~/sfoglia-linux.log`.
- The `tree-sitter-languages` feature of gpui-kit compiles every grammar; enabling it re-resolved the lockfile and `cc` had to come down to 1.2.x (`cargo update -p cc`), see `.zen/evidence/C-005.md`.
- Effects such as a dock close or a theme change land after the window update ends: in headless tests, leave `update_window`, call `cx.run_until_parked()`, then assert.
- Windows captures of the real window: `pwsh scripts/windows-capture.ps1 -Out <png> -Args <file> -Keep` launches the app and saves a PrintWindow capture; `-ProcessId <pid>` captures a running window (after the user pressed a chord, since the agent shell cannot bring windows to the foreground).
- The highlighting rules (C-006) live in `src/rules.rs` (pure) and `src/viewers/rules_panel.rs` (store + panel); headless tests give the workspace a temp rule path through `Workspace::with_rules_path`, never the real one under `%APPDATA%`.
