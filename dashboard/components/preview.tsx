'use client';
// Variant previews render in a sandboxed iframe with no scripts and no same-origin access.
export function Preview({ html, label, height = 140, scale = 1 }: { html: string | null; label: string; height?: number; scale?: number }) {
  if (html === null) {
    return (
      <div className="grid place-items-center rounded-md border border-dashed border-line-strong bg-surface-2/50 px-3 text-center text-xs text-muted" style={{ height }}>
        The page’s own content (the baseline lives in your code, not in Pratibimba)
      </div>
    );
  }
  // An uploaded image is /_prb/assets/... on the site; the dashboard is Core itself, at /assets/...
  const body = html.replace(/(["'(\s])\/_prb\/assets\//g, '$1/assets/');
  const doc = `<!doctype html><html><head><meta charset="utf-8"><style>html{zoom:${scale}}body{margin:12px;font:15px/1.4 Georgia,serif;color:#1d1b16;background:#fcfaf5}h1{font-size:22px;margin:0 0 6px}a{color:#a4532a}img{max-width:100%;height:auto}</style></head><body>${body}</body></html>`;
  return <iframe title={label} loading="eager" sandbox="" srcDoc={doc} className="w-full rounded-md border border-line bg-white" style={{ height }} />;
}
