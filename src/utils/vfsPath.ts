export function normalizeTreePath(input: string): string {
  return input
    .replace(/\\/g, "/")
    .split("/")
    .map((segment) => segment.trim())
    .filter((segment) => segment.length > 0 && segment !== ".")
    .join("/");
}

export function treeParentPath(input: string): string {
  const normalized = normalizeTreePath(input);
  const idx = normalized.lastIndexOf("/");
  return idx === -1 ? "" : normalized.slice(0, idx);
}

export function isDirectTreeChild(parentPath: string, childPath: string): boolean {
  return treeParentPath(childPath) === normalizeTreePath(parentPath);
}
