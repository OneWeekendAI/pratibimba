import Hero from './Hero';
import { PICKLES, SHOP } from '../data/shop';
import { ArrowHand } from './icons';

const [SHIP_PRICE, SHIP_TIME] = SHOP.shippingNote.split('. ').map((s) => s.replace(/\.$/, ''));

/* The hero already holds every jar, so the page closes on the shop's facts:
   a kraft luggage tag hung on a nail, and the way into the full shelf. */
export default function Home() {
  return (
    <div className="home-page">
      <Hero />

      <section className="home-close" aria-labelledby="home-close-title">
        <div className="wrap home-close-inner">
          <div className="home-close-copy">
            <h2 id="home-close-title" className="h-display home-close-title">
              {PICKLES.length} jars, one shelf
            </h2>
            <a href="#/shop" className="shelf-home-all home-close-link">
              <span>Browse the whole shelf</span>
              <ArrowHand size={22} />
            </a>
          </div>

          <div className="home-close-hang">
            <span className="home-close-nail" aria-hidden="true" />
            <svg className="home-close-twine" viewBox="0 0 120 64" aria-hidden="true">
              <path d="M60 4 C 46 22, 46 46, 60 62" />
              <path d="M60 4 C 74 22, 74 46, 60 62" />
            </svg>
            <ul className="home-close-tag" aria-label="About every parcel">
              <li>Made in {SHOP.madeIn}</li>
              <li>Every jar {PICKLES[0].weightGrams} g</li>
              <li>{SHIP_PRICE}</li>
              <li>{SHIP_TIME}</li>
            </ul>
          </div>
        </div>
      </section>
    </div>
  );
}
