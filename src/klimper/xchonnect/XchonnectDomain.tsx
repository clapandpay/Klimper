// SPDX-License-Identifier: AGPL-3.0-or-later
// The verified domain, shown the one way it is allowed to be shown (clapandpay TASK-069).
//
// Pairing is where phishing happens, so this block is the same everywhere a domain
// appears — the pairing sheet, the connected-dApps list, the approval screen — and it is
// the only place any of them renders a domain:
//
//   * `domainUnicode` is what the user reads, and it is the largest text in the block;
//   * when `domainDiffers`, the ASCII (A-label) form is shown next to it, because that is
//     what the computer actually resolved;
//   * every entry of `warnings` gets its own line with its own sentence. A warning we do
//     not have copy for is still shown, by its name, rather than silently dropped;
//   * the name from the origin document is labelled as a claim, because only the domain
//     was verified.

import { Trans } from '@lingui/react/macro';
import { AlertTriangle } from 'lucide-react';
import { claimedText } from './xchonnectPrompt';

export interface DomainIdentity {
  readonly domainUnicode: string;
  readonly domainAscii: string;
  readonly domainDiffers: boolean;
  readonly warnings: readonly string[];
  /** Untrusted text from the origin document. */
  readonly dappName: string;
}

/**
 * A session's identity as this block wants it.
 *
 * `SessionSummary` spells the ASCII form `dapp` (it is the session's key everywhere else
 * in the protocol), so the one translation lives here rather than at every call site.
 */
export function identityOfSession(session: {
  readonly dapp: string;
  readonly domainUnicode: string;
  readonly domainDiffers: boolean;
  readonly warnings: readonly string[];
  readonly dappName: string;
}): DomainIdentity {
  return {
    domainUnicode: session.domainUnicode,
    domainAscii: session.dapp,
    domainDiffers: session.domainDiffers,
    warnings: session.warnings,
    dappName: session.dappName,
  };
}

/** The plain-language sentence for one homograph warning from `xchonnect-core`. */
export function DomainWarning({ warning }: { readonly warning: string }) {
  return (
    <div
      className='flex items-start gap-2 text-destructive'
      data-testid='domain-warning'
    >
      <AlertTriangle className='h-4 w-4 mt-0.5 shrink-0' aria-hidden='true' />
      <span>
        {warning === 'NonAscii' ? (
          <Trans>
            This domain uses letters outside the English alphabet. Check it
            character by character.
          </Trans>
        ) : warning === 'MixedScripts' ? (
          <Trans>
            This domain mixes alphabets, which is how look-alike domains are
            built. Check it carefully.
          </Trans>
        ) : (
          // An unknown warning name is still a warning: show it rather than hide it.
          <Trans>Check this domain carefully ({warning}).</Trans>
        )}
      </span>
    </div>
  );
}

/**
 * The domain block.
 *
 * `size='large'` is the pairing sheet, where the domain is the decision; `size='inline'`
 * is a list row or an approval header.
 */
export function XchonnectDomain({
  identity,
  size = 'large',
  showName = true,
}: {
  readonly identity: DomainIdentity;
  readonly size?: 'large' | 'inline';
  readonly showName?: boolean;
}) {
  return (
    <div className='space-y-2'>
      <div
        className={
          size === 'large'
            ? 'text-xl font-semibold break-all'
            : 'font-medium break-all'
        }
        data-testid='domain-unicode'
      >
        {identity.domainUnicode}
      </div>
      {identity.domainDiffers && (
        <div
          className='text-sm text-muted-foreground break-all'
          data-testid='domain-ascii'
        >
          <Trans>Really this domain</Trans>:{' '}
          <span className='font-mono'>{identity.domainAscii}</span>
        </div>
      )}
      {identity.warnings.map((warning) => (
        <DomainWarning key={warning} warning={warning} />
      ))}
      {showName && claimedText(identity.dappName) !== '' && (
        <div className='text-sm text-muted-foreground break-all'>
          <Trans>It calls itself</Trans>{' '}
          <span className='text-foreground' data-testid='claimed-name'>
            {claimedText(identity.dappName)}
          </span>
        </div>
      )}
    </div>
  );
}
