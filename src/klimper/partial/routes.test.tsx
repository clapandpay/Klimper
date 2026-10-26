// SPDX-License-Identifier: AGPL-3.0-or-later
import { RouteObject } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({ partialOffersEnabled: true }));

const { withPartialOffers } = await import('./routes');

const sage = <div>Sage</div>;
const offers: RouteObject = {
  path: '/offers',
  element: <div />,
  children: [
    { path: '', element: <div /> },
    { path: 'view/:offer', element: sage },
  ],
};

describe('partial offer routes', () => {
  it('add the make screen and route a partial offer before Sage’s page', () => {
    const route = withPartialOffers(offers);
    const paths = route.children?.map((child) => child.path);
    expect(paths).toEqual(['', 'view/:offer', 'make-partial']);
    const view = route.children?.find((child) => child.path === 'view/:offer');
    // Sage's page is kept, behind the switch.
    expect(view?.element).not.toBe(sage);
  });

  it('leave every other route as it is', () => {
    const wallet: RouteObject = { path: '/wallet', element: <div /> };
    expect(withPartialOffers(wallet)).toBe(wallet);
  });
});
