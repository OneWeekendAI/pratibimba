'use client';
// shadcn-style primitives, written against the project's own tokens.
import clsx from 'clsx';
import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from 'react';

export function Button({ variant = 'primary', className, ...p }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary' | 'ghost' | 'danger' }) {
  return (
    <button
      {...p}
      className={clsx(
        'inline-flex h-9 items-center justify-center gap-2 rounded-md px-3.5 text-[13px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-45',
        variant === 'primary' && 'bg-accent text-accent-ink hover:opacity-90',
        variant === 'ghost' && 'border border-line-strong bg-surface text-ink hover:bg-surface-2',
        variant === 'danger' && 'border border-line-strong bg-surface text-pause-ink hover:bg-surface-2',
        className,
      )}
    />
  );
}

export function Card({ className, ...p }: HTMLAttributes<HTMLDivElement>) {
  return <div {...p} className={clsx('rounded-lg border border-line bg-surface shadow-[0_1px_0_rgba(0,0,0,0.03)]', className)} />;
}

export function SectionTitle({ children, hint }: { children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="mb-3 flex items-baseline justify-between gap-4">
      <h2 className="text-[13px] font-semibold tracking-wide text-ink-2 uppercase">{children}</h2>
      {hint && <div className="text-xs text-muted">{hint}</div>}
    </div>
  );
}

export type ChipTone = 'live' | 'warn' | 'paused' | 'off' | 'accent';

const TONE: Record<ChipTone, { dot: string; text: string; icon: string }> = {
  live: { dot: 'var(--good)', text: 'text-good-ink', icon: '●' },
  warn: { dot: 'var(--warn)', text: 'text-warn-ink', icon: '▲' },
  paused: { dot: 'var(--pause)', text: 'text-pause-ink', icon: '❚❚' },
  off: { dot: 'var(--muted)', text: 'text-muted', icon: '○' },
  accent: { dot: 'var(--accent)', text: 'text-accent', icon: '◆' },
};

/** The status chip: a small circle and a label, never colour alone. */
export function Chip({ tone, children, title }: { tone: ChipTone; children: ReactNode; title?: string }) {
  return (
    <span title={title} className={clsx('inline-flex items-center gap-1.5 rounded-full border border-line bg-surface px-2 py-0.5 text-[11.5px] font-medium', TONE[tone].text)}>
      <span aria-hidden className="inline-block size-2 rounded-full" style={{ background: TONE[tone].dot }} />
      {children}
    </span>
  );
}

export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <code className={clsx('font-mono text-[12px]', className)}>{children}</code>;
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="rounded-lg border border-dashed border-line-strong px-6 py-10 text-center text-[13px] text-muted">{children}</div>;
}

export function Loading() {
  return <div className="px-2 py-8 text-[13px] text-muted">Loading…</div>;
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs font-medium text-ink-2">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-xs text-muted">{hint}</span>}
    </label>
  );
}

export const inputClass =
  'h-9 rounded-md border border-line-strong bg-surface px-2.5 text-[13px] text-ink placeholder:text-muted focus:outline-2 focus:outline-offset-1 focus:outline-accent';

/** A system-written sentence: `backticked` names render as code, the rest as prose. */
export function Prose({ text }: { text: string }) {
  return <>{text.split('`').map((part, i) => (i % 2 ? <code key={i} className="rounded bg-surface-2 px-1 font-mono text-[0.82em]">{part}</code> : part))}</>;
}
