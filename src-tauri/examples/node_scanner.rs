/// Node Scanner — probes every known Qilin storage node one at a time
/// with patient timeouts and reports what data is available.
///
/// Usage:  cargo run --example node_scanner 2>&1
///
use anyhow::Result;
use crawli_lib::arti_client::ArtiClient;
use crawli_lib::tor_native::spawn_tor_node;
use std::time::{Duration, Instant};

const UUID: &str = "c9d2ba19-6aa1-3087-8773-f63d023179ed";
const PROBE_TIMEOUT_SECS: u64 = 45;

/// All known storage hosts from the Sled cache + hardcoded mirrors
const KNOWN_HOSTS: &[&str] = &[
    // CMS itself
    "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion",
    // Phase 77D newly discovered
    "onlta6cik443t67n5zqlbbcmhepazzlonhsx27qidmpf6zha6bxsjcid.onion",
    "42hfjtvbstk472gbv42sxqqabupa5d2ow2mahc6zq4orpe4bpo63gcyd.onion",
    "5nqgp7hmstqsvlqu3wr6o5mg6twxpz3fvyiqyctxyx4hfoynqbm74qyd.onion",
    // Previously known QData
    "szgkpzhcrnshftjb5mtvd6bc5oep5yabmgfmwt7u3tiqzfikoew27hqd.onion",
    "25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion",
    "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion",
    "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion",
    "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion",
    // Discovered via sled (prior runs)
    "qjupqf5xbmc76jzne7xu7y2ddmwtfxbbzzeax6gs4lezg3dyr5bfu2qd.onion",
    "sbedmjsyphfctagwoxuspblefvzjvb7yig4gsq5ddwjhnyq4rqcqg3ad.onion",
    "xy6pysqr5myuau4aq6uszwdgdmjx4ypjlvngupxfjdtzfsq6jugcadyd.onion",
    "amkryua4xdnbvk4urxleuxkcdgiirmus7m2wnqj3o4uh2xcgbkpcjoyd.onion",
    "astvjnzh4ftvnp37n47zgr3qhbyftlmjdocjnwjb5xlua5xgdckew6yd.onion",
    "bmwlkiljav3aqxbgyrqgcmotasrnnolqfivzorpn7snrmprj2sqqlbqd.onion",
    "cw2kf4ieepslxvydi7qgb5vc2itst4b6roah5rc3ozeu4ulbqz4v3rqd.onion",
    "ghnqjhi7usidnrnktsctb5do26m4xbaprenpy3fzkfatvf536w5drrid.onion",
    "vzgsc7keieq52csmskmmhop2yc2tys32jpj7wdgzhsoctpi4wx4hx3ad.onion",
    "n2bpey4k45pkwjfsuqpuagm2rjyaefako4hqz2pgwqaew3rs4iy7brid.onion",
    "ckj4f6jmx7rwvr6qcc7bkx3ziluf6s2kas2xua47ze7jcjvrh6bvihyd.onion",
    "7zffbbkye7c7m4676sqfxhcwtjcuslhlmxmeg7yhf3a24xl7ppm36tid.onion",
    // Nodes from failed_nodes.log
    "kiutlxamgt3f5ftdv2mp22xdhufjqajdqjyuzl537c462hhewycovpyd.onion",
    "5hct24dm7tzxpjykqbbdvizuxraor5xi6qosicktbkhsfs2kuvafabyd.onion",
    "6eoxnxd2y5xryvgyh22k3wknwrmw7w5i7l4yi2tu57w52v5ttjcif4ad.onion",
    "r6kenxcmw6dlvzxnq765msdjje337rknr6he3psfpmx3xpdi7k2hneyd.onion",
    "aintbfdyiu5lebqrwptuo5wtgpihhqfm6pwvpyut4nk65yvx2ixdskyd.onion",
    "wljwvyjjwnfgmvdcxrqmokayjwoeeluftgtblqxhbmms6vzdwe744aid.onion",
    "x7feeedrp5dhns6cw6ewprtmeh6tzsqa5kpouapahjav4qnvniwnwiyd.onion",
    "zjucgzp2hjk423wjpav4shyvxp3yb6fxhh2fetigripynbznt5pu32ad.onion",
    "irp4kurbfsrilwfjdqpzwlkbv7ylbwvuhlub7x7qwrbknvrfczxdyuad.onion",
    "jww5jzwem5b65dn5js2w3tnm7s652zukgcam4tfrpksykhcpv3pyoaid.onion",
    "nuaftzulpokwxbqh3n5pn2p6iyu3loguej5kdfzsscqraqv4wycbceyd.onion",
    "pbtv44ncj3ret5gypgdaakl4dxdjcduyr7rcvdfnnuz7b6xf63ckdtyd.onion",
    "r57g6bsvb7k3afxsim22syjj4dxfji2inpkiwi7y5i2pqsq2zlivxoqd.onion",
    "tyxoxeljccxxxm55vlntefoftstbelml6txbqtclhahb63iz34peqiid.onion",
    "txbtz56ngkfif64knbqfihzseay5ticlmkvzi62b556jxeynvriiqpyd.onion",
    // Phase 105 discovered (speed champions from prior sessions)
    "3pe26tqcx2mqwrrfqxpehugzrwfyyuxpklrxbtsa6a3gmogulyvcslqd.onion",
    "4xl2hta3wk7exyxbqxhlr3rnhjfhdpj6q4urcxx7c3lywodqhmcbmqd.onion",
    "lblnwlidqvp4ic7hsnxbglqetkgw2lfl2oqa4xkocepfwwfmr5n34qd.onion",
    "zqetti36k3enp7ww53tyifmgdwckmzdnppraqho6tic5lj4q5qtim2ad.onion",
    "sc2qyv6sxfgptxrruhpfsqmhbefhh7d33nv7lnmmj4ycuaxlq577yqd.onion",
    "rbuio2uge4ox3x2dh2jxnwl2t3h7z3hy2y7yh5sfqwstzfaouqmr3ad.onion",
    "ytbhximfkc5s5n563gufnytxhd2hqibfk7tnxocddg4elzpmxnuquad.onion",
    "aay7nawym667gwn7w3hxfkfpmbqkwbpynprxbndhnb5c7gnwltsojiqd.onion",
    "chygmtdubh4ydq7zg5e3bbndcm5oqltokimfyhwt2e7ybfhj3vy6cqd.onion",
    "2wyohlh5mszg53fgkjqwv2ibf3hl6bxwxfvhrb2k2w6jlrucmzzh7ad.onion",
    "lqcxwo4c46w7ugrymexmxp7p67fgahvcjjcygx3nqwygfrewhdnolad.onion",
];

#[derive(Debug)]
struct ProbeResult {
    host: String,
    status: String,
    latency_ms: u64,
    content_length: Option<usize>,
    has_autoindex: bool,
    has_qdata: bool,
    first_dirs: Vec<String>,
    first_files: Vec<String>,
}

fn parse_autoindex_entries(body: &str) -> (Vec<String>, Vec<String>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for line in body.lines() {
        if let Some(href_start) = line.find("href=\"") {
            let after_href = &line[href_start + 6..];
            if let Some(href_end) = after_href.find('"') {
                let href = &after_href[..href_end];
                if href == "../" || href == ".." || href == "." || href == "/" {
                    continue;
                }
                let decoded = percent_decode(href);
                if href.ends_with('/') {
                    dirs.push(decoded.trim_end_matches('/').to_string());
                } else {
                    files.push(decoded.to_string());
                }
            }
        }
    }
    (dirs, files)
}

fn percent_decode(s: &str) -> String {
    let mut result = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                result.push(byte as char);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║       🔍 QILIN NODE SCANNER — Patient Deep Probe          ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!("  UUID: {}", UUID);
    println!("  Nodes to scan: {}", KNOWN_HOSTS.len());
    println!("  Timeout per node: {}s", PROBE_TIMEOUT_SECS);
    println!("──────────────────────────────────────────────────────────────\n");

    // Bootstrap Arti TorClient
    println!("⏳ Bootstrapping Arti Tor client...");
    let boot_start = Instant::now();
    let tor_client = spawn_tor_node(0, false).await?;
    let client = ArtiClient::new(tor_client, Some(arti_client::IsolationToken::new()));
    println!(
        "✅ Arti ready in {:.1}s\n",
        boot_start.elapsed().as_secs_f64()
    );

    let mut results: Vec<ProbeResult> = Vec::new();
    let mut alive_count = 0usize;
    let mut dead_count = 0usize;

    // ── PRE-SCAN: Check CMS /site/data redirect ──
    println!("━━━ PRE-SCAN: Checking CMS /site/data redirect ━━━");
    let data_url = format!("http://{}/site/data?uuid={}", KNOWN_HOSTS[0], UUID);
    match tokio::time::timeout(
        Duration::from_secs(60),
        client
            .new_isolated()
            .get(&data_url)
            .header(
                "Referer",
                &format!("http://{}/", KNOWN_HOSTS[0]),
            )
            .send_capturing_redirect(),
    )
    .await
    {
        Ok(Ok((resp, redirect_url))) => {
            println!(
                "  CMS /site/data → status={} redirect={:?}",
                resp.status(),
                redirect_url
            );
            if let Some(ref target) = redirect_url {
                println!("  🎯 FRESH REDIRECT: {}", target);
                if let Ok(parsed) = reqwest::Url::parse(target) {
                    if let Some(host) = parsed.host_str() {
                        println!("  📍 Storage host from redirect: {}", host);
                    }
                }
            }
        }
        Ok(Err(e)) => println!("  CMS /site/data failed: {}", e),
        Err(_) => println!("  CMS /site/data timed out (60s)"),
    }
    println!();

    // ── SCAN ALL NODES (3 concurrent, patient timeouts) ──
    println!(
        "━━━ SCANNING {} NODES (3 concurrent, {}s timeout each) ━━━\n",
        KNOWN_HOSTS.len(),
        PROBE_TIMEOUT_SECS
    );

    for chunk in KNOWN_HOSTS.chunks(3) {
        let mut tasks = tokio::task::JoinSet::new();

        for host in chunk {
            let host = host.to_string();
            let isolated = client.new_isolated();
            let url = format!("http://{}/{}/", host, UUID);

            tasks.spawn(async move {
                let start = Instant::now();
                let probe = tokio::time::timeout(
                    Duration::from_secs(PROBE_TIMEOUT_SECS),
                    isolated.get(&url).send(),
                )
                .await;

                let latency = start.elapsed().as_millis() as u64;

                match probe {
                    Ok(Ok(resp)) => {
                        let status_str = format!("{}", resp.status());
                        let content_len = resp.content_length().map(|l| l as usize);

                        match resp.text().await {
                            Ok(body) => {
                                let has_autoindex = body.contains("Index of")
                                    || body.contains("<table id=\"list\">")
                                    || body.contains("<td class=\"link\">");
                                let has_qdata =
                                    body.contains("QData") || body.contains("Data browser");

                                let (dirs, files) = if has_autoindex || has_qdata {
                                    parse_autoindex_entries(&body)
                                } else {
                                    (vec![], vec![])
                                };

                                ProbeResult {
                                    host,
                                    status: status_str,
                                    latency_ms: latency,
                                    content_length: Some(body.len()),
                                    has_autoindex,
                                    has_qdata,
                                    first_dirs: dirs.into_iter().take(10).collect(),
                                    first_files: files.into_iter().take(5).collect(),
                                }
                            }
                            Err(e) => ProbeResult {
                                host,
                                status: format!("{} [body: {}]", status_str, e),
                                latency_ms: latency,
                                content_length: content_len,
                                has_autoindex: false,
                                has_qdata: false,
                                first_dirs: vec![],
                                first_files: vec![],
                            },
                        }
                    }
                    Ok(Err(e)) => {
                        let err_str = format!("{}", e);
                        let category = if err_str.contains("Connect") {
                            "CONNECT_FAIL"
                        } else if err_str.contains("imeout") || err_str.contains("TTFB") {
                            "TIMEOUT"
                        } else {
                            "ERROR"
                        };
                        ProbeResult {
                            host,
                            status: format!(
                                "{}: {}",
                                category,
                                &err_str[..err_str.len().min(80)]
                            ),
                            latency_ms: latency,
                            content_length: None,
                            has_autoindex: false,
                            has_qdata: false,
                            first_dirs: vec![],
                            first_files: vec![],
                        }
                    }
                    Err(_) => ProbeResult {
                        host,
                        status: format!("TIMEOUT ({}s)", PROBE_TIMEOUT_SECS),
                        latency_ms: latency,
                        content_length: None,
                        has_autoindex: false,
                        has_qdata: false,
                        first_dirs: vec![],
                        first_files: vec![],
                    },
                }
            });
        }

        while let Some(result) = tasks.join_next().await {
            if let Ok(probe) = result {
                let is_alive = probe.has_autoindex
                    || probe.has_qdata
                    || probe.status.starts_with("200")
                    || probe.status.starts_with("301")
                    || probe.status.starts_with("302");

                let icon = if is_alive {
                    "✅"
                } else if probe.status.contains("404") {
                    "🔶"
                } else {
                    "❌"
                };

                println!(
                    "  {} [{:>5}ms] {:.55}",
                    icon, probe.latency_ms, probe.host
                );
                println!(
                    "             Status: {}",
                    &probe.status[..probe.status.len().min(100)]
                );

                if is_alive {
                    alive_count += 1;
                    if !probe.first_dirs.is_empty() {
                        println!("             📁 Dirs: {}", probe.first_dirs.join(", "));
                    }
                    if !probe.first_files.is_empty() {
                        println!("             📄 Files: {}", probe.first_files.join(", "));
                    }
                    if probe.content_length.unwrap_or(0) > 0 {
                        println!(
                            "             Size: {} bytes",
                            probe.content_length.unwrap_or(0)
                        );
                    }
                } else {
                    dead_count += 1;
                }
                println!();
                results.push(probe);
            }
        }
    }

    // ── CMS VIEW PAGE DATA EXTRACTION ──
    println!("\n━━━ CMS VIEW PAGE DATA EXTRACTION ━━━");
    let view_url = format!(
        "http://{}/site/view?uuid={}",
        KNOWN_HOSTS[0], UUID
    );
    match tokio::time::timeout(Duration::from_secs(60), client.get(&view_url).send()).await {
        Ok(Ok(resp)) => {
            println!("  View page status: {}", resp.status());
            if let Ok(body) = resp.text().await {
                println!("  Body size: {} bytes", body.len());

                // Extract title
                if let Some(start) = body.find("<title>") {
                    if let Some(end) = body[start + 7..].find("</title>") {
                        println!("  Title: {}", &body[start + 7..start + 7 + end]);
                    }
                }

                // Count images/photos
                let img_count = body.matches("<img").count();
                let photo_count = body.matches("data-fancybox").count();
                println!(
                    "  Images: {} | Gallery photos: {}",
                    img_count, photo_count
                );

                // Extract all unique .onion hosts mentioned
                let onion_re = regex::Regex::new(r"([a-z2-7]{56}\.onion)").unwrap();
                let mut onion_hosts: Vec<String> = onion_re
                    .captures_iter(&body)
                    .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
                    .collect();
                onion_hosts.sort();
                onion_hosts.dedup();
                if !onion_hosts.is_empty() {
                    println!("  .onion hosts referenced in view page:");
                    for host in &onion_hosts {
                        println!("    🧅 {}", host);
                    }
                }

                let _ = std::fs::write("/tmp/qilin_view_page_scanner.html", &body);
                println!("  Full HTML saved to /tmp/qilin_view_page_scanner.html");
            }
        }
        Ok(Err(e)) => println!("  View page error: {}", e),
        Err(_) => println!("  View page timed out"),
    }

    // ── SUMMARY ──
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                     SCAN SUMMARY                           ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!(
        "║  Total Nodes Scanned:  {:>4}                                ║",
        results.len()
    );
    println!(
        "║  Alive (data):         {:>4}  ✅                            ║",
        alive_count
    );
    println!(
        "║  Dead/Unreachable:     {:>4}  ❌                            ║",
        dead_count
    );
    println!("╚══════════════════════════════════════════════════════════════╝");

    if alive_count > 0 {
        println!("\n━━━ ALIVE NODES WITH DATA ━━━");
        for probe in &results {
            if probe.has_autoindex || probe.has_qdata {
                println!("\n  🟢 {}", probe.host);
                println!("     Latency: {}ms", probe.latency_ms);
                println!(
                    "     Size: {} bytes",
                    probe.content_length.unwrap_or(0)
                );
                if !probe.first_dirs.is_empty() {
                    println!("     Top-level dirs:");
                    for dir in &probe.first_dirs {
                        println!("       📁 {}", dir);
                    }
                }
                if !probe.first_files.is_empty() {
                    println!("     Top-level files:");
                    for file in &probe.first_files {
                        println!("       📄 {}", file);
                    }
                }
            }
        }
    }

    // Save results as JSON
    let json_path =
        "/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/node_scan_results.json";
    let scan_data: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "host": r.host,
                "status": r.status,
                "latency_ms": r.latency_ms,
                "content_length": r.content_length,
                "has_autoindex": r.has_autoindex,
                "has_qdata": r.has_qdata,
                "dirs": r.first_dirs,
                "files": r.first_files,
            })
        })
        .collect();
    let _ = std::fs::write(json_path, serde_json::to_string_pretty(&scan_data)?);
    println!("\n📄 JSON results: {}", json_path);

    Ok(())
}
