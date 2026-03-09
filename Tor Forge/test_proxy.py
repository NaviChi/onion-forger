import time
import requests
from bs4 import BeautifulSoup

def test_proxy(url):
    print(f"Testing connection to: {url}")
    proxies = {
        'http': 'socks5h://127.0.0.1:9050',
        'https': 'socks5h://127.0.0.1:9050'
    }
    
    start_time = time.time()
    try:
        # Use a real user-agent to look legit
        headers = {
            'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
        }
        response = requests.get(url, proxies=proxies, headers=headers, timeout=60)
        end_time = time.time()
        
        print(f"[SUCCESS] Status code: {response.status_code}")
        print(f"[TIME] Loading took: {end_time - start_time:.2f} seconds")
        
        # Test BeautifulSoup parsing
        soup = BeautifulSoup(response.text, 'html.parser')
        title = soup.title.string if soup.title else "No Title Found"
        print(f"[CONTENT] Page Title: '{title}'")
        print(f"[CONTENT] Length of HTML parsed: {len(response.text)} chars")
        
        # For check.torproject.org let's also grab the IP it says we have
        if "check.tor" in url:
            strong_tag = soup.find('strong')
            if strong_tag:
                print(f"[TOR CHECK] Public IP detected: {strong_tag.text.strip()}")
                
    except Exception as e:
        end_time = time.time()
        print(f"[FAILED] Error after {end_time - start_time:.2f} seconds: {e}")
    print("-" * 60)

if __name__ == "__main__":
    # Test standard clearnet via Tor
    test_proxy("https://check.torproject.org/")
    
    # Test provided Onion blog (Port 80 HTTP)
    test_proxy("http://incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion/blog/disclosures/697943738f1d14b743d6869e")
