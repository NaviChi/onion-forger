use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let proxy = reqwest::Proxy::all("socks5h://127.0.0.1:9050")?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = "http://fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion/";
    println!("Fetching {} via 127.0.0.1:9050...", url);

    match client.get(url).send().await {
        Ok(resp) => {
            println!("Status: {}", resp.status());
            if let Ok(body) = resp.text().await {
                println!("Body: \n{}", &body.chars().take(2000).collect::<String>());
            }
        }
        Err(e) => {
            println!("Request explicitly failed: {:?}", e);
        }
    }

    Ok(())
}
