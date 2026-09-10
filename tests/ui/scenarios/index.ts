/**
 * The scenarios of the UI suite, in the order they run (T-055, §11.4).
 *
 * The order is the order of the window a person meets: onboarding last, because it is the
 * one scenario that starts from a machine that has never run Baton.
 */
import type { UiScenario } from '../scenario.ts';
import { collapse } from './collapse.ts';
import { onboarding } from './onboarding.ts';
import { preview, previewTextOnly } from './preview.ts';
import { requestSheet } from './request-sheet.ts';
import { settings } from './settings.ts';
import { stepView } from './step-view.ts';

export const SCENARIOS: readonly UiScenario[] = [
  stepView,
  collapse,
  requestSheet,
  preview,
  previewTextOnly,
  settings,
  onboarding,
];
