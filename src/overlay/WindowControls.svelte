<!--
  The three window controls of the title bar (WIN-03, WIN-04, §7.6).

  The window has no decorations, so it has no system buttons: these are them. They are in the
  same order and in the same place on every screen the application shows — **Minimize to
  tray**, **Shrink to bar**, **Expand** — because a control that moves or disappears between
  two screens is a control the user has to look for.

  A control that does nothing where it is shown is **disabled but not hidden**, and it is
  `aria-disabled` rather than `disabled`: a real `disabled` attribute takes the button out of
  the tab order and stops the browser from showing its `title`, and the whole point here is
  that the tooltip says *why* it is unavailable ("The bar is available while you follow a
  step"). The click is ignored in the handler instead.

  Nothing here decides anything: the parent says what is available and what pressing each one
  does. Minimize is the only one that reaches the Rust side, through the same path the close
  button takes (WIN-04) — remember the position, write it, hide the window.
-->
<script lang="ts">
  import { t } from '../i18n';
  import Icon from './Icon.svelte';

  const {
    canCollapse,
    canExpand,
    expanded,
    onminimize,
    oncollapse,
    ontoggleform,
  }: {
    /** Whether there is a step to shrink to (WIN-03). */
    canCollapse: boolean;
    /** Whether this screen has an expanded form at all (the overlay, and nothing else). */
    canExpand: boolean;
    /** Whether the window is in that form right now. */
    expanded: boolean;
    onminimize: () => void;
    oncollapse: () => void;
    ontoggleform: () => void;
  } = $props();
</script>

<div class="win-controls" role="group" aria-label={t('window.controls')}>
  <button
    type="button"
    class="win-button"
    aria-label={t('window.minimize')}
    title={t('window.minimize')}
    onclick={onminimize}
  >
    <Icon name="minimize" />
  </button>

  <button
    type="button"
    class="win-button"
    aria-disabled={canCollapse ? undefined : 'true'}
    aria-label={t('window.collapse')}
    title={canCollapse ? t('window.collapse') : t('window.collapseUnavailable')}
    onclick={() => {
      if (canCollapse) {
        oncollapse();
      }
    }}
  >
    <Icon name="collapse" />
  </button>

  <button
    type="button"
    class="win-button"
    aria-disabled={canExpand ? undefined : 'true'}
    aria-pressed={canExpand && expanded ? 'true' : undefined}
    aria-label={expanded ? t('window.restore') : t('window.expand')}
    title={canExpand
      ? expanded
        ? t('window.restore')
        : t('window.expand')
      : t('window.expandUnavailable')}
    onclick={() => {
      if (canExpand) {
        ontoggleform();
      }
    }}
  >
    <Icon name={expanded ? 'restore' : 'expand'} />
  </button>
</div>
