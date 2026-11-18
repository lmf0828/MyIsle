import os, socket, ssl, threading, datetime
from cryptography import x509
from cryptography.x509.oid import NameOID
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa

BASE = os.path.dirname(os.path.abspath(__file__))
CADIR = os.path.normpath(os.path.join(BASE, '..', 'data', 'mitm'))
os.makedirs(CADIR, exist_ok=True)
CA_KEY_P = os.path.join(CADIR, 'ca.key')
CA_CRT_P = os.path.join(CADIR, 'ca.crt')
LOG = os.path.join(CADIR, 'flows.log')
HOSTS = os.path.join(CADIR, 'hosts.log')
PORT = 8899


def loghost(host):
    with _lock:
        with open(HOSTS, 'a', encoding='utf-8', errors='ignore') as f:
            f.write('%s %s\n' % (datetime.datetime.now().strftime('%H:%M:%S'), host))


def load_ca():
    if os.path.exists(CA_KEY_P) and os.path.exists(CA_CRT_P):
        k = serialization.load_pem_private_key(open(CA_KEY_P, 'rb').read(), None)
        c = x509.load_pem_x509_certificate(open(CA_CRT_P, 'rb').read())
        return k, c
    k = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'WinIsle MITM CA')])
    now = datetime.datetime.utcnow()
    c = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
         .public_key(k.public_key()).serial_number(x509.random_serial_number())
         .not_valid_before(now - datetime.timedelta(days=1))
         .not_valid_after(now + datetime.timedelta(days=3650))
         .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
         .sign(k, hashes.SHA256()))
    open(CA_KEY_P, 'wb').write(k.private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.TraditionalOpenSSL, serialization.NoEncryption()))
    open(CA_CRT_P, 'wb').write(c.public_bytes(serialization.Encoding.PEM))
    return k, c


CA_KEY, CA_CRT = load_ca()
_leaf_cache = {}


def leaf(host):
    if host in _leaf_cache:
        return _leaf_cache[host]
    k = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    now = datetime.datetime.utcnow()
    cert = (x509.CertificateBuilder()
            .subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, host)]))
            .issuer_name(CA_CRT.subject).public_key(k.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(now - datetime.timedelta(days=1))
            .not_valid_after(now + datetime.timedelta(days=365))
            .add_extension(x509.SubjectAlternativeName([x509.DNSName(host)]), critical=False)
            .sign(CA_KEY, hashes.SHA256()))
    safe = host.replace(':', '_')
    cp = os.path.join(CADIR, 'leaf_' + safe + '.pem')
    kp = os.path.join(CADIR, 'leaf_' + safe + '.key')
    open(cp, 'wb').write(cert.public_bytes(serialization.Encoding.PEM))
    open(kp, 'wb').write(k.private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.TraditionalOpenSSL, serialization.NoEncryption()))
    _leaf_cache[host] = (cp, kp)
    return cp, kp


_lock = threading.Lock()


def note(host, text, direction):
    low = text.lower()
    if ('lyric' in low) or ('[00:' in text) or ('"content"' in low) or ('/api/' in low) or ('GET ' in text) or ('POST ' in text):
        with _lock:
            with open(LOG, 'a', encoding='utf-8', errors='ignore') as f:
                f.write('\n=== %s %s %s ===\n' % (datetime.datetime.now().isoformat(), host, direction))
                f.write(text[:30000])


def pump(src, dst, host, direction):
    buf = ''
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
            buf += data.decode('utf-8', 'ignore')
            if len(buf) > 40000:
                note(host, buf, direction)
                buf = buf[-4096:]
    except Exception as e:
        pass
    finally:
        try:
            note(host, buf, direction + '-tail')
        except Exception:
            pass
        for s in (src, dst):
            try:
                s.close()
            except Exception:
                pass


def handle(conn):
    try:
        conn.settimeout(30)
        head = b''
        while b'\r\n\r\n' not in head:
            d = conn.recv(4096)
            if not d:
                return
            head += d
            if len(head) > 65536:
                return
        line = head.split(b'\r\n', 1)[0].decode('latin1')
        parts = line.split(' ')
        if len(parts) < 3 or parts[0].upper() != 'CONNECT':
            conn.sendall(b'HTTP/1.1 400 Bad Request\r\n\r\n')
            conn.close()
            return
        host, _, port = parts[1].partition(':')
        port = int(port or 443)
        loghost(host)
        cp, kp = leaf(host)
        conn.sendall(b'HTTP/1.1 200 Connection Established\r\n\r\n')
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cp, kp)
        try:
            ctx.set_alpn_protocols(['http/1.1'])
        except Exception:
            pass
        cli = ctx.wrap_socket(conn, server_side=True)
        up = socket.create_connection((host, port), timeout=15)
        uctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        uctx.check_hostname = False
        uctx.verify_mode = ssl.CERT_NONE
        try:
            uctx.set_alpn_protocols(['http/1.1'])
        except Exception:
            pass
        srv = uctx.wrap_socket(up, server_hostname=host)
        t1 = threading.Thread(target=pump, args=(cli, srv, host, 'C>S'), daemon=True)
        t2 = threading.Thread(target=pump, args=(srv, cli, host, 'S>C'), daemon=True)
        t1.start(); t2.start()
        t1.join(); t2.join()
    except Exception as e:
        try:
            conn.close()
        except Exception:
            pass


def main():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(('127.0.0.1', PORT))
    s.listen(100)
    print('mitm listening on %d, log=%s' % (PORT, LOG), flush=True)
    while True:
        c, _ = s.accept()
        threading.Thread(target=handle, args=(c,), daemon=True).start()


if __name__ == '__main__':
    main()
