'use client';
// The Preview room's explorer parts: a live thumbnail of one design, and the draggable split.
import clsx from 'clsx';
import { CirclePause, EyeOff } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import type { RoomRect } from '@/lib/preview';

const MAX_H = 200;
/** Page pixels kept around the slot, so a design does not touch the card's edge. */
const PAD = 12;
/** A thumbnail is never drawn larger than this share of the page's own size. */
const MAX_SCALE = 0.8;

/**
 * One design of one slot, rendered by the real page: a hidden-from-input frame of the site in
 * preview mode, told to show `vkey` in `slot` with no marks, then cropped and scaled to that
 * slot's box. It loads when it scrolls near, and follows the room's page width without reloading.
 */
export function Thumb({
  src,
  origin,
  slot,
  vkey,
  ctx,
  persona,
  vw,
  vh,
  root,
  blocked,
}: {
  src: string;
  origin: string;
  slot: string;
  vkey: string;
  /** The decide context of a visitor who gets this design; null when none can. */
  ctx: object | null;
  /** The preview persona whose history fills templates (P1). */
  persona?: string;
  /** The width the page renders at (the room's own page width), and the frame's least height. */
  vw: number;
  vh: number;
  root: RefObject<HTMLElement | null>;
  /** Why there is no picture, when there is none to draw. */
  blocked?: 'paused' | 'visitor' | null;
}) {
  const box = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const [near, setNear] = useState(false);
  const [cw, setCw] = useState(0);
  const [rect, setRect] = useState<RoomRect | null>(null);
  const [fh, setFh] = useState(vh);
  const [gone, setGone] = useState(false);
  const fhRef = useRef(fh);
  fhRef.current = fh;
  const ctxKey = JSON.stringify([ctx, persona ?? null]);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setCw(el.clientWidth));
    ro.observe(el);
    setCw(el.clientWidth);
    if (typeof IntersectionObserver === 'undefined') {
      setNear(true);
      return () => ro.disconnect();
    }
    const io = new IntersectionObserver(
      (es) => {
        if (es.some((e) => e.isIntersecting)) {
          setNear(true);
          io.disconnect();
        }
      },
      { root: root.current, rootMargin: '480px 0px' },
    );
    io.observe(el);
    return () => {
      ro.disconnect();
      io.disconnect();
    };
  }, [root]);

  useEffect(() => {
    if (!near || !ctx) return;
    setRect(null);
    setGone(false);
    const post = (m: object) => frame.current?.contentWindow?.postMessage(m, origin);
    const timers: ReturnType<typeof setTimeout>[] = [];
    let picked = false;
    const on = (e: MessageEvent) => {
      if (e.origin !== origin || !frame.current || e.source !== frame.current.contentWindow) return;
      const m = e.data ?? {};
      if (m.type === 'prb:hello') {
        post({ type: 'prb:view', marks: false });
        post({ type: 'prb:visitor', ctx, persona, quiet: true });
      } else if (m.type === 'prb:state') {
        const s = Array.isArray(m.slots) ? m.slots.find((x: { slot: string }) => x.slot === slot) : null;
        if (!s) return;
        const i = s.versions.findIndex((v: { key: string }) => v.key === vkey);
        if (i < 0) return setGone(true);
        if (m.picked !== slot || s.index !== i) {
          post({ type: 'prb:pick', slot, index: i });
          // Images and fonts move the box after the first render: ask again a few times.
          if (!picked) for (const ms of [500, 1400, 3000]) timers.push(setTimeout(() => post({ type: 'prb:pick', slot, index: i }), ms));
          picked = true;
          return;
        }
        const r: RoomRect | null = m.rect;
        if (!r || r.width <= 0 || r.height <= 0) return;
        const top = Math.max(0, r.top - PAD);
        const left = Math.max(0, r.left - PAD);
        setRect({ top, left, width: r.width + (r.left - left) + PAD, height: r.height + (r.top - top) + PAD, bottom: r.bottom + PAD });
        // The frame is as tall as the page down to the slot, so the slot is never scrolled away.
        if (r.bottom > fhRef.current - 8 && fhRef.current < 6000) setFh(Math.min(6000, Math.ceil(r.bottom + 240)));
      }
    };
    window.addEventListener('message', on);
    // A frame already loaded said hello before this visitor: tell it again (a load not yet done drops this).
    try {
      post({ type: 'prb:visitor', ctx, persona, quiet: true });
    } catch {
      /* not loaded yet */
    }
    return () => {
      window.removeEventListener('message', on);
      timers.forEach(clearTimeout);
    };
    // ctxKey stands for ctx.
  }, [near, ctxKey, origin, slot, vkey]);

  useEffect(() => setFh((h) => Math.max(h, vh)), [vh]);

  const s = rect && cw ? Math.min(MAX_SCALE, cw / rect.width) : 0;
  const h = rect && s ? Math.max(56, Math.min(MAX_H, Math.round(rect.height * s))) : 128;
  const dx = rect && s ? Math.max(0, (cw - rect.width * s) / 2) : 0;
  const why = blocked ?? (gone ? 'visitor' : null);

  return (
    <div ref={box} className="relative overflow-hidden rounded-md bg-surface-2" style={{ height: why ? 96 : h }} data-ready={rect && !why ? '' : undefined}>
      {why ? (
        <Placeholder icon={why === 'paused' ? CirclePause : EyeOff}>{why === 'paused' ? 'Paused, so the page cannot show it' : 'No preview visitor gets this one'}</Placeholder>
      ) : (
        <>
          {near && ctx && (
            <iframe
              ref={frame}
              src={src}
              title={`${vkey}, small`}
              tabIndex={-1}
              aria-hidden
              className={clsx('pointer-events-none absolute top-0 left-0 origin-top-left border-0 bg-white transition-opacity duration-200', rect ? 'opacity-100' : 'opacity-0')}
              style={{ width: vw, height: fh, transform: rect ? `translate(${dx - rect.left * s}px, ${-rect.top * s}px) scale(${s})` : undefined }}
            />
          )}
          {!rect && <div className="absolute inset-0 animate-pulse bg-surface-2 motion-reduce:animate-none" aria-hidden />}
          {rect && rect.height * s > MAX_H && <div aria-hidden className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-gradient-to-t from-black/[0.06] to-transparent" />}
        </>
      )}
    </div>
  );
}

function Placeholder({ icon: I, children }: { icon: typeof EyeOff; children: ReactNode }) {
  return (
    <div className="absolute inset-0 flex items-center justify-center gap-2 px-3 text-center text-[12px] text-muted">
      <I className="size-4 shrink-0" aria-hidden />
      {children}
    </div>
  );
}

const SPLIT_KEY = 'prb.preview.split';

/** The left pane's share of the room, in percent: 50 by default, remembered per browser. */
export function useSplit() {
  const [pct, setPct] = useState(50);
  useEffect(() => {
    try {
      const v = Number(window.localStorage.getItem(SPLIT_KEY));
      if (v >= 10 && v <= 90) setPct(v);
    } catch {
      /* storage blocked: the default */
    }
  }, []);
  const save = useCallback((v: number) => {
    setPct(v);
    try {
      window.localStorage.setItem(SPLIT_KEY, v.toFixed(2));
    } catch {
      /* storage blocked: this visit only */
    }
  }, []);
  return [pct, save] as const;
}

/**
 * The divider between the panes: drag it, use the arrow keys, or double-click to put it back at
 * half. While dragging, a sheet covers both panes so the page's frame does not eat the pointer.
 */
export function Divider({ container, pct, onChange, minLeft = 360, minRight = 340 }: { container: RefObject<HTMLElement | null>; pct: number; onChange: (v: number) => void; minLeft?: number; minRight?: number }) {
  const [drag, setDrag] = useState(false);
  const lastDown = useRef(-Infinity);
  const clamp = useCallback(
    (v: number) => {
      const w = container.current?.clientWidth ?? 0;
      if (!w) return v;
      const lo = (minLeft / w) * 100;
      const hi = 100 - (minRight / w) * 100;
      return lo > hi ? 50 : Math.min(hi, Math.max(lo, v));
    },
    [container, minLeft, minRight],
  );
  useEffect(() => {
    if (!drag) return;
    const move = (e: PointerEvent) => {
      const b = container.current?.getBoundingClientRect();
      if (b && b.width) onChange(clamp(((e.clientX - b.left) / b.width) * 100));
    };
    const up = () => setDrag(false);
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', up);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', up);
    };
  }, [drag, container, clamp, onChange]);
  // A narrower window can leave a remembered split below a pane's least width.
  useEffect(() => {
    const fix = () => {
      const c = clamp(pct);
      if (Math.abs(c - pct) > 0.1) onChange(c);
    };
    fix();
    window.addEventListener('resize', fix);
    return () => window.removeEventListener('resize', fix);
  }, [pct, clamp, onChange]);
  return (
    <>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the panes"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct)}
        tabIndex={0}
        title="Drag to resize; double-click for half and half"
        data-testid="split-divider"
        onPointerDown={(e) => {
          e.preventDefault();
          // A double press resets; detected here, since the drag sheet takes the second click.
          const now = e.timeStamp;
          if (now - lastDown.current < 350) {
            lastDown.current = -Infinity;
            onChange(clamp(50));
            return;
          }
          lastDown.current = now;
          setDrag(true);
        }}
        onKeyDown={(e) => {
          if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
            e.preventDefault();
            onChange(clamp(pct + (e.key === 'ArrowLeft' ? -2 : 2)));
          } else if (e.key === 'Enter') onChange(clamp(50));
        }}
        className="group relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize touch-none max-md:hidden focus-visible:outline-none"
      >
        <span aria-hidden className={clsx('absolute inset-y-0 left-1/2 w-px -translate-x-1/2 transition-colors duration-150', drag ? 'bg-accent' : 'bg-line group-hover:bg-line-strong group-focus-visible:bg-accent')} />
        <span aria-hidden className={clsx('absolute top-1/2 left-1/2 h-8 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full transition-colors duration-150', drag ? 'bg-accent' : 'bg-line-strong opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100')} />
      </div>
      {drag && <div aria-hidden className="fixed inset-0 z-50 cursor-col-resize" />}
    </>
  );
}
