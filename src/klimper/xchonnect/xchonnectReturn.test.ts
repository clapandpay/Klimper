// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper must not be usable as a redirect (clapandpay TASK-085, spec threats T3/T18).

import { describe, expect, it, vi } from 'vitest';
import { returnToDapp } from './xchonnectReturn';

describe('returnToDapp', () => {
  it('opens a return URL on the session domain', async () => {
    const open = vi.fn().mockResolvedValue(undefined);
    await expect(
      returnToDapp('https://pengui.xyz/back', 'pengui.xyz', open),
    ).resolves.toBe('opened');
    expect(open).toHaveBeenCalledWith('https://pengui.xyz/back');
  });

  it('opens nothing at all when the URL leaves the domain', async () => {
    const open = vi.fn().mockResolvedValue(undefined);
    await expect(
      returnToDapp('https://evil.example/back', 'pengui.xyz', open),
    ).resolves.toBe('refused');
    expect(open).not.toHaveBeenCalled();
  });

  it('opens nothing when there is no return URL', async () => {
    const open = vi.fn().mockResolvedValue(undefined);
    await expect(returnToDapp(null, 'pengui.xyz', open)).resolves.toBe(
      'refused',
    );
    await expect(returnToDapp(undefined, 'pengui.xyz', open)).resolves.toBe(
      'refused',
    );
    expect(open).not.toHaveBeenCalled();
  });

  it('reports a failure to open rather than throwing at the screen', async () => {
    const open = vi.fn().mockRejectedValue(new Error('no browser'));
    await expect(
      returnToDapp('https://pengui.xyz/back', 'pengui.xyz', open),
    ).resolves.toBe('failed');
  });
});
