import sys, collections
from scapy.utils import PcapNgReader, PcapReader
from scapy.layers.l2 import Ether
from scapy.layers.inet import IP, TCP
from scapy.layers.inet6 import IPv6

PCAP = sys.argv[1] if len(sys.argv) > 1 else r'D:\LingDao\data\cap\cap.pcapng'


def sni_from_client_hello(data):
    try:
        if len(data) < 6 or data[0] != 0x16 or data[1] != 0x03:
            return None
        # TLS record header: type(1) ver(2) len(2)
        rec_len = int.from_bytes(data[3:5], 'big')
        body = data[5:5 + rec_len]
        if not body or body[0] != 0x01:
            return None
        hs_len = int.from_bytes(body[1:4], 'big')
        hs = body[4:4 + hs_len]
        i = 0
        i += 2  # version
        i += 32  # random
        sid_len = hs[i]; i += 1 + sid_len
        cs_len = int.from_bytes(hs[i:i + 2], 'big'); i += 2 + cs_len
        comp_len = hs[i]; i += 1 + comp_len
        if i + 2 > len(hs):
            return None
        ext_total = int.from_bytes(hs[i:i + 2], 'big'); i += 2
        end = min(len(hs), i + ext_total)
        while i + 4 <= end:
            etype = int.from_bytes(hs[i:i + 2], 'big')
            elen = int.from_bytes(hs[i + 2:i + 4], 'big')
            edata = hs[i + 4:i + 4 + elen]
            if etype == 0:  # server_name
                p = 0
                if len(edata) < 2:
                    return None
                list_len = int.from_bytes(edata[0:2], 'big')
                p = 2
                while p + 3 <= len(edata):
                    ntype = edata[p]
                    nlen = int.from_bytes(edata[p + 1:p + 3], 'big')
                    name = edata[p + 3:p + 3 + nlen]
                    if ntype == 0:
                        return name.decode('idna', 'ignore') if False else name.decode('latin1')
                    p += 3 + nlen
                return None
            i += 4 + elen
    except Exception:
        return None
    return None


stream_host = {}          # (src, sport, dst, dport) -> host
bytes_by_host = collections.Counter()
pkts_by_host = collections.Counter()
reader = PcapNgReader(PCAP) if PCAP.lower().endswith('pcapng') else PcapReader(PCAP)
n = 0
for pkt in reader:
    if IP in pkt:
        ip = pkt[IP]
        sp, dp = ip.src, ip.dst
    elif IPv6 in pkt:
        ip = pkt[IPv6]
        sp, dp = ip.src, ip.dst
    else:
        continue
    if TCP not in pkt:
        continue
    t = pkt[TCP]
    key = (sp, t.sport, dp, t.dport)
    raw = bytes(t.payload)
    if raw:
        h = sni_from_client_hello(raw)
        if h:
            stream_host[key] = h
    rev = (dp, t.dport, sp, t.sport)
    host = stream_host.get(key) or stream_host.get(rev)
    label = host or ('%s:%d' % (dp, t.dport))
    if pkt.haslayer('Raw'):
        bytes_by_host[label] += len(raw)
        pkts_by_host[label] += 1
    n += 1

print('packets scanned:', n)
print('hosts seen (by bytes):')
for host, b in bytes_by_host.most_common(40):
    print('  %-45s %10d bytes  %6d pkts' % (host, b, pkts_by_host[host]))
