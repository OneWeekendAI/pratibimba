'use client';
import clsx from 'clsx';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';
import { api, type SiteRow } from '@/lib/api';

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
        { href: `/approvals/?site=${site}`, label: 'Approvals', match: ['/approvals'], dot: (current?.pending ?? 0) > 0 },
        { href: `/install/?site=${site}`, label: 'Install', match: ['/install'] },
      ]
    : [];
  return (
    <div className="flex min-h-screen">
      <aside className="sticky top-0 flex h-screen w-56 shrink-0 flex-col bg-chrome px-3 py-4 text-chrome-ink max-md:hidden">
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
      </aside>
      <div className="min-w-0 flex-1">
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
