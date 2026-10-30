// SPDX-License-Identifier: AGPL-3.0-or-later
// A partial offer as a QR code, to scan with Klimper's Scan tab (TASK-164).
//
// The text's encoding and why: `qrData.ts`.

import StyledQRCode from '@/components/StyledQrCode';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Trans } from '@lingui/react/macro';
import { QrCode } from 'lucide-react';
import { Options } from 'qr-code-styling';
import { useState } from 'react';
import { partialOfferQrData } from './qrData';

// Module constants: StyledQRCode redraws whenever one of these objects changes identity.
const QR_OPTIONS: Options['qrOptions'] = {
  typeNumber: 0,
  errorCorrectionLevel: 'L',
  mode: 'Alphanumeric',
};
const DOTS: Options['dotsOptions'] = { type: 'square', color: '#000000' };
const BACKGROUND: Options['backgroundOptions'] = { color: '#ffffff' };
const NO_IMAGE: Options['imageOptions'] = { hideBackgroundDots: false };

export function PartialOfferQr({ offer }: { offer: string }) {
  const data = partialOfferQrData(offer);
  if (data === null) {
    return (
      <p className='text-xs text-muted-foreground'>
        <Trans>
          This offer is too long for a QR code. Copy the text instead.
        </Trans>
      </p>
    );
  }
  return (
    <div className='mx-auto aspect-square w-full max-w-[420px] rounded-md bg-white p-2'>
      <StyledQRCode
        data={data}
        width={1024}
        height={1024}
        margin={16}
        qrOptions={QR_OPTIONS}
        dotsOptions={DOTS}
        backgroundOptions={BACKGROUND}
        imageOptions={NO_IMAGE}
      />
    </div>
  );
}

/** A button that shows the offer's QR code in a dialog. */
export function PartialOfferQrButton({ offer }: { offer: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button size='sm' variant='outline' onClick={() => setOpen(true)}>
        <QrCode className='mr-1 h-4 w-4' aria-hidden='true' />
        <Trans>QR code</Trans>
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              <Trans>Scan with Klimper</Trans>
            </DialogTitle>
            <DialogDescription>
              <Trans>
                Open Scan in Klimper on the other device and point it at this
                code.
              </Trans>
            </DialogDescription>
          </DialogHeader>
          <PartialOfferQr offer={offer} />
        </DialogContent>
      </Dialog>
    </>
  );
}
