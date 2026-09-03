import { describe, expect, it } from 'vitest';
import { telegramMentionToChoruz } from './mention.js';

describe('telegramMentionToChoruz', () => {
  it('leaves text unchanged before Telegram has loaded bot metadata', () => {
    expect(telegramMentionToChoruz('hello @other-agent', '')).toBe(
      'hello @other-agent',
    );
  });
});
