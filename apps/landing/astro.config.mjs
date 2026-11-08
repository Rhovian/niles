import { defineConfig } from 'astro/config';

// The production URL Vercel assigns the project; the page's canonical link and share image derive from it.
export default defineConfig({ site: 'https://niles-xi.vercel.app', build: { inlineStylesheets: 'always' } });
