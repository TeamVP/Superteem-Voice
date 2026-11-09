<p align="center">
  <picture>
    <source srcset="public/Superteem-only-light.png" media="(prefers-color-scheme: dark)">
    <img src="public/Superteem-only-dark.png" alt="Superteem" width="320">
  </picture>
</p>

<h1 align="center">Voice by Superteem</h1>

<p align="center">
  <b>Voice notes transcribed on your own device. No cloud.</b><br>
  <a href="https://voice.superteem.com">Download</a> ·
  <a href="https://github.com/TeamVP/Superteem-Voice/releases">Releases</a> ·
  <a href="#license">Open source</a>
</p>

Voice by Superteem turns speech into text on your PC or phone. You can dictate notes, record meetings and give voice commands. Nothing is uploaded: there's no account and no server, and the app doesn't track you.

## Features

- **Voice notes:** start a note and talk. Your words are written into it as you go.
- **Meeting transcripts:** record from several microphones at once. You get a timecoded transcript with speakers recognised by voice print, plus bookmarks and action items.
- **Voice commands (Windows):** say "Hey Superteem, start meeting", "take a note" or "bookmark that".
- **Plain files:** each meeting is a folder of WAV audio and `transcript.json`. Text, Markdown and SRT copies are written next to them and regenerated on every edit. On Windows the folders live in `Documents\Superteem Voice`.
- **Voice Pin (in development):** support for Seeed's wearable [reSpeaker Clip](https://github.com/Seeed-Studio/reSpeaker_Clip), which runs Seeed's own open-source firmware.

Transcription uses [whisper.cpp](https://github.com/ggerganov/whisper.cpp) with the English `base.en` model. The app downloads the model (about 140 MB) once on first run and then works offline.

## Download

Get the latest Windows installer and Android APK from **[voice.superteem.com](https://voice.superteem.com)**.

| Device | Requires | File |
| --- | --- | --- |
| Windows | Windows 10 or 11, 64-bit | `Voice-by-Superteem-Setup-x64.exe` |
| Android | Android 10 or later | `Voice-by-Superteem-Android.apk` |

## Repository layout

| Folder | What's in it |
| --- | --- |
| `core/` | Rust library shared by every app: pin protocol, ADPCM, keyword spotting, voice prints, speaker attribution, exports |
| `desktop/` | The Tauri app for Windows and Android: Rust in `src/`, the interface in `ui/`, the Android project in `gen/android/` |
| `firmware/` | Step 0 radio test firmware for a Seeed XIAO nRF52840 Sense. A development tool for testing Bluetooth audio, not shipped |
| `protocol/` | The Bluetooth protocol the test firmware speaks, and its test vectors |
| `site/` | The download page at voice.superteem.com |
| `public/` | Logos and icons |

## Building from source

The apps are built on Windows. You need [Rust](https://rustup.rs), [Node.js](https://nodejs.org), Python, and Visual Studio 2022 with the C++ tools and the Windows SDK. The build scripts fetch the rest (libclang and a patched whisper-rs-sys).

**Windows.** Build and install for the current user, with a Start Menu shortcut:

```powershell
.\desktop\install.ps1
```

To build the installer instead:

```powershell
. .\desktop\build-env.ps1
cd desktop
npm ci
npm run tauri -- build --bundles nsis
```

**Android.** You also need Android Studio, with the NDK and CMake installed from its SDK Manager. Then run:

```powershell
rustup target add aarch64-linux-android x86_64-linux-android
.\desktop\build-android.ps1 -Install
```

**Radio test firmware (development only).** Install [west](https://docs.zephyrproject.org/latest/develop/west/index.html), then from the repository root:

```sh
west init -l firmware
west update
west sdk install -t arm-zephyr-eabi
west build -b xiao_ble/nrf52840/sense firmware
```

**Tests:**

```sh
cargo test --manifest-path core/Cargo.toml
```

## Releasing

1. Set the version in `desktop/tauri.conf.json`.
2. Push a matching tag, for example `v0.2.0`.

The Release workflow then builds the Windows and Android apps and publishes a GitHub release, and the download page links to it automatically. Android releases are signed with the key in the repository secrets: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`.

## License

Everything in this repository is released under the [GNU General Public License v3.0](LICENSE).

The reSpeaker Clip's firmware is not part of this project. It is Seeed's, published by Seeed Technology under the Apache License 2.0 at [Seeed-Studio/reSpeaker_Clip](https://github.com/Seeed-Studio/reSpeaker_Clip).
