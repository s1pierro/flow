#!/data/data/com.termux/files/usr/bin/bash
# construire.sh <dossier du module> [nom] — compile un module du banc (une crate
# cdylib, dans Termux : le Rust de Termux compile pour Android) et le dépose dans
# Flow/banc/modules/<nom>.so, où le contexte Banc le charge (banc charger <nom>).
set -euo pipefail
src="${1:?usage : construire.sh <dossier du module> [nom]}"
cd "$src"
crate="$(sed -n 's/^name *= *"\(.*\)"/\1/p' Cargo.toml | head -n1)"
nom="${2:-${crate#banc-}}"
cible="${CARGO_TARGET_DIR:-$HOME/.cache/rust-apk/banc}"
RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-Bsymbolic" CARGO_TARGET_DIR="$cible" cargo build --release
so="$cible/release/lib${crate//-/_}.so"
dest=/sdcard/Flow/banc/modules
mkdir -p "$dest"
cp "$so" "$dest/$nom.so"
printf '\033[1;36m▸ module prêt : %s (%s)\033[0m\n' "$dest/$nom.so" "$(du -h "$dest/$nom.so" | cut -f1)"
