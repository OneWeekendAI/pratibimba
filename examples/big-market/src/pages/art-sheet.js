import { h, products, CATEGORIES, esc } from '../lib.js';
import { art, nounOf } from '../art.js';

// Dev route: #/art-sheet[?cat=books&v=0..4] shows every product's art for visual review.
export function artSheet(qs) {
  const cat = qs.get('cat');
  const v = +(qs.get('v') || 0);
  const list = products.filter((p) => !cat || p.category === cat);
  const el = h(`<div data-testid="art-sheet"><h1>Art sheet</h1>
    <div class="sheet-h">${['', ...CATEGORIES].map((c) => `<a class="tag" href="#/art-sheet?${c ? 'cat=' + c + '&' : ''}v=${v}">${c || 'all'}</a>`).join('')}
      ${[0, 1, 2, 3, 4].map((i) => `<a class="tag" href="#/art-sheet?${cat ? 'cat=' + cat + '&' : ''}v=${i}">variant ${i}</a>`).join('')}</div>
    <div class="sheet">${list.map((p) => `<figure>${art(p, v)}<figcaption>${p.id} ${esc(nounOf(p.title) || 'NO MOTIF')}</figcaption></figure>`).join('')}</div></div>`);
  return { el };
}
