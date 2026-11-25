import unittest

import bump_version as b

CHANGELOG = """# Changelog

## [Unreleased]

### Added
- thing

## [0.1.1] - 2026-11-02

### Added
- old

[Unreleased]: https://github.com/KayZedd/remoteplayforever/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/KayZedd/remoteplayforever/compare/v0.1.0...v0.1.1
"""


class BumpTests(unittest.TestCase):
    def test_cargo_toml_only_touches_the_package_version(self):
        text = '[package]\nname = "x"\nversion = "0.1.1"\n\n[dependencies]\nclap = { version = "4" }\n'
        out = b.bump_cargo_toml(text, "0.2.0")
        self.assertIn('version = "0.2.0"', out)
        self.assertIn('clap = { version = "4" }', out)

    def test_cargo_lock_touches_only_this_package(self):
        text = 'name = "other"\nversion = "0.1.1"\n\nname = "remoteplayforever"\nversion = "0.1.1"\n'
        out = b.bump_cargo_lock(text, "0.2.0")
        self.assertEqual(out, 'name = "other"\nversion = "0.1.1"\n\nname = "remoteplayforever"\nversion = "0.2.0"\n')

    def test_installer(self):
        out = b.bump_installer('#define MyAppVersion "0.1.1"\n', "1.0.0")
        self.assertEqual(out, '#define MyAppVersion "1.0.0"\n')

    def test_changelog_gets_new_section_and_links(self):
        out = b.bump_changelog(CHANGELOG, "0.2.0", "2026-12-01")
        self.assertIn("## [Unreleased]\n\n## [0.2.0] - 2026-12-01\n\n### Added\n- thing", out)
        self.assertIn("[Unreleased]: https://github.com/KayZedd/remoteplayforever/compare/v0.2.0...HEAD", out)
        self.assertIn("[0.2.0]: https://github.com/KayZedd/remoteplayforever/compare/v0.1.1...v0.2.0", out)
        self.assertIn("[0.1.1]: https://github.com/KayZedd/remoteplayforever/compare/v0.1.0...v0.1.1", out)

    def test_changelog_refuses_the_current_version(self):
        with self.assertRaises(SystemExit):
            b.bump_changelog(CHANGELOG, "0.1.1", "2026-12-01")

    def test_missing_pattern_fails_loudly(self):
        with self.assertRaises(SystemExit):
            b.bump_installer("nothing here", "1.0.0")

    def test_bad_version_rejected(self):
        for bad in ["1.0", "v1.0.0", "1.0.0-rc1", ""]:
            with self.assertRaises(SystemExit):
                b.main([bad])


if __name__ == "__main__":
    unittest.main()
