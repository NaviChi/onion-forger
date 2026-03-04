export type FixtureEntryType = "File" | "Folder";

export interface FixtureFileEntry {
  path: string;
  size_bytes: number | null;
  entry_type: FixtureEntryType;
  raw_url: string;
}

export const VFS_FIXTURE_ENTRIES: FixtureFileEntry[] = [
  {
    path: "evidence",
    size_bytes: null,
    entry_type: "Folder",
    raw_url: "http://fixture.onion/evidence",
  },
  {
    path: "evidence/report.pdf",
    size_bytes: 4_194_304,
    entry_type: "File",
    raw_url: "http://fixture.onion/evidence/report.pdf",
  },
  {
    path: "evidence/screenshots",
    size_bytes: null,
    entry_type: "Folder",
    raw_url: "http://fixture.onion/evidence/screenshots",
  },
  {
    path: "evidence/screenshots/screen01.png",
    size_bytes: 786_432,
    entry_type: "File",
    raw_url: "http://fixture.onion/evidence/screenshots/screen01.png",
  },
  {
    path: "intel",
    size_bytes: null,
    entry_type: "Folder",
    raw_url: "http://fixture.onion/intel",
  },
  {
    path: "intel/leak_bundle.zip",
    size_bytes: 12_582_912,
    entry_type: "File",
    raw_url: "http://fixture.onion/intel/leak_bundle.zip",
  },
  {
    path: "README.txt",
    size_bytes: 2_048,
    entry_type: "File",
    raw_url: "http://fixture.onion/README.txt",
  },
];

export const VFS_FIXTURE_STATS = VFS_FIXTURE_ENTRIES.reduce(
  (acc, entry) => {
    if (entry.entry_type === "Folder") acc.folders += 1;
    else acc.files += 1;
    acc.totalNodes += 1;
    if (entry.size_bytes) acc.size += entry.size_bytes;
    return acc;
  },
  { files: 0, folders: 0, size: 0, totalNodes: 0 }
);

export function isVfsFixtureMode(): boolean {
  if (typeof window === "undefined") return false;
  const params = new URLSearchParams(window.location.search);
  return params.get("fixture") === "vfs";
}

export function normalizeVfsPath(input: string): string {
  return input.replace(/^\/+|\/+$/g, "");
}

export function fixtureParentPath(input: string): string {
  const normalized = normalizeVfsPath(input);
  const idx = normalized.lastIndexOf("/");
  return idx === -1 ? "" : normalized.slice(0, idx);
}
