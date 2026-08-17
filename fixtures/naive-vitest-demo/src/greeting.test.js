import { expect, test } from 'vitest';
import { formatGreeting } from './greeting';

test('formatGreeting greets by name', () => {
  expect(formatGreeting('Ada')).toBe('Hello, Ada');
});
