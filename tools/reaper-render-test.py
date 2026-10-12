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


def plugin_state(extra_fields: dict | None = None, extra_params: dict | None = None) -> str:
    """The kit as a base64 CLAP state chunk. nice-plug prefixes its JSON with a u64 LE length."""
    kit = json.load(open(KIT))
    kit["fields"].update(extra_fields or {})
    kit["params"].update(extra_params or {})
    base = os.path.dirname(KIT)
    for key, value in kit["fields"].items():
        if key.startswith("layers_"):
            layers = json.loads(value)
            for layer in layers.get("layers", []):
                if layer.get("path"):
                    layer["path"] = os.path.join(base, layer["path"])
            kit["fields"][key] = json.dumps(layers)
    data = json.dumps(kit).encode()
    b64 = base64.b64encode(struct.pack("<Q", len(data)) + data).decode()
    return "\n".join("          " + b64[i : i + 128] for i in range(0, len(b64), 128))


def midi_block(events: list[tuple[float, int, int, int]], length_beats: int) -> str:
    """REAPER MIDI source lines for `(beat, status, data1, data2)` events."""
    ticks = sorted((int(beat * PPQ), status, d1, d2) for beat, status, d1, d2 in events)
    lines, last = [], 0
    for tick, status, d1, d2 in ticks:
        lines.append(f"        E {tick - last} {status:02x} {d1:02x} {d2:02x}")
        last = tick
    lines.append(f"        E {length_beats * PPQ - last} b0 7b 00")
    return "\n".join(lines)


def hits(beats, note, vel=0x7F):
    for beat in beats:
        yield (beat, 0x90, note, vel)
        yield (beat + 0.25, 0x80, note, 0)


def groove() -> tuple[str, list[float]]:
    """Two bars of kick, snare and hats. Returns the MIDI block and the kick times in seconds."""
    events = list(hits(range(8), 36))
    events += hits([1, 3, 5, 7], 37)
    events += hits([e * 0.5 for e in range(16)], 40, 0x60)
    events += hits([7.5], 41)
    kicks = [beat * 60 / BPM for beat in range(8)]
    return midi_block(events, 8), kicks


def project(out_wav: str, state: str, midi: str) -> str:
    return f"""<REAPER_PROJECT 0.1 "7.0" 1700000000
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
{state}
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


def read_left(path: str) -> tuple[list[float], int]:
    with wave.open(path) as w:
        n, ch, width, sr = w.getnframes(), w.getnchannels(), w.getsampwidth(), w.getframerate()
        raw = w.readframes(n)
    scale = float(1 << (8 * width - 1))
    step = ch * width
    left = [int.from_bytes(raw[i : i + width], "little", signed=True) / scale for i in range(0, len(raw), step)]
    return left, sr


def render(reaper: str, rpp: str) -> tuple[list[float], int] | None:
    tmp = tempfile.mkdtemp(prefix="klopfer-reaper-")
    try:
        out_wav = os.path.join(tmp, "render.wav")
        rpp_path = os.path.join(tmp, "test.rpp")
        open(rpp_path, "w").write(rpp.replace("{OUT}", out_wav))
        proc = subprocess.run(
            [reaper, "-nosplash", "-renderproject", rpp_path], capture_output=True, text=True, timeout=120
        )
        time.sleep(1)
        if proc.returncode != 0 or not os.path.exists(out_wav):
            print("REAPER failed:", proc.returncode, proc.stderr[-2000:])
            return None
        return read_left(out_wav)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def rms(s: list[float]) -> float:
    return math.sqrt(sum(x * x for x in s) / max(1, len(s)))


def test_groove(reaper: str) -> list[str]:
    """Kit loads from the state, notes trigger the right pads, levels are sane."""
    midi, kicks = groove()
    result = render(reaper, project("{OUT}", plugin_state(), midi))
    if result is None:
        return ["groove: render failed"]
    left, sr = result
    peak = max(abs(x) for x in left)
    failures = []
    if peak < 0.05:
        failures.append(f"groove: output is (nearly) silent, peak {peak:.3f}")
    if peak >= 0.999:
        failures.append(f"groove: output clips, peak {peak:.3f}")

    # Every kick must produce an onset: the 10 ms after it are much louder than the 10 ms before
    win = sr // 100
    for t in kicks:
        i = int(t * sr)
        before = left[max(0, i - win) : i] or [0.0]
        after = left[i : i + win]
        if rms(after) < 0.05 or rms(after) < 2 * rms(before):
            failures.append(f"groove: no onset at {t:.2f}s (before {rms(before):.3f}, after {rms(after):.3f})")
    print(f"groove: {len(kicks)} kicks, peak {20 * math.log10(max(peak, 1e-9)):.1f} dBFS")
    return failures


def test_midi_learn(reaper: str) -> list[str]:
    """A saved MIDI learn assignment (CC 20 -> master) works in the host: CC value 0 halfway through
    turns the master down to -60 dB."""
    events = list(hits(range(8), 36))
    events.append((3.9, 0xB0, 20, 0))
    state = plugin_state({"midi-map": json.dumps([[20, "master"]])})
    result = render(reaper, project("{OUT}", state, midi_block(events, 8)))
    if result is None:
        return ["midi learn: render failed"]
    left, sr = result
    first, second = left[: 2 * sr], left[2 * sr : 4 * sr]
    print(f"midi learn: rms before CC {rms(first):.4f}, after {rms(second):.5f}")
    if rms(first) < 0.01:
        return ["midi learn: kicks before the CC are silent"]
    if rms(second) > rms(first) / 100:
        return ["midi learn: CC 20 did not turn the master down"]
    return []


def test_time_stretch(reaper: str) -> list[str]:
    """The 2 s loop on pad 14, stretched to 200 %, rendered offline: it must sound for 4 s. Offline
    renders stretch synchronously, so even the first note uses the stretched sample."""
    events = list(hits([0], 36 + 13))
    state = plugin_state(extra_params={"strmode_14": {"string": "smooth"}, "strlen_14": {"f32": 2.0}})
    result = render(reaper, project("{OUT}", state, midi_block(events, 10)))
    if result is None:
        return ["time stretch: render failed"]
    left, sr = result
    late = rms(left[int(2.5 * sr) : int(3.8 * sr)])
    after = rms(left[int(4.3 * sr) : int(4.8 * sr)])
    print(f"time stretch: rms 2.5-3.8 s {late:.4f}, after 4.3 s {after:.5f}")
    if late < 0.005:
        return ["time stretch: the loop doesn't play beyond its original length"]
    if after > late / 10:
        return ["time stretch: the stretched loop is longer than expected"]
    return []


def main() -> int:
    reaper = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_REAPER
    if not os.path.exists(KIT):
        print("Test kit missing, run: cargo run -p testkit -- testkit-out")
        return 1

    failures = test_groove(reaper) + test_midi_learn(reaper) + test_time_stretch(reaper)
    if failures:
        print("FAILED:\n  " + "\n  ".join(failures))
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
