import { describe, expect, it } from "vitest";
import { classifyTargetInputMode, isOnionTarget } from "./App";

describe("App target mode helpers", () => {
  it("detects onion hosts by hostname only", () => {
    expect(isOnionTarget("http://example.onion/files/")).toBe(true);
    expect(isOnionTarget("https://cdn.breachforums.as/pay_or_leak/shouldve_paid_the_ransom_pathstone.com_shinyhunters.7z")).toBe(false);
    expect(isOnionTarget("https://example.com/path/onion/report.txt")).toBe(false);
  });

  it("classifies direct, onion, mega, and torrent inputs correctly", () => {
    expect(classifyTargetInputMode("https://proof.ovh.net/files/10Gb.dat")).toBe("direct");
    expect(classifyTargetInputMode("https://cdn.breachforums.as/pay_or_leak/shouldve_paid_the_ransom_pathstone.com_shinyhunters.7z")).toBe("direct");
    expect(classifyTargetInputMode("http://example.onion/files/")).toBe("onion");
    expect(classifyTargetInputMode("https://mega.nz/folder/ABC#KEY")).toBe("mega");
    expect(classifyTargetInputMode("magnet:?xt=urn:btih:abc")).toBe("torrent");
  });
});
