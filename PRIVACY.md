# Sky Island privacy policy

Last updated: 2026-11-15

Sky Island is a free, open-source notch for the top of a Windows screen, made by Dannyz15
([github.com/Dannyz15/Sky-Island](https://github.com/Dannyz15/Sky-Island)). This page says what the app stores and what it sends.

**In short: your data stays on your computer. There are no accounts, no advertising and no tracking, and the app sends nothing to the developer.**

## What is stored on your PC

Settings, to-do tasks, the file shelf (the paths of files you drop on it), timers, your city and the last weather and prayer times are saved as small files in the app's data folder
(`%APPDATA%\Sky-Island`; for the Microsoft Store version Windows keeps this folder private to the app). The clipboard history is kept in memory only, except text you pin (up to 10).
Nothing is uploaded. Uninstalling the app removes the Store version's data; **Settings > About > Reset to defaults** puts every setting back.

## What is sent over the internet, and why

Each request goes straight from your PC to the service named below. The developer runs no server and never receives any of it.

| Feature | Service | What is sent |
|---|---|---|
| Weather | [Open-Meteo](https://open-meteo.com) | The coordinates of your city |
| Prayer times | [Aladhan](https://aladhan.com); in Malaysia, [JAKIM e-Solat](https://www.e-solat.gov.my) | The coordinates of your city; for e-Solat only the code of your zone (for example SGR01) |
| City search (only when you type a city and press Enter) | [OpenStreetMap Nominatim](https://nominatim.openstreetmap.org) | The name you typed |
| Detect my location (only when you press it) | Windows Location, then OpenStreetMap Nominatim to name the place | The position, rounded to about a kilometre first. If Windows Location is off, [ipwho.is](https://ipwho.is) is asked instead and sees your IP address |
| Synced lyrics (can be switched off with **Find lyrics online**) | [LRCLIB](https://lrclib.net) | The title, artist and length of the song that is playing |
| New version check (not in the Microsoft Store version, which the Store updates) | GitHub | An ordinary request for the latest release |

These services have their own privacy policies. A request also shows them your IP address, as any internet request does.

## What the app reads on your PC, and does not keep

- **Sound:** to make the bars follow the music, the app measures how loud each pitch is in the sound your PC plays (loopback), about 45 times a second. Nothing is recorded, saved or sent, and the microphone is not used. Switch it off with **Bars follow the sound**.
- **Media:** it reads the title, artist and cover of what is playing from Windows so it can show it.
- **Screen around the notch:** with Clear or Smoky glass the app copies the part of the screen right around the notch to show it blurred. The copy stays in memory and is never saved or sent.
- **Keyboard:** to show Caps Lock and Num Lock notices, a keyboard hook checks only whether one of those two keys was pressed. It never records or sends what you type.
- **Location:** only when you press **Detect my location**.

## Children, sharing and changes

The app has no accounts and collects no personal information, so it shares nothing with anyone and is suitable for all ages. If this policy changes, the new version is published here with a new date.

## Contact

Questions or concerns: [open an issue](https://github.com/Dannyz15/Sky-Island/issues).
