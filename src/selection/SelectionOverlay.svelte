<!--
  One monitor's region-selection overlay (CAP-02, DD-29).

  This is not a view of the panel: it is the whole content of a transparent, always-on-top
  window that the Rust side creates over one monitor and destroys the moment the drag ends.
  DD-29 calls these capture *tools* rather than handoff windows, which is what keeps them
  compatible with MULTI-04 — nothing about a handoff is ever drawn here, and none of them
  outlives the selection.

  Three things it has to get right:

  - **the drag may leave this monitor.** The pointer is captured on press, so the coordinates
    keep coming in this window's own CSS pixels even when the mouse is over the screen next
    door; the Rust side turns them into virtual-desktop pixels with *this* monitor's scale
    factor, which is exactly what that means (`capture::geometry`).
  - **the dimming must not cover the selection.** The rectangle carries a spread shadow
    instead of four surrounding panes: one element, no seams, and the part the user framed
    stays as transparent as the rest of the window.
  - **nothing may be styled inline.** The CSP is `default-src 'self'`, which refuses a
    `style` attribute as surely as an injected stylesheet. Svelte's `style:` directive sets
    the property through the CSSOM, which CSP does not govern, and it is the only reason a
    rectangle can be positioned at all here.

  Esc cancels, and a press without a drag cancels too: both put the panel back and take
  nothing.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { LogicalRect } from '../model';

  /** The monitor this window covers, and how many pixels one CSS pixel is worth on it. */
  let monitor = $state(0);
  let scaleFactor = $state(1);

  /** Where the press landed, and where the pointer is now; both `null` between drags. */
  let from = $state<{ x: number; y: number } | null>(null);
  let to = $state<{ x: number; y: number } | null>(null);

  /** What is framed right now, in this window's CSS pixels. */
  const rect = $derived.by<LogicalRect | null>(() => {
    if (from === null || to === null) {
      return null;
    }
    return {
      x: Math.min(from.x, to.x),
      y: Math.min(from.y, to.y),
      width: Math.abs(to.x - from.x),
      height: Math.abs(to.y - from.y),
    };
  });

  /** The same rectangle in the pixels the image will have, which is what a user reads. */
  const size = $derived(
    rect === null
      ? null
      : {
          width: Math.round(rect.width * scaleFactor),
          height: Math.round(rect.height * scaleFactor),
        },
  );

  onMount(() => {
    void bridge()
      .selectionSetup()
      .then((setup) => {
        monitor = setup.monitor;
        scaleFactor = setup.scaleFactor;
      })
      .catch(() => {
        // Without the setup the drag would be reported for the wrong monitor, so the
        // overlay cancels itself rather than cropping the wrong part of the desktop.
        void bridge().cancelRegionCapture();
      });
  });

  function cancel(): void {
    from = null;
    to = null;
    void bridge().cancelRegionCapture();
  }

  function down(event: PointerEvent): void {
    if (event.button !== 0) {
      return;
    }
    // Capturing the pointer is what lets a drag continue onto the screen next door: the
    // coordinates keep arriving in this window's space, which is what CAP-02 needs. It is
    // asked for rather than assumed, because a DOM without pointer capture — jsdom under
    // the component tests — must still report the drag it does see.
    const surface = event.currentTarget;
    if (surface instanceof HTMLElement && typeof surface.setPointerCapture === 'function') {
      surface.setPointerCapture(event.pointerId);
    }
    from = { x: event.clientX, y: event.clientY };
    to = from;
  }

  function move(event: PointerEvent): void {
    if (from !== null) {
      to = { x: event.clientX, y: event.clientY };
    }
  }

  function up(event: PointerEvent): void {
    if (from === null) {
      return;
    }
    to = { x: event.clientX, y: event.clientY };
    const framed = rect;
    from = null;
    if (framed === null || framed.width < 1 || framed.height < 1) {
      // A click, not a drag. There is nothing to crop and nothing went wrong.
      cancel();
      return;
    }
    void bridge().regionCaptured({ monitor, rect: framed });
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === 'Escape') {
      cancel();
    }
  }}
/>

<div
  class="selection"
  role="presentation"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={cancel}
>
  {#if rect === null}
    <div class="selection-dim"></div>
    <p class="selection-hint">{t('capture.dragHint')}</p>
  {:else}
    <div
      class="selection-rect"
      style:left="{rect.x}px"
      style:top="{rect.y}px"
      style:width="{rect.width}px"
      style:height="{rect.height}px"
    >
      {#if size !== null}
        <span class="selection-size"
          >{t('capture.size', { width: size.width, height: size.height })}</span
        >
      {/if}
    </div>
  {/if}
</div>
