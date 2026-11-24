'use client';
// The Preview room: the real site on the left, cut into its slots; every design of the open slot
// on the right, each drawn by the real page. Click a design and the page shows it in place.
// The site runs in preview mode (a signed, short-lived token): it decides nothing, logs nothing
// and learns nothing; it shows every version a visitor could get, and this page picks which.
import clsx from 'clsx';
import Link from 'next/link';
import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowLeft, CornerDownLeft, History, Layers, Megaphone, Moon, Plus, RotateCcw, ShoppingBag, ShoppingCart, Smartphone, StepForward, UserRound, Users, UtensilsCrossed, X, type LucideIcon } from 'lucide-react';
import { Mark, useActor } from '@/components/shell';
import { Divider, Thumb, useSplit } from '@/components/explorer';
import { Chip, Help, Tip, type ChipTone } from '@/components/ui';
import { api, ApiError, type Persona, type SlotDetail } from '@/lib/api';
import { useParam } from '@/lib/page';
import { human } from '@/lib/plain';
import { cardsOf, ctxOf, describePayload, FIRST_VISIT, pageUrl, TOGGLES, type CardStatus, type RoomSlot, type VariantCard, type Visitor } from '@/lib/preview';

interface Minted {
  url: string;
  page: string;
  token: string;
  expires_in: number;
}

const ICON: Record<string, LucideIcon> = { returning: RotateCcw, mobile: Smartphone, from_ad: Megaphone, evening: Moon };
// A persona's chip icon; one the server adds later gets the plain history mark.
const PERSONA_ICON: Record<string, LucideIcon> = { three_orders_40d: ShoppingBag, usual_rajma: UtensilsCrossed, cart_yesterday: ShoppingCart };

const STATUS: Record<CardStatus, { tone: ChipTone; label: string }> = {
  original: { tone: 'off', label: 'Original' },
  live: { tone: 'live', label: 'Live' },
  draft: { tone: 'warn', label: 'Draft' },
  paused: { tone: 'paused', label: 'Paused' },
};

const KIND: Record<string, { label: string; tip: string }> = {
  swap: { label: 'Any design', tip: 'Each design is its own HTML: a different layout, not only different words.' },
  copy: { label: 'Copy', tip: 'Same layout, different words or images.' },
  sorted: { label: 'Order', tip: 'The same items; the order changes for each visitor.' },
  picker: { label: 'Picker', tip: 'Asks the visitor to choose.' },
};

function chip(on: boolean) {
  return clsx(
    'inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full px-3 text-[13px] font-medium transition-colors duration-150 sm:h-7',
    on ? 'bg-primary text-primary-ink' : 'text-ink-2 hover:bg-surface-2 hover:text-ink',
  );
}

function Room({ site }: { site: string }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const split = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [minted, setMinted] = useState<Minted | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [visitor, setVisitor] = useState<Visitor>(FIRST_VISIT);
  const [slots, setSlots] = useState<RoomSlot[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [zones, setZones] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  // A path tried for a slot that declares none, and the box's text.
  const [tries, setTries] = useState<Record<string, string>>({});
  const [tryDraft, setTryDraft] = useState('');
  const [frameW, setFrameW] = useState(0);
  const [thumbW, setThumbW] = useState(0);
  const [frameH, setFrameH] = useState(800);
  const [pct, setPct] = useSplit();
  const [actor, setActor] = useActor();
  const qc = useQueryClient();
  const visitorRef = useRef(visitor);
  visitorRef.current = visitor;
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  // A pick this page sent and has not seen back yet; and a design to show once a visitor switch lands.
  const sentPick = useRef<string | null>(null);
  const showAfter = useRef<{ slot: string; key: string } | null>(null);
  const origin = minted ? new URL(minted.page).origin : null;

  const mint = useCallback(async () => {
    try {
      setMinted(await api.post<Minted>(`/sites/${site}/preview-token`, {}));
      setFailed(null);
    } catch (e) {
      setFailed(e instanceof ApiError ? e.message : 'Could not open a preview.');
    }
  }, [site]);
  useEffect(() => {
    mint();
  }, [mint]);

  const tell = useCallback(
    (msg: object) => {
      if (origin) frame.current?.contentWindow?.postMessage(msg, origin);
    },
    [origin],
  );
  const show = useCallback((v: Visitor, quiet: boolean) => tell({ type: 'prb:visitor', ctx: ctxOf(v), persona: v.persona, quiet }), [tell]);
  const personas = useQuery({ queryKey: ['personas'], queryFn: () => api.get<{ personas: Persona[] }>('/personas'), staleTime: Infinity });
  const pick = useCallback(
    (slot: string, index: number, scroll = false) => {
      sentPick.current = slot;
      tell({ type: 'prb:pick', slot, index, scroll });
    },
    [tell],
  );

  // A fresh token before this one runs out, handed to the page without a reload.
  useEffect(() => {
    if (!minted) return;
    const t = setTimeout(async () => {
      const next = await api.post<Minted>(`/sites/${site}/preview-token`, {}).catch(() => null);
      if (next) tell({ type: 'prb:token', token: next.token });
    }, Math.max(60, minted.expires_in - 300) * 1000);
    return () => clearTimeout(t);
  }, [minted, site, tell]);

  useEffect(() => {
    const on = (e: MessageEvent) => {
      if (!origin || e.origin !== origin || e.source !== frame.current?.contentWindow) return;
      const m = e.data ?? {};
      if (m.type === 'prb:hello') {
        const can = Array.isArray(m.can) && m.can.includes('view');
        setZones(can);
        if (can) tell({ type: 'prb:view', zones: true });
        show(visitorRef.current, true);
      } else if (m.type === 'prb:expired') mint();
      else if (m.type === 'prb:state') {
        const all: RoomSlot[] = Array.isArray(m.slots) ? m.slots : [];
        setSlots(all);
        const got: string | null = typeof m.picked === 'string' ? m.picked : null;
        if (sentPick.current && got === sentPick.current) sentPick.current = null;
        const want = selectedRef.current;
        const waiting = showAfter.current;
        if (waiting) {
          const s = all.find((x) => x.slot === waiting.slot);
          const i = s ? s.versions.findIndex((v) => v.key === waiting.key) : -1;
          if (i >= 0) {
            showAfter.current = null;
            pick(waiting.slot, i, true);
            return;
          }
        }
        if (got && got !== want && !sentPick.current) {
          // A click on a compartment in the page.
          setSelected(got);
        } else if (!got && !sentPick.current) {
          // Nothing open in the page (its first answer, or a click outside every slot): keep ours open.
          const target = want ?? all.find((s) => s.on_page)?.slot ?? all[0]?.slot ?? null;
          const s = all.find((x) => x.slot === target);
          if (target && s) {
            if (!want) setSelected(target);
            pick(target, s.index);
          }
        }
      }
    };
    window.addEventListener('message', on);
    return () => window.removeEventListener('message', on);
  }, [origin, show, mint, tell, pick]);

  // The page's width, for the thumbnails: they render at the same width, so they look the same.
  useEffect(() => {
    const el = frame.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setFrameW(el.clientWidth);
      setFrameH(el.clientHeight);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [minted]);
  useEffect(() => {
    const t = setTimeout(() => setThumbW(frameW), thumbW ? 300 : 0);
    return () => clearTimeout(t);
  }, [frameW, thumbW]);

  const room = slots.find((s) => s.slot === selected) ?? null;
  const detail = useQuery({
    queryKey: ['slot', site, selected],
    queryFn: () => api.get<SlotDetail>(`/sites/${site}/slots/${selected}`),
    enabled: !!selected,
  });
  const cards = useMemo(() => cardsOf(room, detail.data && detail.data.slot === selected ? detail.data : null, visitor), [room, detail.data, selected, visitor]);
  const declared = detail.data && detail.data.slot === selected ? (detail.data.page ?? null) : null;
  const cardPage = declared ?? (selected ? (tries[selected] ?? null) : null);
  const cardSrc = minted ? pageUrl(minted.url, cardPage) : '';
  const words = useMemo(() => {
    const m = new Map<string, ReturnType<typeof describePayload>>();
    if (detail.data && detail.data.slot === selected) for (const v of detail.data.variants) m.set(v.key, v.is_baseline ? null : describePayload(detail.data.kind, v.payload));
    return m;
  }, [detail.data, selected]);
  const shownKey = room ? (room.versions[room.index]?.key ?? null) : null;
  const shown = cards.find((c) => c.key === shownKey) ?? null;
  useEffect(() => setConfirming(false), [shownKey, selected]);

  const choose = (v: Visitor) => {
    setVisitor(v);
    setNote(null);
    show(v, false);
  };
  const toggle = (k: keyof Visitor) => choose({ ...visitor, [k]: !visitor[k] });
  const first = visitor.persona === FIRST_VISIT.persona && !TOGGLES.some((t) => visitor[t.key]);
  const changed = slots.filter((s) => s.changed && s.on_page).length;

  const goTry = () => {
    const p = tryDraft.trim();
    if (!selected || !p.startsWith('/')) return;
    setTries((t) => ({ ...t, [selected]: p }));
  };
  useEffect(() => setTryDraft(selected ? (tries[selected] ?? '') : ''), [selected]); // eslint-disable-line react-hooks/exhaustive-deps

  const openSlot = (slot: string) => {
    setSelected(slot);
    setNote(null);
    const s = slots.find((x) => x.slot === slot);
    if (s) pick(slot, s.index, true);
  };

  const showCard = (c: VariantCard) => {
    if (!room) return;
    setNote(null);
    if (!c.visitor) {
      setNote(c.status === 'paused' ? `${c.key} is paused.` : `No preview visitor gets ${c.key}.`);
      return;
    }
    const i = room.versions.findIndex((v) => v.key === c.key);
    if (i >= 0) return pick(room.slot, i, true);
    // Made for another audience: become that visitor, then show it.
    showAfter.current = { slot: room.slot, key: c.key };
    setVisitor(c.visitor);
    show(c.visitor, false);
  };

  const live = cards.filter((c) => c.visitor);
  const step = (d: 1 | -1) => {
    if (!live.length) return;
    const at = live.findIndex((c) => c.key === shownKey);
    const next = live[(at + d + live.length) % live.length];
    showCard(next);
    list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(next.key)}"]`)?.focus();
  };

  const reject = async (c: VariantCard) => {
    if (!c.id || c.baseline) return;
    setBusy(true);
    try {
      if (c.status === 'draft') await api.post(`/variants/${c.id}/reject`, { actor: actor.trim(), comment: 'Rejected in preview' });
      else await api.post(`/variants/${c.id}/retire`, { actor: actor.trim(), comment: 'Rejected in preview' });
      setNote(`${c.key} is off the site.`);
      setConfirming(false);
      qc.invalidateQueries({ queryKey: ['slot', site] });
      show(visitor, true);
    } catch (e) {
      setNote(e instanceof ApiError ? e.message : 'That did not work.');
    } finally {
      setBusy(false);
    }
  };

  const kind = room?.kind ?? detail.data?.kind ?? null;
  const designs = cards.filter((c) => !c.baseline).length;
  const onlyOwn = !!room && (kind === 'swap' || kind === 'copy') && designs === 0 && !detail.isLoading;
  const drafts = `/drafts/?site=${site}${selected ? `&slot=${selected}` : ''}`;
  const vw = Math.max(320, Math.round(thumbW || frameW || 1024));

  return (
    <div className="flex h-dvh flex-col bg-paper">
      <div role="toolbar" aria-label="Preview" className="flex min-h-12 shrink-0 flex-wrap items-center gap-1 border-b border-line bg-surface px-2 py-1.5 sm:px-3">
        <Tip label="Back to Review">
          <Link href={`/approvals/?site=${site}`} aria-label="Back to the dashboard" className="mr-1 inline-flex h-8 items-center gap-2 rounded-md px-2 text-ink transition-colors hover:bg-surface-2 sm:mr-3">
            <ArrowLeft className="size-4" aria-hidden />
            <Mark className="size-5" />
            <span className="text-[14px] font-semibold tracking-[-0.02em] max-lg:hidden">Preview</span>
          </Link>
        </Tip>
        <Tip label="A visitor with no history">
          <button type="button" aria-pressed={first} aria-label="First visit" className={chip(first)} onClick={() => choose(FIRST_VISIT)}>
            <UserRound className="size-4" aria-hidden /><span className="max-md:hidden">First visit</span>
          </button>
        </Tip>
        {TOGGLES.map((t) => {
          const I = ICON[t.key];
          return (
            <Tip key={t.key} label={t.label}>
              <button type="button" aria-pressed={visitor[t.key]} aria-label={t.label} className={chip(visitor[t.key])} onClick={() => toggle(t.key)}>
                <I className="size-4" aria-hidden /><span className="max-md:hidden">{t.label}</span>
              </button>
            </Tip>
          );
        })}
        {(personas.data?.personas ?? []).some((p) => p.key !== FIRST_VISIT.persona) && <span aria-hidden className="mx-1 h-5 w-px bg-line sm:mx-2" />}
        <div role="group" aria-label="History" className="contents" data-testid="personas">
          {(personas.data?.personas ?? [])
            .filter((p) => p.key !== FIRST_VISIT.persona)
            .map((p) => {
              const I = PERSONA_ICON[p.key] ?? History;
              const on = visitor.persona === p.key;
              return (
                <Tip key={p.key} label={p.about}>
                  <button type="button" aria-pressed={on} aria-label={p.label} data-testid={`persona-${p.key}`} className={chip(on)} onClick={() => choose({ ...visitor, persona: on ? FIRST_VISIT.persona : p.key })}>
                    <I className="size-4" aria-hidden /><span className="max-md:hidden">{p.label}</span>
                  </button>
                </Tip>
              );
            })}
        </div>
        <span aria-hidden className="mx-1 h-5 w-px bg-line sm:mx-2" />
        <Tip label="The same visitor, coming back">
          <button type="button" aria-label="Next visit" className="inline-flex h-8 items-center gap-1.5 rounded-md px-2.5 text-[13px] font-medium text-ink shadow-[0_0_0_1px_var(--line-strong)] transition-colors hover:bg-surface-2 sm:h-7" onClick={() => choose({ ...visitor, returning: true })}>
            <StepForward className="size-4" aria-hidden /><span className="max-md:hidden">Next visit</span>
          </button>
        </Tip>
        <span className="ml-auto truncate pr-1 text-[12.5px] text-ink-2 max-sm:w-full max-sm:pl-1" role="status" data-testid="preview-status">
          {note ?? (changed ? `${changed} ${changed === 1 ? 'change' : 'changes'} from the last view` : '')}
        </span>
      </div>

      <div ref={split} className="flex min-h-0 flex-1 max-md:flex-col">
        <section aria-label="Your site" className="relative min-h-0 min-w-0 bg-surface-2 max-md:h-[52%] shrink-0 md:w-[var(--left)]" style={{ ['--left' as string]: `${pct}%` }}>
          {failed && <p className="p-8 text-[13px] text-pause-ink">{failed}</p>}
          {minted && (
            <iframe
              ref={frame}
              title="Your site, in preview"
              src={minted.url}
              className={clsx('h-full border-0 bg-white', visitor.mobile ? 'mx-auto block w-full max-w-[390px] border-x border-line shadow-sm' : 'w-full')}
            />
          )}
        </section>
        <Divider container={split} pct={pct} onChange={setPct} />
        <aside aria-label="Designs" className="flex min-h-0 min-w-0 flex-1 flex-col bg-paper max-md:border-t max-md:border-line">
          <div className="shrink-0 border-b border-line bg-surface">
            <div role="tablist" aria-label="Slots" className="flex gap-1 overflow-x-auto px-3 py-2 [scrollbar-width:none]">
              {slots.map((s) => {
                const on = s.slot === selected;
                return (
                  <button
                    key={s.slot}
                    type="button"
                    role="tab"
                    aria-selected={on}
                    data-testid={`slot-tab-${s.slot}`}
                    onClick={() => openSlot(s.slot)}
                    className={clsx(
                      'inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 font-mono text-[12px] transition-colors duration-150',
                      on ? 'bg-surface-3 text-ink' : 'text-ink-2 hover:bg-surface-2 hover:text-ink',
                      !s.on_page && 'opacity-50',
                    )}
                  >
                    {s.slot}
                    {s.changed && <span aria-label="changed" className="size-1.5 rounded-full bg-accent" />}
                  </button>
                );
              })}
            </div>
          </div>
          {room && (
            <div className="flex shrink-0 items-center gap-2 px-4 pt-4 pb-2">
              <h2 className="truncate text-[15px] font-semibold tracking-[-0.01em] text-ink">{human(room.slot)}</h2>
              {kind && KIND[kind] && (
                <Tip label={KIND[kind].tip}>
                  <span tabIndex={0} className="inline-flex h-[22px] items-center rounded-full px-2 text-[12px] font-medium text-ink-2 shadow-[0_0_0_1px_var(--line-strong)]">{KIND[kind].label}</span>
                </Tip>
              )}
              <span className="text-[12.5px] text-muted tabular" data-testid="design-count">{cards.length} {cards.length === 1 ? 'design' : 'designs'}</span>
              <Help title="Designs">
                <p>Each card is drawn by your page. Click one to see it in place.</p>
                <p>Up and down move through them. A design made for an audience switches the visitor above.</p>
              </Help>
              <Tip label="Write a new design in Drafts">
                <Link href={drafts} aria-label="Add a design" className="ml-auto grid size-8 shrink-0 place-items-center rounded-md text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink">
                  <Plus className="size-4" aria-hidden />
                </Link>
              </Tip>
            </div>
          )}
          {!room && slots.length === 0 && !failed && <p className="px-4 py-6 text-[13px] text-muted">Opening your site…</p>}
          {room && declared && <p className="px-4 pb-1 font-mono text-[12px] text-muted" data-testid="slot-page">{declared}</p>}
          {room && !declared && !room.on_page && !detail.isLoading && (
            <form className="flex items-center gap-2 px-4 pb-1" onSubmit={(e) => { e.preventDefault(); goTry(); }}>
              <input aria-label="Try another page" placeholder="Try another page, e.g. /cart" value={tryDraft} onChange={(e) => setTryDraft(e.target.value)} className="h-8 min-w-0 flex-1 rounded-md border border-line-strong bg-surface px-2.5 font-mono text-[12.5px] text-ink placeholder:font-sans focus:border-ink focus:outline-none" />
              <button type="submit" disabled={!tryDraft.trim().startsWith('/')} className="inline-flex h-8 items-center rounded-md px-3 text-[13px] font-medium text-ink shadow-[0_0_0_1px_var(--line-strong)] transition-colors enabled:hover:bg-surface-2 disabled:opacity-40">Go</button>
            </form>
          )}
          <div
            ref={list}
            role="listbox"
            aria-label={room ? `Designs of ${room.slot}` : 'Designs'}
            data-testid="variant-list"
            className="grid min-h-0 flex-1 auto-rows-min grid-cols-[repeat(auto-fill,minmax(min(100%,264px),1fr))] content-start gap-3 overflow-y-auto overscroll-contain px-4 pt-2 pb-8 [scrollbar-gutter:stable]"
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                e.preventDefault();
                step(e.key === 'ArrowDown' ? 1 : -1);
              }
            }}
          >
            {room &&
              minted &&
              origin &&
              !detail.isLoading &&
              cards.map((c) => {
                const said = words.get(c.key) ?? null;
                const on = c.key === shownKey;
                const A = c.audienceKey ? ICON[c.audienceKey] : Users;
                return (
                  <div
                    key={`${room.slot}:${c.key}`}
                    role="option"
                    aria-selected={on}
                    tabIndex={on || (!shownKey && c.baseline) ? 0 : -1}
                    data-key={c.key}
                    data-testid={`variant-${c.key}`}
                    title={c.why ?? undefined}
                    onClick={() => showCard(c)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        showCard(c);
                      }
                    }}
                    className={clsx(
                      'group cursor-pointer rounded-lg border bg-surface p-1.5 transition-[border-color,box-shadow] duration-150 focus-visible:outline-none',
                      on ? 'border-accent shadow-[0_0_0_1px_var(--accent)]' : 'border-line hover:border-line-strong focus-visible:border-accent',
                      !c.visitor && 'opacity-70',
                    )}
                  >
                    <Thumb
                      src={cardSrc}
                      text={said?.text ?? null}
                      skip={!!said?.bare}
                      origin={origin}
                      slot={room.slot}
                      vkey={c.key}
                      ctx={c.thumbVisitor ? ctxOf(c.thumbVisitor) : null}
                      persona={visitor.persona}
                      vw={visitor.mobile ? 390 : vw}
                      vh={frameH}
                      root={list}
                      blocked={c.status === 'paused' ? 'paused' : !c.thumbVisitor ? 'visitor' : null}
                    />
                    <div className="flex items-center gap-2 px-1.5 pt-2">
                      <span className="min-w-0 truncate text-[13px] font-medium text-ink">{c.baseline && c.key !== 'default_order' ? 'The page’s own' : human(c.key)}</span>
                      <Chip tone={STATUS[c.status].tone}>{STATUS[c.status].label}</Chip>
                    </div>
                    <div className="flex min-h-7 items-center gap-1.5 px-1.5 pb-0.5 text-[12.5px] text-muted">
                      {!c.closed && <A className="size-3.5 shrink-0" aria-hidden />}
                      {c.closed ? (
                        <Tip label={c.why ?? 'This visitor lacks what it needs'}>
                          <span tabIndex={0} className="inline-flex min-w-0 items-center gap-1 truncate" data-testid="falls-through">
                            <CornerDownLeft className="size-3.5 shrink-0" aria-hidden />Falls to the page’s own
                          </span>
                        </Tip>
                      ) : (
                        <span className="truncate">{c.audience ?? 'Every visitor'}</span>
                      )}
                      {c.share != null && !c.closed && <span className="shrink-0 tabular">· up to {Math.round(c.share * 100)}%</span>}
                      {on && !c.baseline && c.id && c.status !== 'paused' && (
                        <span className="ml-auto flex shrink-0 items-center gap-1.5" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                          {confirming && !actor.trim() && (
                            <input autoFocus aria-label="Your name" placeholder="Your name…" value={actor} onChange={(e) => setActor(e.target.value)} className="h-7 w-28 rounded-md border border-line-strong bg-surface px-2 text-[12.5px] text-ink focus:border-ink focus:outline-none" />
                          )}
                          <button
                            type="button"
                            disabled={busy || (confirming && !actor.trim())}
                            className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12.5px] font-medium text-pause-ink transition-colors enabled:hover:bg-pause-soft disabled:cursor-not-allowed disabled:opacity-40"
                            onClick={() => (confirming ? reject(c) : setConfirming(true))}
                          >
                            <X className="size-3.5" aria-hidden />
                            {confirming ? 'Confirm reject' : 'Reject'}
                          </button>
                        </span>
                      )}
                    </div>
                  </div>
                );
              })}
            {onlyOwn && (
              <div data-testid="only-own" className="col-span-full flex flex-col items-center gap-2 rounded-lg border border-dashed border-line-strong px-6 py-8 text-center">
                <span className="grid size-9 place-items-center rounded-full bg-surface-2 text-ink-2"><Layers className="size-4" aria-hidden /></span>
                <p className="text-[13px] font-medium text-ink">Only the page’s own design</p>
                <p className="max-w-60 text-[12.5px] text-muted">Add another for a different visitor or scenario.</p>
                <Link href={drafts} className="mt-1 inline-flex h-8 items-center gap-1.5 rounded-md bg-surface px-3 text-[13px] font-medium text-ink shadow-[0_0_0_1px_var(--line-strong)] transition-colors hover:bg-surface-2">
                  <Plus className="size-4" aria-hidden />Add a design
                </Link>
              </div>
            )}
            {room?.kind === 'picker' && <p className="col-span-full text-[12.5px] text-muted">This slot asks the visitor; there is nothing to flip through.</p>}
          </div>
          {zones || !room ? null : <p className="shrink-0 border-t border-line px-4 py-2 text-[12px] text-muted">This site’s Pratibimba script is older, so slots are not outlined on the page.</p>}
        </aside>
      </div>
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return site ? <Room site={site} /> : <p className="p-8 text-[13px] text-muted">No site given.</p>;
}

export default function PreviewPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
