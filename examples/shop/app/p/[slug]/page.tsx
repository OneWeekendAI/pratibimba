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

  // Conversions are reported from the server, for this request's visitor. A purchase carries its
  // order: Core stores one row per order_id and credits its value as revenue to the decisions
  // that may have caused it (the promo and the hero both get their share).
  async function addToCart() {
    'use server';
    await prb.addToCart({ id: product!.slug, price: product!.price, quantity: 1 });
    redirect(`/p/${slug}?added=1`);
  }

  return (
    <article>
      <h1>{product.name}</h1>
      <p>{product.blurb}</p>
      <p className="price">₹{product.price}</p>
      <form action={addToCart}><button type="submit">Add to cart</button></form>
      <form action="/checkout" method="post">
        <input type="hidden" name="slug" value={product.slug} />
        <button className="buy" type="submit">Buy now</button>
      </form>
      {added && <p className="note">Added to your cart.</p>}
      <prb.Slot id="promo">
        <p className="promo">Free shipping over ₹999.</p>
      </prb.Slot>
    </article>
  );
}
