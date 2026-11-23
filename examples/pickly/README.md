# Pickly (example)

A small pickle shop built with Vite and React 18: a shelf of jars, a cart, a checkout form and
a thank-you page. It has **no Pratibimba code**. It is here as an ordinary existing site, the
starting point for showing how to add Pratibimba to a site someone else wrote (the
`pratibimba-integrate` skill that `npx @pratibimba/agent install` adds walks through it).

Everything is client-side: the cart lives in `localStorage`, checkout sends nothing anywhere,
and the contact details are placeholders.

```sh
cd examples/pickly
npm ci
npm run dev        # http://localhost:5173
npm run build      # static site in dist/
```

MIT licensed, like the rest of the repo.
