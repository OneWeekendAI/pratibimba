import { ArrowHand } from './icons';

/* An empty glass jar, lid set down beside it. Drawn, not a stock icon. */
function EmptyJar() {
  return (
    <svg viewBox="0 0 250 200" className="pe-jar" aria-hidden="true" focusable="false">
      <ellipse cx="104" cy="184" rx="74" ry="7" fill="rgba(60,35,10,.16)" />
      {/* glass body */}
      <path
        d="M66 52c-1-8 3-12 10-12h56c7 0 11 4 10 12l-2 6c14 8 22 22 22 40v62c0 13-9 22-22 22H68c-13 0-22-9-22-22V98c0-18 8-32 22-40z"
        fill="var(--glass-tint)"
        stroke="var(--glass-edge)"
        strokeWidth="2.2"
        strokeLinejoin="round"
      />
      {/* neck rim */}
      <path d="M70 58c22 3 46 3 68 0" fill="none" stroke="var(--glass-edge)" strokeWidth="1.6" opacity=".7" />
      {/* the last smear of oil at the bottom */}
      <path d="M52 168c16 5 88 5 104 0-2 8-9 13-18 13H70c-9 0-16-5-18-13z" fill="var(--oil)" opacity=".28" />
      {/* shine */}
      <path d="M60 104c0-14 5-24 14-30" fill="none" stroke="var(--glass-shine)" strokeWidth="5" strokeLinecap="round" />
      <path d="M60 122v28" stroke="var(--glass-shine)" strokeWidth="5" strokeLinecap="round" opacity=".7" />
      {/* the kraft lid, set down on its side */}
      <g transform="translate(206 170) rotate(-10)">
        <ellipse cx="0" cy="12" rx="30" ry="8" fill="var(--kraft-light)" stroke="var(--twine-dark)" strokeWidth="1.6" />
        <path d="M-30 12c0-12 13-22 30-22s30 10 30 22" fill="var(--kraft)" stroke="var(--twine-dark)" strokeWidth="1.6" />
        <path d="M-26 6c16 6 36 6 52 0" fill="none" stroke="var(--twine)" strokeWidth="2.4" strokeLinecap="round" />
      </g>
    </svg>
  );
}

export default function ParcelEmpty({ title = 'Your jar is empty', line }) {
  return (
    <div className="pe wrap">
      <EmptyJar />
      <h1 className="h-display pe-title">{title}</h1>
      {line ? <p className="pe-line">{line}</p> : null}
      <a href="#/shop" className="pc-cta pe-cta">
        <span>Back to the shelf</span>
        <ArrowHand size={20} aria-hidden="true" />
      </a>
    </div>
  );
}
