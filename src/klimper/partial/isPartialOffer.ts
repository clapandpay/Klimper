// SPDX-License-Identifier: AGPL-3.0-or-later
// The one test for a partial offer's text (CHIP-0052): bech32m with the prefix `partial`.
// Every entry point (scan, paste, links) asks this, so they cannot disagree.

const PARTIAL = /^partial1[02-9ac-hj-np-z]+$/;

export function isPartialOffer(text: string): boolean {
  return PARTIAL.test(text.trim().toLowerCase());
}
