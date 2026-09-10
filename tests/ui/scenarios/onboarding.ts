/**
 * Onboarding, as far as the consent screen (F-13, INST-01, INST-02).
 *
 * A first launch shows onboarding by itself; its second step lists the agents found on the
 * machine, and **Register** opens the consent screen: "the exact list of changes, with a
 * diff", three modifications on two rows for Claude Code, and each row's **Show** opening the
 * diff of the places that row stands for.
 *
 * Claude Code is "found" through a stand-in `claude.cmd` in the attempt's own `bin/`, first on
 * the application's `PATH` (`install::claude_code::claude_on_path` only asks whether the file
 * is there), so the step looks the same on a runner that has never seen Claude Code and on a
 * developer's machine. The adapter then reads the machine's own `~/.claude.json` and
 * `~/.claude/settings.json` to plan — `app.ts` says why the home cannot be redirected — and
 * that is safe because this walk never presses **Accept**, and never reaches the end of
 * onboarding either, whose autostart box would write the machine's real login items. The last
 * check is INST-01 itself: both files are byte for byte what they were before the walk.
 */
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

import { expect, type UiScenario } from '../scenario.ts';
import { until } from '../wait.ts';

/** The two files the Claude Code adapter would write in user scope (§7.15). */
function adapterFiles(): string[] {
  return [join(homedir(), '.claude.json'), join(homedir(), '.claude', 'settings.json')];
}

/** What a file is, without holding what it says: its digest, or that it is absent. */
function fingerprint(path: string): string {
  return existsSync(path) ? createHash('sha256').update(readFileSync(path)).digest('hex') : 'absent';
}

export const onboarding: UiScenario = {
  id: 'onboarding-consent',
  covers: 'F-13, INST-01, INST-02',
  title: 'onboarding reaches the consent screen, each Show opens its diff, and nothing is written',
  onboarded: false,
  prepare(workspace) {
    // Never run: the adapter only asks whether a file of that name is on `PATH`.
    writeFileSync(join(workspace.bin, 'claude.cmd'), '@echo off\r\nexit /b 0\r\n');
  },
  async run({ page }) {
    const before = adapterFiles().map(fingerprint);

    await page.findText('[data-view="onboarding"] h1', 'Welcome to Baton');
    await page.click('.onboarding-actions button', 'Next');
    await page.findText('[data-view="onboarding"] h1', 'Your agents');
    await page.findText('.agent-list .agent-name', 'Claude Code');
    const registered = await page.query('.agent-list .agent-status', 'Registered');
    expect(
      registered === null,
      "this machine's Claude Code is already registered with Baton, so onboarding offers no consent " +
        'screen for it; the scenario needs a Claude Code configuration without the handoff entry',
    );
    await page.click('.agent-list button', 'Register');

    // INST-02: three modifications, on two rows, every diff closed until asked for.
    const intro = await page.text('.consent-intro');
    expect(intro.startsWith('3 changes'), 'the consent screen counts three modifications', intro);
    expect((await page.count('.consent-line')) === 2, 'on two rows: the MCP entry, and both hooks on one');
    expect((await page.count('.consent-diff')) === 0, 'no diff is open before Show is pressed');
    const closed = await page.attributes('.consent-show', 'aria-expanded');
    expect(closed.every((state) => state === 'false'), 'every Show says it is closed', closed);

    // The MCP entry: the row names the `handoff` entry, and its diff carries the per-server
    // timeout of T-026. What the diff shows around the value depends on what the file holds
    // already, so it is asserted on and never printed: on a developer's machine it is their own.
    const where = (await page.texts('.consent-where'))[0] ?? '';
    expect(where.includes('mcpServers') && where.includes('handoff'), 'the first row is the handoff entry of mcpServers');
    await page.clickNth('.consent-show', 0);
    await until('the first row opened', async () => (await page.attributes('.consent-show', 'aria-expanded'))[0] === 'true');
    const entry = await page.text('.consent-diff');
    expect(entry.includes('1800000'), 'the diff shows the per-server timeout the entry is written with');
    await page.clickNth('.consent-show', 0);
    await page.gone('.consent-diff');

    // The hooks row: both hooks, one command.
    await page.clickNth('.consent-show', 1);
    const hooks = await page.text('.consent-diff');
    expect(hooks.includes('SubagentStop') && hooks.includes('hook stop'), 'the diff shows both hooks running the same command');

    // Not now: back to the list, with Register still offered.
    await page.click('[data-view="onboarding"] .actions button', 'Not now');
    await page.findText('.agent-list button', 'Register');

    // INST-01: nothing is written that the user has not accepted.
    const after = adapterFiles().map(fingerprint);
    expect(
      JSON.stringify(after) === JSON.stringify(before),
      '~/.claude.json and ~/.claude/settings.json are byte for byte what they were',
    );
  },
};
