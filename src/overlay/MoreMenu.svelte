<!--
  The **More** menu of the action bar: Skip, Defer and Abandon (RESP-05, RESP-08).

  Four actions are always on screen — Done, Ask, Screenshot, Note — because those are what a
  person presses while they work (RESP-02). The three that *end* something go one press
  deeper: they are rare, they are not undone the same way, and putting Skip in the same row
  as Done is how a step gets skipped by accident.

  Two rules the design fixes and this component keeps:

  - **Abandon stays immediately after Defer** (RESP-08). The separator above it is a rule and
    not a reordering: Abandon is still the next item, and it is the only one drawn in the
    danger colour.
  - **Nothing here appears on hover.** The menu opens on a press and on the keyboard, and it
    is a real `role="menu"` of real `role="menuitem"` buttons, so a keyboard and a touch
    screen reach exactly what a mouse reaches.

  The keyboard is the usual menu-button pattern: Enter, Space or ArrowDown open it on the
  first item, the arrows wrap, Home and End jump, Escape closes it and gives the focus back
  to the button that opened it — so nobody is left with the focus on something that is no
  longer on screen. Tab and a press anywhere else close it as well.

  It is one component and not two because the panel and the expanded view offer the same
  three actions; only the trigger's shape differs, and that is a class.
-->
<script lang="ts">
  import { tick } from 'svelte';

  import { t } from '../i18n';
  import type { ActionName, ActionsView } from '../model';
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  const {
    actions,
    variant = 'tool',
    onact,
  }: {
    /** The whole block from the core; only the three below are read (§7.4). */
    actions: ActionsView;
    /** `tool` is the panel's icon-over-label column; `button` is the expanded view's row. */
    variant?: 'tool' | 'button';
    onact: (action: ActionName) => void;
  } = $props();

  /** The three, in the order RESP-08 fixes: Skip, Defer, then Abandon. */
  const ITEMS: ReadonlyArray<{
    action: ActionName;
    icon: IconName;
    offered: 'skip' | 'defer' | 'abandon';
  }> = [
    { action: 'skip', icon: 'skip', offered: 'skip' },
    { action: 'defer', icon: 'defer', offered: 'defer' },
    { action: 'abandon', icon: 'abandon', offered: 'abandon' },
  ];

  const offered = $derived(ITEMS.filter((item) => actions[item.offered]));

  let open = $state(false);
  let wrapper = $state<HTMLElement | null>(null);
  let trigger = $state<HTMLButtonElement | null>(null);
  let items = $state<Array<HTMLButtonElement | null>>([]);

  async function show(at: 'first' | 'last' | 'none'): Promise<void> {
    open = true;
    if (at === 'none') {
      return;
    }
    await tick();
    focusItem(at === 'first' ? 0 : offered.length - 1);
  }

  function hide(refocus: boolean): void {
    open = false;
    if (refocus) {
      trigger?.focus();
    }
  }

  /** Moves the focus to an item, wrapping: past the end is the first, before it is the last. */
  function focusItem(index: number): void {
    const count = offered.length;
    if (count === 0) {
      return;
    }
    items[((index % count) + count) % count]?.focus();
  }

  function focusedIndex(): number {
    return items.findIndex((item) => item !== null && item === document.activeElement);
  }

  function onTriggerKey(event: KeyboardEvent): void {
    // Enter and Space already produce a click on a button, which is the toggle below.
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      void show(event.key === 'ArrowDown' ? 'first' : 'last');
    } else if (event.key === 'Escape' && open) {
      event.preventDefault();
      hide(true);
    }
  }

  function onMenuKey(event: KeyboardEvent): void {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        focusItem(focusedIndex() + 1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        focusItem(focusedIndex() - 1);
        break;
      case 'Home':
        event.preventDefault();
        focusItem(0);
        break;
      case 'End':
        event.preventDefault();
        focusItem(offered.length - 1);
        break;
      case 'Escape':
        event.preventDefault();
        hide(true);
        break;
      case 'Tab':
        // Not prevented: the focus really does leave, and the menu goes with it.
        hide(false);
        break;
      default:
        break;
    }
  }

  /** Choosing closes the menu first, so the sheet Defer and Abandon open is not under it. */
  function choose(action: ActionName): void {
    hide(false);
    onact(action);
  }
</script>

<!--
  A press anywhere else closes it. `pointerdown` and not `click`, so the menu is already gone
  by the time the press lands on whatever was underneath it.
-->
<svelte:document
  onpointerdown={(event) => {
    const target = event.target;
    if (!open || (target instanceof Node && wrapper?.contains(target) === true)) {
      return;
    }
    hide(false);
  }}
/>

{#if offered.length > 0}
  <div class="more" bind:this={wrapper}>
    <button
      type="button"
      class={variant === 'tool' ? 'tool' : 'button'}
      bind:this={trigger}
      aria-haspopup="menu"
      aria-expanded={open}
      title={t('overlay.moreActions')}
      onclick={() => {
        if (open) {
          hide(false);
        } else {
          void show('none');
        }
      }}
      onkeydown={onTriggerKey}
    >
      <Icon name="more" size={variant === 'tool' ? 17 : 15} />
      {t('overlay.more')}
    </button>

    {#if open}
      <!--
        `onkeydown` on the container and not on each item: the keys of the pattern are about
        the menu, and an item never has to know where it sits in it.
      -->
      <div
        class="more-menu"
        role="menu"
        tabindex="-1"
        aria-label={t('overlay.moreActions')}
        onkeydown={onMenuKey}
      >
        {#each offered as item, index (item.action)}
          {#if item.action === 'abandon' && index > 0}
            <div class="menu-separator"></div>
          {/if}
          <button
            type="button"
            role="menuitem"
            class="menu-item"
            class:menu-item-danger={item.action === 'abandon'}
            bind:this={items[index]}
            onclick={() => choose(item.action)}
          >
            <Icon name={item.icon} size={15} />
            {t(`action.${item.action}`)}
          </button>
        {/each}
      </div>
    {/if}
  </div>
{/if}
