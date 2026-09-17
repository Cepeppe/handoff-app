<!--
  The `secrets` list (SEC-01, SEC-02).

  The overlay never receives a secret value: what a spec declares here is a variable name and
  the file it belongs in. **Open file** opens that file with the operating system's default
  application; the path is resolved on the Rust side from the spec and the session's project
  folder, so this component sends the entry's *name* and never a path.

  The dashed outline is the one dashed thing in the panel, and it is deliberate: everything
  else is a value the window has, and this is a value the window has not got.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { SecretEntryView } from '../model';
  import Icon from './Icon.svelte';

  const { handoffId, secrets }: { handoffId: string; secrets: SecretEntryView[] } = $props();
</script>

{#if secrets.length > 0}
  <section class="secrets">
    <span class="section-label">{t('overlay.secrets')}</span>
    {#each secrets as secret (secret.name)}
      <div class="secret-entry">
        <Icon name="lock" />
        <span class="secret-what">
          <span class="secret-name">{secret.name}</span>
          <span class="secret-file">{secret.file}</span>
        </span>
        <button
          type="button"
          class="button button-small"
          aria-label={`${t('overlay.openFile')}: ${secret.file}`}
          onclick={() => void bridge().openSecretFile(handoffId, secret.name)}
        >
          {t('overlay.openFile')}
        </button>
      </div>
    {/each}
  </section>
{/if}
