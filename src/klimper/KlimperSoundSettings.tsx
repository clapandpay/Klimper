// SPDX-License-Identifier: AGPL-3.0-or-later
// The Settings hook point for the Klimper sound (decision-009): one element in Settings.tsx,
// nothing when the flag is off, the real card (src/klimper/sound/) loaded lazily when it is on.

import { klimperSoundEnabled } from '@/lib/features';
import { lazy, Suspense } from 'react';

const Card = klimperSoundEnabled
  ? lazy(() => import('./sound/KlimperSoundSettings'))
  : null;

export function KlimperSoundSettings() {
  if (!Card) return null;
  return (
    <Suspense fallback={null}>
      <Card />
    </Suspense>
  );
}
