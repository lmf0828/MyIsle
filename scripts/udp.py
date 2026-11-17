import sys, collections
from scapy.utils import PcapNgReader
from scapy.layers.inet import IP, UDP
from scapy.layers.inet6 import IPv6
P=r'D:\LingDao\data\cap\cap.pcapng'
b=collections.Counter(); pk=collections.Counter()
for pkt in PcapNgReader(P):
    if IP in pkt: ip=pkt[IP]
    elif IPv6 in pkt: ip=pkt[IPv6]
    else: continue
    if UDP in pkt:
        u=pkt[UDP]
        if u.dport==443 or u.sport==443:
            k='%s:%d'%(ip.dst,u.dport)
            b[k]+=len(bytes(u.payload)); pk[k]+=1
print('UDP:443 flows:')
for k,v in b.most_common(20): print('  %-28s %9d B %5d pkts'%(k,v,pk[k]))
