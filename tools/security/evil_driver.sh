#!/bin/bash
# usage: evil_driver.sh MODE [run seconds] [text]
M=$1; R=${2:-8}; TEXT=${3:-"Hello there. How are you today? I am fine."}
HERE=$(cd "$(dirname "$0")" && pwd); RUN=${RUN:?set RUN to a scratch directory holding the server and w.sock}; cd "$RUN" || exit
echo "=== $M" >> srv.log
python3 -I "$HERE/evilworker.py" "$RUN/w.sock" "$M" "$R" 2> "evil_$M.log" &
EP=$!
sleep 1.2
H=$(curl -s -m 3 -o /dev/null -w '%{http_code}' localhost:18881/health)
S=$(curl -s -m 12 -o "$RUN/out_$M.pcm" -w '%{http_code} %{size_download} %{time_total}' -H 'Authorization: Bearer k' -d "{\"input\":\"$TEXT\",\"response_format\":\"pcm\"}" localhost:18881/v1/audio/speech)
B=$(head -c 300 "$RUN/out_$M.pcm" | tr -dc '[:print:]' | head -c 160)
wait "$EP"
H2=$(curl -s -m 3 -o /dev/null -w '%{http_code}' localhost:18881/health)
ALIVE=$(curl -s -m 2 -o /dev/null -w "%{http_code}" localhost:18881/health)
printf '%-28s health=%s speech=[%s] health_after=%s alive=%s %s\n' "$M" "$H" "$S" "$H2" "$ALIVE" "$B"
