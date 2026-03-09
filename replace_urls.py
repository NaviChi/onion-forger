import os
import glob

old_url = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed"
old_url_data = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/data?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed"
new_url = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/"

old_domain = "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion"
new_domain = "25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion"

search_dirs = ["src-tauri/src", "src-tauri/examples", "src-tauri/tests", "docs"]

def replace_in_file(filepath):
    try:
        with open(filepath, 'r', encoding='utf-8') as f:
            content = f.read()
        
        updated = content.replace(old_url, new_url)
        updated = updated.replace(old_url_data, new_url)
        # Also replace standalone domain occurrences in known_domains.json
        if "known_domains.json" in filepath:
            updated = updated.replace(old_domain, new_domain)
            
        if updated != content:
            with open(filepath, 'w', encoding='utf-8') as f:
                f.write(updated)
            print(f"Updated {filepath}")
    except Exception as e:
        pass

for d in search_dirs:
    for root, _, files in os.walk(d):
        for file in files:
            if file.endswith('.rs') or file.endswith('.json') or file.endswith('.md') or file.endswith('.csv'):
                replace_in_file(os.path.join(root, file))

print("Done.")
