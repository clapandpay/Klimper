// SPDX-License-Identifier: AGPL-3.0-or-later
// The info dot every Klimper screen uses to keep its labels to one line. Styles: `klimper.css`.

import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import { t } from '@lingui/core/macro';
import { Info } from 'lucide-react';
import { ReactNode, useState } from 'react';

/**
 * The longer sentence behind a short label: an info dot that opens on tap (a phone has no
 * hover) and on hover with a mouse. The screens keep one line each and put the why here.
 */
export function Hint({
  children,
  label,
}: {
  readonly children: ReactNode;
  /** What the dot is about, for a screen reader. */
  readonly label?: string;
}) {
  const [open, setOpen] = useState(false);
  // A mouse opens it by hovering; its click must not then toggle it shut again.
  const [byMouse, setByMouse] = useState(false);
  const mouse = (event: React.PointerEvent) => event.pointerType === 'mouse';
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        type='button'
        className='klimper-hint'
        aria-label={label ?? t`More about this`}
        onPointerDown={(event) => setByMouse(mouse(event))}
        onClick={(event) => {
          if (byMouse) event.preventDefault();
          setByMouse(false); // the next press may be the keyboard's
        }}
        onPointerEnter={(event) => mouse(event) && setOpen(true)}
        onPointerLeave={(event) => mouse(event) && setOpen(false)}
      >
        <Info className='h-3.5 w-3.5' aria-hidden='true' />
      </PopoverTrigger>
      <PopoverContent
        className='klimper-hint-body'
        side='top'
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        {children}
      </PopoverContent>
    </Popover>
  );
}
