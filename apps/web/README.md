# MindGraph Web

The MindGraph Open frontend: Next.js, TypeScript, Tailwind CSS and shadcn/ui
(ADR-012). Sigma.js and Graphology are added with the Graph Explorer in M6.

Currently a placeholder landing page. See the root [README](../../README.md)
for the full development setup.

```bash
npm ci
npm run dev           # http://localhost:3000
npm run typecheck
npm run lint
npm run format:check
npm test
```

Next.js telemetry is disabled in Docker and CI (`NEXT_TELEMETRY_DISABLED=1`).
To disable it locally as well, run `npx next telemetry disable`.
