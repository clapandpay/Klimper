// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on iOS (clapandpay TASK-111.02). Two moments:
//
// - Coins arrive: `play` rings the tier's ding once (AVAudioPlayer under the ambient session,
//   so the silent switch mutes it and music keeps playing). No haptics: nobody is holding the
//   phone for it.
// - The phone is shaken: `setShake` arms the accelerometer. A shake starts the tier's rattle
//   loop and keeps it going for as long as jolts keep coming; every jolt is a haptic tap whose
//   strength follows the jolt and the tier; a few hundred milliseconds of stillness stops the
//   loop. The webview sends the wallet's tier up front, so a shake answers at once.
//
// Nothing plays or vibrates unless the app is active. Motion state lives on the motion queue;
// players and haptics on main.

import AVFoundation
import CoreMotion
import Tauri
import UIKit

struct PlayArgs: Decodable {
  let tier: String
  let wav: String?
}

struct ShakeArgs: Decodable {
  let enabled: Bool
  let tier: String
  let wav: String?
}

class KlimperJinglePlugin: Plugin {
  // Main queue.
  private var dings: [String: AVAudioPlayer] = [:]
  private var rattles: [String: AVAudioPlayer] = [:]
  private var sessionReady = false
  private var shakeTier = "coin"
  private var shakeEnabled = false
  private var rattling = false
  private var stopTimer: Timer?
  private var lastTap: TimeInterval = 0
  private let taps: [UIImpactFeedbackGenerator.FeedbackStyle: UIImpactFeedbackGenerator] = [
    .rigid: UIImpactFeedbackGenerator(style: .rigid),
    .medium: UIImpactFeedbackGenerator(style: .medium),
    .heavy: UIImpactFeedbackGenerator(style: .heavy),
  ]
  private var observers: [NSObjectProtocol] = []

  // Motion queue.
  private let motion = CMMotionManager()
  private let motionQueue = OperationQueue()
  private var jolts: [TimeInterval] = []

  // A jolt is user acceleration (gravity removed) above this, in g. Two jolts within the window
  // start the rattle; it stops after `still` seconds without one.
  private let jolt: Double = 1.3
  private let window: TimeInterval = 0.45
  private let still: TimeInterval = 0.35
  private let tapSpacing: TimeInterval = 0.07

  override init() {
    super.init()
    motionQueue.name = "klimper.jingle.motion"
    motionQueue.maxConcurrentOperationCount = 1
    let center = NotificationCenter.default
    observers.append(
      center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) {
        [weak self] _ in self?.startMotionIfWanted()
      })
    observers.append(
      center.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) {
        [weak self] _ in
        self?.stopMotion()
        self?.stopRattle()
      })
  }

  deinit {
    observers.forEach(NotificationCenter.default.removeObserver)
  }

  // MARK: Commands

  @objc public func play(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PlayArgs.self)
    DispatchQueue.main.async {
      do {
        if let wav = args.wav { self.dings[args.tier] = try self.player(wav, loops: 0) }
        guard UIApplication.shared.applicationState == .active else {
          invoke.resolve()  // background: a wallet in a pocket makes no sound on its own
          return
        }
        guard let player = self.dings[args.tier] else {
          throw self.missing(args.tier)
        }
        try self.readySession()
        player.currentTime = 0
        player.play()
        invoke.resolve()
      } catch {
        invoke.reject("jingle: \(error.localizedDescription)")
      }
    }
  }

  @objc public func setShake(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ShakeArgs.self)
    DispatchQueue.main.async {
      do {
        if let wav = args.wav { self.rattles[args.tier] = try self.player(wav, loops: -1) }
        let tierChanged = self.shakeTier != args.tier
        self.shakeTier = args.tier
        self.shakeEnabled = args.enabled
        if args.enabled {
          if tierChanged, self.rattling { self.stopRattle(); self.startRattle() }
          self.startMotionIfWanted()
        } else {
          self.stopMotion()
          self.stopRattle()
        }
        invoke.resolve()
      } catch {
        invoke.reject("jingle: \(error.localizedDescription)")
      }
    }
  }

  // MARK: Players

  private func player(_ base64: String, loops: Int) throws -> AVAudioPlayer {
    guard let data = Data(base64Encoded: base64) else {
      throw NSError(domain: "klimper.jingle", code: 1, userInfo: [NSLocalizedDescriptionKey: "bad wav"])
    }
    let player = try AVAudioPlayer(data: data, fileTypeHint: AVFileType.wav.rawValue)
    player.numberOfLoops = loops
    player.prepareToPlay()
    return player
  }

  private func missing(_ tier: String) -> NSError {
    NSError(
      domain: "klimper.jingle", code: 2,
      userInfo: [NSLocalizedDescriptionKey: "no sound for tier \(tier); send it first"])
  }

  private func readySession() throws {
    if sessionReady { return }
    // Ambient: respects the silent switch and mixes with whatever is playing.
    try AVAudioSession.sharedInstance().setCategory(.ambient, options: [.mixWithOthers])
    try AVAudioSession.sharedInstance().setActive(true)
    sessionReady = true
  }

  // MARK: Rattle (main queue)

  private func startRattle() {
    guard !rattling, UIApplication.shared.applicationState == .active,
      let player = rattles[shakeTier]
    else { return }
    do { try readySession() } catch { return }
    rattling = true
    player.currentTime = 0
    player.volume = 1
    player.play()
  }

  private func stopRattle() {
    stopTimer?.invalidate()
    stopTimer = nil
    guard rattling else { return }
    rattling = false
    // A short fade: coins do not stop dead.
    rattles[shakeTier]?.setVolume(0, fadeDuration: 0.12)
    let player = rattles[shakeTier]
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.14) { [weak self] in
      guard let self, !self.rattling else { return }
      player?.stop()
    }
  }

  private func keepRattling() {
    stopTimer?.invalidate()
    stopTimer = Timer.scheduledTimer(withTimeInterval: still, repeats: false) { [weak self] _ in
      self?.stopRattle()
    }
  }

  // One tap per jolt, no closer than `tapSpacing`: the hand feels the coins it moves. Light
  // tiers tap rigid and sharp, heavy ones heavy; the jolt's own force sets the intensity.
  private func tap(force: Double) {
    let now = CACurrentMediaTime()
    guard now - lastTap >= tapSpacing else { return }
    lastTap = now
    let style: UIImpactFeedbackGenerator.FeedbackStyle
    switch shakeTier {
    case "pouch": style = .heavy
    case "handful": style = .medium
    default: style = .rigid
    }
    let weight: Double = ["coin": 0.55, "few": 0.7, "handful": 0.85, "pouch": 1.0][shakeTier] ?? 0.7
    let intensity = min(1.0, max(0.45, weight * (0.6 + 0.4 * force)))
    taps[style]?.impactOccurred(intensity: CGFloat(intensity))
  }

  // MARK: Motion

  private func startMotionIfWanted() {
    guard shakeEnabled, motion.isDeviceMotionAvailable, !motion.isDeviceMotionActive else { return }
    motionQueue.addOperation { [weak self] in self?.jolts.removeAll() }
    motion.deviceMotionUpdateInterval = 1.0 / 40.0
    motion.startDeviceMotionUpdates(to: motionQueue) { [weak self] data, _ in
      guard let self, let data else { return }
      let a = data.userAcceleration
      let magnitude = (a.x * a.x + a.y * a.y + a.z * a.z).squareRoot()
      self.observe(magnitude: magnitude, at: data.timestamp)
    }
  }

  private func stopMotion() {
    if motion.isDeviceMotionActive { motion.stopDeviceMotionUpdates() }
    motionQueue.addOperation { [weak self] in self?.jolts.removeAll() }
  }

  // Runs on the motion queue.
  private func observe(magnitude: Double, at time: TimeInterval) {
    guard magnitude >= jolt else { return }
    jolts.removeAll { time - $0 > window }
    jolts.append(time)
    let starting = jolts.count >= 2
    // 1.3 g is the least that counts, 4 g is as hard as a hand shakes a phone.
    let force = min(1, max(0, (magnitude - jolt) / (4.0 - jolt)))
    DispatchQueue.main.async { [weak self] in
      guard let self, UIApplication.shared.applicationState == .active else { return }
      if self.rattling {
        self.keepRattling()
        self.tap(force: force)
      } else if starting {
        self.startRattle()
        self.keepRattling()
        self.tap(force: force)
      }
    }
  }
}

@_cdecl("init_plugin_klimper_jingle")
func initPlugin() -> Plugin {
  return KlimperJinglePlugin()
}
