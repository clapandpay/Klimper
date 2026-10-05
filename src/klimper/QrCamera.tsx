// SPDX-License-Identifier: AGPL-3.0-or-later
// A live QR reader for the computer's camera.
//
// On a phone the native scanner (`/scan`, `@tauri-apps/plugin-barcode-scanner`) reads
// codes; it has no desktop implementation, so on a computer the webview's own camera is
// read here, frame by frame, with the same `jsQR` decoder that reads pictures
// (`src/lib/scanImage.ts`).
//
// Frames never leave this component: nothing is uploaded, stored or logged, and the
// decoded text goes to `onCode` only — a pairing code is a secret (spec 6.2). The camera
// is released the moment a code is read, the component unmounts, or the user stops it.

import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import jsQR from 'jsqr';
import { CameraOff } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

/** Longest edge a frame is scaled to before decoding: enough for a QR on a screen. */
const DECODE_EDGE = 640;
/** Time between decodes. ~7 a second is instant to a person and cheap on a laptop. */
const DECODE_INTERVAL_MS = 140;

export type CameraProblem = 'unsupported' | 'denied' | 'unavailable';

/** True when this webview can be asked for a camera at all. */
export function cameraSupported(): boolean {
  return (
    typeof navigator !== 'undefined' &&
    typeof navigator.mediaDevices?.getUserMedia === 'function'
  );
}

export function cameraProblemMessage(problem: CameraProblem): string {
  switch (problem) {
    case 'denied':
      return t`Klimper may not use the camera. Allow it in the system settings, or paste the code instead.`;
    case 'unsupported':
      return t`This device cannot share its camera with Klimper. Paste the code or choose a picture of it.`;
    case 'unavailable':
    default:
      return t`No camera could be opened. Paste the code or choose a picture of it.`;
  }
}

function problemOf(error: unknown): CameraProblem {
  const name = error instanceof DOMException ? error.name : '';
  if (name === 'NotAllowedError' || name === 'SecurityError') return 'denied';
  return 'unavailable';
}

export function QrCamera({
  onCode,
  onProblem,
}: {
  readonly onCode: (text: string) => void;
  readonly onProblem: (problem: CameraProblem) => void;
}) {
  const video = useRef<HTMLVideoElement>(null);
  const [live, setLive] = useState(false);
  // The latest callbacks, so the camera is not restarted when a parent re-renders.
  const handlers = useRef({ onCode, onProblem });
  handlers.current = { onCode, onProblem };

  useEffect(() => {
    if (!cameraSupported()) {
      handlers.current.onProblem('unsupported');
      return;
    }
    let stream: MediaStream | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let stopped = false;
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d', { willReadFrequently: true });

    const stop = () => {
      stopped = true;
      if (timer !== null) clearTimeout(timer);
      stream?.getTracks().forEach((track) => track.stop());
      stream = null;
    };

    const decode = () => {
      if (stopped) return;
      const element = video.current;
      if (
        element !== null &&
        context !== null &&
        element.readyState >= element.HAVE_CURRENT_DATA &&
        element.videoWidth > 0
      ) {
        const scale = Math.min(
          1,
          DECODE_EDGE / Math.max(element.videoWidth, element.videoHeight),
        );
        const width = Math.round(element.videoWidth * scale);
        const height = Math.round(element.videoHeight * scale);
        canvas.width = width;
        canvas.height = height;
        context.drawImage(element, 0, 0, width, height);
        const frame = context.getImageData(0, 0, width, height);
        const result = jsQR(frame.data, width, height, {
          inversionAttempts: 'attemptBoth',
        });
        if (result !== null && result.data !== '') {
          stop();
          handlers.current.onCode(result.data);
          return;
        }
      }
      timer = setTimeout(decode, DECODE_INTERVAL_MS);
    };

    navigator.mediaDevices
      .getUserMedia({
        audio: false,
        video: {
          facingMode: 'environment',
          width: { ideal: 1280 },
          height: { ideal: 720 },
        },
      })
      .then(async (opened) => {
        if (stopped) {
          opened.getTracks().forEach((track) => track.stop());
          return;
        }
        stream = opened;
        const element = video.current;
        if (element === null) return;
        element.srcObject = opened;
        await element.play();
        setLive(true);
        decode();
      })
      .catch((error: unknown) => {
        if (!stopped) handlers.current.onProblem(problemOf(error));
      });

    return stop;
  }, []);

  return (
    <div
      className='xc-viewfinder relative mx-auto aspect-square w-full max-w-[18rem] overflow-hidden rounded-[1.25rem] bg-black'
      data-testid='qr-camera'
    >
      <video
        ref={video}
        className='absolute inset-0 h-full w-full object-cover'
        muted
        playsInline
        aria-label={t`Camera view`}
      />
      <span className='xc-corner xc-corner-tl' aria-hidden='true' />
      <span className='xc-corner xc-corner-tr' aria-hidden='true' />
      <span className='xc-corner xc-corner-bl' aria-hidden='true' />
      <span className='xc-corner xc-corner-br' aria-hidden='true' />
      {!live && (
        <div className='absolute inset-0 flex items-center justify-center text-sm text-white/80'>
          <Trans>Opening the camera…</Trans>
        </div>
      )}
    </div>
  );
}

/** What the viewfinder shows when there is no camera to show. */
export function QrCameraOff({ message }: { readonly message: string }) {
  return (
    <div className='xc-viewfinder relative mx-auto flex aspect-square w-full max-w-[18rem] flex-col items-center justify-center gap-3 rounded-[1.25rem] border border-dashed border-border p-6 text-center text-sm text-muted-foreground'>
      <CameraOff className='h-6 w-6' aria-hidden='true' />
      <span>{message}</span>
    </div>
  );
}
