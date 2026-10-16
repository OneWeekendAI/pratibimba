'use client';
// G2: a live variant's planned traffic share, changed after approval. One small control on the
// variant card; Core checks the range again and records the change with the reviewer's name.
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { useActor } from '@/components/shell';
import { ShareSlider } from '@/components/slider';
import { Button } from '@/components/ui';
import { api, ApiError, type VariantRow } from '@/lib/api';

export function ShareEditor({ site, slot, v }: { site: string; slot: string; v: VariantRow }) {
  const qc = useQueryClient();
  const [actor] = useActor();
  const [open, setOpen] = useState(false);
  const [share, setShare] = useState(Math.round((v.traffic_share ?? 0.5) * 100));
  const [error, setError] = useState<string | null>(null);
  const save = useMutation({
    mutationFn: () => api.post(`/variants/${v.id}/traffic-share`, { actor, traffic_share: share / 100 }),
    onSuccess: () => {
      setOpen(false);
      setError(null);
      qc.invalidateQueries({ queryKey: ['slot', site, slot] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'failed'),
  });
  if (!open) {
    return (
      <button type="button" className="mt-1 text-[11px] text-accent hover:underline" onClick={() => setOpen(true)}>
        Change share
      </button>
    );
  }
  return (
    <div className="mt-2 space-y-2" data-testid={`share-editor-${v.key}`}>
      <div className="flex items-baseline justify-between text-[11px]">
        <span className="text-muted">Planned share</span>
        <span className="tabular font-semibold">{share}%</span>
      </div>
      <ShareSlider value={share} onChange={setShare} label={`new traffic share for ${v.key}`} />
      {error && <p role="alert" className="text-[11.5px] text-pause-ink">{error}</p>}
      <div className="flex items-center gap-2">
        <Button onClick={() => save.mutate()} disabled={!actor.trim() || save.isPending}>Save {share}%</Button>
        <Button variant="ghost" onClick={() => setOpen(false)}>Cancel</Button>
      </div>
      {!actor.trim() && <p className="text-[11px] text-muted">Enter your name at the top to change it.</p>}
    </div>
  );
}
