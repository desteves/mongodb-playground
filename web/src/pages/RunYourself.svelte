<script lang="ts">
  import CodeBlock from '../lib/CodeBlock.svelte';
  import { site, registerUrl } from '../lib/site.svelte';

  const g = $derived(site.guide);
  const label = (u: string) => u.replace(/^https?:\/\//, '');
</script>

<div class="page">
  <h1>{g?.title ?? 'Run it yourself, or get a cluster'}</h1>
  <p class="lead">{g?.summary ?? 'Run it locally, against your own free Atlas cluster, or from an agent.'}</p>

  <p>
    <a class="btn btn-primary" href={registerUrl('guide')} target="_blank" rel="noopener">Create a free Atlas cluster ↗</a>
    <span class="muted">Free forever, 512 MB, no credit card.</span>
  </p>

  {#each g?.options ?? [] as o}
    <section id={o.id}>
      <h2>{o.title}</h2>
      {#if o.summary}<p class="muted">{o.summary}</p>{/if}
      <ol>
        {#each o.steps as s}
          <li>
            {s.label}
            {#if s.command}<CodeBlock code={s.command} />{/if}
            {#if s.url}<a href={s.url} target="_blank" rel="noopener">{label(s.url)} ↗</a>{/if}
          </li>
        {/each}
      </ol>
    </section>
  {:else}
    <p class="muted">Loading… (also at <a href="/api/v1/guide">/api/v1/guide</a>)</p>
  {/each}

  {#if g?.note}<p class="muted">{g.note}</p>{/if}
</div>
