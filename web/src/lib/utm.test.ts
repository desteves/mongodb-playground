import { describe, expect, it } from 'vitest';
import { withUtm } from './utm';

const SET = 'utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves';

describe('withUtm', () => {
  it('tags every mongodb.com variant', () => {
    for (const u of [
      'https://mongodb.com/docs/',
      'https://www.mongodb.com/cloud/atlas/register',
      'https://cloud.mongodb.com/v2',
      'https://dochub.mongodb.com/core/x',
    ]) {
      expect(new URL(withUtm(u)).search).toContain(SET);
    }
  });

  it('keeps other parameters and the fragment, and replaces old utm values', () => {
    expect(withUtm('https://www.mongodb.com/docs?a=1&utm_source=old#top')).toBe(`https://www.mongodb.com/docs?a=1&${SET}#top`);
  });

  it('leaves every other link alone', () => {
    for (const u of [
      'https://github.com/desteves/mongodb-playground',
      'https://notmongodb.com/',
      'https://mongodb.com.evil.example/',
      'mongodb+srv://cluster0.abcde.mongodb.net/',
      'not a url',
    ]) {
      expect(withUtm(u)).toBe(u);
    }
  });
});
