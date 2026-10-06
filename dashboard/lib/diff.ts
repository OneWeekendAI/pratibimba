// A token-level diff of two HTML fragments: tags are tokens, words are tokens. Small and exact
// (LCS); drafts are short, so quadratic is fine.

export type Part = { kind: 'same' | 'add' | 'del'; text: string };

export function tokens(s: string): string[] {
  return s.match(/<[^>]*>|\s+|[^<\s]+/g) ?? [];
}

export function diff(before: string, after: string): Part[] {
  const a = tokens(before);
  const b = tokens(after);
  const n = a.length;
  const m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  const out: Part[] = [];
  const push = (kind: Part['kind'], text: string) => {
    const last = out[out.length - 1];
    if (last && last.kind === kind) last.text += text;
    else out.push({ kind, text });
  };
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      push('same', a[i]);
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) push('del', a[i++]);
    else push('add', b[j++]);
  }
  while (i < n) push('del', a[i++]);
  while (j < m) push('add', b[j++]);
  return out;
}

// The code diff: line by line, for a variant that is a component (or any multi-line source).
// A one-line HTML fragment is broken after each closing tag first, so both sides compare as code.

export type Line = { kind: 'same' | 'add' | 'del'; text: string; a: number | null; b: number | null };

/** Multi-line, or written like a module: treat it as code, not as an inline payload. */
export function looksLikeCode(s: string): boolean {
  return /^\s*(import |export |'use client'|"use client")/m.test(s) || s.trim().split('\n').length >= 3;
}

export function codeLines(s: string): string[] {
  if (s.includes('\n')) return s.replace(/\n$/, '').split('\n');
  return s.split(/(?<=<\/[a-zA-Z][^>]*>)/).map((l) => l.trim()).filter((l) => l.length > 0);
}

export function lineDiff(before: string, after: string): Line[] {
  const a = codeLines(before);
  const b = codeLines(after);
  const n = a.length;
  const m = b.length;
  // Indentation is layout, not content: compare trimmed, show the draft's own text.
  const eq = (x: string, y: string) => x.trim() === y.trim();
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) lcs[i][j] = eq(a[i], b[j]) ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  const out: Line[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (eq(a[i], b[j])) out.push({ kind: 'same', text: b[j], a: ++i, b: ++j });
    else if (lcs[i + 1][j] >= lcs[i][j + 1]) out.push({ kind: 'del', text: a[i], a: ++i, b: null });
    else out.push({ kind: 'add', text: b[j], a: null, b: ++j });
  }
  while (i < n) out.push({ kind: 'del', text: a[i], a: ++i, b: null });
  while (j < m) out.push({ kind: 'add', text: b[j], a: null, b: ++j });
  return out;
}
