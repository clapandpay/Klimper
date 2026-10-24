// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper's own entries in the main navigation: Scan, the connected websites, and the card.
//
// `Nav.tsx` is shared with upstream Sage, so it carries one element and no condition;
// every flag is checked here (`docs/UPSTREAM.md`). Connecting a website used to be
// reachable from Settings alone. It is in the navigation now, because the moment a user
// needs it is the moment a website is showing them a code.
//
// "Websites" carries the number of requests waiting for the user (TASK-125.04), and then leads
// to them: a request put off with "Later", or one that arrived while Klimper was in the
// background, is one tap away from every screen.

import { NavLink } from '@/components/NavLink';
import {
  gnosisPayEnabled,
  scanEnabled,
  xchonnectEnabled,
} from '@/lib/features';
import { Plural, Trans } from '@lingui/react/macro';
import { CreditCard, Globe, ScanLine } from 'lucide-react';
import { CARD_ROUTE } from './card/card';
import { SCANNER_ROUTE } from './Scanner';
import { useXchonnectApprovals } from './xchonnect/useXchonnectApprovals';
import { XCHONNECT_ROUTES } from './xchonnect/xchonnectLinkHandoff';

interface KlimperNavLinksProps {
  isCollapsed?: boolean;
  className: string;
}

export function KlimperNavLinks({
  isCollapsed,
  className,
}: KlimperNavLinksProps) {
  return (
    <>
      {scanEnabled && (
        <NavLink
          url={SCANNER_ROUTE}
          isCollapsed={isCollapsed}
          message={<Trans>Scan</Trans>}
        >
          <ScanLine className={className} aria-hidden='true' />
        </NavLink>
      )}
      {xchonnectEnabled && (
        <WebsitesLink isCollapsed={isCollapsed} className={className} />
      )}
      {gnosisPayEnabled && (
        <NavLink
          url={CARD_ROUTE}
          isCollapsed={isCollapsed}
          message={<Trans>Card</Trans>}
        >
          <CreditCard className={className} aria-hidden='true' />
        </NavLink>
      )}
    </>
  );
}

function WebsitesLink({ isCollapsed, className }: KlimperNavLinksProps) {
  const { approvals } = useXchonnectApprovals();
  const waiting = approvals.length;
  const first = approvals[0];
  return (
    <NavLink
      url={
        first
          ? `${XCHONNECT_ROUTES.request}/${first.id}`
          : XCHONNECT_ROUTES.dapps
      }
      isCollapsed={isCollapsed}
      message={<Trans>Websites</Trans>}
    >
      <span className='relative inline-flex'>
        <Globe className={className} aria-hidden='true' />
        {waiting > 0 && (
          <span
            className='absolute -right-1.5 -top-1.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-[10px] font-semibold leading-none text-destructive-foreground'
            aria-hidden='true'
            data-testid='requests-badge'
          >
            {waiting > 9 ? '9+' : waiting}
          </span>
        )}
        {waiting > 0 && (
          <span className='sr-only'>
            <Plural
              value={waiting}
              one='# request waiting'
              other='# requests waiting'
            />
          </span>
        )}
      </span>
    </NavLink>
  );
}
