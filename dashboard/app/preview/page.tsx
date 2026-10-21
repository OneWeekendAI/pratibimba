'use client';
// The Preview room: the real site fills the screen; one slim bar of plain buttons on top.
// The site runs in preview mode (a signed, short-lived token): it decides nothing, logs nothing
// and learns nothing; it shows every version a visitor could get, and this page picks which.
import clsx from 'clsx';
import Link from 'next/link';
import { Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { useActor } from '@/components/shell';
import { api, ApiError } from '@/lib/api';
import { useParam } from '@/lib/page';
import { ctxOf, FIRST_VISIT, TOGGLES, type RoomRect, type RoomSlot, type Visitor } from '@/lib/preview';

interface Minted {
  url: string;
  page: string;
  token: string;
  expires_in: number;
}

function chip(on: boolean) {
  return clsx(
    'h-7 rounded-full px-3 text-[12.5px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent',
    on ? 'bg-accent text-accent-ink' : 'text-ink-2 hover:bg-surface-2 hover:text-ink',
  );
}

function Room({ site }: { site: string }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const [minted, setMinted] = useState<Minted | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [visitor, setVisitor] = useState<Visitor>(FIRST_VISIT);
  const [slots, setSlots] = useState<RoomSlot[]>([]);
  const [picked, setPicked] = useState<string | null>(null);
  const [rect, setRect] = useState<RoomRect | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [actor, setActor] = useActor();
  const visitorRef = useRef(visitor);
  visitorRef.current = visitor;
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
  const show = useCallback((v: Visitor, quiet: boolean) => tell({ type: 'prb:visitor', ctx: ctxOf(v), quiet }), [tell]);

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
      if (m.type === 'prb:hello') show(visitorRef.current, true);
      else if (m.type === 'prb:expired') mint();
      else if (m.type === 'prb:state') {
        setSlots(Array.isArray(m.slots) ? m.slots : []);
        setPicked(typeof m.picked === 'string' ? m.picked : null);
        setRect(m.rect ?? null);
      }
    };
    window.addEventListener('message', on);
    return () => window.removeEventListener('message', on);
  }, [origin, show, mint]);

  useEffect(() => setConfirming(false), [picked]);

  const choose = (v: Visitor) => {
    setVisitor(v);
    setNote(null);
    tell({ type: 'prb:close' });
    show(v, false);
  };
  const toggle = (k: keyof Visitor) => choose({ ...visitor, [k]: !visitor[k] });
  const first = !Object.values(visitor).some(Boolean);
  const changed = slots.filter((s) => s.changed && s.on_page).length;
  const open = slots.find((s) => s.slot === picked && s.versions.length > 1) ?? null;
  const v = open ? open.versions[open.index] : null;

  const reject = async () => {
    if (!v?.id || v.baseline) return;
    setBusy(true);
    try {
      if (v.status === 'draft') await api.post(`/variants/${v.id}/reject`, { actor: actor.trim(), comment: 'Rejected in preview' });
      else await api.post(`/variants/${v.id}/retire`, { actor: actor.trim(), comment: 'Rejected in preview' });
      setNote(`${v.key} is off the site.`);
      setConfirming(false);
      show(visitor, true);
    } catch (e) {
      setNote(e instanceof ApiError ? e.message : 'That did not work.');
    } finally {
      setBusy(false);
    }
  };

  // The popover sits under the slot, inside the stage, clamped to it.
  const W = 300;
  const stageBox = stage.current?.getBoundingClientRect();
  const frameBox = frame.current?.getBoundingClientRect();
  let pop: { top: number; left: number } | null = null;
  if (open && rect && stageBox && frameBox) {
    const dx = frameBox.left - stageBox.left;
    const H = 120;
    // Under the slot when it fits; else inside the slot's visible top, so it never covers the page's own nav.
    const top = rect.bottom + 12 + H < stageBox.height ? rect.bottom + 12 : Math.max(rect.top, 0) + 16;
    pop = { top: Math.min(Math.max(8, top), stageBox.height - H), left: Math.min(Math.max(8, dx + rect.left + 16), stageBox.width - W - 8) };
  }

  return (
    <div className="flex h-screen flex-col bg-paper">
      <div role="toolbar" aria-label="Preview" className="flex h-12 shrink-0 items-center gap-1 border-b border-line bg-surface px-3">
        <Link href={`/site/?id=${site}`} className="mr-3 rounded px-2 py-1 font-serif text-[15px] font-semibold tracking-tight text-ink hover:bg-surface-2" title="Back to the dashboard">
          Pratibimba
        </Link>
        <button type="button" aria-pressed={first} className={chip(first)} onClick={() => choose(FIRST_VISIT)}>
          First visit
        </button>
        {TOGGLES.map((t) => (
          <button key={t.key} type="button" aria-pressed={visitor[t.key]} className={chip(visitor[t.key])} onClick={() => toggle(t.key)}>
            {t.label}
          </button>
        ))}
        <span aria-hidden className="mx-2 h-5 w-px bg-line" />
        <button type="button" className="h-7 rounded-md border border-line-strong px-3 text-[12.5px] font-medium text-ink hover:bg-surface-2" onClick={() => choose({ ...visitor, returning: true })}>
          Next visit
        </button>
        <span className="ml-auto pr-1 text-[12.5px] text-ink-2" role="status" data-testid="preview-status">
          {note ?? (changed ? `${changed} ${changed === 1 ? 'change' : 'changes'} from the last view` : '')}
        </span>
      </div>
      <div ref={stage} className="relative min-h-0 flex-1 overflow-hidden">
        {failed && <p className="p-8 text-[13px] text-pause-ink">{failed}</p>}
        {minted && (
          <iframe
            ref={frame}
            title="Your site, in preview"
            src={minted.url}
            className={clsx('h-full border-0 bg-white', visitor.mobile ? 'mx-auto block w-[390px] border-x border-line shadow-sm' : 'w-full')}
          />
        )}
        {open && v && pop && (
          <div role="dialog" aria-label={`${open.slot} versions`} data-testid="slot-popover" className="absolute z-10 rounded-lg border border-line bg-surface p-3 shadow-lg" style={{ top: pop.top, left: pop.left, width: W }}>
            <div className="text-[13px] font-medium text-ink">
              Version {open.index + 1} of {open.versions.length}
              {v.status === 'draft' && <span className="ml-1.5 font-normal text-warn-ink">· draft</span>}
            </div>
            <div className="mt-0.5 text-[12.5px] text-ink-2">why: {v.why}</div>
            {confirming && !actor.trim() && (
              <input autoFocus aria-label="Your name" placeholder="Your name" value={actor} onChange={(e) => setActor(e.target.value)} className="mt-2 h-7 w-full rounded border border-line-strong bg-surface px-2 text-xs" />
            )}
            <div className="mt-2.5 flex items-center gap-1.5">
              <button type="button" className="h-7 rounded-md border border-line-strong px-2.5 text-[12.5px] hover:bg-surface-2" onClick={() => tell({ type: 'prb:pick', slot: open.slot, index: open.index - 1 })}>
                Prev
              </button>
              <button type="button" className="h-7 rounded-md border border-line-strong px-2.5 text-[12.5px] hover:bg-surface-2" onClick={() => tell({ type: 'prb:pick', slot: open.slot, index: open.index + 1 })}>
                Next
              </button>
              <button
                type="button"
                disabled={v.baseline || busy || (confirming && !actor.trim())}
                title={v.baseline ? 'The page’s own version stays' : undefined}
                className="ml-auto h-7 rounded-md px-2.5 text-[12.5px] font-medium text-pause-ink hover:bg-[color:var(--pause-soft)] disabled:cursor-not-allowed disabled:opacity-40"
                onClick={() => (confirming ? reject() : setConfirming(true))}
              >
                {confirming ? 'Confirm reject' : 'Reject'}
              </button>
            </div>
          </div>
        )}
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
