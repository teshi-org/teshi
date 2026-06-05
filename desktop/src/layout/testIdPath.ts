/** Sanitize a path segment for stable `data-testid` suffixes (Welcome recent, file tree). */
export function sanitizePathForTestId(path: string): string {
  return path.replace(/[^a-zA-Z0-9._-]+/g, "_");
}

/** `data-testid` for a welcome-screen recent-project button. */
export function welcomeRecentTestId(projectPath: string): string {
  return `WelcomeRecent-${sanitizePathForTestId(projectPath)}`;
}
