#!/bin/bash

# Hata olursa scripti durdur
set -e

echo "🦀 Focus derleniyor (Release mod)..."
cargo build --release

echo "🛑 Servis durduruluyor..."
# Eğer servis zaten yoksa veya çalışmıyorsa hata vermemesi için || true ekliyoruz
sudo systemctl stop focus.service || true

echo "🚀 Binary sistem yoluna taşınıyor..."
# Binary dosyasını kopyala
sudo cp target/release/focus /usr/local/bin/

echo "📂 Config klasörü kontrol ediliyor..."
# Config klasörü yoksa oluştur
if [ ! -d "/etc/focus" ]; then
    sudo mkdir -p /etc/focus
    # Config dosyası yoksa boş bir JSON oluştur ki hata vermesin
    if [ ! -f "/etc/focus/config.json" ]; then
        echo '{"rules": []}' | sudo tee /etc/focus/config.json > /dev/null
    fi
fi

echo "⚙️ Systemd servisi oluşturuluyor/güncelleniyor..."
# Systemd dosyasını doğrudan oluştur
sudo bash -c 'cat > /etc/systemd/system/focus.service <<EOF
[Unit]
Description=Focus Website Blocker Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/focus daemon
Restart=always
User=root

[Install]
WantedBy=multi-user.target
EOF'

echo "🔄 Servis yeniden başlatılıyor..."
sudo systemctl daemon-reload
sudo systemctl enable focus.service
sudo systemctl restart focus.service

echo "✅ Kurulum tamamlandı! Focus arkaplanda çalışıyor."
echo "   Test: sudo focus list"
