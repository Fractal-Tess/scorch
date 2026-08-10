# Scorch documentation site

Astro 5 and Tailwind CSS 4 power the documentation site in this directory. The existing design and engineering notes at the directory root remain source documents; the website lives under `src/`.

## Development

```sh
bun install
bun run dev
```

Validate a production build with:

```sh
bun run format:check
bun run check
bun run build
```

The static output is written to `dist/`.

## Docker

The production image follows Astro's static NGINX deployment pattern, using Bun for the build stage and NGINX on port 8080 for serving.

From the repository root:

```sh
docker build -t scorch-docs ./docs
docker run --rm -p 8080:8080 scorch-docs
```

Open <http://127.0.0.1:8080>. The container health check uses <http://127.0.0.1:8080/healthz>.
