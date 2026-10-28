<p align="center">
  <img src="assets/logo.png" alt="Sky Island logo" width="160">
</p>

<h1 align="center">Sky Island</h1>

<p align="center">
  <b>A notch for the top of your Windows screen.</b><br>
  Music, lyrics, a focus timer, water reminders, a file shelf and a to-do list, all tucked into one small pill.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6" alt="Windows 10 and 11">
  <img src="https://img.shields.io/badge/built%20with-Rust-DEA584" alt="Built with Rust">
  <img src="https://img.shields.io/badge/license-MIT-3D5FE0" alt="MIT license">
</p>

<p align="center">
  <a href="#what-it-does">What it does</a> ·
  <a href="#installation">Installation</a> ·
  <a href="#using-it">Using it</a> ·
  <a href="#your-data-and-privacy">Privacy</a> ·
  <a href="#known-limitations">Limitations</a>
</p>

---

Sky Island sits at the top edge of your screen as a small black pill. Hover over it and it opens into a player, a timer, a tracker and more. Move away and it tucks itself back in. It also pops up for a moment when something changes on your PC, such as the volume or the battery.

It is written in Rust, runs natively on Windows 10 and 11, and keeps all of your data on your own computer.

## What it does

| | Tab | What you get |
|---|---|---|
| 🎵 | **Music** | What is playing in Spotify, a browser or any player that reports to Windows. Play, pause, next, previous, album art and a progress bar. Click the album art for synced lyrics. |
| 🎯 | **Focus** | A focus timer with short and long breaks, adjustable lengths, a progress ring and a count of today's sessions. |
| 💧 | **Water** | A daily glass counter with a goal and gentle reminders. |
| 📂 | **Shelf** | A place to park files. Drag them onto the notch, and drag them out again later. |
| ✅ | **To-do** | A short task list with optional reminders. |
| ⚙️ | **Settings** | Start with Windows, hide in fullscreen apps, clock style, idle eyes, sounds, and Quit. |

### It is alive even when closed

- **A clock.** The closed notch shows the time.
- **Idle eyes.** After a while without keyboard or mouse use, a pair of eyes appears and blinks. If you stay away longer, they fall asleep. When you come back, they greet you.
- **Quick notices.** For a moment, the notch grows to show a change in volume or mute, a new audio output such as headphones, brightness, Caps Lock and Num Lock, plugging in or unplugging the charger, low battery, and Wi-Fi or internet changes.
- **Out of the way.** When a fullscreen app such as a game or a video is in front, the notch hides itself.

## Installation

1. Download `Sky-Island-Setup-<version>.exe` from the [Releases page](https://github.com/Dannyz15/Sky-Island/releases).
2. Double-click it and follow the steps. It installs for your user account only, so it never asks for administrator rights. You can choose to add a desktop shortcut and to start Sky Island when you sign in to Windows.
3. Leave **Launch Sky Island** ticked on the last page. The notch appears at the top of your screen, and a tray icon appears near the clock.

> **A note about the Windows warning.** The setup is not code-signed, so Windows may show "Windows protected your PC". Click **More info**, then **Run anyway**. If Smart App Control is turned on, Windows may block the file completely (error 4551). In that case you need to turn Smart App Control off in Windows Security.

**Updating.** Run the newer setup. It closes the running app and installs over the old version, and your data is kept.

**Uninstalling.** Open Windows Settings, then Apps, find Sky Island and click Uninstall. At the end it asks whether to delete your data too. The default answer is No, so your settings and tasks stay unless you choose Yes.

## Using it

Hover over the notch to open it. The tab bar at the top switches between the views.

**🎵 Music.** Click the album art to open the lyrics, and click the small album art at the top left of the lyrics to go back. Click any lyric line to jump to that point in the song. Lyrics come from [LRCLIB](https://lrclib.net).

**🎯 Focus.** Choose Focus, Short or Long. While the timer is stopped, use the minus and plus buttons to change its length, then press start. Four focus sessions lead to a long break. The countdown shows in the closed notch, and when a phase ends the notch opens and plays a short chime. Under the title you can see how many sessions you finished today and for how long. The defaults are 25, 5 and 15 minutes. You can turn the chimes off with **Sounds** in Settings.

**💧 Water.** Press the big plus button each time you drink a glass. Set your daily goal with the minus and plus buttons (1 to 20 glasses, 8 by default). Choose a reminder every 30 minutes, 1 hour or 2 hours, or turn reminders off. Reminders stop once you reach your goal, and the count starts again every day.

**📂 Shelf.** Drag files from Explorer onto the notch. Click a tile to open the file, or press and drag it into another app. The `×` on a tile removes it from the shelf only, and never deletes the file. The shelf holds up to 50 files, and the mouse wheel scrolls through them.

**✅ To-do.** Click the box at the top and type a task. Press Enter to add it, or Esc to cancel. If you like, choose a reminder of 15 minutes, 1 hour, 3 hours or 1 day. Tick a task to finish it, and use `×` to delete it.

## Your data and privacy

Everything stays on your computer, in `%APPDATA%\Sky-Island`:

| File | What it holds |
|---|---|
| `settings.json` | Your settings |
| `focus.json` | The focus, short break and long break lengths |
| `focus_stats.json` | Today's finished focus sessions and focused time |
| `water.json` | Today's count, your goal and your reminder choice |
| `todo.json` | Your tasks |
| `shelf.json` | The paths of the files on the shelf (the files themselves are never copied) |
| `lyrics\` | Lyrics you have already looked up |
| `crash.log` | Only written if the app crashes, with the error message |

The only network request the app makes is the lyrics lookup. It sends the song title, artist and length to LRCLIB. There are no accounts and no tracking.

To show Caps Lock and Num Lock notices, the app installs a Windows keyboard hook. It only checks whether the key pressed is Caps Lock or Num Lock. It never records, stores or sends anything you type. Some antivirus programs are cautious about any keyboard hook, so it is worth knowing about.

If you turn on **Start with Windows**, the app adds one value named `Sky Island` to your per-user startup list (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`). Turning it off removes it.

## Known limitations

- Windows only.
- Chrome and YouTube do not offer next or previous for a single video. Next is greyed out and Previous restarts the video. Playlists work normally.
- Brightness notices only work on laptop screens, and only if the graphics driver lets the app read the brightness.
- Lyrics depend on what LRCLIB has, so some songs have none.

## License

Released under the [MIT License](LICENSE).
