use crawli_lib::adapters::dragonforce::DragonForceAdapter;
use crawli_lib::adapters::{CrawlerAdapter, EntryType, FileEntry};
use crawli_lib::arti_client::ArtiClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("[*] Starting JWT Refresh Integration Test");

    let req_client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("socks5h://127.0.0.1:9050")?)
        .danger_accept_invalid_certs(true)
        .build()?;
    let client = ArtiClient::Clearnet { client: req_client };
    let adapter = DragonForceAdapter::new();

    // Expired or fake token entry
    let test_url = "http://fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion/?path=RJZ-APP1/G/01%20RJZ/2%20-%20Construction/2D%20-%20Design%20Engineering/RJZ%20Hangar%20-%20Arch%20Permit%20Set_Rev%201.pdf&token=eyJhbGciOiJSUzUxMiIsInR5cCI6IkpXVCJ9.eyJjb2xvcl9pbnB1...EXPIRED_TEST_TOKEN";

    let dummy_entry = FileEntry {
        raw_url: test_url.to_string(),
        path: "/RJZ-APP1/G/01 RJZ/2 - Construction/2D - Design Engineering/RJZ Hangar - Arch Permit Set_Rev 1.pdf".to_string(),
        size_bytes: Some(1234),
        jwt_exp: Some(0),
        entry_type: EntryType::File,
    };

    println!("[*] Invoking refresh_jwt on adapter...");

    let result = adapter.refresh_jwt(&dummy_entry, &client).await;

    match result {
        Ok(Some(fresh_entry)) => {
            println!("\n[+] Success! Successfully acquired fresh JWT.");
            println!("    Refreshed URL: {}", fresh_entry.raw_url);
            println!("    New JWT Expiry: {:?}", fresh_entry.jwt_exp);
        }
        Ok(None) => {
            println!("\n[-] Failure/None: The adapter could not refresh the token.");
        }
        Err(e) => {
            println!("\n[!] Error: {}", e);
        }
    }

    Ok(())
}
