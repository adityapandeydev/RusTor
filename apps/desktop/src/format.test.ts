import { describe, expect, it } from "vitest";
import { bytes, displayPath } from "./format";

describe("transfer formatting", () => {
  it("formats zero, invalid numbers, and binary units", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(Number.NaN)).toBe("0 B");
    expect(bytes(1024)).toBe("1 KiB");
  });
  it("hides Windows canonical-path prefixes without changing normal paths", () => {
    expect(displayPath(String.raw`\\?\C:\Downloads`)).toBe(String.raw`C:\Downloads`);
    expect(displayPath(String.raw`\\?\UNC\server\share`)).toBe(String.raw`\\server\share`);
    expect(displayPath("/home/user/downloads")).toBe("/home/user/downloads");
  });
});
