<!--
  A step text, with its `https://` links made clickable (GUIDE-03).

  The text is written by an agent, so it is untrusted input inside our own window: it never
  reaches `{@html}`. `linkify` cuts it into segments and each of them is rendered by Svelte,
  which escapes it; an anchor exists only where `linkify` put one, and clicking it goes
  through `openUrl`, which judges the scheme again on the side that can actually launch
  something.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { linkify } from './linkify';

  const { text }: { text: string } = $props();

  const segments = $derived(linkify(text));
</script>

<p class="step-text">
  {#each segments as segment, index (index)}
    {#if segment.kind === 'link'}
      <a
        class="link"
        href={segment.value}
        onclick={(event) => {
          event.preventDefault();
          void bridge().openUrl(segment.value);
        }}>{segment.value}</a
      >
    {:else}{segment.value}{/if}
  {/each}
</p>
