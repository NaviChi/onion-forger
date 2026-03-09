import time
import requests
import threading
import sys
import urllib3
import os

urllib3.disable_warnings()

# Proxies map to our loki-tor-core daemon
PROXIES = {
    'http': 'socks5h://127.0.0.1:9050',
    'https': 'socks5h://127.0.0.1:9050'
}

HEADERS = {
    'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'
}

def download_chunk(url, start_byte, end_byte, chunk_id, results, max_retries=5):
    """Downloads a specific byte range of a file over the Tor Proxy."""
    headers = HEADERS.copy()
    headers['Range'] = f'bytes={start_byte}-{end_byte}'
    
    for attempt in range(max_retries):
        print(f"[Thread-{chunk_id}] Requesting bytes {start_byte}-{end_byte} (Attempt {attempt+1}/{max_retries})...")
        start_time = time.time()
        
        try:
            # Stream the targeted range
            resp = requests.get(url, headers=headers, proxies=PROXIES, stream=True, timeout=120, verify=False)
            resp.raise_for_status()
            
            chunk_data = bytearray()
            for chunk in resp.iter_content(chunk_size=8192):
                if chunk:
                    chunk_data.extend(chunk)
                    
            expected_size = end_byte - start_byte + 1
            if len(chunk_data) != expected_size:
                raise Exception(f"IncompleteRead: Expected {expected_size} bytes but got {len(chunk_data)} bytes.")
                
            duration = time.time() - start_time
            mb_downloaded = len(chunk_data) / (1024 * 1024)
            print(f"[Thread-{chunk_id}] ✓ Downloaded {mb_downloaded:.2f} MB in {duration:.2f}s (Avg Speed: {mb_downloaded/duration:.2f} MB/s)")
            
            results[chunk_id] = chunk_data
            return # Success
            
        except Exception as e:
            print(f"[Thread-{chunk_id}] ⚠ Retry {attempt+1} Triggered: {e}")
            time.sleep(2)
            
    print(f"[Thread-{chunk_id}] ✗ Fatal error downloading chunk after {max_retries} retries.")
    results[chunk_id] = None

def get_file_size(url, max_retries=5):
    """Fetches the total file size from the server via HEAD request."""
    print(f"Fetching metadata for: {url}")
    
    for attempt in range(max_retries):
        try:
            resp = requests.head(url, headers=HEADERS, proxies=PROXIES, timeout=60, verify=False)
            resp.raise_for_status()
            
            if 'Accept-Ranges' not in resp.headers or resp.headers['Accept-Ranges'] != 'bytes':
                print("WARNING: Server does not appear to support byte ranges.")
                
            size = int(resp.headers.get('Content-Length', 0))
            print(f"Target File Size: {size / (1024 * 1024):.2f} MB")
            return size
        except Exception as e:
            print(f"⚠ Metadata fetch retry {attempt+1}/{max_retries} triggered: {e}")
            time.sleep(3)
            
    raise Exception("Failed to fetch metadata after maximum retries.")

def multipath_download(url, num_threads=4):
    print(f"=== Starting Multipath Download Benchmark over Tor ({num_threads} Threads) ===")
    total_start_time = time.time()
    
    try:
        file_size = get_file_size(url)
    except Exception as e:
        print(f"Failed to fetch metadata: {e}")
        return

    if file_size == 0:
        print("Cannot multipath download: Unknown file size.")
        return

    chunk_size = file_size // num_threads
    
    threads = []
    # Dict to securely store downloaded bytes index by chunk_id
    results = {}
    
    # Spawn concurrent requests to create multiple Tor circuits
    for i in range(num_threads):
        start_byte = i * chunk_size
        # The last chunk takes whatever remains
        end_byte = file_size - 1 if i == num_threads - 1 else (start_byte + chunk_size - 1)
        
        t = threading.Thread(target=download_chunk, args=(url, start_byte, end_byte, i, results))
        threads.append(t)
        t.start()

    # Wait for all Tor circuits to finish their chunk
    for t in threads:
        t.join()

    # Verify integrity
    if None in results.values() or len(results) != num_threads:
        print("Multipath download failed. One or more chunks dropped.")
        return

    # Sequentially write the ordered chunks to disk
    output_filename = "multipath_test_download.bin"
    print("Reassembling chunks into single binary...")
    with open(output_filename, 'wb') as f:
        for i in range(num_threads):
            f.write(results[i])
            
    total_duration = time.time() - total_start_time
    total_mb = file_size / (1024 * 1024)
    avg_speed = total_mb / total_duration
    
    print("=" * 60)
    print(f"Multipath Tor Download Complete!")
    print(f"Total Bytes Downloaded: {file_size}")
    print(f"Total Time: {total_duration:.2f} seconds")
    print(f"Aggregated Tor Speed: {avg_speed:.2f} MB/s")
    print("=" * 60)
    
    # Clean up temp file
    if os.path.exists(output_filename):
        os.remove(output_filename)

if __name__ == "__main__":
    # OVH 10MB test file
    target_url = "https://proof.ovh.net/files/10Mb.dat"
    multipath_download(target_url, num_threads=150)
