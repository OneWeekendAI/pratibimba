'use client';
import clsx from 'clsx';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';
import { api, ApiError, type Overview, type SiteRow } from '@/lib/api';
import { ago } from '@/lib/format';

/** The reviewer's name, shared (and kept in step) by the approvals page and the kill switch. */
export function useActor(): [string, (v: string) => void] {
  const [name, setName] = useState('');
  useEffect(() => {
    const read = () => {
      try {
        setName(localStorage.getItem('prb-approver') ?? '');
      } catch {}
    };
    read();
    window.addEventListener('prb-actor', read);
    return () => window.removeEventListener('prb-actor', read);
  }, []);
  return [
    name,
    (v) => {
      setName(v);
      try {
        localStorage.setItem('prb-approver', v);
      } catch {}
      window.dispatchEvent(new Event('prb-actor'));
    },
  ];
}

/**
 * The live banner and its kill switch: one click pauses every slot on the site (each serves
 * only its baseline), one click resumes. The click is recorded under the reviewer's name.
 */
function LiveBanner({ site }: { site: string }) {
  const qc = useQueryClient();
  const [actor, setActor] = useActor();
  const [error, setError] = useState<string | null>(null);
  const [typing, setTyping] = useState(false);
  const o = useQuery({ queryKey: ['overview', site], queryFn: () => api.get<Overview>(`/sites/${site}/overview`) });
  const toggle = useMutation({
    mutationFn: (paused: boolean) => api.post<{ paused: Overview['paused'] }>(`/sites/${site}/pause`, { actor: actor.trim(), paused }),
    onSuccess: (r) => {
      setError(null);
      qc.setQueryData<Overview>(['overview', site], (d) => (d ? { ...d, paused: r.paused } : d));
      qc.invalidateQueries({ queryKey: ['overview', site] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  if (!o.data) return null;
  const paused = o.data.paused;
  return (
    <div
      role="region"
      aria-label="Site status"
      data-testid="live-banner"
      data-state={paused ? 'paused' : 'live'}
      className={clsx('flex flex-wrap items-center gap-x-4 gap-y-2 border-b px-8 py-2 text-[13px]', paused ? 'border-[color:var(--pause)] bg-[color:var(--pause-soft)]' : 'border-line bg-surface')}
    >
      {paused ? (
        <span className="inline-flex items-center gap-2 font-medium text-pause-ink">
          <span aria-hidden>❚❚</span> Paused by {paused.by} {ago(paused.at)}. Every slot is serving its baseline.
        </span>
      ) : (
        <span className="inline-flex items-center gap-2">
          <span aria-hidden className="size-2 rounded-full bg-[color:var(--good)]" />
          <b className="font-semibold text-good-ink">Live</b>
          <span className="text-ink-2">Approved variants are being served on {o.data.origin}.</span>
        </span>
      )}
      <span className="ml-auto inline-flex items-center gap-2">
        {(!actor.trim() || typing) && <input onFocus={() => setTyping(true)} onBlur={() => setTyping(false)} className="h-7 w-36 rounded border border-line-strong bg-surface px-2 text-xs" placeholder="Your name" aria-label="Your name for the kill switch" value={actor} onChange={(e) => setActor(e.target.value)} />}
        <button
          type="button"
          disabled={!actor.trim() || toggle.isPending}
          onClick={() => toggle.mutate(!paused)}
          className={clsx(
            'h-7 rounded-md px-3 text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-45',
            paused ? 'border border-line-strong bg-surface text-ink hover:bg-surface-2' : 'bg-[color:var(--pause)] text-white hover:opacity-90',
          )}
        >
          {paused ? 'Resume all slots' : 'Pause all slots'}
        </button>
      </span>
      {error && <span role="alert" className="w-full text-xs text-pause-ink">{error}</span>}
    </div>
  );
}

function ThemeToggle() {
  const [theme, setTheme] = useState<string | null>(null);
  useEffect(() => setTheme(document.documentElement.dataset.theme ?? null), []);
  const next = theme === 'dark' ? 'light' : 'dark';
  return (
    <button
      className="w-full rounded px-2 py-1.5 text-left text-xs text-chrome-muted hover:bg-chrome-2 hover:text-chrome-ink"
      onClick={() => {
        document.documentElement.dataset.theme = next;
        try {
          localStorage.setItem('prb-theme', next);
        } catch {}
        setTheme(next);
      }}
    >
      {next === 'dark' ? '☾ Dark mode' : '☀ Light mode'}
    </button>
  );
}

/** The chrome around every site page: navigation, the site, and the privacy banner. */
export function Shell({ site, title, actions, children }: { site: string | null; title: ReactNode; actions?: ReactNode; children: ReactNode }) {
  const path = usePathname();
  const sites = useQuery({ queryKey: ['sites'], queryFn: () => api.get<SiteRow[]>('/sites') });
  const current = sites.data?.find((s) => s.id === site);
  const nav = site
    ? [
        { href: `/site/?id=${site}`, label: 'Home', match: ['/site', '/slot', '/decision'] },
        { href: `/results/?site=${site}`, label: 'Results', match: ['/results'] },
        { href: `/approvals/?site=${site}`, label: 'Approvals', match: ['/approvals', '/audit'], dot: (current?.pending ?? 0) > 0 },
        { href: `/install/?site=${site}`, label: 'Install', match: ['/install'] },
      ]
    : [];
  return (
    <div className="flex min-h-screen">
      {/* The column stretches to the page's full height; its contents stay pinned to the viewport. */}
      <aside className="w-56 shrink-0 bg-chrome text-chrome-ink max-md:hidden" data-testid="sidebar">
        <div className="sticky top-0 flex h-screen flex-col px-3 py-4">
        <Link href="/" className="mb-6 flex items-baseline gap-2 px-2">
          <span className="font-serif text-lg font-semibold tracking-tight">Pratibimba</span>
        </Link>
        {sites.data && sites.data.length > 0 && (
          <div className="mb-4 px-2">
            <div className="mb-1 text-[10.5px] tracking-wider text-chrome-muted uppercase">Site</div>
            <div className="truncate font-mono text-[12.5px]">{current?.id ?? site}</div>
            <div className="truncate text-[11px] text-chrome-muted">{current?.origin}</div>
          </div>
        )}
        <nav className="flex flex-col gap-0.5">
          {nav.map((n) => {
            const active = n.match.some((m) => path.startsWith(m));
            return (
              <Link
                key={n.href}
                href={n.href}
                className={clsx('flex items-center justify-between rounded px-2 py-1.5 text-[13px]', active ? 'bg-chrome-2 text-white' : 'text-chrome-ink/80 hover:bg-chrome-2')}
              >
                {n.label}
                {n.dot && <span aria-label="drafts waiting" className="size-2 rounded-full" style={{ background: 'var(--warn)' }} />}
              </Link>
            );
          })}
        </nav>
        <div className="mt-auto space-y-1">
          <ThemeToggle />
          <button
            className="w-full rounded px-2 py-1.5 text-left text-xs text-chrome-muted hover:bg-chrome-2 hover:text-chrome-ink"
            onClick={async () => {
              await api.logout();
              location.assign('/app/login/');
            }}
          >
            Sign out
          </button>
        </div>
        </div>
      </aside>
      <div className="min-w-0 flex-1">
        {site && <LiveBanner site={site} />}
        <div className="border-b border-line bg-surface-2/60 px-8 py-1.5 text-[11.5px] text-ink-2">
          <span className="font-medium">Privacy:</span> session-scoped subject ids unless the install opts into a persistent cookie; no demographic
          fields are accepted. Data stays in your own Postgres{current ? ` (holdout ${Math.round(current.holdout_fraction * 100)}%)` : ''}.
        </div>
        <header className="flex items-end justify-between gap-6 px-8 pt-7 pb-5">
          <div className="min-w-0">{title}</div>
          {actions}
        </header>
        <main className="px-8 pb-16">{children}</main>
      </div>
    </div>
  );
}
