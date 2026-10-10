flash:
  bash firmware/build.sh run

upload:
  curl --fail \
     -H 'Content-Type: application/json' \
     --data-binary @decks/japanese_100.json \
     http://192.168.178.88/api/japanese/daily
