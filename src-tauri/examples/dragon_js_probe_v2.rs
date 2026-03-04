use std::time::Duration;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1).await.unwrap();

    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", ports[0])).unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let target_js = "http://fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion/_next/static/chunks/pages/index-740b2fdf92edbdce.js";
    println!("Fetching JS bundle: {}", target_js);

    match client.get(target_js).send().await {
        Ok(resp) => {
            let status = resp.status();
            println!("JS Status: {}", status);
            if let Ok(text) = resp.text().await {
                std::fs::write("/tmp/dragon_index.js", &text).unwrap();
                println!("Saved JS to /tmp/dragon_index.js");
                println!("{}", text.chars().take(200).collect::<String>());
            }
        }
        Err(e) => println!("Error: {}", e),
    }
    
    // Instead of guessing index.js hash, let's just get the main _app.js bundle
    let app_js = "http://fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion/_next/static/chunks/pages/_app-784f1a21e7fcbe73.js";
    println!("Fetching JS app bundle: {}", app_js);

    match client.get(app_js).send().await {
        Ok(resp) => {
            let status = resp.status();
            println!("App JS Status: {}", status);
            if let Ok(text) = resp.text().await {
                std::fs::write("/tmp/dragon_app.js", &text).unwrap();
                println!("Saved App JS to /tmp/dragon_app.js");
            }
        }
        Err(e) => println!("Error: {}", e),
    }

    drop(swarm_guard);
}
