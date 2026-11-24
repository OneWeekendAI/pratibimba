'use client';
// The install wizard. Every step after the first is a probe Core runs against the site:
// a step is done when Pratibimba observes it, never when someone ticks a box.
import { useQuery } from '@tanstack/react-query';
import { Suspense, useEffect, useState } from 'react';
import { Shell } from '@/components/shell';
import { Bot, Check, Globe, Layers, PackagePlus, Plug, RefreshCw, Rocket, SquareDashed, TriangleAlert, type LucideIcon } from 'lucide-react';
import { Button, Card, Chip, Empty, Help, Loading, Prose, type ChipTone } from '@/components/ui';
import { ConnectAgentCard } from '@/components/agent';
import { api, type ProbeStep } from '@/lib/api';
import { useParam } from '@/lib/page';

type Path = 'next' | 'edge' | 'script';

const TONE: Record<ProbeStep['state'], [ChipTone, string]> = {
  done: ['live', 'observed'],
  pending: ['warn', 'waiting'],
  failed: ['paused', 'failed'],
  later: ['off', 'later'],
  choice: ['off', 'your choice'],
};

function Code({ children }: { children: string }) {
  return <pre className="overflow-x-auto rounded-md border border-line bg-paper px-4 py-3 font-mono text-[12.5px] leading-relaxed text-ink">{children}</pre>;
}

function snippets(path: Path, site: string) {
  if (path === 'next')
    return (
      <div className="space-y-3 text-[13px]">
        <Code>{`npm install @pratibimba/next@0.1.0`}</Code>
        <p className="font-mono text-[12px] text-muted">lib/prb.ts</p>
        <Code>{`import { createPratibimba } from '@pratibimba/next';
export const prb = createPratibimba({ core: process.env.PRB_CORE_URL!, site: '${site}' });`}</Code>
        <p className="inline-flex items-center gap-0.5 font-mono text-[12px] text-muted">proxy.ts<Help title="proxy.ts"><p>Next.js 16 renamed middleware to proxy.</p></Help></p>
        <Code>{`import { prb } from './lib/prb';
export const proxy = prb.proxy;
export const config = { matcher: ['/((?!_next/|_prb/|favicon.ico).*)'] };`}</Code>
        <p className="inline-flex items-center gap-0.5 font-mono text-[12px] text-muted">app/%5Fprb/[...path]/route.ts<Help title="Why %5Fprb"><p>A plain <code>_prb</code> folder is private in the App Router.</p></Help></p>
        <Code>{`import { prb } from '@/lib/prb';
export const { GET, POST } = prb.route;`}</Code>
      </div>
    );
  if (path === 'edge')
    return (
      <div className="space-y-3 text-[13px]">
        <Code>{`npm install @pratibimba/edge@0.1.0`}</Code>
        <Code>{`import { createEdge } from '@pratibimba/edge';
const prb = createEdge({ core: 'https://core.internal', site: '${site}' });
export default { fetch: (req) => prb.handleRequest(req) };   // Cloudflare; Deno and Vercel Edge take the same handler`}</Code>
        <p className="text-ink-2">Mark regions: <code className="font-mono">{`<!-- prb:slot="hero" -->…<!-- /prb:slot -->`}</code></p>
      </div>
    );
  return (
    <div className="space-y-3 text-[13px]">
      <p className="text-ink-2">Proxy <code className="font-mono">/_prb/*</code> to Core, then in <code>&lt;head&gt;</code>, without <code>defer</code>:</p>
      <Code>{`<script src="/_prb/manifest/${site}.js"></script>
<script src="/_prb/sdk.js" data-site="${site}"></script>`}</Code>
      <p className="flex items-center gap-1.5 text-warn-ink"><TriangleAlert className="size-4 shrink-0" aria-hidden />This path can flicker for up to 400 ms.</p>
    </div>
  );
}

function Step({ n, title, step, children }: { n: number; title: string; icon?: LucideIcon; step?: ProbeStep; children?: React.ReactNode }) {
  const [tone, label] = step ? TONE[step.state] : (['off', ''] as [ChipTone, string]);
  const done = step?.state === 'done';
  return (
    <li className="relative flex gap-4 px-4 py-5 sm:px-6" data-testid={`step-${step?.id ?? n}`}>
      <span aria-label={done ? 'done' : `step ${n}`} className={`grid size-6 shrink-0 place-items-center rounded-full text-[12px] font-medium tabular ${done ? 'bg-primary text-primary-ink' : 'border border-line-strong text-ink-2'}`}>
        {done ? <Check className="size-3.5" strokeWidth={2.5} aria-hidden /> : n}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-h-6 flex-wrap items-center gap-x-3 gap-y-1">
          <h3 className="text-[14px] font-medium text-ink">{title}</h3>
          {step && <span className="ml-auto"><Chip tone={tone}>{label}</Chip></span>}
        </div>
        {step && step.state !== 'choice' && <p className="mt-1 text-[13px] text-muted" data-testid={`detail-${step.id}`}><Prose text={step.detail} /></p>}
        {children && <div className="mt-4">{children}</div>}
      </div>
    </li>
  );
}

function Wizard() {
  const site = useParam('site')!;
  const [path, setPath] = useState<Path>('next');
  useEffect(() => {
    try {
      const p = localStorage.getItem(`prb-install-path-${site}`) as Path | null;
      if (p) setPath(p);
    } catch {}
  }, [site]);
  const q = useQuery({ queryKey: ['install', site], queryFn: () => api.get<{ origin: string; steps: ProbeStep[] }>(`/sites/${site}/install`), refetchInterval: 5000 });
  if (q.isLoading || !q.data) return <Loading />;
  const s = Object.fromEntries(q.data.steps.map((x) => [x.id, x])) as Record<ProbeStep['id'], ProbeStep>;
  return (
    <div className="max-w-3xl space-y-6">
      <div className="flex flex-wrap items-center gap-1.5 text-[13px] text-ink-2">
        <Globe className="size-4 text-muted" aria-hidden />
        <span className="font-mono text-[12.5px] text-ink">{q.data.origin}</span>
        <Help title="Probes"><p>Each step after the first is a probe Core runs against the site every 5 seconds. A step is done when Pratibimba observes it, never when someone ticks a box.</p></Help>
        <Button variant="ghost" icon={RefreshCw} className="ml-auto" onClick={() => q.refetch()} disabled={q.isFetching} aria-label="Check now">{q.isFetching ? 'Checking…' : 'Check now'}</Button>
      </div>
      <Card>
      <ol className="divide-y divide-line">
      <Step n={1} icon={Layers} title="How is your site built?" step={s.framework}>
        <div role="radiogroup" className="space-y-2 text-[13px]">
          {([
            ['next', 'Next.js', 'decided in the server render'],
            ['edge', 'Another code-based framework', 'edge worker rewrites the HTML'],
            ['script', 'Script tag fallback', 'works anywhere, can flicker'],
          ] as [Path, string, string][]).map(([id, name, note]) => (
            <label key={id} className="flex cursor-pointer items-start gap-2.5 rounded-md">
              <input
                type="radio"
                name="path"
                className="mt-[3px] size-3.5 accent-[color:var(--primary)]"
                checked={path === id}
                onChange={() => {
                  setPath(id);
                  try {
                    localStorage.setItem(`prb-install-path-${site}`, id);
                  } catch {}
                }}
              />
              <span><b className="font-medium">{name}</b> <span className="text-muted">{note}</span></span>
            </label>
          ))}
          <p className="inline-flex items-center gap-1 pt-1 text-xs text-muted">Shopify, WordPress, Webflow? <Help title="Hosted platforms"><p>Shopify, WordPress, Webflow, Framer and Wix are not listed yet: each needs its own research round, and Liquid themes have no pre-paint path.</p></Help></p>
        </div>
      </Step>
      <Step n={2} icon={PackagePlus} title="Install the package" step={s.package}>{snippets(path, site)}</Step>
      <Step n={3} icon={Plug} title="Prove we can reach it" step={s.reach} />
      <Step n={4} icon={SquareDashed} title="Mark a slot" step={s.slot}>
        <p className="text-[13px] text-ink-2">
          {path === 'next' ? <>Wrap a region: <code className="font-mono">{`<prb.Slot id="hero"><Hero /></prb.Slot>`}</code>, and declare <code>hero</code> in your site file.</> : path === 'edge' ? <>Wrap a region in <code className="font-mono">{`<!-- prb:slot="hero" -->`}</code> markers.</> : <>Give the region a selector and declare it in your site file.</>}
        </p>
      </Step>
      <Step n={5} icon={Bot} title="Connect your coding agent" step={s.agent}>
        <ConnectAgentCard site={site} bare />
      </Step>
      <Step n={6} icon={Rocket} title="Add a second variant and turn it on" step={s.live}>
        <p className="text-[13px] text-ink-2">Write a draft, approve it in Review, and wait for a visitor to see it.</p>
      </Step>
      </ol>
      </Card>
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title="Install">
      {site ? <Wizard /> : <Empty>No site given.</Empty>}
    </Shell>
  );
}

export default function InstallPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
