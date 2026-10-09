// SPDX-License-Identifier: AGPL-3.0-or-later
// Never a black screen (clapandpay TASK-105).
//
// React unmounts the whole tree when a render throws and nothing catches it, and what is
// left is the window's background: a black screen with no way out but quitting. Three
// boundaries make sure some screen with a way back is always there:
//
//   * `main.tsx` wraps the app in `KlimperErrorBoundary` with the recovery screen: the
//     last resort, for the providers and everything else above the router;
//   * `routes.tsx` gives every route `RouteRecovery` as its error element, so a broken
//     screen keeps the rest of the app;
//   * `KlimperRoot` wraps the approval overlay on its own: if a request cannot be shown,
//     a pill points to the requests page and the wallet stays usable.

import { Button } from '@/components/ui/button';
import { t } from '@lingui/core/macro';
import { Component, ErrorInfo, ReactNode } from 'react';
import { useNavigate, useRouteError } from 'react-router-dom';

const WALLET_ROUTE = '/wallet';

function describe(error: unknown): string {
  if (error instanceof Error) return error.message || error.name;
  if (typeof error === 'string') return error;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

/** A full screen in the app's own colours, with the way back to the wallet. */
export function RecoveryScreen({
  error,
  onWallet,
}: {
  readonly error: unknown;
  readonly onWallet: () => void;
}) {
  return (
    <div
      className='flex min-h-screen items-center justify-center bg-background p-6 text-foreground'
      role='alert'
      data-testid='recovery-screen'
    >
      <div className='w-full max-w-md space-y-4'>
        <h1 className='text-2xl font-semibold'>{t`Something went wrong`}</h1>
        <p className='text-sm text-muted-foreground'>
          {t`This screen could not be shown. Your wallet and your money are not affected.`}
        </p>
        <pre
          className='max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border p-3 text-xs text-muted-foreground'
          data-testid='recovery-error'
        >
          {describe(error)}
        </pre>
        <div className='flex flex-wrap gap-2'>
          <Button onClick={onWallet} data-testid='recovery-wallet'>
            {t`Back to my wallet`}
          </Button>
          <Button
            variant='outline'
            onClick={() => window.location.reload()}
            data-testid='recovery-reload'
          >
            {t`Reload`}
          </Button>
        </div>
      </div>
    </div>
  );
}

interface BoundaryProps {
  readonly children: ReactNode;
  /** What to show instead; `reset` renders the children again. */
  readonly renderFallback: (error: unknown, reset: () => void) => ReactNode;
  /** A change of this value clears the error, e.g. the route. */
  readonly resetKey?: unknown;
}

interface BoundaryState {
  readonly error: unknown;
  readonly failed: boolean;
  readonly resetKey: unknown;
}

// An error boundary has to be a class, and it must re-render whenever its children do.
// eslint-disable-next-line react/require-optimization
export class KlimperErrorBoundary extends Component<
  BoundaryProps,
  BoundaryState
> {
  state: BoundaryState = {
    error: null,
    failed: false,
    resetKey: this.props.resetKey,
  };

  static getDerivedStateFromError(error: unknown): Partial<BoundaryState> {
    return { error, failed: true };
  }

  static getDerivedStateFromProps(
    props: BoundaryProps,
    state: BoundaryState,
  ): Partial<BoundaryState> | null {
    if (Object.is(props.resetKey, state.resetKey)) return null;
    return { error: null, failed: false, resetKey: props.resetKey };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error('Klimper caught a render error', error, info.componentStack);
  }

  reset = () => this.setState({ error: null, failed: false });

  render() {
    if (this.state.failed) {
      return this.props.renderFallback(this.state.error, this.reset);
    }
    return this.props.children;
  }
}

/** The app-wide boundary of `main.tsx`: outside the router, so it moves the hash itself. */
export function AppRecovery({ children }: { readonly children: ReactNode }) {
  return (
    <KlimperErrorBoundary
      renderFallback={(error, reset) => (
        <RecoveryScreen
          error={error}
          onWallet={() => {
            window.location.hash = `#${WALLET_ROUTE}`;
            reset();
          }}
        />
      )}
    >
      {children}
    </KlimperErrorBoundary>
  );
}

/** Every route's error element (`routes.tsx`). */
export function RouteRecovery() {
  const error = useRouteError();
  const navigate = useNavigate();
  return (
    <RecoveryScreen
      error={error}
      onWallet={() => navigate(WALLET_ROUTE, { replace: true })}
    />
  );
}
