// G5 (edge-cache.md, Decision 22): the snapshot's `erased` Bloom filter over the site's erasures,
// as Core builds it (core/src/edge.rs erased_filter). Bits are least significant first within a
// byte; index j < k of a tombstone hash is (h1 + j*h2) mod m, h1 and h2 the hash's first two
// big-endian 32-bit words, h2 with its low bit set. A Bloom filter has no false negatives, so
// `false` means "not erased as of this snapshot"; `true` may be a false positive.

const MAX_K = 32;
const decoded = new WeakMap();

function bitsOf(f) {
  let b = decoded.get(f);
  if (b !== undefined) return b;
  b = null;
  try {
    const s = atob(f.bits);
    if (s.length * 8 === f.m) {
      b = new Uint8Array(s.length);
      for (let i = 0; i < s.length; i++) b[i] = s.charCodeAt(i);
    }
  } catch {}
  decoded.set(f, b);
  return b;
}

/** Whether the tombstone hash `hashHex` (sha256(site 0 subject), hex) is in filter `f`: true,
 * false, or null when there is no usable filter (absent or malformed; the caller reads gone/). */
export function inErased(f, hashHex) {
  if (!f || typeof f !== 'object' || typeof f.bits !== 'string') return null;
  const { n, m, k } = f;
  if (!Number.isInteger(n) || !Number.isInteger(m) || !Number.isInteger(k) || n < 0 || m < 0 || m % 8 !== 0 || k < 1 || k > MAX_K) return null;
  if ((n === 0) !== (m === 0)) return null;
  const bits = bitsOf(f);
  if (!bits) return null;
  if (n === 0) return false;
  if (typeof hashHex !== 'string' || !/^[0-9a-f]{16}/.test(hashHex)) return null;
  const h1 = parseInt(hashHex.slice(0, 8), 16);
  const h2 = (parseInt(hashHex.slice(8, 16), 16) | 1) >>> 0;
  for (let j = 0; j < k; j++) {
    const idx = (h1 + j * h2) % m; // below 2^37: exact in a double
    if (((bits[Math.floor(idx / 8)] >> (idx % 8)) & 1) === 0) return false;
  }
  return true;
}
