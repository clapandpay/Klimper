// SPDX-License-Identifier: AGPL-3.0-or-later
// The switch in Settings (clapandpay TASK-111.03). Same card as the other Klimper entries.
// Loaded lazily, with the flag, like the sound itself.

import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Trans } from '@lingui/react/macro';
import { onPhone } from './player';
import { useKlimperSoundSetting } from './setting';

export function KlimperSoundSettings() {
  const [on, setOn] = useKlimperSoundSetting();
  const phone = onPhone();
  return (
    <div className='divide-y rounded-md border bg-card text-card-foreground overflow-hidden'>
      <div className='p-3'>
        <h3 className='text-sm font-medium'>
          <Trans>Klimper sound</Trans>
        </h3>
      </div>
      <div className='p-3'>
        <div className='flex flex-col sm:flex-row sm:items-center justify-between gap-x-4 gap-y-2'>
          <div className='space-y-1'>
            <Label className='text-sm font-medium'>
              <Trans>Coins you can hear</Trans>
            </Label>
            <div className='text-sm text-muted-foreground'>
              {phone ? (
                <Trans>
                  A jingle when coins arrive, and when you shake the phone: the
                  more they are worth, the heavier it sounds and feels. Silent
                  mode mutes it.
                </Trans>
              ) : (
                <Trans>
                  A jingle when coins arrive: the more they are worth, the
                  heavier it sounds. Shaking is for the phone.
                </Trans>
              )}
            </div>
          </div>
          <div className='flex sm:justify-end'>
            <Switch
              checked={on}
              onCheckedChange={setOn}
              aria-label='Klimper sound'
              data-testid='klimper-sound-switch'
            />
          </div>
        </div>
      </div>
    </div>
  );
}

export default KlimperSoundSettings;
