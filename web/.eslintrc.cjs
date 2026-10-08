/** C6: ESLint for the dashboard. Generated OpenAPI types are not hand-edited. */
module.exports = {
  root: true,
  env: { browser: true, es2020: true, node: true },
  extends: [
    'eslint:recommended',
    'plugin:@typescript-eslint/recommended',
    'plugin:react-hooks/recommended',
  ],
  ignorePatterns: [
    'dist',
    'src/lib/api.gen.ts',
    // Handwritten 2.6k-line client + pages: C6 covers numeric `src/lib` (C2: no api.ts big-bang).
    'src/lib/api.ts',
    'src/pages/**',
    'src/components/**',
    'src/hooks/**',
    'src/lib/i18n.tsx',
    'src/lib/strategyFormShared.tsx',
  ],
  parser: '@typescript-eslint/parser',
  parserOptions: { ecmaVersion: 'latest', sourceType: 'module' },
  plugins: ['react-refresh'],
  rules: {
    'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
    '@typescript-eslint/no-unused-vars': [
      'error',
      { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
    ],
  },
}
