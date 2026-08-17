const { formatGreeting } = require('./greeting');

test('formatGreeting greets by name', () => {
  expect(formatGreeting('Ada')).toBe('Hello, Ada');
});
