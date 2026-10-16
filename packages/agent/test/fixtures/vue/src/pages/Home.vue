<script setup lang="ts">
import { ref } from 'vue';
import Welcome from '../components/Welcome.vue';
const products = ref<Product[]>([]);
const open = ref(false);
const fake = '<section class="hero"><h1>Not markup</h1><a class="cta" href="#">Shop fake</a></section>';
</script>

<template>
  <nav><a class="cta" href="/join">Join now</a></nav>
  <section class="hero">
    <h1>Hand-poured candles</h1>
    <p>Soy wax, cotton wicks.</p>
    <a class="cta" href="#shop">Shop candles</a>
  </section>
  <p class="note" v-if="count<limit">Free shipping over ₹999 this week.</p>
  <span class="tally">{{ count<limit ? 'More to go' : 'All done' }}</span>
  <ul class="products">
    <li v-for="p in products" :key="p.slug" class="card">{{ p.name }} ₹{{ p.price }}</li>
  </ul>
  <div data-prb-slot="comeback"><Welcome /></div>
  <button @click="open = !open">{{ open ? 'Close' : 'Get started' }}</button>
  <button class="cta" @click="go">Book a call</button>
  <aside>Welcome back! {{ seen<p && p>0 ? 'Pick up where you left off.' : '' }}</aside>
</template>

<style scoped>
.hero > h1 { color: red; }
.x::before { content: '<button>Buy now</button>'; }
</style>
