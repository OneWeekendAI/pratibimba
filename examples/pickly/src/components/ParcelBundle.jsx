import { useId } from 'react';

/*
 * The parcel: jars standing on a sheet of newspaper that folds around them,
 * then twine ties a cross with a bow. Pure SVG + CSS transitions.
 *
 * Geometry (viewBox 500x360): the parcel face is x140..360, y150..310.
 * Each flap is hinged on a parcel edge. Flat = scale(-1) on its hinge axis,
 * so it lies outside the face as part of the open sheet; folded = scale(1).
 * The CSS transition passes through 0, which reads as a real fold.
 *
 * props
 *   jars    [{ id, image }] up to 3 are drawn
 *   left, right, top, tied  booleans: which steps are done
 *   delays  { left, right, top, tie } ms, used only while a sequence plays
 *   still   true -> no transitions (reduced motion, or the thanks page)
 */

const FACE = { x: 140, y: 150, w: 220, h: 160 };

function jarSlots(n) {
  if (n <= 1) return [{ cx: 250, h: 150 }];
  if (n === 2) return [{ cx: 212, h: 140 }, { cx: 288, h: 146 }];
  return [{ cx: 196, h: 136 }, { cx: 304, h: 138 }, { cx: 250, h: 150 }];
}

/* Newsprint: column rules, a headline bar, grey "text" lines. Nothing readable. */
function Newsprint({ id }) {
  const lines = [];
  const cols = [4, 44, 84];
  cols.forEach((x, ci) => {
    for (let r = 0; r < 15; r += 1) {
      const y = 6 + r * 5.6;
      if (ci === 1 && r < 4) continue; // headline block sits here
      const w = r % 6 === 5 ? 18 + ((r * 7 + ci * 5) % 12) : 32 - ((r * 3 + ci) % 4);
      lines.push(<rect key={`${ci}-${r}`} x={x} y={y} width={w} height="2.1" rx="1" />);
    }
  });
  return (
    <pattern id={id} width="124" height="90" patternUnits="userSpaceOnUse" patternTransform="rotate(-4)">
      <rect width="124" height="90" fill="var(--newsprint)" />
      <g fill="var(--newsprint-ink)" opacity=".22">{lines}</g>
      <g fill="var(--newsprint-ink)" opacity=".42">
        <rect x="44" y="6" width="34" height="5" rx="1" />
        <rect x="44" y="14" width="26" height="5" rx="1" />
      </g>
      <g stroke="var(--newsprint-ink)" strokeWidth=".6" opacity=".28">
        <path d="M40 2v86M80 2v86" />
      </g>
      <rect x="86" y="40" width="30" height="22" fill="var(--newsprint-ink)" opacity=".1" />
    </pattern>
  );
}

export default function ParcelBundle({ jars = [], left, right, top, tied, delays = {}, still = false, className = '', viewBox = '10 55 480 275' }) {
  const uid = useId().replace(/:/g, '');
  const news = `pp-news-${uid}`;
  const crease = `pp-crease-${uid}`;
  const creaseY = `pp-creaseY-${uid}`;
  const shown = jars.slice(0, 3);
  const slots = jarSlots(shown.length);
  const d = (ms) => (still || ms == null ? undefined : { transitionDelay: `${ms}ms` });
  const tie = delays.tie ?? 0;
  const folded = left && right && top;

  return (
    <svg
      viewBox={viewBox}
      className={['pp', still ? 'pp-still' : '', folded ? 'is-folded' : '', tied ? 'is-tied' : '', className].join(' ')}
      aria-hidden="true"
      focusable="false"
    >
      <defs>
        <Newsprint id={news} />
        <linearGradient id={crease} x1="0" x2="1">
          <stop offset="0" stopColor="#3a2a14" stopOpacity=".2" />
          <stop offset=".18" stopColor="#3a2a14" stopOpacity="0" />
          <stop offset=".9" stopColor="#fff" stopOpacity="0" />
          <stop offset="1" stopColor="#fff" stopOpacity=".18" />
        </linearGradient>
        <linearGradient id={creaseY} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#3a2a14" stopOpacity=".22" />
          <stop offset=".3" stopColor="#3a2a14" stopOpacity="0" />
        </linearGradient>
      </defs>

      {/* counter shadow */}
      <ellipse cx="250" cy="314" rx="150" ry="11" fill="rgba(60,35,10,.22)" className="pp-shadow" />

      {/* base of the sheet, under the jars */}
      <path
        d="M140 150h220v160H140z"
        fill={`url(#${news})`}
        stroke="rgba(74,67,56,.25)"
        strokeWidth="1"
      />

      {/* the jars, standing on the sheet */}
      <g className="pp-jars">
        {shown.map((j, i) => {
          const s = slots[i];
          const w = s.h * 0.82;
          return (
            <image
              key={j.id}
              href={j.image}
              x={s.cx - w / 2}
              y={310 - s.h + 6}
              width={w}
              height={s.h}
              preserveAspectRatio="xMidYMax meet"
            />
          );
        })}
      </g>

      {/* left flap: hinge x=140, free edge torn at x~262 */}
      <g className={`pp-flap pp-flap-l ${left ? 'on' : ''}`} style={d(delays.left)}>
        <path
          d="M140 150 L262 150 L259 168 L264 190 L260 214 L265 238 L259 262 L264 288 L261 310 L140 310Z"
          fill={`url(#${news})`}
        />
        <path d="M140 150 L262 150 L259 168 L264 190 L260 214 L265 238 L259 262 L264 288 L261 310 L140 310Z" fill={`url(#${crease})`} />
        <path d="M262 150 L259 168 L264 190 L260 214 L265 238 L259 262 L264 288 L261 310" fill="none" stroke="rgba(74,67,56,.3)" strokeWidth="1" />
      </g>

      {/* right flap: hinge x=360 */}
      <g className={`pp-flap pp-flap-r ${right ? 'on' : ''}`} style={d(delays.right)}>
        <path
          d="M360 150 L238 150 L241 172 L236 196 L240 220 L235 246 L240 270 L236 292 L239 310 L360 310Z"
          fill={`url(#${news})`}
        />
        <path d="M360 150 L238 150 L241 172 L236 196 L240 220 L235 246 L240 270 L236 292 L239 310 L360 310Z" fill={`url(#${crease})`} transform="matrix(-1 0 0 1 598 0)" />
        <path d="M238 150 L241 172 L236 196 L240 220 L235 246 L240 270 L236 292 L239 310" fill="none" stroke="rgba(74,67,56,.3)" strokeWidth="1" />
      </g>

      {/* top flap: hinge y=150, a soft point at the bottom */}
      <g className={`pp-flap pp-flap-t ${top ? 'on' : ''}`} style={d(delays.top)}>
        <path d="M140 150 H360 L356 196 L250 238 L144 196Z" fill={`url(#${news})`} />
        <path d="M140 150 H360 L356 196 L250 238 L144 196Z" fill={`url(#${creaseY})`} />
        <path d="M360 150 L356 196 L250 238 L144 196 L140 150" fill="none" stroke="rgba(74,67,56,.32)" strokeWidth="1" />
      </g>

      {/* twine: the cross, then the bow */}
      <g className={`pp-twine ${tied ? 'on' : ''}`} fill="none" strokeLinecap="round" strokeLinejoin="round">
        <g className="pp-tw-under">
          <path className="pp-draw" pathLength="1" style={d(tie)} d="M250.5 150.6 C251.5 196 249 262 250.5 309.4" />
          <path className="pp-draw" pathLength="1" style={d(tie + 160)} d="M140.6 229 C196 227 304 232 359.4 229.5" />
        </g>
        {/* where the twine turns round each edge: the paper pinches in under it and
            the cord rolls over the edge and away behind the parcel */}
        <g className="pp-tw-wrap" style={d(tie + 300)}>
          <path className="pp-pinch" d="M140 222.5 Q143.6 229 140 235.5 Z M360 223 Q356.4 229.5 360 236 Z M243.5 150 Q250.5 153.4 257.5 150 Z M243.5 310 Q250.5 306.6 257.5 310 Z" />
          {/* the cord darkens as it turns away, with a thin lip of it showing past the edge */}
          <path className="pp-turn" d="M148 228.9 L141 229 M352 229.4 L359 229.5 M250.6 158 L250.6 151 M250.4 302 L250.4 309" />
          <path className="pp-roll" d="M141 226.9 Q138.9 229 141 231.1 M359 227.4 Q361.1 229.5 359 231.6 M248.5 151 Q250.6 148.9 252.7 151 M248.3 309 Q250.4 311.1 252.5 309" />
        </g>
        <g className="pp-tw-bow">
          <path className="pp-draw" pathLength="1" style={d(tie + 400)} d="M250 229 C236 208 210 206 212 222 C214 236 236 236 250 229" />
          <path className="pp-draw" pathLength="1" style={d(tie + 470)} d="M250 229 C266 207 292 207 290 223 C288 237 265 236 250 229" />
          <path className="pp-draw pp-tail" pathLength="1" style={d(tie + 560)} d="M249 231 C245 246 239 256 229 266" />
          <path className="pp-draw pp-tail" pathLength="1" style={d(tie + 600)} d="M251 231 C258 246 265 254 276 262" />
        </g>
        <circle className="pp-knot" cx="250" cy="229.5" r="4.4" style={d(tie + 560)} />
      </g>
    </svg>
  );
}

/*
 * Delays for a full wrap-and-tie from the current state. Returns { delays, total }.
 * `lead` holds everything back (e.g. while the page scrolls the parcel into view).
 * Even when every flap is already folded, the finish is a full beat: the paper is
 * pressed flat, the twine crosses and bows, the knot is pulled (the parcel cinches),
 * then it is lifted off the counter. delays.cinch is when that pull lands.
 */
export function wrapSequence({ left, right, top }, lead = 0) {
  let t = lead;
  const delays = {};
  let lastFoldEnd = lead;
  const allFolded = left && right && top;
  if (!left) { delays.left = t; lastFoldEnd = t + 440; t += 210; }
  if (!right) { delays.right = t; lastFoldEnd = t + 440; t += 230; }
  if (!top) { delays.top = t; lastFoldEnd = t + 460; t += 230; }
  if (allFolded) { delays.press = lead; lastFoldEnd = lead + 260; }
  const tie = Math.max(lastFoldEnd - 80, 0);
  delays.tie = tie;
  delays.cinch = tie + 640;
  return { delays, total: tie + 1260 };
}
