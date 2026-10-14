# Sfoglia

*Sfoglia* (Italian: leaf through) is a desktop viewer for any file, with editing where it is worth the effort: source code with syntax highlighting plus your own regex highlighting rules, documents, PDFs, spreadsheets, images, email, CAD drawings, archives, audio and video. One Rust code base for Windows and Linux, built on [gpui-kit](https://gpui-kit.com).

## Status

Early. The application shell opens a window; everything else is planned. [FORMATS.md](FORMATS.md) is the checklist of every extension Sfoglia intends to read or write and how far each family has come.

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
