use crawli_lib::adapters::dragonforce::parse_dragonforce_fsguest;

#[tokio::test]
async fn test_dragonforce_cli_crawl() {
    if std::env::var("RUN_LIVE_ONION_TESTS").ok().as_deref() != Some("1") {
        eprintln!("Skipping live DragonForce crawl test (set RUN_LIVE_ONION_TESTS=1 to enable).");
        return;
    }

    let url = "http://fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion/?path=RJZ-APP1/G/01%20RJZ/02%20RJZ%20Estimating&token=eyJhbGciOiJSUzUxMiIsInR5cCI6IkpXVCJ9.eyJjb2xvcl9pbnB1dF9iYWNrZ3JvdW5kIjoiIzJCMkEzM0ZGIiwiY29sb3JfbWFpbiI6IiMyMjIyMjJGRiIsImNvbG9yX21haW5fZGFyayI6IiMxQjFCMUJGRiIsImNvbG9yX21haW5fbGlnaHQiOiIjNDQ0NDQ0RkYiLCJjb2xvcl9wcmltYXJ5IjoiI0YyOEM0NkZGIiwiY29sb3JfdGV4dCI6IiNGRkZGRkZGRiIsImRlcGxveV91dWlkIjoiMmE3ZDY3YjEtOWY3My00ZDI4LWIzZjAtOTgxMGM1YWU2Y2YyIiwiZXhwIjoxNzcyMzc5NzAzLCJpYXQiOjE3NzIzMzY1MDMsIndlYnNpdGUiOiJ3d3cucmp6YXZvcmFsLmNvbSJ9.TM0tWeaFyJ_UXd_5JYlnyWBjMPOt3rvDxBoqhoDWn78d9fLE5gNFLR1dcoeohjqvy8ya9Or98uX-lBwSAsncl1DPMOz-GUDdJL0e-lxdVCWGCvbr9_Ul-HDASzV3SVmGlpayHQk2AAU3esVX536ku8cn6tDmJC5qv5zALb7c-TM5z7Kpg6WiBwzVucFv6GjKr4twOoMF_IEwQO_3EeJVPn3FYBzStQUfYoYdXOf4_KiOHUxS_EZEwhnnfl25356icdYfkAICd1ck1PClniBqK8_USy6q6CdAK7GMyd7GET0y1tcRkgd6tPkF8yWFN8Ko3miUQXQkGvW3EEsKofbvmwf16wBcOOx2mQJX5ZD7UQXXdjam4pYBAbr7D4F_niN-PERV1JfF2gW_MsRWnfi_C-8ANoYvjfZLnItv4KTQwMHmbQSXVJ-BL6S5fNBHNGUhbZxaV0l1v4pdBKNZbH6ZvP5TXrKhFpvAp87jDc-8BcxvQ6LJ6-bV_70YmB3HDhYW-uLTM0-MezCe8_yOR9qhcwWQetQYjfP0Pzh3dX_oel7z4nMfOjo2YUXpE4pOkUR_QttK7Mc10zM4IXxxAQMKdH3MUW473Pkd3gWj-IgQ5D6cm7HvkwJjVtpBux_cT6wkJuFQTQlK-P-fdybb-DcRvvKHFLAO9FdI8UE4LFF4TO8";

    // Setup reqwest to route through local Tor proxy matching the one running in the CLI test env (9150 from Tor Browser, or 9051)
    // We'll use 9150 since `curl` worked on it above.
    let proxy = reqwest::Proxy::all("socks5h://127.0.0.1:9150").unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .build()
        .unwrap();

    let resp = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            eprintln!("Skipping live DragonForce crawl test: {}", err);
            return;
        }
    };
    let html = match resp.text().await {
        Ok(html) => html,
        Err(err) => {
            eprintln!("Skipping live DragonForce crawl test: failed reading response body: {}", err);
            return;
        }
    };

    let host = "fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion";
    let entries = parse_dragonforce_fsguest(&html, host);

    println!("=================== CLI CRAWL RESULTS ===================");
    println!("Crawled: {}", url);
    println!("Found {} entries.", entries.len());
    for e in &entries {
        println!("{:?} | Size: {:?} | Path: {}", e.entry_type, e.size_bytes, e.path);
        println!("URL: {}", e.raw_url);
        println!("---------------------------------------------------------");
    }

    assert!(!entries.is_empty(), "Failed to extract items from DragonForce SPA!");
    assert!(entries.iter().any(|e| e.size_bytes.is_some()), "Failed to extract accurate byte sizes!");
}
