// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Gnosis Pay onboarding in Klimper (TASK-150): each step shows its one action, the identity
// check opens outside the wallet, and waiting steps poll.

import {
  act,
  cleanup,
  fireEvent,
  screen,
  waitFor,
} from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { Step } from './card';

const openUrl = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }));

const api = vi.hoisted(() => ({
  onboarding: vi.fn(),
  sendEmailCode: vi.fn(),
  register: vi.fn(),
  acceptTerms: vi.fn(),
  answerSourceOfFunds: vi.fn(),
  createAccount: vi.fn(),
  sendPhoneCode: vi.fn(),
  verifyPhone: vi.fn(),
  orderCard: vi.fn(),
}));
vi.mock('./card', async (original) => ({
  ...(await original<typeof import('./card')>()),
  card: api,
}));

const { renderScreen } = await import('../testHarness');
const { CardHome, POLL_MS } = await import('./CardHome');

const signedOut = vi.fn();

function show(...steps: Step[]) {
  for (const step of steps) api.onboarding.mockResolvedValueOnce(step);
  return renderScreen(
    <CardHome address='0x0' onSignOut={vi.fn()} onSignedOut={signedOut} />,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  for (const fn of Object.values(api)) fn.mockResolvedValue(null);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it('registers with an e-mail code', async () => {
  api.sendEmailCode.mockResolvedValue(true);
  show(
    { step: 'register' },
    { step: 'register' },
    { step: 'acceptTerms', terms: [] },
  );
  fireEvent.change(await screen.findByLabelText('E-mail'), {
    target: { value: 'someone@example.org' },
  });
  fireEvent.click(screen.getByTestId('card-send-email-code'));
  fireEvent.change(
    await screen.findByLabelText('The six-digit code from the e-mail'),
    {
      target: { value: '12a3456' },
    },
  );
  fireEvent.click(screen.getByTestId('card-register'));
  await screen.findByTestId('card-step-acceptTerms');
  expect(api.register).toHaveBeenCalledWith('someone@example.org', '123456');
});

it('registers without a code when the programme needs none', async () => {
  api.sendEmailCode.mockResolvedValue(false);
  show({ step: 'register' }, { step: 'acceptTerms', terms: [] });
  fireEvent.change(await screen.findByLabelText('E-mail'), {
    target: { value: 'someone@example.org' },
  });
  fireEvent.click(screen.getByTestId('card-send-email-code'));
  await screen.findByTestId('card-step-acceptTerms');
  expect(api.register).toHaveBeenCalledWith('someone@example.org', null);
});

it('accepts the terms only after the box is ticked', async () => {
  const terms = [
    {
      type: 'general-tos',
      version: 'V1',
      name: 'Terms',
      url: 'https://legal.example',
      accepted: false,
    },
  ];
  show({ step: 'acceptTerms', terms }, { step: 'waitingKyc' });
  const agree = await screen.findByTestId('card-accept-terms');
  expect((agree as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByText('Terms'));
  expect(openUrl).toHaveBeenCalledWith('https://legal.example');
  fireEvent.click(screen.getByTestId('card-terms-agree'));
  fireEvent.click(agree);
  await screen.findByTestId('card-step-waitingKyc');
  expect(api.acceptTerms).toHaveBeenCalledWith(terms);
});

it('opens the identity check in the browser and polls until it is done', async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  show(
    {
      step: 'completeKyc',
      url: 'https://in.sumsub.com/websdk/p/x',
      resubmission: false,
    },
    { step: 'kycReview' },
    { step: 'sourceOfFunds', questions: [] },
  );
  fireEvent.click(await screen.findByTestId('card-open-kyc'));
  expect(openUrl).toHaveBeenCalledWith('https://in.sumsub.com/websdk/p/x');
  await act(() => vi.advanceTimersByTimeAsync(POLL_MS));
  await screen.findByTestId('card-step-kycReview');
  await act(() => vi.advanceTimersByTimeAsync(POLL_MS));
  await screen.findByTestId('card-step-sourceOfFunds');
});

it('without a Sumsub link there is nothing to open', async () => {
  show({ step: 'completeKyc', url: null, resubmission: true });
  await screen.findByText('Gnosis Pay needs another look at your documents');
  expect(screen.queryByTestId('card-open-kyc')).toBeNull();
});

it('sends the source-of-funds answers by question text', async () => {
  show(
    {
      step: 'sourceOfFunds',
      questions: [
        { id: 'q1', question: 'Income?', answers: ['Work', 'Savings'] },
        { id: 'q2', question: 'Monthly?', answers: ['Little', 'Much'] },
      ],
    },
    { step: 'createAccount' },
  );
  fireEvent.click(await screen.findByLabelText('Savings'));
  const send = screen.getByTestId('card-answer-sof') as HTMLButtonElement;
  expect(send.disabled).toBe(true);
  fireEvent.click(screen.getByLabelText('Little'));
  fireEvent.click(send);
  await screen.findByTestId('card-step-createAccount');
  expect(api.answerSourceOfFunds).toHaveBeenCalledWith([
    { question: 'Income?', answer: 'Savings' },
    { question: 'Monthly?', answer: 'Little' },
  ]);
});

it('opens the account, verifies the phone and orders the card', async () => {
  show(
    { step: 'createAccount' },
    { step: 'verifyPhone' },
    { step: 'verifyPhone' },
    { step: 'completed' },
  );
  fireEvent.click(await screen.findByTestId('card-create-account'));
  fireEvent.change(
    await screen.findByLabelText('Phone, with the country code'),
    {
      target: { value: '+43 123 4567890' },
    },
  );
  fireEvent.click(screen.getByTestId('card-send-phone-code'));
  fireEvent.change(
    await screen.findByLabelText('The six-digit code from the text message'),
    { target: { value: '123456' } },
  );
  fireEvent.click(screen.getByTestId('card-verify-phone'));
  fireEvent.click(await screen.findByTestId('card-order'));
  await waitFor(() => expect(api.orderCard).toHaveBeenCalledWith('Klimper'));
  expect(api.sendPhoneCode).toHaveBeenCalledWith('+43 123 4567890');
  expect(api.verifyPhone).toHaveBeenCalledWith('123456');
});

it('shows an error from Gnosis Pay and stays on the step', async () => {
  api.createAccount.mockRejectedValue({
    kind: 'api',
    message: 'Gnosis Pay answered 409 (ACCOUNT_ALREADY_EXISTS)',
    code: 'ACCOUNT_ALREADY_EXISTS',
  });
  show({ step: 'createAccount' });
  fireEvent.click(await screen.findByTestId('card-create-account'));
  expect((await screen.findByTestId('card-step-error')).textContent).toContain(
    'ACCOUNT_ALREADY_EXISTS',
  );
  expect(screen.getByTestId('card-step-createAccount')).toBeTruthy();
});

it('a rejection says whose decision it is, nothing about the person', async () => {
  show({ step: 'rejected' });
  await screen.findByText('Gnosis Pay cannot offer you a card');
});

it('goes back to the sign-in when the session has ended', async () => {
  api.onboarding.mockRejectedValue({
    kind: 'signedOut',
    message: 'x',
    code: null,
  });
  renderScreen(
    <CardHome address='0x0' onSignOut={vi.fn()} onSignedOut={signedOut} />,
  );
  await waitFor(() => expect(signedOut).toHaveBeenCalled());
});
