// SPDX-License-Identifier: AGPL-3.0-or-later
// The Xchonnect routes, as one element (clapandpay TASK-072, TASK-080, TASK-083,
// TASK-084).
//
// `App.tsx` is shared with upstream Sage, so the whole feature is one import and one
// `{xchonnectRoutes}` there rather than a route block per screen: the fewer lines we own
// in a shared file, the smaller the next upstream merge (`docs/UPSTREAM.md`).
//
// `createRoutesFromElements` walks into fragments, so this is a plain element and not a
// component — a component would be a route element, not a group of routes.

import { featureElement } from '@/lib/featureElement';
import { xchonnectEnabled } from '@/lib/features';
import { Route } from 'react-router-dom';
import XchonnectApproval from './XchonnectApproval';
import XchonnectConnect from './XchonnectConnect';
import XchonnectDapps from './XchonnectDapps';
import XchonnectDappSettings from './XchonnectDappSettings';

export const xchonnectRoutes = (
  <>
    <Route
      path='/xchonnect'
      element={featureElement(xchonnectEnabled, <XchonnectDapps />)}
    />
    <Route
      path='/xchonnect/connect'
      element={featureElement(xchonnectEnabled, <XchonnectConnect />)}
    />
    {/* The path the first screens used; kept so an older note or link still works. */}
    <Route
      path='/xchonnect/pair'
      element={featureElement(xchonnectEnabled, <XchonnectConnect />)}
    />
    <Route
      path='/xchonnect/request'
      element={featureElement(xchonnectEnabled, <XchonnectApproval />)}
    />
    <Route
      path='/xchonnect/dapp/:sessionId'
      element={featureElement(xchonnectEnabled, <XchonnectDappSettings />)}
    />
  </>
);
