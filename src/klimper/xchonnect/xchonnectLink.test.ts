// SPDX-License-Identifier: AGPL-3.0-or-later
// Every form a link can arrive in, and every form it must be refused in
// (clapandpay TASK-072, TASK-085).

import { describe, expect, it } from 'vitest';
import {
  isReturnUrlAllowed,
  parseXchonnectLink,
  redactLink,
  XCHONNECT_LINK_BASE,
} from './xchonnectLink';

const PARAMS =
  'r=https%3A%2F%2Frelay.example%2F&m=bWJ4&w=dG9rZW4&k=a2V5&s=c2VjcmV0&d=pengui.xyz&x=1899999999&i=k1&o=c2ln';

describe('parseXchonnectLink', () => {
  it('reads a scanned pairing code', () => {
    const link = parseXchonnectLink(`xchonnect:v1?${PARAMS}`);
    expect(link).toEqual({ kind: 'pair', uri: `xchonnect:v1?${PARAMS}` });
  });

  it('reads a universal link and keeps the whole URL for the parser in Rust', () => {
    // xchonnect-core parses both forms, so nothing here has to understand the
    // parameters — which is also why nothing here can leak them.
    const url = `${XCHONNECT_LINK_BASE}/pair#${PARAMS}`;
    expect(parseXchonnectLink(url)).toEqual({ kind: 'pair', uri: url });
  });

  it('reads a same-device request hint', () => {
    expect(
      parseXchonnectLink(`${XCHONNECT_LINK_BASE}/req#mbx=bWFpbGJveA`),
    ).toEqual({
      kind: 'request',
      mailboxHint: 'bWFpbGJveA',
    });
  });

  it('accepts a request link with no hint at all', () => {
    // The hint is a convenience; the mailbox is fetched either way.
    expect(parseXchonnectLink(`${XCHONNECT_LINK_BASE}/req`)).toEqual({
      kind: 'request',
      mailboxHint: null,
    });
  });

  it('drops a hint that is not shaped like a mailbox id', () => {
    const link = parseXchonnectLink(
      `${XCHONNECT_LINK_BASE}/req#mbx=${'../etc/passwd'}`,
    );
    expect(link).toEqual({ kind: 'request', mailboxHint: null });
  });

  it.each([
    ['empty input', '   '],
    ['another scheme', 'http://clapandpay.com/pair#r=1'],
    ['a plain address', 'xch1qqqq'],
    ['an offer', 'offer1abcdef'],
    ['the wrong version', 'xchonnect:v2?r=x'],
    ['a pairing link with no parameters', `${XCHONNECT_LINK_BASE}/pair`],
    ['an unclaimed path', `${XCHONNECT_LINK_BASE}/settings#x=1`],
    ['another host', `https://evil.example/pair#${PARAMS}`],
    [
      'a host that only starts the same',
      `https://clapandpay.com.evil.example/pair#${PARAMS}`,
    ],
    [
      'userinfo pretending to be the host',
      `https://clapandpay.com@evil.example/pair#${PARAMS}`,
    ],
    ['a port on our host', `https://clapandpay.com:8443/pair#${PARAMS}`],
    ['something far too long', `xchonnect:v1?${'a'.repeat(3000)}`],
  ])('refuses %s', (_name, input) => {
    expect(parseXchonnectLink(input).kind).toBe('notALink');
  });

  it('never puts the fragment in the result of a refusal', () => {
    const refusal = parseXchonnectLink(
      `https://evil.example/pair#s=THE-PAIRING-SECRET`,
    );
    expect(JSON.stringify(refusal)).not.toContain('THE-PAIRING-SECRET');
  });
});

describe('redactLink', () => {
  it('keeps the host and the path and drops the fragment', () => {
    expect(redactLink(`${XCHONNECT_LINK_BASE}/pair#s=THE-PAIRING-SECRET`)).toBe(
      'https://clapandpay.com/pair#…',
    );
  });

  it('reduces a scanned code to its scheme', () => {
    expect(redactLink(`xchonnect:v1?${PARAMS}`)).toBe('xchonnect:v1?…');
  });

  it('says nothing at all about something unparseable', () => {
    expect(redactLink('not a url at all')).toBe('…');
  });
});

describe('isReturnUrlAllowed', () => {
  it('allows a URL on the session domain', () => {
    expect(
      isReturnUrlAllowed('https://pengui.xyz/xchonnect/return', 'pengui.xyz'),
    ).toBe(true);
    expect(isReturnUrlAllowed('https://pengui.xyz', 'pengui.xyz')).toBe(true);
    expect(
      isReturnUrlAllowed('https://pengui.xyz/back?id=7#top', 'pengui.xyz'),
    ).toBe(true);
  });

  it.each([
    ['a different domain', 'https://evil.example/back'],
    ['a prefix of the domain', 'https://evil-pengui.xyz/back'],
    ['a subdomain of somebody else', 'https://pengui.xyz.evil.example/back'],
    ['a subdomain of the domain', 'https://app.pengui.xyz/back'],
    ['userinfo', 'https://pengui.xyz@evil.example/back'],
    ['a port', 'https://pengui.xyz:8443/back'],
    ['a trailing dot', 'https://pengui.xyz./back'],
    ['plain http', 'http://pengui.xyz/back'],
    ['a javascript url', 'javascript:alert(1)'],
    ['a file url', 'file:///etc/passwd'],
    ['a deep link into our own app', 'https://clapandpay.com/pair#x=1'],
  ])('refuses %s', (_name, url) => {
    expect(isReturnUrlAllowed(url, 'pengui.xyz')).toBe(false);
  });

  it('refuses everything when the session has no verified domain', () => {
    expect(isReturnUrlAllowed('https://pengui.xyz/back', '')).toBe(false);
  });
});
