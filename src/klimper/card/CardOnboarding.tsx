// SPDX-License-Identifier: AGPL-3.0-or-later
// Gnosis Pay's onboarding, one step at a time (TASK-150). Gnosis Pay decides the order;
// this screen shows the step it names and nothing else.
//
// What the user types here (e-mail, phone, answers) goes to Gnosis Pay and is not kept by
// Klimper. The identity check runs on Sumsub's own page in the browser.

import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ExternalLink, Loader2 } from 'lucide-react';
import { ReactNode, useState } from 'react';
import { card, cardErrorMessage, Step, Term } from './card';

interface StepProps {
  /** Runs an action, shows its error, then reloads the step. */
  readonly act: (action: () => Promise<unknown>) => Promise<boolean>;
  readonly busy: boolean;
}

export function CardOnboarding({
  step,
  onChanged,
}: {
  readonly step: Step;
  readonly onChanged: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const act = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await onChanged();
      return true;
    } catch (e) {
      setError(cardErrorMessage(e));
      return false;
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className='space-y-4' data-testid={`card-step-${step.step}`}>
      <Progress step={step.step} />
      {error && (
        <div
          className='rounded-lg border border-destructive p-3 text-sm text-destructive'
          role='alert'
          data-testid='card-step-error'
        >
          {error}
        </div>
      )}
      <StepBody step={step} act={act} busy={busy} />
    </div>
  );
}

const ORDER: Step['step'][] = [
  'register',
  'acceptTerms',
  'completeKyc',
  'sourceOfFunds',
  'createAccount',
  'verifyPhone',
  'completed',
];

function Progress({ step }: { readonly step: Step['step'] }) {
  const position: Partial<Record<Step['step'], number>> = {
    waitingKyc: 2,
    kycReview: 2,
    waitingAccount: 4,
  };
  const at = position[step] ?? Math.max(0, ORDER.indexOf(step));
  const current = at + 1;
  const total = ORDER.length;
  return (
    <div
      className='flex gap-1'
      aria-label={t`Step ${current} of ${total}`}
      role='img'
    >
      {ORDER.map((name, i) => (
        <div
          key={name}
          className={`h-1 flex-1 rounded-full ${i <= at ? 'bg-primary' : 'bg-muted'}`}
        />
      ))}
    </div>
  );
}

function StepCard({
  title,
  description,
  children,
}: {
  readonly title: ReactNode;
  readonly description?: ReactNode;
  readonly children?: ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className='text-base'>{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      {children && <CardContent className='space-y-3'>{children}</CardContent>}
    </Card>
  );
}

function Busy({
  busy,
  children,
}: {
  readonly busy: boolean;
  readonly children: ReactNode;
}) {
  return (
    <>
      {busy && (
        <Loader2 className='mr-2 h-4 w-4 animate-spin' aria-hidden='true' />
      )}
      {children}
    </>
  );
}

function Waiting({
  title,
  text,
}: {
  readonly title: ReactNode;
  readonly text: ReactNode;
}) {
  return (
    <StepCard title={title} description={text}>
      <div className='flex items-center gap-2 text-sm text-muted-foreground'>
        <Loader2 className='h-4 w-4 animate-spin' aria-hidden='true' />
        <Trans>Checking again every few seconds</Trans>
      </div>
    </StepCard>
  );
}

function StepBody({ step, act, busy }: { readonly step: Step } & StepProps) {
  switch (step.step) {
    case 'register':
      return <Register act={act} busy={busy} />;
    case 'acceptTerms':
      return <AcceptTerms terms={step.terms} act={act} busy={busy} />;
    case 'waitingKyc':
      return (
        <Waiting
          title={<Trans>Preparing the identity check</Trans>}
          text={
            <Trans>Gnosis Pay is setting it up. This takes a moment.</Trans>
          }
        />
      );
    case 'completeKyc':
      return <Kyc url={step.url} resubmission={step.resubmission} />;
    case 'kycReview':
      return (
        <Waiting
          title={<Trans>Your identity check is being reviewed</Trans>}
          text={
            <Trans>
              A person at Gnosis Pay is looking at it. There is nothing to do
              here; Gnosis Pay writes to you if they need more.
            </Trans>
          }
        />
      );
    case 'sourceOfFunds':
      return <SourceOfFunds step={step} act={act} busy={busy} />;
    case 'createAccount':
      return (
        <StepCard
          title={<Trans>Open the card account</Trans>}
          description={
            <Trans>
              Gnosis Pay sets up a Safe account on Gnosis Chain. Its owner is
              your card address, for good: only your recovery words can control
              it.
            </Trans>
          }
        >
          <Button
            disabled={busy}
            onClick={() => act(card.createAccount)}
            data-testid='card-create-account'
          >
            <Busy busy={busy}>
              <Trans>Open the account</Trans>
            </Busy>
          </Button>
        </StepCard>
      );
    case 'waitingAccount':
      return (
        <Waiting
          title={<Trans>Opening the account</Trans>}
          text={<Trans>The Safe is being set up on Gnosis Chain.</Trans>}
        />
      );
    case 'verifyPhone':
      return <VerifyPhone act={act} busy={busy} />;
    case 'completed':
      return <OrderCard act={act} busy={busy} />;
    case 'rejected':
      return (
        <StepCard
          title={<Trans>Gnosis Pay cannot offer you a card</Trans>}
          description={
            <Trans>
              The decision is Gnosis Pay’s, and they tell you about it directly.
              Your wallet is not affected.
            </Trans>
          }
        />
      );
    case 'unknown':
      return (
        <Waiting
          title={
            <Trans>Gnosis Pay is on a step this version does not know</Trans>
          }
          text={step.status}
        />
      );
  }
}

function Register({ act, busy }: StepProps) {
  const [email, setEmail] = useState('');
  const [code, setCode] = useState('');
  const [sent, setSent] = useState<boolean | null>(null);

  const send = async () => {
    let needsCode = true;
    const ok = await act(async () => {
      needsCode = await card.sendEmailCode(email);
      if (!needsCode) await card.register(email, null);
    });
    if (ok && needsCode) setSent(true);
  };

  return (
    <StepCard
      title={<Trans>Your e-mail address</Trans>}
      description={
        <Trans>
          Gnosis Pay writes to you there about the card. Klimper does not keep
          it.
        </Trans>
      }
    >
      <div className='space-y-1'>
        <Label htmlFor='card-email'>
          <Trans>E-mail</Trans>
        </Label>
        <Input
          id='card-email'
          type='email'
          autoComplete='email'
          value={email}
          disabled={sent === true}
          onChange={(e) => setEmail(e.target.value)}
        />
      </div>
      {sent ? (
        <>
          <div className='space-y-1'>
            <Label htmlFor='card-email-code'>
              <Trans>The six-digit code from the e-mail</Trans>
            </Label>
            <Input
              id='card-email-code'
              inputMode='numeric'
              autoComplete='one-time-code'
              maxLength={6}
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/\D/g, ''))}
            />
          </div>
          <Button
            disabled={busy || code.length !== 6}
            onClick={() => act(() => card.register(email, code))}
            data-testid='card-register'
          >
            <Busy busy={busy}>
              <Trans>Confirm</Trans>
            </Busy>
          </Button>
        </>
      ) : (
        <Button
          disabled={busy || !email.includes('@')}
          onClick={send}
          data-testid='card-send-email-code'
        >
          <Busy busy={busy}>
            <Trans>Send the code</Trans>
          </Busy>
        </Button>
      )}
    </StepCard>
  );
}

function AcceptTerms({
  terms,
  act,
  busy,
}: { readonly terms: Term[] } & StepProps) {
  const [agreed, setAgreed] = useState(false);
  return (
    <StepCard
      title={<Trans>Gnosis Pay’s terms</Trans>}
      description={
        <Trans>
          The card contract is between you and Gnosis Pay and its card issuer.
          Read them before you agree.
        </Trans>
      }
    >
      <ul className='space-y-1 text-sm'>
        {terms.map((term) => (
          <li key={term.type}>
            {term.url ? (
              <button
                type='button'
                className='inline-flex items-center gap-1 underline'
                onClick={() => void openUrl(term.url as string)}
              >
                {term.name}
                <ExternalLink className='h-3 w-3' aria-hidden='true' />
              </button>
            ) : (
              term.name
            )}
          </li>
        ))}
      </ul>
      <label className='flex items-start gap-2 text-sm'>
        <input
          type='checkbox'
          checked={agreed}
          onChange={(e) => setAgreed(e.target.checked)}
          data-testid='card-terms-agree'
        />
        <Trans>I have read and agree to these terms.</Trans>
      </label>
      <Button
        disabled={busy || !agreed}
        onClick={() => act(() => card.acceptTerms(terms))}
        data-testid='card-accept-terms'
      >
        <Busy busy={busy}>
          <Trans>Agree</Trans>
        </Busy>
      </Button>
    </StepCard>
  );
}

function Kyc({
  url,
  resubmission,
}: {
  readonly url: string | null;
  readonly resubmission: boolean;
}) {
  return (
    <StepCard
      title={
        resubmission ? (
          <Trans>Gnosis Pay needs another look at your documents</Trans>
        ) : (
          <Trans>Check your identity</Trans>
        )
      }
      description={
        <Trans>
          The check runs on the page of Sumsub, Gnosis Pay’s identity provider,
          in your browser. Your documents go to them, not to Klimper. Come back
          here when you are done.
        </Trans>
      }
    >
      {url ? (
        <Button onClick={() => void openUrl(url)} data-testid='card-open-kyc'>
          <ExternalLink className='mr-2 h-4 w-4' aria-hidden='true' />
          <Trans>Open the identity check</Trans>
        </Button>
      ) : (
        <p className='text-sm text-muted-foreground'>
          <Trans>The link is not ready yet; checking again.</Trans>
        </p>
      )}
    </StepCard>
  );
}

function SourceOfFunds({
  step,
  act,
  busy,
}: { readonly step: Extract<Step, { step: 'sourceOfFunds' }> } & StepProps) {
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const complete = step.questions.every((q) => answers[q.id]);
  return (
    <StepCard
      title={<Trans>Where your money comes from</Trans>}
      description={
        <Trans>Card issuers must ask this. The answers go to Gnosis Pay.</Trans>
      }
    >
      {step.questions.map((q) => (
        <fieldset key={q.id} className='space-y-1'>
          <legend className='text-sm font-medium'>{q.question}</legend>
          {q.answers.map((answer) => (
            <label key={answer} className='flex items-center gap-2 text-sm'>
              <input
                type='radio'
                name={`sof-${q.id}`}
                checked={answers[q.id] === answer}
                onChange={() => setAnswers({ ...answers, [q.id]: answer })}
              />
              {answer}
            </label>
          ))}
        </fieldset>
      ))}
      <Button
        disabled={busy || !complete}
        onClick={() =>
          act(() =>
            card.answerSourceOfFunds(
              step.questions.map((q) => ({
                question: q.question,
                answer: answers[q.id],
              })),
            ),
          )
        }
        data-testid='card-answer-sof'
      >
        <Busy busy={busy}>
          <Trans>Send the answers</Trans>
        </Busy>
      </Button>
    </StepCard>
  );
}

function VerifyPhone({ act, busy }: StepProps) {
  const [phone, setPhone] = useState('');
  const [code, setCode] = useState('');
  const [sent, setSent] = useState(false);
  return (
    <StepCard
      title={<Trans>Your phone number</Trans>}
      description={
        <Trans>
          Gnosis Pay uses it to confirm online payments. Klimper does not keep
          it.
        </Trans>
      }
    >
      <div className='space-y-1'>
        <Label htmlFor='card-phone'>
          <Trans>Phone, with the country code</Trans>
        </Label>
        <Input
          id='card-phone'
          type='tel'
          autoComplete='tel'
          placeholder='+1 555 …'
          value={phone}
          disabled={sent}
          onChange={(e) => setPhone(e.target.value)}
        />
      </div>
      {sent ? (
        <>
          <div className='space-y-1'>
            <Label htmlFor='card-phone-code'>
              <Trans>The six-digit code from the text message</Trans>
            </Label>
            <Input
              id='card-phone-code'
              inputMode='numeric'
              autoComplete='one-time-code'
              maxLength={6}
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/\D/g, ''))}
            />
          </div>
          <Button
            disabled={busy || code.length !== 6}
            onClick={() => act(() => card.verifyPhone(code))}
            data-testid='card-verify-phone'
          >
            <Busy busy={busy}>
              <Trans>Confirm</Trans>
            </Busy>
          </Button>
        </>
      ) : (
        <Button
          disabled={busy || phone.trim().length < 8}
          onClick={async () => {
            if (await act(() => card.sendPhoneCode(phone))) setSent(true);
          }}
          data-testid='card-send-phone-code'
        >
          <Busy busy={busy}>
            <Trans>Send the code</Trans>
          </Busy>
        </Button>
      )}
    </StepCard>
  );
}

function OrderCard({ act, busy }: StepProps) {
  return (
    <StepCard
      title={<Trans>Your account is ready</Trans>}
      description={
        <Trans>
          Order a virtual card. It works as soon as it is made. Showing its
          number and adding it to Apple Pay or Google Pay come in a later
          version of Klimper.
        </Trans>
      }
    >
      <Button
        disabled={busy}
        onClick={() => act(() => card.orderCard('Klimper'))}
        data-testid='card-order'
      >
        <Busy busy={busy}>
          <Trans>Order a virtual card</Trans>
        </Busy>
      </Button>
    </StepCard>
  );
}
