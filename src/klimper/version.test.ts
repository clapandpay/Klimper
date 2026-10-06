// SPDX-License-Identifier: AGPL-3.0-or-later
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getVersion, KLIMPER_VERSION } from './version';

const sageVersion = vi.hoisted(() => vi.fn<() => Promise<string>>());
vi.mock('@tauri-apps/api/app', () => ({ getVersion: sageVersion }));

describe('the version Settings shows', () => {
  beforeEach(() => {
    sageVersion.mockReset();
  });

  it("is Klimper's own, with the Sage release the app reports", async () => {
    sageVersion.mockResolvedValue('0.13.1');
    expect(await getVersion()).toBe(`${KLIMPER_VERSION} (Sage 0.13.1)`);
    expect(KLIMPER_VERSION).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("is Klimper's alone when the app does not answer", async () => {
    sageVersion.mockRejectedValue(new Error('no ipc'));
    expect(await getVersion()).toBe(KLIMPER_VERSION);
  });
});
