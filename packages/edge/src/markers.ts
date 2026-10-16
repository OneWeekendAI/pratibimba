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
  /** A sorted slot's decided order (item keys, best first): the baseline's items are reordered. */
  order?: string[];
  /** Told the item keys a sorted region now shows in the decided order. */
  rendered?: (keys: string[]) => void;
}

// --- sorted slots (F2, G3) ---------------------------------------------------------------------
// The page marks each item with data-prb-item="<key>". The items are the first marked element in
// the region and its siblings (an item inside an item is part of its card). The decided items take
// the items' places in order; an item the order leaves out (gated for this visitor) follows them
// in page order, as the client and next SDKs do. Text between the items stays where it was.
// The region is read with a small tag scanner, not a parser: markup it cannot balance (an implied
// end tag, a stray close) is served in page order rather than guessed at.

const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr']);
const RAW = new Set(['script', 'style', 'textarea', 'title']);
const TOKEN = /<!--[\s\S]*?-->|<\/([a-zA-Z][\w:-]*)\s*>|<([a-zA-Z][\w:-]*)((?:\s+[^\s"'>\/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'=<>`]+))?)*)\s*(\/?)>/g;
const ITEM_ATTR = /(?:^|\s)data-prb-item\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))/;

interface Item {
  key: string;
  start: number;
  end: number;
  parent: number;
}

/** Find the region's items, or `null` when its markup does not balance. */
function scanItems(html: string): Item[] | null {
  const stack: { name: string; id: number; item: Item | null }[] = [];
  const items: Item[] = [];
  let next = 1;
  TOKEN.lastIndex = 0;
  for (let m: RegExpExecArray | null; (m = TOKEN.exec(html)); ) {
    if (m[0].startsWith('<!--')) continue;
    if (m[1] !== undefined) {
      const top = stack.pop();
      if (!top || top.name !== m[1].toLowerCase()) return null;
      if (top.item) {
        top.item.end = TOKEN.lastIndex;
        items.push(top.item);
      }
      continue;
    }
    const name = m[2].toLowerCase();
    if (VOID.has(name) || m[4] === '/') continue;
    if (RAW.has(name)) {
      const close = new RegExp(`</${name}\\s*>`, 'ig');
      close.lastIndex = TOKEN.lastIndex;
      if (!close.exec(html)) return null;
      TOKEN.lastIndex = close.lastIndex;
      continue;
    }
    const a = ITEM_ATTR.exec(m[3]);
    const parent = stack.length ? stack[stack.length - 1].id : 0;
    const item = a ? { key: a[1] ?? a[2] ?? a[3], start: m.index, end: -1, parent } : null;
    stack.push({ name, id: next++, item });
  }
  if (stack.length) return null;
  // Closed inner-first: back to page order, then keep the first item's siblings (an item inside a
  // card is part of the card).
  items.sort((x, y) => x.start - y.start);
  return items.length ? items.filter((i) => i.parent === items[0].parent) : [];
}

/**
 * Reorder a sorted region's items to `order`. Returns the new HTML and the keys that now stand
 * in the decided order (the items the order leaves out are not among them).
 */
export function reorderItems(html: string, order: string[]): { html: string; keys: string[] } {
  const items = scanItems(html);
  if (!items || !items.length) return { html, keys: [] };
  const placed: Item[] = [];
  const keys: string[] = [];
  for (const k of order) {
    const it = items.find((i) => i.key === k);
    if (it && !placed.includes(it)) {
      placed.push(it);
      keys.push(k);
    }
  }
  for (const it of items) if (!placed.includes(it)) placed.push(it);
  let out = '';
  let at = 0;
  items.forEach((slot, n) => {
    out += html.slice(at, slot.start) + html.slice(placed[n].start, placed[n].end);
    at = slot.end;
  });
  return { html: out + html.slice(at), keys };
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
        let html = r.html ?? baseline;
        if (r.html === null && Array.isArray(r.order)) {
          const sorted = reorderItems(baseline, r.order);
          html = sorted.html;
          r.rendered?.(sorted.keys);
        }
        controller.enqueue(startMarker + html + END);
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
