#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"
if [[ -f .env ]]; then
    set -a
    source .env
    set +a
fi
: "${WIFI_SSID:?Set WIFI_SSID in firmware/.env or the build environment}"
: "${WIFI_PASSWORD:?Set WIFI_PASSWORD in firmware/.env or the build environment}"
mkdir -p target

# esp-idf-sys creates an IDF build project under target/. Point the partition
# table at this checkout explicitly because that generated project is not the
# same directory as the Rust workspace. Do not touch the generated file when
# its contents are unchanged; its timestamp would force esp-idf-sys to rebuild.
generated_defaults="$(mktemp)"
sed "s|^CONFIG_PARTITION_TABLE_CUSTOM_FILENAME=.*|CONFIG_PARTITION_TABLE_CUSTOM_FILENAME=\"${PWD}/partitions.csv\"|" \
    sdkconfig.defaults > "$generated_defaults"
if ! cmp -s "$generated_defaults" target/sdkconfig.defaults; then
    mv "$generated_defaults" target/sdkconfig.defaults
else
    rm "$generated_defaults"
fi
export ESP_IDF_SDKCONFIG_DEFAULTS="$PWD/target/sdkconfig.defaults"

source "$HOME/export-esp.sh"
export IDF_PATH="${IDF_PATH:-$HOME/esp/esp-idf}"
export IDF_PYTHON_ENV_PATH="${IDF_PYTHON_ENV_PATH:-$HOME/.espressif/python_env/idf5.5_py3.12_env}"
source "$IDF_PATH/export.sh"

case "${1:-build}" in
    build) cargo +esp build --release ;;
    run) cargo +esp run --release ;;
    *) echo "Usage: $0 [build|run]" >&2; exit 2 ;;
esac
