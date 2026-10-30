'use client';
// Primitives on the project's own tokens. Explanations live in Tip (one line) and Help (a few
// lines); both open on hover, on keyboard focus and on a tap, never on hover alone.
import clsx from 'clsx';
import { CircleCheck, CircleDashed, CircleHelp, CirclePause, Diamond, Inbox, LoaderCircle, TriangleAlert, type LucideIcon } from 'lucide-react';
import {
  cloneElement,
  isValidElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type HTMLAttributes,
  type ReactElement,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';

/** Places a floating box under (or over) an anchor, clamped inside the viewport with a 8px gutter. */
function useFloat(anchor: React.RefObject<HTMLElement | null>, open: boolean, width: number) {
  const [pos, setPos] = useState<{ top: number; left: number; above: boolean } | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const place = useCallback(() => {
    const a = anchor.current?.getBoundingClientRect();
    if (!a) return;
    const h = box.current?.offsetHeight ?? 40;
    const w = Math.min(width, window.innerWidth - 16);
    const left = Math.min(Math.max(8, a.left + a.width / 2 - w / 2), window.innerWidth - w - 8);
    const below = a.bottom + 8;
    const above = below + h > window.innerHeight - 8 && a.top - h - 8 > 8;
    setPos({ top: above ? a.top - h - 8 : below, left, above });
  }, [anchor, width]);
  useLayoutEffect(() => {
    if (!open) return;
    place();
    window.addEventListener('scroll', place, true);
    window.addEventListener('resize', place);
    return () => {
      window.removeEventListener('scroll', place, true);
      window.removeEventListener('resize', place);
    };
  }, [open, place]);
  return { pos, box, place };
}

/**
 * A one-line tooltip on any focusable child. Hover, focus and a touch tap all show it; Escape
 * and blur hide it. The child gets aria-describedby so the text is also read aloud.
 */
export function Tip({ label, children, width = 240, className }: { label: ReactNode; children: ReactElement; width?: number; className?: string }) {
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const { pos, box } = useFloat(anchor, open, width);
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  useEffect(() => {
    if (!open) return;
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
    window.addEventListener('keydown', esc);
    return () => window.removeEventListener('keydown', esc);
  }, [open]);
  const child = isValidElement(children) ? cloneElement(children as ReactElement<Record<string, unknown>>, { 'aria-describedby': open ? id : undefined }) : children;
  return (
    <span
      ref={anchor}
      className={className ?? 'inline-flex'}
      onPointerEnter={(e) => e.pointerType === 'mouse' && setOpen(true)}
      onPointerLeave={(e) => e.pointerType === 'mouse' && setOpen(false)}
      onPointerDown={(e) => {
        if (e.pointerType === 'mouse') return;
        setOpen(true);
        if (timer.current) clearTimeout(timer.current);
        timer.current = setTimeout(() => setOpen(false), 2200);
      }}
      onFocus={() => setOpen(true)}
      onBlur={() => setOpen(false)}
    >
      {child}
      {mounted && open &&
        createPortal(
          <div
            ref={box}
            id={id}
            role="tooltip"
            className="pop-in pointer-events-none fixed z-50 w-max rounded-md bg-[color:var(--tip)] px-2 py-1 text-[12px] leading-snug font-medium text-[color:var(--tip-ink)]"
            style={{ top: pos?.top ?? -999, left: pos?.left ?? -999, maxWidth: Math.min(width, typeof window === 'undefined' ? width : window.innerWidth - 16) }}
          >
            {label}
          </div>,
          document.body,
        )}
    </span>
  );
}

/** A "?" that opens a short explanation. Click or tap toggles; Escape or a click outside closes. */
export function Help({ title, children, label, className }: { title?: ReactNode; children: ReactNode; label?: string; className?: string }) {
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const { pos, box } = useFloat(anchor, open, 320);
  useEffect(() => {
    if (!open) return;
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false);
        button.current?.focus();
      }
    };
    const out = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node) && !anchor.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener('keydown', esc);
    window.addEventListener('pointerdown', out);
    return () => {
      window.removeEventListener('keydown', esc);
      window.removeEventListener('pointerdown', out);
    };
  }, [open, box]);
  return (
    <span ref={anchor} className={clsx('inline-flex align-middle', className)}>
      <button
        ref={button}
        type="button"
        aria-label={label ?? 'What is this?'}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen((o) => !o)}
        className={clsx('grid size-6 place-items-center rounded-full text-muted transition-colors hover:text-ink', open && 'text-ink')}
      >
        <CircleHelp className="size-[15px]" aria-hidden />
      </button>
      {open &&
        createPortal(
          <div
            ref={box}
            id={id}
            role="dialog"
            aria-label={typeof title === 'string' ? title : (label ?? 'Help')}
            className="pop-in fixed z-50 rounded-lg bg-surface p-3.5 text-[13px] leading-relaxed text-ink-2 shadow-pop"
            style={{ top: pos?.top ?? -999, left: pos?.left ?? -999, width: Math.min(320, window.innerWidth - 16) }}
          >
            {title && <div className="mb-1 text-[13px] font-semibold text-ink">{title}</div>}
            <div className="space-y-2">{children}</div>
          </div>,
          document.body,
        )}
    </span>
  );
}

type Variant = 'primary' | 'ghost' | 'danger' | 'quiet';

export function Button({ variant = 'primary', icon: Icon, className, children, ...p }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; icon?: LucideIcon }) {
  return (
    <button
      type="button"
      {...p}
      className={clsx(
        'inline-flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-md px-3 text-[13px] font-medium whitespace-nowrap transition-[background-color,border-color,color,box-shadow] duration-150 disabled:cursor-not-allowed disabled:opacity-50 max-sm:h-9',
        variant === 'primary' && 'bg-primary text-primary-ink enabled:hover:bg-primary-hover',
        variant === 'ghost' && 'bg-surface text-ink shadow-[0_0_0_1px_var(--line-strong)] enabled:hover:bg-surface-2',
        variant === 'danger' && 'bg-surface text-pause-ink shadow-[0_0_0_1px_var(--line-strong)] enabled:hover:bg-pause-soft enabled:hover:shadow-[0_0_0_1px_var(--pause)]',
        variant === 'quiet' && 'text-ink-2 enabled:hover:bg-surface-2 enabled:hover:text-ink',
        className,
      )}
    >
      {Icon && <Icon className="size-4 shrink-0" aria-hidden />}
      {children}
    </button>
  );
}

/** An icon-only button: the label is its accessible name and its tooltip. */
export function IconButton({ icon: Icon, label, tip, className, ...p }: ButtonHTMLAttributes<HTMLButtonElement> & { icon: LucideIcon; label: string; tip?: ReactNode }) {
  return (
    <Tip label={tip ?? label}>
      <button
        type="button"
        aria-label={label}
        {...p}
        className={clsx('grid size-8 shrink-0 place-items-center rounded-md text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink disabled:cursor-not-allowed disabled:opacity-40', className)}
      >
        <Icon className="size-4" aria-hidden />
      </button>
    </Tip>
  );
}

/** The one container: a hairline-bordered surface. Never nest one inside another. */
export function Card({ className, ...p }: HTMLAttributes<HTMLDivElement>) {
  return <div {...p} className={clsx('rounded-lg border border-line bg-surface', className)} />;
}

/** A section heading: plain words, an optional "?" and a quiet hint on the right. */
export function SectionTitle({ children, hint, help }: { children: ReactNode; hint?: ReactNode; help?: ReactNode; icon?: LucideIcon }) {
  return (
    <div className="mb-3 flex min-h-6 items-center justify-between gap-4">
      <h2 className="inline-flex items-center gap-1 text-[15px] font-semibold tracking-[-0.01em] text-ink">
        {children}
        {help && <Help title={typeof children === 'string' ? children : undefined}>{help}</Help>}
      </h2>
      {hint && <div className="text-[13px] text-muted">{hint}</div>}
    </div>
  );
}

export type ChipTone = 'live' | 'warn' | 'paused' | 'off' | 'accent';

const TONE: Record<ChipTone, { cls: string; icon: LucideIcon }> = {
  live: { cls: 'bg-good-soft text-good-ink', icon: CircleCheck },
  warn: { cls: 'bg-warn-soft text-warn-ink', icon: TriangleAlert },
  paused: { cls: 'bg-pause-soft text-pause-ink', icon: CirclePause },
  off: { cls: 'bg-surface-2 text-ink-2', icon: CircleDashed },
  accent: { cls: 'bg-accent-soft text-accent', icon: Diamond },
};

/** The status badge: an icon and a label, never colour alone. */
export function Chip({ tone, children, title, icon }: { tone: ChipTone; children: ReactNode; title?: string; icon?: LucideIcon }) {
  const I = icon ?? TONE[tone].icon;
  const chip = (
    <span tabIndex={title ? 0 : undefined} className={clsx('inline-flex h-[22px] items-center gap-1 rounded-full px-2 text-[12px] font-medium whitespace-nowrap', TONE[tone].cls)}>
      <I className="size-3 shrink-0" strokeWidth={2.25} aria-hidden />
      {children}
    </span>
  );
  return title ? <Tip label={title}>{chip}</Tip> : chip;
}

export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <code className={clsx('font-mono text-[12px]', className)}>{children}</code>;
}

export function Empty({ children, icon: Icon = Inbox }: { children: ReactNode; icon?: LucideIcon }) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line-strong px-6 py-12 text-center text-[13px] text-muted">
      <span className="grid size-9 place-items-center rounded-full bg-surface-2 text-ink-2"><Icon className="size-4" aria-hidden /></span>
      <div className="max-w-sm">{children}</div>
    </div>
  );
}

export function Loading() {
  return (
    <div role="status" className="flex items-center gap-2 px-1 py-10 text-[13px] text-muted">
      <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" aria-hidden /> Loading…
    </div>
  );
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[13px] font-medium text-ink-2">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-xs text-muted">{hint}</span>}
    </label>
  );
}

export const inputClass =
  'h-8 rounded-md border border-line-strong bg-surface px-2.5 text-[13px] text-ink transition-[border-color,box-shadow] duration-150 placeholder:text-muted hover:border-[color:var(--muted)] focus:border-ink focus:shadow-[0_0_0_3px_color-mix(in_srgb,var(--accent)_22%,transparent)] focus:outline-none focus-visible:outline-none disabled:opacity-50 max-sm:h-9 max-sm:text-[16px]';

/** A system-written sentence: `backticked` names render as code, the rest as prose. */
export function Prose({ text }: { text: string }) {
  return <>{text.split('`').map((part, i) => (i % 2 ? <code key={i} className="rounded bg-surface-2 px-1 py-px font-mono text-[0.86em]">{part}</code> : part))}</>;
}
