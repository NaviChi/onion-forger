export const UI_TEST_IDS = {
  toolbar: "toolbar",
  targetUrlInput: "input-target-url",
  startQueueButton: "btn-start-queue",
  loadTargetButton: "btn-load-target",
  directModeButton: "btn-direct",
  onionModeButton: "btn-onion",
  resourceMetricsCard: "resource-metrics-card",
  resourceProcessCpu: "resource-process-cpu",
} as const;

export const NATIVE_WEBVIEW_SMOKE_TEST_IDS = [
  UI_TEST_IDS.toolbar,
  UI_TEST_IDS.targetUrlInput,
  UI_TEST_IDS.startQueueButton,
  UI_TEST_IDS.loadTargetButton,
  UI_TEST_IDS.resourceMetricsCard,
] as const;
