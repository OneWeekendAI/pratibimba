// A variant that lives in the repo as a real component. The site file's `gift` html is only
// the fallback for script and edge installs; this app renders the component.
export function GiftHero() {
  return (
    <section className="hero">
      <h1>Gift sets, wrapped and ready</h1>
      <p>Three roasts, a tasting card and a note in your words.</p>
      <a className="cta" href="#gifts">Shop gifts</a>
    </section>
  );
}
