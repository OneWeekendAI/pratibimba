# Big Market

A demo marketplace (directory and site id `haat`) used to show Pratibimba on a store big enough for personalisation to be visible. Original brand and generated illustrations (`src/art.js`: one style per category, one motif per product type); no third-party assets. Demo only: no payments, orders live in `localStorage`.

## Run

```
npm install
npm run dev        # vite --port 5174 --strictPort  -> http://localhost:5174
npm run e2e        # Playwright smoke (starts the dev server if it is not running)
npm run gen        # regenerate src/data/catalogue.json (seeded, deterministic)
```

`/_prb/*` is proxied to Pratibimba Core at `http://localhost:8080` with the prefix stripped. Nothing calls it yet.

## Pages (hash routed)

`#/` home, `#/c/<category>` listing (8 categories), `#/product/<id>`, `#/search?q=`, `#/cart`, `#/checkout`, plus the dev route `#/art-sheet` (every product's art in a grid; `?cat=books` filters, `?v=2` shows an art variant).

## Slot regions

Each region below is rendered once into a stable container carrying a `data-slot` attribute. Add-to-cart only writes `localStorage`, patches the header count and shows a toast, so none of these regions re-render on unrelated state changes. The e2e suite asserts this.

| data-slot | where |
|---|---|
| `promo-bar` | shell, every page |
| `hero`, `category-rows` (`category-row` each), `picks-strip`, `deals-strip` | home |
| `listing-sort` | category listing (only the grid repaints on filter/sort) |
| `product-badge`, `buy-button`, `delivery-message` | product page |
| `cart-cross-sell` | cart (cart lines repaint; the strip does not) |
