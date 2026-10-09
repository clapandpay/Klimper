// SPDX-License-Identifier: AGPL-3.0-or-later
// Slide to approve: a glass track and a sun knob that has to be carried to the end.
//
// A tap is not an answer here, so a stray thumb on a busy screen approves nothing: the knob
// must travel almost the whole track, and a short slide springs back. The phone ticks as the
// knob passes the half and the point where letting go approves, and confirms with a success
// haptic at the end. One finger at a time; a release counts only for the request the slide
// began on.
//
// The whole track takes the finger, so a thumb that lands beside the knob still carries it,
// but only a slide counts: no tap, long press or slow click approves, and a swipe that turns
// into a vertical scroll lets go. Past the point where letting go approves, the knob turns
// into a check and the track says so, for screens without haptics too. Keyboards and assistive technology (VoiceOver, TalkBack, switch control) get their
// own button named after the action, hidden until it has focus, which approves at once since
// pressing it is deliberate. The two never share an event, so no timing rule is involved.
//
// Light by construction: the drag never re-renders React. It writes one custom property
// (`--p`, 0 to 1) on the track, and CSS moves the knob with a transform and reveals the trail
// with a clip; the travel in pixels (`--travel`) is measured when the knob is taken.

import { Check, ChevronsRight } from 'lucide-react';
import { ReactNode, useEffect, useRef, useState } from 'react';
import { feel } from '../haptics';

/** How far the knob must travel to count, as a share of the track. */
const DONE_AT = 0.92;
const MARKS = [0.5, DONE_AT];
/** A finger that wanders this far up or down was scrolling, not sliding. */
const VERTICAL_LETGO_PX = 64;

export function SlideToApprove({
  label,
  hint,
  releaseHint,
  offHint,
  onConfirm,
  disabled = false,
  resetKey,
  testId,
}: {
  /** The action, e.g. "Approve": the button's name and the text on the track. */
  readonly label: string;
  /** What the track says while waiting, e.g. "Slide to approve". */
  readonly hint: ReactNode;
  /** What the track says once letting go approves, e.g. "Release to approve". */
  readonly releaseHint?: ReactNode;
  /** Why the slider is off, shown instead of `hint` while `disabled`. */
  readonly offHint?: ReactNode;
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
    y: number;
    width: number;
    p: number;
    id: number;
    key: string | undefined;
  } | null>(null);
  // The latest key, read by a release that may land after the request was replaced.
  const currentKey = useRef(resetKey);
  currentKey.current = resetKey;
  const [done, setDone] = useState(false);

  const set = (p: number) => {
    track.current?.style.setProperty('--p', String(p));
    track.current?.toggleAttribute('data-armed', p >= DONE_AT);
  };

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

  const letGo = () => {
    drag.current = null;
    track.current?.removeAttribute('data-dragging');
    set(0);
  };

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (drag.current || !event.isPrimary || event.button !== 0) return;
    if (disabled || done || !track.current || !knob.current) return;
    const width = measure();
    if (width <= 0) return;
    drag.current = {
      x: event.clientX,
      y: event.clientY,
      width,
      p: 0,
      id: event.pointerId,
      key: resetKey,
    };
    track.current.setPointerCapture?.(event.pointerId);
    track.current.setAttribute('data-dragging', '');
    feel('grab');
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== event.pointerId) return;
    if (Math.abs(event.clientY - d.y) > VERTICAL_LETGO_PX) {
      letGo();
      return;
    }
    const p = Math.min(1, Math.max(0, (event.clientX - d.x) / d.width));
    for (const mark of MARKS) {
      if (d.p < mark !== p < mark) feel('tick');
    }
    d.p = p;
    set(p);
  };

  const onPointerEnd = (event: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== event.pointerId) return;
    letGo();
    const same = d.key === currentKey.current;
    if (same && d.p >= DONE_AT && event.type === 'pointerup') finish();
  };

  return (
    <div
      ref={track}
      className='xc-slide'
      data-done={done}
      data-disabled={disabled}
      data-testid={testId}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
    >
      <span className='xc-slide-fill' aria-hidden='true' />
      <span className='xc-slide-label' aria-hidden='true'>
        {disabled && !done && offHint ? offHint : hint}
      </span>
      {releaseHint && (
        <span className='xc-slide-label xc-slide-release' aria-hidden='true'>
          {releaseHint}
        </span>
      )}
      <button
        type='button'
        className='xc-slide-press'
        disabled={disabled || done}
        onClick={finish}
      >
        {label}
      </button>
      <button
        ref={knob}
        type='button'
        className='xc-slide-knob'
        aria-hidden='true'
        tabIndex={-1}
        disabled={disabled}
        data-testid={testId ? `${testId}-knob` : undefined}
      >
        {/* Both drawn; CSS shows the check once letting go approves, without a render. */}
        <ChevronsRight className='xc-slide-go h-6 w-6' aria-hidden='true' />
        <Check className='xc-slide-check h-6 w-6' aria-hidden='true' />
      </button>
    </div>
  );
}
