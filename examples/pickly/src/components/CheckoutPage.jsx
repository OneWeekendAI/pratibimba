import { useEffect, useRef, useState } from 'react';
import { PICKLES, SHOP } from '../data/shop';
import { inr } from '../lib/helpers';
import { clearCart } from '../lib/cart';
import { useReducedMotion } from '../lib/motion';
import { Back, Twine } from './icons';
import ParcelBundle, { wrapSequence } from './ParcelBundle';
import ParcelEmpty from './ParcelEmpty';

const SHIPPING_FLAT = 60;

const okName = (f) => !!f.name.trim();
const okPhone = (f) => f.phone.replace(/\D/g, '').length === 10;
const okPin = (f) => f.pincode.replace(/\D/g, '').length === 6;
const okAddr = (f) => !!f.address1.trim();

const FIELD_IDS = { name: 'c-name', phone: 'c-phone', pincode: 'c-pincode', address1: 'c-addr1' };

export default function CheckoutPage({ cart }) {
  const reduced = useReducedMotion();
  const [form, setForm] = useState({
    name: '',
    phone: '',
    pincode: '',
    address1: '',
    address2: '',
    notes: '',
  });
  const [errors, setErrors] = useState({});
  // While the parcel is being tied, hold the items we are wrapping so the
  // page does not flash an empty state when the cart clears.
  const [tying, setTying] = useState(null);
  const timer = useRef(0);
  const previewRef = useRef(null);
  useEffect(() => () => clearTimeout(timer.current), []);

  const liveItems = (cart.items || [])
    .map((item) => {
      const pickle = PICKLES.find((p) => p.id === item.id);
      if (!pickle) return null;
      return { ...pickle, qty: item.qty, lineTotal: pickle.price * item.qty };
    })
    .filter(Boolean);
  const lineItems = tying ? tying.items : liveItems;

  const subtotal = lineItems.reduce((acc, it) => acc + it.lineTotal, 0);
  const total = subtotal > 0 ? subtotal + SHIPPING_FLAT : 0;
  const jarCount = lineItems.reduce((a, b) => a + b.qty, 0);

  const handleChange = (field) => (e) => {
    setForm((prev) => ({ ...prev, [field]: e.target.value }));
    if (errors[field]) {
      setErrors((prev) => {
        const next = { ...prev };
        delete next[field];
        return next;
      });
    }
  };

  const validate = () => {
    const nextErrors = {};
    if (!okName(form)) nextErrors.name = 'Please enter your full name';
    if (!okPhone(form)) nextErrors.phone = 'Please enter a valid 10-digit phone number';
    if (!okPin(form)) nextErrors.pincode = 'Please enter a valid 6-digit postal pincode';
    if (!okAddr(form)) nextErrors.address1 = 'Please enter your street delivery address';
    setErrors(nextErrors);
    const first = Object.keys(nextErrors)[0];
    if (first) document.getElementById(FIELD_IDS[first])?.focus();
    return Object.keys(nextErrors).length === 0;
  };

  // Progressive wrap: each finished part of the label folds one flap.
  const steps = {
    left: okName(form),
    right: okPhone(form) && okPin(form),
    top: okAddr(form),
  };

  const handleSubmit = (e) => {
    e.preventDefault();
    if (tying) return;
    if (!validate()) {
      return;
    }

    const finish = () => {
      console.log('Order submitted (stub):', {
        customer: {
          name: form.name.trim(),
          phone: form.phone.replace(/\D/g, ''),
          pincode: form.pincode.replace(/\D/g, ''),
          address1: form.address1.trim(),
          address2: form.address2.trim(),
          notes: form.notes.trim(),
        },
        items: cart.items,
        total,
        currency: SHOP.currency,
      });

      clearCart();
      window.location.hash = '#/thanks';
    };

    if (reduced) {
      finish();
      return;
    }
    // On a phone the preview sits above the label, out of sight while the
    // button is pressed: bring it into view first, then play the wrap there.
    let lead = 0;
    const el = previewRef.current;
    if (el) {
      const r = el.getBoundingClientRect();
      const headerBottom = document.querySelector('.header')?.getBoundingClientRect().bottom || 0;
      if (r.top < headerBottom - 4 || r.bottom > window.innerHeight + 4) {
        const y = window.scrollY + r.top - headerBottom - 12;
        window.scrollTo({ top: Math.max(0, y), behavior: 'smooth' });
        lead = 520;
      }
    }
    const seq = wrapSequence(steps, lead);
    setTying({ items: liveItems, delays: seq.delays });
    // hold a beat on the lifted parcel before moving on
    timer.current = setTimeout(finish, seq.total + 240);
  };

  if (!lineItems.length) {
    return (
      <ParcelEmpty
        title="Nothing to wrap yet"
        line="Pick a jar or two from the shelf and we will fold them up in newspaper."
      />
    );
  }

  const wrapped = reduced || !!tying;
  const err = (k) => (errors[k] ? { 'aria-invalid': true, 'aria-describedby': `${FIELD_IDS[k]}-err` } : {});

  return (
    <div className="pc-checkout wrap">
      <a href="#/cart" className="pc-back">
        <Back size={16} aria-hidden="true" /> Back to the counter
      </a>

      <header className="pc-head">
        <span className="eyebrow">Fill in the label</span>
        <h1 className="h-display">Where should it go?</h1>
      </header>

      <div className="pc-co-layout">
        <div
          ref={previewRef}
          className={`pc-preview ${tying ? 'is-tying' : ''}`}
          aria-hidden="true"
          style={tying ? {
            '--press': `${tying.delays.press ?? 0}ms`,
            '--cinch': `${tying.delays.cinch}ms`,
          } : undefined}
        >
          <div className={`pc-stage ${tying && tying.delays.press != null ? 'is-pressing' : ''}`}>
          <ParcelBundle
            jars={lineItems}
            left={wrapped || steps.left}
            right={wrapped || steps.right}
            top={wrapped || steps.top}
            tied={wrapped}
            delays={tying ? tying.delays : {}}
            still={reduced}
          />
          </div>
          <p className="pc-preview-cap">
            {tying ? 'Tying the knot' : `${jarCount} ${jarCount === 1 ? 'jar' : 'jars'} on the sheet`}
          </p>
        </div>

        <form onSubmit={handleSubmit} className="pc-label" noValidate aria-label="Delivery details">
          <div className="pc-label-top">
            <span className="pc-stamp-word">Consignment</span>
            <span className="pc-label-from">
              From {SHOP.name}, {SHOP.madeIn}
            </span>
          </div>

          <fieldset className="pc-fs" disabled={!!tying}>
            <legend className="pc-to">To</legend>

            <div className="pc-field">
              <label htmlFor="c-name"><span className="pc-no">01</span>Full name<span className="pc-req">*</span></label>
              <input id="c-name" type="text" value={form.name} onChange={handleChange('name')} placeholder="e.g. Ramesh Kumar" autoComplete="name" {...err('name')} />
              {errors.name && <span className="err" id="c-name-err">{errors.name}</span>}
            </div>

            <div className="pc-row">
              <div className="pc-field">
                <label htmlFor="c-phone"><span className="pc-no">02</span>Phone (WhatsApp)<span className="pc-req">*</span></label>
                <input id="c-phone" type="tel" inputMode="tel" value={form.phone} onChange={handleChange('phone')} placeholder="10-digit mobile" autoComplete="tel" maxLength={14} {...err('phone')} />
                {errors.phone && <span className="err" id="c-phone-err">{errors.phone}</span>}
              </div>
              <div className="pc-field pc-field-pin">
                <label htmlFor="c-pincode"><span className="pc-no">03</span>Pincode<span className="pc-req">*</span></label>
                <input id="c-pincode" type="text" inputMode="numeric" value={form.pincode} onChange={handleChange('pincode')} placeholder="6 digits" autoComplete="postal-code" maxLength={6} {...err('pincode')} />
                {errors.pincode && <span className="err" id="c-pincode-err">{errors.pincode}</span>}
              </div>
            </div>

            <div className="pc-field">
              <label htmlFor="c-addr1"><span className="pc-no">04</span>House / flat, street<span className="pc-req">*</span></label>
              <input id="c-addr1" type="text" value={form.address1} onChange={handleChange('address1')} placeholder="e.g. Flat 302, Sri Krishna Apts, MG Road" autoComplete="street-address" {...err('address1')} />
              {errors.address1 && <span className="err" id="c-addr1-err">{errors.address1}</span>}
            </div>

            <div className="pc-field">
              <label htmlFor="c-addr2"><span className="pc-no">05</span>Landmark, area <em>optional</em></label>
              <input id="c-addr2" type="text" value={form.address2} onChange={handleChange('address2')} placeholder="e.g. Near Clock Tower" />
            </div>

            <div className="pc-field">
              <label htmlFor="c-notes"><span className="pc-no">06</span>Notes for us <em>optional</em></label>
              <textarea id="c-notes" value={form.notes} onChange={handleChange('notes')} placeholder="Anything we should know about delivery" rows={2} />
            </div>
          </fieldset>

          <div className="pc-contents">
            <span className="pc-stamp-word pc-stamp-sm">Contents</span>
            <ul>
              {lineItems.map((item) => (
                <li key={item.id}>
                  <span>{item.name} <small>× {item.qty}</small></span>
                  <span>{inr(item.lineTotal)}</span>
                </li>
              ))}
              <li className="pc-c-sub"><span>Shipping, flat</span><span>{inr(SHIPPING_FLAT)}</span></li>
              <li className="pc-c-total"><span>Total</span><span>{inr(total)}</span></li>
            </ul>
          </div>

          <button type="submit" className="pc-tie-btn" disabled={!!tying}>
            <Twine size={20} aria-hidden="true" />
            <span>{tying ? 'Tying it up…' : `Tie it up · ${inr(total)}`}</span>
          </button>
          <p className="pc-fine">{SHOP.shippingNote}</p>
        </form>
      </div>
    </div>
  );
}
