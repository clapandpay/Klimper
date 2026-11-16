// SPDX-License-Identifier: AGPL-3.0-or-later
// Test XCH from Chia's official testnet11 faucet, on testnet only. The faucet asks for a
// reCAPTCHA, so the wallet can't request coins itself: the button copies the receive address
// and opens the faucet in the system browser, where the address is pasted.
//
// The faucet's URL is fixed here, never taken from settings or links, and the page opens
// outside the app, so the faucet never gets the wallet's commands. Mainnet is left out:
// its faucet asks for the master public key, which reveals every address of the wallet.

import { useNetwork } from '@/hooks/useNetwork';
import { useWalletState } from '@/state';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Droplets } from 'lucide-react';
import { toast } from 'react-toastify';
import { Button } from '@/components/ui/button';
import { Hint } from './Hint';

export const TESTNET_FAUCET_URL = 'https://testnet11-faucet.chia.net/';

/** A testnet11 address the faucet can pay. */
export function isTestnetAddress(address: string | null | undefined): boolean {
  return /^txch1[02-9ac-hj-np-z]{6,}$/.test(address ?? '');
}

export function TestnetFaucet() {
  const { network } = useNetwork();
  const { receive_address } = useWalletState().sync;

  if (network !== 'testnet' || !isTestnetAddress(receive_address)) return null;

  const getTestXch = async () => {
    try {
      await writeText(receive_address);
      toast.info(t`Address copied. Paste it into the faucet.`);
    } catch {
      toast.info(t`Copy your receive address and paste it into the faucet.`);
    }
    await openUrl(TESTNET_FAUCET_URL).catch(() =>
      toast.error(t`The faucet could not be opened.`),
    );
  };

  return (
    <div className='mb-4 flex items-center gap-2'>
      <Button variant='outline' onClick={getTestXch}>
        <Droplets className='mr-2 h-4 w-4' aria-hidden='true' />
        <Trans>Get test XCH</Trans>
      </Button>
      <Hint label={t`About the faucet`}>
        <Trans>
          Opens Chia&apos;s testnet faucet in your browser with your address
          copied. Test XCH has no value. Coins arrive after a short queue.
        </Trans>
      </Hint>
    </div>
  );
}
