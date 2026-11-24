'use client';
// Variant previews render in a sandboxed iframe with no scripts and no same-origin access.
// A copy slot's variant is a JSON object of fields, not markup: it shows as labelled fields.
import { FieldView } from '@/components/assets';
import type { FieldValue } from '@/lib/api';

/** The fields of a copy variant's payload, or null when the payload is markup. */
export function copyFields(html: string | null): Record<string, FieldValue> | null {
  const t = html?.trim();
  if (!t || !t.startsWith('{')) return null;
  try {
    const v = JSON.parse(t);
    return v && typeof v === 'object' && !Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}

export function Preview({ html, label, height = 140, scale = 1, site = '' }: { html: string | null; label: string; height?: number; scale?: number; site?: string }) {
  const fields = copyFields(html);
  if (fields) {
    return (
      <dl className="min-h-16 space-y-2 rounded-md border border-line bg-paper px-3 py-2.5 text-[13px] leading-snug" data-testid="copy-fields" aria-label={label}>
        {Object.entries(fields).map(([n, v]) => (
          <div key={n}>
            <dt className="font-mono text-[11px] text-muted">{n}</dt>
            <dd className="text-ink"><FieldView value={v} site={site} compact /></dd>
          </div>
        ))}
      </dl>
    );
  }
  if (html === null) {
    return (
      <div className="grid place-items-center rounded-md border border-dashed border-line-strong bg-paper px-3 text-center text-[12px] text-muted" style={{ height }}>
        The page’s own content
      </div>
    );
  }
  // An uploaded image is /_prb/assets/... on the site; the dashboard is Core itself, at /assets/...
  const body = html.replace(/(["'(\s])\/_prb\/assets\//g, '$1/assets/');
  const doc = `<!doctype html><html><head><meta charset="utf-8"><style>html{zoom:${scale}}body{margin:12px;font:15px/1.4 Georgia,serif;color:#1d1b16;background:#fcfaf5}h1{font-size:22px;margin:0 0 6px}a{color:#a4532a}img{max-width:100%;height:auto}</style></head><body>${body}</body></html>`;
  return <iframe title={label} loading="eager" sandbox="" srcDoc={doc} className="w-full rounded-md border border-line bg-white" style={{ height }} />;
}
