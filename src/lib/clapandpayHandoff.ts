// SPDX-License-Identifier: AGPL-3.0-or-later
// Handoff, the receiving side: how the Klimper app gets a wallet from the clapandpay.com page.
//
// A passkey belongs to a website. The app cannot ask for it, but the page can. So the app makes
// a one-time key pair and opens the page with the public half; the page asks for Face ID or the
// fingerprint, encrypts the wallet key to that public key and gives back one line of text. Only
// the app instance that made the request can read it: copying it, or sending it through a link,
// reveals nothing.
//
//   request   {origin}/connect?v=1&k={P-256 public key, raw, base64url}
//   response  cp1.{base64url( sender public key 65 | iv 12 | AES-256-GCM ciphertext )}
//   key       HKDF-SHA256( ECDH(shared), salt = SHA-256(receiver key | sender key), info = TAG )
//   payload   JSON { v, network, address, masterSk }, additional data = TAG
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

/** A pending request: keep it in memory until the response arrives, never store it. */
export interface HandoffRequest {
  /** The page to open. */
  url: string;
  /** Six digits shown by both sides, so a person can see the page answers this app. */
  checkCode: string;
  /** Opens the response. Rejects with `HandoffError` for anything not meant for this request. */
  open(response: string): Promise<HandoffPayload>;
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

/** The AES key both sides arrive at. */
export async function sharedKey(
  ownPrivate: CryptoKey,
  otherPublicRaw: Uint8Array<ArrayBuffer>,
  receiverRaw: Uint8Array<ArrayBuffer>,
  senderRaw: Uint8Array<ArrayBuffer>,
): Promise<CryptoKey> {
  const { subtle } = crypto;
  const other = await subtle.importKey('raw', otherPublicRaw, CURVE, false, []);
  const secret = await subtle.deriveBits({ name: 'ECDH', public: other }, ownPrivate, 256);
  const both = new Uint8Array(RAW_KEY_BYTES * 2);
  both.set(receiverRaw, 0);
  both.set(senderRaw, RAW_KEY_BYTES);
  const salt = await subtle.digest('SHA-256', both);
  const material = await subtle.importKey('raw', secret, 'HKDF', false, ['deriveKey']);
  return subtle.deriveKey(
    { name: 'HKDF', hash: 'SHA-256', salt, info: new TextEncoder().encode(TAG) },
    material,
    { name: 'AES-GCM', length: 256 },
    false,
    ['encrypt', 'decrypt'],
  );
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
  const url = `${origin.replace(/\/+$/, '')}/connect?v=1&k=${toBase64Url(receiverRaw)}`;

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
      try {
        const key = await sharedKey(pair.privateKey, senderRaw, receiverRaw, senderRaw);
        plain = await subtle.decrypt(
          { name: 'AES-GCM', iv, additionalData: new TextEncoder().encode(TAG) },
          key,
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
      return payload;
    },
  };
}

export const HANDOFF = { TAG, RESPONSE_PREFIX, RAW_KEY_BYTES, IV_BYTES, CURVE } as const;
