// SPDX-License-Identifier: AGPL-3.0-or-later
// Small pieces the Xchonnect screens share, in the Klimper Coin look (docs/design-guide.md).
// The styles are `xchonnect.css`; nothing here decides anything.

import { Trans } from '@lingui/react/macro';
import { Check } from 'lucide-react';
import { ReactNode } from 'react';
import './xchonnect.css';

/** The sun coin for a website: the first letter of its verified domain. */
export function SiteCoin({
  domain,
  size = 'md',
}: {
  readonly domain: string;
  readonly size?: 'md' | 'lg';
}) {
  const letter = Array.from(domain.replace(/^www\./, ''))[0] ?? '?';
  return (
    <span
      className={`xc-coin ${size === 'lg' ? 'h-14 w-14 text-2xl' : 'h-10 w-10 text-lg'}`}
      aria-hidden='true'
    >
      {letter.toUpperCase()}
    </span>
  );
}

export type ConnectStep = 'scan' | 'check' | 'compare';

const STEPS: readonly ConnectStep[] = ['scan', 'check', 'compare'];

function stepLabel(step: ConnectStep): ReactNode {
  switch (step) {
    case 'scan':
      return <Trans>Scan</Trans>;
    case 'check':
      return <Trans>Check</Trans>;
    case 'compare':
      return <Trans>Compare</Trans>;
  }
}

/** Where the user is in connecting a website. Plain text, never a control. */
export function ConnectSteps({ current }: { readonly current: ConnectStep }) {
  const at = STEPS.indexOf(current);
  return (
    <ol className='flex items-center gap-2 text-sm text-muted-foreground'>
      {STEPS.map((step, index) => {
        const state =
          index < at ? 'done' : index === at ? 'current' : 'upcoming';
        return (
          <li
            key={step}
            className='xc-step flex items-center gap-2'
            data-state={state}
            aria-current={state === 'current' ? 'step' : undefined}
          >
            {index > 0 && (
              <span className='h-px w-6 bg-border' aria-hidden='true' />
            )}
            <span className='xc-step-dot'>
              {state === 'done' ? (
                <Check className='h-3.5 w-3.5' aria-hidden='true' />
              ) : (
                index + 1
              )}
            </span>
            <span className='xc-step-label'>{stepLabel(step)}</span>
          </li>
        );
      })}
    </ol>
  );
}

/** The six digits, as two groups of three tiles. Read out as one number. */
export function SasDigits({ sas }: { readonly sas: string }) {
  const digits = Array.from(sas);
  const half = Math.ceil(digits.length / 2);
  const group = (part: string[], offset: number) => (
    <span className='flex gap-1.5' aria-hidden='true'>
      {part.map((digit, index) => (
        // A digit's place is its identity: the six never reorder.
        // eslint-disable-next-line react/no-array-index-key
        <span key={offset + index} className='xc-sas-digit'>
          {digit}
        </span>
      ))}
    </span>
  );
  return (
    <div
      className='flex items-center justify-center gap-3 py-2'
      data-testid='sas-digits'
      role='img'
      aria-label={sas}
    >
      {group(digits.slice(0, half), 0)}
      {group(digits.slice(half), half)}
    </div>
  );
}

/** One labelled fact on a card: a small mono label over its value. */
export function Fact({
  label,
  children,
  mono = false,
}: {
  readonly label: ReactNode;
  readonly children: ReactNode;
  readonly mono?: boolean;
}) {
  return (
    <div className='min-w-0 space-y-0.5'>
      <dt className='xc-label'>{label}</dt>
      <dd
        className={`text-foreground break-words ${mono ? 'font-mono text-xs break-all' : 'text-sm'}`}
      >
        {children}
      </dd>
    </div>
  );
}

/** Whether a connection is listening right now. */
export function LivePill({ live }: { readonly live: boolean }) {
  return (
    <span className='xc-pill' data-live={live}>
      <span className='xc-pill-dot' aria-hidden='true' />
      {live ? (
        <Trans>listening for requests</Trans>
      ) : (
        <Trans>not listening</Trans>
      )}
    </span>
  );
}
