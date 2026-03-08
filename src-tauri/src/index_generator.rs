use anyhow::Result;
use std::path::Path;

pub fn generate_index_html(
    target_key: &str,
    output_path: &Path,
    json_listing_path: &Path,
) -> Result<()> {
    // We expect the JSON file to contain an array of FileEntry (from target_state.rs)
    // We will build a clean, static, aesthetic "SnoozeSlayer / Military Grade" HTML index for it.

    let entries = match std::fs::read_to_string(json_listing_path) {
        Ok(data) => data,
        Err(_) => "[]".to_string(), // In case it wasn't written yet or is empty
    };

    let html_content = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Qilin Output: {target_key}</title>
    <style>
        :root {{
            --bg: #09090b;
            --surface: #18181b;
            --border: #27272a;
            --accent: #22c55e;
            --text-main: #f4f4f5;
            --text-muted: #a1a1aa;
            --font-mono: 'Space Mono', 'Courier New', Courier, monospace;
            --font-sans: 'Inter', -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        }}
        body {{
            background-color: var(--bg);
            color: var(--text-main);
            font-family: var(--font-sans);
            margin: 0;
            padding: 2rem;
            line-height: 1.5;
        }}
        .container {{
            max-width: 1200px;
            margin: 0 auto;
        }}
        header {{
            border-bottom: 1px solid var(--border);
            padding-bottom: 2rem;
            margin-bottom: 2rem;
        }}
        h1 {{
            font-size: 1.5rem;
            font-weight: 600;
            margin: 0 0 0.5rem 0;
            letter-spacing: -0.025em;
        }}
        .mono-id {{
            font-family: var(--font-mono);
            color: var(--text-muted);
            font-size: 0.875rem;
        }}
        .badge {{
            display: inline-flex;
            align-items: center;
            padding: 0.25rem 0.75rem;
            border-radius: 9999px;
            background-color: rgba(34, 197, 94, 0.1);
            color: var(--accent);
            font-size: 0.75rem;
            font-weight: 500;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            margin-top: 1rem;
            border: 1px solid rgba(34, 197, 94, 0.2);
        }}
        .tree-container {{
            background-color: var(--surface);
            border: 1px solid var(--border);
            border-radius: 0.5rem;
            padding: 1.5rem;
            overflow-x: auto;
        }}
        .item {{
            display: flex;
            align-items: center;
            padding: 0.5rem;
            border-radius: 0.25rem;
            font-family: var(--font-mono);
            font-size: 0.875rem;
        }}
        .item:hover {{
            background-color: rgba(255, 255, 255, 0.05);
        }}
        .icon {{
            width: 1rem;
            height: 1rem;
            margin-right: 0.75rem;
            opacity: 0.7;
            flex-shrink: 0;
        }}
        .type-dir {{ color: #fbbf24; }}
        .type-file {{ color: var(--text-main); }}
        .name {{ flex-grow: 1; }}
        .meta {{
            color: var(--text-muted);
            font-size: 0.75rem;
            display: flex;
            gap: 1rem;
        }}
        .size {{ width: 80px; text-align: right; }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>Onion Forger: Target Manifest</h1>
            <div class="mono-id">KEY // {target_key}</div>
            <div class="badge">AEROSPACE GRADE CRAWL</div>
        </header>

        <main>
            <div class="tree-container" id="tree">
                <!-- Data populated by JS -->
            </div>
        </main>
    </div>

    <script>
        const rawData = {entries};
        const treeEl = document.getElementById('tree');

        // Simple flat list renderer for now (a real tree would parse the paths)
        rawData.sort((a,b) => a.path.localeCompare(b.path)).forEach(entry => {{
            const isDir = entry.entryType === "folder" || entry.entry_type === "Folder";
            const item = document.createElement('div');
            item.className = 'item';
            
            const iconSvg = isDir 
                ? `<svg class="icon type-dir" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>`
                : `<svg class="icon type-file" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"/></svg>`;
            
            const sizeStr = entry.sizeBytes !== null && entry.size_bytes !== null 
                ? (entry.sizeBytes || entry.size_bytes || 0).toLocaleString() + ' B'
                : '--';

            item.innerHTML = `
                ${{iconSvg}}
                <span class="name ${{isDir ? 'type-dir' : 'type-file'}}">${{entry.path}}</span>
                <span class="meta">
                    <span class="size">${{isDir ? '' : sizeStr}}</span>
                </span>
            `;
            treeEl.appendChild(item);
        }});
    </script>
</body>
</html>"#,
        target_key = target_key,
        entries = entries
    );

    std::fs::write(output_path, html_content)?;
    Ok(())
}
