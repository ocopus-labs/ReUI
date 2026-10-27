#!/bin/sh
# Usage: shoot.sh name "query" [w] [h]
EDGE="/c/Program Files (x86)/Microsoft/EdgeCore/155.0.4283.62/msedge.exe"
DIR="$(cd "$(dirname "$0")" && pwd -W)"
W=${3:-760}; H=${4:-430}
"$EDGE" --headless=new --user-data-dir="$TEMP/reui-shot-profile" --disable-gpu --hide-scrollbars --force-device-scale-factor=${DSF:-1} --virtual-time-budget=2500 --window-size=$W,$H --screenshot="$DIR/$1.png" "file:///D:/ReUI/crates/reui-island/web/clipboard/dev/mock.html?still=1&$2" 2>/dev/null
