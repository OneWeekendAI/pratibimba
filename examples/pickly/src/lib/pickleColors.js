/*
 * The colour of each pickle as it looks through the glass in its photo
 * (public/images/jars/<id>.webp), lifted a little so it reads at header size.
 * Used for the cart jar's liquid bands and for fly-to-jar tints.
 *
 *   import { pickleColor, PICKLE_COLORS } from '../lib/pickleColors';
 *   pickleColor('gongura') // '#4d5a26'
 */
export const PICKLE_COLORS = {
  avakaya: '#c4471d', // orange-red mango in chilli oil
  gongura: '#4d5a26', // deep sorrel green-brown
  'tomato-thokku': '#9e2a17', // thick brick-red paste
  usirikaya: '#b79a35', // whole yellow-green amla
  allam: '#8a5f22', // ginger-tamarind brown with mustard
  chicken: '#8c3519', // dark red-brown masala
  mutton: '#5c2f17', // near-black brown
  prawn: '#c4612f', // coral-orange prawns
  fish: '#bb8530', // golden fried pieces
  'boneless-chicken': '#ad3a1f', // bright chilli red
};

export const FALLBACK_PICKLE_COLOR = '#b87b1c';

export function pickleColor(id) {
  return PICKLE_COLORS[id] || FALLBACK_PICKLE_COLOR;
}
