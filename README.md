# Umbra

A clipping app for Windows 11. It keeps the last seconds of your screen in memory, and a hotkey
saves them as a clip, with the game, Discord and your mic on their own tracks.

## Building

You need Windows 11, Rust (stable, MSVC toolchain), Visual Studio 2022 with the C++ workload,
and Node.js. Then run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build.ps1
```

It builds everything and opens the folder with the installer and a portable zip.

## Privacy

Umbra doesn't connect to the internet at all. You don't need an account and nothing gets sent
anywhere, so your clips only ever end up on your own PC.

Licensed under the [GNU GPL 3.0](LICENSE).
