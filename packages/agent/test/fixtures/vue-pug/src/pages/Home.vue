<script setup>
const items = [];
// a(class="cta") Not a template
</script>

<template lang="pug">
//- the home page, in Pug
main.page
  section.hero#top
    h1 Hand-poured candles
    p.lede Soy wax, cotton wicks.
    a.btn(href="/shop", :class="{ wide: true }") Shop candles
    img(src="/img/hero.jpg" alt="Candles on a shelf")
  .banner
    | Free shipping this week only
  ul.products
    li.card(
      v-for="p in items"
      :key="p.slug"
    )
      router-link(:to="'/p/' + p.slug") {{ p.name }}
      span.price ₹{{ p.price }}
  // prb:slot="comeback"
  aside
    | Welcome back! Pick up where you left off.
    Welcome
  h1= title
  pre.
    h1 Words in a block
    not tags: section.hero
  button(@click="book") Book a call
</template>

<style lang="scss">
.hero { color: red; }
</style>
