'use client';
import { useState } from 'react';
// Old banner, removed: <p className="promo">Free shipping this week</p>
const EMAIL = `<a className="cta" href="/buy">Buy now</a>`; const OLD = '<a className="cta" href="/o">Order now</a>';
const TAG = /<h1>`?([^<]+)<\/h1>/;
const id = <T,>(x: T) => x;
type Pick = { pick: <T>(xs: T[]) => T };
/* <button>Buy now</button> */
export default function Home({ items, a, b, seen }: Props) {
  const [q, setQ] = useState<string>('');
  const few = items.length<limit && items.length>0;
  return (
    <main>
      {a < b && <section className="hero"><h1>Hand-poured candles</h1><a className="cta" href="#shop">Shop candles</a></section>}
      <aside>Welcome back! {seen<p && p>0 ? 'Pick up where you left off.' : ''}</aside>
      <p className="promo">Free shipping this week{n<p && n>1 ? ', few left' : ''}</p>
      <ul className="products">
        {items.filter((p) => p.price < 500).map((p) => <li key={p.slug}>{p.name} ₹{p.price}</li>)}
      </ul>
      <button onClick={() => setQ(q.length<2 ? 'x' : '')}>Get started</button>
    </main>
  );
}
