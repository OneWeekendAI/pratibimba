import { useEffect, useRef } from 'react';
import { motion, useMotionValue, useTransform, animate } from 'motion/react';
import { addItem } from '../lib/cart';
import { inr } from '../lib/helpers';
import { flyToJar, useReducedMotion, SPRING_WOBBLE } from '../lib/motion';
import { pickleColor } from '../lib/pickleColors';
import { Leaf, Drumstick, ChilliSolid, Plus } from './icons';

/* Lift: quick, firm, barely overshoots. Settle: heavier, lets the glass rock once or twice. */
const LIFT = { type: 'spring', stiffness: 380, damping: 24, mass: 0.9 };
const TILT = { type: 'spring', stiffness: 220, damping: 18, mass: 0.8 };
const SETTLE = { type: 'spring', stiffness: 320, damping: 13, mass: 1 };
const LIFT_Y = -16;

/* Labels are tacked on by hand: each one leans a little differently. */
const LEANS = [-1.6, 0.9, -0.4, 1.4, -1.1, 0.5, -1.8, 1.1, -0.7, 1.7];

export default function ShelfJar({ pickle, index = 0, back = false, pulse = 0, headingLevel = 3 }) {
  const reduced = useReducedMotion();
  const y = useMotionValue(0);
  const x = useMotionValue(0);
  const rotate = useMotionValue(0);
  const chit = useMotionValue(0);
  const shadowScale = useTransform(y, [LIFT_Y, 0], [0.66, 1]);
  const shadowOpacity = useTransform(y, [LIFT_Y, 0], [0.38, 0.9]);
  const lifted = useRef(false);
  const stepRef = useRef(null);
  const Heading = `h${headingLevel}`;

  const lift = (dx = 0) => {
    if (reduced) return;
    lifted.current = true;
    animate(y, LIFT_Y, LIFT);
    animate(rotate, dx * 4.5, TILT);
    animate(x, dx * 3, TILT);
  };
  const settle = () => {
    lifted.current = false;
    if (reduced) return;
    animate(y, 0, SETTLE);
    animate(rotate, 0, SPRING_WOBBLE);
    animate(x, 0, SETTLE);
  };
  const lean = (e) => {
    if (reduced || e.pointerType !== 'mouse') return;
    const r = e.currentTarget.getBoundingClientRect();
    const dx = Math.max(-1, Math.min(1, ((e.clientX - r.left) / r.width) * 2 - 1));
    if (!lifted.current) lift(dx);
    else {
      animate(rotate, dx * 4.5, TILT);
      animate(x, dx * 3, TILT);
    }
  };

  // When the filter brings this jar forward, it steps out of the shadow with a small nudge.
  useEffect(() => {
    if (!pulse || back || reduced || !stepRef.current?.animate) return;
    stepRef.current.animate(
      [
        { transform: 'translateY(0) scale(1)' },
        { transform: 'translateY(-5px) scale(1.035)', offset: 0.4 },
        { transform: 'translateY(0) scale(1)' },
      ],
      { duration: 560, delay: 90 + index * 34, easing: 'cubic-bezier(.3,1.18,.55,1)' }
    );
  }, [pulse]); // eslint-disable-line react-hooks/exhaustive-deps

  // A dimmed jar must not stay lifted.
  useEffect(() => {
    if (back) settle();
  }, [back]); // eslint-disable-line react-hooks/exhaustive-deps

  const onAdd = (e) => {
    const img = e.currentTarget.closest('.pickle-card')?.querySelector('img');
    addItem(pickle.id, 1);
    flyToJar(img, { imageSrc: pickle.image, color: pickleColor(pickle.id) });
    if (!reduced) {
      chit.set(-14);
      animate(chit, 0, SPRING_WOBBLE);
    }
  };

  const off = back ? -1 : undefined;

  return (
    <article
      className={`pickle-card shelf-slot${back ? ' is-back' : ''}`}
      style={{ '--i': index, '--lean': `${LEANS[index % LEANS.length]}deg` }}
      aria-hidden={back || undefined}
    >
      <div className="shelf-slot-inner">
        <a
          href={`#/product/${pickle.id}`}
          className="shelf-jar-link"
          tabIndex={off}
          aria-label={`${pickle.name}, ${inr(pickle.price)} for ${pickle.weightGrams} g`}
          onPointerEnter={lean}
          onPointerMove={lean}
          onPointerLeave={settle}
          onPointerDown={(e) => e.pointerType !== 'mouse' && lift(0)}
          onPointerUp={(e) => e.pointerType !== 'mouse' && settle()}
          onPointerCancel={settle}
          onFocus={() => lift(0)}
          onBlur={settle}
        >
          <motion.span
            className="shelf-jar-shadow"
            aria-hidden="true"
            style={reduced ? undefined : { scaleX: shadowScale, opacity: shadowOpacity }}
          />
          <span className="shelf-jar-step" ref={stepRef}>
            <motion.img
              className="shelf-jar-img"
              src={pickle.image}
              alt={pickle.imageAlt}
              width="984"
              height="1200"
              loading="lazy"
              draggable="false"
              style={{ x, y, rotate }}
            />
          </span>
        </a>

        <div className="shelf-plank" aria-hidden="true" />

        <div className="shelf-label">
          <span className={`shelf-label-diet ${pickle.veg ? 'is-veg' : 'is-nonveg'}`}>
            {pickle.veg ? <Leaf size={13} /> : <Drumstick size={13} />}
            <span>{pickle.veg ? 'Veg' : 'Non-veg'}</span>
          </span>
          <Heading className="shelf-label-native">{pickle.nativeName}</Heading>
          <p className="shelf-label-name">{pickle.name}</p>
          <p className="shelf-label-price">
            <span className="shelf-label-rupees">{inr(pickle.price)}</span>
            <span className="shelf-label-weight">{pickle.weightGrams} g</span>
          </p>
          <span className="shelf-label-heat" role="img" aria-label={`Spice ${pickle.spice} of 5`}>
            {[1, 2, 3, 4, 5].map((n) => (
              <ChilliSolid key={n} size={11} className={n <= pickle.spice ? 'is-hot' : ''} />
            ))}
          </span>
        </div>

        <motion.button
          type="button"
          className="shelf-chit"
          style={{ rotate: chit }}
          tabIndex={off}
          onClick={onAdd}
          aria-label={`Add ${pickle.name} to cart`}
        >
          <Plus size={13} strokeWidth={2.2} />
          <span>jar</span>
        </motion.button>
      </div>
    </article>
  );
}
