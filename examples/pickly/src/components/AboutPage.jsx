import { PICKLES, SHOP } from '../data/shop';
import { inr } from '../lib/helpers';
import { pickleColor } from '../lib/pickleColors';
import { Leaf, Drumstick, Parcel, Mail, ChilliSolid } from './icons';

const veg = PICKLES.filter((p) => p.veg);
const nonVeg = PICKLES.filter((p) => !p.veg);
const weights = [...new Set(PICKLES.map((p) => p.weightGrams))];

function LedgerRow({ p }) {
  return (
    <li className="ledger-row">
      <a href={`#/product/${p.id}`}>
        <span className="ledger-swatch" style={{ background: pickleColor(p.id) }} aria-hidden="true" />
        <span className="ledger-name">{p.nativeName}</span>
        <span className="ledger-dots" aria-hidden="true" />
        <span className="ledger-heat" aria-label={`Spice ${p.spice} of 5`}>
          {[1, 2, 3, 4, 5].map((i) => (
            <ChilliSolid key={i} size={13} className={i > p.spice ? 'ledger-heat-off' : undefined} />
          ))}
        </span>
        <span className="ledger-price">{inr(p.price)}</span>
      </a>
    </li>
  );
}

export default function AboutPage() {
  return (
    <div className="about-page wrap">
      <header className="about-head">
        <span className="eyebrow">About {SHOP.name}</span>
        <h1 className="h-display about-title">
          The pantry ledger
        </h1>
        <p className="about-lede">{SHOP.tagline}. Made in {SHOP.madeIn}.</p>
      </header>

      <div className="about-book">
        <section className="ledger-page" aria-labelledby="ledger-veg">
          <h2 id="ledger-veg" className="ledger-head">
            <Leaf size={20} /> Veg <small>{veg.length} jars</small>
          </h2>
          <ul>{veg.map((p) => <LedgerRow key={p.id} p={p} />)}</ul>
        </section>
        <section className="ledger-page" aria-labelledby="ledger-nonveg">
          <h2 id="ledger-nonveg" className="ledger-head">
            <Drumstick size={20} /> Non-veg <small>{nonVeg.length} jars</small>
          </h2>
          <ul>{nonVeg.map((p) => <LedgerRow key={p.id} p={p} />)}</ul>
        </section>
      </div>

      <div className="about-tags">
        <p className="about-tag">
          <span className="about-tag-k">Jar</span>
          {weights.map((w) => `${w} g`).join(" or ")} each
        </p>
        <p className="about-tag">
          <Parcel size={20} />
          {SHOP.shippingNote}
        </p>
        <p className="about-tag">
          <Mail size={20} />
          <a href={`mailto:${SHOP.supportEmail}`}>{SHOP.supportEmail}</a>
        </p>
      </div>

      <p className="about-cta">
        <a href="#/shop" className="btn btn-ink">To the shelf</a>
      </p>
    </div>
  );
}
