import { notFound, redirect } from 'next/navigation';
import { prb } from '@/lib/prb';
import { productBySlug } from '@/lib/products';

export const dynamic = 'force-dynamic';

export default async function ProductPage({ params, searchParams }: {
  params: Promise<{ slug: string }>;
  searchParams: Promise<{ added?: string }>;
}) {
  const { slug } = await params;
  const { added } = await searchParams;
  const product = productBySlug(slug);
  if (!product) notFound();

  // Conversions are reported from the server, for this request's visitor.
  async function addToCart() {
    'use server';
    await prb.track('add_to_cart');
    redirect(`/p/${slug}?added=1`);
  }
  async function buyNow() {
    'use server';
    await prb.track('purchase');
    redirect('/thanks');
  }

  return (
    <article>
      <h1>{product.name}</h1>
      <p>{product.blurb}</p>
      <p className="price">₹{product.price}</p>
      <form action={addToCart}><button type="submit">Add to cart</button></form>
      <form action={buyNow}><button className="buy" type="submit">Buy now</button></form>
      {added && <p className="note">Added to your cart.</p>}
      <prb.Slot id="promo">
        <p className="promo">Free shipping over ₹999.</p>
      </prb.Slot>
    </article>
  );
}
