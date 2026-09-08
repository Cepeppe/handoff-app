import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

// The components are written in TypeScript (`<script lang="ts">`), which the preprocessor
// strips before the Svelte compiler sees them. Declared explicitly rather than left to the
// plugin's default so that `svelte-check` and the editor read the same configuration the
// build uses.
export default {
  preprocess: vitePreprocess(),
};
