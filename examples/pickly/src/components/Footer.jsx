import { SHOP } from '../data/shop';
import { Mail } from './icons';

export default function Footer() {
  return (
    <footer className="footer">
      <div className="footer-plank" aria-hidden="true" />
      <div className="wrap footer-inner">
        <div className="footer-brand">
          <span className="footer-name">{SHOP.name}</span>
          <span className="footer-tagline">{SHOP.tagline}</span>
        </div>
        <nav className="footer-links" aria-label="Footer">
          <a href="#/shop">Shop</a>
          <a href="#/about">About</a>
          <a href="#/cart">Cart</a>
        </nav>
        <div className="footer-note">
          <p>{SHOP.shippingNote}</p>
          <p>
            <Mail size={16} /> <a href={`mailto:${SHOP.supportEmail}`}>{SHOP.supportEmail}</a>
          </p>
        </div>
      </div>
      <div className="wrap footer-bottom">
        <span>© {new Date().getFullYear()} {SHOP.name} · {SHOP.madeIn}</span>
      </div>
    </footer>
  );
}
