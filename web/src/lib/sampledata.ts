import { fmt } from './shellfmt';

/** What `GET /api/v1/samples/{db}` returns for the prebuilt dataset (sample_playground). */
export interface SampleDefinition {
  database: string;
  collections: { name: string; documents: unknown[]; searchIndex: { name?: string; type?: string; definition: unknown } }[];
}

/**
 * The dataset in the Data panel's own shape (`db = { collection: { documents, searchIndexes } }`), so what the examples
 * query reads like what you would type yourself.
 */
export function dataPanelText(d: SampleDefinition): string {
  const body = d.collections
    .map((c) => {
      const key = c.searchIndex.type === 'vectorSearch' ? 'vectorSearchIndexes' : 'searchIndexes';
      const index = fmt([{ definition: c.searchIndex.definition }], 2);
      return `  ${c.name}: {\n    documents: ${fmt(c.documents, 2)},\n    ${key}: ${index}\n  }`;
    })
    .join(',\n');
  return `// ${d.database}: prebuilt and already indexed, so Search queries run at once. Read-only.\ndb = {\n${body}\n}`;
}
