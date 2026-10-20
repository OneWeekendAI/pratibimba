'use client';
// S11: images. The shelf uploads and lists a site's images; FieldView shows a copy field's value
// as what a visitor sees: text as text, an image as the image.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useRef, useState } from 'react';
import { Button, Card, Chip, SectionTitle } from '@/components/ui';
import { api, ApiError, type Asset, type CopyField, type FieldValue, type ImageValue } from '@/lib/api';

/** Where the dashboard (served by Core at /app) loads an uploaded image. */
export const assetSrc = (site: string, id: string) => `/assets/${encodeURIComponent(site)}/${id}`;

const kb = (n: number) => (n < 1024 * 1024 ? `${Math.max(1, Math.round(n / 1024))} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);

/** An image value's preview source: an upload, or an https URL. A root-relative path lives on the site, not here. */
function imageSrc(site: string, v: ImageValue): string | null {
  if (v.asset && /^[0-9a-f]{64}$/.test(v.asset)) return assetSrc(site, v.asset);
  if (v.url && v.url.startsWith('https://')) return v.url;
  return null;
}

export function FieldView({ value, field, site, compact = false }: { value: FieldValue | undefined; field?: CopyField; site: string; compact?: boolean }) {
  if (value === undefined || value === '') return <span className="text-muted">—</span>;
  if (typeof value === 'string') {
    return (
      <>
        {value}
        {field && !compact && <span className="ml-2 text-xs text-muted">{[...value].length}/{field.max}</span>}
      </>
    );
  }
  const src = imageSrc(site, value);
  return (
    <figure className="m-0" data-testid="field-image">
      {src ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={src} alt={value.alt} className={compact ? 'max-h-16 rounded border border-line' : 'max-h-40 max-w-full rounded border border-line'} />
      ) : (
        <div className="rounded border border-dashed border-line-strong px-2 py-3 text-xs text-muted">{value.url} on your site</div>
      )}
      <figcaption className="mt-1 text-xs text-muted">{value.alt}</figcaption>
    </figure>
  );
}

/** Upload and list the site's images. Uploading shows nothing to visitors; a variant that uses one does, once approved. */
export function AssetShelf({ site, onPick }: { site: string; onPick?: (a: Asset) => void }) {
  const qc = useQueryClient();
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const q = useQuery({ queryKey: ['assets', site], queryFn: () => api.get<{ assets: Asset[] }>(`/sites/${site}/assets`) });
  const up = useMutation({
    mutationFn: (f: File) => api.upload(site, f),
    onSuccess: () => {
      setError(null);
      qc.invalidateQueries({ queryKey: ['assets', site] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'upload failed'),
  });
  const assets = q.data?.assets ?? [];
  const copy = (a: Asset) => {
    void navigator.clipboard?.writeText(a.id).catch(() => {});
    setCopied(a.id);
  };
  return (
    <Card className="p-5" data-testid="asset-shelf">
      <SectionTitle hint="png, jpeg, webp, avif, gif or svg">Images</SectionTitle>
      <div className="flex flex-wrap items-center gap-3">
        <input
          ref={input}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/avif,image/gif,image/svg+xml"
          className="hidden"
          data-testid="asset-input"
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) up.mutate(f);
            e.target.value = '';
          }}
        />
        <Button variant="ghost" onClick={() => input.current?.click()} disabled={up.isPending}>{up.isPending ? 'Uploading…' : 'Upload an image'}</Button>
        {error && <span role="alert" className="text-[13px] text-pause-ink">{error}</span>}
      </div>
      {assets.length > 0 && (
        <ul className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4" data-testid="asset-list">
          {assets.map((a) => (
            <li key={a.id} className="overflow-hidden rounded-md border border-line" data-testid={`asset-${a.id.slice(0, 12)}`}>
              <div className="grid h-24 place-items-center bg-surface-2/50">
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img src={assetSrc(site, a.id)} alt={a.name ?? ''} className="max-h-24 max-w-full" />
              </div>
              <div className="space-y-1 p-2 text-[11.5px]">
                <div className="truncate" title={a.name ?? a.id}>{a.name ?? a.id.slice(0, 12)}</div>
                <div className="flex items-center justify-between text-muted">
                  <span>{a.width && a.height ? `${a.width}×${a.height} · ` : ''}{kb(a.bytes)}</span>
                  {a.in_use ? <Chip tone="live">in use</Chip> : null}
                </div>
                <button type="button" className="text-accent underline" onClick={() => (onPick ? onPick(a) : copy(a))}>
                  {onPick ? 'Use' : copied === a.id ? 'Copied' : 'Copy id'}
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
