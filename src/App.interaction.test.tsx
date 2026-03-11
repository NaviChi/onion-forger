import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

describe("App interactions", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/?fixture=vfs");
  });

  afterEach(() => {
    vi.restoreAllMocks();
    window.history.replaceState({}, "", "/");
  });

  it("toggles crawl option checkboxes without destabilizing the renderer", async () => {
    render(<App />);

    const listing = await screen.findByTestId("chk-listing");
    const sizes = screen.getByTestId("chk-sizes");
    const autoDownload = screen.getByTestId("chk-auto-download");
    const agnosticState = screen.getByTestId("chk-agnostic-state");
    const stealthRamp = screen.getByTestId("chk-stealth-ramp");

    expect(listing).toBeChecked();
    expect(sizes).toBeChecked();
    expect(autoDownload).not.toBeChecked();
    expect(agnosticState).not.toBeChecked();
    expect(stealthRamp).toBeChecked();

    fireEvent.click(listing);
    fireEvent.click(sizes);
    fireEvent.click(autoDownload);
    fireEvent.click(agnosticState);
    fireEvent.click(stealthRamp);

    expect(listing).not.toBeChecked();
    expect(sizes).not.toBeChecked();
    expect(autoDownload).toBeChecked();
    expect(agnosticState).toBeChecked();
    expect(stealthRamp).not.toBeChecked();
  });

  it("keeps the app interactive after Start Queue is clicked in browser fixture mode", async () => {
    render(<App />);

    const urlInput = await screen.findByTestId("input-target-url");
    fireEvent.change(urlInput, { target: { value: "http://fixture-target.onion/root" } });

    const startQueue = screen.getByTestId("btn-start-queue");
    fireEvent.click(startQueue);

    await waitFor(() => {
      expect(screen.getByText("Task Failed")).toBeInTheDocument();
    });

    expect(screen.getByText("Execution Environment Mismatch: Not running in native Tauri container.")).toBeInTheDocument();
    expect(screen.getByTestId("chk-listing")).toBeInTheDocument();
    expect(screen.getByTestId("btn-start-queue")).toBeEnabled();
  });
});
