import { prb } from '@/lib/prb';

// The home page's own hero: the baseline of the `hero` slot. Its words are a copy slot
// (`hero_copy`, F4): agents propose a headline, subhead and button label as plain text, and the
// page renders them as text, never HTML. The fallback is the page's own copy.
export function Hero() {
  return (
    <prb.Copy
      id="hero_copy"
      fallback={{
        headline: 'Single-origin coffee, roasted on Monday',
        subhead: 'Small batches from Chikmagalur, shipped the day they rest.',
        cta_label: 'Shop espresso',
      }}
    >
      {(t) => (
        <section className="hero">
          <h1>{t.headline}</h1>
          <p>{t.subhead}</p>
          <a className="cta" href="/p/monday-espresso">{t.cta_label}</a>
        </section>
      )}
    </prb.Copy>
  );
}
