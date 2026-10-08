// SPDX-License-Identifier: AGPL-3.0-or-later
// Slide to approve: a glass track and a sun knob that has to be carried to the end.
//
// A tap is not an answer here, so a stray thumb on a busy screen approves nothing: the knob
// must travel almost the whole track, and a short slide springs back. The phone ticks as the
// knob passes the half and the point where letting go approves, and confirms with a success
// haptic at the end. One finger at a time; a release counts only for the request the slide
// began on.
//
// The knob is a real button named after the action. A press that did not start with a finger
// or the mouse on it (Enter, Space, VoiceOver, TalkBack, a switch control) approves at once,
// since that is deliberate; a tap on it does nothing, only a slide does.
//
// Light by construction: the drag never re-renders React. It writes one custom property
// (`--p`, 0 to 1) on the track, and CSS moves the knob with a transform and reveals the trail
// with a clip; the travel in pixels (`--travel`) is measured when the knob is taken.

import { Check, ChevronsRight } from 'lucide-react';
import { ReactNode, useEffect, useRef, useState } from 'react';
import { feel } from './haptics';

/** How far the knob must travel to count, as a share of the track. */
const DONE_AT = 0.92;
const MARKS = [0.5, DONE_AT];
/** A click this soon after a finger or the mouse touched the knob is that touch, not a press. */
const TOUCHED_MS = 700;

export function SlideToApprove({
  label,
  hint,
  onConfirm,
  disabled = false,
  resetKey,
  testId,
}: {
  /** The action, e.g. "Approve": the button's name and the text on the track. */
  readonly label: string;
  /** What the track says while waiting, e.g. "Slide to approve". */
  readonly hint: ReactNode;
  readonly onConfirm: () => void;
  readonly disabled?: boolean;
  /** A new value puts the knob back at the start (the next request). */
  readonly resetKey?: string;
  readonly testId?: string;
}) {
  const track = useRef<HTMLDivElement>(null);
  const knob = useRef<HTMLButtonElement>(null);
  const drag = useRef<{
    x: number;
    width: number;
    p: number;
    id: number;
    key: string | undefined;
  } | null>(null);
  const touched = useRef(0);
  // The latest key, read by a release that may land after the request was replaced.
  const currentKey = useRef(resetKey);
  currentKey.current = resetKey;
  const [done, setDone] = useState(false);

  const set = (p: number) => track.current?.style.setProperty('--p', String(p));

  /** How far the knob can go, in pixels; measured when it is taken, never per frame. */
  const measure = () => {
    if (!track.current || !knob.current) return 0;
    const width = track.current.clientWidth - knob.current.offsetWidth - 8;
    track.current.style.setProperty('--travel', `${Math.max(0, width)}px`);
    return width;
  };

  useEffect(() => {
    drag.current = null;
    setDone(false);
    set(0);
  }, [resetKey]);

  // A rotation moves the end of the track: keep a finished knob there.
  useEffect(() => {
    if (!done) return;
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, [done]);

  // Turned off mid-slide (the request was replaced): let go.
  useEffect(() => {
    if (!disabled || done) return;
    drag.current = null;
    track.current?.removeAttribute('data-dragging');
    set(0);
  }, [disabled, done]);

  const finish = () => {
    if (done || disabled) return;
    measure();
    set(1);
    setDone(true);
    feel('done');
    onConfirm();
  };

  const onPointerDown = (event: React.PointerEvent<HTMLButtonElement>) => {
    touched.current = event.timeStamp;
    if (drag.current || !event.isPrimary || event.button !== 0) return;
    if (disabled || done || !track.current || !knob.current) return;
    const width = measure();
    if (width <= 0) return;
    drag.current = {
      x: event.clientX,
      width,
      p: 0,
      id: event.pointerId,
      key: resetKey,
    };
    knob.current.setPointerCapture?.(event.pointerId);
    track.current.setAttribute('data-dragging', '');
    feel('grab');
  };

  const onPointerMove = (event: React.PointerEvent<HTMLButtonElement>) => {
    const d = drag.current;
    if (!d || d.id !== event.pointerId) return;
    const p = Math.min(1, Math.max(0, (event.clientX - d.x) / d.width));
    for (const mark of MARKS) {
      if (d.p < mark !== p < mark) feel('tick');
    }
    d.p = p;
    set(p);
  };

  const onPointerEnd = (event: React.PointerEvent<HTMLButtonElement>) => {
    const d = drag.current;
    if (!d || d.id !== event.pointerId) return;
    drag.current = null;
    track.current?.removeAttribute('data-dragging');
    const same = d.key === currentKey.current;
    if (same && d.p >= DONE_AT && event.type === 'pointerup') finish();
    else set(0);
  };

  return (
    <div
      ref={track}
      className='xc-slide'
      data-done={done}
      data-disabled={disabled}
      data-testid={testId}
    >
      <span className='xc-slide-fill' aria-hidden='true' />
      <span className='xc-slide-label' aria-hidden='true'>
        {hint}
      </span>
      <button
        ref={knob}
        type='button'
        className='xc-slide-knob'
        aria-label={label}
        disabled={disabled}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerEnd}
        onPointerCancel={onPointerEnd}
        onClick={(event) => {
          // Keyboard and assistive presses come without a touch on the knob just before.
          if (event.timeStamp - touched.current > TOUCHED_MS) finish();
        }}
      >
        {done ? (
          <Check className='h-6 w-6' aria-hidden='true' />
        ) : (
          <ChevronsRight className='h-6 w-6' aria-hidden='true' />
        )}
      </button>
    </div>
  );
}
