// SPDX-License-Identifier: AGPL-3.0-or-later
// A partial offer's text as it goes into a QR code (TASK-164).
//
// A `partial1…` is a couple of kilobytes. Messengers cut texts that long (Telegram splits at
// 4,096 characters), and a cut text fails with "invalid checksum"; a QR code carries it
// whole. bech32 may be written in capitals, and capitals fit QR's alphanumeric mode, which
// holds 4,296 characters at the lowest error correction: enough for a partial offer. The Scan
// tab lowercases it again (`scanTarget.ts`).

/** The most alphanumeric characters a QR code holds (version 40, level L). */
export const QR_MAX_ALPHANUMERIC = 4296;

/** The text as it goes into the QR code, or null if it is too long for one. */
export function partialOfferQrData(offer: string): string | null {
  const data = offer.trim().toUpperCase();
  return data.length <= QR_MAX_ALPHANUMERIC ? data : null;
}
