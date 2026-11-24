'use client';
// Time travel: skip one visitor forward in the site's sandbox copy and see what each slot would
// show them then. The sandbox has its own clock; the real site is never touched.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Suspense, useState } from 'react';
import { Shell } from '@/components/shell';
import { FastForward as Skip, FlaskConical, RotateCcw } from 'lucide-react';
import { Button, Card, Empty, Loading, inputClass } from '@/components/ui';
import { api, ApiError, type FastForward, type SimStatus } from '@/lib/api';
import { when } from '@/lib/format';
import { useParam } from '@/lib/page';

function Sandbox({ site, status }: { site: string; status: SimStatus }) {
  const qc = useQueryClient();
  const [subject, setSubject] = useState('');
  const [days, setDays] = useState('7');
  const [error, setError] = useState<string | null>(null);
  const [last, setLast] = useState<FastForward | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: ['sim', site] });
  const jump = useMutation({
    mutationFn: () => api.post<FastForward>(`/sites/${site}/sim/fast-forward`, { subject: subject.trim(), days: Number(days) }),
    onSuccess: (r) => {
      setError(null);
      setLast(r);
      refresh();
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const reset = useMutation({
    mutationFn: () => api.post(`/sites/${site}/sim`, {}),
    onSuccess: () => {
      setLast(null);
      setError(null);
      refresh();
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  const n = Number(days);
  return (
    <div className="max-w-3xl space-y-10">
      <Card className="space-y-4 p-5" data-testid="tt-form">
        <div className="flex items-center justify-between gap-3 text-[13px]">
          <span className="text-muted">Sandbox clock</span>
          <span className="font-medium tabular-nums" data-testid="tt-clock">{when(status.clock)}</span>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <input className={`${inputClass} min-w-0 flex-1 font-mono`} placeholder="visitor id" aria-label="Visitor" value={subject} onChange={(e) => setSubject(e.target.value)} />
          <label className="inline-flex items-center gap-2 text-[13px] text-muted">
            Days
            <input className={`${inputClass} w-20 tabular-nums`} type="number" min={0} max={366} aria-label="Days" value={days} onChange={(e) => setDays(e.target.value)} />
          </label>
        </div>
        <div className="flex items-center gap-3 border-t border-line pt-4">
          <Button icon={Skip} disabled={!subject.trim() || !(n >= 0 && n <= 366) || jump.isPending} onClick={() => jump.mutate()}>Skip forward</Button>
          <Button variant="ghost" icon={RotateCcw} disabled={reset.isPending} onClick={() => reset.mutate()}>Start over</Button>
          {error && <span role="alert" className="text-xs text-pause-ink">{error}</span>}
        </div>
      </Card>
      {last && (
        <Card data-testid="tt-result">
          <div className="border-b border-line px-4 py-3 text-[13px] text-muted">
            <span className="font-mono text-ink">{subject.trim()}</span> at {when(last.clock)}
          </div>
          <table className="w-full text-[13px]">
            <tbody className="divide-y divide-line">
              {last.decisions.map((d) => (
                <tr key={d.slot} data-testid={`tt-slot-${d.slot}`}>
                  <td className="h-10 px-4 font-medium">{d.slot}</td>
                  <td className="h-10 px-4 font-mono" data-testid="tt-variant">{'variant' in d ? d.variant : <span className="text-muted">{d.error}</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </div>
  );
}

function Page() {
  const site = useParam('site');
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ['sim', site],
    enabled: !!site,
    // A site with no sandbox answers 404: that is a state, not a failure.
    queryFn: () => api.get<SimStatus>(`/sites/${site}/sim`).catch((e) => (e instanceof ApiError && e.status === 404 ? null : Promise.reject(e))),
    retry: false,
  });
  const start = useMutation({ mutationFn: () => api.post(`/sites/${site}/sim`, {}), onSuccess: () => qc.invalidateQueries({ queryKey: ['sim', site] }) });
  return (
    <Shell site={site} title="Time travel" help="Pick a visitor and skip the sandbox clock forward. Each slot shows what that visitor would be served then. This runs on a copy of your site; real visitors are never touched.">
      {!site || q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <Empty>Could not reach the sandbox.</Empty>
      ) : q.data ? (
        <Sandbox site={site} status={q.data} />
      ) : (
        <div className="max-w-3xl" data-testid="tt-empty">
          <Empty icon={FlaskConical} action={<Button icon={FlaskConical} disabled={start.isPending} onClick={() => start.mutate()}>Start a sandbox</Button>}>No sandbox yet.</Empty>
        </div>
      )}
    </Shell>
  );
}

export default function TimeTravelPage() {
  return (
    <Suspense>
      <Page />
    </Suspense>
  );
}
