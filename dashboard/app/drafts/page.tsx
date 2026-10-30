'use client';
// Build: write a draft for a slot, and upload the images a draft can use. Nothing here reaches a
// visitor until someone approves it in Review.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import Link from 'next/link';
import clsx from 'clsx';
import { ChevronRight, CircleCheck, FilePen, RotateCcw, Save } from 'lucide-react';
import { Suspense, useEffect, useState } from 'react';
import { AssetShelf } from '@/components/assets';
import { Preview } from '@/components/preview';
import { Shell } from '@/components/shell';
import { Button, Card, Empty, Field, Help, Loading, SectionTitle, inputClass } from '@/components/ui';
import { api, ApiError, type Approvals, type Asset, type CopyField, type FieldValue, type HistorySettings, type Overview, type SlotDetail } from '@/lib/api';
import { useParam } from '@/lib/page';

/** A copy slot's fields as inputs: a line of text, or an image (an upload, with alt text). */
function CopyInputs({ fields, values, setValues, assets }: { fields: CopyField[]; values: Record<string, FieldValue>; setValues: (v: Record<string, FieldValue>) => void; assets: Asset[] }) {
  const set = (k: string, v: FieldValue | undefined) => {
    const next = { ...values };
    if (v === undefined || v === '') delete next[k];
    else next[k] = v;
    setValues(next);
  };
  return (
    <div className="grid gap-3 md:col-span-3 md:grid-cols-2" data-testid="copy-inputs">
      {fields.map((f) => {
        const v = values[f.name];
        if (f.type !== 'image') {
          return (
            <Field key={f.name} label={`${f.name}${f.required ? ' *' : ''}`} hint={`up to ${f.max}`}>
              <input className={`${inputClass} w-full`} aria-label={f.name} maxLength={f.max} value={typeof v === 'string' ? v : ''} onChange={(e) => set(f.name, e.target.value)} />
            </Field>
          );
        }
        const img = typeof v === 'object' ? v : undefined;
        return (
          <Field key={f.name} label={`${f.name}${f.required ? ' *' : ''} (image)`}>
            <div className="flex gap-2">
              <select className={`${inputClass} w-1/2`} aria-label={`${f.name} image`} value={img?.asset ?? ''}
                onChange={(e) => {
                  const a = assets.find((x) => x.id === e.target.value);
                  set(f.name, a ? { asset: a.id, alt: img?.alt ?? '', ...(a.width && a.height ? { width: a.width, height: a.height } : {}) } : undefined);
                }}>
                <option value="">Pick an upload</option>
                {assets.map((a) => <option key={a.id} value={a.id}>{a.name ?? a.id.slice(0, 12)}</option>)}
              </select>
              <input className={`${inputClass} w-1/2`} aria-label={`${f.name} alt text`} placeholder="Alt text" value={img?.alt ?? ''} disabled={!img}
                onChange={(e) => img && set(f.name, { ...img, alt: e.target.value })} />
            </div>
          </Field>
        );
      })}
    </div>
  );
}

function NewDraft({ site, slots, initial }: { site: string; slots: string[]; initial: string | null }) {
  const qc = useQueryClient();
  const [slot, setSlot] = useState(initial ?? slots[0] ?? '');
  const [key, setKey] = useState('');
  const [html, setHtml] = useState('');
  const [values, setValues] = useState<Record<string, FieldValue>>({});
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  useEffect(() => {
    if (!slot && slots[0]) setSlot(slots[0]);
  }, [slots, slot]);
  const detail = useQuery({ queryKey: ['slot', site, slot], queryFn: () => api.get<SlotDetail>(`/sites/${site}/slots/${slot}`), enabled: !!slot });
  const assets = useQuery({ queryKey: ['assets', site], queryFn: () => api.get<{ assets: Asset[] }>(`/sites/${site}/assets`) });
  const fields = detail.data?.kind === 'copy' ? detail.data.fields : null;
  const payload = fields ? JSON.stringify(values) : html;
  const create = useMutation({
    mutationFn: () => api.post(`/sites/${site}/slots/${slot}/variants`, { key, html: payload }),
    onSuccess: () => {
      setSaved(key);
      setKey('');
      setHtml('');
      setValues({});
      setError(null);
      qc.invalidateQueries({ queryKey: ['approvals', site] });
      qc.invalidateQueries({ queryKey: ['sites'] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  return (
    <Card className="p-5 sm:p-6">
      <SectionTitle icon={FilePen} help={<><p>Pick a slot, give the draft a lowercase key such as <code className="font-mono">gift_v2</code>, and write its content.</p><p>For an image, upload it under Images and use <code className="font-mono">{'<img src="/_prb/assets/…" alt="…">'}</code>.</p></>}>Write a draft</SectionTitle>
      <form
        className="grid gap-4 md:grid-cols-[180px_220px_1fr]"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <Field label="Slot">
          <select className={`${inputClass} w-full`} value={slot} onChange={(e) => { setSlot(e.target.value); setValues({}); }}>{slots.map((s) => <option key={s}>{s}</option>)}</select>
        </Field>
        <Field label="Key">
          <input className={`${inputClass} w-full font-mono`} value={key} onChange={(e) => setKey(e.target.value)} placeholder="new_variant" />
        </Field>
        {fields ? (
          <div className="inline-flex items-center gap-1 text-xs text-muted md:self-end">Copy slot <Help title="Copy slot"><p>Fill the fields you want to change. The page keeps its own text for any you leave empty.</p></Help></div>
        ) : (
          <Field label="HTML">
            <textarea className={`${inputClass} h-24 w-full resize-y py-2 font-mono max-sm:h-24 text-[12.5px]`} value={html} onChange={(e) => setHtml(e.target.value)} placeholder="<h1>…</h1>" />
          </Field>
        )}
        {fields && <CopyInputs fields={fields} values={values} setValues={setValues} assets={assets.data?.assets ?? []} />}
        {!fields && html.trim() && <div className="md:col-span-3"><Preview html={html} label="draft preview" height={120} /></div>}
        <div className="flex flex-wrap items-center gap-3 border-t border-line pt-4 md:col-span-3">
          <Button type="submit" icon={Save} disabled={!key || !slot || (fields ? Object.keys(values).length === 0 : !html) || create.isPending}>Save as draft</Button>
          {error && <span role="alert" className="text-[13px] text-pause-ink">{error}</span>}
          {saved && !error && (
            <span role="status" className="contents">
            <Link href={`/approvals/?site=${site}`} className="inline-flex items-center gap-1.5 text-[13px] text-good-ink hover:underline" data-testid="draft-saved">
              <CircleCheck className="size-4" aria-hidden /><span className="font-mono">{saved}</span> saved. Review it <ChevronRight className="size-4" aria-hidden />
            </Link>
            </span>
          )}
        </div>
      </form>
    </Card>
  );
}

/** P1: how a template reads the visitor's history (per site): the look-back and what one visit is. */
function HistoryPanel({ site }: { site: string }) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['history-settings', site], queryFn: () => api.get<HistorySettings>(`/sites/${site}/history-settings`) });
  const [days, setDays] = useState('');
  const [visit, setVisit] = useState<'day' | 'session'>('day');
  const [gap, setGap] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!q.data) return;
    setDays(String(q.data.top_item_days));
    setVisit(q.data.visit);
    setGap(String(q.data.session_gap_minutes));
  }, [q.data]);
  const save = useMutation({
    mutationFn: () => api.post<HistorySettings>(`/sites/${site}/history-settings`, { top_item_days: Number(days), visit, ...(visit === 'session' ? { session_gap_minutes: Number(gap) } : {}) }),
    onSuccess: (d) => {
      qc.setQueryData(['history-settings', site], d);
      setError(null);
      setSaved(true);
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  if (!q.data) return null;
  const { bounds: b, defaults: d } = q.data;
  const within = (v: string, [lo, hi]: [number, number]) => /^\d+$/.test(v) && Number(v) >= lo && Number(v) <= hi;
  const daysOk = within(days, b.top_item_days);
  const gapOk = visit === 'day' || within(gap, b.session_gap_minutes);
  const dirty = Number(days) !== q.data.top_item_days || visit !== q.data.visit || (visit === 'session' && Number(gap) !== q.data.session_gap_minutes);
  const isDefault = days === String(d.top_item_days) && visit === d.visit && (visit === 'day' || gap === String(d.session_gap_minutes));
  const edit = (f: () => void) => {
    f();
    setSaved(false);
  };
  return (
    <Card className="p-5 sm:p-6" data-testid="history-settings">
      <SectionTitle help={<><p>A template such as “your usual {'{visitor.top_item.name}'}” reads the visitor’s orders and visits from this window.</p><p>A visit is a day with any activity, or a session that ends after a quiet gap.</p></>}>Visitor history</SectionTitle>
      <form
        className="flex flex-wrap items-start gap-x-6 gap-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          if (daysOk && gapOk) save.mutate();
        }}
      >
        <div className="w-40"><Field label="Look back (days)" hint={<span className={clsx(!daysOk && 'text-pause-ink')}>{b.top_item_days[0]}–{b.top_item_days[1]}, default {d.top_item_days}</span>}>
          <input type="number" inputMode="numeric" min={b.top_item_days[0]} max={b.top_item_days[1]} aria-invalid={!daysOk} className={`${inputClass} w-full tabular`} value={days} onChange={(e) => edit(() => setDays(e.target.value))} />
        </Field></div>
        <div>
          <span className="mb-1.5 block text-[13px] font-medium text-ink-2" id="visit-label">A visit is</span>
          <div role="radiogroup" aria-labelledby="visit-label" className="inline-flex h-8 rounded-md bg-surface-2 p-0.5 text-[13px]">
            {(['day', 'session'] as const).map((v) => (
              <button key={v} type="button" role="radio" aria-checked={visit === v} onClick={() => edit(() => setVisit(v))} className={clsx('h-7 rounded-[5px] px-3 transition-colors duration-150', visit === v ? 'bg-surface font-medium text-ink shadow-[0_0_0_1px_var(--line),0_1px_2px_rgb(0_0_0/0.06)]' : 'text-muted hover:text-ink')}>
                {v === 'day' ? 'A day' : 'A session'}
              </button>
            ))}
          </div>
        </div>
        {visit === 'session' && (
          <div className="w-40"><Field label="Session gap (minutes)" hint={<span className={clsx(!gapOk && 'text-pause-ink')}>{b.session_gap_minutes[0]}–{b.session_gap_minutes[1]}, default {d.session_gap_minutes}</span>}>
            <input type="number" inputMode="numeric" min={b.session_gap_minutes[0]} max={b.session_gap_minutes[1]} aria-invalid={!gapOk} className={`${inputClass} w-full tabular`} value={gap} onChange={(e) => edit(() => setGap(e.target.value))} />
          </Field></div>
        )}
        <div className="flex w-full flex-wrap items-center gap-2 border-t border-line pt-4">
          <Button type="submit" icon={Save} disabled={!dirty || !daysOk || !gapOk || save.isPending}>Save</Button>
          <Button variant="quiet" icon={RotateCcw} disabled={isDefault} onClick={() => edit(() => { setDays(String(d.top_item_days)); setVisit(d.visit); setGap(String(d.session_gap_minutes)); })}>Restore defaults</Button>
          {error && <span role="alert" className="text-[13px] text-pause-ink">{error}</span>}
          {saved && !error && !dirty && <span role="status" className="inline-flex items-center gap-1.5 text-[13px] text-good-ink"><CircleCheck className="size-4" aria-hidden />Saved</span>}
        </div>
      </form>
    </Card>
  );
}

function Waiting({ site }: { site: string }) {
  const q = useQuery({ queryKey: ['approvals', site], queryFn: () => api.get<Approvals>(`/sites/${site}/approvals`) });
  const n = q.data?.drafts.length ?? 0;
  if (!n) return null;
  return (
    <Link href={`/approvals/?site=${site}`} className="inline-flex items-center gap-2 rounded-md bg-warn-soft px-3 py-2 text-[13px] font-medium text-warn-ink transition-opacity hover:opacity-85" data-testid="drafts-waiting">
      {n} waiting for review <ChevronRight className="size-4" aria-hidden />
    </Link>
  );
}

function View() {
  const site = useParam('site')!;
  const initialSlot = useParam('slot');
  const ov = useQuery({ queryKey: ['overview', site], queryFn: () => api.get<Overview>(`/sites/${site}/overview`) });
  if (!ov.data) return <Loading />;
  const slots = ov.data.slots.map((s) => s.key);
  return (
    <div className="max-w-5xl space-y-8">
      <Waiting site={site} />
      <NewDraft site={site} slots={slots} initial={initialSlot} />
      <AssetShelf site={site} />
      <HistoryPanel site={site} />
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title="Drafts" help="A draft is never shown to a visitor until someone approves it in Review.">
      {site ? <View /> : <Empty>No site given.</Empty>}
    </Shell>
  );
}

export default function DraftsPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
