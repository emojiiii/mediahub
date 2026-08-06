import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'vite'
import { viteStaticCopy } from 'vite-plugin-static-copy'

const viewerChunkGroups = [
  {
    name: 'docx-preview',
    test: /node_modules[\\/]docx-preview[\\/]/,
    priority: 10,
    // Keep docx-preview's bundled helpers in the same lazy chunk. Leaving
    // dependencies outside the group creates a circular import with the
    // ObjectFileViewer chunk (the docx chunk calls an uninitialized export).
    includeDependenciesRecursively: true,
  },
]

const pdfJsAssetTargets = [
  {
    src: 'node_modules/pdfjs-dist/cmaps/*',
    dest: 'pdfjs/cmaps',
    rename: { stripBase: true as const },
  },
  {
    src: 'node_modules/pdfjs-dist/standard_fonts/*',
    dest: 'pdfjs/standard_fonts',
    rename: { stripBase: true as const },
  },
]

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    viteStaticCopy({ targets: pdfJsAssetTargets }),
  ],
  build: {
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: viewerChunkGroups,
        },
      },
    },
  },
  worker: {
    format: 'es',
  },
})
