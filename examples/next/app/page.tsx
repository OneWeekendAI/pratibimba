import { prb } from '@/lib/prb';
import { GiftHero } from './components/GiftHero';
import { Hero } from './components/Hero';

// Personalised, so never statically cached.
export const dynamic = 'force-dynamic';

export default function Home() {
  return (
    <>
      <nav>
        <b>Kaapi Co.</b>
        <span>Espresso</span>
        <span>Filter</span>
        <span>Gifts</span>
      </nav>
      <main>
        <prb.Slot id="hero" variants={{ gift: <GiftHero /> }}>
          <Hero />
        </prb.Slot>
        <prb.Slot id="promo">
          <p className="promo">Free shipping over ₹999.</p>
        </prb.Slot>
      </main>
      <footer>An example store for Pratibimba. Nothing here is for sale.</footer>
    </>
  );
}
