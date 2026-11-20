/** Per-page title, description and canonical URL (the page is a single-page app, so we update the head ourselves). */
const SITE = 'https://mongoplayground.com';

function meta(selector: string, attr: string, value: string) {
  document.head.querySelector(selector)?.setAttribute(attr, value);
}

export function setPage(title: string, description: string, path: string) {
  document.title = title;
  meta('meta[name="description"]', 'content', description);
  meta('meta[property="og:title"]', 'content', title);
  meta('meta[property="og:description"]', 'content', description);
  meta('meta[name="twitter:title"]', 'content', title);
  meta('meta[name="twitter:description"]', 'content', description);
  const url = SITE + (path === '/' ? '/' : path);
  meta('link[rel="canonical"]', 'href', url);
  meta('meta[property="og:url"]', 'content', url);
}

export const PAGES: Record<string, { title: string; description: string }> = {
  '/': {
    title: 'Online MongoDB Shell – Test Your Queries in the Browser | MongoDB Playground',
    description:
      'Free online MongoDB shell and query playground. Paste documents, run mongosh-style find, aggregate, update and delete commands, see query plans, try Atlas Search and Vector Search, then share or export. No install, no sign-up.',
  },
  '/docs/quickstart': {
    title: 'MongoDB Playground Quickstart – Test a MongoDB Query in 60 Seconds',
    description:
      'Learn the online MongoDB shell in minutes: load sample documents, run find, aggregate and update commands, read query plans, add indexes, try Atlas Search and Vector Search, share and export.',
  },
  '/docs/api': {
    title: 'MongoDB Playground REST API – Run mongosh Scripts Over HTTP',
    description:
      'Run MongoDB shell scripts, explain queries, share and export playgrounds through a REST API with an OpenAPI 3.1 spec. Built for scripts, CI and AI agents.',
  },
  '/docs/mcp': {
    title: 'MongoDB Playground MCP Server – Test MongoDB Queries from AI Agents',
    description:
      'Connect Claude, Cursor or any MCP client to a MongoDB query playground. Tools to run, validate, explain, share and export mongosh scripts. No sign-up.',
  },
  '/docs/run-it-yourself': {
    title: 'Run MongoDB Playground Locally or on a Free Atlas Cluster',
    description:
      'Run the MongoDB playground on your machine with the Atlas CLI, Docker or the desktop app, or point it at a free-forever or ephemeral MongoDB Atlas cluster.',
  },
  '/about': {
    title: 'About MongoDB Playground – Runs on a MongoDB Atlas M20 Cluster',
    description:
      'How the online MongoDB shell works: throwaway databases on a MongoDB Atlas M20 cluster, limits, privacy, and the stack behind it.',
  },
};
