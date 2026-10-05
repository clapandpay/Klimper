// SPDX-License-Identifier: AGPL-3.0-or-later
// What the Scan tab makes of a code. The rule that must not regress: an Xchonnect code,
// well-formed or not, is never handed back as text to show.

import { describe, expect, it } from 'vitest';
import { classifyScan } from './scanTarget';

const ON = { xchonnect: true, offers: true };
const OFF = { xchonnect: false, offers: false };

const PAIR = 'xchonnect:v1?r=wss%3A%2F%2Frelay.example&s=c2VjcmV0';
const ADDRESS =
  'xch1dtfukqqka3ftqtdlhmc5spc5vd44h7ejrtnjcewxlueam5yrnnqqyczg8t';
const TESTNET =
  'txch19hutewzq3z4l6y3fsw5laatre79tuz5p43jlvag0yz466xx9l7vs4vnpem';

describe('classifyScan', () => {
  it('sends every Xchonnect form to Xchonnect', () => {
    for (const code of [
      PAIR,
      'https://clapandpay.com/pair#r=a&s=b',
      'https://clapandpay.com/req#mbx=abc',
    ]) {
      expect(classifyScan(`  ${code}\n`, ON)).toEqual({ kind: 'xchonnect' });
    }
  });

  it('never echoes an Xchonnect code it cannot use', () => {
    expect(classifyScan(PAIR, OFF)).toEqual({ kind: 'unknown', text: '' });
    expect(classifyScan('xchonnect:v2?s=secret', ON)).toEqual({
      kind: 'unknown',
      text: '',
    });
    expect(classifyScan('XCHONNECT:v1', ON)).toEqual({
      kind: 'unknown',
      text: '',
    });
  });

  it('recognises an address on either network, with or without chia:', () => {
    expect(classifyScan(ADDRESS, OFF)).toEqual({
      kind: 'address',
      address: ADDRESS,
    });
    expect(classifyScan(`chia:${TESTNET}`, OFF)).toEqual({
      kind: 'address',
      address: TESTNET,
    });
  });

  it('does not take a damaged address for one', () => {
    const damaged = `${ADDRESS.slice(0, -1)}q`;
    expect(classifyScan(damaged, ON)).toEqual({
      kind: 'unknown',
      text: damaged,
    });
  });

  it('opens an offer only when offers are built in', () => {
    const offer =
      'offer1qqr83wcuu2rykcmqvpsxzgqqemhmlaekcenaz02ma6hs5w600dhjlvfjn477';
    expect(classifyScan(offer, ON)).toEqual({ kind: 'offer', offer });
    expect(classifyScan(offer, OFF)).toEqual({ kind: 'unknown', text: offer });
  });

  it('shows anything else as it is', () => {
    expect(classifyScan(' https://example.com/x ', ON)).toEqual({
      kind: 'unknown',
      text: 'https://example.com/x',
    });
  });
});
