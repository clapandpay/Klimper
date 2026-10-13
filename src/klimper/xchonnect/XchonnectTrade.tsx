// SPDX-License-Identifier: AGPL-3.0-or-later
// The trade card at the top of an approval (clapandpay TASK-139): what the user pays, what
// the user receives, the price, how many offers are taken together and the user's own fee.
//
// Every figure is the wallet's (`TradeView`, computed in Rust from the simulation of the
// bundle that is signed, read by `readPrompt` by explicit key). What the wallet knows about
// each asset from its own records (ticker, name, icon, balance, price) is shown with it; a
// CAT's asset id stays on screen next to every name, because whoever made a CAT chose its
// name, and a ticker that reads as the network's coin is not used at all, nor its icon
// (`ticker.ts`). A CAT has no dollar value here: its market price is the maker's to set.

import { Trans } from '@lingui/react/macro';
import { t } from '@lingui/core/macro';
import BigNumber from 'bignumber.js';
import { ArrowDown, ArrowLeftRight, Layers } from 'lucide-react';
import { useState } from 'react';
import { Hint } from '../Hint';
import { safeTicker } from './ticker';
import {
  AssetFacts,
  AssetRef,
  factsOf,
  formatUsd,
  xchUsd,
} from './useApprovalAssets';
import {
  catLabel,
  claimedText,
  formatAmount,
  PricedAssetView,
  TradeLegView,
  TradeView,
  XCH_PRECISION,
} from './xchonnectPrompt';

/** How many assets a side shows before "and N more". */
export const TRADE_VISIBLE = 3;

/**
 * Monogram colours: the Klimper coins and their warmer, deeper shades (docs/design-guide.md).
 * Blue is the network's coin, so a CAT never wears it or anything close. Each pair is fill
 * and glyph.
 */
const MONOGRAM: readonly (readonly [string, string])[] = [
  ['#e75839', '#ffffff'], // red light
  ['#e9b23a', '#1c1b19'], // saffron
  ['#8c5a3c', '#ffffff'], // copper
  ['#b8432a', '#ffffff'], // deep red
  ['#b9862a', '#1c1b19'], // ochre
  ['#5e5a52', '#ffffff'], // warm grey
];

/** The same colour for the same asset id, every time and on every phone. */
export function monogramColour(assetId: string): readonly [string, string] {
  let hash = 0;
  for (const character of assetId) {
    hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  }
  return MONOGRAM[hash % MONOGRAM.length] ?? MONOGRAM[0];
}

/**
 * The wallet's ticker for a CAT as a unit, or null when there is none it may use: one that is
 * empty, not a plain ASCII word, or that reads as the network's coin (`safeTicker`).
 */
export function walletTicker(
  facts: AssetFacts | undefined,
  xchLabel: string,
): string | null {
  return safeTicker(claimedText(facts?.ticker), xchLabel);
}

/**
 * What an amount of this asset is called in the price: XCH, or a CAT's ticker with the start
 * of its id (`SBX 5b5b5b5b`), or `CAT 1a2b3c4d`. The id is always there: two CATs may share
 * a ticker, and the price is the line a user compares with the website's.
 */
export function unitOf(
  asset: AssetRef & { readonly label: string },
  facts: AssetFacts | undefined,
  xchLabel: string,
): string {
  if (asset.isXch) return asset.label;
  if (!asset.assetId) return asset.label;
  const short = asset.assetId.slice(0, 8);
  const ticker = walletTicker(facts, xchLabel);
  return ticker === null ? `CAT ${short}` : `${ticker} ${short}`;
}

/**
 * The asset's round coin: XCH's blue coin; for a CAT the wallet's icon, only when its ticker
 * may be used (an icon can wear the network's logo as easily as a ticker its name) and the
 * wallet holds and shows the CAT (`useApprovalAssets`); else a monogram.
 */
export function TokenCoin({
  asset,
  facts,
  xchLabel,
}: {
  readonly asset: AssetRef;
  readonly facts: AssetFacts | undefined;
  readonly xchLabel: string;
}) {
  const [broken, setBroken] = useState(false);
  const ticker = asset.isXch ? null : walletTicker(facts, xchLabel);
  const icon = ticker === null ? null : (facts?.iconUrl ?? null);
  if (icon !== null && !broken) {
    return (
      <img
        className='xc-token xc-token-img'
        src={icon}
        alt=''
        aria-hidden='true'
        loading='lazy'
        referrerPolicy='no-referrer'
        onError={() => setBroken(true)}
        data-testid='trade-icon'
      />
    );
  }
  if (asset.isXch) {
    return (
      <span className='xc-token xc-token-xch' aria-hidden='true'>
        {xchLabel.startsWith('T') ? 'T' : 'X'}
      </span>
    );
  }
  const id = asset.assetId ?? '';
  const letters = ticker
    ? Array.from(ticker.replace(/[^\p{L}\p{N}]/gu, '') || ticker)
        .slice(0, 2)
        .join('')
    : id.slice(0, 2);
  const [fill, glyph] = monogramColour(id);
  return (
    <span
      className='xc-token'
      style={{ background: fill, color: glyph }}
      aria-hidden='true'
      data-testid='trade-monogram'
    >
      {letters.toUpperCase()}
    </span>
  );
}

/**
 * An amount in whole units with its thousands set apart by a narrow space, so 99999.999 reads
 * as 99 999.999 on any locale. Exact: the digits are `formatAmount`'s.
 */
export function groupedAmount(amount: BigNumber, precision: number): string {
  const [whole = '', fraction] = formatAmount(amount, precision).split('.');
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, '\u202f');
  return fraction === undefined ? grouped : `${grouped}.${fraction}`;
}

/**
 * Dollars for an amount of XCH, when Klimper has a price. Never for a CAT: its last price is
 * whatever its maker last traded it at, with themselves if they like (TASK-139 audit, M1).
 */
function usdOf(leg: TradeLegView): number | null {
  return leg.isXch ? xchUsd(leg.amount) : null;
}

function Leg({
  leg,
  facts,
  xchLabel,
  side,
}: {
  readonly leg: TradeLegView;
  readonly facts: AssetFacts | undefined;
  readonly xchLabel: string;
  readonly side: 'pay' | 'receive';
}) {
  // Without a ticker the id is the line under the amount; the unit only says what it is.
  const ticker = leg.isXch ? null : walletTicker(facts, xchLabel);
  const unit = leg.isXch ? leg.label : (ticker ?? 'CAT');
  // Another CAT the wallet knows reads the same: say so, under the id.
  const twin = ticker !== null && (facts?.twins?.length ?? 0) > 0;
  const usd = usdOf(leg);
  const name = leg.isXch ? null : claimedText(facts?.name).trim();
  // Looked up, and the wallet holds none of it: a CAT the user has never had.
  const unheld = !leg.isXch && facts !== undefined && facts.balance === null;
  return (
    <li className='xc-leg' data-testid='trade-leg' data-side={side}>
      <TokenCoin asset={leg} facts={facts} xchLabel={xchLabel} />
      <div className='min-w-0 flex-1'>
        <div className='xc-leg-line'>
          <span
            className={`xc-leg-amount ${side === 'pay' ? 'text-destructive' : ''}`}
            data-testid='trade-amount'
          >
            {side === 'pay' ? '−' : '+'}
            {groupedAmount(leg.amount, leg.precision)}{' '}
            <span className='xc-leg-unit'>{unit}</span>
          </span>
          {usd !== null && (
            <span className='xc-sub shrink-0' data-testid='trade-usd'>
              ≈ {formatUsd(usd)}
            </span>
          )}
        </div>
        {!leg.isXch && (
          <div className='xc-leg-meta' title={leg.assetId ?? undefined}>
            {name !== null && name !== '' && (
              <span className='xc-leg-name'>{name}</span>
            )}
            <span className='xc-leg-id' data-testid='trade-asset-id'>
              {leg.assetId ? catLabel(leg.assetId) : leg.label}
            </span>
            {unheld && (
              <span className='xc-leg-new' data-testid='trade-unheld'>
                <Trans>new to your wallet</Trans>
              </span>
            )}
          </div>
        )}
        {twin && (
          <div className='xc-leg-twin' data-testid='trade-twin'>
            <Trans>
              Two tokens in your wallet use the ticker {ticker}. Check the id.
            </Trans>
          </div>
        )}
      </div>
    </li>
  );
}

function Side({
  title,
  legs,
  facts,
  xchLabel,
  side,
}: {
  readonly title: React.ReactNode;
  readonly legs: readonly TradeLegView[];
  readonly facts: ReadonlyMap<string, AssetFacts>;
  readonly xchLabel: string;
  readonly side: 'pay' | 'receive';
}) {
  const [open, setOpen] = useState(false);
  // "and 1 more" hides nothing worth a tap: show the whole of a side one over the limit.
  const fold = legs.length > TRADE_VISIBLE + 1;
  const shown = fold && !open ? legs.slice(0, TRADE_VISIBLE) : legs;
  const hidden = legs.length - TRADE_VISIBLE;
  const count = legs.length;
  return (
    <section className='xc-trade-side' data-testid={`trade-${side}`}>
      <h3 className='xc-trade-label'>
        {title}
        {count > 1 && (
          <span className='xc-trade-count'>
            <Trans>{count} assets</Trans>
          </span>
        )}
      </h3>
      <ul className='xc-legs'>
        {shown.map((leg) => (
          <Leg
            key={leg.assetId ?? 'xch'}
            leg={leg}
            facts={factsOf(facts, leg)}
            xchLabel={xchLabel}
            side={side}
          />
        ))}
      </ul>
      {fold && (
        <button
          type='button'
          className='xc-trade-more'
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          data-testid={`trade-${side}-more`}
        >
          {open ? <Trans>Show fewer</Trans> : <Trans>and {hidden} more</Trans>}
        </button>
      )}
    </section>
  );
}

function Price({
  base,
  quote,
  quotePerBase,
  basePerQuote,
  facts,
  xchLabel,
}: {
  readonly base: PricedAssetView;
  readonly quote: PricedAssetView;
  readonly quotePerBase: string;
  readonly basePerQuote: string;
  readonly facts: ReadonlyMap<string, AssetFacts>;
  readonly xchLabel: string;
}) {
  const [flipped, setFlipped] = useState(false);
  const baseUnit = unitOf(base, factsOf(facts, base), xchLabel);
  const quoteUnit = unitOf(quote, factsOf(facts, quote), xchLabel);
  const [one, other, value] = flipped
    ? [quoteUnit, baseUnit, basePerQuote]
    : [baseUnit, quoteUnit, quotePerBase];
  return (
    <div className='xc-trade-row' data-testid='trade-price'>
      <span className='text-muted-foreground'>
        <Trans>Price</Trans>
      </span>
      <span className='xc-trade-price'>
        <span data-testid='trade-price-text'>
          1 {one} = {value} {other}
        </span>
        <button
          type='button'
          className='xc-trade-flip'
          onClick={() => setFlipped((value) => !value)}
          aria-label={t`Show the price the other way round`}
          data-testid='trade-price-flip'
        >
          <ArrowLeftRight className='h-3.5 w-3.5' aria-hidden='true' />
        </button>
      </span>
    </div>
  );
}

/** The trade card: the first thing on the approval of anything that pays for something. */
export function TradeCard({
  trade,
  facts,
  xchLabel,
}: {
  readonly trade: TradeView;
  readonly facts: ReadonlyMap<string, AssetFacts>;
  readonly xchLabel: string;
}) {
  const offers = trade.offers ?? 1;
  const feeUsd = trade.fee.isGreaterThan(0) ? xchUsd(trade.fee) : null;
  return (
    <div className='xc-trade' data-testid='prompt-trade'>
      <Side
        title={<Trans>You pay</Trans>}
        legs={trade.pays}
        facts={facts}
        xchLabel={xchLabel}
        side='pay'
      />
      <div className='xc-trade-turn' aria-hidden='true'>
        <ArrowDown className='h-4 w-4' />
      </div>
      <Side
        title={<Trans>You receive</Trans>}
        legs={trade.receives}
        facts={facts}
        xchLabel={xchLabel}
        side='receive'
      />
      <div className='xc-trade-facts'>
        {trade.price !== null && (
          <Price {...trade.price} facts={facts} xchLabel={xchLabel} />
        )}
        {offers > 1 && (
          <div className='xc-trade-row' data-testid='trade-offers'>
            <span className='flex items-start gap-1.5'>
              <Layers
                className='h-4 w-4 mt-0.5 shrink-0 text-muted-foreground'
                aria-hidden='true'
              />
              <span>
                <Trans>{offers} separate offers taken together</Trans>
                <span className='text-muted-foreground'>
                  {' '}
                  — <Trans>all or nothing</Trans>
                </span>{' '}
                <Hint>
                  <Trans>
                    Klimper counted the offers in the offer itself: groups of
                    coins that are not tied to each other, each asking for its
                    own payment. They are taken in one transaction: either every
                    one of them goes through, or none does and you pay nothing.
                  </Trans>
                </Hint>
              </span>
            </span>
          </div>
        )}
        {trade.fee.isGreaterThan(0) && (
          <div className='xc-trade-row' data-testid='trade-fee'>
            <span className='text-muted-foreground'>
              <Trans>Your network fee</Trans>
            </span>
            <span className='text-right tabular-nums'>
              {formatAmount(trade.fee, XCH_PRECISION)} {xchLabel}
              {feeUsd !== null && (
                <span className='xc-sub ml-2'>≈ {formatUsd(feeUsd)}</span>
              )}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

/** Every asset of the trade, for one lookup of what the wallet knows about them. */
export function tradeAssets(trade: TradeView | null): AssetRef[] {
  if (trade === null) return [];
  const legs: AssetRef[] = [...trade.pays, ...trade.receives];
  if (trade.price !== null) legs.push(trade.price.base, trade.price.quote);
  return legs;
}
