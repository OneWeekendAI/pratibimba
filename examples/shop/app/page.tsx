import { prb } from '@/lib/prb';
import { FilterHero } from './components/FilterHero';
import { Hero } from './components/Hero';
import { PriceHero } from './components/PriceHero';
import { TastingHero } from './components/TastingHero';
import { products } from '@/lib/products';

// Personalised, so never statically cached.
export const dynamic = 'force-dynamic';

export default function Home() {
  return (
    <>
      <prb.Slot
        id="hero"
        variants={{ 'filter-angle': <FilterHero />, 'tasting-notes': <TastingHero />, 'price-upfront': <PriceHero /> }}
      >
        <Hero />
      </prb.Slot>
      <div className="grid">
        {/* A sorted slot: Pratibimba learns which order of these cards gets clicked. Each
            card's key is its item key in site.json; a click on a card reports that card. */}
        <prb.Sorted id="grid">
          {products.map((p) => (
            <a key={p.slug} className="card" href={`/p/${p.slug}`} style={{ color: 'inherit', textDecoration: 'none' }}>
              <h2>{p.name}</h2>
              <p>{p.blurb}</p>
              <span className="price">₹{p.price}</span>
            </a>
          ))}
        </prb.Sorted>
      </div>
    </>
  );
}
