'use client';
// The install wizard. Every step after the first is a probe Core runs against the site:
// a step is done when Pratibimba observes it, never when someone ticks a box.
import { useQuery } from '@tanstack/react-query';
import { Suspense, useEffect, useState } from 'react';
import { Shell } from '@/components/shell';
import { Button, Card, Chip, Empty, Loading, Prose, type ChipTone } from '@/components/ui';
import { api, type ProbeStep } from '@/lib/api';
import { useParam } from '@/lib/page';

type Path = 'next' | 'edge' | 'script';

const TONE: Record<ProbeStep['state'], [ChipTone, string]> = {
  done: ['live', 'observed'],
  pending: ['warn', 'waiting'],
  failed: ['paused', 'failed'],
  later: ['off', 'Phase 2'],
  choice: ['off', 'your choice'],
};

function Code({ children }: { children: string }) {
  return <pre className="overflow-x-auto rounded-md bg-chrome px-4 py-3 font-mono text-[12px] leading-relaxed text-chrome-ink">{children}</pre>;
}

function snippets(path: Path, site: string) {
  if (path === 'next')
    return (
      <div className="space-y-3 text-[13px]">
        <Code>{`npm install @pratibimba/next@0.1.0`}</Code>
        <p className="text-ink-2"><b className="font-mono text-ink">lib/prb.ts</b></p>
        <Code>{`import { createPratibimba } from '@pratibimba/next';
export const prb = createPratibimba({ core: process.env.PRB_CORE_URL!, site: '${site}' });`}</Code>
        <p className="text-ink-2"><b className="font-mono text-ink">proxy.ts</b> (Next.js 16 renamed middleware to proxy)</p>
        <Code>{`import { prb } from './lib/prb';
export const proxy = prb.proxy;
export const config = { matcher: ['/((?!_next/|_prb/|favicon.ico).*)'] };`}</Code>
        <p className="text-ink-2"><b className="font-mono text-ink">app/%5Fprb/[...path]/route.ts</b> (a plain <code>_prb</code> folder is private in the App Router)</p>
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
        <p className="text-ink-2">Mark regions in your HTML: <code className="font-mono">{`<!-- prb:slot="hero" -->…<!-- /prb:slot -->`}</code>. No flicker: they are rewritten before the browser sees a byte.</p>
      </div>
    );
  return (
    <div className="space-y-3 text-[13px]">
      <p className="text-ink-2">Proxy <code className="font-mono">/_prb/*</code> on your origin to Core, then put both tags in <code>&lt;head&gt;</code>, without <code>defer</code>:</p>
      <Code>{`<script src="/_prb/manifest/${site}.js"></script>
<script src="/_prb/sdk.js" data-site="${site}"></script>`}</Code>
      <p className="text-warn-ink">▲ The script path can flicker: slots stay hidden for up to 400 ms while it decides. The two server-side paths above cannot.</p>
    </div>
  );
}

function Step({ n, title, step, children }: { n: number; title: string; step?: ProbeStep; children?: React.ReactNode }) {
  const [tone, label] = step ? TONE[step.state] : (['off', ''] as [ChipTone, string]);
  return (
    <Card className="p-5" data-testid={`step-${step?.id ?? n}`}>
      <div className="mb-3 flex items-center gap-3">
        <span className={`grid size-7 place-items-center rounded-full text-[13px] font-semibold ${step?.state === 'done' ? 'bg-[color:var(--good)] text-white' : 'bg-surface-2 text-ink-2'}`}>{step?.state === 'done' ? '✓' : n}</span>
        <h3 className="text-[15px] font-semibold">{title}</h3>
        {step && <span className="ml-auto"><Chip tone={tone}>{label}</Chip></span>}
      </div>
      {step && step.state !== 'choice' && <p className="mb-3 text-[13px] text-ink-2" data-testid={`detail-${step.id}`}><Prose text={step.detail} /></p>}
      {children}
    </Card>
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
    <div className="max-w-3xl space-y-4">
      <p className="text-[13px] text-ink-2">
        Probing <b className="font-mono text-ink">{q.data.origin}</b> every 5 seconds.{' '}
        <Button variant="ghost" className="ml-2 h-7 px-2 text-xs" onClick={() => q.refetch()} disabled={q.isFetching}>{q.isFetching ? 'Checking…' : 'Check now'}</Button>
      </p>
      <Step n={1} title="How is your site built?" step={s.framework}>
        <div role="radiogroup" className="space-y-2 text-[13px]">
          {([
            ['next', 'Next.js', 'middleware + <Slot />: decided during the server render. No flicker.'],
            ['edge', 'Another code-based framework', 'the generic edge worker rewrites marked HTML as it streams. No flicker.'],
            ['script', 'Script tag fallback', 'works anywhere; can flicker for up to 400 ms.'],
          ] as [Path, string, string][]).map(([id, name, note]) => (
            <label key={id} className="flex cursor-pointer items-start gap-2">
              <input
                type="radio"
                name="path"
                className="mt-1 accent-[color:var(--accent)]"
                checked={path === id}
                onChange={() => {
                  setPath(id);
                  try {
                    localStorage.setItem(`prb-install-path-${site}`, id);
                  } catch {}
                }}
              />
              <span><b className="font-medium">{name}</b> <span className="text-ink-2">— {note}</span></span>
            </label>
          ))}
          <p className="pt-1 text-xs text-muted">Shopify, WordPress, Webflow, Framer and Wix are not listed yet: each needs its own research round first, and Liquid themes have no pre-paint path.</p>
        </div>
      </Step>
      <Step n={2} title="Install the package" step={s.package}>{snippets(path, site)}</Step>
      <Step n={3} title="Prove we can reach it" step={s.reach} />
      <Step n={4} title="Mark a slot" step={s.slot}>
        <p className="text-[13px] text-ink-2">
          {path === 'next' ? <>Wrap a region: <code className="font-mono">{`<prb.Slot id="hero"><Hero /></prb.Slot>`}</code>, and declare <code>hero</code> in your site file.</> : path === 'edge' ? <>Wrap a region in <code className="font-mono">{`<!-- prb:slot="hero" -->`}</code> markers.</> : <>Give the region a selector and declare it in your site file.</>} This step is done when the first exposure arrives.
        </p>
      </Step>
      <Step n={5} title="Connect your coding agent" step={s.agent}>
        <p className="text-[13px] text-muted">The MCP endpoint and the instruction pack arrive in Phase 2. Until then, write drafts on the Approvals page.</p>
      </Step>
      <Step n={6} title="Add a second variant and turn it on" step={s.live}>
        <p className="text-[13px] text-ink-2">Write a draft on the Approvals page and approve it; this step is done when a visitor has seen it.</p>
      </Step>
    </div>
  );
}

function Page() {
  const site = useParam('site');
  return (
    <Shell site={site} title={<><div className="text-xs text-muted">{site}</div><h1 className="font-serif text-3xl">Install</h1></>}>
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
