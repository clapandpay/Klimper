// SPDX-License-Identifier: AGPL-3.0-or-later
// The address a coin in a transaction belongs to, and whose it is: the wallet's own, a burn,
// an offer's settlement, a launcher, or someone else's. Sage knows all of this per coin
// (`address_kind`) and its transaction page showed none of it.

import { AddressKind, TransactionCoinRecord } from '@/bindings';
import { t } from '@lingui/core/macro';
import { AddressText } from './AddressText';

function kindLabel(kind: AddressKind): string | null {
  switch (kind) {
    case 'own':
      return t`Yours`;
    case 'burn':
      return t`Burned`;
    case 'offer':
      return t`Offer`;
    case 'launcher':
      return t`Launcher`;
    case 'external':
      return t`Other`;
    default:
      return null;
  }
}

export function CoinAddress({
  coin,
}: {
  readonly coin: TransactionCoinRecord;
}) {
  if (!coin.address) return null;
  return (
    <div
      className='mt-3 flex min-w-0 items-center border-t border-border pt-3'
      data-testid='coin-address'
    >
      <AddressText
        address={coin.address}
        badge={kindLabel(coin.address_kind) ?? undefined}
      />
    </div>
  );
}
