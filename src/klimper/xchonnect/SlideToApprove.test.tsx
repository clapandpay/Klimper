// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Slide to approve: no tap, long press or slow click approves; a full slide does, and so does
// the separate press for keyboards and assistive technology.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SlideToApprove } from './SlideToApprove';

afterEach(cleanup);

// jsdom has no PointerEvent: a MouseEvent that carries the pointer fields the slider reads.
if (typeof window.PointerEvent === 'undefined') {
  class PointerEventStub extends MouseEvent {
    pointerId: number;
    isPrimary: boolean;
    pointerType: string;
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 0;
      this.isPrimary = init.isPrimary ?? false;
      this.pointerType = init.pointerType ?? 'touch';
    }
  }
  window.PointerEvent = PointerEventStub as unknown as typeof PointerEvent;
}

// jsdom has no layout: a 300 px track and a 52 px knob leave 240 px of travel.
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(300);
  vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(52);
});

function slider(onConfirm: () => void, disabled = false, resetKey = 'a') {
  const view = render(
    <SlideToApprove
      label='Approve'
      hint='Slide to approve'
      onConfirm={onConfirm}
      disabled={disabled}
      resetKey={resetKey}
      testId='slide'
    />,
  );
  return { knob: screen.getByTestId('slide-knob'), view };
}

const down = (el: Element, x = 0) =>
  fireEvent.pointerDown(el, {
    pointerId: 1,
    isPrimary: true,
    button: 0,
    clientX: x,
  });
const move = (el: Element, x: number, y = 0) =>
  fireEvent.pointerMove(el, { pointerId: 1, clientX: x, clientY: y });
const up = (el: Element) => fireEvent.pointerUp(el, { pointerId: 1 });

describe('slide to approve', () => {
  it('approves after a full slide', () => {
    const onConfirm = vi.fn();
    const { knob } = slider(onConfirm);
    down(knob);
    move(knob, 239);
    up(knob);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('springs back from a short or abandoned slide', () => {
    const onConfirm = vi.fn();
    const { knob } = slider(onConfirm);
    down(knob);
    move(knob, 120);
    up(knob);
    down(knob);
    move(knob, 239);
    move(knob, 30); // changed their mind
    up(knob);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('ignores a tap, a long press and a slow click on the knob', () => {
    const onConfirm = vi.fn();
    const { knob } = slider(onConfirm);
    down(knob);
    up(knob);
    fireEvent.click(knob, { detail: 1, timeStamp: 5_000 });
    fireEvent.click(knob, { detail: 0 });
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('does not count a slide that began on the request before', () => {
    const onConfirm = vi.fn();
    const { knob, view } = slider(onConfirm, false, 'a');
    down(knob);
    move(knob, 239);
    view.rerender(
      <SlideToApprove
        label='Approve'
        hint='Slide to approve'
        onConfirm={onConfirm}
        resetKey='b'
        testId='slide'
      />,
    );
    up(knob);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('approves from the keyboard and assistive press, once', () => {
    const onConfirm = vi.fn();
    slider(onConfirm);
    const press = screen.getByRole('button', { name: 'Approve' });
    fireEvent.click(press);
    fireEvent.click(press);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('takes a finger anywhere on the track, but only a slide counts', () => {
    const onConfirm = vi.fn();
    slider(onConfirm);
    const track = screen.getByTestId('slide');
    down(track, 200);
    up(track);
    expect(onConfirm).not.toHaveBeenCalled();
    down(track, 10);
    move(track, 249);
    up(track);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('shows when letting go approves', () => {
    const { knob } = slider(vi.fn());
    const track = screen.getByTestId('slide');
    down(knob);
    move(knob, 200);
    expect(track.hasAttribute('data-armed')).toBe(false);
    move(knob, 230);
    expect(track.hasAttribute('data-armed')).toBe(true);
    move(knob, 100);
    expect(track.hasAttribute('data-armed')).toBe(false);
  });

  it('lets go when the finger turns into a vertical scroll', () => {
    const onConfirm = vi.fn();
    const { knob } = slider(onConfirm);
    down(knob);
    move(knob, 120, 10);
    move(knob, 239, 90);
    up(knob);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('says why it is off', () => {
    render(
      <SlideToApprove
        label='Approve'
        hint='Slide to approve'
        offHint='Time is up'
        onConfirm={vi.fn()}
        disabled
      />,
    );
    expect(screen.getByText('Time is up')).toBeTruthy();
    expect(screen.queryByText('Slide to approve')).toBeNull();
  });

  it('does nothing while turned off', () => {
    const onConfirm = vi.fn();
    const { knob } = slider(onConfirm, true);
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));
    down(knob);
    move(knob, 239);
    up(knob);
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
