#!/usr/bin/env python3
"""End-to-end test in a real host: renders a REAPER project that plays the test kit through the
installed Klopfer CLAP plugin and checks the result.

Prerequisites:
  cargo run -p testkit -- testkit-out
  cargo xtask bundle klopfer --release   (then install Klopfer.clap to your CLAP folder)

Usage: python3 tools/reaper-render-test.py [path/to/REAPER]

REAPER opens briefly while rendering.
"""

import base64
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import wave

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
KIT = os.path.join(ROOT, "testkit-out", "Testkit.klopfer")
DEFAULT_REAPER = "/Applications/REAPER.app/Contents/MacOS/REAPER"
BPM = 120
PPQ = 960


def plugin_state() -> str:
    """The kit as a base64 CLAP state chunk. nih-plug prefixes its JSON with a u64 LE length."""
    kit = json.load(open(KIT))
    base = os.path.dirname(KIT)
    for key, value in kit["fields"].items():
        if key.startswith("path_"):
            kit["fields"][key] = json.dumps(os.path.join(base, json.loads(value)))
    data = json.dumps(kit).encode()
    b64 = base64.b64encode(struct.pack("<Q", len(data)) + data).decode()
    return "\n".join("          " + b64[i : i + 128] for i in range(0, len(b64), 128))


def midi_events() -> tuple[str, list[float]]:
    """Two bars of kick, snare and hats. Returns the MIDI block and the kick times in seconds."""
    events = []

    def hit(beat, note, vel=0x7F):
        events.append((int(beat * PPQ), 0x90, note, vel))
        events.append((int((beat + 0.25) * PPQ), 0x80, note, 0))

    for bar in range(2):
        b = bar * 4
        for q in range(4):
            hit(b + q, 36)
        hit(b + 1, 37)
        hit(b + 3, 37)
        for e in range(8):
            hit(b + e * 0.5, 40, 0x60)
    hit(7.5, 41)
    events.sort()

    lines, last = [], 0
    for tick, status, note, vel in events:
        lines.append(f"        E {tick - last} {status:02x} {note:02x} {vel:02x}")
        last = tick
    lines.append(f"        E {8 * PPQ - last} b0 7b 00")
    kicks = [beat * 60 / BPM for beat in range(8)]
    return "\n".join(lines), kicks


def project(out_wav: str) -> tuple[str, list[float]]:
    midi, kicks = midi_events()
    rpp = f"""<REAPER_PROJECT 0.1 "7.0" 1700000000
  TEMPO {BPM} 4 4
  SAMPLERATE 48000 0 0
  RENDER_FILE "{out_wav}"
  RENDER_PATTERN ""
  RENDER_FMT 0 2 48000
  RENDER_1X 0
  RENDER_RANGE 1 0 0 18 1000
  RENDER_RESAMPLE 3 0 1
  RENDER_ADDTOPROJ 0
  RENDER_STEMS 0
  RENDER_DITHER 0
  <RENDER_CFG
    ZXZhdxgAAQ==
  >
  <TRACK
    NAME Klopfer
    NCHAN 2
    <FXCHAIN
      SHOW 0
      LASTSEL 0
      DOCKED 0
      BYPASS 0 0 0
      <CLAP "CLAPi: Klopfer (Klopfer)" com.klopfer.drum-sampler ""
        CFG 4 760 456 ""
        <STATE
{plugin_state()}
        >
      >
      FLOATPOS 0 0 0 0
      WAK 0 0
    >
    <ITEM
      POSITION 0
      LENGTH 4
      <SOURCE MIDI
        HASDATA 1 {PPQ} QN
{midi}
      >
    >
  >
>
"""
    return rpp, kicks


def read_left(path: str) -> tuple[list[float], int]:
    with wave.open(path) as w:
        n, ch, width, sr = w.getnframes(), w.getnchannels(), w.getsampwidth(), w.getframerate()
        raw = w.readframes(n)
    scale = float(1 << (8 * width - 1))
    step = ch * width
    left = [int.from_bytes(raw[i : i + width], "little", signed=True) / scale for i in range(0, len(raw), step)]
    return left, sr


def main() -> int:
    reaper = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_REAPER
    if not os.path.exists(KIT):
        print("Test kit missing, run: cargo run -p testkit -- testkit-out")
        return 1

    tmp = tempfile.mkdtemp(prefix="klopfer-reaper-")
    out_wav = os.path.join(tmp, "render.wav")
    rpp_path = os.path.join(tmp, "test.rpp")
    rpp, kicks = project(out_wav)
    open(rpp_path, "w").write(rpp)

    proc = subprocess.run([reaper, "-nosplash", "-renderproject", rpp_path], capture_output=True, text=True, timeout=120)
    time.sleep(1)
    if proc.returncode != 0 or not os.path.exists(out_wav):
        print("REAPER failed:", proc.returncode, proc.stderr[-2000:])
        return 1

    left, sr = read_left(out_wav)
    peak = max(abs(x) for x in left)
    failures = []
    if peak < 0.05:
        failures.append(f"output is (nearly) silent, peak {peak:.3f}")
    if peak >= 0.999:
        failures.append(f"output clips, peak {peak:.3f}")

    # Every kick must produce an onset: the 10 ms after it are much louder than the 10 ms before
    win = sr // 100
    for t in kicks:
        i = int(t * sr)
        before = left[max(0, i - win) : i] or [0.0]
        after = left[i : i + win]
        rms = lambda s: math.sqrt(sum(x * x for x in s) / len(s))
        if rms(after) < 0.05 or rms(after) < 2 * rms(before):
            failures.append(f"no onset at {t:.2f}s (before {rms(before):.3f}, after {rms(after):.3f})")

    shutil.rmtree(tmp, ignore_errors=True)
    if failures:
        print("FAILED:\n  " + "\n  ".join(failures))
        return 1
    print(f"OK: {len(kicks)} onsets, peak {20 * math.log10(peak):.1f} dBFS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
