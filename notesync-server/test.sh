#!/usr/bin/env bash
# Проверка сервера через curl. Нужны: bash, curl, python3.
# Запуск:  ./test.sh            (сам поднимет сервер на порту 18787 с временной базой)
#          ./test.sh <адрес>    (проверит уже запущенный сервер, нужен NOTESYNC_TOKEN)
set -u
cd "$(dirname "$0")"

BIN=./target/release/notesync-server
TOKEN=testtoken123
PID=""
if [ $# -ge 1 ]; then
  BASE="$1"; TOKEN="${NOTESYNC_TOKEN:?задайте NOTESYNC_TOKEN}"
else
  PORT=18787; BASE="http://127.0.0.1:$PORT"
  TMP=$(mktemp -d)
  NOTESYNC_PORT=$PORT NOTESYNC_DB="$TMP/t.db" NOTESYNC_TOKEN=$TOKEN $BIN >"$TMP/server.log" 2>&1 &
  PID=$!
  trap 'kill $PID 2>/dev/null; rm -rf "$TMP"' EXIT
  for i in $(seq 1 50); do curl -s "$BASE/health" >/dev/null 2>&1 && break; sleep 0.1; done
fi

FAIL=0
H=(-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json")

# jget '<json>' 'python-выражение над d'
jget() { python3 -c "import sys,json; d=json.loads(sys.argv[1]); print($2)" "$1"; }
check() { # check "описание" "получено" "ожидалось"
  if [ "$2" == "$3" ]; then echo "  ok   $1"; else echo "  FAIL $1: получено [$2], ожидалось [$3]"; FAIL=1; fi
}
put() { # put id body base_rev edit_id device
  curl -s -X PUT "${H[@]}" "$BASE/notes/$1" \
    -d "$(python3 -c "import json,sys; print(json.dumps({'body':sys.argv[1],'base_rev':int(sys.argv[2]),'edit_id':sys.argv[3],'device_id':sys.argv[4]}, ensure_ascii=False))" "$2" "$3" "$4" "$5")"
}
get() { curl -s "${H[@]}" "$BASE$1"; }
resolve() { # resolve id vid action
  curl -s -X POST "${H[@]}" "$BASE/notes/$1/conflicts/$2/resolve" -d "{\"action\":\"$3\",\"device_id\":\"pc\"}"
}

echo "== доступ"
check "health без токена" "$(jget "$(curl -s $BASE/health)" "d['name']")" "notesync"
check "без токена 401" "$(curl -s -o /dev/null -w '%{http_code}' $BASE/notes)" "401"
check "неверный токен 401" "$(curl -s -o /dev/null -w '%{http_code}' -H 'Authorization: Bearer nope' $BASE/notes)" "401"

echo "== создание и правка"
R=$(put n1 "你好，世界" 0 e1 phone)
check "создание: status" "$(jget "$R" "d['status']")" "ok"
check "создание: rev" "$(jget "$R" "d['rev']")" "1"
R=$(put n1 "你好，世界！第二версия" 1 e2 phone)
check "правка: rev" "$(jget "$R" "d['rev']")" "2"
R=$(put n1 "другой текст, который не должен записаться" 1 e2 phone)
check "повтор той же правки (edit_id): тот же ответ" "$(jget "$R" "d['rev']")" "2"
N=$(get /notes/n1)
check "повтор не создал дубль: rev" "$(jget "$N" "d['rev']")" "2"
check "UTF-8 (иероглифы) сохранены" "$(jget "$N" "d['body']")" "你好，世界！第二версия"

echo "== конфликт: телефон правил старую ревизию"
R=$(put n1 "правка с ПК" 1 e3 pc)
check "status = conflict" "$(jget "$R" "d['status']")" "conflict"
VID=$(jget "$R" "d['conflict_version_id']")
N=$(get /notes/n1)
check "основной текст не изменился" "$(jget "$N" "d['body']")" "你好，世界！第二версия"
check "rev не изменился" "$(jget "$N" "d['rev']")" "2"
check "conflicts = 1" "$(jget "$N" "d['conflicts']")" "1"
V=$(get /notes/n1/versions)
check "конфликтная версия сохранена с текстом" "$(jget "$V" "[v['body'] for v in d['versions'] if v['id']==$VID][0]")" "правка с ПК"
check "в логе есть conflict" "$(jget "$(get /log)" "any(e['event']=='conflict' for e in d['log'])")" "True"

echo "== одинаковый текст при старой ревизии — не конфликт"
R=$(put n1 "你好，世界！第二версия" 1 e4 pc)
check "status ok" "$(jget "$R" "d['status']")" "ok"
check "conflicts всё ещё 1" "$(jget "$(get /notes/n1)" "d['conflicts']")" "1"

echo "== resolve: use_this"
R=$(resolve n1 $VID use_this)
check "status ok" "$(jget "$R" "d['status']")" "ok"
N=$(get /notes/n1)
check "текст стал конфликтным" "$(jget "$N" "d['body']")" "правка с ПК"
check "rev = 3" "$(jget "$N" "d['rev']")" "3"
check "conflicts = 0" "$(jget "$N" "d['conflicts']")" "0"
R=$(resolve n1 $VID use_this)
check "повторный resolve отклонён" "$(jget "$R" "'error' in d")" "True"

echo "== resolve: keep_both"
R=$(put n1 "вторая правка с телефона" 1 e5 phone)
VID2=$(jget "$R" "d['conflict_version_id']")
R=$(resolve n1 $VID2 keep_both)
NEW=$(jget "$R" "d['new_note_id']")
check "создана новая заметка с конфликтным текстом" "$(jget "$(get /notes/$NEW)" "d['body']")" "вторая правка с телефона"
check "у исходной заметки текст прежний" "$(jget "$(get /notes/n1)" "d['body']")" "правка с ПК"

echo "== resolve: keep_server"
R=$(put n1 "ещё одна старая правка" 2 e6 phone)
VID3=$(jget "$R" "d['conflict_version_id']")
R=$(resolve n1 $VID3 keep_server)
check "текст остался серверным" "$(jget "$(get /notes/n1)" "d['body']")" "правка с ПК"
check "conflicts = 0" "$(jget "$(get /notes/n1)" "d['conflicts']")" "0"

echo "== удаление"
R=$(curl -s -X DELETE "${H[@]}" "$BASE/notes/n1?base_rev=1&device_id=pc")
check "удаление по старой rev отклонено" "$(jget "$R" "d['status']")" "conflict"
check "заметка не удалена" "$(jget "$(get /notes/n1)" "d['deleted']")" "False"
REV=$(jget "$(get /notes/n1)" "d['rev']")
R=$(curl -s -X DELETE "${H[@]}" "$BASE/notes/n1?base_rev=$REV&device_id=pc")
check "удаление по актуальной rev" "$(jget "$R" "d['status']")" "ok"
check "заметка помечена удалённой" "$(jget "$(get /notes/n1)" "d['deleted']")" "True"
R=$(curl -s -X DELETE "${H[@]}" "$BASE/notes/n1?base_rev=$REV&device_id=pc")
check "повторное удаление — ok" "$(jget "$R" "d['status']")" "ok"
check "удалённая заметка есть в списке изменений" "$(jget "$(get '/notes?since=0')" "any(n['id']=='n1' and n['deleted'] for n in d['notes'])")" "True"

echo "== список изменений (since)"
S=$(jget "$(get '/notes?since=0')" "d['seq']")
check "since=<последний seq> даёт пустой список" "$(jget "$(get "/notes?since=$S")" "len(d['notes'])")" "0"
check "нет заметки — 404" "$(curl -s -o /dev/null -w '%{http_code}' "${H[@]}" $BASE/notes/nope)" "404"

echo
if [ $FAIL -eq 0 ]; then echo "ВСЕ ПРОВЕРКИ ПРОШЛИ"; else echo "ЕСТЬ ОШИБКИ"; fi
exit $FAIL
