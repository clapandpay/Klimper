// SPDX-License-Identifier: AGPL-3.0-or-later
// What one website may do, and how much it may cost (clapandpay TASK-083, spec 9.3 and
// 11.1 item 5).
//
// The defaults are the spec's: the five required CHIP-0002 methods, one key of the website's
// own, no limits, no auto-approval, and every dangerous switch off. Everything here narrows
// that, except the dangerous switches, each of which needs its own confirmation.
//
// Limits are enforced in Rust, by wallet-kit, on the **guaranteed** loss — what leaves the
// user's coins minus what comes back through the user's own spends, plus the fees those
// spends reserve. A conditional receipt never offsets a loss, so a limit cannot be talked
// around by promising a payment. A request over a limit is refused before it reaches the
// approval screen, which is why this screen says so in words: a user who wonders why
// nothing appeared should not have to guess.

import Container from '@/components/Container';
import Header from '@/components/Header';
import Layout from '@/components/Layout';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { commands } from '@/bindings';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import BigNumber from 'bignumber.js';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { AlertTriangle, Info, KeyRound } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { identityOfSession, XchonnectDomain } from './XchonnectDomain';
import { Hint } from './XchonnectUi';
import { AssetLimit, DappPolicy, SessionSummary, xchonnect } from './xchonnect';
import { XCHONNECT_ROUTES } from './xchonnectLinkHandoff';
import {
  CAT_PRECISION,
  catLabel,
  claimedText,
  XCH_PRECISION,
} from './xchonnectPrompt';
import { useXchonnectSessions } from './useXchonnectSessions';
import { useTicker } from './useTicker';
import './xchonnect.css';

/**
 * The methods a pairing grants by default: wallet-kit's `DEFAULT_METHODS` and the ones
 * Klimper builds itself (`execute::WALLET_BUILT_METHODS`). Every one still asks.
 */
export const DEFAULT_METHODS = [
  'chainId',
  'connect',
  'getPublicKeys',
  'signCoinSpends',
  'signMessage',
  'xchonnect_submitCoinSpends',
  'chia_send',
] as const;

/** Methods that cannot be taken away: without them a session cannot even say hello. */
const ESSENTIAL_METHODS = ['chainId', 'connect'] as const;

/** `xch` or `cat:<64 hex>` (the plugin's `asset_key`). */
function assetLabel(key: string, ticker: string): string {
  if (key === 'xch') return ticker;
  return catLabel(key.startsWith('cat:') ? key.slice(4) : key);
}

function precisionOf(key: string): number {
  return key === 'xch' ? XCH_PRECISION : CAT_PRECISION;
}

/**
 * A limit the user typed, in the asset's own units, as a whole number of the smallest
 * unit.
 *
 * `null` means "no limit". A value the plugin cannot carry exactly is refused rather than
 * silently rounded: `StoredLimit` is a `u128` in Rust but arrives as a JSON number, and a
 * JSON number stops being exact above 2^53 — about 9007 XCH. The plugin should take these
 * as strings; until it does, this is the honest boundary.
 */
export type LimitInput =
  | { readonly kind: 'none' }
  | { readonly kind: 'amount'; readonly mojos: number }
  | { readonly kind: 'invalid'; readonly reason: 'notANumber' | 'tooLarge' };

export function parseLimit(text: string, precision: number): LimitInput {
  const trimmed = text.trim();
  if (trimmed === '') return { kind: 'none' };
  // A plain amount, as the hint says: BigNumber alone would also read "1e3", "0x10"
  // or ".5", and a user who typed one of those did not mean what it parses to.
  if (!/^\d+(\.\d*)?$/.test(trimmed)) {
    return { kind: 'invalid', reason: 'notANumber' };
  }
  const value = BigNumber(trimmed);
  if (value.isNaN() || value.isNegative()) {
    return { kind: 'invalid', reason: 'notANumber' };
  }
  const smallest = value.multipliedBy(BigNumber(10).pow(precision));
  if (!smallest.isInteger()) {
    return { kind: 'invalid', reason: 'notANumber' };
  }
  if (smallest.isGreaterThan(Number.MAX_SAFE_INTEGER)) {
    return { kind: 'invalid', reason: 'tooLarge' };
  }
  return { kind: 'amount', mojos: smallest.toNumber() };
}

function limitText(
  value: string | number | null | undefined,
  precision: number,
): string {
  if (value === null || value === undefined) return '';
  return BigNumber(value).dividedBy(BigNumber(10).pow(precision)).toFixed();
}

interface Draft {
  readonly methods: string[];
  /** Keyed by `<assetKey>` then per-request / per-day, as typed. */
  readonly limits: Record<string, { perRequest: string; perDay: string }>;
}

function draftOf(session: SessionSummary): Draft {
  const keys = new Set<string>(['xch', ...Object.keys(session.limits)]);
  const limits: Draft['limits'] = {};
  for (const key of keys) {
    const limit: AssetLimit = session.limits[key] ?? {};
    limits[key] = {
      perRequest: limitText(limit.perRequest, precisionOf(key)),
      perDay: limitText(limit.perDay, precisionOf(key)),
    };
  }
  return { methods: [...session.methods], limits };
}

/** `0x1234…cdef`: enough to recognise a key, short enough to read. */
export function shortKey(key: string): string {
  const hex = key.startsWith('0x') ? key.slice(2) : key;
  return hex.length <= 12
    ? `0x${hex}`
    : `0x${hex.slice(0, 4)}…${hex.slice(-4)}`;
}

/** The keys this website was given, one line, the explanation in a tooltip. */
function SharedKeys({ keys }: { readonly keys: readonly string[] }) {
  const count = keys.length;
  return (
    <div
      className='flex flex-wrap items-center gap-2 text-sm'
      data-testid='shared-keys'
    >
      <KeyRound className='h-4 w-4 text-muted-foreground' aria-hidden='true' />
      <span className='text-muted-foreground'>
        {keys.length === 1 ? (
          <Trans>Shared key</Trans>
        ) : (
          <Trans>Shared keys</Trans>
        )}
      </span>
      {keys.length > 3 ? (
        <span className='xc-chip'>
          <Trans>all {count} receive addresses</Trans>
        </span>
      ) : null}
      {keys.length <= 3 &&
        keys.map((key) => (
          <span key={key} className='xc-chip font-mono' title={key}>
            {shortKey(key)}
          </span>
        ))}
      <TooltipProvider delayDuration={150}>
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type='button'
              className='text-muted-foreground hover:text-foreground'
              aria-label={t`What does the website see?`}
            >
              <Info className='h-4 w-4' aria-hidden='true' />
            </button>
          </TooltipTrigger>
          <TooltipContent className='max-w-xs'>
            {count === 1 ? (
              <Trans>
                The website sees only this key and the address made from it —
                nothing else in your wallet.
              </Trans>
            ) : (
              <Trans>
                The website sees these keys and the addresses made from them.
              </Trans>
            )}
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </div>
  );
}

type Dangerous = 'unsafe' | 'unknown' | 'addresses';

function policyFor(which: Dangerous, on: boolean): DappPolicy {
  switch (which) {
    case 'unsafe':
      return { allowAggSigUnsafe: on };
    case 'unknown':
      return { allowUnknownContracts: on };
    case 'addresses':
      return { allReceiveKeys: on };
  }
}

export default function XchonnectDappSettings() {
  const navigate = useNavigate();
  const ticker = useTicker();
  const { sessionId } = useParams<{ sessionId: string }>();
  const { sessions, loading, refresh } = useXchonnectSessions();
  const session = useMemo(
    () => sessions.find((item) => item.sessionId === sessionId) ?? null,
    [sessions, sessionId],
  );

  const [draft, setDraft] = useState<Draft | null>(null);
  const [asking, setAsking] = useState<Dangerous | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  // The screen stays mounted when the route moves from one dApp to another (back and
  // forward), so a draft is only ever the current dApp's.
  useEffect(() => {
    setDraft(null);
    setProblem(null);
    setSaved(false);
  }, [sessionId]);

  useEffect(() => {
    if (session !== null && draft === null) setDraft(draftOf(session));
  }, [session, draft]);

  // The wallet's tokens, so each can be given a limit too (a token with none is unlimited).
  const [tokens, setTokens] = useState<ReadonlyMap<string, string>>(new Map());
  useEffect(() => {
    let live = true;
    commands
      .getCats({})
      .then(({ cats }) => {
        if (!live) return;
        const found = new Map<string, string>();
        for (const cat of cats) {
          if (cat.asset_id) found.set(`cat:${cat.asset_id}`, cat.ticker ?? '');
        }
        setTokens(found);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  useEffect(() => {
    if (draft === null) return;
    const missing = [...tokens.keys()].filter((key) => !(key in draft.limits));
    if (missing.length === 0) return;
    const limits = { ...draft.limits };
    for (const key of missing) limits[key] = { perRequest: '', perDay: '' };
    setDraft({ ...draft, limits });
  }, [tokens, draft]);

  if (loading && session === null) {
    return (
      <Layout>
        <Header
          title={t`What this website may do`}
          back={() => navigate(XCHONNECT_ROUTES.dapps)}
        />
        <Container className='max-w-xl' />
      </Layout>
    );
  }

  if (session === null || draft === null) {
    return (
      <Layout>
        <Header
          title={t`What this website may do`}
          back={() => navigate(XCHONNECT_ROUTES.dapps)}
        />
        <Container className='max-w-xl'>
          <Card>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>That connection no longer exists</Trans>
              </CardTitle>
            </CardHeader>
            <CardContent>
              <Button onClick={() => navigate(XCHONNECT_ROUTES.dapps)}>
                <Trans>Connected websites</Trans>
              </Button>
            </CardContent>
          </Card>
        </Container>
      </Layout>
    );
  }

  const apply = async (
    change: Parameters<typeof xchonnect.setDappPolicy>[1],
  ) => {
    setProblem(null);
    try {
      await xchonnect.setDappPolicy(session.sessionId, change);
      setSaved(true);
      await refresh();
    } catch (failure) {
      setProblem(String(failure));
    }
  };

  const toggleMethod = (method: string, allowed: boolean) => {
    const methods = allowed
      ? [...new Set([...draft.methods, method])]
      : draft.methods.filter((item) => item !== method);
    setDraft({ ...draft, methods });
    setSaved(false);
  };

  const saveLimits = async () => {
    const limits: Record<string, { perRequest?: number; perDay?: number }> = {};
    for (const [key, typed] of Object.entries(draft.limits)) {
      const precision = precisionOf(key);
      const perRequest = parseLimit(typed.perRequest, precision);
      const perDay = parseLimit(typed.perDay, precision);
      for (const parsed of [perRequest, perDay]) {
        if (parsed.kind === 'invalid') {
          setProblem(
            parsed.reason === 'tooLarge'
              ? t`That limit is too large for Klimper to store exactly. Use a smaller one.`
              : t`A limit has to be a plain amount, like 1.5.`,
          );
          return;
        }
      }
      const entry: { perRequest?: number; perDay?: number } = {};
      if (perRequest.kind === 'amount') entry.perRequest = perRequest.mojos;
      if (perDay.kind === 'amount') entry.perDay = perDay.mojos;
      if (entry.perRequest !== undefined || entry.perDay !== undefined) {
        limits[key] = entry;
      }
    }
    await apply({ limits, methods: draft.methods });
  };

  /** Turning a dangerous switch on is a separate decision; turning it off is not. */
  const dangerous = (which: Dangerous, on: boolean) => {
    if (on) {
      setAsking(which);
      return;
    }
    void apply(policyFor(which, false));
  };

  const confirmDangerous = () => {
    const which = asking;
    setAsking(null);
    if (which === null) return;
    void apply(policyFor(which, true));
  };

  const assetKeys = Object.keys(draft.limits).sort((left, right) =>
    left === 'xch' ? -1 : right === 'xch' ? 1 : left.localeCompare(right),
  );

  return (
    <Layout>
      <Header
        title={t`What this website may do`}
        back={() => navigate(XCHONNECT_ROUTES.dapps)}
      />
      <Container className='max-w-xl space-y-3'>
        <Card>
          <CardHeader className='p-4 pb-2'>
            <CardTitle className='text-base'>
              <XchonnectDomain
                identity={identityOfSession(session)}
                size='inline'
                showName
              />
            </CardTitle>
          </CardHeader>
        </Card>

        {problem !== null && (
          <Card className='border-destructive'>
            <CardHeader className='p-4 pb-2'>
              <CardDescription className='text-destructive break-words'>
                {problem}
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        <Card>
          <CardHeader className='p-4 pb-2'>
            <CardTitle className='text-base flex items-center gap-1'>
              <Trans>What it may ask for</Trans>
              <Hint>
                <Trans>
                  Taking a method away means the website is refused when it
                  tries. Nothing here is approved automatically.
                </Trans>
              </Hint>
            </CardTitle>
          </CardHeader>
          <CardContent className='space-y-3 p-4 pt-0'>
            <div className='grid grid-cols-2 gap-x-3 gap-y-2'>
              {DEFAULT_METHODS.map((method) => {
                const essential = (
                  ESSENTIAL_METHODS as readonly string[]
                ).includes(method);
                return (
                  <div key={method} className='flex items-center gap-2 min-w-0'>
                    <Checkbox
                      id={`method-${method}`}
                      checked={draft.methods.includes(method)}
                      disabled={essential}
                      onCheckedChange={(value) =>
                        toggleMethod(method, value === true)
                      }
                    />
                    <Label
                      htmlFor={`method-${method}`}
                      className='font-mono text-xs truncate'
                      title={essential ? t`always allowed` : undefined}
                    >
                      {method}
                    </Label>
                  </div>
                );
              })}
            </div>
            <SharedKeys keys={session.exposedKeys} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader className='p-4 pb-2'>
            <CardTitle className='text-base flex items-center gap-1'>
              <Trans>How much it may cost</Trans>
              <Hint>
                <Trans>
                  Leave a field empty for no limit. A token with no limit here
                  is not limited, and NFTs never are. A day is a UTC day.
                </Trans>
              </Hint>
            </CardTitle>
            <CardDescription>
              <Trans>
                A request over a limit is refused before it reaches you — you
                will not see a signing prompt for it.
              </Trans>
            </CardDescription>
          </CardHeader>
          <CardContent className='space-y-2 p-4 pt-0'>
            <div
              className='grid grid-cols-[1fr_5.5rem_5.5rem] gap-2 xc-label'
              aria-hidden='true'
            >
              <span />
              <span>
                <Trans>Per request</Trans>
              </span>
              <span>
                <Trans>Per day</Trans>
              </span>
            </div>
            {assetKeys.map((key) => (
              <div
                key={key}
                className='grid grid-cols-[1fr_5.5rem_5.5rem] items-center gap-2'
              >
                <div className='min-w-0 text-sm font-medium truncate'>
                  {assetLabel(key, ticker)}
                  {tokens.get(key) ? (
                    <span className='ml-1.5 font-normal text-muted-foreground'>
                      {claimedText(tokens.get(key))}
                    </span>
                  ) : null}
                </div>
                <div>
                  <Label htmlFor={`per-request-${key}`} className='sr-only'>
                    <Trans>Per request</Trans>
                  </Label>
                  <Input
                    className='h-9'
                    id={`per-request-${key}`}
                    inputMode='decimal'
                    value={draft.limits[key].perRequest}
                    onChange={(event) => {
                      setSaved(false);
                      setDraft({
                        ...draft,
                        limits: {
                          ...draft.limits,
                          [key]: {
                            ...draft.limits[key],
                            perRequest: event.target.value,
                          },
                        },
                      });
                    }}
                  />
                </div>
                <div>
                  <Label htmlFor={`per-day-${key}`} className='sr-only'>
                    <Trans>Per day</Trans>
                  </Label>
                  <Input
                    className='h-9'
                    id={`per-day-${key}`}
                    inputMode='decimal'
                    value={draft.limits[key].perDay}
                    onChange={(event) => {
                      setSaved(false);
                      setDraft({
                        ...draft,
                        limits: {
                          ...draft.limits,
                          [key]: {
                            ...draft.limits[key],
                            perDay: event.target.value,
                          },
                        },
                      });
                    }}
                  />
                </div>
              </div>
            ))}
            <div className='flex items-center gap-3'>
              <Button onClick={() => void saveLimits()}>
                <Trans>Save</Trans>
              </Button>
              {saved && (
                <span
                  className='text-sm text-muted-foreground'
                  data-testid='saved'
                >
                  <Trans>Saved. It applies to the next request.</Trans>
                </span>
              )}
            </div>
          </CardContent>
        </Card>

        <Card className='border-destructive'>
          <CardHeader className='p-4 pb-2'>
            <CardTitle className='text-base flex items-center gap-2'>
              <AlertTriangle className='h-4 w-4' aria-hidden='true' />
              <Trans>Dangerous permissions</Trans>
              <Hint>
                <Trans>
                  All are off for every new website. Turn one on only if you
                  know exactly why this one needs it.
                </Trans>
              </Hint>
            </CardTitle>
          </CardHeader>
          <CardContent className='space-y-3 p-4 pt-0'>
            <div className='flex items-center justify-between gap-3'>
              <div className='flex min-w-0 items-center gap-1'>
                <Label htmlFor='all-addresses' className='font-normal'>
                  <Trans>Let it see all my receive addresses</Trans>
                </Label>
                <Hint>
                  <Trans>
                    Otherwise it sees one address of its own. Websites that
                    build trades from your coins, like Pengui, need this.
                  </Trans>
                </Hint>
              </div>
              <Switch
                id='all-addresses'
                checked={session.allReceiveKeys}
                onCheckedChange={(value) => dangerous('addresses', value)}
              />
            </div>
            <div className='flex items-center justify-between gap-3'>
              <div className='flex min-w-0 items-center gap-1'>
                <Label htmlFor='allow-unsafe' className='font-normal'>
                  <Trans>Allow unsafe signatures</Trans>
                </Label>
                <Hint>
                  <Trans>
                    An unsafe signature is valid in any spend, not just the one
                    you were shown.
                  </Trans>
                </Hint>
              </div>
              <Switch
                id='allow-unsafe'
                checked={session.allowAggSigUnsafe}
                onCheckedChange={(value) => dangerous('unsafe', value)}
              />
            </div>
            <div className='flex items-center justify-between gap-3'>
              <div className='flex min-w-0 items-center gap-1'>
                <Label htmlFor='allow-unknown' className='font-normal'>
                  <Trans>Allow contracts Klimper cannot read</Trans>
                </Label>
                <Hint>
                  <Trans>
                    Klimper can still show you the net effect, but not what the
                    contract does afterwards.
                  </Trans>
                </Hint>
              </div>
              <Switch
                id='allow-unknown'
                checked={session.allowUnknownContracts}
                onCheckedChange={(value) => dangerous('unknown', value)}
              />
            </div>
          </CardContent>
        </Card>

        <Dialog
          open={asking !== null}
          onOpenChange={(open) => !open && setAsking(null)}
        >
          <DialogContent data-testid='dangerous-confirm'>
            <DialogHeader>
              <DialogTitle className='text-destructive'>
                {asking === 'unsafe' ? (
                  <Trans>Allow unsafe signatures?</Trans>
                ) : asking === 'addresses' ? (
                  <Trans>Show it all your receive addresses?</Trans>
                ) : (
                  <Trans>Allow unreadable contracts?</Trans>
                )}
              </DialogTitle>
              <DialogDescription>
                {asking === 'unsafe' ? (
                  <Trans>
                    A signature this website asks for could then be reused in a
                    completely different spend, including one that takes
                    everything. Almost nothing needs this.
                  </Trans>
                ) : asking === 'addresses' ? (
                  <Trans>
                    It can then see every coin at those addresses, and connect
                    them to you. Every request still needs your approval.
                  </Trans>
                ) : (
                  <Trans>
                    Klimper will stop refusing spends whose contract it cannot
                    read. Nobody can tell you what such a contract does with
                    your coins.
                  </Trans>
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant='outline' onClick={() => setAsking(null)}>
                <Trans>Leave it off</Trans>
              </Button>
              <Button variant='destructive' onClick={confirmDangerous}>
                <Trans>I understand, turn it on</Trans>
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </Container>
    </Layout>
  );
}
