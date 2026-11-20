#!/usr/bin/env bash
# The translations (src/i18n.rs). Run it in the azrak-dev distrobox (it has gettext, xtr and
# slint-tr-extractor; see .claude/skills/azrak-fork/SKILL.md).
#
#   scripts/i18n.sh extract   # the texts in ui/*.slint (@tr) and src/ (tr!) → i18n/azrak.pot, then
#                             # each i18n/<lang>/LC_MESSAGES/azrak.po updated: new texts come in
#                             # empty, gone ones are marked obsolete (#~), translations are kept
#   scripts/i18n.sh check     # fails when a text has no translation, or a translation's {…} don't
#                             # match its English (scripts/check.sh runs it)
set -euo pipefail
cd "$(dirname "$0")/.."
bin="$HOME/.cargo/bin"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pot() {
    # No default context: the build doesn't use one either (build.rs), so "Back" is one text.
    "$bin/slint-tr-extractor" --no-default-translation-context -o "$work/ui.pot" ui/*.slint
    "$bin/xtr" --add-location file -o "$work/rust.pot" src/main.rs
    msgcat --use-first --sort-by-file "$work/ui.pot" "$work/rust.pot" -o "$1"
}

case "${1:-}" in
extract)
    pot i18n/azrak.pot
    for po in i18n/*/LC_MESSAGES/azrak.po; do
        # No fuzzy guesses: a near-match in another text's words would ship as its translation.
        msgmerge --quiet --update --backup=none --no-fuzzy-matching --no-wrap "$po" i18n/azrak.pot
        echo "$po: $(msgattrib --untranslated --no-obsolete "$po" | grep '^msgid "' | grep -vc '^msgid ""$' || true) to translate"
    done
    ;;
check)
    pot "$work/azrak.pot"
    status=0
    for po in i18n/*/LC_MESSAGES/azrak.po; do
        msgfmt --check-format --check-header -o /dev/null "$po" || status=1
        msgmerge --quiet --no-fuzzy-matching -o "$work/merged.po" "$po" "$work/azrak.pot"
        # The header is the only empty msgid; anything else empty is a text with no translation.
        missing=$(msgattrib --untranslated --no-obsolete "$work/merged.po" | grep '^msgid "' | grep -vc '^msgid ""$' || true)
        if [ "$missing" != 0 ]; then
            echo "$po: $missing texts without a translation (run scripts/i18n.sh extract, then translate them):"
            msgattrib --untranslated --no-obsolete "$work/merged.po" | grep '^msgid "' | grep -v '^msgid ""$' | head -20
            status=1
        fi
        python3 - "$work/merged.po" <<'PY' || status=1
import re, sys
# Every {…} in the English must be in the translation, and no other: a typo'd {nmae} would show.
text = open(sys.argv[1], encoding="utf-8").read()
bad = 0
for block in text.split("\n\n"):
    if block.lstrip().startswith("#~"):
        continue
    def strings(key):
        out = []
        for m in re.finditer(r'^%s "(.*)"\n((?:".*"\n?)*)' % re.escape(key), block + "\n", re.M):
            out.append(m.group(1) + "".join(re.findall(r'^"(.*)"', m.group(2), re.M)))
        return out
    ids = strings("msgid") + strings("msgid_plural")
    strs = strings("msgstr") + [s for i in range(6) for s in strings("msgstr[%d]" % i)]
    if not ids or not ids[0] or not any(strs):
        continue
    want = set(re.findall(r"\{[^{}]*\}", " ".join(ids))) - {"{n}"}
    for s in strs:
        if not s:
            continue
        have = set(re.findall(r"\{[^{}]*\}", s)) - {"{n}"}
        if have != want:
            print(f"placeholders differ: {ids[0]!r} → {s!r}")
            bad += 1
sys.exit(1 if bad else 0)
PY
    done
    exit $status
    ;;
*)
    echo "usage: scripts/i18n.sh extract|check" >&2
    exit 2
    ;;
esac
