use reqwest::Proxy;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let proxy = Proxy::all("socks5h://127.0.0.1:9050").unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(90))
        .build()
        .unwrap();

    let target_url = "http://fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion/?path=/&token=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"; // We need a real token, but we can just fetch the iframe without token first, maybe it redirects, or we fetch the root domain and parse the iframe src.

    let root_url = "http://dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion/www.rjzavoral.com";
    println!("Fetching Root: {}", root_url);
    match client.get(root_url).send().await {
        Ok(resp) => {
            match resp.text().await {
                Ok(text) => {
                    std::fs::write("/tmp/df_root.html", &text).unwrap();
                    println!("Saved root to /tmp/df_root.html");
                    let doc = scraper::Html::parse_document(&text);
                    let selector = scraper::Selector::parse("iframe").unwrap();
                    if let Some(iframe) = doc.select(&selector).next() {
                        if let Some(src) = iframe.value().attr("src") {
                            println!("Found iframe src: {}", src);
                            println!("Fetching iframe...");
                            match client.get(src).send().await {
                                Ok(resp2) => {
                                    if let Ok(text2) = resp2.text().await {
                                        std::fs::write("/tmp/df_fsguest.html", &text2).unwrap();
                                        println!("Saved fsguest to /tmp/df_fsguest.html");
                                        
                                        let doc2 = scraper::Html::parse_document(&text2);
                                        let script_sel = scraper::Selector::parse("script#__NEXT_DATA__").unwrap();
                                        if let Some(script) = doc2.select(&script_sel).next() {
                                            std::fs::write("/tmp/df_next.json", script.inner_html()).unwrap();
                                            println!("Saved next data to /tmp/df_next.json");
                                        }
                                    }
                                }
                                Err(e) => println!("Error fetching iframe: {}", e),
                            }
                        }
                    } else {
                        println!("No iframe found.");
                    }
                }
                Err(e) => println!("Error reading text: {}", e),
            }
        }
        Err(e) => println!("Error fetching root: {}", e),
    }
}
