export interface Product {
  slug: string;
  name: string;
  blurb: string;
  price: number;
}

export const products: Product[] = [
  { slug: 'monday-espresso', name: 'Monday Espresso', blurb: 'Chikmagalur, dark chocolate and jaggery. 250 g.', price: 549 },
  { slug: 'estate-filter', name: 'Estate Filter Blend', blurb: '70/30 coffee and chicory for a South Indian filter. 500 g.', price: 699 },
];

export const productBySlug = (slug: string) => products.find((p) => p.slug === slug);
