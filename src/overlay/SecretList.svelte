<!--
  The `secrets` list (SEC-01, SEC-02).

  The overlay never receives a secret value: what a spec declares here is a variable name and
  the file it belongs in. **Open file** opens that file with the operating system's default
  application; the path is resolved on the Rust side from the spec and the session's project
  folder, so this component sends the entry's *name* and never a path.
-->
<script lang="ts">
  import { bridge } from '../bridge';
  import { t } from '../i18n';
  import type { SecretEntryView } from '../model';

  const { handoffId, secrets }: { handoffId: string; secrets: SecretEntryView[] } = $props();
</script>

{#if secrets.length > 0}
  <section class="secrets">
    <h2>{t('overlay.secrets')}</h2>
    <ul>
      {#each secrets as secret (secret.name)}
        <li>
          <span class="secret-name">{secret.name}</span>
          <span class="secret-file">{secret.file}</span>
          <button
            type="button"
            class="chip-action"
            aria-label={`${t('overlay.openFile')}: ${secret.file}`}
            onclick={() => void bridge().openSecretFile(handoffId, secret.name)}
          >
            {t('overlay.openFile')}
          </button>
        </li>
      {/each}
    </ul>
  </section>
{/if}
