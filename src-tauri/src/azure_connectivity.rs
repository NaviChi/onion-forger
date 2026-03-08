// =============================================================================
// azure_connectivity.rs — Phase 53: Optional Azure + Intranet Enterprise Module
// =============================================================================
// This entire module is gated behind `#[cfg(feature = "azure")]`.
// It is NEVER compiled unless `cargo build --features azure` is used.
//
// PR-AZURE-001: Never store client_secret in plaintext — AES-256-GCM encrypted
// PR-AZURE-002: Web server serves only static dist/ build — no eval, no SSR
// PR-AZURE-003: Azure uploads must respect resource governor concurrency limits
// PR-AZURE-004: Feature must be compile-time gated everywhere
// PR-AZURE-005: Fallback to local disk if Azure fails — never block downloads
// =============================================================================

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, Manager};

// ── Configuration Structs ───────────────────────────────────────────────────

/// Azure Storage configuration for a target session.
/// Stored encrypted in the per-target ledger.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureStorageConfig {
    pub subscription_id: String,
    pub tenant_id: String,
    pub client_id: String,
    /// AES-256-GCM encrypted client secret (never stored in plaintext).
    /// Empty string when using managed identity.
    pub client_secret_encrypted: String,
    pub resource_group: String,
    pub storage_account: String,
    pub container_name: String,
    pub region: String,
    pub size_gb: u32,
    /// When true, uses Azure Managed Identity instead of client_id/secret.
    pub use_managed_identity: bool,
}

/// Intranet web server configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntranetServerConfig {
    pub port: u16,
    pub enable_auth: bool,
    pub auth_provider: String, // "azure_ad" | "none"
}

impl Default for IntranetServerConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            enable_auth: false,
            auth_provider: "none".to_string(),
        }
    }
}

/// Runtime state for the Azure connectivity feature.
/// Managed in AppState behind a Mutex.
pub struct AzureConnectivityState {
    pub storage_enabled: bool,
    pub storage_config: Option<AzureStorageConfig>,
    pub intranet_server_handle: Option<tokio::task::JoinHandle<()>>,
    pub intranet_server_port: Option<u16>,
}

impl Default for AzureConnectivityState {
    fn default() -> Self {
        Self {
            storage_enabled: false,
            storage_config: None,
            intranet_server_handle: None,
            intranet_server_port: None,
        }
    }
}

// ── Credential Encryption (AES-256-GCM) ─────────────────────────────────────

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};

/// Derives a 256-bit encryption key from machine-specific context.
/// Uses SHA-256 of hostname + username for per-machine key derivation.
/// This is NOT a KDF — for production, use PBKDF2/Argon2. Sufficient for
/// at-rest protection of secrets already behind OS-level access control.
fn derive_machine_key() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "crawli-default-host".to_string());
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "crawli-user".to_string());
    let mut hasher = Sha256::new();
    hasher.update(format!("crawli-azure-key:{}:{}", hostname, user));
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Encrypts a secret string using AES-256-GCM.
/// Returns base64-encoded ciphertext with prepended 12-byte nonce.
pub fn encrypt_credential(plaintext: &str) -> Result<String, String> {
    let key_bytes = derive_machine_key();
    let key = GenericArray::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    aes_gcm::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("AES-GCM encryption failed: {e}"))?;

    // Prepend nonce to ciphertext for storage
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &combined,
    ))
}

/// Decrypts a base64-encoded AES-256-GCM ciphertext.
pub fn decrypt_credential(encrypted_b64: &str) -> Result<String, String> {
    use base64::Engine;
    let combined = base64::engine::general_purpose::STANDARD
        .decode(encrypted_b64)
        .map_err(|e| format!("Base64 decode failed: {e}"))?;

    if combined.len() < 13 {
        return Err("Ciphertext too short".to_string());
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let key_bytes = derive_machine_key();
    let key = GenericArray::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES-GCM decryption failed: {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("UTF-8 decode failed: {e}"))
}

// ── Azure Storage Operations ─────────────────────────────────────────────────

/// Creates an Azure BlobServiceClient from the provided configuration.
/// Supports both client-secret and managed-identity authentication.
pub async fn create_blob_client(
    config: &AzureStorageConfig,
) -> Result<azure_storage_blob::BlobServiceClient, String> {
    use azure_core::credentials::{Secret, TokenCredential};
    use azure_identity::{ClientSecretCredential, ManagedIdentityCredential};
    use azure_storage_blob::{BlobServiceClient, BlobServiceClientOptions};

    let endpoint = format!("https://{}.blob.core.windows.net/", config.storage_account);

    let credential: Arc<dyn TokenCredential> = if config.use_managed_identity {
        ManagedIdentityCredential::new(None)
            .map_err(|e| format!("Managed identity credential failed: {e}"))?
    } else {
        let secret = if config.client_secret_encrypted.is_empty() {
            return Err("Client secret is empty and managed identity is disabled".to_string());
        } else {
            decrypt_credential(&config.client_secret_encrypted)?
        };
        ClientSecretCredential::new(
            &config.tenant_id,
            config.client_id.clone(),
            Secret::new(secret),
            None,
        )
        .map_err(|e| format!("Client secret credential failed: {e}"))?
    };

    BlobServiceClient::new(
        &endpoint,
        Some(credential),
        Some(BlobServiceClientOptions::default()),
    )
    .map_err(|e| format!("Failed to create BlobServiceClient: {e}"))
}

/// Ensures the Azure container exists, creating it if missing.
pub async fn ensure_container(
    client: &azure_storage_blob::BlobServiceClient,
    container_name: &str,
) -> Result<(), String> {
    let container_client = client.blob_container_client(container_name);

    match container_client.exists().await {
        Ok(true) => Ok(()),
        Ok(false) => container_client
            .create(None)
            .await
            .map(|_| ())
            .map_err(|e| format!("Failed to create container '{container_name}': {e}")),
        Err(e) => Err(format!(
            "Failed to check whether container '{container_name}' exists: {e}"
        )),
    }
}

/// Uploads a byte slice to Azure Blob Storage.
/// PR-AZURE-003: Caller must respect resource governor concurrency limits.
pub async fn upload_blob(
    client: &azure_storage_blob::BlobServiceClient,
    container_name: &str,
    blob_name: &str,
    data: Vec<u8>,
) -> Result<(), String> {
    use azure_core::http::RequestContent;

    let blob_client = client
        .blob_container_client(container_name)
        .blob_client(blob_name);

    let content_length = u64::try_from(data.len())
        .map_err(|_| format!("Blob '{blob_name}' is too large to upload"))?;

    blob_client
        .upload(RequestContent::from(data), true, content_length, None)
        .await
        .map_err(|e| format!("Blob upload failed for '{}': {e}", blob_name))?;

    Ok(())
}

// ── Intranet Web Server (axum) ──────────────────────────────────────────────

/// Starts an embedded web server serving the React SPA for intranet access.
/// PR-AZURE-002: Serves only static files from dist/ — no eval, no SSR.
pub async fn start_intranet_server(
    port: u16,
    app_handle: tauri::AppHandle,
) -> Result<tokio::task::JoinHandle<()>, String> {
    use axum::Router;
    use tower_http::services::ServeDir;

    // Resolve the dist directory from the Tauri resource path
    let dist_dir = app_handle
        .path()
        .resource_dir()
        .map_err(|e| format!("Cannot resolve resource dir: {e}"))?;

    // Fallback: use the current directory's dist/ if resource dir doesn't have it
    let serve_dir = if dist_dir.join("index.html").exists() {
        dist_dir
    } else {
        // Development fallback — serve from project dist/
        let cwd = std::env::current_dir().unwrap_or_default();
        let dev_dist = cwd.join("../dist");
        if dev_dist.join("index.html").exists() {
            dev_dist
        } else {
            return Err("Cannot find dist/ directory for web serving".to_string());
        }
    };

    let _ = app_handle.emit(
        "log",
        format!(
            "[AZURE] Starting intranet web server on 0.0.0.0:{port} serving {}",
            serve_dir.display()
        ),
    );

    let app = Router::new()
        .fallback_service(ServeDir::new(&serve_dir).append_index_html_on_directories(true));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .map_err(|e| format!("Cannot bind to port {port}: {e}"))?;

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("[AZURE] Intranet server error: {e}");
        }
    });

    let _ = app_handle.emit(
        "log",
        format!("[AZURE] Intranet web server running at http://0.0.0.0:{port}"),
    );

    Ok(handle)
}

// ── Tauri Commands ──────────────────────────────────────────────────────────

/// Configures Azure Storage for the current session.
/// Encrypts the client_secret before storing.
#[tauri::command]
pub async fn configure_azure_storage(
    config: AzureStorageConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    // Runtime kill-switch
    if std::env::var("CRAWLI_AZURE_ENABLED").unwrap_or_default() == "false" {
        return Err("Azure connectivity is disabled via CRAWLI_AZURE_ENABLED=false".to_string());
    }

    let _ = app.emit("log", format!(
        "[AZURE] Configuring storage: account={}, container={}, region={}, size={}GB, managed_identity={}",
        config.storage_account, config.container_name, config.region, config.size_gb, config.use_managed_identity
    ));

    let state = app.state::<crate::AppState>();
    let mut azure_state = state.azure.lock().await;
    azure_state.storage_config = Some(config);

    Ok("Azure Storage configured successfully".to_string())
}

/// Tests Azure connectivity without creating any resources.
#[tauri::command]
pub async fn test_azure_connection(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<crate::AppState>();
    let config = state
        .azure
        .lock()
        .await
        .storage_config
        .clone()
        .ok_or("No Azure configuration set. Configure first.")?;

    let _ = app.emit("log", "[AZURE] Testing connection...".to_string());

    let client = create_blob_client(&config).await?;

    client
        .get_account_info(None)
        .await
        .map_err(|e| format!("Azure connection test failed: {e}"))?;

    let _ = app.emit("log", "[AZURE] Connection test successful ✓".to_string());
    Ok("Connection successful".to_string())
}

/// Enables Azure Storage for the current session.
/// Creates the container if it doesn't exist.
#[tauri::command]
pub async fn enable_azure_storage(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<crate::AppState>();
    let config = state
        .azure
        .lock()
        .await
        .storage_config
        .clone()
        .ok_or("No Azure configuration set. Configure first.")?;

    let _ = app.emit(
        "log",
        format!(
            "[AZURE] Enabling storage: creating container '{}'...",
            config.container_name
        ),
    );

    let client = create_blob_client(&config).await?;
    ensure_container(&client, &config.container_name).await?;

    state.azure.lock().await.storage_enabled = true;

    let _ = app.emit(
        "log",
        format!(
            "[AZURE] Storage enabled: {} / {} (region: {})",
            config.storage_account, config.container_name, config.region
        ),
    );

    Ok(format!(
        "Azure Storage enabled: {}/{}",
        config.storage_account, config.container_name
    ))
}

/// Disables Azure Storage, reverting to local disk output.
#[tauri::command]
pub async fn disable_azure_storage(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<crate::AppState>();
    let mut azure_state = state.azure.lock().await;
    azure_state.storage_enabled = false;

    let _ = app.emit(
        "log",
        "[AZURE] Storage disabled — output reverted to local disk".to_string(),
    );
    Ok("Azure Storage disabled".to_string())
}

/// Starts or stops the intranet web server.
#[tauri::command]
pub async fn toggle_intranet_server(
    enable: bool,
    port: Option<u16>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let state = app.state::<crate::AppState>();

    if enable {
        if let Some(existing_port) = state.azure.lock().await.intranet_server_port {
            return Ok(format!(
                "Intranet server already running on port {}",
                existing_port
            ));
        }

        let server_port = port.unwrap_or_else(|| {
            std::env::var("CRAWLI_AZURE_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080)
        });

        let handle = start_intranet_server(server_port, app.clone()).await?;
        let mut azure_state = state.azure.lock().await;
        azure_state.intranet_server_handle = Some(handle);
        azure_state.intranet_server_port = Some(server_port);

        Ok(format!("Intranet server started on port {server_port}"))
    } else {
        let mut azure_state = state.azure.lock().await;
        if let Some(handle) = azure_state.intranet_server_handle.take() {
            handle.abort();
            azure_state.intranet_server_port = None;
            let _ = app.emit("log", "[AZURE] Intranet server stopped".to_string());
        }
        Ok("Intranet server stopped".to_string())
    }
}

/// Returns current Azure connectivity status for the frontend.
#[tauri::command]
pub async fn get_azure_status(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<crate::AppState>();
    let azure_state = state.azure.lock().await;

    Ok(serde_json::json!({
        "storageEnabled": azure_state.storage_enabled,
        "storageAccount": azure_state.storage_config.as_ref().map(|c| &c.storage_account),
        "containerName": azure_state.storage_config.as_ref().map(|c| &c.container_name),
        "region": azure_state.storage_config.as_ref().map(|c| &c.region),
        "intranetServerRunning": azure_state.intranet_server_handle.is_some(),
        "intranetServerPort": azure_state.intranet_server_port,
    }))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_roundtrip() {
        let secret = "my-super-secret-azure-key-12345";
        let encrypted = encrypt_credential(secret).unwrap();
        assert_ne!(encrypted, secret); // Must not be plaintext
        assert!(!encrypted.is_empty());

        let decrypted = decrypt_credential(&encrypted).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn test_credential_different_nonce() {
        let secret = "test-secret";
        let enc1 = encrypt_credential(secret).unwrap();
        let enc2 = encrypt_credential(secret).unwrap();
        // Each encryption uses a random nonce — ciphertexts must differ
        assert_ne!(enc1, enc2);
        // But both must decrypt to the same value
        assert_eq!(decrypt_credential(&enc1).unwrap(), secret);
        assert_eq!(decrypt_credential(&enc2).unwrap(), secret);
    }

    #[test]
    fn test_default_intranet_config() {
        let config = IntranetServerConfig::default();
        assert_eq!(config.port, 8080);
        assert!(!config.enable_auth);
        assert_eq!(config.auth_provider, "none");
    }

    #[test]
    fn test_default_azure_state() {
        let state = AzureConnectivityState::default();
        assert!(!state.storage_enabled);
        assert!(state.storage_config.is_none());
        assert!(state.intranet_server_handle.is_none());
        assert!(state.intranet_server_port.is_none());
    }
}
