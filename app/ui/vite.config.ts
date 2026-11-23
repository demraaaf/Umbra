import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

// Tauri expects a fixed dev port and a static build in ./dist (see README.md).
export default defineConfig({
	plugins: [tailwindcss(), svelte()],
	resolve: {
		alias: {
			$lib: fileURLToPath(new URL('./src/lib', import.meta.url))
		}
	},
	clearScreen: false,
	server: {
		port: 1420,
		strictPort: true,
		host: '127.0.0.1'
	},
	build: {
		target: 'es2022',
		outDir: 'dist',
		emptyOutDir: true,
		sourcemap: false,
		chunkSizeWarningLimit: 900
	}
});
