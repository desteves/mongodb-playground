<script lang="ts">
  let { code, label = 'Copy' }: { code: string; label?: string } = $props();
  let done = $state(false);

  async function copy() {
    await navigator.clipboard?.writeText(code).catch(() => {});
    done = true;
    setTimeout(() => (done = false), 1400);
  }
</script>

<div class="codeblock">
  <pre><code>{code}</code></pre>
  <button class="mini" onclick={copy}>{done ? 'Copied' : label}</button>
</div>

<style>
  .mini {
    min-height: 0;
    padding: 0.1rem 0.6rem;
    font-size: 0.78rem;
  }
</style>
