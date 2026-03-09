#[tokio::test]
async fn test_reqwest_coexistence() {
    // Both reqwest 0.13 (reqwest) and reqwest 0.12 (reqwest_mega) exist in the dependency tree
    // Compile-time check: Can we instantiate both without linking errors?
    let client_13 = reqwest::Client::new();
    let client_12 = reqwest_mega::Client::new();
    
    // Smoke check they aren't somehow the same struct or shadowing
    let _req13 = client_13.get("https://example.com").build().unwrap();
    let _req12 = client_12.get("https://example.com").build().unwrap();
    
    // If we reach here, Rust linked both distinct HTTP/hyper stacks correctly.
    assert!(true, "Both versions of reqwest co-exist and build successfully");
}
