/**
 * Test accounts. `seed.sh` creates them; nothing here is a real person's data or a secret. Override with
 * E2E_EMAIL_A / E2E_EMAIL_B / E2E_PASSWORD to use accounts you made yourself.
 */
export const PASSWORD = process.env.E2E_PASSWORD ?? 'E2E-Pass-2026-local-only'

export const accountA = { email: process.env.E2E_EMAIL_A ?? 'e2e-a@example.test', name: process.env.E2E_NAME_A ?? 'E2E Alpha' }
export const accountB = { email: process.env.E2E_EMAIL_B ?? 'e2e-b@example.test', name: process.env.E2E_NAME_B ?? 'E2E Bravo' }
