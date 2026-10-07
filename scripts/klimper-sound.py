#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# The Klimper sound (clapandpay TASK-111.01, decision-009): coins colliding, synthesised, in
# four tiers by the value of the coins. Pure Python, no dependencies, deterministic from a fixed
# seed, so `python3 scripts/klimper-sound.py` always writes byte-identical files:
#
#   src/klimper/sound/klimper-coin.wav      one coin
#   src/klimper/sound/klimper-few.wav       a few coins
#   src/klimper/sound/klimper-handful.wav   a handful
#   src/klimper/sound/klimper-pouch.wav     a heavy pouch (a thud, then many coins settling)
#
# How a coin sounds: a struck disc rings with a handful of partials at non-integer ratios (a
# circular plate: 1, 1.59, 2.14, 2.30, 2.65, 2.92 ...), the high ones dying first, behind a
# 1 to 2 ms burst of noise where the metal meets. Two coins touching are two such strikes a few
# milliseconds apart at different pitches. A jingle is many strikes whose density falls off as
# the coins come to rest. The Klimper signature is the tail: every tier ends on the same three
# strikes (low, high, middle: "klim-per"), so the four files are one sound at four weights.
#
# Listening notes and the tier table (USD bounds) are in docs/sound.md; the table the app uses is
# src/klimper/sound/tiers.ts.

import math
import os
import random
import struct
import sys
import wave

RATE = 44_100
PEAK = 10 ** (-1 / 20)  # -1 dBFS

# Partial ratios of a struck disc, and how much slower each lower partial decays.
PARTIALS = [(1.0, 1.0), (1.59, 0.62), (2.14, 0.44), (2.30, 0.36), (2.65, 0.28), (2.92, 0.22)]

# The closing motif: offsets in seconds and pitch ratios against the tier's base.
SIGNATURE = [(0.0, 1.0), (0.072, 1.26), (0.150, 1.12)]

# name -> (seed, base pitch Hz, pitch spread, strikes, scatter seconds, decay seconds, thud)
TIERS = {
    "coin": dict(seed=11, base=5200.0, spread=0.10, strikes=2, scatter=0.0, decay=0.11, thud=0.0),
    "few": dict(seed=23, base=4700.0, spread=0.18, strikes=6, scatter=0.32, decay=0.10, thud=0.0),
    "handful": dict(seed=37, base=4000.0, spread=0.26, strikes=13, scatter=0.62, decay=0.09, thud=0.0),
    "pouch": dict(seed=53, base=3300.0, spread=0.32, strikes=24, scatter=0.9, decay=0.085, thud=1.0),
}


def strike(buf, at, freq, amp, decay, rng):
    """One coin hit: inharmonic partials with an exponential decay and a short noise edge."""
    start = int(at * RATE)
    length = int(decay * 6 * RATE)
    phases = [rng.random() * math.tau for _ in PARTIALS]
    for i in range(length):
        if start + i >= len(buf):
            break
        t = i / RATE
        s = 0.0
        for (ratio, weight), ph in zip(PARTIALS, phases):
            tau = decay * (0.35 + 0.65 * weight)
            s += weight * math.exp(-t / tau) * math.sin(math.tau * freq * ratio * t + ph)
        if i < int(0.0018 * RATE):  # the edge of the impact
            s += (rng.random() * 2 - 1) * 0.9 * (1 - i / (0.0018 * RATE))
        buf[start + i] += amp * s * min(1.0, i / (0.0006 * RATE))  # 0.6 ms attack


def thud(buf, at, amp, rng):
    """The mass of a pouch landing: a pitch drop with a little filtered noise."""
    start = int(at * RATE)
    length = int(0.16 * RATE)
    noise = 0.0
    for i in range(length):
        if start + i >= len(buf):
            break
        t = i / RATE
        f = 95.0 * math.exp(-t * 9) + 48.0
        noise = noise * 0.93 + (rng.random() * 2 - 1) * 0.07  # one-pole lowpass
        env = math.exp(-t / 0.055)
        buf[start + i] += amp * env * (math.sin(math.tau * f * t) * 0.9 + noise * 2.2)


def render(name, spec):
    rng = random.Random(spec["seed"])
    base, spread = spec["base"], spec["spread"]
    scatter, decay = spec["scatter"], spec["decay"]
    tail_at = 0.05 + scatter
    total = tail_at + SIGNATURE[-1][0] + decay * 6 + 0.05
    buf = [0.0] * int(total * RATE)

    if spec["thud"]:
        thud(buf, 0.0, 0.75 * spec["thud"], rng)

    # The coins: more of them early, thinning out as they settle (scatter squared).
    for k in range(spec["strikes"]):
        at = 0.02 + scatter * (rng.random() ** 1.7)
        pitch = base * (1 + (rng.random() * 2 - 1) * spread)
        amp = 0.35 + 0.45 * rng.random()
        amp *= 1 - 0.5 * (at / max(scatter, 0.01)) if scatter else 1  # quieter as they rest
        strike(buf, at, pitch, amp, decay * (0.8 + 0.4 * rng.random()), rng)
        if rng.random() < 0.5:  # the other coin of the touch
            strike(buf, at + 0.002 + rng.random() * 0.006, pitch * (1.05 + rng.random() * 0.2),
                   amp * 0.6, decay * 0.9, rng)

    # The signature: low, high, middle, clean and a touch louder than the rest.
    for offset, ratio in SIGNATURE:
        strike(buf, tail_at + offset, base * 0.92 * ratio, 0.85, decay * 1.1, rng)

    peak = max(abs(x) for x in buf) or 1.0
    scale = PEAK / peak
    out = [max(-1.0, min(1.0, x * scale)) for x in buf]
    # Cut the tail after the last ring: everything past -48 dBFS (a real coin stops ringing within a
    # few hundred milliseconds), plus 25 ms of air.
    last = max(i for i, x in enumerate(out) if abs(x) > 0.004)
    return out[: min(len(out), last + int(0.025 * RATE))]


def write(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(round(s * 32767))) for s in samples))


def main():
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src", "klimper", "sound")
    os.makedirs(out, exist_ok=True)
    for name, spec in TIERS.items():
        samples = render(name, spec)
        path = os.path.join(out, f"klimper-{name}.wav")
        write(path, samples)
        seconds = len(samples) / RATE
        print(f"{os.path.relpath(path):45} {seconds:5.2f} s  {os.path.getsize(path) // 1024:4} kB")
        if not 0.4 <= seconds <= 1.6:
            print("  duration out of bounds", file=sys.stderr)
            sys.exit(1)


if __name__ == "__main__":
    main()
