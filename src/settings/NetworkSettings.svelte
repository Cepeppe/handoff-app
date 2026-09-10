<!--
  Settings → Network (§7.6, §7.13, NET-01, NET-02, PRIN-05, NFR-05).

  The page NFR-05 rests one of its three verifiable facts on: *local-only communication*. It
  lists every outbound connection the app has recorded, and there is one writer of those rows
  — `net::egress`, which records a connection **before** making it — so a connection missing
  from this list is a connection that could not have happened.

  Two sentences sit above the list and neither is decoration:

  - **This build connects to nothing.** The update check is deferred with the public release
    (T-078), so the one caller the design foresees does not exist and the list is empty on
    every machine. An empty list on its own would leave a reader guessing whether nothing
    happened or nothing was recorded.
  - **The page is a self-declaration; the firewall test is the verification.** NET-02 asks for
    both, and says which is which. Claiming the page proves anything would be the one thing an
    application asking for trust must not do.

  There is nothing to press here. A list of what left the machine is something to read.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { moment } from '../datetime';
  import { t } from '../i18n';
  import type { NetworkEventView } from '../model';

  /** The stable `purpose` keys `net::egress` writes, and the sentence each one is. */
  const PURPOSES: Record<string, string> = { 'update-check': 'net.purposeUpdateCheck' };

  let events = $state<NetworkEventView[]>([]);
  let problem = $state<string | null>(null);

  onMount(() => {
    void load();
  });

  async function load(): Promise<void> {
    try {
      events = await bridge().networkEvents();
    } catch (error) {
      events = [];
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /**
   * The purpose as a sentence, or the key itself.
   *
   * A row written by a later version carries a key this one does not know; showing it is
   * more useful than hiding a connection because its label is missing.
   */
  function purpose(key: string): string {
    const catalogueKey = PURPOSES[key];
    return catalogueKey === undefined ? key : t(catalogueKey);
  }
</script>

<section class="settings-section" data-settings="network">
  <h2>{t('settings.network')}</h2>
  <p class="settings-explain">{t('net.zero')}</p>
  <p class="settings-explain">{t('net.declaration')}</p>

  {#if events.length === 0}
    <p class="settings-explain">{t('net.empty')}</p>
  {:else}
    <ul class="net-list">
      {#each events as event, index (index)}
        <li class="net-entry" data-net-entry={event.domain}>
          <p class="net-domain">{event.domain}</p>
          <p class="net-meta">
            {moment(event.at)} · {t('net.bytes', { bytes: event.bytesSent })} · {purpose(
              event.purpose,
            )}
          </p>
        </li>
      {/each}
    </ul>
  {/if}

  {#if problem !== null}
    <p class="settings-problem" role="alert">{problem}</p>
  {/if}
</section>
