import time
import requests

proxies = {
    'http': 'socks5h://127.0.0.1:9050',
    'https': 'socks5h://127.0.0.1:9050'
}

headers = {
    'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'
}

def test_page_load(url):
    print(f"Testing page load: {url}")
    start = time.time()
    try:
        resp = requests.get(url, proxies=proxies, headers=headers, timeout=60, verify=False)
        end = time.time()
        print(f"[SUCCESS] Status: {resp.status_code}")
        print(f"[SUCCESS] Time: {end - start:.2f} seconds")
        print(f"[SUCCESS] Length: {len(resp.text)} chars")
    except Exception as e:
        print(f"[FAILED] Error: {e}")
    print("-" * 60)

def test_download(url):
    print(f"Testing download speed: {url}")
    start = time.time()
    try:
        resp = requests.get(url, proxies=proxies, headers=headers, stream=True, timeout=120, verify=False)
        size = 0
        for chunk in resp.iter_content(chunk_size=8192):
            if chunk:
                size += len(chunk)
        end = time.time()
        duration = end - start
        mb = size / 1024 / 1024
        print(f"[SUCCESS] Downloaded {mb:.2f} MB in {duration:.2f} seconds")
        if duration > 0:
            print(f"[SUCCESS] Average speed: {size / 1024 / duration:.2f} KB/s")
    except Exception as e:
        print(f"[FAILED] Error: {e}")
    print("-" * 60)

if __name__ == "__main__":
    import urllib3
    urllib3.disable_warnings()
    # Test clearnet site over Tor
    test_page_load("https://duckduckgo.com")
    
    # Test onion site over Tor
    test_page_load("http://duckduckgogg42xjoc72x3sjiqbvqdz2x5bgcocnmq3jc6pxndbd.onion/")
    
    # Test 10MB download over Tor
    test_download("https://proof.ovh.net/files/10Mb.dat")
