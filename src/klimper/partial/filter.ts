// SPDX-License-Identifier: AGPL-3.0-or-later
// Sage's offer filter (all, active, completed, cancelled, expired) applied to partial offers,
// so one filter on the Offers screen covers both groups.

import { PartialStatus } from './api';

export function matchesOfferFilter(
  status: PartialStatus,
  filter: string,
): boolean {
  switch (filter) {
    case 'active':
      // Still on offer, or not known to have ended (offline, or a cancel not confirmed).
      return (
        status === 'open' ||
        status === 'partly_filled' ||
        status === 'cancelling' ||
        status === 'unknown'
      );
    case 'completed':
      return status === 'filled';
    case 'cancelled':
      return status === 'cancelled';
    case 'expired':
      return status === 'expired';
    case 'all':
    default:
      return true;
  }
}
