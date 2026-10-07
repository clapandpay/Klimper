#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# The Klimper sound (clapandpay TASK-111.01, decision-009), synthesised, deterministic from fixed
# seeds, pure Python, no dependencies. `python3 scripts/klimper-sound.py` always writes
# byte-identical files into src/klimper/sound/:
#
#   klimper-ding-<tier>.wav     coins arrive: one bright coin "ding", richer per tier (one shot)
#   klimper-rattle-<tier>.wav   the phone is shaken: coins rattling, heavier per tier (a loop)
#
# Two sounds, two moments. The ding is the old "money arrived" coin sound of the first mobile
# wallets, made ours: a struck coin with a long bright ring and a second, higher coin a moment
# later (the "klim-per" rise, the same two pitch ratios as the rattle's signature), plus a
# small cascade for the bigger tiers. The rattle is what the name means: coins in a hand being
# shaken. Not bells: a coin in a pile is damped by its neighbours, so each knock is a short
# inharmonic tick (disc partials 1, 1.59, 2.14, 2.30, 2.65, 2.92, the high ones gone first)
# with a noisy edge where metal meets metal, and between the knocks coins slide on coins, a
# short high hiss. The rattle plays as long as the shaking lasts, so it is a seamless loop:
# knocks are spread evenly and the last 60 ms are crossfaded into the first.
#
# Tiers (USD bounds, in src/klimper/sound/tiers.ts): coin, few, handful, pouch. Listening
# notes in docs/sound.md.

import math
import os
import random
import struct
import sys
import wave

RATE = 44_100
PEAK = 10 ** (-1 / 20)  # -1 dBFS
LOOP_SECONDS = 0.9
CROSSFADE = 0.06

# Partial ratios of a struck disc, with weight and relative decay.
DISC = [(1.0, 1.0), (1.59, 0.7), (2.14, 0.5), (2.30, 0.42), (2.65, 0.3), (2.92, 0.24)]
# A coin that rings (the ding): fewer, cleaner partials, the fundamental strong.
RING = [(1.0, 1.0), (2.0, 0.35), (2.76, 0.22), (3.4, 0.12), (5.2, 0.06)]

# The two pitch ratios of "klim-per": the ding's second coin, the rattle's signature.
RISE = 1.26
MID = 1.12

DINGS = {
    # base pitch Hz, how many coins follow the first (cascade), ring decay s, a thud under it
    "coin": dict(seed=101, base=1760.0, cascade=0, decay=0.2, thud=0.0),
    "few": dict(seed=103, base=1660.0, cascade=1, decay=0.22, thud=0.0),
    "handful": dict(seed=107, base=1560.0, cascade=2, decay=0.24, thud=0.15),
    "pouch": dict(seed=109, base=1480.0, cascade=4, decay=0.26, thud=0.6),
}

RATTLES = {
    # base pitch Hz, pitch spread, knocks per loop, hiss grains per loop, knock decay s, rumble
    "coin": dict(seed=201, base=5600.0, spread=0.10, knocks=5, grains=3, decay=0.06, rumble=0.0),
    "few": dict(seed=203, base=4900.0, spread=0.25, knocks=14, grains=12, decay=0.05, rumble=0.0),
    "handful": dict(seed=207, base=4200.0, spread=0.32, knocks=30, grains=30, decay=0.045, rumble=0.12),
    "pouch": dict(seed=209, base=3400.0, spread=0.38, knocks=60, grains=70, decay=0.04, rumble=0.3),
}


def strike(buf, at, freq, amp, decay, partials, rng, edge=0.002, edge_amp=1.2, bend=0.06):
    """A struck coin: partials with exponential decays, a noisy edge, a pitch bend at contact."""
    start = int(at * RATE)
    length = int(decay * 5 * RATE)
    phases = [rng.random() * math.tau for _ in partials]
    edge_n = int(edge * RATE)
    for i in range(length):
        j = (start + i) % len(buf)  # wraps: a loop's tail lands on its head
        t = i / RATE
        b = 1 + bend * math.exp(-t / 0.004)
        s = 0.0
        for (ratio, weight), ph in zip(partials, phases):
            tau = decay * (0.3 + 0.7 * weight)
            s += weight * math.exp(-t / tau) * math.sin(math.tau * freq * ratio * b * t + ph)
        if i < edge_n:
            s += (rng.random() * 2 - 1) * edge_amp * (1 - i / edge_n)
        buf[j] += amp * s * min(1.0, i / (0.0004 * RATE))


def grain(buf, at, amp, rng):
    """Coins sliding on coins: a few milliseconds of high hiss."""
    start = int(at * RATE)
    length = int((0.003 + rng.random() * 0.009) * RATE)
    centre = 6000.0 + rng.random() * 4000.0
    noise = 0.0
    for i in range(length):
        j = (start + i) % len(buf)
        t = i / RATE
        noise = noise * 0.55 + (rng.random() * 2 - 1) * 0.45
        env = math.sin(math.pi * i / length)
        buf[j] += amp * env * noise * math.sin(math.tau * centre * t)


def thud(buf, at, amp, rng, f0=110.0, f1=52.0, decay=0.045):
    """Mass landing: a pitch drop with a little filtered noise."""
    start = int(at * RATE)
    length = int(decay * 3.2 * RATE)
    noise = 0.0
    for i in range(length):
        if start + i >= len(buf):
            break
        t = i / RATE
        f = (f0 - f1) * math.exp(-t * 11) + f1
        noise = noise * 0.94 + (rng.random() * 2 - 1) * 0.06
        buf[start + i] += amp * math.exp(-t / decay) * (math.sin(math.tau * f * t) * 0.9 + noise * 2.0)


def normalise(buf):
    peak = max(abs(x) for x in buf) or 1.0
    scale = PEAK / peak
    return [max(-1.0, min(1.0, x * scale)) for x in buf]


def render_ding(spec):
    rng = random.Random(spec["seed"])
    base, decay = spec["base"], spec["decay"]
    total = 0.12 * (spec["cascade"] + 1) + decay * 5 + 0.1
    buf = [0.0] * int(total * RATE)
    if spec["thud"]:
        thud(buf, 0.0, spec["thud"], rng, f0=140.0, f1=70.0, decay=0.06)
    # The first coin, and its "per": a second, higher coin 90 ms later, a little softer.
    strike(buf, 0.0, base, 1.0, decay, RING, rng, edge=0.0015, edge_amp=0.8, bend=0.03)
    strike(buf, 0.09, base * RISE, 0.75, decay * 0.9, RING, rng, edge=0.0012, edge_amp=0.6, bend=0.03)
    # Bigger tiers: more coins follow, each a little higher and softer, slightly uneven.
    for k in range(spec["cascade"]):
        at = 0.19 + 0.085 * k + (rng.random() - 0.5) * 0.02
        ratio = MID * (1.06 ** k)
        strike(buf, at, base * ratio, 0.55 * (0.88 ** k), decay * 0.8, RING, rng,
               edge=0.0012, edge_amp=0.6, bend=0.03)
        # and a few small damped knocks underneath: the coins touching
        for _ in range(2):
            strike(buf, at + rng.random() * 0.05, base * 3 * math.exp((rng.random() - 0.5) * 0.4),
                   0.18, 0.04, DISC, rng)
    out = normalise(buf)
    # Cut the ring once it is below -44 dBFS, plus 30 ms of air.
    last = max(i for i, x in enumerate(out) if abs(x) > 0.006)
    return out[: min(len(out), last + int(0.03 * RATE))]


def render_rattle(spec):
    rng = random.Random(spec["seed"])
    n = int(LOOP_SECONDS * RATE)
    buf = [0.0] * n
    base, spread = spec["base"], spec["spread"]

    def pitch():
        return base * math.exp((rng.random() * 2 - 1) * spread)

    for _ in range(spec["knocks"]):
        at = rng.random() * LOOP_SECONDS
        amp = 0.3 + 0.6 * rng.random()
        decay = spec["decay"] * (0.6 + 0.8 * rng.random())
        strike(buf, at, pitch(), amp, decay, DISC, rng)
        if rng.random() < 0.6:  # the other coin of the touch
            strike(buf, at + 0.0015 + rng.random() * 0.004, pitch() * 1.08, amp * 0.55,
                   decay * 0.8, DISC, rng)
    for _ in range(spec["grains"]):
        grain(buf, rng.random() * LOOP_SECONDS, 0.08 + 0.14 * rng.random(), rng)
    # The signature, once per loop, cleaner and a little lower: low, high, middle.
    sig_at = 0.5
    for offset, ratio in ((0.0, 1.0), (0.068, RISE), (0.142, MID)):
        strike(buf, sig_at + offset, base * 0.92 * ratio, 0.5, spec["decay"] * 1.3, DISC, rng,
               edge=0.0012, edge_amp=0.6)
    if spec["rumble"]:
        # The mass moving: slow filtered noise under everything, breathing with the shake.
        noise = 0.0
        for i in range(n):
            noise = noise * 0.985 + (rng.random() * 2 - 1) * 0.015
            swell = 0.7 + 0.3 * math.sin(math.tau * i / n)  # one cycle per loop: seamless
            buf[i] += spec["rumble"] * 6.0 * noise * swell
    # Seamless: the strikes already wrapped; now overlap the last 60 ms onto the first 60 ms
    # (tail fading out, head fading in) and drop the tail, so the loop's last sample runs
    # straight into its first.
    fade = int(CROSSFADE * RATE)
    for i in range(fade):
        w = i / fade
        buf[i] = buf[i] * w + buf[n - fade + i] * (1 - w)
    return normalise(buf[: n - fade])


def write(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(round(s * 32767))) for s in samples))


def main():
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src", "klimper", "sound")
    os.makedirs(out, exist_ok=True)
    for kind, table, render in (("ding", DINGS, render_ding), ("rattle", RATTLES, render_rattle)):
        for name, spec in table.items():
            samples = render(spec)
            path = os.path.join(out, f"klimper-{kind}-{name}.wav")
            write(path, samples)
            seconds = len(samples) / RATE
            print(f"{os.path.relpath(path):50} {seconds:5.2f} s  {os.path.getsize(path) // 1024:4} kB")
            if not 0.3 <= seconds <= 1.6:
                print("  duration out of bounds", file=sys.stderr)
                sys.exit(1)
    for stale in ("coin", "few", "handful", "pouch"):
        path = os.path.join(out, f"klimper-{stale}.wav")
        if os.path.exists(path):
            os.remove(path)


if __name__ == "__main__":
    main()
