#!/bin/bash

echo "=== Raw info output ==="
cos-cli info

echo ""
echo "=== JSON workspace data ==="
cos-cli info --json | python3 -c "
import sys, json
d = json.load(sys.stdin)

print('Workspaces:', json.dumps(d.get('workspaces', []), indent=2))
print()

for a in d['apps']:
    ws = a.get('workspaces', [])
    print(f'  [{a[\"index\"]}] {a[\"app_id\"]:30s} ws={ws}  state={a[\"state\"]}')
"
