// `scan`: find places in a repo that could become Pratibimba slots.
//
// Deterministic heuristics over JSX/TSX and plain HTML, no model call. The output is a ranked
// list of candidates (file:line, the slot kind, a suggested audience and goal, and why), plus
// the slots the repo already has. Judging which candidates are worth a slot, and writing the
// variants, is the coding agent's job; this only points.
//
// Nothing is written unless `writeDrafts` is called (the CLI's --write). It writes
// .pratibimba/slot-drafts.json, never the site file. Each draft carries the exact `create_slot`
// arguments for its kind (swap, copy or sorted); the slot still lands as a draft over MCP and a
// person approves its baseline.

import { readFile, readdir, writeFile, mkdir } from 'node:fs/promises';
import { join, relative, sep, dirname } from 'node:path';

export const DRAFTS_PATH = '.pratibimba/slot-drafts.json';
const EXTS = /\.(tsx|jsx|html?)$/;
const SKIP_DIRS = new Set(['node_modules', '.next', '.git', 'dist', 'build', 'out', 'coverage', 'test', 'tests',
  '__tests__', 'e2e', 'test-results', '.pratibimba', '.claude', '.cursor', '.vercel', '.turbo']);
const SKIP_FILES = /\.(test|spec|stories)\.[jt]sx?$/;

// Kind order is also the dedupe priority: an element inside a chosen candidate is not offered again.
const KINDS = {
  hero: { score: 90 },
  grid: { score: 80 },
  returning: { score: 70 },
  promo: { score: 65 },
  cta: { score: 55 },
  headline: { score: 45 },
};
const HOME = /^(src\/)?(app\/page|pages\/index)\.[jt]sx$|^(public\/)?index\.html?$/;

const HERO_TOKENS = /^(hero|jumbotron|masthead|splash)([-_].*)?$/i;
const GRID_TOKENS = /^(grid|products?|product-list|cards|catalog(ue)?|items|listing|collection)([-_].*)?$/i;
const PROMO_TOKENS = /^(promo|banner|announcement|announce|offer|deal|sale|notice|ribbon)([-_].*)?$/i;
const CTA_TOKENS = /^(cta|btn|button|btn-primary|primary)([-_].*)?$/i;
const NAV_TOKENS = /^(nav|navbar|menu|breadcrumbs?|footer|pagination|tabs)([-_].*)?$/i;
const PROMO_TEXT = /free (shipping|delivery)|\d+\s?% off|\bsale\b|discount|limited time|this week|today only|use code|\boffer\b/i;
const RETURNING_TEXT = /welcome back|recently viewed|continue (shopping|where)|pick up where|order again|\breorder\b|buy again|your (past )?(orders|favou?rites|wishlist)|for returning/i;
const RETURNING_TOKENS = /^(welcome-back|recently-viewed|returning|reorder|buy-again)([-_].*)?$/i;
const CTA_TEXT = /^(shop|buy|get|start|try|sign up|join|book|order|subscribe|add to (cart|bag|basket)|learn more|explore|discover|see|view|claim)\b/i;
const CONTAINERS = new Set(['section', 'div', 'header', 'aside', 'article', 'main', 'p', 'span', 'strong', 'ul', 'ol']);

/** Scan `dir`. Returns `{ candidates, existing }`, both sorted deterministically. */
export async function scan({ dir, limit } = {}) {
  const files = (await walk(dir)).sort();
  const parsed = [];
  for (const abs of files) {
    const rel = relative(dir, abs).split(sep).join('/');
    const text = await readFile(abs, 'utf8');
    parsed.push({ rel, text, html: /\.html?$/.test(rel), ...parse(text) });
  }
  const site = await readSite(dir);

  // Pass 1: what is already a slot, and which components render inside one.
  const existing = [];
  const coveredNames = new Set();
  for (const f of parsed) {
    f.covered = [];
    for (const el of f.els) {
      const prb = /^prb\.(Slot|Sorted|Copy|Text)$/.exec(el.name);
      const dataKey = attr(el.attrs, 'data-prb-slot');
      const commentKey = f.html ? f.comments.find((c) => c.before === el.start)?.key : undefined;
      const siteKey = f.html ? site.find((s) => matches(s.selector, el))?.key : undefined;
      const key = prb ? attr(el.attrs, 'id') : dataKey ?? commentKey ?? siteKey;
      if (!key) continue;
      const kind = prb ? { Slot: 'swap', Sorted: 'sorted', Copy: 'copy', Text: 'copy' }[prb[1]] : 'swap';
      existing.push({ key, kind, file: f.rel, line: el.line });
      f.covered.push([el.start, el.end]);
      for (const m of f.text.slice(el.start, el.end).matchAll(/<([A-Z]\w*)\b/g)) coveredNames.add(m[1]);
    }
  }
  for (const f of parsed) {
    const exported = [...f.text.matchAll(/export\s+(?:default\s+)?(?:function|const|class)\s+([A-Z]\w*)/g)].map((m) => m[1]);
    if (exported.some((n) => coveredNames.has(n))) f.covered.push([0, f.text.length]);
  }
  const takenKeys = new Set([...existing.map((e) => e.key), ...site.map((s) => s.key)]);

  // Pass 2: candidates outside every covered range.
  const candidates = [];
  for (const f of parsed) {
    const chosen = [];
    const inside = (el, ranges) => ranges.some(([a, b]) => el.start >= a && el.start < b);
    for (const el of f.els) {
      if (inside(el, f.covered) || inside(el, chosen)) continue;
      // A container that already holds a slot is not offered: the slot inside it is the unit.
      if (f.covered.some(([a]) => a > el.start && a < el.end)) continue;
      if (ancestors(el).some((a) => a.name === 'nav' || a.name === 'footer' || tokens(a).some((t) => NAV_TOKENS.test(t)))) continue;
      const c = classify(el, f);
      if (!c) continue;
      chosen.push([el.start, el.end]);
      candidates.push({ file: f.rel, line: el.line, home: HOME.test(f.rel), el, ...c });
    }
  }
  for (const c of candidates) c.score = KINDS[c.type].score + (c.home ? 5 : 0);
  candidates.sort((a, b) => b.score - a.score || a.file.localeCompare(b.file) || a.line - b.line);

  const out = candidates.slice(0, limit ?? candidates.length).map((c, i) => {
    const key = unique(c.key, takenKeys);
    const id = attr(c.el.attrs, 'id');
    return {
      rank: i + 1,
      file: c.file,
      line: c.line,
      kind: c.kind,
      key,
      selector: id ? `#${id}` : `[data-prb-slot=${key}]`,
      ...(c.fields ? { fields: c.fields } : {}),
      metric: c.metric ?? 'click',
      audience: c.audience,
      goal: c.goal,
      why: c.why,
      excerpt: excerpt(c.el, f2(parsed, c.file)),
      mark: mark(c, key, id, c.file),
    };
  });
  existing.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line);
  return { candidates: out, existing };
}

function classify(el, f) {
  const toks = tokens(el);
  const inner = f.text.slice(el.openEnd, el.closeStart ?? el.openEnd);
  const lit = literal(inner);
  const has = (re) => toks.some((t) => re.test(t));
  const isContainer = CONTAINERS.has(el.name);
  // Text rules apply only to a leaf-ish element, so a page wrapper is not called a promo.
  const leaf = !el.children.some((k) => CONTAINERS.has(k.name) || /^[A-Z]|^prb\./.test(k.name));

  // A hero: a marked hero container, or a section/header that holds an h1 and a call to action.
  const kids = descendants(el);
  const h1 = kids.find((k) => k.name === 'h1');
  const action = kids.find((k) => isAction(k, f));
  if ((isContainer && has(HERO_TOKENS)) || (['section', 'header'].includes(el.name) && h1 && action)) {
    const fields = {};
    if (h1 && literalOnly(h1, f)) fields.headline = literal(text(h1, f));
    const p = kids.find((k) => k.name === 'p' && literalOnly(k, f));
    if (p) fields.subhead = literal(text(p, f));
    if (action && literalOnly(action, f)) fields.cta_label = literal(text(action, f));
    const copy = fields.headline !== undefined;
    return {
      type: 'hero', kind: copy ? 'copy' : 'swap', key: 'hero', ...(copy ? { fields: copyFields(fields) } : {}),
      audience: 'everyone first; then new vs returning visitors', goal: 'conversion',
      why: copy
        ? 'the first thing a visitor reads, with static words: a copy slot lets an agent test headline, subhead and button label as plain text'
        : 'the first thing a visitor sees, built from dynamic content: a swap slot lets variants change the markup',
    };
  }

  // A sorted list: the innermost element that renders `.map(` of keyed elements, or (HTML) 3+
  // same-tag children that look like cards.
  if (!f.html && isContainer && /\.map\(/.test(inner) && !kids.some((k) => k.closeStart !== undefined && CONTAINERS.has(k.name) && /\.map\(/.test(f.text.slice(k.openEnd, k.closeStart)))) {
    const body = inner.slice(inner.indexOf('.map('));
    if (/key=/.test(body) && /<[a-z]/.test(body)) {
      const priced = /price|₹|\$|€|£/i.test(body);
      return {
        type: 'grid', kind: 'sorted', key: toks.find((t) => GRID_TOKENS.test(t))?.toLowerCase() ?? 'list',
        audience: 'everyone; the order is learned from visitor features', goal: priced ? 'revenue' : 'conversion',
        why: `a list rendered from data${priced ? ' with prices' : ''}: a sorted slot learns which order gets clicked, with each item's key as its id`,
      };
    }
  }
  if (f.html && isContainer) {
    const direct = el.children.filter((k) => k.closeStart !== undefined);
    const byTag = {};
    for (const k of direct) byTag[k.name] = (byTag[k.name] ?? 0) + 1;
    const [tag, n] = Object.entries(byTag).sort((a, b) => b[1] - a[1])[0] ?? [];
    if (n >= 3 && direct.filter((k) => k.name === tag).every((k) => tokens(k).some((t) => /card|product|item|tile/i.test(t)))) {
      return {
        type: 'grid', kind: 'sorted', key: toks.find((t) => GRID_TOKENS.test(t))?.toLowerCase() ?? 'list',
        audience: 'everyone; the order is learned from visitor features', goal: 'conversion',
        why: `${n} repeated <${tag}> cards: a sorted slot learns which order gets clicked; give each card a stable item key`,
      };
    }
  }

  // A spot for returning visitors.
  if (isContainer && (has(RETURNING_TOKENS) || (leaf && RETURNING_TEXT.test(lit)))) {
    return {
      type: 'returning', kind: 'swap', key: 'returning',
      audience: 'returning visitors (a history audience, e.g. viewed an item in 30 days)', goal: 'conversion',
      why: 'speaks to a visitor who has been here before: gate a variant on a history audience and keep this as the baseline for everyone else',
    };
  }

  // A promo banner.
  if (isContainer && (has(PROMO_TOKENS) || (leaf && PROMO_TEXT.test(lit) && lit.length <= 160))) {
    return {
      type: 'promo', kind: 'swap', key: 'promo',
      audience: 'everyone', goal: 'revenue',
      why: 'an offer line: an offer moves order value, so judge its variants on revenue, not clicks',
    };
  }

  // A call to action with static words.
  if (isAction(el, f) && literalOnly(el, f)) {
    const label = literal(text(el, f));
    return {
      type: 'cta', kind: 'copy', key: `${slug(label)}_cta`, fields: copyFields({ cta_label: label }),
      audience: 'everyone', goal: 'conversion',
      why: `a call to action ("${label}"): a copy slot tests the button's words without touching its behaviour`,
    };
  }

  // A headline with static words.
  if (el.name === 'h1' && literalOnly(el, f)) {
    const words = literal(text(el, f));
    return {
      type: 'headline', kind: 'copy', key: `${slug(words)}_headline`, fields: copyFields({ headline: words }),
      audience: 'new visitors', goal: 'conversion',
      why: 'a page headline with static words: a copy slot tests the headline as plain text',
    };
  }
  return null;
}

// ---- parsing: a small tag scanner, enough for formatted JSX and HTML ----

function parse(src) {
  const comments = [];
  // Blank out comments (keeping offsets and newlines), remembering `prb:slot` markers.
  let text = src.replace(/<!--([\s\S]*?)-->/g, (m, body, at) => {
    const k = /prb:slot\s*=\s*"([^"]+)"/.exec(body);
    if (k) comments.push({ key: k[1], at: at + m.length });
    return m.replace(/[^\n]/g, ' ');
  });
  text = text.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, ' '));
  const lineStarts = [0];
  for (let i = 0; i < text.length; i++) if (text[i] === '\n') lineStarts.push(i + 1);
  const lineOf = (i) => {
    let lo = 0, hi = lineStarts.length - 1;
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (lineStarts[mid] <= i) lo = mid; else hi = mid - 1; }
    return lo + 1;
  };

  const tags = [];
  for (let i = 0; i < text.length; i++) {
    if (text[i] !== '<') continue;
    const close = text[i + 1] === '/';
    const m = /^[A-Za-z][\w.:-]*/.exec(text.slice(i + (close ? 2 : 1), i + 80));
    if (!m) continue;
    let j = i + (close ? 2 : 1) + m[0].length, depth = 0, q = null;
    for (; j < text.length; j++) {
      const ch = text[j];
      if (q) { if (ch === q) q = null; continue; }
      if (ch === '"' || ch === "'" || ch === '`') q = ch;
      else if (ch === '{') depth++;
      else if (ch === '}') depth--;
      else if (ch === '>' && depth <= 0) break;
      else if (ch === '<' && depth <= 0 && !close) break; // not a tag after all (a generic, a comparison)
    }
    if (text[j] !== '>') continue;
    const attrs = text.slice(i + 1 + m[0].length, j);
    tags.push({ name: m[0], close, self: !close && /\/\s*$/.test(attrs), start: i, openEnd: j + 1, attrs });
    i = j;
  }

  // Match each opening tag to its closing tag with a stack per name; void HTML tags never close.
  const VOID = /^(area|base|br|col|embed|hr|img|input|link|meta|source|track|wbr)$/i;
  const els = [];
  const stacks = {};
  for (const t of tags) {
    if (t.close) {
      const s = stacks[t.name];
      const el = s?.pop();
      if (el) { el.closeStart = t.start; el.end = t.openEnd; }
      continue;
    }
    const el = { name: t.name, attrs: t.attrs, start: t.start, openEnd: t.openEnd, end: t.openEnd, line: lineOf(t.start), children: [] };
    els.push(el);
    if (!t.self && !VOID.test(t.name)) (stacks[t.name] ??= []).push(el);
  }
  // Parents: the nearest enclosing element.
  const open = [];
  for (const el of els) {
    while (open.length && open[open.length - 1].end <= el.start) open.pop();
    el.parent = open[open.length - 1];
    el.parent?.children.push(el);
    if (el.closeStart !== undefined) open.push(el);
  }
  // A prb:slot comment marks the next element.
  for (const c of comments) c.before = els.find((e) => e.start >= c.at)?.start;
  return { text, els, comments };
}

function attr(attrs, name) {
  const m = new RegExp(`(?:^|\\s)${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|\\{\\s*["'\`]([^"'\`]*)["'\`]\\s*\\})`).exec(attrs);
  return m ? (m[1] ?? m[2] ?? m[3]) : undefined;
}
function tokens(el) {
  return [...(attr(el.attrs, 'className') ?? attr(el.attrs, 'class') ?? '').split(/\s+/), attr(el.attrs, 'id') ?? ''].filter(Boolean);
}
function ancestors(el) {
  const out = [];
  for (let p = el.parent; p; p = p.parent) out.push(p);
  return out;
}
function descendants(el) {
  const out = [];
  const go = (e) => { for (const k of e.children) { out.push(k); go(k); } };
  go(el);
  return out;
}
const text = (el, f) => f.text.slice(el.openEnd, el.closeStart ?? el.openEnd);
/** Visible static text: tags and `{...}` expressions removed, whitespace collapsed. */
function literal(s) {
  let t = s.replace(/<[^>]*>/g, ' ');
  for (let prev; prev !== t;) { prev = t; t = t.replace(/\{[^{}]*\}/g, ' '); }
  return t.replace(/\s+/g, ' ').trim();
}
/** True when the element's words are all static: no `{expr}` other than whitespace or a string. */
function literalOnly(el, f) {
  if (el.closeStart === undefined) return false;
  const inner = text(el, f).replace(/\{\s*(["'`])[^"'`]*\1\s*\}/g, '');
  return !/[{}]/.test(inner.replace(/<[^>]*>/g, '')) && literal(inner).length > 0;
}
function isAction(el, f) {
  if (el.name !== 'a' && el.name !== 'button') return false;
  if (tokens(el).some((t) => CTA_TOKENS.test(t))) return true;
  return el.closeStart !== undefined && CTA_TEXT.test(literal(text(el, f)));
}
function matches(selector, el) {
  if (!selector) return false;
  let m;
  if ((m = /^#([\w-]+)$/.exec(selector))) return attr(el.attrs, 'id') === m[1];
  if ((m = /^\.([\w-]+)$/.exec(selector))) return tokens(el).includes(m[1]);
  if ((m = /^\[data-prb-slot=["']?([\w-]+)["']?\]$/.exec(selector))) return attr(el.attrs, 'data-prb-slot') === m[1];
  return false;
}
function copyFields(fields) {
  return Object.entries(fields).map(([name, value]) => ({ name, max: Math.min(500, Math.max(40, Math.ceil((value.length * 1.5) / 10) * 10)), required: true, baseline: value }));
}
function slug(s) {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '').split('_').slice(0, 3).join('_') || 'text';
}
function unique(base, taken) {
  let k = base, n = 2;
  while (taken.has(k)) k = `${base}-${n++}`;
  taken.add(k);
  return k;
}
const f2 = (parsed, rel) => parsed.find((f) => f.rel === rel);
function excerpt(el, f) {
  const line = f.text.slice(el.start).split('\n')[0].trim();
  return line.length > 100 ? `${line.slice(0, 97)}...` : line;
}
function mark(c, key, id, file) {
  if (/\.html?$/.test(file)) return id ? `selector #${id} (already unique)` : `add data-prb-slot="${key}" to the element`;
  if (c.kind === 'sorted') return `wrap the mapped items in <prb.Sorted id="${key}">, each with its item key`;
  if (c.kind === 'copy') return `render the words through <prb.Copy id="${key}" fallback={...}>`;
  return `wrap it in <prb.Slot id="${key}">; the current markup is the baseline`;
}

async function walk(dir) {
  const out = [];
  for (const e of await readdir(dir, { withFileTypes: true })) {
    if (e.isDirectory()) {
      if (!SKIP_DIRS.has(e.name)) out.push(...(await walk(join(dir, e.name))));
    } else if (EXTS.test(e.name) && !SKIP_FILES.test(e.name)) out.push(join(dir, e.name));
  }
  return out;
}
async function readSite(dir) {
  try {
    const doc = JSON.parse(await readFile(join(dir, 'site.json'), 'utf8'));
    return Array.isArray(doc.slots) ? doc.slots.filter((s) => s && typeof s.key === 'string') : [];
  } catch {
    return [];
  }
}

// ---- output ----

export function formatText({ candidates, existing }) {
  const lines = [];
  if (existing.length) {
    lines.push('Already slots:');
    for (const e of existing) lines.push(`  ${e.key.padEnd(14)} ${e.kind.padEnd(7)} ${e.file}:${e.line}`);
    lines.push('');
  }
  if (!candidates.length) {
    lines.push('No candidate slots found outside the existing ones.');
    return lines.join('\n');
  }
  lines.push('Candidate slots, best first:');
  for (const c of candidates) {
    lines.push(`${String(c.rank).padStart(3)}. ${c.file}:${c.line}  ${c.kind}  key ${c.key}`);
    lines.push(`     ${c.excerpt}`);
    if (c.fields) lines.push(`     fields: ${c.fields.map((x) => `${x.name} (max ${x.max}) "${x.baseline}"`).join(', ')}`);
    lines.push(`     audience: ${c.audience}; goal: ${c.goal}`);
    lines.push(`     why: ${c.why}`);
    lines.push(`     mark: ${c.mark}`);
  }
  return lines.join('\n');
}

/**
 * The MCP `create_slot` arguments for a candidate. A copy slot's fields go without their
 * baseline text (the server's copy spec refuses unknown keys). A sorted slot's items are not
 * guessed: scan cannot see the data the list is rendered from.
 */
export function createSlotArgs(c) {
  const args = { key: c.key, selector: c.selector, metric: c.metric, kind: c.kind };
  if (c.kind === 'copy') args.copy = { fields: c.fields.map(({ name, max, required }) => ({ name, max, required })) };
  return args;
}

/**
 * Add candidates to .pratibimba/slot-drafts.json as drafts. A key already in the file is kept
 * as it is (the developer may have edited it). Returns `{ path, added, kept }`.
 */
export async function writeDrafts({ dir, candidates }) {
  const path = join(dir, DRAFTS_PATH);
  let doc = { drafts: [] };
  try {
    doc = JSON.parse(await readFile(path, 'utf8'));
    if (!Array.isArray(doc.drafts)) throw new Error(`${DRAFTS_PATH} has no "drafts" array`);
  } catch (e) {
    if (e.code !== 'ENOENT') throw new Error(`${DRAFTS_PATH} is not a drafts file we can update: ${e.message}`);
  }
  const have = new Set(doc.drafts.map((d) => d.key));
  let added = 0;
  for (const c of candidates) {
    if (have.has(c.key)) continue;
    const { rank, excerpt: _e, ...rest } = c;
    const needs = c.kind === 'sorted' ? 'items: add the item keys, in the default order, to create_slot.items before calling it' : undefined;
    doc.drafts.push({ status: 'draft', ...rest, create_slot: createSlotArgs(c), ...(needs ? { needs } : {}) });
    have.add(c.key);
    added++;
  }
  doc = {
    note: 'Slot drafts from `pratibimba-agent scan --write`. None of these is live: every kind through create_slot (pass each draft\'s create_slot arguments, adding a sorted draft\'s items), which makes a draft slot, and a person approves each baseline.',
    drafts: doc.drafts,
  };
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(doc, null, 2)}\n`);
  return { path: DRAFTS_PATH, added, kept: candidates.length - added };
}
