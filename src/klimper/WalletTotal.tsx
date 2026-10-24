// SPDX-License-Identifier: AGPL-3.0-or-later
// The wallet's total, on top of the token list: one large figure in the display face, what it
// is in XCH beneath it, and "an estimate" behind an info dot. Masked in privacy mode like every
// amount (NumberFormat). One element in TokenList.tsx.
//
// With the card (TASK-158) the figure is the wallet and the card together, and two lines
// beneath split it: what is in the wallet, and what the card can spend, with top-ups still on
// the way and a link to top up. The card's euros count in the total at the current rate; with
// no rate the total stays the wallet's and the card line shows its own currency.

import { NumberFormat } from '@/components/NumberFormat';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { ChevronRight, CreditCard, Snowflake, Wallet } from 'lucide-react';
import { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { CARD_ROUTE } from './card/card';
import { CardBalance, cardInUsd, useCardBalance } from './card/useCardBalance';
import { lastPrices } from './price';
import { useMaskAmounts } from './privacy';
import { Hint } from './Hint';

export function WalletTotal({
  usd,
  hasTokens = true,
}: {
  readonly usd: number;
  /** The wallet has priced tokens; without them the box shows only for a ready card. */
  readonly hasTokens?: boolean;
}) {
  const balance = useCardBalance();
  return <WalletTotalView usd={usd} hasTokens={hasTokens} card={balance} />;
}

export function WalletTotalView({
  usd,
  hasTokens,
  card,
}: {
  readonly usd: number;
  readonly hasTokens: boolean;
  readonly card: CardBalance;
}) {
  const masked = useMaskAmounts();
  const prices = lastPrices();
  const xchUsd = prices?.xchUsd ?? 0;
  const cardUsd =
    card.state === 'ready'
      ? cardInUsd(card.spendable, card.currency, prices?.eurUsd ?? null)
      : null;
  if (!hasTokens && card.state !== 'ready') return null;

  const total = usd + (cardUsd ?? 0);
  const inXch = xchUsd > 0 ? total / xchUsd : null;
  const withCard = card.state !== 'none';

  return (
    <section className='klimper-total' data-testid='wallet-total'>
      <div className='flex items-center gap-1 text-sm font-medium opacity-80'>
        <Trans>Total balance</Trans>
        <Hint>
          {cardUsd !== null &&
          card.state === 'ready' &&
          card.currency !== 'USD' ? (
            <Trans>
              An estimate from current prices, with the card counted at the
              current euro rate. It does not account for liquidity.
            </Trans>
          ) : (
            <Trans>
              An estimate from current prices. It does not account for
              liquidity.
            </Trans>
          )}
        </Hint>
      </div>
      <div className='klimper-total-amount' data-testid='wallet-total-amount'>
        <NumberFormat
          value={total}
          style='currency'
          currency='USD'
          maximumFractionDigits={2}
        />
      </div>
      {inXch !== null && !masked && (
        <div className='text-sm opacity-80 tabular-nums'>
          ≈ <NumberFormat value={inXch} maximumFractionDigits={4} /> XCH
        </div>
      )}
      {withCard && (
        <div className='klimper-total-split'>
          <Row
            icon={<Wallet className='h-4 w-4' aria-hidden='true' />}
            label={<Trans>In the wallet</Trans>}
            testId='wallet-total-wallet'
          >
            <NumberFormat
              value={usd}
              style='currency'
              currency='USD'
              maximumFractionDigits={2}
            />
          </Row>
          <CardRow card={card} />
        </div>
      )}
    </section>
  );
}

function Row({
  icon,
  label,
  note,
  children,
  testId,
}: {
  readonly icon: ReactNode;
  readonly label: ReactNode;
  readonly note?: ReactNode;
  readonly children?: ReactNode;
  readonly testId: string;
}) {
  return (
    <div className='klimper-total-row' data-testid={testId}>
      <span className='klimper-total-row-icon'>{icon}</span>
      <span className='min-w-0 flex-1'>
        <span className='block truncate text-sm font-medium'>{label}</span>
        {note && (
          <span className='block truncate text-xs opacity-80'>{note}</span>
        )}
      </span>
      <span className='shrink-0 text-right text-sm font-semibold tabular-nums'>
        {children}
      </span>
    </div>
  );
}

function CardRow({ card }: { readonly card: CardBalance }) {
  const masked = useMaskAmounts();
  if (card.state === 'none') return null;
  const icon =
    card.state === 'ready' && card.frozen ? (
      <Snowflake className='h-4 w-4' aria-hidden='true' />
    ) : (
      <CreditCard className='h-4 w-4' aria-hidden='true' />
    );

  if (card.state !== 'ready') {
    return (
      <Link
        to={CARD_ROUTE}
        className='klimper-total-row klimper-total-link'
        data-testid='wallet-total-card'
      >
        <span className='klimper-total-row-icon'>{icon}</span>
        <span className='min-w-0 flex-1'>
          <span className='block truncate text-sm font-medium'>
            <Trans>Card</Trans>
          </span>
          <span className='block truncate text-xs opacity-80'>
            {card.state === 'signedOut' ? (
              <Trans>Sign in to see what you can spend</Trans>
            ) : (
              <Trans>Finish setting up the card</Trans>
            )}
          </span>
        </span>
        <ChevronRight className='h-4 w-4 shrink-0' aria-hidden='true' />
      </Link>
    );
  }

  const last4 = card.last4 ?? '····';
  const pending = card.pending > 0 && !masked;
  return (
    <div className='space-y-2'>
      <Row
        icon={icon}
        label={
          card.frozen ? (
            <Trans>Card ••{last4} · frozen</Trans>
          ) : (
            <Trans>Card ••{last4} · you can spend</Trans>
          )
        }
        note={
          pending ? (
            <span data-testid='wallet-total-card-pending'>
              <Trans>
                Top-up on the way:{' '}
                <NumberFormat
                  value={card.pending}
                  style='currency'
                  currency={card.currency}
                  maximumFractionDigits={2}
                />
              </Trans>
            </span>
          ) : undefined
        }
        testId='wallet-total-card'
      >
        <NumberFormat
          value={card.spendable}
          style='currency'
          currency={card.currency}
          maximumFractionDigits={2}
        />
      </Row>
      <Link
        to={CARD_ROUTE}
        className='klimper-total-action'
        data-testid='wallet-total-top-up'
        aria-label={t`Top up the card`}
      >
        <Trans>Top up</Trans>
        <ChevronRight className='h-4 w-4' aria-hidden='true' />
      </Link>
    </div>
  );
}
