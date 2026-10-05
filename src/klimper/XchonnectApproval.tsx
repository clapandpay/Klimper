// SPDX-License-Identifier: AGPL-3.0-or-later
// The requests page (clapandpay TASK-080, TASK-085): everything waiting for the user, and
// the one they picked in the same large view the overlay shows
// (`XchonnectApprovalDetail`).
//
// The list only shows what is waiting and when it is refused by itself; it carries no
// amount and no Approve. A request is answered from its full view and nowhere else, so
// picking one out of order can never approve something the user has not seen.
//
// On a wide window the list and the view sit side by side. On a phone the list comes
// first and the view replaces it; with a single request the view is shown at once.

import Container from '@/components/Container';
import Header from '@/components/Header';
import Layout from '@/components/Layout';
import { Button } from '@/components/ui/button';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { ChevronRight, Inbox } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { Approval, xchonnect } from './xchonnect';
import {
  approvalHeadline,
  Countdown,
  XchonnectApprovalDetail,
} from './XchonnectApprovalDetail';
import {
  subscribeXchonnectLink,
  takeXchonnectLink,
  XCHONNECT_ROUTES,
} from './xchonnectLinkHandoff';
import { SiteCoin } from './XchonnectUi';
import { useXchonnectApprovals } from './useXchonnectApprovals';
import { returnToDapp } from './xchonnectReturn';

function RequestRow({
  approval,
  selected,
  onSelect,
}: {
  readonly approval: Approval;
  readonly selected: boolean;
  readonly onSelect: () => void;
}) {
  return (
    <li>
      <button
        type='button'
        className='xc-request-row'
        data-selected={selected}
        aria-current={selected ? 'true' : undefined}
        onClick={onSelect}
        data-testid='request-row'
      >
        <SiteCoin domain={approval.dapp} />
        <span className='min-w-0 flex-1 text-left'>
          <span className='block font-semibold truncate'>{approval.dapp}</span>
          <span className='block text-sm text-muted-foreground truncate'>
            {approvalHeadline(approval)}
          </span>
          <Countdown expiresAt={approval.expiresAt} />
        </span>
        <ChevronRight
          className='h-4 w-4 shrink-0 text-muted-foreground'
          aria-hidden='true'
        />
      </button>
    </li>
  );
}

export default function XchonnectApproval() {
  const navigate = useNavigate();
  const { approvalId } = useParams();
  const { approvals, error, answer, cancel, reload } = useXchonnectApprovals();
  /** A `/req` link arrived with nothing pending: say so, neutrally. */
  const [cameFromLink, setCameFromLink] = useState(false);
  /** The domain of the last request answered here, for the "you can go back" state. */
  const [answered, setAnswered] = useState<string | null>(null);

  // A same-device request hint. The hint itself is not acted on: `listSessions` resumes
  // every stored session's message loop, which is what actually fetches the mailbox, and
  // only an authenticated envelope can produce an approval.
  useEffect(() => {
    const act = (kind: string | undefined) => {
      if (kind !== 'request') return;
      setCameFromLink(true);
      void xchonnect
        .listSessions()
        .then(reload)
        .catch(() => undefined);
    };
    act(takeXchonnectLink()?.kind);
    return subscribeXchonnectLink((link) => act(link.kind));
  }, [reload]);

  // The one the user picked, while it is still waiting; otherwise the oldest.
  const picked = approvals.find((item) => item.id === approvalId) ?? null;
  const current = picked ?? approvals[0] ?? null;
  // On a phone: the list unless a request was picked or there is only one.
  const showDetailFirst = picked !== null || approvals.length === 1;

  const select = (id: string) =>
    navigate(`${XCHONNECT_ROUTES.request}/${id}`, { replace: true });

  const resolve = (approval: Approval, approved: boolean) => {
    const dapp = approval.dapp;
    void answer(approval.id, approved).then((taken) => {
      // Not taken: the error banner says so, and there is no answer to go back with.
      if (!taken) return;
      // Integration guide 5.4: with no usable return URL the user is left here with a
      // clear "you can go back now" state rather than a guessed navigation. The guard
      // that would open one lives in `xchonnectReturn.ts`.
      void returnToDapp(undefined, dapp).then((outcome) => {
        if (outcome !== 'opened') setAnswered(dapp);
      });
    });
    if (picked !== null) {
      navigate(XCHONNECT_ROUTES.request, { replace: true });
    }
  };

  const withdraw = (approval: Approval) => {
    void cancel(approval.id);
    if (picked !== null) {
      navigate(XCHONNECT_ROUTES.request, { replace: true });
    }
  };

  const count = approvals.length;

  return (
    <Layout>
      <Header
        title={t`Requests`}
        back={() => navigate(XCHONNECT_ROUTES.dapps)}
      />
      <Container className='max-w-5xl space-y-4'>
        {error !== null && (
          <div
            className='rounded-[20px] border border-destructive p-4 text-sm text-destructive break-words'
            role='alert'
          >
            {error}
          </div>
        )}

        {current === null && answered !== null && (
          <div className='xc-notice' data-testid='go-back-notice'>
            <div className='font-semibold'>
              <Trans>Answered</Trans>
            </div>
            <div className='text-sm text-muted-foreground'>
              <Trans>
                {answered} has your answer. You can go back to it now.
              </Trans>
            </div>
          </div>
        )}

        {current === null && (
          <div className='xc-empty' data-testid='nothing-waiting'>
            <span className='xc-empty-icon' aria-hidden='true'>
              <Inbox className='h-6 w-6' />
            </span>
            <div className='space-y-1'>
              <div className='text-lg font-semibold'>
                <Trans>Nothing is waiting for you</Trans>
              </div>
              <p className='text-sm text-muted-foreground max-w-md'>
                {cameFromLink ? (
                  <Trans>
                    A website asked for your signature, but nothing has arrived
                    yet. Klimper is checking its mailbox; if nothing appears,
                    ask the website to try again.
                  </Trans>
                ) : (
                  <Trans>
                    When a connected website asks you to sign something, it
                    appears here.
                  </Trans>
                )}
              </p>
            </div>
            <Button
              variant='outline'
              onClick={() => navigate(XCHONNECT_ROUTES.dapps)}
            >
              <Trans>Connected websites</Trans>
            </Button>
          </div>
        )}

        {current !== null && (
          <div className='grid gap-4 md:grid-cols-[minmax(15rem,20rem)_1fr] md:items-start'>
            <section
              className={showDetailFirst ? 'hidden md:block' : 'block'}
              aria-label={t`Waiting for you`}
            >
              <div className='xc-label mb-2 px-1'>
                <Trans>Waiting for you · {count}</Trans>
              </div>
              <ul className='space-y-2' data-testid='request-list'>
                {approvals.map((approval) => (
                  <RequestRow
                    key={approval.id}
                    approval={approval}
                    selected={approval.id === current.id}
                    onSelect={() => select(approval.id)}
                  />
                ))}
              </ul>
            </section>

            <section className={showDetailFirst ? 'block' : 'hidden md:block'}>
              {picked !== null && count > 1 && (
                <button
                  type='button'
                  className='mb-2 text-sm text-primary md:hidden'
                  onClick={() =>
                    navigate(XCHONNECT_ROUTES.request, { replace: true })
                  }
                >
                  <Trans>All requests</Trans>
                </button>
              )}
              <XchonnectApprovalDetail
                approval={current}
                waiting={count - 1}
                onAnswer={(approved) => resolve(current, approved)}
                onCancel={() => withdraw(current)}
              />
            </section>
          </div>
        )}
      </Container>
    </Layout>
  );
}
