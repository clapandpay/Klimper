// SPDX-License-Identifier: AGPL-3.0-or-later
// Handoff, the receiving side: how the Klimper app gets a wallet from the clapandpay.com page.
//
// A passkey belongs to a website. The app cannot ask for it, but the page can. So the app makes
// a one-time key pair and a nonce and opens the page with them; the page asks for Face ID or the
// fingerprint, encrypts the wallet's 24 words to that key and answers by opening a link on
// clapandpay.com that only the signed Klimper app is given (version 2, TASK-129):
//
//   request   {origin}/connect#v=2&k={P-256 public key, raw, base64url}&n={nonce, 16 bytes}
//   answer    https://clapandpay.com/klimper/return#cp2.{base64url( sender key 65 | iv 12 | ct )}
//   key       HKDF-SHA256( ECDH(shared), salt = SHA-256(receiver key | sender key | nonce), info = TAG )
//   payload   JSON { v: 2, network, address, words }, additional data = TAG | nonce
//
// Why this is enough without codes to compare (the v1 handoff had two): the answer reaches this
// app only, because iOS and Android hand the return path to the app that the domain names in
// apple-app-site-association and assetlinks.json. A page that made its own request (a phishing
// site, a chat bot) can still have a person approve it, but the answer goes to Klimper, which
// never made that key and throws it away; nobody else sees a byte. Klimper accepts an answer
// only for its own key and nonce, once, within two minutes of the request. The key travels in
// the URL fragment, which the browser never sends to the server; so does the answer.
//
// This file has no imports and uses WebCrypto only. klimper/src/lib/clapandpayHandoff.ts is a
// byte-for-byte copy of it (a test in this package fails when they differ): edit here, copy there.

const TAG = 'clapandpay:handoff:v2';
const RESPONSE_PREFIX = 'cp2.';
const RAW_KEY_BYTES = 65;
const IV_BYTES = 12;
const NONCE_BYTES = 16;
const CURVE = { name: 'ECDH', namedCurve: 'P-256' } as const;
/** Where the page sends the answer. In Klimper's applinks; nothing else is given it. */
const RETURN_URL = 'https://clapandpay.com/klimper/return';
/** How long a request can be answered. Long enough for Face ID, short enough to be this one. */
const REQUEST_TTL_MS = 2 * 60 * 1000;

/** What the page hands over: the wallet's 24 recovery words, and what they should give. */
export interface HandoffPayload {
  v: 2;
  network: 'mainnet' | 'testnet11';
  /** The first address of the wallet, so the app can check the words are the page's wallet. */
  address: string;
  /** 24 lowercase BIP-39 words, single spaces. */
  words: string;
}

/** Why an answer could not be opened. */
export type HandoffErrorCode =
  | 'bad_request'
  | 'bad_response'
  | 'wrong_receiver'
  | 'expired'
  | 'used';

/** The only error this module throws. */
export class HandoffError extends Error {
  constructor(public readonly code: HandoffErrorCode) {
    super(code);
  }
}

/** A pending request: keep it in memory until the answer arrives, never store it. */
export interface HandoffRequest {
  /** The page to open. */
  url: string;
  /**
   * Opens the answer: the return link as the system delivered it, or the text after `#`.
   * Rejects with `HandoffError` for anything not meant for this request, after two minutes
   * (`expired`), and once an answer was opened (`used`): the one-time key is gone by then.
   */
  open(answer: string): Promise<HandoffPayload>;
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

/** The answer part of a return link (`…/klimper/return#cp2.…`), or the text as given. */
export function answerOf(link: string): string {
  const text = link.trim();
  if (text.startsWith(RETURN_URL)) return text.slice(RETURN_URL.length).replace(/^#/, '');
  return text;
}

/** Whether `link` is the handoff's return link: the app routes it here, not elsewhere. */
export function isReturnLink(link: string): boolean {
  const text = link.trim();
  return text === RETURN_URL || text.startsWith(`${RETURN_URL}#`);
}

/** The AES key both sides arrive at from the ECDH secret, bound to both keys and the nonce. */
export async function sharedKey(
  ownPrivate: CryptoKey,
  otherPublicRaw: Uint8Array<ArrayBuffer>,
  receiverRaw: Uint8Array<ArrayBuffer>,
  senderRaw: Uint8Array<ArrayBuffer>,
  nonce: Uint8Array<ArrayBuffer>,
): Promise<CryptoKey> {
  const { subtle } = crypto;
  const other = await subtle.importKey('raw', otherPublicRaw, CURVE, false, []);
  const secret = await subtle.deriveBits({ name: 'ECDH', public: other }, ownPrivate, 256);
  const all = new Uint8Array(RAW_KEY_BYTES * 2 + NONCE_BYTES);
  all.set(receiverRaw, 0);
  all.set(senderRaw, RAW_KEY_BYTES);
  all.set(nonce, RAW_KEY_BYTES * 2);
  const salt = await subtle.digest('SHA-256', all);
  const material = await subtle.importKey('raw', secret, 'HKDF', false, ['deriveKey']);
  return subtle.deriveKey(
    { name: 'HKDF', hash: 'SHA-256', salt, info: new TextEncoder().encode(TAG) },
    material,
    { name: 'AES-GCM', length: 256 },
    false,
    ['encrypt', 'decrypt'],
  );
}

/** The additional data of the seal: the protocol tag and the request's nonce. */
export function additionalData(nonce: Uint8Array<ArrayBuffer>): Uint8Array<ArrayBuffer> {
  const tag = new TextEncoder().encode(TAG);
  const out = new Uint8Array(tag.length + nonce.length);
  out.set(tag, 0);
  out.set(nonce, tag.length);
  return out;
}

function isPayload(value: unknown): value is HandoffPayload {
  if (typeof value !== 'object' || value === null) return false;
  const { v, network, address, words } = value as Record<string, unknown>;
  return (
    v === 2 &&
    (network === 'mainnet' || network === 'testnet11') &&
    typeof address === 'string' &&
    typeof words === 'string' &&
    /^[a-z]+( [a-z]+){23}$/.test(words)
  );
}

/** Starts a handoff. `origin` is the page's origin, for example https://envelope.clapandpay.com */
export async function createHandoffRequest(
  origin: string,
  options: { now?: () => number } = {},
): Promise<HandoffRequest> {
  const { subtle } = crypto;
  const now = options.now ?? Date.now;
  // Not extractable: the private half cannot leave this app instance, even by our own code.
  let pair: CryptoKeyPair | null = await subtle.generateKey(CURVE, false, ['deriveBits']);
  const receiverRaw = new Uint8Array(await subtle.exportKey('raw', pair.publicKey));
  const nonce = crypto.getRandomValues(new Uint8Array(NONCE_BYTES));
  const madeAt = now();
  const url = `${origin.replace(/\/+$/, '')}/connect#v=2&k=${toBase64Url(receiverRaw)}&n=${toBase64Url(nonce)}`;

  return {
    url,
    async open(answer) {
      if (pair === null) throw new HandoffError('used');
      if (now() - madeAt > REQUEST_TTL_MS) {
        pair = null; // forget the one-time key: nothing can be opened with it any more
        throw new HandoffError('expired');
      }
      const text = answerOf(answer);
      if (!text.startsWith(RESPONSE_PREFIX)) throw new HandoffError('bad_response');
      const bytes = fromBase64Url(text.slice(RESPONSE_PREFIX.length));
      if (bytes === null || bytes.length <= RAW_KEY_BYTES + IV_BYTES) {
        throw new HandoffError('bad_response');
      }
      const senderRaw = bytes.slice(0, RAW_KEY_BYTES);
      const iv = bytes.slice(RAW_KEY_BYTES, RAW_KEY_BYTES + IV_BYTES);
      const sealed = bytes.slice(RAW_KEY_BYTES + IV_BYTES);
      let plain: Uint8Array;
      try {
        const key = await sharedKey(pair.privateKey, senderRaw, receiverRaw, senderRaw, nonce);
        plain = new Uint8Array(
          await subtle.decrypt(
            { name: 'AES-GCM', iv, additionalData: additionalData(nonce) },
            key,
            sealed,
          ),
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
      } finally {
        plain.fill(0);
      }
      if (!isPayload(payload)) throw new HandoffError('bad_response');
      pair = null; // one answer per request: the one-time key is forgotten
      return payload;
    },
  };
}

export const HANDOFF = {
  TAG,
  RESPONSE_PREFIX,
  RAW_KEY_BYTES,
  IV_BYTES,
  NONCE_BYTES,
  CURVE,
  RETURN_URL,
  REQUEST_TTL_MS,
} as const;
