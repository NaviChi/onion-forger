// =============================================================================
// AzureConnectivityModal.tsx — Phase 53B
// Optional enterprise Azure + Intranet integration modal.
// Displays two tabs: "Intranet Web Access" and "Azure Storage".
// =============================================================================

import { useState, useEffect } from "react";
import { Cloud, Globe, Server, X } from "lucide-react";
import "./AzureConnectivityModal.css";

interface AzureConnectivityModalProps {
    isOpen: boolean;
    onClose: () => void;
}

interface AzureStatus {
    storageEnabled: boolean;
    storageAccount: string | null;
    containerName: string | null;
    region: string | null;
    intranetServerRunning: boolean;
    intranetServerPort: number | null;
}

export function AzureConnectivityModal({ isOpen, onClose }: AzureConnectivityModalProps) {
    const [activeTab, setActiveTab] = useState<"intranet" | "storage">("intranet");
    const [status, setStatus] = useState<AzureStatus | null>(null);
    const [loading, setLoading] = useState(false);
    const [message, setMessage] = useState("");

    // Intranet form state
    const [intranetPort, setIntranetPort] = useState("8080");

    // Azure Storage form state
    const [subscriptionId, setSubscriptionId] = useState("");
    const [tenantId, setTenantId] = useState("");
    const [clientId, setClientId] = useState("");
    const [clientSecret, setClientSecret] = useState("");
    const [resourceGroup, setResourceGroup] = useState("");
    const [storageAccount, setStorageAccount] = useState("");
    const [containerName, setContainerName] = useState("");
    const [region, setRegion] = useState("eastus");
    const [sizeGb, setSizeGb] = useState("500");
    const [useManagedIdentity, setUseManagedIdentity] = useState(false);

    // Fetch Azure status on open
    useEffect(() => {
        if (isOpen) {
            fetchStatus();
        }
    }, [isOpen]);

    const isTauri = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";

    async function fetchStatus() {
        if (!isTauri) return;
        try {
            const { invoke } = await import("@tauri-apps/api/core");
            const result = await invoke<AzureStatus>("get_azure_status");
            setStatus(result);
        } catch {
            // Azure feature not compiled in — that's expected
            setStatus(null);
        }
    }

    async function handleToggleIntranet(enable: boolean) {
        if (!isTauri) { setMessage("Azure features require native Tauri runtime"); return; }
        setLoading(true);
        setMessage("");
        try {
            const { invoke } = await import("@tauri-apps/api/core");
            const result = await invoke<string>("toggle_intranet_server", {
                enable,
                port: parseInt(intranetPort) || 8080,
            });
            setMessage(result);
            await fetchStatus();
        } catch (err: any) {
            setMessage(`Error: ${err.message || err}`);
        }
        setLoading(false);
    }

    async function handleConfigureAzure() {
        if (!isTauri) { setMessage("Azure features require native Tauri runtime"); return; }
        setLoading(true);
        setMessage("");
        try {
            const { invoke } = await import("@tauri-apps/api/core");
            await invoke<string>("configure_azure_storage", {
                config: {
                    subscriptionId,
                    tenantId,
                    clientId,
                    clientSecretEncrypted: clientSecret, // Backend will encrypt
                    resourceGroup,
                    storageAccount,
                    containerName: containerName || "crawli-downloads",
                    region,
                    sizeGb: parseInt(sizeGb) || 500,
                    useManagedIdentity,
                },
            });
            setMessage("Azure Storage configured ✓");
            await fetchStatus();
        } catch (err: any) {
            setMessage(`Error: ${err.message || err}`);
        }
        setLoading(false);
    }

    async function handleTestConnection() {
        if (!isTauri) return;
        setLoading(true);
        setMessage("");
        try {
            const { invoke } = await import("@tauri-apps/api/core");
            const result = await invoke<string>("test_azure_connection");
            setMessage(result);
        } catch (err: any) {
            setMessage(`Error: ${err.message || err}`);
        }
        setLoading(false);
    }

    async function handleToggleStorage(enable: boolean) {
        if (!isTauri) return;
        setLoading(true);
        setMessage("");
        try {
            const { invoke } = await import("@tauri-apps/api/core");
            const cmd = enable ? "enable_azure_storage" : "disable_azure_storage";
            const result = await invoke<string>(cmd);
            setMessage(result);
            await fetchStatus();
        } catch (err: any) {
            setMessage(`Error: ${err.message || err}`);
        }
        setLoading(false);
    }

    if (!isOpen) return null;

    return (
        <div className="azure-modal-overlay" onClick={onClose}>
            <div className="azure-modal" onClick={(e) => e.stopPropagation()}>
                {/* Header */}
                <div className="azure-modal-header">
                    <h2>
                        <Cloud size={20} style={{ color: "var(--accent-secondary)" }} />
                        Azure Connectivity
                    </h2>
                    <button className="azure-modal-close" data-testid="btn-azure-close" onClick={onClose}>
                        <X size={18} />
                    </button>
                </div>

                {/* Tabs */}
                <div className="azure-tabs">
                    <button
                        className={`azure-tab ${activeTab === "intranet" ? "active" : ""}`}
                        data-testid="btn-azure-tab-intranet"
                        onClick={() => setActiveTab("intranet")}
                    >
                        <Globe size={14} style={{ marginRight: 6, verticalAlign: "middle" }} />
                        Intranet Web Access
                    </button>
                    <button
                        className={`azure-tab ${activeTab === "storage" ? "active" : ""}`}
                        data-testid="btn-azure-tab-storage"
                        onClick={() => setActiveTab("storage")}
                    >
                        <Server size={14} style={{ marginRight: 6, verticalAlign: "middle" }} />
                        Azure Storage
                    </button>
                </div>

                {/* Body */}
                <div className="azure-modal-body">
                    {activeTab === "intranet" && (
                        <>
                            <div className={`azure-status ${status?.intranetServerRunning ? "connected" : "disconnected"}`}>
                                <div className="azure-status-dot" />
                                {status?.intranetServerRunning
                                    ? `Running on port ${status.intranetServerPort}`
                                    : "Server not running"}
                            </div>

                            <div className="azure-form-group">
                                <label>Server Port</label>
                                <input
                                    data-testid="input-azure-intranet-port"
                                    type="number"
                                    value={intranetPort}
                                    onChange={(e) => setIntranetPort(e.target.value)}
                                    placeholder="8080"
                                    min={1024}
                                    max={65535}
                                />
                            </div>

                            <p style={{ fontSize: "0.78rem", color: "var(--text-muted)", lineHeight: 1.5 }}>
                                When enabled, the Crawli dashboard will be accessible from any machine on
                                your internal network via <code>http://&lt;your-ip&gt;:{intranetPort}</code>.
                                The web UI mirrors the desktop app 1:1.
                            </p>
                        </>
                    )}

                    {activeTab === "storage" && (
                        <>
                            <div className={`azure-status ${status?.storageEnabled ? "connected" : "disconnected"}`}>
                                <div className="azure-status-dot" />
                                {status?.storageEnabled
                                    ? `Connected: ${status.storageAccount} / ${status.containerName} (${status.region})`
                                    : "Azure Storage not enabled"}
                            </div>

                            <div className="azure-toggle-row">
                                <span className="azure-toggle-label">Use Managed Identity</span>
                                <input
                                    data-testid="chk-azure-managed-identity"
                                    type="checkbox"
                                    checked={useManagedIdentity}
                                    onChange={(e) => setUseManagedIdentity(e.target.checked)}
                                />
                            </div>

                            <div className="azure-form-group">
                                <label>Subscription ID</label>
                                <input data-testid="input-azure-subscription-id" value={subscriptionId} onChange={(e) => setSubscriptionId(e.target.value)} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" />
                            </div>
                            <div className="azure-form-group">
                                <label>Tenant ID</label>
                                <input data-testid="input-azure-tenant-id" value={tenantId} onChange={(e) => setTenantId(e.target.value)} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" />
                            </div>

                            {!useManagedIdentity && (
                                <>
                                    <div className="azure-form-group">
                                        <label>Client ID</label>
                                        <input data-testid="input-azure-client-id" value={clientId} onChange={(e) => setClientId(e.target.value)} placeholder="App (client) ID" />
                                    </div>
                                    <div className="azure-form-group">
                                        <label>Client Secret</label>
                                        <input data-testid="input-azure-client-secret" type="password" value={clientSecret} onChange={(e) => setClientSecret(e.target.value)} placeholder="Client secret value" />
                                    </div>
                                </>
                            )}

                            <div className="azure-form-group">
                                <label>Resource Group</label>
                                <input data-testid="input-azure-resource-group" value={resourceGroup} onChange={(e) => setResourceGroup(e.target.value)} placeholder="my-resource-group" />
                            </div>
                            <div className="azure-form-group">
                                <label>Storage Account Name</label>
                                <input data-testid="input-azure-storage-account" value={storageAccount} onChange={(e) => setStorageAccount(e.target.value)} placeholder="crawlistorage" />
                            </div>
                            <div className="azure-form-group">
                                <label>Container Name</label>
                                <input data-testid="input-azure-container-name" value={containerName} onChange={(e) => setContainerName(e.target.value)} placeholder="crawli-downloads" />
                            </div>
                            <div className="azure-form-group">
                                <label>Region</label>
                                <select data-testid="sel-azure-region" value={region} onChange={(e) => setRegion(e.target.value)}>
                                    <option value="eastus">East US</option>
                                    <option value="westus2">West US 2</option>
                                    <option value="westeurope">West Europe</option>
                                    <option value="northeurope">North Europe</option>
                                    <option value="southeastasia">Southeast Asia</option>
                                    <option value="japaneast">Japan East</option>
                                    <option value="australiaeast">Australia East</option>
                                </select>
                            </div>
                            <div className="azure-form-group">
                                <label>Storage Size (GB)</label>
                                <input data-testid="input-azure-size-gb" type="number" value={sizeGb} onChange={(e) => setSizeGb(e.target.value)} min={1} max={10000} />
                            </div>
                        </>
                    )}

                    {/* Status message */}
                    {message && (
                        <div style={{
                            marginTop: 12,
                            padding: "8px 12px",
                            borderRadius: 6,
                            fontSize: "0.82rem",
                            background: message.startsWith("Error") ? "rgba(255,70,70,0.1)" : "rgba(0,229,100,0.08)",
                            color: message.startsWith("Error") ? "#ff6b6b" : "#00e564",
                            border: `1px solid ${message.startsWith("Error") ? "rgba(255,70,70,0.2)" : "rgba(0,229,100,0.2)"}`,
                        }}>
                            {message}
                        </div>
                    )}
                </div>

                {/* Actions */}
                <div className="azure-modal-actions">
                    {activeTab === "intranet" && (
                        <>
                            {status?.intranetServerRunning ? (
                                <button className="azure-btn azure-btn-danger" data-testid="btn-azure-intranet-stop" onClick={() => handleToggleIntranet(false)} disabled={loading}>
                                    Stop Server
                                </button>
                            ) : (
                                <button className="azure-btn azure-btn-primary" data-testid="btn-azure-intranet-start" onClick={() => handleToggleIntranet(true)} disabled={loading}>
                                    {loading ? "Starting..." : "Start Intranet Server"}
                                </button>
                            )}
                        </>
                    )}
                    {activeTab === "storage" && (
                        <>
                            <button className="azure-btn azure-btn-secondary" data-testid="btn-azure-test-connection" onClick={handleTestConnection} disabled={loading}>
                                Test Connection
                            </button>
                            <button className="azure-btn azure-btn-primary" data-testid="btn-azure-configure" onClick={handleConfigureAzure} disabled={loading}>
                                {loading ? "Saving..." : "Save & Configure"}
                            </button>
                            {status?.storageEnabled ? (
                                <button className="azure-btn azure-btn-danger" data-testid="btn-azure-storage-disable" onClick={() => handleToggleStorage(false)} disabled={loading}>
                                    Disable
                                </button>
                            ) : (
                                <button className="azure-btn azure-btn-secondary" data-testid="btn-azure-storage-enable" onClick={() => handleToggleStorage(true)} disabled={loading}>
                                    Enable Storage
                                </button>
                            )}
                        </>
                    )}
                </div>
            </div>
        </div>
    );
}
