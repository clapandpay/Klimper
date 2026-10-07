// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on iOS (clapandpay TASK-111.02): the coin jingle through AVAudioPlayer under
// the ambient audio session (the silent switch mutes it, music keeps playing), the haptic pattern
// the webview chose, and a shake detector on the accelerometer that reports to Rust over a
// channel. Nothing plays or vibrates unless the app is active.

import AVFoundation
import CoreMotion
import Tauri
import UIKit

struct ImpactArg: Decodable {
  let atMs: UInt32
  let strength: Float
}

struct PlayArgs: Decodable {
  let tier: String
  let wav: String?
  let impacts: [ImpactArg]
}

struct ShakeArgs: Decodable {
  let enabled: Bool
  let channel: Channel?
}

class KlimperJinglePlugin: Plugin {
  private var players: [String: AVAudioPlayer] = [:]
  private var sessionReady = false
  private let generators: [UIImpactFeedbackGenerator.FeedbackStyle: UIImpactFeedbackGenerator] = [
    .light: UIImpactFeedbackGenerator(style: .light),
    .medium: UIImpactFeedbackGenerator(style: .medium),
    .heavy: UIImpactFeedbackGenerator(style: .heavy),
  ]

  // Shake detection.
  private let motion = CMMotionManager()
  private let motionQueue = OperationQueue()
  private var shakeChannel: Channel?
  private var shakeEnabled = false
  private var peaks: [TimeInterval] = []
  private var peakIntensity: Double = 0
  private var lastShake: TimeInterval = 0
  private var observers: [NSObjectProtocol] = []

  // A shake is several strong jolts in a short time, then a pause before the next one counts.
  private let jolt: Double = 1.6  // g of user acceleration (gravity removed)
  private let joltsNeeded = 3
  private let window: TimeInterval = 0.7
  private let cooldown: TimeInterval = 1.5

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
        [weak self] _ in self?.stopMotion()
      })
  }

  deinit {
    observers.forEach(NotificationCenter.default.removeObserver)
  }

  // MARK: Commands

  @objc public func play(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PlayArgs.self)
    DispatchQueue.main.async {
      guard UIApplication.shared.applicationState == .active else {
        invoke.resolve()  // background: a wallet in a pocket makes no sound on its own
        return
      }
      do {
        try self.playOnMain(args)
        invoke.resolve()
      } catch {
        invoke.reject("jingle: \(error.localizedDescription)")
      }
    }
  }

  @objc public func setShakeEnabled(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ShakeArgs.self)
    DispatchQueue.main.async {
      self.shakeEnabled = args.enabled
      if args.enabled {
        self.shakeChannel = args.channel ?? self.shakeChannel
        self.startMotionIfWanted()
      } else {
        self.stopMotion()
        self.shakeChannel = nil
      }
      invoke.resolve()
    }
  }

  // MARK: Sound and touch

  private func playOnMain(_ args: PlayArgs) throws {
    if !sessionReady {
      // Ambient: respects the silent switch and mixes with whatever is playing.
      try AVAudioSession.sharedInstance().setCategory(.ambient, options: [.mixWithOthers])
      try AVAudioSession.sharedInstance().setActive(true)
      sessionReady = true
    }
    if let wav = args.wav {
      guard let data = Data(base64Encoded: wav) else {
        throw NSError(domain: "klimper.jingle", code: 1, userInfo: [NSLocalizedDescriptionKey: "bad wav"])
      }
      let player = try AVAudioPlayer(data: data, fileTypeHint: AVFileType.wav.rawValue)
      player.prepareToPlay()
      players[args.tier] = player
    }
    guard let player = players[args.tier] else {
      throw NSError(
        domain: "klimper.jingle", code: 2,
        userInfo: [NSLocalizedDescriptionKey: "no sound for tier \(args.tier); send it first"])
    }
    player.currentTime = 0
    player.play()
    feel(args.impacts)
  }

  private func feel(_ impacts: [ImpactArg]) {
    for impact in impacts {
      let style: UIImpactFeedbackGenerator.FeedbackStyle =
        impact.strength < 0.4 ? .light : impact.strength < 0.7 ? .medium : .heavy
      guard let generator = generators[style] else { continue }
      generator.prepare()
      DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(Int(impact.atMs))) {
        generator.impactOccurred(intensity: CGFloat(max(0.1, min(1, impact.strength))))
      }
    }
  }

  // MARK: Shake

  private func startMotionIfWanted() {
    guard shakeEnabled, shakeChannel != nil, motion.isDeviceMotionAvailable, !motion.isDeviceMotionActive
    else { return }
    peaks.removeAll()
    peakIntensity = 0
    motion.deviceMotionUpdateInterval = 1.0 / 30.0
    motion.startDeviceMotionUpdates(to: motionQueue) { [weak self] data, _ in
      guard let self, let data else { return }
      let a = data.userAcceleration
      let magnitude = (a.x * a.x + a.y * a.y + a.z * a.z).squareRoot()
      self.observe(magnitude: magnitude, at: data.timestamp)
    }
  }

  private func stopMotion() {
    if motion.isDeviceMotionActive { motion.stopDeviceMotionUpdates() }
    peaks.removeAll()
    peakIntensity = 0
  }

  // Runs on the motion queue.
  private func observe(magnitude: Double, at time: TimeInterval) {
    guard magnitude >= jolt else { return }
    peaks.removeAll { time - $0 > window }
    peaks.append(time)
    peakIntensity = max(peakIntensity, magnitude)
    guard peaks.count >= joltsNeeded, time - lastShake >= cooldown else { return }
    lastShake = time
    peaks.removeAll()
    // 1.6 g is the least that counts, 4 g is as hard as a hand shakes a phone.
    let intensity = min(1, max(0, (peakIntensity - jolt) / (4.0 - jolt)))
    peakIntensity = 0
    let channel = shakeChannel
    DispatchQueue.main.async {
      guard UIApplication.shared.applicationState == .active else { return }
      try? channel?.send(["intensity": intensity])
    }
  }
}

@_cdecl("init_plugin_klimper_jingle")
func initPlugin() -> Plugin {
  return KlimperJinglePlugin()
}
