// Core's stable bucketing (core/src/hash.rs): FNV-1a 64 over byte parts with a separator, then
// MurmurHash3's finaliser, then the top 53 bits as a fraction. Holdout, traffic share and
// experiment arms all hash through here, so a visitor lands in the same bucket at the edge and
// in Core.

const M64 = (1n << 64n) - 1n;
const OFFSET = 0xcbf29ce484222325n;
const PRIME = 0x00000100000001b3n;
const enc = new TextEncoder();

export function fnv1a64(parts) {
  let h = OFFSET;
  parts.forEach((part, i) => {
    if (i > 0) {
      h ^= 0xffn;
      h = (h * PRIME) & M64;
    }
    for (const b of part) {
      h ^= BigInt(b);
      h = (h * PRIME) & M64;
    }
  });
  return h;
}

export function fmix64(k) {
  k ^= k >> 33n;
  k = (k * 0xff51afd7ed558ccdn) & M64;
  k ^= k >> 33n;
  k = (k * 0xc4ceb9fe1a85ec53n) & M64;
  k ^= k >> 33n;
  return k;
}

/** A point in [0, 1). */
export function unit(parts) {
  return Number(fmix64(fnv1a64(parts)) >> 11n) / 9007199254740992; // 2^53
}

/** An i64 as Rust's `to_le_bytes`. */
export function leBytes(id) {
  const out = new Uint8Array(8);
  let v = BigInt.asUintN(64, BigInt(id));
  for (let i = 0; i < 8; i++) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

export const bytes = (s) => enc.encode(s);

export function inHoldout(site, subject, fraction) {
  return unit([bytes('holdout'), bytes(site), bytes(subject)]) < fraction;
}

export function withinShare(variantId, subject, share) {
  return unit([bytes('share'), leBytes(variantId), bytes(subject)]) < share;
}

/** core/src/experiments.rs `assign`: `arms` is `[[variant_id, weight]]`. */
export function assign(experiment, subject, arms) {
  let total = -0.0;
  for (const a of arms) total += a[1];
  const u = unit([bytes('experiment'), leBytes(experiment), bytes(subject)]) * total;
  let acc = 0.0;
  for (const [id, w] of arms) {
    acc += w;
    if (u < acc) return id;
  }
  return arms[arms.length - 1][0];
}
