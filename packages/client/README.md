# @pratibimba/client

The Pratibimba script-tag fallback for any page, static HTML included. It hides only the marked slots, asks Core for a decision, swaps and reveals within a hard timeout (400 ms by default). Single-page apps that mount slots later are handled too.

Pratibimba Core already serves this file at `/sdk.js`, so most sites need no package. Install it when you want to pin and self-host an exact version:

```sh
npm i @pratibimba/client
```

```html
<!-- proxy /_prb/* to Core; optionally serve node_modules/@pratibimba/client/sdk.js yourself -->
<script src="/_prb/manifest/demo.js"></script>
<script src="/_prb/sdk.js" data-site="demo"></script>
<!-- data-timeout="400" sets the hard reveal; data-base points the calls elsewhere -->
```

Plain script, no build step, no dependencies. Privacy modes, SPA behaviour and the rest: see the [main README](https://github.com/OneWeekendAI/pratibimba#readme).

MIT licensed.
