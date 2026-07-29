#!/usr/bin/env python3
import http.server
import pathlib
import socket
import socketserver
import sys
import threading

state = pathlib.Path(sys.argv[1])
webhooks = pathlib.Path(sys.argv[2])

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            code = 503 if state.exists() else 200
            self.send_response(code)
            self.end_headers()
            self.wfile.write(b"ok" if code == 200 else b"failed")
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length)
        with webhooks.open("ab") as output:
            output.write(body + b"\n")
        self.send_response(204)
        self.end_headers()

    def log_message(self, *_args):
        return


def tcp_server():
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 18100))
    server.listen()
    while True:
        connection, _ = server.accept()
        connection.close()


def dns_response(query):
    end = 12
    while end < len(query) and query[end] != 0:
        end += query[end] + 1
    end = min(end + 5, len(query))
    response = bytearray(query[:end])
    response[2:4] = b"\x81\x80"
    response[6:12] = b"\x00\x01\x00\x00\x00\x00"
    response.extend(b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04\x7f\x00\x00\x01")
    return bytes(response)


def udp_dns():
    server = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    server.bind(("127.0.0.1", 18102))
    while True:
        query, peer = server.recvfrom(2048)
        server.sendto(dns_response(query), peer)


def tcp_dns():
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 18102))
    server.listen()
    while True:
        connection, _ = server.accept()
        with connection:
            length = int.from_bytes(connection.recv(2), "big")
            query = b""
            while len(query) < length:
                query += connection.recv(length - len(query))
            response = dns_response(query)
            connection.sendall(len(response).to_bytes(2, "big") + response)


class ReusableHttpServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True


for target in (tcp_server, udp_dns, tcp_dns):
    threading.Thread(target=target, daemon=True).start()
ReusableHttpServer(("127.0.0.1", 18104), Handler).serve_forever()
