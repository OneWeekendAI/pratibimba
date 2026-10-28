/*
 * Shelf physics for the hero ("knock the shelf").
 * Loaded lazily (dynamic import) so matter-js stays out of the home chunk.
 * matter-js only integrates; drawing is done by writing CSS transforms onto
 * the jar DOM nodes we are handed. No matter renderer.
 */
import Matter from 'matter-js';

const { Engine, Bodies, Body, Composite, Events } = Matter;

const SUBSTEPS = 2;
const STEP_MS = 1000 / 60 / SUBSTEPS;
const MAX_TILT = 0.42;       // rad; beyond this a righting torque pushes back
const SETTLE_FRAMES = 45;
const TIP_KNOCK = +(globalThis.__TK || 0.6);  // knock: angular kick scale (divided by sqrt of jar height)
const TIP_POINTER = +(globalThis.__TP || 1.3);
const RIPPLE = +(globalThis.__RP || 0.62);
const HOME_K = +(globalThis.__HK || 0.02);
const HOME_DAMP = +(globalThis.__HD || 0.86);

/**
 * @param {object} cfg
 *  jars:   [{ x, y, w, h, bw, bh, home }]  centre, image box, body box (px, stage coords)
 *  rows:   [{ plankY, x0, x1, ceilY }]
 *  els:    [{ jar: HTMLElement, tag: HTMLElement|null, shadow: HTMLElement|null }]
 *  glintLayer: HTMLElement (glints are appended here)
 *  rowOf:  jar index -> row index
 */
export function createShelfPhysics(cfg) {
  const { jars, rows, els, glintLayer, rowOf } = cfg;
  const engine = Engine.create({ gravity: { x: 0, y: 1, scale: 0.0032 }, enableSleeping: false });
  engine.positionIterations = 10;
  engine.velocityIterations = 8;

  const statics = [];
  // each plank's statics only collide with that plank's jars: a lower row's ceiling
  // would otherwise sit inside the upper row and hold its jars off the wood
  const rowCat = (r) => 2 << r;
  rows.forEach((r, k) => {
    const span = r.x1 - r.x0;
    const midX = (r.x0 + r.x1) / 2;
    const filt = { category: rowCat(k), mask: 1 };
    statics.push(Bodies.rectangle(midX, r.plankY + 20, span + 200, 40, { isStatic: true, friction: 1, collisionFilter: filt }));
    statics.push(Bodies.rectangle(midX, r.ceilY - 20, span + 200, 40, { isStatic: true, collisionFilter: filt }));
    const wallH = r.plankY - r.ceilY + 80;
    statics.push(Bodies.rectangle(r.x0 - 20, r.plankY - wallH / 2 + 40, 40, wallH, { isStatic: true, friction: 0.2, collisionFilter: filt }));
    statics.push(Bodies.rectangle(r.x1 + 20, r.plankY - wallH / 2 + 40, 40, wallH, { isStatic: true, friction: 0.2, collisionFilter: filt }));
  });

  const bodies = jars.map((j, i) =>
    Bodies.rectangle(j.x, j.y, j.bw, j.bh, {
      chamfer: { radius: [j.bw * 0.14, j.bw * 0.14, j.bw * 0.06, j.bw * 0.06] },
      friction: 0.85,
      frictionStatic: 1.2,
      frictionAir: 0.012,
      restitution: 0.04,
      density: 0.0024,
      slop: 0.02,
      label: 'jar' + i,
      collisionFilter: { category: 1, mask: 1 | rowCat(rowOf ? rowOf[i] : 0) },
    }),
  );
  Composite.add(engine.world, [...statics, ...bodies]);

  // tag pendulums (world angle a, angular velocity v) and per-jar bookkeeping
  const tags = jars.map(() => ({ a: 0, v: 0, prevW: 0 }));
  const lastTouch = jars.map(() => -1e9);
  let now = 0;

  Events.on(engine, 'beforeUpdate', () => {
    bodies.forEach((b, i) => {
      const a = b.angle;
      let w = Body.getAngularVelocity(b);
      // rotational damping: matter has no separate angular damping
      w *= 0.985;
      // righting torque past the tilt limit (gravity rights small tilts by itself)
      if (Math.abs(a) > MAX_TILT) w -= Math.sign(a) * (Math.abs(a) - MAX_TILT) * 0.06;
      // a hand that straightens a jar left leaning on its neighbour
      if (now - lastTouch[i] > 500) w -= a * 0.0025;
      Body.setAngularVelocity(b, w);
      // shuffle back toward home slot once nobody has touched it for a moment
      // (near-critically damped: a deliberate slide home, no overshoot, ~0.5s)
      if (now - lastTouch[i] > 700) {
        const v = Body.getVelocity(b);
        const dx = b.position.x - jars[i].home;
        if (Math.abs(dx) > 0.4 && Math.abs(a) < 0.12) {
          Body.setVelocity(b, { x: v.x * HOME_DAMP - dx * HOME_K, y: v.y });
        }
      }
    });
  });

  // clink glints: jar-on-jar contacts with some closing speed
  const lastGlint = new Map();
  const glintPool = [];
  for (let k = 0; k < 6; k++) {
    const g = document.createElement('span');
    g.className = 'hs-glint';
    g.innerHTML =
      '<svg viewBox="-14 -14 28 28" width="28" height="28"><path d="M0 -5 L0 -12 M-4.5 -3.5 L-9.5 -8.5 M4.5 -3.5 L9.5 -8.5 M-6 1 L-11 1 M6 1 L11 1" /></svg>';
    glintLayer.appendChild(g);
    glintPool.push(g);
  }
  let glintIdx = 0;
  function glint(x, y, s, flip) {
    const g = glintPool[glintIdx++ % glintPool.length];
    g.style.transform = `translate(${x - 14}px, ${y - 14}px) scaleX(${flip ? -1 : 1})`;
    g.getAnimations().forEach((an) => an.cancel());
    g.animate(
      [
        { opacity: 0, transform: `translate(${x - 14}px, ${y - 14}px) scale(${0.55 * (flip ? -1 : 1)}, .55)` },
        { opacity: Math.min(1, 0.7 + s), offset: 0.18 },
        { opacity: 0, transform: `translate(${x - 14}px, ${y - 14}px) scale(${1.15 * (flip ? -1 : 1)}, 1.15)` },
      ],
      { duration: 420, easing: 'cubic-bezier(.16,1,.3,1)' },
    );
  }
  Events.on(engine, 'collisionStart', (e) => {
    e.pairs.forEach((p) => {
      const A = p.bodyA, B = p.bodyB;
      if (A.isStatic || B.isStatic) return;
      const va = A.velocity, vb = B.velocity;
      const rel = Math.hypot(va.x - vb.x, va.y - vb.y) + Math.abs(A.angularVelocity - B.angularVelocity) * 40;
      if (rel < 0.9) return;
      const key = A.id < B.id ? A.id + ':' + B.id : B.id + ':' + A.id;
      if (now - (lastGlint.get(key) || -1e9) < 220) return;
      lastGlint.set(key, now);
      const sup = p.collision.supports && p.collision.supports[0];
      if (!sup) return;
      // the glint rises from the gap between the two lids, where it reads against the wall
      const y = Math.max(A.bounds.min.y, B.bounds.min.y) + 4;
      glint(sup.x, y, rel / 6, false);
    });
  });

  // --- frame loop -------------------------------------------------------
  let raf = 0;
  let running = false;
  let visible = true;
  let calm = 0;
  let lastT = 0;
  let acc = 0;

  function draw() {
    bodies.forEach((b, i) => {
      const j = jars[i];
      const x = b.position.x - j.w / 2;
      const y = b.position.y - j.h / 2;
      els[i].jar.style.transform = `translate3d(${x.toFixed(2)}px, ${y.toFixed(2)}px, 0) rotate(${b.angle.toFixed(4)}rad)`;
      const sh = els[i].shadow;
      if (sh) {
        const lift = Math.max(0, rows[rowOf[i]].plankY - b.bounds.max.y);
        const tilt = Math.min(1, Math.abs(b.angle) / 0.5);
        sh.style.transform = `translate3d(${(b.position.x - j.w / 2).toFixed(2)}px, 0, 0) scaleX(${(1 - tilt * 0.35).toFixed(3)})`;
        sh.style.opacity = String(Math.max(0, 1 - lift / 18 - tilt * 0.4).toFixed(3));
      }
      const t = tags[i];
      if (els[i].tag) els[i].tag.style.transform = `rotate(${(t.a - b.angle).toFixed(4)}rad)`;
    });
  }

  function stepTags() {
    bodies.forEach((b, i) => {
      const t = tags[i];
      const w = b.angularVelocity;
      // the tag hangs in world space; jar angular acceleration whips it
      t.v += -(w - t.prevW) * 0.9;
      t.prevW = w;
      t.v += (-0.012 * t.a - 0.05 * t.v);
      t.a += t.v;
      const local = t.a - b.angle;
      if (local > 0.9) t.a = b.angle + 0.9;
      if (local < -0.9) t.a = b.angle - 0.9;
    });
  }

  function isCalm() {
    for (let i = 0; i < bodies.length; i++) {
      const b = bodies[i];
      if (b.speed > 0.06 || b.angularSpeed > 0.0015) return false;
      if (Math.abs(tags[i].v) > 0.0015 || Math.abs(tags[i].a) > 0.01) return false;
      if (now - lastTouch[i] > 700 && Math.abs(b.position.x - jars[i].home) > 2.5 && Math.abs(b.angle) < 0.12) return false;
    }
    return true;
  }

  function frame(t) {
    raf = 0;
    if (!running) return;
    const dt = lastT ? Math.min(t - lastT, 34) : 1000 / 60;
    lastT = t;
    acc += dt;
    let n = 0;
    while (acc >= STEP_MS && n < SUBSTEPS * 3) {
      Engine.update(engine, STEP_MS);
      now += STEP_MS;
      acc -= STEP_MS;
      n++;
      if (n % SUBSTEPS === 0) stepTags();
    }
    draw();
    calm = isCalm() ? calm + 1 : 0;
    if (calm > SETTLE_FRAMES) {
      running = false; // settled: sleep until the next nudge
      return;
    }
    raf = requestAnimationFrame(frame);
  }

  function wake() {
    calm = 0;
    if (running || !visible) return;
    running = true;
    lastT = 0;
    acc = 0;
    raf = requestAnimationFrame(frame);
  }

  /**
   * rock jar i about its leading bottom corner: angular velocity dw (rad per
   * 60Hz frame, + = top moves right) with the matching centre velocity, so the
   * jar tips on its base instead of skating across the plank.
   */
  function tip(i, dw, lift = 0) {
    const b = bodies[i];
    const j = jars[i];
    const w = Body.getAngularVelocity(b) + dw;
    const v = Body.getVelocity(b);
    const sx = Math.sign(dw) || 1;
    // centre relative to the pivot corner
    const rx = -sx * j.bw / 2, ry = -j.bh / 2;
    Body.setAngularVelocity(b, w);
    Body.setVelocity(b, { x: v.x * 0.5 + -dw * ry, y: Math.min(v.y, dw * rx) - lift });
    lastTouch[i] = now;
  }

  /** soft invisible finger: pointer at (px,py) moving at (vx,vy) px/ms */
  function pointer(px, py, vx, vy, gain = 1) {
    let hit = false;
    const sp = Math.hypot(vx, vy);
    if (sp < 0.02) return;
    const vxc = Math.max(-2.5, Math.min(2.5, vx));
    bodies.forEach((b, i) => {
      const j = jars[i];
      const R = j.h * 0.3;
      const cx = Math.max(b.bounds.min.x, Math.min(px, b.bounds.max.x));
      const cy = Math.max(b.bounds.min.y, Math.min(py, b.bounds.max.y));
      const d = Math.hypot(px - cx, py - cy);
      if (d > R) return;
      const f = Math.pow(1 - d / R, 1.3);
      // touching near the lid rocks more than touching near the base
      const height = Math.max(0, Math.min(1, (b.bounds.max.y - py) / j.bh));
      const dw = vxc * f * (0.25 + 0.75 * height) * TIP_POINTER * gain / j.h;
      if (Math.abs(dw) < 1e-4) return;
      tip(i, dw);
      hit = true;
    });
    if (hit) wake();
  }

  /** a deliberate knock (click / tap / Enter). dir: -1 left, +1 right */
  function knock(i, dir = 1, strength = 1) {
    tip(i, (dir * TIP_KNOCK * strength) / Math.sqrt(jars[i].h));
    tags[i].v += dir * 0.07 * strength;
    wake();
  }

  /** opening ripple: a finger run along both planks, left to right, the lower plank half a beat behind */
  let timers = [];
  function ripple(delay = 0) {
    jars.forEach((j, i) => {
      const col = j.col != null ? j.col : i;
      const row = rowOf ? rowOf[i] : 0;
      timers.push(setTimeout(() => knock(i, 1, RIPPLE + (col % 2) * 0.06), delay + col * 120 + row * 70));
    });
  }

  function setVisible(v) {
    visible = v;
    if (!v) {
      running = false;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    } else {
      wake();
    }
  }

  function destroy() {
    running = false;
    if (raf) cancelAnimationFrame(raf);
    timers.forEach(clearTimeout);
    Events.off(engine);
    Composite.clear(engine.world, false);
    Engine.clear(engine);
    glintPool.forEach((g) => g.remove());
  }

  draw();
  return { pointer, knock, ripple, wake, setVisible, destroy, _engine: engine, _bodies: bodies, _isRunning: () => running };
}
