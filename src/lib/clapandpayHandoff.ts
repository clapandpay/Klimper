// SPDX-License-Identifier: AGPL-3.0-or-later
// Handoff, the receiving side: how the Klimper app gets a wallet from the clapandpay.com page.
//
// A passkey belongs to a website. The app cannot ask for it, but the page can. So the app makes
// a one-time key pair and opens the page with the public half; the page asks for Face ID or the
// fingerprint, encrypts the wallet key to that public key and gives back one line of text. Only
// the app instance that made the request can read it: copying it, or sending it through a link,
// reveals nothing.
//
//   request   {origin}/connect#v=1&k={P-256 public key, raw, base64url}
//   response  cp1.{base64url( sender public key 65 | iv 12 | AES-256-GCM ciphertext )}
//   key       HKDF-SHA256( ECDH(shared), salt = SHA-256(receiver key | sender key), info = TAG )
//   payload   JSON { v, network, address, masterSk }, additional data = TAG
//
// Two six-digit codes, each shown on both screens. The check code (from the app's public key)
// comes first, before the page seals anything: it catches the wrong page or a mixed-up request.
// It is made from public data, so it cannot stop someone who sees the key first and searches for
// another key with the same six digits; that takes control of the device or the browser, and no
// code a person reads can stop it. The answer code comes last: the page shows it next to the
// answer and the app after opening it, before anything is imported. It is drawn from the ECDH
// secret, which only the app and that page hold, so an answer sealed by anyone else (a swapped
// clipboard) shows another number, and no search can help: the number to match is secret. The
// key travels in the URL fragment, which the browser never sends to the server.
//
// This file has no imports and uses WebCrypto only. klimper/src/lib/clapandpayHandoff.ts is a
// byte-for-byte copy of it (a test in this package fails when they differ): edit here, copy there.

const TAG = 'clapandpay:handoff:v1';
const RESPONSE_PREFIX = 'cp1.';
const RAW_KEY_BYTES = 65;
const IV_BYTES = 12;
const CURVE = { name: 'ECDH', namedCurve: 'P-256' } as const;

/** What the page hands over. `masterSk` is 64 hex characters: a Chia master secret key. */
export interface HandoffPayload {
  v: 1;
  network: 'mainnet' | 'testnet11';
  address: string;
  masterSk: string;
}

/** Why a response could not be opened. */
export type HandoffErrorCode = 'bad_request' | 'bad_response' | 'wrong_receiver';

/** The only error this module throws. */
export class HandoffError extends Error {
  constructor(public readonly code: HandoffErrorCode) {
    super(code);
  }
}

/** An opened response: the wallet, and the code the page shows next to its answer. */
export interface HandoffOpened {
  payload: HandoffPayload;
  /** Six digits; import only once the person confirms the page shows the same. */
  answerCode: string;
}

/** A pending request: keep it in memory until the response arrives, never store it. */
export interface HandoffRequest {
  /** The page to open. */
  url: string;
  /** Six digits shown by both sides, so a person can see the page answers this app. */
  checkCode: string;
  /** Opens the response. Rejects with `HandoffError` for anything not meant for this request. */
  open(response: string): Promise<HandoffOpened>;
}

export function toBase64Url(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function fromBase64Url(text: string): Uint8Array<ArrayBuffer> | null {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) return null;
  try {
    const binary = atob(text.replace(/-/g, '+').replace(/_/g, '/'));
    const bytes = new Uint8Array(binary.length);
    for (let at = 0; at < binary.length; at++) bytes[at] = binary.charCodeAt(at);
    return bytes;
  } catch {
    return null;
  }
}

/** Six digits from the receiver's public key: the same on both screens, or something is wrong. */
export async function checkCodeOf(receiverKey: Uint8Array<ArrayBuffer>): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', receiverKey));
  const number = ((digest[0] ?? 0) << 16) | ((digest[1] ?? 0) << 8) | (digest[2] ?? 0);
  return String(number % 1_000_000).padStart(6, '0');
}

/**
 * What both sides arrive at from the ECDH secret: the AES key, and the answer code. The code is
 * drawn from the secret (HKDF, its own `info`), never from public keys, so nobody without the
 * app's or the page's private key can know it.
 */
export async function sharedKey(
  ownPrivate: CryptoKey,
  otherPublicRaw: Uint8Array<ArrayBuffer>,
  receiverRaw: Uint8Array<ArrayBuffer>,
  senderRaw: Uint8Array<ArrayBuffer>,
): Promise<{ key: CryptoKey; answerCode: string }> {
  const { subtle } = crypto;
  const other = await subtle.importKey('raw', otherPublicRaw, CURVE, false, []);
  const secret = await subtle.deriveBits({ name: 'ECDH', public: other }, ownPrivate, 256);
  const both = new Uint8Array(RAW_KEY_BYTES * 2);
  both.set(receiverRaw, 0);
  both.set(senderRaw, RAW_KEY_BYTES);
  const salt = await subtle.digest('SHA-256', both);
  const material = await subtle.importKey('raw', secret, 'HKDF', false, [
    'deriveKey',
    'deriveBits',
  ]);
  const key = await subtle.deriveKey(
    { name: 'HKDF', hash: 'SHA-256', salt, info: new TextEncoder().encode(TAG) },
    material,
    { name: 'AES-GCM', length: 256 },
    false,
    ['encrypt', 'decrypt'],
  );
  const bits = new Uint8Array(
    await subtle.deriveBits(
      { name: 'HKDF', hash: 'SHA-256', salt, info: new TextEncoder().encode(`${TAG}:answer`) },
      material,
      32,
    ),
  );
  const number =
    (((bits[0] ?? 0) << 24) | ((bits[1] ?? 0) << 16) | ((bits[2] ?? 0) << 8) | (bits[3] ?? 0)) >>>
    0;
  return { key, answerCode: String(number % 1_000_000).padStart(6, '0') };
}

function isPayload(value: unknown): value is HandoffPayload {
  if (typeof value !== 'object' || value === null) return false;
  const { v, network, address, masterSk } = value as Record<string, unknown>;
  return (
    v === 1 &&
    (network === 'mainnet' || network === 'testnet11') &&
    typeof address === 'string' &&
    typeof masterSk === 'string' &&
    /^[0-9a-f]{64}$/.test(masterSk)
  );
}

/** Starts a handoff. `origin` is the page's origin, for example https://clapandpay.com */
export async function createHandoffRequest(origin: string): Promise<HandoffRequest> {
  const { subtle } = crypto;
  // Not extractable: the private half cannot leave this app instance, even by our own code.
  const pair = await subtle.generateKey(CURVE, false, ['deriveBits']);
  const receiverRaw = new Uint8Array(await subtle.exportKey('raw', pair.publicKey));
  const url = `${origin.replace(/\/+$/, '')}/connect#v=1&k=${toBase64Url(receiverRaw)}`;

  return {
    url,
    checkCode: await checkCodeOf(receiverRaw),
    async open(response) {
      const text = response.trim();
      if (!text.startsWith(RESPONSE_PREFIX)) throw new HandoffError('bad_response');
      const bytes = fromBase64Url(text.slice(RESPONSE_PREFIX.length));
      if (bytes === null || bytes.length <= RAW_KEY_BYTES + IV_BYTES) {
        throw new HandoffError('bad_response');
      }
      const senderRaw = bytes.slice(0, RAW_KEY_BYTES);
      const iv = bytes.slice(RAW_KEY_BYTES, RAW_KEY_BYTES + IV_BYTES);
      const sealed = bytes.slice(RAW_KEY_BYTES + IV_BYTES);
      let plain: ArrayBuffer;
      let answerCode: string;
      try {
        const shared = await sharedKey(pair.privateKey, senderRaw, receiverRaw, senderRaw);
        answerCode = shared.answerCode;
        plain = await subtle.decrypt(
          { name: 'AES-GCM', iv, additionalData: new TextEncoder().encode(TAG) },
          shared.key,
          sealed,
        );
      } catch {
        // Sealed for another request (or tampered with): indistinguishable, and both are a no.
        throw new HandoffError('wrong_receiver');
      }
      let payload: unknown;
      try {
        payload = JSON.parse(new TextDecoder().decode(plain));
      } catch {
        throw new HandoffError('bad_response');
      }
      if (!isPayload(payload)) throw new HandoffError('bad_response');
      return { payload, answerCode };
    },
  };
}

export const HANDOFF = { TAG, RESPONSE_PREFIX, RAW_KEY_BYTES, IV_BYTES, CURVE } as const;
