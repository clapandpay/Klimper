// SPDX-License-Identifier: AGPL-3.0-or-later
import { i18n } from '@lingui/core';
import { describe, expect, it } from 'vitest';
import { classifyScan } from '../scanTarget';
import { viewErrorText } from './api';
import { partialOfferQrData, QR_MAX_ALPHANUMERIC } from './qrData';

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

const FEATURES = { xchonnect: true, offers: true, partialOffers: true };

describe('partialOfferQrData', () => {
  it('writes the text in capitals, which QR holds in alphanumeric mode', () => {
    const qr = partialOfferQrData(' partial1qqr83wcuu2ry ');
    expect(qr).toBe('PARTIAL1QQR83WCUU2RY');
    // Every character is in QR's alphanumeric set.
    expect(qr).toMatch(/^[0-9A-Z $%*+\-./:]+$/);
  });

  it('refuses a text longer than a QR code holds', () => {
    const long = 'partial1' + 'q'.repeat(QR_MAX_ALPHANUMERIC);
    expect(partialOfferQrData(long)).toBeNull();
  });

  it('is read back by the Scan tab as the same offer', () => {
    const text = 'partial1qqr83wcuu2rykcmqvpsxzgqqemhmlaekcenaz02ma6hs5w6';
    expect(classifyScan(partialOfferQrData(text) ?? '', FEATURES)).toEqual({
      kind: 'offer',
      offer: text,
    });
  });
});

describe('viewErrorText', () => {
  it('says a text with a bad checksum was cut off', () => {
    const text = viewErrorText({
      code: 'not_partial',
      message: 'not a partial offer: bech32 error: invalid checksum',
    });
    expect(text).toMatch(/cut off/);
    expect(text).not.toMatch(/bech32/);
  });

  it('passes other errors through', () => {
    expect(viewErrorText({ code: 'closed', message: 'filled' })).toBe('filled');
    expect(viewErrorText('boom')).toBe('boom');
  });
});
