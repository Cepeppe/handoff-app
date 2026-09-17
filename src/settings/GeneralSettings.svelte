<!--
  Settings → General (§7.6, §7.16, APP-01, APP-02, OPEN-03, R-10).

  Four preferences that share nothing but this page, and one habit that holds them together:
  every control shows what is **in force**, not what was once asked for. The language is the
  one the window is running in, the autostart box is the login entry the operating system
  actually has, the shortcut line is the combination the system accepted, and the collapse
  box is the setting the window read at mount.

  - **Language** is applied at once, with no restart (APP-02). Switching it repaints every
    label on screen, because `t()` reads a rune (`i18n.svelte.ts`); the Rust side is told
    separately so the tray menu changes in the same instant. **System** is the *absence* of a
    setting rather than a third value, so a machine whose system language changes follows it.
  - **Autostart** writes the operating system's login items straight away and remembers the
    answer. Onboarding asked the same question with the same sentence (APP-01), and it is
    deliberately the same two texts: a preference said twice in two wordings is a preference
    the user has to read twice.
  - **The shortcut** reuses the recorder of the FM-18 dialog. Nothing is wrong here, so the
    second button cancels rather than dismissing a question for ever.
  - **Collapse on a timer** is the R-10 fallback, off unless asked for: it shrinks a panel
    somebody may be reading, which is only worth it where the blur event is unreliable.

  # The controls are the platform's, wearing something else

  The language choice looks like a segmented control and the two settings look like switches,
  and all three **are** what they were: three `<input type="radio">` in one group, and two
  `<input type="checkbox">`. Nothing here carries `role="switch"` or rebuilds a control out of
  buttons, because that is how a page loses the keyboard, the form semantics and the labels
  that a test and a screen reader both read. What changes is the paint: `appearance: none` and
  a knob drawn by the stylesheet.
-->
<script lang="ts">
  import { onMount } from 'svelte';

  import { bridge } from '../bridge';
  import { resolveLanguage, setLanguage, systemLanguages, t, type Language } from '../i18n';
  import { keycaps } from '../keys';
  import { fallbackIsOn, loadWindowSettings } from '../overlay/collapse.svelte';
  import ShortcutRecorder from '../overlay/ShortcutRecorder.svelte';

  /** What the language control offers: the two languages, and following the system. */
  const LANGUAGE_CHOICES = [
    { value: 'system', labelKey: 'settings.languageSystem' },
    { value: 'en', labelKey: 'settings.languageEn' },
    { value: 'it', labelKey: 'settings.languageIt' },
  ] as const;

  type LanguageChoice = (typeof LANGUAGE_CHOICES)[number]['value'];

  let chosen = $state<LanguageChoice>('system');
  let autostart = $state(false);
  let collapseFallback = $state(false);
  let accelerator = $state<string | null>(null);
  let shortcutRegistered = $state(true);
  let recording = $state(false);
  let problem = $state<string | null>(null);

  onMount(() => {
    void load();
  });

  /** Everything this page shows, read from the core rather than remembered anywhere. */
  async function load(): Promise<void> {
    await loadGeneral();
    await loadShortcut();
    // R-10: the window read this at mount, and this page is also reachable on its own.
    await loadCollapseFallback();
  }

  /**
   * The language and the login entry, as the core has them.
   *
   * Every control on this page is bound to its variable in both directions, so a re-read is
   * also how a refused write is undone: the box goes back to what the machine has rather
   * than staying where the click left it.
   */
  async function loadGeneral(): Promise<void> {
    try {
      const settings = await bridge().generalSettings();
      chosen = settings.language ?? 'system';
      autostart = settings.autostart;
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
  }

  /** The R-10 setting, into the window's own state and into this checkbox. */
  async function loadCollapseFallback(): Promise<void> {
    await loadWindowSettings();
    collapseFallback = fallbackIsOn();
  }

  async function loadShortcut(): Promise<void> {
    try {
      const status = await bridge().shortcutStatus();
      accelerator = status.accelerator;
      shortcutRegistered = status.registered;
    } catch {
      // A shortcut whose state cannot be read is not worth a red line on a settings page:
      // the line simply says nothing, and the tray's `New request` is unaffected (FM-18).
      accelerator = null;
    }
  }

  /**
   * The language the user picked, applied now and remembered (APP-02).
   *
   * The order is: store the choice, resolve what to run in, switch the window, tell the
   * core. Resolving after storing is what makes **System** work — it falls through to the
   * platform's preference list, which is the second step of the §7.16 rule.
   */
  async function chooseLanguage(choice: LanguageChoice): Promise<void> {
    chosen = choice;
    problem = null;
    const setting: Language | null = choice === 'system' ? null : choice;
    try {
      await bridge().setLanguageSetting(setting);
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
    const resolved = resolveLanguage(setting, systemLanguages());
    setLanguage(resolved);
    void bridge().setUiLanguage(resolved);
  }

  /** APP-01, both halves: the login entry and the answer behind it. */
  async function chooseAutostart(enabled: boolean): Promise<void> {
    problem = null;
    try {
      await bridge().setAutostart(enabled);
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
      // The box goes back to what the machine actually has: a checkbox that stayed ticked
      // over a login entry that was never written is the one thing this page must not do.
      await loadGeneral();
    }
  }

  /** R-10: the fallback timer, and the re-read the window needs to arm or disarm it. */
  async function chooseCollapseFallback(enabled: boolean): Promise<void> {
    problem = null;
    try {
      await bridge().setCollapseFallback(enabled);
    } catch (error) {
      problem = t('install.failed', { reason: String(error) });
    }
    await loadCollapseFallback();
  }
</script>

<section class="settings-section" data-settings="general">
  <h2>{t('settings.general')}</h2>

  <fieldset class="settings-group">
    <legend class="section-label">{t('settings.language')}</legend>
    <div class="seg">
      {#each LANGUAGE_CHOICES as choice (choice.value)}
        <label class="seg-option">
          <input
            type="radio"
            name="language"
            value={choice.value}
            checked={chosen === choice.value}
            onchange={() => void chooseLanguage(choice.value)}
          />
          <span class="seg-pill">{t(choice.labelKey)}</span>
        </label>
      {/each}
    </div>
  </fieldset>

  <fieldset class="settings-group">
    <legend class="section-label">{t('settings.startup')}</legend>
    <div class="settings-box">
      <p class="settings-explain">{t('onboarding.autostartText')}</p>
      <label class="switch-row">
        <span class="switch-text">{t('onboarding.autostart')}</span>
        <input
          type="checkbox"
          class="switch"
          bind:checked={autostart}
          onchange={() => void chooseAutostart(autostart)}
        />
      </label>
    </div>
  </fieldset>

  <fieldset class="settings-group">
    <legend class="section-label">{t('settings.shortcut')}</legend>
    <div class="settings-box">
      {#if accelerator !== null}
        <!--
          The combination as the keys a person presses. The sentence around them is the
          catalogue's, with its placeholder left empty: the caps *are* the accelerator, and
          `Ctrl` and `Alt` are the same word in both languages (`keys.ts`).
        -->
        <p class="settings-shortcut">
          <span class="settings-said">{t('settings.shortcutInForce', { accelerator: '' })}</span>
          <span class="keycaps">
            {#each keycaps(accelerator) as cap, index (index)}<span class="kbd">{cap}</span>{/each}
          </span>
        </p>
        {#if !shortcutRegistered}
          <p class="settings-problem" role="alert">{t('settings.shortcutRefused')}</p>
        {/if}
      {/if}
      {#if recording}
        <ShortcutRecorder
          onsaved={() => {
            recording = false;
            void loadShortcut();
          }}
          oncancel={() => (recording = false)}
          cancelLabel={t('sheet.cancel')}
        />
      {:else}
        <button type="button" class="button" onclick={() => (recording = true)}>
          {t('settings.shortcutChange')}
        </button>
      {/if}
    </div>
  </fieldset>

  <fieldset class="settings-group">
    <legend class="section-label">{t('settings.panel')}</legend>
    <div class="settings-box">
      <p class="settings-explain">{t('settings.collapseFallbackText')}</p>
      <label class="switch-row">
        <span class="switch-text">{t('settings.collapseFallback')}</span>
        <input
          type="checkbox"
          class="switch"
          bind:checked={collapseFallback}
          onchange={() => void chooseCollapseFallback(collapseFallback)}
        />
      </label>
    </div>
  </fieldset>

  {#if problem !== null}
    <p class="settings-problem" role="alert">{problem}</p>
  {/if}
</section>
