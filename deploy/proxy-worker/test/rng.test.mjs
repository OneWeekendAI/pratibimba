// ChaCha8Rng::seed_from_u64 + rand_distr's StandardNormal, pinned to what Rust printed
// (rand_chacha 0.9.0, rand_core 0.9.5, rand_distr 0.5.1). The tail draws (|x| above the
// ziggurat's R = 3.654) come from the bottom box's own loop, which uses Math.log.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ChaCha8 } from '../src/rng.js';

const PINS = {
 "0": {
  "u64": [
   "13080132717333068652",
   "8594738769458413623",
   "12896916468484187878",
   "1109962093070354556"
  ],
  "normal": [
   0.6999607946268154,
   -0.14406163542784764,
   0.30288628024558556,
   -1.374513899829971
  ],
  "uniform": [
   0.7090754154265618,
   0.46592172228961015
  ],
  "tail": [
   [
    1060,
    3.6213243417418024
   ]
  ]
 },
 "1": {
  "u64": [
   "7424550030962593201",
   "1482817706323250795",
   "11004592982271133285",
   "4045824405258374466"
  ],
  "normal": [
   -0.23484705855592097,
   -1.4108177006109994,
   0.3330286294792717,
   -1.1267663243742092
  ],
  "uniform": [
   0.40248566366484806,
   0.08038370892978197
  ],
  "tail": [
   [
    1968,
    -3.5103995591505637
   ]
  ]
 },
 "42": {
  "u64": [
   "12578764544318200737",
   "17529487244874322312",
   "7886285670807131020",
   "11572758976476374866"
  ],
  "normal": [
   0.47798123835102174,
   1.3340706102318078,
   -0.21086668327103028,
   0.4763469238088213
  ],
  "uniform": [
   0.6818961923066714,
   0.950275407672484
  ],
  "tail": [
   [
    153,
    -3.791710975478962
   ],
   [
    221,
    -3.5181572886267034
   ]
  ]
 },
 "18446744073709551615": {
  "u64": [
   "12619125218144107694",
   "7899558952931569112",
   "7024312652144319452",
   "7984919490939168151"
  ],
  "normal": [
   0.4510299827951996,
   -0.12931526405511196,
   -0.2059482902040233,
   -0.18543614703146855
  ],
  "uniform": [
   0.6840841488189228,
   0.4282359489222861
  ],
  "tail": []
 },
 "1311768467463790320": {
  "u64": [
   "1846600907505102423",
   "11472018663571619602",
   "9120510585805609864",
   "10843869218630246802"
  ],
  "normal": [
   -1.4638496402281325,
   0.6475984995706795,
   -0.016520892780057268,
   0.24851329043736659
  ],
  "uniform": [
   0.10010443578153672,
   0.6218993778919301
  ],
  "tail": [
   [
    615,
    4.493213929331461
   ]
  ]
 }
};

for (const [seed, pin] of Object.entries(PINS)) {
  test(`seed ${seed}: u64 stream, uniforms and normals match Rust bit for bit`, () => {
    let r = new ChaCha8(BigInt(seed));
    assert.deepEqual(pin.u64.map(() => r.nextU64().toString()), pin.u64);
    r = new ChaCha8(BigInt(seed));
    assert.deepEqual(pin.uniform.map(() => r.uniform()), pin.uniform);
    r = new ChaCha8(BigInt(seed));
    const normals = [];
    const last = Math.max(4, ...pin.tail.map(([i]) => i + 1));
    for (let i = 0; i < last; i++) normals.push(r.normal());
    assert.deepEqual(normals.slice(0, 4), pin.normal);
    for (const [i, x] of pin.tail) assert.ok(Object.is(normals[i], x), `normal #${i}: ${normals[i]} vs ${x}`);
  });
}
