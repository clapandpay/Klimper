// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on iOS (clapandpay TASK-111). Two moments:
//
// - Coins arrive: `play` rings the tier's ding once (AVAudioPlayer).
// - The phone is shaken: every turn of the hand is a few coins landing in it, heard and felt at
//   the same instant. One Core Haptics pattern per turn holds both: each coin is a single coin
//   hit (one of eight recordings, at its own pitch and loudness) and, where the hand can tell
//   them apart, a tap. The first tap is the dull weight of the wallet, the rest are the sharp
//   clicks of coin on coin. How many coins, which, in what order, how far apart, how high and
//   how loud are drawn fresh every turn, so no two turns are alike; the tier sets the ranges. A
//   near-empty wallet is one coin.
//
// Sound follows the ambient audio session (the silent switch mutes it; the taps stay). Nothing
// runs unless the shake is armed and the app is active.

import AVFoundation
import CoreHaptics
import CoreMotion
import Tauri
import UIKit

struct PlayArgs: Decodable {
  let tier: String
  let wav: String?
}

struct FeelArgs: Decodable {
  let kind: String
}

struct ShakeArgs: Decodable {
  let enabled: Bool
  let tier: String
  let grains: [String]?
}

struct PouchArgs: Decodable {
  let channel: Channel?
  let grains: [String]?
}

struct ClinkArgs: Decodable {
  let size: Double
  let force: Double
}

class KlimperJinglePlugin: Plugin {
  // How each tier lands: coins per turn, the weight of the first tap, how dull it is (Core
  // Haptics sharpness), the pitch range of the coins and their loudness.
  private static let tiers: [String: (coins: ClosedRange<Int>, weight: Double, dull: Double,
    pitch: ClosedRange<Double>, loud: Double)] = [
      "coin": (1...1, 0.65, 0.55, 0.05...0.25, 0.6),
      "few": (2...4, 0.8, 0.45, -0.1...0.15, 0.7),
      "handful": (3...6, 0.9, 0.35, -0.15...0.1, 0.85),
      "pouch": (5...8, 1.0, 0.25, -0.2...0.05, 1.0),
    ]
  // Everyday handling (walking, picking the phone up, putting it away) stays quiet: only a
  // deliberate shake or tilt moves the coins.
  private let jolt = 1.8  // g that counts as a turn of the hand
  private let calm = 0.6  // g below which the hand has turned and the next jolt counts
  private let turn = 0.35  // seconds: at most one turn this often
  private let tiltDegrees = 30.0  // a tilt this far lets the coins slide
  private let tiltWithin = 0.7  // seconds: a turn slower than this is not a tilt
  private let tiltCalm = 0.3  // g: a tilt only counts with a steady hand
  private let slideEvery = 0.6  // seconds: at most one slide this often
  private var tiltFrom: CMAcceleration?  // gravity where the coins last settled
  private var tiltAt = 0.0  // when `tiltFrom` was taken

  private var dings: [String: AVAudioPlayer] = [:]
  private var grainFiles: [URL] = []
  private var grains: [CHHapticAudioResourceID] = []
  private var engine: CHHapticEngine?
  private var current: CHHapticPatternPlayer?
  private let motion = CMMotionManager()
  private var tier = "coin"
  private var armed = false
  private var turned = true
  private var lastTurn: TimeInterval = 0
  private var lastFirst = -1
  private var sessionReady = false
  /// The coin pouch is open (TASK-124): tilt and jolts go to the webview, whose coins knock
  /// and call `clink`, instead of the shake and tilt sounding here on their own.
  private var pouchChannel: Channel?
  private var pouchGravity: CMAcceleration?
  private var lastClink: TimeInterval = 0
  /// Knocks closer than this are one: the ear and the hand cannot tell them apart anyway.
  private let clinkEvery = 0.025

  override init() {
    super.init()
    // A shake is the coins, never iOS's "Undo Typing" alert.
    DispatchQueue.main.async { UIApplication.shared.applicationSupportsShakeToEdit = false }
    let center = NotificationCenter.default
    center.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) {
      [weak self] _ in self?.rest()
    }
    center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) {
      [weak self] _ in self?.watch()
    }
  }

  // MARK: Commands

  @objc public func play(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PlayArgs.self)
    DispatchQueue.main.async {
      self.readySession()
      if let wav = args.wav, let data = Data(base64Encoded: wav) {
        let player = try? AVAudioPlayer(data: data, fileTypeHint: AVFileType.wav.rawValue)
        player?.prepareToPlay()
        self.dings[args.tier] = player
      }
      if UIApplication.shared.applicationState == .active { self.dings[args.tier]?.play() }
      invoke.resolve()
    }
  }

  @objc public func setShake(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ShakeArgs.self)
    DispatchQueue.main.async {
      if let wavs = args.grains { self.keepGrains(wavs) }
      self.tier = args.tier
      self.armed = args.enabled
      self.watchOrRest()
      invoke.resolve()
    }
  }

  @objc public func pouch(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PouchArgs.self)
    DispatchQueue.main.async {
      if let wavs = args.grains { self.keepGrains(wavs) }
      self.pouchChannel = args.channel
      self.pouchGravity = nil
      // Restart the stream so it picks up (or drops) the pouch.
      self.motion.stopDeviceMotionUpdates()
      self.watchOrRest()
      invoke.resolve()
    }
  }

  /// A knock in the pouch: one coin hit and its tap, from the same pieces as a shake.
  @objc public func clink(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ClinkArgs.self)
    invoke.resolve()
    DispatchQueue.main.async {
      let now = CACurrentMediaTime()
      guard self.pouchChannel != nil, !self.grains.isEmpty, let t = Self.tiers[self.tier],
        now - self.lastClink >= self.clinkEvery
      else { return }
      self.lastClink = now
      // A big coin sounds lower and feels duller than a small one.
      let pitch = t.pitch.upperBound - args.size * (t.pitch.upperBound - t.pitch.lowerBound)
      let events = self.coin(
        self.grains.indices.randomElement() ?? 0, at: 0,
        volume: t.loud * (0.3 + 0.7 * args.force), pitch: pitch,
        tap: t.weight * (0.45 + 0.55 * args.force), sharpness: 0.75 - 0.45 * args.size)
      self.perform(events, cut: false)
    }
  }

  private func keepGrains(_ wavs: [String]) {
    let dir = FileManager.default.temporaryDirectory
    grainFiles = wavs.enumerated().compactMap { i, wav in
      let url = dir.appendingPathComponent("klimper-grain-\(i + 1).wav")
      guard let data = Data(base64Encoded: wav), (try? data.write(to: url)) != nil else { return nil }
      return url
    }
    register()
  }

  private func watchOrRest() {
    if armed || pouchChannel != nil { watch() } else { rest() }
  }

  /// A control's system haptic (the slide to approve): grab, a tick per mark, done. The
  /// generators follow the phone's own haptics setting.
  private let selection = UISelectionFeedbackGenerator()
  private let impact = UIImpactFeedbackGenerator(style: .light)
  private let notice = UINotificationFeedbackGenerator()

  @objc public func feel(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(FeelArgs.self)
    invoke.resolve()  // nothing to wait for: free the bridge before the main thread
    DispatchQueue.main.async {
      switch args.kind {
      case "grab":
        self.impact.impactOccurred(intensity: 0.7)
        self.selection.prepare()
        self.notice.prepare()  // the success at the end, ready on a warm Taptic Engine
      case "tick":
        self.selection.selectionChanged()
        self.selection.prepare()
      case "done": self.notice.notificationOccurred(.success)
      default: break
      }
    }
  }

  // MARK: Arming

  private func readySession() {
    guard !sessionReady else { return }
    // Ambient: muted by the silent switch, mixes with music. Set only once the feature is used.
    let session = AVAudioSession.sharedInstance()
    do {
      try session.setCategory(.ambient)
      try session.setActive(true)
      sessionReady = true
    } catch {}
  }

  private func register() {
    guard let engine else { return }
    for id in grains { try? engine.unregisterAudioResource(id) }
    // No volume envelope: a coin hit keeps its edge (iOS 15; the app's minimum anyway).
    var options: [AnyHashable: Any] = [:]
    if #available(iOS 15.0, *) { options[CHHapticAudioResourceKeyUseVolumeEnvelope] = false }
    grains = grainFiles.compactMap { try? engine.registerAudioResource($0, options: options) }
  }

  private func watch() {
    let can = CHHapticEngine.capabilitiesForHardware()
    guard armed || pouchChannel != nil, !grainFiles.isEmpty,
      UIApplication.shared.applicationState == .active,
      can.supportsHaptics || can.supportsAudio,
      motion.isDeviceMotionAvailable, !motion.isDeviceMotionActive
    else { return }
    readySession()
    if engine == nil, let made = try? CHHapticEngine(audioSession: .sharedInstance()) {
      // After a reset the sounds must be registered again; restart only while wanted.
      made.resetHandler = { [weak self] in
        DispatchQueue.main.async {
          self?.register()
          self?.restartIfWanted()
        }
      }
      // Stopped by the system (a call, Siri, another app's audio) while the app stays active.
      made.stoppedHandler = { [weak self] _ in
        DispatchQueue.main.async { self?.restartIfWanted() }
      }
      engine = made
      register()
    }
    engine?.start { _ in }
    turned = true
    motion.deviceMotionUpdateInterval = 1.0 / 40.0
    tiltFrom = nil
    motion.startDeviceMotionUpdates(to: .main) { [weak self] data, _ in
      guard let self, let data else { return }
      let a = data.userAcceleration
      let g = (a.x * a.x + a.y * a.y + a.z * a.z).squareRoot()
      if g < self.calm { self.turned = true }
      let now = CACurrentMediaTime()
      if let pouch = self.pouchChannel {
        self.feed(pouch, data, g: g, now: now)
        return
      }
      // A slow tilt (the hand is calm): the coins slide to the low side, softly.
      if g < self.tiltCalm, now - self.lastTurn >= self.slideEvery {
        let gravity = data.gravity
        // Only a turn within `tiltWithin` counts: a phone slowly drifting, or set down, stays quiet.
        if now - self.tiltAt > self.tiltWithin {
          self.tiltFrom = gravity
          self.tiltAt = now
        } else if let from = self.tiltFrom {
          let dot = from.x * gravity.x + from.y * gravity.y + from.z * gravity.z
          let angle = acos(max(-1, min(1, dot))) * 180 / .pi
          if angle >= self.tiltDegrees {
            self.tiltFrom = gravity
            self.tiltAt = now
            self.lastTurn = now
            self.slide(by: min(1, angle / 60))
          }
        }
      }
      // Once per reversal of the hand: above `jolt` after having come back below `calm`.
      guard self.turned, g >= self.jolt, now - self.lastTurn >= self.turn else { return }
      self.turned = false
      self.lastTurn = now
      self.turnOfTheHand(force: min(1, (g - self.jolt) / 2.5))
    }
  }

  /// The pouch is open: its coins move with the phone. Gravity goes over only when it turned
  /// by a degree or so (a resting phone sends nothing); a jolt of the hand goes over as a push.
  private func feed(_ pouch: Channel, _ data: CMDeviceMotion, g: Double, now: TimeInterval) {
    let gravity = data.gravity
    let turnedEnough = pouchGravity.map { from in
      from.x * gravity.x + from.y * gravity.y + from.z * gravity.z < cos(1.5 * .pi / 180)
    } ?? true
    if turnedEnough {
      pouchGravity = gravity
      pouch.send(["gx": gravity.x, "gy": gravity.y] as JsonObject)
    }
    guard turned, g >= jolt, now - lastTurn >= turn else { return }
    turned = false
    lastTurn = now
    let a = data.userAcceleration
    pouch.send(["jx": a.x, "jy": a.y] as JsonObject)
  }

  private func restartIfWanted() {
    guard armed || pouchChannel != nil, UIApplication.shared.applicationState == .active else { return }
    engine?.start { _ in }
  }

  private func rest() {
    motion.stopDeviceMotionUpdates()
    try? current?.stop(atTime: CHHapticTimeImmediate)
    engine?.stop(completionHandler: nil)
  }

  // MARK: A turn of the hand

  /// The phone was tilted: the coins roll to the low side and knock into each other. A low
  /// rumble while they roll, then two to four firm taps; softer than a shake.
  private func slide(by amount: Double) {
    guard !grains.isEmpty, let t = Self.tiers[tier] else { return }
    let order = Array(grains.indices).shuffled()
    let coins = min(order.count, max(2, min(4, Int.random(in: t.coins) + 1)))
    var events: [CHHapticEvent] = []
    var at = 0.04  // the coins roll a moment before the first lands
    for k in 0..<coins {
      if k > 0 { at += .random(in: 0.04...0.08) }
      events += coin(
        order[k], at: at, volume: t.loud * (0.35 + 0.25 * amount) * pow(0.8, Double(k)),
        pitch: .random(in: t.pitch), tap: t.weight * (0.65 + 0.3 * amount) * pow(0.88, Double(k)),
        sharpness: .random(in: 0.4...0.6))
    }
    // The roll under the taps: dull and low, for as long as the coins move.
    events.append(CHHapticEvent(eventType: .hapticContinuous, parameters: [
      CHHapticEventParameter(parameterID: .hapticIntensity, value: Float(0.3 + 0.2 * amount)),
      CHHapticEventParameter(parameterID: .hapticSharpness, value: 0.1),
    ], relativeTime: 0, duration: at + 0.06))
    perform(events, cut: true)
  }

  private func turnOfTheHand(force: Double) {
    guard !grains.isEmpty, let t = Self.tiers[tier] else { return }
    var order = Array(grains.indices).shuffled()
    if order.count > 1, order[0] == lastFirst { order.swapAt(0, 1) }  // never the same opener
    lastFirst = order[0]
    var coins = min(Int.random(in: t.coins), order.count)
    let base = max(0.6, t.loud * (0.6 + 0.4 * force))
    var events: [CHHapticEvent] = []
    var at = 0.0
    var lastTap = -1.0
    var taps = 0
    // A lone coin sometimes has a faint neighbour: the wallet is not quite empty.
    if coins == 1, order.count > 1, Double.random(in: 0...1) < 0.3 { coins = 2 }
    for k in 0..<coins {
      if k > 0 {
        // Coins settle in a cluster: gaps mostly short, now and then longer.
        let gap = -log(Double.random(in: 0.05...1)) * 0.022 * (0.8 + 0.6 * force)
        at += min(0.06, max(0.008, gap))
      }
      let volume = k == 0 ? base : base * .random(in: 0.2...0.7) * pow(0.85, Double(k))
      events += coin(order[k], at: at, volume: volume, pitch: .random(in: t.pitch))
      // The hand feels separate coins only some 45 ms apart; at most four.
      guard at - lastTap >= 0.045, taps < 4 else { continue }
      let intensity = k == 0
        ? max(0.85, t.weight * (0.9 + 0.2 * force))
        : max(0.5, t.weight * pow(0.85, Double(taps)) * .random(in: 0.92...1.12))
      lastTap = at
      taps += 1
      let sharpness = k == 0 ? t.dull + .random(in: -0.05...0.05) : .random(in: 0.6...0.85)
      events.append(Self.tap(at, intensity, sharpness))
    }
    // Now and then one coin slides off a moment later.
    if Double.random(in: 0...1) < 0.25 { events.append(Self.tap(at + .random(in: 0.06...0.1), 0.3, 0.9)) }

    perform(events, cut: true)  // a new turn cuts the last one's settle
  }

  /// One coin: its hit (one of the eight recordings, at a volume and pitch) and, if `tap` is
  /// given, the tap the hand feels at the same instant. The shake, the tilt and the pouch's
  /// knocks are all made of these.
  private func coin(
    _ grain: Int, at: Double, volume: Double, pitch: Double,
    tap: Double? = nil, sharpness: Double = 0.6
  ) -> [CHHapticEvent] {
    var events = [CHHapticEvent(audioResourceID: grains[grain], parameters: [
      CHHapticEventParameter(parameterID: .audioVolume, value: Float(min(1, volume))),
      CHHapticEventParameter(parameterID: .audioPitch, value: Float(pitch)),
    ], relativeTime: at)]
    if let tap { events.append(Self.tap(at, tap, sharpness)) }
    return events
  }

  /// Play a pattern now. `cut`: it replaces the one still settling (a shake, a tilt); a knock in
  /// the pouch plays alongside the others. Not running (stopped and not yet back): skipped, the
  /// stopped handler restarts the engine.
  private func perform(_ events: [CHHapticEvent], cut: Bool) {
    guard let engine, let pattern = try? CHHapticPattern(events: events, parameters: []) else { return }
    if cut {
      try? current?.stop(atTime: CHHapticTimeImmediate)
      current = try? engine.makePlayer(with: pattern)
      try? current?.start(atTime: CHHapticTimeImmediate)
    } else {
      try? engine.makePlayer(with: pattern).start(atTime: CHHapticTimeImmediate)
    }
  }

  private static func tap(_ at: Double, _ intensity: Double, _ sharpness: Double) -> CHHapticEvent {
    CHHapticEvent(eventType: .hapticTransient, parameters: [
      CHHapticEventParameter(parameterID: .hapticIntensity, value: Float(min(1, intensity))),
      CHHapticEventParameter(parameterID: .hapticSharpness, value: Float(min(1, max(0, sharpness)))),
    ], relativeTime: at)
  }
}

@_cdecl("init_plugin_klimper_jingle")
func initPlugin() -> Plugin {
  return KlimperJinglePlugin()
}
