// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Slide to approve: a tap approves nothing, a deliberate press or a full slide does.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SlideToApprove } from './SlideToApprove';

afterEach(cleanup);

function slider(onConfirm: () => void, disabled = false) {
  render(
    <SlideToApprove
      label='Approve'
      hint='Slide to approve'
      onConfirm={onConfirm}
      disabled={disabled}
    />,
  );
  return screen.getByRole('button', { name: 'Approve' });
}

describe('slide to approve', () => {
  it('ignores a tap on the knob', () => {
    const onConfirm = vi.fn();
    fireEvent.click(slider(onConfirm), { detail: 1 });
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('approves on a keyboard or assistive press, once', () => {
    const onConfirm = vi.fn();
    const knob = slider(onConfirm);
    fireEvent.click(knob, { detail: 0 });
    fireEvent.click(knob, { detail: 0 });
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('does nothing while turned off', () => {
    const onConfirm = vi.fn();
    fireEvent.click(slider(onConfirm, true), { detail: 0 });
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
