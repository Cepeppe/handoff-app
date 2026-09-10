<!--
  Settings → Updates (§7.6, §7.13, UPD-01, UPD-02).

  One line, because there is one fact: this build never checks. UPD-01 and UPD-02 are
  suspended with the public release (`TASKS.md` §0.4 item 8) and the update domain of OI-12 is
  not decided, so there is no setting to offer and nothing to switch off — a "check for
  updates" box that did nothing either way would be worse than the sentence.

  T-078 completes the page: the setting, the last check, and the notice with its **Download**
  and **Remind me later**. The section exists now so that the Network page beside it is not
  the only place the user can look for "does this thing phone home".
-->
<script lang="ts">
  import { t } from '../i18n';
</script>

<section class="settings-section" data-settings="updates">
  <h2>{t('settings.updates')}</h2>
  <p class="settings-explain">{t('updates.none')}</p>
</section>
