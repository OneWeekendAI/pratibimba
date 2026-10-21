<script lang="ts">
  let { data } = $props();
  let open = $state(false);
</script>

{#snippet card(p)}
  <a href="/p/{p.slug}" class="card"><h2>{p.name}</h2><span>₹{p.price}</span></a>
{/snippet}

{#snippet offer()}
  <p class="notice">Free delivery this week only</p>
{/snippet}

<section class="hero">
  <h1>Hand-poured candles</h1>
  <a class="btn" href="#shop">Shop candles</a>
</section>

<div class="products">
  {#each data.items as p (p.slug)}
    {@render card(p)}
  {/each}
</div>

<aside data-prb-slot="weekly">
  {@render offer()}
</aside>

{#snippet thanks(name = '}')}
  <h1>Thanks for visiting</h1>
{/snippet}
{@render thanks()}

<button onclick={() => (open = !open)}>Get started</button>
