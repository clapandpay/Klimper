// SPDX-License-Identifier: AGPL-3.0-or-later
// The option-contract methods a website may be granted (clapandpay TASK-140).

/**
 * The option-contract methods (TASK-140). Never granted by default: a website gets them only
 * when the user turns on "Option contracts" when connecting or in its settings, since each
 * one locks funds until a date (or pays to free them).
 */
export const OPTION_METHODS = [
  'chia_mintOption',
  'chia_exerciseOption',
  'chia_claimOption',
  'chia_transferOption',
] as const;

/** All four option methods are granted. */
export function allowsOptions(methods: readonly string[]): boolean {
  return OPTION_METHODS.every((method) => methods.includes(method));
}

/** `methods` with the option methods added or taken away, every other method kept. */
export function withOptionMethods(
  methods: readonly string[],
  on: boolean,
): string[] {
  const rest = methods.filter(
    (method) => !(OPTION_METHODS as readonly string[]).includes(method),
  );
  return on ? [...rest, ...OPTION_METHODS] : rest;
}
