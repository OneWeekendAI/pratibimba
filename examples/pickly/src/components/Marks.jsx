/*
 * Food marks, drawn for the pantry.
 *   VegMark({ veg })        small inline wax dot (cards, cart rows). Same props as v1.
 *   SpiceLevel({ level })   five small chillies, lit up to level. Same props as v1.
 *   WaxSeal({ veg, size })  the big pressed seal on the product page (presses in on mount).
 *   SpiceGauge({ level })   the brass chilli thermometer on the product page.
 */

const CHILLI_BODY =
  'M8.3 6.9c-2.3 2.4-3.6 6-3.5 9.6.05 2.2 1 3.6 2.4 3.9 3.1.6 8.7-2.4 11.9-6.6.4-.55-1.3.9-2.4.6-1.8-.5-2.9-1.5-3.2-3.1-.3-2.3-2-3.7-4.2-3.3z';
const CHILLI_STEM = 'M8.4 6.8c-.3-1.6.4-3 1.9-3.6M10.3 3.2c.7-.3 1.4-.25 2 .1';

/* Leaf and drumstick, same drawings as icons.jsx, so the seal's emboss matches the set. */
const LEAF_PATHS = [
  'M4.3 19.8C4 12.6 8.6 5.3 19.6 4.2c.7 9.6-5 15.2-12.3 15.4',
  'M3.4 20.7c3.6-4.2 7.2-7.9 11.4-11.6',
  'M8.6 15.6l-.3-3.2M11.2 13.1l-.1-3.2M13.6 10.9l3.2.1M10.7 16.1l3.3.2',
];
// Drumstick: the meat oval, the bone, its two knobs (icons.jsx, redrawn), knobs as closed paths.
const knob = (cx, cy, r) => `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0`;
const DRUM_PATHS = [
  'M12.9 13.4c-1.6 1.7-4.6 1.9-6.6.2C4 11.6 3.6 8 5.6 5.7 7.7 3.4 11.6 3.5 13.5 5.9c1.5 1.9 1.6 4.6.6 6.3Z',
  'M12.8 13.3l4 4.1',
  knob(18.6, 17.1, 1.45),
  knob(16.7, 19.2, 1.45),
];
// seal version: a longer bone so the knobs sit clear of the meat at seal size
const SEAL_DRUM_PATHS = [
  'M11.6 12.4c-1.5 1.6-4.2 1.7-6-.1-2-1.8-2.3-4.9-.6-7 1.9-2.1 5.4-2 7.1.1 1.3 1.7 1.4 4.1.5 5.6Z',
  'M11.4 12.2l6.4 6.4',
  knob(20, 18.1, 1.75),
  knob(17.4, 20.7, 1.75),
];

/* A wax blob: a circle pushed out unevenly where the wax spread. */
const BLOB_R = [44, 45.5, 44.2, 46.8, 45, 43.6, 44.8, 47.5, 45.2, 44, 43.2, 45.6, 44.4, 46.2, 44.9, 43.8, 45.1, 48.6, 45.3, 44.1, 43.9, 45.8, 44.6, 45];
function blobPath(cx, cy, scale = 1) {
  const n = BLOB_R.length;
  const pts = BLOB_R.map((r, i) => {
    const a = (i / n) * Math.PI * 2 - Math.PI / 2;
    return [cx + Math.cos(a) * r * scale, cy + Math.sin(a) * r * scale];
  });
  let d = `M${pts[0][0].toFixed(1)} ${pts[0][1].toFixed(1)}`;
  for (let i = 0; i < n; i++) {
    const p0 = pts[(i - 1 + n) % n], p1 = pts[i], p2 = pts[(i + 1) % n], p3 = pts[(i + 2) % n];
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    d += `C${c1[0].toFixed(1)} ${c1[1].toFixed(1)} ${c2[0].toFixed(1)} ${c2[1].toFixed(1)} ${p2[0].toFixed(1)} ${p2[1].toFixed(1)}`;
  }
  return d + 'Z';
}
const BLOB = blobPath(50, 50);
const BLOB_SMALL = blobPath(8, 8, 0.165);

function waxTones(veg) {
  return veg
    ? { base: 'var(--wax-green)', light: 'var(--wax-green-light)', deep: '#1d3a21' }
    : { base: 'var(--wax-red)', light: 'var(--wax-red-light)', deep: '#5e140d' };
}

export function VegMark({ veg }) {
  const t = waxTones(veg);
  const label = veg ? 'Vegetarian' : 'Non-Vegetarian';
  const paths = veg ? LEAF_PATHS : DRUM_PATHS;
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      role="img"
      aria-label={label}
      className="veg-mark"
      style={{ display: 'inline-block', verticalAlign: 'middle', flexShrink: 0, overflow: 'visible' }}
    >
      <path d={BLOB_SMALL} fill={t.base} />
      <path d={BLOB_SMALL} fill="none" stroke={t.deep} strokeOpacity=".5" strokeWidth=".8" />
      <g transform="translate(3.4 3.4) scale(.38)" fill="none" stroke="#fff" strokeOpacity=".85" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round">
        {paths.slice(0, 2).map((d) => <path key={d} d={d} />)}
      </g>
    </svg>
  );
}

export function SpiceLevel({ level = 1 }) {
  const count = Math.max(1, Math.min(5, level));
  return (
    <span
      className="spice-level"
      role="img"
      aria-label={`Spice level ${count} of 5`}
      title={`Spice level ${count} of 5`}
      style={{ display: 'inline-flex', gap: 1, alignItems: 'center' }}
    >
      {[0, 1, 2, 3, 4].map((i) => (
        <svg key={i} width="13" height="13" viewBox="0 0 24 24" aria-hidden="true" style={{ display: 'block', transform: `rotate(${i % 2 ? 6 : -4}deg)` }}>
          <path d={CHILLI_BODY} fill={i < count ? 'var(--chili)' : 'none'} stroke={i < count ? 'var(--chili)' : 'var(--ink-faint)'} strokeOpacity={i < count ? 1 : 0.55} strokeWidth="1.6" strokeLinejoin="round" />
          <path d={CHILLI_STEM} fill="none" stroke={i < count ? 'var(--leaf)' : 'var(--ink-faint)'} strokeOpacity={i < count ? 1 : 0.55} strokeWidth="1.7" strokeLinecap="round" />
        </svg>
      ))}
    </span>
  );
}

/* The product page's pressed seal. Visible small-caps label sits beside it (in the page). */
export function WaxSeal({ veg, size = 84, className = '' }) {
  const t = waxTones(veg);
  const paths = veg ? LEAF_PATHS : SEAL_DRUM_PATHS;
  const gid = veg ? 'pp-wax-veg' : 'pp-wax-nonveg';
  // the drumstick is drawn bigger than the leaf and filled, so it reads as meat on a bone
  const art = veg ? 'translate(30 30) scale(1.67)' : 'translate(22 22.5) scale(2.25) rotate(-6 12 12)';
  return (
    <span className={`pp-seal ${className}`} style={{ width: size, height: size }}>
      <svg viewBox="0 0 100 100" width={size} height={size} aria-hidden="true" focusable="false">
        <defs>
          <radialGradient id={gid} cx="38%" cy="32%" r="75%">
            <stop offset="0" stopColor={t.light} />
            <stop offset=".55" stopColor={t.base} />
            <stop offset="1" stopColor={t.deep} />
          </radialGradient>
        </defs>
        <path d={BLOB} fill={t.deep} transform="translate(1.5 2.5)" opacity=".55" />
        <path d={BLOB} fill={`url(#${gid})`} />
        <circle cx="50" cy="50" r="31" fill={t.deep} opacity=".28" />
        <circle cx="50" cy="50" r="31" fill="none" stroke="#fff" strokeOpacity=".22" strokeWidth="1.4" transform="translate(-.8 -.8)" />
        <circle cx="50" cy="50" r="31" fill="none" stroke={t.deep} strokeOpacity=".7" strokeWidth="1.4" transform="translate(.6 .9)" />
        <circle cx="50" cy="50" r="36.5" fill="none" stroke={t.deep} strokeOpacity=".45" strokeWidth="1" strokeDasharray="1.2 3.1" />
        <g transform={art} fill="none" strokeLinecap="round" strokeLinejoin="round">
          <g stroke={t.deep} strokeOpacity=".85" strokeWidth="2.4" transform="translate(.55 .7)">
            {paths.map((d) => <path key={d} d={d} />)}
          </g>
          <g stroke="#fff" strokeOpacity=".38" strokeWidth="2.2" transform="translate(-.45 -.5)">
            {paths.map((d) => <path key={d} d={d} />)}
          </g>
          {!veg && [0, 2, 3].map((i) => <path key={i} d={SEAL_DRUM_PATHS[i]} fill={t.light} fillOpacity=".92" stroke="none" />)}
          <g stroke={t.light} strokeWidth="1.9">
            {paths.map((d) => <path key={d} d={d} />)}
          </g>
        </g>
        <path d="M26 30c5-8 13-12 22-12" fill="none" stroke="#fff" strokeOpacity=".32" strokeWidth="3" strokeLinecap="round" />
      </svg>
    </span>
  );
}

/* Brass thermometer with five chillies; lit ones flicker, hotter levels flicker faster. */
export function SpiceGauge({ level = 1, className = '' }) {
  const n = Math.max(1, Math.min(5, level));
  const x0 = 26, x1 = 176;
  const step = (x1 - x0) / 5;
  const fillTo = x0 + step * n - step / 2 + 6;
  return (
    <span
      className={`pp-gauge pp-gauge-${n} ${className}`}
      role="img"
      aria-label={`Spice ${n} of 5`}
      style={{ '--flick': `${(2.3 - n * 0.32).toFixed(2)}s`, '--amp': `${(n * 1.3).toFixed(1)}deg` }}
    >
      <svg viewBox="0 0 190 70" aria-hidden="true" focusable="false">
        <defs>
          <linearGradient id="pp-brass" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#f3d58a" />
            <stop offset=".45" stopColor="#c99a3e" />
            <stop offset="1" stopColor="#7d5a1d" />
          </linearGradient>
          <linearGradient id="pp-mercury" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#ef6a4a" />
            <stop offset="1" stopColor="#8f1f10" />
          </linearGradient>
        </defs>
        <rect x="14" y="50" width="166" height="13" rx="6.5" fill="url(#pp-brass)" />
        <rect x="20" y="54" width="154" height="5" rx="2.5" fill="#3a2410" opacity=".55" />
        <rect className="pp-gauge-fill" x="18" y="54" width={fillTo - 18} height="5" rx="2.5" fill="url(#pp-mercury)" />
        <circle cx="15" cy="56.5" r="10" fill="url(#pp-brass)" />
        <circle cx="15" cy="56.5" r="6.2" fill="url(#pp-mercury)" />
        <circle cx="13" cy="54.3" r="1.8" fill="#fff" opacity=".55" />
        {[0, 1, 2, 3, 4].map((i) => {
          const cx = x0 + step * i + step / 2;
          const lit = i < n;
          return (
            <g key={i} transform={`translate(${cx - 14} 12) rotate(${i % 2 ? 8 : -6} 14 14) scale(1.17)`}>
              <path d={`M12 33v-3.5`} stroke="#5a3d14" strokeWidth="1.2" strokeLinecap="round" opacity=".7" />
              <g className={lit ? 'pp-chilli is-lit' : 'pp-chilli'} style={{ animationDelay: `${-((i * 0.37) % 1)}s` }}>
                <path d={CHILLI_BODY} fill={lit ? 'var(--chili)' : 'none'} stroke={lit ? '#7a1a0c' : 'rgba(250,240,220,.45)'} strokeWidth="1.3" strokeLinejoin="round" />
                {lit && <path d="M7.2 15.2c.3-2 1.1-3.6 2.2-4.8" fill="none" stroke="#fff" strokeOpacity=".45" strokeWidth="1.2" strokeLinecap="round" />}
                <path d={CHILLI_STEM} fill="none" stroke={lit ? '#4f8a3c' : 'rgba(250,240,220,.45)'} strokeWidth="1.6" strokeLinecap="round" />
              </g>
            </g>
          );
        })}
      </svg>
    </span>
  );
}
