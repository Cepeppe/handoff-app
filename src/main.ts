/**
 * Entry point of the overlay frontend.
 *
 * Order matters here. The language is resolved and reported to the core *before* the
 * application is mounted, so the first paint is already in the right language and the tray
 * menu — whose texts the Rust side owns — is relabelled at the same moment (APP-02, §7.16).
 */
import { mount } from 'svelte';

import App from './App.svelte';
import { bridge } from './bridge';
import { resolveLanguage, setLanguage, systemLanguages } from './i18n';

// The stored setting does not exist yet: the settings store is built later, so the language
// comes from the system alone, which is the second step of the §7.16 rule.
// TASK: T-041 — read the stored language and pass it here.
const language = resolveLanguage(null, systemLanguages());
setLanguage(language);
void bridge().setUiLanguage(language);

const target = document.getElementById('app');

if (target === null) {
  throw new Error('the #app mount point is missing from index.html');
}

mount(App, { target });
