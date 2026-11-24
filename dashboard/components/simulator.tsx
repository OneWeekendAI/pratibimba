'use client';
// The Simulator: a dialog on a slot. Try one draft on a copy of the site (the sandbox) with
// synthetic visitors, and see whether it would have beaten the current champion. The real site
// is never touched; the sandbox is the same one Time travel uses.
import { useMutation } from '@tanstack/react-query';
import dynamic from 'next/dynamic';
import Link from 'next/link';
import { useEffect, useRef, useState } from 'react';
import { FlaskConical, X } from 'lucide-react';
import { Button, IconButton, inputClass } from '@/components/ui';
import { api, ApiError, type Counts, type CurvePoint, type Interval, type SlotDetail } from '@/lib/api';

const LiftCurve = dynamic(() => import('@/components/lift-curve').then((m) => m.LiftCurve), {
  ssr: false,
  loading: () => <div className="h-[160px] animate-pulse rounded-md bg-surface-2/60 motion-reduce:animate-none" aria-hidden />,
});

const BASE_RATE = 0.05;
const VISITORS_PER_DAY = 150;
const MAX_STEPS = 6;

interface SimVariant { key: string; counts: Counts; lift: Interval | null }
interface SimStatus { clock: string; results: { slots: { key: string; variants: SimVariant[] }[] } }

export type Verdict = 'better' | 'worse' | 'unclear';
export interface Outcome {
  champion: string;
  draft: string;
  curve: CurvePoint[];
  verdict: Verdict;
  champion_rate: number | null;
  draft_rate: number | null;
}

/** Two-proportion z test on credited conversions: better or worse only when it clears 95%. */
export function judge(champ: Counts, draft: Counts): { verdict: Verdict; a: number | null; b: number | null } {
  if (champ.n === 0 || draft.n === 0) return { verdict: 'unclear', a: null, b: null };
  const a = champ.credit / champ.n;
  const b = draft.credit / draft.n;
  const pool = (champ.credit + draft.credit) / (champ.n + draft.n);
  const se = Math.sqrt(pool * (1 - pool) * (1 / champ.n + 1 / draft.n));
  const z = se > 0 ? (b - a) / se : 0;
  return { verdict: z > 1.96 ? 'better' : z < -1.96 ? 'worse' : 'unclear', a, b };
}

const SAY: Record<Verdict, string> = {
  better: 'The draft would have beaten the champion.',
  worse: 'The champion holds up. The draft would have done worse.',
  unclear: 'No clear difference yet. Try more days.',
};

async function run(site: string, slot: string, champion: string, draft: string, days: number, points: number): Promise<Outcome> {
  const sandbox = `${site}~sim`;
  await api.post(`/sites/${site}/sim`, {});
  const inSandbox = await api.get<SlotDetail>(`/sites/${sandbox}/slots/${slot}`);
  const id = inSandbox.variants.find((v) => v.key === draft)?.id;
  if (id === undefined) throw new ApiError(404, { error: 'The draft is not in the sandbox.' });
  await api.post(`/variants/${id}/approve`, { approver: 'simulator', traffic_share: 0.5 });
  const rates = { [slot]: { [champion]: BASE_RATE, [draft]: Math.min(1, Math.max(0, BASE_RATE + points / 100)) } };
  // The cohort arrives a day at a time inside each step; a step ends in a reading, so the curve has a few points.
  const steps = Math.max(1, Math.min(MAX_STEPS, days));
  const curve: CurvePoint[] = [];
  let status: SimStatus | null = null;
  let done = 0;
  for (let i = 1; i <= steps; i++) {
    const upto = Math.round((days * i) / steps);
    const chunk = upto - done;
    if (chunk < 1) continue;
    done = upto;
    await api.post(`/sites/${site}/sim/advance`, { days: chunk, cohort: { visitors: VISITORS_PER_DAY, seed: 1000 + i, default_rate: BASE_RATE, rates } });
    status = await api.get<SimStatus>(`/sites/${site}/sim`);
    const s = status.results.slots.find((x) => x.key === slot);
    const at = (k: string) => s?.variants.find((v) => v.key === k)?.lift ?? null;
    curve.push({ t: status.clock, policy: null, variants: { [champion]: at(champion), [draft]: at(draft) } });
  }
  const last = status?.results.slots.find((x) => x.key === slot);
  const c = last?.variants.find((v) => v.key === champion)?.counts;
  const d = last?.variants.find((v) => v.key === draft)?.counts;
  const j = c && d ? judge(c, d) : { verdict: 'unclear' as Verdict, a: null, b: null };
  return { champion, draft, curve, verdict: j.verdict, champion_rate: j.a, draft_rate: j.b };
}

export function SimulatorButton({ site, slot, slotDetail }: { site: string; slot: string; slotDetail: SlotDetail }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="ghost" icon={FlaskConical} onClick={() => setOpen(true)} data-testid="open-simulator">Simulate</Button>
      {open && <SimulatorDialog site={site} slot={slot} s={slotDetail} onClose={() => setOpen(false)} />}
    </>
  );
}

function SimulatorDialog({ site, slot, s, onClose }: { site: string; slot: string; s: SlotDetail; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const drafts = s.variants.filter((v) => v.status === 'draft');
  const champion = s.champion ?? s.variants.find((v) => v.is_baseline)?.key ?? '';
  const [draft, setDraft] = useState(drafts[0]?.key ?? '');
  const [days, setDays] = useState('14');
  const [points, setPoints] = useState('3');
  const d = Number(days);
  const p = Number(points);
  const valid = !!draft && !!champion && Number.isInteger(d) && d >= 1 && d <= 60 && Number.isFinite(p) && p >= -5 && p <= 10;
  useEffect(() => {
    const el = ref.current;
    if (el && !el.open) el.showModal();
  }, []);
  const go = useMutation({ mutationFn: () => run(site, slot, champion, draft, d, p) });
  const out = go.data;
  const err = go.error ? (go.error instanceof ApiError ? go.error.message : 'The simulation failed.') : null;
  return (
    <dialog ref={ref} aria-label="Simulator" data-testid="simulator" onClose={onClose} className="m-auto w-[min(34rem,calc(100vw-2rem))] rounded-lg border border-line bg-surface p-0 text-ink shadow-pop backdrop:bg-black/30">
      <div className="flex items-center justify-between border-b border-line px-5 py-3">
        <h2 className="text-[14px] font-semibold">Simulator</h2>
        <IconButton icon={X} label="Close" onClick={() => ref.current?.close()} />
      </div>
      <div className="space-y-4 p-5 text-[13px]">
        {drafts.length === 0 ? (
          <p className="text-muted" data-testid="sim-no-drafts">No drafts on this slot. <Link href={`/drafts/?site=${site}&slot=${slot}`} className="text-ink underline">Add one</Link> to try it.</p>
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-3">
              <select aria-label="Draft" className={`${inputClass} w-44 font-mono`} value={draft} onChange={(e) => setDraft(e.target.value)}>
                {drafts.map((v) => <option key={v.id} value={v.key}>{v.key}</option>)}
              </select>
              <span className="text-muted">against <span className="font-mono text-ink">{champion}</span></span>
            </div>
            <div className="flex flex-wrap items-center gap-4">
              <label className="inline-flex items-center gap-2 text-muted">
                Days
                <input className={`${inputClass} w-20 tabular-nums`} type="number" min={1} max={60} aria-label="Days" value={days} onChange={(e) => setDays(e.target.value)} />
              </label>
              <label className="inline-flex items-center gap-2 text-muted">
                Draft converts
                <input className={`${inputClass} w-20 tabular-nums`} type="number" min={-5} max={10} step={1} aria-label="Points better" value={points} onChange={(e) => setPoints(e.target.value)} />
                points better
              </label>
            </div>
            <div className="flex items-center gap-3 border-t border-line pt-4">
              <Button icon={FlaskConical} disabled={!valid || go.isPending} onClick={() => go.mutate()}>{go.isPending ? 'Running' : 'Run'}</Button>
              <span className="text-[12px] text-muted">Synthetic visitors, on a copy of your site.</span>
            </div>
          </>
        )}
        {err && <p role="alert" className="text-xs text-pause-ink">{err}</p>}
        {out && (
          <div className="space-y-3 border-t border-line pt-4" data-testid="sim-result">
            <p className="text-[14px] font-medium" data-testid="sim-verdict" data-verdict={out.verdict}>{SAY[out.verdict]}</p>
            <p className="text-muted tabular" data-testid="sim-rates">
              <span className="font-mono text-ink-2">{out.champion}</span> {out.champion_rate === null ? '—' : `${(out.champion_rate * 100).toFixed(1)}%`}
              {' · '}
              <span className="font-mono text-ink-2">{out.draft}</span> {out.draft_rate === null ? '—' : `${(out.draft_rate * 100).toFixed(1)}%`}
            </p>
            <LiftCurve curve={out.curve} series={[{ key: out.champion, index: 0 }, { key: out.draft, index: 1 }]} height={180} highlight={out.draft} />
          </div>
        )}
      </div>
    </dialog>
  );
}
