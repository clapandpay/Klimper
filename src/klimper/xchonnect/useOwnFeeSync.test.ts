// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it, vi } from 'vitest';

vi.mock('./xchonnect', () => ({ xchonnect: {} }));

const { feeMojos } = await import('./useOwnFeeSync');

describe('the default fee in mojos', () => {
  it('is a whole number of mojos', () => {
    expect(feeMojos('0')).toBe('0');
    expect(feeMojos('')).toBe('0');
    expect(feeMojos('0.00005')).toBe('50000000');
    expect(feeMojos('1')).toBe('1000000000000');
    expect(feeMojos('0.0000000000001')).toBe('0');
  });

  it('is nothing for what is not a fee', () => {
    expect(feeMojos('-1')).toBeNull();
    expect(feeMojos('abc')).toBeNull();
  });
});
