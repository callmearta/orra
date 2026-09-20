import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { Problem } from './api.ts';
import { problemOf } from './api.ts';

/**
 * What a rejected command becomes in the interface.
 *
 * Worth pinning down because the two shapes are not interchangeable: a command
 * that talks to a service rejects with a whole `Problem`, and one that does not
 * still rejects with a plain string. Getting this wrong is not a crash — it is
 * a card reading "[object Object]", which tells the user nothing and copies
 * nothing useful into a bug report.
 */

const reported: Problem = {
  kind: 'network',
  title: 'Could not reach the service',
  summary: 'Could not start dictating',
  advice: 'Check this machine’s connection and try again.',
  detail: 'could not reach the transcription service: Connection refused',
  log: 'Orra 0.1.2 (linux)\n...',
};

test('a reported failure is passed through whole', () => {
  // The log in particular: only the backend can build it, because only it
  // knows which values are keys.
  assert.equal(problemOf(reported), reported);
});

test('a plain rejection is wrapped, not rendered as an object', () => {
  const fromString = problemOf('could not save settings: permission denied');
  assert.equal(fromString.summary, 'could not save settings: permission denied');
  assert.equal(fromString.detail, 'could not save settings: permission denied');
  // Nothing that claims to know more than it does.
  assert.equal(fromString.kind, 'unknown');
  assert.equal(fromString.advice, '');

  const fromError = problemOf(new Error('boom'));
  assert.equal(fromError.summary, 'boom');
});

test('an object that is not a Problem is wrapped rather than trusted', () => {
  // Tauri can reject with its own shape; reading `title` off it would put an
  // empty headline on the card.
  const wrapped = problemOf({ code: 'E_NO_SUCH_COMMAND', message: 'nope' });
  assert.equal(wrapped.kind, 'unknown');
  assert.equal(wrapped.detail, '[object Object]');

  // And what it takes to be one: both fields, since `log` alone is what the
  // copy button reads.
  assert.equal(problemOf({ log: 'x' }).kind, 'unknown');
});
