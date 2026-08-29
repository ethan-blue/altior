import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Vitest runs with explicit imports (`globals: false`), so Testing Library
// cannot detect the framework and register its own cleanup.
afterEach(cleanup);
