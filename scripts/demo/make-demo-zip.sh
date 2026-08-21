#!/usr/bin/env bash
# Build a synthetic WhatsApp chat-export zip for the README demo.
#
# The voice notes are TTS-generated (piper if available, espeak-ng otherwise),
# so the zip contains no real conversation and is safe to show publicly.
# Nothing here is committed: the script is the artifact, the zip is disposable.
#
# Usage: make-demo-zip.sh [output.zip]   (default: ./demo-export.zip)
set -euo pipefail

OUT_ZIP="$(realpath -m "${1:-demo-export.zip}")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# PIPER_VOICE may point at a .onnx voice model (e.g. pt_BR-faber-medium.onnx).
tts() { # tts <text> <out.wav>
    if command -v piper >/dev/null && [[ -n "${PIPER_VOICE:-}" ]]; then
        # slightly slower speech transcribes more cleanly
        echo "$1" | piper -m "$PIPER_VOICE" -f "$2" --length-scale 1.3 >/dev/null 2>&1
    else
        espeak-ng -v pt-br -s 150 -w "$2" "$1"
    fi
}

# A coworker explaining one task in five voice notes — the scenario the README
# pitch describes. Note 3 corrects note 1: in audio you only find that out at
# the end, which is the whole point of having the text.
NOTES=(
    "Preciso que você mexa no relatório de fechamento do mês."
    "O problema é que o total de horas sai duplicado quando a semana cai virando o mês."
    "Corrigindo o que eu falei: o erro não está na tela. É a função que junta os turnos."
    "A prioridade é média, mas não pode passar de sexta. O financeiro fecha no sábado."
    "Faz um teste para o mês de fevereiro. Foi lá que o erro apareceu primeiro."
)

# Attachment numbers are chronological; 00000003 is the (absent) photo.
ATTACH_NUMS=(1 2 4 5 6)

for i in "${!NOTES[@]}"; do
    tts "${NOTES[$i]}" "$WORK/note$i.wav"
    # WhatsApp-style PTT: opus, 16 kHz mono, low bitrate, 8-digit attachment name
    ffmpeg -loglevel error -y -i "$WORK/note$i.wav" \
        -c:a libopus -b:a 24k -ar 16000 -ac 1 \
        "$WORK/$(printf '%08d' "${ATTACH_NUMS[$i]}")-AUDIO.opus"
done

# iOS export format: [DD/MM/YYYY, HH:MM:SS] Sender: body
cat > "$WORK/_chat.txt" <<'EOF'
[10/08/2026, 09:12:10] Ana: As mensagens e as chamadas são protegidas com a criptografia de ponta a ponta.
[10/08/2026, 09:12:31] Ana: Bruno, te explico a task por áudio 👇
[10/08/2026, 09:12:58] Ana: <anexado: 00000001-AUDIO.opus>
[10/08/2026, 09:14:10] Ana: <anexado: 00000002-AUDIO.opus>
[10/08/2026, 09:15:02] Ana: <anexado: 00000003-PHOTO.jpg>
[10/08/2026, 09:16:44] Ana: <anexado: 00000004-AUDIO.opus>
[10/08/2026, 09:18:09] Ana: <anexado: 00000005-AUDIO.opus>
[10/08/2026, 09:19:35] Ana: <anexado: 00000006-AUDIO.opus>
[10/08/2026, 09:20:02] Bruno: Fechou, vou olhar hoje.
EOF

rm -f "$OUT_ZIP"
(cd "$WORK" && zip -q -j "$OUT_ZIP" _chat.txt ./*.opus)
echo "wrote $OUT_ZIP" >&2
