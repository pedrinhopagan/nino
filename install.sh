#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

missing=()
command -v gpu-screen-recorder >/dev/null || missing+=(gpu-screen-recorder)
pkg-config --exists gtk-layer-shell-0 || missing+=(gtk-layer-shell)
pkg-config --exists webkit2gtk-4.1 || missing+=(webkit2gtk-4.1)
if ((${#missing[@]})); then
    echo "Faltam pacotes. Instale antes: sudo pacman -S ${missing[*]}" >&2
    exit 1
fi
if ! command -v bun >/dev/null; then
    echo "bun não encontrado. Instale antes: https://bun.sh" >&2
    exit 1
fi

(cd editor && bun install --frozen-lockfile && bun run build)
cargo build --release

bin="$HOME/.local/bin/nino"
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor/scalable/apps"
units="$HOME/.config/systemd/user"

install -Dm755 target/release/nino "$bin"
install -Dm644 assets/icon.svg "$icons/nino.svg"
install -Dm644 packaging/nino.service "$units/nino.service"
mkdir -p "$apps"
cat > "$apps/nino.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Nino
Comment=Print, gravação e anotações para IA
Exec=$bin
Icon=nino
NoDisplay=true
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
DESKTOP

kbuildsycoca6 >/dev/null 2>&1 || true
systemctl --user daemon-reload
systemctl --user enable nino.service
systemctl --user restart nino.service

echo "nino instalado. Win+R grava, Win+N anota, Win+Shift+S tira print."
