import { describe, expect, it } from "vitest";
import {
  buildHeightIndex,
  isAtEnd,
  offsetOf,
  prependShift,
  restoreScroll,
  rowAtOffset,
  windowFor,
} from "./virtualWindow";

/** Uniform heights, the simplest case. */
function uniform(count: number, height: number) {
  return buildHeightIndex(count, () => height);
}

describe("buildHeightIndex", () => {
  it("builds prefix sums", () => {
    const index = buildHeightIndex(3, (i) => 10 + i * 5);
    expect(index.count).toBe(3);
    expect(index.total).toBe(45);
    expect(index.offsets).toEqual([0, 10, 25, 45]);
  });

  it("rejects non-positive heights loudly", () => {
    expect(() => buildHeightIndex(2, () => 0)).toThrow(/non-positive/);
  });
});

describe("rowAtOffset", () => {
  it("maps offsets back to rows including boundaries", () => {
    const index = buildHeightIndex(4, () => 10);
    expect(rowAtOffset(index, 0)).toBe(0);
    expect(rowAtOffset(index, 9)).toBe(0);
    expect(rowAtOffset(index, 10)).toBe(1);
    expect(rowAtOffset(index, 39)).toBe(3);
    expect(rowAtOffset(index, 10_000)).toBe(3);
    expect(rowAtOffset(index, -5)).toBe(0);
  });
});

describe("windowFor", () => {
  it("renders only the viewport plus overscan in a 100k-row list", () => {
    const count = 100_000;
    const index = uniform(count, 24);
    // scrollTop lands exactly on row 50_000; 600 px shows rows 50_000..50_024.
    const window = windowFor(index, 24 * 50_000, 600, 4);
    expect(window.startIndex).toBe(50_000 - 4);
    expect(window.endIndex).toBe(50_000 + 24 + 4);
    expect(window.padTop).toBe(24 * (50_000 - 4));
    expect(window.padBottom).toBe(index.total - 24 * (50_000 + 29));
    // The rendered slice stays tiny no matter where the user is.
    expect(window.endIndex - window.startIndex + 1).toBeLessThanOrEqual(
      Math.ceil(600 / 24) + 2 * 4,
    );
  });

  it("clamps the window at both ends of the list", () => {
    const index = uniform(10, 40);
    expect(windowFor(index, 0, 200, 3)).toMatchObject({ startIndex: 0, endIndex: 7 });
    expect(windowFor(index, 200, 200, 3)).toMatchObject({
      startIndex: 2,
      endIndex: 9,
      padBottom: 0,
    });
  });

  it("handles the empty list", () => {
    const index = buildHeightIndex(0, () => 1);
    expect(windowFor(index, 0, 400, 3)).toEqual({
      startIndex: 0,
      endIndex: -1,
      padTop: 0,
      padBottom: 0,
    });
  });

  it("rejects a non-positive viewport", () => {
    expect(() => windowFor(uniform(5, 10), 0, 0, 1)).toThrow(/positive/);
  });
});

describe("prepend stability", () => {
  it("shifts scrollTop by exactly the prepended block height", () => {
    const before = uniform(100, 20);
    const firstVisible = rowAtOffset(before, 400);
    expect(firstVisible).toBe(20);

    const prepended = [20, 20, 20];
    const after = buildHeightIndex(103, (i) =>
      i < prepended.length ? prepended[i]! : 20,
    );
    const shifted = 400 + prependShift(prepended);
    // The same row is the first visible one after the prepend.
    expect(rowAtOffset(after, shifted)).toBe(firstVisible + prepended.length);
    expect(offsetOf(after, firstVisible + prepended.length)).toBe(shifted);
  });

  it("restores a scroll anchor by row index", () => {
    const index = buildHeightIndex(5, (i) => 10 + i * 5);
    expect(restoreScroll(index, 2)).toBe(25);
    expect(rowAtOffset(index, restoreScroll(index, 2))).toBe(2);
  });
});

describe("isAtEnd", () => {
  it("follows the tail only within the threshold", () => {
    const index = uniform(100, 20);
    expect(isAtEnd(index, index.total - 600, 600)).toBe(true);
    expect(isAtEnd(index, index.total - 624, 600)).toBe(true);
    expect(isAtEnd(index, index.total - 700, 600)).toBe(false);
  });
});
