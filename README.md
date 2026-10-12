# Klopfer

A 16 pad drum sampler plugin (VST3 + CLAP) for Windows, macOS and Linux, written in Rust with
[nice-plug](https://codeberg.org/RustAudio/nice-plug), the successor of nih-plug. Layout and
workflow are inspired by TAL-Drum.

License: GPL-3.0-or-later.

## Features

- 32 pads in two banks (A/B), MIDI notes from C1 (36) upwards, freely assignable; pads sharing a
  note play together
- Up to 4 sample layers per pad with velocity ranges, gain and tuning; layer modes velocity,
  round robin and random
- Per pad: tune ±48 st + fine, start/end, reverse, one-shot/gate mode, AHDSR, velocity
  sensitivity, LP/HP/BP filter with envelope and drive, pitch envelope, parametric EQ band,
  bit/sample rate reduction, volume, pan, mute, choke group (16), output (main + 15 stereo aux)
- Modulation per pad: LFO (6 shapes, free or tempo-synced, retrigger), two AHDSR mod envelopes and
  an 8 slot matrix. Sources: LFO, mod envelopes, amp envelope, velocity, random, noise. Targets:
  pitch, cutoff, resonance, drive, pan, volume, sample start/end, LFO rate, bits, downsample, amp
  attack/decay
- MIDI learn: right-click any knob, toggle or fader. Assignments are stored in the project, not in
  kits, and work while the editor is closed
- Slicing (tab EDIT): transient detection or a beat grid with tempo guess; slices go onto pads
  that share the file with their own start/end; export of the slice rhythm as a MIDI file
- Time stretching (tab EDIT): Smooth (WSOLA) and Cycle (grainy, vintage sampler style) modes,
  length in percent or synced to the host tempo; Tune then changes pitch without changing length.
  Rendered in the background per layer, synchronously when the host renders offline
- Recording (tab REC) from a stereo sidechain input into the selected pad layer, optionally
  waiting for the signal and normalizing; saved as WAV to a configurable folder (default
  `~/Documents/Klopfer/Recordings`, stored in the user config directory)
- SFZ import via Load Kit or drag and drop: keys become pads, regions on a key become layers
  (velocity, `seq_length` round robin, `lorand`/`hirand` random), `group`/`off_by` become choke
  groups; volume, pan, tune, transpose and the amp envelope are taken over
- WAV, AIFF, FLAC, OGG and MP3 samples, decoded on a background thread
- Drag and drop from the file manager: onto a pad (several files fill the following pads), onto the
  waveform or a layer row (several files fill the following layers), `.klopfer` files anywhere load
  the kit
- Projects store sample paths; missing samples can be relinked per layer or by searching a folder
- Kit files (`.klopfer`) store samples relative to the kit, so kit folders stay portable. Kits and
  projects from the single-sample version are migrated to layer 1
- GUI: pad grid with audition, pad editor (MAIN / LAYERS / MOD) with waveform and draggable
  start/end markers, mixer view

Parameter IDs and enum variant IDs are stable and must not change, saved projects depend on them.

## Layout

```
dsp/                  realtime DSP (engine, voices, envelopes, filters), no plugin dependency
src/                  plugin: parameters, sample bank/loader, kits, process(), editor/
xtask/                bundler (`cargo xtask bundle`)
tools/testkit/        generates a synthetic drum kit for development
tools/reaper-render-test.py   end-to-end test rendering through REAPER
vendor/               forks of nice-plug, baseview and egui-baseview (git submodules), see vendor/README.md
```

## Building

```bash
git submodule update --init                     # patched dependencies in vendor/
cargo xtask bundle klopfer --release            # target/bundled/Klopfer.{vst3,clap}
cargo xtask bundle-universal klopfer --release  # macOS universal binary (arm64 + x86_64)
```

Install by copying the bundles to your plugin folders, e.g. on macOS
`~/Library/Audio/Plug-Ins/CLAP` and `~/Library/Audio/Plug-Ins/VST3`. Windows and Linux builds have
to be made on those systems.

Linux needs the X11/OpenGL/Wayland development headers (e.g. `libx11-dev libxcursor-dev
libxcb-dri2-0-dev libxcb-icccm4-dev libx11-xcb-dev libgl-dev libwayland-dev libasound2-dev
libjack-dev`). The GUI runs on X11 and
via XWayland.

## Testing

```bash
cargo test --workspace
cargo run -p testkit -- testkit-out              # synthetic kit (+ SFZ and a loop) in testkit-out/
python3 tools/reaper-render-test.py [REAPER]     # needs REAPER and the installed CLAP:
                                                 # groove, MIDI learn and offline time stretch
```

Standalone with the test kit preloaded (`KLOPFER_KIT` only works in the standalone build):

```bash
cargo build --release --features standalone --bin klopfer_standalone
KLOPFER_KIT=$PWD/testkit-out/Testkit.klopfer ./target/release/klopfer_standalone -b core-audio -l 2
```

`KLOPFER_EDITOR=pad=13,tab=mod` (also `tab=layers`, `tab=edit`, `tab=rec`, `view=mixer`) opens the standalone editor in
a given state, e.g. for screenshots. `-l 2` selects the stereo layout. Unless `-r` is given, the
standalone uses the JACK server's sample rate (JACK backend, also tried first without `-b`) or the
output device's: nice-plug's CoreAudio/WASAPI backend aborts on a sample rate mismatch, and its JACK
backend would initialize the plugin with the wrong rate. Use `-b dummy` for a GUI check without
audio, `--midi-input "<device>"` for a MIDI keyboard.

## Known limitations

- Drag and drop needs the patched GUI dependencies in `vendor/`. On Linux it is untested.
- When a time stretched rendering finishes, voices still playing the old one of that layer stop.
- SFZ: sample offsets, loops and opcodes not listed in `src/sfz.rs` are ignored; Battery and RX2
  kits aren't supported (proprietary formats).
- Recording needs audio routed to the sidechain input; takes are limited to 60 seconds.
- macOS builds are only ad-hoc signed; distribution needs a Developer ID and notarization.
- Loading a kit while the host is playing can cause a short dropout.

## Roadmap

All planned phases are done. Ideas: Windows/Linux CI builds, macOS notarization, per-layer
sample start/end, more SFZ opcodes.
