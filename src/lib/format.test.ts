import { expect, it } from 'vitest';
import { bytes, megabytes, megabytesOf, percent, sentence } from './format';

const MEGABYTE = 1024 * 1024;

it('megabytes have one decimal', () => {
  expect(megabytes(18 * MEGABYTE)).toBe('18.0 MB');
  expect(megabytesOf(11.2 * MEGABYTE, 18 * MEGABYTE)).toBe('11.2 of 18.0 MB');
});

it('a byte count gets the unit that fits it', () => {
  expect(bytes(512)).toBe('512 B');
  expect(bytes(3482)).toBe('3.4 KB');
  expect(bytes(2 * MEGABYTE)).toBe('2.0 MB');
});

it('a percentage stays between 0 and 100, also without a total', () => {
  expect(percent(5, 10)).toBe(50);
  expect(percent(20, 10)).toBe(100);
  expect(percent(-1, 10)).toBe(0);
  expect(percent(5, 0)).toBe(0);
  expect(percent(Number.NaN, 10)).toBe(0);
});

it('a sentence ends with one full stop', () => {
  expect(sentence('Close the game first')).toBe('Close the game first.');
  expect(sentence('Done.')).toBe('Done.');
});
