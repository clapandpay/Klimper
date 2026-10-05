import { emit } from '@tauri-apps/api/event';
import { mockIPC } from '@tauri-apps/api/mocks';

const now = () => Math.floor(Date.now() / 1000);
const TAIL = '1a2b3c4d'.repeat(8);
function spend(id: string, dapp: string, assets: unknown[], extra: Record<string, unknown> = {}) {
  return {
    id, sessionId: 's1', dapp, method: 'signCoinSpends', expiresAt: now() + 271, kind: 'request',
    prompt: {
      type: 'sign_coin_spends', dapp,
      summary: { assets, implied_fee: 500_000, reserve_fee: 0, time_locks: {}, unknown_puzzles: [], cost: 1, spends: [] },
      plan: { ours: [{ spend: 0, opcode: 50, public_key: 'a'.repeat(96), message: 'ff', is_unsafe: false }], foreign: [], has_unsafe: false },
      partial: false, binding: null, ...extra,
    },
  };
}
let pending: unknown[] = [
  spend('a1', 'pengui.space', [{ asset: { type: 'xch' }, sent: 1_500_000_000_000, received: 0, conditional_received: 0, net: -1_500_000_000_000 }]),
  spend('a2', 'dexie.space', [
    { asset: { type: 'xch' }, sent: 250_000_000_000, received: 0, conditional_received: 0, net: -250_000_000_000 },
    { asset: { type: 'cat', asset_id: TAIL }, sent: 0, received: 0, conditional_received: 1_000_000, net: 0 },
  ], { partial: true, binding: { all_bound: true, unbound_user_spends: [], bound_payments: [{ user_spend: 0, settlement_spend: 1, asset: { type: 'cat', asset_id: TAIL }, amount: 1_000_000 }], bound_received: [] } }),
  { id: 'm1', sessionId: 's2', dapp: 'mintgarden.io', method: 'signMessage', expiresAt: now() + 48, kind: 'request',
    prompt: { type: 'sign_message', dapp: 'mintgarden.io', message_hex: '00', message_text: 'Sign in to mintgarden.io\nNonce: 8f2c41', public_key: 'b'.repeat(96) } },
];
const params = new URLSearchParams(location.search);
if (params.get('n')) pending = pending.slice(0, Number(params.get('n')));

mockIPC((cmd, args) => {
  console.log("[preview] ipc", cmd, JSON.stringify(args));
  if (cmd.endsWith('|pending_approvals')) return pending;
  if (cmd.endsWith('|list_sessions')) return [];
  if (cmd === 'get_user_themes') return { themes: [] };
  if (cmd.endsWith('|resolve_approval')) {
    const id = (args as { approvalId: string }).approvalId;
    pending = pending.filter((p) => (p as { id: string }).id !== id);
    void emit('klimper-xchonnect://approval-resolved', { id });
    return null;
  }
  return null;
}, { shouldMockEvents: true });

console.log('[preview] flag', import.meta.env.VITE_ENABLE_XCHONNECT);
