Onion Forger Windows Portable
=============================

Files in this folder:
- crawli.exe      : GUI application
- crawli-cli.exe  : Console CLI application
- crawli-cli.cmd  : Convenience wrapper for terminal use

Use the GUI:
  Double-click crawli.exe

Use the CLI from PowerShell:
  .\crawli-cli.exe --help
  .\crawli-cli.exe detect-input-mode --input "https://proof.ovh.net/files/10Gb.dat"

Use the CLI from cmd.exe:
  crawli-cli.exe --help

Direct file download example:
  .\crawli-cli.exe --progress-summary --progress-summary-interval-ms 5000 initiate-download --url "https://proof.ovh.net/files/10Gb.dat" --path "10Gb.dat" --output-root "D:\Crawli\downloads" --connections 240

Site crawl example:
  .\crawli-cli.exe --progress-summary --progress-summary-interval-ms 5000 crawl --url "https://example.com/" --output-dir "D:\Crawli\crawl-output" --daemons 12 --circuits 240
