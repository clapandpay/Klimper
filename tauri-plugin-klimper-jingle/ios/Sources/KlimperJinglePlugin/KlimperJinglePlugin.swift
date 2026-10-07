// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on iOS (clapandpay TASK-111). Two moments, both small:
//
// - Coins arrive: `play` rings the tier's ding once.
// - The phone is shaken: `setShake` arms the accelerometer. Every turn of the hand (a jolt, at
//   most one every `turn` seconds) is one knock of a few coins and one haptic tap, together, so
//   the sound and the feel follow the hand and stop when it does. Heavy wallets tap twice, a
//   thud and a clack, like a pouch landing; a near-empty one ticks once.
//
// Sounds play under the ambient session (the silent switch mutes them, music keeps playing).
// Nothing plays or taps unless the app is active.

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
  let knocks: [String]?
}

class KlimperJinglePlugin: Plugin {
  private var dings: [String: AVAudioPlayer] = [:]
  private var knocks: [String: [AVAudioPlayer]] = [:]
  private var tier = "coin"
  private var lastTurn: TimeInterval = 0
  private let motion = CMMotionManager()
  private let heavy = UIImpactFeedbackGenerator(style: .heavy)
  private let rigid = UIImpactFeedbackGenerator(style: .rigid)

  private let jolt = 1.3  // g of user acceleration that counts as a turn of the hand
  private let turn = 0.28  // seconds: slower than this is a shake, faster is a buzz

  override init() {
    super.init()
    try? AVAudioSession.sharedInstance().setCategory(.ambient, options: [.mixWithOthers])
    NotificationCenter.default.addObserver(
      forName: UIApplication.willResignActiveNotification, object: nil, queue: .main
    ) { [weak self] _ in self?.motion.stopDeviceMotionUpdates() }
    NotificationCenter.default.addObserver(
      forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
    ) { [weak self] _ in self?.watch() }
  }

  // MARK: Commands

  @objc public func play(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PlayArgs.self)
    DispatchQueue.main.async {
      if let wav = args.wav, let player = Self.player(wav) { self.dings[args.tier] = player }
      if UIApplication.shared.applicationState == .active { self.dings[args.tier]?.play() }
      invoke.resolve()
    }
  }

  @objc public func setShake(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ShakeArgs.self)
    DispatchQueue.main.async {
      if let wavs = args.knocks { self.knocks[args.tier] = wavs.compactMap(Self.player) }
      self.tier = args.tier
      if args.enabled { self.watch() } else { self.motion.stopDeviceMotionUpdates() }
      invoke.resolve()
    }
  }

  // MARK: Shake

  private func watch() {
    guard knocks[tier] != nil, motion.isDeviceMotionAvailable, !motion.isDeviceMotionActive
    else { return }
    motion.deviceMotionUpdateInterval = 1.0 / 40.0
    motion.startDeviceMotionUpdates(to: .main) { [weak self] data, _ in
      guard let self, let a = data?.userAcceleration else { return }
      let g = (a.x * a.x + a.y * a.y + a.z * a.z).squareRoot()
      let now = CACurrentMediaTime()
      guard g >= self.jolt, now - self.lastTurn >= self.turn,
        UIApplication.shared.applicationState == .active
      else { return }
      self.lastTurn = now
      self.turnOfTheHand(force: min(1, (g - self.jolt) / 2.5))
    }
  }

  // One knock and one tap, together. The harder the hand, the louder; the heavier the wallet,
  // the heavier the tap.
  private func turnOfTheHand(force: Double) {
    if let player = knocks[tier]?.randomElement() {
      player.currentTime = 0
      player.rate = Float(0.9 + 0.15 * Double.random(in: 0...1))
      player.volume = Float(0.6 + 0.4 * force)
      player.play()
    }
    switch tier {
    case "coin":
      rigid.impactOccurred(intensity: 0.7)
    case "few":
      heavy.impactOccurred(intensity: 0.8)
    default:
      heavy.impactOccurred(intensity: 1.0)
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.06) {
        self.rigid.impactOccurred(intensity: self.tier == "pouch" ? 0.9 : 0.6)
      }
    }
  }

  private static func player(_ base64: String) -> AVAudioPlayer? {
    guard let data = Data(base64Encoded: base64),
      let player = try? AVAudioPlayer(data: data, fileTypeHint: AVFileType.wav.rawValue)
    else { return nil }
    player.enableRate = true
    player.prepareToPlay()
    return player
  }
}

@_cdecl("init_plugin_klimper_jingle")
func initPlugin() -> Plugin {
  return KlimperJinglePlugin()
}
