// SPDX-License-Identifier: AGPL-3.0-or-later
// Push wake-ups for Xchonnect sessions (clapandpay TASK-087).
//
// Tauri has no remote-push plugin on iOS, so this one asks for permission, registers with APNs
// and hands the two things Rust needs over a channel: the device token, and "a notification
// arrived". The notification itself is never read: not its payload, not its text. Rust answers a
// wake-up by fetching the mailbox, and only what the relay returns is acted on.
//
// iOS reports the device token and silent arrivals to the application delegate, which is tao's
// and has no push methods. They are added to its class at run time (class_addMethod), so
// Sage's window code stays untouched.
//
// Requests with the phone locked (clapandpay TASK-125.04): a push that carries content-available
// wakes the app in the background. Rust reads every mailbox and calls backgroundDone; only then
// is iOS told the fetch finished, at the latest after 25 seconds. And when the user leaves
// Klimper (for the browser, right after pairing or approving), the app asks for about 25 more
// seconds, so the website's next reads are answered before iOS suspends it.

import ObjectiveC
import SwiftRs
import Tauri
import UIKit
import UserNotifications

struct StartArgs: Decodable {
  let channel: Channel
}

class KlimperXchonnectPlugin: Plugin, UNUserNotificationCenterDelegate {
  static weak var current: KlimperXchonnectPlugin?

  private var channel: Channel?
  // A notification can start the app, and its tap reaches us before Rust has given us a
  // channel. One flag is enough: a wake-up carries nothing else.
  private var wakeBeforeChannel = false
  // Background fetches waiting for Rust to have read the mailboxes, and the time asked for.
  private var pendingFetches: [(UIBackgroundFetchResult) -> Void] = []
  private var fetchTask: UIBackgroundTaskIdentifier = .invalid
  private var leaveTask: UIBackgroundTaskIdentifier = .invalid

  // Inside the roughly 30 seconds iOS allows either way.
  private static let backgroundSeconds: TimeInterval = 25

  override init() {
    super.init()
    KlimperXchonnectPlugin.current = self
    onMain {
      UNUserNotificationCenter.current().delegate = self
      self.installApplicationDelegateHooks()
      let center = NotificationCenter.default
      center.addObserver(
        self, selector: #selector(self.didEnterBackground),
        name: UIApplication.didEnterBackgroundNotification, object: nil)
      center.addObserver(
        self, selector: #selector(self.willEnterForeground),
        name: UIApplication.willEnterForegroundNotification, object: nil)
    }
  }

  // MARK: Background time

  // The user switched to the browser: keep answering for a little while.
  @objc private func didEnterBackground() {
    endLeaveTask()
    leaveTask = UIApplication.shared.beginBackgroundTask(withName: "klimper-xchonnect") {
      self.endLeaveTask()
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + Self.backgroundSeconds) {
      self.endLeaveTask()
    }
  }

  @objc private func willEnterForeground() {
    endLeaveTask()
  }

  private func endLeaveTask() {
    if leaveTask != .invalid {
      UIApplication.shared.endBackgroundTask(leaveTask)
      leaveTask = .invalid
    }
  }

  // A content-available push: wake Rust, and hold iOS's completion until it has read.
  fileprivate func backgroundWake(_ completion: @escaping (UIBackgroundFetchResult) -> Void) {
    onMain {
      self.pendingFetches.append(completion)
      if self.fetchTask == .invalid {
        self.fetchTask = UIApplication.shared.beginBackgroundTask(withName: "klimper-push") {
          self.finishFetches()
        }
      }
      DispatchQueue.main.asyncAfter(deadline: .now() + Self.backgroundSeconds) {
        self.finishFetches()
      }
      if self.channel == nil {
        self.wakeBeforeChannel = true
      } else {
        self.send(["kind": "backgroundWake"])
      }
    }
  }

  private func finishFetches() {
    let fetches = pendingFetches
    pendingFetches = []
    for completion in fetches { completion(.newData) }
    if fetchTask != .invalid {
      UIApplication.shared.endBackgroundTask(fetchTask)
      fetchTask = .invalid
    }
  }

  // Rust has read every mailbox after a background wake-up.
  @objc public func backgroundDone(_ invoke: Invoke) {
    onMain {
      self.finishFetches()
      invoke.resolve()
    }
  }

  // MARK: Commands

  // Called when the app starts. Registers silently when the user already allowed
  // notifications; it never shows the permission prompt (that waits for the first pairing).
  @objc public func start(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(StartArgs.self)
    channel = args.channel
    if wakeBeforeChannel {
      wakeBeforeChannel = false
      send(["kind": "wake"])
    }
    UNUserNotificationCenter.current().getNotificationSettings { settings in
      let state = Self.state(settings.authorizationStatus)
      if state == "granted" { self.registerWithApns() }
      invoke.resolve(["state": state])
    }
  }

  // The first pairing asks. iOS shows its prompt once; after a refusal this only reports it.
  @objc public func requestPermission(_ invoke: Invoke) {
    UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) {
      granted, _ in
      if granted { self.registerWithApns() }
      invoke.resolve(["state": granted ? "granted" : "denied"])
    }
  }

  // MARK: Notifications

  // In the foreground the session loop is already reading the mailbox: no banner, just a wake.
  func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
    withCompletionHandler completionHandler:
      @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    wake()
    completionHandler([])
  }

  // A tap opens the app; the wake makes the loop read the mailbox at once, so the request
  // is waiting on screen.
  func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    wake()
    completionHandler()
  }

  // MARK: Plumbing

  private static func state(_ status: UNAuthorizationStatus) -> String {
    switch status {
    case .authorized, .provisional, .ephemeral: return "granted"
    case .denied: return "denied"
    default: return "undetermined"
    }
  }

  private func registerWithApns() {
    onMain {
      self.installApplicationDelegateHooks()
      UIApplication.shared.registerForRemoteNotifications()
    }
  }

  fileprivate func wake() {
    guard channel != nil else {
      wakeBeforeChannel = true
      return
    }
    send(["kind": "wake"])
  }

  fileprivate func deviceToken(_ data: Data) {
    send(["kind": "token", "platform": "apns", "token": data.map { String(format: "%02x", $0) }.joined()])
  }

  private func send(_ event: JsonObject) {
    channel?.send(event)
  }

  private func onMain(_ work: @escaping () -> Void) {
    if Thread.isMainThread { work() } else { DispatchQueue.main.async(execute: work) }
  }

  private var hooksInstalled = false

  // Tauri creates plugins before UIApplicationMain runs, so at init there is no application
  // and no delegate object yet. tao has already registered its delegate class by then, under
  // the name "AppDelegate": the methods go onto the class, before UIKit ever looks at it.
  // Called again before registering with APNs, in case the class was not there the first time.
  private func installApplicationDelegateHooks() {
    if hooksInstalled { return }
    guard let target: AnyClass = NSClassFromString("AppDelegate") else {
      NSLog("klimper push: no AppDelegate class yet; the device token cannot reach us")
      return
    }
    hooksInstalled = true

    let registered: @convention(block) (AnyObject, UIApplication, Data) -> Void = { _, _, token in
      KlimperXchonnectPlugin.current?.deviceToken(token)
    }
    add(
      target, #selector(UIApplicationDelegate.application(_:didRegisterForRemoteNotificationsWithDeviceToken:)),
      registered, "v@:@@")

    // Polling carries on without push: a failure here is logged and nothing else.
    let failed: @convention(block) (AnyObject, UIApplication, Error) -> Void = { _, _, error in
      NSLog("klimper push: APNs registration failed: \(error.localizedDescription)")
    }
    add(
      target, #selector(UIApplicationDelegate.application(_:didFailToRegisterForRemoteNotificationsWithError:)),
      failed, "v@:@@")

    // Reached when a push carries content-available, in front or in the background (with the
    // phone locked, too). The payload is not read: the mailbox is.
    let received:
      @convention(block) (
        AnyObject, UIApplication, [AnyHashable: Any],
        @escaping (UIBackgroundFetchResult) -> Void
      ) -> Void = { _, _, _, completion in
        guard let plugin = KlimperXchonnectPlugin.current else {
          completion(.noData)
          return
        }
        plugin.backgroundWake(completion)
      }
    add(
      target,
      #selector(UIApplicationDelegate.application(_:didReceiveRemoteNotification:fetchCompletionHandler:)),
      received, "v@:@@@?")
  }

  private func add(_ target: AnyClass, _ selector: Selector, _ block: Any, _ types: String) {
    let implementation = imp_implementationWithBlock(block)
    if !class_addMethod(target, selector, implementation, types) {
      // tao implements it itself now: that is a change to review, not to override.
      NSLog("klimper push: \(target) already implements \(selector); push may not reach us")
    }
  }
}

@_cdecl("init_plugin_klimper_xchonnect")
func initPlugin() -> Plugin {
  return KlimperXchonnectPlugin()
}
