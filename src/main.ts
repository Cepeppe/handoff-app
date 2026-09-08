// Placeholder entry point of the frontend (TASK: T-028 replaces it with the Svelte
// application: view switching, the typed `invoke`/`listen` bridge and the en/it resources).
//
// It deliberately calls nothing on the Rust side: the capability file grants the webview
// no command yet, so an `invoke` here would fail at runtime rather than at build time.
const app = document.querySelector<HTMLElement>('#app');

if (app) {
  app.textContent = 'Baton';
}
