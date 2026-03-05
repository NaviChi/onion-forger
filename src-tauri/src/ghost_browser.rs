use headless_chrome::{Browser, LaunchOptions};
use anyhow::{Result, anyhow};
use std::ffi::OsStr;
use std::time::Duration;

/// Launches a headless Chromium instance securely bound to a specific Tor proxy port.
/// This acts as a completely passive, structural "Ghost Browser" to render
/// React/Vue DOMs without requiring malicious API fuzzing or payload injection.
pub fn launch_tor_ghost_browser(tor_port: u16) -> Result<Browser> {
    let proxy_arg = format!("--proxy-server=socks5://127.0.0.1:{}", tor_port);
    
    let launch_opts = LaunchOptions::default_builder()
        .headless(true)
        .sandbox(true)
        // Ensure DNS is also routed through the Tor SOCKS5 proxy
        .args(vec![
            OsStr::new(&proxy_arg),
            OsStr::new("--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1"),
            OsStr::new("--disable-gpu"),
            OsStr::new("--disable-dev-shm-usage"),
            OsStr::new("--no-zygote"),
        ])
        .idle_browser_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| anyhow!("Failed to build Chrome launch options: {}", e))?;

    let browser = Browser::new(launch_opts)
        .map_err(|e| anyhow!("Failed to launch Headless Chrome: {}", e))?;

    Ok(browser)
}

/// Navigates to a URL through the Ghost Browser, waiting for the network to idle
/// (meaning all React/XHR requests have finished legitimately), and returns the fully rendered HTML.
pub fn extract_rendered_dom(browser: &Browser, url: &str) -> Result<String> {
    let tab = browser.new_tab()
        .map_err(|e| anyhow!("Failed to create new tab: {}", e))?;
    
    // Navigate and strictly wait for the page to finish all network requests
    tab.navigate_to(url)
        .map_err(|e| anyhow!("Ghost Browser failed to navigate to {}: {}", url, e))?;
        
    // Wait for the specific QData DOM elements to appear (or just wait for idle)
    tab.wait_until_navigated()
        .map_err(|e| anyhow!("Ghost Browser navigation timeout: {}", e))?;
        
    // Specifically wait for the table or list to render in QData
    let _ = tab.wait_for_element(".el-table__body, .item_box_photos");

    // Extract the final, fully-rendered HTML
    let html = tab.get_content()
        .map_err(|e| anyhow!("Failed to extract DOM content: {}", e))?;

    Ok(html)
}
