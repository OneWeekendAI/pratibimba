'use client';
// The slot's audience gates: which audience each live variant is shown to, and a short form for a
// new rule. History rules (F3) read a returning visitor's own orders and views, so they match only
// identified visitors; everyone else fails closed.
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { useActor } from '@/components/shell';
import { Button, SectionTitle, inputClass } from '@/components/ui';
import { api, ApiError, type AudienceRow, type VariantRow } from '@/lib/api';

const HISTORY_KEYS = ['days_since_last_purchase', 'purchase_count', 'lifetime_value', 'cart_abandoned', 'viewed'];

type Kind = 'lapsed' | 'orders' | 'ltv' | 'cart' | 'viewed_item' | 'viewed_category';
const KINDS: [Kind, string][] = [
  ['lapsed', 'Last order at least N days ago'],
  ['orders', 'At least N orders'],
  ['ltv', 'Lifetime value'],
  ['cart', 'Left a cart'],
  ['viewed_item', 'Viewed an item'],
  ['viewed_category', 'Viewed a category'],
];

export function readsHistory(p: Record<string, unknown>): boolean {
  return Object.keys(p).some((k) => HISTORY_KEYS.includes(k));
}

function predicate(kind: Kind, n: number, text: string, ltv: string, days: number): Record<string, unknown> {
  switch (kind) {
    case 'lapsed': return { days_since_last_purchase: { gte: n } };
    case 'orders': return { purchase_count: { gte: n } };
    case 'ltv': return { lifetime_value: ltv };
    case 'cart': return { cart_abandoned: true };
    case 'viewed_item': return { viewed: { item: text.trim(), days } };
    case 'viewed_category': return { viewed: { category: text.trim(), days } };
  }
}

export function Audiences({ site, slot, variants, audiences }: { site: string; slot: string; variants: VariantRow[]; audiences: AudienceRow[] }) {
  const qc = useQueryClient();
  const [actor] = useActor();
  const who = actor.trim() || 'owner';
  const [error, setError] = useState<string | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: ['slot', site, slot] });
  const fail = (e: unknown) => setError(e instanceof ApiError ? e.message : 'failed');
  const gate = useMutation({
    mutationFn: ({ id, audience }: { id: number; audience: number | null }) => api.post(`/variants/${id}/audience`, { audience, actor: who }),
    onSuccess: () => { setError(null); refresh(); },
    onError: fail,
  });

  const [kind, setKind] = useState<Kind>('lapsed');
  const [n, setN] = useState(30);
  const [text, setText] = useState('');
  const [ltv, setLtv] = useState('high');
  const [days, setDays] = useState(30);
  const create = useMutation({
    mutationFn: async () => {
      const seg = await api.post<{ id: number }>(`/sites/${site}/segments`, { predicate: predicate(kind, n, text, ltv, days), actor: who });
      await api.post(`/segments/${seg.id}/promote`, { actor: who });
    },
    onSuccess: () => { setError(null); refresh(); },
    onError: fail,
  });

  const live = variants.filter((v) => v.status === 'approved');
  const needsText = kind === 'viewed_item' || kind === 'viewed_category';

  return (
    <div>
      <SectionTitle>Audiences</SectionTitle>
      <ul className="space-y-2 text-xs text-ink-2" aria-label="Variant audiences">
        {live.map((v) => (
          <li key={v.id} className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
            <span className="font-mono whitespace-nowrap text-ink">{v.key}</span>
            {v.is_baseline ? (
              <span className="text-muted">everyone</span>
            ) : (
              <select
                aria-label={`Audience for ${v.key}`}
                className={`${inputClass} h-8 max-w-full text-xs`}
                value={v.audience ?? ''}
                disabled={gate.isPending}
                onChange={(e) => gate.mutate({ id: v.id, audience: e.target.value ? Number(e.target.value) : null })}
              >
                <option value="">everyone</option>
                {audiences.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}
              </select>
            )}
          </li>
        ))}
      </ul>
      {audiences.some((a) => readsHistory(a.predicate)) && (
        <p className="mt-2 text-xs text-muted">Rules on order or view history match identified visitors only.</p>
      )}
      <form
        className="mt-4 space-y-2 border-t border-line pt-3"
        aria-label="New audience"
        onSubmit={(e) => { e.preventDefault(); create.mutate(); }}
      >
        <select aria-label="Rule" className={`${inputClass} w-full text-xs`} value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
          {KINDS.map(([k, label]) => <option key={k} value={k}>{label}</option>)}
        </select>
        <div className="flex gap-2">
          {(kind === 'lapsed' || kind === 'orders') && (
            <input aria-label="N" type="number" min={kind === 'lapsed' ? 0 : 1} className={`${inputClass} w-20 text-xs`} value={n} onChange={(e) => setN(Number(e.target.value))} />
          )}
          {kind === 'ltv' && (
            <select aria-label="Value" className={`${inputClass} text-xs`} value={ltv} onChange={(e) => setLtv(e.target.value)}>
              {['none', 'low', 'mid', 'high'].map((b) => <option key={b}>{b}</option>)}
            </select>
          )}
          {needsText && (
            <>
              <input aria-label={kind === 'viewed_item' ? 'Item id' : 'Category'} className={`${inputClass} min-w-0 flex-1 text-xs`} value={text} onChange={(e) => setText(e.target.value)} />
              <input aria-label="Days" type="number" min={1} max={90} className={`${inputClass} w-16 text-xs`} value={days} onChange={(e) => setDays(Number(e.target.value))} />
            </>
          )}
        </div>
        <Button type="submit" variant="ghost" className="h-8 w-full text-xs" disabled={create.isPending || (needsText && !text.trim())}>
          Add audience
        </Button>
      </form>
      {error && <p role="alert" className="mt-2 text-xs text-pause-ink">{error}</p>}
    </div>
  );
}
