<!--
  The Verifying view of §7.6 (VER-04, VER-05, VER-09, PRIN-08).

  Two halves, and the order matters. First what the agent is *about* to check, under "The
  agent should now check:", so that the user knows what "done" is being measured against
  (VER-04). Then, when the report arrives, what it said — with the label **declared by
  agent** on it (VER-05).

  That label is not decoration. PRIN-08 says a handoff is never "verified" on trust: the
  report is a declaration, the log keeps it as one, and the user is told which of the two
  they are reading. A `late` report — one that arrived after the handoff had already been
  declared not verified (DD-16, FM-26) — says so too, because "verified" and "verified
  three days later" are different facts.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { VerifyResultView } from '../model';

  const {
    verify,
    result,
  }: {
    /** What the agent will check: the spec's own `verify` text. */
    verify: string | null;
    /** What it reported, once it has. */
    result: VerifyResultView | null;
  } = $props();
</script>

<section class="verifying" data-ui-state="verifying">
  <!--
    Before the report, what matters is what is being checked; after it, what came back. The
    state word itself is not repeated here: the §8.4 banner of a final tab already carries
    it, and this says the two things the banner cannot — that it is a declaration, and when
    the agent could not check at all.
  -->
  {#if result === null}
    {#if verify !== null}
      <p class="verify-intro">{t('overlay.shouldCheck')}</p>
      <blockquote class="verify-text">{verify}</blockquote>
    {/if}
  {:else}
    <p class="verify-result" data-verify-ok={result.ok === null ? 'unknown' : String(result.ok)}>
      {#if result.ok === null}
        <span class="verify-state">{t('overlay.verifyUnknown')}</span>
      {/if}
      <span class="tag">{t('overlay.declaredByAgent')}</span>
      {#if result.late}
        <span class="tag tag-late">{t('overlay.late')}</span>
      {/if}
    </p>
    {#if result.detail !== null}
      <p class="verify-detail">{result.detail}</p>
    {/if}
  {/if}
</section>
