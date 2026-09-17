# Repository audit (2026-09-17)

## State of the repository

- `/Users/srikanth/Desktop/Ok` was **empty**: no source, no build system, no git history.
  This is a **greenfield** project. Nothing to retain, nothing to migrate.

## Host toolchain findings (these shaped the architecture)

| Tool | Status | Consequence |
|---|---|---|
| Rust | not installed → installed via Homebrew `rustup` (stable 1.98, `aarch64` + `x86_64-apple-darwin`) | Rust core is buildable; universal binaries possible |
| Xcode | **not installed** — only Command Line Tools (Swift 6.4, macOS 27.0 SDK) | No `xcodebuild`; the macOS app is built with **SwiftPM** and bundled by script |
| SwiftUI on CLT | compiles, links AppKit / UserNotifications / QuickLookUI / Network / ServiceManagement / IOKit | Native macOS UI is feasible |
| `@State` macro | the macOS 26+/27 SDK implements `@State` as a macro whose plugin (`SwiftUIMacros`) ships only with Xcode | The app uses a thin `@ViewState` property wrapper over `SwiftUICore.State` (see `apps/macos/Sources/OspreyApp/Support/ViewState.swift`) |
| Node | v26.8 / npm 11 | browser extension + remote web UI toolchain |
| Homebrew | 7.0 | `pkg-config`, `cmake` installed; `ffmpeg` optional (runtime-detected, never required) |
| libtorrent-rasterbar | absent | not used — see ADR-001 (librqbit chosen) |
| Docker / gh | absent | Dockerfile is provided; image build must run on a host with Docker |
| Network | intermittent DNS failures observed during setup | all build scripts retry network steps |

## Architecture assessment

Because the repository is empty there is no technical debt to remove. The assessment therefore
concentrates on *what the constraints permit*:

1. A native SwiftUI/AppKit macOS UI is buildable here and is the only way to get first-class
   menu-bar, Quick Look, Finder-tag, notification-action, login-item and power-assertion behaviour
   without bridging hacks.
2. A Rust core is buildable and gives one authoritative engine for desktop, CLI, headless and
   remote use.
3. A pure-Rust BitTorrent engine (librqbit) integrates directly with the Tokio runtime; a C++
   libtorrent bridge would add a Boost build, `cxx` shims and a Windows build burden with no UX
   benefit for the user.

See `001-architecture-decision.md` for the decision itself.
