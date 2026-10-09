#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# The Klimper sound (clapandpay TASK-111.01, decision-009): real coins, cut from CC0 recordings
# (src/klimper/sound/sources/, see SOURCES.md there) into the files the app plays. Pure Python,
# no dependencies, deterministic: `python3 scripts/klimper-sound.py` always writes byte-identical
# files into src/klimper/sound/:
#
#   klimper-ding-<tier>.wav   coins arrive: a small rattle, longer and fuller per tier
#   klimper-grain-<1..8>.wav  single coin hits; the phone builds every turn of a shaking hand
#                             out of these, at random, and taps with each
#
# Tiers (USD bounds in src/klimper/sound/tiers.ts): coin, few, handful, pouch. A ding's weight is
# made of layers (more coins), speed (heavy coins move slower and sound lower) and, for the
# pouch, a low-passed body underneath. Listening notes in docs/sound.md.

import math
import os
import struct
import sys
import wave

RATE = 44_100
PEAK = 10 ** (-1 / 20)  # -1 dBFS

HERE = os.path.dirname(os.path.abspath(__file__))
SOUND = os.path.join(HERE, "..", "src", "klimper", "sound")
SOURCES = os.path.join(SOUND, "sources")


def load(name):
    with wave.open(os.path.join(SOURCES, f"{name}.wav")) as w:
        assert w.getnchannels() == 1 and w.getframerate() == RATE and w.getsampwidth() == 2, name
        n = w.getnframes()
        return [s / 32768 for s in struct.unpack(f"<{n}h", w.readframes(n))]


def write(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(round(s * 32767))) for s in samples))


# ---- editing primitives --------------------------------------------------------------------


def cut(x, start, duration=None):
    a = int(start * RATE)
    b = len(x) if duration is None else min(len(x), a + int(duration * RATE))
    return x[a:b]


def speed(x, rate):
    """Play `x` at `rate` (0.9 is slower and lower). Linear interpolation is enough for coins."""
    n = int(len(x) / rate)
    out = []
    for i in range(n):
        p = i * rate
        j = int(p)
        f = p - j
        a = x[min(j, len(x) - 1)]
        b = x[min(j + 1, len(x) - 1)]
        out.append(a * (1 - f) + b * f)
    return out


def gain(x, g):
    return [v * g for v in x]


def fade(x, fade_in=0.0, fade_out=0.0):
    out = list(x)
    n_in = int(fade_in * RATE)
    for i in range(min(n_in, len(out))):
        out[i] *= i / n_in
    n_out = int(fade_out * RATE)
    for i in range(min(n_out, len(out))):
        out[len(out) - 1 - i] *= i / n_out
    return out


def lowpass(x, cutoff):
    """One-pole low-pass: the body of a pouch, what you hear through a hand."""
    k = 1 - math.exp(-math.tau * cutoff / RATE)
    y = 0.0
    out = []
    for v in x:
        y += k * (v - y)
        out.append(y)
    return out


def mix(layers, length=None):
    """Sum of (samples, offset seconds); the result is as long as needed, or `length` seconds."""
    n = length and int(length * RATE)
    if n is None:
        n = max(int(off * RATE) + len(s) for s, off in layers)
    out = [0.0] * n
    for s, off in layers:
        start = int(off * RATE)
        for i, v in enumerate(s):
            j = start + i
            if j >= n:
                break
            out[j] += v
    return out


def normalise(x):
    peak = max(abs(v) for v in x) or 1.0
    return [max(-1.0, min(1.0, v * PEAK / peak)) for v in x]


def trim(x, floor=0.006, air=0.03):
    last = max(i for i, v in enumerate(x) if abs(v) > floor)
    return x[: min(len(x), last + int(air * RATE))]


# ---- the sounds ----------------------------------------------------------------------------
#
# The recordings are single gestures: a clink of 0.1 to 0.25 s and silence. The dings layer a
# few of them; the grains are single hits cut out of them.


def build():
    coins = load("kenney-handle-coins")  # coins moved in a hand: the clink is at 0.10 to 0.45 s
    toss = load("kenney-handle-coins-2")  # a smaller handful: the clink is at 0.15 to 0.34 s
    body = gain(lowpass(load("kenney-metal-heavy-0"), 180), 1.6)  # weight, felt more than heard
    clink = fade(cut(coins, 0.10, 0.40), 0.004, 0.05)
    tick = fade(cut(toss, 0.15, 0.19), 0.003, 0.05)

    out = {}
    # Coins arrive: a small rattle, denser and lower per tier, dying away.
    out["ding-coin"] = fade(speed(tick, 1.1), 0, 0.06)
    out["ding-few"] = fade(mix([(speed(clink, 1.05), 0.0), (gain(speed(tick, 0.95), 0.7), 0.09)]), 0, 0.08)
    out["ding-handful"] = fade(
        mix([(clink, 0.0), (gain(speed(clink, 1.08), 0.8), 0.07), (gain(tick, 0.7), 0.16), (gain(speed(clink, 0.94), 0.6), 0.24)]),
        0, 0.12,
    )
    out["ding-pouch"] = fade(
        mix(
            [(body, 0.0), (speed(clink, 0.92), 0.0), (gain(clink, 0.9), 0.06), (gain(tick, 0.8), 0.12),
             (gain(speed(clink, 0.88), 0.8), 0.17), (gain(speed(clink, 1.05), 0.6), 0.26), (gain(tick, 0.5), 0.33),
             (gain(speed(clink, 0.9), 0.4), 0.4)]
        ),
        0, 0.18,
    )
    # The shake: single coin hits, one contact each, at their own pitch. The phone builds every
    # turn of the hand out of these, at random (count, choice, order, spacing, pitch, loudness),
    # so no two turns are alike; the tiers differ only in those numbers.
    for i, (src, ms) in enumerate(GRAINS, 1):
        x = coins if src == "coins" else toss
        onsets = hits(x)
        t = min(onsets, key=lambda o: abs(o - ms / 1000))
        later = [o for o in onsets if o > t + 0.01]
        out[f"grain-{i}"] = grain(x, t, later[0] if later else None)
    return out


# The hits picked from the two recordings: (recording, onset in ms). Each is one coin touching
# another, about 0.04 to 0.14 s, cut at its own onset and before the next one.
GRAINS = [("coins", 54), ("coins", 154), ("coins", 270), ("coins", 484),
          ("toss", 68), ("toss", 114), ("toss", 168), ("toss", 252)]


def hits(x, frame=0.002, rise=6.0, floor=-30, gap=0.018):
    """Onsets in seconds: where the energy of the signal's slope jumps by `rise` dB."""
    h = int(frame * RATE)
    d = [x[i] - x[i - 1] for i in range(1, len(x))]
    e = [sum(v * v for v in d[i:i + h]) / h + 1e-12 for i in range(0, len(d) - h, h)]
    top = max(e)
    out = []
    for i in range(3, len(e)):
        if (10 * math.log10(e[i] / max(e[i - 3:i])) >= rise and 10 * math.log10(e[i] / top) >= floor
                and (not out or i * frame - out[-1] >= gap)):
            out.append(i * frame)
    return out


def grain(x, t, nxt, longest=0.14):
    """One hit: from 1 ms before `t` to the next onset (or `longest`), raised-cosine edges, equal
    peak (loudness is the phone's business)."""
    a = max(0, int((t - 0.001) * RATE))
    end = min(t + longest, nxt - 0.002 if nxt else t + longest)
    g = list(x[a:int(end * RATE)])
    ni, no = int(0.001 * RATE), min(len(g) // 2, int(0.025 * RATE))
    g = [v * (0.5 - 0.5 * math.cos(math.pi * i / ni)) if i < ni else v for i, v in enumerate(g)]
    for i in range(no):
        g[-1 - i] *= 0.5 - 0.5 * math.cos(math.pi * i / no)
    p = max(abs(v) for v in g) or 1.0
    return [v / p * 0.89 for v in g]


def main():
    bounds = {"ding": (0.15, 1.6), "grain": (0.02, 0.16)}
    for name, samples in build().items():
        kind = name.split("-")[0]
        if kind == "ding":  # trim, then fade, so the file ends on silence
            samples = fade(trim(normalise(samples)), 0, 0.03)
        path = os.path.join(SOUND, f"klimper-{name}.wav")
        write(path, samples)
        seconds = len(samples) / RATE
        print(f"{os.path.relpath(path):50} {seconds:5.2f} s  {os.path.getsize(path) // 1024:4} kB")
        lo, hi = bounds[kind]
        if not lo <= seconds <= hi:
            print("  duration out of bounds", file=sys.stderr)
            sys.exit(1)


if __name__ == "__main__":
    main()
