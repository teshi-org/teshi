# teshi-web-ui

React SPA served by `teshi web` / `teshi-daemon` for project workspace, locator review, and replay screenshot browsing.

## Develop

```bash
npm --prefix apps/teshi-web-ui install
npm --prefix apps/teshi-web-ui run build
cargo run -p teshi-cli -- web --dist apps/teshi-web-ui/dist
```

For Vite HMR against a running daemon, build once then point `--dist` at `apps/teshi-web-ui/dist`, or use `npm run dev` with a reverse proxy if configured locally.

## Notes

- This package is **web-only** (HTTP to the daemon). There is no Tauri shell.
- Python sidecar scripts live in repo `resources/` (`browser_service.py`, `winapp_service.py`).
