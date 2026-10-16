<script lang="ts">
  export let data;
  let n = 0;
  const cheap = (p) => p.price<limit && p.stock>0;
  const tpl = `<h1>Not a heading</h1><a class="cta">Shop fake</a>`;
</script>

<svelte:head><title>Candles</title></svelte:head>

<section class="hero">
  <h1>Hand-poured candles</h1>
  <a class="btn" href="#shop">Shop candles</a>
</section>

{#if data.items.length<limit}
  <p class="banner">Free shipping this week only</p>
{/if}

<div class="products">
  {#each data.items as p (p.slug)}
    <a href="/p/{p.slug}" class="card"><h2>{p.name}</h2><span>₹{p.price}</span></a>
  {/each}</div>

<!-- prb:slot="welcome" -->
<aside>Welcome back! Pick up where you left off.</aside>

<button on:click={() => (n = n<3 ? n + 1 : 0)}>Get started</button>
<h1>{data.title}</h1>

<style>
  .card > h2 { margin: 0; }
</style>
