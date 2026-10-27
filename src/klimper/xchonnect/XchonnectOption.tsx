// SPDX-License-Identifier: AGPL-3.0-or-later
// The option card at the top of an approval (clapandpay TASK-140): what an option contract
// locks and until when, what its holder may pay for the collateral and to whom, and who gets
// it back after the expiry; and the same facts worded as a loan, for a loan request and for
// lending.
//
// Every figure is the wallet's: `summary.options` is wallet-kit's own reading of the spends
// (spec 11.1.1), and `loan` is Klimper's own key, about a request it built itself. Both reach
// this file only through `readPrompt`. Locked collateral is said to be locked, under the
// option, and never shown as a payment to anyone: it is not one (it comes back to whoever the
// option says, on the option's terms).

import { AddressText } from '@/klimper/AddressText';
import { toAddress } from '@/lib/utils';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import {
  ArrowDown,
  CalendarClock,
  FileSignature,
  HandCoins,
  Lock,
  Send,
  Undo2,
  User,
} from 'lucide-react';
import { AssetFacts, AssetRef, factsOf } from './useApprovalAssets';
import { groupedAmount, unitOf } from './XchonnectTrade';
import {
  optionLabel,
  LoanView,
  OfferView,
  OptionEffectView,
  PartialTermsView,
  SpendPromptView,
  TradeLegView,
} from './xchonnectPrompt';

type Facts = ReadonlyMap<string, AssetFacts>;

/** An amount with its unit: `100 XCH`, `500 wUSDC.b 3c3c3c3c`, `500 CAT 1a2b3c4d`. */
export function amountText(
  leg: TradeLegView,
  facts: Facts,
  xchLabel: string,
): string {
  // A few mojos read as what they are, not as twelve decimals of XCH.
  if (leg.isXch && leg.amount.isLessThan(1_000_000)) {
    const mojos = leg.amount.toFixed(0);
    return leg.amount.isEqualTo(1) ? t`1 mojo` : t`${mojos} mojos`;
  }
  return `${groupedAmount(leg.amount, leg.precision)} ${unitOf(
    leg,
    factsOf(facts, leg),
    xchLabel,
  )}`;
}

const MONTH = new Intl.DateTimeFormat(undefined, { month: 'short' });

/** `13 Nov 14:00`, the year added when it is not this one. Local time, as the phone shows it. */
export function whenText(seconds: number, withTime = true): string {
  const date = new Date(seconds * 1000);
  const day = `${date.getDate()} ${MONTH.format(date)}`;
  const year =
    date.getFullYear() === new Date().getFullYear()
      ? ''
      : ` ${date.getFullYear()}`;
  if (!withTime) return `${day}${year}`;
  const time = `${String(date.getHours()).padStart(2, '0')}:${String(
    date.getMinutes(),
  ).padStart(2, '0')}`;
  return `${day}${year} ${time}`;
}

function Addr({
  puzzleHash,
  prefix,
}: {
  readonly puzzleHash: string;
  readonly prefix: string;
}) {
  let address: string;
  try {
    address = toAddress(puzzleHash, prefix);
  } catch {
    address = `0x${puzzleHash}`;
  }
  return <AddressText address={address} copy={false} testId='option-address' />;
}

function Line({
  icon,
  children,
  testId,
}: {
  readonly icon: React.ReactNode;
  readonly children: React.ReactNode;
  readonly testId?: string;
}) {
  return (
    <li className='xc-option-line' data-testid={testId ?? 'option-line'}>
      {icon}
      <span>{children}</span>
    </li>
  );
}

const ICON = 'h-4 w-4';

function OptionId({ launcherId }: { readonly launcherId: string | null }) {
  return (
    <div className='xc-trade-facts'>
      <div className='xc-trade-row'>
        <span className='text-muted-foreground'>
          <Trans>Option</Trans>
        </span>
        <span
          className='xc-leg-id'
          title={launcherId ?? undefined}
          data-testid='option-id'
        >
          {launcherId === null ? (
            <Trans>not named by this request</Trans>
          ) : (
            optionLabel(launcherId)
          )}
        </span>
      </div>
    </div>
  );
}

/** One option effect as a card. */
function EffectCard({
  effect,
  facts,
  xchLabel,
}: {
  readonly effect: OptionEffectView;
  readonly facts: Facts;
  readonly xchLabel: string;
}) {
  const prefix = xchLabel.toLowerCase();
  const amount = (leg: TradeLegView) => (
    <strong>{amountText(leg, facts, xchLabel)}</strong>
  );

  if (effect.action === 'mint') {
    const { terms } = effect;
    const until = whenText(terms.expiresAt);
    const collateral = amount(terms.collateral);
    const strike = amount(terms.strike);
    const creator = <Addr puzzleHash={terms.creator} prefix={prefix} />;
    const writes = effect.creatorIsUser && effect.fundedByUser;
    return (
      <div
        className='xc-trade xc-option'
        data-testid='prompt-option'
        data-action='mint'
      >
        <div className='xc-option-title'>
          <FileSignature className={ICON} aria-hidden='true' />
          {writes ? <Trans>Write option</Trans> : <Trans>New option</Trans>}
        </div>
        <ul className='xc-option-lines'>
          <Line icon={<Lock className={ICON} aria-hidden='true' />}>
            {effect.fundedByUser ? (
              <Trans>
                Lock {collateral} until {until}
              </Trans>
            ) : (
              <Trans>
                {collateral} locked until {until}
              </Trans>
            )}
          </Line>
          <Line icon={<HandCoins className={ICON} aria-hidden='true' />}>
            {effect.creatorIsUser ? (
              <Trans>
                Whoever holds it can buy it for {strike}, paid to you
              </Trans>
            ) : (
              <Trans>
                Whoever holds it can buy it for {strike}, paid to {creator}
              </Trans>
            )}
          </Line>
          <Line icon={<Undo2 className={ICON} aria-hidden='true' />}>
            {effect.creatorIsUser ? (
              <Trans>After that you can claim it back</Trans>
            ) : (
              <Trans>After that {creator} can claim it back</Trans>
            )}
          </Line>
          <Line
            icon={<User className={ICON} aria-hidden='true' />}
            testId='option-holder'
          >
            {effect.ownerIsUser ? (
              <Trans>You hold the option</Trans>
            ) : (
              <Trans>
                The option goes to{' '}
                <Addr puzzleHash={effect.owner} prefix={prefix} />
              </Trans>
            )}
          </Line>
        </ul>
        {!effect.guaranteed && (
          <div className='xc-option-warn' data-testid='option-conditional'>
            <Trans>
              The new option is only conditional: spends you do not sign decide
              whether it is made.
            </Trans>
          </div>
        )}
        <OptionId launcherId={effect.launcherId} />
      </div>
    );
  }

  if (effect.action === 'exercise') {
    const { terms } = effect;
    const strike = amount(terms.strike);
    const collateral = amount(terms.collateral);
    return (
      <div
        className='xc-trade xc-option'
        data-testid='prompt-option'
        data-action='exercise'
      >
        <div className='xc-option-title'>
          <HandCoins className={ICON} aria-hidden='true' />
          {effect.byUser ? (
            <Trans>Use option</Trans>
          ) : (
            <Trans>Option used</Trans>
          )}
        </div>
        <div className='xc-option-lead' data-testid='option-lead'>
          {effect.byUser ? (
            <Trans>
              Pay {strike}, receive {collateral}
            </Trans>
          ) : (
            <Trans>
              {strike} paid for {collateral}
            </Trans>
          )}
        </div>
        <ul className='xc-option-lines'>
          <Line icon={<Send className={ICON} aria-hidden='true' />}>
            {effect.creatorIsUser ? (
              <Trans>Paid to you, the option&apos;s writer</Trans>
            ) : (
              <Trans>
                Paid to the option&apos;s writer,{' '}
                <Addr puzzleHash={terms.creator} prefix={prefix} />
              </Trans>
            )}
          </Line>
        </ul>
        <OptionId launcherId={effect.launcherId} />
      </div>
    );
  }

  if (effect.action === 'clawback') {
    const collateral = amount(effect.collateral);
    const date = whenText(effect.unlocksAt, false);
    return (
      <div
        className='xc-trade xc-option'
        data-testid='prompt-option'
        data-action='clawback'
      >
        <div className='xc-option-title'>
          <Undo2 className={ICON} aria-hidden='true' />
          <Trans>Claim back</Trans>
        </div>
        <div className='xc-option-lead' data-testid='option-lead'>
          {effect.creatorIsUser ? (
            <Trans>
              Claim back {collateral} (option expired {date})
            </Trans>
          ) : (
            <Trans>
              {collateral} claimed back by{' '}
              <Addr puzzleHash={effect.creator} prefix={prefix} /> (option
              expired {date})
            </Trans>
          )}
        </div>
        <OptionId launcherId={effect.launcherId} />
      </div>
    );
  }

  // transfer
  const label = optionLabel(effect.launcherId);
  return (
    <div
      className='xc-trade xc-option'
      data-testid='prompt-option'
      data-action='transfer'
    >
      <div className='xc-option-title'>
        <Send className={ICON} aria-hidden='true' />
        {effect.fromUser ? (
          <Trans>Send option</Trans>
        ) : (
          <Trans>Receive option</Trans>
        )}
      </div>
      <div className='xc-option-lead' data-testid='option-lead'>
        {effect.toUser ? (
          <Trans>{label} to your own address</Trans>
        ) : (
          <Trans>
            {label} to <Addr puzzleHash={effect.to} prefix={prefix} />
          </Trans>
        )}
      </div>
      <p className='xc-sub mt-2'>
        <Trans>
          Whoever holds an option may use it. Its terms are not part of this
          request.
        </Trans>
      </p>
    </div>
  );
}

/** A loan request (borrower) or a loan (lender), from Klimper's `loan` key and its mint. */
function LoanCard({
  loan,
  makesOffer,
  facts,
  xchLabel,
}: {
  readonly loan: LoanView;
  readonly makesOffer: boolean;
  readonly facts: Facts;
  readonly xchLabel: string;
}) {
  const prefix = xchLabel.toLowerCase();
  const amount = (leg: TradeLegView) => (
    <strong>{amountText(leg, facts, xchLabel)}</strong>
  );
  const { terms } = loan.mint;
  const principal = amount(loan.principal);
  const collateral = amount(terms.collateral);
  const repay = amount(terms.strike);
  const until = whenText(terms.expiresAt);
  const other = <Addr puzzleHash={loan.counterparty} prefix={prefix} />;
  const tie = loan.tie === null ? null : amountText(loan.tie, facts, xchLabel);
  const borrower = loan.role === 'borrower';
  return (
    <div
      className='xc-trade xc-option'
      data-testid='prompt-loan'
      data-role={loan.role}
    >
      <div className='xc-option-title'>
        <HandCoins className={ICON} aria-hidden='true' />
        {borrower ? <Trans>Request a loan</Trans> : <Trans>Lend</Trans>}
      </div>
      <div className='xc-option-lead' data-testid='loan-lead'>
        {borrower ? (
          <Trans>
            Borrow {principal} from {other}
          </Trans>
        ) : (
          <Trans>
            Lend {principal} to {other}
          </Trans>
        )}
      </div>
      <div className='xc-trade-turn' aria-hidden='true'>
        <ArrowDown className='h-4 w-4' />
      </div>
      <ul className='xc-option-lines' data-testid='loan-terms'>
        <Line icon={<Lock className={ICON} aria-hidden='true' />}>
          {borrower ? (
            <Trans>
              Lock {collateral} until {until}
            </Trans>
          ) : (
            <Trans>
              They lock {collateral} until {until}
            </Trans>
          )}
        </Line>
        <Line icon={<CalendarClock className={ICON} aria-hidden='true' />}>
          {borrower ? (
            <Trans>Repay {repay} to get it back</Trans>
          ) : (
            <Trans>If they repay {repay} by then, they get it back</Trans>
          )}
        </Line>
        <Line icon={<Undo2 className={ICON} aria-hidden='true' />}>
          {borrower ? (
            <Trans>Otherwise the lender keeps it</Trans>
          ) : (
            <Trans>Otherwise you can claim it</Trans>
          )}
        </Line>
      </ul>
      {!loan.mint.guaranteed && borrower && (
        <div className='xc-option-warn' data-testid='option-conditional'>
          <Trans>
            The option to repay is only conditional: spends you do not sign
            decide whether it is made.
          </Trans>
        </div>
      )}
      {makesOffer && (
        <div className='xc-option-note' data-testid='loan-offer-note'>
          <Trans>
            Klimper signs a loan request. Nothing moves until a lender takes it.
          </Trans>
        </div>
      )}
      <div className='xc-trade-facts'>
        {tie !== null && (
          <div className='xc-trade-row' data-testid='loan-tie'>
            <span className='text-muted-foreground'>
              {borrower ? (
                <Trans>{tie} to the lender, to tie the loan together</Trans>
              ) : (
                <Trans>
                  {tie} from the borrower ties your payment to their lock
                </Trans>
              )}
            </span>
          </div>
        )}
        <div className='xc-trade-row'>
          <span className='text-muted-foreground'>
            <Trans>Option</Trans>
          </span>
          <span
            className='xc-leg-id'
            title={loan.mint.launcherId}
            data-testid='option-id'
          >
            {optionLabel(loan.mint.launcherId)}
          </span>
        </div>
      </div>
    </div>
  );
}

/**
 * An offer the wallet makes (`chia_createOffer`, TASK-142), from Klimper's `offer` key: each
 * option it gives with what it locks, its strike and expiry, each other asset it gives, and
 * what whoever takes it pays. Nothing is sent: Klimper signs the offer.
 */
function OfferCard({
  offer,
  facts,
  xchLabel,
}: {
  readonly offer: OfferView;
  readonly facts: Facts;
  readonly xchLabel: string;
}) {
  const prefix = xchLabel.toLowerCase();
  const closes = offer.expiresAt === null ? '' : whenText(offer.expiresAt);
  const amount = (leg: TradeLegView) => (
    <strong>{amountText(leg, facts, xchLabel)}</strong>
  );
  if (offer.partial !== null) {
    return (
      <PartialOfferCard
        offer={offer}
        partial={offer.partial}
        amount={amount}
        closes={closes}
      />
    );
  }
  return (
    <div className='xc-trade xc-option' data-testid='prompt-offer'>
      <div className='xc-option-title'>
        <FileSignature className={ICON} aria-hidden='true' />
        <Trans>Make an offer</Trans>
      </div>
      <div className='xc-option-lead'>
        <Trans>You give</Trans>
      </div>
      <ul className='xc-option-lines' data-testid='offer-gives'>
        {offer.options.map((option) => {
          const collateral = amount(option.terms.collateral);
          const strike = amount(option.terms.strike);
          const until = whenText(option.terms.expiresAt);
          const label = optionLabel(option.launcherId);
          const creator = (
            <Addr puzzleHash={option.terms.creator} prefix={prefix} />
          );
          return (
            <li
              key={option.launcherId}
              className='xc-option-line'
              data-testid='offer-option'
            >
              <Lock className={ICON} aria-hidden='true' />
              <span>
                <Trans>
                  Option {label}: whoever takes the offer may pay {strike} to{' '}
                  {creator} for {collateral} until {until}. After that,{' '}
                  {creator} takes the {collateral} back.
                </Trans>
              </span>
            </li>
          );
        })}
        {offer.gives.map((leg) => (
          <Line
            key={leg.label + leg.amount.toFixed(0)}
            icon={<Send className={ICON} aria-hidden='true' />}
            testId='offer-give'
          >
            {amount(leg)}
          </Line>
        ))}
      </ul>
      <div className='xc-trade-turn' aria-hidden='true'>
        <ArrowDown className='h-4 w-4' />
      </div>
      <div className='xc-option-lead'>
        <Trans>You ask for</Trans>
      </div>
      <ul className='xc-option-lines' data-testid='offer-asks'>
        {offer.asks.map((leg) => (
          <Line
            key={leg.label + leg.amount.toFixed(0)}
            icon={<HandCoins className={ICON} aria-hidden='true' />}
            testId='offer-ask'
          >
            {amount(leg)}
          </Line>
        ))}
      </ul>
      <div className='xc-option-note' data-testid='offer-note'>
        <Trans>
          Klimper signs an offer. Nothing moves until someone takes it.
        </Trans>
        {offer.expiresAt !== null && (
          <>
            {' '}
            <Trans>
              Nobody can take it from {closes}, when the option expires.
            </Trans>
          </>
        )}
      </div>
    </div>
  );
}

/**
 * A partial offer the wallet makes (`chia_createPartialOffer`, TASK-165), from Klimper's
 * `offer` key: what it gives for what it asks, that takers may fill any part from the least
 * fill up, the fee each fill pays, and the expiry. Nothing is sent: Klimper signs it.
 */
function PartialOfferCard({
  offer,
  partial,
  amount,
  closes,
}: {
  readonly offer: OfferView;
  readonly partial: PartialTermsView;
  readonly amount: (leg: TradeLegView) => React.ReactNode;
  readonly closes: string;
}) {
  const least = amount(partial.minFill);
  const buys = amount(partial.minFillOffered);
  const feePerFill =
    partial.feePerFill === null ? null : amount(partial.feePerFill);
  return (
    <div className='xc-trade xc-option' data-testid='prompt-partial-offer'>
      <div className='xc-option-title'>
        <FileSignature className={ICON} aria-hidden='true' />
        <Trans>Make a partial offer</Trans>
      </div>
      <div className='xc-option-lead'>
        <Trans>You give</Trans>
      </div>
      <ul className='xc-option-lines' data-testid='offer-gives'>
        {offer.gives.map((leg) => (
          <Line
            key={leg.label + leg.amount.toFixed(0)}
            icon={<Send className={ICON} aria-hidden='true' />}
            testId='offer-give'
          >
            {amount(leg)}
          </Line>
        ))}
      </ul>
      <div className='xc-trade-turn' aria-hidden='true'>
        <ArrowDown className='h-4 w-4' />
      </div>
      <div className='xc-option-lead'>
        <Trans>You ask for</Trans>
      </div>
      <ul className='xc-option-lines' data-testid='offer-asks'>
        {offer.asks.map((leg) => (
          <Line
            key={leg.label + leg.amount.toFixed(0)}
            icon={<HandCoins className={ICON} aria-hidden='true' />}
            testId='offer-ask'
          >
            {amount(leg)}
          </Line>
        ))}
      </ul>
      <ul className='xc-option-lines' data-testid='partial-terms'>
        <Line
          icon={<HandCoins className={ICON} aria-hidden='true' />}
          testId='partial-min-fill'
        >
          <Trans>
            Takers may fill any part from {least} up (for {buys}), at the same
            price, until it is all taken.
          </Trans>
        </Line>
        {feePerFill !== null && (
          <Line
            icon={<Send className={ICON} aria-hidden='true' />}
            testId='partial-fee-per-fill'
          >
            <Trans>Each fill pays a network fee of {feePerFill}.</Trans>
          </Line>
        )}
        {offer.expiresAt !== null && (
          <Line
            icon={<CalendarClock className={ICON} aria-hidden='true' />}
            testId='partial-expiry'
          >
            <Trans>Nobody can fill it from {closes}.</Trans>
          </Line>
        )}
      </ul>
      <div className='xc-option-note' data-testid='offer-note'>
        <Trans>
          Klimper signs a partial offer. Nothing moves until someone fills part
          of it. It is listed under your partial offers, where you can cancel
          it.
        </Trans>
      </div>
    </div>
  );
}

/** The option card(s) of a request: one loan, or one card per option effect. */
export function OptionCards({
  view,
  facts,
}: {
  readonly view: SpendPromptView;
  readonly facts: Facts;
}) {
  if (view.loan !== null) {
    return (
      <LoanCard
        loan={view.loan}
        makesOffer={view.makesOffer}
        facts={facts}
        xchLabel={view.xchLabel}
      />
    );
  }
  if (view.offer !== null) {
    return (
      <OfferCard offer={view.offer} facts={facts} xchLabel={view.xchLabel} />
    );
  }
  if (view.options.length === 0) return null;
  return (
    <div data-testid='prompt-options'>
      {view.options.map((effect, index) => (
        <EffectCard
          // eslint-disable-next-line react/no-array-index-key
          key={index}
          effect={effect}
          facts={facts}
          xchLabel={view.xchLabel}
        />
      ))}
    </div>
  );
}

/** Every asset the option cards name, for one lookup of what the wallet knows about them. */
export function optionAssets(view: SpendPromptView): AssetRef[] {
  const legs: AssetRef[] = [];
  for (const effect of view.options) {
    if (effect.action === 'clawback') legs.push(effect.collateral);
    else if (effect.action !== 'transfer') {
      legs.push(effect.terms.collateral, effect.terms.strike);
    }
  }
  if (view.loan !== null) {
    legs.push(view.loan.principal);
    if (view.loan.tie !== null) legs.push(view.loan.tie);
  }
  if (view.offer !== null) {
    for (const option of view.offer.options) {
      legs.push(option.terms.collateral, option.terms.strike);
    }
    legs.push(...view.offer.gives, ...view.offer.asks);
    if (view.offer.partial !== null) {
      legs.push(view.offer.partial.minFill, view.offer.partial.minFillOffered);
    }
  }
  return legs;
}
