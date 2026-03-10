type CoreModule = typeof import("@tauri-apps/api/core");
type EventModule = typeof import("@tauri-apps/api/event");
type PathModule = typeof import("@tauri-apps/api/path");
type DialogModule = typeof import("@tauri-apps/plugin-dialog");

let corePromise: Promise<CoreModule> | null = null;
let eventPromise: Promise<EventModule> | null = null;
let pathPromise: Promise<PathModule> | null = null;
let dialogPromise: Promise<DialogModule> | null = null;

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
}

async function loadCore(): Promise<CoreModule> {
  corePromise ??= import("@tauri-apps/api/core");
  return corePromise;
}

async function loadEvent(): Promise<EventModule> {
  eventPromise ??= import("@tauri-apps/api/event");
  return eventPromise;
}

async function loadPath(): Promise<PathModule> {
  pathPromise ??= import("@tauri-apps/api/path");
  return pathPromise;
}

async function loadDialog(): Promise<DialogModule> {
  dialogPromise ??= import("@tauri-apps/plugin-dialog");
  return dialogPromise;
}

export async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await loadCore();
  return invoke<T>(cmd, args);
}

export async function listenEvent<T>(
  event: string,
  handler: (event: { event: string; id: number; payload: T }) => void,
): Promise<() => void> {
  const { listen } = await loadEvent();
  return listen<T>(event, handler);
}

export async function getDownloadDir(): Promise<string> {
  const { downloadDir } = await loadPath();
  return downloadDir();
}

export async function joinPath(...segments: string[]): Promise<string> {
  const { join } = await loadPath();
  return join(...segments);
}

export async function openDialog(options: Parameters<DialogModule["open"]>[0]) {
  const { open } = await loadDialog();
  return open(options);
}

export async function saveDialog(options: Parameters<DialogModule["save"]>[0]) {
  const { save } = await loadDialog();
  return save(options);
}
