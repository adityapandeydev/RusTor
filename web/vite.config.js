import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fetchReleases } from './release-feed.mjs';

export default defineConfig({
  root: import.meta.dirname,
  base: './',
  plugins: [react(), tailwindcss(), {
    name: 'public-release-feed',
    configureServer(server) {
      server.middlewares.use('/releases.json', async (_request, response) => {
        try {
          const releases = await fetchReleases(fetch, process.env.GITHUB_TOKEN);
          response.setHeader('Content-Type', 'application/json');
          response.end(JSON.stringify({ releases }));
        } catch {
          response.statusCode = 503;
          response.end('Release data unavailable');
        }
      });
    },
  }],
  test: { environment: 'jsdom', include: ['tests/**/*.test.jsx'], restoreMocks: true },
});
