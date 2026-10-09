// SPDX-License-Identifier: AGPL-3.0-or-later
// Every way an Xchonnect link can reach Klimper, parsed in one place (clapandpay
// TASK-072, TASK-085).
//
// Three forms arrive:
//
//   xchonnect:v1?r=…&s=…                       a scanned QR code (spec 6.2)
//   https://clapandpay.com/pair#r=…&s=…        a universal link, same parameters
//   https://clapandpay.com/req#mbx=…           "there is something in your mailbox" (spec 8.2)
//
// The parameters are in the **fragment** on purpose: a fragment is never sent to a web
// server and never appears in a server log. For `/pair` the fragment contains the pairing
// secret `s`, which is the out-of-band secret that authenticates the whole handshake, so
// this module is also where the rule "never log the fragment" is kept:
//
//   * every failure is reported as a code, never as the text that failed;
//   * `redactLink` is the only way a link is allowed to appear in a message;
//   * the parsed link is handed to the screen in memory (`xchonnectLinkHandoff.ts`),
//     never through the app's URL — Klimper uses a hash router, so a fragment in the URL
//     would both break routing and write the secret into the history stack.
//
// Nothing here throws: a link from outside the app is untrusted input, and
// `parseXchonnectLink` is a total function whose failure is just another result.

/** Where our universal links live (clapandpay decision-003 and decision-004). */
export const XCHONNECT_LINK_HOST = 'clapandpay.com';
/** The base a dApp appends `/req#mbx=…` to; announced at pairing (integration guide 5.3). */
export const XCHONNECT_LINK_BASE = `https://${XCHONNECT_LINK_HOST}`;
/** The only two paths the app claims. The AASA file covers exactly these. */
export const XCHONNECT_PAIR_PATH = '/pair';
export const XCHONNECT_REQUEST_PATH = '/req';

const SCHEME_PREFIX = 'xchonnect:v1?';
/** The same cap `xchonnect-core` applies before it parses anything. */
const MAX_LENGTH = 2048;

/** Why a string was not an Xchonnect link. A code, so no fragment can ride along. */
export type LinkProblem =
  | 'empty'
  | 'tooLong'
  | 'unknownScheme'
  | 'otherHost'
  | 'unknownPath'
  | 'noParameters';

export type XchonnectLink =
  /** A pairing code. `uri` is passed whole to `inspectPairingUri`, which parses both forms. */
  | { kind: 'pair'; uri: string }
  /**
   * A same-device request hint. `mailboxHint` is untrusted and is not acted on: the
   * screen syncs the sessions and renders only what an authenticated envelope produced.
   */
  | { kind: 'request'; mailboxHint: string | null }
  | { kind: 'notALink'; problem: LinkProblem };

/**
 * Parse any of the three forms. Never throws, never echoes the input.
 *
 * A `https://` link is accepted only on our own host and only on `/pair` or `/req`, so a
 * QR code that happens to hold some other URL keeps whatever meaning it had elsewhere in
 * the app.
 */
export function parseXchonnectLink(raw: string): XchonnectLink {
  const text = raw.trim();
  if (text === '') return { kind: 'notALink', problem: 'empty' };
  if (text.length > MAX_LENGTH) return { kind: 'notALink', problem: 'tooLong' };

  if (text.startsWith(SCHEME_PREFIX)) {
    return text.length > SCHEME_PREFIX.length
      ? { kind: 'pair', uri: text }
      : { kind: 'notALink', problem: 'noParameters' };
  }

  if (!text.startsWith('https://')) {
    return { kind: 'notALink', problem: 'unknownScheme' };
  }

  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return { kind: 'notALink', problem: 'unknownScheme' };
  }

  // `host` carries the port, and userinfo is checked separately: an attacker's
  // `https://clapandpay.com@evil.example/pair` must not look like our host.
  if (
    url.host !== XCHONNECT_LINK_HOST ||
    url.username !== '' ||
    url.password !== ''
  ) {
    return { kind: 'notALink', problem: 'otherHost' };
  }

  const fragment = url.hash.startsWith('#') ? url.hash.slice(1) : url.hash;

  if (url.pathname === XCHONNECT_PAIR_PATH) {
    // The whole URL goes to the plugin: xchonnect-core parses the universal-link form
    // itself, so nothing here has to understand the parameters.
    return fragment === ''
      ? { kind: 'notALink', problem: 'noParameters' }
      : { kind: 'pair', uri: text };
  }

  if (url.pathname === XCHONNECT_REQUEST_PATH) {
    return { kind: 'request', mailboxHint: mailboxHintOf(fragment) };
  }

  return { kind: 'notALink', problem: 'unknownPath' };
}

/**
 * The `mbx` parameter of a `/req` fragment, if it is there and looks like a mailbox id.
 *
 * It is a hint and nothing more — anyone can open that link. A request exists only once
 * the session has opened an authenticated envelope, so the hint is never compared against
 * anything that decides what the user sees.
 */
function mailboxHintOf(fragment: string): string | null {
  if (fragment === '') return null;
  const value = new URLSearchParams(fragment).get('mbx');
  if (value === null || value === '') return null;
  // base64url, as every binary value in the protocol is, and short.
  return /^[A-Za-z0-9_-]{1,128}$/.test(value) ? value : null;
}

/**
 * How a link may be shown or logged: the host and the path, never the fragment.
 *
 * Used wherever a message would otherwise interpolate the link the user pasted.
 */
export function redactLink(raw: string): string {
  const text = raw.trim();
  if (text.startsWith(SCHEME_PREFIX)) return 'xchonnect:v1?…';
  try {
    const url = new URL(text);
    if (url.protocol !== 'https:' && url.protocol !== 'http:') {
      return `${url.protocol}…`;
    }
    const suffix = url.hash === '' && url.search === '' ? '' : '#…';
    return `${url.protocol}//${url.host}${url.pathname}${suffix}`;
  } catch {
    return '…';
  }
}

/**
 * May Klimper navigate the user to this URL after answering a request?
 *
 * Only if it is on the session's own verified domain (spec 8.2, integration guide 5.4).
 * `xchonnect-core` already refuses an origin document whose `return_url` leaves the
 * claimed domain, so pairing fails rather than handing us a URL to police. This is the
 * second check, in the place that actually performs the navigation: a dApp whose origin
 * key leaked (T18) must not be able to use Klimper as a one-tap redirect into a page the
 * user has every reason to trust (T3).
 *
 * The comparison is byte-exact on the whole authority, which is what refuses the cases a
 * host-prefix check waves through: `evil-pengui.xyz`, `pengui.xyz.evil.com`, userinfo, a
 * port, and a trailing dot.
 */
export function isReturnUrlAllowed(
  returnUrl: string,
  verifiedDomainAscii: string,
): boolean {
  if (verifiedDomainAscii === '') return false;
  const text = returnUrl.trim();
  if (text.length > MAX_LENGTH) return false;
  if (!text.startsWith('https://')) return false;

  // The raw authority, before any normalisation: `new URL` drops a default port and
  // lowercases the host, and both of those differences matter to a byte-exact rule.
  const rest = text.slice('https://'.length);
  const end = rest.search(/[/?#]/);
  const authority = end === -1 ? rest : rest.slice(0, end);
  if (authority !== verifiedDomainAscii) return false;

  try {
    const url = new URL(text);
    return (
      url.protocol === 'https:' &&
      url.username === '' &&
      url.password === '' &&
      url.port === '' &&
      url.hostname === verifiedDomainAscii
    );
  } catch {
    return false;
  }
}
