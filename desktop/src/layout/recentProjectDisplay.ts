export interface RecentProjectEntry {
  name: string;
  parent: string;
}

/** Strip Windows extended-length path prefixes for display. */
export function normalizeProjectPath(path: string): string {
  if (path.startsWith("\\\\?\\UNC\\")) {
    return `\\\\${path.slice("\\\\?\\UNC\\".length)}`;
  }
  if (path.startsWith("\\\\?\\")) {
    return path.slice("\\\\?\\".length);
  }
  return path;
}

/** Split a project path into a folder name and parent directory for welcome UI. */
export function formatRecentProjectEntry(path: string): RecentProjectEntry {
  const normalized = normalizeProjectPath(path).replace(/[/\\]+$/, "");
  const isAbsolutePosix = normalized.startsWith("/");
  const isWindowsPath = /^[a-zA-Z]:/.test(normalized);
  const segments = normalized.split(/[/\\]/).filter(Boolean);

  if (segments.length === 0) {
    return { name: path, parent: "" };
  }

  const name = segments[segments.length - 1] ?? path;
  const parentSegments = segments.slice(0, -1);

  if (parentSegments.length === 0) {
    return { name, parent: "" };
  }

  const separator = normalized.includes("\\") ? "\\" : "/";
  const isWindowsDriveRoot =
    isWindowsPath &&
    /^[a-zA-Z]:$/.test(parentSegments[0] ?? "") &&
    parentSegments.length === 1;

  let parent = isWindowsDriveRoot
    ? `${parentSegments[0]}${separator}`
    : parentSegments.join(separator);

  if (isAbsolutePosix && !isWindowsPath) {
    parent = `/${parent}`;
  }

  return { name, parent };
}

/** Keyboard hint for the Open Project action on the welcome screen. */
export function formatOpenProjectShortcut(): string {
  if (typeof navigator !== "undefined") {
    const platform = navigator.platform ?? "";
    const userAgent = navigator.userAgent ?? "";
    if (/mac/i.test(platform) || /mac/i.test(userAgent)) {
      return "⌘O";
    }
  }
  return "Ctrl+O";
}
