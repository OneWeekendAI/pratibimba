import { randomUUID } from 'node:crypto';
import { prb } from '@/lib/prb';
import { productBySlug } from '@/lib/products';

// "Buy now": one order of one product. A real store calls prb.purchase where its order is
// confirmed (or sends it from the payment webhook instead, see the README). order_id is the
// idempotency key: the cohort script sends its own to check that a retry is counted once.
export async function POST(request: Request) {
  const form = await request.formData();
  const product = productBySlug(String(form.get('slug') ?? ''));
  if (!product) return new Response('unknown product', { status: 404 });
  const quantity = Math.max(1, Math.min(10, Number(form.get('quantity') ?? 1) || 1));
  const order_id = String(form.get('order_id') ?? '') || randomUUID();
  await prb.purchase({
    order_id,
    value: product.price * quantity,
    currency: 'INR',
    items: [{ id: product.slug, price: product.price, quantity }],
  });
  return new Response(null, { status: 303, headers: { location: `/thanks?order=${encodeURIComponent(order_id)}` } });
}
