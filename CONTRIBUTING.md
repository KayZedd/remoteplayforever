# Contributing

Bug reports and game reports are the most useful contribution. Use the issue templates: [bug](https://github.com/KayZedd/remoteplayforever/issues/new?template=bug_report.yml), [game that worked or did not](https://github.com/KayZedd/remoteplayforever/issues/new?template=game_report.yml).

## Code

The app is Windows only (Steam's client libraries and registry). You can still edit and check from another OS with `cargo xwin`.

Before a pull request, run what CI runs:

```
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python scripts/test_bump_version.py
```

- New text in the window goes through `tr("English", "Polski")`. Both languages are required.
- Logic goes in small functions with a test; the egui code only calls them.
- Add a line to `CHANGELOG.md` under `[Unreleased]`.

## Releasing

```
python scripts/bump_version.py 0.2.0
git diff            # review
git commit -am "Release 0.2.0"
git tag v0.2.0 && git push origin master v0.2.0
```

The script updates `Cargo.toml`, `Cargo.lock`, `installer/installer.iss` and `CHANGELOG.md`. The release workflow refuses a tag that does not match them.
