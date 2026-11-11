// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The transaction list's explorer links and clickable rows.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const openUrl = vi.fn((_url: string) => Promise.resolve());
vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: (url: string) => openUrl(url),
}));

// Activates the locale `t` needs.
await import('./testHarness');

const {
  opensRow,
  TransactionExplorerLink,
  TransactionExplorerProvider,
  transactionExplorerUrl,
} = await import('./TransactionExplorer');

const ID = 'ab'.repeat(32);

afterEach(() => {
  cleanup();
  openUrl.mockClear();
});

describe('transactionExplorerUrl', () => {
  it('opens a confirmed transaction at its block', () => {
    expect(
      transactionExplorerUrl({ transactionHeight: 7_000_000 }, false),
    ).toBe('https://mempoolxch.space/block/7000000');
  });

  it('opens a pending one by its spend bundle id, on the right network', () => {
    expect(
      transactionExplorerUrl(
        { isPending: true, transactionId: ID, transactionHeight: 0 },
        true,
      ),
    ).toBe(`https://mempoolxch.space/tx/0x${ID}?network=testnet11`);
  });
});

describe('TransactionExplorerLink', () => {
  it('opens the explorer without also opening the row', () => {
    const row = vi.fn();
    render(
      <TransactionExplorerProvider network='mainnet'>
        <div onClick={row}>
          <TransactionExplorerLink tx={{ transactionHeight: 42 }} />
        </div>
      </TransactionExplorerProvider>,
    );
    fireEvent.click(
      screen.getByRole('button', { name: 'View on mempoolxch.space' }),
    );
    expect(openUrl).toHaveBeenCalledWith('https://mempoolxch.space/block/42');
    expect(row).not.toHaveBeenCalled();
  });

  it.each(['unknown', null] as const)(
    'shows nothing on a network without an explorer (%s)',
    (network) => {
      render(
        <TransactionExplorerProvider network={network}>
          <TransactionExplorerLink tx={{ transactionHeight: 42 }} />
        </TransactionExplorerProvider>,
      );
      expect(screen.queryByRole('button')).toBeNull();
    },
  );
});

describe('opensRow', () => {
  function clickOn(markup: string, selector: string): boolean {
    let result: boolean | null = null;
    const { container } = render(
      <table>
        <tbody>
          <tr
            onClick={(event) => {
              result = opensRow(event);
            }}
            // eslint-disable-next-line react/no-danger
            dangerouslySetInnerHTML={{ __html: markup }}
          />
        </tbody>
      </table>,
    );
    const target = container.querySelector(selector);
    if (target === null) throw new Error(`nothing matches ${selector}`);
    fireEvent.click(target);
    if (result === null) throw new Error('the row saw no click');
    return result;
  }

  it('opens on a click on plain text', () => {
    expect(clickOn('<td><span>1 XCH</span></td>', 'span')).toBe(true);
  });

  it('leaves links, buttons and menus to themselves', () => {
    expect(clickOn('<td><a href="#">XCH</a></td>', 'a')).toBe(false);
    expect(clickOn('<td><button><svg></svg></button></td>', 'svg')).toBe(false);
    expect(clickOn('<td><div role="menuitem">Copy</div></td>', 'div')).toBe(
      false,
    );
  });
});
