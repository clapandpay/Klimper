// SPDX-License-Identifier: AGPL-3.0-or-later
import { HandleLookup } from '@/hooks/useHandle';
import {
  HandleError,
  HandleErrorCode,
  isHandleInput,
  resolveHandle,
} from '@/lib/handles';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import {
  CheckCircle2Icon,
  LoaderCircleIcon,
  TriangleAlertIcon,
} from 'lucide-react';

function handleErrorMessage(code: HandleErrorCode): string {
  switch (code) {
    case 'invalid':
      return t`A handle is 3 to 63 letters and digits`;
    case 'not_found':
      return t`Nobody has this handle`;
    case 'expired':
      return t`This handle has expired`;
    case 'not_payable':
      return t`This handle cannot receive payments right now`;
    case 'unavailable':
      return t`Handles cannot be looked up right now. Use an address instead`;
  }
}

/**
 * Resolve "@anna" again at submit. The address on screen is the one the user agreed to: if
 * none is shown yet, or the handle now points somewhere else, nothing is sent and the user
 * looks again. `shown` is read after the registry answered, so a live lookup that finished
 * meanwhile counts. Input that is not a handle passes through unchanged.
 */
export async function recheckHandle(
  input: string,
  prefix: string | null,
  shownNow: () => string | null,
): Promise<{ address: string } | { error: string }> {
  if (!prefix || !isHandleInput(input)) return { address: input };
  let address: string;
  try {
    address = (await resolveHandle(input, prefix)).address;
  } catch (error) {
    return {
      error: handleErrorMessage(
        error instanceof HandleError ? error.code : 'unavailable',
      ),
    };
  }
  const shown = shownNow();
  if (shown === null) {
    return { error: t`Wait until the address is shown, then try again` };
  }
  if (shown !== address) {
    return {
      error: t`This handle just changed. Check the address and try again`,
    };
  }
  return { address };
}

// Shows what "@anna" stands for, in full: the address is what the money is sent to.
export function HandleResolution({ lookup }: { lookup: HandleLookup }) {
  if (lookup.status === 'idle') return null;

  if (lookup.status === 'loading') {
    return (
      <div className='flex items-center gap-2 text-sm text-muted-foreground'>
        <LoaderCircleIcon className='h-4 w-4 animate-spin' aria-hidden='true' />
        <Trans>Looking up handle…</Trans>
      </div>
    );
  }

  if (lookup.status === 'error') {
    return (
      <div className='flex items-center gap-2 text-sm text-destructive'>
        <TriangleAlertIcon className='h-4 w-4 shrink-0' aria-hidden='true' />
        {handleErrorMessage(lookup.error.code)}
      </div>
    );
  }

  const { handle, address, pendingTransfer } = lookup.resolved;

  return (
    <div className='space-y-1 text-sm'>
      <div className='flex items-start gap-2'>
        <CheckCircle2Icon
          className='h-4 w-4 mt-0.5 shrink-0 text-green-600'
          aria-hidden='true'
        />
        <div className='min-w-0'>
          <div className='font-medium'>@{handle}</div>
          <div className='font-mono text-xs text-muted-foreground break-all'>
            {address}
          </div>
        </div>
      </div>
      {pendingTransfer && (
        <div className='flex items-center gap-2 text-amber-600'>
          <TriangleAlertIcon className='h-4 w-4 shrink-0' aria-hidden='true' />
          <Trans>
            This handle is changing owner. Check with the recipient.
          </Trans>
        </div>
      )}
    </div>
  );
}
