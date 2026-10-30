'use client';
import clsx from 'clsx';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowRight, ChevronRight, LogOut, Moon, Pause, Play, Sun, TriangleAlert, UserRound } from 'lucide-react';
import { useEffect, useState, ViewTransition, type ReactNode } from 'react';
import { Help, IconButton, Tip } from '@/components/ui';
import { api, ApiError, type Overview, type SiteRow } from '@/lib/api';
import { ago } from '@/lib/format';
import { AUDIENCE, locate, nextStage, STAGES } from '@/lib/flow';

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

/** The mark: a half-disc standing on a line, and its fainter reflection beneath. */
export function Mark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={className} aria-hidden fill="currentColor">
      <rect x="9" y="2" width="14" height="12" rx="3.5" />
      <path d="M5 16h22" stroke="currentColor" strokeWidth="1" strokeLinecap="round" opacity="0.3" />
      <circle cx="16" cy="24" r="6" opacity="0.5" />
    </svg>
  );
}

/**
 * Who is acting: the one name field. Approvals, rejections, the kill switch and share changes
 * are all recorded under it, so it lives in the header, once.
 */
function ActorField() {
  const [actor, setActor] = useActor();
  return (
    <Tip label="Recorded with every approval, rejection and pause">
      <label className="group relative flex h-8 items-center">
        <UserRound className="pointer-events-none absolute left-2 size-3.5 text-muted group-focus-within:text-ink" aria-hidden />
        <input
          className="h-8 w-[7.5rem] rounded-md border border-transparent bg-transparent pr-2 pl-7 text-[13px] text-ink transition-[border-color,background-color] duration-150 placeholder:text-muted hover:bg-surface-2 focus:border-line-strong focus:bg-surface focus:outline-none sm:w-36 max-sm:text-[16px]"
          placeholder="Your name…"
          aria-label="Your name"
          autoComplete="name"
          spellCheck={false}
          value={actor}
          onChange={(e) => setActor(e.target.value)}
        />
      </label>
    </Tip>
  );
}

/**
 * The site's state and its kill switch: one click pauses every slot on the site (each serves
 * only its baseline), one click resumes. The click is recorded under the name in the header.
 */
function LiveStatus({ site }: { site: string }) {
  const qc = useQueryClient();
  const [actor] = useActor();
  const [error, setError] = useState<string | null>(null);
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
    <div role="region" aria-label="Site status" data-testid="live-banner" data-state={paused ? 'paused' : 'live'} className="flex min-w-0 items-center gap-3 text-[13px]">
      {paused ? (
        <span className="inline-flex min-w-0 items-center gap-1.5 font-medium text-pause-ink">
          <Pause className="size-3.5 shrink-0" aria-hidden />
          <span className="truncate">Paused by {paused.by} {ago(paused.at)}<span className="max-sm:hidden">. Baselines only</span></span>
        </span>
      ) : (
        <Tip label={`Serving ${o.data.origin}`}>
          <span tabIndex={0} className="inline-flex min-w-0 items-center gap-2 rounded-sm">
            <span aria-hidden className="relative grid size-2 place-items-center">
              <span className="absolute size-2 animate-ping rounded-full bg-good opacity-50 [animation-duration:2s] motion-reduce:hidden" />
              <span className="size-2 rounded-full bg-good" />
            </span>
            <span className="font-medium text-ink">Live</span>
            <span className="truncate text-muted max-md:hidden">{o.data.origin.replace(/^https?:\/\//, '')}</span>
          </span>
        </Tip>
      )}
      <Tip label={!actor.trim() ? 'Add your name at the top first' : paused ? 'Serve approved variants again' : 'Every slot serves only its baseline until you resume'}>
        <button
          type="button"
          aria-label={paused ? 'Resume all slots' : 'Pause all slots'}
          disabled={!actor.trim() || toggle.isPending}
          onClick={() => toggle.mutate(!paused)}
          className={clsx(
            'inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-[12.5px] font-medium transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-50',
            paused ? 'bg-primary text-primary-ink enabled:hover:bg-primary-hover' : 'text-ink-2 shadow-[0_0_0_1px_var(--line-strong)] enabled:hover:bg-pause-soft enabled:hover:text-pause-ink enabled:hover:shadow-[0_0_0_1px_var(--pause)]',
          )}
        >
          {paused ? <Play className="size-3.5" aria-hidden /> : <Pause className="size-3.5" aria-hidden />}
          {paused ? 'Resume' : 'Pause'}
        </button>
      </Tip>
      {error && <span role="alert" className="text-xs text-pause-ink">{error}</span>}
    </div>
  );
}

/** A plain warning for each installed de-anonymiser plugin; nothing when there is none. */
function WarningBanner({ site }: { site: string }) {
  const o = useQuery({ queryKey: ['overview', site], queryFn: () => api.get<Overview>(`/sites/${site}/overview`) });
  const warnings = o.data?.warnings ?? [];
  if (!warnings.length) return null;
  return (
    <div role="alert" data-testid="warning-banner" className="border-b border-line bg-warn-soft">
      <div className="mx-auto flex max-w-6xl gap-2 px-4 py-2.5 text-[13px] text-warn-ink sm:px-6">
        <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
        <div>{warnings.map((w) => <p key={w}>{w}</p>)}</div>
      </div>
    </div>
  );
}

function ThemeToggle() {
  const [theme, setTheme] = useState<string | null>(null);
  useEffect(() => setTheme(document.documentElement.dataset.theme ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')), []);
  const next = theme === 'dark' ? 'light' : 'dark';
  return (
    <IconButton
      icon={next === 'dark' ? Moon : Sun}
      label={next === 'dark' ? 'Dark mode' : 'Light mode'}
      onClick={() => {
        document.documentElement.dataset.theme = next;
        try {
          localStorage.setItem('prb-theme', next);
        } catch {}
        setTheme(next);
      }}
    />
  );
}

/** The five stages as one line, the current one filled. */
function StageRail({ site, current, pending }: { site: string; current: string | null; pending: number }) {
  return (
    <nav aria-label="Stages" className="w-full md:w-auto">
      <ol className="grid grid-cols-5 items-center gap-1 md:flex md:gap-0">
        {STAGES.map((s, i) => {
          const on = s.id === current;
          const I = s.icon;
          return (
            <li key={s.id} className="flex items-center">
              {i > 0 && <span aria-hidden className="hidden h-px w-4 bg-line-strong md:block lg:w-6" />}
              <Tip label={s.tip} className="flex w-full md:w-auto">
                <Link
                  href={s.pages[0].href(site)}
                  aria-current={on ? 'step' : undefined}
                  className={clsx(
                    'relative flex w-full flex-col items-center gap-1 rounded-md px-1 py-1.5 text-[11.5px] font-medium transition-colors duration-150 md:h-8 md:flex-row md:gap-1.5 md:rounded-full md:px-3 md:py-0 md:text-[13px]',
                    on ? 'bg-primary text-primary-ink' : 'text-ink-2 hover:bg-surface-2 hover:text-ink',
                  )}
                >
                  <I className="size-4 shrink-0 md:size-3.5" aria-hidden />
                  {s.label}
                  {s.id === 'approve' && pending > 0 && (
                    <span aria-label={`${pending} drafts waiting`} className={clsx('absolute top-1 right-1.5 size-1.5 rounded-full bg-warn md:static', on && 'ring-2 ring-[color:var(--primary)]')} />
                  )}
                </Link>
              </Tip>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

/** The chrome around every page: brand and site, the stages, the stage's pages and the live switch. */
export function Shell({ site, title, crumbs, help, mono, actions, children }: { site: string | null; title: string; crumbs?: ReactNode; help?: ReactNode; mono?: boolean; actions?: ReactNode; children: ReactNode }) {
  const path = usePathname();
  const sites = useQuery({ queryKey: ['sites'], queryFn: () => api.get<SiteRow[]>('/sites') });
  const current = sites.data?.find((s) => s.id === site);
  const here = locate(path);
  const next = here ? nextStage(here.stage.id) : null;
  return (
    <div className="flex min-h-screen flex-col">
      <a href="#main" className="sr-only z-50 rounded-md bg-primary px-3 py-2 text-[13px] font-medium text-primary-ink focus:not-sr-only focus:fixed focus:top-2 focus:left-2">Skip to content</a>
      <header className="sticky top-0 z-30 border-b border-line bg-surface/85 backdrop-blur-md backdrop-saturate-150" style={{ viewTransitionName: 'prb-header' }} data-testid="sidebar">
        <div className="mx-auto flex max-w-6xl flex-wrap items-center gap-x-3 gap-y-2 px-4 pt-2.5 pb-2 sm:px-6 md:grid md:grid-cols-[1fr_auto_1fr] md:py-2.5">
          <div className="flex min-w-0 items-center gap-2">
            <Link href="/" className="flex shrink-0 items-center gap-2 rounded-md text-ink" aria-label="Pratibimba, all sites">
              <Mark className="size-[22px]" />
              <span className="text-[15px] font-semibold tracking-[-0.02em] max-lg:hidden">Pratibimba</span>
            </Link>
            {site && (
              <>
                <span aria-hidden className="text-[18px] font-light text-line-strong">/</span>
                <Tip label={current?.origin ?? 'All sites'}>
                  <Link href="/" className="min-w-0 truncate rounded-md px-1.5 py-1 font-mono text-[12.5px] text-ink transition-colors hover:bg-surface-2">{site}</Link>
                </Tip>
              </>
            )}
          </div>
          <div className="order-last w-full md:order-none md:w-auto">{site && <StageRail site={site} current={here?.stage.id ?? null} pending={current?.pending ?? 0} />}</div>
          <div className="ml-auto flex items-center gap-0.5 md:justify-self-end">
            <ActorField />
            <span aria-hidden className="mx-1 h-4 w-px bg-line max-sm:hidden" />
            <Help label="Privacy" title="Privacy">
              <p>Visitors get session-scoped ids unless the install opts into a persistent cookie. No demographic fields are accepted.</p>
              <p>Data stays in your own Postgres{current ? `. Holdout ${Math.round(current.holdout_fraction * 100)}%.` : '.'}</p>
            </Help>
            <ThemeToggle />
            <IconButton
              icon={LogOut}
              label="Sign out"
              onClick={async () => {
                await api.logout();
                location.assign('/app/login/');
              }}
            />
          </div>
        </div>
        {site && (
          <div className="mx-auto flex h-11 max-w-6xl items-stretch justify-between gap-4 px-4 sm:px-6">
            {here ? (
              <nav aria-label={`${here.stage.label} pages`} className="-mb-px flex min-w-0 items-stretch gap-1 overflow-x-auto [scrollbar-width:none]">
                {here.stage.pages.map((p) => {
                  const on = p.id === here.page.id;
                  const A = AUDIENCE[p.audience];
                  return (
                    <Tip key={p.id} className="flex" label={<>{p.tip}. <span className="opacity-60">{A.label}.</span></>}>
                      <Link
                        href={p.href(site)}
                        aria-current={on ? 'page' : undefined}
                        className={clsx(
                          'relative inline-flex shrink-0 items-center gap-1.5 px-2 text-[13px] transition-colors duration-150 after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:rounded-full',
                          on ? 'font-medium text-ink after:bg-ink' : 'text-muted hover:text-ink',
                        )}
                      >
                        <span className="rounded-md px-1 py-1">{p.label}</span>
                      </Link>
                    </Tip>
                  );
                })}
              </nav>
            ) : (
              <span />
            )}
            <div className="flex shrink-0 items-center"><LiveStatus site={site} /></div>
          </div>
        )}
      </header>
      {site && <WarningBanner site={site} />}
      {/* Moving between pages: the header stays put, the page fades through in ~150 ms. */}
      <ViewTransition enter="prb-in" exit="prb-out" default="none">
      <div className="mx-auto w-full max-w-6xl flex-1 px-4 sm:px-6">
        <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-3 pt-8 pb-6 sm:pt-10 sm:pb-8">
          <div className="min-w-0">
            {crumbs && <div className="mb-2 flex items-center gap-1 text-[13px] text-muted">{crumbs}</div>}
            <div className="flex min-w-0 items-center gap-1.5">
              <h1 className={clsx('min-w-0 truncate text-ink', mono ? 'font-mono text-[22px] font-medium tracking-[-0.02em] sm:text-[26px]' : 'text-[24px] font-semibold tracking-[-0.03em] sm:text-[28px]')}>{title}</h1>
              {help && <Help title={title}>{help}</Help>}
            </div>
          </div>
          {actions}
        </div>
        <main id="main" tabIndex={-1} className="pb-16 focus:outline-none">{children}</main>
      </div>
      </ViewTransition>
      {site && next && (
        <footer className="border-t border-line">
          <div className="mx-auto flex max-w-6xl justify-end px-4 py-5 sm:px-6">
            <Link href={next.pages[0].href(site)} className="group inline-flex items-center gap-2 rounded-md px-2 py-1.5 text-[13px] text-muted transition-colors hover:text-ink">
              Next
              <span className="inline-flex items-center gap-1.5 font-medium text-ink"><next.icon className="size-3.5" aria-hidden />{next.label}</span>
              <ArrowRight className="size-3.5 transition-transform duration-150 group-hover:translate-x-0.5" aria-hidden />
            </Link>
          </div>
        </footer>
      )}
    </div>
  );
}

/** "a / b" crumbs. */
export function Crumb({ href, children }: { href: string; children: ReactNode }) {
  return (
    <>
      <Link href={href} className="rounded-sm transition-colors hover:text-ink">{children}</Link>
      <ChevronRight className="size-3.5 text-line-strong" aria-hidden />
    </>
  );
}
