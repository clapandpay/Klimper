// SPDX-License-Identifier: AGPL-3.0-or-later
// The rendering table of `docs/xchonnect-screens.md`, in code (clapandpay TASK-080).
//
// It renders a `PromptView` and nothing else. A `PromptView` can only be made by
// `readPrompt`, which reads the wallet's own simulation by explicit key, so there is no
// path from a dApp's description of a request to anything on this screen.
//
// The order is the order the decision is made in: what happens to the money, then what it
// costs, then what could go wrong, and only then the detail.

import { toAddress } from '@/lib/utils';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import BigNumber from 'bignumber.js';
import {
  AlertTriangle,
  ArrowRight,
  BadgeCheck,
  Clock,
  Home,
  KeyRound,
  PenLine,
  Send,
  ShieldCheck,
} from 'lucide-react';
import {
  AssetFacts,
  factsOf,
  formatUsd,
  xchUsd,
  useApprovalAssets,
} from './useApprovalAssets';
import { Hint } from './XchonnectUi';
import {
  AssetView,
  BindingView,
  claimedText,
  IntentView,
  formatAmount,
  formatMagnitude,
  MessagePromptView,
  OutputView,
  PromptView,
  SpendPromptView,
  TimeLockView,
  XCH_PRECISION,
} from './xchonnectPrompt';

function Danger({
  title,
  children,
  testId,
}: {
  readonly title: React.ReactNode;
  readonly children?: React.ReactNode;
  readonly testId: string;
}) {
  return (
    <div
      className='rounded-2xl border border-destructive p-3 space-y-1'
      data-testid={testId}
      role='alert'
    >
      <div className='flex items-start gap-2 text-destructive font-semibold'>
        <AlertTriangle className='h-4 w-4 mt-0.5 shrink-0' aria-hidden='true' />
        <span>{title}</span>
      </div>
      {children && <div className='text-sm pl-6'>{children}</div>}
    </div>
  );
}

/** A short fact with its reason behind an info dot. */
function Badge({
  icon,
  tone,
  hint,
  testId,
  children,
}: {
  readonly icon: React.ReactNode;
  readonly tone?: 'verified';
  readonly hint: React.ReactNode;
  readonly testId?: string;
  readonly children: React.ReactNode;
}) {
  return (
    <span className='xc-badge' data-tone={tone} data-testid={testId}>
      {icon}
      {children}
      <Hint>{hint}</Hint>
    </span>
  );
}

/** Dollars next to an amount of XCH, when Klimper has a price for it. */
function Usd({ mojos }: { readonly mojos: BigNumber }) {
  const usd = xchUsd(mojos);
  return usd === null ? null : <span>≈ {formatUsd(usd)}</span>;
}

/**
 * One asset.
 *
 * `net` is the verdict, and it is the only figure in large type. `conditional_received`
 * gets its own line and its own sentence, and is never added to anything: it arrives from
 * spends the user neither owns nor signs, so it is income only if somebody else acts. Under
 * the net: what the wallet holds afterwards, the dollars for XCH, and the wallet's ticker for a
 * CAT — beside its asset id, never in its place, since anyone can call a CAT "XCH".
 */
function Asset({
  asset,
  facts,
  payLabel,
}: {
  readonly asset: AssetView;
  readonly facts: AssetFacts | undefined;
  readonly payLabel?: React.ReactNode;
}) {
  const magnitude = formatMagnitude(asset.net, asset.precision);
  const conditional = formatAmount(asset.conditionalReceived, asset.precision);
  const label = asset.label;
  const after =
    facts?.balance != null && !asset.net.isZero()
      ? facts.balance.plus(asset.net)
      : null;
  const left =
    after === null ? '' : `${formatAmount(after, asset.precision)} ${label}`;
  const ticker = claimedText(facts?.ticker);
  // Looked up, and the wallet holds none of it: a CAT the user has never had.
  const unheld = !asset.isXch && facts !== undefined && facts.balance === null;
  const sign = asset.net.isNegative() ? '−' : asset.net.isZero() ? '' : '+';
  return (
    <div className='flex items-start gap-3' data-testid='prompt-asset'>
      <span className='xc-asset-icon' aria-hidden='true'>
        {Array.from(label)[0]}
      </span>
      <div className='min-w-0 flex-1 space-y-0.5'>
        <div className='flex items-baseline justify-between gap-3'>
          <span className='text-sm text-muted-foreground'>
            {asset.net.isNegative() ? (
              (payLabel ?? <Trans>You pay</Trans>)
            ) : asset.net.isZero() ? (
              <Trans>No change</Trans>
            ) : (
              <Trans>You receive</Trans>
            )}
          </span>
          <span
            className={`text-2xl font-semibold text-right break-all ${
              asset.net.isNegative()
                ? 'text-destructive'
                : asset.net.isZero()
                  ? 'text-muted-foreground'
                  : ''
            }`}
            data-testid='asset-net'
          >
            {sign}
            {magnitude} {label}
          </span>
        </div>
        <div className='xc-sub flex flex-wrap justify-between gap-x-3'>
          <span title={asset.assetId ?? undefined} data-testid='asset-ticker'>
            {ticker !== '' && <Trans>your wallet: {ticker}</Trans>}
            {unheld && (
              <span data-testid='asset-unheld'>
                <Trans>not in your wallet</Trans>
              </span>
            )}
          </span>
          <span className='flex gap-2'>
            {asset.isXch && <Usd mojos={asset.net.abs()} />}
            {after !== null && after.isGreaterThanOrEqualTo(0) && (
              <span data-testid='asset-after'>
                <Trans>then {left}</Trans>
              </span>
            )}
          </span>
        </div>
        {asset.conditionalReceived.isGreaterThan(0) && (
          <div
            className='flex items-baseline justify-between gap-3 text-sm text-muted-foreground'
            data-testid='asset-conditional'
          >
            <span>
              <Trans>Only if the counterparty completes</Trans>
            </span>
            <span>
              +{conditional} {label}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

function TimeLocks({ locks }: { readonly locks: TimeLockView }) {
  const time = (seconds: number) => new Date(seconds * 1000).toLocaleString();
  const lines: React.ReactNode[] = [];
  if (locks.notBeforeSeconds !== null) {
    const when = time(locks.notBeforeSeconds);
    lines.push(<Trans key='nbs'>Not valid before {when}</Trans>);
  }
  if (locks.notBeforeHeight !== null) {
    const height = locks.notBeforeHeight;
    lines.push(<Trans key='nbh'>Not valid before block {height}</Trans>);
  }
  if (locks.expiresSeconds !== null) {
    const when = time(locks.expiresSeconds);
    lines.push(<Trans key='xs'>Expires {when}</Trans>);
  }
  if (locks.expiresHeight !== null) {
    const height = locks.expiresHeight;
    lines.push(<Trans key='xh'>Expires at block {height}</Trans>);
  }
  if (locks.hasRelative) {
    lines.push(<Trans key='rel'>Time-limited</Trans>);
  }
  if (lines.length === 0) return null;
  return (
    <div
      className='flex items-start gap-2 text-sm text-muted-foreground'
      data-testid='prompt-time-locks'
    >
      <Clock className='h-4 w-4 mt-0.5 shrink-0' aria-hidden='true' />
      <div className='space-y-0.5'>
        {lines.map((line, index) => (
          // eslint-disable-next-line react/no-array-index-key
          <div key={index}>{line}</div>
        ))}
      </div>
    </div>
  );
}

function Binding({
  binding,
  partial,
}: {
  readonly binding: BindingView | null;
  readonly partial: boolean;
}) {
  if (!partial) return null;
  if (binding !== null && !binding.allBound) {
    return (
      <Danger
        testId='prompt-unbound'
        title={<Trans>Not bound — do not sign</Trans>}
      >
        <Trans>
          Your signature would not be tied to the payment you are expecting.
          Klimper refuses this; if you are seeing it, reject and tell us.
        </Trans>
      </Danger>
    );
  }
  return (
    <div
      className='rounded-2xl border border-border p-3 space-y-1.5 text-sm'
      data-testid='prompt-partial'
    >
      <div className='font-medium'>
        <Trans>One half of a trade</Trans>
      </div>
      <p className='text-muted-foreground'>
        {binding?.allBound ? (
          <Trans>
            Klimper checked that your signature only pays out if the other side
            pays you.
          </Trans>
        ) : (
          <Trans>You are signing your side.</Trans>
        )}
      </p>
      {binding?.payments.map((payment, index) => (
        <div
          // eslint-disable-next-line react/no-array-index-key
          key={index}
          className='text-muted-foreground'
          data-testid='bound-payment'
        >
          <Trans>Depends on receiving</Trans>{' '}
          {formatAmount(payment.amount, payment.precision)} {payment.label}
        </div>
      ))}
    </div>
  );
}

/** A puzzle hash as the network's address, shortened, the whole on hover. */
function Address({
  puzzleHash,
  prefix,
  testId,
}: {
  readonly puzzleHash: string | null;
  readonly prefix: string;
  readonly testId?: string;
}) {
  if (puzzleHash === null) {
    return (
      <span className='text-muted-foreground'>
        <Trans>an address Klimper cannot show</Trans>
      </span>
    );
  }
  let address: string;
  try {
    address = toAddress(puzzleHash, prefix);
  } catch {
    address = `0x${puzzleHash}`;
  }
  const short =
    address.length > 22
      ? `${address.slice(0, 12)}…${address.slice(-6)}`
      : address;
  return (
    <span className='font-mono text-xs' title={address} data-testid={testId}>
      {short}
    </span>
  );
}

/** Totals per asset label, in the order the assets first appear. */
function totals(outputs: readonly OutputView[]) {
  const sums = new Map<string, { amount: BigNumber; precision: number }>();
  for (const output of outputs) {
    const current = sums.get(output.label);
    sums.set(output.label, {
      amount: (current?.amount ?? BigNumber(0)).plus(output.amount),
      precision: output.precision,
    });
  }
  return [...sums.entries()];
}

/**
 * Where the money goes, from wallet-kit's `outputs`: every coin the user's own spends
 * create. A send to someone else is listed with its address; change is one quiet line;
 * a request whose every output is the user's own says so plainly, because its "you pay" is
 * only the fee and that looks wrong without the reason next to it.
 */
function Destinations({ view }: { readonly view: SpendPromptView }) {
  if (view.outputs.length === 0) return null;
  const prefix = view.xchLabel.toLowerCase();
  const toOthers = view.outputs.filter((output) => !output.toUser);
  const toUser = view.outputs.filter((output) => output.toUser);

  if (toOthers.length === 0) {
    return (
      <div className='xc-own space-y-2' data-testid='prompt-only-own'>
        <div className='flex items-center gap-2 font-semibold'>
          <Home className='h-4 w-4 shrink-0' aria-hidden='true' />
          <Trans>Only to your own addresses</Trans>
          <Hint>
            <Trans>Nothing leaves your wallet except the network fee.</Trans>
          </Hint>
        </div>
        <ul className='space-y-1 text-sm'>
          {toUser.map((output, index) => (
            // eslint-disable-next-line react/no-array-index-key
            <li key={index} className='flex items-center justify-between gap-3'>
              <Address
                puzzleHash={output.recipient}
                prefix={prefix}
                testId='output-address'
              />
              <span className='tabular-nums'>
                {formatAmount(output.amount, output.precision)} {output.label}
              </span>
            </li>
          ))}
        </ul>
      </div>
    );
  }

  return (
    <div className='space-y-1.5' data-testid='prompt-outputs'>
      <ul className='space-y-1.5'>
        {toOthers.map((output, index) => (
          <li
            // eslint-disable-next-line react/no-array-index-key
            key={index}
            className='flex flex-wrap items-center gap-x-2 gap-y-1'
            data-testid='prompt-output'
          >
            <Send
              className='h-4 w-4 shrink-0 text-muted-foreground'
              aria-hidden='true'
            />
            <span className='xc-send-amount'>
              {formatAmount(output.amount, output.precision)} {output.label}
            </span>
            <ArrowRight
              className='h-4 w-4 text-muted-foreground'
              aria-hidden='true'
            />
            <Address
              puzzleHash={output.recipient}
              prefix={prefix}
              testId='output-address'
            />
          </li>
        ))}
      </ul>
      {toUser.length > 0 && (
        <div className='xc-sub' data-testid='prompt-change'>
          <Trans>Change back to you:</Trans>{' '}
          {totals(toUser)
            .map(
              ([label, { amount, precision }]) =>
                `${formatAmount(amount, precision)} ${label}`,
            )
            .join(', ')}
        </div>
      )}
    </div>
  );
}

/**
 * Who built the request, that Klimper sends it, and the two-engine check, as badges with
 * their sentences behind the info dot. Shown before any amount: it says whose figures follow.
 */
function Handling({ view }: { readonly view: SpendPromptView }) {
  return (
    <div className='xc-badges' data-testid='prompt-handling'>
      <Badge
        icon={<ShieldCheck className='h-3.5 w-3.5' aria-hidden='true' />}
        hint={
          <Trans>
            This is what Klimper worked out by running the spend itself, not
            what the website says it does.
          </Trans>
        }
      >
        <Trans>Simulated by Klimper</Trans>
      </Badge>
      {view.broadcasts &&
        (view.walletBuilt ? (
          <Badge
            icon={<Send className='h-3.5 w-3.5' aria-hidden='true' />}
            hint={
              <Trans>
                Klimper built this from your own coins and sends it to the
                network itself.
              </Trans>
            }
          >
            <Trans>Built by Klimper</Trans>
          </Badge>
        ) : (
          <Badge
            icon={<Send className='h-3.5 w-3.5' aria-hidden='true' />}
            hint={
              <Trans>
                The website built this transaction. Klimper checks it, signs it
                and sends it to the network itself.
              </Trans>
            }
          >
            <Trans>Built by the website</Trans>
          </Badge>
        ))}
      {view.crossChecked && (
        <Badge
          tone='verified'
          testId='prompt-cross-checked'
          icon={<BadgeCheck className='h-3.5 w-3.5' aria-hidden='true' />}
          hint={
            <Trans>
              Your wallet’s records and an independent simulation agree on what
              leaves your wallet.
            </Trans>
          }
        >
          <Trans>Checked twice</Trans>
        </Badge>
      )}
    </div>
  );
}

/**
 * The website's declared intent, as the wallet verified it. The label is the website's
 * own words and says so; every line under it is a claim the simulation proved.
 */
function Intent({
  intent,
  prefix,
  xchLabel,
}: {
  readonly intent: IntentView;
  readonly prefix: string;
  readonly xchLabel: string;
}) {
  if (intent.kind === null && intent.verified.length === 0) return null;
  const kind = intent.kind;
  return (
    <div className='xc-intent space-y-2' data-testid='prompt-intent'>
      <div className='flex items-center gap-1 text-sm'>
        <span className='text-muted-foreground'>
          <Trans>The website says</Trans>
        </span>
        {kind !== null && (
          <span className='font-mono' data-testid='intent-kind'>
            {kind}
          </span>
        )}
        <Hint>
          <Trans>
            The label is the website’s own. Each line with a tick is checked by
            Klimper.
          </Trans>
        </Hint>
      </div>
      {intent.verified.length > 0 && (
        <ul className='space-y-1 text-sm'>
          {intent.verified.map((fact, index) => (
            <li
              // eslint-disable-next-line react/no-array-index-key
              key={index}
              className='flex items-center gap-2'
              data-testid='intent-fact'
            >
              <BadgeCheck
                className='h-4 w-4 shrink-0 xc-verified'
                aria-hidden='true'
              />
              {fact.type === 'recipient' ? (
                <span>
                  <span className='tabular-nums'>
                    {formatAmount(fact.amount, fact.precision)} {fact.label}
                  </span>{' '}
                  <Trans>to</Trans>{' '}
                  <Address puzzleHash={fact.recipient} prefix={prefix} />
                </span>
              ) : fact.type === 'fee' ? (
                <span>
                  <Trans>Network fee</Trans>{' '}
                  <span className='tabular-nums'>
                    {formatAmount(fact.amount, XCH_PRECISION)} {xchLabel}
                  </span>
                </span>
              ) : (
                <span>
                  <Trans>Your balance changes by</Trans>{' '}
                  <span className='tabular-nums'>
                    {fact.amount.isNegative() ? '−' : '+'}
                    {formatMagnitude(fact.amount, fact.precision)} {fact.label}
                  </span>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {intent.recipientsComplete && (
        <p className='xc-sub'>
          <Trans>
            Verified by Klimper: these are the only payments to anyone else.
          </Trans>
        </p>
      )}
    </div>
  );
}

function SpendPrompt({ view }: { readonly view: SpendPromptView }) {
  const facts = useApprovalAssets(view.assets);
  const sendsToOthers = view.outputs.some((output) => !output.toUser);
  // The fee is one figure. Both readings are shown only when they disagree.
  const showReserve =
    view.reserveFee.isGreaterThan(0) &&
    (view.impliedFee === null || !view.reserveFee.isEqualTo(view.impliedFee));
  return (
    <div className='space-y-4'>
      <Handling view={view} />

      <div className='xc-verdict space-y-4'>
        {view.assets.length === 0 ? (
          <p className='text-sm'>
            <Trans>No change to your assets.</Trans>
          </p>
        ) : (
          view.assets.map((asset) => (
            <Asset
              key={asset.assetId ?? asset.label}
              asset={asset}
              facts={factsOf(facts, asset)}
              payLabel={
                sendsToOthers && asset.isXch ? (
                  <Trans>In total, with the fee</Trans>
                ) : undefined
              }
            />
          ))
        )}
      </div>

      <Destinations view={view} />

      <div className='space-y-1 text-sm'>
        {view.impliedFee === null ? (
          <div className='flex items-center gap-1 text-muted-foreground'>
            <Trans>Network fee unknown</Trans>
            <Hint>
              <Trans>
                This request spends coins it does not include, so Klimper cannot
                work out the network fee.
              </Trans>
            </Hint>
          </div>
        ) : (
          view.impliedFee.isGreaterThan(0) && (
            <div
              className='flex justify-between gap-3'
              data-testid='prompt-implied-fee'
            >
              <span className='text-muted-foreground'>
                <Trans>Network fee</Trans>
              </span>
              <span className='text-right'>
                {formatAmount(view.impliedFee, XCH_PRECISION)} {view.xchLabel}
                <span className='xc-sub ml-2'>
                  <Usd mojos={view.impliedFee} />
                </span>
              </span>
            </div>
          )
        )}
        {showReserve && (
          <div
            className='flex justify-between gap-3'
            data-testid='prompt-reserve-fee'
          >
            <span className='text-muted-foreground'>
              <Trans>Fee reserved by the spend</Trans>
            </span>
            <span>
              {formatAmount(view.reserveFee, XCH_PRECISION)} {view.xchLabel}
            </span>
          </div>
        )}
      </div>

      {view.intent !== null && (
        <Intent
          intent={view.intent}
          prefix={view.xchLabel.toLowerCase()}
          xchLabel={view.xchLabel}
        />
      )}

      <TimeLocks locks={view.timeLocks} />

      <Binding binding={view.binding} partial={view.partial} />

      {view.unknownPuzzles.length > 0 && (
        <Danger
          testId='prompt-unknown-contract'
          title={<Trans>Unknown contract</Trans>}
        >
          <p>
            <Trans>
              Klimper does not recognise this contract. Nobody can tell you what
              it does with your coins.
            </Trans>
          </p>
          {view.unknownPuzzles.map((hash) => (
            <div key={hash} className='font-mono text-xs break-all mt-1'>
              {hash}
            </div>
          ))}
        </Danger>
      )}

      {view.hasUnsafe && (
        <Danger
          testId='prompt-unsafe-signature'
          title={<Trans>Unsafe signature</Trans>}
        >
          <Trans>
            This signature would be valid in any spend, not just this one.
            Reject it unless you know exactly why it is needed.
          </Trans>
        </Danger>
      )}

      <ComputedDetail view={view}>
        <SignatureCounts
          ours={view.signatures.length}
          foreign={view.foreignSignatures}
        />
      </ComputedDetail>
    </div>
  );
}

function MessagePrompt({ view }: { readonly view: MessagePromptView }) {
  const length = view.length;
  return (
    <div className='space-y-3 text-sm'>
      <div className='xc-badges'>
        <Badge
          icon={<PenLine className='h-3.5 w-3.5' aria-hidden='true' />}
          hint={
            <Trans>Signing proves you hold the key. It moves no money.</Trans>
          }
        >
          {view.messageText === null ? (
            <Trans>The website asks you to sign this data</Trans>
          ) : (
            <Trans>The website asks you to sign this text</Trans>
          )}
        </Badge>
        <span className='xc-badge' data-tone='verified'>
          <Trans>moves no money</Trans>
        </span>
      </div>
      <div>
        <pre
          className='font-mono text-xs break-all whitespace-pre-wrap bg-muted p-3 rounded-2xl max-h-48 overflow-y-auto'
          data-testid='message-body'
        >
          {view.messageText ?? view.messageHex}
        </pre>
        <div className='xc-sub mt-1'>
          <Trans>{length} characters</Trans>
          {view.truncated && (
            <>
              {' · '}
              <Trans>shown shortened</Trans>
            </>
          )}
        </div>
      </div>
      <div className='flex items-start gap-2'>
        <KeyRound
          className='h-4 w-4 mt-0.5 shrink-0 text-muted-foreground'
          aria-label={t`With this key`}
        />
        <div className='font-mono text-xs break-all' data-testid='message-key'>
          {view.publicKey}
        </div>
      </div>
    </div>
  );
}

function SignatureCounts({
  ours,
  foreign,
}: {
  readonly ours: number;
  readonly foreign: number;
}) {
  return (
    <div className='text-xs text-muted-foreground'>
      <Trans>Signatures you would produce: {ours}</Trans>
      {foreign > 0 && (
        <>
          {' · '}
          <Trans>Signatures other people still owe: {foreign}</Trans>
        </>
      )}
    </div>
  );
}

/**
 * Everything the wallet computed, for anyone who wants to check the detail.
 *
 * It prints the **parsed** view, not the prompt JSON. Printing the JSON would put every
 * key the prompt happens to carry on the screen, which is the one thing this screen must
 * not do: a field a dApp could influence would be rendered after all, just inside a
 * disclosure. Everything here has been through `readPrompt`.
 */
function ComputedDetail({
  view,
  children,
}: {
  readonly view: PromptView;
  readonly children?: React.ReactNode;
}) {
  return (
    <details className='space-y-2'>
      <summary className='cursor-pointer text-xs text-muted-foreground'>
        <Trans>Details</Trans>
      </summary>
      {children}
      <pre className='text-[10px] overflow-x-auto bg-muted p-2 rounded-xl'>
        {JSON.stringify(view, null, 2)}
      </pre>
    </details>
  );
}

export function XchonnectPromptView({ view }: { readonly view: PromptView }) {
  if (view.kind === 'unreadable') {
    return (
      <Danger
        testId='prompt-unreadable'
        title={<Trans>Klimper cannot read this request</Trans>}
      >
        <Trans>
          This build does not understand what is being asked. Reject it: there
          is nothing here you could agree to.
        </Trans>
      </Danger>
    );
  }
  return view.kind === 'message' ? (
    <MessagePrompt view={view} />
  ) : (
    <SpendPrompt view={view} />
  );
}

/** The one-line summary for a queued approval, with no amount in it. */
export function promptHeadline(view: PromptView): React.ReactNode {
  if (view.kind === 'message') return <Trans>Sign a message</Trans>;
  if (view.kind === 'unreadable') return <Trans>Unreadable request</Trans>;
  if (view.walletBuilt) return <Trans>Send</Trans>;
  if (view.broadcasts) return <Trans>Sign and send</Trans>;
  return view.partial ? (
    <Trans>Sign one half of a trade</Trans>
  ) : (
    <Trans>Sign a spend</Trans>
  );
}
