#!/usr/bin/env bash
# The terminal session asciinema records for the README GIF.
# Expects: cwd contains demo-export.zip, `harken` on PATH, small model cached.
set -euo pipefail

type_run() { # print "$ cmd" char by char, then run it
    printf '\033[1;32m$\033[0m '
    local cmd="$1" i
    for ((i = 0; i < ${#cmd}; i++)); do
        printf '%s' "${cmd:i:1}"
        sleep 0.03
    done
    printf '\n'
    sleep 0.3
    eval "$cmd"
    sleep 1.2
}

type_run 'harken whatsapp demo-export.zip --merge'
type_run 'ls demo-export-transcripts/'
type_run 'tail -n 8 demo-export-transcripts/_chat.transcribed.txt'
sleep 2
