# @pratibimba/next

Pratibimba for Next.js 16: the variant is decided during SSR, so the first byte already carries it. No flicker, no client-side swap.

```sh
npm i @pratibimba/next
```

```ts
// lib/prb.ts
import { createPratibimba } from '@pratibimba/next';
export const prb = createPratibimba({ core: process.env.PRB_CORE_URL!, site: 'demo' });
// proxy.ts:                       export const proxy = prb.proxy;
// app/%5Fprb/[...path]/route.ts:  export const { GET, POST } = prb.route;
// any server component:           <prb.Slot id="hero" variants={{ gift: <GiftHero /> }}><Hero /></prb.Slot>
```

Needs a running Pratibimba Core. Setup, sorted and copy slots, and the rest: see the [main README](https://github.com/OneWeekendAI/pratibimba#readme).

MIT licensed.
