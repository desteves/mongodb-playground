import { describe, expect, it } from 'vitest';
import { dataPanelText } from './sampledata';

const def = {
  database: 'sample_playground',
  collections: [
    { name: 'movies', documents: [{ title: 'Alien', year: 1979 }], searchIndex: { name: 'default', definition: { mappings: { dynamic: true } } } },
    { name: 'items', documents: [{ name: 'red apple', embedding: [0.9, 0.1, 0] }], searchIndex: { name: 'vector_index', type: 'vectorSearch', definition: { fields: [{ type: 'vector', path: 'embedding', numDimensions: 3, similarity: 'cosine' }] } } },
  ],
};

describe('dataPanelText', () => {
  const text = dataPanelText(def);

  it('uses the data panel shape, with the right index key per collection', () => {
    expect(text).toContain('db = {');
    expect(text).toContain('movies: {');
    expect(text).toContain('documents: [');
    expect(text).toMatch(/searchIndexes: \[/);
    expect(text).toMatch(/vectorSearchIndexes: \[/);
    expect(text).toContain('title: "Alien"');
  });

  it('says it is prebuilt and read-only', () => {
    expect(text.split('\n')[0]).toContain('sample_playground');
    expect(text.split('\n')[0]).toContain('Read-only');
  });

  it('parses as the same braces and brackets it opens', () => {
    const count = (c: string) => text.split(c).length - 1;
    expect(count('{')).toBe(count('}'));
    expect(count('[')).toBe(count(']'));
  });
});
