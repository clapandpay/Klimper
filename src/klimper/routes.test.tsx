// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper's one hook into Sage's router (docs/UPSTREAM.md, "Hook points"): Sage's routes
// pass through untouched except where a feature is switched off, and Klimper's screens are
// added. The flags are mocked: offers, token issuing and NFT minting off, NFTs on.

import { isValidElement } from 'react';
import {
  createMemoryRouter,
  createRoutesFromElements,
  Navigate,
  Route,
} from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({
  nftMintingEnabled: false,
  nftsEnabled: true,
  offersEnabled: false,
  optionsEnabled: true,
  profilesEnabled: true,
  swapEnabled: true,
  themeGalleryEnabled: true,
  tokenIssuingEnabled: false,
  xchonnectEnabled: true,
  scanEnabled: true,
  cardEnabled: true,
}));
vi.mock('./Scanner', () => ({
  scannerRoutes: <Route path='/scanner' element={<div />} />,
}));
vi.mock('./xchonnect/XchonnectRoutes', () => ({
  xchonnectRoutes: <Route path='/xchonnect' element={<div />} />,
}));
vi.mock('./card/CardRoutes', () => ({
  cardRoutes: <Route path='/card' element={<div />} />,
}));

const { withKlimper } = await import('./routes');

const Page = () => null;

/** A slice of Sage's route block, written the way App.tsx writes it. */
const sage = () =>
  createRoutesFromElements(
    <>
      <Route path='/' element={<Page />} />
      <Route path='/wallet' element={<Page />}>
        <Route path='' element={<Page />} />
        <Route path='issue-token' element={<Page />} />
      </Route>
      <Route path='/nfts' element={<Page />}>
        <Route path='' element={<Page />} />
        <Route path='mint' element={<Page />} />
      </Route>
      <Route path='/offers' element={<Page />}>
        <Route path='make' element={<Page />} />
      </Route>
    </>,
  );

const redirects = (element: unknown) =>
  isValidElement(element) && element.type === Navigate;

describe('withKlimper', () => {
  it('turns a switched-off feature into a redirect, nested ones too', () => {
    const routes = withKlimper(sage());
    const byPath = new Map(routes.map((r) => [r.path, r]));
    expect(redirects(byPath.get('/offers')?.element)).toBe(true);
    const issue = byPath
      .get('/wallet')
      ?.children?.find((c) => c.path === 'issue-token');
    expect(redirects(issue?.element)).toBe(true);
  });

  it('shows the NFT gallery but not minting (TASK-138)', () => {
    const nfts = new Map(
      withKlimper(sage())
        .find((r) => r.path === '/nfts')
        ?.children?.map((c) => [c.path, c]),
    );
    expect(redirects(nfts.get('')?.element)).toBe(false);
    expect(redirects(nfts.get('mint')?.element)).toBe(true);
  });

  it('leaves everything else exactly as Sage declared it', () => {
    const original = sage();
    const routes = withKlimper(original);
    const byPath = new Map(routes.map((r) => [r.path, r]));
    expect(redirects(byPath.get('/nfts')?.element)).toBe(false);
    expect(redirects(byPath.get('/')?.element)).toBe(false);
    const home = byPath.get('/wallet')?.children?.find((c) => c.path === '');
    expect(redirects(home?.element)).toBe(false);
    // Sage's own route objects are not mutated.
    expect(redirects(original[3].element)).toBe(false);
  });

  it('adds Klimper’s own screens after Sage’s', () => {
    const paths = withKlimper(sage()).map((r) => r.path);
    expect(paths.slice(0, 4)).toEqual(['/', '/wallet', '/nfts', '/offers']);
    expect(paths).toEqual(
      expect.arrayContaining(['/xchonnect', '/scanner', '/card']),
    );
  });

  it('builds a data router: Sage’s and Klimper’s route ids never collide', () => {
    // Both lists come from separate createRoutesFromElements calls, which number routes from
    // "0" each time; a data router refuses the duplicates and the app renders nothing.
    expect(() => createMemoryRouter(withKlimper(sage()))).not.toThrow();
  });
});
