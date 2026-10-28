/*
 * Pickly shared motion API (frozen for page agents).
 *
 *   useReducedMotion()      -> boolean, reactive to the OS setting.
 *   prefersReducedMotion()  -> same, non-hook (for event handlers / plain JS).
 *   SPRING_SOFT / SPRING_SNAPPY / SPRING_HEAVY / SPRING_WOBBLE
 *                           -> transition objects for motion/react:
 *                              <motion.div animate={{ y: 0 }} transition={SPRING_SOFT} />
 *   stepSpring(s, target, cfg, dt)
 *                           -> one semi-implicit Euler step of a damped spring for
 *                              your own rAF loops. s = { x, v }; cfg = { k, c, m }.
 *                              Returns true while still moving (stop your loop on false).
 *
 *   flyToJar(sourceEl, { imageSrc, color })
 *     Clones the jar image at sourceEl's on-screen rect, arcs it (curved path, shrinking,
 *     a little tumble) into the header cart jar, then dispatches
 *       window 'pickly:jar-landed'  detail: { color }
 *     It also dispatches 'pickly:jar-launch' at take-off. The header jar holds its
 *     liquid level until the flight lands, so call addItem() first, then flyToJar().
 *     Reduced motion, missing source or hidden jar: no flight, the landed event fires
 *     on the next frame. Returns a Promise that resolves on landing.
 *
 *   Example (Shop's Add button is the reference integration):
 *     onClick={(e) => {
 *       const img = e.currentTarget.closest('.pickle-card')?.querySelector('img');
 *       addItem(pickle.id, 1);
 *       flyToJar(img, { imageSrc: pickle.image, color: pickleColor(pickle.id) });
 *     }}
 */
import { useEffect, useState } from 'react';

const RM_QUERY = '(prefers-reduced-motion: reduce)';

export function prefersReducedMotion() {
  if (typeof window === 'undefined' || !window.matchMedia) return false;
  return window.matchMedia(RM_QUERY).matches;
}

export function useReducedMotion() {
  const [reduced, setReduced] = useState(prefersReducedMotion);
  useEffect(() => {
    if (!window.matchMedia) return undefined;
    const mq = window.matchMedia(RM_QUERY);
    const on = () => setReduced(mq.matches);
    on();
    mq.addEventListener ? mq.addEventListener('change', on) : mq.addListener(on);
    return () => (mq.removeEventListener ? mq.removeEventListener('change', on) : mq.removeListener(on));
  }, []);
  return reduced;
}

/* Springs with real mass: small overshoot, nothing bouncy-for-its-own-sake. */
export const SPRING_SOFT = { type: 'spring', stiffness: 170, damping: 24, mass: 1 };
export const SPRING_SNAPPY = { type: 'spring', stiffness: 420, damping: 32, mass: 0.8 };
export const SPRING_HEAVY = { type: 'spring', stiffness: 120, damping: 20, mass: 1.8 };
export const SPRING_WOBBLE = { type: 'spring', stiffness: 260, damping: 9, mass: 0.6 };

export function stepSpring(s, target, { k = 170, c = 14, m = 1 } = {}, dt = 1 / 60) {
  const a = (-k * (s.x - target) - c * s.v) / m;
  s.v += a * dt;
  s.x += s.v * dt;
  return Math.abs(s.v) > 0.002 || Math.abs(s.x - target) > 0.002;
}

/* ---------- fly to jar ---------- */

let pending = 0;
export function jarFlightsPending() {
  return pending;
}

function land(color) {
  window.dispatchEvent(new CustomEvent('pickly:jar-landed', { detail: { color } }));
}

function findTarget() {
  const el = document.querySelector('[data-cart-jar-target]');
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.bottom < 0 || r.top > window.innerHeight) return null;
  return r;
}

const easeInOut = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

export function flyToJar(sourceEl, { imageSrc, color } = {}) {
  return new Promise((resolve) => {
    const done = () => {
      land(color);
      resolve();
    };
    const target = typeof document !== 'undefined' ? findTarget() : null;
    const src = sourceEl && sourceEl.getBoundingClientRect ? sourceEl.getBoundingClientRect() : null;
    const visible = src && src.width > 0 && src.bottom > 0 && src.top < window.innerHeight;
    const img = imageSrc || (sourceEl && sourceEl.currentSrc) || (sourceEl && sourceEl.src);
    if (prefersReducedMotion() || !target || !visible || !img || !document.body.animate) {
      requestAnimationFrame(done);
      return;
    }

    window.dispatchEvent(new CustomEvent('pickly:jar-launch', { detail: { color } }));
    pending += 1;

    // The image is a 984x1200 cutout; fit its aspect inside the source rect.
    const aspect = 984 / 1200;
    let w = src.width;
    let h = w / aspect;
    if (h > src.height) {
      h = src.height;
      w = h * aspect;
    }
    const x0 = src.left + src.width / 2;
    const y0 = src.top + src.height / 2;
    const x1 = target.left + target.width / 2;
    const y1 = target.top + target.height * 0.45;
    const endScale = Math.max(0.06, (target.width * 0.4) / w);

    const node = document.createElement('img');
    node.src = img;
    node.alt = '';
    node.setAttribute('aria-hidden', 'true');
    Object.assign(node.style, {
      position: 'fixed',
      left: `${x0 - w / 2}px`,
      top: `${y0 - h / 2}px`,
      width: `${w}px`,
      height: `${h}px`,
      zIndex: 9999,
      pointerEvents: 'none',
      willChange: 'transform, opacity',
      filter: 'drop-shadow(4px 10px 10px rgba(60,35,10,.35))',
    });
    document.body.appendChild(node);

    // Quadratic bezier: lift up and toward the jar, then drop in from above.
    // Control point sits up and to the near side of the jar so the clone
    // swings up, then drops in from above (and never leaves the viewport).
    const side = x1 >= x0 ? -1 : 1;
    const cx = x1 + side * Math.min(160, Math.abs(x1 - x0) * 0.3 + 40);
    const cy = Math.max(-4, y1 - 60);
    const N = 28;
    const frames = [];
    for (let i = 0; i <= N; i++) {
      const p = i / N;
      const t = easeInOut(p);
      const u = 1 - t;
      const x = u * u * x0 + 2 * u * t * cx + t * t * x1;
      const y = u * u * y0 + 2 * u * t * cy + t * t * y1;
      // a slight lift at take-off, then shrink hard on the way in
      const s = p < 0.12 ? 1 + 0.06 * (p / 0.12) : 1.06 + (endScale - 1.06) * Math.pow((p - 0.12) / 0.88, 1.4);
      const rot = Math.sin(p * Math.PI) * -14 + p * 6;
      frames.push({
        transform: `translate(${x - x0}px, ${y - y0}px) rotate(${rot}deg) scale(${s})`,
        opacity: p > 0.84 ? 1 - (p - 0.84) / 0.16 : 1,
      });
    }
    const dist = Math.hypot(x1 - x0, y1 - y0);
    const duration = Math.min(980, Math.max(620, 420 + dist * 0.45));
    const anim = node.animate(frames, { duration, easing: 'linear', fill: 'forwards' });
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      node.remove();
      pending = Math.max(0, pending - 1);
      done();
    };
    anim.onfinish = finish;
    anim.oncancel = finish;
    setTimeout(finish, duration + 400); // belt and braces (background tab)
  });
}
