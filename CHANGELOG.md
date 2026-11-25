# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Application icon (window, taskbar, installer) and Windows version information embedded in the exe.
- Drag a game's `.exe` onto the window to select it; Enter in the path field launches.
- "New version available" banner from a single anonymous GitHub releases lookup on start; switch in Settings.
- Header shows when Steam is not running, with a "Start Steam" button; the status refreshes every 2 seconds.
- Tooltips for the guest input switches, Copy and End session; recent games can be removed one by one and show when they were last played; Steam games list is sorted and counts results.
- First start follows the Windows display language.
- `scripts/bump_version.py` (version in Cargo.toml, Cargo.lock, installer.iss and CHANGELOG in one step), `SECURITY.md`, `CONTRIBUTING.md`.
- CI: `cargo deny` audit and release-script tests. Release workflow signs the installer and the portable exe with SignPath when `SIGNPATH_ORGANIZATION_ID` is configured.

### Changed
- Release builds use thin LTO, one codegen unit and stripped symbols (smaller exe).
- `launch.log` rotates at 1 MB (previous log kept as `launch.log.old`).
- A guest appearing twice in the session card is listed once.
- Dependencies updated for RUSTSEC-2026-0592 (`url` 2.5.9).
- The installer-free release asset is now named `remoteplayforever-X.Y.Z-portable.exe` (was `-x64.exe`) and is described in the release notes and README.

## [0.1.1] - 2026-11-02

### Added
- GitHub Actions: CI (fmt, clippy, tests, release build) on every push and PR; release workflow that builds the installer on a `vX.Y.Z` tag and publishes it with SHA-256 checksums and build provenance attestations.
- README with screenshots, FAQ and a "Works with" table; Polish README (`README.pl.md`).
- Game compatibility list (`docs/COMPATIBILITY.md`) and issue templates for bug and compatibility reports.

### Changed
- Code formatted with `rustfmt` and cleaned up for `clippy -D warnings`. No behaviour change.

## [0.1.0] - 2026-10-13

### Added
- First release: Remote Play Together sessions for Steam and non-Steam games, Steam overlay launcher for non-Steam games, guest links, live guest keyboard / mouse / controller switches, English and Polish interface, Inno Setup installer.

[Unreleased]: https://github.com/KayZedd/remoteplayforever/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/KayZedd/remoteplayforever/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/KayZedd/remoteplayforever/releases/tag/v0.1.0
