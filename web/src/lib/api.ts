import { invoke } from '@tauri-apps/api/core';
import { sessionHeader } from './turnstile';

/** True inside the Tauri desktop app, where queries run in-process against a local MongoDB. */
export const isDesktop = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export class ApiError extends Error {
  constructor(public code: string, message: string, public hint?: string) {
    super(message);
  }
}

/** The desktop shell returns the same {code, message, hint} object the HTTP API does. */
async function ipc<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (e: any) {
    throw new ApiError(e?.code ?? 'error', e?.message ?? String(e), e?.hint);
  }
}

/** A shell-style request: the data panel (documents, indexes, search indexes) and a script. */
export interface ScriptBody {
  script: string;
  /** The data panel text: `[docs]`, `db = { coll: [docs] }`, or per-collection `{documents, indexes, ...}`. */
  config?: string;
  datasource?: { sample: string };
}

export interface Example {
  id: string;
  title: string;
  kind: 'template' | 'example';
  config?: string;
  script: string;
  datasource?: { sample: string };
}

export interface ExplainOutput {
  raw: unknown;
  summary: {
    verbosity: string;
    stages: string[];
    indexUsed: string | null;
    collscan: boolean;
    keysExamined: number | null;
    docsExamined: number | null;
    returned: number | null;
    executionTimeMs: number | null;
    advice: string[];
  };
}

export interface StatementOutput {
  index: number;
  line: number;
  statement: string;
  collection: string;
  op: string;
  results: unknown[];
  write?: Record<string, number>;
  explain?: ExplainOutput;
  truncated?: boolean;
  error?: { code: string; message: string; hint: string };
  elapsed_ms: number;
}

export interface ScriptOutput {
  statements: StatementOutput[];
  ok: boolean;
  error?: { code: string; message: string; hint: string };
  elapsed_ms: number;
}

const headers = () => ({ 'content-type': 'application/json', 'x-session-id': sessionId(), ...sessionHeader() });

async function post<T>(url: string, body: unknown): Promise<T> {
  const res = await fetch(url, { method: 'POST', headers: headers(), body: JSON.stringify(body) });
  const text = await res.text();
  let data: any;
  try {
    data = JSON.parse(text);
  } catch {
    throw new ApiError('bad_response', text || `HTTP ${res.status}`);
  }
  if (!res.ok) {
    throw new ApiError(data?.error?.code ?? 'error', data?.error?.message ?? `HTTP ${res.status}`, data?.error?.hint);
  }
  return data as T;
}

export const run = (body: ScriptBody): Promise<ScriptOutput> =>
  isDesktop ? ipc('run_query', { requestBody: body }) : post('/api/v1/run', body);

export const explain = (body: ScriptBody): Promise<ScriptOutput> =>
  isDesktop ? ipc('explain_query', { requestBody: body }) : post('/api/v1/explain', body);

export async function examples(): Promise<Example[]> {
  if (isDesktop) return ipc('list_examples');
  const res = await fetch('/api/v1/examples');
  if (!res.ok) throw new ApiError('examples', `HTTP ${res.status}`);
  return res.json();
}

/** Random id kept in localStorage and rotated daily: lets usage be counted without identifying anyone. */
export function sessionId(): string {
  try {
    const day = Math.floor(Date.now() / 86_400_000);
    const saved = JSON.parse(localStorage.getItem('sid') ?? 'null');
    if (saved && saved.day === day) return saved.id;
    const id = crypto.randomUUID();
    localStorage.setItem('sid', JSON.stringify({ day, id }));
    return id;
  } catch {
    return 'anonymous';
  }
}

/** Fire-and-forget usage event (metadata only, never the editor contents). */
export function sendEvent(type: string, extra: Record<string, string> = {}): void {
  if (isDesktop) return; // the desktop app sends nothing anywhere
  fetch('/api/v1/events', { method: 'POST', headers: headers(), body: JSON.stringify({ type, ...extra }) }).catch(() => {});
}

export async function share(body: ScriptBody): Promise<{ id: string; url: string }> {
  if (isDesktop) throw new ApiError('unavailable', 'Share links need the hosted playground (mongoplayground.com).');
  // The server stores the data panel under `collections`.
  return post('/api/v1/share', { script: body.script, collections: body.config, datasource: body.datasource });
}

export interface Shared {
  id: string;
  createdAt: number;
  script?: string;
  collections?: unknown;
  datasource?: { sample: string };
}

export async function loadShared(id: string): Promise<Shared> {
  const res = await fetch(`/api/v1/p/${encodeURIComponent(id)}`);
  const data = await res.json();
  if (!res.ok) throw new ApiError(data?.error?.code ?? 'error', data?.error?.message ?? `HTTP ${res.status}`);
  return data;
}

export interface SampleInfo {
  available: boolean;
  databases: { name: string; collections: string[] }[];
}

export async function samples(): Promise<SampleInfo> {
  if (isDesktop) return { available: false, databases: [] };
  const res = await fetch('/api/v1/samples');
  if (!res.ok) return { available: false, databases: [] };
  return res.json();
}

export async function samplePreview(db: string, collection: string): Promise<unknown[]> {
  const res = await fetch(`/api/v1/samples/${encodeURIComponent(db)}/${encodeURIComponent(collection)}`);
  const data = await res.json();
  if (!res.ok) throw new ApiError(data?.error?.code ?? 'error', data?.error?.message ?? `HTTP ${res.status}`);
  return data.documents;
}

export const getConnection = () => ipc<string>('get_connection');
export const setConnection = (uri: string) => ipc<void>('set_connection', { uri });

export interface GuideStep {
  label: string;
  command?: string;
  url?: string;
}

export interface Guide {
  title: string;
  summary: string;
  atlasRegister: { url: string; utm: Record<string, string> };
  note?: string;
  options: { id: string; title: string; summary?: string; steps: GuideStep[] }[];
}

export async function guide(): Promise<Guide | null> {
  if (isDesktop) return null;
  try {
    const res = await fetch('/api/v1/guide');
    return res.ok ? res.json() : null;
  } catch {
    return null;
  }
}

/** The Atlas sign-up link with UTM parameters; `content` says where it was shown. */
export function atlasUrl(g: Guide | null, content: string): string {
  const base = g?.atlasRegister.url ?? 'https://www.mongodb.com/cloud/atlas/register';
  const utm = g?.atlasRegister.utm ?? {
    utm_source: 'mongoplayground',
    utm_medium: 'referral',
    utm_campaign: 'playground-free-cluster',
  };
  return `${base}?${new URLSearchParams({ ...utm, utm_content: content })}`;
}

export interface Limits {
  maxCollections: number;
  maxDocsPerCollection: number;
  maxPayloadBytes: number;
  maxTimeMs: number;
  maxIndexesPerCollection: number;
  maxSearchIndexesPerCollection: number;
  searchSupported: boolean;
  sampleSupported: boolean;
  maxShareBytes: number;
  shareTtlDays: number;
  features: { sampleData: boolean };
  deployment?: { cluster: string };
  rateLimit?: { basic: { burst: number; perSecond: number }; verified: { burst: number; perSecond: number } };
  turnstile?: { enabled: boolean; siteKey: string | null };
}

export async function limits(): Promise<Limits | null> {
  if (isDesktop) return null;
  try {
    const res = await fetch('/api/v1/limits');
    return res.ok ? res.json() : null;
  } catch {
    return null;
  }
}

/** Ask the server to export the playground (a mongosh script, or the data panel as JSON). */
export async function exportPlayground(
  body: { script: string; config?: string },
  format: 'mongosh' | 'json',
): Promise<{ filename: string; mimeType: string; content: string }> {
  return post('/api/v1/export', { script: body.script, config: body.config, format });
}
