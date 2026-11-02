#!/usr/bin/env bash
# Renders docs/social-preview.html to docs/img/social-preview.png (1280x640) with headless Chrome.
# Upload the PNG in GitHub: Settings -> General -> Social preview.
set -euo pipefail
cd "$(dirname "$0")"
chrome=$(command -v google-chrome-stable || command -v google-chrome || command -v chromium)
"$chrome" --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files \
  --virtual-time-budget=5000 --window-size=1280,640 \
  --screenshot="$PWD/img/social-preview.png" "file://$PWD/social-preview.html" 2>/dev/null
echo "docs/img/social-preview.png"
