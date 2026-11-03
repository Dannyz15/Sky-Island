<p align="center">
  <img src="assets/logo.png" alt="Sky Island logo" width="160">
</p>

<h1 align="center">Sky Island</h1>

<p align="center">
  <b>A notch for the top of your Windows screen.</b><br>
  Like the Dynamic Island on the iPhone, but for Windows.
</p>

<p align="center">
  <img src="https://img.shields.io/github/v/release/Dannyz15/Sky-Island?label=release&color=3D5FE0" alt="Latest release">
  <img src="https://img.shields.io/github/downloads/Dannyz15/Sky-Island/total?color=3D5FE0" alt="Downloads">
  <img src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6" alt="Windows 10 and 11">
  <img src="https://img.shields.io/badge/built%20with-Rust-DEA584" alt="Built with Rust">
  <img src="https://img.shields.io/badge/license-MIT-3D5FE0" alt="MIT license">
</p>

<p align="center">
  <a href="#tabs">Tabs</a> ·
  <a href="#installation">Installation</a> ·
  <a href="#using-it">Using it</a> ·
  <a href="#questions">Questions</a> ·
  <a href="#privacy">Privacy</a> ·
  <a href="#known-limitations">Limitations</a>
</p>

<p align="center">
  <a href="https://github.com/Dannyz15/Sky-Island/releases/latest"><b>⬇ Download the latest version</b></a>
</p>

---

<div align="center">
<table>
  <tr>
    <td align="center"><img src="assets/screenshots/closed.png" alt="The closed notch showing the clock" width="215"><br><sub>Closed</sub></td>
    <td align="center"><img src="assets/screenshots/music.png" alt="The Music tab" width="320"><br><sub>Music</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="assets/screenshots/stats.png" alt="The Stats tab" width="320"><br><sub>Stats</sub></td>
    <td align="center"><img src="assets/screenshots/timer.png" alt="The Timer tab" width="320"><br><sub>Timer</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="assets/screenshots/calendar.png" alt="The Calendar tab" width="320"><br><sub>Calendar</sub></td>
    <td align="center"><img src="assets/screenshots/clipboard.png" alt="The Clipboard tab" width="320"><br><sub>Clipboard</sub></td>
  </tr>
</table>
</div>

## Tabs

| Tab | What it does |
|---|---|
| 🎵 **Music** | Now playing, controls, a seek bar, synced lyrics and a volume slider. |
| ⏱️ **Timer** | Countdown to the exact second, with an alarm. |
| 💧 **Water** | Daily glass counter with optional reminders. |
| 🌤️ **Weather** | Now and the next five days for your city. |
| 🕌 **Prayer** | Today's prayer times, a countdown and the Hijri date. |
| 📊 **Stats** | Processor, memory, disk and network. |
| 📂 **Shelf** | Park files by dragging them in; drag them out later. |
| ✅ **To-do** | Tasks with reminders. |
| 📅 **Calendar** | Month or week view with your tasks. |
| 📋 **Clipboard** | History of text and pictures; pin text to keep it. |

Every tab can be switched off in Settings. Weather and Prayer need your city (Settings > Location).

## Installation

**Requirements:** Windows 10 or 11 (64-bit). The setup is about 4 MB and needs no administrator rights.

1. Download `Sky-Island-Setup-<version>.exe` from the [Releases page](https://github.com/Dannyz15/Sky-Island/releases).
2. Run it. It installs for your user only.
3. The notch appears at the top of the screen, and a tray icon appears near the clock.

To update, run a newer setup, or use **Check for updates** in Settings > About. Your data is kept. To uninstall, use Windows Settings > Apps.

## Using it

Hover over the notch at the top edge of your screen to open it. Move away and it closes. It also pops up briefly when something changes on your PC, such as the volume or the battery.

## Questions

<details>
<summary>Windows shows a warning when I run the setup.</summary>

The setup is not code-signed, so Windows may show "Windows protected your PC". Click **More info**, then **Run anyway**. If Smart App Control blocks it completely (error 4551), turn Smart App Control off in Windows Security.
</details>

<details>
<summary>The notch disappeared.</summary>

It hides while a full-screen app, such as a game or a video, is on the same screen (Settings > Notch > **Hide in full screen**). **Auto-hide notch** in the same page slides it away when you are not using it: move the mouse to the top edge to bring it back.
</details>

<details>
<summary>Weather and Prayer ask for a city.</summary>

Open Settings > Location, type your city and press Enter, or press **Detect my location**.
</details>

<details>
<summary>I want the notch on another screen.</summary>

Settings > Notch > **Show on**: pick Screen 2 or Screen 3, or **All** to have it follow the mouse to the top edge of whichever screen you use.
</details>

<details>
<summary>Where is my data, and how do I start over?</summary>

In `%APPDATA%\Sky-Island`. Settings > About has **Open data folder** and **Reset to defaults**.
</details>

Found a bug or have an idea? [Open an issue](https://github.com/Dannyz15/Sky-Island/issues).

## Privacy

Your data stays on your computer. There are no accounts and no tracking.

<details>
<summary>What is stored and what is sent</summary>

Everything is saved in `%APPDATA%\Sky-Island`. The clipboard history is kept in memory only, except text you pin (up to 10).

Network requests, and why:

- **Weather:** Open-Meteo, with your city's coordinates.
- **Prayer times:** Aladhan, with your city's coordinates.
- **Detect my location** (only when you press it): Windows Location, then OpenStreetMap's Nominatim to name the place. If Windows Location is off, ipwho.is is asked instead and sees your IP address.
- **Lyrics:** LRCLIB, with the song title, artist and length.
- **Updates:** GitHub, to look for a newer version at start and every 6 hours (switch off in Settings > Notices). Nothing is downloaded until you press Download.

To show Caps Lock and Num Lock notices, a Windows keyboard hook checks only whether one of those two keys was pressed. It never records or sends what you type. **Start with Windows** adds one startup entry under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
</details>

## Known limitations

- Windows only.
- Brightness notices work only on laptop screens whose driver reports the brightness.
- Lyrics depend on what LRCLIB has.
- Weather and prayer times need an internet connection to refresh; the last ones are kept for offline use.

## License

Released under the [MIT License](LICENSE). The crates it is built with, and their licences, are listed in [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt).
