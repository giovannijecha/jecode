"""Loopback OTLP JSON collector that retains only scalar timing/usage metadata."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import re
import threading
import time

FIELDS = {
    'event.name', 'event_name', 'model', 'conversation.id', 'conversation_id',
    'kind', 'event_kind', 'duration_ms', 'duration', 'success', 'status', 'status_code',
    'attempt', 'tool', 'tool_name', 'call_id', 'decision', 'source',
    'input_tokens', 'output_tokens', 'cached_tokens', 'cached_input_tokens',
    'reasoning_tokens', 'reasoning_output_tokens', 'total_tokens', 'auth_mode',
    'reasoning_effort', 'service_tier', 'terminal.type', 'app.version',
    'event.timestamp', 'event.kind', 'input_token_count', 'output_token_count',
    'cached_token_count', 'reasoning_token_count', 'total_token_count',
    'http.response.status_code', 'auth.connection_reused',
}


class Collector:
    def __init__(self, file):
        self.file = file
        self.records = 0
        self.errors = 0
        self.lock = threading.Lock()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass

            def do_POST(self):
                try: length = int(self.headers.get('Content-Length', '0'))
                except ValueError: length = -1
                if length < 0:
                    owner.errors += 1
                    self.send_error(400)
                    return
                if length > 4*1024*1024:
                    owner.errors += 1
                    self.send_error(413)
                    return
                try:
                    data = json.loads(self.rfile.read(length))
                    rows = []
                    for resource in data.get('resourceLogs', []):
                        for scope in resource.get('scopeLogs', []):
                            for log in scope.get('logRecords', []):
                                fields = {}
                                for attr in log.get('attributes', []):
                                    if attr['key'] in FIELDS:
                                        value = attr.get('value', {})
                                        for scalar in ('stringValue','intValue','doubleValue','boolValue'):
                                            if scalar in value:
                                                fields[attr['key']] = value[scalar]
                                body = (log.get('body') or {}).get('stringValue','')
                                if re.fullmatch(r'codex\.[a-z0-9_.]{1,80}', body):
                                    fields['event'] = body
                                rows.append({'atNs':log.get('timeUnixNano'), 'receivedNs':time.time_ns(),
                                             'fields':fields, 'attributeKeys':[a['key'] for a in log.get('attributes',[])]})
                    with owner.lock, owner.file.open('a') as handle:
                        for row in rows: handle.write(json.dumps(row)+'\n')
                        owner.records += len(rows)
                    self.send_response(200)
                    self.send_header('Content-Type','application/json')
                    self.end_headers()
                    self.wfile.write(b'{}')
                except (ValueError,KeyError,TypeError,AttributeError):
                    owner.errors += 1
                    self.send_error(400)
        self.server = ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.thread = threading.Thread(target=self.server.serve_forever,daemon=True)
        self.thread.start()

    @property
    def endpoint(self): return f'http://127.0.0.1:{self.server.server_port}/v1/logs'

    def close(self):
        self.server.shutdown(); self.server.server_close(); self.thread.join()
