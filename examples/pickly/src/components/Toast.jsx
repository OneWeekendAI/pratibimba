import { useEffect, useRef, useState } from 'react';
import { onToast } from '../lib/cart';

/*
 * A small kraft chit that slides out from under the header's cart jar.
 * The live region is always mounted so screen readers announce each message.
 */
export default function Toast() {
  const [toast, setToast] = useState(null); // { msg, n }
  const [pos, setPos] = useState(null);
  const [shown, setShown] = useState(false);
  const count = useRef(0);

  useEffect(() => {
    return onToast((msg) => {
      count.current += 1;
      const jar = document.querySelector('[data-cart-jar-target]');
      const header = document.querySelector('.header');
      const vw = window.innerWidth;
      // anchor the chit's right edge under the jar's right edge, so it reads as
      // slipping out from under the jar and reaches as little of the page as possible
      let left = vw - 16;
      if (jar) {
        const r = jar.getBoundingClientRect();
        if (r.width) left = r.right + 4;
      }
      left = Math.max(130, Math.min(vw - 8, left));
      const top = header ? Math.max(0, header.getBoundingClientRect().bottom) : 0;
      setPos({ left, top });
      setToast({ msg, n: count.current });
      setShown(true);
    });
  }, []);

  useEffect(() => {
    if (!toast) return undefined;
    const hide = setTimeout(() => setShown(false), 1250);
    const clear = setTimeout(() => setToast(null), 1600);
    return () => {
      clearTimeout(hide);
      clearTimeout(clear);
    };
  }, [toast]);

  return (
    <div className="pc-toast" role="status" aria-live="polite" style={pos ? { left: pos.left, top: pos.top } : undefined}>
      {toast ? (
        <span key={toast.n} className={`pc-chit ${shown ? 'in' : 'out'}`}>
          <span className="pc-chit-hole" aria-hidden="true" />
          {toast.msg}
        </span>
      ) : null}
    </div>
  );
}
