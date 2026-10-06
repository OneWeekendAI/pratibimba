// A streaming rewriter for slot markers in HTML:
//
//   <!-- prb:slot="hero" -->the page's own content<!-- /prb:slot -->
//
// The region between the markers is the baseline. When a start marker passes, the rewriter
// asks for that slot's decision and holds the region until its end marker; then it writes
// the variant's HTML in its place, or the baseline unchanged when the decision is null.
// Text before a marker streams straight through; a marker split across chunks is held back
// only as long as it could still be a marker.

const START = /<!--\s*prb:slot="([a-z0-9_-]{1,64})"\s*-->/;
const END = '<!-- /prb:slot -->';
const END_RE = /<!--\s*\/prb:slot\s*-->/;
/** Longest a start marker can be: a partial one at the end of a chunk is held back. */
const MAX_MARKER = 128;

export interface Replacement {
  /** HTML for the region, or `null` to keep the baseline. */
  html: string | null;
}

export type Decider = (slot: string) => Promise<Replacement>;

/** Where a chunk may be cut without splitting something that could become a marker. */
function safeCut(buf: string): number {
  const open = buf.lastIndexOf('<');
  if (open === -1 || buf.length - open > MAX_MARKER) return buf.length;
  // Anything from the last '<' on might be the start of a marker: hold it.
  return open;
}

export function rewriter(decide: Decider): TransformStream<string, string> {
  let buf = '';
  let slot: string | null = null; // inside a region
  let startMarker = '';
  let pending: Promise<Replacement> | null = null;

  async function pump(controller: TransformStreamDefaultController<string>, final: boolean) {
    for (;;) {
      if (slot === null) {
        const m = START.exec(buf);
        if (!m) {
          const cut = final ? buf.length : safeCut(buf);
          if (cut > 0) controller.enqueue(buf.slice(0, cut));
          buf = buf.slice(cut);
          return;
        }
        if (m.index > 0) controller.enqueue(buf.slice(0, m.index));
        slot = m[1];
        startMarker = m[0];
        buf = buf.slice(m.index + m[0].length);
        // Ask now; the region's own bytes arrive while Core answers.
        pending = decide(slot).catch(() => ({ html: null }));
      } else {
        const e = END_RE.exec(buf);
        if (!e) {
          if (final) {
            // An unterminated region: give it back untouched.
            controller.enqueue(startMarker + buf);
            buf = '';
          }
          return;
        }
        const baseline = buf.slice(0, e.index);
        const r = await pending!;
        controller.enqueue(startMarker + (r.html ?? baseline) + END);
        buf = buf.slice(e.index + e[0].length);
        slot = null;
        pending = null;
      }
    }
  }

  return new TransformStream<string, string>({
    async transform(chunk, controller) {
      buf += chunk;
      await pump(controller, false);
    },
    async flush(controller) {
      await pump(controller, true);
    },
  });
}

/**
 * Insert `tag()` before `</body>`, or at the end if the page has none. It is called only when
 * `</body>` arrives, so it can describe everything decided above it.
 */
export function beforeBodyEnd(tag: () => string): TransformStream<string, string> {
  let buf = '';
  let done = false;
  return new TransformStream<string, string>({
    transform(chunk, controller) {
      if (done) return controller.enqueue(chunk);
      buf += chunk;
      const i = buf.search(/<\/body\s*>/i);
      if (i >= 0) {
        controller.enqueue(buf.slice(0, i) + tag() + buf.slice(i));
        buf = '';
        done = true;
        return;
      }
      const keep = Math.min(buf.length, 8); // a split "</body>"
      controller.enqueue(buf.slice(0, buf.length - keep));
      buf = buf.slice(buf.length - keep);
    },
    flush(controller) {
      if (!done) controller.enqueue(buf + tag());
    },
  });
}
