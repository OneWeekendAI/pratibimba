// The random draws Core's LinTS makes, bit for bit: `ChaCha8Rng::seed_from_u64(seed)` (rand_chacha
// 0.9, rand_core 0.9) and rand_distr 0.5's `StandardNormal` (the ZIGNOR ziggurat). A decision
// stores its u64 seed, and replaying it in Core must give the scores the edge served, so this is
// a port, not a lookalike. test/rng.test.mjs holds it to streams Core printed.

import { ZIG_NORM_R, ZIG_NORM_X, ZIG_NORM_F } from './ziggurat.js';

const M64 = (1n << 64n) - 1n;

/** rand_core's seed_from_u64: PCG32 stretched over the 32-byte seed, as little-endian words. */
function seedWords(seed) {
  const MUL = 6364136223846793005n;
  const INC = 11634580027462260723n;
  let state = BigInt.asUintN(64, seed);
  const words = new Uint32Array(8);
  for (let i = 0; i < 8; i++) {
    state = (state * MUL + INC) & M64;
    const xorshifted = Number((((state >> 18n) ^ state) >> 27n) & 0xffffffffn) >>> 0;
    const rot = Number(state >> 59n);
    words[i] = ((xorshifted >>> rot) | (xorshifted << ((32 - rot) & 31))) >>> 0;
  }
  return words;
}

function rotl(v, n) {
  return ((v << n) | (v >>> (32 - n))) >>> 0;
}

/** ChaCha with 8 rounds, a 64-bit block counter and stream 0, buffered four blocks at a time. */
export class ChaCha8 {
  constructor(seed) {
    this.key = seedWords(seed);
    this.counter = 0n;
    this.buf = new Uint32Array(64);
    this.index = 64;
  }

  block(out, offset) {
    const s = new Uint32Array(16);
    s[0] = 0x61707865; s[1] = 0x3320646e; s[2] = 0x79622d32; s[3] = 0x6b206574;
    s.set(this.key, 4);
    s[12] = Number(this.counter & 0xffffffffn);
    s[13] = Number((this.counter >> 32n) & 0xffffffffn);
    s[14] = 0; s[15] = 0;
    const x = Uint32Array.from(s);
    const qr = (a, b, c, d) => {
      x[a] = (x[a] + x[b]) >>> 0; x[d] = rotl(x[d] ^ x[a], 16);
      x[c] = (x[c] + x[d]) >>> 0; x[b] = rotl(x[b] ^ x[c], 12);
      x[a] = (x[a] + x[b]) >>> 0; x[d] = rotl(x[d] ^ x[a], 8);
      x[c] = (x[c] + x[d]) >>> 0; x[b] = rotl(x[b] ^ x[c], 7);
    };
    for (let r = 0; r < 4; r++) {
      qr(0, 4, 8, 12); qr(1, 5, 9, 13); qr(2, 6, 10, 14); qr(3, 7, 11, 15);
      qr(0, 5, 10, 15); qr(1, 6, 11, 12); qr(2, 7, 8, 13); qr(3, 4, 9, 14);
    }
    for (let i = 0; i < 16; i++) out[offset + i] = (x[i] + s[i]) >>> 0;
    this.counter = (this.counter + 1n) & M64;
  }

  refill() {
    for (let b = 0; b < 4; b++) this.block(this.buf, b * 16);
  }

  /** rand_core's BlockRng::next_u64, as `[lo, hi]` u32 halves (low word first, as Rust reads it). */
  nextU64Parts() {
    const len = 64;
    if (this.index < len - 1) {
      const lo = this.buf[this.index];
      const hi = this.buf[this.index + 1];
      this.index += 2;
      return [lo, hi];
    }
    if (this.index >= len) {
      this.refill();
      this.index = 2;
      return [this.buf[0], this.buf[1]];
    }
    const lo = this.buf[len - 1];
    this.refill();
    this.index = 1;
    return [lo, this.buf[0]];
  }

  nextU64() {
    const [lo, hi] = this.nextU64Parts();
    return (BigInt(hi) << 32n) | BigInt(lo);
  }

  /** rand's StandardUniform f64: the top 53 bits times 2^-53, in [0, 1). */
  uniform() {
    const [lo, hi] = this.nextU64Parts();
    // (hi:lo) >> 11 as an exact integer below 2^53.
    const v = hi * 2097152 + (lo >>> 11); // hi * 2^21 + lo >> 11
    return v * 1.1102230246251565e-16; // 2^-53
  }

  /** rand's Open01 f64: 52 random bits as the fraction of [1, 2), minus (1 - eps/2). */
  open01() {
    const [lo, hi] = this.nextU64Parts();
    return floatFromFraction(hi >>> 12, ((lo >>> 12) | ((hi & 0xfff) << 20)) >>> 0, 0) - (1.0 - Number.EPSILON / 2);
  }

  /** rand_distr's StandardNormal (ziggurat, symmetric). */
  normal() {
    for (;;) {
      const [lo, hi] = this.nextU64Parts();
      const i = lo & 0xff;
      // (bits >> 12) with exponent 1 is a value in [2, 4); minus 3 is [-1, 1).
      const u = floatFromFraction(hi >>> 12, ((lo >>> 12) | ((hi & 0xfff) << 20)) >>> 0, 1) - 3.0;
      const x = u * ZIG_NORM_X[i];
      if (Math.abs(x) < ZIG_NORM_X[i + 1]) return x;
      if (i === 0) return zeroCase(this, u);
      if (ZIG_NORM_F[i + 1] + (ZIG_NORM_F[i] - ZIG_NORM_F[i + 1]) * this.uniform() < Math.exp((-x * x) / 2.0)) return x;
    }
  }
}

function zeroCase(rng, u) {
  let x = 1.0;
  let y = 0.0;
  while (-2.0 * y < x * x) {
    const x_ = rng.open01();
    const y_ = rng.open01();
    x = Math.log(x_) / ZIG_NORM_R;
    y = Math.log(y_);
  }
  return u < 0.0 ? x - ZIG_NORM_R : ZIG_NORM_R - x;
}

const scratch = new DataView(new ArrayBuffer(8));

/** An f64 from a 52-bit fraction (20 high bits, 32 low bits) and an unbiased exponent. */
function floatFromFraction(fracHi20, fracLo32, exponent) {
  scratch.setUint32(0, (((1023 + exponent) << 20) | fracHi20) >>> 0);
  scratch.setUint32(4, fracLo32 >>> 0);
  return scratch.getFloat64(0);
}
