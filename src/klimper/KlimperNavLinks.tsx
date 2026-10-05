// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper's own entries in the main navigation: Scan, and the connected websites.
//
// `Nav.tsx` is shared with upstream Sage, so it carries one element and no condition;
// every flag is checked here (`docs/UPSTREAM.md`). Connecting a website used to be
// reachable from Settings alone. It is in the navigation now, because the moment a user
// needs it is the moment a website is showing them a code.

import { NavLink } from '@/components/NavLink';
import { scanEnabled, xchonnectEnabled } from '@/lib/features';
import { Trans } from '@lingui/react/macro';
import { Globe, ScanLine } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { SCANNER_ROUTE, ScannerState } from './Scanner';
import { XCHONNECT_ROUTES } from './xchonnectLinkHandoff';

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
        <ScanNavLink isCollapsed={isCollapsed} className={className} />
      )}
      {xchonnectEnabled && (
        <NavLink
          url={XCHONNECT_ROUTES.dapps}
          isCollapsed={isCollapsed}
          message={<Trans>Websites</Trans>}
        >
          <Globe className={className} aria-hidden='true' />
        </NavLink>
      )}
    </>
  );
}

/** A button rather than a link, so the tap can carry `start` and open the camera at once. */
function ScanNavLink({ isCollapsed, className }: KlimperNavLinksProps) {
  const navigate = useNavigate();
  const state: ScannerState = { start: true };

  return (
    <NavLink
      url={() => navigate(SCANNER_ROUTE, { state })}
      isCollapsed={isCollapsed}
      message={<Trans>Scan</Trans>}
    >
      <ScanLine className={className} aria-hidden='true' />
    </NavLink>
  );
}
