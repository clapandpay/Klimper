// SPDX-License-Identifier: AGPL-3.0-or-later
// Passkey registration and assertion with the PRF extension (iOS 18). The PRF output is
// resolved to the Rust side of the plugin, which unwraps the wallet key; the webview never
// sees it. Also keeps the wallet data directory out of backups.

import AuthenticationServices
import CryptoKit
import SwiftRs
import Tauri
import UIKit

struct RegisterArgs: Decodable {
  let rpId: String
  let userId: String
  let userName: String
  let challenge: String
  let prfSalt: String
}

struct AssertArgs: Decodable {
  let rpId: String
  let credentialId: String
  let challenge: String
  let prfSalt: String
}

struct BackupArgs: Decodable {
  let path: String
}

extension Data {
  init?(base64URL value: String) {
    var base64 = value.replacingOccurrences(of: "-", with: "+")
      .replacingOccurrences(of: "_", with: "/")
    base64 += String(repeating: "=", count: (4 - base64.count % 4) % 4)
    self.init(base64Encoded: base64)
  }

  var base64URL: String {
    base64EncodedString().replacingOccurrences(of: "+", with: "-")
      .replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "=", with: "")
  }
}

class KlimperPasskeyPlugin: Plugin, ASAuthorizationControllerDelegate,
  ASAuthorizationControllerPresentationContextProviding
{
  private var pending: Invoke?
  private var controller: ASAuthorizationController?

  // Set on the directory, so it covers files Sage replaces or creates later.
  @objc public func excludeFromBackup(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(BackupArgs.self)
    var url = URL(fileURLWithPath: args.path, isDirectory: true)
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try url.setResourceValues(values)
    invoke.resolve()
  }

  @objc public func register(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(RegisterArgs.self)
    guard #available(iOS 18.0, *) else {
      invoke.reject("passkeys with PRF need iOS 18")
      return
    }
    guard let userId = Data(base64URL: args.userId),
      let challenge = Data(base64URL: args.challenge),
      let salt = Data(base64URL: args.prfSalt)
    else {
      invoke.reject("invalid arguments")
      return
    }

    let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
      relyingPartyIdentifier: args.rpId)
    let request = provider.createCredentialRegistrationRequest(
      challenge: challenge, name: args.userName, userID: userId)
    request.prf = .inputValues(.init(saltInput1: salt))
    // Face ID or the passcode at enrolment too, not only at unlock.
    request.userVerificationPreference = .required
    perform(request, invoke)
  }

  @objc public func assert(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(AssertArgs.self)
    guard #available(iOS 18.0, *) else {
      invoke.reject("passkeys with PRF need iOS 18")
      return
    }
    guard let credentialId = Data(base64URL: args.credentialId),
      let challenge = Data(base64URL: args.challenge),
      let salt = Data(base64URL: args.prfSalt)
    else {
      invoke.reject("invalid arguments")
      return
    }

    let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
      relyingPartyIdentifier: args.rpId)
    let request = provider.createCredentialAssertionRequest(challenge: challenge)
    request.allowedCredentials = [
      ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: credentialId)
    ]
    request.userVerificationPreference = .required
    request.prf = .inputValues(.init(saltInput1: salt))
    perform(request, invoke)
  }

  private func perform(_ request: ASAuthorizationRequest, _ invoke: Invoke) {
    DispatchQueue.main.async {
      if let previous = self.pending {
        previous.reject("superseded by a newer passkey request")
      }
      self.pending = invoke
      let controller = ASAuthorizationController(authorizationRequests: [request])
      controller.delegate = self
      controller.presentationContextProvider = self
      self.controller = controller
      controller.performRequests()
    }
  }

  private enum Outcome {
    case success(JsonObject)
    case failure(String)
  }

  /// The app went to the background: tell Rust, which locks the wallet itself (TASK-117), so
  /// the lock never waits on the webview, which iOS may suspend first.
  private var background: Channel?
  private var backgroundObserver: NSObjectProtocol?

  @objc public func watchBackground(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(WatchBackgroundArgs.self)
    background = args.channel
    if backgroundObserver == nil {
      backgroundObserver = NotificationCenter.default.addObserver(
        forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main
      ) { [weak self] _ in
        // Time to finish the lock before iOS suspends the process, even when a command
        // holds the wallet for a moment. Ended once: by the timer or by iOS, whichever is
        // first (both run on the main queue).
        var task = UIBackgroundTaskIdentifier.invalid
        let end = {
          guard task != .invalid else { return }
          UIApplication.shared.endBackgroundTask(task)
          task = .invalid
        }
        task = UIApplication.shared.beginBackgroundTask(expirationHandler: end)
        self?.background?.send([:] as JsonObject)
        DispatchQueue.main.asyncAfter(deadline: .now() + 5, execute: end)
      }
    }
    invoke.resolve()
  }

  private func finish(_ outcome: Outcome) {
    guard let invoke = pending else { return }
    pending = nil
    controller = nil
    switch outcome {
    case .success(let data): invoke.resolve(data)
    case .failure(let message): invoke.reject(message)
    }
  }

  func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
    manager.viewController?.view.window ?? ASPresentationAnchor()
  }

  func authorizationController(
    controller: ASAuthorizationController,
    didCompleteWithAuthorization authorization: ASAuthorization
  ) {
    // A prompt that was superseded by a newer one never answers the newer request.
    guard controller === self.controller else { return }
    guard #available(iOS 18.0, *) else {
      finish(.failure("passkeys with PRF need iOS 18"))
      return
    }

    if let registration = authorization.credential
      as? ASAuthorizationPlatformPublicKeyCredentialRegistration
    {
      var data: JsonObject = ["credentialId": registration.credentialID.base64URL]
      if let prf = registration.prf {
        guard prf.isSupported else {
          finish(.failure("this passkey provider does not support PRF"))
          return
        }
        if let first = prf.first {
          data["prfOutput"] = first.withUnsafeBytes { Data($0) }.base64URL
        }
      }
      finish(.success(data))
    } else if let assertion = authorization.credential
      as? ASAuthorizationPlatformPublicKeyCredentialAssertion
    {
      var data: JsonObject = ["credentialId": assertion.credentialID.base64URL]
      if let prf = assertion.prf {
        data["prfOutput"] = prf.first.withUnsafeBytes { Data($0) }.base64URL
      }
      finish(.success(data))
    } else {
      finish(.failure("unexpected credential type"))
    }
  }

  func authorizationController(
    controller: ASAuthorizationController, didCompleteWithError error: Error
  ) {
    guard controller === self.controller else { return }
    finish(.failure(error.localizedDescription))
  }
}

struct WatchBackgroundArgs: Decodable {
  let channel: Channel
}

@_cdecl("init_plugin_klimper_passkey")
func initPlugin() -> Plugin {
  return KlimperPasskeyPlugin()
}
