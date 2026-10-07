// SPDX-License-Identifier: AGPL-3.0-or-later
// The one switch (clapandpay TASK-111.03): on unless the user turned it off. Kept in
// localStorage like Klimper's other look-and-feel choices; a wallet backup carries no sound
// preference and does not need to.

import { useLocalStorage } from 'usehooks-ts';

export const SOUND_SETTING_KEY = 'klimper.sound';

export function useKlimperSoundSetting() {
  return useLocalStorage<boolean>(SOUND_SETTING_KEY, true);
}
