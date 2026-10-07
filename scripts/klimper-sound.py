#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# The Klimper sound (clapandpay TASK-111.01, decision-009): real coins, cut from CC0 recordings
# (src/klimper/sound/sources/, see SOURCES.md there) into the files the app plays. Pure Python,
# no dependencies, deterministic: `python3 scripts/klimper-sound.py` always writes byte-identical
# files into src/klimper/sound/:
#
#   klimper-ding-<tier>.wav          coins arrive: a small rattle, longer and fuller per tier
#   klimper-rattle-<tier>-{a,b}.wav  the phone is shaken: coins rattling, a seamless loop; two
#                                    variants, the phone picks one at random each shake
#   klimper-knock-<tier>.wav         one jolt: a knock on top of the loop, at a random pitch
#
# Tiers (USD bounds in src/klimper/sound/tiers.ts): coin, few, handful, pouch. A tier's weight is
# made of layers (more coins), speed (heavy coins move slower and sound lower) and, for the
# pouch, a low-passed body underneath. Listening notes in docs/sound.md.

import math
import os
import struct
import sys
import wave

RATE = 44_100
PEAK = 10 ** (-1 / 20)  # -1 dBFS
LOOP_SECONDS = 0.84
CROSSFADE = 0.06

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


def wrap_mix(layers, length):
    """Like `mix`, but what runs past the end wraps to the start: for loops."""
    n = int(length * RATE)
    out = [0.0] * n
    for s, off in layers:
        start = int(off * RATE)
        for i, v in enumerate(s):
            out[(start + i) % n] += v
    return out


def loop(x):
    """Seamless: overlap the last 60 ms onto the first 60 ms and drop the tail."""
    n = len(x)
    f = int(CROSSFADE * RATE)
    out = list(x)
    for i in range(f):
        w = i / f
        out[i] = x[i] * w + x[n - f + i] * (1 - w)
    return out[: n - f]


def normalise(x):
    peak = max(abs(v) for v in x) or 1.0
    return [max(-1.0, min(1.0, v * PEAK / peak)) for v in x]


def trim(x, floor=0.006, air=0.03):
    last = max(i for i, v in enumerate(x) if abs(v) > floor)
    return x[: min(len(x), last + int(air * RATE))]


# ---- the sounds ----------------------------------------------------------------------------
#
# The recordings are single gestures: a clink of 0.1 to 0.25 s and silence. So they are used as
# grains: many copies spread through a loop at seeded offsets, speeds and gains, which is what
# a hand full of coins is anyway. Seeds are fixed, so the files are reproducible.

import random


def scatter(grains, count, length, seed, speeds=(0.9, 1.12), gains=(0.5, 1.0)):
    """`count` grains across `length` seconds, wrapping, each at its own speed and gain."""
    rng = random.Random(seed)
    layers = []
    for _ in range(count):
        g = rng.choice(grains)
        layers.append(
            (gain(speed(g, rng.uniform(*speeds)), rng.uniform(*gains)), rng.uniform(0, length))
        )
    return wrap_mix(layers, length)


def build():
    coins = load("kenney-handle-coins")  # coins moved in a hand: the clink is at 0.10 to 0.45 s
    toss = load("kenney-handle-coins-2")  # a smaller handful: the clink is at 0.15 to 0.34 s
    light0 = load("kenney-metal-light-0")
    light2 = load("kenney-metal-light-2")
    medium = load("kenney-metal-medium-1")
    heavy = load("kenney-metal-heavy-0")
    body = gain(lowpass(heavy, 180), 1.6)  # the weight of the pouch, felt more than heard

    clink = fade(cut(coins, 0.10, 0.40), 0.004, 0.05)
    tick = fade(cut(toss, 0.15, 0.19), 0.003, 0.05)
    grains = [clink, tick]

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
            [(body, 0.0), (speed(clink, 0.92), 0.0), (gain(speed(clink, 1.0), 0.9), 0.06), (gain(tick, 0.8), 0.12),
             (gain(speed(clink, 0.88), 0.8), 0.17), (gain(speed(clink, 1.05), 0.6), 0.26), (gain(tick, 0.5), 0.33),
             (gain(speed(clink, 0.9), 0.4), 0.4)]
        ),
        0, 0.18,
    )

    # Shaken: loops. The count of grains is the weight; speed sets the size of the coins.
    out["rattle-coin-a"] = scatter([tick, speed(clink, 1.15)], 3, LOOP_SECONDS, 11, (1.0, 1.2), (0.6, 1.0))
    out["rattle-coin-b"] = scatter([tick, speed(clink, 1.15)], 3, LOOP_SECONDS, 12, (1.0, 1.2), (0.6, 1.0))
    out["rattle-few-a"] = scatter(grains, 8, LOOP_SECONDS, 21, (0.95, 1.12))
    out["rattle-few-b"] = scatter(grains, 8, LOOP_SECONDS, 22, (0.95, 1.12))
    out["rattle-handful-a"] = scatter(grains, 16, LOOP_SECONDS, 31, (0.9, 1.08))
    out["rattle-handful-b"] = scatter(grains, 16, LOOP_SECONDS, 32, (0.9, 1.08))
    for v, seed in (("a", 41), ("b", 42)):
        rattle = scatter(grains, 28, LOOP_SECONDS, seed, (0.82, 1.02))
        rumble = scatter([body], 5, LOOP_SECONDS, seed + 100, (0.8, 1.1), (0.3, 0.5))
        out[f"rattle-pouch-{v}"] = mix([(rattle, 0.0), (rumble, 0.0)], LOOP_SECONDS)
    for k in list(out):
        if k.startswith("rattle-"):
            out[k] = loop(out[k])

    # One jolt: a knock. Dull and single for a coin, hard and heavy for a pouch.
    out["knock-coin"] = fade(cut(light2, 0.0, 0.12), 0, 0.03)
    out["knock-few"] = fade(mix([(tick, 0.0), (gain(speed(tick, 0.9), 0.6), 0.015)]), 0, 0.03)
    out["knock-handful"] = fade(mix([(cut(light0, 0.0, 0.14), 0.0), (gain(cut(clink, 0.0, 0.14), 0.7), 0.01)]), 0, 0.04)
    out["knock-pouch"] = fade(mix([(medium, 0.0), (gain(body, 0.6), 0.0), (gain(cut(speed(clink, 0.9), 0.0, 0.16), 0.7), 0.02)]), 0, 0.04)

    return out


def main():
    bounds = {"ding": (0.15, 1.6), "rattle": (0.3, 1.6), "knock": (0.03, 0.3)}
    for name, samples in build().items():
        kind = name.split("-")[0]
        samples = normalise(samples)
        if kind != "rattle":
            samples = trim(samples)
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
