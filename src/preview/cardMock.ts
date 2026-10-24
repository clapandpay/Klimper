// SPDX-License-Identifier: AGPL-3.0-or-later
// The card plugin for the card preview (`card.tsx`): a small Gnosis Pay in memory that
// walks through the same steps as the fake server in tauri-plugin-klimper-card. All data
// is made up. `?state=` picks where it starts: signin (default), register, terms, kyc,
// review, sof, account, phone, order, active, frozen, nobuild, nowords, rejected.
import { mockIPC } from '@tauri-apps/api/mocks';

const params = new URLSearchParams(location.search);
const start = params.get('state') ?? 'signin';

type Stage =
  | 'register'
  | 'terms'
  | 'kyc'
  | 'review'
  | 'sof'
  | 'account'
  | 'phone'
  | 'completed'
  | 'rejected';

const state = {
  configured: start !== 'nobuild',
  words: start !== 'nowords',
  signedIn: !['signin', 'nobuild', 'nowords'].includes(start),
  stage:
    (
      {
        register: 'register',
        terms: 'terms',
        kyc: 'kyc',
        review: 'review',
        sof: 'sof',
        account: 'account',
        phone: 'phone',
        order: 'completed',
        active: 'completed',
        frozen: 'completed',
        rejected: 'rejected',
      } as Record<string, Stage>
    )[start] ?? ('register' as Stage),
  cardStatus:
    start === 'frozen' ? 'blocked' : start === 'order' ? null : 'active',
  spendable: 123.45,
};

const terms = [
  {
    type: 'general-tos',
    version: 'V1',
    name: 'Gnosis Pay Terms of Service',
    url: 'https://legal.gnosispay.com/',
    accepted: false,
  },
  {
    type: 'card-tos',
    version: 'V1',
    name: 'Cardholder Terms',
    url: 'https://legal.gnosispay.com/',
    accepted: false,
  },
];
const questions = [
  {
    id: 'q1',
    question: 'What is your main source of income?',
    answers: ['Employment', 'Self-employment', 'Savings', 'Other'],
  },
  {
    id: 'q2',
    question: 'How much do you expect to spend each month?',
    answers: ['Up to 1,000 EUR', '1,000 to 5,000 EUR', 'More than 5,000 EUR'],
  },
];

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function step() {
  switch (state.stage) {
    case 'register':
      return { step: 'register' };
    case 'terms':
      return { step: 'acceptTerms', terms };
    case 'kyc':
      return {
        step: 'completeKyc',
        url: 'https://in.sumsub.com/websdk/p/sbx_preview',
        resubmission: false,
      };
    case 'review':
      return { step: 'kycReview' };
    case 'sof':
      return { step: 'sourceOfFunds', questions };
    case 'account':
      return { step: 'createAccount' };
    case 'phone':
      return { step: 'verifyPhone' };
    case 'completed':
      return { step: 'completed' };
    case 'rejected':
      return { step: 'rejected' };
  }
}

const now = Date.now();
const ago = (minutes: number) => new Date(now - minutes * 60_000).toISOString();

mockIPC(async (cmd, args) => {
  await wait(250);
  const command = cmd.replace('plugin:klimper-card|', '');
  switch (command) {
    case 'card_status':
      return {
        configured: state.configured,
        environment: 'sandbox',
        available: state.words,
        unavailable: state.words ? null : 'noRecoveryWords',
        address: state.words
          ? '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266'
          : null,
        signedIn: state.signedIn,
      };
    case 'card_sign_in':
      state.signedIn = true;
      return null;
    case 'card_sign_out':
      state.signedIn = false;
      return null;
    case 'card_onboarding':
      return step();
    case 'card_send_email_code':
      return true;
    case 'card_register':
      state.stage = 'terms';
      return null;
    case 'card_accept_terms':
      state.stage = 'kyc';
      // The identity check "passes" a few seconds after the link is shown.
      setTimeout(() => (state.stage = 'sof'), 6000);
      return null;
    case 'card_answer_source_of_funds':
      state.stage = 'account';
      return null;
    case 'card_create_account':
      state.stage = 'phone';
      return null;
    case 'card_send_phone_code':
      return null;
    case 'card_verify_phone':
      state.stage = 'completed';
      state.cardStatus = null;
      return null;
    case 'card_order_card':
      state.cardStatus = 'active';
      return null;
    case 'card_account':
      return {
        safeAddress: '0x5afe5afE5afE5AfE5aFE5aFe5AFe5afe5afE5aFE',
        ownerAddress: '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266',
        status: 'active',
        currency: 'EUR',
        spendable: state.spendable.toFixed(2),
        pending: '20.00',
        cards: state.cardStatus
          ? [
              {
                id: 'card-1',
                status: state.cardStatus,
                kind: 'virtual',
                last4: '4242',
                brand: 'VISA',
                expires: '10/29',
              },
            ]
          : [],
      };
    case 'card_transactions':
      return {
        items: [
          {
            id: 't-4',
            kind: 'card',
            createdAt: ago(42),
            status: 'pending',
            merchant: 'Bakery Example',
            amount: '4.20',
            currency: 'EUR',
          },
          {
            id: 't-3',
            kind: 'card',
            createdAt: ago(60 * 26),
            status: 'settled',
            merchant: 'Bookshop Example',
            amount: '18.90',
            currency: 'EUR',
          },
          {
            id: 't-2',
            kind: 'card',
            createdAt: ago(60 * 50),
            status: 'declined',
            merchant: 'Electronics Example',
            amount: '899.00',
            currency: 'EUR',
          },
          {
            id: 't-1',
            kind: 'card',
            createdAt: ago(60 * 73),
            status: 'settled',
            merchant: 'Coffee Example',
            amount: '3.10',
            currency: 'EUR',
          },
        ],
        nextCursor: null,
      };
    case 'card_details':
      // The Visa test number: made up, works nowhere.
      return { number: '4000000000004242', cvv: '737', expires: '10/29' };
    case 'card_set_frozen': {
      const frozen = (args as { frozen?: boolean } | undefined)?.frozen;
      state.cardStatus = frozen ? 'blocked' : 'active';
      return null;
    }
    default:
      // The opener: a link would open in the browser.
      if (cmd.startsWith('plugin:opener')) return null;
      if (cmd.startsWith('plugin:clipboard-manager')) return null;
      throw new Error(`preview: no mock for ${cmd}`);
  }
});
