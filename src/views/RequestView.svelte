<!--
  The request sheet of §7.7 (OPEN-03, OPEN-04, OPEN-04a, OPEN-05).

  The one place a *person* starts a handoff. The global shortcut and the tray's `New request`
  both land here; it is a mode of the single window (DD-10), so the tab strip stays behind it
  and there is never a second window to position (MULTI-04).

  Two fields and two keys, exactly as OPEN-04 writes them: a session selector, pre-selected
  when only one session is registered, and "What are you about to do?". **Enter sends, Esc
  cancels.** It is a single-line input for that reason — a textarea would make Enter a
  newline, and the design gives Enter to sending.

  What happens on send is the Rust side's (`create_request`): the entry is queued, the tab
  appears in "waiting for spec" at once, the sentence goes on the clipboard in the user's
  language and the session's terminal is brought forward if it can be found. Nothing here
  waits for any of that beyond the id, because none of it can fail in a way the user has to
  answer: the Stop hook delivers the same request at the end of the agent's next turn
  (OPEN-06).

  With no session registered the sheet still opens and still sends (OPEN-04a): the request is
  queued for the first session that starts, and the notice says so.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { SessionChoice } from '../model';
  import { refreshTabs, select } from '../overlay/state.svelte';
  import { resetView } from '../view-state.svelte';

  let sessions = $state<SessionChoice[]>([]);
  let chosen = $state<string | null>(null);
  let text = $state('');
  let sending = $state(false);
  let problem = $state<string | null>(null);
  let field = $state<HTMLInputElement | null>(null);

  const ready = $derived(text.trim().length > 0);

  onMount(() => {
    void load();
    // The sheet is opened by a shortcut pressed in another application, so the caret has to
    // be here without a click: what the user does next is type.
    field?.focus();
  });

  /** The sessions the request may be addressed to, and the pre-selection of OPEN-04. */
  async function load(): Promise<void> {
    sessions = await bridge().sessions();
    // "Pre-selected when only one session is active" (OPEN-04). With several, the first one
    // is offered and the user changes it: a `<select>` has to show something, and the oldest
    // registration is the one they have most likely been working in. With none, the request
    // is addressed to nobody and OPEN-04a queues it for the first session that starts.
    chosen = sessions.length === 0 ? null : sessions[0].sessionRef;
  }

  async function send(): Promise<void> {
    if (!ready || sending) {
      return;
    }
    sending = true;
    problem = null;
    try {
      const id = await bridge().createRequest(text.trim(), chosen);
      text = '';
      // Back to the overlay, on the tab that has just appeared: the user's next move is to
      // paste into the agent, and what they will come back to is that tab (OPEN-04).
      //
      // Without asking for the front (OPEN-05). `create_request` has just raised the
      // session's terminal so the user can paste, and MULTI-03's "a handoff arrived" rule
      // would take the focus straight back — measured against the running app, where the
      // terminal came forward and the overlay was in front of it again a tick later.
      await refreshTabs(false);
      await select(id);
      resetView();
    } catch {
      problem = t('request.failed');
    } finally {
      sending = false;
    }
  }

  /** Esc cancels: the sheet closes and nothing is queued (OPEN-04). */
  function cancel(): void {
    text = '';
    problem = null;
    resetView();
  }
</script>

<section class="view" data-view="request">
  <h1>{t('view.request')}</h1>

  {#if sessions.length === 0}
    <p class="request-no-session" role="status">{t('request.noSession')}</p>
  {:else}
    <label class="request-session-label" for="request-session">{t('request.session')}</label>
    <select
      id="request-session"
      class="request-session"
      bind:value={chosen}
      onkeydown={(event) => {
        if (event.key === 'Escape') {
          cancel();
        }
      }}
    >
      {#each sessions as session (session.sessionRef)}
        <option value={session.sessionRef}>{session.label}</option>
      {/each}
    </select>
  {/if}

  <label class="request-what-label" for="request-what">{t('request.what')}</label>
  <input
    id="request-what"
    class="request-what"
    type="text"
    bind:this={field}
    bind:value={text}
    onkeydown={(event) => {
      if (event.key === 'Escape') {
        cancel();
      } else if (event.key === 'Enter') {
        event.preventDefault();
        void send();
      }
    }}
  />

  <p class="request-hint">{t('request.hint')}</p>

  {#if problem !== null}
    <p class="request-problem" role="alert">{problem}</p>
  {/if}
  {#if !ready && text.length > 0}
    <p class="request-problem" role="alert">{t('request.empty')}</p>
  {/if}

  <div class="request-actions" role="group" aria-label={t('overlay.actions')}>
    <button type="button" class="button" disabled={!ready || sending} onclick={() => void send()}>
      {t('request.send')}
    </button>
    <button type="button" class="button button-quiet" onclick={cancel}>
      {t('sheet.cancel')}
    </button>
  </div>
</section>
