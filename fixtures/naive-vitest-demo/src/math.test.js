import { expect, test } from 'vitest';
import { sum, isEqual, defaultActive } from './math';

test('sum adds two numbers', () => {
  expect(sum(2, 3)).toBe(5);
});

test('isEqual compares strictly', () => {
  expect(isEqual(2, 2)).toBe(true);
  expect(isEqual(2, 3)).toBe(false);
});

// Intentionally weak: checks the type, not the value, so a mutated boolean
// literal in defaultActive() survives — this is the demo's known survivor.
test('defaultActive returns a boolean', () => {
  expect(typeof defaultActive()).toBe('boolean');
});
