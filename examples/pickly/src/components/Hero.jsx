import { SHOP } from '../data/shop';
import HeroShelf from './HeroShelf';

export default function Hero() {
  return (
    <section id="hero" className="hs" aria-labelledby="hs-title">
      <div className="hs-wall" aria-hidden="true" />
      <HeroShelf>
          <p className="hs-eyebrow">Made in {SHOP.madeIn}</p>
          <h1 id="hs-title" className="hs-title">
            <span className="hs-line">Small-batch</span>{' '}
            <span className="hs-line"><em>Andhra</em> pickles</span>
          </h1>
          <p className="hs-tagline">{SHOP.tagline}</p>
          <a className="hs-cta" href="#/shop">
            Shop pickles <span className="hs-cta-arrow" aria-hidden="true">→</span>
          </a>
      </HeroShelf>
    </section>
  );
}
