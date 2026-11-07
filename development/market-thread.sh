#!/bin/bash
# 20-message thread at Mara's fruit stall: her name, where the apples come from, the price, a
# discount on one apple (should be refused), a discount on five (should be granted). The elf and
# the warrior speak through `persona say` from a one-line goal per beat; Mara answers through
# `persona ask` (coverage, backfill), fully live. Every call gets the last lines of the thread
# (`recent=`) and who is speaking to her (`from=`), so nobody repeats themselves; the standing
# context is stated once, in the first beat. Needs the market scene up (see
# development/runbook-dev-playground.md). Usage: bash development/market-thread.sh
cd "$(dirname "$0")/.."
D=${DF_TABLE_EXE:-./artifacts/build/bubbles/debug/df-table.exe}
P=${DF_TABLE_PORT:-7879}
N=0
LAST=""
LAST_SPEAKER=""
HISTORY=()

# Mara's discount rules live in her persona (a trigger is a statement the model judges, never a
# keyword). Adding them again to a persona that has them is refused and ignored.
$D dev --port $P persona behavior fruit_vendor add firm-price "Holds her price on a single piece of fruit and politely refuses any discount on just one; she says each piece is already priced fairly." >/dev/null 2>&1
$D dev --port $P persona behavior fruit_vendor add bulk-discount "Offers a modest discount, warmly and with a little pride, when a customer buys five or more pieces of fruit." >/dev/null 2>&1
$D dev --port $P persona trigger fruit_vendor add one-piece-haggle behavior=firm-price "A customer asks for a discount on a single piece of fruit" >/dev/null 2>&1
$D dev --port $P persona trigger fruit_vendor add bulk-buyer behavior=bulk-discount "A customer buys five or more pieces of fruit" >/dev/null 2>&1

# The last six lines of the thread, as repeated `recent=` options.
recent_args() {
  local start=$(( ${#HISTORY[@]} > 6 ? ${#HISTORY[@]} - 6 : 0 )) i
  RECENT=()
  for ((i = start; i < ${#HISTORY[@]}; i++)); do RECENT+=("recent=${HISTORY[$i]}"); done
}

say_as() { # speaker, instruction
  local who="$1" instruction="$2" out line
  recent_args
  out=$(timeout 200 $D dev --port $P persona say "$who" "${RECENT[@]}" "$instruction" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  N=$((N + 1))
  printf '[%02d] %s: %s\n' "$N" "$who" "${line:-<no line: $out>}"
  printf '%s\n' "$out" | sed -n '2,$p' | grep -E 'quoted|fired triggers' | sed 's/^/       /'
  LAST="$line"
  LAST_SPEAKER="$who"
  HISTORY+=("$who: $line")
  sleep 3
}

mara() { # what is happening now
  local out line
  recent_args
  out=$(timeout 280 $D dev --port $P persona ask fruit_vendor "from=$LAST_SPEAKER" "${RECENT[@]}" "situation=$1" "$LAST" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  N=$((N + 1))
  printf '[%02d] fruit_vendor: %s\n' "$N" "${line:-<no line: $out>}"
  printf '%s\n' "$out" | sed -n '2,$p' | sed 's/^/       /'
  HISTORY+=("fruit_vendor: $line")
  sleep 3
}

say_as elf "You are at Mara's fruit stall in the town market with the warrior, buying fruit for the road north. Greet her warmly and ask her name, in your own words."
mara "A customer greets her at her stall and asks her name."
say_as elf "Pick up an apple and ask her where her apples come from."
mara "The customer asks where the apples come from."
say_as warrior "Step in gruffly: ask if the apples will survive a week on the road."
mara "The warrior asks how long the apples keep."
say_as elf "Ask how much one apple costs."
mara "The customer asks the price of one apple."
say_as elf "Ask for a discount on just that one apple."
mara "The customer asks for a discount on one apple."
say_as warrior "Grumble that you are not made of coin; one apple at that price is robbery."
mara "The warrior grumbles about the price."
say_as elf "Offer to buy five apples instead and ask for a discount on the five."
mara "The customer now wants five apples and asks for a discount."
say_as elf "Ask what the five apples will cost with her discount."
mara "The customer asks the total for five apples with the discount."
say_as warrior "Grunt that five is fine and you will carry the sack; ask her to pack them."
mara "The warrior asks her to pack the five apples."
say_as elf "Thank her and ask once more for her name so you can find the stall again."
mara "The customer thanks her and asks her name again and where to find the stall."
