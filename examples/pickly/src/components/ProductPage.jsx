import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { PICKLES, SHOP } from '../data/shop';
import { inr } from '../lib/helpers';
import { addItem } from '../lib/cart';
import { flyToJar, useReducedMotion } from '../lib/motion';
import { pickleColor } from '../lib/pickleColors';
import { WaxSeal, SpiceGauge } from './Marks';
import ProductJar, { JAR_GEOMETRY } from './ProductJar';
import ProductTray from './ProductTray';
import { Back, Cycle, Twine, Lid, Check } from './icons';

export default function ProductPage({ productId }) {
  const pickle = PICKLES.find((p) => p.id === productId);
  const reduced = useReducedMotion();
  const [qty, setQty] = useState(1);
  const [open, setOpen] = useState(false);
  const [added, setAdded] = useState(0);
  const jarRef = useRef(null);
  const nicheRef = useRef(null);
  const wordRef = useRef(null);

  // The ghost native name: scaled so the whole word fits the niche, never cut mid-word.
  useLayoutEffect(() => {
    const niche = nicheRef.current;
    const word = wordRef.current;
    if (!niche || !word) return undefined;
    const fit = () => {
      word.style.fontSize = '';
      const base = parseFloat(getComputedStyle(word).fontSize) || 100;
      const avail = niche.clientWidth * 0.92;
      const w = word.scrollWidth;
      if (w > avail) word.style.fontSize = `${Math.max(28, (base * avail) / w).toFixed(1)}px`;
    };
    fit();
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(fit) : null;
    ro?.observe(niche);
    document.fonts?.ready?.then(fit);
    return () => ro?.disconnect();
  }, [productId]);

  useEffect(() => {
    setQty(1);
    setOpen(false);
    setAdded(0);
    if (!pickle) window.location.hash = '#/shop';
  }, [productId, pickle]);

  useEffect(() => {
    if (!added) return undefined;
    const t = setTimeout(() => setAdded(0), 2200);
    return () => clearTimeout(t);
  }, [added]);

  if (!pickle) return null;

  const color = pickleColor(pickle.id);
  const idx = PICKLES.indexOf(pickle);
  const more = [1, 2, 3].map((k) => PICKLES[(idx + k) % PICKLES.length]);

  const handleAdd = () => {
    addItem(pickle.id, qty);
    // Fly from the big jar when it is on screen; on a phone it has usually scrolled
    // away by now, so the last jar in the tray takes off instead.
    const big = jarRef.current?.el;
    const r = big?.getBoundingClientRect();
    const bigVisible = r && r.top > 40 && r.bottom < window.innerHeight + r.height * 0.3;
    const trayJars = document.querySelectorAll('.pp-slot-jar');
    const src = bigVisible ? big : trayJars[trayJars.length - 1] || big;
    flyToJar(src, { imageSrc: pickle.image, color });
    setAdded(qty);
  };

  return (
    <div className={`pp ${reduced ? 'pp-calm' : ''}`} style={{ '--pickle': color }}>
      <div className="pp-wrap">
        <a href="#/shop" className="pp-back">
          <Back size={18} /> Back to the shelf
        </a>
      </div>

      <div className="pp-wrap pp-grid">
        <section className="pp-stage" aria-label={`${pickle.name} jar`}>
          <div className="pp-niche" ref={nicheRef}>
          <span className="pp-bigword" aria-hidden="true" ref={wordRef}>{pickle.nativeName}</span>
          <div className="pp-light" aria-hidden="true" />
          {/* every photo is 1200 px tall: size the slot by photo width so all jars share one scale */}
          <div className="pp-jar-slot" style={{ '--jw': ((JAR_GEOMETRY[pickle.id]?.W || 984) / 1177).toFixed(3) }}>
            <ProductJar ref={jarRef} pickle={pickle} reduced={reduced} color={color} onOpenChange={setOpen} />
          </div>
          <div className="pp-plank">
            <div className="pp-plank-top" aria-hidden="true" />
            <div className="pp-plank-face">
              <div className="pp-seal-wrap">
                <WaxSeal veg={pickle.veg} size={64} className={reduced ? '' : 'is-pressing'} />
                <span className="pp-seal-label">{pickle.veg ? 'Veg' : 'Non-veg'}</span>
              </div>
              <div className="pp-gauge-wrap">
                <SpiceGauge level={pickle.spice} />
                <span className="pp-gauge-label">Spice {pickle.spice}/5</span>
              </div>
            </div>
          </div>
          </div>
          <div className="pp-stage-controls">
            <button
              type="button"
              className="pp-untie"
              aria-pressed={open}
              onClick={() => jarRef.current?.toggle()}
            >
              {open ? <Lid size={20} /> : <Twine size={20} />}
              {open ? 'Tie the cover back' : 'Untie the twine'}
            </button>
            {!open && <span className="pp-stage-hint" aria-hidden="true">or pull the loose end</span>}
          </div>
        </section>

        <article className="pp-info">
          <p className="pp-kicker">{pickle.nativeName}</p>
          <h1 className="pp-title">{pickle.name}</h1>
          <p className="pp-blurb">{pickle.longBlurb}</p>

          {pickle.eatWith && (
            <aside className="pp-note" aria-label="Eat it with">
              <span className="pp-note-pin" aria-hidden="true" />
              <span className="pp-note-label">Eat it with</span>
              <span className="pp-note-text">{pickle.eatWith}</span>
            </aside>
          )}

          <div className="pp-buy">
            <div className="pp-price-row">
              <span className="pp-price">{inr(pickle.price)}</span>
              <span className="pp-weight">{pickle.weightGrams} g jar</span>
            </div>
            <ProductTray qty={qty} setQty={setQty} image={pickle.image} name={pickle.name} reduced={reduced} />
            <button type="button" className={`pp-add ${added ? 'is-added' : ''}`} onClick={handleAdd}>
              {added ? (
                <>
                  <Check size={20} strokeWidth={2.2} /> {added === 1 ? 'Jar' : `${added} jars`} in your cart
                </>
              ) : (
                <>
                  Add {qty === 1 ? 'the jar' : `${qty} jars`}
                  <span className="pp-add-sum">{inr(pickle.price * qty)}</span>
                </>
              )}
            </button>
            <p className="pp-ship">
              <Cycle size={22} /> {SHOP.shippingNote}
            </p>
          </div>
        </article>
      </div>

      <section className="pp-wrap pp-more" aria-labelledby="pp-more-h">
        <h2 id="pp-more-h" className="pp-more-h">More from the shelf</h2>
        <ul className="pp-more-row">
          {more.map((p) => (
            <li key={p.id}>
              <a href={`#/product/${p.id}`} className="pp-more-jar">
                <img src={p.image} alt="" loading="lazy" />
                <span className="pp-more-name">{p.name}</span>
                <span className="pp-more-price">{inr(p.price)}</span>
              </a>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
