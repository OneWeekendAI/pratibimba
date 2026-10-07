// A minimal IAB TCF v2 TC string (core segment only), with purpose 1 granted or not.
export function tcString(purposeOne: boolean, version = 2): string {
  const field = (v: number, n: number) => v.toString(2).padStart(n, '0');
  let bits = field(version, 6) + field(1, 36) + field(1, 36) + field(10, 12) + field(1, 12) + field(1, 6) + field(0, 12) + field(100, 12) + field(4, 6) + '0' + '0' + field(0, 12);
  bits += (purposeOne ? '1' : '0') + '1'.repeat(23); // purposes 1..24: everything but #1 granted
  bits += '0'.repeat(24 + 1 + 16 + 1 + 16 + 1); // legit. interests, publisher CC, vendor sections
  while (bits.length % 6) bits += '0';
  const ALPHA = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  let out = '';
  for (let i = 0; i < bits.length; i += 6) out += ALPHA[parseInt(bits.slice(i, i + 6), 2)];
  return out;
}
