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
const move = (el: Element, x: number) =>
  fireEvent.pointerMove(el, { pointerId: 1, clientX: x });
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
