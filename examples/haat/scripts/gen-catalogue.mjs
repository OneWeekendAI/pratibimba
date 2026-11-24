// Seeded catalogue generator: `npm run gen` rewrites src/data/catalogue.json deterministically.
import { writeFileSync } from 'node:fs';

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const r = rng(20261124);
const pick = (a) => a[Math.floor(r() * a.length)];

const CATS = {
  electronics: { n: 19, price: [499, 24999], nouns: ['Wireless Earbuds', 'Bluetooth Speaker', 'Power Bank', 'Smart Watch', 'USB-C Charger', 'Webcam', 'Mechanical Keyboard', 'Gaming Mouse', 'Desk Lamp LED', 'Tablet Stand', 'Action Camera', 'Noise-Cancelling Headphones', 'Portable SSD', 'Monitor 24in', 'Fitness Band', 'Smart Plug', 'Router Dual-Band', 'Laptop Sleeve', 'Phone Gimbal'], brands: ['Voltix', 'Nadi', 'Orbit', 'Kiran', 'Zenar'] },
  kitchen: { n: 19, price: [149, 7999], nouns: ['Pressure Cooker', 'Non-stick Tawa', 'Spice Box', 'Chopping Board', 'Steel Tiffin', 'Mixer Grinder', 'Glass Jar Set', 'Chai Kettle', 'Knife Set', 'Air Fryer', 'Cast Iron Kadai', 'Water Bottle', 'Dinner Set', 'Cold Press Juicer', 'Rolling Pin', 'Induction Cooktop', 'Salad Bowl', 'Idli Maker', 'Coffee Press'], brands: ['Rasoi', 'Annapurna', 'Tanvi', 'Copperlane', 'Chulha'] },
  books: { n: 19, price: [99, 1299], nouns: ['Short Stories', 'Cookbook', 'Travel Memoir', 'Science Primer', 'Poetry Collection', 'Mystery Novel', 'History of Trade', 'Gardening Guide', 'Chess Openings', 'Children Picture Book', 'Business Essays', 'Mythology Retold', 'Sketching Handbook', 'Mindful Living', 'Space Atlas', 'Detective Omnibus', 'Learn Python', 'Folk Tales', 'Yoga Basics'], brands: ['Pustak', 'Katha', 'Lekhak', 'Panna', 'Akshar'] },
  fashion: { n: 19, price: [199, 5999], nouns: ['Cotton Kurta', 'Denim Jacket', 'Running Shoes', 'Linen Shirt', 'Silk Dupatta', 'Canvas Backpack', 'Leather Belt', 'Wool Shawl', 'Sports Tee', 'Block-print Saree', 'Sunglasses', 'Wrist Watch', 'Chino Trousers', 'Kolhapuri Sandals', 'Rain Jacket', 'Socks Pack of 5', 'Tote Bag', 'Hoodie', 'Cap'], brands: ['Rangrez', 'Thaan', 'Dhaga', 'Urbane', 'Mehr'] },
  home: { n: 19, price: [199, 14999], nouns: ['Cotton Bedsheet', 'Floor Cushion', 'Wall Clock', 'Jute Rug', 'Table Lamp', 'Storage Basket', 'Curtain Pair', 'Bookshelf', 'Photo Frame Set', 'Ceramic Vase', 'Door Mat', 'Bath Towels', 'Wall Mirror', 'Study Chair', 'Scented Candles', 'Clothes Rack', 'Quilt', 'Planter Pots', 'Shoe Rack'], brands: ['Ghar', 'Aangan', 'Neend', 'Kothi', 'Sajawat'] },
  toys: { n: 19, price: [129, 3999], nouns: ['Wooden Blocks', 'Jigsaw 500 pc', 'Remote Car', 'Board Game', 'Art Kit', 'Plush Elephant', 'Science Kit', 'Building Bricks', 'Play Kitchen', 'Puppet Set', 'Spinning Tops', 'Kite Combo', 'Magnetic Tiles', 'Doll House', 'Ludo Classic', 'Rubik Cube', 'Toy Train', 'Clay Set', 'Bubble Gun'], brands: ['Khel', 'Gudda', 'Bachpan', 'Tarang', 'Mela'] },
  grocery: { n: 19, price: [49, 1499], nouns: ['Basmati Rice 5kg', 'Cold-pressed Oil', 'Assam Tea', 'Mixed Nuts', 'Millet Flour', 'Wild Honey', 'Masala Combo', 'Filter Coffee', 'Dark Chocolate', 'Rolled Oats', 'Ghee 1L', 'Dry Fruit Box', 'Pickle Trio', 'Peanut Butter', 'Jaggery Cubes', 'Green Tea', 'Quinoa', 'Papad Pack', 'Instant Poha'], brands: ['Khet', 'Mitti', 'Sabzi', 'Anaaj', 'Dehat'] },
  beauty: { n: 19, price: [99, 3499], nouns: ['Face Serum', 'Sunscreen SPF 50', 'Hair Oil', 'Lip Balm Trio', 'Herbal Shampoo', 'Body Lotion', 'Kajal Pencil', 'Sandalwood Soap', 'Beard Kit', 'Face Mask Set', 'Perfume 50ml', 'Nail Polish Set', 'Moisturiser', 'Bamboo Comb', 'Rose Water', 'Hand Cream', 'Makeup Brushes', 'Trimmer', 'Gift Hamper'], brands: ['Roop', 'Neem & Co', 'Chandan', 'Gulab', 'Saundarya'] },
};
const ADJ = ['Classic', 'Everyday', 'Pro', 'Mini', 'Deluxe', 'Essential', 'Heritage', 'Smart', 'Compact', 'Signature'];

const products = [];
let id = 1;
for (const [category, c] of Object.entries(CATS)) {
  for (let i = 0; i < c.n; i++) {
    const noun = c.nouns[i];
    const lo = Math.log(c.price[0]), hi = Math.log(c.price[1]);
    const u = r();
    const price = Math.max(29, Math.round(Math.exp(lo + (hi - lo) * u) / 10) * 10 - 1);
    const disc = r() < 0.45 ? 0.05 + r() * 0.3 : 0.02 + r() * 0.08;
    const mrp = Math.ceil(price / (1 - disc) / 10) * 10;
    const tags = [];
    if (u < 0.3) tags.push('budget');
    if (u > 0.72) tags.push('premium');
    if (r() < 0.25) tags.push('gift');
    if (r() < 0.18) tags.push('new');
    products.push({
      id: `p${String(id++).padStart(3, '0')}`,
      title: `${pick(c.brands)} ${pick(ADJ)} ${noun}`,
      price,
      mrp,
      rating: Math.round((3.2 + r() * 1.7) * 10) / 10,
      reviews: Math.floor(10 + r() * r() * 9000),
      stock: r() < 0.12 ? Math.floor(1 + r() * 4) : Math.floor(5 + r() * 200),
      tags,
      category,
      seed: Math.floor(r() * 1e6),
    });
  }
}
writeFileSync(new URL('../src/data/catalogue.json', import.meta.url), JSON.stringify(products, null, 1) + '\n');
console.log(`wrote ${products.length} products`);
