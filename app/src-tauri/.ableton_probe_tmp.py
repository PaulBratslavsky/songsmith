import socket, json, sys

def probe(cmd, params=None, timeout=4.0):
    try:
        s = socket.create_connection(("127.0.0.1", 9877), timeout=timeout)
        s.settimeout(timeout)
        s.sendall(json.dumps({"type": cmd, "params": params or {}}).encode())
        chunks = b""
        while True:
            try:
                d = s.recv(65536)
            except socket.timeout:
                break
            if not d:
                break
            chunks += d
            try:
                json.loads(chunks.decode())
                break
            except Exception:
                continue
        s.close()
        return chunks.decode() if chunks else "<no response (busy?)>"
    except Exception as e:
        return f"<connect error: {e}>"

if __name__ == "__main__":
    cmd = sys.argv[1]
    params = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
    out = probe(cmd, params)
    print(out[:2000])
