import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

afterEach(() => {
  cleanup();
});

// Suppress noisy `act` warnings from library internals.
const originalConsoleError = console.error;
console.error = (...args: any[]) => {
  if (typeof args[0] === "string" && args[0].includes("not wrapped in act")) {
    return;
  }
  originalConsoleError(...args);
};

// Stub global fetch by default to avoid accidental network calls.
vi.stubGlobal("fetch", vi.fn());
