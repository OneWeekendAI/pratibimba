/*
 * Quantity as a little teak tray with six slots: each jar you add drops into a slot.
 * Minus / Plus are brass knobs at the tray ends. 1..6 (cart.js caps a jar at 6).
 */
import { AnimatePresence, motion } from 'motion/react';
import { Minus, Plus } from './icons';
import { SPRING_SNAPPY } from '../lib/motion';

export const TRAY_MAX = 6;

export default function ProductTray({ qty, setQty, image, name, reduced }) {
  const dec = () => setQty((q) => Math.max(1, q - 1));
  const inc = () => setQty((q) => Math.min(TRAY_MAX, q + 1));
  return (
    <div className="pp-tray" role="group" aria-label="Quantity">
      <button type="button" className="pp-knob" onClick={dec} disabled={qty <= 1} aria-label="One jar fewer">
        <Minus size={18} strokeWidth={2.2} />
      </button>
      <div className="pp-tray-bed" aria-hidden="true">
        {Array.from({ length: TRAY_MAX }).map((_, i) => (
          <span className="pp-slot" key={i}>
            <AnimatePresence initial={false}>
              {i < qty && (
                <motion.img
                  key="jar"
                  src={image}
                  alt=""
                  className="pp-slot-jar"
                  initial={reduced ? false : { y: -34, scale: 0.92, opacity: 0, rotate: i % 2 ? 6 : -6 }}
                  animate={{ y: 0, scale: 1, opacity: 1, rotate: 0 }}
                  exit={reduced ? { opacity: 0, transition: { duration: 0 } } : { y: -22, opacity: 0, scale: 0.9, transition: { duration: 0.18 } }}
                  transition={reduced ? { duration: 0 } : SPRING_SNAPPY}
                />
              )}
            </AnimatePresence>
          </span>
        ))}
      </div>
      <button type="button" className="pp-knob" onClick={inc} disabled={qty >= TRAY_MAX} aria-label="One more jar">
        <Plus size={18} strokeWidth={2.2} />
      </button>
      <output className="pp-tray-count" aria-live="polite">
        {qty} {qty === 1 ? 'jar' : 'jars'}
        <span className="sr-only"> of {name}</span>
      </output>
    </div>
  );
}
