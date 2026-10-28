import { useRef } from 'react';
import { motion } from 'motion/react';
import { useReducedMotion } from '../lib/motion';
import { Leaf, Drumstick, Jar } from './icons';

/* A pantry slider: a recessed teak track with a cream block that slides between three stops. */
const KNOB = { type: 'spring', stiffness: 360, damping: 28, mass: 1.1 };

export default function ShelfFilter({ value, onChange, counts }) {
  const reduced = useReducedMotion();
  const refs = useRef([]);
  const opts = [
    { v: 'all', label: 'All', Icon: Jar, n: counts.all },
    { v: 'veg', label: 'Veg', Icon: Leaf, n: counts.veg },
    { v: 'nonveg', label: 'Non-veg', Icon: Drumstick, n: counts.nonveg },
  ];
  const at = opts.findIndex((o) => o.v === value);

  const onKeyDown = (e) => {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    let next = null;
    if (step) next = (at + step + opts.length) % opts.length;
    if (e.key === 'Home') next = 0;
    if (e.key === 'End') next = opts.length - 1;
    if (next === null) return;
    e.preventDefault();
    onChange(opts[next].v);
    refs.current[next]?.focus();
  };

  return (
    <div className="shelf-filter" role="radiogroup" aria-label="Show jars" onKeyDown={onKeyDown}>
      {opts.map((o, i) => {
        const on = o.v === value;
        return (
          <button
            key={o.v}
            ref={(el) => (refs.current[i] = el)}
            type="button"
            role="radio"
            aria-checked={on}
            tabIndex={on ? 0 : -1}
            className={`shelf-filter-stop${on ? ' is-on' : ''} is-${o.v}`}
            onClick={() => onChange(o.v)}
          >
            {on && (
              <motion.span
                layoutId="shelf-filter-knob"
                className="shelf-filter-knob"
                transition={reduced ? { duration: 0 } : KNOB}
                aria-hidden="true"
              />
            )}
            <span className="shelf-filter-text">
              <o.Icon size={16} />
              <span>{o.label}</span>
              <span className="shelf-filter-count">{o.n}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
