#!/bin/bash
# Hata olsa bile devam et
set +e

# IPv4 Temizliği
sudo iptables -w -D OUTPUT -p tcp --dport 443 -j REJECT --reject-with tcp-reset 2>/dev/null
sudo iptables -w -D OUTPUT -p udp --dport 443 -j REJECT --reject-with icmp-port-unreachable 2>/dev/null
sudo iptables -w -D OUTPUT -p tcp --dport 80 -j REJECT --reject-with tcp-reset 2>/dev/null
sudo iptables -w -D INPUT -p tcp --sport 443 -j REJECT --reject-with tcp-reset 2>/dev/null
sudo iptables -w -D INPUT -p udp --sport 443 -j REJECT --reject-with icmp-port-unreachable 2>/dev/null
sudo iptables -w -D INPUT -p tcp --sport 80 -j REJECT --reject-with tcp-reset 2>/dev/null

# IPv6 Temizliği
sudo ip6tables -w -D OUTPUT -p tcp --dport 443 -j REJECT --reject-with tcp-reset 2>/dev/null
sudo ip6tables -w -D OUTPUT -p udp --dport 443 -j REJECT --reject-with icmp6-port-unreachable 2>/dev/null
sudo ip6tables -w -D OUTPUT -p tcp --dport 80 -j REJECT --reject-with tcp-reset 2>/dev/null
sudo ip6tables -w -D INPUT -p tcp --sport 443 -j REJECT --reject-with tcp-reset 2>/dev/null
sudo ip6tables -w -D INPUT -p udp --sport 443 -j REJECT --reject-with icmp6-port-unreachable 2>/dev/null
sudo ip6tables -w -D INPUT -p tcp --sport 80 -j REJECT --reject-with tcp-reset 2>/dev/null
