/**
 * Entry point of the overlay frontend.
 *
 * Order matters here. The language is resolved and reported to the core *before* the
 * application is mounted, so the first paint is already in the right language and the tray
 * menu — whose texts the Rust side owns — is relabelled at the same moment (APP-02, §7.16).
 *
 * That costs one round trip to the core before anything is drawn, and it is the right trade:
 * the alternative is mounting in the system language and repainting the whole window a tick
 * later, which every user who chose the other language would see at every launch. A core
 * that cannot answer — a browser, a settings table that would not open — leaves the setting
 * unread, and the resolution falls through to the system, which is the §7.16 rule anyway.
 */
import { mount } from 'svelte';

import App from './App.svelte';
import { bridge } from './bridge';
import { resolveLanguage, setLanguage, systemLanguages } from './i18n';

const stored = await bridge()
  .generalSettings()
  .then((settings) => settings.language)
  .catch(() => null);

const language = resolveLanguage(stored, systemLanguages());
setLanguage(language);
void bridge().setUiLanguage(language);

const target = document.getElementById('app');

if (target === null) {
  throw new Error('the #app mount point is missing from index.html');
}

mount(App, { target });
