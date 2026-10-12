# Klopfer

A 16 pad drum sampler plugin (VST3 + CLAP) for Windows, macOS and Linux, written in Rust with
[nih-plug](https://github.com/robbert-vdh/nih-plug). Layout and workflow are inspired by TAL-Drum.

License: GPL-3.0-or-later (required by the VST3 bindings nih-plug uses).

## Features

- 16 pads, MIDI notes from C1 (36) upwards, freely assignable; pads sharing a note are layered
- Per pad: tune ±48 st + fine, start/end, reverse, one-shot/gate mode, AHDSR, velocity
  sensitivity, LP/HP/BP filter with envelope and drive, pitch envelope, parametric EQ band,
  bit/sample rate reduction, volume, pan, mute, choke group (16), output (main + 15 stereo aux)
- WAV, AIFF, FLAC, OGG and MP3 samples, decoded on a background thread
- Projects store sample paths; missing samples can be relinked per pad or by searching a folder
- Kit files (`.klopfer`) store samples relative to the kit, so kit folders stay portable
- GUI: pad grid with audition, pad editor with waveform and draggable start/end markers, mixer view

## Layout

```
dsp/                  realtime DSP (engine, voices, envelopes, filters), no plugin dependency
src/                  plugin: parameters, sample bank/loader, kits, process(), editor/
xtask/                bundler (`cargo xtask bundle`)
tools/testkit/        generates a synthetic drum kit for development
tools/reaper-render-test.py   end-to-end test rendering through REAPER
```

## Building

```bash
cargo xtask bundle klopfer --release            # target/bundled/Klopfer.{vst3,clap}
cargo xtask bundle-universal klopfer --release  # macOS universal binary (arm64 + x86_64)
```

Install by copying the bundles to your plugin folders, e.g. on macOS
`~/Library/Audio/Plug-Ins/CLAP` and `~/Library/Audio/Plug-Ins/VST3`. Windows and Linux builds have
to be made on those systems.

Linux needs the X11/OpenGL development headers (e.g. `libx11-dev libxcursor-dev libxcb-dri2-0-dev
libxcb-icccm4-dev libx11-xcb-dev libgl-dev libasound2-dev libjack-dev`). The GUI runs on X11 and
via XWayland.

## Testing

```bash
cargo test --workspace
cargo run -p testkit -- testkit-out              # synthetic kit in testkit-out/
python3 tools/reaper-render-test.py              # needs REAPER and the installed CLAP
```

Standalone with the test kit preloaded (`KLOPFER_KIT` only works in the standalone build):

```bash
cargo build --release --features standalone --bin klopfer_standalone
KLOPFER_KIT=$PWD/testkit-out/Testkit.klopfer ./target/release/klopfer_standalone -b core-audio -l 2
```

`-l 2` selects the stereo layout. The standalone uses the output device's sample rate unless
`-r` is given, because nih-plug's CoreAudio/WASAPI backend aborts on a sample rate mismatch. Use
`-b dummy` for a GUI check without audio, `--midi-input "<device>"` for a MIDI keyboard.

## Known limitations

- Dragging files from the file manager onto pads is not supported yet: baseview delivers the drop
  events, but egui-baseview doesn't forward them to egui.
- macOS builds are only ad-hoc signed; distribution needs a Developer ID and notarization.
- Loading a kit while the host is playing can cause a short dropout.

## Roadmap

- Phase 4: velocity layers and round robin, LFO + mod matrix, 32/64 pads, MIDI learn
- Phase 5: slicing, recording, time stretch, SFZ import
