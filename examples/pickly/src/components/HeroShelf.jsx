import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { PICKLES } from '../data/shop';
import { useReducedMotion } from '../lib/motion';

/* image width / height of each jar cutout (all are 1200px tall) */
const ASPECT = {
  avakaya: 984 / 1200, gongura: 1036 / 1200, 'tomato-thokku': 1177 / 1200, usirikaya: 920 / 1200,
  allam: 1162 / 1200, chicken: 1112 / 1200, mutton: 903 / 1200, prawn: 1064 / 1200,
  fish: 946 / 1200, 'boneless-chicken': 1122 / 1200,
};
/* opaque glass inside the cutout: ~2.6% margin each side, bottom at 97.7% */
const BODY_W = 0.945, BODY_H = 0.953;

const GAP = 0.04;       // gap between jars, fraction of jar height (close enough to clink)

/* Always two planks: veg above, non-veg below. `wide` = the side-by-side desktop layout,
   where the stage bleeds off the right edge of the page like a shelf running off the wall. */
export function shelfLayout(W, wide) {
  const groups = [PICKLES.filter((p) => p.veg), PICKLES.filter((p) => !p.veg)];
  const inset = wide ? 10 : 6;
  const usable = W - inset * 2;
  let H = Infinity;
  groups.forEach((g) => {
    const sumA = g.reduce((s, p) => s + (ASPECT[p.id] || 0.85) * BODY_W, 0);
    H = Math.min(H, (usable * 0.985) / (sumA + GAP * (g.length - 1)));
  });
  H = Math.min(Math.floor(H), wide ? 196 : 170);
  const plankT = wide ? Math.round(Math.max(16, H * 0.11)) : 14; // plank front-face thickness
  const rowGap = wide ? 0 : 8;
  const jars = [];
  const rows = [];
  const rowOf = [];
  let y = 0;
  groups.forEach((g, r) => {
    const plankY = y + H * (r === 0 ? 1.14 : 1.3);
    rows.push({ plankY, x0: inset, x1: W - inset, ceilY: plankY - H * 1.24, veg: g[0].veg });
    const widths = g.map((p) => (ASPECT[p.id] || 0.85) * H);
    const bodyWs = widths.map((w) => w * BODY_W);
    const total = bodyWs.reduce((s, w) => s + w, 0) + GAP * H * (g.length - 1);
    let x = (W - total) / 2;
    g.forEach((p, k) => {
      const w = widths[k];
      const cx = x + bodyWs[k] / 2;
      const bh = H * BODY_H;
      jars.push({ id: p.id, x: cx, y: plankY - bh / 2, w, h: H, bw: bodyWs[k], bh, home: cx, row: r, col: k });
      rowOf.push(r);
      x += bodyWs[k] + GAP * H;
    });
    y = plankY + plankT + rowGap;
  });
  return { W, H, wide, jars, rows, rowOf, plankT, height: Math.ceil(y - rowGap + (wide ? 10 : 4)) };
}

const byId = (id) => PICKLES.find((q) => q.id === id);
const WIDE_MQ = '(min-width: 900px)';

/* The hero's working parts: the copy column (children) with the label reader under it,
   and the pantry. Owns the grid so the label can sit beside the shelf on wide screens. */
export default function HeroShelf({ children }) {
  const reduced = useReducedMotion();
  const stageRef = useRef(null);
  const jarEls = useRef([]);
  const tagEls = useRef([]);
  const shadowEls = useRef([]);
  const glintRef = useRef(null);
  const physRef = useRef(null);
  const [layout, setLayout] = useState(null);
  const [selected, setSelected] = useState(-1);

  // measure before paint so the static shelf and the physics shelf are the same picture
  useLayoutEffect(() => {
    const el = stageRef.current;
    if (!el) return undefined;
    let key = '';
    const measure = () => {
      const w = Math.round(el.clientWidth);
      const wide = window.matchMedia(WIDE_MQ).matches;
      if (w && `${w}:${wide}` !== key) {
        key = `${w}:${wide}`;
        setLayout(shelfLayout(w, wide));
      }
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // physics: lazy matter-js, never under reduced motion
  useEffect(() => {
    if (!layout || reduced) return undefined;
    let cancelled = false;
    let phys = null;
    let io = null;
    let onVis = null;
    let inView = true;
    import('./HeroPhysics.js').then(({ createShelfPhysics }) => {
      if (cancelled || !glintRef.current) return;
      const els = layout.jars.map((_, i) => ({
        jar: jarEls.current[i], tag: tagEls.current[i], shadow: shadowEls.current[i],
      }));
      if (els.some((e) => !e.jar)) return;
      phys = createShelfPhysics({ ...layout, els, glintLayer: glintRef.current });
      physRef.current = phys;
      const first = !window.__picklyShelfRippled;
      window.__picklyShelfRippled = true;
      onVis = () => phys.setVisible(inView && !document.hidden);
      document.addEventListener('visibilitychange', onVis);
      io = new IntersectionObserver(([e]) => { inView = e.isIntersecting; onVis(); }, { threshold: 0 });
      io.observe(stageRef.current);
      if (first) phys.ripple(450);
    });
    return () => {
      cancelled = true;
      if (io) io.disconnect();
      if (onVis) document.removeEventListener('visibilitychange', onVis);
      if (phys) phys.destroy();
      physRef.current = null;
    };
  }, [layout, reduced]);

  // pointer = soft invisible finger
  const last = useRef(null);
  const onPointerMove = useCallback((e) => {
    const phys = physRef.current;
    const stage = stageRef.current;
    if (!phys || !stage) return;
    const r = stage.getBoundingClientRect();
    const x = e.clientX - r.left, y = e.clientY - r.top;
    const t = e.timeStamp;
    const p = last.current;
    last.current = { x, y, t };
    if (!p || t - p.t > 160) return;
    const dt = Math.max(t - p.t, 4);
    phys.pointer(x, y, (x - p.x) / dt, (y - p.y) / dt, e.pointerType === 'touch' ? 2.4 : 1);
  }, []);
  const onPointerLeave = useCallback(() => { last.current = null; }, []);

  const knock = (i, dir) => {
    setSelected(i);
    if (physRef.current) physRef.current.knock(i, dir, 1);
  };

  const onJarClick = (i) => (e) => {
    let dir = i % 2 ? -1 : 1;
    if (e.detail > 0 && e.currentTarget) {
      const r = e.currentTarget.getBoundingClientRect();
      dir = e.clientX < r.left + r.width / 2 ? 1 : -1; // tap the left side, it rocks right
    }
    knock(i, dir);
  };

  // one tab stop for the whole pantry; arrows walk along it (then Tab reaches the jar's link)
  const onJarKey = (i) => (e) => {
    const n = layout ? layout.jars.length : 0;
    let k = -1;
    if (e.key === 'ArrowRight') k = (i + 1) % n;
    else if (e.key === 'ArrowLeft') k = (i - 1 + n) % n;
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      // jump plank, keeping the column
      const j = layout.jars[i];
      const other = layout.jars.findIndex((q) => q.row !== j.row && q.col === j.col);
      k = other >= 0 ? other : i;
    } else if (e.key === 'Home') k = 0;
    else if (e.key === 'End') k = n - 1;
    if (k < 0) return;
    e.preventDefault();
    const btn = jarEls.current[k]?.querySelector('button');
    if (btn) btn.focus();
  };

  const selJar = selected >= 0 && layout ? layout.jars[selected] : null;
  const sel = selJar ? byId(selJar.id) : null;
  const stageStyle = layout ? { height: layout.height } : undefined;

  return (
    <div className={`hs-inner${reduced ? ' is-still' : ''}${sel ? ' has-pick' : ''}`}>
      <div className="hs-copy">{children}</div>

      <div className="hs-shelf">
        <div
          className="hs-stage"
          ref={stageRef}
          role="group"
          aria-label="Pantry shelves of pickle jars: veg above, non-veg below"
          style={stageStyle}
          onPointerMove={onPointerMove}
          onPointerLeave={onPointerLeave}
          onPointerCancel={onPointerLeave}
        >
          {layout && layout.rows.map((r, k) => (
            <div key={k} className="hs-plank" aria-hidden="true"
              style={{ top: r.plankY - 5, height: layout.plankT + 5 }}>
              <span className="hs-plank-label">{r.veg ? 'veg' : 'non-veg'}</span>
              {selJar && selJar.row === k && (
                <span className="hs-plank-pick" style={{ transform: `translate3d(${selJar.home}px,0,0)` }} />
              )}
            </div>
          ))}
          {layout && layout.jars.map((j, i) => (
            <span key={j.id + '-s'} className="hs-shadow" aria-hidden="true"
              ref={(n) => { shadowEls.current[i] = n; }}
              style={{
                width: j.w, top: layout.rows[j.row].plankY - 7,
                transform: `translate3d(${j.x - j.w / 2}px,0,0)`,
              }} />
          ))}
          {layout && layout.jars.map((j, i) => {
            const p = byId(j.id);
            return (
              <div key={j.id} className={`hs-jar${i === selected ? ' is-picked' : ''}`}
                ref={(n) => { jarEls.current[i] = n; }}
                style={{ width: j.w, height: j.h, transform: `translate3d(${j.x - j.w / 2}px, ${j.y - j.h / 2}px, 0)` }}>
                <button type="button" className="hs-jar-btn" aria-label={p.imageAlt}
                  aria-describedby="hs-knock-hint" onClick={onJarClick(i)} onFocus={() => setSelected(i)}
                  tabIndex={i === (selected < 0 ? 0 : selected) ? 0 : -1} onKeyDown={onJarKey(i)}>
                  <img src={p.image} alt="" width={Math.round(j.w)} height={Math.round(j.h)} draggable="false"
                    loading="eager" decoding="async" fetchpriority={i < 5 ? 'high' : 'auto'}
                    ref={(n) => { if (n && n.complete) n.parentNode.parentNode.classList.add('is-in'); }}
                    onLoad={(e) => e.currentTarget.parentNode.parentNode.classList.add('is-in')} />
                </button>
                <span className="hs-tag" aria-hidden="true" ref={(n) => { tagEls.current[i] = n; }}
                  style={{ '--tag-w': `${Math.max(18, j.h * 0.27)}px` }}>
                  <span className="hs-tag-paper">{layout.H >= 96 ? `₹${p.price}` : ''}</span>
                </span>
              </div>
            );
          })}
          <div className="hs-glints" ref={glintRef} aria-hidden="true" />
        </div>
      </div>

      <p id="hs-knock-hint" className="hs-sr">Arrow keys move along the shelves, up and down between them. Enter knocks the jar; its page is linked after the shelves.</p>
      <div className="hs-aside">
        <div className="hs-caption" aria-live="polite">
          {sel ? (
            <div className="hs-label" key={sel.id}>
              <span className="hs-cap-kind">{sel.veg ? 'Veg' : 'Non-veg'} · {sel.weightGrams} g</span>
              <span className="hs-cap-name">{sel.nativeName}</span>
              <span className="hs-cap-blurb">{sel.shortBlurb}</span>
              <span className="hs-cap-row">
                <span className="hs-cap-price">₹{sel.price}</span>
                <a className="hs-cap-link" href={`#/product/${sel.id}`}>
                  Open this jar <span aria-hidden="true">→</span>
                </a>
              </span>
            </div>
          ) : (
            <p className="hs-note" aria-hidden="true">
              <span>{reduced ? 'pick a jar, read its label' : 'go on, give it a nudge'}</span>
              <svg className="hs-note-arrow hs-note-arrow--side" viewBox="0 0 96 56" width="96" height="56">
                <path d="M4 40c20 10 46 10 66-6 7-6 12-13 16-21" />
                <path d="M72 14c5-1 10-2 15-5 1 5 2 10 1 16" />
              </svg>
              <svg className="hs-note-arrow hs-note-arrow--up" viewBox="0 0 90 70" width="90" height="70">
                <path d="M8 6c18 2 38 10 50 24 8 9 12 19 13 31" />
                <path d="M62 52c3 4 6 7 9 10 3-4 6-8 10-11" />
              </svg>
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
