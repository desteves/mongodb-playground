<script lang="ts">
  import CodeBlock from '../lib/CodeBlock.svelte';
  import { tick } from 'svelte';
  import { site, registerUrl, mongodbUrl } from '../lib/site.svelte';
  import { scrollToHash } from '../lib/ui.svelte';

  const g = $derived(site.guide);

  // Opened with a #fragment (a shared link): the options render after the guide loads, so scroll once they are there.
  $effect(() => {
    if (g) tick().then(() => scrollToHash(location.hash));
  });
</script>

<div class="page">
  <h1>{g?.title ?? 'Run it yourself, or get a cluster'}</h1>
  <p class="lead">{g?.summary ?? 'Run it locally, against your own free Atlas cluster, or from an agent.'}</p>

  <p>
    <a class="btn btn-primary" href={registerUrl()} target="_blank" rel="noopener">Create a free Atlas cluster ↗</a>
    <span class="muted">Free forever, 512 MB, no credit card.</span>
  </p>

  {#if g?.choose?.length}
    <h2>Which one should I use?</h2>
    <ul class="choose">
      {#each g.choose as c}
        <li>{c.want}: <a href={`#${c.use}`}>{g.options.find((o) => o.id === c.use)?.title ?? c.use}</a></li>
      {/each}
    </ul>
  {/if}

  {#each g?.options ?? [] as o, i}
    {#if o.group && o.group !== g?.options[i - 1]?.group}<h2 class="group">{o.group}</h2>{/if}
    <section id={o.id}>
      <h3>{o.title}</h3>
      {#if o.summary}<p class="muted">{o.summary}</p>{/if}
      <ol>
        {#each o.steps as s}
          <li>
            {#if s.url}
              <a href={mongodbUrl(s.url)} target="_blank" rel="noopener">{s.label} ↗</a>
            {:else}
              {s.label}
            {/if}
            {#if s.command}<CodeBlock code={s.command} />{/if}
          </li>
        {/each}
      </ol>
    </section>
  {:else}
    <p class="muted">Loading… (also at <a href="/api/v1/guide">/api/v1/guide</a>)</p>
  {/each}

  {#if g?.note}<p class="muted">{g.note}</p>{/if}
</div>

<style>
  .group {
    margin-top: 2.2rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border);
  }
  section h3 {
    margin: 1.4rem 0 0.3rem;
    font-size: 1.15rem;
  }
  .choose {
    line-height: 1.7;
  }
</style>
