// SPDX-License-Identifier: AGPL-3.0-or-later
// The card route, as one element (see XchonnectRoutes.tsx for why it is an element). With
// VITE_ENABLE_CARD unset the route redirects to the wallet and the screen never loads.

import { featureElement } from '@/lib/featureElement';
import { cardEnabled } from '@/lib/features';
import { Route } from 'react-router-dom';
import { CARD_ROUTE } from './card';
import CardScreen from './CardScreen';

export const cardRoutes = (
  <Route
    path={CARD_ROUTE}
    element={featureElement(cardEnabled, <CardScreen />)}
  />
);
