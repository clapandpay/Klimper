// SPDX-License-Identifier: AGPL-3.0-or-later
// The card account (TASK-151): is the card usable, how much can be spent, what was paid
// where. Read from Gnosis Pay while the screen is open, every few seconds, and never kept.
//
// Freezing is the safe direction and needs no confirmation. Unfreezing asks once. Privacy
// mode hides the amounts here as everywhere else in the wallet.

import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { CreditCard, Loader2, Snowflake, Sun } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import {
  Account,
  card,
  cardErrorMessage,
  CardSummary,
  isCardError,
  Payment,
} from './card';
import { Masked, useMaskAmounts } from '../privacy';
import { AddToWallet } from './AddToWallet';
import { CardDetails } from './CardDetails';
import { CardOnboarding } from './CardOnboarding';

export const ACCOUNT_POLL_MS = 15000;

export function CardAccount({
  onSignedOut,
}: {
  readonly onSignedOut: () => void;
}) {
  const [account, setAccount] = useState<Account | null>(null);
  const [payments, setPayments] = useState<Payment[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [nextAccount, statement] = await Promise.all([
        card.account(),
        card.transactions(),
      ]);
      setAccount(nextAccount);
      setPayments(statement.items);
      setError(null);
    } catch (e) {
      if (isCardError(e) && e.kind === 'signedOut') {
        onSignedOut();
        return;
      }
      setError(cardErrorMessage(e));
    }
  }, [onSignedOut]);

  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), ACCOUNT_POLL_MS);
    return () => clearInterval(timer);
  }, [load]);

  if (account === null) {
    return error ? (
      <div
        className='rounded-lg border border-destructive p-3 text-sm text-destructive'
        role='alert'
      >
        {error}
      </div>
    ) : (
      <div
        className='flex justify-center p-8'
        data-testid='card-account-loading'
      >
        <Loader2
          className='h-6 w-6 animate-spin text-muted-foreground'
          aria-hidden='true'
        />
      </div>
    );
  }

  const usable = account.cards.filter((c) => c.status !== 'canceled');

  return (
    <div className='space-y-4' data-testid='card-account'>
      {error && (
        <div
          className='rounded-lg border border-destructive p-3 text-sm text-destructive'
          role='alert'
        >
          {error}
        </div>
      )}
      <Balance account={account} />
      {usable.length === 0 ? (
        <CardOnboarding step={{ step: 'completed' }} onChanged={load} />
      ) : (
        usable.map((c) => (
          <PaymentCard key={c.id} summary={c} onChanged={load} />
        ))
      )}
      <Payments payments={payments ?? []} currency={account.currency} />
    </div>
  );
}

function money(amount: string, currency: string) {
  const value = Number(amount);
  if (!Number.isFinite(value)) return `${amount} ${currency}`;
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency,
    }).format(value);
  } catch {
    return `${amount} ${currency}`;
  }
}

function Balance({ account }: { readonly account: Account }) {
  const pending = Number(account.pending) > 0;
  const pendingText = money(account.pending, account.currency);
  const mask = useMaskAmounts();
  return (
    <Card>
      <CardHeader>
        <CardDescription>
          <Trans>You can spend</Trans>
        </CardDescription>
        <CardTitle className='text-3xl' data-testid='card-spendable'>
          {mask ? <Masked /> : money(account.spendable, account.currency)}
        </CardTitle>
        {pending && !mask && (
          <div
            className='text-sm text-muted-foreground'
            data-testid='card-pending'
          >
            <Trans>On the way: {pendingText}</Trans>
          </div>
        )}
      </CardHeader>
      {account.safeAddress && (
        <CardContent className='space-y-1'>
          <div className='text-xs text-muted-foreground'>
            <Trans>Top up: send EURe on Gnosis Chain to the card account</Trans>
          </div>
          <div
            className='break-all font-mono text-xs'
            data-testid='card-safe-address'
          >
            {account.safeAddress}
          </div>
        </CardContent>
      )}
    </Card>
  );
}

function PaymentCard({
  summary,
  onChanged,
}: {
  readonly summary: CardSummary;
  readonly onChanged: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const frozen = summary.status === 'blocked';

  const setFrozen = async (next: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await card.setFrozen(summary.id, next);
      await onChanged();
    } catch (e) {
      setError(cardErrorMessage(e));
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  };

  const label =
    summary.status === 'active'
      ? t`Ready to pay`
      : frozen
        ? t`Frozen`
        : summary.status === 'provisioning'
          ? t`Being made`
          : summary.status;

  return (
    <Card data-testid='card-card'>
      <CardHeader>
        <div className='flex items-center gap-3'>
          <CreditCard className='h-6 w-6 shrink-0' aria-hidden='true' />
          <div className='min-w-0 flex-1'>
            <CardTitle className='text-base'>
              {summary.brand ?? 'Visa'} ••{' '}
              <span data-testid='card-last4'>{summary.last4 ?? '····'}</span>
            </CardTitle>
            <CardDescription data-testid='card-status'>
              {label}
              {summary.expires ? ` · ${summary.expires}` : ''}
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className='space-y-2'>
        {error && (
          <div className='text-sm text-destructive' role='alert'>
            {error}
          </div>
        )}
        {frozen ? (
          confirming ? (
            <div className='flex flex-wrap items-center gap-2'>
              <span className='text-sm'>
                <Trans>Unfreeze? The card can pay again at once.</Trans>
              </span>
              <Button
                size='sm'
                disabled={busy}
                onClick={() => setFrozen(false)}
                data-testid='card-unfreeze-confirm'
              >
                <Trans>Unfreeze</Trans>
              </Button>
              <Button
                size='sm'
                variant='ghost'
                onClick={() => setConfirming(false)}
              >
                <Trans>Keep frozen</Trans>
              </Button>
            </div>
          ) : (
            <Button
              size='sm'
              variant='outline'
              disabled={busy}
              onClick={() => setConfirming(true)}
              data-testid='card-unfreeze'
            >
              <Sun className='mr-2 h-4 w-4' aria-hidden='true' />
              <Trans>Unfreeze the card</Trans>
            </Button>
          )
        ) : (
          summary.status === 'active' && (
            <Button
              size='sm'
              variant='outline'
              disabled={busy}
              onClick={() => setFrozen(true)}
              data-testid='card-freeze'
            >
              <Snowflake className='mr-2 h-4 w-4' aria-hidden='true' />
              <Trans>Freeze the card</Trans>
            </Button>
          )
        )}
        {summary.status === 'active' && (
          <CardDetails cardId={summary.id}>
            {(details) => <AddToWallet details={details} />}
          </CardDetails>
        )}
      </CardContent>
    </Card>
  );
}

function Payments({
  payments,
  currency,
}: {
  readonly payments: Payment[];
  readonly currency: string;
}) {
  const mask = useMaskAmounts();
  return (
    <Card>
      <CardHeader>
        <CardTitle className='text-base'>
          <Trans>Payments</Trans>
        </CardTitle>
      </CardHeader>
      <CardContent>
        {payments.length === 0 ? (
          <p className='text-sm text-muted-foreground'>
            <Trans>No payments yet.</Trans>
          </p>
        ) : (
          <ul className='divide-y' data-testid='card-payments'>
            {payments.map((p) => (
              <li
                key={p.id}
                className='flex items-center justify-between gap-3 py-2'
              >
                <div className='min-w-0'>
                  <div className='truncate text-sm font-medium'>
                    {p.merchant ?? p.kind}
                  </div>
                  <div className='text-xs text-muted-foreground'>
                    {p.createdAt ? new Date(p.createdAt).toLocaleString() : ''}
                    {p.status !== 'settled'
                      ? ` · ${statusLabel(p.status)}`
                      : ''}
                  </div>
                </div>
                <div
                  className={`shrink-0 text-sm tabular-nums ${p.status === 'declined' ? 'text-muted-foreground line-through' : ''}`}
                >
                  {mask ? (
                    <Masked />
                  ) : p.amount ? (
                    money(p.amount, p.currency ?? currency)
                  ) : (
                    ''
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function statusLabel(status: string) {
  switch (status) {
    case 'pending':
      return t`pending`;
    case 'declined':
      return t`declined`;
    case 'cancelled':
      return t`cancelled`;
    default:
      return status;
  }
}
