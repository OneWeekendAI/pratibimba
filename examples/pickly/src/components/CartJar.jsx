/*
 * CartJar — the header cart, drawn as a glass jar that fills with pickle.
 *
 * Liquid = one band per pickle in the cart, stacked bottom-up in cart order,
 * height proportional to qty (12 jars = full). A lighter sesame-oil line sits on
 * top. The surface is a damped spring system (level, tilt, ripple) driven by one
 * rAF loop that writes path data straight to the DOM and stops once settled.
 * 'pickly:jar-landed' (from lib/motion flyToJar) wobbles the lid and sloshes.
 * While a flight is in the air the visual level waits for the landing.
 */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { onChange } from '../lib/cart';
import { PICKLES } from '../data/shop';
import { pickleColor } from '../lib/pickleColors';
import { useReducedMotion, stepSpring, jarFlightsPending } from '../lib/motion';

const FULL_AT = 12;
const BOTTOM = 75.2;
const TOP = 31; // liquid y when full
const XS = Array.from({ length: 15 }, (_, i) => 6 + i * 3.7);

const BODY =
  'M17.5 21.5C17.5 25.5 8.5 25.2 8.5 32.5L8.5 69C8.5 73.4 11.2 75.6 15.4 75.6L48.6 75.6C52.8 75.6 55.5 73.4 55.5 69L55.5 32.5C55.5 25.2 46.5 25.5 46.5 21.5Z';

const nameOf = (id) => PICKLES.find((p) => p.id === id)?.name || 'Pickle';
const totalOf = (items) => items.reduce((a, it) => a + (it.qty || 0), 0);
// Fill curve: the first jars read clearly, the last few top it off.
const fillOf = (n) => (n > 0 ? 0.12 + 0.88 * (1 - Math.pow(1 - Math.min(n, FULL_AT) / FULL_AT, 1.8)) : 0);
const jars = (n) => `${n} ${n === 1 ? 'jar' : 'jars'}`;

function surface(base, tilt, amp, phase, weight, seed = 0) {
  // y at each x: base + tilt across the jar + a travelling ripple
  // + a small fixed lumpiness so band boundaries look packed, not ruled
  return XS.map((x, i) => {
    const dx = (x - 32) / 24;
    const lump = seed ? Math.sin(i * 1.9 + seed * 2.3) * 0.55 + Math.sin(i * 0.7 + seed) * 0.35 : 0;
    return base + lump + tilt * dx * 6 * weight + amp * weight * Math.sin(dx * 3.4 + phase) * 1.6;
  });
}

function bandPath(ys, offset = 0) {
  let d = `M${XS[0]} ${ys[0] + offset}`;
  for (let i = 1; i < XS.length; i++) d += `L${XS[i]} ${(ys[i] + offset).toFixed(2)}`;
  return `${d}L58 80L6 80Z`;
}

function linePath(ys, offset = 0) {
  let d = `M${XS[0]} ${ys[0] + offset}`;
  for (let i = 1; i < XS.length; i++) d += `L${XS[i]} ${(ys[i] + offset).toFixed(2)}`;
  return d;
}

export default function CartJar() {
  const reduced = useReducedMotion();
  const [items, setItems] = useState([]); // real cart (labels, aria)
  const [shown, setShown] = useState([]); // what the glass shows (waits for flights)
  const [announce, setAnnounce] = useState('');
  const prevRef = useRef(null);
  const heldRef = useRef(null);

  const bandRefs = useRef(new Map());
  const oilRef = useRef(null);
  const oilLineRef = useRef(null);
  const liquidClipRef = useRef(null);
  const lidRef = useRef(null);
  const sim = useRef({
    level: { x: 0, v: 0 },
    tilt: { x: 0, v: 0 },
    amp: 0,
    phase: 0,
    lidRot: { x: 0, v: 0 },
    lidHop: { x: 0, v: 0 },
    raf: 0,
    last: 0,
  });
  const shownRef = useRef(shown);
  shownRef.current = shown;
  const reducedRef = useRef(reduced);
  reducedRef.current = reduced;

  // ---- cart subscription + announcements ----
  useEffect(() => {
    return onChange((cart) => {
      const next = (cart.items || []).filter((it) => it.qty > 0).map((it) => ({ id: it.id, qty: it.qty }));
      setItems(next);
      const prev = prevRef.current;
      prevRef.current = next;
      if (prev) {
        const total = totalOf(next);
        let msg = '';
        for (const it of next) {
          const before = prev.find((p) => p.id === it.id)?.qty || 0;
          if (it.qty > before) msg = `${nameOf(it.id)} added. Your cart has ${jars(total)}.`;
          else if (it.qty < before) msg = `${nameOf(it.id)} reduced. Your cart has ${jars(total)}.`;
        }
        for (const p of prev) {
          if (!next.find((it) => it.id === p.id)) msg = `${nameOf(p.id)} removed. ${total ? `Your cart has ${jars(total)}.` : 'Your cart is empty.'}`;
        }
        if (msg) setAnnounce(msg);
        // flyToJar is called right after addItem; give it a microtask to register.
        queueMicrotask(() => {
          if (jarFlightsPending() > 0) heldRef.current = next;
          else setShown(next);
        });
      } else {
        setShown(next);
        // first paint: start at the level, no rise
        const s = sim.current;
        s.level.x = fillOf(totalOf(next));
      }
    });
  }, []);

  // ---- the simulation ----
  const draw = () => {
    const s = sim.current;
    const list = shownRef.current;
    const total = Math.min(totalOf(list), FULL_AT);
    const capped = totalOf(list);
    const H = (BOTTOM - TOP) * s.level.x;
    let acc = 0;
    let topYs = null;
    list.forEach((it, idx) => {
      acc += it.qty;
      const frac = capped ? Math.min(acc, capped) / capped : 0;
      const base = BOTTOM - H * frac;
      const weight = 0.35 + 0.65 * frac; // lower boundaries slosh less
      const isTop = idx === list.length - 1;
      const ys = surface(base, s.tilt.x, s.amp, s.phase, weight, isTop ? 0 : idx + 1);
      const el = bandRefs.current.get(it.id);
      if (el) el.setAttribute('d', bandPath(ys));
      topYs = ys;
    });
    const hasLiquid = total > 0 && H > 0.3;
    if (oilRef.current) {
      oilRef.current.setAttribute('d', hasLiquid ? bandPath(topYs, -2.4) : '');
      oilLineRef.current.setAttribute('d', hasLiquid ? linePath(topYs, -2.4) : '');
      liquidClipRef.current.setAttribute('d', hasLiquid ? bandPath(topYs, -2.4) : 'M0 0Z');
    }
    if (lidRef.current) {
      lidRef.current.setAttribute(
        'transform',
        `translate(0 ${s.lidHop.x.toFixed(2)}) rotate(${s.lidRot.x.toFixed(2)} 32 17)`
      );
    }
  };

  const tick = (now) => {
    const s = sim.current;
    const dt = Math.min(0.034, (now - (s.last || now)) / 1000) || 1 / 60;
    s.last = now;
    const target = fillOf(totalOf(shownRef.current));
    const a = stepSpring(s.level, target, { k: 55, c: 13, m: 1 }, dt);
    const b = stepSpring(s.tilt, 0, { k: 70, c: 2.6, m: 1 }, dt);
    const c = stepSpring(s.lidRot, 0, { k: 200, c: 3.6, m: 0.6 }, dt);
    const d = stepSpring(s.lidHop, 0, { k: 340, c: 16, m: 0.6 }, dt);
    s.amp *= Math.exp(-dt * 2.2);
    s.phase += dt * 7.5;
    draw();
    const moving = a || b || c || d || Math.abs(s.amp) > 0.01;
    if (moving && !document.hidden) s.raf = requestAnimationFrame(tick);
    else {
      s.raf = 0;
      s.last = 0;
    }
  };

  const wake = () => {
    const s = sim.current;
    if (reducedRef.current) {
      s.level.x = fillOf(totalOf(shownRef.current));
      s.level.v = s.tilt.x = s.tilt.v = s.amp = 0;
      s.lidRot.x = s.lidRot.v = s.lidHop.x = s.lidHop.v = 0;
      draw();
      return;
    }
    if (!s.raf) s.raf = requestAnimationFrame(tick);
  };

  const slosh = (strength = 1) => {
    const s = sim.current;
    s.tilt.v += (s.tilt.x > 0 ? -1 : 1) * 2.2 * strength;
    s.amp = Math.min(1.4, s.amp + 0.9 * strength);
  };

  // contents changed -> slosh a little and rise/fall to the new level
  useLayoutEffect(() => {
    slosh(0.6);
    draw();
    wake();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown, reduced]);

  // landing: release any held level, wobble the lid, slosh harder
  useEffect(() => {
    const onLand = () => {
      if (heldRef.current && jarFlightsPending() === 0) {
        setShown(heldRef.current);
        heldRef.current = null;
      }
      const s = sim.current;
      s.lidRot.v += 170 * (Math.random() > 0.5 ? 1 : -1);
      s.lidHop.v -= 70;
      slosh(1);
      wake();
    };
    const onVis = () => !document.hidden && wake();
    window.addEventListener('pickly:jar-landed', onLand);
    document.addEventListener('visibilitychange', onVis);
    return () => {
      window.removeEventListener('pickly:jar-landed', onLand);
      document.removeEventListener('visibilitychange', onVis);
      cancelAnimationFrame(sim.current.raf);
      sim.current.raf = 0;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const count = totalOf(items);
  const label = count ? `Cart, ${jars(count)}` : 'Cart, empty';
  const bandsTopFirst = [...shown].reverse();

  return (
    <>
    <a href="#/cart" className={`cart-jar${count ? ' has-items' : ''}`} aria-label={label} title={count ? jars(count) + ' in your jar' : 'your jar is empty'}>
      <svg viewBox="0 0 64 80" className="cart-jar-svg" aria-hidden="true" focusable="false" data-cart-jar-target>
        <defs>
          <clipPath id="cj-inside">
            <path d={BODY} transform="translate(32 48) scale(.94 .96) translate(-32 -48)" />
          </clipPath>
          <clipPath id="cj-liquid">
            <path ref={liquidClipRef} d="M0 0Z" />
          </clipPath>
          <pattern id="cj-specks" width="9" height="7" patternUnits="userSpaceOnUse" patternTransform="rotate(12)">
            <circle cx="1.5" cy="1.2" r=".55" fill="#2a1404" opacity=".55" />
            <circle cx="6.2" cy="3.9" r=".45" fill="#f3c25a" opacity=".7" />
            <ellipse cx="3.6" cy="5.6" rx=".9" ry=".4" fill="#5a1708" opacity=".45" />
            <circle cx="8.1" cy="0.8" r=".35" fill="#fff0c8" opacity=".5" />
          </pattern>
          <linearGradient id="cj-round" x1="0" x2="1" y1="0" y2="0">
            <stop offset="0" stopColor="#2a1606" stopOpacity=".38" />
            <stop offset=".28" stopColor="#2a1606" stopOpacity="0" />
            <stop offset=".72" stopColor="#2a1606" stopOpacity="0" />
            <stop offset="1" stopColor="#2a1606" stopOpacity=".42" />
          </linearGradient>
          <linearGradient id="cj-glass" x1="0" x2="1">
            <stop offset="0" stopColor="#fffaf0" stopOpacity=".5" />
            <stop offset=".45" stopColor="#fffaf0" stopOpacity=".12" />
            <stop offset="1" stopColor="#d9c7a3" stopOpacity=".38" />
          </linearGradient>
          <radialGradient id="cj-kraft" cx=".4" cy=".3" r=".9">
            <stop offset="0" stopColor="var(--kraft-light)" />
            <stop offset="1" stopColor="var(--kraft)" />
          </radialGradient>
        </defs>

        {/* shadow on the shelf */}
        <ellipse cx="33" cy="77" rx="23" ry="2.4" fill="#3a2410" opacity=".18" />

        {/* glass back wall */}
        <path d={BODY} fill="url(#cj-glass)" />

        <g clipPath="url(#cj-inside)">
          {/* crumbs of spice when empty */}
          {count === 0 && shown.length === 0 && (
            <g className="cj-crumbs">
              <ellipse cx="22" cy="73.4" rx="1.3" ry=".8" fill="var(--chili)" opacity=".75" />
              <ellipse cx="27.5" cy="74.1" rx=".8" ry=".6" fill="var(--mustard-deep)" />
              <ellipse cx="38" cy="73.8" rx="1.1" ry=".7" fill="var(--chili)" opacity=".6" />
              <circle cx="43" cy="74.2" r=".6" fill="#3b2a12" />
              <circle cx="31" cy="74.3" r=".5" fill="#3b2a12" />
            </g>
          )}
          <path ref={oilRef} fill="var(--oil-light)" opacity=".85" />
          {bandsTopFirst.map((it) => (
            <path
              key={it.id}
              ref={(el) => (el ? bandRefs.current.set(it.id, el) : bandRefs.current.delete(it.id))}
              fill={pickleColor(it.id)}
            />
          ))}
          <path ref={oilLineRef} fill="none" stroke="#fff3cf" strokeWidth=".9" opacity=".9" />
          <rect x="0" y="0" width="64" height="80" fill="url(#cj-specks)" clipPath="url(#cj-liquid)" />
          {/* cylinder shading over the contents */}
          <rect x="0" y="0" width="64" height="80" fill="url(#cj-round)" />
        </g>

        {/* glass: outline, rim and highlights */}
        <path d={BODY} fill="none" stroke="var(--glass-edge)" strokeWidth="1.5" />
        <path d="M13.2 35c-.6 9-.6 21 .2 31" stroke="var(--glass-shine)" strokeWidth="2.4" strokeLinecap="round" fill="none" />
        <path d="M17.2 33.5c-.3 2.4-.3 4.6 0 6.6" stroke="var(--glass-shine)" strokeWidth="1.3" strokeLinecap="round" fill="none" opacity=".8" />
        <path d="M50.5 40c.3 7 .3 14-.2 22" stroke="#fff" strokeWidth="1" strokeLinecap="round" fill="none" opacity=".45" />
        <path d="M17 21.6c9.8.9 20.2.9 30 0" stroke="var(--glass-edge)" strokeWidth="1.1" fill="none" />

        {/* the lid: kraft paper cover tied with twine */}
        <g ref={lidRef} className="cj-lid">
          <path
            d="M14.4 12.5C14.6 8.4 21 6.2 32 6.2S49.4 8.4 49.6 12.5L49.4 18.4 47.6 20.3 45.4 18.9 43 20.6 40.6 19.1 38 20.8 35.4 19.2 32.8 20.9 30.2 19.3 27.6 20.8 25 19.2 22.4 20.6 20 19 17.6 20.4 15 18.7Z"
            fill="url(#cj-kraft)"
            stroke="#7a5a33"
            strokeWidth=".9"
            strokeLinejoin="round"
          />
          <path d="M19 9.4c4-.9 9-1.3 14-1.2" stroke="#fff6e4" strokeWidth="1" strokeLinecap="round" opacity=".6" fill="none" />
          <path d="M14.9 15.2c11.3 1.2 22.7 1.2 34.2 0" stroke="var(--twine-dark)" strokeWidth="1.5" fill="none" />
          <path d="M14.9 16.6c11.3 1.1 22.7 1.1 34.2 0" stroke="var(--twine)" strokeWidth="1" fill="none" />
          <path d="M23.2 16.4c-2.6-2.6-5.6-2.4-5.4-.8.2 1.4 3 1.4 5.4.8zm0 0c-1 2-2.4 3.8-4 5M23.2 16.4c1 2.1 1.6 4 1.8 5.8" stroke="var(--twine-dark)" strokeWidth="1" fill="none" strokeLinecap="round" />
          {count > 0 && (
            <g className="cj-sticker">
              <circle cx="41" cy="11.4" r="8.4" fill="var(--card)" stroke="var(--chili)" strokeWidth="1.1" />
              <circle cx="41" cy="11.4" r="6.6" fill="none" stroke="var(--chili)" strokeWidth=".5" strokeDasharray="1.2 1.1" opacity=".7" />
              <text x="41" y="15.4" textAnchor="middle" className="cj-count">
                {count}
              </text>
            </g>
          )}
        </g>
      </svg>
    </a>
    <span className="sr-only" aria-live="polite" role="status">
      {announce}
    </span>
    </>
  );
}
