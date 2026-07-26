import { describe, expect, it } from "vitest";
import {
  formatOpenProjectShortcut,
  formatRecentProjectEntry,
  normalizeProjectPath,
} from "./recentProjectDisplay";

describe("normalizeProjectPath", () => {
  it("strips Windows extended drive prefix", () => {
    expect(normalizeProjectPath("\\\\?\\D:\\Dev\\CloudFlareWorker\\feedback")).toBe(
      "D:\\Dev\\CloudFlareWorker\\feedback",
    );
  });

  it("strips Windows extended UNC prefix", () => {
    expect(normalizeProjectPath("\\\\?\\UNC\\server\\share\\project")).toBe(
      "\\\\server\\share\\project",
    );
  });

  it("returns POSIX paths unchanged", () => {
    expect(normalizeProjectPath("/home/user/project")).toBe("/home/user/project");
  });
});

describe("formatRecentProjectEntry", () => {
  it("formats extended Windows paths", () => {
    expect(
      formatRecentProjectEntry("\\\\?\\D:\\Dev\\CloudFlareWorker\\feedback"),
    ).toEqual({
      name: "feedback",
      parent: "D:\\Dev\\CloudFlareWorker",
    });
  });

  it("formats regular Windows paths", () => {
    expect(formatRecentProjectEntry("D:\\Dev\\Python\\engine")).toEqual({
      name: "engine",
      parent: "D:\\Dev\\Python",
    });
  });

  it("formats POSIX paths", () => {
    expect(formatRecentProjectEntry("/home/user/projects/teshi")).toEqual({
      name: "teshi",
      parent: "/home/user/projects",
    });
  });

  it("handles drive root folders", () => {
    expect(formatRecentProjectEntry("C:\\Project")).toEqual({
      name: "Project",
      parent: "C:\\",
    });
  });

  it("handles single-segment paths", () => {
    expect(formatRecentProjectEntry("project")).toEqual({
      name: "project",
      parent: "",
    });
  });
});

describe("formatOpenProjectShortcut", () => {
  it("returns a non-empty shortcut label", () => {
    expect(["Ctrl+O", "⌘O"]).toContain(formatOpenProjectShortcut());
  });
});
