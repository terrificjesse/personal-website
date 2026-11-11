# Internships site

The internships listing and application tracker, split out of the personal site's Internships tab
on 2026-11-11. It shares the backend in `apps/fridge-app/backend/` and the account with the
personal site. See `docs/HUNT.md` § "The internships site, split out".

```bash
npm ci
npm run dev
```

Locally it runs on port 3001 beside the personal site on 3000, and both call the backend on 8080.
