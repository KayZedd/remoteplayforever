# Security

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private reporting: **Security** tab, **Report a vulnerability**. Expect a first answer within a week.

## What the app does on your computer

- Talks to the local Steam client only (its own process and the Steam registry keys). It does not send game data anywhere.
- One optional network call: on start it asks `api.github.com` for the latest release number to show an "update available" banner. Nothing about you or your games is sent. Turn it off in **Settings**.
- Writes `config.json` and `launch.log` under `%LOCALAPPDATA%\RemotePlayForever` and `%APPDATA%\RemotePlayForever`. The log is capped at 1 MB plus one `.old` copy.

## Verifying a download

Every release is built by GitHub Actions from the tagged commit.

```
gh attestation verify RemotePlayForever-Setup-X.Y.Z.exe --repo KayZedd/remoteplayforever
```

or compare `Get-FileHash` with `SHA256SUMS.txt` on the release page.

## SmartScreen

Releases are not yet signed with a code-signing certificate, so Windows SmartScreen may show "Windows protected your PC" for a new download. Choose **More info**, then **Run anyway**, after checking the attestation or checksum above. A signature is a build-time option: once a SignPath project exists, set the repository variable `SIGNPATH_ORGANIZATION_ID` and the secret `SIGNPATH_API_TOKEN` and the release workflow signs the installer and the portable exe (project slug `remoteplayforever`, signing policy `release-signing`).

## Dependencies

CI runs `cargo deny` (advisories, licenses, sources) on every push and pull request, and Dependabot proposes updates weekly.
