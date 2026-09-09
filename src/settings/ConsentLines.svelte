<!--
  The consent screen's list of changes (INST-01, INST-02, F-13).

  The same component in both places it is needed: onboarding shows it before the first
  registration, and the Agents settings page shows it again for a Register or a Repair. There
  is one screen because there is one promise — the user reads the exact change before it is
  written — and a second rendering of it would be a second chance to get that wrong.

  Two numbers that are deliberately different. `modificationCount` is what INST-02 counts:
  three for Claude Code. The rows are what the user reads: two, because the Stop and the
  SubagentStop hook are "the same command" and INST-02 asks for them on one line. The Rust
  side decides both (`install::InstallAdapter::consent_lines`); nothing here groups anything.

  **Show** is per row and closed by default. What it reveals is the diff of the places that
  row stands for — both hooks on the hooks row — so "same command" is something the user can
  check rather than something they are told.

  There is no timeout checkbox and no sentence about all MCP servers: the installer writes
  the per-server `timeout` field alone (T-026, Option B, 2026-09-08), which needs no consent
  of its own because it bounds our server and nothing else.
-->
<script lang="ts">
  import { t } from '../i18n';
  import type { ConsentView } from '../model';

  const { plan }: { plan: ConsentView } = $props();

  /** Which rows are open. Closed by default: the summary is the screen, the diff is on ask. */
  let opened = $state<Record<string, boolean>>({});

  function key(index: number, locations: string[]): string {
    return `${index}:${locations.join('|')}`;
  }
</script>

<p class="consent-intro">
  {t('install.consentIntro', {
    count: plan.modificationCount,
    agent: t(plan.nameKey),
  })}
</p>

<ul class="consent-lines">
  {#each plan.lines as line, index (key(index, line.locations))}
    {@const id = key(index, line.locations)}
    <li class="consent-line" class:consent-noop={line.isNoop}>
      <p class="consent-what">{t(line.description.key, line.description.args)}</p>
      <p class="consent-where">{line.locations.join(' · ')}</p>
      <p class="consent-state">{line.isNoop ? t('install.inOrder') : t('install.willChange')}</p>
      <button
        type="button"
        class="button button-quiet consent-show"
        aria-expanded={opened[id] === true}
        onclick={() => (opened = { ...opened, [id]: opened[id] !== true })}
      >
        {opened[id] === true ? t('install.hide') : t('install.show')}
      </button>
      {#if opened[id] === true}
        <pre class="consent-diff">{line.diff}</pre>
      {/if}
    </li>
  {/each}
</ul>
