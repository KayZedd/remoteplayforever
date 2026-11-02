#!/usr/bin/env bash
# Regenerates docs/img/{launch,steam-games,guide,settings}[-pl].png from the real app, on Linux.
#
# Needs: wine, Xvfb, xdotool, ImageMagick 7 (magick), cargo-xwin, Mesa lavapipe (lvp_icd.json).
# The app runs under wine on a private Xvfb display with a throwaway prefix in target/screenshots/,
# a fake Steam library (empty .exe stubs + appmanifests) and a registry entry that makes the
# status badge read "Steam is running". No real Steam is involved, so there is no session card.
#
# Usage: docs/screenshots/capture.sh            (builds the exe with cargo xwin first)
#        EXE=path/to/remoteplayforever.exe docs/screenshots/capture.sh
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=$ROOT/target/screenshots
OUT=$ROOT/docs/img
DISP=:${XVFB_DISPLAY:-97}
mkdir -p "$WORK/raw" "$OUT"

if [ -z "${EXE:-}" ]; then
  (cd "$ROOT" && cargo xwin build --release --target x86_64-pc-windows-msvc)
  EXE=$ROOT/target/x86_64-pc-windows-msvc/release/remoteplayforever.exe
fi

# wgpu picks DX12 under wine, whose shader compiler fails on egui; force Vulkan on lavapipe.
export DISPLAY=$DISP WINEPREFIX=$WORK/wineprefix WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml=" \
  WGPU_BACKEND=vulkan VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json LIBGL_ALWAYS_SOFTWARE=1

Xvfb "$DISP" -screen 0 1400x1000x24 >/dev/null 2>&1 &
XVFB=$!
trap 'wineserver -k 2>/dev/null || true; kill $XVFB 2>/dev/null || true' EXIT
sleep 2

[ -d "$WINEPREFIX/drive_c" ] || wineboot -i >/dev/null 2>&1
C=$WINEPREFIX/drive_c
ST="$C/Program Files (x86)/Steam"
mkdir -p "$ST/steamapps/common"
while IFS='|' read -r id name dir exe; do
  mkdir -p "$ST/steamapps/common/$dir"
  printf 'MZ' > "$ST/steamapps/common/$dir/$exe"
  printf '"AppState"\n{\n\t"appid"\t\t"%s"\n\t"name"\t\t"%s"\n\t"installdir"\t\t"%s"\n}\n' \
    "$id" "$name" "$dir" > "$ST/steamapps/appmanifest_$id.acf"
done <<'L'
413150|Stardew Valley|Stardew Valley|Stardew Valley.exe
105600|Terraria|Terraria|Terraria.exe
367520|Hollow Knight|Hollow Knight|hollow_knight.exe
268910|Cuphead|Cuphead|Cuphead.exe
504230|Celeste|Celeste|Celeste.exe
1145360|Hades|Hades|Hades.exe
646570|Slay the Spire|SlayTheSpire|SlayTheSpire.exe
1794680|Vampire Survivors|Vampire Survivors|VampireSurvivors.exe
L
mkdir -p "$C/Games/Ryujinx" "$C/Games/RetroArch" "$C/Games/GOG/Hotline Miami"
printf MZ > "$C/Games/Ryujinx/Ryujinx.exe"
printf MZ > "$C/Games/RetroArch/retroarch.exe"
printf MZ > "$C/Games/GOG/Hotline Miami/HotlineMiami.exe"
wine reg add 'HKCU\Software\Valve\Steam' /v SteamPath /t REG_SZ /d 'C:/Program Files (x86)/Steam' /f >/dev/null 2>&1
wine reg add 'HKCU\Software\Valve\Steam\ActiveProcess' /v ActiveUser /t REG_DWORD /d 123456 /f >/dev/null 2>&1

CFG_DIR=$C/users/$USER/AppData/Roaming/RemotePlayForever
mkdir -p "$CFG_DIR"

# capture <Language> <suffix> <x of each of the 4 tabs, relative to the window>
capture() {
  cat > "$CFG_DIR/config.json" <<J
{"language":"$1","guest_controller_mode":"Auto","guest_keyboard":true,"guest_mouse":true,"copy_link_automatically":true,
 "recent_games":[
  {"name":"Stardew Valley","path":"C:\\\\Program Files (x86)\\\\Steam\\\\steamapps\\\\common\\\\Stardew Valley\\\\Stardew Valley.exe","args":[],"last_played":"2026-11-02 20:31"},
  {"name":"Ryujinx","path":"C:\\\\Games\\\\Ryujinx\\\\Ryujinx.exe","args":[],"last_played":"2026-11-01 21:14"},
  {"name":"retroarch","path":"C:\\\\Games\\\\RetroArch\\\\retroarch.exe","args":[],"last_played":"2026-10-30 19:02"},
  {"name":"HotlineMiami","path":"C:\\\\Games\\\\GOG\\\\Hotline Miami\\\\HotlineMiami.exe","args":[],"last_played":"2026-10-27 22:40"}]}
J
  wineserver -k 2>/dev/null || true
  wine "$EXE" >"$WORK/wine-$1.log" 2>&1 &
  local w=""
  for _ in $(seq 1 60); do w=$(xdotool search --name '^RemotePlayForever$' 2>/dev/null | head -1) && [ -n "$w" ] && break; sleep 1; done
  [ -n "$w" ] || { echo "window did not appear, see $WORK/wine-$1.log" >&2; exit 1; }
  sleep 4
  eval "$(xdotool getwindowgeometry --shell "$w")"
  local suffix=$2; shift 2
  local tab
  for tab in launch steam-games guide settings; do
    xdotool mousemove $((X + $1)) $((Y + 88)) click 1; shift
    sleep 1.5; xdotool mousemove $((X + 700)) $((Y + 700)); sleep 0.8
    import -window "$w" "$WORK/raw/$tab$suffix.png"
  done
}

capture English "" 53 170 282 374
capture Polish -pl 60 213 330 438

for f in "$WORK"/raw/*.png; do
  name=$(basename "$f")
  case $name in
    # The Data card at the bottom shows the local user name in paths: cut it off.
    settings*) magick "$f" -crop 820x418+0+0 +repage -strip "$OUT/$name" ;;
    *) magick "$f" -strip "$OUT/$name" ;;
  esac
  echo "docs/img/$name"
done
