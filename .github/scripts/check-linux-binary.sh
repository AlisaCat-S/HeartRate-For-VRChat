#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "Usage: $0 <binary> <expected-machine> <max-glibc-version>" >&2
    exit 2
fi

binary=$1
expected_machine=$2
max_glibc=$3

if [[ ! -f "$binary" ]]; then
    echo "Binary not found: $binary" >&2
    exit 1
fi

for command_name in readelf grep sed sort tail; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Required command not found: $command_name" >&2
        exit 1
    fi
done

machine=$(readelf --file-header "$binary" | sed -n 's/^[[:space:]]*Machine:[[:space:]]*//p')
if [[ -z "$machine" ]]; then
    echo "Could not read ELF machine from: $binary" >&2
    exit 1
fi
if [[ "$machine" != "$expected_machine" ]]; then
    echo "Unexpected ELF machine: $machine (expected $expected_machine)" >&2
    exit 1
fi

mapfile -t glibc_versions < <(
    readelf --version-info --wide "$binary" \
        | grep -oE 'GLIBC_[0-9]+([.][0-9]+)+' \
        | sed 's/^GLIBC_//' \
        | sort -Vu \
        || true
)

if [[ ${#glibc_versions[@]} -eq 0 ]]; then
    echo "No GLIBC version requirements found in: $binary" >&2
    exit 1
fi

highest_glibc=${glibc_versions[${#glibc_versions[@]}-1]}
highest_comparison=$(printf '%s\n%s\n' "$highest_glibc" "$max_glibc" | sort -V | tail -n 1)
if [[ "$highest_glibc" != "$max_glibc" && "$highest_comparison" == "$highest_glibc" ]]; then
    echo "GLIBC requirement $highest_glibc exceeds allowed maximum $max_glibc" >&2
    exit 1
fi

echo "ELF machine: $machine"
echo "Required GLIBC versions: ${glibc_versions[*]}"
echo "Highest required GLIBC version: $highest_glibc (maximum allowed: $max_glibc)"
