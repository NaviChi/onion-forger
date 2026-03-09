import time
import requests
from bs4 import BeautifulSoup
import urllib.request
import certifi

def test_proxy(url):
    print(f"Testing connection to: {url}")
    proxies = {
        'http': 'socks5h://127.0.0.1:9050',
        'https': 'socks5h://127.0.0.1:9050'
    }
    
    start_time = time.time()
    try:
        headers = {
            'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
        }
        # Use verify=False for onion sites if there are cert issues, or just normal request
        response = requests.get(url, proxies=proxies, headers=headers, timeout=120, verify=False)
        end_time = time.time()
        
        print(f"[SUCCESS] Status code: {response.status_code}")
        print(f"[TIME] Loading took: {end_time - start_time:.2f} seconds")
        
        # Test BeautifulSoup parsing
        soup = BeautifulSoup(response.text, 'html.parser')
        title = soup.title.string if soup.title else "No Title Found"
        print(f"[CONTENT] Page Title: '{title}'")
        print(f"[CONTENT] Length of HTML parsed: {len(response.text)} chars")
                
    except Exception as e:
        end_time = time.time()
        print(f"[FAILED] Error after {end_time - start_time:.2f} seconds: {e}")
    print("-" * 60)

if __name__ == "__main__":
    import urllib3
    urllib3.disable_warnings()

    # DuckDuckGo Onion
    test_proxy("https://duckduckgogg42xjoc72x3sjiqbvqdz2x5bgcocnmq3jc6pxndbd.onion/")
    
    # ProPublica Onion
    test_proxy("http://p53lf57qovyuvwsc6xnrppyply3vtqm7l6pcobkmyqsiofyeznfu5uqd.onion/")
