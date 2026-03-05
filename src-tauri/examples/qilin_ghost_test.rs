use crawli_lib::ghost_browser::{launch_tor_ghost_browser, extract_rendered_dom};
use anyhow::Result;

fn main() -> Result<()> {
    println!("👻 Booting Ghost Browser Engine...");

    // We assume Tor daemon is running on port 9051 for this test
    let browser = match launch_tor_ghost_browser(9051) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to boot Tor Chromium instance: {}", e);
            return Err(e);
        }
    };
    
    println!("✅ Chrome Sandboxed Engine Online (Proxy: 127.0.0.1:9051)");

    let test_url = "http://qjupqf5xbmc76jzne7xu7y2ddmwtfxbbzzeax6gs4lezg3dyr5bfu2qd.onion/d7789467-8d06-4778-895c-4b81c203881f/";
    println!("🎯 Navigating to Qilin Storage Node...");
    println!("URL: {}", test_url);

    match extract_rendered_dom(&browser, test_url) {
        Ok(html_content) => {
            let html_string: String = html_content;
            println!("\n✅ Successfully extracted POST-JAVASCRIPT Rendered DOM!");
            println!("HTML Length: {} bytes", html_string.len());
            println!("--- HTML DUMP START ---");
            println!("{}", html_string);
            println!("--- HTML DUMP END ---");
            
            // Print a small snippet to prove we got the actual data table, 
            // not a blank screen waiting for JS.
            if html_string.contains("data-v-") || html_string.contains("el-table") {
                println!("Confirmed: Vue/React data table is present in the extracted HTML structure.");
            } else {
                println!("Warning: The rendered HTML seems to be missing the known QData UI elements.");
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to extract DOM: {}", e);
        }
    }

    Ok(())
}
