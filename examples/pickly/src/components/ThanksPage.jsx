import { useId, useMemo } from 'react';
import { SHOP } from '../data/shop';
import { useReducedMotion } from '../lib/motion';
import { Back } from './icons';
import ParcelBundle from './ParcelBundle';

const MONTHS = ['JAN', 'FEB', 'MAR', 'APR', 'MAY', 'JUN', 'JUL', 'AUG', 'SEP', 'OCT', 'NOV', 'DEC'];

/* A round postmark: ring text from shop.js, today's date, wavy cancel lines. */
function PostmarkStamp({ date }) {
  const uid = useId().replace(/:/g, '');
  const ring = `pm-ring-${uid}`;
  const rough = `pm-rough-${uid}`;
  const word = `${SHOP.name} · ${SHOP.madeIn} · `.toUpperCase();
  const ringText = word + word;
  const d = `${String(date.getDate()).padStart(2, '0')} ${MONTHS[date.getMonth()]} ${date.getFullYear()}`;

  const art = (
    <>
      <circle cx="100" cy="100" r="88" fill="none" strokeWidth="4" />
      <circle cx="100" cy="100" r="58" fill="none" strokeWidth="2.4" />
      <text className="pm-ring-text" fontSize="15.5">
        <textPath href={`#${ring}`} startOffset="0" textLength="440" lengthAdjust="spacing">{ringText}</textPath>
      </text>
      <path d="M58 100h84" strokeWidth="1.6" />
      <text x="100" y="94" textAnchor="middle" className="pm-date" fontSize="16.5">{d}</text>
      <text x="100" y="121" textAnchor="middle" className="pm-small" fontSize="11">POSTED</text>
      <g fill="none" strokeWidth="3.4" strokeLinecap="round">
        <path d="M196 70c14-8 26 8 40 0s26 8 40 0 26 8 40 0 26 8 36 2" />
        <path d="M200 100c14-8 26 8 40 0s26 8 40 0 26 8 40 0 26 8 32 2" />
        <path d="M196 130c14-8 26 8 40 0s26 8 40 0 26 8 40 0 26 8 36 2" />
      </g>
    </>
  );

  return (
    <svg viewBox="0 0 360 200" className="pm" role="img" aria-label={`Postmark: ${SHOP.name}, ${SHOP.madeIn}, ${d}`}>
      <defs>
        <path id={ring} d="M100 100 m-73 0 a73 73 0 1 1 146 0 a73 73 0 1 1 -146 0" />
        <filter id={rough} x="-5%" y="-5%" width="110%" height="110%">
          <feTurbulence type="fractalNoise" baseFrequency=".85" numOctaves="2" seed="4" result="n" />
          <feColorMatrix in="n" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  -2.6 0 0 0 2.05" result="a" />
          <feComposite in="SourceGraphic" in2="a" operator="in" result="ink" />
          <feDisplacementMap in="ink" in2="n" scale="1.6" />
        </filter>
      </defs>
      <g className="pm-bleed" stroke="currentColor" fill="currentColor">{art}</g>
      <g className="pm-ink" stroke="currentColor" fill="currentColor" filter={`url(#${rough})`}>{art}</g>
    </svg>
  );
}

export default function ThanksPage() {
  const reduced = useReducedMotion();
  const today = useMemo(() => new Date(), []);
  return (
    <div className={`pt wrap ${reduced ? 'pt-still' : ''}`}>
      <div className="pt-stage">
        <div className="pt-parcel">
          <ParcelBundle left right top tied still viewBox="118 126 264 206" />
          <div className="pt-mark">
            <PostmarkStamp date={today} />
          </div>
        </div>
      </div>
      <div className="pt-copy">
        <h1 className="h-display pt-title">Tied, stamped. Thank you.</h1>
        <p className="pt-line">
          Thank you for ordering from {SHOP.name}. {SHOP.shippingNote}
        </p>
        <a href="#/shop" className="pc-cta pt-cta">
          <Back size={18} aria-hidden="true" />
          <span>Back to the shelf</span>
        </a>
      </div>
    </div>
  );
}
