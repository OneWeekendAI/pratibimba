'use client';
// Sheet 01, the site home: what is happening right now, the install strip, the slot list.
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Suspense, useEffect, useState } from 'react';
import { LiftPill, Sparkline, TrafficBar } from '@/components/charts';
import { ConnectAgentCard, McpTailCard } from '@/components/agent';
import { Shell } from '@/components/shell';
import { Card, Chip, Empty, Loading, SectionTitle, type ChipTone } from '@/components/ui';
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

function Tile({ label, value, sub, children, tone }: { label: string; value: React.ReactNode; sub?: React.ReactNode; children?: React.ReactNode; tone?: 'amber' }) {
  return (
    <Card className={`flex flex-col p-4 ${tone === 'amber' ? 'border-[color:var(--warn)]' : ''}`} data-testid={`tile-${label}`}>
      <div className="text-xs text-ink-2">{label}</div>
      <div className="mt-1 text-[32px] leading-none font-semibold tracking-tight">{value}</div>
      {sub && <div className="mt-1.5 text-xs text-muted">{sub}</div>}
      <div className="mt-auto pt-3">{children}</div>
    </Card>
  );
}

const STATE_TONE: Record<ProbeStep['state'], ChipTone> = { done: 'live', pending: 'warn', failed: 'paused', later: 'off', choice: 'off' };

function InstallStrip({ site }: { site: string }) {
  const probes = useQuery({ queryKey: ['install', site], queryFn: () => api.get<{ steps: ProbeStep[] }>(`/sites/${site}/install`), refetchInterval: 30_000 });
  const steps = probes.data?.steps ?? [];
  const pkg = steps.find((s) => s.id === 'package');
  const sdk = pkg?.state === 'done' ? (/x-prb-sdk: (\S+)/.exec(pkg.detail)?.[1] ?? 'fallback script') : 'not detected';
  const show: [ProbeStep['id'], string][] = [
    ['package', 'SDK on the site'],
    ['reach', '/_prb route'],
    ['slot', 'first exposure'],
  ];
  return (
    <Link href={`/install/?site=${site}`} className="mb-6 flex flex-wrap items-center gap-x-5 gap-y-2 rounded-lg border border-line bg-surface px-4 py-2.5 text-xs hover:border-line-strong" data-testid="install-strip">
      <span className="text-ink-2">
        Install mode <b className="font-medium text-ink">session cookie</b>
      </span>
      <span className="text-ink-2">
        SDK <b className="font-mono font-medium text-ink">{probes.isLoading ? '…' : sdk}</b>
      </span>
      {show.map(([id, label]) => {
        const s = steps.find((x) => x.id === id);
        return (
          <span key={id} title={s?.detail}>
            <Chip tone={s ? STATE_TONE[s.state] : 'off'}>{label}</Chip>
          </span>
        );
      })}
      <span className="ml-auto text-accent">Install guide →</span>
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
        <div role="alert" className="mb-4 rounded-lg border border-[color:var(--warn)] bg-surface px-4 py-2.5 text-[13px] text-warn-ink">
          ▲ {d.stale}
        </div>
      )}
      <SectionTitle hint={<span className="inline-flex items-center gap-2">{o.live ? <Chip tone="live">live</Chip> : <Chip tone="off">reconnecting</Chip>}</span>}>Right now</SectionTitle>
      <div className="mb-8 grid gap-3 md:grid-cols-3">
        <Tile label="Decisions in the last minute" value={int(d.decisions_last_minute)} sub="per minute, last 30 minutes" tone={d.stale ? 'amber' : undefined}>
          <Sparkline values={d.sparkline} label="decisions per minute, last 30 minutes" />
        </Tile>
        <Tile label="Conversions in the last hour" value={int(d.conversions_last_hour)} sub="credited to a decision" />
        <Tile
          label="Lift today vs holdout"
          value={d.lift_today ? pp(d.lift_today.estimate) : '—'}
          sub={d.lift_today ? `95% CS ${pp(d.lift_today.lo)} to ${pp(d.lift_today.hi)} · ${int(d.treated_today.n + d.holdout_today.n)} closed decisions` : 'no closed windows today yet'}
        />
      </div>

      <SectionTitle hint="last 7 days · lift is the slot's policy against its holdout">Slots</SectionTitle>
      {d.slots.length === 0 ? (
        <Empty>No slots yet. Mark one in code with <code>&lt;Slot id="hero"&gt;</code>, then declare it in the site file.</Empty>
      ) : (
        <Card className="overflow-hidden">
          <table className="w-full text-[13px]">
            <thead className="border-b border-line text-left text-xs text-muted">
              <tr>
                <th className="px-4 py-2 font-medium">Slot</th>
                <th className="px-4 py-2 font-medium">Champion</th>
                <th className="px-4 py-2 font-medium">7-day lift</th>
                <th className="px-4 py-2 text-right font-medium">Variants</th>
                <th className="w-48 px-4 py-2 font-medium">Traffic</th>
              </tr>
            </thead>
            <tbody>
              {d.slots.map((s) => (
                <tr
                  key={s.key}
                  className="cursor-pointer border-b border-line last:border-0 hover:bg-surface-2/60"
                  onClick={() => router.push(`/slot/?site=${site}&slot=${s.key}`)}
                  data-testid={`slot-row-${s.key}`}
                >
                  <td className="px-4 py-3">
                    <Link href={`/slot/?site=${site}&slot=${s.key}`} className="font-mono font-medium">{s.key}</Link>
                    <div className="text-xs text-muted">metric {s.metric}</div>
                  </td>
                  <td className="px-4 py-3 font-mono text-[12.5px]">{s.champion ?? '—'}</td>
                  <td className="px-4 py-3"><LiftPill lift={s.lift_7d} small /></td>
                  <td className="tabular px-4 py-3 text-right">{s.variants}</td>
                  <td className="px-4 py-3"><TrafficBar parts={s.traffic} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {d.pending_approvals > 0 && (
        <p className="mt-4 text-[13px]">
          <Link className="text-accent underline" href={`/approvals/?site=${site}`}>
            {d.pending_approvals} draft{d.pending_approvals === 1 ? '' : 's'} waiting for review
          </Link>
        </p>
      )}
      <div className="mt-8 space-y-8">
        <ConnectAgentCard site={site} />
        <McpTailCard site={site} />
      </div>
    </>
  );
}

function Page() {
  const site = useParam('id');
  return (
    <Shell site={site} title={<><div className="text-xs text-muted">Site</div><h1 className="font-serif text-3xl">{site}</h1></>}>
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
