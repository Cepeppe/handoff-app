<!--
  One icon, drawn inline (§7.6).

  Three rules hold for every icon in this window, and they are enforced here rather than
  asked of each call site:

  - **It is never a resource.** The CSP is `default-src 'self'` with no `unsafe-inline`, so
    an icon font, a remote sprite and an injected `<style>` are all refused. The drawing is
    markup this component writes, from the data in `icons.ts`.
  - **It has no colour of its own.** `stroke="currentColor"` means the icon is whatever
    colour the text around it is, so a muted control, a primary button and a red menu item
    all draw the same shapes with no variants.
  - **It is decoration.** `aria-hidden="true"` on every one of them: what a screen reader
    reads is the button's label, and an icon that announced itself as well would say the
    same thing twice. That is also why an icon-only button always carries an `aria-label`.
-->
<script lang="ts">
  import { ICONS, type IconName } from './icons';

  const {
    name,
    size = 16,
  }: {
    name: IconName;
    /** The side of the square, in pixels. The grid is 16, so anything scales cleanly. */
    size?: number;
  } = $props();

  const icon = $derived(ICONS[name]);
  const filled = $derived('filled' in icon);
  const stroke = $derived('width' in icon ? icon.width : 1.5);
</script>

<svg
  class="icon"
  class:icon-filled={filled}
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#each icon.shapes as shape, index (index)}
    {#if shape.kind === 'path'}
      <path d={shape.d} />
    {:else if shape.kind === 'circle'}
      <circle cx={shape.cx} cy={shape.cy} r={shape.r} />
    {:else}
      <rect x={shape.x} y={shape.y} width={shape.width} height={shape.height} rx={shape.rx} />
    {/if}
  {/each}
</svg>
