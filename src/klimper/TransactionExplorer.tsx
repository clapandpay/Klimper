// SPDX-License-Identifier: AGPL-3.0-or-later
// The transaction list's way out to mempoolxch.space, and its rows that open a transaction.
//
// A confirmed wallet transaction has no id of its own: it is every coin of this wallet spent
// and created at one height, so its explorer page is that block. A pending one is a spend
// bundle in the mempool, with its hash as id, so its page is that transaction.

import { NetworkKind } from '@/bindings';
import { t } from '@lingui/core/macro';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ExternalLink } from 'lucide-react';
import { createContext, MouseEvent, ReactNode, useContext } from 'react';
import { mempoolBlockUrl, mempoolTxUrl } from './explorer';

/** Whether the list links to the explorer, and on which network. Null: it does not. */
const ExplorerNetwork = createContext<{ testnet: boolean } | null>(null);

/**
 * Gives the list's rows the network once, rather than each row asking the backend. Only
 * mainnet and testnet11 have an explorer.
 */
export function TransactionExplorerProvider({
  network,
  children,
}: {
  readonly network: NetworkKind | null;
  readonly children: ReactNode;
}) {
  const value =
    network === 'mainnet' || network === 'testnet'
      ? { testnet: network === 'testnet' }
      : null;
  return (
    <ExplorerNetwork.Provider value={value}>
      {children}
    </ExplorerNetwork.Provider>
  );
}

/** The explorer page of a list row's transaction, or null when there is none. */
export function transactionExplorerUrl(
  tx: {
    readonly isPending?: boolean;
    readonly transactionId?: string;
    readonly transactionHeight: number;
  },
  testnet: boolean,
): string | null {
  return tx.isPending
    ? mempoolTxUrl(tx.transactionId, testnet)
    : mempoolBlockUrl(tx.transactionHeight, testnet);
}

/** A small external-link button to the transaction on mempoolxch.space. */
export function TransactionExplorerLink({
  tx,
}: {
  readonly tx: Parameters<typeof transactionExplorerUrl>[0];
}) {
  const network = useContext(ExplorerNetwork);
  const url = network ? transactionExplorerUrl(tx, network.testnet) : null;
  if (url === null) return null;
  const label = t`View on mempoolxch.space`;
  return (
    <button
      type='button'
      className='inline-flex h-6 w-6 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground'
      aria-label={label}
      title={label}
      onClick={(event) => {
        event.stopPropagation();
        void openUrl(url).catch(() => undefined);
      }}
    >
      <ExternalLink className='h-3.5 w-3.5' aria-hidden='true' />
    </button>
  );
}

/**
 * Whether a click on a row should open its transaction: not when it landed on something
 * that acts by itself (a link, a button, a menu, which may sit in a portal but still bubbles
 * through React), and not when it ended a text selection.
 */
export function opensRow(event: MouseEvent<HTMLElement>): boolean {
  const target = event.target as Element | null;
  if (target?.closest('a, button, input, [role="menu"], [role="menuitem"]')) {
    return false;
  }
  return (window.getSelection()?.toString() ?? '') === '';
}
