#!/bin/sh
# Install on the CUPS server, as root, from a directory holding the built
# p1505-duplex binary and dist/. Idempotent; keeps an existing config.
set -e
cd "$(dirname "$0")"
BIN=${BIN:-p1505-duplex}

install -m755 "$BIN" /usr/local/bin/p1505-duplex
# A real file, not a symlink: Ubuntu's AppArmor profile for cupsd only lets
# it execute backends under /usr/lib/cups/backend.
rm -f /usr/lib/cups/backend/duplex
install -m755 "$BIN" /usr/lib/cups/backend/duplex
install -m644 dist/local.convs /etc/cups/local.convs
[ -e /etc/p1505-duplex.conf ] || install -m644 dist/p1505-duplex.conf /etc/p1505-duplex.conf
install -m644 dist/p1505-duplex-web.service dist/p1505-firmware.service /etc/systemd/system/
install -m644 dist/57-p1505-firmware.rules /etc/udev/rules.d/

id p1505-duplex >/dev/null 2>&1 ||
    useradd --system --no-create-home --shell /usr/sbin/nologin -g lpadmin p1505-duplex
install -d -m2775 -o lp -g lpadmin /var/spool/p1505-duplex

lpstat -p P1505_Duplex >/dev/null 2>&1 ||
    lpadmin -p P1505_Duplex -E -v duplex:/ \
        -P /usr/share/ppd/cupsfilters/Generic-PDF_Printer-PDF.ppd \
        -D "P1505 手動雙面" -o printer-is-shared=true -o PageSize=A4

systemctl daemon-reload
systemctl enable p1505-duplex-web
systemctl restart cups p1505-duplex-web
udevadm control --reload
echo "installed; put dist/nginx.conf (or similar) in front of 127.0.0.1:8631"
