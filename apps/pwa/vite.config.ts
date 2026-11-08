import { defineConfig } from 'vite';
import preact from '@preact/preset-vite';
import { VitePWA } from 'vite-plugin-pwa';
import { mockApi } from './mock';

export default defineConfig(({ mode }) => ({
  plugins: [preact(), ...(mode === 'mock' ? [mockApi()] : []), VitePWA({
    strategies: 'injectManifest', srcDir: 'src', filename: 'sw.ts',
    injectRegister: false,
    injectManifest: { globPatterns: ['**/*.{js,css,html,png,svg,webmanifest}'] },
    manifest: {
      name: 'Niles', short_name: 'Niles', description: 'Chat with your session agent',
      start_url: '/', scope: '/', display: 'minimal-ui',
      theme_color: '#12201e', background_color: '#12201e',
      icons: [192, 512].map(size => ({ src: `/icon-${size}.png`, sizes: `${size}x${size}`, type: 'image/png' })),
    },
  })],
  server: { host: '127.0.0.1' },
}));
