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

Voice by Superteem turns speech into text on your PC, Mac or phone. You can dictate notes, record meetings and give voice commands. Recordings, transcripts and notes stay on the device, and there's no account. Checking for an update sends Superteem the app version, the operating system and its version, the models in use, the configured languages, and whether the check was automatic, requested, or the start of an install.

## Features

- **Voice notes:** start a note and talk. Your words are written into it as you go.
- **Meeting transcripts:** record from several microphones at once. You get a timecoded transcript with speakers recognised by voice print, plus bookmarks and action items.
- **Summaries and action items:** decisions, action items (who, what, when) and open questions, written by a small language model that runs on your device. Templates for standups, user interviews, sales calls, lectures and more stack at the top of a meeting or note; every point links to the line it came from. Make your own from a list of headings.
- **Many languages:** each note or meeting is in one language, or two for people who switch between them, such as Chinese and English or Welsh and English. Choosing a language the app hasn't used yet adds it to yours and offers the speech model it needs.
- **Translation:** translate a meeting or note line by line into your other languages, and correct any line. For languages the language model writes less well, analyses are made from an English translation and can be translated back.
- **Listen to translations:** read a translation aloud line by line, following along, or start from any line. Natural voices for about 40 languages download in one click (Windows).
- **Optimise for this device:** one button picks the speech model, analysis model, graphics processor and a reading voice for each of your languages, for what your device runs well and the room it has, then tries each one. Settings › AI on this computer shows what runs and how fast.
- **Voice commands (Windows and Mac):** say "Hey Superteem, start meeting", "take a note" or "bookmark that".
- **Plain files:** each meeting is a folder of WAV audio and `transcript.json`. Text, Markdown and SRT copies are written next to them and regenerated on every edit. On Windows and the Mac the folders live in `Documents/Superteem Voice`; on iPhone, in the Files app under On My iPhone.
- **Voice Pin (in development):** support for Seeed's wearable [reSpeaker Clip](https://github.com/Seeed-Studio/reSpeaker_Clip), which runs Seeed's own open-source firmware.

Summaries and translations use [llama.cpp](https://github.com/ggml-org/llama.cpp) with a small model you choose in Settings, downloaded once: Qwen3 1.7B (about 1.1 GB) is quick, and Qwen3.5 4B (about 2.7 GB) is much better for translation and for languages such as Welsh. You can also point it at a local server such as Ollama. On Windows, models run on the graphics processor through Vulkan when there's one that can (NVIDIA, AMD or Intel), and on the processor otherwise; the Mac uses Metal.

Transcription uses [whisper.cpp](https://github.com/ggerganov/whisper.cpp). English uses the English `base.en` model, which the app downloads (about 150 MB) once on first run. Other languages use a multilingual Whisper model, offered when you first choose one: `large-v3-turbo` (about 570 MB) on PCs with a graphics card, `small` (about 260 MB) on phones and other PCs. Both then work offline. Spoken commands are English for now.

## Download

Get the latest Windows installer, Mac disk image and Android APK from **[voice.superteem.com](https://voice.superteem.com)**.

| Device | Requires | File |
| --- | --- | --- |
| Windows | Windows 10 or 11, 64-bit | `Voice-by-Superteem-Setup-x64.exe` |
| Mac | macOS 13.3 or later, Apple silicon | `Voice-by-Superteem-macOS.dmg` |
| Android | Android 10 or later | `Voice-by-Superteem-Android.apk` |
| iPhone | iOS 16.4 or later | Through TestFlight and the App Store, once signing is set up |

## Repository layout

| Folder | What's in it |
| --- | --- |
| `core/` | Rust library shared by every app: pin protocol, ADPCM, keyword spotting, voice prints, speaker attribution, exports |
| `desktop/` | The Tauri app for Windows, Mac, Android and iPhone: Rust in `src/`, the interface in `ui/`, the Android project in `gen/android/`. The iPhone's Xcode project is generated in `gen/apple/` |
| `firmware/` | Step 0 radio test firmware for a Seeed XIAO nRF52840 Sense. A development tool for testing Bluetooth audio, not shipped |
| `protocol/` | The Bluetooth protocol the test firmware speaks, and its test vectors |
| `site/` | The download page at voice.superteem.com |
| `public/` | Logos and icons |

## Building from source

The Windows and Android apps are built on Windows, and the Mac and iPhone apps on a Mac. You need [Rust](https://rustup.rs), [Node.js](https://nodejs.org), Python, and Visual Studio 2022 with the C++ tools and the Windows SDK. Windows builds also need the [Vulkan SDK](https://vulkan.lunarg.com) (`winget install KhronosGroup.VulkanSDK`) for the graphics processor. The build scripts fetch the rest (libclang and a patched whisper-rs-sys).

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

**Mac.** On a Mac with Xcode, [Rust](https://rustup.rs), [Node.js](https://nodejs.org) and CMake (`brew install cmake`):

```sh
source desktop/build-env.sh
cd desktop
npm ci
npm run tauri -- build --bundles dmg
```

**iPhone.** Also on a Mac, with XcodeGen (`brew install xcodegen`). The first time, make the Xcode project:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
source desktop/build-env.sh
cd desktop
npm run tauri -- ios init
```

Then `npm run tauri -- ios dev` runs it on a connected iPhone or the Simulator. To sign it, set your Apple team ID in `APPLE_DEVELOPMENT_TEAM`.

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

The Release workflow then builds the Windows, Mac and Android apps and publishes a GitHub release, and the download page links to it automatically. Android releases are signed with the key in the repository secrets: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`.

The Mac app is signed ad hoc unless the Developer ID secrets are set: `APPLE_CERTIFICATE` (the .p12, base64), `APPLE_CERTIFICATE_PASSWORD` and `APPLE_SIGNING_IDENTITY`. It is notarized too when the App Store Connect key is set: `APPLE_API_ISSUER`, `APPLE_API_KEY` (the key ID) and `APPLE_API_KEY_P8` (the key file's text). The iPhone app is built for the Simulator unless `APPLE_DEVELOPMENT_TEAM`, `IOS_CERTIFICATE`, `IOS_CERTIFICATE_PASSWORD` and `IOS_MOBILE_PROVISION` are set; then it is built for the App Store, and with the App Store Connect key a tagged release is uploaded to TestFlight.

## License

Everything in this repository is released under the [GNU General Public License v3.0](LICENSE).

The reSpeaker Clip's firmware is not part of this project. It is Seeed's, published by Seeed Technology under the Apache License 2.0 at [Seeed-Studio/reSpeaker_Clip](https://github.com/Seeed-Studio/reSpeaker_Clip).
