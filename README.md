<p align="center">
  <img src="assets/logo.png" alt="Sky Island logo" width="160">
</p>

<h1 align="center">Sky Island</h1>

<p align="center">
  <b>A notch for the top of your Windows screen.</b><br>
  Music, lyrics, a timer, a water tracker, the weather, prayer times, a file shelf, a to-do list, a calendar and a clipboard history, all tucked into one small pill.
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

Sky Island sits at the top edge of your screen as a small black pill, with curved flares on both sides of its top edge, like a notch cut into the screen. Hover over it and it opens into a player, a timer, a tracker and more. Move away and it tucks itself back in. It also pops up for a moment when something changes on your PC, such as the volume or the battery.

It is written in Rust, runs natively on Windows 10 and 11, and keeps all of your data on your own computer. Everything in it moves on spring physics, so opening, hiding, switching tabs, hovering and toggling all glide instead of snapping.

## What it does

| | Tab | What you get |
|---|---|---|
| 🎵 | **Music** | What is playing in Spotify, a browser or any player that reports to Windows. Play, pause, next, previous, album art, the artist and album, and a progress bar you can drag to jump in the song, with the time played on the left and the time left on the right. The speech-bubble button opens synced lyrics beside the player. At the right end of the controls there is a volume slider for your speakers (the whole PC's volume): drag it, scroll the mouse wheel over it, or press the speaker to mute. |
| ⏱️ | **Timer** | A countdown timer you set to the exact second, from 1 second to 24 hours, with quick choices and a progress ring. |
| 💧 | **Water** | A daily glass counter with a goal and gentle reminders (the reminders start off; pick an interval in the tab). |
| 🌤️ | **Weather** | The weather now in your city (temperature, what it feels like, humidity, wind, today's high and low) and the next five days. It needs your city, which you set in the settings window. |
| 🕌 | **Prayer** | Today's prayer times for your city (Fajr, Sunrise, Dhuhr, Asr, Maghrib and Isha), the next prayer with a countdown to it, and the Hijri date. When a prayer time comes, the notch says so with a notice and a chime. It needs your city, which you set in the settings window. |
| 📊 | **Stats** | The processor load in a ring (with its last minute as a line), memory and disk use as bars, and the network's speed. It looks at the numbers once a second, only while this tab is open. |
| 📂 | **Shelf** | A place to park files. Drag them onto the notch, and drag them out again later. |
| ✅ | **To-do** | A task list with reminders at the day and time you choose, and tasks you can edit. |
| 📅 | **Calendar** | A month view (or just one week, as you choose) where each day shows its own tasks. |
| 📋 | **Clipboard** | A history of what you copy, both text and pictures such as screenshots. Click an item to copy it again, or clear everything with one button. Press the pin on a text item to pin it: pinned items stay at the top, are never forgotten or cleared, and are kept (up to 10) in `pinned.json` so they are still there after a restart. |
| ⚙️ | **Settings** | Not a tab: right-click the tray icon and choose **Settings…** for a window with every switch and more (theme, animation, eye style, hover delays, volume, the timer alarm and its sound, checking for updates). |

### It is alive even when closed

- **Eyes.** The closed notch has a pair of blinking eyes (turn on **Enable eyes** in the settings window; off by default). Choose their look with **Eye style** in the settings window: **Classic** (white with a black pupil), **Dots**, **Cat** (amber with a slit pupil) or **Glow** (glowing blue). They take the place of the clock, so they show while you work only when the clock is off.
- **A clock.** **Show clock** is on by default, so the closed notch shows the time. Turn it off to see the eyes instead.
- **Idle eyes.** After a while without keyboard or mouse use, the eyes appear and blink (even with the clock on). If you stay away longer, they fall asleep. When you come back, they greet you.
- **Quick notices.** For a moment, the notch grows to show a change in volume or mute, a new audio output such as headphones, brightness, Caps Lock and Num Lock, plugging in or unplugging the charger, low battery, and Wi-Fi or internet changes.
- **Out of the way.** When a fullscreen app such as a game or a video is in front, the notch hides itself. With more than one screen it hides only when that app is on the same screen as the notch, so a video on your second screen leaves it alone.

## Installation

1. Download `Sky-Island-Setup-<version>.exe` from the [Releases page](https://github.com/Dannyz15/Sky-Island/releases).
2. Double-click it and follow the steps. It installs for your user account only, so it never asks for administrator rights. You can choose to add a desktop shortcut and to start Sky Island when you sign in to Windows.
3. Leave **Launch Sky Island** ticked on the last page. The notch appears at the top of your screen, and a tray icon appears near the clock.

> **A note about the Windows warning.** The setup is not code-signed, so Windows may show "Windows protected your PC". Click **More info**, then **Run anyway**. If Smart App Control is turned on, Windows may block the file completely (error 4551). In that case you need to turn Smart App Control off in Windows Security.

**Updating.** Run the newer setup, or press **Check for updates** in the settings window and then **Download** and **Install**. The setup asks the running app to close (and only forces it if it has not closed after a few seconds) and installs over the old version, and your data is kept. Leave **Launch Sky Island** ticked on its last page to start the app again.

**One copy at a time.** Starting Sky Island while it is already running does nothing, so you never get two notches writing to the same files.

**Uninstalling.** Open Windows Settings, then Apps, find Sky Island and click Uninstall. At the end it asks whether to delete your data too. The default answer is No, so your settings and tasks stay unless you choose Yes.

## Using it

Hover over the notch to open it. The tab bar at the bottom (a rounded pill with grey icons) switches between the views, which slide to the left or right, the way the tab is. You can also switch tabs with the mouse wheel: scroll down for the next tab and up for the one before. It works over the tab bar on every tab, and anywhere on the Music, Water and Calendar tabs. On the tabs that scroll something themselves (the lists, the lyrics, the timer numbers) the wheel keeps doing that away from the tab bar.

**🎵 Music.** Press on the progress bar and drag to jump to any point in the song, or just click where you want to be. Press the speech-bubble button next to the controls to open the lyrics. The notch grows wider, staying centred on the screen, and the lyrics appear to the right of the player, with the line being sung in bold. Press the button again to close them. Click any lyric line to jump to that point in the song. Scroll the mouse wheel over the lyrics to look ahead or back; the lyrics then stop following the music and a **Sync** button appears at the top right, with an arrow showing where the line being sung is. Press **Sync** to glide back to it. Lyrics come from [LRCLIB](https://lrclib.net).

**⏱️ Timer.** Set the length exactly as you like. Each plus or minus button above and below the hours, minutes and seconds changes that number by exactly one, so you can set the timer a second at a time (the mouse wheel over a number works too). Or click a number and type it: type 25, press Tab or a colon to move on, type 37, then press Enter, and the timer is 25:37. The Up and Down arrow keys change the number by one while you type, and Esc cancels. There are also quick choices: 5, 10, 15, 30 minutes or 1 hour. Press start to begin, press it again to pause and carry on later, and use **Reset** to stop and go back to the full length. The ring empties as the time runs down. The countdown also shows in the closed notch, and when time is up the notch opens and the chime sounds again every 3 seconds until you press the big red **Stop** button, like a phone's timer (only that button stops it, otherwise it stops by itself after a minute). The length you set is remembered. You can turn the chimes off with **Sounds** in the settings window.

**💧 Water.** Press the big plus button each time you drink a glass. Set your daily goal with the minus and plus buttons (1 to 20 glasses, 8 by default). Choose a reminder every 30 minutes, 1 hour or 2 hours, or leave reminders off (they start off). Reminders stop once you reach your goal, and the count starts again every day.

**📂 Shelf.** Drag files from Explorer onto the notch. Click a tile to open the file, or press and drag it into another app. Once you drop it somewhere that takes it, the file leaves the shelf (it is moved or copied there as usual). Pressing Esc, or letting go over nothing, or dropping it back on the notch keeps it on the shelf. To take several files at once, hold **Ctrl** and click each one: it lights up in the accent colour (Ctrl+click it again to let it go, and **Esc** clears the highlight). Then drag any highlighted file and they all go together. The `×` on a tile removes it from the shelf only, and never deletes the file. The red **Clear** button, to the right of the tab bar, takes every file off the shelf, and never deletes any of them. The shelf holds up to 50 files, and the mouse wheel scrolls through them.

**✅ To-do.** Click the box at the top and type a task. Press Enter to add it, or Esc to cancel. Tick a task to finish it, and use `×` to delete it. Click a task, or its pencil, to change it later.

The task editor lets you pick a **day** (use the arrows, or None, Today and Tomorrow) and switch on a **reminder** for an exact time. Set the hour and minute with the plus and minus buttons or with the mouse wheel, or use the quick choices: in 15 minutes, 1 hour, 3 hours, or tomorrow at 9:00. When the time comes, the notch opens with a chime. The clock follows your **24-hour clock** setting.

**📅 Calendar.** The month is on the left and today is marked in amber. Click a day to see its tasks on the right, and use the arrows to change month. Click the month name to jump back to today. Press **+ Add task** to add a task to the day you picked, and tick or delete tasks the same way as in the To-do tab. A dot under a date means it has tasks. The same tasks also show in the To-do tab, with their date at the right.

**🌤️ Weather.** Set your city first (Settings, Location page: type it and press Enter, or press **Detect my location**). The tab shows the temperature, what it feels like, humidity, wind, today's high and low, and the next five days. It refreshes every 30 minutes and when you open the tab, and keeps the last forecast for when you are offline.

**🕌 Prayer.** Also needs your city. It shows today's five prayers and sunrise, the next prayer with a countdown, and the Hijri date; the next one is highlighted. When a prayer time comes, the notch shows a notice and plays the reminder chime (the **Prayer time** switch in Notices turns that off). Pick the calculation **Method** and the **Asr** setting on the Location page.

**📊 Stats.** The processor, memory, disk and network speed, looked at once a second while this tab is open.

**🔊 Volume and 📌 pins.** In the Music view the speaker and slider at the right of the controls set your PC's volume. In the Clipboard view, the pin on a text item keeps it at the top.

**⚙️ Settings window.** Right-click the tray icon (next to the clock) and choose **Settings…**, and a normal window opens. A sidebar on the left has the pages, and the settings of the chosen page are shown on the right in cards, each with a short line explaining it. The window can be made bigger or smaller (down to its starting width), maximised, or put in full screen with **F11** (**Esc** leaves full screen). It opens again with the size and place it had when you closed it (on the same screen if that is still there). When a page is taller than the window, scroll it with the mouse wheel, the **Page Up**, **Page Down**, **Home** and **End** keys, or by dragging the thin scroll bar on the right (a click on its track jumps there); the title stays in place.

- **General:** **Start with Windows**, and the **Hotkey**: **Open with a key** switches it on (it is off to begin with), and **Key combination** takes the keys you press (it needs Ctrl, Alt or Win with a letter, digit, F key or Space; Ctrl + Alt + N to begin with). Press it anywhere to open the notch and again to close it. Sky Island registers just that one combination with Windows and sees no other key; if another program already uses it, the row says so.
- **Notch:** everything about the notch itself. How it opens: **Open the notch** by hovering over it or by clicking it, with an **Open delay** and a **Close delay** (how long the mouse rests on the notch before it opens and how long it is away before it closes). How it hides: **Hide in full screen** and **Auto-hide notch**. Then **Notch size** (Auto, 100%, 115%, 130% or 150%) makes the whole notch bigger or smaller; Auto keeps it the same share of your screen's width. **Minimal notch** makes the closed notch a smaller pill while nothing is playing. Then **Show clock** and **24-hour clock**, and **Enable eyes**, **Idle eyes** and **Eye style** (Classic, Dots, Cat or Glow). With more than one screen, **Show on** picks the screen the notch is on (Main, Screen 2 or Screen 3, as Windows numbers them, or **All**: the notch is on one screen at a time, and it moves to another screen when you touch the top edge of that screen where the notch would be; for that, Sky Island watches the mouse position with a Windows mouse hook, only while All is chosen and there is more than one screen, and it records and sends nothing); a taskbar along the top keeps the notch below it.
- **Appearance:** **Theme** (Dark, Light, or Follow System, the Windows theme) and **Animation** (Calm, Normal, Snappy, or None, which has no animation at all).
- **Tabs:** switch each tab on or off (**Music**, **Timer**, **Water tracker**, **Weather**, **Prayer times**, **System stats**, **Shelf**, **To-do**, **Calendar** and **Clipboard**; they are all on to begin with, and the notch always keeps at least one), and **Clipboard history**, with **Forget copied items** to remove them from the history after 5 minutes, 30 minutes or an hour.
- **Location:** your city, for the weather. Click the **City** box, type a city and press **Enter** (add a country or region after a comma to pick the right one, like `Springfield, Illinois`); Sky Island looks it up once and remembers where it is. **Detect my location** uses Windows Location, so it is accurate (Windows' Location setting has to be on for desktop apps; the place name for the coordinates comes from OpenStreetMap's Nominatim, which is sent the coordinates). If Windows Location is off, it asks ipwho.is where your internet connection is instead, which only gives an approximate place and sends your IP address; the row says when that happened. It only happens when you press the button, and **Clear location** forgets the city and deletes the weather and prayer times kept for it. **Temperature** is in °C or °F. **Prayer times** has the calculation **Method** (click the arrows to step through the list; choosing a city picks the usual one for its country, such as JAKIM for Malaysia, MUIS for Singapore or Kemenag for Indonesia) and the **Asr** setting (Standard or Hanafi).
- **Sounds:** **Sounds** on or off, the **Volume** of the chimes (you hear the new level when you let go of the slider), and the timer alarm: how often it sounds again, how long it rings if nobody stops it, and the **Alarm sound**: click the button to pick your own sound file (WAV, MP3, M4A, WMA or AAC) in the normal Windows Open dialog, hear it right away, and click the × to go back to the built-in chime. A long file is left to finish instead of restarting every repeat, and the Stop button cuts it off. It follows the Volume slider.
- **Calendar:** a preview of the calendar, **Week starts on** (Monday by default, or Sunday) and **Default view** (a month, or just one week with a row for each day).
- **Shelf:** **Start with an empty shelf** clears the shelf's files each time Sky Island starts.
- **Notices:** choose which changes pop up on the notch: volume, brightness, Caps Lock and Num Lock, battery, Wi-Fi and internet, headphones and speakers, and **Prayer time** (a notice and a chime when a prayer time comes, with the Prayer tab on). **New version** looks on GitHub when Sky Island starts and every 6 hours; when a newer version has been released the notch says so (once for each version) and a dot appears next to **About**, where you can download it. Switch it off and Sky Island never looks by itself.
- **About:** the version, **Check for updates**, the **Changelog** (press View to read what changed in each version, taken from the project's releases on GitHub, with Back to return), **Open data folder**, **Lyrics cache** (the **Clear** button deletes the lyrics saved for the songs you played) and **Reset to defaults** (click it twice to confirm), then **Info** (the project's page and who made it) and **Thanks** (LRCLIB for the lyrics, Open-Meteo for the weather, Aladhan for the prayer times, and the Rust crates it is built with). **Check for updates** asks GitHub whether a newer version has been released and tells you, or that you have the latest. If there is one, the button becomes **Download**: press it and the new setup is downloaded (with the percentage shown), checked against the size and SHA-256 that GitHub reports for it, and the button becomes **Install**. Press that and the setup opens and closes Sky Island; follow its steps and leave **Launch Sky Island** ticked to start the app again. Nothing is downloaded or installed unless you press the buttons.
- To quit, use **Exit** in the tray menu. The tray menu is short: **Settings…**, **About Sky Island** (opens the About page) and **Exit**.

Changes apply right away and are saved as you make them. The Up and Down arrow keys change page, and Esc closes the window.

**🌗 Windows theme.** In the settings window choose **Follow System** under Theme and the notch switches to a light look whenever Windows is using its light theme, and back to dark when Windows goes dark. It follows changes while the app is running. With Dark or Light chosen, the notch keeps that look whatever Windows does. Album covers and file icons keep their own colours.

**🙈 Auto-hide.** Turn on **Auto-hide notch** in the settings window and the notch slides up out of view whenever you are not using it, leaving only a thin edge at the top of the screen. Move the mouse to where it was and it slides back down. It also comes down by itself for timers, reminders and system notices.

**📋 Clipboard.** Everything you copy is added to the top of the list, newest first. Text shows as one line, and a picture, such as a screenshot, shows as a small preview with its size. Click an item to put it back on the clipboard, and use the `×` to remove one item. The **Clear** button empties the list and the Windows clipboard itself. The list keeps the last 30 items and the mouse wheel scrolls through them.

## Your data and privacy

Everything stays on your computer, in `%APPDATA%\Sky-Island`. Files are saved all at once, so a crash cannot leave one half-written, and a file that is found damaged is kept next to the others as `<name>.corrupt` instead of being overwritten.

| File | What it holds |
|---|---|
| `settings.json` | Your settings |
| `settings-window.json` | The size and place the settings window had when you closed it |
| `timer.json` | The timer length you last set |
| `water.json` | Today's count, your goal and your reminder choice |
| `todo.json` | Your tasks, with their calendar day |
| `shelf.json` | The paths of the files on the shelf (the files themselves are never copied) |
| `pinned.json` | The clipboard items you pinned (text only), if any |
| `prayer.json` | The prayer times fetched last, so the Prayer tab and its reminders work offline |
| `weather.json` | The last forecast, so the Weather tab shows something at once and offline |
| `lyrics\` | Lyrics you have already looked up (the **Clear** button of the Lyrics cache row in About deletes them) |
| `crash.log` | Only written if the app crashes, with the error message |

The clipboard history is kept in memory only and is gone when you close the app. The one exception is the text items you pin yourself, which are kept in `pinned.json` (up to 10) until you unpin or remove them. It holds text and pictures, and it skips anything that an app, such as a password manager, marks as private. Pictures are stored compressed, and the oldest ones are dropped once they add up to about 48 MB. You can turn it off with **Clipboard history** in the settings window, or have items forgotten after a while with **Forget copied items**. Windows has its own clipboard history (Windows key + V), which Sky Island does not touch.

The app makes a few kinds of network request. The weather (only while the **Weather** tab is on) asks Open-Meteo for the forecast of your city's coordinates, every 30 minutes and when you open the tab, and searching for a city sends the name you typed to Open-Meteo. **Detect my location** (only when you press it) takes the position from Windows Location and sends its coordinates to OpenStreetMap's Nominatim to get the city's name; with Windows Location off it asks ipwho.is instead, which sends it your IP address. The prayer times (only while the **Prayer** tab is on) ask Aladhan for this month and the next for your city's coordinates, time zone, method and Asr setting, when the tab turns on, when one of those changes and every 6 hours; the weather is kept in `weather.json` so the tab has something to show offline. The lyrics lookup sends the song title, artist and length to LRCLIB. **Check for updates** asks GitHub for the latest release, and the Download button fetches its setup file from GitHub; both only when you press them, except that **New version** (Settings > Notices, on by default) also asks GitHub for the latest release at start and every 6 hours (it never downloads anything by itself); nothing about you is sent. The downloaded setup goes in your temporary folder and is deleted by itself the next time Sky Island starts after the update (a newer setup you downloaded but did not install is kept). There are no accounts and no tracking.

To show Caps Lock and Num Lock notices, the app installs a Windows keyboard hook. It only checks whether the key pressed is Caps Lock or Num Lock. It never records, stores or sends anything you type. Some antivirus programs are cautious about any keyboard hook, so it is worth knowing about.

If you turn on **Start with Windows**, the app adds one value named `Sky Island` to your per-user startup list (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`). Turning it off removes it.

## Known limitations

- Windows only.
- Chrome and YouTube do not offer next or previous for a single video. Next is greyed out and Previous restarts the video. Playlists work normally.
- Brightness notices only work on laptop screens, and only if the graphics driver lets the app read the brightness.
- Lyrics depend on what LRCLIB has, so some songs have none.
- The clipboard history holds text and pictures, not copied files, and only text can be pinned.
- The weather and prayer times need an internet connection to refresh (the last ones are kept for offline). **Detect my location** is only as exact as Windows Location; without it, it is a guess from your internet connection.
- With **Show on** set to All, the notch follows the mouse to the top edge of another screen; it has not been tried on every multi-screen layout.

## License

Released under the [MIT License](LICENSE). The open-source crates it is built with, and their licences, are listed in [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt); `scripts/third-party.ps1` rebuilds that file.
