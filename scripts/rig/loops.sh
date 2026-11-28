#!/bin/zsh
# RIG-CHECKLIST 4.1 (`loop`): Loop 1 · 2 · 4 · 8 on a timecode deck.
#
# Before: the cable loop, Dub on the scratch library in Performance, deck A on
# TC with a programmed track (drum machine, locked grid) loaded, BPM given
# below. Plays steady stretches of two takes into deck A, clicks each loop
# length on and off, records deck A, and measures it with `dub loop-check`.
#
#     scripts/rig/loops.sh OUT_DIR BPM
#
# Button centres (points, Performance, 1792×1120): override to suit.
source "${0:A:h}/lib.sh"
OUT=${1:?out dir}; BPM=${2:?track bpm}
L1=${RIG_LOOP1_X:-488}; L2=${RIG_LOOP2_X:-521}; L4=${RIG_LOOP4_X:-553}; L8=${RIG_LOOP8_X:-585}; LY=${RIG_LOOP_Y:-431}
T=$RIG_ROOT/testdata/timecode/full

# deck B take, 38–65 s: steady at 0 % (testdata/timecode/README.md)
rig_play $OUT a $T/sl3-deckB-60s.flac 38 27 || exit 1
rig_press 5 in-1 $L1 $LY;  rig_press 10.5 out-1 $L1 $LY
rig_press 12 in-2 $L2 $LY; rig_press 18 out-2 $L2 $LY
rig_press 20 in-4 $L4 $LY; rig_press 27.3 out-4 $L4 $LY
rig_wait_take
# pitch-step take, 261–284 s: steady at 0 %
rig_play $OUT b $T/sl3-serato-1122-0844-left.wav 261 23 || exit 1
rig_press 6 in-8 $L8 $LY; rig_press 20 out-8 $L8 $LY
rig_wait_take

loops() { awk '{t[$1]=$2} END {for (n in t) if (n ~ /^in-/) {b=substr(n,4); printf "%s,%s,%s ", t[n], t["out-" b], b}}' "$1"; }
"$RIG_DUB" loop-check $OUT/a.wav --bpm $BPM ${=$(loops $OUT/a-presses.txt)}
"$RIG_DUB" loop-check $OUT/b.wav --bpm $BPM ${=$(loops $OUT/b-presses.txt)}
