# Security policy

## Reporting a problem

Please report a security problem **privately**, not as a public issue:

1. Open the [Security tab](https://github.com/Dannyz15/Sky-Island/security) of this repository.
2. Click **Report a vulnerability** and describe what you found.

Useful to include: the Sky Island version (Settings > About), your Windows version, what you did, what happened and what you expected, and a small example if you have one. Please do not include other people's personal data.

Sky Island is made by one person in their free time, so there is no fixed answer time, but every report is read and taken seriously. I will tell you what I found and, when it is a real problem, fix it in a new version and say so in the release notes. If you want to be named in the notes, say so in your report.

## Which versions get fixes

Only the latest release on the [Releases page](https://github.com/Dannyz15/Sky-Island/releases). Older versions are not patched; the app tells you when a newer one is available.

## What counts

Problems in Sky Island itself, for example: the installer or the update check installing something it should not, the app reading or sending data it says it does not, a crafted file, picture or online answer that crashes or takes over the app, or a way to make it run a program.

## What does not count

- **An antivirus or SmartScreen warning.** The setup is not signed with a code-signing certificate yet, so some programs flag it. That is known; please use a normal [issue](https://github.com/Dannyz15/Sky-Island/issues) if you want to tell me which one.
- Problems in the services the app asks (Open-Meteo, LRCLIB, Aladhan, OpenStreetMap, JAKIM e-Solat, GitHub) or in Windows itself. Please report those to them.
- Anything that needs an attacker who already controls your PC or your Windows account.

What the app stores and sends is written in [PRIVACY.md](PRIVACY.md).
