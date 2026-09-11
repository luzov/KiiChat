"""Throwaway OpenAI-compatible mock used to verify KiiChat end to end.

GET  /v1/models            -> two models
POST /v1/chat/completions  -> SSE stream of markdown chunks, then [DONE]
"""

import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 18080
CHUNKS = ["你好", "，这是 ", "**mock**", " 回复。\n\n", "```py\nprint(1)\n```"]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        print("%s - %s" % (self.address_string(), fmt % args), flush=True)

    def do_GET(self):
        if self.path.rstrip("/").endswith("/models"):
            body = json.dumps(
                {"object": "list", "data": [{"id": "mock-mini"}, {"id": "mock-large"}, {"id": "mock-new"}]}
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        payload = json.loads(self.rfile.read(length) or b"{}")
        print("request model=%s messages=%d" % (payload.get("model"), len(payload.get("messages", []))), flush=True)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for chunk in CHUNKS:
            event = json.dumps({"choices": [{"delta": {"content": chunk}}]})
            self.wfile.write(("data: " + event + "\n\n").encode())
            self.wfile.flush()
            time.sleep(0.15)
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()