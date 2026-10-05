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
import { Trans } from '@lingui/react/macro';
import BigNumber from 'bignumber.js';
import { AlertTriangle, ArrowRight, Clock, Home } from 'lucide-react';
import {
  AssetView,
  BindingView,
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
      className='rounded-lg border border-destructive p-3 space-y-1'
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

/**
 * One asset.
 *
 * `net` is the verdict, and it is the only figure in large type. `conditional_received`
 * gets its own line and its own sentence, and is never added to anything: it arrives from
 * spends the user neither owns nor signs, so it is income only if somebody else acts.
 */
function Asset({
  asset,
  payLabel,
}: {
  readonly asset: AssetView;
  readonly payLabel?: React.ReactNode;
}) {
  const magnitude = formatMagnitude(asset.net, asset.precision);
  const conditional = formatAmount(asset.conditionalReceived, asset.precision);
  const label = asset.label;
  return (
    <div className='space-y-1' data-testid='prompt-asset'>
      {asset.net.isNegative() ? (
        <div className='flex items-baseline justify-between gap-3'>
          <span className='text-muted-foreground'>
            {payLabel ?? <Trans>You pay</Trans>}
          </span>
          <span
            className='text-2xl font-semibold text-destructive'
            data-testid='asset-net'
          >
            −{magnitude} {label}
          </span>
        </div>
      ) : asset.net.isZero() ? (
        <div className='flex items-baseline justify-between gap-3 text-muted-foreground'>
          <span>
            <Trans>No change</Trans>
          </span>
          <span data-testid='asset-net'>0 {label}</span>
        </div>
      ) : (
        <div className='flex items-baseline justify-between gap-3'>
          <span className='text-muted-foreground'>
            <Trans>You receive</Trans>
          </span>
          <span className='text-2xl font-semibold' data-testid='asset-net'>
            +{magnitude} {label}
          </span>
        </div>
      )}
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
    lines.push(
      <Trans key='rel'>
        Some spends are time-limited relative to their coins.
      </Trans>,
    );
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
      className='rounded-lg border border-border p-3 space-y-2 text-sm'
      data-testid='prompt-partial'
    >
      <div className='font-medium'>
        <Trans>This is one half of a trade</Trans>
      </div>
      <p className='text-muted-foreground'>
        <Trans>
          You are signing your side. Klimper checked that your signature only
          pays out if the other side pays you.
        </Trans>
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

/**
 * An address the wallet encoded itself from the recipient's puzzle hash, shortened for the
 * line it sits on; the whole address is on hover.
 */
function Recipient({
  output,
  prefix,
}: {
  readonly output: OutputView;
  readonly prefix: string;
}) {
  if (output.recipient === null) {
    return (
      <span className='text-muted-foreground'>
        <Trans>an address Klimper cannot show</Trans>
      </span>
    );
  }
  let address: string;
  try {
    address = toAddress(output.recipient, prefix);
  } catch {
    address = `0x${output.recipient}`;
  }
  const short =
    address.length > 22
      ? `${address.slice(0, 12)}…${address.slice(-6)}`
      : address;
  return (
    <span
      className='font-mono text-xs'
      title={address}
      data-testid='output-address'
    >
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
        </div>
        <p className='text-sm text-muted-foreground'>
          <Trans>Nothing leaves your wallet except the network fee.</Trans>
        </p>
        <ul className='space-y-1 text-sm'>
          {toUser.map((output, index) => (
            // eslint-disable-next-line react/no-array-index-key
            <li key={index} className='flex items-center justify-between gap-3'>
              <Recipient output={output} prefix={prefix} />
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
    <div className='space-y-2' data-testid='prompt-outputs'>
      <div className='xc-label'>
        <Trans>Sending</Trans>
      </div>
      <ul className='space-y-2'>
        {toOthers.map((output, index) => (
          <li
            // eslint-disable-next-line react/no-array-index-key
            key={index}
            className='flex flex-wrap items-center gap-x-2 gap-y-1'
            data-testid='prompt-output'
          >
            <span className='xc-send-amount'>
              {formatAmount(output.amount, output.precision)} {output.label}
            </span>
            <ArrowRight
              className='h-4 w-4 text-muted-foreground'
              aria-hidden='true'
            />
            <Recipient output={output} prefix={prefix} />
          </li>
        ))}
      </ul>
      {toUser.length > 0 && (
        <div
          className='text-sm text-muted-foreground'
          data-testid='prompt-change'
        >
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

function SpendPrompt({ view }: { readonly view: SpendPromptView }) {
  const sendsToOthers = view.outputs.some((output) => !output.toUser);
  // The fee is one figure. Both readings are shown only when they disagree.
  const showReserve =
    view.reserveFee.isGreaterThan(0) &&
    (view.impliedFee === null || !view.reserveFee.isEqualTo(view.impliedFee));
  return (
    <div className='space-y-4'>
      <p className='text-sm text-muted-foreground'>
        <Trans>
          This is what Klimper worked out by running the spend itself, not what
          the website says it does.
        </Trans>
      </p>

      <Destinations view={view} />

      <div className='xc-verdict space-y-3'>
        {view.assets.length === 0 ? (
          <p className='text-sm'>
            <Trans>No change to your assets.</Trans>
          </p>
        ) : (
          view.assets.map((asset) => (
            <Asset
              key={asset.label}
              asset={asset}
              payLabel={
                sendsToOthers && asset.isXch ? (
                  <Trans>In total, with the fee</Trans>
                ) : undefined
              }
            />
          ))
        )}
      </div>

      <div className='space-y-1 text-sm'>
        {view.impliedFee === null ? (
          <div className='text-muted-foreground'>
            <Trans>
              This request spends coins it does not include, so Klimper cannot
              work out the network fee.
            </Trans>
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
              <span>
                {formatAmount(view.impliedFee, XCH_PRECISION)} {view.xchLabel}
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

      <SignatureCounts
        ours={view.signatures.length}
        foreign={view.foreignSignatures}
      />

      <ComputedDetail view={view} />
    </div>
  );
}

function MessagePrompt({ view }: { readonly view: MessagePromptView }) {
  const length = view.length;
  return (
    <div className='space-y-3 text-sm'>
      <p>
        <Trans>
          This website asks you to sign a message. Signing proves you hold the
          key; it moves no money.
        </Trans>
      </p>
      <div>
        <div className='text-muted-foreground mb-1'>
          {view.messageText === null ? (
            <Trans>The website asks you to sign this data</Trans>
          ) : (
            <Trans>The website asks you to sign this text</Trans>
          )}
        </div>
        <pre
          className='font-mono text-xs break-all whitespace-pre-wrap bg-muted p-2 rounded max-h-48 overflow-y-auto'
          data-testid='message-body'
        >
          {view.messageText ?? view.messageHex}
        </pre>
        <div className='text-xs text-muted-foreground mt-1'>
          <Trans>{length} characters</Trans>
          {view.truncated && (
            <>
              {' · '}
              <Trans>shown shortened</Trans>
            </>
          )}
        </div>
      </div>
      <div>
        <div className='text-muted-foreground'>
          <Trans>With this key</Trans>
        </div>
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
    <div className='text-sm text-muted-foreground'>
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
function ComputedDetail({ view }: { readonly view: PromptView }) {
  return (
    <details>
      <summary className='cursor-pointer text-xs text-muted-foreground'>
        <Trans>Everything Klimper computed</Trans>
      </summary>
      <pre className='text-[10px] overflow-x-auto bg-muted p-2 rounded mt-1'>
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
  return view.partial ? (
    <Trans>Sign one half of a trade</Trans>
  ) : (
    <Trans>Sign a spend</Trans>
  );
}
