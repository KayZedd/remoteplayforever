# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
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
