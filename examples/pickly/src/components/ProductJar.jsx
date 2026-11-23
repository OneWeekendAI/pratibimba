/*
 * The big jar on the product page, and the open-the-jar interaction.
 *
 * The photo is split along the kraft cover's hem (measured per jar, below) into two
 * layers inside one SVG that uses the photo's own pixel space as its viewBox:
 *   body  = everything under the hem (glass, pickle, the twine tails that hang on the glass)
 *   cover = the kraft cap with its tied twine, plus the loose twine that hangs off the jar
 * Closed, the two layers line up exactly and look like the untouched photo. Under the
 * cover sits a drawn glass neck and mouth. Pull the twine end (drag from the bow) far
 * enough and the knot gives: the cap pops, flutters aside and comes to rest on the shelf,
 * the twine loop slumps to the shoulder, aroma rises and an oil glint crosses the glass.
 * Click / Enter on the control (or tap the bow) does the same; again closes it.
 */
import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { stepSpring } from '../lib/motion';

/* Photo-space geometry (px in the 1200-tall cutout). top: cover top; L/R: cover skirt
   edges; hem: [left, centre, right] y of the skirt's lower edge; knot: the bow's knot;
   bL/bR: the glass body's widest edges (twine outside them leaves with the cover). */
export const JAR_GEOMETRY = {
  avakaya: { W: 984, top: 30, L: 160, R: 930, hem: [282, 272, 282], knot: [270, 200], bL: 145, bR: 952 },
  gongura: { W: 1036, top: 40, L: 64, R: 857, hem: [348, 372, 338], knot: [507, 211], bL: 40, bR: 880 },
  'tomato-thokku': { W: 1177, top: 30, L: 93, R: 943, hem: [307, 314, 307], knot: [857, 214], bL: 43, bR: 1000 },
  usirikaya: { W: 920, top: 40, L: 50, R: 871, hem: [354, 361, 347], knot: [486, 233], bL: 36, bR: 893 },
  allam: { W: 1162, top: 79, L: 100, R: 914, hem: [379, 386, 364], knot: [914, 271], bL: 100, bR: 914 },
  chicken: { W: 1112, top: 40, L: 70, R: 915, hem: [326, 350, 334], knot: [628, 247], bL: 35, bR: 965 },
  mutton: { W: 903, top: 43, L: 43, R: 857, hem: [364, 393, 357], knot: [479, 271], bL: 36, bR: 864 },
  prawn: { W: 1064, top: 37, L: 114, R: 943, hem: [326, 340, 333], knot: [529, 211], bL: 36, bR: 1029 },
  fish: { W: 946, top: 36, L: 64, R: 886, hem: [350, 379, 350], knot: [529, 257], bL: 36, bR: 914 },
  'boneless-chicken': { W: 1122, top: 54, L: 50, R: 929, hem: [358, 376, 347], knot: [886, 226], bL: 43, bR: 929 },
};
const H = 1200;
const HEM_PAD = 5;
const PULL_DIST = 200; // photo px of pull before the knot gives

function hemAt(g, x) {
  const t = Math.min(1, Math.max(0, (x - g.L) / (g.R - g.L)));
  const [a, m, b] = g.hem;
  const c = 2 * m - (a + b) / 2; // quadratic through the three measured points
  return (1 - t) * (1 - t) * a + 2 * t * (1 - t) * c + t * t * b + HEM_PAD;
}

function coverPath(g) {
  const side = Math.max(...g.hem) + 290;
  let d = `M-400 -400H${g.W + 400}V${side}H${g.bR}V${hemAt(g, g.R).toFixed(1)}`;
  for (let i = 0; i <= 24; i++) {
    const x = g.R - ((g.R - g.L) * i) / 24;
    d += `L${x.toFixed(1)} ${hemAt(g, x).toFixed(1)}`;
  }
  d += `L${g.bL} ${hemAt(g, g.L).toFixed(1)}V${side}H-400Z`;
  return d;
}

function neckGeometry(g) {
  const span = g.R - g.L;
  const nL = g.L + span * 0.075;
  const nR = g.R - span * 0.075;
  const cx = (g.L + g.R) / 2;
  const hemC = hemAt(g, cx);
  const top = hemC - (hemC - g.top) * 0.32;
  const rx = (nR - nL) / 2;
  const ry = rx * 0.085;
  return { nL, nR, cx, top, rx, ry, hemC };
}

/* Twine strand as two strokes: body and a twisted darker ply. */
function Strand({ d, width = 13, opacity = 1, innerRef, plyRef }) {
  return (
    <g opacity={opacity} fill="none" strokeLinecap="round" strokeLinejoin="round">
      <path ref={innerRef} d={d} stroke="var(--twine)" strokeWidth={width} />
      <path ref={plyRef} d={d} stroke="var(--twine-dark)" strokeWidth={width * 0.8} strokeDasharray={`${width * 0.55} ${width * 0.6}`} opacity=".7" />
      <path d={d} stroke="#ecd6a8" strokeWidth={width * 0.22} strokeDasharray={`${width * 0.3} ${width * 0.85}`} opacity=".6" transform={`translate(${-width * 0.15} ${-width * 0.2})`} />
    </g>
  );
}

const ProductJar = forwardRef(function ProductJar({ pickle, reduced, color, onOpenChange }, ref) {
  const g = JAR_GEOMETRY[pickle.id] || JAR_GEOMETRY.avakaya;
  const uid = `pj-${pickle.id}`;
  const svgRef = useRef(null);
  const stageRef = useRef(null);
  const coverRef = useRef(null);
  const strandGroupRef = useRef(null);
  const strandA = useRef(null);
  const strandB = useRef(null);
  const loopRef = useRef(null);
  const [open, setOpen] = useState(false);
  const [everOpened, setEverOpened] = useState(false);
  const [dragging, setDragging] = useState(false);

  const neck = neckGeometry(g);
  const [kx, ky] = g.knot;
  const ccx = (g.L + g.R) / 2;
  const ccy = (g.top + neck.hemC) / 2;

  // physics state lives in refs; the rAF loop writes attributes directly.
  const sim = useRef({
    x: { x: 0, v: 0 }, y: { x: 0, v: 0 }, r: { x: 0, v: 0 }, loop: { x: 0, v: 0 },
    target: { x: 0, y: 0, r: 0, loop: 0 }, flutterT: -1, lift: 0,
    strand: null, // { x, y, vx, vy, falling, alpha }
    raf: 0, last: 0,
  });

  useImperativeHandle(ref, () => ({ el: svgRef.current, toggle: () => toggle() }));

  const writeFrame = useCallback(() => {
    const s = sim.current;
    if (coverRef.current) {
      coverRef.current.setAttribute(
        'transform',
        `translate(${s.x.x.toFixed(2)} ${s.y.x.toFixed(2)}) rotate(${s.r.x.toFixed(3)} ${ccx} ${ccy})`
      );
    }
    if (loopRef.current) {
      const p = Math.max(0, Math.min(1.2, s.loop.x));
      const drop = (1 - p) * (ky - neck.hemC - 14);
      loopRef.current.setAttribute('transform', `translate(0 ${drop.toFixed(1)})`);
      loopRef.current.style.opacity = Math.min(1, p * 1.6).toFixed(3);
    }
    const st = s.strand;
    if (strandGroupRef.current) {
      if (!st || st.alpha <= 0) {
        strandGroupRef.current.style.opacity = '0';
      } else {
        const dx = st.x - kx, dy = st.y - ky;
        const len = Math.hypot(dx, dy);
        const sag = Math.max(0, 70 - len * 0.25) + (st.falling ? 40 : 0);
        const mx = kx + dx * 0.5, my = ky + dy * 0.5 + sag;
        const d = `M${kx} ${ky}Q${mx.toFixed(1)} ${my.toFixed(1)} ${st.x.toFixed(1)} ${st.y.toFixed(1)}`;
        strandA.current?.setAttribute('d', d);
        strandB.current?.setAttribute('d', d);
        strandGroupRef.current.style.opacity = st.alpha.toFixed(3);
      }
    }
  }, [ccx, ccy, kx, ky, neck.hemC]);

  const tick = useCallback(
    (now) => {
      const s = sim.current;
      const dt = Math.min(1 / 30, (now - (s.last || now)) / 1000 || 1 / 60);
      s.last = now;
      let moving = false;
      const opening = s.target.loop > 0.5;
      // a light sheet of paper: soft spring, air drag; the cap pops up before it drifts.
      const cfgXY = opening ? { k: 34, c: 8.5, m: 1 } : { k: 150, c: 22, m: 1 };
      let ty = s.target.y;
      if (opening && s.flutterT >= 0 && s.flutterT < 0.22) ty = -150;
      moving = stepSpring(s.x, s.target.x, cfgXY, dt) || moving;
      moving = stepSpring(s.y, ty, opening ? { k: 46, c: 9, m: 1 } : cfgXY, dt) || moving;
      let tr = s.target.r;
      if (s.flutterT >= 0) {
        s.flutterT += dt;
        const f = s.flutterT;
        tr += 11 * Math.sin(f * 6.4) * Math.exp(-f * 1.5);
        if (f > 3.2) s.flutterT = -1;
        else moving = true;
      }
      moving = stepSpring(s.r, tr, opening ? { k: 70, c: 9, m: 1 } : { k: 160, c: 24, m: 1 }, dt) || moving;
      moving = stepSpring(s.loop, s.target.loop, opening ? { k: 90, c: 13, m: 1 } : { k: 200, c: 30, m: 1 }, dt) || moving;
      const st = s.strand;
      if (st && st.falling) {
        st.vy += 1500 * dt;
        st.x += st.vx * dt;
        st.y += st.vy * dt;
        st.alpha -= dt * 1.25;
        if (st.alpha > 0) moving = true;
        else s.strand = null;
      } else if (st && st.retract) {
        st.x += (kx - st.x) * Math.min(1, dt * 14);
        st.y += (ky - st.y) * Math.min(1, dt * 14);
        if (Math.hypot(st.x - kx, st.y - ky) < 3) s.strand = null;
        else moving = true;
      }
      writeFrame();
      if (moving && !document.hidden) s.raf = requestAnimationFrame(tick);
      else s.raf = 0;
    },
    [writeFrame, kx, ky]
  );

  const kick = useCallback(() => {
    const s = sim.current;
    if (!s.raf) {
      s.last = 0;
      s.raf = requestAnimationFrame(tick);
    }
  }, [tick]);

  useEffect(() => () => cancelAnimationFrame(sim.current.raf), []);

  // Where the cap comes to rest: on the shelf, to the right of the jar, mostly on stage.
  const restPose = useCallback(() => {
    const svg = svgRef.current;
    const stage = svg?.closest('.pp-stage');
    if (!svg || !stage) return { x: g.W * 0.7, y: H - neck.hemC - 30, r: 7 };
    const sr = svg.getBoundingClientRect();
    const tr = stage.getBoundingClientRect();
    const scale = sr.width / g.W || 1;
    const cw = (g.R - g.L) * scale;
    // beside the jar, just touching it, unless the niche is too narrow
    const want = Math.min(sr.right + cw * 0.48, tr.right - cw * 0.54);
    const dx = (want - (sr.left + ccx * scale)) / scale;
    return { x: dx, y: H - 14 - neck.hemC, r: 8 };
  }, [g, ccx, neck.hemC]);

  const setOpenState = useCallback(
    (next, { fromPull = false } = {}) => {
      const s = sim.current;
      setOpen(next);
      onOpenChange?.(next);
      if (next) setEverOpened(true);
      if (reduced) {
        const rp = next ? restPose() : { x: 0, y: 0, r: 0 };
        s.x = { x: rp.x, v: 0 };
        s.y = { x: rp.y, v: 0 };
        s.r = { x: rp.r, v: 0 };
        s.loop = { x: next ? 1 : 0, v: 0 };
        s.target = { ...rp, loop: next ? 1 : 0 };
        s.strand = null;
        s.flutterT = -1;
        writeFrame();
        return;
      }
      if (next) {
        const rp = restPose();
        s.target = { x: rp.x, y: rp.y, r: rp.r, loop: 1 };
        s.flutterT = 0;
        s.x.v += 220;
        s.r.v -= 40;
        if (s.strand) {
          s.strand.falling = true;
          s.strand.vx = (s.strand.x - kx) * 1.5;
          s.strand.vy = -260;
        } else if (!fromPull) {
          // keyboard / tap: a quick visible tug on the end before it gives.
          s.strand = { x: kx - 120, y: ky + 150, vx: -260, vy: -320, falling: true, alpha: 1 };
        }
      } else {
        s.target = { x: 0, y: 0, r: 0, loop: 0 };
        s.flutterT = -1;
        s.strand = null;
      }
      kick();
    },
    [reduced, restPose, writeFrame, kick, kx, ky, onOpenChange]
  );

  const toggle = useCallback(() => setOpenState(!open), [open, setOpenState]);

  // Re-measure the rest pose on resize while open.
  useEffect(() => {
    if (!open) return undefined;
    const on = () => {
      const s = sim.current;
      const rp = restPose();
      s.target = { ...s.target, x: rp.x, y: rp.y };
      if (reduced) {
        s.x.x = rp.x;
        s.y.x = rp.y;
        writeFrame();
      } else kick();
    };
    window.addEventListener('resize', on);
    return () => window.removeEventListener('resize', on);
  }, [open, restPose, reduced, kick, writeFrame]);

  // Reset when the product changes.
  useEffect(() => {
    const s = sim.current;
    cancelAnimationFrame(s.raf);
    s.raf = 0;
    s.x = { x: 0, v: 0 }; s.y = { x: 0, v: 0 }; s.r = { x: 0, v: 0 }; s.loop = { x: 0, v: 0 };
    s.target = { x: 0, y: 0, r: 0, loop: 0 };
    s.strand = null;
    s.flutterT = -1;
    setOpen(false);
    setEverOpened(false);
    writeFrame();
  }, [pickle.id, writeFrame]);

  /* ---- pulling the twine ---- */
  const drag = useRef(null);
  const toSvg = (e) => {
    const svg = svgRef.current;
    const m = svg.getScreenCTM();
    if (!m) return { x: kx, y: ky };
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
    return { x: p.x, y: p.y };
  };

  const onPointerDown = (e) => {
    if (open) return; // a tap on the open jar closes via click handler
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const p = toSvg(e);
    drag.current = { id: e.pointerId, start: p, moved: 0, gave: false, startDist: Math.hypot(p.x - kx, p.y - ky) };
    setDragging(true);
    if (!reduced) {
      sim.current.strand = { x: kx, y: ky, alpha: 1 };
    }
  };
  const onPointerMove = (e) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId || d.gave) return;
    const p = toSvg(e);
    d.moved = Math.max(d.moved, Math.hypot(p.x - d.start.x, p.y - d.start.y));
    if (reduced) {
      if (d.moved > PULL_DIST * 0.6) {
        d.gave = true;
        setOpenState(true, { fromPull: true });
      }
      return;
    }
    const s = sim.current;
    const dx = p.x - kx, dy = p.y - ky;
    const dist = Math.hypot(dx, dy);
    const pull = Math.max(0, dist - d.startDist);
    const tension = Math.min(1, pull / PULL_DIST);
    s.strand = { x: p.x, y: p.y, alpha: 1 };
    // the cap leans into the pull a little, as if the knot were tightening.
    s.target = { x: dx * 0.04 * tension, y: -8 * tension, r: Math.max(-4, Math.min(4, dx * 0.012)) * tension, loop: 0 };
    kick();
    if (pull >= PULL_DIST && d.moved > 40) {
      d.gave = true;
      setOpenState(true, { fromPull: true });
    }
  };
  const onPointerUp = (e) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    drag.current = null;
    setDragging(false);
    if (d.gave) return;
    if (d.moved < 12) {
      // a tap on the bow unties it.
      sim.current.strand = null;
      setOpenState(true);
      return;
    }
    const s = sim.current;
    if (s.strand) s.strand.retract = true;
    s.target = { x: 0, y: 0, r: 0, loop: 0 };
    kick();
  };
  const onKnotKey = (e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      toggle();
    }
  };

  const mouthD = `M${neck.nL} ${neck.hemC + 24}L${neck.nL} ${neck.top}A${neck.rx} ${neck.ry} 0 0 1 ${neck.nR} ${neck.top}L${neck.nR} ${neck.hemC + 24}Z`;
  const thread1 = neck.top + (neck.hemC - neck.top) * 0.36;
  const thread2 = neck.top + (neck.hemC - neck.top) * 0.62;
  const loopY = neck.hemC + 14;
  const loopD = `M${g.L - 6} ${loopY - 10}C${g.L + (g.R - g.L) * 0.25} ${loopY + 34} ${g.L + (g.R - g.L) * 0.75} ${loopY + 34} ${g.R + 6} ${loopY - 10}`;
  // y on the loop's curve at the knot's x (for the little knot lump)
  const tK = Math.min(0.98, Math.max(0.02, (kx - g.L + 6) / (g.R - g.L + 12)));
  const knotLoopY = loopY - 10 + 3 * tK * (1 - tK) * 44;

  return (
    <div className={`pp-jar ${open ? 'is-open' : ''} ${dragging ? 'is-dragging' : ''}`} ref={stageRef} style={{ '--jar-aspect': `${g.W} / ${H}`, '--pickle': color }}>
      <svg
        ref={svgRef}
        className="pp-jar-svg"
        viewBox={`0 0 ${g.W} ${H}`}
        role="img"
        aria-label={open ? `${pickle.imageAlt}, kraft cover off` : pickle.imageAlt}
      >
        <defs>
          <filter id={`${uid}-feather`} x="-10%" y="-10%" width="120%" height="120%">
            <feGaussianBlur stdDeviation="1.6" />
          </filter>
          <filter id={`${uid}-steam`} x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="7" />
          </filter>
          <mask id={`${uid}-cover`} maskUnits="userSpaceOnUse" x="-400" y="-400" width={g.W + 800} height={H + 800}>
            <path d={coverPath(g)} fill="#fff" filter={`url(#${uid}-feather)`} />
          </mask>
          <mask id={`${uid}-body`} maskUnits="userSpaceOnUse" x="-400" y="-400" width={g.W + 800} height={H + 800}>
            <rect x="-400" y="-400" width={g.W + 800} height={H + 800} fill="#fff" />
            <path d={coverPath(g)} fill="#000" filter={`url(#${uid}-feather)`} />
          </mask>
          <mask id={`${uid}-alpha`} maskUnits="userSpaceOnUse" x="0" y="0" width={g.W} height={H} style={{ maskType: 'alpha' }}>
            <image href={pickle.image} x="0" y="0" width={g.W} height={H} />
          </mask>
          <linearGradient id={`${uid}-glass`} x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stopColor="#fff3dc" stopOpacity=".34" />
            <stop offset=".07" stopColor="#e8d6b4" stopOpacity=".1" />
            <stop offset=".2" stopColor="#fff8ea" stopOpacity=".42" />
            <stop offset=".3" stopColor="#e8d6b4" stopOpacity=".08" />
            <stop offset=".82" stopColor="#e8d6b4" stopOpacity=".08" />
            <stop offset=".94" stopColor="#fff3dc" stopOpacity=".3" />
            <stop offset="1" stopColor="#fff3dc" stopOpacity=".12" />
          </linearGradient>
          <radialGradient id={`${uid}-mouth`} cx=".5" cy=".35" r=".65">
            <stop offset="0" stopColor="#0d0704" />
            <stop offset=".6" stopColor="#24130a" />
            <stop offset="1" stopColor={color} stopOpacity=".9" />
          </radialGradient>
          <linearGradient id={`${uid}-inner`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#120a05" stopOpacity=".4" />
            <stop offset="1" stopColor="#120a05" stopOpacity="0" />
          </linearGradient>
          <linearGradient id={`${uid}-glint`} x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stopColor="#fff6dc" stopOpacity="0" />
            <stop offset=".45" stopColor="#fff6dc" stopOpacity=".55" />
            <stop offset=".55" stopColor="#ffe9a8" stopOpacity=".7" />
            <stop offset="1" stopColor="#fff6dc" stopOpacity="0" />
          </linearGradient>
        </defs>

        {/* drawn glass neck and mouth, hidden under the cap until it lifts */}
        <g className="pp-neck">
          <path d={mouthD} fill={`url(#${uid}-glass)`} />
          <rect x={neck.nL + 4} y={neck.top} width={neck.nR - neck.nL - 8} height={neck.ry * 3.2} fill={`url(#${uid}-inner)`} />
          <path d={`M${neck.nL} ${thread1}Q${neck.cx} ${thread1 + neck.ry * 2.2} ${neck.nR} ${thread1}`} fill="none" stroke="#fff" strokeOpacity=".4" strokeWidth="5" />
          <path d={`M${neck.nL} ${thread1 + 6}Q${neck.cx} ${thread1 + 6 + neck.ry * 2.2} ${neck.nR} ${thread1 + 6}`} fill="none" stroke="#3b2612" strokeOpacity=".22" strokeWidth="3" />
          <path d={`M${neck.nL} ${thread2}Q${neck.cx} ${thread2 + neck.ry * 2.2} ${neck.nR} ${thread2}`} fill="none" stroke="#fff" strokeOpacity=".32" strokeWidth="5" />
          <path d={`M${neck.nL} ${thread2 + 6}Q${neck.cx} ${thread2 + 6 + neck.ry * 2.2} ${neck.nR} ${thread2 + 6}`} fill="none" stroke="#3b2612" strokeOpacity=".2" strokeWidth="3" />
          <ellipse cx={neck.cx} cy={neck.top} rx={neck.rx - 6} ry={neck.ry - 2} fill={`url(#${uid}-mouth)`} />
          <ellipse cx={neck.cx} cy={neck.top} rx={neck.rx} ry={neck.ry} fill="none" stroke="#5a4128" strokeOpacity=".4" strokeWidth="6" />
          <ellipse cx={neck.cx} cy={neck.top - 2} rx={neck.rx - 2} ry={neck.ry - 1} fill="none" stroke="#fffaf0" strokeOpacity=".8" strokeWidth="3.5" />
          <rect x={neck.nL + (neck.nR - neck.nL) * 0.14} y={neck.top + neck.ry + 8} width="12" height={neck.hemC - neck.top - neck.ry - 4} rx="6" fill="#fff" opacity=".45" />
        </g>

        <image href={pickle.image} x="0" y="0" width={g.W} height={H} mask={`url(#${uid}-body)`} />

        {/* glint across the glass once the cap is off */}
        <g mask={`url(#${uid}-body)`} aria-hidden="true">
          <g mask={`url(#${uid}-alpha)`}>
            <g transform={`rotate(14 0 ${H / 2})`}>
              <rect className="pp-glint" x={-320} y={-200} width="220" height={H + 400} fill={`url(#${uid}-glint)`} style={{ '--glint-dist': `${g.W + 760}px` }} />
            </g>
          </g>
        </g>

        {/* the loosened twine loop resting on the shoulder */}
        <g ref={loopRef} style={{ opacity: 0 }} aria-hidden="true">
          <Strand d={loopD} width={12} />
          <ellipse cx={kx} cy={knotLoopY} rx="15" ry="11" fill="var(--twine)" stroke="var(--twine-dark)" strokeWidth="3" />
        </g>

        {/* aroma */}
        <g className="pp-steam" filter={`url(#${uid}-steam)`} aria-hidden="true">
          <path d={`M${neck.cx - 90} ${neck.top - 10}c-40 -60 50 -90 10 -160s40 -110 20 -180`} />
          <path d={`M${neck.cx + 10} ${neck.top - 14}c45 -55 -40 -100 5 -170s-30 -120 10 -200`} />
          <path d={`M${neck.cx + 110} ${neck.top - 8}c-35 -50 45 -80 5 -140s35 -90 15 -150`} />
        </g>

        {/* the cap */}
        <g ref={coverRef} className="pp-cover" onClick={open ? toggle : undefined}>
          <image href={pickle.image} x="0" y="0" width={g.W} height={H} mask={`url(#${uid}-cover)`} />
        </g>

        {/* the end being pulled */}
        <g ref={strandGroupRef} style={{ opacity: 0 }} aria-hidden="true">
          <Strand d={`M${kx} ${ky}L${kx} ${ky}`} width={16} innerRef={strandA} plyRef={strandB} />
        </g>

        {/* grab area on the bow (pointer); the real control is the button below the jar */}
        {!open && (
          <circle
            className="pp-knot-hit"
            cx={kx}
            cy={ky + 40}
            r="135"
            fill="transparent"
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={onPointerUp}
            onKeyDown={onKnotKey}
          />
        )}
      </svg>

      {!everOpened && (
        <span className="pp-pull-hint" aria-hidden="true" style={{ left: `${(kx / g.W) * 100}%`, top: `${((ky + 40) / H) * 100}%` }}>
          <span className="pp-pull-ring" />
        </span>
      )}
    </div>
  );
});

export default ProductJar;
