'use client';
// Run: what is happening right now, the install strip, the slot list.
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Suspense, useEffect, useState } from 'react';
import { LiftPill, Sparkline, TrafficBar } from '@/components/charts';
import { ChevronRight, Cookie, Crown, Eye, Gauge, LayoutGrid, PackageCheck, Plug, ShieldCheck, Target, TrendingUp, TriangleAlert, Wifi, WifiOff, Zap, type LucideIcon } from 'lucide-react';
import { Shell } from '@/components/shell';
import { Card, Chip, Empty, Help, Loading, SectionTitle, type ChipTone } from '@/components/ui';
import { api, type Overview, type ProbeStep } from '@/lib/api';
import { int, pp } from '@/lib/format';
import { useParam } from '@/lib/page';

/** Live numbers over SSE; falls back to the last fetch if the stream drops. */
function useOverview(site: string) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['overview', site], queryFn: () => api.get<Overview>(`/sites/${site}/overview`) });
  const [live, setLive] = useState(false);
  useEffect(() => {
    const es = new EventSource(`/api/sites/${site}/stream`);
    es.addEventListener('overview', (e) => {
      setLive(true);
      qc.setQueryData(['overview', site], JSON.parse((e as MessageEvent).data));
    });
    es.onerror = () => setLive(false);
    return () => es.close();
  }, [site, qc]);
  return { ...q, live };
}

function Stat({ label, short, value, sub, help, children, tone }: { label: string; short: string; icon?: LucideIcon; value: React.ReactNode; sub?: React.ReactNode; help?: React.ReactNode; children?: React.ReactNode; tone?: 'amber' }) {
  return (
    <div className="flex min-w-0 flex-col px-3 py-4 sm:px-5 sm:py-5" data-testid={`tile-${label}`}>
      <div className="flex min-h-6 items-center gap-0.5 text-[12px] leading-tight text-muted sm:text-[13px]" title={label}>
        <span className="sr-only">{label}</span><span aria-hidden>{short}</span>
        {help && <span className="max-sm:hidden"><Help title={label}>{help}</Help></span>}
      </div>
      <div className={`mt-1.5 text-[22px] leading-none font-semibold tracking-[-0.03em] tabular sm:text-[30px] ${tone === 'amber' ? 'text-warn-ink' : 'text-ink'}`}>{value}</div>
      {sub && <div className="mt-2 text-[12px] text-muted tabular max-sm:hidden">{sub}</div>}
      {children && <div className="mt-auto pt-3">{children}</div>}
    </div>
  );
}

const STATE_TONE: Record<ProbeStep['state'], ChipTone> = { done: 'live', pending: 'warn', failed: 'paused', later: 'off', choice: 'off' };

function InstallStrip({ site }: { site: string }) {
  const probes = useQuery({ queryKey: ['install', site], queryFn: () => api.get<{ steps: ProbeStep[] }>(`/sites/${site}/install`), refetchInterval: 30_000 });
  const steps = probes.data?.steps ?? [];
  const pkg = steps.find((s) => s.id === 'package');
  const sdk = pkg?.state === 'done' ? (/x-prb-sdk: (\S+)/.exec(pkg.detail)?.[1] ?? 'fallback script') : 'not detected';
  const show: [ProbeStep['id'], string, LucideIcon][] = [
    ['package', 'SDK', PackageCheck],
    ['reach', '/_prb', Plug],
    ['slot', 'exposure', Eye],
  ];
  return (
    <Link href={`/install/?site=${site}`} className="group mb-8 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-line bg-surface px-4 py-2.5 text-[13px] transition-colors hover:border-line-strong" data-testid="install-strip">
      <span className="inline-flex items-center gap-1.5 text-muted">
        <Cookie className="size-3.5" aria-label="Install mode" />session cookie
      </span>
      <span className="text-muted">
        SDK <span className="font-mono text-[12.5px] text-ink">{probes.isLoading ? '…' : sdk}</span>
      </span>
      <span className="flex flex-wrap gap-1.5">
        {show.map(([id, label, I]) => {
          const s = steps.find((x) => x.id === id);
          return <Chip key={id} tone={s ? STATE_TONE[s.state] : 'off'} icon={I} title={s?.detail}>{label}</Chip>;
        })}
      </span>
      <ChevronRight className="ml-auto size-4 text-muted transition-transform duration-150 group-hover:translate-x-0.5" aria-label="Install guide" />
    </Link>
  );
}

function Home() {
  const site = useParam('id')!;
  const router = useRouter();
  const o = useOverview(site);
  if (o.isLoading || !o.data) return <Loading />;
  const d = o.data;
  return (
    <>
      <InstallStrip site={site} />
      {d.stale && (
        <div role="alert" className="mb-6 flex items-center gap-2 rounded-lg bg-warn-soft px-4 py-2.5 text-[13px] text-warn-ink">
          <TriangleAlert className="size-4 shrink-0" aria-hidden /> {d.stale}
        </div>
      )}
      <SectionTitle hint={o.live ? <span className="inline-flex items-center gap-1.5"><Wifi className="size-3.5" aria-hidden />live</span> : <span className="inline-flex items-center gap-1.5"><WifiOff className="size-3.5" aria-hidden />reconnecting</span>}>Right now</SectionTitle>
      <Card className="mb-12 grid grid-cols-3 divide-x divide-line">
        <Stat label="Decisions in the last minute" short="Decisions / min" icon={Zap} value={int(d.decisions_last_minute)} tone={d.stale ? 'amber' : undefined} help="How many visitors Pratibimba chose a version for in the last minute. The line is the last 30 minutes.">
          <Sparkline values={d.sparkline} label="decisions per minute, last 30 minutes" />
        </Stat>
        <Stat label="Conversions in the last hour" short="Conversions / hour" icon={Target} value={int(d.conversions_last_hour)} help="Goals reached in the last hour that were credited to a decision." />
        <Stat
          label="Lift today vs holdout"
          short="Lift today"
          icon={TrendingUp}
          value={d.lift_today ? pp(d.lift_today.estimate) : '—'}
          sub={d.lift_today ? `${pp(d.lift_today.lo)} to ${pp(d.lift_today.hi)}` : 'no closed windows today'}
          help={<><p>Today&apos;s conversion rate with personalisation, minus the rate for the holdout (visitors who always see the baseline).</p><p>The range is a 95% confidence sequence over {int(d.treated_today.n + d.holdout_today.n)} closed decisions.</p></>}
        />
      </Card>

      <SectionTitle help="Each slot is a region of the site that can change. Lift compares the slot's choices with its holdout over the last 7 days.">Slots</SectionTitle>
      {d.slots.length === 0 ? (
        <Empty icon={LayoutGrid}>No slots yet. Wrap a region in <code className="font-mono">&lt;Slot id="hero"&gt;</code>.</Empty>
      ) : (
        <>
        <Card className="divide-y divide-line sm:hidden">
          {d.slots.map((s) => (
            <Link key={s.key} href={`/slot/?site=${site}&slot=${s.key}`} className="block space-y-2 px-4 py-3.5 active:bg-surface-2" data-testid={`slot-item-${s.key}`}>
              <div className="flex items-center gap-2">
                <span className="font-medium">{s.key}</span>
                <span className="ml-auto"><LiftPill lift={s.lift_7d} small /></span>
              </div>
              <div className="flex items-center gap-2 text-xs text-muted">
                <Crown className="size-3.5 shrink-0" aria-label="champion" /><span className="truncate font-mono">{s.champion ?? '—'}</span>
                <span className="ml-auto w-24 shrink-0"><TrafficBar parts={s.traffic} /></span>
              </div>
            </Link>
          ))}
        </Card>
        <Card className="overflow-x-auto max-sm:hidden">
          <table className="w-full min-w-[560px] text-[13px]">
            <thead className="border-b border-line text-left text-[12px] text-muted">
              <tr>
                <th className="h-10 px-4 font-normal">Slot</th>
                <th className="h-10 px-4 font-normal">Champion</th>
                <th className="h-10 px-4 font-normal">Lift, 7 days</th>
                <th className="h-10 px-4 text-right font-normal">Variants</th>
                <th className="h-10 w-44 px-4 font-normal">Traffic</th>
              </tr>
            </thead>
            <tbody>
              {d.slots.map((s) => (
                <tr
                  key={s.key}
                  className="cursor-pointer border-b border-line transition-colors last:border-0 hover:bg-surface-2/60"
                  onClick={() => router.push(`/slot/?site=${site}&slot=${s.key}`)}
                  data-testid={`slot-row-${s.key}`}
                >
                  <td className="px-4 py-3.5">
                    <Link href={`/slot/?site=${site}&slot=${s.key}`} className="rounded-sm font-medium text-ink">{s.key}</Link>
                    <div className="mt-0.5 flex items-center gap-1 text-[12px] text-muted" title="metric"><Target className="size-3" aria-label="metric" />{s.metric}</div>
                  </td>
                  <td className="px-4 py-3.5"><span className="inline-flex items-center gap-1.5 font-mono text-[12.5px] text-ink-2">{s.champion && <Crown className="size-3.5 text-muted" aria-label="champion" />}{s.champion ?? '—'}</span></td>
                  <td className="px-4 py-3.5"><LiftPill lift={s.lift_7d} small /></td>
                  <td className="tabular px-4 py-3.5 text-right text-ink-2">{s.variants}</td>
                  <td className="px-4 py-3.5"><TrafficBar parts={s.traffic} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
        </>
      )}
      {d.pending_approvals > 0 && (
        <Link className="mt-4 inline-flex items-center gap-2 rounded-md bg-warn-soft px-3 py-2 text-[13px] font-medium text-warn-ink transition-opacity hover:opacity-85" href={`/approvals/?site=${site}`}>
          <ShieldCheck className="size-4" aria-hidden />
          {d.pending_approvals} draft{d.pending_approvals === 1 ? '' : 's'} waiting for review
          <ChevronRight className="size-4" aria-hidden />
        </Link>
      )}
    </>
  );
}

function Page() {
  const site = useParam('id');
  return (
    <Shell site={site} title={site ?? 'Site'} help="What visitors are being shown right now. Pause stops every slot at once.">
      {site ? <Home /> : <Empty>No site given.</Empty>}
    </Shell>
  );
}

export default function SitePage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
