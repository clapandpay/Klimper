// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper's own settings, first in Settings: the coin sound and feel (decision-009, TASK-111)
// and privacy mode. One element in Settings.tsx. The sound engine itself stays in its lazy
// chunk (src/klimper/sound/); this card only reads and writes the two switches.

import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { klimperCoinsEnabled } from '@/lib/features';
import { Trans } from '@lingui/react/macro';
import { Coins, EyeOff } from 'lucide-react';
import { ReactNode } from 'react';
import { onPhone } from './haptics';
import { usePrivacy } from './privacy';
import { useKlimperSoundSetting } from './sound/setting';
import { Hint } from './Hint';

function Row({
  id,
  icon,
  label,
  hint,
  checked,
  disabled = false,
  onChange,
}: {
  readonly id: string;
  readonly icon: ReactNode;
  readonly label: ReactNode;
  readonly hint: ReactNode;
  readonly checked: boolean;
  readonly disabled?: boolean;
  readonly onChange: (on: boolean) => void;
}) {
  return (
    <div className='flex items-center justify-between gap-3 p-4'>
      <div className='flex min-w-0 items-center gap-3'>
        <span className='klimper-setting-icon' aria-hidden='true'>
          {icon}
        </span>
        <Label htmlFor={id} className='text-base font-medium'>
          {label}
        </Label>
        <Hint>{hint}</Hint>
      </div>
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
        data-testid={id}
      />
    </div>
  );
}

export function KlimperSettings() {
  const [sound, setSound] = useKlimperSoundSetting();
  const [privacy, setPrivacy] = usePrivacy();
  return (
    <div className='mb-6 divide-y overflow-hidden rounded-2xl border bg-card text-card-foreground shadow-sm'>
      {klimperCoinsEnabled && (
        <Row
          id='klimper-sound-switch'
          icon={<Coins className='h-5 w-5' />}
          label={<Trans>Enable coins sound and feel</Trans>}
          hint={
            onPhone() ? (
              <Trans>
                A coin rings when coins arrive. Shake or tilt the phone and you
                hear and feel the coins on the open page: the more they are
                worth, the heavier. Silent mode mutes the sound.
              </Trans>
            ) : (
              <Trans>A coin rings when coins arrive.</Trans>
            )
          }
          checked={sound && !privacy}
          disabled={privacy}
          onChange={setSound}
        />
      )}
      <Row
        id='klimper-privacy-switch'
        icon={<EyeOff className='h-5 w-5' />}
        label={<Trans>Privacy mode</Trans>}
        hint={
          <Trans>
            Hides amounts and keeps the coins quiet and still. Confirmations
            still show amounts.
          </Trans>
        }
        checked={privacy}
        onChange={setPrivacy}
      />
    </div>
  );
}
