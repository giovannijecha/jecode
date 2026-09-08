"""Record the Codex completion notification locally without prompt content."""
import json
from pathlib import Path
import sys
import time

event = json.loads(sys.argv[2])
if event.get('type') == 'agent-turn-complete':
    destination = Path(sys.argv[1])
    temporary = destination.with_suffix('.tmp')
    temporary.write_text(json.dumps({
        'type': event['type'], 'atNs': time.time_ns(),
        'monotonicNs': time.monotonic_ns(),
        'threadId': event.get('thread-id'), 'turnId': event.get('turn-id'),
    })+'\n')
    temporary.replace(destination)
